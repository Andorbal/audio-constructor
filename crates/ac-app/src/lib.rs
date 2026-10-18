//! The Audio Constructor desktop editor (docs/UX.md).

pub mod actions;
mod app;
pub mod audio;
pub mod bindings;
pub mod canvas;
pub mod dialogs;
pub mod document;
pub mod edit;
pub mod editors;
pub mod export;
pub mod files;
pub mod icons;
pub mod menubar;
pub mod palette;
pub mod panels;
pub mod params;
pub mod perform;
pub mod play_keys;
pub mod theme;
pub mod timing;
pub mod transport;
pub mod undo;

pub use app::{AppOptions, EditorApp};
