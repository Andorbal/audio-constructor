//! The Devices tab (audio device, latency, MIDI inputs) and the Problems
//! tab (compile diagnostics, click to jump).

use ac_engine::Severity;
use egui::RichText;

use ac_io::InputChoice;

use super::{DevicesState, PanelIo, PanelRequest, section};
use crate::audio::LatencyState;
use crate::icons;
use crate::theme;

pub(super) fn devices(ui: &mut egui::Ui, io: &mut PanelIo, state: &mut DevicesState) {
    let status = io.engine.handle.status().clone();
    let info = io.engine.info.clone();
    section(ui, "Running");
    egui::Grid::new("devices-now")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            ui.label("Audio");
            if !io.engine.is_running() {
                ui.label(RichText::new("off").color(theme::TEXT_MUTED));
            } else if io.engine.is_null() {
                ui.label(RichText::new("no device (null)").color(theme::TEXT_STRONG));
            } else {
                ui.label(RichText::new(&info.host).color(theme::TEXT_STRONG));
            }
            ui.end_row();
            if io.engine.is_running() && !io.engine.is_null() {
                ui.label("Output");
                ui.label(info.output_device.as_deref().unwrap_or("default"));
                ui.end_row();
                ui.label("Input");
                ui.label(info.input_device.as_deref().unwrap_or("none"));
                ui.end_row();
            }
            ui.label("Sample rate");
            ui.label(format!("{} Hz", info.sample_rate));
            ui.end_row();
            ui.label("Buffer");
            ui.label(format!(
                "{} frames ({:.1} ms)",
                info.buffer_size,
                f64::from(info.buffer_size) * 1000.0 / f64::from(info.sample_rate.max(1))
            ));
            ui.end_row();
            ui.label("Channels");
            ui.label(format!(
                "{} in · {} out",
                info.in_channels, info.out_channels
            ));
            ui.end_row();
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
            )
            .on_hover_text("Engine blocks over budget plus device underruns/overruns");
            ui.end_row();
            if let Some(st) = io.engine.stats() {
                if st.input_underruns + st.input_overruns + st.device_errors > 0 {
                    ui.label("Device");
                    ui.colored_label(
                        theme::WARNING,
                        format!(
                            "{} under · {} over · {} errors",
                            st.input_underruns, st.input_overruns, st.device_errors
                        ),
                    );
                    ui.end_row();
                }
                if st.realtime_denied {
                    ui.label("Priority");
                    ui.colored_label(theme::WARNING, "real-time scheduling was denied");
                    ui.end_row();
                }
                if st.disconnected {
                    ui.label("Status");
                    ui.colored_label(theme::ERROR, "device disconnected");
                    ui.end_row();
                }
            }
            ui.label("Latency");
            match &io.engine.latency {
                LatencyState::Idle => {
                    ui.label(RichText::new("not measured").color(theme::TEXT_MUTED));
                }
                LatencyState::Running => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("measuring…");
                    });
                }
                LatencyState::Done(Ok(l)) => {
                    ui.label(
                        RichText::new(format!("{:.2} ms round trip", l.ms))
                            .color(theme::TEXT_STRONG),
                    )
                    .on_hover_text(l.to_string());
                }
                LatencyState::Done(Err(e)) => {
                    ui.colored_label(theme::WARNING, e.to_string());
                }
            }
            ui.end_row();
        });
    if let Some(e) = &io.engine.device_error {
        ui.colored_label(theme::WARNING, format!("{} {e}", icons::WARNING));
        ui.label(
            RichText::new("Running on the silent null stream so the editor keeps working.")
                .small()
                .color(theme::TEXT_FAINT),
        );
    }
    for n in &info.notes {
        ui.label(
            RichText::new(format!("{} {n}", icons::INFO))
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    ui.add_space(4.0);
    let can_measure = io.engine.is_running() && !io.engine.is_null() && info.in_channels > 0;
    let r = ui.add_enabled(
        can_measure && io.engine.latency != LatencyState::Running,
        egui::Button::new(format!("{}  Measure latency", icons::TIMER)),
    );
    if r.clicked() {
        io.engine.measure_latency();
    }
    r.on_hover_text(
        "Plays a short burst on every output and listens on the inputs. Connect an output to an \
         input with a cable first (turn speakers down).",
    )
    .on_disabled_hover_text("Needs an audio device with an input.");

    section(ui, "Device");
    if state.hosts.is_none() {
        state.hosts = Some(ac_io::enumerate());
    }
    let hosts = state.hosts.clone().unwrap_or_default();
    let draft = state.draft.get_or_insert_with(|| io.engine.config.clone());
    if hosts.is_empty() {
        ui.label(RichText::new("No audio hosts on this machine.").color(theme::TEXT_MUTED));
    } else {
        let host_name = draft
            .host
            .clone()
            .or_else(|| hosts.iter().find(|h| h.default).map(|h| h.name.clone()))
            .unwrap_or_default();
        let host = hosts.iter().find(|h| h.name == host_name).or(hosts.first());
        egui::Grid::new("devices-pick")
            .num_columns(2)
            .spacing(egui::vec2(10.0, 6.0))
            .show(ui, |ui| {
                ui.label("Host");
                egui::ComboBox::from_id_salt("audio-host")
                    .selected_text(&host_name)
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for h in &hosts {
                            if ui.selectable_label(h.name == host_name, &h.name).clicked() {
                                draft.host = Some(h.name.clone());
                                draft.output_device = None;
                                draft.input = InputChoice::Default;
                            }
                        }
                    });
                ui.end_row();
                if let Some(h) = host {
                    ui.label("Output");
                    let out_name = draft.output_device.clone().unwrap_or_else(|| {
                        h.outputs
                            .iter()
                            .find(|d| d.default)
                            .map_or("default".into(), |d| d.name.clone())
                    });
                    egui::ComboBox::from_id_salt("audio-out")
                        .selected_text(&out_name)
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            for d in &h.outputs {
                                if ui.selectable_label(d.name == out_name, &d.name).clicked() {
                                    draft.output_device = Some(d.name.clone());
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Input");
                    let in_name = match &draft.input {
                        InputChoice::Default => "default".to_string(),
                        InputChoice::None => "none".to_string(),
                        InputChoice::Named(n) => n.clone(),
                    };
                    egui::ComboBox::from_id_salt("audio-in")
                        .selected_text(&in_name)
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.input == InputChoice::None, "none")
                                .clicked()
                            {
                                draft.input = InputChoice::None;
                            }
                            if ui
                                .selectable_label(draft.input == InputChoice::Default, "default")
                                .clicked()
                            {
                                draft.input = InputChoice::Default;
                            }
                            for d in &h.inputs {
                                if ui.selectable_label(in_name == d.name, &d.name).clicked() {
                                    draft.input = InputChoice::Named(d.name.clone());
                                }
                            }
                        });
                    ui.end_row();
                    let dev = h
                        .outputs
                        .iter()
                        .find(|d| Some(&d.name) == draft.output_device.as_ref())
                        .or_else(|| h.outputs.iter().find(|d| d.default));
                    ui.label("Sample rate");
                    let rate_text = draft
                        .sample_rate
                        .map_or("device default".into(), |r| format!("{r} Hz"));
                    egui::ComboBox::from_id_salt("audio-rate")
                        .selected_text(rate_text)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.sample_rate.is_none(), "device default")
                                .clicked()
                            {
                                draft.sample_rate = None;
                            }
                            for r in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
                                let ok = dev
                                    .and_then(|d| d.sample_rates)
                                    .is_none_or(|(lo, hi)| (lo..=hi).contains(&r));
                                if ok
                                    && ui
                                        .selectable_label(
                                            draft.sample_rate == Some(r),
                                            format!("{r} Hz"),
                                        )
                                        .clicked()
                                {
                                    draft.sample_rate = Some(r);
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Buffer");
                    let buf_text = draft
                        .buffer_size
                        .map_or("default".into(), |b| format!("{b} frames"));
                    egui::ComboBox::from_id_salt("audio-buffer")
                        .selected_text(buf_text)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.buffer_size.is_none(), "default")
                                .clicked()
                            {
                                draft.buffer_size = None;
                            }
                            for b in [32u32, 64, 128, 256, 512, 1024, 2048] {
                                let ok = dev
                                    .and_then(|d| d.buffer_sizes)
                                    .is_none_or(|(lo, hi)| (lo..=hi).contains(&b));
                                if ok
                                    && ui
                                        .selectable_label(
                                            draft.buffer_size == Some(b),
                                            format!("{b} frames"),
                                        )
                                        .clicked()
                                {
                                    draft.buffer_size = Some(b);
                                }
                            }
                        });
                    ui.end_row();
                    if let Some(e) = &h.error {
                        ui.label("");
                        ui.colored_label(theme::WARNING, e);
                        ui.end_row();
                    }
                }
            });
    }
    ui.horizontal(|ui| {
        let changed = state.draft.as_ref() != Some(&io.engine.config) || io.engine.is_null();
        if ui
            .add_enabled(
                !hosts.is_empty() && changed,
                egui::Button::new(format!("{}  Apply", icons::CHECK)),
            )
            .clicked()
            && let Some(d) = state.draft.clone()
        {
            io.requests.push(PanelRequest::SwitchAudio(d));
        }
        if ui.button(format!("{}  Refresh", icons::REDO)).clicked() {
            state.hosts = Some(ac_io::enumerate());
        }
    });

    section(ui, "MIDI inputs");
    match io.engine.midi.as_mut() {
        None => {
            ui.label(RichText::new("MIDI is off (no device runtime).").color(theme::TEXT_MUTED));
        }
        Some(m) => {
            let connected = m.connected();
            if let Some(e) = m.last_error() {
                ui.label(RichText::new(e.to_string()).color(theme::TEXT_MUTED));
            } else if connected.is_empty() {
                ui.label(RichText::new("No MIDI inputs found.").color(theme::TEXT_MUTED));
            }
            for p in &connected {
                ui.label(format!("{}  {p}", icons::MUSIC_NOTES));
            }
            if m.dropped() > 0 {
                ui.colored_label(theme::WARNING, format!("{} events dropped", m.dropped()));
            }
            if ui.button(format!("{}  Rescan", icons::REDO)).clicked() {
                io.engine.rescan_midi();
            }
        }
    }
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
