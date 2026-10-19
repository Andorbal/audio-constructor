//! Running the engine live: the control half ([`EngineHandle`]) lives with
//! the editor; the audio half ([`Processor`]) runs in an `ac-io` stream on
//! the chosen audio device, or on the paced null stream when there is no
//! device (this falls back silently, so the editor always runs). In a plugin
//! ([`AudioMode::Hosted`]) the host's audio thread runs the processor and
//! the editor opens no devices at all.
//!
//! Switching devices at the same rate moves the processor to the new stream
//! without rebuilding the engine; a different rate rebuilds the engine at
//! that rate (the transport and node state start over).
//!
//! Besides MIDI, the engine hears control surfaces: the computer keys (when
//! Play keys is on, see `play_keys.rs`), gamepads, and OSC on a UDP port
//! chosen in the Devices tab (off until turned on). OSC Out nodes send
//! through a sender thread here.

use crate::i18n::{tr, tr_args};
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use ac_core::{CONTROL_CAPACITY, Control, ControlEvent, Key, Project, Registry, Resources};
use ac_engine::{CompileReport, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};
use ac_io::{
    AudioConfig, AudioHost, AudioStream, Baker, ControlInputs, ControlReceiver, ControlSender,
    CpalAudio, DEFAULT_OSC_PORT, EventPorts, Gamepads, Latency, LatencyError, MidiInputs,
    MidiOutputs, MidiSelect, NullAudio, OscInput, OscOutput, StreamInfo, StreamStats, worker_pool,
};

/// Rate and buffer of the null stream when there's no device.
const NULL_RATE: u32 = 48_000;
const NULL_BUFFER: u32 = 256;
const MIDI_RESCAN: Duration = Duration::from_secs(2);

/// How the editor drives the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioMode {
    /// Open this device setup; fall back to the null stream if it fails.
    Device(AudioConfig),
    /// The paced null stream only (tests, headless).
    Null,
    /// Nothing runs the processor (unit tests tick it by hand).
    Off,
    /// A host runs the processor on its own audio thread (a plugin): the
    /// editor keeps the control half and opens no devices, MIDI ports,
    /// gamepads, or OSC sockets. The host takes the processor with
    /// [`EngineHost::take_processor`].
    Hosted(HostAudio),
}

/// The audio a host provides when it runs the processor itself (a plugin
/// host's sample rate, largest block, and channels).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostAudio {
    /// What the Devices tab and the status bar call the host.
    pub host: String,
    pub sample_rate: u32,
    /// The most frames the host passes to one `process` call. Longer calls
    /// are fine too: the processor splits them.
    pub max_block: usize,
    pub in_channels: usize,
    pub out_channels: usize,
}

impl Default for HostAudio {
    /// Stand-in values until the host says what it runs at (a plugin is
    /// created before it's activated).
    fn default() -> Self {
        HostAudio {
            host: "plugin host".into(),
            sample_rate: NULL_RATE,
            max_block: 512,
            in_channels: 2,
            out_channels: 2,
        }
    }
}

impl HostAudio {
    fn info(&self) -> StreamInfo {
        StreamInfo {
            host: self.host.clone(),
            output_device: None,
            input_device: None,
            sample_rate: self.sample_rate.max(1),
            buffer_size: u32::try_from(self.max_block.max(1)).unwrap_or(u32::MAX),
            in_channels: self.in_channels,
            out_channels: self.out_channels,
            notes: Vec::new(),
        }
    }
}

/// `Send` where the editor must be (everywhere but a browser page, whose
/// devices live on its one thread).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}
/// `Send` where the editor must be (everywhere but a browser page, whose
/// devices live on its one thread).
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

/// What a host that runs the processor shows in the Devices tab under its
/// audio (the web build: the microphone, MIDI ports, and gamepads it gives
/// the engine). Without one the tab says the host provides audio and MIDI.
pub trait DevicesPanel: MaybeSend {
    fn show(&mut self, ui: &mut egui::Ui);
}

/// A latency measurement's progress.
#[derive(Clone, Debug, PartialEq)]
pub enum LatencyState {
    Idle,
    Running,
    Done(Result<Latency, LatencyError>),
}

/// The engine as the editor sees it: the handle, the stream running the
/// processor, MIDI inputs, and the last compile report.
pub struct EngineHost {
    pub handle: EngineHandle,
    stream: Option<Box<dyn AudioStream>>,
    /// The processor when no stream runs it (`AudioMode::Off`, or every
    /// backend failed).
    idle: Option<Processor>,
    pub midi: Option<MidiInputs>,
    /// Where MIDI Out nodes send; nothing until chosen in the Devices tab.
    pub midi_out: Option<MidiOutputs>,
    pub midi_out_choice: MidiSelect,
    last_rescan: Instant,
    pub report: CompileReport,
    /// What the engine runs at (from the stream, or the null defaults).
    pub info: StreamInfo,
    /// The setup that was asked for (the Devices tab edits a copy).
    pub config: AudioConfig,
    /// Why the device couldn't be used, when running on the null stream.
    pub device_error: Option<String>,
    pub latency: LatencyState,
    /// Document engine revision last sent.
    pub synced_revision: u64,
    mode_off: bool,
    /// Bakes whole-input nodes in the background.
    pub baker: Baker,
    /// Key, gamepad, and OSC sources feeding the engine.
    pub controls: Option<ControlInputs>,
    /// The computer keys (Play keys): one of `controls`' sources.
    keys: Option<ControlSender>,
    keys_sent: u64,
    /// Whether gamepads are listened to (with a device runtime).
    device_sources: bool,
    pub gamepads: Option<Gamepads>,
    /// The OSC listener, while it's on.
    pub osc_in: Option<OscInput>,
    /// The port OSC listens on (the Devices tab edits it).
    pub osc_port: u16,
    /// Why OSC couldn't listen, if it couldn't.
    pub osc_error: Option<String>,
    /// Where OSC Out nodes' messages go (with a device runtime).
    pub osc_out: Option<OscOutput>,
    /// The idle processor's event ports (`AudioMode::Off`), so
    /// [`EngineHost::tick_idle`] can feed it control events.
    idle_ports: EventPorts,
    idle_controls: Vec<ControlEvent>,
    /// What the host provides, when a host runs the processor
    /// (`AudioMode::Hosted`).
    hosted: Option<HostAudio>,
    /// The host's own devices, shown in the Devices tab.
    pub devices_panel: Option<Box<dyn DevicesPanel>>,
    /// The host keeps the project and the engine (a plugin's session): the
    /// editor compiles nothing itself and leaves the engine's upkeep and
    /// transport to the host. See [`EngineHost::keep_in_host`].
    in_host: bool,
    /// The project the engine should run next, waiting for the host
    /// (while `in_host`).
    outgoing: Option<Project>,
    /// Plays block previews over the patch (not when a host runs the
    /// processor: its audio thread doesn't mix them).
    audition: Option<ac_io::Audition>,
}

fn engine_at(
    project: &Project,
    registry: Arc<Registry>,
    resources: Arc<dyn Resources>,
    info: &StreamInfo,
) -> (EngineHandle, Processor, CompileReport) {
    let (engine, report) = Engine::new(
        project.clone(),
        registry,
        resources,
        EngineOptions {
            sample_rate: f64::from(info.sample_rate),
            max_block: info.max_block(),
            socket_telemetry: true,
            offline: false,
        },
    );
    let (h, p) = engine.split();
    (h, p, report)
}

/// A processor a host's audio thread runs measures its load with the same
/// clock the device streams give theirs.
fn timed(mut processor: Processor) -> Processor {
    ac_io::clock::init();
    processor.set_clock(Some(ac_io::clock::now_ns));
    processor
}

fn null_host(project: &Project, rate: u32) -> NullAudio {
    NullAudio::new(
        rate,
        NULL_BUFFER,
        2,
        ac_engine::required_output_channels(project),
    )
}

impl EngineHost {
    pub fn start(
        project: &Project,
        registry: Arc<Registry>,
        resources: Arc<dyn Resources>,
        mode: &AudioMode,
    ) -> EngineHost {
        let mut device_error = None;
        let (host, config): (Option<Box<dyn AudioHost>>, AudioConfig) = match mode {
            AudioMode::Device(cfg) => match CpalAudio::open(cfg) {
                Ok(h) => (Some(Box::new(h)), cfg.clone()),
                Err(e) => {
                    device_error = Some(e.to_string());
                    (Some(Box::new(null_host(project, NULL_RATE))), cfg.clone())
                }
            },
            AudioMode::Null => (
                Some(Box::new(null_host(project, NULL_RATE))),
                AudioConfig::default(),
            ),
            AudioMode::Off | AudioMode::Hosted(_) => (None, AudioConfig::default()),
        };
        let hosted = match mode {
            AudioMode::Hosted(a) => Some(a.clone()),
            _ => None,
        };
        let info = match (&host, &hosted) {
            (Some(h), _) => h.info().clone(),
            (None, Some(a)) => a.info(),
            (None, None) => null_host(project, NULL_RATE).info().clone(),
        };
        let (handle, processor, report) = engine_at(project, registry, resources, &info);
        let mut me = EngineHost {
            handle,
            stream: None,
            idle: None,
            midi: None,
            midi_out: None,
            midi_out_choice: MidiSelect::None,
            last_rescan: Instant::now(),
            report,
            info,
            config,
            device_error,
            latency: LatencyState::Idle,
            synced_revision: 0,
            mode_off: host.is_none() && hosted.is_none(),
            baker: Baker::new(),
            controls: None,
            keys: None,
            keys_sent: 0,
            device_sources: matches!(mode, AudioMode::Device(_)),
            gamepads: None,
            osc_in: None,
            osc_port: DEFAULT_OSC_PORT,
            osc_error: None,
            osc_out: None,
            idle_ports: EventPorts::default(),
            idle_controls: Vec::with_capacity(CONTROL_CAPACITY),
            hosted,
            devices_panel: None,
            in_host: false,
            outgoing: None,
            audition: None,
        };
        let mut ports = if me.device_sources {
            EventPorts {
                midi_in: Some(me.new_midi_inputs()),
                midi_out: Some(me.new_midi_outputs()),
                osc_out: Some(me.new_osc_output()),
                ..EventPorts::default()
            }
        } else if matches!(mode, AudioMode::Null) {
            // No ports, but room for virtual MIDI sources (`virtual_midi`).
            let (inputs, rx) = MidiInputs::new();
            me.midi = Some(inputs);
            EventPorts {
                midi_in: Some(rx),
                ..EventPorts::default()
            }
        } else {
            EventPorts::default()
        };
        // The computer keys work in every mode (tests play them too).
        ports.controls = Some(me.new_controls());
        if me.hosted.is_none() {
            ports.audition = Some(me.new_audition());
        }
        match host {
            Some(h) => me.run(h, processor, ports, project),
            None if me.hosted.is_some() => {
                me.idle = Some(timed(processor));
                me.idle_ports = ports;
            }
            None => {
                me.idle = Some(processor);
                me.idle_ports = ports;
            }
        }
        me
    }

    /// What the host provides, when a host runs the processor
    /// (`AudioMode::Hosted`).
    pub fn hosted(&self) -> Option<&HostAudio> {
        self.hosted.as_ref()
    }

    /// The audio-thread half, for a host that runs it (`AudioMode::Hosted`;
    /// tests in `AudioMode::Off` can take it too). `None` once taken: the
    /// next one comes from [`EngineHost::restart_hosted`].
    pub fn take_processor(&mut self) -> Option<Processor> {
        self.idle.take()
    }

    /// The control events (Play keys) for a host that runs the processor:
    /// its audio thread drains them into each block's `HostIo::controls`
    /// (`ControlReceiver::drain` is real-time safe). `None` once taken. The
    /// receiver outlives engine restarts.
    pub fn take_control_receiver(&mut self) -> Option<ControlReceiver> {
        self.idle_ports.controls.take()
    }

    /// The host runs at a new rate or block size (a plugin reactivated):
    /// build a new engine for `project` there and hand back its processor.
    /// The old processor's graphs are the old engine's; drop it with the old
    /// handle. Nothing changes outside `AudioMode::Hosted`.
    pub fn restart_hosted(
        &mut self,
        audio: HostAudio,
        project: &Project,
        registry: Arc<Registry>,
        resources: Arc<dyn Resources>,
    ) -> Option<Processor> {
        self.hosted.as_ref()?;
        let info = audio.info();
        let (handle, processor, report) = engine_at(project, registry, resources, &info);
        self.adopt(handle, report, audio);
        Some(timed(processor))
    }

    /// Use an engine the host built (its processor runs on the host's audio
    /// thread at `audio`'s rate). Turns the editor into `AudioMode::Hosted`
    /// if it wasn't, closing any device stream it had.
    pub fn attach_hosted(&mut self, handle: EngineHandle, audio: HostAudio) {
        if let Some(s) = self.stream.take() {
            drop(s.stop());
        }
        let report = handle.report().clone();
        self.adopt(handle, report, audio);
    }

    /// The host keeps the project and the engine from now on (a plugin's
    /// session, `AudioMode::Hosted` only): edits wait in
    /// [`EngineHost::take_outgoing`] for the host to compile, and the
    /// host's own thread drops retired graphs, bakes, and runs the
    /// transport. The editor's handle is a stand-in between the frames the
    /// host lends it its own (`EditorApp::with_engine`).
    pub fn keep_in_host(&mut self) {
        if self.hosted.is_some() {
            self.in_host = true;
        }
    }

    /// Whether the host keeps the project ([`EngineHost::keep_in_host`]).
    pub fn in_host(&self) -> bool {
        self.in_host
    }

    /// The project the engine should run now, if the document changed in a
    /// way the engine hears since the last call (while the host keeps it).
    pub fn take_outgoing(&mut self) -> Option<Project> {
        self.outgoing.take()
    }

    /// Swap `handle` (the host's, lent for a frame) with the editor's own,
    /// and take the host's rate and block size from it. Swapping again
    /// gives it back.
    pub(crate) fn swap_handle(&mut self, handle: &mut EngineHandle, follow_rate: bool) {
        std::mem::swap(&mut self.handle, handle);
        if !follow_rate {
            return;
        }
        let rate = self.handle.options().sample_rate;
        let block = self.handle.options().max_block;
        if let Some(h) = self.hosted.as_mut() {
            let rate = rate.round().clamp(1.0, f64::from(u32::MAX)) as u32;
            if h.sample_rate != rate || h.max_block != block {
                h.sample_rate = rate;
                h.max_block = block;
                self.info = h.info();
            }
        }
    }

    /// Whether Play keys reaches the engine: a host running the processor
    /// has to take the keys' events ([`EngineHost::take_control_receiver`]).
    pub fn keys_reach_engine(&self) -> bool {
        self.hosted.is_none() || self.idle_ports.controls.is_none()
    }

    fn adopt(&mut self, handle: EngineHandle, report: CompileReport, audio: HostAudio) {
        self.handle = handle;
        self.report = report;
        self.info = audio.info();
        self.hosted = Some(audio);
        self.idle = None;
        // The host's audio thread doesn't play previews.
        self.audition = None;
        self.mode_off = false;
        self.device_error = None;
        self.latency = LatencyState::Idle;
    }

    /// Fresh control rings: the computer keys, gamepads (with a device
    /// runtime), and OSC if it was listening, all on the new rings.
    fn new_controls(&mut self) -> ControlReceiver {
        // Old sources first, so OSC can bind its port again.
        self.keys = None;
        self.gamepads = None;
        let osc = self.osc_in.take().map(|l| l.port());
        let (mut inputs, rx) = ControlInputs::new();
        self.keys = inputs.sender("computer keys").ok();
        if self.device_sources {
            self.gamepads = inputs.sender("gamepads").ok().map(Gamepads::start);
        }
        self.controls = Some(inputs);
        if osc.is_some() {
            self.set_osc_in(osc);
        }
        rx
    }

    fn new_audition(&mut self) -> ac_io::AuditionTap {
        let (player, tap) = ac_io::audition();
        self.audition = Some(player);
        tap
    }

    /// What plays block previews over the patch, and the rate it plays
    /// at. `None` when a host runs the processor.
    pub fn audition(&mut self) -> Option<(&mut ac_io::Audition, u32)> {
        let rate = self.info.sample_rate;
        self.audition.as_mut().map(|a| (a, rate))
    }

    /// The id of the preview playing, if one is.
    pub fn audition_playing(&self) -> Option<u64> {
        self.audition.as_ref().and_then(ac_io::Audition::playing)
    }

    fn new_osc_output(&mut self) -> ac_io::OscOutQueue {
        let (out, queue) = OscOutput::new();
        self.osc_out = Some(out);
        queue
    }

    /// Listen for OSC on `port` (`None` or 0: stop listening). A port that
    /// can't be opened is reported in `osc_error`.
    pub fn set_osc_in(&mut self, port: Option<u16>) {
        // Close the old socket first, so the same port can be bound again.
        self.osc_in = None;
        self.osc_error = None;
        let Some(port) = port.filter(|p| *p != 0) else {
            return;
        };
        self.osc_port = port;
        let Some(inputs) = self.controls.as_mut() else {
            self.osc_error = Some("control input isn't running".into());
            return;
        };
        let listener = inputs
            .sender("osc")
            .map_err(|e| e.to_string())
            .and_then(|s| {
                OscInput::listen(port, s)
                    .map_err(|e| format!("couldn't listen on UDP port {port}: {e}"))
            });
        match listener {
            Ok(l) => self.osc_in = Some(l),
            Err(e) => self.osc_error = Some(e),
        }
    }

    /// Send a computer key going down or up to the engine (Play keys).
    pub fn send_key(&mut self, key: Key, down: bool) {
        if let Some(k) = self.keys.as_mut() {
            k.send(Control::Key(key), if down { 1.0 } else { 0.0 });
            self.keys_sent += 1;
        }
    }

    /// Key events sent to the engine so far.
    pub fn keys_sent(&self) -> u64 {
        self.keys_sent
    }

    fn new_midi_inputs(&mut self) -> ac_io::MidiReceiver {
        let (mut inputs, rx) = MidiInputs::new();
        inputs.connect(MidiSelect::All);
        self.midi = Some(inputs);
        rx
    }

    fn new_midi_outputs(&mut self) -> ac_io::MidiOutQueue {
        let (mut outputs, queue) = MidiOutputs::new();
        if self.midi_out_choice != MidiSelect::None {
            outputs.connect(self.midi_out_choice.clone());
        }
        self.midi_out = Some(outputs);
        queue
    }

    /// A virtual MIDI source feeding the engine like a port (an on-screen
    /// controller, a test). `None` when nothing runs MIDI input.
    pub fn virtual_midi(&mut self, name: &str) -> Option<ac_io::MidiSender> {
        self.midi.as_mut()?.sender(name).ok()
    }

    /// Send MIDI Out nodes' messages to these ports from now on.
    pub fn set_midi_out(&mut self, choice: MidiSelect) {
        self.midi_out_choice = choice.clone();
        if let Some(m) = self.midi_out.as_mut() {
            m.connect(choice);
        }
    }

    /// Start `processor` on `host`; if it won't start, try the null stream
    /// at the same rate so the engine keeps running.
    fn run(
        &mut self,
        host: Box<dyn AudioHost>,
        mut processor: Processor,
        ports: EventPorts,
        project: &Project,
    ) {
        processor.set_workers(worker_pool(self.config.threads));
        let is_null = host.info().host == "null";
        match host.start_with(processor, ports) {
            Ok(s) => {
                self.info = s.info().clone();
                self.stream = Some(s);
            }
            Err(e) => {
                self.device_error = Some(e.error.to_string());
                let Some(back) = e.returned else {
                    return;
                };
                if is_null {
                    self.idle = Some(back.processor);
                    self.idle_ports = back.ports;
                    return;
                }
                let null = Box::new(null_host(project, self.info.sample_rate));
                match null.start_with(back.processor, back.ports) {
                    Ok(s) => {
                        self.info = s.info().clone();
                        self.stream = Some(s);
                    }
                    Err(e2) => {
                        if let Some(b) = e2.returned {
                            self.idle = Some(b.processor);
                            self.idle_ports = b.ports;
                        }
                    }
                }
            }
        }
    }

    /// Switch to another device setup. At the same rate the processor
    /// moves over as is; at another rate the engine is rebuilt.
    pub fn switch(
        &mut self,
        config: AudioConfig,
        project: &Project,
        registry: Arc<Registry>,
        resources: Arc<dyn Resources>,
    ) {
        if self.hosted.is_some() {
            // The Devices tab offers no devices then; nothing to switch.
            self.device_error = Some("the host provides audio".into());
            return;
        }
        let host = match CpalAudio::open(&config) {
            Ok(h) => h,
            Err(e) => {
                self.device_error = Some(e.to_string());
                return;
            }
        };
        self.config = config;
        self.device_error = None;
        self.latency = LatencyState::Idle;
        let stopped = self.stream.take().and_then(|s| s.stop());
        let (processor, mut ports) = match stopped {
            Some(s) => (Some(s.processor), s.ports),
            None => (self.idle.take(), std::mem::take(&mut self.idle_ports)),
        };
        let first_device = !self.device_sources;
        self.device_sources = true;
        if ports.midi_in.is_none() {
            ports.midi_in = Some(self.new_midi_inputs());
        } else if first_device && let Some(m) = self.midi.as_mut() {
            // The null stream's virtual-only MIDI now takes the ports too.
            m.connect(MidiSelect::All);
        }
        if ports.midi_out.is_none() {
            ports.midi_out = Some(self.new_midi_outputs());
        }
        if ports.osc_out.is_none() {
            ports.osc_out = Some(self.new_osc_output());
        }
        if ports.controls.is_none() {
            ports.controls = Some(self.new_controls());
        } else if first_device && let Some(inputs) = self.controls.as_mut() {
            self.gamepads = inputs.sender("gamepads").ok().map(Gamepads::start);
        }
        if ports.audition.is_none() {
            ports.audition = Some(self.new_audition());
        }
        let same_rate = host.info().sample_rate == self.info.sample_rate;
        let processor = match processor {
            Some(p) if same_rate => p,
            _ => {
                // A new rate (or the processor didn't come back): rebuild.
                let info = host.info().clone();
                let (handle, p, report) = engine_at(project, registry, resources, &info);
                self.handle = handle;
                self.report = report;
                self.info = info;
                p
            }
        };
        self.mode_off = false;
        self.run(Box::new(host), processor, ports, project);
    }

    /// Recompile with the editor's project (state carries over). While the
    /// host keeps the project, it waits for the host instead.
    pub fn replace(&mut self, project: &Project, revision: u64) {
        if self.in_host {
            self.outgoing = Some(project.clone());
        } else {
            self.report = self
                .handle
                .edit(GraphEdit::Replace(Box::new(project.clone())));
        }
        self.synced_revision = revision;
    }

    /// What the host's compile of the outgoing project found. `None`: the
    /// host has no engine running (a plugin the host hasn't activated), so
    /// the editor compiles `project` on its stand-in engine to show the
    /// problems anyway.
    pub fn host_compiled(&mut self, report: Option<CompileReport>, project: &Project) {
        self.report = match report {
            Some(r) => r,
            None => self
                .handle
                .edit(GraphEdit::Replace(Box::new(project.clone()))),
        };
    }

    /// Whether a whole-input node at `path` is being baked right now.
    pub fn bake_state(&self, path: &ac_core::NodePath) -> Option<ac_engine::WholeState> {
        self.handle
            .bakes()
            .iter()
            .find(|b| &b.path == path)
            .map(|b| b.state.clone())
    }

    /// New assets or recorder: rebuild nodes with them.
    pub fn set_resources(&mut self, project: &Project, resources: Arc<dyn Resources>) {
        if self.in_host {
            // The host's engine reads the host's own copies of the files.
            self.outgoing = Some(project.clone());
            return;
        }
        // The handle recompiles from its own copy of the project; make it current first.
        let _ = self
            .handle
            .edit(GraphEdit::Replace(Box::new(project.clone())));
        self.report = self.handle.set_resources(resources);
    }

    /// Per-frame upkeep: drop retired graphs and MIDI ports, rescan MIDI
    /// now and then, bake whole-input nodes, and pick up a latency result.
    pub fn tick(&mut self) {
        // The host looks after its own engine (a plugin's control thread).
        if !self.in_host {
            self.handle.collect_garbage();
            if self.baker.poll(&mut self.handle) {
                self.report = self.handle.report().clone();
            }
        }
        if let Some(m) = self.midi.as_mut() {
            m.collect_garbage();
        }
        if let Some(c) = self.controls.as_mut() {
            c.collect_garbage();
        }
        if let Some(a) = self.audition.as_mut() {
            a.collect_garbage();
        }
        // Only device setups have ports to find (null MIDI is virtual).
        if self.device_sources && self.last_rescan.elapsed() > MIDI_RESCAN {
            self.rescan_midi();
        }
        if self.latency == LatencyState::Running
            && let Some(s) = self.stream.as_mut()
            && let Some(r) = s.poll_latency()
        {
            self.latency = LatencyState::Done(r);
        }
    }

    pub fn rescan_midi(&mut self) {
        if let Some(m) = self.midi.as_mut() {
            m.rescan();
        }
        if self.midi_out_choice != MidiSelect::None
            && let Some(m) = self.midi_out.as_mut()
        {
            m.rescan();
        }
        self.last_rescan = Instant::now();
    }

    pub fn measure_latency(&mut self) {
        let Some(s) = self.stream.as_mut() else {
            self.latency = LatencyState::Done(Err(LatencyError::Unsupported));
            return;
        };
        self.latency = match s.start_latency() {
            Ok(()) => LatencyState::Running,
            Err(e) => LatencyState::Done(Err(e)),
        };
    }

    /// Whether the engine runs on the silent null stream (or nothing runs
    /// it). A host running the processor is not null.
    pub fn is_null(&self) -> bool {
        self.hosted.is_none() && self.stream.as_ref().is_none_or(|s| s.info().host == "null")
    }

    /// Whether something runs the processor: a stream, or the host.
    pub fn is_running(&self) -> bool {
        self.stream.is_some() || self.hosted.is_some()
    }

    pub fn is_off(&self) -> bool {
        self.mode_off
    }

    pub fn stats(&self) -> Option<StreamStats> {
        self.stream.as_ref().map(|s| s.stats())
    }

    /// A one-line description: "no device (null) · 48000 Hz · 256 frames".
    pub fn describe(&self) -> String {
        let i = &self.info;
        let rate = |dev: String| {
            tr_args(
                "audio-describe",
                &[
                    ("device", dev.into()),
                    ("rate", i.sample_rate.to_string().into()),
                    ("frames", i.buffer_size.to_string().into()),
                ],
            )
        };
        if let Some(h) = &self.hosted {
            return rate(h.host.clone());
        }
        if self.stream.is_none() {
            return tr("audio-off");
        }
        let dev = if self.is_null() {
            tr("devices-null")
        } else {
            format!(
                "{} · {}",
                i.host,
                i.output_device
                    .clone()
                    .unwrap_or_else(|| tr("audio-default-output"))
            )
        };
        rate(dev)
    }

    /// Run the idle processor for one block (tests drive the engine this way
    /// when no stream runs it), with the control events sent since the last
    /// one (Play keys, gamepads, OSC).
    pub fn tick_idle(&mut self, frames: usize) {
        let Some(p) = self.idle.as_mut() else {
            return;
        };
        let rate = f64::from(self.info.sample_rate);
        match self.idle_ports.controls.as_mut() {
            Some(c) => c.drain(
                ac_io::clock::now_ns(),
                frames,
                rate,
                &mut self.idle_controls,
            ),
            None => self.idle_controls.clear(),
        }
        let ch = self.info.out_channels.max(1);
        let mut out = vec![0.0f32; ch * frames];
        let mut io = HostIo {
            frames,
            input: &[],
            in_channels: 0,
            output: &mut out,
            out_channels: ch,
            midi: &[],
            midi_out: None,
            controls: &self.idle_controls,
            osc_out: None,
        };
        p.process(&mut io);
        if let Some(a) = self.idle_ports.audition.as_mut() {
            a.mix(&mut out, ch, frames);
        }
        self.handle.collect_garbage();
    }
}

impl Drop for EngineHost {
    fn drop(&mut self) {
        if let Some(s) = self.stream.take() {
            drop(s.stop());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_stream_runs_the_engine() {
        let registry = Arc::new(ac_nodes::registry());
        let mut host = EngineHost::start(
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
            &AudioMode::Null,
        );
        assert!(host.is_null() && host.is_running());
        host.handle.play();
        let t0 = Instant::now();
        while host.stats().is_some_and(|s| s.callbacks < 3) && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(Duration::from_millis(5));
            host.tick();
        }
        assert!(host.stats().unwrap().callbacks >= 3);
        assert!(host.handle.is_playing());
        assert!(host.describe().contains("no device (null)"));
        host.measure_latency();
        assert!(matches!(
            host.latency,
            LatencyState::Done(Err(_)) | LatencyState::Running
        ));
    }

    #[test]
    fn keys_reach_the_idle_engine_and_osc_can_listen_and_stop() {
        let registry = Arc::new(ac_nodes::registry());
        let mut host = EngineHost::start(
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
            &AudioMode::Off,
        );
        assert!(host.gamepads.is_none(), "no gamepad thread without devices");
        host.send_key(Key::from_name("A").unwrap(), true);
        assert_eq!(host.keys_sent(), 1);
        // The idle tick drains the keys' ring into the engine.
        host.tick_idle(64);
        let mut rest = Vec::with_capacity(8);
        let rx = host.idle_ports.controls.as_mut().unwrap();
        rx.drain(ac_io::clock::now_ns(), 64, 48_000.0, &mut rest);
        assert!(rest.is_empty(), "already delivered");
        assert_eq!(host.idle_controls.len(), 1);
        assert_eq!(host.idle_controls[0].value, 1.0);
        // Port 0 asks the system for a free one; then stop.
        host.set_osc_in(Some(0));
        assert!(host.osc_in.is_none(), "0 means off");
        let free = std::net::UdpSocket::bind("0.0.0.0:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        host.set_osc_in(Some(free));
        assert!(host.osc_error.is_none(), "{:?}", host.osc_error);
        assert_eq!(host.osc_in.as_ref().map(|l| l.port()), Some(free));
        host.set_osc_in(None);
        assert!(host.osc_in.is_none());
        assert_eq!(host.osc_port, free, "the port is remembered");
    }

    #[test]
    fn hosted_mode_opens_nothing_and_hands_out_the_processor() {
        let registry = Arc::new(ac_nodes::registry());
        let audio = HostAudio {
            host: "DAW".into(),
            sample_rate: 44_100,
            max_block: 100,
            in_channels: 0,
            out_channels: 2,
        };
        let mut host = EngineHost::start(
            &Project::default(),
            registry.clone(),
            Arc::new(ac_core::NoResources),
            &AudioMode::Hosted(audio.clone()),
        );
        assert_eq!(host.hosted(), Some(&audio));
        assert!(host.midi.is_none() && host.midi_out.is_none() && host.gamepads.is_none());
        assert!(host.osc_out.is_none() && host.stats().is_none());
        assert!(host.is_running() && !host.is_null() && !host.is_off());
        assert_eq!(host.describe(), "DAW · 44100 Hz · 100 frames");
        assert_eq!(host.handle.options().sample_rate, 44_100.0);
        assert_eq!(host.handle.options().max_block, 100);
        // Switching devices is the host's business.
        host.switch(
            AudioConfig::default(),
            &Project::default(),
            registry.clone(),
            Arc::new(ac_core::NoResources),
        );
        assert!(host.hosted().is_some() && host.stats().is_none());
        // The host takes the processor and the keys' control events.
        let mut p = host
            .take_processor()
            .expect("the processor waits for the host");
        assert!(host.take_processor().is_none());
        let mut keys = host.take_control_receiver().expect("keys for the host");
        host.send_key(Key::from_name("A").unwrap(), true);
        let mut events = Vec::with_capacity(8);
        keys.drain(ac_io::clock::now_ns(), 64, 44_100.0, &mut events);
        assert_eq!(events.len(), 1);
        host.handle.play();
        let mut out = vec![0.0f32; 2 * 64];
        let mut io = HostIo {
            frames: 64,
            input: &[],
            in_channels: 0,
            output: &mut out,
            out_channels: 2,
            midi: &[],
            midi_out: None,
            controls: &events,
            osc_out: None,
        };
        p.process(&mut io);
        assert_eq!(host.handle.position(), 64);
        // A new rate: a new engine, and its processor for the host.
        let p2 = host.restart_hosted(
            HostAudio {
                sample_rate: 96_000,
                ..audio
            },
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
        );
        assert!(p2.is_some());
        assert_eq!(host.info.sample_rate, 96_000);
        assert_eq!(host.handle.position(), 0, "the transport starts over");
    }

    #[test]
    fn idle_mode_can_be_ticked() {
        let registry = Arc::new(ac_nodes::registry());
        let mut host = EngineHost::start(
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
            &AudioMode::Off,
        );
        host.handle.play();
        host.tick_idle(64);
        assert_eq!(host.handle.position(), 64);
        assert!(!host.is_running());
    }
}
