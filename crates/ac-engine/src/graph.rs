//! The compiled graph: a flat schedule the audio thread runs without
//! traversal, hashing, or allocation (ARCHITECTURE §7).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{
    AutoFlags, HostAccess, InputSlot, Inputs, Io, MidiEvent, Node, NodeMeta, NodePath, OutputSlot,
    Outputs, ParamSlot, Phase, PrepareCtx, ProcessCtx, Signature, TelemetryAccess, TempoMap,
    TransportView,
};

use crate::telemetry::TelemetryStore;

/// A zero-sized stand-in for a node whose state will be carried over from the
/// previous graph at install time. `Box<Vacant>` doesn't allocate.
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
    /// Float → Audio from a block-rate value: fill a buffer with it.
    FillFromScalar { scalar: u32, dst: u32 },
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
    pub inputs: Range,
    pub outputs: Range,
    pub o_samples: Range,
    pub o_scalars: Range,
    pub o_ints: Range,
    pub o_triggers: Range,
    pub o_midi: Range,
    pub tele_values: Range,
    pub tele_scope: Range,
    pub tele_scope_pos: Option<u32>,
    pub tele_time: Option<u32>,
    /// One per output socket (parallel to `outputs`), if socket telemetry is on.
    pub socket_tele: Range,
}

/// Preallocated swap space for one step's outputs.
#[derive(Default)]
pub(crate) struct Scratch {
    pub samples: Vec<Vec<f32>>,
    pub scalars: Vec<f32>,
    pub ints: Vec<i32>,
    pub triggers: Vec<Vec<u32>>,
    pub midi: Vec<Vec<MidiEvent>>,
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
    pub(crate) input_slots: Vec<InputSlot>,
    pub(crate) output_slots: Vec<OutputSlot>,
    pub(crate) out_samples: Vec<u32>,
    pub(crate) out_scalars: Vec<u32>,
    pub(crate) out_ints: Vec<u32>,
    pub(crate) out_triggers: Vec<u32>,
    pub(crate) out_midi: Vec<u32>,
    pub(crate) chan_table: Vec<u32>,
    pub(crate) samples: Vec<Vec<f32>>,
    pub(crate) scalars: Vec<f32>,
    pub(crate) ints: Vec<i32>,
    pub(crate) triggers: Vec<Vec<u32>>,
    pub(crate) midi: Vec<Vec<MidiEvent>>,
    pub(crate) params: Vec<ParamSlot>,
    /// Smoothing length in samples per param (0 = no smoothing).
    pub(crate) param_smooth: Vec<u32>,
    /// Params currently ramping or fired (indices); capacity = params.len().
    pub(crate) active_params: Vec<u32>,
    pub(crate) telemetry: Arc<TelemetryStore>,
    pub(crate) socket_tele: Vec<SocketTele>,
    pub(crate) scratch: Scratch,
    pub(crate) level_decay: f32,
}

impl CompiledGraph {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Paths of the nodes in this graph, in creation order.
    pub fn node_paths(&self) -> impl Iterator<Item = &NodePath> {
        self.nodes.iter().map(|n| &n.path)
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

    /// Run every step for one block.
    pub(crate) fn run(
        &mut self,
        frames: usize,
        block: BlockTransport,
        host: &mut HostAccess<'_>,
        clock: Option<fn() -> u64>,
    ) {
        let frames = frames.min(self.max_block).max(1);
        let CompiledGraph {
            nodes,
            steps,
            input_slots,
            output_slots,
            out_samples,
            out_scalars,
            out_ints,
            out_triggers,
            out_midi,
            chan_table,
            samples,
            scalars,
            ints,
            triggers,
            midi,
            params,
            telemetry,
            socket_tele,
            scratch,
            sample_rate,
            level_decay,
            tempo,
            ..
        } = self;
        let transport = TransportView {
            playing: block.playing,
            recording: block.recording,
            position: block.position,
            tempo,
            started: block.started,
            stopped: block.stopped,
        };
        let auto_base = AutoFlags {
            play: transport.started,
            stop: transport.stopped,
            load: false,
        };
        for step in steps.iter() {
            match step.kind {
                StepKind::FillFromScalar { scalar, dst } => {
                    let v = scalars.get(scalar as usize).copied().unwrap_or(0.0);
                    if let Some(buf) = samples.get_mut(dst as usize) {
                        let n = frames.min(buf.len());
                        buf[..n].fill(v);
                    }
                }
                StepKind::Node { node, phase } => {
                    let Some(nb) = nodes.get_mut(node as usize) else {
                        continue;
                    };
                    // Move this step's output buffers out of the pool (D23).
                    let os = range(out_samples, step.o_samples);
                    let ot = range(out_triggers, step.o_triggers);
                    let om = range(out_midi, step.o_midi);
                    let osc = range(out_scalars, step.o_scalars);
                    let oi = range(out_ints, step.o_ints);
                    for (k, &g) in os.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (samples.get_mut(g as usize), scratch.samples.get_mut(k))
                        {
                            std::mem::swap(a, b);
                            let n = frames.min(b.len());
                            b[..n].fill(0.0);
                        }
                    }
                    for (k, &g) in ot.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (triggers.get_mut(g as usize), scratch.triggers.get_mut(k))
                        {
                            std::mem::swap(a, b);
                            b.clear();
                        }
                    }
                    for (k, &g) in om.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (midi.get_mut(g as usize), scratch.midi.get_mut(k))
                        {
                            std::mem::swap(a, b);
                            b.clear();
                        }
                    }
                    for (k, &g) in osc.iter().enumerate() {
                        if let Some(s) = scratch.scalars.get_mut(k) {
                            *s = scalars.get(g as usize).copied().unwrap_or(0.0);
                        }
                    }
                    for (k, &g) in oi.iter().enumerate() {
                        if let Some(s) = scratch.ints.get_mut(k) {
                            *s = ints.get(g as usize).copied().unwrap_or(0);
                        }
                    }

                    let mut auto = auto_base;
                    auto.load = nb.load_pending;
                    let in_slots = range(input_slots, step.inputs);
                    let out_slots = range(output_slots, step.outputs);
                    let (vs, ve) = (step.tele_values.0 as usize, step.tele_values.1 as usize);
                    let (ss, se) = (step.tele_scope.0 as usize, step.tele_scope.1 as usize);
                    let tele = TelemetryAccess {
                        values: telemetry.node_values.get(vs..ve).unwrap_or(&[]),
                        scope: telemetry.scope.get(ss..se).unwrap_or(&[]),
                        scope_pos: step
                            .tele_scope_pos
                            .and_then(|p| telemetry.scope_pos.get(p as usize)),
                    };
                    let n_s = os.len();
                    let n_t = ot.len();
                    let n_m = om.len();
                    let n_sc = osc.len();
                    let n_i = oi.len();
                    {
                        let mut io = Io {
                            inputs: Inputs {
                                slots: in_slots,
                                frames,
                                params,
                                scalars,
                                ints,
                                samples,
                                chan_table,
                                triggers,
                                midi,
                                auto,
                            },
                            outputs: Outputs {
                                slots: out_slots,
                                frames,
                                samples: scratch.samples.get_mut(..n_s).unwrap_or(&mut []),
                                scalars: scratch.scalars.get_mut(..n_sc).unwrap_or(&mut []),
                                ints: scratch.ints.get_mut(..n_i).unwrap_or(&mut []),
                                triggers: scratch.triggers.get_mut(..n_t).unwrap_or(&mut []),
                                midi: scratch.midi.get_mut(..n_m).unwrap_or(&mut []),
                            },
                            host: HostAccess {
                                input: host.input,
                                in_channels: host.in_channels,
                                output: &mut *host.output,
                                out_channels: host.out_channels,
                                stride: host.stride,
                                offset: host.offset,
                                frames,
                                midi: host.midi,
                            },
                            telemetry: tele,
                        };
                        let ctx = ProcessCtx {
                            frames,
                            sample_rate: *sample_rate,
                            transport,
                            phase,
                        };
                        let t0 = clock.map(|c| c());
                        if step.muted {
                            bypass(step.bypass, &mut io);
                        } else {
                            nb.node.process(&ctx, &mut io);
                        }
                        if let (Some(t0), Some(c), Some(slot)) = (t0, clock, step.tele_time)
                            && let Some(a) = telemetry.node_time.get(slot as usize)
                        {
                            a.store(
                                c().saturating_sub(t0).min(u64::from(u32::MAX)) as u32,
                                Ordering::Relaxed,
                            );
                        }
                    }
                    if phase != Phase::FeedbackRead {
                        nb.load_pending = false;
                    }

                    // Socket telemetry, then move buffers back.
                    let st = range(socket_tele, step.socket_tele);
                    for (o, (slot, tele)) in out_slots.iter().zip(st).enumerate() {
                        let _ = o;
                        let i = tele.slot as usize;
                        let (level, value, events) = match *slot {
                            OutputSlot::Float {
                                local,
                                sample: true,
                            } => {
                                let b = scratch
                                    .samples
                                    .get(local as usize)
                                    .map_or(&[][..], |b| &b[..frames.min(b.len())]);
                                (peak(b), b.last().copied().unwrap_or(0.0), None)
                            }
                            OutputSlot::Float {
                                local,
                                sample: false,
                            } => {
                                let v = scratch.scalars.get(local as usize).copied().unwrap_or(0.0);
                                (v.abs(), v, None)
                            }
                            OutputSlot::Int(k) => {
                                let v = scratch.ints.get(k as usize).copied().unwrap_or(0) as f32;
                                (v.abs(), v, None)
                            }
                            OutputSlot::Audio { start, count } => {
                                let mut p = 0.0f32;
                                for c in start..start + count {
                                    if let Some(b) = scratch.samples.get(c as usize) {
                                        p = p.max(peak(&b[..frames.min(b.len())]));
                                    }
                                }
                                (p, p, None)
                            }
                            OutputSlot::Trigger(k) => (
                                0.0,
                                0.0,
                                Some(scratch.triggers.get(k as usize).map_or(0, Vec::len)),
                            ),
                            OutputSlot::Midi(k) => (
                                0.0,
                                0.0,
                                Some(scratch.midi.get(k as usize).map_or(0, Vec::len)),
                            ),
                            OutputSlot::None => continue,
                        };
                        if let Some(a) = telemetry.socket_level.get(i) {
                            let prev = f32::from_bits(a.load(Ordering::Relaxed));
                            a.store(level.max(prev * *level_decay).to_bits(), Ordering::Relaxed);
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
                    for (k, &g) in os.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (samples.get_mut(g as usize), scratch.samples.get_mut(k))
                        {
                            std::mem::swap(a, b);
                        }
                    }
                    for (k, &g) in ot.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (triggers.get_mut(g as usize), scratch.triggers.get_mut(k))
                        {
                            std::mem::swap(a, b);
                        }
                    }
                    for (k, &g) in om.iter().enumerate() {
                        if let (Some(a), Some(b)) =
                            (midi.get_mut(g as usize), scratch.midi.get_mut(k))
                        {
                            std::mem::swap(a, b);
                        }
                    }
                    for (k, &g) in osc.iter().enumerate() {
                        if let (Some(d), Some(s)) =
                            (scalars.get_mut(g as usize), scratch.scalars.get(k))
                        {
                            *d = *s;
                        }
                    }
                    for (k, &g) in oi.iter().enumerate() {
                        if let (Some(d), Some(s)) = (ints.get_mut(g as usize), scratch.ints.get(k))
                        {
                            *d = *s;
                        }
                    }
                }
            }
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
