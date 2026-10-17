//! Audio Constructor device and file layer: WAV files, project files, asset
//! loading, and (in the device modules) audio and MIDI backends. The engine
//! never touches files or devices; hosts use this crate to feed it.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod assets;
pub mod project_file;
pub mod wav;

pub use assets::{AssetStore, FileResources, RecorderFactory};
pub use project_file::{load_project, save_project};
pub use wav::{BitDepth, read_wav, write_wav};
