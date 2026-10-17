//! Compilation: project → flat schedule (ARCHITECTURE §7).
//!
//! 1. Flatten groups and remove reroutes (`flatten.rs`).
//! 2. Validate wires: types, one wire per input.
//! 3. Topologically sort, cutting cycles at `Feedback` nodes (split into a
//!    read step that runs early and a write step that runs after its input).
//! 4. Infer output rates and channel counts in schedule order.
//! 5. Build steps with virtual buffers, inserting implicit conversions.
//! 6. Assign physical buffers by lifetime so they're reused.
//! 7. Create nodes, or plan to carry state over from the previous graph.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use ac_core::{
    Channels, Conversion, FloatSrc, InputSlot, IntSrc, MidiEvent, NodePath, OutRate, OutputSlot,
    ParamSlot, Phase, PrepareCtx, Project, Rate, Registry, Resources, SignalType, SocketSpec,
    TempoMap, TriggerSrc, conversion, param_value, util::derive_seed,
};

use crate::flatten::{Flat, flatten};
use crate::graph::{
    Bypass, CompiledGraph, NodeBox, Range, Scratch, SocketTele, Step, StepKind, Vacant,
};
use crate::report::{CompileReport, Severity};
use crate::telemetry::{NodeTelemetry, TelemetryLayout, TelemetryStore, atomics};

/// Smoothing time for parameter changes ("a few ms", ARCHITECTURE §8).
const SMOOTH_SECONDS: f64 = 0.005;
/// Event capacity per MIDI buffer per block.
const MIDI_CAPACITY: usize = 1024;
/// Upper bound on channels per wire, to keep a bad config from allocating wildly.
const MAX_CHANNELS: usize = 64;

/// Settings that shape a compile.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub sample_rate: f64,
    pub max_block: usize,
    /// Publish per-socket levels/values/event counts (the editor wants them;
    /// offline renders don't).
    pub socket_telemetry: bool,
    pub generation: u64,
}

/// Identity of a node in a compiled graph, for state carry-over.
#[derive(Clone, Debug, PartialEq)]
struct CarryKey {
    kind: String,
    config: String,
    index: u32,
}

/// What the previous compile produced, so the next one can carry state over
/// for nodes whose path, kind, and config are unchanged (DECISIONS D11).
#[derive(Clone, Debug, Default)]
pub struct CarryLayout {
    nodes: HashMap<NodePath, CarryKey>,
    sample_rate: f64,
    max_block: usize,
}

/// How a runtime parameter value maps to param slots.
#[derive(Clone, Debug)]
pub struct ParamTarget {
    pub slot: u32,
    pub spec: SocketSpec,
}

/// (node path, socket) → param slots, including group-instance inputs.
pub type ParamMap = HashMap<(NodePath, String), Vec<ParamTarget>>;

pub struct Compiled {
    pub graph: Box<CompiledGraph>,
    pub layout: Arc<TelemetryLayout>,
    pub telemetry: Arc<TelemetryStore>,
    pub params: ParamMap,
    pub carry: CarryLayout,
    pub report: CompileReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutStore {
    None,
    Audio(usize),
    FloatSample,
    FloatBlock,
    Int,
    Trigger,
    Midi,
}

/// Storage assigned to an output: virtual sample buffers, a scalar, etc.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Alloc {
    None,
    Audio(Vec<u32>),
    Sample(u32),
    Scalar(u32),
    Int(u32),
    Trigger(u32),
    Midi(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Item {
    Main(usize),
    FbRead(usize),
    FbWrite(usize),
}

impl Item {
    fn node(self) -> usize {
        match self {
            Item::Main(n) | Item::FbRead(n) | Item::FbWrite(n) => n,
        }
    }
}

/// Lifetime of a virtual buffer: defined at step `def`, last read at `last`.
#[derive(Clone, Copy, Debug)]
struct Life {
    def: usize,
    last: usize,
}

struct Builder {
    steps: Vec<Step>,
    input_slots: Vec<InputSlot>,
    output_slots: Vec<OutputSlot>,
    out_samples: Vec<u32>,
    out_scalars: Vec<u32>,
    out_ints: Vec<u32>,
    out_triggers: Vec<u32>,
    out_midi: Vec<u32>,
    chan_table: Vec<u32>,
    /// Virtual sample buffers; index 0 is permanent silence.
    vsamples: Vec<Life>,
    vtriggers: Vec<Life>,
    vmidi: Vec<Life>,
    n_scalars: u32,
    n_ints: u32,
    params: Vec<ParamSlot>,
    param_smooth: Vec<u32>,
    param_map: ParamMap,
    /// Fill steps reference virtual sample buffers; remember which to remap.
    fill_steps: Vec<usize>,
}

impl Builder {
    fn new_vsample(&mut self, def: usize) -> u32 {
        self.vsamples.push(Life { def, last: def });
        (self.vsamples.len() - 1) as u32
    }
    fn new_vtrigger(&mut self, def: usize) -> u32 {
        self.vtriggers.push(Life { def, last: def });
        (self.vtriggers.len() - 1) as u32
    }
    fn new_vmidi(&mut self, def: usize) -> u32 {
        self.vmidi.push(Life { def, last: def });
        (self.vmidi.len() - 1) as u32
    }
    fn use_sample(&mut self, v: u32, at: usize) {
        if v != 0
            && let Some(l) = self.vsamples.get_mut(v as usize)
        {
            l.last = l.last.max(at);
        }
    }
}

/// Compile a project. Never fails: problems are reported and the offending
/// parts left out, so the rest of the graph still runs.
pub fn compile(
    project: &Project,
    registry: &Registry,
    resources: &dyn Resources,
    opts: &CompileOptions,
    previous: Option<&CarryLayout>,
) -> Compiled {
    let mut report = CompileReport::default();
    let sample_rate = if opts.sample_rate.is_finite() && opts.sample_rate > 0.0 {
        opts.sample_rate
    } else {
        report.warning(None, None, "invalid sample rate; using 48000");
        48_000.0
    };
    let max_block = opts.max_block.clamp(1, 16_384);

    let flat = flatten(project, registry, &mut report);
    let n = flat.nodes.len();

    // --- 2. wires: one per input, compatible types ---
    let mut src_of: Vec<Vec<Option<(usize, usize)>>> = flat
        .nodes
        .iter()
        .map(|f| vec![None; f.sig.inputs.len()])
        .collect();
    for w in &flat.wires {
        let (sn, so) = w.from;
        let (dn, di) = w.to;
        let (Some(out), Some(inp)) = (
            flat.nodes[sn].sig.outputs.get(so),
            flat.nodes[dn].sig.inputs.get(di),
        ) else {
            continue;
        };
        if src_of[dn][di].is_some() {
            report.error(
                Some(&flat.nodes[dn].path),
                Some(&inp.name),
                "input has more than one wire; keeping the first",
            );
            continue;
        }
        if conversion(out.ty, inp.ty).is_none() {
            report.error(
                Some(&flat.nodes[dn].path),
                Some(&inp.name),
                format!(
                    "can't connect {} `{}` of {} to {} input `{}`",
                    out.ty, out.name, flat.nodes[sn].name, inp.ty, inp.name
                ),
            );
            continue;
        }
        src_of[dn][di] = Some(w.from);
    }

    // --- 3. schedule ---
    let order = schedule(&flat, &mut src_of, &mut report);

    // --- 4. rates and channels ---
    let mut store: Vec<Vec<OutStore>> = flat
        .nodes
        .iter()
        .map(|f| vec![OutStore::None; f.sig.outputs.len()])
        .collect();
    for &item in &order {
        let node = item.node();
        if matches!(item, Item::FbWrite(_)) {
            continue;
        }
        let f = &flat.nodes[node];
        let feedback = matches!(item, Item::FbRead(_));
        let mut poly = Rate::Block;
        let mut follow = 1usize;
        if !feedback {
            for (i, spec) in f.sig.inputs.iter().enumerate() {
                let src = src_of[node][i].map(|(s, o)| store[s][o]);
                match spec.ty {
                    SignalType::Float | SignalType::Bool => {
                        let r = match src {
                            Some(OutStore::FloatSample) | Some(OutStore::Audio(_)) => Rate::Sample,
                            Some(OutStore::FloatBlock) | Some(OutStore::Int) => Rate::Block,
                            _ => Rate::Const,
                        };
                        poly = poly.max(r);
                    }
                    SignalType::Audio => {
                        let ch = match src {
                            Some(OutStore::Audio(c)) => c,
                            _ => 1,
                        };
                        follow = follow.max(input_channels(spec.channels, ch));
                    }
                    _ => {}
                }
            }
        }
        for (o, spec) in f.sig.outputs.iter().enumerate() {
            store[node][o] = match spec.ty {
                SignalType::Audio => OutStore::Audio(match spec.channels {
                    Channels::Fixed(c) => (c as usize).clamp(1, MAX_CHANNELS),
                    _ => follow.clamp(1, MAX_CHANNELS),
                }),
                SignalType::Float | SignalType::Bool => {
                    let r = match spec.rate {
                        OutRate::Fixed(r) => r,
                        OutRate::Poly => poly,
                    };
                    if r == Rate::Sample {
                        OutStore::FloatSample
                    } else {
                        OutStore::FloatBlock
                    }
                }
                SignalType::Int => OutStore::Int,
                SignalType::Trigger => OutStore::Trigger,
                SignalType::Midi => OutStore::Midi,
                SignalType::Any => OutStore::None,
            };
        }
    }

    // --- 5. steps with virtual buffers ---
    let mut b = Builder {
        steps: Vec::new(),
        input_slots: Vec::new(),
        output_slots: Vec::new(),
        out_samples: Vec::new(),
        out_scalars: Vec::new(),
        out_ints: Vec::new(),
        out_triggers: Vec::new(),
        out_midi: Vec::new(),
        chan_table: Vec::new(),
        vsamples: vec![Life {
            def: 0,
            last: usize::MAX,
        }],
        vtriggers: Vec::new(),
        vmidi: Vec::new(),
        n_scalars: 0,
        n_ints: 0,
        params: Vec::new(),
        param_smooth: Vec::new(),
        param_map: HashMap::new(),
        fill_steps: Vec::new(),
    };
    let smooth_len = (SMOOTH_SECONDS * sample_rate).round().max(1.0) as u32;
    let mut alloc: Vec<Vec<Alloc>> = flat
        .nodes
        .iter()
        .map(|f| vec![Alloc::None; f.sig.outputs.len()])
        .collect();
    // node index → step indices that run it (for telemetry wiring)
    let mut node_steps: Vec<Vec<usize>> = vec![Vec::new(); n];

    for &item in &order {
        let node = item.node();
        let f = &flat.nodes[node];
        let phase = match item {
            Item::Main(_) => Phase::Main,
            Item::FbRead(_) => Phase::FeedbackRead,
            Item::FbWrite(_) => Phase::FeedbackWrite,
        };
        let wants_inputs = !matches!(item, Item::FbRead(_));
        let wants_outputs = !matches!(item, Item::FbWrite(_));

        // Inputs (conversion steps may be pushed first).
        let mut slots = Vec::with_capacity(f.sig.inputs.len());
        for (i, spec) in f.sig.inputs.iter().enumerate() {
            if !wants_inputs {
                slots.push(InputSlot::None);
                continue;
            }
            let slot = match src_of[node][i] {
                Some((s, o)) => {
                    let src_spec = &flat.nodes[s].sig.outputs[o];
                    let a = alloc[s][o].clone();
                    wired_input(&mut b, &f.path, spec, src_spec.ty, &a, &mut report)
                }
                None => unwired_input(&mut b, f, spec, smooth_len),
            };
            slots.push(slot);
        }
        let step_index = b.steps.len();
        // Record reads for buffer lifetimes.
        for s in &slots {
            match *s {
                InputSlot::Float(FloatSrc::Sample(v)) => b.use_sample(v, step_index),
                InputSlot::Audio { start, count } => {
                    for k in start..start + count {
                        let v = b.chan_table[k as usize];
                        b.use_sample(v, step_index);
                    }
                }
                InputSlot::Trigger(TriggerSrc::Wired(v)) => {
                    if let Some(l) = b.vtriggers.get_mut(v as usize) {
                        l.last = l.last.max(step_index);
                    }
                }
                InputSlot::Midi(Some(v)) => {
                    if let Some(l) = b.vmidi.get_mut(v as usize) {
                        l.last = l.last.max(step_index);
                    }
                }
                _ => {}
            }
        }
        let in_start = b.input_slots.len() as u32;
        b.input_slots.extend(slots);
        let inputs: Range = (in_start, b.input_slots.len() as u32);

        // Outputs.
        let out_start = b.output_slots.len() as u32;
        let (s0, sc0, i0, t0, m0) = (
            b.out_samples.len() as u32,
            b.out_scalars.len() as u32,
            b.out_ints.len() as u32,
            b.out_triggers.len() as u32,
            b.out_midi.len() as u32,
        );
        for (o, spec) in f.sig.outputs.iter().enumerate() {
            if !wants_outputs {
                b.output_slots.push(OutputSlot::None);
                continue;
            }
            let (slot, a) = match store[node][o] {
                OutStore::Audio(ch) => {
                    let local = b.out_samples.len() as u32 - s0;
                    let mut vs = Vec::with_capacity(ch);
                    for _ in 0..ch {
                        let v = b.new_vsample(step_index);
                        b.out_samples.push(v);
                        vs.push(v);
                    }
                    (
                        OutputSlot::Audio {
                            start: local,
                            count: ch as u32,
                        },
                        Alloc::Audio(vs),
                    )
                }
                OutStore::FloatSample => {
                    let local = b.out_samples.len() as u32 - s0;
                    let v = b.new_vsample(step_index);
                    b.out_samples.push(v);
                    (
                        OutputSlot::Float {
                            local,
                            sample: true,
                        },
                        Alloc::Sample(v),
                    )
                }
                OutStore::FloatBlock => {
                    let local = b.out_scalars.len() as u32 - sc0;
                    let k = b.n_scalars;
                    b.n_scalars += 1;
                    b.out_scalars.push(k);
                    (
                        OutputSlot::Float {
                            local,
                            sample: false,
                        },
                        Alloc::Scalar(k),
                    )
                }
                OutStore::Int => {
                    let local = b.out_ints.len() as u32 - i0;
                    let k = b.n_ints;
                    b.n_ints += 1;
                    b.out_ints.push(k);
                    (OutputSlot::Int(local), Alloc::Int(k))
                }
                OutStore::Trigger => {
                    let local = b.out_triggers.len() as u32 - t0;
                    let v = b.new_vtrigger(step_index);
                    b.out_triggers.push(v);
                    (OutputSlot::Trigger(local), Alloc::Trigger(v))
                }
                OutStore::Midi => {
                    let local = b.out_midi.len() as u32 - m0;
                    let v = b.new_vmidi(step_index);
                    b.out_midi.push(v);
                    (OutputSlot::Midi(local), Alloc::Midi(v))
                }
                OutStore::None => (OutputSlot::None, Alloc::None),
            };
            let _ = spec;
            b.output_slots.push(slot);
            alloc[node][o] = a;
        }
        let outputs: Range = (out_start, b.output_slots.len() as u32);

        let bypass = if f.muted { bypass_for(f) } else { Bypass::None };
        b.steps.push(Step {
            kind: StepKind::Node {
                node: node as u32,
                phase,
            },
            muted: f.muted,
            bypass,
            inputs,
            outputs,
            o_samples: (s0, b.out_samples.len() as u32),
            o_scalars: (sc0, b.out_scalars.len() as u32),
            o_ints: (i0, b.out_ints.len() as u32),
            o_triggers: (t0, b.out_triggers.len() as u32),
            o_midi: (m0, b.out_midi.len() as u32),
            tele_values: (0, 0),
            tele_scope: (0, 0),
            tele_scope_pos: None,
            tele_time: None,
            socket_tele: (0, 0),
        });
        node_steps[node].push(step_index);
    }

    // --- 6. physical buffers by lifetime ---
    let n_steps = b.steps.len();
    let sample_map = assign(&b.vsamples, n_steps, true);
    let trig_map = assign(&b.vtriggers, n_steps, false);
    let midi_map = assign(&b.vmidi, n_steps, false);
    let n_samples = sample_map
        .iter()
        .copied()
        .max()
        .map_or(1, |m| m as usize + 1);
    let n_trig = trig_map.iter().copied().max().map_or(0, |m| m as usize + 1);
    let n_midi = midi_map.iter().copied().max().map_or(0, |m| m as usize + 1);
    let map_s = |v: u32| sample_map.get(v as usize).copied().unwrap_or(0);
    for s in &mut b.input_slots {
        match s {
            InputSlot::Float(FloatSrc::Sample(v)) => *v = map_s(*v),
            InputSlot::Trigger(TriggerSrc::Wired(v)) => {
                *v = trig_map.get(*v as usize).copied().unwrap_or(0)
            }
            InputSlot::Midi(Some(v)) => *v = midi_map.get(*v as usize).copied().unwrap_or(0),
            _ => {}
        }
    }
    for v in &mut b.chan_table {
        *v = map_s(*v);
    }
    for v in &mut b.out_samples {
        *v = map_s(*v);
    }
    for v in &mut b.out_triggers {
        *v = trig_map.get(*v as usize).copied().unwrap_or(0);
    }
    for v in &mut b.out_midi {
        *v = midi_map.get(*v as usize).copied().unwrap_or(0);
    }
    for &si in &b.fill_steps {
        if let StepKind::FillFromScalar { dst, .. } = &mut b.steps[si].kind {
            *dst = map_s(*dst);
        }
    }

    // --- 7. nodes, carry-over, telemetry ---
    let reuse = previous.filter(|p| p.sample_rate == sample_rate && p.max_block == max_block);
    let mut nodes: Vec<NodeBox> = Vec::new();
    let mut carry = Vec::new();
    let mut carry_layout = CarryLayout {
        nodes: HashMap::new(),
        sample_rate,
        max_block,
    };
    let mut node_index: Vec<Option<u32>> = vec![None; n];
    let mut layout = TelemetryLayout::default();
    let (mut n_values, mut n_scope, mut n_scope_pos, mut n_time) = (0usize, 0usize, 0usize, 0usize);
    let mut socket_tele = Vec::new();
    let mut n_socket = 0usize;
    let mut live_sources = Vec::new();
    let mut latency = 0u32;
    for (fi, f) in flat.nodes.iter().enumerate() {
        if node_steps[fi].is_empty() {
            continue;
        }
        let idx = nodes.len() as u32;
        node_index[fi] = Some(idx);
        let config_key = serde_json::to_string(&f.config).unwrap_or_default();
        let key = CarryKey {
            kind: f.kind.clone(),
            config: config_key.clone(),
            index: idx,
        };
        let carried = reuse
            .and_then(|p| p.nodes.get(&f.path))
            .filter(|k| k.kind == f.kind && k.config == config_key);
        let node: Box<dyn ac_core::Node> = match carried {
            Some(old) => {
                carry.push((idx, old.index));
                Box::new(Vacant)
            }
            None => {
                let key = f.path.to_string();
                let ctx = PrepareCtx {
                    sample_rate,
                    max_block,
                    seed: derive_seed(project.settings.seed, f.path.stable_hash()),
                    node_key: &key,
                    name: &f.name,
                    resources,
                };
                let node = (f.create)(&f.config, &ctx);
                latency = latency.max(node.latency());
                node
            }
        };
        nodes.push(NodeBox {
            node,
            path: f.path.clone(),
            load_pending: carried.is_none(),
        });
        carry_layout.nodes.insert(f.path.clone(), key);
        if f.meta.live {
            live_sources.push(f.path.clone());
        }

        let values = (n_values, n_values + f.meta.telemetry);
        n_values += f.meta.telemetry;
        let scope = (n_scope, n_scope + f.meta.scope);
        n_scope += f.meta.scope;
        let scope_pos = (f.meta.scope > 0).then(|| {
            n_scope_pos += 1;
            n_scope_pos - 1
        });
        let time = n_time;
        n_time += 1;
        layout.nodes.insert(
            f.path.clone(),
            NodeTelemetry {
                values,
                scope,
                scope_pos,
                time: Some(time),
            },
        );
        // Socket telemetry slots, one per output.
        let mut slots = Vec::with_capacity(f.sig.outputs.len());
        for spec in &f.sig.outputs {
            layout
                .sockets
                .insert((f.path.clone(), spec.name.to_string()), n_socket);
            slots.push(n_socket);
            n_socket += 1;
        }
        for &si in &node_steps[fi] {
            let step = &mut b.steps[si];
            if let StepKind::Node { node, .. } = &mut step.kind {
                *node = idx;
            }
            step.tele_values = (values.0 as u32, values.1 as u32);
            step.tele_scope = (scope.0 as u32, scope.1 as u32);
            step.tele_scope_pos = scope_pos.map(|p| p as u32);
            step.tele_time = Some(time as u32);
            if opts.socket_telemetry {
                let start = socket_tele.len() as u32;
                socket_tele.extend(slots.iter().map(|&s| SocketTele { slot: s as u32 }));
                step.socket_tele = (start, socket_tele.len() as u32);
            }
        }
    }
    for ((path, socket), src) in &flat.group_outputs {
        let f = &flat.nodes[src.0];
        if let Some(name) = f.sig.outputs.get(src.1).map(|s| s.name.to_string())
            && let Some(&slot) = layout.sockets.get(&(f.path.clone(), name))
        {
            layout.sockets.insert((path.clone(), socket.clone()), slot);
        }
    }
    // Group-instance inputs set the params of their inner targets.
    for ((path, socket), targets) in &flat.group_inputs {
        let mut list = Vec::new();
        for &(tn, ti) in targets {
            let f = &flat.nodes[tn];
            if let Some(name) = f.sig.inputs.get(ti).map(|s| s.name.to_string())
                && let Some(t) = b.param_map.get(&(f.path.clone(), name))
            {
                list.extend(t.iter().cloned());
            }
        }
        if !list.is_empty() {
            b.param_map.insert((path.clone(), socket.clone()), list);
        }
    }

    let telemetry = Arc::new(TelemetryStore {
        node_values: atomics(n_values),
        scope: atomics(n_scope),
        scope_pos: atomics(n_scope_pos),
        socket_level: atomics(n_socket),
        socket_value: atomics(n_socket),
        socket_events: atomics(n_socket),
        node_time: atomics(n_time),
    });

    // Scratch space sized to the widest step.
    let widest = |f: fn(&Step) -> Range| {
        b.steps
            .iter()
            .map(|s| {
                let r = f(s);
                (r.1 - r.0) as usize
            })
            .max()
            .unwrap_or(0)
    };
    let scratch = Scratch {
        samples: (0..widest(|s| s.o_samples)).map(|_| Vec::new()).collect(),
        scalars: vec![0.0; widest(|s| s.o_scalars)],
        ints: vec![0; widest(|s| s.o_ints)],
        triggers: (0..widest(|s| s.o_triggers)).map(|_| Vec::new()).collect(),
        midi: (0..widest(|s| s.o_midi)).map(|_| Vec::new()).collect(),
    };

    report.stats.nodes = nodes.len();
    report.stats.steps = b.steps.len();
    report.stats.sample_buffers = n_samples;
    report.stats.params = b.params.len();
    report.stats.live_sources = live_sources;
    report.stats.latency = latency;
    report
        .diagnostics
        .sort_by_key(|d| std::cmp::Reverse(d.severity));

    let block_seconds = max_block as f64 / sample_rate;
    let level_decay = (-(block_seconds / 0.3)).exp() as f32;
    let n_params = b.params.len();
    let tempo = TempoMap::new(sample_rate, &project.settings.tempo);
    let graph = Box::new(CompiledGraph {
        generation: opts.generation,
        sample_rate,
        max_block,
        tempo,
        nodes,
        carry,
        steps: b.steps,
        input_slots: b.input_slots,
        output_slots: b.output_slots,
        out_samples: b.out_samples,
        out_scalars: b.out_scalars,
        out_ints: b.out_ints,
        out_triggers: b.out_triggers,
        out_midi: b.out_midi,
        chan_table: b.chan_table,
        samples: (0..n_samples).map(|_| vec![0.0; max_block]).collect(),
        scalars: vec![0.0; b.n_scalars as usize],
        ints: vec![0; b.n_ints as usize],
        triggers: (0..n_trig)
            .map(|_| Vec::with_capacity(max_block + 64))
            .collect(),
        midi: (0..n_midi)
            .map(|_| Vec::<MidiEvent>::with_capacity(MIDI_CAPACITY))
            .collect(),
        params: b.params,
        param_smooth: b.param_smooth,
        active_params: Vec::with_capacity(n_params),
        telemetry: telemetry.clone(),
        socket_tele,
        scratch,
        level_decay,
    });
    Compiled {
        graph,
        layout: Arc::new(layout),
        telemetry,
        params: b.param_map,
        carry: carry_layout,
        report,
    }
}

/// Channels an input sees from a source with `src` channels, after upmix.
fn input_channels(rule: Channels, src: usize) -> usize {
    match rule {
        Channels::Fixed(n) => n as usize,
        _ => src,
    }
}

fn wired_input(
    b: &mut Builder,
    path: &NodePath,
    spec: &SocketSpec,
    src_ty: SignalType,
    a: &Alloc,
    report: &mut CompileReport,
) -> InputSlot {
    match spec.ty {
        SignalType::Audio => {
            let bufs: Vec<u32> = match (a, conversion(src_ty, spec.ty)) {
                (Alloc::Audio(v), _) => v.clone(),
                (Alloc::Sample(v), _) => vec![*v],
                (Alloc::Scalar(k), Some(Conversion::FloatToAudio)) => {
                    // Materialize the block value into a buffer just before the consumer.
                    let at = b.steps.len();
                    let v = b.new_vsample(at);
                    b.fill_steps.push(at);
                    b.steps.push(Step {
                        kind: StepKind::FillFromScalar { scalar: *k, dst: v },
                        muted: false,
                        bypass: Bypass::None,
                        inputs: (0, 0),
                        outputs: (0, 0),
                        o_samples: (0, 0),
                        o_scalars: (0, 0),
                        o_ints: (0, 0),
                        o_triggers: (0, 0),
                        o_midi: (0, 0),
                        tele_values: (0, 0),
                        tele_scope: (0, 0),
                        tele_scope_pos: None,
                        tele_time: None,
                        socket_tele: (0, 0),
                    });
                    vec![v]
                }
                _ => return silent_audio(b, spec),
            };
            let src = bufs.len();
            let ok = match spec.channels {
                Channels::Fixed(n) => src == 1 || src == n as usize,
                Channels::UpTo(n) => src <= n as usize,
                Channels::Follow => true,
            };
            if !ok {
                report.push(
                    Severity::Error,
                    Some(path),
                    Some(&spec.name),
                    format!(
                        "channel mismatch: {src} channels into an input that takes {}",
                        match spec.channels {
                            Channels::Fixed(n) => format!("{n}"),
                            Channels::UpTo(n) => format!("up to {n}"),
                            Channels::Follow => "any".into(),
                        }
                    ),
                );
                return silent_audio(b, spec);
            }
            let want = input_channels(spec.channels, src);
            let start = b.chan_table.len() as u32;
            for c in 0..want {
                b.chan_table.push(bufs[c % src.max(1)]);
            }
            InputSlot::Audio {
                start,
                count: want as u32,
            }
        }
        SignalType::Float | SignalType::Bool => match a {
            Alloc::Sample(v) => InputSlot::Float(FloatSrc::Sample(*v)),
            Alloc::Scalar(k) => InputSlot::Float(FloatSrc::Scalar(*k)),
            Alloc::Int(k) => InputSlot::Float(FloatSrc::Int(*k)),
            Alloc::Audio(v) => InputSlot::Float(FloatSrc::Sample(v.first().copied().unwrap_or(0))),
            _ => InputSlot::Float(FloatSrc::Scalar(u32::MAX)),
        },
        SignalType::Int => match a {
            Alloc::Int(k) => InputSlot::Int(IntSrc::Int(*k)),
            _ => InputSlot::None,
        },
        SignalType::Trigger => match a {
            Alloc::Trigger(v) => InputSlot::Trigger(TriggerSrc::Wired(*v)),
            _ => InputSlot::None,
        },
        SignalType::Midi => match a {
            Alloc::Midi(v) => InputSlot::Midi(Some(*v)),
            _ => InputSlot::Midi(None),
        },
        SignalType::Any => InputSlot::None,
    }
}

fn silent_audio(b: &mut Builder, spec: &SocketSpec) -> InputSlot {
    let want = match spec.channels {
        Channels::Fixed(n) => n as usize,
        _ => 1,
    };
    let start = b.chan_table.len() as u32;
    b.chan_table.extend(std::iter::repeat_n(0, want));
    InputSlot::Audio {
        start,
        count: want as u32,
    }
}

fn unwired_input(
    b: &mut Builder,
    f: &crate::flatten::FlatNode,
    spec: &SocketSpec,
    smooth_len: u32,
) -> InputSlot {
    let new_param = |b: &mut Builder| -> u32 {
        let value = param_value(spec, f.params.get(spec.name.as_ref()));
        b.params.push(ParamSlot {
            value,
            target: value,
            ..ParamSlot::default()
        });
        let smooth = spec.smooth && spec.ty == SignalType::Float && spec.choices.is_empty();
        b.param_smooth.push(if smooth { smooth_len } else { 0 });
        let slot = (b.params.len() - 1) as u32;
        b.param_map
            .entry((f.path.clone(), spec.name.to_string()))
            .or_default()
            .push(ParamTarget {
                slot,
                spec: spec.clone(),
            });
        slot
    };
    match spec.ty {
        SignalType::Audio => silent_audio(b, spec),
        SignalType::Float | SignalType::Bool => InputSlot::Float(FloatSrc::Param(new_param(b))),
        SignalType::Int => InputSlot::Int(IntSrc::Param(new_param(b))),
        SignalType::Trigger => InputSlot::Trigger(TriggerSrc::Unwired {
            param: new_param(b),
            auto: spec.auto,
        }),
        SignalType::Midi => InputSlot::Midi(None),
        SignalType::Any => InputSlot::None,
    }
}

fn bypass_for(f: &crate::flatten::FlatNode) -> Bypass {
    let (Some(i), Some(o)) = (f.sig.input_index("in"), f.sig.output_index("out")) else {
        return Bypass::None;
    };
    let (it, ot) = (f.sig.inputs[i].ty, f.sig.outputs[o].ty);
    match (it, ot) {
        (SignalType::Audio, SignalType::Audio) => Bypass::Audio {
            input: i as u32,
            output: o as u32,
        },
        (SignalType::Float | SignalType::Bool, SignalType::Float | SignalType::Bool) => {
            Bypass::Float {
                input: i as u32,
                output: o as u32,
            }
        }
        _ => Bypass::None,
    }
}

/// Topological order of schedule items; Feedback nodes become a read item
/// (no dependencies) and a write item (after their input). Nodes on a cycle
/// without a Feedback node are reported and left out; inputs they fed become
/// unwired so everything downstream still runs.
fn schedule(
    flat: &Flat,
    src_of: &mut [Vec<Option<(usize, usize)>>],
    report: &mut CompileReport,
) -> Vec<Item> {
    let mut excluded: BTreeSet<usize> = BTreeSet::new();
    loop {
        let (order, stuck) = kahn(flat, src_of, &excluded);
        if stuck.is_empty() {
            return order;
        }
        let members = cycle_members(flat, src_of, &stuck);
        if members.is_empty() {
            // Can't happen (a stuck set always contains a cycle); bail out safely.
            return order;
        }
        let names: Vec<String> = members
            .iter()
            .map(|&i| format!("{} ({})", flat.nodes[i].name, flat.nodes[i].path))
            .collect();
        for &i in &members {
            report.error(
                Some(&flat.nodes[i].path),
                None,
                format!(
                    "cycle without a Feedback node through: {}. Insert a Feedback node to close the loop.",
                    names.join(", ")
                ),
            );
        }
        for srcs in src_of.iter_mut() {
            for s in srcs.iter_mut() {
                if s.is_some_and(|(sn, _)| members.contains(&sn)) {
                    *s = None;
                }
            }
        }
        excluded.extend(members);
    }
}

fn kahn(
    flat: &Flat,
    src_of: &[Vec<Option<(usize, usize)>>],
    excluded: &BTreeSet<usize>,
) -> (Vec<Item>, BTreeSet<usize>) {
    let n = flat.nodes.len();
    let is_fb = |i: usize| flat.nodes[i].meta.feedback;
    let mut items: Vec<Item> = Vec::new();
    for i in 0..n {
        let f = &flat.nodes[i];
        if (f.sig.inputs.is_empty() && f.sig.outputs.is_empty()) || excluded.contains(&i) {
            continue;
        }
        if is_fb(i) {
            items.push(Item::FbRead(i));
            items.push(Item::FbWrite(i));
        } else {
            items.push(Item::Main(i));
        }
    }
    let producer = |s: usize| {
        if is_fb(s) {
            Item::FbRead(s)
        } else {
            Item::Main(s)
        }
    };
    let index: HashMap<Item, usize> = items.iter().enumerate().map(|(k, it)| (*it, k)).collect();
    let mut indeg = vec![0usize; items.len()];
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    for (k, it) in items.iter().enumerate() {
        if matches!(it, Item::FbRead(_)) {
            continue;
        }
        let node = it.node();
        let mut deps = BTreeSet::new();
        for (s, _) in src_of[node].iter().flatten().copied() {
            if let Some(&d) = index.get(&producer(s)) {
                deps.insert(d);
            }
        }
        // A feedback write must follow its own read (it overwrites the store).
        if let Item::FbWrite(nn) = it
            && let Some(&r) = index.get(&Item::FbRead(*nn))
        {
            deps.insert(r);
        }
        indeg[k] = deps.len();
        for d in deps {
            users[d].push(k);
        }
    }
    let mut ready: BTreeSet<usize> = (0..items.len()).filter(|&k| indeg[k] == 0).collect();
    let mut order = Vec::with_capacity(items.len());
    while let Some(k) = ready.pop_first() {
        order.push(items[k]);
        for &u in &users[k] {
            indeg[u] -= 1;
            if indeg[u] == 0 {
                ready.insert(u);
            }
        }
    }
    let stuck = (0..items.len())
        .filter(|&k| indeg[k] > 0)
        .map(|k| items[k].node())
        .collect();
    (order, stuck)
}

/// Nodes among `stuck` that can reach themselves through non-Feedback wires.
fn cycle_members(
    flat: &Flat,
    src_of: &[Vec<Option<(usize, usize)>>],
    stuck: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    // Forward edges source → consumer, restricted to stuck nodes; edges out of
    // a Feedback node don't count (its read side has no dependencies).
    let mut fwd: HashMap<usize, Vec<usize>> = HashMap::new();
    for &c in stuck {
        for (s, _) in src_of[c].iter().flatten().copied() {
            if stuck.contains(&s) && !flat.nodes[s].meta.feedback {
                fwd.entry(s).or_default().push(c);
            }
        }
    }
    let mut members = BTreeSet::new();
    for &start in stuck {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = fwd.get(&start).cloned().unwrap_or_default();
        while let Some(x) = stack.pop() {
            if x == start {
                members.insert(start);
                break;
            }
            if seen.insert(x)
                && let Some(next) = fwd.get(&x)
            {
                stack.extend(next.iter().copied());
            }
        }
    }
    members
}

/// Assign physical buffers to virtual ones by lifetime. Outputs of a step are
/// allocated before that step's inputs are freed, so a node never writes a
/// buffer it reads. For sample buffers, physical 0 is reserved silence.
fn assign(lives: &[Life], n_steps: usize, reserve_zero: bool) -> Vec<u32> {
    let mut map = vec![0u32; lives.len()];
    let first = usize::from(reserve_zero);
    let mut by_def: Vec<Vec<usize>> = vec![Vec::new(); n_steps + 1];
    let mut by_last: Vec<Vec<usize>> = vec![Vec::new(); n_steps + 1];
    for (v, l) in lives.iter().enumerate().skip(first) {
        by_def[l.def.min(n_steps)].push(v);
        by_last[l.last.min(n_steps)].push(v);
    }
    let mut free: Vec<u32> = Vec::new();
    let mut next = first as u32;
    for step in 0..=n_steps {
        for &v in &by_def[step] {
            map[v] = free.pop().unwrap_or_else(|| {
                next += 1;
                next - 1
            });
        }
        for &v in &by_last[step] {
            free.push(map[v]);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetimes_reuse_buffers_without_aliasing_a_step() {
        // v1 defined at 0, read at 1; v2 defined at 1 (must not get v1's buffer);
        // v3 defined at 2 (may reuse v1's).
        let lives = vec![
            Life {
                def: 0,
                last: usize::MAX,
            },
            Life { def: 0, last: 1 },
            Life { def: 1, last: 2 },
            Life { def: 2, last: 3 },
        ];
        let m = assign(&lives, 4, true);
        assert_eq!(m[0], 0);
        assert_ne!(m[1], m[2]);
        assert_eq!(m[3], m[1]);
        assert!(m.iter().skip(1).all(|&p| p != 0));
    }
}
