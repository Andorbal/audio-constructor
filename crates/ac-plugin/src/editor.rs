//! The editor in the plugin window: the node editor from `ac-app`, drawn
//! in the host's window with egui-baseview (`ac_app::embed`), on the
//! instance's [`Session`](crate::Session) (ARCHITECTURE §13).
//!
//! The CLAP plugins offer it ([`WithEditor`]); the VST3 plugins have no
//! window for now ([`NoEditor`]): the same code would run there, but only
//! CLAP hosts have been tried, and a VST3 host without a window shows its
//! generic controls for the macros, as before. Without the `editor` feature
//! neither has one.
//!
//! # How the window and the session meet
//!
//! One `EditorApp` lives as long as the plugin instance (made when the host
//! first opens the window), so its undo history, selection, and view
//! survive closing and reopening. Each frame, on the window's thread,
//! [`frame`] locks the session ([`Shared::lock`], never taken on the audio
//! thread) and:
//!
//! - when [`Session::revision`](crate::Session::revision) moved (a state
//!   restore, a file load, going back to the default), loads the session's
//!   project and audio files into the editor's document;
//! - lends the editor the session's `EngineHandle` for the frame, so
//!   telemetry, inline controls (`set_param`, `fire`), binding values, and
//!   learning work exactly as in the app;
//! - hands every change back with
//!   [`Session::set_project`](crate::Session::set_project) (at once for
//!   what the engine hears, at most every quarter second for moves and
//!   other layout), which recompiles off the audio thread and keeps the
//!   macro readouts current;
//! - does what the editor asked of the host: New
//!   ([`Session::reset_to_default`](crate::Session::reset_to_default)),
//!   Open ([`Session::load_project_file`](crate::Session::load_project_file)),
//!   and keeping a picked audio file
//!   ([`Session::add_asset`](crate::Session::add_asset)).
//!
//! The macros show in the editor's Devices tab and move through the
//! framework's `ParamSetter` (`begin_set_parameter` / `set_parameter` /
//! `end_set_parameter`), so the host records the gesture as automation;
//! [`Shared::labels`] names each macro's target and formats its value. The
//! transport belongs to the host: the editor shows whether it plays and
//! offers no transport buttons.

use std::sync::Arc;

use crate::params::PluginParams;
use crate::session::Shared;

#[cfg(feature = "editor")]
mod frame;
#[cfg(feature = "editor")]
mod host_macros;
#[cfg(all(test, feature = "editor"))]
mod tests;
#[cfg(feature = "editor")]
mod window;

#[cfg(feature = "editor")]
pub use window::{AcEditor, AcEditorHandle};

/// What the plugin hands its editor.
#[derive(Clone)]
pub struct EditorContext {
    pub shared: Arc<Shared>,
    pub params: Arc<PluginParams>,
}

/// Which window a plugin build offers.
pub trait WindowKind: Send + 'static {
    #[cfg(feature = "editor")]
    type Editor: nice_plug::editor::Editor;
    #[cfg(not(feature = "editor"))]
    type Editor;

    /// The editor for an instance, or `None` for no window.
    fn create(context: EditorContext) -> Option<Self::Editor>;
}

/// The node editor in the plugin window (the CLAP plugins).
#[derive(Debug)]
pub struct WithEditor;

/// No window: hosts show their generic controls for the macros. The VST3
/// plugins, until a VST3 host has been tried with the editor.
#[derive(Debug)]
pub struct NoEditor;

impl WindowKind for NoEditor {
    type Editor = ();

    fn create(_context: EditorContext) -> Option<()> {
        None
    }
}

impl WindowKind for WithEditor {
    #[cfg(feature = "editor")]
    type Editor = AcEditor;
    #[cfg(not(feature = "editor"))]
    type Editor = ();

    #[cfg(feature = "editor")]
    fn create(context: EditorContext) -> Option<AcEditor> {
        Some(AcEditor::new(context))
    }

    #[cfg(not(feature = "editor"))]
    fn create(_context: EditorContext) -> Option<()> {
        None
    }
}

/// The editor the CLAP plugins declare.
pub type PluginEditor = <WithEditor as WindowKind>::Editor;

/// Make the editor for the plugin window, or `None` for no window.
pub fn create(context: EditorContext) -> Option<PluginEditor> {
    WithEditor::create(context)
}
