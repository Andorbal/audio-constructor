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
use crate::i18n::{tr, tr_args};
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
    /// Align, distribute, and auto-layout for the selection.
    Arrange,
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
                        .color(theme::text_muted())
                        .small(),
                );
                if wired && item(ui, icons::LINK_BREAK, &tr("canvas-disconnect"), None) {
                    io.doc.edit(io.registry, &tr("undo-disconnect"), |g, _| {
                        if *input {
                            g.disconnect_input(&ep);
                        } else {
                            g.wires.retain(|w| w.from != ep);
                        }
                    });
                    return true;
                }
                if wired && !iface && item(ui, icons::CROSSHAIR, &tr("canvas-add-reroute"), None) {
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
                        let new = io.doc.edit(io.registry, &tr("undo-add-reroute"), |g, ctx| {
                            edit::add_reroutes(g, ctx, &[(w, [p.x, p.y])])
                        });
                        self.mark_born(&new, now);
                    }
                    return true;
                }
                if io.doc.current_def().is_some() && !iface {
                    if *input
                        && !wired
                        && item(ui, icons::ARROW_SQUARE_IN, &tr("canvas-expose-input"), None)
                    {
                        io.doc
                            .edit(io.registry, &tr("undo-expose-input"), |g, ctx| {
                                edit::expose_input(g, ctx, &ep);
                            });
                        return true;
                    }
                    if !*input
                        && item(
                            ui,
                            icons::ARROW_SQUARE_OUT,
                            &tr("canvas-expose-output"),
                            None,
                        )
                    {
                        io.doc
                            .edit(io.registry, &tr("undo-expose-output"), |g, ctx| {
                                edit::expose_output(g, ctx, &ep);
                            });
                        return true;
                    }
                }
                if iface && !name.is_empty() {
                    let input_side = *node == GROUP_IN;
                    if item(ui, icons::PENCIL, &tr("canvas-rename-socket"), None) {
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
                    if item(ui, icons::TRASH, &tr("canvas-remove-socket"), None) {
                        io.doc
                            .edit(io.registry, &tr("undo-remove-group-socket"), |g, ctx| {
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
                    if bindable && !wired && item(ui, icons::LINK, &tr("canvas-bind"), None) {
                        io.events.push(CanvasEvent::Bind {
                            node: *node,
                            socket: name.clone(),
                        });
                        return true;
                    }
                    if io.bound_input(*node, name).is_some()
                        && item(ui, icons::LINK_BREAK, &tr("canvas-remove-bindings"), None)
                    {
                        let target = ac_core::Target::Input {
                            node: io.doc.node_path(*node),
                            socket: name.clone(),
                        };
                        io.doc.edit_project(&tr("undo-remove-bindings"), |p| {
                            p.bindings.retain(|b| b.target != target);
                        });
                        return true;
                    }
                }
                if !iface
                    && item(
                        ui,
                        icons::BOOK_OPEN,
                        &Action::NodeHelp.label(),
                        Some(Action::NodeHelp),
                    )
                {
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
                        RichText::new(tr("canvas-group-interface"))
                            .color(theme::text_muted())
                            .small(),
                    );
                    ui.label(RichText::new(tr("canvas-group-interface-tip")).small());
                    return false;
                }
                if let Class::Portal(info) = &n.class {
                    return self.portal_items(ui, io, *node, info, now);
                }
                ui.label(RichText::new(&n.title).color(theme::text_muted()).small());
                if n.class == Class::Reroute
                    && item(
                        ui,
                        icons::TAG,
                        &tr("canvas-make-portal"),
                        Some(Action::Portal),
                    )
                {
                    let r = self.portal_from_reroute(io, *node, now);
                    io.events.push(CanvasEvent::Status(r.unwrap_or_else(|e| e)));
                    return true;
                }
                if let Class::Group(_) = n.class
                    && item(
                        ui,
                        icons::ARROW_SQUARE_IN,
                        &tr("canvas-enter-group"),
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
                    if item(ui, icon, &a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                if let Class::Group(_) = n.class
                    && item(
                        ui,
                        icons::ARROW_SQUARE_OUT,
                        &Action::Ungroup.label(),
                        Some(Action::Ungroup),
                    )
                {
                    run(io, Action::Ungroup);
                    return true;
                }
                if n.kind == "clip" && item(ui, icons::FILE_AUDIO, &tr("canvas-choose-file"), None)
                {
                    io.events.push(CanvasEvent::ChooseClipFile(*node));
                    return true;
                }
                if n.kind == ac_core::PLUGIN_KIND
                    && crate::plugins::has_window(&io.doc.node_path(*node).to_string())
                    && item(ui, icons::APP_WINDOW, &tr("plugin-show-window"), None)
                {
                    io.events.push(CanvasEvent::ShowPluginWindow(*node));
                    return true;
                }
                if let Some(all) = io
                    .doc
                    .graph()
                    .node(*node)
                    .filter(|p| {
                        p.kind == ac_core::PLUGIN_KIND
                            && edit::knob_count(p) > edit::PLUGIN_KNOBS_SHOWN
                    })
                    .map(edit::shows_all_knobs)
                {
                    let label = if all {
                        tr("canvas-fewer-knobs")
                    } else {
                        tr("canvas-all-knobs")
                    };
                    if item(ui, icons::SLIDERS, &label, None) {
                        let id = *node;
                        io.doc.edit(io.registry, &tr("undo-show-knobs"), |g, _| {
                            if let Some(p) = g.node_mut(id) {
                                edit::set_show_all_knobs(p, !all);
                            }
                        });
                        return true;
                    }
                }
                // A control binds its main value (an XY Pad either axis).
                let main = crate::bindings::control_inputs(&n.kind);
                for socket in main {
                    let free = n.socket(socket, true).is_some_and(|s| !s.wired);
                    let label = if main.len() > 1 {
                        tr_args(
                            "canvas-bind-axis",
                            &[("axis", socket.to_uppercase().into())],
                        )
                    } else {
                        tr("canvas-bind")
                    };
                    if free && item(ui, icons::LINK, &label, None) {
                        io.events.push(CanvasEvent::Bind {
                            node: *node,
                            socket: socket.to_string(),
                        });
                        return true;
                    }
                }
                let several = io
                    .doc
                    .selection
                    .iter()
                    .filter(|id| !edit::is_virtual(**id))
                    .count();
                if several >= 2 {
                    ui.separator();
                    if several == 2
                        && item(
                            ui,
                            icons::PLUGS_CONNECTED,
                            &Action::LazyConnect.label(),
                            Some(Action::LazyConnect),
                        )
                    {
                        run(io, Action::LazyConnect);
                        return true;
                    }
                    if item(
                        ui,
                        icons::FLOW_ARROW,
                        &Action::AutoLayout.label(),
                        Some(Action::AutoLayout),
                    ) {
                        run(io, Action::AutoLayout);
                        return true;
                    }
                    // Its own menu, in place: a submenu would close with
                    // this one on the click that picks from it.
                    if item(
                        ui,
                        icons::ALIGN_LEFT,
                        &Action::ArrangeMenu.label(),
                        Some(Action::ArrangeMenu),
                    ) {
                        if let Some((_, at)) = self.menu {
                            self.menu = Some((Target::Arrange, at));
                        }
                        return false;
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
                    if item(ui, icon, &a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                ui.separator();
                if item(
                    ui,
                    icons::BOOK_OPEN,
                    &Action::NodeHelp.label(),
                    Some(Action::NodeHelp),
                ) {
                    io.events.push(CanvasEvent::ShowHelp(*node));
                    return true;
                }
                false
            }
            Target::Frame(i) => {
                if item(ui, icons::PENCIL, &tr("canvas-rename-frame"), None) {
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
                if item(ui, icons::TRASH, &tr("canvas-remove-frame"), None) {
                    let i = *i;
                    io.doc.edit(io.registry, &tr("undo-remove-frame"), |g, _| {
                        if i < g.frames.len() {
                            g.frames.remove(i);
                        }
                    });
                    return true;
                }
                false
            }
            Target::Wire(w, at) => {
                if item(ui, icons::LINK_BREAK, &tr("canvas-disconnect"), None) {
                    let w = w.clone();
                    io.doc.edit(io.registry, &tr("undo-disconnect"), |g, _| {
                        g.wires.retain(|x| *x != w)
                    });
                    return true;
                }
                if item(ui, icons::CROSSHAIR, &tr("canvas-add-reroute-here"), None) {
                    let crossing = (w.clone(), [at.x, at.y]);
                    let new = io.doc.edit(io.registry, &tr("undo-add-reroute"), |g, ctx| {
                        edit::add_reroutes(g, ctx, &[crossing])
                    });
                    self.mark_born(&new, now);
                    return true;
                }
                if item(
                    ui,
                    icons::TAG,
                    &tr("canvas-make-portal"),
                    Some(Action::Portal),
                ) {
                    let r = self.portal_from_wire(io, w, now);
                    io.events.push(CanvasEvent::Status(r.unwrap_or_else(|e| e)));
                    return true;
                }
                false
            }
            Target::Arrange => {
                ui.label(
                    RichText::new(tr("canvas-arrange-selection"))
                        .color(theme::text_muted())
                        .small(),
                );
                for (i, (icon, a)) in arrange_items().into_iter().enumerate() {
                    if i == 1 || i == 7 {
                        ui.separator();
                    }
                    if item(ui, icon, &a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                false
            }
            Target::Canvas(at) => {
                if item(
                    ui,
                    icons::PLUS,
                    &Action::AddNode.label(),
                    Some(Action::AddNode),
                ) {
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
                    if item(ui, icon, &a.label(), Some(a)) {
                        run(io, a);
                        return true;
                    }
                }
                if !io.doc.scope.is_empty()
                    && item(
                        ui,
                        icons::ARROW_LEFT,
                        &tr("canvas-leave-group"),
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

    /// The menu of a portal tag: jump to the other end, rename, back to a
    /// wire, and the usual duplicate and delete.
    fn portal_items(
        &mut self,
        ui: &mut egui::Ui,
        io: &mut CanvasIo,
        node: NodeId,
        info: &edit::portal::Info,
        now: f64,
    ) -> bool {
        let _ = now;
        let role = if info.send {
            tr("canvas-portal-sender")
        } else {
            tr("canvas-portal-receiver")
        };
        ui.label(
            RichText::new(tr_args(
                "canvas-portal-title",
                &[("name", (&info.name).into()), ("role", role.into())],
            ))
            .color(theme::text_muted())
            .small(),
        );
        let jump = match (info.send, info.partners.len()) {
            (_, 0) => None,
            (true, 1) => Some(tr("canvas-jump-receiver")),
            (true, n) => Some(tr_args("canvas-jump-next-receiver", &[("count", n.into())])),
            (false, _) => Some(tr("canvas-jump-sender")),
        };
        if let Some(label) = jump
            && item(ui, icons::ARROW_SQUARE_OUT, &label, None)
        {
            self.jump_to_partner(io.doc, io.registry, node);
            return true;
        }
        for (icon, a, label) in [
            (icons::PENCIL, Action::Rename, tr("canvas-rename-portal")),
            (icons::LINK, Action::Portal, tr("canvas-back-to-wire")),
            (icons::COPY, Action::Duplicate, Action::Duplicate.label()),
        ] {
            if item(ui, icon, &label, Some(a)) {
                io.events.push(CanvasEvent::Run(a));
                return true;
            }
        }
        ui.separator();
        if item(
            ui,
            icons::TRASH,
            &Action::Delete.label(),
            Some(Action::Delete),
        ) {
            io.events.push(CanvasEvent::Run(Action::Delete));
            return true;
        }
        false
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
                io.doc.edit(io.registry, &tr("undo-rename"), |g, _| {
                    if let Some(n) = g.node_mut(node) {
                        n.ui.label =
                            (!text.is_empty() && Some(&text) != default.as_ref()).then_some(text);
                    }
                });
            }
            EditTarget::Portal(node) => {
                let (node, text) = (*node, e.text.clone());
                io.doc.edit(io.registry, &tr("undo-rename-portal"), |g, _| {
                    edit::portal::rename(g, node, &text);
                });
            }
            EditTarget::Comment(node) => {
                let text = e.text.clone();
                let node = *node;
                io.doc.edit(io.registry, &tr("undo-edit-note"), |g, _| {
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
                io.doc.edit(io.registry, &tr("undo-rename-frame"), |g, _| {
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
        if !self.screen.contains(p) || self.minimap_now.is_some_and(|m| m.rect.contains(p)) {
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
                        tr("canvas-new-socket")
                    } else {
                        super::paint::pretty(&s.name)
                    })
                    .strong()
                    .color(theme::text_strong()),
                );
                ui.label(RichText::new(s.ty.name()).color(theme::type_color(s.ty)));
                if let Some(spec) = &s.spec
                    && !spec.unit.suffix().is_empty()
                {
                    ui.label(RichText::new(spec.unit.suffix()).color(theme::text_muted()));
                }
            });
            let mut facts = Vec::new();
            if input {
                facts.push(if s.wired {
                    tr("canvas-tip-wired")
                } else {
                    tr("canvas-tip-unwired")
                });
            } else if let Some(spec) = &s.spec {
                facts.push(match spec.rate {
                    ac_core::OutRate::Fixed(r) => tr_args(
                        "canvas-tip-rate",
                        &[("rate", format!("{r:?}").to_lowercase().into())],
                    ),
                    ac_core::OutRate::Poly => tr("canvas-tip-rate-follows"),
                });
            }
            if let Some(spec) = &s.spec
                && s.ty == SignalType::Audio
            {
                facts.push(match spec.channels {
                    ac_core::Channels::Fixed(n) => {
                        tr_args("canvas-tip-channels", &[("count", n.into())])
                    }
                    ac_core::Channels::Follow => tr("canvas-tip-channels-follow"),
                    ac_core::Channels::UpTo(n) => {
                        tr_args("canvas-tip-channels-up-to", &[("count", n.into())])
                    }
                });
            }
            if let Some(spec) = &s.spec
                && let Some((a, b)) = spec.range
            {
                facts.push(tr_args(
                    "canvas-tip-range",
                    &[
                        ("from", widgets::format(a, spec).into()),
                        ("to", widgets::format(b, spec).into()),
                    ],
                ));
            }
            if !facts.is_empty() {
                ui.label(
                    RichText::new(facts.join(" · "))
                        .small()
                        .color(theme::text_muted()),
                );
            }
            if let Some((path, sock)) = &reading {
                let value = tele.socket_value(path, sock);
                let level = tele.socket_level(path, sock);
                let text = match s.ty {
                    SignalType::Audio => level.map(|l| {
                        if l < 1e-6 {
                            tr("canvas-tip-silent")
                        } else {
                            tr_args(
                                "canvas-tip-peak",
                                &[("db", format!("{:.1}", 20.0 * l.log10()).into())],
                            )
                        }
                    }),
                    SignalType::Trigger | SignalType::Midi => tele
                        .socket_events(path, sock)
                        .map(|c| tr_args("canvas-tip-events", &[("count", c.into())])),
                    SignalType::Text => None,
                    ty => value.map(|v| widgets::live_text(v, ty, s.spec.as_ref())),
                };
                if let Some(t) = text {
                    ui.label(
                        RichText::new(t)
                            .font(theme::mono_font(18.0))
                            .color(theme::text_strong()),
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
                            tr("canvas-tip-on")
                        } else {
                            tr("canvas-tip-off")
                        }
                    }
                    widgets::Field::Text => widgets::text(node, spec),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    ui.label(
                        RichText::new(text)
                            .font(theme::mono_font(18.0))
                            .color(theme::text_strong()),
                    );
                }
            }
            if let Some(doc) = s.spec.as_ref().map(|s| s.doc).filter(|d| !d.is_empty()) {
                ui.label(RichText::new(doc).color(theme::text()));
            }
            if s.is_new_slot() {
                ui.label(tr("canvas-tip-new-socket"));
            }
        });
    }
}

/// The Pin to Panel entries of a node's menu: pin the selection to each
/// panel it isn't on, or to a new one, and unpin it from the ones it is.
/// Returns true when one was chosen.
fn pin_items(ui: &mut egui::Ui, io: &mut CanvasIo, n: &super::scene::NodeView) -> bool {
    let pinnable = io.doc.graph().node(n.id).is_some_and(edit::pins::pinnable);
    let generates = io
        .doc
        .graph()
        .node(n.id)
        .is_some_and(|x| matches!(x.kind.as_str(), "sampler" | "clip"));
    if !pinnable && !generates {
        return false;
    }
    ui.separator();
    if !io.doc.scope.is_empty() {
        ui.label(
            RichText::new(format!(
                "{}  {}",
                icons::PUSH_PIN,
                tr("canvas-pin-top-level-only")
            ))
            .small()
            .color(theme::text_faint()),
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
        if !on
            && item(
                ui,
                icons::PUSH_PIN,
                &tr_args("canvas-pin-to", &[("panel", name.into())]),
                None,
            )
        {
            io.events.push(CanvasEvent::Pin(pin(Some(*i))));
            return true;
        }
    }
    if pinnable && item(ui, icons::PUSH_PIN, &tr("canvas-pin-to-new"), None) {
        io.events.push(CanvasEvent::Pin(pin(None)));
        return true;
    }
    if pinnable && item(ui, icons::SQUARES_FOUR, &tr("canvas-pin-as-strip"), None) {
        io.events
            .push(CanvasEvent::Pin(crate::perform::PinRequest::Strip {
                nodes: nodes.clone(),
                panel: None,
            }));
        return true;
    }
    let mut made = None;
    ui.menu_button(
        format!(
            "{}  {}",
            icons::SQUARES_FOUR,
            tr("canvas-panel-from-selection")
        ),
        |ui| {
            use crate::perform::Generated;
            for (kind, label) in [
                (Generated::Mixer, tr("canvas-mixer-from-selection")),
                (Generated::Pads, tr("canvas-pads-from-selection")),
                (Generated::Grid, tr("canvas-grid-from-selection")),
            ] {
                if ui.button(label).clicked() {
                    made = Some(kind);
                    ui.close();
                }
            }
        },
    );
    if let Some(kind) = made {
        io.events.push(CanvasEvent::Pin(
            crate::perform::PinRequest::FromSelection {
                nodes: nodes.clone(),
                kind,
            },
        ));
        return true;
    }
    for (i, name, on) in &panels {
        if *on
            && item(
                ui,
                icons::PUSH_PIN_SLASH,
                &tr_args("canvas-unpin-from", &[("panel", name.into())]),
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

/// The arrange menu's entries: auto-layout, the six alignments, and the
/// two ways to distribute (the Edit menu lists the same).
pub fn arrange_items() -> [(&'static str, Action); 9] {
    [
        (icons::FLOW_ARROW, Action::AutoLayout),
        (icons::ALIGN_LEFT, Action::AlignLeft),
        (icons::ALIGN_CENTER_HORIZONTAL, Action::AlignCenter),
        (icons::ALIGN_RIGHT, Action::AlignRight),
        (icons::ALIGN_TOP, Action::AlignTop),
        (icons::ALIGN_CENTER_VERTICAL, Action::AlignMiddle),
        (icons::ALIGN_BOTTOM, Action::AlignBottom),
        (
            icons::ARROWS_OUT_LINE_HORIZONTAL,
            Action::DistributeHorizontally,
        ),
        (
            icons::ARROWS_OUT_LINE_VERTICAL,
            Action::DistributeVertically,
        ),
    ]
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
