//! The plugin parts of the side panel: a Plugin node's section in the Node
//! tab (who the plugin is, whether it runs, its window), and the Plugins
//! section of the Devices tab (what was found, rescanning, what failed).

use ac_core::{NodeId, NodeInstance, PluginDesc, PluginFormat};
use ac_engine::Severity;
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::icons;
use crate::theme;

/// A Plugin node's plugin: its description, whether it runs here, and a
/// button for its window.
pub(super) fn node_section(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &NodeInstance) {
    section(ui, "Plugin");
    let Some(desc) = PluginDesc::from_config(&node.config) else {
        ui.label(
            RichText::new(
                "No plugin chosen. Add one from the palette (Shift+A) or the Library tab, \
                 where the plugins installed on this computer are listed by name.",
            )
            .color(theme::text_muted()),
        );
        return;
    };
    egui::Grid::new(("plugin-info", id))
        .num_columns(2)
        .spacing(egui::vec2(10.0, 4.0))
        .show(ui, |ui| {
            let mut row = |label: &str, value: &str| {
                if !value.is_empty() {
                    ui.label(RichText::new(label).color(theme::text_muted()));
                    ui.label(value);
                    ui.end_row();
                }
            };
            row("Name", &desc.name);
            row("Vendor", &desc.vendor);
            row("Kind", &crate::plugins::kind_label(&desc));
            row("Version", &desc.version);
            row("Id", &desc.id);
            row("Found at", &desc.path);
        });
    let path = io.doc.node_path(id);
    let key = path.to_string();
    let running = crate::plugins::running().contains(&key);
    let problem = io
        .engine
        .report
        .diagnostics
        .iter()
        .find(|d| d.node.as_ref() == Some(&path) && d.severity >= Severity::Warning)
        .map(|d| d.message.clone());
    if let Some(p) = &problem {
        ui.colored_label(theme::warning(), format!("{}  {p}", icons::WARNING));
        ui.label(
            RichText::new(
                "Its sockets and wires stay, so the project works again on a computer \
                 that has it. Installed it just now? Rescan in the Devices tab.",
            )
            .small()
            .color(theme::text_muted()),
        );
    } else if running {
        ui.label(RichText::new("Running").color(theme::text_strong()));
    } else if io.plugins.is_none() {
        ui.label(RichText::new("Plugins don't run in this editor.").color(theme::text_muted()));
    }
    ui.horizontal(|ui| {
        let has_window = running && crate::plugins::has_window(&key);
        let b = ui
            .add_enabled(
                has_window,
                egui::Button::new(format!("{}  Show Plugin Window", icons::APP_WINDOW)),
            )
            .on_hover_text("The plugin's own interface, in a window of its own")
            .on_disabled_hover_text(if running {
                "This plugin has no window of its own"
            } else {
                "The plugin isn't running"
            });
        if b.clicked() {
            io.requests.push(PanelRequest::ShowPluginWindow(id));
        }
    });
    let mut note = String::from(
        "Its parameters are the inputs above: moving one in the plugin's window moves it \
         here too. Saving the project saves everything the plugin keeps.",
    );
    if !io.doc.scope.is_empty() {
        note.push_str(
            " Inside a group the plugin's parameters are saved, but not the rest of its \
             state (every instance of the group shares the node).",
        );
    }
    ui.label(RichText::new(note).small().color(theme::text_muted()));
}

/// The Devices tab's Plugins section.
pub(super) fn devices_section(ui: &mut egui::Ui, io: &mut PanelIo) {
    let Some(p) = io.plugins else {
        return;
    };
    section(ui, "Plugins");
    let count = |f: PluginFormat| p.list().iter().filter(|d| d.format == f).count();
    let line = if p.scanning() {
        "Looking for plugins…".to_string()
    } else {
        format!(
            "{} found: {} CLAP, {} VST3",
            p.list().len(),
            count(PluginFormat::Clap),
            count(PluginFormat::Vst3)
        )
    };
    ui.label(RichText::new(line).color(theme::text_strong()));
    ui.horizontal(|ui| {
        let can = p.can_scan() && !p.scanning();
        if ui
            .add_enabled(
                can,
                egui::Button::new(format!("{}  Rescan", icons::ARROWS_CLOCKWISE)),
            )
            .on_hover_text("Look for plugins installed or changed since the last scan")
            .clicked()
        {
            io.requests.push(PanelRequest::RescanPlugins(false));
        }
        if ui
            .add_enabled(can, egui::Button::new("Rescan all"))
            .on_hover_text("Open every plugin again, changed or not")
            .clicked()
        {
            io.requests.push(PanelRequest::RescanPlugins(true));
        }
    });
    for format in [PluginFormat::Clap, PluginFormat::Vst3] {
        let dirs: Vec<String> = crate::plugins::search_paths(format)
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        ui.label(
            RichText::new(format!("{} folders: {}", format.name(), dirs.join(", ")))
                .small()
                .color(theme::text_muted()),
        );
    }
    if !p.failed().is_empty() {
        egui::CollapsingHeader::new(
            RichText::new(format!("{} couldn't be opened", p.failed().len()))
                .color(theme::warning()),
        )
        .id_salt("plugins-failed")
        .show(ui, |ui| {
            for f in p.failed() {
                ui.label(RichText::new(&f.path).small());
                ui.label(
                    RichText::new(format!("    {}", f.reason))
                        .small()
                        .color(theme::text_muted()),
                );
            }
        });
    }
    ui.label(
        RichText::new(
            "Each plugin is opened in a separate process while it's looked at, so one that \
             crashes is listed here instead of closing the editor.",
        )
        .small()
        .color(theme::text_faint()),
    );
}
