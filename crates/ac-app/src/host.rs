//! What a host that keeps the project itself tells the editor, and what the
//! editor asks of it (ARCHITECTURE §13).
//!
//! A plugin keeps the project, its audio files, and the engine in its own
//! session, which the host saves and restores. Its editor
//! ([`EditorApp::host_keeps_project`](crate::EditorApp::host_keeps_project))
//! borrows the session's engine handle for each frame
//! ([`EditorApp::with_engine`](crate::EditorApp::with_engine)), hands its
//! edits back ([`EditorApp::take_changes`](crate::EditorApp::take_changes)),
//! and asks the host for what only the session can do: start over, open a
//! project file with its audio, keep a file someone picked
//! ([`HostRequest`]). The host's automatable parameters show in the Devices
//! tab through [`HostControls`].

use std::path::PathBuf;

/// Something only the host can do, asked by the editor during a frame
/// ([`EditorApp::take_host_requests`](crate::EditorApp::take_host_requests)).
#[derive(Clone, Debug, PartialEq)]
pub enum HostRequest {
    /// File ▸ New: start over with the host's default project.
    New,
    /// File ▸ Open (or an example): load this project file and the audio
    /// files it refers to.
    Open(PathBuf),
    /// Keep this audio file's bytes under the config path `path` (a file
    /// picked for a Clip, or one found for a missing reference). The
    /// document already refers to it.
    AddAsset { path: String, bytes: Vec<u8> },
}

/// One of the host's automatable parameters (a plugin's macros), as the
/// editor shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct HostParam {
    /// "Macro 1".
    pub name: String,
    /// What it drives, if anything: "Level › gain".
    pub target: Option<String>,
    /// Its position, 0–1.
    pub value: f32,
    /// The position as the host shows it ("−6.0 dB").
    pub text: String,
}

/// The host's automatable parameters. Moves go through the host so it
/// records them as automation: a gesture is `begin`, any number of `set`,
/// then `end`.
pub trait HostControls: Send {
    /// Every parameter, in order (read once a frame while shown).
    fn params(&self) -> Vec<HostParam>;
    fn begin(&mut self, index: usize);
    fn set(&mut self, index: usize, value: f32);
    fn end(&mut self, index: usize);
    /// `value` (0–1) of parameter `index` as the host would show it.
    fn format(&self, index: usize, value: f32) -> String;
}
