//! Running the engine live: the control half ([`EngineHandle`]) lives with
//! the editor; the audio half ([`Processor`]) runs in an `ac-io` stream on
//! the chosen audio device, or on the paced null stream when there is no
//! device (this falls back silently, so the editor always runs).
//!
//! Switching devices at the same rate moves the processor to the new stream
//! without rebuilding the engine; a different rate rebuilds the engine at
//! that rate (the transport and node state start over).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ac_core::{Project, Registry, Resources};
use ac_engine::{CompileReport, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};
use ac_io::{
    AudioConfig, AudioHost, AudioStream, Baker, CpalAudio, Latency, LatencyError, MidiInputs,
    MidiReceiver, MidiSelect, NullAudio, StreamInfo, StreamStats, worker_pool,
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
            AudioMode::Off => (None, AudioConfig::default()),
        };
        let info = host
            .as_ref()
            .map(|h| h.info().clone())
            .unwrap_or_else(|| null_host(project, NULL_RATE).info().clone());
        let (handle, processor, report) = engine_at(project, registry, resources, &info);
        let mut me = EngineHost {
            handle,
            stream: None,
            idle: None,
            midi: None,
            last_rescan: Instant::now(),
            report,
            info,
            config,
            device_error,
            latency: LatencyState::Idle,
            synced_revision: 0,
            mode_off: host.is_none(),
            baker: Baker::new(),
        };
        let midi_rx = if matches!(mode, AudioMode::Device(_)) {
            let (mut inputs, rx) = MidiInputs::new();
            inputs.connect(MidiSelect::All);
            me.midi = Some(inputs);
            Some(rx)
        } else {
            None
        };
        match host {
            Some(h) => me.run(h, processor, midi_rx, project),
            None => me.idle = Some(processor),
        }
        me
    }

    /// Start `processor` on `host`; if it won't start, try the null stream
    /// at the same rate so the engine keeps running.
    fn run(
        &mut self,
        host: Box<dyn AudioHost>,
        mut processor: Processor,
        midi: Option<MidiReceiver>,
        project: &Project,
    ) {
        processor.set_workers(worker_pool(self.config.threads));
        let is_null = host.info().host == "null";
        match host.start(processor, midi) {
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
                    return;
                }
                let null = Box::new(null_host(project, self.info.sample_rate));
                match null.start(back.processor, back.midi) {
                    Ok(s) => {
                        self.info = s.info().clone();
                        self.stream = Some(s);
                    }
                    Err(e2) => {
                        if let Some(b) = e2.returned {
                            self.idle = Some(b.processor);
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
        let (processor, midi) = match stopped {
            Some(s) => (Some(s.processor), s.midi),
            None => (self.idle.take(), None),
        };
        let midi = midi.or_else(|| {
            let (mut inputs, rx) = MidiInputs::new();
            inputs.connect(MidiSelect::All);
            self.midi = Some(inputs);
            Some(rx)
        });
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
        self.run(Box::new(host), processor, midi, project);
    }

    /// Recompile with the editor's project (state carries over).
    pub fn replace(&mut self, project: &Project, revision: u64) {
        self.report = self
            .handle
            .edit(GraphEdit::Replace(Box::new(project.clone())));
        self.synced_revision = revision;
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
        // The handle recompiles from its own copy of the project; make it current first.
        let _ = self
            .handle
            .edit(GraphEdit::Replace(Box::new(project.clone())));
        self.report = self.handle.set_resources(resources);
    }

    /// Per-frame upkeep: drop retired graphs and MIDI ports, rescan MIDI
    /// now and then, bake whole-input nodes, and pick up a latency result.
    pub fn tick(&mut self) {
        self.handle.collect_garbage();
        if self.baker.poll(&mut self.handle) {
            self.report = self.handle.report().clone();
        }
        if let Some(m) = self.midi.as_mut() {
            m.collect_garbage();
            if self.last_rescan.elapsed() > MIDI_RESCAN {
                m.rescan();
                self.last_rescan = Instant::now();
            }
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
            self.last_rescan = Instant::now();
        }
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

    pub fn is_null(&self) -> bool {
        self.stream.as_ref().is_none_or(|s| s.info().host == "null")
    }

    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }

    pub fn is_off(&self) -> bool {
        self.mode_off
    }

    pub fn stats(&self) -> Option<StreamStats> {
        self.stream.as_ref().map(|s| s.stats())
    }

    /// A one-line description: "no device (null) · 48000 Hz · 256 frames".
    pub fn describe(&self) -> String {
        if self.stream.is_none() {
            return "audio off".into();
        }
        let i = &self.info;
        let dev = if self.is_null() {
            "no device (null)".to_string()
        } else {
            format!(
                "{} · {}",
                i.host,
                i.output_device.as_deref().unwrap_or("default output")
            )
        };
        format!("{dev} · {} Hz · {} frames", i.sample_rate, i.buffer_size)
    }

    /// Run the idle processor for one block (tests drive the engine this way
    /// when no stream runs it).
    pub fn tick_idle(&mut self, frames: usize) {
        let Some(p) = self.idle.as_mut() else {
            return;
        };
        let ch = self.info.out_channels.max(1);
        let mut out = vec![0.0f32; ch * frames];
        let mut io = HostIo {
            frames,
            input: &[],
            in_channels: 0,
            output: &mut out,
            out_channels: ch,
            midi: &[],
        };
        p.process(&mut io);
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
