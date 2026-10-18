//! The compiled graph: a flat schedule the audio thread runs without
//! traversal, hashing, or allocation (ARCHITECTURE §7).
//!
//! Steps are grouped into waves. No step in a wave reads another's outputs,
//! so a wave's steps can run in any order or at the same time. Each wave
//! runs in three phases:
//!
//! 1. Move out: every step's output buffers and its node's state are swapped
//!    out of the shared pool into a work slot (pointer swaps, no allocation;
//!    DECISIONS D23).
//! 2. Run: the pool is now only read, so steps run on the audio thread or
//!    spread across the real-time worker pool, each writing only its own
//!    slot. Steps that write device outputs run last, one at a time, in
//!    schedule order, so outputs sum the same way on any number of cores.
//! 3. Move back.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{
    AutoFlags, BufferRef, ControlEvent, EngineInfo, HostAccess, InputSlot, Inputs, Io, MidiEvent,
    Node, NodeMeta, NodePath, OscMessage, OutputSlot, Outputs, ParamSlot, Phase, PrepareCtx,
    ProcessCtx, Signature, SpectrumBuf, TelemetryAccess, TempoMap, TransportView,
};

use crate::pool::{Job, RtPool};
use crate::telemetry::TelemetryStore;

/// A zero-sized stand-in for a node whose state lives elsewhere for a moment
/// (carried over from the previous graph at install time, or moved into a
/// work slot while its step runs). `Box<Vacant>` doesn't allocate.
pub(crate) struct Vacant;

impl Node for Vacant {
    fn meta() -> NodeMeta {
        NodeMeta::new("vacant", "Vacant", ac_core::Category::Utility)
    }
    fn describe(_: &ac_core::Config) -> Signature {
        Signature::new()
    }
    fn new(_: &ac_core::Config, _: &PrepareCtx) -> Self {
        Vacant
    }
    fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}
    fn reset(&mut self) {}
}

pub(crate) struct NodeBox {
    pub node: Box<dyn Node>,
    pub path: NodePath,
    /// The first block after creation or reset fires *On load* start triggers.
    pub load_pending: bool,
}

/// What a step does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StepKind {
    /// Run a node (or one phase of a Feedback node).
    Node { node: u32, phase: Phase },
    /// Float → Audio from a block-rate value: fill the step's one sample
    /// output with it.
    FillFromScalar { scalar: u32 },
}

/// How a muted node passes signal through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bypass {
    None,
    Audio { input: u32, output: u32 },
    Float { input: u32, output: u32 },
}

pub(crate) type Range = (u32, u32);

/// The transport for one block, without the tempo map (the graph owns it).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BlockTransport {
    pub playing: bool,
    pub recording: bool,
    pub position: u64,
    pub started: bool,
    pub stopped: bool,
}

fn range<T>(v: &[T], r: Range) -> &[T] {
    v.get(r.0 as usize..r.1 as usize).unwrap_or(&[])
}

/// Telemetry for one output socket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SocketTele {
    pub slot: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub kind: StepKind,
    pub muted: bool,
    pub bypass: Bypass,
    /// Writes device outputs: runs on the audio thread after the wave's
    /// parallel steps.
    pub serial: bool,
    pub inputs: Range,
    pub outputs: Range,
    pub o_samples: Range,
    pub o_scalars: Range,
    pub o_ints: Range,
    pub o_triggers: Range,
    pub o_midi: Range,
    pub o_spectra: Range,
    pub o_buffers: Range,
    pub tele_values: Range,
    pub tele_scope: Range,
    pub tele_scope_pos: Option<u32>,
    pub tele_time: Option<u32>,
    /// One per output socket (parallel to `outputs`), if socket telemetry is on.
    pub socket_tele: Range,
}

/// Steps `start..end` of the schedule; `start..parallel_end` may run at the
/// same time, `parallel_end..end` write device outputs and run one by one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wave {
    pub start: u32,
    pub parallel_end: u32,
    pub end: u32,
}

/// Every buffer and value wires carry, shared by all steps.
#[derive(Default)]
pub(crate) struct Pool {
    pub samples: Vec<Vec<f32>>,
    pub scalars: Vec<f32>,
    pub ints: Vec<i32>,
    pub triggers: Vec<Vec<u32>>,
    pub midi: Vec<Vec<MidiEvent>>,
    pub spectra: Vec<SpectrumBuf>,
    pub buffers: Vec<BufferRef>,
}

/// One step's outputs and node state while its wave runs.
pub(crate) struct StepWork {
    node: Box<dyn Node>,
    load_pending: bool,
    samples: Vec<Vec<f32>>,
    scalars: Vec<f32>,
    ints: Vec<i32>,
    triggers: Vec<Vec<u32>>,
    midi: Vec<Vec<MidiEvent>>,
    spectra: Vec<SpectrumBuf>,
    buffers: Vec<BufferRef>,
}

impl StepWork {
    /// Room for a step with up to these many outputs of each kind.
    pub fn sized(
        samples: usize,
        scalars: usize,
        ints: usize,
        triggers: usize,
        midi: usize,
        spectra: usize,
        buffers: usize,
    ) -> Self {
        StepWork {
            node: Box::new(Vacant),
            load_pending: false,
            samples: (0..samples).map(|_| Vec::new()).collect(),
            scalars: vec![0.0; scalars],
            ints: vec![0; ints],
            triggers: (0..triggers).map(|_| Vec::new()).collect(),
            midi: (0..midi).map(|_| Vec::new()).collect(),
            spectra: (0..spectra).map(|_| SpectrumBuf::default()).collect(),
            buffers: vec![None; buffers],
        }
    }

    pub fn clone_empty(&self) -> Self {
        Self::sized(
            self.samples.len(),
            self.scalars.len(),
            self.ints.len(),
            self.triggers.len(),
            self.midi.len(),
            self.spectra.len(),
            self.buffers.len(),
        )
    }
}

pub struct CompiledGraph {
    pub(crate) generation: u64,
    pub(crate) sample_rate: f64,
    pub(crate) max_block: usize,
    pub(crate) tempo: TempoMap,
    pub(crate) nodes: Vec<NodeBox>,
    /// (new node index, old node index) pairs whose state moves over on install.
    pub(crate) carry: Vec<(u32, u32)>,
    pub(crate) steps: Vec<Step>,
    pub(crate) waves: Vec<Wave>,
    pub(crate) input_slots: Vec<InputSlot>,
    pub(crate) output_slots: Vec<OutputSlot>,
    pub(crate) out_samples: Vec<u32>,
    pub(crate) out_scalars: Vec<u32>,
    pub(crate) out_ints: Vec<u32>,
    pub(crate) out_triggers: Vec<u32>,
    pub(crate) out_midi: Vec<u32>,
    pub(crate) out_spectra: Vec<u32>,
    pub(crate) out_buffers: Vec<u32>,
    pub(crate) chan_table: Vec<u32>,
    pub(crate) pool: Pool,
    pub(crate) params: Vec<ParamSlot>,
    /// Smoothing length in samples per param (0 = no smoothing).
    pub(crate) param_smooth: Vec<u32>,
    /// Params currently ramping or fired (indices); capacity = params.len().
    pub(crate) active_params: Vec<u32>,
    pub(crate) telemetry: Arc<TelemetryStore>,
    pub(crate) socket_tele: Vec<SocketTele>,
    /// One work slot per step of the widest wave.
    pub(crate) work: Vec<StepWork>,
    pub(crate) level_decay: f32,
    /// Output latency in samples.
    pub(crate) latency: u32,
}

/// What every step of a block shares (read-only, so it can be shared
/// across threads).
struct Shared<'a> {
    steps: &'a [Step],
    input_slots: &'a [InputSlot],
    output_slots: &'a [OutputSlot],
    chan_table: &'a [u32],
    pool: &'a Pool,
    params: &'a [ParamSlot],
    telemetry: &'a TelemetryStore,
    socket_tele: &'a [SocketTele],
    level_decay: f32,
    transport: TransportView<'a>,
    auto: AutoFlags,
    clock: Option<fn() -> u64>,
    sample_rate: f64,
    frames: usize,
    engine: EngineInfo,
    host_input: &'a [f32],
    host_in_channels: usize,
    host_stride: usize,
    host_offset: usize,
    host_midi: &'a [MidiEvent],
    host_controls: &'a [ControlEvent],
}

/// A run of steps for one worker: `steps[i]` uses `work[i]`.
struct Chunk<'a, 'b> {
    shared: &'b Shared<'a>,
    steps: &'b [Step],
    work: &'b mut [StepWork],
}

impl Job for Chunk<'_, '_> {
    fn run(&mut self) {
        for (step, work) in self.steps.iter().zip(self.work.iter_mut()) {
            run_step(self.shared, step, work, &mut [], None, None);
        }
    }
}

/// Upper bound on jobs per wave; more parallel steps share jobs.
const MAX_JOBS: usize = 64;

impl CompiledGraph {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Paths of the nodes in this graph, in creation order.
    pub fn node_paths(&self) -> impl Iterator<Item = &NodePath> {
        self.nodes.iter().map(|n| &n.path)
    }

    /// Waves in the schedule.
    pub fn wave_count(&self) -> usize {
        self.waves.len()
    }

    /// Apply a parameter change: ramp Float params, set others immediately.
    pub(crate) fn set_param(&mut self, slot: u32, value: f32) {
        let i = slot as usize;
        let smooth = self.param_smooth.get(i).copied().unwrap_or(0);
        let Some(p) = self.params.get_mut(i) else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        if smooth == 0 {
            p.value = value;
            p.target = value;
            p.remaining = 0;
            p.step = 0.0;
        } else {
            let start = p.value;
            p.target = value;
            p.remaining = smooth;
            p.step = (value - start) / smooth as f32;
            if self.active_params.len() < self.active_params.capacity()
                && !self.active_params.contains(&slot)
            {
                self.active_params.push(slot);
            } else if !self.active_params.contains(&slot) {
                // No room to track the ramp: jump instead of allocating.
                p.value = value;
                p.remaining = 0;
                p.step = 0.0;
            }
        }
    }

    /// Press an unwired trigger's inline button for the next block.
    pub(crate) fn fire(&mut self, slot: u32) {
        if let Some(p) = self.params.get_mut(slot as usize) {
            p.fired = true;
            if self.active_params.len() < self.active_params.capacity()
                && !self.active_params.contains(&slot)
            {
                self.active_params.push(slot);
            }
        }
    }

    /// Advance ramps by `frames` and clear fired flags (after a block).
    pub(crate) fn advance_params(&mut self, frames: usize) {
        let mut i = 0;
        while i < self.active_params.len() {
            let slot = self.active_params[i] as usize;
            let mut done = true;
            if let Some(p) = self.params.get_mut(slot) {
                p.fired = false;
                if p.remaining > 0 {
                    let n = (frames as u32).min(p.remaining);
                    p.value += p.step * n as f32;
                    p.remaining -= n;
                    if p.remaining == 0 {
                        p.value = p.target;
                        p.step = 0.0;
                    } else {
                        done = false;
                    }
                }
            }
            if done {
                self.active_params.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn reset_nodes(&mut self) {
        for n in &mut self.nodes {
            n.node.reset();
            n.load_pending = true;
        }
    }

    /// Run every step for one block, spreading waves across `workers` when
    /// given.
    pub(crate) fn run(
        &mut self,
        frames: usize,
        block: BlockTransport,
        host: &mut HostAccess<'_>,
        clock: Option<fn() -> u64>,
        workers: Option<&RtPool>,
        engine: EngineInfo,
    ) {
        let frames = frames.min(self.max_block).max(1);
        let transport = TransportView {
            playing: block.playing,
            recording: block.recording,
            position: block.position,
            tempo: &self.tempo,
            started: block.started,
            stopped: block.stopped,
        };
        let auto = AutoFlags {
            play: transport.started,
            stop: transport.stopped,
            load: false,
        };
        for wi in 0..self.waves.len() {
            let wave = self.waves[wi];
            let (start, mid, end) = (
                wave.start as usize,
                wave.parallel_end as usize,
                wave.end as usize,
            );
            let Some(steps) = self.steps.get(start..end) else {
                continue;
            };
            for (k, step) in steps.iter().enumerate() {
                if let Some(work) = self.work.get_mut(k) {
                    move_out(
                        step,
                        work,
                        &mut self.nodes,
                        &mut self.pool,
                        &self.out_samples,
                        &self.out_scalars,
                        &self.out_ints,
                        &self.out_triggers,
                        &self.out_midi,
                        &self.out_spectra,
                        &self.out_buffers,
                        frames,
                    );
                }
            }
            {
                let shared = Shared {
                    steps: &self.steps,
                    input_slots: &self.input_slots,
                    output_slots: &self.output_slots,
                    chan_table: &self.chan_table,
                    pool: &self.pool,
                    params: &self.params,
                    telemetry: &self.telemetry,
                    socket_tele: &self.socket_tele,
                    level_decay: self.level_decay,
                    transport,
                    auto,
                    clock,
                    sample_rate: self.sample_rate,
                    frames,
                    engine,
                    host_input: host.input,
                    host_in_channels: host.in_channels,
                    host_stride: host.stride,
                    host_offset: host.offset,
                    host_midi: host.midi,
                    host_controls: host.controls,
                };
                let parallel = mid - start;
                let width = (end - start).min(self.work.len());
                let work = &mut self.work[..width];
                let (par_work, serial_work) = work.split_at_mut(parallel.min(work.len()));
                let par_steps = &shared.steps[start..mid];
                match workers {
                    Some(rt) if parallel >= 2 && rt.workers() > 0 => {
                        run_parallel(rt, &shared, par_steps, par_work);
                    }
                    _ => {
                        for (step, w) in par_steps.iter().zip(par_work.iter_mut()) {
                            run_step(&shared, step, w, &mut [], None, None);
                        }
                    }
                }
                let serial_steps = &shared.steps[mid..end];
                for (step, w) in serial_steps.iter().zip(serial_work.iter_mut()) {
                    run_step(
                        &shared,
                        step,
                        w,
                        host.output,
                        host.midi_out.as_deref_mut(),
                        host.osc_out.as_deref_mut(),
                    );
                }
            }
            for (k, step) in self.steps[start..end].iter().enumerate() {
                if let Some(work) = self.work.get_mut(k) {
                    move_back(
                        step,
                        work,
                        &mut self.nodes,
                        &mut self.pool,
                        &self.out_samples,
                        &self.out_scalars,
                        &self.out_ints,
                        &self.out_triggers,
                        &self.out_midi,
                        &self.out_spectra,
                        &self.out_buffers,
                    );
                }
            }
        }
    }
}

/// Split a wave's parallel steps into jobs and run them on the pool.
fn run_parallel(rt: &RtPool, shared: &Shared<'_>, steps: &[Step], work: &mut [StepWork]) {
    let threads = rt.workers() + 1;
    // A few jobs per thread so a slow step doesn't hold up the wave.
    let jobs = (threads * 4).min(steps.len()).clamp(1, MAX_JOBS);
    let per = steps.len().div_ceil(jobs);
    let mut chunks: [Option<Chunk<'_, '_>>; MAX_JOBS] = std::array::from_fn(|_| None);
    let mut n = 0;
    for (s, w) in steps.chunks(per).zip(work.chunks_mut(per)) {
        if let Some(slot) = chunks.get_mut(n) {
            *slot = Some(Chunk {
                shared,
                steps: s,
                work: w,
            });
            n += 1;
        }
    }
    rt.run(&mut chunks[..n]);
}

impl<J: Job> Job for Option<J> {
    fn run(&mut self) {
        if let Some(j) = self {
            j.run();
        }
    }
}

/// Swap a step's output buffers and node out of the pool into its work slot.
#[allow(clippy::too_many_arguments)]
fn move_out(
    step: &Step,
    work: &mut StepWork,
    nodes: &mut [NodeBox],
    pool: &mut Pool,
    out_samples: &[u32],
    out_scalars: &[u32],
    out_ints: &[u32],
    out_triggers: &[u32],
    out_midi: &[u32],
    out_spectra: &[u32],
    out_buffers: &[u32],
    frames: usize,
) {
    if let StepKind::Node { node, .. } = step.kind
        && let Some(nb) = nodes.get_mut(node as usize)
    {
        std::mem::swap(&mut nb.node, &mut work.node);
        work.load_pending = nb.load_pending;
    }
    for (k, &g) in range(out_samples, step.o_samples).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.samples.get_mut(g as usize), work.samples.get_mut(k)) {
            std::mem::swap(a, b);
            let n = frames.min(b.len());
            b[..n].fill(0.0);
        }
    }
    for (k, &g) in range(out_triggers, step.o_triggers).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.triggers.get_mut(g as usize), work.triggers.get_mut(k)) {
            std::mem::swap(a, b);
            b.clear();
        }
    }
    for (k, &g) in range(out_midi, step.o_midi).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.midi.get_mut(g as usize), work.midi.get_mut(k)) {
            std::mem::swap(a, b);
            b.clear();
        }
    }
    for (k, &g) in range(out_spectra, step.o_spectra).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.spectra.get_mut(g as usize), work.spectra.get_mut(k)) {
            std::mem::swap(a, b);
            b.clear();
        }
    }
    for (k, &g) in range(out_buffers, step.o_buffers).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.buffers.get_mut(g as usize), work.buffers.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_scalars, step.o_scalars).iter().enumerate() {
        if let Some(s) = work.scalars.get_mut(k) {
            *s = pool.scalars.get(g as usize).copied().unwrap_or(0.0);
        }
    }
    for (k, &g) in range(out_ints, step.o_ints).iter().enumerate() {
        if let Some(s) = work.ints.get_mut(k) {
            *s = pool.ints.get(g as usize).copied().unwrap_or(0);
        }
    }
}

/// Undo [`move_out`].
#[allow(clippy::too_many_arguments)]
fn move_back(
    step: &Step,
    work: &mut StepWork,
    nodes: &mut [NodeBox],
    pool: &mut Pool,
    out_samples: &[u32],
    out_scalars: &[u32],
    out_ints: &[u32],
    out_triggers: &[u32],
    out_midi: &[u32],
    out_spectra: &[u32],
    out_buffers: &[u32],
) {
    if let StepKind::Node { node, phase } = step.kind
        && let Some(nb) = nodes.get_mut(node as usize)
    {
        std::mem::swap(&mut nb.node, &mut work.node);
        if phase != Phase::FeedbackRead {
            nb.load_pending = false;
        }
    }
    for (k, &g) in range(out_samples, step.o_samples).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.samples.get_mut(g as usize), work.samples.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_triggers, step.o_triggers).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.triggers.get_mut(g as usize), work.triggers.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_midi, step.o_midi).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.midi.get_mut(g as usize), work.midi.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_spectra, step.o_spectra).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.spectra.get_mut(g as usize), work.spectra.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_buffers, step.o_buffers).iter().enumerate() {
        if let (Some(a), Some(b)) = (pool.buffers.get_mut(g as usize), work.buffers.get_mut(k)) {
            std::mem::swap(a, b);
        }
    }
    for (k, &g) in range(out_scalars, step.o_scalars).iter().enumerate() {
        if let (Some(d), Some(s)) = (pool.scalars.get_mut(g as usize), work.scalars.get(k)) {
            *d = *s;
        }
    }
    for (k, &g) in range(out_ints, step.o_ints).iter().enumerate() {
        if let (Some(d), Some(s)) = (pool.ints.get_mut(g as usize), work.ints.get(k)) {
            *d = *s;
        }
    }
}

/// Run one step whose outputs are in `work`. `host_output`, `midi_out`,
/// and `osc_out` are the device outputs for serial steps, and empty for the
/// rest.
fn run_step(
    shared: &Shared<'_>,
    step: &Step,
    work: &mut StepWork,
    host_output: &mut [f32],
    midi_out: Option<&mut Vec<MidiEvent>>,
    osc_out: Option<&mut Vec<OscMessage>>,
) {
    let frames = shared.frames;
    let phase = match step.kind {
        StepKind::FillFromScalar { scalar } => {
            let v = shared
                .pool
                .scalars
                .get(scalar as usize)
                .copied()
                .unwrap_or(0.0);
            if let Some(buf) = work.samples.first_mut() {
                let n = frames.min(buf.len());
                buf[..n].fill(v);
            }
            return;
        }
        StepKind::Node { phase, .. } => phase,
    };
    let mut auto = shared.auto;
    auto.load = work.load_pending;
    let in_slots = range(shared.input_slots, step.inputs);
    let out_slots = range(shared.output_slots, step.outputs);
    let (vs, ve) = (step.tele_values.0 as usize, step.tele_values.1 as usize);
    let (ss, se) = (step.tele_scope.0 as usize, step.tele_scope.1 as usize);
    let telemetry = shared.telemetry;
    let tele = TelemetryAccess {
        values: telemetry.node_values.get(vs..ve).unwrap_or(&[]),
        scope: telemetry.scope.get(ss..se).unwrap_or(&[]),
        scope_pos: step
            .tele_scope_pos
            .and_then(|p| telemetry.scope_pos.get(p as usize)),
    };
    let n_s = (step.o_samples.1 - step.o_samples.0) as usize;
    let n_sc = (step.o_scalars.1 - step.o_scalars.0) as usize;
    let n_i = (step.o_ints.1 - step.o_ints.0) as usize;
    let n_t = (step.o_triggers.1 - step.o_triggers.0) as usize;
    let n_m = (step.o_midi.1 - step.o_midi.0) as usize;
    let n_sp = (step.o_spectra.1 - step.o_spectra.0) as usize;
    let n_b = (step.o_buffers.1 - step.o_buffers.0) as usize;
    let out_channels = if host_output.is_empty() {
        0
    } else {
        host_output.len() / shared.host_stride.max(1)
    };
    let pool = shared.pool;
    {
        let mut io = Io {
            inputs: Inputs {
                slots: in_slots,
                frames,
                params: shared.params,
                scalars: &pool.scalars,
                ints: &pool.ints,
                samples: &pool.samples,
                chan_table: shared.chan_table,
                triggers: &pool.triggers,
                midi: &pool.midi,
                spectra: &pool.spectra,
                buffers: &pool.buffers,
                auto,
            },
            outputs: Outputs {
                slots: out_slots,
                frames,
                samples: work.samples.get_mut(..n_s).unwrap_or(&mut []),
                scalars: work.scalars.get_mut(..n_sc).unwrap_or(&mut []),
                ints: work.ints.get_mut(..n_i).unwrap_or(&mut []),
                triggers: work.triggers.get_mut(..n_t).unwrap_or(&mut []),
                midi: work.midi.get_mut(..n_m).unwrap_or(&mut []),
                spectra: work.spectra.get_mut(..n_sp).unwrap_or(&mut []),
                buffers: work.buffers.get_mut(..n_b).unwrap_or(&mut []),
            },
            host: HostAccess {
                input: shared.host_input,
                in_channels: shared.host_in_channels,
                output: host_output,
                out_channels,
                stride: shared.host_stride,
                offset: shared.host_offset,
                frames,
                midi: shared.host_midi,
                midi_out,
                controls: shared.host_controls,
                osc_out,
            },
            telemetry: tele,
        };
        let ctx = ProcessCtx {
            frames,
            sample_rate: shared.sample_rate,
            transport: shared.transport,
            phase,
            engine: shared.engine,
        };
        let t0 = shared.clock.map(|c| c());
        if step.muted {
            bypass(step.bypass, &mut io);
        } else {
            work.node.process(&ctx, &mut io);
        }
        if let (Some(t0), Some(c), Some(slot)) = (t0, shared.clock, step.tele_time)
            && let Some(a) = telemetry.node_time.get(slot as usize)
        {
            a.store(
                c().saturating_sub(t0).min(u64::from(u32::MAX)) as u32,
                Ordering::Relaxed,
            );
        }
    }
    socket_telemetry(shared, step, work, out_slots);
}

/// Publish per-socket levels, values, and event counts for a step.
fn socket_telemetry(shared: &Shared<'_>, step: &Step, work: &StepWork, out_slots: &[OutputSlot]) {
    let frames = shared.frames;
    let telemetry = shared.telemetry;
    let st = range(shared.socket_tele, step.socket_tele);
    for (slot, tele) in out_slots.iter().zip(st) {
        let i = tele.slot as usize;
        let (level, value, events) = match *slot {
            OutputSlot::Float {
                local,
                sample: true,
            } => {
                let b = work
                    .samples
                    .get(local as usize)
                    .map_or(&[][..], |b| &b[..frames.min(b.len())]);
                (peak(b), b.last().copied().unwrap_or(0.0), None)
            }
            OutputSlot::Float {
                local,
                sample: false,
            } => {
                let v = work.scalars.get(local as usize).copied().unwrap_or(0.0);
                (v.abs(), v, None)
            }
            OutputSlot::Int(k) => {
                let v = work.ints.get(k as usize).copied().unwrap_or(0) as f32;
                (v.abs(), v, None)
            }
            OutputSlot::Audio { start, count } => {
                let mut p = 0.0f32;
                for c in start..start + count {
                    if let Some(b) = work.samples.get(c as usize) {
                        p = p.max(peak(&b[..frames.min(b.len())]));
                    }
                }
                (p, p, None)
            }
            OutputSlot::Trigger(k) => (
                0.0,
                0.0,
                Some(work.triggers.get(k as usize).map_or(0, Vec::len)),
            ),
            OutputSlot::Midi(k) => (
                0.0,
                0.0,
                Some(work.midi.get(k as usize).map_or(0, Vec::len)),
            ),
            OutputSlot::Spectrum(k) => (
                0.0,
                0.0,
                Some(work.spectra.get(k as usize).map_or(0, SpectrumBuf::frames)),
            ),
            OutputSlot::Buffer(k) => {
                // Value: the buffer's length in seconds.
                let secs = work
                    .buffers
                    .get(k as usize)
                    .and_then(Option::as_ref)
                    .map_or(0.0, |b| b.frames() as f32 / (b.sample_rate.max(1) as f32));
                (secs, secs, None)
            }
            OutputSlot::None => continue,
        };
        if let Some(a) = telemetry.socket_level.get(i) {
            let prev = f32::from_bits(a.load(Ordering::Relaxed));
            a.store(
                level.max(prev * shared.level_decay).to_bits(),
                Ordering::Relaxed,
            );
        }
        if let Some(a) = telemetry.socket_value.get(i) {
            a.store(value.to_bits(), Ordering::Relaxed);
        }
        if let (Some(n), Some(a)) = (events, telemetry.socket_events.get(i))
            && n > 0
        {
            a.fetch_add(n as u32, Ordering::Relaxed);
        }
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// A muted node: outputs are already zeroed; copy `in` to `out` where types match.
fn bypass(b: Bypass, io: &mut Io) {
    match b {
        Bypass::None => {}
        Bypass::Audio { input, output } => {
            let src = io.inputs.audio(input as usize);
            let mut out = io.outputs.audio(output as usize);
            for c in 0..out.channels() {
                let s = src.channel(c);
                let d = out.channel(c);
                let n = s.len().min(d.len());
                d[..n].copy_from_slice(&s[..n]);
            }
        }
        Bypass::Float { input, output } => {
            let src = io.inputs.float(input as usize);
            let mut out = io.outputs.float(output as usize);
            out.fill_with(|i| src.at(i));
        }
    }
}
