//! Audio Constructor device and file layer: WAV files, project files, asset
//! loading, MIDI input, recording, and latency measurement. The engine never
//! touches files or devices; hosts use this crate to feed it.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod assets;
pub mod clock;
pub mod convert;
pub mod latency;
pub mod midi;
pub mod project_file;
pub mod record;
pub mod wav;

pub use assets::{AssetStore, FileResources, RecorderFactory};
pub use latency::{Latency, LatencyError, find_delay};
pub use midi::{MidiInputs, MidiPortInfo, MidiReceiver, MidiSelect, MidiSender, block_offset};
pub use project_file::{load_project, save_project};
pub use record::{DiskRecorder, Take};
pub use wav::{BitDepth, read_wav, write_wav};

// Unit tests run under the allocation guard, so a real-time path that
// allocates (inside `assert_no_alloc`) aborts the test run.
#[cfg(all(test, debug_assertions))]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;
