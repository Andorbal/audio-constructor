//! The side panel (`N`): Node, Library, Project, Devices, Problems, and
//! Help tabs.

mod bindings_ui;
mod help_tab;
mod library;
pub mod markdown;
mod node_tab;
mod plugins_ui;
mod project_tab;
mod status_tabs;

use std::collections::BTreeSet;

use ac_core::{BlockSource, GroupId, NodeId, NodePath, Registry};
use ac_io::{AssetStore, Library};
use egui::{RichText, vec2};

use crate::audio::EngineHost;
use crate::document::Document;
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::palette::Candidate;
use crate::theme;

pub use help_tab::HelpView;
pub use library::{BlockFilter, LibraryItem, filter_blocks, first_sentence};
pub use status_tabs::midi_text;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Node,
    Library,
    Project,
    Devices,
    Problems,
    /// A node kind's reference (F1), or every node.
    Help,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Node,
        Tab::Library,
        Tab::Project,
        Tab::Devices,
        Tab::Problems,
        Tab::Help,
    ];

    /// The tab's icon and name.
    pub fn label(self) -> (&'static str, String) {
        match self {
            Tab::Node => (icons::SLIDERS, tr("tab-node")),
            Tab::Library => (icons::BOOKS, tr("tab-library")),
            Tab::Project => (icons::GEAR, tr("tab-project")),
            Tab::Devices => (icons::PLUGS, tr("tab-devices")),
            Tab::Problems => (icons::WARNING, tr("tab-problems")),
            Tab::Help => (icons::BOOK_OPEN, tr("tab-help")),
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
    /// Open a node's editor in a window of its own, or close it.
    EditorWindow(NodeId),
    /// Dock a node's editor under the canvas.
    DockEditor(NodeId),
    /// Show a node kind's reference in the Help tab.
    Help(String),
    /// Open a shipped example (after the unsaved-changes check).
    OpenExample(std::path::PathBuf),
    /// Run an editor command.
    Run(crate::actions::Action),
    /// Play a block's preview, or stop it if it's playing.
    Preview(crate::online::PreviewKey),
    /// Write a block file (with its preview) somewhere to share.
    ExportBlock(crate::dialogs::BlockToExport),
    /// Read, or read again, the shared library at this link.
    RefreshLibrary(String),
    /// Stop listing the shared library at this link.
    RemoveLibrary(String),
    /// Download a block from a shared library into the library.
    Install {
        link: String,
        id: String,
    },
    /// Open a Plugin node's plugin window.
    ShowPluginWindow(NodeId),
    /// Look for plugins again (every bundle when `true`).
    RescanPlugins(bool),
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
    /// What the Help tab shows.
    pub help: HelpView,
    /// The shown tab's button: where F6 puts the keyboard in the panel.
    pub focus_target: Option<egui::Id>,
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
            help: HelpView::default(),
            focus_target: None,
        }
    }
}

impl SidePanel {
    /// Open the Help tab on `kind`'s reference (or on the list of every
    /// node, for `None`).
    pub fn show_help(&mut self, kind: Option<&str>) {
        self.open = true;
        self.tab = Tab::Help;
        if let Some(k) = kind {
            self.help.show_kind(k);
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
    /// Per-node timing, while it's shown (`Ctrl+T`).
    pub timing: Option<&'a crate::timing::Timings>,
    /// The plugin host, when this editor hosts plugins.
    pub plugins: Option<&'a crate::plugins::Plugins>,
    pub requests: Vec<PanelRequest>,
    pub now: f64,
    /// The host's automatable parameters (a plugin's macros), if any.
    pub host: Option<&'a mut Box<dyn crate::host::HostControls>>,
    /// Shared libraries, imports in progress, and the preview playing.
    pub online: &'a crate::online::Online,
}

/// Set one config key on a node, as one undo step ("Change setting").
pub(crate) fn set_config(io: &mut PanelIo, id: NodeId, key: &str, value: serde_json::Value) {
    let key = key.to_string();
    io.doc
        .edit(io.registry, &tr("undo-change-setting"), |g, _| {
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
            let font = theme::ui_font(theme::FONT_SMALL + 0.5);
            let count_of = |tab: Tab| {
                if tab == Tab::Problems {
                    io.engine.report.diagnostics.len()
                } else {
                    0
                }
            };
            let text_of = |tab: Tab, named: bool| {
                let (icon, name) = tab.label();
                match (named, count_of(tab)) {
                    (true, 0) => format!("{icon} {name}"),
                    (true, n) => format!("{icon} {name} {n}"),
                    (false, 0) => icon.to_string(),
                    (false, n) => format!("{icon} {n}"),
                }
            };
            // Too narrow for every name: the tabs not shown keep only
            // their icons (named on hover).
            let pad = ui.spacing().button_padding.x * 2.0 + 2.0;
            let needed: f32 = Tab::ALL
                .iter()
                .map(|t| {
                    let g =
                        ui.painter()
                            .layout_no_wrap(text_of(*t, true), font.clone(), theme::text());
                    g.size().x + pad
                })
                .sum();
            let compact = needed > ui.available_width();
            for tab in Tab::ALL {
                let selected = self.tab == tab;
                let count = count_of(tab);
                let mut rt = RichText::new(text_of(tab, selected || !compact)).font(font.clone());
                if selected {
                    rt = rt.color(theme::text_strong());
                } else if count > 0 && io.engine.report.has_errors() {
                    rt = rt.color(theme::error());
                } else {
                    rt = rt.color(theme::text_muted());
                }
                let b = egui::Button::new(rt)
                    .selected(selected)
                    .frame_when_inactive(false)
                    .min_size(vec2(0.0, 24.0));
                let mut r = ui.add(b);
                if compact && !selected {
                    r = r.on_hover_text(tab.label().1);
                }
                // A tab to screen readers, named even when only its icon
                // shows, with its count of problems.
                let name = match count {
                    0 => tab.label().1,
                    n => tr_args(
                        "tab-named-count",
                        &[("tab", tab.label().1.into()), ("count", n.into())],
                    ),
                };
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Tab, true, selected, &name)
                });
                if selected {
                    self.focus_target = Some(r.id);
                }
                if r.clicked() {
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
                    Tab::Help => help_tab::show(ui, io, &mut self.help),
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
            .color(theme::text_faint()),
    );
    ui.add_space(2.0);
}
