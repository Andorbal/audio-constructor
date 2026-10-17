//! Running the engine: the control half ([`EngineHandle`]) lives with the
//! editor; the audio half ([`Processor`]) runs on a backend's thread.
//!
//! Until the device runtime lands, the only backend is [`NoDevice`]: a
//! thread that calls the processor every buffer period with silent input,
//! paced by the clock, so meters, scopes, and the transport behave as they
//! will with a device. A device backend implements [`AudioBackend`] and
//! takes the processor the same way.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ac_core::{Project, Registry, Resources};
use ac_engine::{CompileReport, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};

/// What the audio side runs at.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioSettings {
    pub sample_rate: u32,
    pub buffer_frames: usize,
    pub in_channels: usize,
    pub out_channels: usize,
}

impl Default for AudioSettings {
    fn default() -> Self {
        AudioSettings {
            sample_rate: 48_000,
            buffer_frames: 256,
            in_channels: 2,
            out_channels: 2,
        }
    }
}

/// What the Devices tab shows about the running backend.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendStatus {
    /// Backend name ("no device (null)", or the device host's name).
    pub name: String,
    /// The device in use, if any.
    pub device: Option<String>,
    pub sample_rate: u32,
    pub buffer_frames: usize,
    /// Callbacks run so far.
    pub callbacks: u64,
    /// Callbacks that started late (the pacing thread fell behind).
    pub late: u64,
    pub error: Option<String>,
}

/// A running audio backend. Dropping it stops the audio thread.
pub trait AudioBackend: Send {
    fn status(&self) -> BackendStatus;
}

/// How the editor drives the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioMode {
    /// The real-time-paced silent backend.
    NoDevice,
    /// Nothing runs the processor (tests). The engine still compiles.
    Off,
}

/// Silent, clock-paced backend (see the module docs).
pub struct NoDevice {
    stop: Arc<AtomicBool>,
    callbacks: Arc<AtomicU64>,
    late: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
    settings: AudioSettings,
}

fn clock_ns() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

impl NoDevice {
    pub fn start(mut processor: Processor, settings: AudioSettings) -> NoDevice {
        let stop = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicU64::new(0));
        let late = Arc::new(AtomicU64::new(0));
        let (s, c, l) = (stop.clone(), callbacks.clone(), late.clone());
        let cfg = settings.clone();
        processor.set_clock(Some(clock_ns));
        let thread = std::thread::Builder::new()
            .name("audio (no device)".into())
            .spawn(move || {
                let frames = cfg.buffer_frames.max(1);
                let input = vec![0.0f32; cfg.in_channels * frames];
                let mut output = vec![0.0f32; cfg.out_channels.max(1) * frames];
                let period =
                    Duration::from_secs_f64(frames as f64 / f64::from(cfg.sample_rate.max(1)));
                let mut next = Instant::now();
                while !s.load(Ordering::Relaxed) {
                    let mut io = HostIo {
                        frames,
                        input: &input,
                        in_channels: cfg.in_channels,
                        output: &mut output,
                        out_channels: cfg.out_channels.max(1),
                        midi: &[],
                    };
                    processor.process(&mut io);
                    c.fetch_add(1, Ordering::Relaxed);
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > period * 8 {
                        // Far behind (the machine was suspended, or a debugger
                        // stopped us): start pacing afresh rather than racing.
                        l.fetch_add(1, Ordering::Relaxed);
                        next = now;
                    }
                }
            })
            .ok();
        NoDevice {
            stop,
            callbacks,
            late,
            thread,
            settings,
        }
    }
}

impl AudioBackend for NoDevice {
    fn status(&self) -> BackendStatus {
        BackendStatus {
            name: "no device (null)".into(),
            device: None,
            sample_rate: self.settings.sample_rate,
            buffer_frames: self.settings.buffer_frames,
            callbacks: self.callbacks.load(Ordering::Relaxed),
            late: self.late.load(Ordering::Relaxed),
            error: self
                .thread
                .is_none()
                .then(|| "couldn't start the audio thread".to_string()),
        }
    }
}

impl Drop for NoDevice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The engine as the editor sees it: the handle, the backend running the
/// processor, and the last compile report.
pub struct EngineHost {
    pub handle: EngineHandle,
    backend: Option<Box<dyn AudioBackend>>,
    /// The processor, when no backend runs it (`AudioMode::Off`).
    idle_processor: Option<Processor>,
    pub report: CompileReport,
    pub settings: AudioSettings,
    /// Document engine revision last sent.
    pub synced_revision: u64,
}

impl EngineHost {
    pub fn start(
        project: &Project,
        registry: Arc<Registry>,
        resources: Arc<dyn Resources>,
        mode: AudioMode,
    ) -> EngineHost {
        let settings = AudioSettings {
            out_channels: ac_engine::required_output_channels(project),
            ..AudioSettings::default()
        };
        let (engine, report) = Engine::new(
            project.clone(),
            registry,
            resources,
            EngineOptions {
                sample_rate: f64::from(settings.sample_rate),
                max_block: settings.buffer_frames.max(1024),
                socket_telemetry: true,
            },
        );
        let (handle, processor) = engine.split();
        let (backend, idle_processor): (Option<Box<dyn AudioBackend>>, _) = match mode {
            AudioMode::NoDevice => (
                Some(Box::new(NoDevice::start(processor, settings.clone()))),
                None,
            ),
            AudioMode::Off => (None, Some(processor)),
        };
        EngineHost {
            handle,
            backend,
            idle_processor,
            report,
            settings,
            synced_revision: 0,
        }
    }

    /// Recompile with the editor's project (state carries over).
    pub fn replace(&mut self, project: &Project, revision: u64) {
        self.report = self
            .handle
            .edit(GraphEdit::Replace(Box::new(project.clone())));
        self.synced_revision = revision;
    }

    /// New assets: rebuild nodes with them.
    pub fn set_resources(&mut self, project: &Project, resources: Arc<dyn Resources>) {
        // The handle recompiles from its own copy of the project; make it current first.
        let _ = self
            .handle
            .edit(GraphEdit::Replace(Box::new(project.clone())));
        self.report = self.handle.set_resources(resources);
    }

    pub fn backend_status(&self) -> Option<BackendStatus> {
        self.backend.as_ref().map(|b| b.status())
    }

    /// Run the idle processor for one block (tests drive the engine this way
    /// when no backend runs it).
    pub fn tick_idle(&mut self, frames: usize) {
        let Some(p) = self.idle_processor.as_mut() else {
            return;
        };
        let ch = self.settings.out_channels.max(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_device_backend_runs_and_stops() {
        let registry = Arc::new(ac_nodes::registry());
        let mut host = EngineHost::start(
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
            AudioMode::NoDevice,
        );
        host.handle.play();
        let t0 = Instant::now();
        while host.backend_status().is_some_and(|s| s.callbacks < 3) && t0.elapsed().as_secs() < 5 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host.backend_status().unwrap().callbacks >= 3);
        assert!(host.handle.is_playing());
        drop(host);
    }

    #[test]
    fn idle_mode_can_be_ticked() {
        let registry = Arc::new(ac_nodes::registry());
        let mut host = EngineHost::start(
            &Project::default(),
            registry,
            Arc::new(ac_core::NoResources),
            AudioMode::Off,
        );
        host.handle.play();
        host.tick_idle(64);
        assert_eq!(host.handle.position(), 64);
        assert!(host.backend_status().is_none());
    }
}
