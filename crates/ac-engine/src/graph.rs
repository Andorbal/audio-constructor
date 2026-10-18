//! The compiled graph: a flat schedule the audio thread runs without
//! traversal, hashing, or allocation (ARCHITECTURE §7).
//!
//! Steps are grouped into chains, and chains into waves (`chains.rs`). A
//! chain's steps run in order on one thread; no chain in a wave reads
//! another's outputs, so a wave's chains can run in any order or at the
//! same time. Each wave runs in three phases:
//!
//! 1. Move out: every step's output buffers and its node's state are swapped
//!    out of the shared pool into the wave's work space (pointer swaps, no
//!    allocation; DECISIONS D23), laid out step after step, so a chain's
//!    outputs are one contiguous run of it.
//! 2. Run: the pool is now only read, so chains run on the audio thread or
//!    spread across the real-time worker pool, each writing only its own
//!    part of the work space. A step reads what earlier steps of its chain
//!    wrote from that part ([`ac_core::LOCAL`] indices) and everything else
//!    from the pool. Steps that write device outputs run last, one at a
//!    time, in schedule order, so outputs sum the same way on any number of
//!    cores.
//! 3. Move back.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{
    AutoFlags, BufferRef, ControlEvent, EngineInfo, HostAccess, InputSlot, Inputs, Io, LOCAL,
    Local, MidiEvent, Node, NodeMeta, NodePath, OscMessage, OutputSlot, Outputs, ParamSlot, Phase,
    PrepareCtx, ProcessCtx, Signature, SpectrumBuf, TelemetryAccess, TempoMap, TransportView,
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
    /// The chain's own sample buffers, in [`Work`]'s stash: the whole
    /// chain's range on its first step, empty on the rest.
    pub o_stash: Range,
    /// Entries of [`CompiledGraph::recycle`] to apply before the step runs
    /// (stash → output slot) and after (slot → stash).
    pub acquire: Range,
    pub release: Range,
    pub tele_values: Range,
    pub tele_scope: Range,
    pub tele_scope_pos: Option<u32>,
    pub tele_time: Option<u32>,
    /// One per output socket (parallel to `outputs`), if socket telemetry is on.
    pub socket_tele: Range,
}

/// Steps `start..end` of the schedule; `start..parallel_end` are the
/// chains `chains` (indices into [`CompiledGraph::chains`]), which may run
/// at the same time; `parallel_end..end` write device outputs and run one
/// by one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Wave {
    pub start: u32,
    pub parallel_end: u32,
    pub end: u32,
    pub chains: Range,
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

/// A position in the schedule's output layout: a step, and where its
/// outputs of each kind start (or, as an end, where the previous step's
/// end). Steps' outputs are laid out in schedule order, so a wave's, a
/// chain's, and a step's outputs of each kind are one contiguous range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct At {
    pub step: usize,
    pub samples: usize,
    pub scalars: usize,
    pub ints: usize,
    pub triggers: usize,
    pub midi: usize,
    pub spectra: usize,
    pub buffers: usize,
    pub stash: usize,
}

impl At {
    /// Where step `k`'s outputs start.
    pub fn start(steps: &[Step], k: usize) -> At {
        match steps.get(k) {
            Some(s) => At {
                step: k,
                samples: s.o_samples.0 as usize,
                scalars: s.o_scalars.0 as usize,
                ints: s.o_ints.0 as usize,
                triggers: s.o_triggers.0 as usize,
                midi: s.o_midi.0 as usize,
                spectra: s.o_spectra.0 as usize,
                buffers: s.o_buffers.0 as usize,
                stash: s.o_stash.0 as usize,
            },
            None => At::end(steps, k),
        }
    }

    /// Where the outputs of the steps before step `k` end.
    pub fn end(steps: &[Step], k: usize) -> At {
        match k.checked_sub(1).and_then(|p| steps.get(p)) {
            Some(s) => At {
                step: k,
                samples: s.o_samples.1 as usize,
                scalars: s.o_scalars.1 as usize,
                ints: s.o_ints.1 as usize,
                triggers: s.o_triggers.1 as usize,
                midi: s.o_midi.1 as usize,
                spectra: s.o_spectra.1 as usize,
                buffers: s.o_buffers.1 as usize,
                stash: s.o_stash.1 as usize,
            },
            None => At::default(),
        }
    }

    /// The larger of each field.
    pub fn max(self, o: At) -> At {
        At {
            step: self.step.max(o.step),
            samples: self.samples.max(o.samples),
            scalars: self.scalars.max(o.scalars),
            ints: self.ints.max(o.ints),
            triggers: self.triggers.max(o.triggers),
            midi: self.midi.max(o.midi),
            spectra: self.spectra.max(o.spectra),
            buffers: self.buffers.max(o.buffers),
            stash: self.stash.max(o.stash),
        }
    }

    /// `self` relative to `base` (which comes no later).
    pub fn minus(self, base: At) -> At {
        At {
            step: self.step.saturating_sub(base.step),
            samples: self.samples.saturating_sub(base.samples),
            scalars: self.scalars.saturating_sub(base.scalars),
            ints: self.ints.saturating_sub(base.ints),
            triggers: self.triggers.saturating_sub(base.triggers),
            midi: self.midi.saturating_sub(base.midi),
            spectra: self.spectra.saturating_sub(base.spectra),
            buffers: self.buffers.saturating_sub(base.buffers),
            stash: self.stash.saturating_sub(base.stash),
        }
    }
}

/// The outputs and node states of a wave's steps while the wave runs, laid
/// out like the wave's part of the schedule (see [`At`]). Sized for the
/// widest wave; between waves it holds empty buffers and vacant nodes.
///
/// A sample buffer that only its own chain reads never goes through the
/// pool: the chain takes one from its part of `stash` just before the step
/// that writes it and puts it back after the last step that reads it, so a
/// long chain gets by with two or three buffers.
pub(crate) struct Work {
    nodes: Vec<Box<dyn Node>>,
    load: Vec<bool>,
    samples: Vec<Vec<f32>>,
    scalars: Vec<f32>,
    ints: Vec<i32>,
    triggers: Vec<Vec<u32>>,
    midi: Vec<Vec<MidiEvent>>,
    spectra: Vec<SpectrumBuf>,
    buffers: Vec<BufferRef>,
    stash: Vec<Vec<f32>>,
}

impl Work {
    /// Room for `size.step` steps with `size.samples` sample outputs, etc.,
    /// and `size.stash` chain buffers of `max_block` samples.
    pub fn sized(size: At, max_block: usize) -> Work {
        Work {
            nodes: (0..size.step)
                .map(|_| Box::new(Vacant) as Box<dyn Node>)
                .collect(),
            load: vec![false; size.step],
            samples: (0..size.samples).map(|_| Vec::new()).collect(),
            scalars: vec![0.0; size.scalars],
            ints: vec![0; size.ints],
            triggers: (0..size.triggers).map(|_| Vec::new()).collect(),
            midi: (0..size.midi).map(|_| Vec::new()).collect(),
            spectra: (0..size.spectra).map(|_| SpectrumBuf::default()).collect(),
            buffers: vec![None; size.buffers],
            stash: (0..size.stash).map(|_| vec![0.0; max_block]).collect(),
        }
    }

    fn view(&mut self) -> WorkMut<'_> {
        WorkMut {
            nodes: &mut self.nodes,
            load: &mut self.load,
            samples: &mut self.samples,
            scalars: &mut self.scalars,
            ints: &mut self.ints,
            triggers: &mut self.triggers,
            midi: &mut self.midi,
            spectra: &mut self.spectra,
            buffers: &mut self.buffers,
            stash: &mut self.stash,
        }
    }
}

/// Part of the work space: some consecutive steps' outputs and nodes.
struct WorkMut<'w> {
    nodes: &'w mut [Box<dyn Node>],
    load: &'w mut [bool],
    samples: &'w mut [Vec<f32>],
    scalars: &'w mut [f32],
    ints: &'w mut [i32],
    triggers: &'w mut [Vec<u32>],
    midi: &'w mut [Vec<MidiEvent>],
    spectra: &'w mut [SpectrumBuf],
    buffers: &'w mut [BufferRef],
    stash: &'w mut [Vec<f32>],
}

/// `split_at_mut` that clamps instead of panicking.
fn split<T>(v: &mut [T], at: usize) -> (&mut [T], &mut [T]) {
    let at = at.min(v.len());
    v.split_at_mut(at)
}

impl<'w> WorkMut<'w> {
    /// Everything before `at` (relative to this view) and everything from it.
    fn split_at(self, at: At) -> (WorkMut<'w>, WorkMut<'w>) {
        let (n0, n1) = split(self.nodes, at.step);
        let (l0, l1) = split(self.load, at.step);
        let (s0, s1) = split(self.samples, at.samples);
        let (c0, c1) = split(self.scalars, at.scalars);
        let (i0, i1) = split(self.ints, at.ints);
        let (t0, t1) = split(self.triggers, at.triggers);
        let (m0, m1) = split(self.midi, at.midi);
        let (p0, p1) = split(self.spectra, at.spectra);
        let (b0, b1) = split(self.buffers, at.buffers);
        let (x0, x1) = split(self.stash, at.stash);
        (
            WorkMut {
                nodes: n0,
                load: l0,
                samples: s0,
                scalars: c0,
                ints: i0,
                triggers: t0,
                midi: m0,
                spectra: p0,
                buffers: b0,
                stash: x0,
            },
            WorkMut {
                nodes: n1,
                load: l1,
                samples: s1,
                scalars: c1,
                ints: i1,
                triggers: t1,
                midi: m1,
                spectra: p1,
                buffers: b1,
                stash: x1,
            },
        )
    }

    fn reborrow(&mut self) -> WorkMut<'_> {
        WorkMut {
            nodes: self.nodes,
            load: self.load,
            samples: self.samples,
            scalars: self.scalars,
            ints: self.ints,
            triggers: self.triggers,
            midi: self.midi,
            spectra: self.spectra,
            buffers: self.buffers,
            stash: self.stash,
        }
    }

    /// Swap chain buffers between the stash and output slots, by
    /// `(stash, slot)` pairs relative to `at`; zero what lands in a slot
    /// when `clear` (a step's outputs start silent).
    fn recycle(&mut self, moves: &[(u32, u32)], at: At, frames: usize, clear: bool) {
        for &(j, q) in moves {
            if let (Some(s), Some(b)) = (
                self.stash.get_mut(at.stash + j as usize),
                self.samples.get_mut(at.samples + q as usize),
            ) {
                std::mem::swap(s, b);
                if clear {
                    let n = frames.min(b.len());
                    b[..n].fill(0.0);
                }
            }
        }
    }

    /// Read access to what this part holds from `from` on, as chain values.
    fn local(self, from: At) -> Local<'w> {
        fn tail<T>(v: &[T], from: usize) -> &[T] {
            v.get(from..).unwrap_or(&[])
        }
        let WorkMut {
            samples,
            scalars,
            ints,
            triggers,
            midi,
            spectra,
            buffers,
            ..
        } = self;
        Local {
            scalars: tail(scalars, from.scalars),
            ints: tail(ints, from.ints),
            samples: tail(samples, from.samples),
            triggers: tail(triggers, from.triggers),
            midi: tail(midi, from.midi),
            spectra: tail(spectra, from.spectra),
            buffers: tail(buffers, from.buffers),
        }
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
    /// Step ranges of the chains that run in parallel, wave by wave.
    pub(crate) chains: Vec<Range>,
    /// Chain buffer moves, `(stash, sample slot)` relative to the chain's
    /// first step (see [`Step::acquire`]).
    pub(crate) recycle: Vec<(u32, u32)>,
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
    /// Room for the outputs and nodes of the widest wave.
    pub(crate) work: Work,
    pub(crate) level_decay: f32,
    /// Output latency in samples.
    pub(crate) latency: u32,
    /// The project's bindings, resolved to this graph's param slots.
    pub(crate) bindings: crate::bindings::BindingTable,
}

/// What every step of a block shares (read-only, so it can be shared
/// across threads).
struct Shared<'a> {
    steps: &'a [Step],
    recycle: &'a [(u32, u32)],
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

/// Consecutive chains for one thread: `work` holds their outputs, starting
/// at schedule position `base`.
struct Chunk<'a, 'b> {
    shared: &'b Shared<'a>,
    chains: &'b [Range],
    work: WorkMut<'b>,
    base: At,
}

impl Job for Chunk<'_, '_> {
    fn run(&mut self) {
        for &chain in self.chains {
            run_chain(self.shared, chain, self.work.reborrow(), self.base);
        }
    }
}

/// Upper bound on jobs per wave; more chains share jobs.
const MAX_JOBS: usize = 64;

/// Run a chain's steps in order. `work` holds the chain's outputs (and
/// maybe more), starting at schedule position `base`.
fn run_chain(shared: &Shared<'_>, chain: Range, mut work: WorkMut<'_>, base: At) {
    let steps = shared.steps;
    let (k0, k1) = (chain.0 as usize, chain.1 as usize);
    let first = At::start(steps, k0).minus(base);
    for k in k0..k1 {
        let Some(step) = steps.get(k) else {
            return;
        };
        let frames = shared.frames;
        work.recycle(range(shared.recycle, step.acquire), first, frames, true);
        let start = At::start(steps, k).minus(base);
        let size = At::end(steps, k + 1).minus(At::start(steps, k));
        let (before, rest) = work.reborrow().split_at(start);
        let (mine, _) = rest.split_at(size);
        run_step(shared, step, mine, before.local(first), &mut [], None, None);
        work.recycle(range(shared.recycle, step.release), first, frames, false);
    }
}

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
                    p.remaining -= n;
                    if p.remaining == 0 {
                        p.value = p.target;
                        p.step = 0.0;
                    } else {
                        // Counted back from the target, not accumulated,
                        // so any split of the ramp into blocks lands on
                        // the same values (FloatIn::Ramp).
                        p.value = p.target - p.step * p.remaining as f32;
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
        let auto = AutoFlags {
            play: block.started,
            stop: block.stopped,
            load: false,
        };
        for wi in 0..self.waves.len() {
            let wave = self.waves[wi];
            let (start, mid, end) = (
                wave.start as usize,
                wave.parallel_end as usize,
                wave.end as usize,
            );
            if start >= end || end > self.steps.len() {
                continue;
            }
            self.move_out(start, end, frames);
            {
                let transport = TransportView {
                    playing: block.playing,
                    recording: block.recording,
                    position: block.position,
                    tempo: &self.tempo,
                    started: block.started,
                    stopped: block.stopped,
                };
                let shared = Shared {
                    steps: &self.steps,
                    recycle: &self.recycle,
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
                let steps = shared.steps;
                let base = At::start(steps, start);
                let serial_base = At::start(steps, mid);
                let chains = self
                    .chains
                    .get(wave.chains.0 as usize..wave.chains.1 as usize)
                    .unwrap_or(&[]);
                let (mut par_work, mut serial_work) =
                    self.work.view().split_at(serial_base.minus(base));
                match workers {
                    Some(rt) if chains.len() >= 2 && rt.workers() > 0 => {
                        run_parallel(rt, &shared, chains, par_work, base);
                    }
                    _ => {
                        for &chain in chains {
                            run_chain(&shared, chain, par_work.reborrow(), base);
                        }
                    }
                }
                for k in mid..end {
                    let Some(step) = steps.get(k) else {
                        break;
                    };
                    let at = At::start(steps, k).minus(serial_base);
                    let size = At::end(steps, k + 1).minus(At::start(steps, k));
                    let (_, rest) = serial_work.reborrow().split_at(at);
                    let (mine, _) = rest.split_at(size);
                    run_step(
                        &shared,
                        step,
                        mine,
                        Local::default(),
                        host.output,
                        host.midi_out.as_deref_mut(),
                        host.osc_out.as_deref_mut(),
                    );
                }
            }
            self.move_back(start, end);
        }
    }

    /// Swap the outputs and nodes of steps `start..end` (one wave) out of
    /// the pool into the work space, cleared for the block.
    fn move_out(&mut self, start: usize, end: usize, frames: usize) {
        let (a, e) = (At::start(&self.steps, start), At::end(&self.steps, end));
        let work = &mut self.work;
        let pool = &mut self.pool;
        let steps = self.steps.get(start..end).unwrap_or(&[]);
        for (k, step) in steps.iter().enumerate() {
            if let StepKind::Node { node, .. } = step.kind
                && let (Some(nb), Some(slot), Some(load)) = (
                    self.nodes.get_mut(node as usize),
                    work.nodes.get_mut(k),
                    work.load.get_mut(k),
                )
            {
                std::mem::swap(&mut nb.node, slot);
                *load = nb.load_pending;
            }
        }
        for (&g, b) in span(&self.out_samples, a.samples, e.samples).zip(&mut work.samples) {
            if let Some(p) = pool.samples.get_mut(g as usize) {
                std::mem::swap(p, b);
                let n = frames.min(b.len());
                b[..n].fill(0.0);
            }
        }
        for (&g, b) in span(&self.out_triggers, a.triggers, e.triggers).zip(&mut work.triggers) {
            if let Some(p) = pool.triggers.get_mut(g as usize) {
                std::mem::swap(p, b);
                b.clear();
            }
        }
        for (&g, b) in span(&self.out_midi, a.midi, e.midi).zip(&mut work.midi) {
            if let Some(p) = pool.midi.get_mut(g as usize) {
                std::mem::swap(p, b);
                b.clear();
            }
        }
        for (&g, b) in span(&self.out_spectra, a.spectra, e.spectra).zip(&mut work.spectra) {
            if let Some(p) = pool.spectra.get_mut(g as usize) {
                std::mem::swap(p, b);
                b.clear();
            }
        }
        for (&g, b) in span(&self.out_buffers, a.buffers, e.buffers).zip(&mut work.buffers) {
            if let Some(p) = pool.buffers.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, s) in span(&self.out_scalars, a.scalars, e.scalars).zip(&mut work.scalars) {
            *s = pool.scalars.get(g as usize).copied().unwrap_or(0.0);
        }
        for (&g, s) in span(&self.out_ints, a.ints, e.ints).zip(&mut work.ints) {
            *s = pool.ints.get(g as usize).copied().unwrap_or(0);
        }
    }

    /// Undo [`Self::move_out`].
    fn move_back(&mut self, start: usize, end: usize) {
        let (a, e) = (At::start(&self.steps, start), At::end(&self.steps, end));
        let work = &mut self.work;
        let pool = &mut self.pool;
        let steps = self.steps.get(start..end).unwrap_or(&[]);
        for (k, step) in steps.iter().enumerate() {
            if let StepKind::Node { node, phase } = step.kind
                && let (Some(nb), Some(slot)) =
                    (self.nodes.get_mut(node as usize), work.nodes.get_mut(k))
            {
                std::mem::swap(&mut nb.node, slot);
                if phase != Phase::FeedbackRead {
                    nb.load_pending = false;
                }
            }
        }
        for (&g, b) in span(&self.out_samples, a.samples, e.samples).zip(&mut work.samples) {
            if let Some(p) = pool.samples.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, b) in span(&self.out_triggers, a.triggers, e.triggers).zip(&mut work.triggers) {
            if let Some(p) = pool.triggers.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, b) in span(&self.out_midi, a.midi, e.midi).zip(&mut work.midi) {
            if let Some(p) = pool.midi.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, b) in span(&self.out_spectra, a.spectra, e.spectra).zip(&mut work.spectra) {
            if let Some(p) = pool.spectra.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, b) in span(&self.out_buffers, a.buffers, e.buffers).zip(&mut work.buffers) {
            if let Some(p) = pool.buffers.get_mut(g as usize) {
                std::mem::swap(p, b);
            }
        }
        for (&g, s) in span(&self.out_scalars, a.scalars, e.scalars).zip(&work.scalars) {
            if let Some(d) = pool.scalars.get_mut(g as usize) {
                *d = *s;
            }
        }
        for (&g, s) in span(&self.out_ints, a.ints, e.ints).zip(&work.ints) {
            if let Some(d) = pool.ints.get_mut(g as usize) {
                *d = *s;
            }
        }
    }
}

/// `v[a..e]`, or nothing if that's out of range.
fn span(v: &[u32], a: usize, e: usize) -> std::slice::Iter<'_, u32> {
    v.get(a..e).unwrap_or(&[]).iter()
}

/// Split a wave's chains into jobs of about equal length and run them on
/// the pool. `work` holds the chains' outputs from schedule position `base`.
fn run_parallel(rt: &RtPool, shared: &Shared<'_>, chains: &[Range], work: WorkMut<'_>, base: At) {
    let threads = rt.workers() + 1;
    // A few jobs per thread so a slow chain doesn't hold up the wave.
    let jobs = (threads * 4).min(chains.len()).clamp(1, MAX_JOBS);
    let total: usize = chains.iter().map(|c| (c.1 - c.0) as usize).sum();
    let target = total.div_ceil(jobs).max(1);
    let mut chunks: [Option<Chunk<'_, '_>>; MAX_JOBS] = std::array::from_fn(|_| None);
    let (mut rest, mut at) = (work, base);
    let (mut n, mut i) = (0, 0);
    while i < chains.len() && n < MAX_JOBS {
        let first = i;
        let mut steps = 0;
        while i < chains.len() && (i == first || steps < target || n == MAX_JOBS - 1) {
            steps += (chains[i].1 - chains[i].0) as usize;
            i += 1;
        }
        let last = chains.get(i.saturating_sub(1)).map_or(0, |c| c.1 as usize);
        let end = At::end(shared.steps, last);
        let (mine, after) = rest.split_at(end.minus(at));
        if let Some(slot) = chunks.get_mut(n) {
            *slot = Some(Chunk {
                shared,
                chains: chains.get(first..i).unwrap_or(&[]),
                work: mine,
                base: at,
            });
        }
        (rest, at) = (after, end);
        n += 1;
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

/// Run one step whose outputs and node are in `work` (exactly this step's
/// part of the work space). `local` holds what earlier steps of its chain
/// wrote. `host_output`, `midi_out`, and `osc_out` are the device outputs
/// for serial steps, and empty for the rest.
fn run_step(
    shared: &Shared<'_>,
    step: &Step,
    work: WorkMut<'_>,
    local: Local<'_>,
    host_output: &mut [f32],
    midi_out: Option<&mut Vec<MidiEvent>>,
    osc_out: Option<&mut Vec<OscMessage>>,
) {
    let frames = shared.frames;
    let phase = match step.kind {
        StepKind::FillFromScalar { scalar } => {
            let v = if scalar & LOCAL == 0 {
                shared.pool.scalars.get(scalar as usize)
            } else {
                local.scalars.get((scalar & !LOCAL) as usize)
            };
            let v = v.copied().unwrap_or(0.0);
            if let Some(buf) = work.samples.first_mut() {
                let n = frames.min(buf.len());
                buf[..n].fill(v);
            }
            return;
        }
        StepKind::Node { phase, .. } => phase,
    };
    let Some(node) = work.nodes.first_mut() else {
        return;
    };
    let mut auto = shared.auto;
    auto.load = work.load.first().copied().unwrap_or(false);
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
                local,
            },
            outputs: Outputs {
                slots: out_slots,
                frames,
                samples: &mut *work.samples,
                scalars: &mut *work.scalars,
                ints: &mut *work.ints,
                triggers: &mut *work.triggers,
                midi: &mut *work.midi,
                spectra: &mut *work.spectra,
                buffers: &mut *work.buffers,
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
            node.process(&ctx, &mut io);
        }
        // Summed over the node's steps and sub-blocks, published when the
        // host block ends (`TelemetryStore::publish_node_times`).
        if let (Some(t0), Some(c), Some(slot)) = (t0, shared.clock, step.tele_time)
            && let Some(a) = telemetry.node_time_acc.get(slot as usize)
        {
            a.fetch_add(
                c().saturating_sub(t0).min(u64::from(u32::MAX)) as u32,
                Ordering::Relaxed,
            );
        }
    }
    socket_telemetry(shared, step, &work, out_slots);
}

/// Publish per-socket levels, values, and event counts for a step.
fn socket_telemetry(
    shared: &Shared<'_>,
    step: &Step,
    work: &WorkMut<'_>,
    out_slots: &[OutputSlot],
) {
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
