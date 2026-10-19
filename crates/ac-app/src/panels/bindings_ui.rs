//! Bindings in the side panel: every binding in the Project tab, the
//! selected node's in the Node tab, and the transport buttons in the
//! Devices tab (UX.md "Bindings").

use ac_core::{Binding, GateMode, NodePath, SignalType, SocketSpec, Target, TransportAction};
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::bindings;
use crate::canvas::widgets;
use crate::i18n::{tr, tr_args};
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
        ui.label(RichText::new(format!("{icon} {}", b.source.label())).color(theme::text_strong()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::spoken(ui.small_button(icons::X))
                .on_hover_text(tr("bindings-remove"))
                .accessible_name(tr_args(
                    "bindings-remove-named",
                    &[("source", b.source.label().into())],
                ))
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
                        theme::warning()
                    } else {
                        theme::accent()
                    });
                    if ui
                        .add(egui::Button::new(text).frame(false))
                        .on_hover_text(tr("bindings-show-node"))
                        .clicked()
                    {
                        action = RowAction::Navigate(node.clone());
                    }
                }
                Target::Transport { .. } => {
                    ui.label(RichText::new(format!("→ {label}")).color(theme::text()));
                }
            }
        }
        let now = io.now;
        match spec.as_ref().map(|s| (s.ty, s)) {
            Some((SignalType::Float | SignalType::Int, spec)) => {
                let (lo, hi) = b.range(spec);
                let (mut lo, mut hi) = (lo, hi);
                let speed = ((hi - lo).abs() / 200.0).max(0.001);
                let from = ui.label(
                    RichText::new(tr("bindings-from"))
                        .small()
                        .color(theme::text_muted()),
                );
                let r1 = ui
                    .add(egui::DragValue::new(&mut lo).speed(speed))
                    .labelled_by(from.id);
                let to = ui.label(
                    RichText::new(tr("bindings-to"))
                        .small()
                        .color(theme::text_muted()),
                );
                let r2 = ui
                    .add(egui::DragValue::new(&mut hi).speed(speed))
                    .labelled_by(to.id);
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
                            .color(theme::text_muted()),
                    )
                    .on_hover_text(tr("bindings-last-value"));
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
                    })
                    .response
                    .accessible_name(tr("bindings-mode"));
                if mode != b.mode {
                    io.doc.edit_binding(index, "mode", now, |b| b.mode = mode);
                    io.doc.end_coalescing();
                }
                invert_box(ui, io, index, b.invert);
            }
            Some((SignalType::Trigger, _)) => {
                ui.label(
                    RichText::new(tr("bindings-fires-on-press"))
                        .small()
                        .color(theme::text_muted()),
                );
            }
            _ if matches!(b.target, Target::Transport { .. }) => {
                ui.label(
                    RichText::new(tr("bindings-on-press"))
                        .small()
                        .color(theme::text_muted()),
                );
            }
            _ => {
                ui.label(
                    RichText::new(tr("bindings-does-nothing"))
                        .small()
                        .color(theme::warning()),
                );
            }
        }
    });
    action
}

fn mode_label(m: GateMode) -> String {
    match m {
        GateMode::Momentary => tr("bindings-momentary"),
        GateMode::Toggle => tr("bindings-toggle"),
    }
}

fn invert_box(ui: &mut egui::Ui, io: &mut PanelIo, index: usize, invert: bool) {
    let mut inv = invert;
    if ui
        .checkbox(&mut inv, tr("bindings-invert"))
        .on_hover_text(tr("bindings-invert-tip"))
        .changed()
    {
        io.doc
            .edit_binding(index, "invert", io.now, |b| b.invert = inv);
        io.doc.end_coalescing();
    }
}

/// Act on a row's request. Returns true when the list changed under the
/// caller (the rest of it is drawn next frame).
fn apply(io: &mut PanelIo, index: usize, action: RowAction) -> bool {
    match action {
        RowAction::None => false,
        RowAction::Remove => {
            io.doc.edit_project(&tr("undo-remove-binding"), |p| {
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
    section(ui, &tr("section-bindings"));
    let n = io.doc.project.bindings.len();
    if n == 0 {
        ui.label(RichText::new(tr("bindings-none")).color(theme::text_muted()));
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
    section(ui, &tr("section-bindings"));
    if mine.is_empty() {
        ui.label(
            RichText::new(tr("bindings-node-none"))
                .small()
                .color(theme::text_muted()),
        );
        return;
    }
    for i in mine {
        let socket = match &io.doc.project.bindings[i].target {
            Target::Input { node, socket } => {
                let mut s = crate::canvas::paint_pretty(socket);
                if *node != entered {
                    s.push(' ');
                    s.push_str(&tr_args(
                        "bindings-instance",
                        &[("instance", node.to_string().into())],
                    ));
                }
                s
            }
            Target::Transport { .. } => continue,
        };
        ui.label(RichText::new(socket).small().color(theme::text_muted()));
        let a = row(ui, io, i, false);
        if apply(io, i, a) {
            break;
        }
    }
}

/// The Devices tab's transport buttons: the gamepad preset, the
/// transport bindings, and learning one.
pub(super) fn transport_section(ui: &mut egui::Ui, io: &mut PanelIo, choice: &mut TransportAction) {
    section(ui, &tr("section-transport-buttons"));
    if icons::spoken(ui.button(format!(
        "{}  {}",
        icons::GAME_CONTROLLER,
        tr("bindings-add-gamepad-transport")
    )))
    .on_hover_text(tr("bindings-add-gamepad-transport-tip"))
    .clicked()
    {
        let n = bindings::add_all(
            io.doc,
            &tr("undo-add-transport-buttons"),
            bindings::transport_preset(),
        );
        let msg = if n == 0 {
            tr("bindings-transport-already")
        } else {
            tr_args("bindings-transport-added", &[("count", n.into())])
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
            })
            .response
            .accessible_name(tr("bindings-transport-action"));
        if icons::spoken(ui.button(format!("{}  {}", icons::LINK, tr("canvas-bind"))))
            .on_hover_text(tr("bindings-learn-transport-tip"))
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
            RichText::new(tr("bindings-no-transport"))
                .small()
                .color(theme::text_muted()),
        );
    }
    for i in list {
        let a = row(ui, io, i, true);
        if apply(io, i, a) {
            break;
        }
    }
    ui.label(
        RichText::new(tr("bindings-transport-stored"))
            .small()
            .color(theme::text_faint()),
    );
}
