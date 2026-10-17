//! The Devices tab (audio backend status) and the Problems tab (compile
//! diagnostics, click to jump).

use ac_engine::Severity;
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::icons;
use crate::theme;

pub(super) fn devices(ui: &mut egui::Ui, io: &mut PanelIo) {
    let status = io.engine.handle.status().clone();
    section(ui, "Audio");
    egui::Grid::new("devices")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            match io.engine.backend_status() {
                Some(b) => {
                    ui.label("Backend");
                    ui.label(RichText::new(&b.name).color(theme::TEXT_STRONG));
                    ui.end_row();
                    ui.label("Device");
                    ui.label(b.device.as_deref().unwrap_or("no device (null)"));
                    ui.end_row();
                    ui.label("Sample rate");
                    ui.label(format!("{} Hz", b.sample_rate));
                    ui.end_row();
                    ui.label("Buffer");
                    ui.label(format!(
                        "{} frames ({:.1} ms)",
                        b.buffer_frames,
                        b.buffer_frames as f64 * 1000.0 / f64::from(b.sample_rate.max(1))
                    ));
                    ui.end_row();
                    if let Some(e) = &b.error {
                        ui.label("Error");
                        ui.colored_label(theme::ERROR, e);
                        ui.end_row();
                    }
                }
                None => {
                    ui.label("Backend");
                    ui.label(RichText::new("not running").color(theme::TEXT_MUTED));
                    ui.end_row();
                }
            }
            ui.label("Load");
            let load = status.load_fraction();
            ui.add(
                egui::ProgressBar::new(load.clamp(0.0, 1.0))
                    .text(format!("{:.0} %", load * 100.0))
                    .desired_width(160.0),
            );
            ui.end_row();
            ui.label("Xruns");
            let x = status.total_xruns();
            ui.colored_label(
                if x > 0 { theme::WARNING } else { theme::TEXT },
                x.to_string(),
            );
            ui.end_row();
            ui.label("Latency");
            ui.label(RichText::new("not measured").color(theme::TEXT_MUTED));
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.add_enabled(
        false,
        egui::Button::new(format!("{}  Measure latency", icons::TIMER)),
    )
    .on_disabled_hover_text(
        "Needs an audio device with a loopback cable from an output to an input.",
    );
    ui.label(
        RichText::new(
            "No audio device is in use: the engine runs on a silent, clock-paced thread so meters, \
             scopes, and the transport work. Device selection arrives with the device runtime.",
        )
        .small()
        .color(theme::TEXT_FAINT),
    );
    section(ui, "MIDI inputs");
    ui.label(RichText::new("No MIDI inputs available.").color(theme::TEXT_MUTED));
}

pub(super) fn problems(ui: &mut egui::Ui, io: &mut PanelIo) {
    let report = io.engine.report.clone();
    if report.diagnostics.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(icons::CHECK_CIRCLE)
                    .size(30.0)
                    .color(theme::OK),
            );
            ui.label(RichText::new("No problems").color(theme::TEXT_MUTED));
        });
        return;
    }
    for d in &report.diagnostics {
        let (icon, color) = match d.severity {
            Severity::Error => (icons::WARNING_CIRCLE, theme::ERROR),
            Severity::Warning => (icons::WARNING, theme::WARNING),
            Severity::Info => (icons::INFO, theme::ACCENT),
        };
        let where_ = d.node.as_ref().map(|p| node_name(io, p));
        let text = match (&where_, &d.socket) {
            (Some(n), Some(s)) => format!("{n} · {s}"),
            (Some(n), None) => n.clone(),
            _ => "project".into(),
        };
        let resp = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 4))
            .corner_radius(theme::radius(5.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.colored_label(color, icon);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(text).small().color(theme::TEXT_MUTED));
                        ui.label(&d.message);
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_stroke(
                resp.rect,
                theme::radius(5.0),
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }
        if resp.clicked()
            && let Some(p) = &d.node
        {
            io.requests.push(PanelRequest::Navigate(p.clone()));
        }
        resp.on_hover_text("Click to show the node");
    }
}

/// A readable name for a node path: labels or kind names, joined by `›`.
fn node_name(io: &PanelIo, path: &ac_core::NodePath) -> String {
    let mut graph = &io.doc.project.graph;
    let mut parts = Vec::new();
    for id in &path.0 {
        let Some(n) = graph.node(*id) else {
            parts.push(id.to_string());
            break;
        };
        let name = n.ui.label.clone().unwrap_or_else(|| match n.group_def() {
            Some(d) => io
                .doc
                .project
                .group_defs
                .get(&d)
                .map_or("Group".into(), |d| d.name.clone()),
            None => io
                .registry
                .get(&n.kind)
                .map_or(n.kind.clone(), |e| e.meta.name.to_string()),
        });
        parts.push(name);
        match n
            .group_def()
            .and_then(|d| io.doc.project.group_defs.get(&d))
        {
            Some(def) => graph = &def.graph,
            None => break,
        }
    }
    parts.join(" › ")
}
