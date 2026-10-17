//! The side panel (`N`): Node, Library, Project, Devices, and Problems tabs.

mod library;
pub mod markdown;
mod node_tab;
mod project_tab;
mod status_tabs;

use std::collections::BTreeSet;

use ac_core::{NodeId, NodePath, Registry};
use ac_io::AssetStore;
use egui::{RichText, vec2};

use crate::audio::EngineHost;
use crate::document::Document;
use crate::icons;
use crate::palette::Candidate;
use crate::theme;

pub use library::LibraryItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Node,
    Library,
    Project,
    Devices,
    Problems,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Node,
        Tab::Library,
        Tab::Project,
        Tab::Devices,
        Tab::Problems,
    ];

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Tab::Node => (icons::SLIDERS, "Node"),
            Tab::Library => (icons::BOOKS, "Library"),
            Tab::Project => (icons::GEAR, "Project"),
            Tab::Devices => (icons::PLUGS, "Devices"),
            Tab::Problems => (icons::WARNING, "Problems"),
        }
    }
}

/// Things the panel asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelRequest {
    ChooseClipFile(NodeId),
    Locate(String),
    /// Add a node at the center of the view.
    Add(Candidate),
    /// Jump to a node (entering groups as needed).
    Navigate(NodePath),
    Status(String),
}

/// The side panel's own state.
pub struct SidePanel {
    pub open: bool,
    pub tab: Tab,
    pub library_query: String,
    label_edit: Option<(NodeId, String)>,
}

impl Default for SidePanel {
    fn default() -> Self {
        SidePanel {
            open: true,
            tab: Tab::Node,
            library_query: String::new(),
            label_edit: None,
        }
    }
}

/// What the panel reads and changes.
pub struct PanelIo<'a> {
    pub doc: &'a mut Document,
    pub registry: &'a Registry,
    pub engine: &'a mut EngineHost,
    pub assets: &'a AssetStore,
    pub missing: &'a BTreeSet<String>,
    pub requests: Vec<PanelRequest>,
    pub now: f64,
}

impl SidePanel {
    pub fn show(&mut self, ui: &mut egui::Ui, io: &mut PanelIo) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for tab in Tab::ALL {
                let (icon, name) = tab.label();
                let count = if tab == Tab::Problems {
                    io.engine.report.diagnostics.len()
                } else {
                    0
                };
                let text = if count > 0 {
                    format!("{icon} {name} {count}")
                } else {
                    format!("{icon} {name}")
                };
                let selected = self.tab == tab;
                let mut rt = RichText::new(text).size(theme::FONT_SMALL + 0.5);
                if selected {
                    rt = rt.color(theme::TEXT_STRONG);
                } else if count > 0 && io.engine.report.has_errors() {
                    rt = rt.color(theme::ERROR);
                } else {
                    rt = rt.color(theme::TEXT_MUTED);
                }
                let b = egui::Button::new(rt)
                    .selected(selected)
                    .frame_when_inactive(false)
                    .min_size(vec2(0.0, 24.0));
                if ui.add(b).clicked() {
                    self.tab = tab;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(2.0);
                match self.tab {
                    Tab::Node => node_tab::show(ui, io, &mut self.label_edit),
                    Tab::Library => library::show(ui, io, &mut self.library_query),
                    Tab::Project => project_tab::show(ui, io),
                    Tab::Devices => status_tabs::devices(ui, io),
                    Tab::Problems => status_tabs::problems(ui, io),
                }
                ui.add_space(12.0);
            });
    }
}

/// A section heading inside a tab.
pub(crate) fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(theme::FONT_SMALL - 0.5)
            .color(theme::TEXT_FAINT),
    );
    ui.add_space(2.0);
}
