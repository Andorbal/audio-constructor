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

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{HostAccess, MidiEvent, NodePath, Project, Registry, Resources, Value, param_value};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::compile::{CarryLayout, CompileOptions, Compiled, ParamMap, compile};
use crate::graph::{BlockTransport, CompiledGraph};
use crate::report::CompileReport;
use crate::telemetry::{EngineStatus, TelemetryLayout, TelemetryReader, TelemetryStore};

const GRAPH_RING: usize = 4;
const GARBAGE_RING: usize = 8;
const PARAM_RING: usize = 4096;
const COMMAND_RING: usize = 256;
const MIDI_SCRATCH: usize = 1024;

/// How the engine runs: the device (or render) rate and the largest block.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub sample_rate: f64,
    pub max_block: usize,
    pub socket_telemetry: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            sample_rate: 48_000.0,
            max_block: 1024,
            socket_telemetry: true,
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
        let compiled = compile(
            &project,
            &registry,
            resources.as_ref(),
            &CompileOptions {
                sample_rate: options.sample_rate,
                max_block: options.max_block,
                socket_telemetry: options.socket_telemetry,
                generation: 0,
            },
            None,
        );
        let (graph_tx, graph_rx) = RingBuffer::new(GRAPH_RING);
        let (garbage_tx, garbage_rx) = RingBuffer::new(GARBAGE_RING);
        let (param_tx, param_rx) = RingBuffer::new(PARAM_RING);
        let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_RING);
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
            params,
            carry,
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
        // New resources mean nodes must be rebuilt, not carried.
        self.carry = CarryLayout::default();
        self.recompile()
    }

    fn recompile(&mut self) -> CompileReport {
        self.collect_garbage();
        self.generation += 1;
        let compiled = compile(
            &self.project,
            &self.registry,
            self.resources.as_ref(),
            &CompileOptions {
                sample_rate: self.options.sample_rate,
                max_block: self.options.max_block,
                socket_telemetry: self.options.socket_telemetry,
                generation: self.generation,
            },
            Some(&self.carry),
        );
        let report = compiled.report.clone();
        self.report = report.clone();
        self.pending = Some(compiled);
        self.flush_pending();
        report
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

    /// Stop playback, keeping the position.
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
}

impl Processor {
    /// Use a monotonic clock for load measurement, xrun detection, and per-node timing.
    pub fn set_clock(&mut self, clock: Option<ClockFn>) {
        self.clock = clock;
    }

    pub fn status(&self) -> &Arc<EngineStatus> {
        &self.status
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
        io.output.fill(0.0);
        if frames == 0 {
            return;
        }
        if self.transport.reset_flag {
            self.graph.reset_nodes();
            self.transport.reset_flag = false;
        }
        let timing = self.status.timing_enabled.load(Ordering::Relaxed);
        let node_clock = if timing { self.clock } else { None };
        let max_block = self.graph.max_block.max(1);
        let mut done = 0usize;
        while done < frames {
            let mut n = (frames - done).min(max_block);
            let t = self.transport;
            if t.playing
                && let Some((_, le)) = t.loop_region
                && t.position < le
                && t.position + n as u64 > le
            {
                n = (le - t.position) as usize;
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
            };
            self.graph.run(n, block, &mut host, node_clock);
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
        self.publish_status(io, frames, t_start);
    }

    fn publish_status(&mut self, io: &HostIo, frames: usize, t_start: Option<u64>) {
        let s = &self.status;
        s.playing.store(self.transport.playing, Ordering::Relaxed);
        s.recording
            .store(self.transport.recording, Ordering::Relaxed);
        s.position.store(self.transport.position, Ordering::Relaxed);
        s.blocks.fetch_add(1, Ordering::Relaxed);
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
        if let (Some(t0), Some(clock)) = (t_start, self.clock) {
            let elapsed = clock().saturating_sub(t0) as f64;
            let budget = frames as f64 / self.graph.sample_rate * 1e9;
            let load = if budget > 0.0 { elapsed / budget } else { 0.0 };
            s.load.store((load as f32).to_bits(), Ordering::Relaxed);
            if load > 1.0 {
                s.xruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn apply_commands(&mut self) {
        while let Ok(c) = self.cmd_rx.pop() {
            let t = &mut self.transport;
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
            }
        }
    }
}
