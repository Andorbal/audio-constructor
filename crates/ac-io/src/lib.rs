//! Audio Constructor device and file layer: WAV files, project files, asset
//! loading, audio and MIDI devices, the real-time runtime around the
//! engine, recording, and latency measurement. The engine never touches
//! files or devices; hosts use this crate to feed it.
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
//!     EngineOptions { sample_rate: f64::from(info.sample_rate), max_block: info.max_block(), socket_telemetry: true },
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
pub mod convert;
pub mod latency;
pub mod midi;
pub mod null;
pub mod priority;
pub mod project_file;
pub mod record;
pub mod runtime;
pub mod wav;

pub use assets::{AssetStore, FileResources, RecorderFactory};
pub use audio::{
    AudioConfig, CpalAudio, DeviceInfo, HostInfo, InputChoice, RunningAudio, enumerate,
};
pub use latency::{Latency, LatencyError, find_delay};
pub use midi::{MidiInputs, MidiPortInfo, MidiReceiver, MidiSelect, MidiSender, block_offset};
pub use null::{NullAudio, NullHost, NullStream};
pub use project_file::{load_project, save_project};
pub use record::{DiskRecorder, Take};
pub use runtime::{AudioError, AudioHost, AudioStream, Stopped, StreamInfo, StreamStats};
pub use wav::{BitDepth, read_wav, write_wav};

// Unit tests run under the allocation guard, so a real-time path that
// allocates (inside `assert_no_alloc`) aborts the test run.
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
