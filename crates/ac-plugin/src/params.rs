//! What the host sees as the plugin's parameters and state.
//!
//! - **Macros**: [`MACRO_COUNT`] parameters with stable ids `macro_1` …
//!   `macro_16`, named `Macro 1` … in every host, 0–1, 0.5 by default.
//!   The project's bindings say what each one drives (DECISIONS D117).
//! - **State**: one persisted field holding the session's state document
//!   (see [`crate::state`]); the framework saves it with the macro values.

use std::sync::Arc;

use ac_core::MACRO_COUNT;
use nice_plug::params::persist::PersistentField;
use nice_plug::prelude::*;

use crate::session::Shared;

/// One macro parameter.
#[derive(Params)]
pub struct MacroParam {
    #[id = "macro"]
    pub value: FloatParam,
}

/// Every parameter of a plugin instance, and its state.
#[derive(Params)]
pub struct PluginParams {
    /// The project, its assets, and where it came from.
    #[persist = "audio-constructor"]
    pub state: StateField,
    /// `macro_1` … `macro_16`.
    #[nested(array)]
    pub macros: [MacroParam; MACRO_COUNT],
}

impl PluginParams {
    pub fn new(shared: Arc<Shared>) -> PluginParams {
        let labels = shared.labels().clone();
        PluginParams {
            macros: std::array::from_fn(|i| {
                let (show, read) = (labels.clone(), labels.clone());
                MacroParam {
                    value: FloatParam::new(
                        format!("Macro {}", i + 1),
                        0.5,
                        FloatRange::Linear { min: 0.0, max: 1.0 },
                    )
                    .with_value_to_string(Arc::new(move |v| show.format(i, v)))
                    .with_string_to_value(Arc::new(move |s| read.parse(i, s))),
                }
            }),
            state: StateField { shared },
        }
    }

    /// Every macro's current value (after host modulation), in order.
    /// Real-time safe: atomic reads.
    pub fn macro_values(&self) -> [f32; MACRO_COUNT] {
        std::array::from_fn(|i| self.macros.get(i).map_or(0.5, |m| m.value.value()))
    }
}

/// The persisted state: reading it saves the session, writing it restores
/// one. Both happen on the host's main thread.
pub struct StateField {
    shared: Arc<Shared>,
}

impl PersistentField<'_, String> for StateField {
    fn set(&self, text: String) {
        self.shared.lock().restore_state(&text);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&String) -> R,
    {
        let text = self.shared.lock().save_state();
        f(&text)
    }
}
