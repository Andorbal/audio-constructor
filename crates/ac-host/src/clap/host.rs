//! The host side of a CLAP instance: the callbacks a plugin calls, split
//! the way CLAP splits threads (DECISIONS D132).
//!
//! - [`ClapShared`] answers thread-safe calls (any thread, the audio
//!   thread included) with atomic flags only, so nothing a plugin calls on
//!   the audio thread allocates, locks, or blocks.
//! - [`ClapMain`] answers main-thread calls: timers and file descriptors
//!   the plugin wants serviced, which [`crate::registry`] polls.
//!
//! An instance's main thread is the thread that created it: the editor's
//! and `audioc`'s main thread for the running graph (both compile there),
//! or a bake's own thread for the copy that bake renders.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::ThreadId;

use clack_extensions::audio_ports::{
    AudioPortRescanFlags, HostAudioPorts, HostAudioPortsImpl, PluginAudioPorts,
};
use clack_extensions::gui::{GuiSize, HostGui, HostGuiImpl, PluginGui};
use clack_extensions::latency::{HostLatency, HostLatencyImpl, PluginLatency};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::note_ports::{
    HostNotePorts, HostNotePortsImpl, NoteDialects, NotePortRescanFlags, PluginNotePorts,
};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
    PluginParams,
};
#[cfg(unix)]
use clack_extensions::posix_fd::{FdFlags, HostPosixFd, HostPosixFdImpl, PluginPosixFd};
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_extensions::timer::{HostTimer, HostTimerImpl, PluginTimer, TimerId};
use clack_host::prelude::*;

thread_local! {
    /// Set while this thread calls a plugin's audio-thread functions: the
    /// thread is the audio thread for those calls, whichever it is (an
    /// offline render's is the main thread too).
    static AUDIO: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` as the audio thread (for the thread check).
pub(crate) fn as_audio_thread<R>(f: impl FnOnce() -> R) -> R {
    let was = AUDIO.with(|a| a.replace(true));
    let r = f();
    AUDIO.with(|a| a.set(was));
    r
}

/// The handler types of every CLAP instance this crate creates.
pub(crate) struct AcClap;

impl HostHandlers for AcClap {
    type Shared<'a> = ClapShared;
    type MainThread<'a> = ClapMain<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostThreadCheck>()
            .register::<HostLatency>()
            .register::<HostParams>()
            .register::<HostGui>()
            .register::<HostState>()
            .register::<HostTimer>()
            .register::<HostAudioPorts>()
            .register::<HostNotePorts>();
        #[cfg(unix)]
        builder.register::<HostPosixFd>();
    }
}

/// What a plugin asked for, from any thread, for the main thread to answer
/// in [`crate::poll`]. Every field is atomic, so setting one never blocks.
#[derive(Debug, Default)]
pub(crate) struct Requests {
    /// `request_callback`: run `on_main_thread`.
    pub callback: AtomicBool,
    /// `request_restart`, or ports or parameters that changed shape: the
    /// node has to be rebuilt around a new instance.
    pub restart: AtomicBool,
    /// Parameter values changed outside the audio thread (a preset).
    pub rescan_values: AtomicBool,
    /// The latency changed.
    pub latency: AtomicBool,
    /// The plugin's window was closed by the plugin or the person.
    pub gui_closed: AtomicBool,
    /// A size the plugin asked its window to be, packed (0: none).
    pub resize: AtomicU64,
    pub show: AtomicBool,
    pub hide: AtomicBool,
    /// The state changed and the project should be saved again.
    pub dirty: AtomicBool,
    /// The audio half is gone: the main half can be destroyed.
    pub orphaned: AtomicBool,
    /// Log messages dropped because they came from the audio thread.
    pub dropped_logs: AtomicU32,
}

/// Extensions the plugin offers, found while it initializes.
#[derive(Clone, Copy, Default)]
pub(crate) struct Extensions {
    pub params: Option<PluginParams>,
    pub gui: Option<PluginGui>,
    pub timer: Option<PluginTimer>,
    #[cfg(unix)]
    pub posix_fd: Option<PluginPosixFd>,
    pub latency: Option<PluginLatency>,
    pub state: Option<PluginState>,
    pub audio_ports: Option<PluginAudioPorts>,
    pub note_ports: Option<PluginNotePorts>,
}

pub(crate) struct ClapShared {
    pub owner: ThreadId,
    pub requests: Arc<Requests>,
    pub ext: OnceLock<Extensions>,
}

impl ClapShared {
    pub fn new(requests: Arc<Requests>) -> ClapShared {
        ClapShared {
            owner: std::thread::current().id(),
            requests,
            ext: OnceLock::new(),
        }
    }

    pub fn extensions(&self) -> Extensions {
        self.ext.get().copied().unwrap_or_default()
    }

    fn on_owner(&self) -> bool {
        std::thread::current().id() == self.owner
    }
}

impl<'a> SharedHandler<'a> for ClapShared {
    fn initializing(&self, instance: InitializingPluginHandle<'a>) {
        let _ = self.ext.set(Extensions {
            params: instance.get_extension(),
            gui: instance.get_extension(),
            timer: instance.get_extension(),
            #[cfg(unix)]
            posix_fd: instance.get_extension(),
            latency: instance.get_extension(),
            state: instance.get_extension(),
            audio_ports: instance.get_extension(),
            note_ports: instance.get_extension(),
        });
    }

    fn request_restart(&self) {
        self.requests.restart.store(true, Ordering::Release);
    }

    fn request_process(&self) {
        // The engine processes every block anyway.
    }

    fn request_callback(&self) {
        self.requests.callback.store(true, Ordering::Release);
    }
}

impl HostLogImpl for ClapShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        // Printing locks stderr; the audio thread mustn't, so its messages
        // are only counted.
        if !self.on_owner() || AUDIO.with(Cell::get) {
            self.requests.dropped_logs.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if matches!(
            severity,
            LogSeverity::Warning
                | LogSeverity::Error
                | LogSeverity::Fatal
                | LogSeverity::HostMisbehaving
                | LogSeverity::PluginMisbehaving
        ) {
            eprintln!("plugin: {severity}: {message}");
        }
    }
}

impl HostThreadCheckImpl for ClapShared {
    fn is_main_thread(&self) -> bool {
        self.on_owner()
    }

    fn is_audio_thread(&self) -> bool {
        AUDIO.with(Cell::get)
    }
}

impl HostGuiImpl for ClapShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        self.requests
            .resize
            .store(new_size.pack_to_u64().max(1), Ordering::Release);
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        self.requests.show.store(true, Ordering::Release);
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        self.requests.hide.store(true, Ordering::Release);
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.requests.gui_closed.store(true, Ordering::Release);
    }
}

impl HostParamsImplShared for ClapShared {
    fn request_flush(&self) {
        // Every block is processed, which flushes parameter changes.
    }
}

/// A timer a plugin registered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timer {
    pub id: u32,
    pub period_ms: u32,
    pub due: std::time::Instant,
}

pub(crate) struct ClapMain<'a> {
    pub shared: &'a ClapShared,
    pub timers: RefCell<Vec<Timer>>,
    next_timer: Cell<u32>,
    #[cfg(unix)]
    pub fds: RefCell<Vec<(i32, FdFlags)>>,
}

impl<'a> ClapMain<'a> {
    pub fn new(shared: &'a ClapShared) -> ClapMain<'a> {
        ClapMain {
            shared,
            timers: RefCell::new(Vec::new()),
            next_timer: Cell::new(0),
            #[cfg(unix)]
            fds: RefCell::new(Vec::new()),
        }
    }
}

impl<'a> MainThreadHandler<'a> for ClapMain<'a> {}

impl HostLatencyImpl for ClapMain<'_> {
    fn changed(&self) {
        self.shared.requests.latency.store(true, Ordering::Release);
    }
}

impl HostParamsImplMainThread for ClapMain<'_> {
    fn rescan(&self, flags: ParamRescanFlags) {
        if flags.requires_restart() || flags.contains(ParamRescanFlags::INFO) {
            self.shared.requests.restart.store(true, Ordering::Release);
        } else if flags.contains(ParamRescanFlags::VALUES) {
            self.shared
                .requests
                .rescan_values
                .store(true, Ordering::Release);
        }
    }

    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {
        // Nothing holds automation or modulation by parameter id.
    }
}

impl HostStateImpl for ClapMain<'_> {
    fn mark_dirty(&self) {
        self.shared.requests.dirty.store(true, Ordering::Release);
    }
}

impl HostTimerImpl for ClapMain<'_> {
    fn register_timer(&self, period_ms: u32) -> Result<TimerId, HostError> {
        let id = self.next_timer.get();
        self.next_timer.set(id.wrapping_add(1));
        // At least 30 Hz is what plugins can count on; faster than every
        // 10 ms isn't serviced.
        let period_ms = period_ms.max(10);
        self.timers.borrow_mut().push(Timer {
            id,
            period_ms,
            due: std::time::Instant::now() + std::time::Duration::from_millis(period_ms.into()),
        });
        Ok(TimerId(id))
    }

    fn unregister_timer(&self, timer_id: TimerId) -> Result<(), HostError> {
        let mut t = self.timers.borrow_mut();
        let before = t.len();
        t.retain(|x| x.id != timer_id.0);
        if t.len() == before {
            Err(HostError::Message("no such timer"))
        } else {
            Ok(())
        }
    }
}

#[cfg(unix)]
impl HostPosixFdImpl for ClapMain<'_> {
    fn register_fd(&self, fd: i32, flags: FdFlags) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        if fds.iter().any(|(f, _)| *f == fd) {
            return Err(HostError::Message("fd already registered"));
        }
        fds.push((fd, flags));
        Ok(())
    }

    fn modify_fd(&self, fd: i32, flags: FdFlags) -> Result<(), HostError> {
        match self.fds.borrow_mut().iter_mut().find(|(f, _)| *f == fd) {
            Some(e) => {
                e.1 = flags;
                Ok(())
            }
            None => Err(HostError::Message("fd not registered")),
        }
    }

    fn unregister_fd(&self, fd: i32) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        let before = fds.len();
        fds.retain(|(f, _)| *f != fd);
        if fds.len() == before {
            Err(HostError::Message("fd not registered"))
        } else {
            Ok(())
        }
    }
}

impl HostAudioPortsImpl for ClapMain<'_> {
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        // Any change rebuilds the node around a new instance.
        true
    }

    fn rescan(&self, _flags: AudioPortRescanFlags) {
        self.shared.requests.restart.store(true, Ordering::Release);
    }
}

impl HostNotePortsImpl for ClapMain<'_> {
    fn supported_dialects(&self) -> NoteDialects {
        NoteDialects::CLAP | NoteDialects::MIDI
    }

    fn rescan(&self, _flags: NotePortRescanFlags) {
        self.shared.requests.restart.store(true, Ordering::Release);
    }
}

/// The host's name and version as plugins see it.
pub(crate) fn host_info() -> Option<HostInfo> {
    HostInfo::new(
        "Audio Constructor",
        "Audio Constructor contributors",
        "https://github.com/Andorbal/audio-constructor",
        env!("CARGO_PKG_VERSION"),
    )
    .ok()
}
