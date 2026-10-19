//! Audio Constructor device and file layer: WAV files, project files, asset
//! loading, audio and MIDI devices, control surfaces (computer keys,
//! gamepads, OSC), the real-time runtime around the engine, recording, and
//! latency measurement. The engine never touches files or devices; hosts
//! use this crate to feed it.
//!
//! Control surfaces feed a running stream the way MIDI does: make a
//! [`ControlInputs`], hand its [`ControlReceiver`] to the stream in
//! [`EventPorts::controls`], and give each source a [`ControlSender`]
//! ([`Gamepads::start`], [`OscInput::listen`], or your own, like the
//! editor's computer keys). OSC Out nodes send through an [`OscOutput`]
//! whose queue goes in [`EventPorts::osc_out`].
//!
//! Running a project live:
//!
//! ```no_run
//! use std::sync::Arc;
//! use ac_engine::{Engine, EngineOptions};
//! use ac_io::{AudioConfig, AudioHost, CpalAudio, MidiInputs, MidiSelect};
//! # fn run(project: ac_core::Project, registry: Arc<ac_core::Registry>) -> Result<(), Box<dyn std::error::Error>> {
//! let host = Box::new(CpalAudio::open(&AudioConfig::default())?);
//! let info = host.info().clone();
//! let (engine, _report) = Engine::new(
//!     project,
//!     registry,
//!     Arc::new(ac_core::NoResources),
//!     EngineOptions { sample_rate: f64::from(info.sample_rate), max_block: info.max_block(), socket_telemetry: true, offline: false },
//! );
//! let (mut handle, processor) = engine.split();
//! let (mut midi, receiver) = MidiInputs::new();
//! midi.connect(MidiSelect::All);
//! let stream = host.start(processor, Some(receiver))?;
//! handle.play();
//! // ... call handle.collect_garbage() regularly; stream.stats(), handle.status() for telemetry.
//! # drop(stream); Ok(()) }
//! ```
//!
//! Backends: ALSA, CoreAudio, and WASAPI are always built. The `jack`
//! feature adds JACK; the `asio` feature (off by default) adds ASIO on
//! Windows, which needs the Steinberg ASIO SDK at build time (set
//! `CPAL_ASIO_DIR`) and a license from Steinberg to distribute binaries.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod assets;
pub mod audio;
pub mod clock;
pub mod control;
pub mod convert;
pub mod engine_threads;
pub mod gamepad;
pub mod latency;
pub mod library;
pub mod midi;
pub mod midi_file;
pub mod midi_out;
pub mod monitor;
#[cfg(not(target_arch = "wasm32"))]
pub mod net;
pub mod null;
pub mod osc;
pub mod osc_out;
pub mod pack;
pub mod priority;
pub mod project_file;
pub mod record;
pub mod rings;
pub mod runtime;
pub mod wav;
pub mod zip;

pub use assets::{AssetStore, FileResources, RecorderFactory};
pub use audio::{
    AudioConfig, CpalAudio, DeviceInfo, HostInfo, InputChoice, RunningAudio, enumerate,
};
pub use control::{ControlError, ControlInputs, ControlReceiver, ControlSender};
pub use engine_threads::{BAKE_DEBOUNCE, Baker, available_threads, worker_pool};
pub use gamepad::{GamepadError, GamepadInfo, Gamepads, list_gamepads};
pub use latency::{Latency, LatencyError, find_delay};
pub use library::{Library, LibraryBlock, Update, shipped_blocks};
pub use midi::{
    MidiInputs, MidiPortInfo, MidiReceiver, MidiSeen, MidiSelect, MidiSender, block_offset,
};
pub use midi_file::{midi_file_bytes, write_midi_file};
pub use midi_out::{MidiOutQueue, MidiOutputs, list_outputs};
pub use monitor::{RECENT_MESSAGES, Recent};
pub use null::{NullAudio, NullHost, NullStream};
pub use osc::{DEFAULT_OSC_PORT, OscInput, OscSeen, OscStats};
pub use osc_out::{OscOutQueue, OscOutput};
pub use pack::{PACK_EXTENSION, PackReport, unpack, unpack_beside, write_pack};
pub use project_file::{load_project, save_project};
pub use record::{DiskRecorder, Take};
pub use runtime::{
    AudioError, AudioHost, AudioStream, EventPorts, StartError, Stopped, StreamInfo, StreamStats,
};
pub use wav::{BitDepth, read_wav, write_wav};

// Unit tests run under the allocation guard, so a real-time path that
// allocates (inside `assert_no_alloc`) aborts the test run.
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
