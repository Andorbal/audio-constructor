//! The engine API every host uses (ARCHITECTURE §8).
//!
//! An [`Engine`] is two halves joined by lock-free rings:
//! - [`EngineHandle`] lives on a control thread (the UI, the CLI): it owns the
//!   project, compiles it, and sends graphs, parameter changes, and transport
//!   commands.
//! - [`Processor`] lives on the audio thread: it swaps graphs in at block
//!   boundaries, applies messages, and runs the schedule. It never allocates,
//!   locks, or blocks; old graphs go back to the handle to be dropped.
//!
//! Offline hosts keep both halves together and call [`Engine::process`].

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{
    CONTROL_CAPACITY, ControlEvent, EngineInfo, HostAccess, InputEvent, MidiEvent, NodePath,
    OscMessage, Project, Registry, Resources, TransportAction, Value, param_value,
};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::bake::{BakeInput, BakeJob, BakeResult, BakeSpec, BakeStore, WholeState};
use crate::bindings::{BOUND_CAPACITY, BindingValues, Bound, MAX_SPLITS, carry_over};
use crate::compile::{CarryLayout, CompileOptions, Compiled, ParamMap, compile};
use crate::graph::{BlockTransport, CompiledGraph};
use crate::pool::{MAX_WORKERS, RtPool};
use crate::report::CompileReport;
use crate::telemetry::{EngineStatus, TelemetryLayout, TelemetryReader, TelemetryStore};

const GRAPH_RING: usize = 4;
const GARBAGE_RING: usize = 8;
const PARAM_RING: usize = 4096;
const COMMAND_RING: usize = 256;
const MIDI_SCRATCH: usize = 1024;
/// Raw input events waiting for the editor to learn from (dropped when
/// full).
const LEARN_RING: usize = 512;

/// How the engine runs: the device (or render) rate and the largest block.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub sample_rate: f64,
    pub max_block: usize,
    pub socket_telemetry: bool,
    /// An offline render: every input is known, so whole-input nodes are
    /// baked even when Audio In or MIDI In feeds them.
    pub offline: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            sample_rate: 48_000.0,
            max_block: 1024,
            socket_telemetry: true,
            offline: false,
        }
    }
}

/// A change to the engine's project. Every edit recompiles and hot-swaps.
#[derive(Clone, Debug)]
pub enum GraphEdit {
    /// Replace the whole project (the editor's usual path after any edit).
    Replace(Box<Project>),
}

/// Buffers a host hands to [`Processor::process`] each block. Channel-major:
/// channel `c` is `[c * frames .. (c + 1) * frames]`.
#[derive(Debug)]
pub struct HostIo<'a> {
    pub frames: usize,
    pub input: &'a [f32],
    pub in_channels: usize,
    pub output: &'a mut [f32],
    pub out_channels: usize,
    /// MIDI received during this block, offsets relative to its first frame.
    pub midi: &'a [MidiEvent],
    /// Where `MidiOut` nodes put MIDI for the devices, offsets relative to
    /// this block's first frame, in order. The host clears it and gives it
    /// capacity; messages beyond its capacity are dropped. `None`: no MIDI
    /// output.
    pub midi_out: Option<&'a mut Vec<MidiEvent>>,
    /// Control-surface events (keys, gamepads, OSC) received during this
    /// block, offsets relative to its first frame, in order.
    pub controls: &'a [ControlEvent],
    /// Where `OscOut` nodes put OSC messages, like `midi_out`. `None`: no
    /// OSC output.
    pub osc_out: Option<&'a mut Vec<OscMessage>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Play,
    Stop,
    Seek(u64),
    SetRecording(bool),
    SetLoop(Option<(u64, u64)>),
    Reset,
}

#[derive(Clone, Copy, Debug)]
struct ParamMsg {
    generation: u64,
    slot: u32,
    value: f32,
    fire: bool,
}

/// Clock in nanoseconds, for per-node timing and load measurement. Hosts
/// without a monotonic clock (wasm) pass none.
pub type ClockFn = fn() -> u64;

#[derive(Debug, Default, Clone, Copy)]
struct Transport {
    playing: bool,
    recording: bool,
    position: u64,
    loop_region: Option<(u64, u64)>,
    start_flag: bool,
    stop_flag: bool,
    reset_flag: bool,
}

/// The audio-thread half.
pub struct Processor {
    graph: Box<CompiledGraph>,
    graph_rx: Consumer<Box<CompiledGraph>>,
    garbage_tx: Producer<Box<CompiledGraph>>,
    param_rx: Consumer<ParamMsg>,
    cmd_rx: Consumer<Command>,
    transport: Transport,
    status: Arc<EngineStatus>,
    clock: Option<ClockFn>,
    midi_scratch: Vec<MidiEvent>,
    control_scratch: Vec<ControlEvent>,
    /// This block's events that match a binding, in offset order.
    bound: Vec<Bound>,
    learn_tx: Producer<InputEvent>,
    workers: Option<Arc<RtPool>>,
    busy: [u64; MAX_WORKERS + 1],
}

/// The control-thread half.
pub struct EngineHandle {
    registry: Arc<Registry>,
    resources: Arc<dyn Resources>,
    project: Project,
    options: EngineOptions,
    generation: u64,
    /// Layout of the last graph pushed to the processor.
    carry: CarryLayout,
    params: ParamMap,
    layout: Arc<TelemetryLayout>,
    store: Arc<TelemetryStore>,
    status: Arc<EngineStatus>,
    report: CompileReport,
    pending: Option<Compiled>,
    graph_tx: Producer<Box<CompiledGraph>>,
    garbage_rx: Consumer<Box<CompiledGraph>>,
    param_tx: Producer<ParamMsg>,
    cmd_tx: Producer<Command>,
    learn_rx: Consumer<InputEvent>,
    /// What the bindings of the last graph pushed set.
    binding_values: Arc<BindingValues>,
    bakes: BakeStore,
    bake_specs: Vec<BakeSpec>,
    in_flight: HashSet<(NodePath, u64)>,
    bake_input: BakeInput,
    input_hash: u64,
    bake_frames: Option<u64>,
    resources_generation: u64,
    bakes_dirty: bool,
}

/// Both halves together (offline hosts, tests).
pub struct Engine {
    pub handle: EngineHandle,
    pub processor: Processor,
}

impl Engine {
    /// Compile a project and build an engine. Compile problems are in the
    /// report; the engine always runs whatever compiled.
    pub fn new(
        project: Project,
        registry: Arc<Registry>,
        resources: Arc<dyn Resources>,
        options: EngineOptions,
    ) -> (Engine, CompileReport) {
        let bakes = BakeStore::default();
        let compiled = compile(
            &project,
            &registry,
            resources.as_ref(),
            &compile_options(&project, &options, 0, None, 0, 0),
            None,
            &bakes,
        );
        let (graph_tx, graph_rx) = RingBuffer::new(GRAPH_RING);
        let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_RING);
        let (param_tx, param_rx) = RingBuffer::new(PARAM_RING);
        let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_RING);
        let (learn_tx, learn_rx) = RingBuffer::new(LEARN_RING);
        let status = Arc::new(EngineStatus::default());
        status
            .sample_rate
            .store(options.sample_rate as u32, Ordering::Relaxed);
        let loop_region = project.settings.loop_region.and_then(|[a, b]| {
            let t = &compiled.graph.tempo;
            let (s, e) = (t.sample_at(a), t.sample_at(b));
            (e > s).then_some((s, e))
        });
        let report = compiled.report.clone();
        let Compiled {
            graph,
            layout,
            telemetry,
            bindings: binding_values,
            params,
            carry,
            bakes: bake_specs,
            ..
        } = compiled;
        let processor = Processor {
            graph,
            graph_rx,
            garbage_tx,
            param_rx,
            cmd_rx,
            transport: Transport {
                loop_region,
                ..Transport::default()
            },
            status: status.clone(),
            clock: None,
            midi_scratch: Vec::with_capacity(MIDI_SCRATCH),
            control_scratch: Vec::with_capacity(CONTROL_CAPACITY),
            bound: Vec::with_capacity(BOUND_CAPACITY),
            learn_tx,
            workers: None,
            busy: [0; MAX_WORKERS + 1],
        };
        let handle = EngineHandle {
            registry,
            resources,
            project,
            options,
            generation: 0,
            carry,
            params,
            layout,
            store: telemetry,
            status,
            report: report.clone(),
            pending: None,
            graph_tx,
            garbage_rx,
            param_tx,
            cmd_tx,
            learn_rx,
            binding_values,
            bakes,
            bake_specs,
            in_flight: HashSet::new(),
            bake_input: BakeInput::default(),
            input_hash: 0,
            bake_frames: None,
            resources_generation: 0,
            bakes_dirty: false,
        };
        (Engine { handle, processor }, report)
    }

    pub fn split(self) -> (EngineHandle, Processor) {
        (self.handle, self.processor)
    }

    /// Process one block on this thread (offline rendering).
    pub fn process(&mut self, io: &mut HostIo) {
        self.processor.process(io);
        self.handle.collect_garbage();
    }
}

impl EngineHandle {
    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// The report from the last compile.
    pub fn report(&self) -> &CompileReport {
        &self.report
    }

    pub fn status(&self) -> &Arc<EngineStatus> {
        &self.status
    }

    /// Apply an edit: recompile and hot-swap at the next block boundary.
    /// Nodes whose path, kind, and config are unchanged keep their state.
    pub fn edit(&mut self, edit: GraphEdit) -> CompileReport {
        match edit {
            GraphEdit::Replace(p) => self.project = *p,
        }
        self.recompile()
    }

    /// Change resources (e.g. after loading a new clip) and recompile.
    pub fn set_resources(&mut self, resources: Arc<dyn Resources>) -> CompileReport {
        self.resources = resources;
        self.resources_generation += 1;
        // New resources mean nodes must be rebuilt, not carried.
        self.carry = CarryLayout::default();
        self.recompile()
    }

    /// Recompile the current project (e.g. after parameter changes upstream
    /// of a baked node, so its bake is redone).
    pub fn refresh(&mut self) -> CompileReport {
        self.recompile()
    }

    fn options_for_compile(&self) -> CompileOptions {
        compile_options(
            &self.project,
            &self.options,
            self.generation,
            self.bake_frames,
            self.input_hash,
            self.resources_generation,
        )
    }

    fn recompile(&mut self) -> CompileReport {
        self.collect_garbage();
        self.generation += 1;
        let compiled = compile(
            &self.project,
            &self.registry,
            self.resources.as_ref(),
            &self.options_for_compile(),
            Some(&self.carry),
            &self.bakes,
        );
        let report = compiled.report.clone();
        self.report = report.clone();
        self.bake_specs = compiled.bakes.clone();
        self.bakes_dirty = false;
        let live: HashSet<NodePath> = self.bake_specs.iter().map(|s| s.path.clone()).collect();
        self.bakes.retain(&live);
        self.pending = Some(compiled);
        self.flush_pending();
        report
    }

    // --- whole-input nodes and baking ---

    /// Audio, MIDI, and control events that `Audio In`, `MIDI In`, and the
    /// control-surface nodes will receive, for bakes in an offline render.
    /// Recompiles.
    pub fn set_bake_input(&mut self, input: BakeInput) -> CompileReport {
        let mut h = ac_core::util::Fnv64::new();
        if let Some(a) = &input.audio {
            h.write(&a.sample_rate.to_le_bytes());
            for ch in &a.channels {
                for v in ch {
                    h.write(&v.to_bits().to_le_bytes());
                }
                h.write(b"|");
            }
        }
        for (at, msg) in input.midi.iter() {
            h.write(&at.to_le_bytes());
            h.write(format!("{msg:?}").as_bytes());
        }
        if !input.controls.is_empty() {
            h.write(b"controls");
        }
        for (at, control, value) in input.controls.iter() {
            h.write(&at.to_le_bytes());
            h.write(format!("{control:?}").as_bytes());
            h.write(&value.to_bits().to_le_bytes());
        }
        self.input_hash = h.finish();
        self.bake_input = input;
        self.recompile()
    }

    /// Frames bakes cover (default: the project length). Recompiles.
    pub fn set_bake_frames(&mut self, frames: Option<u64>) -> CompileReport {
        self.bake_frames = frames;
        self.recompile()
    }

    /// Every whole-input node in the running graph and where it stands.
    pub fn bakes(&self) -> &[BakeSpec] {
        &self.bake_specs
    }

    /// Whether a bake for `path` is running.
    pub fn is_baking(&self, path: &NodePath) -> bool {
        self.in_flight.iter().any(|(p, _)| p == path)
    }

    /// Bake jobs that can start now: nodes waiting for a bake whose upstream
    /// bakes are done, minus those already handed out. Run each with
    /// [`BakeJob::run`] (on a worker thread live) and pass the result to
    /// [`EngineHandle::finish_bake`].
    pub fn take_bake_jobs(&mut self) -> Vec<BakeJob> {
        let mut jobs = Vec::new();
        let opts = self.options_for_compile();
        let project = Arc::new(self.project.clone());
        for spec in &self.bake_specs {
            if !matches!(spec.state, WholeState::Pending | WholeState::Stale) {
                continue;
            }
            let id = (spec.path.clone(), spec.key);
            if self.in_flight.contains(&id) {
                continue;
            }
            let deps_ready = spec.deps.iter().all(|d| {
                self.bake_specs
                    .iter()
                    .find(|s| &s.path == d)
                    .is_some_and(|s| s.state == WholeState::Baked)
            });
            if !deps_ready {
                continue;
            }
            self.in_flight.insert(id);
            jobs.push(BakeJob {
                path: spec.path.clone(),
                key: spec.key,
                project: project.clone(),
                registry: self.registry.clone(),
                resources: self.resources.clone(),
                options: opts.clone(),
                store: self.bakes.clone(),
                input: self.bake_input.clone(),
            });
        }
        jobs
    }

    /// Install a finished bake. A result for an outdated key is dropped (a
    /// newer job will come). Recompiles so the node streams it; returns
    /// whether it was used.
    pub fn finish_bake(&mut self, result: BakeResult) -> bool {
        self.in_flight.remove(&(result.path.clone(), result.key));
        let current = self
            .bake_specs
            .iter()
            .any(|s| s.path == result.path && s.key == result.key);
        if !current {
            return false;
        }
        self.bakes.insert(result);
        self.recompile();
        true
    }

    /// Run every bake to completion on this thread, in dependency order
    /// (offline renders and tests).
    pub fn bake_all(&mut self) {
        // Each pass finishes at least one bake or stops; the bound guards
        // against a node whose bake can never become current.
        for _ in 0..=self.bake_specs.len() * 2 + 1 {
            let jobs = self.take_bake_jobs();
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                let r = job.run();
                self.finish_bake(r);
            }
        }
    }

    /// True when a parameter changed upstream of a baked node since the last
    /// compile: call [`EngineHandle::refresh`] (debounced) to rebake.
    pub fn bakes_dirty(&self) -> bool {
        self.bakes_dirty
    }

    /// Push a compiled graph that couldn't be sent earlier (ring full).
    fn flush_pending(&mut self) {
        if self.graph_tx.slots() == 0 {
            return;
        }
        let Some(c) = self.pending.take() else {
            return;
        };
        let Compiled {
            graph,
            layout,
            telemetry,
            bindings,
            params,
            carry,
            ..
        } = c;
        match self.graph_tx.push(graph) {
            Ok(()) => {
                self.carry = carry;
                self.params = params;
                self.layout = layout;
                self.store = telemetry;
                self.binding_values = bindings;
            }
            Err(rtrb::PushError::Full(_)) => {
                // Unreachable given the slots() check; the graph is dropped here, on
                // the control thread, and the next edit recompiles.
            }
        }
    }

    /// Drop graphs the processor has finished with. Call regularly (each UI frame).
    pub fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage_rx.pop() {
            drop(g);
        }
        self.flush_pending();
    }

    /// Set an unwired input's value. Float inputs are smoothed. Also updates the
    /// stored project (for root-level nodes) so the next compile keeps it.
    /// Returns false if the socket has no parameter (wired, or unknown).
    pub fn set_param(&mut self, path: &NodePath, socket: &str, value: Value) -> bool {
        self.collect_garbage();
        if path.depth() == 1
            && let Some(id) = path.leaf()
            && let Some(node) = self.project.graph.node_mut(id)
        {
            node.params.insert(socket.to_string(), value.clone());
            if self
                .bake_specs
                .iter()
                .any(|s| s.upstream.iter().any(|u| u.0.first() == path.0.first()))
            {
                self.bakes_dirty = true;
            }
        }
        let Some(targets) = self.params.get(&(path.clone(), socket.to_string())) else {
            return false;
        };
        let mut ok = true;
        for t in targets {
            let v = param_value(&t.spec, Some(&value));
            let msg = ParamMsg {
                generation: self.generation_pushed(),
                slot: t.slot,
                value: v,
                fire: false,
            };
            if self.param_tx.push(msg).is_err() {
                self.status.dropped_messages.fetch_add(1, Ordering::Relaxed);
                ok = false;
            }
        }
        ok
    }

    /// Press an unwired trigger input's inline button.
    pub fn fire(&mut self, path: &NodePath, socket: &str) -> bool {
        let Some(targets) = self.params.get(&(path.clone(), socket.to_string())) else {
            return false;
        };
        let generation = self.generation_pushed();
        for t in targets {
            let _ = self.param_tx.push(ParamMsg {
                generation,
                slot: t.slot,
                value: 0.0,
                fire: true,
            });
        }
        true
    }

    fn generation_pushed(&self) -> u64 {
        if self.pending.is_some() {
            self.generation.saturating_sub(1)
        } else {
            self.generation
        }
    }

    fn command(&mut self, c: Command) {
        if self.cmd_tx.push(c).is_err() {
            self.status.dropped_messages.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Start playback from the current position (resets every node if at zero).
    pub fn play(&mut self) {
        self.command(Command::Play);
    }

    /// Stop playback, keeping the position. Stopping ends recording too, as
    /// a recorder's stop button ends the take.
    pub fn stop(&mut self) {
        self.command(Command::Stop);
    }

    /// Move the transport (samples at the engine rate).
    pub fn seek(&mut self, sample: u64) {
        self.command(Command::Seek(sample));
    }

    /// Seek to zero and play.
    pub fn play_from_start(&mut self) {
        self.command(Command::Seek(0));
        self.command(Command::Play);
    }

    pub fn set_recording(&mut self, on: bool) {
        self.command(Command::SetRecording(on));
    }

    /// Loop region in samples, `[start, end)`.
    pub fn set_loop(&mut self, region: Option<(u64, u64)>) {
        self.command(Command::SetLoop(region));
    }

    /// Reset every node (as on a restart from zero) at the next block.
    pub fn reset(&mut self) {
        self.command(Command::Reset);
    }

    pub fn is_playing(&self) -> bool {
        self.status.playing.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> u64 {
        self.status.position.load(Ordering::Relaxed)
    }

    /// Telemetry for the most recently compiled graph.
    pub fn telemetry(&self) -> TelemetryReader {
        TelemetryReader {
            store: self.store.clone(),
            layout: self.layout.clone(),
            status: self.status.clone(),
        }
    }

    /// What each of the project's bindings last set on its input, for the
    /// most recently compiled graph (indexed like `Project::bindings`).
    pub fn binding_values(&self) -> Arc<BindingValues> {
        self.binding_values.clone()
    }

    /// While on, the processor copies every MIDI and control event it
    /// receives for [`EngineHandle::take_input_events`] (dropping them when
    /// nobody takes them fast enough). Turning it on forgets anything left
    /// from before.
    pub fn set_input_monitor(&mut self, on: bool) {
        if on {
            while self.learn_rx.pop().is_ok() {}
        }
        self.status.input_monitor.store(on, Ordering::Relaxed);
    }

    /// Events received since the last call while the input monitor is on,
    /// oldest first: what the editor learns bindings from.
    pub fn take_input_events(&mut self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        while let Ok(e) = self.learn_rx.pop() {
            out.push(e);
        }
        out
    }
}

impl Processor {
    /// Use a monotonic clock for load measurement, xrun detection, and per-node timing.
    pub fn set_clock(&mut self, clock: Option<ClockFn>) {
        self.clock = clock;
    }

    /// Spread independent steps across a pool of real-time worker threads
    /// (or run everything on the audio thread with `None`). Results are
    /// identical either way.
    pub fn set_workers(&mut self, workers: Option<Arc<RtPool>>) {
        let threads = workers.as_ref().map_or(1, |w| w.workers() + 1);
        self.status.cores.store(threads as u32, Ordering::Relaxed);
        self.workers = workers;
    }

    pub fn status(&self) -> &Arc<EngineStatus> {
        &self.status
    }

    /// Swap in the newest graph the handle sent, as [`Processor::process`]
    /// does at the start of every block, and return the running graph's
    /// generation. A host that feeds a graph's bindings their current
    /// values (a plugin's host parameters) calls it first, so a new graph
    /// gets them in the block it starts. Real-time safe.
    pub fn install_pending(&mut self) -> u64 {
        self.install_graphs();
        self.graph.generation
    }

    /// Reset every node (as on a restart from zero) at the start of the
    /// next block, from the audio thread: what a plugin host's reset asks
    /// for. Real-time safe.
    pub fn reset(&mut self) {
        self.transport.reset_flag = true;
    }

    /// Follow an external transport (a plugin host's): start or stop to
    /// match `playing`, and move to `position` (samples at the engine
    /// rate) when it is more than `tolerance` samples from where the
    /// engine is, or exactly when the engine is stopped or starting. Moves
    /// smaller than the tolerance while playing are taken as the host's
    /// rounding, not a jump, so they don't restart generators. Call it
    /// before [`Processor::process`]; real-time safe.
    pub fn follow_transport(&mut self, playing: bool, position: Option<u64>, tolerance: u64) {
        let t = &mut self.transport;
        if let Some(p) = position {
            let keep_playing = t.playing && playing;
            if p != t.position && (!keep_playing || p.abs_diff(t.position) > tolerance) {
                command(t, Command::Seek(p));
            }
        }
        if playing != t.playing {
            command(
                t,
                if playing {
                    Command::Play
                } else {
                    Command::Stop
                },
            );
        }
    }

    /// Process one host block. Real-time safe: no allocation (asserted in
    /// debug builds), no locks, no blocking.
    pub fn process(&mut self, io: &mut HostIo) {
        assert_no_alloc::assert_no_alloc(|| self.process_inner(io));
    }

    fn process_inner(&mut self, io: &mut HostIo) {
        let t_start = self.clock.map(|c| c());
        let frames = io.frames.min(io.output.len() / io.out_channels.max(1));
        self.apply_commands();
        self.install_graphs();
        self.apply_params();
        self.monitor_inputs(io);
        self.graph
            .bindings
            .gather(io.midi, io.controls, &mut self.bound);
        io.output.fill(0.0);
        if frames == 0 {
            return;
        }
        let timing = self.status.timing_enabled.load(Ordering::Relaxed);
        let node_clock = if timing { self.clock } else { None };
        let max_block = self.graph.max_block.max(1);
        let mut done = 0usize;
        let mut next_bound = 0usize;
        let mut splits = 0usize;
        while done < frames {
            // Bound events due by now act before this sub-block runs.
            while let Some(&b) = self.bound.get(next_bound)
                && b.offset as usize <= done
            {
                if let Some(action) = self.graph.apply_binding(b) {
                    press(&mut self.transport, action);
                }
                next_bound += 1;
            }
            if self.transport.reset_flag {
                self.graph.reset_nodes();
                self.transport.reset_flag = false;
            }
            let mut n = (frames - done).min(max_block);
            let t = self.transport;
            if t.playing
                && let Some((_, le)) = t.loop_region
                && t.position < le
                && t.position + n as u64 > le
            {
                n = (le - t.position) as usize;
            }
            // End the sub-block where the next bound event lands, so it
            // acts on its own sample whatever the block size.
            if splits < MAX_SPLITS
                && let Some(b) = self.bound.get(next_bound)
                && (b.offset as usize) < done + n
            {
                n = b.offset as usize - done;
                splits += 1;
            }
            let n = n.max(1);
            // MIDI for this sub-block, rebased.
            self.midi_scratch.clear();
            for e in io.midi {
                let off = e.offset as usize;
                if off >= done
                    && off < done + n
                    && self.midi_scratch.len() < self.midi_scratch.capacity()
                {
                    self.midi_scratch
                        .push(MidiEvent::new((off - done) as u32, e.msg));
                }
            }
            self.control_scratch.clear();
            for e in io.controls {
                let off = e.offset as usize;
                if off >= done
                    && off < done + n
                    && self.control_scratch.len() < self.control_scratch.capacity()
                {
                    self.control_scratch.push(ControlEvent {
                        offset: (off - done) as u32,
                        ..*e
                    });
                }
            }
            let block = BlockTransport {
                playing: t.playing,
                recording: t.recording,
                position: t.position,
                started: t.start_flag,
                stopped: t.stop_flag,
            };
            let mut host = HostAccess {
                input: io.input,
                in_channels: io.in_channels,
                output: &mut *io.output,
                out_channels: io.out_channels,
                stride: io.frames,
                offset: done,
                frames: n,
                midi: &self.midi_scratch,
                midi_out: io.midi_out.as_deref_mut(),
                controls: &self.control_scratch,
                osc_out: io.osc_out.as_deref_mut(),
            };
            let engine = EngineInfo {
                load: self.status.load_fraction(),
                xruns: self.status.total_xruns(),
                latency: self.graph.latency,
            };
            self.graph.run(
                n,
                block,
                &mut host,
                node_clock,
                self.workers.as_deref(),
                engine,
            );
            self.transport.start_flag = false;
            self.transport.stop_flag = false;
            self.graph.advance_params(n);
            if self.transport.playing {
                self.transport.position += n as u64;
                if let Some((ls, le)) = self.transport.loop_region
                    && le > ls
                    && self.transport.position >= le
                {
                    self.transport.position = ls;
                    self.transport.start_flag = true;
                }
            }
            done += n;
        }
        if node_clock.is_some() {
            self.graph.telemetry.publish_node_times();
        }
        self.publish_status(io, frames, t_start);
    }

    fn publish_status(&mut self, io: &HostIo, frames: usize, t_start: Option<u64>) {
        let s = &self.status;
        s.playing.store(self.transport.playing, Ordering::Relaxed);
        s.recording
            .store(self.transport.recording, Ordering::Relaxed);
        s.position.store(self.transport.position, Ordering::Relaxed);
        s.blocks.fetch_add(1, Ordering::Relaxed);
        s.block_frames.store(frames as u32, Ordering::Relaxed);
        let decay = self.graph.level_decay;
        for c in 0..2usize.min(io.out_channels) {
            let st = c * io.frames;
            let p = io
                .output
                .get(st..st + frames)
                .map_or(0.0, |x| x.iter().fold(0.0f32, |m, v| m.max(v.abs())));
            let prev = f32::from_bits(s.master_peak[c].load(Ordering::Relaxed));
            s.master_peak[c].store(p.max(prev * decay).to_bits(), Ordering::Relaxed);
        }
        s.latency.store(self.graph.latency, Ordering::Relaxed);
        if let (Some(t0), Some(clock)) = (t_start, self.clock) {
            let elapsed = clock().saturating_sub(t0) as f64;
            let budget = frames as f64 / self.graph.sample_rate * 1e9;
            let load = if budget > 0.0 { elapsed / budget } else { 0.0 };
            s.load.store((load as f32).to_bits(), Ordering::Relaxed);
            if load > 1.0 {
                s.xruns.fetch_add(1, Ordering::Relaxed);
            }
            // Per-core load: time each thread spent on steps this block.
            if let Some(w) = self.workers.as_deref() {
                w.take_busy(&mut self.busy);
                for (k, b) in self.busy.iter().enumerate().take(w.workers() + 1) {
                    let l = if budget > 0.0 {
                        *b as f64 / budget
                    } else {
                        0.0
                    };
                    if let Some(a) = s.core_load.get(k) {
                        a.store((l as f32).to_bits(), Ordering::Relaxed);
                    }
                }
            } else if let Some(a) = s.core_load.first() {
                a.store((load as f32).to_bits(), Ordering::Relaxed);
            }
        }
    }

    fn apply_commands(&mut self) {
        while let Ok(c) = self.cmd_rx.pop() {
            command(&mut self.transport, c);
        }
    }

    /// Copy this block's raw events for the editor while it's learning.
    fn monitor_inputs(&mut self, io: &HostIo) {
        if !self.status.input_monitor.load(Ordering::Relaxed) {
            return;
        }
        for e in io.midi {
            // A full ring drops the event; learning only needs a few.
            let _ = self.learn_tx.push(InputEvent::Midi(e.msg));
        }
        for e in io.controls {
            let _ = self.learn_tx.push(InputEvent::Control(e.control, e.value));
        }
    }

    fn install_graphs(&mut self) {
        while self.garbage_tx.slots() > 0 {
            let Ok(mut new) = self.graph_rx.pop() else {
                break;
            };
            for i in 0..new.carry.len() {
                let (ni, oi) = new.carry[i];
                if let (Some(n), Some(o)) = (
                    new.nodes.get_mut(ni as usize),
                    self.graph.nodes.get_mut(oi as usize),
                ) {
                    std::mem::swap(&mut n.node, &mut o.node);
                }
            }
            carry_over(&mut new, &self.graph);
            let old = std::mem::replace(&mut self.graph, new);
            self.status
                .generation
                .store(self.graph.generation, Ordering::Relaxed);
            // slots() > 0 was checked, so this can't fail; if it somehow did, the
            // graph would be dropped here, which allocates nothing but frees.
            let _ = self.garbage_tx.push(old);
        }
    }

    fn apply_params(&mut self) {
        let generation = self.graph.generation;
        while let Ok(m) = self.param_rx.pop() {
            if m.generation != generation {
                continue;
            }
            if m.fire {
                self.graph.fire(m.slot);
            } else {
                self.graph.set_param(m.slot, m.value);
                self.graph.forget_bound_value(m.slot);
            }
        }
    }
}

/// Apply a transport command.
fn command(t: &mut Transport, c: Command) {
    match c {
        Command::Play => {
            if !t.playing {
                t.playing = true;
                t.start_flag = true;
                t.stop_flag = false;
                if t.position == 0 {
                    t.reset_flag = true;
                }
            }
        }
        Command::Stop => {
            if t.playing {
                t.playing = false;
                t.stop_flag = true;
                t.start_flag = false;
            }
            t.recording = false;
        }
        Command::Seek(p) => {
            t.position = p;
            if t.playing {
                t.start_flag = true;
                if p == 0 {
                    t.reset_flag = true;
                }
            }
        }
        Command::SetRecording(on) => t.recording = on,
        Command::SetLoop(r) => t.loop_region = r.filter(|(a, b)| b > a),
        Command::Reset => t.reset_flag = true,
    }
}

/// A bound control pressed a transport action. These do what the editor's
/// transport buttons do.
fn press(t: &mut Transport, action: TransportAction) {
    match action {
        TransportAction::Play => command(t, Command::Play),
        TransportAction::Stop => command(t, Command::Stop),
        TransportAction::PlayStop => {
            let c = if t.playing {
                Command::Stop
            } else {
                Command::Play
            };
            command(t, c);
        }
        TransportAction::Record => {
            t.recording = !t.recording;
            if t.recording {
                command(t, Command::Play);
            }
        }
        TransportAction::PlayFromStart => {
            command(t, Command::Seek(0));
            command(t, Command::Play);
        }
    }
}

/// Compile settings for a project under engine options.
fn compile_options(
    project: &Project,
    options: &EngineOptions,
    generation: u64,
    bake_frames: Option<u64>,
    input_hash: u64,
    resources_generation: u64,
) -> CompileOptions {
    let length = project.settings.length;
    let length = if length.is_finite() {
        length.clamp(0.0, 24.0 * 3600.0)
    } else {
        0.0
    };
    CompileOptions {
        sample_rate: options.sample_rate,
        max_block: options.max_block,
        socket_telemetry: options.socket_telemetry,
        generation,
        offline: options.offline,
        bake_block: (project.settings.block_size as usize).clamp(1, 16_384),
        bake_frames: bake_frames.unwrap_or((length * options.sample_rate).ceil() as u64),
        input_hash,
        resources_generation,
    }
}
