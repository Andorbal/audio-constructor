//! Bindings in the side panel: every binding in the Project tab, the
//! selected node's in the Node tab, and the transport buttons in the
//! Devices tab (UX.md "Bindings").

use ac_core::{
    Binding, GateMode, NodePath, SignalType, SocketSpec, Target, TransportAction,
};
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::bindings;
use crate::canvas::widgets;
use crate::icons;
use crate::theme;

/// The socket a binding targets, if it resolves.
fn target_spec(io: &PanelIo, b: &Binding) -> Option<SocketSpec> {
    let Target::Input { node, socket } = &b.target else {
        return None;
    };
    let n = io.doc.project.node_at(node)?;
    io.registry
        .signature_of(n, &io.doc.project.group_defs)?
        .inputs
        .into_iter()
        .find(|s| s.name == *socket)
}

/// What one row asks for.
enum RowAction {
    None,
    Remove,
    Navigate(NodePath),
}

/// One binding: what drives what, and its mapping (range and invert for
/// numbers, momentary or toggle for switches), editable in place.
fn row(ui: &mut egui::Ui, io: &mut PanelIo, index: usize, show_target: bool) -> RowAction {
    let Some(b) = io.doc.project.bindings.get(index).cloned() else {
        return RowAction::None;
    };
    let spec = target_spec(io, &b);
    let mut action = RowAction::None;
    ui.horizontal(|ui| {
        let icon = match b.source {
            ac_core::Source::GamepadButton { .. } | ac_core::Source::GamepadAxis { .. } => {
                icons::GAME_CONTROLLER
            }
            ac_core::Source::Key { .. } => icons::KEYBOARD,
            _ => icons::LINK,
        };
        ui.label(
            RichText::new(format!("{icon} {}", b.source.label()))
                .color(theme::TEXT_STRONG),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(icons::X)
                .on_hover_text("Remove this binding")
                .clicked()
            {
                action = RowAction::Remove;
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.add_space(14.0);
        if show_target {
            let label = bindings::target_label(&io.doc.project, io.registry, &b.target);
            match &b.target {
                Target::Input { node, .. } => {
                    let missing = io.doc.project.node_at(node).is_none();
                    let text = RichText::new(format!("→ {label}")).color(if missing {
                        theme::WARNING
                    } else {
                        theme::ACCENT
                    });
                    if ui
                        .add(egui::Button::new(text).frame(false))
                        .on_hover_text("Show the node")
                        .clicked()
                    {
                        action = RowAction::Navigate(node.clone());
                    }
                }
                Target::Transport { .. } => {
                    ui.label(RichText::new(format!("→ {label}")).color(theme::TEXT));
                }
            }
        }
        let now = io.now;
        match spec.as_ref().map(|s| (s.ty, s)) {
            Some((SignalType::Float | SignalType::Int, spec)) => {
                let (lo, hi) = b.range(spec);
                let (mut lo, mut hi) = (lo, hi);
                let speed = ((hi - lo).abs() / 200.0).max(0.001);
                ui.label(RichText::new("from").small().color(theme::TEXT_MUTED));
                let r1 = ui.add(egui::DragValue::new(&mut lo).speed(speed));
                ui.label(RichText::new("to").small().color(theme::TEXT_MUTED));
                let r2 = ui.add(egui::DragValue::new(&mut hi).speed(speed));
                if r1.changed() {
                    io.doc.edit_binding(index, "min", now, |b| b.min = Some(lo));
                }
                if r2.changed() {
                    io.doc.edit_binding(index, "max", now, |b| b.max = Some(hi));
                }
                if r1.drag_stopped() || r2.drag_stopped() || r1.lost_focus() || r2.lost_focus() {
                    io.doc.end_coalescing();
                }
                invert_box(ui, io, index, b.invert);
                if let Some(v) = io.engine.handle.binding_values().get(index) {
                    ui.label(
                        RichText::new(widgets::format(f64::from(v), spec))
                            .font(theme::mono_font(theme::FONT_SMALL))
                            .color(theme::TEXT_MUTED),
                    )
                    .on_hover_text("Where the control last set it");
                }
            }
            Some((SignalType::Bool, _)) => {
                let mut mode = b.mode;
                egui::ComboBox::from_id_salt(("binding-mode", index))
                    .width(96.0)
                    .selected_text(mode_label(mode))
                    .show_ui(ui, |ui| {
                        for m in [GateMode::Momentary, GateMode::Toggle] {
                            ui.selectable_value(&mut mode, m, mode_label(m));
                        }
                    });
                if mode != b.mode {
                    io.doc.edit_binding(index, "mode", now, |b| b.mode = mode);
                    io.doc.end_coalescing();
                }
                invert_box(ui, io, index, b.invert);
            }
            Some((SignalType::Trigger, _)) => {
                ui.label(RichText::new("fires on press").small().color(theme::TEXT_MUTED));
            }
            _ if matches!(b.target, Target::Transport { .. }) => {
                ui.label(RichText::new("on press").small().color(theme::TEXT_MUTED));
            }
            _ => {
                ui.label(
                    RichText::new("does nothing (see Problems)")
                        .small()
                        .color(theme::WARNING),
                );
            }
        }
    });
    action
}

fn mode_label(m: GateMode) -> &'static str {
    match m {
        GateMode::Momentary => "Momentary",
        GateMode::Toggle => "Toggle",
    }
}

fn invert_box(ui: &mut egui::Ui, io: &mut PanelIo, index: usize, invert: bool) {
    let mut inv = invert;
    if ui
        .checkbox(&mut inv, "invert")
        .on_hover_text("Turn the control around")
        .changed()
    {
        io.doc.edit_binding(index, "invert", io.now, |b| b.invert = inv);
        io.doc.end_coalescing();
    }
}

/// Act on a row's request. Returns true when the list changed under the
/// caller (the rest of it is drawn next frame).
fn apply(io: &mut PanelIo, index: usize, action: RowAction) -> bool {
    match action {
        RowAction::None => false,
        RowAction::Remove => {
            io.doc.edit_project("Remove binding", |p| {
                if index < p.bindings.len() {
                    p.bindings.remove(index);
                }
            });
            true
        }
        RowAction::Navigate(path) => {
            io.requests.push(PanelRequest::Navigate(path));
            false
        }
    }
}

/// The Project tab's list of every binding.
pub(super) fn project_section(ui: &mut egui::Ui, io: &mut PanelIo) {
    section(ui, "Bindings");
    let n = io.doc.project.bindings.len();
    if n == 0 {
        ui.label(
            RichText::new(
                "No bindings. Right-click an input (or a control's title) and choose Bind… \
                 to drive it from a MIDI controller, a key, a gamepad, or OSC.",
            )
            .color(theme::TEXT_MUTED),
        );
        return;
    }
    for i in 0..n {
        let a = row(ui, io, i, true);
        if apply(io, i, a) {
            break;
        }
        if i + 1 < n {
            ui.separator();
        }
    }
}

/// The Node tab's list of the selected node's bindings (through every
/// instance when it sits inside a group).
pub(super) fn node_section(ui: &mut egui::Ui, io: &mut PanelIo, id: ac_core::NodeId) {
    let paths = io.doc.all_node_paths(id);
    let entered = io.doc.node_path(id);
    let mine: Vec<usize> = io
        .doc
        .project
        .bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.node().is_some_and(|p| paths.contains(p)))
        .map(|(i, _)| i)
        .collect();
    section(ui, "Bindings");
    if mine.is_empty() {
        ui.label(
            RichText::new("Right-click an unwired input and choose Bind… to drive it from a control.")
                .small()
                .color(theme::TEXT_MUTED),
        );
        return;
    }
    for i in mine {
        let socket = match &io.doc.project.bindings[i].target {
            Target::Input { node, socket } => {
                let mut s = crate::canvas::paint_pretty(socket);
                if *node != entered {
                    s.push_str(&format!(" (instance {node})"));
                }
                s
            }
            Target::Transport { .. } => continue,
        };
        ui.label(RichText::new(socket).small().color(theme::TEXT_MUTED));
        let a = row(ui, io, i, false);
        if apply(io, i, a) {
            break;
        }
    }
}

/// The Devices tab's transport buttons: the gamepad preset, the
/// transport bindings, and learning one.
pub(super) fn transport_section(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    choice: &mut TransportAction,
) {
    section(ui, "Transport buttons");
    if ui
        .button(format!("{}  Add gamepad transport buttons", icons::GAME_CONTROLLER))
        .on_hover_text("A plays and stops, B stops, X records, on any gamepad.")
        .clicked()
    {
        let n = bindings::add_all(io.doc, "Add transport buttons", bindings::transport_preset());
        let msg = if n == 0 {
            "The gamepad transport buttons are already set up.".to_string()
        } else {
            format!("Added {n} gamepad transport buttons (A play/stop, B stop, X record).")
        };
        io.requests.push(PanelRequest::Status(msg));
    }
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("transport-learn-action")
            .width(130.0)
            .selected_text(choice.label())
            .show_ui(ui, |ui| {
                for a in TransportAction::ALL {
                    ui.selectable_value(choice, a, a.label());
                }
            });
        if ui
            .button(format!("{}  Bind…", icons::LINK))
            .on_hover_text("Press the control (MIDI, key, gamepad, or OSC) that should do this.")
            .clicked()
        {
            io.requests
                .push(PanelRequest::Learn(Target::Transport { action: *choice }));
        }
    });
    let list: Vec<usize> = io
        .doc
        .project
        .bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.target, Target::Transport { .. }))
        .map(|(i, _)| i)
        .collect();
    if list.is_empty() {
        ui.label(
            RichText::new("No transport buttons yet.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    for i in list {
        let a = row(ui, io, i, true);
        if apply(io, i, a) {
            break;
        }
    }
    ui.label(
        RichText::new("Transport buttons are stored in the project, like other bindings.")
            .small()
            .color(theme::TEXT_FAINT),
    );
}
