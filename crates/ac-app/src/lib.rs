//! The Audio Constructor editor (docs/UX.md). It has two hosts
//! (ARCHITECTURE §13): the desktop app, where it owns the engine and the
//! device layer (`desktop` feature, eframe), and a plugin, where the host
//! runs the engine's processor and the editor draws in the plugin's window
//! (`AudioMode::Hosted`, [`EditorApp::embedded`], and the `embed`
//! feature's window adapter). Both call [`EditorApp::ui`] once a frame.

pub mod actions;
mod app;
pub mod audio;
pub mod automation;
pub mod bindings;
pub mod canvas;
pub mod dialogs;
pub mod document;
pub mod edit;
pub mod editor_windows;
pub mod editors;
#[cfg(feature = "embed")]
pub mod embed;
pub mod export;
pub mod files;
pub mod find;
pub mod help;
pub mod host;
pub mod i18n;
pub mod icons;
pub mod menubar;
pub mod online;
pub mod palette;
pub mod panels;
pub mod params;
pub mod perform;
pub mod play_keys;
pub mod plugins;
#[cfg(not(target_arch = "wasm32"))]
pub mod product;
pub mod ruler;
pub mod settings;
pub mod sounding;
pub mod theme;
pub mod timing;
pub mod transport;
pub mod undo;
pub mod user_themes;

pub use app::{AppOptions, AudioPurpose, EditorApp, FileRequest, SaveKind, clock_seed};
pub use audio::HostAudio;
