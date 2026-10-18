//! Popups on the canvas: right-click menus, choice dropdowns, typing into
//! fields, and hover tooltips.

use ac_core::{Endpoint, NodeId, SignalType, Wire};
use egui::{Align2, Pos2, Rect, RichText, vec2};

use super::controls::{self, FieldResult};
use super::live::source_path;
use super::scene::{Class, Scene};
use super::{Canvas, CanvasEvent, CanvasIo, EditTarget, Hit, InlineEdit, widgets};
use crate::actions::Action;
use crate::edit::{self, GROUP_IN};
use crate::icons;
use crate::theme;

/// What a right-click landed on.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Socket {
        node: NodeId,
        name: String,
        input: bool,
    },
    Node(NodeId),
    Frame(usize),
    Wire(Wire, Pos2),
    Canvas(Pos2),
}

impl Target {
    pub fn from_hit(hit: &Hit, at: Pos2) -> Target {
        match hit {
            Hit::Socket { node, name, input } => Target::Socket {
                node: *node,
                name: name.clone(),
                input: *input,
            },
            Hit::FrameHeader(i) | Hit::FrameResize(i) => Target::Frame(*i),
            Hit::Wire(w) => Target::Wire(w.clone(), at),
            Hit::None => Target::Canvas(at),
            other => match other.node() {
                Some(n) => Target::Node(n),
                None => Target::Canvas(at),
            },
        }
    }
}

fn item(ui: &mut egui::Ui, icon: &str, label: &str, shortcut: Option<Action>) -> bool {
    let text = format!("{icon}  {label}");
    let mut b = egui::Button::new(text).frame(false);
    if let Some(sc) = shortcut.and_then(|a| a.shortcut()) {
        b = b.shortcut_text(ui.ctx().format_shortcut(&sc));
    }
    ui.add(b).clicked()
}

impl Canvas {
    pub(super) fn show_menus(&mut self, ui: &egui::Ui, io: &mut CanvasIo, scene: &Scene, now: f64) {
        let ctx = ui.ctx().clone();
        if let Some((target, at)) = self.menu.clone() {
            let mut close = false;
            let area = egui::Area::new(egui::Id::unique("canvas-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(at)
                .constrain(true)
                .show(&ctx, |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        ui.set_min_width(220.0);
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            close = self.menu_items(ui, io, scene, &target, now);
                        });
                    });
                });
            // The click that opened the menu doesn't close it.
            let clicked_outside = self.menu_age > 0
                && ctx.input(|i| i.pointer.any_pressed())
                && !area.response.contains_pointer();
            if close || clicked_outside {
                self.menu = None;
            }
        }
        self.menu_age = if self.menu.is_some() {
            self.menu_age + 1
        } else {
            0
        };
        if let Some((node, socket, at)) = self.choice.clone() {
            let close = match scene.node(node) {
                Some(n) => controls::choice_popup(
                    &ctx,
                    egui::Id::unique("canvas-choice"),
                    io,
                    n,
                    &socket,
                    at,
                    self.choice_age,
                    now,
                ),
                None => true,
            };
            if close {
                self.choice = None;
            }
        }
        self.choice_age = if self.choice.is_some() {
            self.choice_age + 1
        } else {
            0
        };
    }

    /// The items of a right-click menu. Returns true to close it.
    fn menu_items(
        &mut self,
        ui: &mut egui::Ui,
        io: &mut CanvasIo,
        scene: &Scene,
        target: &Target,
        now: f64,
    ) -> bool {
        let run = |io: &mut CanvasIo, a: Action| io.events.push(CanvasEvent::Run(a));
        match target {
            Target::Socket { node, name, input } => {
                let ep = Endpoint::new(*node, name.clone());
                let graph = io.doc.display_graph();
                let wired = if *input {
                    graph.wire_into(&ep).is_some()
                } else {
                    graph.wires_from(*node).any(|w| w.from.socket == *name)
                };
                let iface = edit::is_virtual(*node);
                ui.label(
                    RichText::new(super::paint::pretty(name))
                        .color(theme::TEXT_MUTED)
                        .small(),
                );
                if wired && item(ui, icons::LINK_BREAK, "Disconnect", None) {
                    io.doc.edit(io.registry, "Disconnect", |g, _| {
                        if *input {
                            g.disconnect_input(&ep);
                        } else {
                            g.wires.retain(|w| w.from != ep);
                        }
                    });
                    return true;
                }
                if wired && !iface && item(ui, icons::CROSSHAIR, "Add Reroute", None) {
                    let w = if *input {
                        graph.wire_into(&ep).cloned()
                    } else {
                        graph
                            .wires_from(*node)
                            .find(|w| w.from.socket == *name)
                            .cloned()
                    };
                    if let Some(w) = w
                        && let Some(s) = scene.node(*node).and_then(|n| n.socket(name, *input))
                    {
                        let p = s.pos + vec2(if *input { -40.0 } else { 40.0 }, 0.0);
                        let new = io.doc.edit(io.registry, "Add reroute", |g, ctx| {
                            edit::add_reroutes(g, ctx, &[(w, [p.x, p.y])])
                        });
                        self.mark_born(&new, now);
                    }
                    return true;
                }
                if io.doc.current_def().is_some() && !iface {
                    if *input
                        && !wired
                        && item(ui, icons::ARROW_SQUARE_IN, "Expose as Group Input", None)
                    {
                        io.doc.edit(io.registry, "Expose input", |g, ctx| {
                            edit::expose_input(g, ctx, &ep);
                        });
                        return true;
                    }
                    if !*input && item(ui, icons::ARROW_SQUARE_OUT, "Expose as Group Output", None)
                    {
                        io.doc.edit(io.registry, "Expose output", |g, ctx| {
                            edit::expose_output(g, ctx, &ep);
                        });
                        return true;
                    }
                }
                if iface && !name.is_empty() {
                    let input_side = *node == GROUP_IN;
                    if item(ui, icons::PENCIL, "Rename Socket", None) {
                        if let Some(s) = scene.node(*node).and_then(|n| n.socket(name, *input)) {
                            self.edit = Some(InlineEdit {
                                target: EditTarget::Interface {
                                    input_side,
                                    name: name.clone(),
                                },
                                text: name.clone(),
                                rect: super::field_rect(s),
                                multiline: false,
                                started: false,
                            });
                        }
                        return true;
                    }
                    if item(ui, icons::TRASH, "Remove from Group Interface", None) {
                        io.doc.edit(io.registry, "Remove group socket", |g, ctx| {
                            edit::remove_interface_socket(g, ctx, input_side, name);
                        });
                        return true;
                    }
                }
                if *input && !iface {
                    let bindable = scene
                        .node(*node)
                        .and_then(|n| n.socket(name, true))
                        .and_then(|s| s.spec.as_ref())
                        .is_some_and(ac_core::binding::bindable);
                    if bindable && !wired && item(ui, icons::LINK, "Bind…", None) {
                        io.events.push(CanvasEvent::Bind {
                            node: *node,
                            socket: name.clone(),
                        });
                        return true;
                    }
                    if io.bound_input(*node, name).is_some()
                        && item(ui, icons::LINK_BREAK, "Remove Bindings", None)
                    {
                        let target = ac_core::Target::Input {
                            node: io.doc.node_path(*node),
                            socket: name.clone(),
                        };
                        io.doc.edit_project("Remove bindings", |p| {
                            p.bindings.retain(|b| b.target != target);
                        });
                        return true;
                    }
                }
                if !iface && item(ui, icons::BOOK_OPEN, "Node Help", Some(Action::NodeHelp)) {
                    io.events.push(CanvasEvent::ShowHelp(*node));
                    return true;
                }
                false
            }
            Target::Node(node) => {
                let Some(n) = scene.node(*node) else {
                    return true;
                };
                if matches!(n.class, Class::Interface { .. }) {
                    ui.label(
                        RichText::new("Group interface")
                            .color(theme::TEXT_MUTED)
                            .small(),
                    );
                    ui.label(RichText::new("Drag to its empty socket to add one;\nright-click a socket to rename or remove it.").small());
                    return false;
                }
                ui.label(RichText::new(&n.title).color(theme::TEXT_MUTED).small());
                if let Class::Group(_) = n.class
                    && item(
                        ui,
                        icons::ARROW_SQUARE_IN,
                        "Enter Group",
                        Some(Action::EnterLeaveGroup),
                    )
                {
                    io.doc.enter_group(*node);
                    return true;
                }
                for (icon, a) in [
                    (icons::PENCIL, Action::Rename),
                    (icons::SPEAKER_SLASH, Action::Mute),
                    (icons::CARET_DOWN, Action::Collapse),
                    (icons::SELECTION, Action::HideUnwired),
                    (icons::COPY, Action::Duplicate),
                    (icons::STACK, Action::Group),
                ] {
                    if item(ui, icon, a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                if let Class::Group(_) = n.class
                    && item(
                        ui,
                        icons::ARROW_SQUARE_OUT,
                        "Ungroup",
                        Some(Action::Ungroup),
                    )
                {
                    run(io, Action::Ungroup);
                    return true;
                }
                if n.kind == "clip" && item(ui, icons::FILE_AUDIO, "Choose File…", None) {
                    io.events.push(CanvasEvent::ChooseClipFile(*node));
                    return true;
                }
                // A control binds its main value (an XY Pad either axis).
                let main = crate::bindings::control_inputs(&n.kind);
                for socket in main {
                    let free = n.socket(socket, true).is_some_and(|s| !s.wired);
                    let label = if main.len() > 1 {
                        format!("Bind {}…", socket.to_uppercase())
                    } else {
                        "Bind…".to_string()
                    };
                    if free && item(ui, icons::LINK, &label, None) {
                        io.events.push(CanvasEvent::Bind {
                            node: *node,
                            socket: socket.to_string(),
                        });
                        return true;
                    }
                }
                if pin_items(ui, io, n) {
                    return true;
                }
                ui.separator();
                for (icon, a) in [
                    (icons::TRASH, Action::Delete),
                    (icons::SCISSORS, Action::DeleteReconnect),
                ] {
                    if item(ui, icon, a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                ui.separator();
                if item(ui, icons::BOOK_OPEN, "Node Help", Some(Action::NodeHelp)) {
                    io.events.push(CanvasEvent::ShowHelp(*node));
                    return true;
                }
                false
            }
            Target::Frame(i) => {
                if item(ui, icons::PENCIL, "Rename Frame", None) {
                    if let Some(f) = io.doc.graph().frames.get(*i) {
                        let r = super::frame_rect(f);
                        self.edit = Some(InlineEdit {
                            target: EditTarget::Frame(*i),
                            text: f.label.clone(),
                            rect: Rect::from_min_size(r.min, vec2(r.width(), theme::FRAME_HEADER))
                                .shrink2(vec2(8.0, 4.0)),
                            multiline: false,
                            started: false,
                        });
                    }
                    return true;
                }
                if item(ui, icons::TRASH, "Remove Frame (keep nodes)", None) {
                    let i = *i;
                    io.doc.edit(io.registry, "Remove frame", |g, _| {
                        if i < g.frames.len() {
                            g.frames.remove(i);
                        }
                    });
                    return true;
                }
                false
            }
            Target::Wire(w, at) => {
                if item(ui, icons::LINK_BREAK, "Disconnect", None) {
                    let w = w.clone();
                    io.doc.edit(io.registry, "Disconnect", |g, _| {
                        g.wires.retain(|x| *x != w)
                    });
                    return true;
                }
                if item(ui, icons::CROSSHAIR, "Add Reroute Here", None) {
                    let crossing = (w.clone(), [at.x, at.y]);
                    let new = io.doc.edit(io.registry, "Add reroute", |g, ctx| {
                        edit::add_reroutes(g, ctx, &[crossing])
                    });
                    self.mark_born(&new, now);
                    return true;
                }
                false
            }
            Target::Canvas(at) => {
                if item(ui, icons::PLUS, "Add Node…", Some(Action::AddNode)) {
                    let screen = self.transform().to_screen(*at);
                    io.events.push(CanvasEvent::OpenPalette {
                        screen,
                        canvas: *at,
                        pending: None,
                    });
                    return true;
                }
                for (icon, a) in [
                    (icons::CLIPBOARD, Action::Paste),
                    (icons::SELECTION, Action::SelectAll),
                    (icons::FRAME_CORNERS, Action::Frame),
                    (icons::CORNERS_OUT, Action::FitAll),
                ] {
                    if item(ui, icon, a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                if !io.doc.scope.is_empty()
                    && item(
                        ui,
                        icons::ARROW_LEFT,
                        "Leave Group",
                        Some(Action::EnterLeaveGroup),
                    )
                {
                    io.doc.leave_group();
                    return true;
                }
                false
            }
        }
    }

    /// The text field for typing a value, a label, or a note.
    pub(super) fn show_inline_edit(&mut self, ui: &mut egui::Ui, io: &mut CanvasIo, now: f64) {
        let Some(mut e) = self.edit.take() else {
            return;
        };
        let t = self.transform();
        let z = self.view.zoom;
        let rect = t.rect_to_screen(e.rect);
        let font = theme::canvas_font(
            if e.multiline {
                theme::FONT_NODE_TITLE
            } else {
                theme::FONT_VALUE
            },
            z.max(0.6),
            !e.multiline,
        );
        let id = egui::Id::unique("canvas-inline-edit");
        let multiline = e.multiline;
        match controls::text_field(ui, id, rect, font, &mut e.text, &mut e.started, multiline) {
            FieldResult::Editing => {
                self.edit = Some(e);
                return;
            }
            FieldResult::Cancel => return,
            FieldResult::Commit => {}
        }
        match &e.target {
            EditTarget::Param { node, socket, spec } => {
                controls::commit_value(io, *node, socket, spec, &e.text, now);
            }
            EditTarget::Label(node) => {
                let text = e.text.trim().to_string();
                let node = *node;
                let default = io
                    .doc
                    .graph()
                    .node(node)
                    .and_then(|n| io.registry.get(&n.kind).map(|en| en.meta.name.to_string()));
                io.doc.edit(io.registry, "Rename", |g, _| {
                    if let Some(n) = g.node_mut(node) {
                        n.ui.label =
                            (!text.is_empty() && Some(&text) != default.as_ref()).then_some(text);
                    }
                });
            }
            EditTarget::Comment(node) => {
                let text = e.text.clone();
                let node = *node;
                io.doc.edit(io.registry, "Edit note", |g, _| {
                    if let Some(n) = g.node_mut(node) {
                        if !n.config.is_object() {
                            n.config = ac_core::empty_config();
                        }
                        if let Some(obj) = n.config.as_object_mut() {
                            obj.insert("text".into(), text.into());
                        }
                    }
                });
            }
            EditTarget::Frame(i) => {
                let (i, text) = (*i, e.text.trim().to_string());
                io.doc.edit(io.registry, "Rename frame", |g, _| {
                    if let Some(f) = g.frames.get_mut(i)
                        && !text.is_empty()
                    {
                        f.label = text;
                    }
                });
            }
            EditTarget::Interface { input_side, name } => {
                io.doc
                    .rename_interface_socket(io.registry, *input_side, name, &e.text);
            }
        }
    }

    /// Tooltips: sockets (type, unit, rate, live value, docs), badges.
    pub(super) fn show_tooltips(&self, ui: &egui::Ui, io: &mut CanvasIo, scene: &Scene) {
        if !self.is_idle() || self.menu.is_some() || self.edit.is_some() {
            return;
        }
        let Some(p) = self.pointer else { return };
        if !self.screen.contains(p) {
            return;
        }
        let ctx = ui.ctx();
        let still = ctx.input(|i| i.pointer.time_since_last_movement() > 0.25);
        if !still {
            if ctx.input(|i| i.pointer.is_moving()) {
                return;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(260));
            return;
        }
        if let Some((_, tip)) = self.tips.iter().find(|(r, _)| r.contains(p)) {
            tooltip(ui, p, |ui| {
                ui.set_max_width(320.0);
                ui.label(tip.as_str());
            });
            return;
        }
        let Hit::Socket { node, name, input } = self.hit(scene, io.doc, p) else {
            return;
        };
        let Some(n) = scene.node(node) else { return };
        let Some(s) = n.socket(&name, input) else {
            return;
        };
        let tele = io.engine.telemetry();
        let graph = io.doc.display_graph();
        let reading = if input {
            graph
                .wire_into(&Endpoint::new(node, name.clone()))
                .and_then(|w| source_path(io.doc, &w.from))
        } else {
            source_path(io.doc, &Endpoint::new(node, name.clone()))
        };
        tooltip(ui, p, |ui| {
            ui.set_max_width(300.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if s.is_new_slot() {
                        "new socket".to_string()
                    } else {
                        super::paint::pretty(&s.name)
                    })
                    .strong()
                    .color(theme::TEXT_STRONG),
                );
                ui.label(RichText::new(s.ty.name()).color(theme::type_color(s.ty)));
                if let Some(spec) = &s.spec
                    && !spec.unit.suffix().is_empty()
                {
                    ui.label(RichText::new(spec.unit.suffix()).color(theme::TEXT_MUTED));
                }
            });
            let mut facts = Vec::new();
            if input {
                facts.push(if s.wired {
                    "wired".to_string()
                } else {
                    "unwired: set inline (Const rate)".to_string()
                });
            } else if let Some(spec) = &s.spec {
                facts.push(match spec.rate {
                    ac_core::OutRate::Fixed(r) => format!("{r:?} rate").to_lowercase(),
                    ac_core::OutRate::Poly => "rate follows inputs".to_string(),
                });
            }
            if let Some(spec) = &s.spec
                && s.ty == SignalType::Audio
            {
                facts.push(match spec.channels {
                    ac_core::Channels::Fixed(n) => format!("{n} ch"),
                    ac_core::Channels::Follow => "channels follow input".into(),
                    ac_core::Channels::UpTo(n) => format!("up to {n} ch"),
                });
            }
            if let Some(spec) = &s.spec
                && let Some((a, b)) = spec.range
            {
                facts.push(format!(
                    "range {} – {}",
                    widgets::format(a, spec),
                    widgets::format(b, spec)
                ));
            }
            if !facts.is_empty() {
                ui.label(
                    RichText::new(facts.join(" · "))
                        .small()
                        .color(theme::TEXT_MUTED),
                );
            }
            if let Some((path, sock)) = &reading {
                let value = tele.socket_value(path, sock);
                let level = tele.socket_level(path, sock);
                let text = match s.ty {
                    SignalType::Audio => level.map(|l| {
                        if l < 1e-6 {
                            "silent".to_string()
                        } else {
                            format!("peak {:.1} dBFS", 20.0 * l.log10())
                        }
                    }),
                    SignalType::Trigger | SignalType::Midi => tele
                        .socket_events(path, sock)
                        .map(|c| format!("{c} events")),
                    SignalType::Text => None,
                    ty => value.map(|v| widgets::live_text(v, ty, s.spec.as_ref())),
                };
                if let Some(t) = text {
                    ui.label(
                        RichText::new(t)
                            .font(theme::mono_font(18.0))
                            .color(theme::TEXT_STRONG),
                    );
                }
            } else if input
                && !s.wired
                && let (Some(spec), Some(node)) = (&s.spec, io.doc.graph().node(node))
            {
                let text = match widgets::field(spec) {
                    widgets::Field::Number => widgets::format(widgets::number(node, spec), spec),
                    widgets::Field::Choice => {
                        widgets::choice_label(spec, &widgets::choice(node, spec))
                    }
                    widgets::Field::Check => {
                        if widgets::value(node, spec).as_bool().unwrap_or(false) {
                            "on".into()
                        } else {
                            "off".into()
                        }
                    }
                    widgets::Field::Text => widgets::text(node, spec),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    ui.label(
                        RichText::new(text)
                            .font(theme::mono_font(18.0))
                            .color(theme::TEXT_STRONG),
                    );
                }
            }
            if let Some(doc) = s.spec.as_ref().map(|s| s.doc).filter(|d| !d.is_empty()) {
                ui.label(RichText::new(doc).color(theme::TEXT));
            }
            if s.is_new_slot() {
                ui.label("Drop a wire here to add a socket to the group's interface.");
            }
        });
    }
}

/// The Pin to Panel entries of a node's menu: pin the selection to each
/// panel it isn't on, or to a new one, and unpin it from the ones it is.
/// Returns true when one was chosen.
fn pin_items(ui: &mut egui::Ui, io: &mut CanvasIo, n: &super::scene::NodeView) -> bool {
    if n.body.is_none() {
        return false;
    }
    ui.separator();
    if !io.doc.scope.is_empty() {
        ui.label(
            RichText::new(format!(
                "{}  Pin to Panel: top-level nodes only",
                icons::PUSH_PIN
            ))
            .small()
            .color(theme::TEXT_FAINT),
        );
        return false;
    }
    // The right-clicked node is in the selection; pin all of it.
    let nodes: Vec<NodeId> = if io.doc.selection.contains(&n.id) {
        io.doc
            .selection
            .iter()
            .copied()
            .filter(|id| !edit::is_virtual(*id))
            .collect()
    } else {
        vec![n.id]
    };
    let panels: Vec<(usize, String, bool)> = io
        .doc
        .project
        .panels
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.name.clone(), p.contains(n.id)))
        .collect();
    let pin = |panel| crate::perform::PinRequest::Pin {
        nodes: nodes.clone(),
        panel,
    };
    for (i, name, on) in &panels {
        if !on && item(ui, icons::PUSH_PIN, &format!("Pin to {name}"), None) {
            io.events.push(CanvasEvent::Pin(pin(Some(*i))));
            return true;
        }
    }
    if item(ui, icons::PUSH_PIN, "Pin to New Panel", None) {
        io.events.push(CanvasEvent::Pin(pin(None)));
        return true;
    }
    for (i, name, on) in &panels {
        if *on
            && item(
                ui,
                icons::PUSH_PIN_SLASH,
                &format!("Unpin from {name}"),
                None,
            )
        {
            io.events
                .push(CanvasEvent::Pin(crate::perform::PinRequest::Unpin {
                    nodes: nodes.clone(),
                    panel: Some(*i),
                }));
            return true;
        }
    }
    false
}

fn tooltip(ui: &egui::Ui, p: Pos2, add: impl FnOnce(&mut egui::Ui)) {
    egui::Area::new(egui::Id::unique("canvas-tooltip"))
        .order(egui::Order::Tooltip)
        .fixed_pos(p + vec2(16.0, 16.0))
        .constrain(true)
        .interactable(false)
        .pivot(Align2::LEFT_TOP)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, add);
        });
}
