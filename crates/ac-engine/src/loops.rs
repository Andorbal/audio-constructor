//! Single-sample feedback (ARCHITECTURE §7 step 3, DECISIONS D140).
//!
//! A `Feedback` node whose `delay` config is `"sample"` closes its loop with
//! one sample of delay instead of one block. The compiler finds the
//! strongly connected component of the wire graph that holds it (the loops
//! block-mode Feedback nodes close are already cut, so they stay outside),
//! and compiles that component to a per-sample sub-schedule: one *loop
//! step* in the outer schedule whose inputs are every wire into the
//! component and whose outputs are every member output read outside it.
//!
//! Each block, the loop step runs its members one sample at a time, in the
//! component's own topological order (each Feedback member split into its
//! read, first, and its write, last, as in the outer schedule), against a
//! one-frame store of its own, so a Feedback member's read and write are one
//! sample apart. Everything else keeps running block by block, and the
//! outer scheduler sees one ordinary step: chains, waves, buffer reuse, and
//! the worker pool need nothing new, the whole loop runs on one thread, and
//! output is bit-identical on any number of threads. Graphs without a
//! sample-mode Feedback compile and run exactly as before.
//!
//! Nodes that can't run one sample at a time (FFT-frame nodes, whole-input
//! nodes, device nodes) make the compiler report the loop and fall back to
//! one block of delay for it, which is what the schedule already does.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::Ordering;

use ac_core::{
    AutoFlags, BufferRef, EngineInfo, HostAccess, InputSlot, Inputs, Io, Local, MidiEvent, Node,
    OutputSlot, Outputs, ParamSlot, Phase, ProcessCtx, SignalType, TelemetryAccess, TextBuf,
    TransportView,
};

use crate::compile::Item;
use crate::flatten::{Flat, FlatNode};
use crate::graph::{Bypass, Range, Vacant, bypass};
use crate::report::{CompileReport, Severity};
use crate::telemetry::TelemetryStore;

/// The Feedback config key that picks the delay, and its single-sample value.
pub const DELAY_KEY: &str = "delay";
pub const DELAY_SAMPLE: &str = "sample";

/// Triggers an inner output can hold in one frame (an output fires at most
/// once per frame, D63; the rest is headroom).
const TRIGGER_ROOM: usize = 4;

/// Whether `f` is a Feedback node set to one sample of delay.
pub(crate) fn per_sample(f: &FlatNode) -> bool {
    f.meta.feedback && f.config.get(DELAY_KEY).and_then(|v| v.as_str()) == Some(DELAY_SAMPLE)
}

/// Why a node can't run inside a single-sample loop, if it can't.
fn cannot(f: &FlatNode) -> Option<&'static str> {
    if f.meta.whole_input {
        Some("it needs its whole input before it plays")
    } else if f.meta.host_output {
        Some("it writes to a device")
    } else if f.meta.live {
        Some("it reads a device")
    } else if f
        .sig
        .inputs
        .iter()
        .chain(&f.sig.outputs)
        .any(|s| s.ty == SignalType::Spectrum)
    {
        Some("it works in FFT frames")
    } else {
        None
    }
}

/// One single-sample loop: the nodes of a strongly connected component
/// holding a sample-mode Feedback node.
#[derive(Clone, Debug, Default)]
pub(crate) struct Region {
    /// Flat node indices, ascending.
    pub members: Vec<usize>,
    /// The members' schedule items in run order (filled by [`contiguous`]).
    pub items: Vec<Item>,
}

/// Find every single-sample loop among the scheduled nodes. `src_of` is the
/// wiring after cycles without a Feedback node were cut. Loops holding a
/// node that can't run sample by sample are reported and left out, so
/// their Feedback nodes delay by a block (the schedule already treats every
/// Feedback node as a cut).
pub(crate) fn find(
    flat: &Flat,
    src_of: &[Vec<Option<(usize, usize)>>],
    order: &[Item],
    report: &mut CompileReport,
) -> Vec<Region> {
    let n = flat.nodes.len();
    let mut scheduled = vec![false; n];
    for it in order {
        if let Some(s) = scheduled.get_mut(it.node()) {
            *s = true;
        }
    }
    let roots: Vec<usize> = (0..n)
        .filter(|&i| scheduled[i] && per_sample(&flat.nodes[i]))
        .collect();
    if roots.is_empty() {
        return Vec::new();
    }
    // Wires as edges source → consumer. A block-mode Feedback node already
    // cuts its loop, so edges out of it don't count.
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for d in (0..n).filter(|&d| scheduled[d]) {
        for &(s, _) in src_of[d].iter().flatten() {
            let f = &flat.nodes[s];
            if scheduled[s] && !(f.meta.feedback && !per_sample(f)) {
                succ[s].push(d);
            }
        }
    }
    let mut regions = Vec::new();
    for mut members in strongly_connected(&succ, &roots) {
        if !members.iter().any(|&m| per_sample(&flat.nodes[m])) {
            continue;
        }
        members.sort_unstable();
        let feedbacks: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&m| per_sample(&flat.nodes[m]))
            .collect();
        let fb_names = names(flat, &feedbacks);
        let refused: Vec<(usize, &'static str)> = members
            .iter()
            .filter_map(|&m| cannot(&flat.nodes[m]).map(|why| (m, why)))
            .collect();
        if refused.is_empty() {
            for &fb in &feedbacks {
                report.push(
                    Severity::Info,
                    Some(&flat.nodes[fb].path),
                    None,
                    format!(
                        "closes a single-sample loop: {} nodes run one sample at a time",
                        members.len()
                    ),
                );
            }
            regions.push(Region {
                members,
                items: Vec::new(),
            });
            continue;
        }
        let culprits: Vec<String> = refused
            .iter()
            .map(|&(m, why)| format!("{} ({why})", flat.nodes[m].name))
            .collect();
        for &(m, why) in &refused {
            report.warning(
                Some(&flat.nodes[m].path),
                None,
                format!(
                    "can't run one sample at a time ({why}), so the loop through {fb_names} \
                     delays by one block instead of one sample"
                ),
            );
        }
        for &fb in &feedbacks {
            report.warning(
                Some(&flat.nodes[fb].path),
                None,
                format!(
                    "delays by one block, not one sample: {} can't run one sample at a time",
                    culprits.join(", ")
                ),
            );
        }
    }
    regions
}

fn names(flat: &Flat, nodes: &[usize]) -> String {
    let v: Vec<String> = nodes
        .iter()
        .map(|&i| format!("{} ({})", flat.nodes[i].name, flat.nodes[i].path))
        .collect();
    v.join(", ")
}

/// The strongly connected components reachable from `roots` (Tarjan 1972,
/// "Depth-first search and linear graph algorithms", iteratively, so a long
/// chain can't overflow the stack).
fn strongly_connected(succ: &[Vec<usize>], roots: &[usize]) -> Vec<Vec<usize>> {
    const NONE: usize = usize::MAX;
    let n = succ.len();
    let mut index = vec![NONE; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    let mut next = 0usize;
    for &root in roots {
        if index[root] != NONE {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(top) = call.len().checked_sub(1) {
            let (v, i) = call[top];
            if let Some(&w) = succ[v].get(i) {
                call[top].1 += 1;
                if index[w] == NONE {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut comp = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(comp);
            }
        }
    }
    out
}

/// Reorder a schedule so each region's items run back to back, keeping
/// everything else in its order where it can: a topological sort of the
/// schedule with each region taken as one unit (they can't form a cycle,
/// since a region is a whole strongly connected component). Fills each
/// region's `items` with its members' items in run order.
pub(crate) fn contiguous(
    order: &[Item],
    regions: &mut [Region],
    region_of: &[Option<usize>],
    flat: &Flat,
    src_of: &[Vec<Option<(usize, usize)>>],
) -> Vec<Item> {
    let r = regions.len();
    let pos: HashMap<Item, usize> = order.iter().enumerate().map(|(k, it)| (*it, k)).collect();
    let unit = |k: usize| {
        region_of
            .get(order[k].node())
            .copied()
            .flatten()
            .unwrap_or(r + k)
    };
    let producer = |s: usize| {
        if flat.nodes[s].meta.feedback {
            Item::FbRead(s)
        } else {
            Item::Main(s)
        }
    };
    let mut first: HashMap<usize, usize> = HashMap::new();
    let mut items_of: HashMap<usize, Vec<Item>> = HashMap::new();
    for (k, it) in order.iter().enumerate() {
        let u = unit(k);
        first.entry(u).or_insert(k);
        items_of.entry(u).or_default().push(*it);
    }
    let mut edges: HashSet<(usize, usize)> = HashSet::new();
    for (k, it) in order.iter().enumerate() {
        if matches!(it, Item::FbRead(_)) {
            continue;
        }
        let u = unit(k);
        let node = it.node();
        let mut deps: Vec<Item> = src_of[node]
            .iter()
            .flatten()
            .map(|&(s, _)| producer(s))
            .collect();
        if let Item::FbWrite(n) = it {
            deps.push(Item::FbRead(*n));
        }
        for d in deps {
            if let Some(&dk) = pos.get(&d) {
                let du = unit(dk);
                if du != u {
                    edges.insert((du, u));
                }
            }
        }
    }
    let mut indeg: HashMap<usize, usize> = first.keys().map(|&u| (u, 0)).collect();
    let mut users: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(a, b) in &edges {
        *indeg.entry(b).or_insert(0) += 1;
        users.entry(a).or_default().push(b);
    }
    let mut ready: BTreeSet<(usize, usize)> = indeg
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&u, _)| (first[&u], u))
        .collect();
    let mut out = Vec::with_capacity(order.len());
    while let Some((_, u)) = ready.pop_first() {
        if let Some(items) = items_of.get(&u) {
            out.extend(items.iter().copied());
            if let Some(region) = regions.get_mut(u).filter(|_| u < r) {
                region.items = items.clone();
            }
        }
        for &v in users.get(&u).map_or(&[][..], Vec::as_slice) {
            if let Some(d) = indeg.get_mut(&v) {
                *d -= 1;
                if *d == 0 {
                    ready.insert((first[&v], v));
                }
            }
        }
    }
    if out.len() != order.len() {
        // A cycle between units can't happen (see above); keep the original
        // order rather than lose steps, and run no loop out of order.
        for region in regions.iter_mut() {
            region.items.clear();
        }
        return order.to_vec();
    }
    out
}

/// Where a value of the loop's one-frame store lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inner {
    /// `count` consecutive one-frame sample buffers from `at` (audio
    /// channels, or one for a float).
    Samples {
        at: u32,
        count: u32,
    },
    Int(u32),
    Trigger(u32),
    Midi(u32),
    Buffer(u32),
    Text(u32),
}

/// Copies a loop-step input into the store at each sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundaryIn {
    Audio {
        input: u32,
        channels: u32,
        at: u32,
    },
    Float {
        input: u32,
        at: u32,
    },
    /// Block-rate values: copied at the first sample.
    Int {
        input: u32,
        at: u32,
    },
    Buffer {
        input: u32,
        at: u32,
    },
    Text {
        input: u32,
        at: u32,
    },
    /// Events: the ones at this sample, moved to frame 0. `cursor` keeps the
    /// read position through the block.
    Trigger {
        input: u32,
        at: u32,
        cursor: u32,
    },
    Midi {
        input: u32,
        at: u32,
        cursor: u32,
    },
}

/// Copies a member output from the store to a loop-step output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundaryOut {
    pub output: u32,
    pub src: Inner,
}

/// One member's step inside the loop: which member, which part of a
/// Feedback node, and its slots and outputs (ranges into the store).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InnerStep {
    pub member: u32,
    pub phase: Phase,
    pub muted: bool,
    pub bypass: Bypass,
    pub inputs: Range,
    pub outputs: Range,
    pub samples: Range,
    pub ints: Range,
    pub triggers: Range,
    pub midi: Range,
    pub buffers: Range,
    pub texts: Range,
}

/// A member's telemetry slots in the graph's store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberTele {
    pub values: Range,
    pub scope: Range,
    pub scope_pos: Option<u32>,
    pub time: Option<u32>,
}

/// Per-socket telemetry of a member output, gathered over the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SocketAcc {
    pub slot: u32,
    pub src: Inner,
    level: f32,
    value: f32,
    events: u32,
}

impl SocketAcc {
    pub fn new(slot: u32, src: Inner) -> SocketAcc {
        SocketAcc {
            slot,
            src,
            level: 0.0,
            value: 0.0,
            events: 0,
        }
    }
}

/// A single-sample loop, compiled: what its loop step runs each block.
///
/// The members' nodes live in the graph's node list like every other node
/// (so state carries over across recompiles and resets reach them); the
/// scheduler moves them in here for the loop step and back after it.
pub(crate) struct LoopRunner {
    pub steps: Vec<InnerStep>,
    /// Inner slots read the store through [`ac_core::LOCAL`] indices.
    pub input_slots: Vec<InputSlot>,
    pub output_slots: Vec<OutputSlot>,
    pub chan_table: Vec<u32>,
    pub boundary_in: Vec<BoundaryIn>,
    pub boundary_out: Vec<BoundaryOut>,
    /// The graph's param slot behind each inner param.
    pub param_src: Vec<u32>,
    /// Inner params as they stand at the current sample.
    pub params: Vec<ParamSlot>,
    /// The one-frame store: boundary inputs first, then each step's outputs
    /// in run order, so a step only reads below its own outputs.
    pub samples: Vec<Vec<f32>>,
    pub ints: Vec<i32>,
    pub triggers: Vec<Vec<u32>>,
    pub midi: Vec<Vec<MidiEvent>>,
    pub buffers: Vec<BufferRef>,
    pub texts: Vec<TextBuf>,
    pub cursors: Vec<usize>,
    /// Graph node index of each member, and its state while the loop runs.
    pub member_node: Vec<u32>,
    pub members: Vec<Box<dyn Node>>,
    pub member_load: Vec<bool>,
    pub member_tele: Vec<MemberTele>,
    /// Each member's outputs in the store (by output index).
    pub member_outputs: Vec<Vec<Option<Inner>>>,
    pub sockets: Vec<SocketAcc>,
}

/// What a loop step shares with the rest of the block.
pub(crate) struct LoopCtx<'a> {
    pub sample_rate: f64,
    pub transport: TransportView<'a>,
    pub engine: EngineInfo,
    pub telemetry: &'a TelemetryStore,
    pub level_decay: f32,
    pub clock: Option<fn() -> u64>,
}

impl LoopRunner {
    pub fn new(member_node: Vec<u32>) -> LoopRunner {
        let n = member_node.len();
        LoopRunner {
            steps: Vec::new(),
            input_slots: Vec::new(),
            output_slots: Vec::new(),
            chan_table: Vec::new(),
            boundary_in: Vec::new(),
            boundary_out: Vec::new(),
            param_src: Vec::new(),
            params: Vec::new(),
            samples: Vec::new(),
            ints: Vec::new(),
            triggers: Vec::new(),
            midi: Vec::new(),
            buffers: Vec::new(),
            texts: Vec::new(),
            cursors: Vec::new(),
            members: (0..n).map(|_| Box::new(Vacant) as Box<dyn Node>).collect(),
            member_load: vec![false; n],
            member_tele: vec![MemberTele::default(); n],
            member_outputs: vec![Vec::new(); n],
            member_node,
            sockets: Vec::new(),
        }
    }

    /// Size the store once its layout is known (off the audio thread).
    pub fn allocate(&mut self, samples: usize, ints: usize, triggers: usize, midi: usize) {
        self.samples = (0..samples).map(|_| vec![0.0; 1]).collect();
        self.ints = vec![0; ints];
        self.triggers = (0..triggers)
            .map(|_| Vec::with_capacity(TRIGGER_ROOM))
            .collect();
        self.midi = (0..midi)
            .map(|_| Vec::with_capacity(ac_core::MIDI_CAPACITY))
            .collect();
        self.params = vec![ParamSlot::default(); self.param_src.len()];
    }

    /// Move the members' nodes in from the graph's node list.
    pub fn take_members(&mut self, nodes: &mut [crate::graph::NodeBox]) {
        for (j, &n) in self.member_node.iter().enumerate() {
            if let (Some(nb), Some(slot), Some(load)) = (
                nodes.get_mut(n as usize),
                self.members.get_mut(j),
                self.member_load.get_mut(j),
            ) {
                std::mem::swap(&mut nb.node, slot);
                *load = nb.load_pending;
            }
        }
    }

    /// Move them back; they have run, so *On load* triggers are spent.
    pub fn give_members(&mut self, nodes: &mut [crate::graph::NodeBox]) {
        for (j, &n) in self.member_node.iter().enumerate() {
            if let (Some(nb), Some(slot)) = (nodes.get_mut(n as usize), self.members.get_mut(j)) {
                std::mem::swap(&mut nb.node, slot);
                nb.load_pending = false;
            }
        }
    }

    /// Run the loop for one block: every member, one sample at a time.
    /// `outer` is the loop step's own view of the block (its inputs from the
    /// rest of the graph, its outputs to it). Real-time safe.
    pub fn run(&mut self, ctx: &LoopCtx<'_>, outer: &mut Io<'_>) {
        let frames = outer.frames();
        let t0 = ctx.clock.map(|c| c());
        let auto = outer.inputs.auto;
        self.cursors.fill(0);
        for s in &mut self.sockets {
            s.level = 0.0;
            s.events = 0;
        }
        let graph_params = outer.inputs.params;
        let param = |p: u32| graph_params.get(p as usize).copied().unwrap_or_default();
        let varying = self.param_src.iter().any(|&p| {
            let s = param(p);
            s.remaining > 0 || s.fired
        });
        for i in 0..frames {
            if i == 0 || varying {
                for (k, &p) in self.param_src.iter().enumerate() {
                    if let Some(q) = self.params.get_mut(k) {
                        *q = shift(param(p), i);
                    }
                }
            }
            self.fill_boundary(&outer.inputs, i, frames);
            let first = i == 0;
            let transport = TransportView {
                position: if ctx.transport.playing {
                    ctx.transport.position + i as u64
                } else {
                    ctx.transport.position
                },
                started: ctx.transport.started && first,
                stopped: ctx.transport.stopped && first,
                ..ctx.transport
            };
            let inner_ctx = InnerCtx {
                sample_rate: ctx.sample_rate,
                transport,
                engine: ctx.engine,
                telemetry: ctx.telemetry,
                auto: AutoFlags {
                    play: auto.play && first,
                    stop: auto.stop && first,
                    load: false,
                },
                first,
            };
            for k in 0..self.steps.len() {
                self.run_step(k, &inner_ctx);
            }
            self.write_boundary(&mut outer.outputs, i, frames);
            if !self.sockets.is_empty() {
                self.measure();
            }
        }
        self.member_load.fill(false);
        self.publish_sockets(ctx);
        if let (Some(t0), Some(c)) = (t0, ctx.clock) {
            // The loop's time, shared among its members (they interleave
            // sample by sample, so timing each would cost more than they do).
            let n = self.member_tele.iter().filter(|t| t.time.is_some()).count();
            let each = c().saturating_sub(t0) / n.max(1) as u64;
            for t in &self.member_tele {
                if let Some(a) = t
                    .time
                    .and_then(|slot| ctx.telemetry.node_time_acc.get(slot as usize))
                {
                    a.fetch_add(each.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
                }
            }
        }
    }

    /// Copy the loop step's inputs at sample `i` into the store.
    fn fill_boundary(&mut self, inputs: &Inputs<'_>, i: usize, frames: usize) {
        let last = i + 1 == frames;
        for b in &self.boundary_in {
            match *b {
                BoundaryIn::Audio {
                    input,
                    channels,
                    at,
                } => {
                    let a = inputs.audio(input as usize);
                    for c in 0..channels as usize {
                        if let Some(x) = self.samples.get_mut(at as usize + c) {
                            x[0] = a.channel(c).get(i).copied().unwrap_or(0.0);
                        }
                    }
                }
                BoundaryIn::Float { input, at } => {
                    if let Some(x) = self.samples.get_mut(at as usize) {
                        x[0] = inputs.float(input as usize).at(i);
                    }
                }
                BoundaryIn::Int { input, at } => {
                    if i == 0
                        && let Some(x) = self.ints.get_mut(at as usize)
                    {
                        *x = inputs.int(input as usize);
                    }
                }
                BoundaryIn::Buffer { input, at } => {
                    if i == 0
                        && let Some(x) = self.buffers.get_mut(at as usize)
                    {
                        let next = inputs.buffer(input as usize);
                        let same = match (x.as_ref(), next) {
                            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                            (None, None) => true,
                            _ => false,
                        };
                        if !same {
                            *x = next.cloned();
                        }
                    }
                }
                BoundaryIn::Text { input, at } => {
                    if i == 0
                        && let Some(x) = self.texts.get_mut(at as usize)
                    {
                        *x = *inputs.text(input as usize);
                    }
                }
                BoundaryIn::Trigger { input, at, cursor } => {
                    let (Some(buf), Some(c)) = (
                        self.triggers.get_mut(at as usize),
                        self.cursors.get_mut(cursor as usize),
                    ) else {
                        continue;
                    };
                    buf.clear();
                    let t = inputs.triggers(input as usize);
                    let mut fired = false;
                    while let Some(&o) = t.get(*c) {
                        if o as usize > i && !last {
                            break;
                        }
                        fired = true;
                        *c += 1;
                    }
                    if fired {
                        buf.push(0);
                    }
                }
                BoundaryIn::Midi { input, at, cursor } => {
                    let (Some(buf), Some(c)) = (
                        self.midi.get_mut(at as usize),
                        self.cursors.get_mut(cursor as usize),
                    ) else {
                        continue;
                    };
                    buf.clear();
                    let m = inputs.midi(input as usize);
                    while let Some(e) = m.get(*c) {
                        if e.offset as usize > i && !last {
                            break;
                        }
                        if buf.len() < buf.capacity() {
                            buf.push(MidiEvent::new(0, e.msg));
                        }
                        *c += 1;
                    }
                }
            }
        }
    }

    /// Copy the members' outputs at sample `i` to the loop step's outputs.
    fn write_boundary(&self, outputs: &mut Outputs<'_>, i: usize, frames: usize) {
        let last = i + 1 == frames;
        for b in &self.boundary_out {
            let o = b.output as usize;
            match b.src {
                Inner::Samples { at, count } => {
                    let count = count.max(1) as usize;
                    let read = |c: usize| {
                        self.samples
                            .get(at as usize + c % count)
                            .map_or(0.0, |x| x[0])
                    };
                    if matches!(outputs.slots.get(o), Some(OutputSlot::Audio { .. })) {
                        let mut a = outputs.audio(o);
                        for c in 0..a.channels() {
                            if let Some(d) = a.channel(c).get_mut(i) {
                                *d = read(c);
                            }
                        }
                    } else {
                        outputs.float(o).set(i, read(0));
                    }
                }
                Inner::Int(at) => {
                    outputs.set_int(o, self.ints.get(at as usize).copied().unwrap_or(0));
                }
                Inner::Trigger(at) => {
                    if self
                        .triggers
                        .get(at as usize)
                        .is_some_and(|t| !t.is_empty())
                    {
                        outputs.trigger(o).push(i as u32);
                    }
                }
                Inner::Midi(at) => {
                    if let Some(events) = self.midi.get(at as usize) {
                        let mut out = outputs.midi(o);
                        for e in events {
                            out.push(MidiEvent::new(i as u32, e.msg));
                        }
                    }
                }
                Inner::Buffer(at) => {
                    if last {
                        outputs
                            .set_buffer(o, self.buffers.get(at as usize).and_then(Option::as_ref));
                    }
                }
                Inner::Text(at) => {
                    if last
                        && let (Some(t), Some(src)) = (outputs.text(o), self.texts.get(at as usize))
                    {
                        *t = *src;
                    }
                }
            }
        }
    }

    /// Run inner step `k` for one sample.
    fn run_step(&mut self, k: usize, ctx: &InnerCtx<'_>) {
        let LoopRunner {
            steps,
            input_slots,
            output_slots,
            chan_table,
            params,
            samples,
            ints,
            triggers,
            midi,
            buffers,
            texts,
            members,
            member_load,
            member_tele,
            ..
        } = self;
        let Some(st) = steps.get(k) else {
            return;
        };
        let m = st.member as usize;
        let Some(node) = members.get_mut(m) else {
            return;
        };
        // A step's outputs start silent, as in the outer schedule; values
        // (ints, buffers, text) hold until written.
        let (s_before, s_mine) = cut(samples, st.samples);
        for b in s_mine.iter_mut() {
            if let Some(x) = b.first_mut() {
                *x = 0.0;
            }
        }
        let (t_before, t_mine) = cut(triggers, st.triggers);
        for t in t_mine.iter_mut() {
            t.clear();
        }
        let (m_before, m_mine) = cut(midi, st.midi);
        for e in m_mine.iter_mut() {
            e.clear();
        }
        let (i_before, i_mine) = cut(ints, st.ints);
        let (b_before, b_mine) = cut(buffers, st.buffers);
        let (x_before, x_mine) = cut(texts, st.texts);
        let tele = member_tele.get(m).copied().unwrap_or_default();
        let store = ctx.telemetry;
        let mut io = Io {
            inputs: Inputs {
                slots: range(input_slots, st.inputs),
                frames: 1,
                params,
                scalars: &[],
                ints: &[],
                samples: &[],
                chan_table,
                triggers: &[],
                midi: &[],
                spectra: &[],
                buffers: &[],
                texts: &[],
                auto: AutoFlags {
                    load: ctx.first && member_load.get(m).copied().unwrap_or(false),
                    ..ctx.auto
                },
                local: Local {
                    scalars: &[],
                    ints: i_before,
                    samples: s_before,
                    triggers: t_before,
                    midi: m_before,
                    spectra: &[],
                    buffers: b_before,
                    texts: x_before,
                },
            },
            outputs: Outputs {
                slots: range(output_slots, st.outputs),
                frames: 1,
                samples: s_mine,
                scalars: &mut [],
                ints: i_mine,
                triggers: t_mine,
                midi: m_mine,
                spectra: &mut [],
                buffers: b_mine,
                texts: x_mine,
            },
            host: HostAccess::none(1),
            telemetry: TelemetryAccess {
                values: store
                    .node_values
                    .get(tele.values.0 as usize..tele.values.1 as usize)
                    .unwrap_or(&[]),
                scope: store
                    .scope
                    .get(tele.scope.0 as usize..tele.scope.1 as usize)
                    .unwrap_or(&[]),
                scope_pos: tele.scope_pos.and_then(|p| store.scope_pos.get(p as usize)),
            },
        };
        if st.muted {
            bypass(st.bypass, &mut io);
        } else {
            let pctx = ProcessCtx {
                frames: 1,
                sample_rate: ctx.sample_rate,
                transport: ctx.transport,
                phase: st.phase,
                engine: ctx.engine,
            };
            node.process(&pctx, &mut io);
        }
    }

    /// Gather this sample's socket telemetry.
    fn measure(&mut self) {
        for s in &mut self.sockets {
            match s.src {
                Inner::Samples { at, count } => {
                    let mut p = 0.0f32;
                    for c in 0..count.max(1) {
                        if let Some(x) = self.samples.get((at + c) as usize) {
                            p = p.max(x[0].abs());
                        }
                    }
                    s.level = s.level.max(p);
                    s.value = self.samples.get(at as usize).map_or(0.0, |x| x[0]);
                    if count > 1 {
                        s.value = p;
                    }
                }
                Inner::Int(at) => {
                    let v = self.ints.get(at as usize).copied().unwrap_or(0) as f32;
                    s.level = v.abs();
                    s.value = v;
                }
                Inner::Trigger(at) => {
                    s.events += self.triggers.get(at as usize).map_or(0, Vec::len) as u32;
                }
                Inner::Midi(at) => {
                    s.events += self.midi.get(at as usize).map_or(0, Vec::len) as u32;
                }
                Inner::Buffer(at) => {
                    let secs = self
                        .buffers
                        .get(at as usize)
                        .and_then(Option::as_ref)
                        .map_or(0.0, |b| b.frames() as f32 / (b.sample_rate.max(1) as f32));
                    s.level = secs;
                    s.value = secs;
                }
                Inner::Text(at) => {
                    let n = self.texts.get(at as usize).map_or(0, TextBuf::len) as f32;
                    s.level = n;
                    s.value = n;
                }
            }
        }
    }

    /// Publish the block's socket telemetry as the outer schedule does.
    fn publish_sockets(&self, ctx: &LoopCtx<'_>) {
        let t = ctx.telemetry;
        for s in &self.sockets {
            let i = s.slot as usize;
            if let Some(a) = t.socket_level.get(i) {
                let prev = f32::from_bits(a.load(Ordering::Relaxed));
                a.store(
                    s.level.max(prev * ctx.level_decay).to_bits(),
                    Ordering::Relaxed,
                );
            }
            if let Some(a) = t.socket_value.get(i) {
                a.store(s.value.to_bits(), Ordering::Relaxed);
            }
            if s.events > 0
                && let Some(a) = t.socket_events.get(i)
            {
                a.fetch_add(s.events, Ordering::Relaxed);
            }
        }
    }
}

/// What every inner step of one sample shares.
struct InnerCtx<'a> {
    sample_rate: f64,
    transport: TransportView<'a>,
    engine: EngineInfo,
    telemetry: &'a TelemetryStore,
    auto: AutoFlags,
    first: bool,
}

/// A parameter as it stands `i` samples into the block: a ramp that far
/// along (counted back from its target, as `FloatIn::Ramp` reads it), and
/// a pressed inline button only at the first sample.
fn shift(p: ParamSlot, i: usize) -> ParamSlot {
    let mut q = p;
    if i > 0 {
        q.fired = false;
        let i = i as u32;
        if p.remaining > i {
            q.remaining = p.remaining - i;
        } else if p.remaining > 0 {
            q.remaining = 0;
            q.value = p.target;
            q.step = 0.0;
        }
    }
    q
}

/// `v[..r.0]` read-only and `v[r.0..r.1]` writable.
fn cut<T>(v: &mut [T], r: Range) -> (&[T], &mut [T]) {
    let at = (r.0 as usize).min(v.len());
    let (before, rest) = v.split_at_mut(at);
    let len = (r.1.saturating_sub(r.0) as usize).min(rest.len());
    (before, &mut rest[..len])
}

fn range<T>(v: &[T], r: Range) -> &[T] {
    v.get(r.0 as usize..r.1 as usize).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_found_from_their_roots() {
        // 0 → 1 → 2 → 0 is a loop; 2 → 3 leaves it; 4 ↔ 5 is another one
        // not reachable from the root.
        let succ = vec![vec![1], vec![2], vec![0, 3], vec![], vec![5], vec![4]];
        let mut comps = strongly_connected(&succ, &[1]);
        for c in &mut comps {
            c.sort_unstable();
        }
        assert!(comps.contains(&vec![0, 1, 2]));
        assert!(comps.contains(&vec![3]));
        assert!(!comps.iter().any(|c| c.contains(&4)));
        // A node alone, and one wired to itself, are components of one.
        assert_eq!(strongly_connected(&[vec![]], &[0]), vec![vec![0]]);
        assert_eq!(strongly_connected(&[vec![0]], &[0]), vec![vec![0]]);
    }

    #[test]
    fn a_long_chain_does_not_overflow() {
        let n = 200_000;
        let mut succ: Vec<Vec<usize>> = (0..n).map(|i| vec![i + 1]).collect();
        succ[n - 1] = vec![0];
        let comps = strongly_connected(&succ, &[0]);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].len(), n);
    }

    #[test]
    fn params_shift_along_their_ramp() {
        let p = ParamSlot {
            value: 0.0,
            step: 0.25,
            remaining: 4,
            target: 1.0,
            fired: true,
        };
        // FloatIn::Ramp reads frame 0 as target - step * remaining.
        let at0 = |q: ParamSlot| {
            if q.remaining > 0 {
                q.target - q.step * q.remaining as f32
            } else {
                q.value
            }
        };
        assert_eq!(at0(shift(p, 0)), 0.0);
        assert!(shift(p, 0).fired);
        assert_eq!(at0(shift(p, 1)), 0.25);
        assert!(!shift(p, 1).fired);
        assert_eq!(at0(shift(p, 3)), 0.75);
        assert_eq!(at0(shift(p, 4)), 1.0);
        assert_eq!(at0(shift(p, 100)), 1.0);
    }
}
