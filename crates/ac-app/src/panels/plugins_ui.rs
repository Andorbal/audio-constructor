//! The plugin parts of the side panel: a Plugin node's section in the Node
//! tab (who the plugin is, whether it runs, its window), and the Plugins
//! section of the Devices tab (what was found, rescanning, what failed).

use ac_core::{NodeId, NodeInstance, PluginDesc, PluginFormat};
use ac_engine::Severity;
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::theme;

/// A Plugin node's plugin: its description, whether it runs here, and a
/// button for its window.
pub(super) fn node_section(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &NodeInstance) {
    section(ui, &tr("section-plugin"));
    let Some(desc) = PluginDesc::from_config(&node.config) else {
        ui.label(RichText::new(tr("plugin-none-chosen")).color(theme::text_muted()));
        return;
    };
    egui::Grid::new(("plugin-info", id))
        .num_columns(2)
        .spacing(egui::vec2(10.0, 4.0))
        .show(ui, |ui| {
            let mut row = |label: String, value: &str| {
                if !value.is_empty() {
                    ui.label(RichText::new(label).color(theme::text_muted()));
                    ui.label(value);
                    ui.end_row();
                }
            };
            row(tr("plugin-name"), &desc.name);
            row(tr("plugin-vendor"), &desc.vendor);
            row(tr("plugin-kind"), &crate::plugins::kind_label(&desc));
            row(tr("plugin-version"), &desc.version);
            row(tr("plugin-id"), &desc.id);
            row(tr("plugin-found-at"), &desc.path);
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
            RichText::new(tr("plugin-missing-note"))
                .small()
                .color(theme::text_muted()),
        );
    } else if running {
        ui.label(RichText::new(tr("plugin-running")).color(theme::text_strong()));
    } else if io.plugins.is_none() {
        ui.label(RichText::new(tr("plugin-not-hosted")).color(theme::text_muted()));
    }
    ui.horizontal(|ui| {
        let has_window = running && crate::plugins::has_window(&key);
        let b = ui
            .add_enabled(
                has_window,
                egui::Button::new(format!(
                    "{}  {}",
                    icons::APP_WINDOW,
                    tr("plugin-show-window")
                )),
            )
            .on_hover_text(tr("plugin-show-window-tip"))
            .on_disabled_hover_text(if running {
                tr("plugin-no-window-tip")
            } else {
                tr("plugin-not-running-tip")
            });
        if b.clicked() {
            io.requests.push(PanelRequest::ShowPluginWindow(id));
        }
    });
    let note = if io.doc.scope.is_empty() {
        tr("plugin-params-note")
    } else {
        tr("plugin-params-note-in-group")
    };
    ui.label(RichText::new(note).small().color(theme::text_muted()));
}

/// Above a Plugin node's inputs in the Node tab, when it has more knobs
/// than its card shows: whether the card shows them all.
pub(super) fn knobs_toggle(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &NodeInstance) {
    if node.kind != ac_core::PLUGIN_KIND
        || crate::edit::knob_count(node) <= crate::edit::PLUGIN_KNOBS_SHOWN
    {
        return;
    }
    let mut all = crate::edit::shows_all_knobs(node);
    if ui
        .checkbox(&mut all, tr("plugin-show-all-knobs"))
        .on_hover_text(tr_args(
            "plugin-show-all-knobs-tip",
            &[("count", crate::edit::PLUGIN_KNOBS_SHOWN.into())],
        ))
        .changed()
    {
        io.doc.edit(io.registry, &tr("undo-show-knobs"), |g, _| {
            if let Some(p) = g.node_mut(id) {
                crate::edit::set_show_all_knobs(p, all);
            }
        });
    }
}

/// The Devices tab's Plugins section.
pub(super) fn devices_section(ui: &mut egui::Ui, io: &mut PanelIo) {
    let Some(p) = io.plugins else {
        return;
    };
    section(ui, &tr("section-plugins"));
    let count = |f: PluginFormat| p.list().iter().filter(|d| d.format == f).count();
    let line = if p.scanning() {
        tr("plugins-looking")
    } else {
        tr_args(
            "plugins-found",
            &[
                ("count", p.list().len().into()),
                ("clap", count(PluginFormat::Clap).into()),
                ("vst3", count(PluginFormat::Vst3).into()),
            ],
        )
    };
    ui.label(RichText::new(line).color(theme::text_strong()));
    ui.horizontal(|ui| {
        let can = p.can_scan() && !p.scanning();
        if ui
            .add_enabled(
                can,
                egui::Button::new(format!(
                    "{}  {}",
                    icons::ARROWS_CLOCKWISE,
                    tr("devices-rescan")
                )),
            )
            .on_hover_text(tr("plugins-rescan-tip"))
            .clicked()
        {
            io.requests.push(PanelRequest::RescanPlugins(false));
        }
        if ui
            .add_enabled(can, egui::Button::new(tr("plugins-rescan-all")))
            .on_hover_text(tr("plugins-rescan-all-tip"))
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
            RichText::new(tr_args(
                "plugins-folders",
                &[
                    ("format", format.name().into()),
                    ("folders", dirs.join(", ").into()),
                ],
            ))
            .small()
            .color(theme::text_muted()),
        );
    }
    if !p.failed().is_empty() {
        egui::CollapsingHeader::new(
            RichText::new(tr_args(
                "plugins-failed",
                &[("count", p.failed().len().into())],
            ))
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
        RichText::new(tr("plugins-separate-process"))
            .small()
            .color(theme::text_faint()),
    );
}
