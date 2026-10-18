//! Compilation: project → flat schedule (ARCHITECTURE §7).
//!
//! 1. Flatten groups and voices and remove reroutes (`flatten.rs`).
//! 2. Validate wires: types, one wire per input.
//! 3. Topologically sort, cutting cycles at `Feedback` nodes (split into a
//!    read step that runs early and a write step that runs after its input).
//! 4. Causality: each whole-input node is disabled (a live source feeds it)
//!    or streams its bake (`bake.rs`).
//! 5. Infer output rates, channel counts, and spectrum shapes in schedule
//!    order.
//! 6. Create nodes, or plan to carry state over from the previous graph.
//! 7. Latency compensation: where paths of different latency merge, delay
//!    the early inputs so they line up with the slowest.
//! 8. Build steps with virtual buffers, inserting implicit conversions.
//! 9. Group steps into chains (runs one thread works through in order) and
//!    chains into waves of mutually independent chains, what the multi-core
//!    scheduler runs in parallel (`chains.rs`); assign physical buffers by
//!    wave so they're reused, and let chains recycle the buffers only they
//!    read.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ac_core::{
    Channels, Config, Conversion, FloatSrc, InputSlot, IntSrc, LOCAL, MIDI_CAPACITY, MidiEvent,
    Node, NodePath, OutRate, OutputSlot, ParamSlot, Phase, PrepareCtx, Project, Rate, Registry,
    RenderWholeFn, Resources, SignalType, SocketSpec, SpectrumBuf, SpectrumShape, TempoMap,
    TriggerSrc, WholeInputs, conversion, param_value,
    util::{Fnv64, derive_seed},
};
use serde_json::json;

use crate::bake::{BakeSpec, BakeStore, WholeState};
use crate::bindings::{BindingTable, BindingValues, compile_bindings};
use crate::chains::{self, StepInfo};
use crate::flatten::{Flat, FlatNode, flatten, voice_copies};
use crate::graph::{
    At, Bypass, CompiledGraph, NodeBox, Pool, Range, SocketTele, Step, StepKind, Vacant, Wave, Work,
};
use crate::internal::{self, Baked, COMP_DELAY_KIND, Capture, CompDelay, CompType};
use crate::report::{CompileReport, Severity};
use crate::telemetry::{NodeTelemetry, TelemetryLayout, TelemetryStore, atomics};

/// Smoothing time for parameter changes ("a few ms", ARCHITECTURE §8).
const SMOOTH_SECONDS: f64 = 0.005;
/// Upper bound on channels per wire, to keep a bad config from allocating wildly.
const MAX_CHANNELS: usize = 64;
/// Node ids of compensation delays, under the node whose input they feed;
/// the input index is added. High enough not to meet hand-written ids.
const COMP_ID_BASE: u64 = 0xC0FF_EE00_0000_0000;

/// Settings that shape a compile.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub sample_rate: f64,
    pub max_block: usize,
    /// Publish per-socket levels/values/event counts (the editor wants them;
    /// offline renders don't).
    pub socket_telemetry: bool,
    pub generation: u64,
    /// An offline render knows every input in advance, so whole-input nodes
    /// fed by Audio In or MIDI In are baked instead of disabled.
    pub offline: bool,
    /// Block size bakes render at (the project's render block size, so a bake
    /// is the same live and offline).
    pub bake_block: usize,
    /// Frames a bake covers.
    pub bake_frames: u64,
    /// Hash of the audio and MIDI an offline render feeds in (part of bake
    /// keys when a bake depends on it).
    pub input_hash: u64,
    /// Bumped by the host when assets change, so bakes that read them redo.
    pub resources_generation: u64,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            sample_rate: 48_000.0,
            max_block: 1024,
            socket_telemetry: false,
            generation: 0,
            offline: false,
            bake_block: 256,
            bake_frames: 0,
            input_hash: 0,
            resources_generation: 0,
        }
    }
}

/// Identity of a node in a compiled graph, for state carry-over.
#[derive(Clone, Debug, PartialEq)]
struct CarryKey {
    kind: String,
    config: String,
    index: u32,
    latency: u32,
    probe: u32,
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
    /// Where the graph publishes what its bindings set.
    pub bindings: Arc<BindingValues>,
    pub params: ParamMap,
    pub carry: CarryLayout,
    pub report: CompileReport,
    /// Every whole-input node and where it stands.
    pub bakes: Vec<BakeSpec>,
    /// For a bake: how to render the captured node.
    pub(crate) capture_entry: Option<(RenderWholeFn, Config, u64)>,
}

/// The node a bake captures the inputs of (see `bake.rs`).
pub(crate) struct CaptureTarget {
    pub path: NodePath,
    pub sink: Arc<Mutex<WholeInputs>>,
    pub frames: usize,
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
    Spectrum(SpectrumShape),
    Buffer,
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
    Spectrum(u32),
    Buffer(u32),
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

/// Lifetime of a virtual buffer: defined at `def`, last read at `last`
/// (`def` is a step index while building; both are waves when assigning).
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
    out_spectra: Vec<u32>,
    out_buffers: Vec<u32>,
    chan_table: Vec<u32>,
    /// Virtual sample buffers; index 0 is permanent silence.
    vsamples: Vec<Life>,
    vtriggers: Vec<Life>,
    vmidi: Vec<Life>,
    /// Defining step of each scalar, int, spectrum, and buffer slot.
    scalar_def: Vec<usize>,
    int_def: Vec<usize>,
    spectra: Vec<(SpectrumShape, usize)>,
    buffer_def: Vec<usize>,
    params: Vec<ParamSlot>,
    param_smooth: Vec<u32>,
    param_map: ParamMap,
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

    fn empty_step(kind: StepKind) -> Step {
        Step {
            kind,
            muted: false,
            bypass: Bypass::None,
            serial: false,
            inputs: (0, 0),
            outputs: (0, 0),
            o_samples: (0, 0),
            o_scalars: (0, 0),
            o_ints: (0, 0),
            o_triggers: (0, 0),
            o_midi: (0, 0),
            o_spectra: (0, 0),
            o_buffers: (0, 0),
            o_stash: (0, 0),
            acquire: (0, 0),
            release: (0, 0),
            reads_chain: false,
            tele_values: (0, 0),
            tele_scope: (0, 0),
            tele_scope_pos: None,
            tele_time: None,
            socket_tele: (0, 0),
        }
    }
}

/// Compile a project. Never fails: problems are reported and the offending
/// parts left out, so the rest of the graph still runs. `bakes` holds the
/// finished bakes whole-input nodes stream.
pub fn compile(
    project: &Project,
    registry: &Registry,
    resources: &dyn Resources,
    opts: &CompileOptions,
    previous: Option<&CarryLayout>,
    bakes: &BakeStore,
) -> Compiled {
    compile_with(project, registry, resources, opts, previous, bakes, None)
}

/// [`compile`], optionally for a bake: only the target's upstream is kept,
/// and the target records its inputs instead of running.
pub(crate) fn compile_with(
    project: &Project,
    registry: &Registry,
    resources: &dyn Resources,
    opts: &CompileOptions,
    previous: Option<&CarryLayout>,
    bakes: &BakeStore,
    capture: Option<&CaptureTarget>,
) -> Compiled {
    let mut report = CompileReport::default();
    let sample_rate = if opts.sample_rate.is_finite() && opts.sample_rate > 0.0 {
        opts.sample_rate
    } else {
        report.warning(None, None, "invalid sample rate; using 48000");
        48_000.0
    };
    let max_block = opts.max_block.clamp(1, 16_384);

    let mut flat = flatten(project, registry, &mut report);
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
    let mut order = schedule(&flat, &mut src_of, &mut report);
    let capture_index = capture.and_then(|c| flat.nodes.iter().position(|f| f.path == c.path));
    if let Some(t) = capture_index {
        let keep = upstream(&src_of, t);
        order.retain(|it| it.node() == t || keep.contains(&it.node()));
    }
    let scheduled: HashSet<usize> = order.iter().map(|it| it.node()).collect();

    // --- 4. causality ---
    let mut whole: HashMap<usize, BakeSpec> = HashMap::new();
    for (i, f) in flat.nodes.iter().enumerate() {
        if !f.meta.whole_input || !scheduled.contains(&i) || Some(i) == capture_index {
            continue;
        }
        let up = upstream(&src_of, i);
        let mut live: Vec<String> = up
            .iter()
            .filter(|&&u| flat.nodes[u].meta.live)
            .map(|&u| flat.nodes[u].name.clone())
            .collect();
        live.sort();
        live.dedup();
        let key = bake_key(project, &flat, &src_of, i, &up, opts, !live.is_empty());
        let state = if !live.is_empty() && !opts.offline {
            report.push(
                Severity::Info,
                Some(&f.path),
                None,
                format!(
                    "needs a known input; disabled while live (fed by {})",
                    live.join(", ")
                ),
            );
            WholeState::Disabled { sources: live }
        } else {
            match bakes.get(&f.path) {
                Some(r) if r.key == key => WholeState::Baked,
                Some(_) => WholeState::Stale,
                None => WholeState::Pending,
            }
        };
        let deps = up
            .iter()
            .filter(|&&u| flat.nodes[u].meta.whole_input)
            .map(|&u| flat.nodes[u].path.clone())
            .collect();
        let mut upstream_paths: Vec<NodePath> =
            up.iter().map(|&u| flat.nodes[u].path.clone()).collect();
        upstream_paths.sort();
        whole.insert(
            i,
            BakeSpec {
                path: f.path.clone(),
                key,
                state,
                deps,
                upstream: upstream_paths,
            },
        );
    }

    // --- 5. rates, channels, spectrum shapes ---
    let mut store: Vec<Vec<OutStore>> = flat
        .nodes
        .iter()
        .map(|f| vec![OutStore::None; f.sig.outputs.len()])
        .collect();
    for &item in &order {
        if matches!(item, Item::FbWrite(_)) {
            continue;
        }
        let node = item.node();
        let feedback = matches!(item, Item::FbRead(_));
        let stores = out_stores(&flat.nodes[node], &src_of[node], &store, feedback);
        store[node] = stores;
    }

    // --- 6. nodes ---
    let reuse = previous.filter(|p| p.sample_rate == sample_rate && p.max_block == max_block);
    let mut nb = NodeBuilder {
        nodes: Vec::new(),
        carry: Vec::new(),
        layout: CarryLayout {
            nodes: HashMap::new(),
            sample_rate,
            max_block,
        },
        index: vec![None; n],
        latency: vec![0; n],
        reuse,
    };
    let seed = project.settings.seed;
    let mut capture_entry = None;
    for &item in &order {
        let fi = item.node();
        if nb.index[fi].is_some() {
            continue;
        }
        let f = &flat.nodes[fi];
        if Some(fi) == capture_index
            && let Some(c) = capture
        {
            let node = Box::new(Capture::new(&f.sig, c.sink.clone(), c.frames));
            capture_entry = Some((
                f.render_whole,
                f.config.clone(),
                derive_seed(seed, f.path.stable_hash()),
            ));
            nb.push_fresh(f, fi, node, String::new());
            continue;
        }
        if let Some(spec) = whole.get(&fi) {
            let result = match spec.state {
                WholeState::Disabled { .. } => None,
                _ => bakes.get(&f.path).cloned(),
            };
            let extra = match &result {
                Some(r) => format!("baked:{:016x}", r.key),
                None => format!("silent:{}", spec.state.label()),
            };
            let sig = f.sig.clone();
            nb.prepare(
                f,
                fi,
                &extra,
                |_, _| Box::new(Baked::new(&sig, result.as_ref().map(|r| r.outputs.clone()))),
                sample_rate,
                max_block,
                seed,
                resources,
            );
            continue;
        }
        nb.prepare(f, fi, "", f.create, sample_rate, max_block, seed, resources);
    }

    // --- 7. latency compensation ---
    let mut lat_out = vec![0u32; n];
    let mut comps: Vec<(usize, usize, u32)> = Vec::new(); // (node, input, samples)
    for &item in &order {
        let node = item.node();
        match item {
            Item::FbRead(_) | Item::FbWrite(_) => continue,
            Item::Main(_) => {}
        }
        let mut in_lat = 0u32;
        let mut ins: Vec<(usize, u32)> = Vec::new();
        for (i, spec) in flat.nodes[node].sig.inputs.iter().enumerate() {
            let Some((s, o)) = src_of[node][i] else {
                continue;
            };
            if comp_type(spec.ty, store[s][o]).is_none() {
                // Spectra carry their producer's path latency (an FFT's
                // window) but can't be delayed (D56), so the other inputs
                // line up with them.
                if spec.ty == SignalType::Spectrum {
                    in_lat = in_lat.max(lat_out[s]);
                }
                continue;
            }
            let l = lat_out[s];
            ins.push((i, l));
            in_lat = in_lat.max(l);
        }
        for (i, l) in ins {
            if l < in_lat {
                comps.push((node, i, in_lat - l));
            }
        }
        lat_out[node] = in_lat.saturating_add(nb.latency[node]);
    }
    for &(node, input, samples) in &comps {
        let (s, o) = match src_of[node][input] {
            Some(x) => x,
            None => continue,
        };
        let Some(ty) = comp_type(flat.nodes[node].sig.inputs[input].ty, store[s][o]) else {
            continue;
        };
        let channels = match store[s][o] {
            OutStore::Audio(c) => c,
            _ => 1,
        };
        let config = json!({ "type": ty.name(), "samples": samples, "channels": channels });
        let path = flat.nodes[node]
            .path
            .child(ac_core::NodeId(COMP_ID_BASE + input as u64));
        let sig = CompDelay::describe(&config);
        let ci = flat.nodes.len();
        flat.nodes.push(FlatNode {
            path,
            kind: COMP_DELAY_KIND.to_string(),
            config,
            params: Default::default(),
            muted: false,
            name: format!("{} compensation", flat.nodes[node].name),
            sig,
            meta: CompDelay::meta(),
            create: internal::create::<CompDelay>,
            render_whole: CompDelay::render_whole,
        });
        src_of.push(vec![Some((s, o))]);
        src_of[node][input] = Some((ci, 0));
        let st = out_stores(&flat.nodes[ci], &src_of[ci], &store, false);
        store.push(st);
        nb.index.push(None);
        nb.latency.push(0);
        lat_out.push(lat_out[s] + samples);
        let f = &flat.nodes[ci];
        nb.prepare(f, ci, "", f.create, sample_rate, max_block, seed, resources);
        let at = order
            .iter()
            .position(|it| *it == Item::Main(node))
            .unwrap_or(order.len());
        order.insert(at, Item::Main(ci));
    }
    // Probes learn the path latency at their output; a carried probe whose
    // value changed is rebuilt.
    for (fi, f) in flat.nodes.iter().enumerate() {
        if !f.meta.latency_probe {
            continue;
        }
        let Some(idx) = nb.index[fi] else { continue };
        nb.set_probe(f, idx, lat_out[fi], sample_rate, max_block, seed, resources);
    }
    let total_latency = {
        let sinks: Vec<u32> = (0..flat.nodes.len())
            .filter(|&i| flat.nodes[i].meta.host_output && nb.index[i].is_some())
            .map(|i| lat_out[i])
            .collect();
        sinks
            .into_iter()
            .max()
            .unwrap_or_else(|| lat_out.iter().copied().max().unwrap_or(0))
    };
    let n = flat.nodes.len();

    // --- 8. steps with virtual buffers ---
    let mut b = Builder {
        steps: Vec::new(),
        input_slots: Vec::new(),
        output_slots: Vec::new(),
        out_samples: Vec::new(),
        out_scalars: Vec::new(),
        out_ints: Vec::new(),
        out_triggers: Vec::new(),
        out_midi: Vec::new(),
        out_spectra: Vec::new(),
        out_buffers: Vec::new(),
        chan_table: Vec::new(),
        vsamples: vec![Life {
            def: 0,
            last: usize::MAX,
        }],
        vtriggers: Vec::new(),
        vmidi: Vec::new(),
        scalar_def: Vec::new(),
        int_def: Vec::new(),
        spectra: Vec::new(),
        buffer_def: Vec::new(),
        params: Vec::new(),
        param_smooth: Vec::new(),
        param_map: HashMap::new(),
    };
    let smooth_len = (SMOOTH_SECONDS * sample_rate).round().max(1.0) as u32;
    let mut alloc: Vec<Vec<Alloc>> = flat
        .nodes
        .iter()
        .map(|f| vec![Alloc::None; f.sig.outputs.len()])
        .collect();
    // node index → step indices that run it (for telemetry wiring)
    let mut node_steps: Vec<Vec<usize>> = vec![Vec::new(); n];
    // Feedback node → its read step (its write step must come later).
    let mut fb_read_step: HashMap<usize, usize> = HashMap::new();
    let mut extra_deps: Vec<(usize, usize)> = Vec::new();

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
        match item {
            Item::FbRead(_) => {
                fb_read_step.insert(node, step_index);
            }
            Item::FbWrite(_) => {
                if let Some(&r) = fb_read_step.get(&node) {
                    extra_deps.push((step_index, r));
                }
            }
            Item::Main(_) => {}
        }
        let in_start = b.input_slots.len() as u32;
        b.input_slots.extend(slots);
        let inputs: Range = (in_start, b.input_slots.len() as u32);

        // Outputs.
        let out_start = b.output_slots.len() as u32;
        let (s0, sc0, i0, t0, m0, sp0, bu0) = (
            b.out_samples.len() as u32,
            b.out_scalars.len() as u32,
            b.out_ints.len() as u32,
            b.out_triggers.len() as u32,
            b.out_midi.len() as u32,
            b.out_spectra.len() as u32,
            b.out_buffers.len() as u32,
        );
        for o in 0..f.sig.outputs.len() {
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
                    let k = b.scalar_def.len() as u32;
                    b.scalar_def.push(step_index);
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
                    let k = b.int_def.len() as u32;
                    b.int_def.push(step_index);
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
                OutStore::Spectrum(shape) => {
                    let local = b.out_spectra.len() as u32 - sp0;
                    let k = b.spectra.len() as u32;
                    b.spectra.push((shape, step_index));
                    b.out_spectra.push(k);
                    (OutputSlot::Spectrum(local), Alloc::Spectrum(k))
                }
                OutStore::Buffer => {
                    let local = b.out_buffers.len() as u32 - bu0;
                    let k = b.buffer_def.len() as u32;
                    b.buffer_def.push(step_index);
                    b.out_buffers.push(k);
                    (OutputSlot::Buffer(local), Alloc::Buffer(k))
                }
                OutStore::None => (OutputSlot::None, Alloc::None),
            };
            b.output_slots.push(slot);
            alloc[node][o] = a;
        }
        let outputs: Range = (out_start, b.output_slots.len() as u32);

        let bypass = if f.muted { bypass_for(f) } else { Bypass::None };
        let mut step = Builder::empty_step(StepKind::Node { node: 0, phase });
        step.muted = f.muted;
        step.bypass = bypass;
        step.serial = f.meta.host_output;
        step.inputs = inputs;
        step.outputs = outputs;
        step.o_samples = (s0, b.out_samples.len() as u32);
        step.o_scalars = (sc0, b.out_scalars.len() as u32);
        step.o_ints = (i0, b.out_ints.len() as u32);
        step.o_triggers = (t0, b.out_triggers.len() as u32);
        step.o_midi = (m0, b.out_midi.len() as u32);
        step.o_spectra = (sp0, b.out_spectra.len() as u32);
        step.o_buffers = (bu0, b.out_buffers.len() as u32);
        b.steps.push(step);
        node_steps[node].push(step_index);
    }

    // --- telemetry ---
    let mut layout = TelemetryLayout::default();
    let (mut n_values, mut n_scope, mut n_scope_pos, mut n_time) = (0usize, 0usize, 0usize, 0usize);
    let mut socket_tele = Vec::new();
    let mut n_socket = 0usize;
    let mut live_sources = Vec::new();
    for (fi, f) in flat.nodes.iter().enumerate() {
        let Some(idx) = nb.index[fi] else {
            continue;
        };
        if node_steps[fi].is_empty() {
            continue;
        }
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
    // Bindings resolve through the same map (a bake runs without them).
    let bindings = match capture {
        None => compile_bindings(project, registry, &b.param_map, &mut report),
        Some(_) => BindingTable::default(),
    };
    let binding_values = bindings.values.clone();

    // --- 9. chains, waves, and physical buffers ---
    let n_steps = b.steps.len();
    let mut step_node: Vec<Option<usize>> = vec![None; n_steps];
    for (fi, steps) in node_steps.iter().enumerate() {
        for &s in steps {
            step_node[s] = Some(fi);
        }
    }
    let copies = voice_copies(&flat.nodes);
    let mut infos: Vec<StepInfo> = step_reads(&b)
        .into_iter()
        .enumerate()
        .map(|(s, reads)| StepInfo {
            reads,
            after: Vec::new(),
            serial: b.steps[s].serial,
            node: step_node[s].map(|fi| fi as u32),
            voice: step_node[s].and_then(|fi| copies.get(fi).copied().flatten()),
        })
        .collect();
    // A conversion step goes with the step it feeds, the next one with a node.
    let mut next_voice = None;
    for info in infos.iter_mut().rev() {
        if info.node.is_some() {
            next_voice = info.voice;
        } else {
            info.voice = next_voice;
        }
    }
    for &(s, d) in &extra_deps {
        if let Some(info) = infos.get_mut(s) {
            info.after.push(d);
        }
    }
    let plan = chains::plan(&infos);
    let waves_of: Vec<usize> = plan
        .chain_of
        .iter()
        .map(|&c| plan.chains.get(c).map_or(0, |ch| ch.wave))
        .collect();
    let n_waves = waves_of.iter().copied().max().map_or(0, |m| m + 1);
    let (own, release_after) = chain_owned(&b, &plan);
    // Lifetimes in waves: from the defining step's wave to the latest wave
    // that reads the buffer (not the latest reading step, which can sit in
    // an earlier wave).
    let def_wave = |l: &Life| Life {
        def: waves_of.get(l.def).copied().unwrap_or(0),
        last: if l.last == usize::MAX {
            usize::MAX
        } else {
            waves_of.get(l.def).copied().unwrap_or(0)
        },
    };
    let mut vsamples: Vec<Life> = b.vsamples.iter().map(def_wave).collect();
    let mut vtriggers: Vec<Life> = b.vtriggers.iter().map(def_wave).collect();
    let mut vmidi: Vec<Life> = b.vmidi.iter().map(def_wave).collect();
    for (s, step) in b.steps.iter().enumerate() {
        let w = waves_of[s];
        let read = |lives: &mut [Life], v: u32| {
            if let Some(l) = lives.get_mut(v as usize)
                && l.last != usize::MAX
            {
                l.last = l.last.max(w);
            }
        };
        for slot in range_of(&b.input_slots, step.inputs) {
            match *slot {
                InputSlot::Float(FloatSrc::Sample(v)) => read(&mut vsamples, v),
                InputSlot::Audio { start, count } => {
                    for c in start..start + count {
                        read(&mut vsamples, b.chan_table[c as usize]);
                    }
                }
                InputSlot::Trigger(TriggerSrc::Wired(v)) => read(&mut vtriggers, v),
                InputSlot::Midi(Some(v)) => read(&mut vmidi, v),
                _ => {}
            }
        }
    }
    let sample_map = assign(&vsamples, n_waves, true, &own);
    let trig_map = assign(&vtriggers, n_waves, false, &[]);
    let midi_map = assign(&vmidi, n_waves, false, &[]);
    let n_samples = sample_map
        .iter()
        .copied()
        .max()
        .map_or(1, |m| m as usize + 1);
    let n_trig = trig_map.iter().copied().max().map_or(0, |m| m as usize + 1);
    let n_midi = midi_map.iter().copied().max().map_or(0, |m| m as usize + 1);

    // Steps in run order, chain by chain, with their outputs laid out in the
    // same order, so a wave's, a chain's, and a step's outputs are each one
    // contiguous range of the work space. `pos` is where each value lands.
    // A chain hands its own buffers from value to value: each takes the
    // one freed last (still warm in cache), and a step's outputs never get
    // a buffer one of its inputs is in.
    let mut lay = Layout::new(&b);
    let mut steps: Vec<Step> = Vec::with_capacity(n_steps);
    let mut chain_base: Vec<At> = Vec::with_capacity(plan.chains.len());
    let mut run_index = vec![0usize; n_steps];
    let mut recycle: Vec<(u32, u32)> = Vec::new();
    let mut stash_of = vec![0u32; b.vsamples.len()];
    let mut stash_at = 0u32;
    for chain in &plan.chains {
        let mut base = lay.at(steps.len());
        base.stash = stash_at as usize;
        chain_base.push(base);
        let k0 = steps.len();
        let (mut free, mut used) = (Vec::<u32>::new(), 0u32);
        for &s in &chain.steps {
            run_index[s] = steps.len();
            let mut st = lay.place(&b, &b.steps[s]);
            let slot = |v: u32| lay.pos_samples[v as usize] - base.samples as u32;
            let a0 = recycle.len() as u32;
            for &v in range_of(&b.out_samples, b.steps[s].o_samples) {
                if own[v as usize] {
                    let j = free.pop().unwrap_or_else(|| {
                        used += 1;
                        used - 1
                    });
                    stash_of[v as usize] = j;
                    recycle.push((j, slot(v)));
                }
            }
            st.acquire = (a0, recycle.len() as u32);
            let r0 = recycle.len() as u32;
            for &v in &release_after[s] {
                let j = stash_of[v as usize];
                recycle.push((j, slot(v)));
                free.push(j);
            }
            st.release = (r0, recycle.len() as u32);
            steps.push(st);
        }
        // The chain's first step holds its part of the stash.
        let end = stash_at + used;
        for (i, st) in steps[k0..].iter_mut().enumerate() {
            st.o_stash = if i == 0 { (stash_at, end) } else { (end, end) };
        }
        stash_at = end;
    }
    // Reads of what an earlier step of the same chain wrote come from the
    // chain's work space; the rest from the pool's physical buffers.
    for (s, &run) in run_index.iter().enumerate() {
        let chain = plan.chain_of[s];
        let base = chain_base.get(chain).copied().unwrap_or_default();
        // Value `v`, written by step `def` and laid out by `pos`, as a LOCAL
        // index when `def` is in this chain.
        let reads_chain = std::cell::Cell::new(false);
        let local = |v: &mut u32, def: Option<usize>, pos: &[u32], base: usize| {
            if def.is_some_and(|d| plan.chain_of.get(d) == Some(&chain))
                && let Some(&p) = pos.get(*v as usize)
                && p != u32::MAX
                && p as usize >= base
            {
                *v = LOCAL | (p - base as u32);
                reads_chain.set(true);
            }
        };
        let sample_def = |v: u32| b.vsamples.get(v as usize).map(|l| l.def);
        let (i0, i1) = (b.steps[s].inputs.0 as usize, b.steps[s].inputs.1 as usize);
        for i in i0..i1 {
            let Some(slot) = b.input_slots.get_mut(i) else {
                continue;
            };
            match slot {
                InputSlot::Float(FloatSrc::Sample(v)) if *v != 0 => {
                    local(v, sample_def(*v), &lay.pos_samples, base.samples);
                }
                InputSlot::Float(FloatSrc::Scalar(k)) => {
                    let def = b.scalar_def.get(*k as usize).copied();
                    local(k, def, &lay.pos_scalars, base.scalars);
                }
                InputSlot::Float(FloatSrc::Int(k)) | InputSlot::Int(IntSrc::Int(k)) => {
                    let def = b.int_def.get(*k as usize).copied();
                    local(k, def, &lay.pos_ints, base.ints);
                }
                InputSlot::Audio { start, count } => {
                    for c in *start..*start + *count {
                        if let Some(v) = b.chan_table.get_mut(c as usize)
                            && *v != 0
                        {
                            local(v, sample_def(*v), &lay.pos_samples, base.samples);
                        }
                    }
                }
                InputSlot::Trigger(TriggerSrc::Wired(v)) => {
                    let def = b.vtriggers.get(*v as usize).map(|l| l.def);
                    local(v, def, &lay.pos_triggers, base.triggers);
                }
                InputSlot::Midi(Some(v)) => {
                    let def = b.vmidi.get(*v as usize).map(|l| l.def);
                    local(v, def, &lay.pos_midi, base.midi);
                }
                InputSlot::Spectrum(Some(k)) => {
                    let def = b.spectra.get(*k as usize).map(|sp| sp.1);
                    local(k, def, &lay.pos_spectra, base.spectra);
                }
                InputSlot::Buffer(Some(k)) => {
                    let def = b.buffer_def.get(*k as usize).copied();
                    local(k, def, &lay.pos_buffers, base.buffers);
                }
                _ => {}
            }
        }
        if let Some(st) = steps.get_mut(run) {
            if let StepKind::FillFromScalar { scalar } = &mut st.kind {
                let def = b.scalar_def.get(*scalar as usize).copied();
                local(scalar, def, &lay.pos_scalars, base.scalars);
            }
            st.reads_chain = reads_chain.get();
        }
    }
    let map_s = |v: u32| {
        if v & LOCAL != 0 {
            v
        } else {
            sample_map.get(v as usize).copied().unwrap_or(0)
        }
    };
    let map_t = |v: u32| {
        if v & LOCAL != 0 {
            v
        } else {
            trig_map.get(v as usize).copied().unwrap_or(0)
        }
    };
    let map_m = |v: u32| {
        if v & LOCAL != 0 {
            v
        } else {
            midi_map.get(v as usize).copied().unwrap_or(0)
        }
    };
    for s in &mut b.input_slots {
        match s {
            InputSlot::Float(FloatSrc::Sample(v)) => *v = map_s(*v),
            InputSlot::Trigger(TriggerSrc::Wired(v)) => *v = map_t(*v),
            InputSlot::Midi(Some(v)) => *v = map_m(*v),
            _ => {}
        }
    }
    for v in &mut b.chan_table {
        *v = map_s(*v);
    }
    for v in &mut lay.out_samples {
        // Chain-owned values have no pool buffer to move.
        *v = if own.get(*v as usize).copied().unwrap_or(false) {
            u32::MAX
        } else {
            map_s(*v)
        };
    }
    for v in &mut lay.out_triggers {
        *v = map_t(*v);
    }
    for v in &mut lay.out_midi {
        *v = map_m(*v);
    }

    // Waves: each wave's parallel chains, then its device-output steps.
    let mut waves: Vec<Wave> = Vec::new();
    let mut chain_ranges: Vec<Range> = Vec::new();
    let mut k = 0u32;
    let mut current = usize::MAX;
    for chain in &plan.chains {
        let (k0, k1) = (k, k + chain.steps.len() as u32);
        k = k1;
        if chain.wave != current {
            current = chain.wave;
            let c = chain_ranges.len() as u32;
            waves.push(Wave {
                start: k0,
                parallel_end: k0,
                end: k0,
                chains: (c, c),
            });
        }
        if let Some(w) = waves.last_mut() {
            w.end = k1;
            if !chain.serial {
                chain_ranges.push((k0, k1));
                w.parallel_end = k1;
                w.chains.1 = chain_ranges.len() as u32;
            }
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
        node_time_acc: atomics(n_time),
    });

    // Work space for the widest wave of each kind.
    let mut room = At::default();
    for w in &waves {
        let size = At::end(&steps, w.end as usize).minus(At::start(&steps, w.start as usize));
        room = room.max(size);
    }
    let work = Work::sized(room, max_block);

    let nodes = nb.nodes;
    report.stats.nodes = nodes.len();
    report.stats.steps = steps.len();
    report.stats.waves = waves.len();
    report.stats.chains = chain_ranges.len();
    report.stats.sample_buffers = n_samples + room.stash;
    report.stats.params = b.params.len();
    report.stats.live_sources = live_sources;
    report.stats.latency = total_latency;
    report.stats.node_latency = flat
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, f)| nb.latency[*i] > 0 && f.kind != COMP_DELAY_KIND)
        .map(|(i, f)| (f.path.clone(), nb.latency[i]))
        .collect();
    report.stats.compensation = comps
        .iter()
        .map(|&(node, input, samples)| {
            (
                flat.nodes[node].path.clone(),
                flat.nodes[node].sig.inputs[input].name.to_string(),
                samples,
            )
        })
        .collect();
    let mut bake_specs: Vec<BakeSpec> = whole.into_values().collect();
    bake_specs.sort_by(|a, b| a.path.cmp(&b.path));
    report.stats.whole_input = bake_specs
        .iter()
        .map(|s| (s.path.clone(), s.state.clone()))
        .collect();
    report
        .diagnostics
        .sort_by_key(|d| std::cmp::Reverse(d.severity));

    let block_seconds = max_block as f64 / sample_rate;
    let level_decay = (-(block_seconds / 0.3)).exp() as f32;
    let n_params = b.params.len();
    let tempo = TempoMap::new(sample_rate, &project.settings.tempo);
    let pool = Pool {
        samples: (0..n_samples).map(|_| vec![0.0; max_block]).collect(),
        scalars: vec![0.0; b.scalar_def.len()],
        ints: vec![0; b.int_def.len()],
        triggers: (0..n_trig)
            .map(|_| Vec::with_capacity(max_block + 64))
            .collect(),
        midi: (0..n_midi)
            .map(|_| Vec::<MidiEvent>::with_capacity(MIDI_CAPACITY))
            .collect(),
        spectra: b
            .spectra
            .iter()
            .map(|(shape, _)| SpectrumBuf::new(*shape, shape.max_frames(max_block)))
            .collect(),
        buffers: vec![None; b.buffer_def.len()],
    };
    let graph = Box::new(CompiledGraph {
        generation: opts.generation,
        sample_rate,
        max_block,
        tempo,
        nodes,
        carry: nb.carry,
        steps,
        waves,
        chains: chain_ranges,
        recycle,
        input_slots: b.input_slots,
        output_slots: b.output_slots,
        out_samples: lay.out_samples,
        out_scalars: lay.out_scalars,
        out_ints: lay.out_ints,
        out_triggers: lay.out_triggers,
        out_midi: lay.out_midi,
        out_spectra: lay.out_spectra,
        out_buffers: lay.out_buffers,
        chan_table: b.chan_table,
        pool,
        params: b.params,
        param_smooth: b.param_smooth,
        active_params: Vec::with_capacity(n_params),
        telemetry: telemetry.clone(),
        socket_tele,
        work,
        level_decay,
        latency: total_latency,
        bindings,
    });
    Compiled {
        graph,
        layout: Arc::new(layout),
        telemetry,
        bindings: binding_values,
        params: b.param_map,
        carry: nb.layout,
        report,
        bakes: bake_specs,
        capture_entry,
    }
}

/// Creates or carries over node instances, tracking indices and latencies.
struct NodeBuilder<'a> {
    nodes: Vec<NodeBox>,
    carry: Vec<(u32, u32)>,
    layout: CarryLayout,
    index: Vec<Option<u32>>,
    latency: Vec<u32>,
    reuse: Option<&'a CarryLayout>,
}

impl NodeBuilder<'_> {
    fn push_fresh(&mut self, f: &FlatNode, fi: usize, node: Box<dyn Node>, config: String) {
        let idx = self.nodes.len() as u32;
        self.index[fi] = Some(idx);
        self.latency[fi] = node.latency();
        self.layout.nodes.insert(
            f.path.clone(),
            CarryKey {
                kind: f.kind.clone(),
                config,
                index: idx,
                latency: self.latency[fi],
                probe: 0,
            },
        );
        self.nodes.push(NodeBox {
            node,
            path: f.path.clone(),
            load_pending: true,
        });
    }

    /// Create node `fi` with `create`, or plan to carry its state over when
    /// its path, kind, and config (plus `extra`) match the previous graph.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        f: &FlatNode,
        fi: usize,
        extra: &str,
        create: impl FnOnce(&Config, &PrepareCtx) -> Box<dyn Node>,
        sample_rate: f64,
        max_block: usize,
        seed: u64,
        resources: &dyn Resources,
    ) {
        let mut config_key = serde_json::to_string(&f.config).unwrap_or_default();
        config_key.push_str(extra);
        let carried = self
            .reuse
            .and_then(|p| p.nodes.get(&f.path))
            .filter(|k| k.kind == f.kind && k.config == config_key)
            .cloned();
        match carried {
            Some(old) => {
                let idx = self.nodes.len() as u32;
                self.index[fi] = Some(idx);
                self.latency[fi] = old.latency;
                self.carry.push((idx, old.index));
                self.layout.nodes.insert(
                    f.path.clone(),
                    CarryKey {
                        kind: f.kind.clone(),
                        config: config_key,
                        index: idx,
                        latency: old.latency,
                        probe: old.probe,
                    },
                );
                self.nodes.push(NodeBox {
                    node: Box::new(Vacant),
                    path: f.path.clone(),
                    load_pending: false,
                });
            }
            None => {
                let key = f.path.to_string();
                let ctx = PrepareCtx {
                    sample_rate,
                    max_block,
                    seed: derive_seed(seed, f.path.stable_hash()),
                    node_key: &key,
                    name: &f.name,
                    resources,
                };
                let node = create(&f.config, &ctx);
                self.push_fresh(f, fi, node, config_key);
            }
        }
    }

    /// Tell a latency probe its path latency, rebuilding it if it was
    /// carried over with a different value.
    #[allow(clippy::too_many_arguments)]
    fn set_probe(
        &mut self,
        f: &FlatNode,
        idx: u32,
        value: u32,
        sample_rate: f64,
        max_block: usize,
        seed: u64,
        resources: &dyn Resources,
    ) {
        let carried = self.carry.iter().position(|&(n, _)| n == idx);
        let previous = self
            .reuse
            .and_then(|p| p.nodes.get(&f.path))
            .map(|k| k.probe);
        match carried {
            Some(pos) if previous != Some(value) => {
                // Rebuild in place: same index, fresh state.
                self.carry.remove(pos);
                let key = f.path.to_string();
                let ctx = PrepareCtx {
                    sample_rate,
                    max_block,
                    seed: derive_seed(seed, f.path.stable_hash()),
                    node_key: &key,
                    name: &f.name,
                    resources,
                };
                let mut node = (f.create)(&f.config, &ctx);
                node.path_latency(value);
                if let Some(nb) = self.nodes.get_mut(idx as usize) {
                    nb.node = node;
                    nb.load_pending = true;
                }
            }
            Some(_) => {}
            None => {
                if let Some(nb) = self.nodes.get_mut(idx as usize) {
                    nb.node.path_latency(value);
                }
            }
        }
        if let Some(k) = self.layout.nodes.get_mut(&f.path) {
            k.probe = value;
        }
    }
}

/// Output storage of one node from its inputs' storage.
fn out_stores(
    f: &FlatNode,
    srcs: &[Option<(usize, usize)>],
    store: &[Vec<OutStore>],
    feedback_read: bool,
) -> Vec<OutStore> {
    let mut poly = Rate::Block;
    let mut follow = 1usize;
    let mut shape: Option<SpectrumShape> = None;
    if !feedback_read {
        for (i, spec) in f.sig.inputs.iter().enumerate() {
            let src = srcs.get(i).copied().flatten().map(|(s, o)| store[s][o]);
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
                SignalType::Spectrum => {
                    if shape.is_none()
                        && let Some(OutStore::Spectrum(s)) = src
                    {
                        shape = Some(s);
                    }
                }
                _ => {}
            }
        }
    }
    f.sig
        .outputs
        .iter()
        .map(|spec| match spec.ty {
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
            SignalType::Spectrum => spec
                .spectrum
                .or(shape)
                .map_or(OutStore::None, OutStore::Spectrum),
            SignalType::Buffer => OutStore::Buffer,
            SignalType::Any => OutStore::None,
        })
        .collect()
}

/// The kind of compensation delay an input needs, if it takes part in
/// latency compensation: audio, sample-rate floats, and events do;
/// block-rate values, ints, spectra, and buffers don't.
fn comp_type(input: SignalType, src: OutStore) -> Option<CompType> {
    match (input, src) {
        (SignalType::Audio, OutStore::Audio(_) | OutStore::FloatSample) => Some(CompType::Audio),
        (SignalType::Float | SignalType::Bool, OutStore::FloatSample | OutStore::Audio(_)) => {
            Some(CompType::Float)
        }
        (SignalType::Trigger, OutStore::Trigger) => Some(CompType::Trigger),
        (SignalType::Midi, OutStore::Midi) => Some(CompType::Midi),
        _ => None,
    }
}

/// Every node `node` depends on through wires (Feedback edges included).
fn upstream(src_of: &[Vec<Option<(usize, usize)>>], node: usize) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<usize> = src_of[node].iter().flatten().map(|&(s, _)| s).collect();
    while let Some(x) = stack.pop() {
        if x != node && seen.insert(x) {
            stack.extend(src_of[x].iter().flatten().map(|&(s, _)| s));
        }
    }
    seen
}

/// Hash of everything a bake of node `i` depends on.
fn bake_key(
    project: &Project,
    flat: &Flat,
    src_of: &[Vec<Option<(usize, usize)>>],
    i: usize,
    up: &BTreeSet<usize>,
    opts: &CompileOptions,
    reads_input: bool,
) -> u64 {
    let mut h = Fnv64::new();
    let mut members: Vec<usize> = up.iter().copied().chain([i]).collect();
    members.sort_by(|a, b| flat.nodes[*a].path.cmp(&flat.nodes[*b].path));
    for &m in &members {
        let f = &flat.nodes[m];
        h.write(f.path.to_string().as_bytes());
        h.write(f.kind.as_bytes());
        h.write(
            serde_json::to_string(&f.config)
                .unwrap_or_default()
                .as_bytes(),
        );
        h.write(
            serde_json::to_string(&f.params)
                .unwrap_or_default()
                .as_bytes(),
        );
        h.write(&[u8::from(f.muted)]);
        for src in &src_of[m] {
            match src {
                Some((s, o)) => {
                    h.write(flat.nodes[*s].path.to_string().as_bytes());
                    h.write(&(*o as u64).to_le_bytes());
                }
                None => h.write(b"-"),
            }
        }
    }
    h.write(&opts.sample_rate.to_bits().to_le_bytes());
    h.write(&opts.bake_frames.to_le_bytes());
    h.write(&(opts.bake_block as u64).to_le_bytes());
    h.write(&opts.resources_generation.to_le_bytes());
    h.write(&project.settings.seed.to_le_bytes());
    h.write(
        serde_json::to_string(&project.settings.tempo)
            .unwrap_or_default()
            .as_bytes(),
    );
    if reads_input {
        h.write(&opts.input_hash.to_le_bytes());
    }
    h.finish()
}

/// Sample buffers that only the chain writing them reads (outside
/// device-output steps): those live in the chain's own recycled buffers
/// instead of the pool. Also, for every step, the owned values whose last
/// reader it is (or that it writes and nothing reads), to free after it.
fn chain_owned(b: &Builder, plan: &chains::Plan) -> (Vec<bool>, Vec<Vec<u32>>) {
    let n = b.vsamples.len();
    let mut place = vec![0usize; b.steps.len()];
    for ch in &plan.chains {
        for (i, &s) in ch.steps.iter().enumerate() {
            place[s] = i;
        }
    }
    let chain_of = |s: usize| plan.chain_of.get(s).copied();
    let mut own = vec![false; n];
    let mut last = vec![0usize; n];
    for (v, l) in b.vsamples.iter().enumerate().skip(1) {
        own[v] = l.last != usize::MAX
            && chain_of(l.def)
                .and_then(|c| plan.chains.get(c))
                .is_some_and(|c| !c.serial);
        last[v] = place.get(l.def).copied().unwrap_or(0);
    }
    for (s, step) in b.steps.iter().enumerate() {
        let mut read = |v: u32| {
            let Some(l) = b.vsamples.get(v as usize) else {
                return;
            };
            if v == 0 {
                return;
            }
            if chain_of(s) == chain_of(l.def) {
                last[v as usize] = last[v as usize].max(place[s]);
            } else {
                own[v as usize] = false;
            }
        };
        for slot in range_of(&b.input_slots, step.inputs) {
            match *slot {
                InputSlot::Float(FloatSrc::Sample(v)) => read(v),
                InputSlot::Audio { start, count } => {
                    for c in start..start + count {
                        if let Some(&v) = b.chan_table.get(c as usize) {
                            read(v);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut release = vec![Vec::new(); b.steps.len()];
    for v in (1..n).filter(|&v| own[v]) {
        let step = chain_of(b.vsamples[v].def)
            .and_then(|c| plan.chains.get(c))
            .and_then(|c| c.steps.get(last[v]));
        if let Some(&s) = step {
            release[s].push(v as u32);
        }
    }
    (own, release)
}

/// The steps whose outputs each step reads (the chain planner's input).
fn step_reads(b: &Builder) -> Vec<Vec<usize>> {
    let sample_def = |v: u32| b.vsamples.get(v as usize).map(|l| l.def);
    b.steps
        .iter()
        .map(|step| {
            let mut d = Vec::new();
            if let StepKind::FillFromScalar { scalar } = step.kind
                && let Some(&def) = b.scalar_def.get(scalar as usize)
            {
                d.push(def);
            }
            for slot in range_of(&b.input_slots, step.inputs) {
                let def = match *slot {
                    InputSlot::Float(FloatSrc::Sample(v)) if v != 0 => sample_def(v),
                    InputSlot::Float(FloatSrc::Scalar(k)) => b.scalar_def.get(k as usize).copied(),
                    InputSlot::Float(FloatSrc::Int(k)) | InputSlot::Int(IntSrc::Int(k)) => {
                        b.int_def.get(k as usize).copied()
                    }
                    InputSlot::Audio { start, count } => {
                        for c in start..start + count {
                            if let Some(&v) = b.chan_table.get(c as usize)
                                && v != 0
                            {
                                d.extend(sample_def(v));
                            }
                        }
                        None
                    }
                    InputSlot::Trigger(TriggerSrc::Wired(v)) => {
                        b.vtriggers.get(v as usize).map(|l| l.def)
                    }
                    InputSlot::Midi(Some(v)) => b.vmidi.get(v as usize).map(|l| l.def),
                    InputSlot::Spectrum(Some(k)) => b.spectra.get(k as usize).map(|sp| sp.1),
                    InputSlot::Buffer(Some(k)) => b.buffer_def.get(k as usize).copied(),
                    _ => None,
                };
                d.extend(def);
            }
            d
        })
        .collect()
}

/// Steps' outputs laid out in run order (see `graph::At`), and where each
/// virtual value landed (`u32::MAX`: nowhere).
struct Layout {
    out_samples: Vec<u32>,
    out_scalars: Vec<u32>,
    out_ints: Vec<u32>,
    out_triggers: Vec<u32>,
    out_midi: Vec<u32>,
    out_spectra: Vec<u32>,
    out_buffers: Vec<u32>,
    pos_samples: Vec<u32>,
    pos_scalars: Vec<u32>,
    pos_ints: Vec<u32>,
    pos_triggers: Vec<u32>,
    pos_midi: Vec<u32>,
    pos_spectra: Vec<u32>,
    pos_buffers: Vec<u32>,
}

impl Layout {
    fn new(b: &Builder) -> Layout {
        Layout {
            out_samples: Vec::with_capacity(b.out_samples.len()),
            out_scalars: Vec::with_capacity(b.out_scalars.len()),
            out_ints: Vec::with_capacity(b.out_ints.len()),
            out_triggers: Vec::with_capacity(b.out_triggers.len()),
            out_midi: Vec::with_capacity(b.out_midi.len()),
            out_spectra: Vec::with_capacity(b.out_spectra.len()),
            out_buffers: Vec::with_capacity(b.out_buffers.len()),
            pos_samples: vec![u32::MAX; b.vsamples.len()],
            pos_scalars: vec![u32::MAX; b.scalar_def.len()],
            pos_ints: vec![u32::MAX; b.int_def.len()],
            pos_triggers: vec![u32::MAX; b.vtriggers.len()],
            pos_midi: vec![u32::MAX; b.vmidi.len()],
            pos_spectra: vec![u32::MAX; b.spectra.len()],
            pos_buffers: vec![u32::MAX; b.buffer_def.len()],
        }
    }

    /// Where the next step placed will start (the stash is laid out by chain).
    fn at(&self, step: usize) -> At {
        At {
            step,
            samples: self.out_samples.len(),
            scalars: self.out_scalars.len(),
            ints: self.out_ints.len(),
            triggers: self.out_triggers.len(),
            midi: self.out_midi.len(),
            spectra: self.out_spectra.len(),
            buffers: self.out_buffers.len(),
            stash: 0,
        }
    }

    /// Append `step`'s outputs and return it with its new ranges.
    fn place(&mut self, b: &Builder, step: &Step) -> Step {
        fn put(src: &[u32], r: Range, dst: &mut Vec<u32>, pos: &mut [u32]) -> Range {
            let start = dst.len() as u32;
            for &v in range_of(src, r) {
                if let Some(p) = pos.get_mut(v as usize) {
                    *p = dst.len() as u32;
                }
                dst.push(v);
            }
            (start, dst.len() as u32)
        }
        let mut s = step.clone();
        s.o_samples = put(
            &b.out_samples,
            s.o_samples,
            &mut self.out_samples,
            &mut self.pos_samples,
        );
        s.o_scalars = put(
            &b.out_scalars,
            s.o_scalars,
            &mut self.out_scalars,
            &mut self.pos_scalars,
        );
        s.o_ints = put(
            &b.out_ints,
            s.o_ints,
            &mut self.out_ints,
            &mut self.pos_ints,
        );
        s.o_triggers = put(
            &b.out_triggers,
            s.o_triggers,
            &mut self.out_triggers,
            &mut self.pos_triggers,
        );
        s.o_midi = put(
            &b.out_midi,
            s.o_midi,
            &mut self.out_midi,
            &mut self.pos_midi,
        );
        s.o_spectra = put(
            &b.out_spectra,
            s.o_spectra,
            &mut self.out_spectra,
            &mut self.pos_spectra,
        );
        s.o_buffers = put(
            &b.out_buffers,
            s.o_buffers,
            &mut self.out_buffers,
            &mut self.pos_buffers,
        );
        s
    }
}

fn range_of<T>(v: &[T], r: Range) -> &[T] {
    v.get(r.0 as usize..r.1 as usize).unwrap_or(&[])
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
                    let local = b.out_samples.len() as u32;
                    b.out_samples.push(v);
                    let mut step = Builder::empty_step(StepKind::FillFromScalar { scalar: *k });
                    step.o_samples = (local, local + 1);
                    b.steps.push(step);
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
        SignalType::Spectrum => match a {
            Alloc::Spectrum(k) => InputSlot::Spectrum(Some(*k)),
            _ => InputSlot::Spectrum(None),
        },
        SignalType::Buffer => match a {
            Alloc::Buffer(k) => InputSlot::Buffer(Some(*k)),
            _ => InputSlot::Buffer(None),
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

fn unwired_input(b: &mut Builder, f: &FlatNode, spec: &SocketSpec, smooth_len: u32) -> InputSlot {
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
        SignalType::Spectrum => InputSlot::Spectrum(None),
        SignalType::Buffer => InputSlot::Buffer(None),
        SignalType::Any => InputSlot::None,
    }
}

fn bypass_for(f: &FlatNode) -> Bypass {
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

/// Assign physical buffers to virtual ones by lifetime (in waves). Outputs
/// of a wave are allocated before that wave's inputs are freed, so no step
/// writes a buffer another step of its wave reads. For sample buffers,
/// physical 0 is reserved silence. Values marked in `own` live in their
/// chain's buffers and get none.
fn assign(lives: &[Life], n_waves: usize, reserve_zero: bool, own: &[bool]) -> Vec<u32> {
    let mut map = vec![0u32; lives.len()];
    let first = usize::from(reserve_zero);
    let mut by_def: Vec<Vec<usize>> = vec![Vec::new(); n_waves + 1];
    let mut by_last: Vec<Vec<usize>> = vec![Vec::new(); n_waves + 1];
    for (v, l) in lives.iter().enumerate().skip(first) {
        if own.get(v).copied().unwrap_or(false) {
            continue;
        }
        by_def[l.def.min(n_waves)].push(v);
        by_last[l.last.min(n_waves)].push(v);
    }
    let mut free: Vec<u32> = Vec::new();
    let mut next = first as u32;
    for step in 0..=n_waves {
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
    fn lifetimes_reuse_buffers_without_aliasing_a_wave() {
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
        let m = assign(&lives, 4, true, &[]);
        assert_eq!(m[0], 0);
        assert_ne!(m[1], m[2]);
        assert_eq!(m[3], m[1]);
        assert!(m.iter().skip(1).all(|&p| p != 0));
    }
}
