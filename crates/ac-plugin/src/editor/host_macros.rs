//! The macros as the editor shows and moves them: through the framework's
//! `ParamSetter`, so the host hears each move and records the gesture as
//! automation, and the macro then reaches the engine the way host
//! automation does (the audio thread reads the parameter every block).

use std::sync::Arc;

use ac_app::host::{HostControls, HostParam};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::Param;

use crate::macros::MacroLabels;
use crate::params::PluginParams;

pub(crate) struct PluginMacros {
    params: Arc<PluginParams>,
    labels: Arc<MacroLabels>,
    gui: GuiContext,
}

impl PluginMacros {
    pub(crate) fn new(
        params: Arc<PluginParams>,
        labels: Arc<MacroLabels>,
        gui: GuiContext,
    ) -> PluginMacros {
        PluginMacros {
            params,
            labels,
            gui,
        }
    }
}

impl HostControls for PluginMacros {
    fn params(&self) -> Vec<HostParam> {
        self.params
            .macros
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let value = m.value.unmodulated_normalized_value();
                HostParam {
                    name: m.value.name().to_string(),
                    target: self.labels.target(i).map(|t| t.name),
                    value,
                    text: self.labels.format(i, value),
                }
            })
            .collect()
    }

    fn begin(&mut self, index: usize) {
        if let Some(m) = self.params.macros.get(index) {
            self.gui.param_setter().begin_set_parameter(&m.value);
        }
    }

    fn set(&mut self, index: usize, value: f32) {
        if let Some(m) = self.params.macros.get(index) {
            self.gui
                .param_setter()
                .set_parameter_normalized(&m.value, value.clamp(0.0, 1.0));
        }
    }

    fn end(&mut self, index: usize) {
        if let Some(m) = self.params.macros.get(index) {
            self.gui.param_setter().end_set_parameter(&m.value);
        }
    }

    fn format(&self, index: usize, value: f32) -> String {
        self.labels.format(index, value)
    }
}
