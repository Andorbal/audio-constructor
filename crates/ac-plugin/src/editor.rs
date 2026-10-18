//! The seam for the editor in the plugin window.
//!
//! The plugin builds and runs without a window: hosts show their generic
//! controls for the macros. The node editor comes from `ac-app`, embedded
//! in the host's window (an egui-baseview window); until that lands,
//! [`PluginEditor`] is `()` and [`create`] returns `None`, so hosts show no
//! editor button. The `editor` feature turns on the framework's window
//! support for that work; nothing else in the plugin changes.
//!
//! # What the embedded editor gets
//!
//! [`create`] receives an [`EditorContext`] when the host first asks for
//! an editor, and returns the editor type the plugin declares. Through it:
//!
//! - **The project and engine**: lock [`EditorContext::shared`] once per
//!   frame ([`Shared::lock`], never on the audio thread) for the
//!   [`Session`](crate::Session):
//!   - [`Session::project`](crate::Session::project) is the project to show
//!     (build the editor's document from it when the window opens and
//!     after [`Session::revision`](crate::Session::revision) changes, as a
//!     state load replaces it);
//!   - [`Session::engine`](crate::Session::engine) is the running engine's
//!     `EngineHandle` (`None` while the host has the plugin inactive): its
//!     telemetry, `set_param` / `fire` for inline controls,
//!     `binding_values`, and `set_input_monitor` / `take_input_events` for
//!     learning work exactly as in the app;
//!   - push every project change with
//!     [`Session::set_project`](crate::Session::set_project): it recompiles
//!     off the audio thread and swaps the graph in at the next block (and
//!     keeps the macro readouts current). Don't call the handle's `edit`
//!     directly;
//!   - [`Session::add_asset`](crate::Session::add_asset) when someone picks
//!     an audio file for a Clip or Sampler (the bytes go into the host's
//!     saved state), and
//!     [`Session::load_project_file`](crate::Session::load_project_file) /
//!     [`Session::reset_to_default`](crate::Session::reset_to_default) for
//!     Open and New; [`Session::report`](crate::Session::report) says what
//!     the last load found.
//! - **The macros**: [`EditorContext::params`]'s `macros[i].value` are the
//!   host-visible parameters. Read them for display; change them through
//!   the framework's `ParamSetter` (from the `GuiContext` the editor's
//!   `spawn` receives) with `begin_set_parameter` / `set_parameter` /
//!   `end_set_parameter`, so the host records the gesture as automation.
//!   [`Shared::labels`] names each macro's target and formats its value.
//!   Bindings to macros are ordinary project bindings with
//!   `Source::Macro { number }`; learning one works through the input
//!   monitor, which sees macro moves as `Control::Macro` events.
//! - **The transport** belongs to the host: the engine follows its play,
//!   stop, and position every block, so the editor should hide its own
//!   transport buttons (or show the host's state, read from
//!   `EngineHandle::is_playing` / `position`).

use std::sync::Arc;

use crate::params::PluginParams;
use crate::session::Shared;

/// What the plugin hands its editor.
#[derive(Clone)]
pub struct EditorContext {
    pub shared: Arc<Shared>,
    pub params: Arc<PluginParams>,
}

/// The editor type the plugin declares. `()` means none.
pub type PluginEditor = ();

/// Make the editor for the plugin window, or `None` for no window.
pub fn create(_context: EditorContext) -> Option<PluginEditor> {
    None
}
