//! The side panel (`N`): Node, Library, Project, Devices, and Problems tabs.

mod bindings_ui;
mod library;
pub mod markdown;
mod node_tab;
mod project_tab;
mod status_tabs;

use std::collections::BTreeSet;

use ac_core::{BlockSource, GroupId, NodeId, NodePath, Registry};
use ac_io::{AssetStore, Library};
use egui::{RichText, vec2};

use crate::audio::EngineHost;
use crate::document::Document;
use crate::icons;
use crate::palette::Candidate;
use crate::theme;

pub use library::{BlockFilter, LibraryItem, filter_blocks, first_sentence};

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
    /// Open another audio device setup.
    SwitchAudio(ac_io::AudioConfig),
    /// Choose a block file to add to the library.
    ImportBlockFile,
    /// Take a saved or imported block out of the library.
    RemoveBlock(BlockSource),
    /// Save a group definition to the library.
    SaveBlock(GroupId),
    /// Replace a definition with the library's newer version.
    UpdateBlock(GroupId),
    UpdateAllBlocks,
    /// Learn a binding for an input or a transport action.
    Learn(ac_core::Target),
}

/// The Devices tab's state: the device list (read when first shown, since
/// enumerating can be slow) and the setup being edited.
#[derive(Default)]
pub struct DevicesState {
    pub hosts: Option<Vec<ac_io::HostInfo>>,
    pub draft: Option<ac_io::AudioConfig>,
    /// MIDI output port names, read when first shown.
    pub midi_outputs: Option<Vec<String>>,
    /// The transport action "Bind…" learns a control for.
    pub transport_choice: Option<ac_core::TransportAction>,
}

/// The side panel's own state.
pub struct SidePanel {
    pub open: bool,
    pub tab: Tab,
    pub library_query: String,
    /// Which blocks the Library tab lists.
    pub block_filter: BlockFilter,
    label_edit: Option<(NodeId, String)>,
    pub devices: DevicesState,
}

impl Default for SidePanel {
    fn default() -> Self {
        SidePanel {
            open: true,
            tab: Tab::Node,
            library_query: String::new(),
            block_filter: BlockFilter::default(),
            label_edit: None,
            devices: DevicesState::default(),
        }
    }
}

/// What the panel reads and changes.
pub struct PanelIo<'a> {
    pub doc: &'a mut Document,
    pub registry: &'a Registry,
    pub library: &'a Library,
    pub engine: &'a mut EngineHost,
    pub assets: &'a AssetStore,
    pub missing: &'a BTreeSet<String>,
    pub requests: Vec<PanelRequest>,
    pub now: f64,
}

/// Set one config key on a node, as one undo step ("Change setting").
pub(crate) fn set_config(io: &mut PanelIo, id: NodeId, key: &str, value: serde_json::Value) {
    let key = key.to_string();
    io.doc.edit(io.registry, "Change setting", |g, _| {
        if let Some(n) = g.node_mut(id) {
            if !n.config.is_object() {
                n.config = ac_core::empty_config();
            }
            if let Some(obj) = n.config.as_object_mut() {
                obj.insert(key, value);
            }
        }
    });
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
                    Tab::Library => {
                        library::show(ui, io, &mut self.library_query, &mut self.block_filter)
                    }
                    Tab::Project => project_tab::show(ui, io),
                    Tab::Devices => status_tabs::devices(ui, io, &mut self.devices),
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
