//! The canvas for screen readers (docs/UX.md "Accessibility and
//! scaling"). The canvas hit-tests and paints by hand, so egui knows
//! nothing of its cards and sockets; each frame this gives every card and
//! socket in view a widget of its own that senses nothing (the pointer
//! still goes to the canvas) but can take keyboard focus, with a name, a
//! role, and its state for AccessKit. The canvas's keyboard focus
//! ([`Focus`], moved by `keys.rs`) is kept in step with egui's, so a
//! screen reader follows Tab, and a screen reader moving its own focus
//! moves the canvas's.

use std::collections::HashMap;

use ac_core::{Endpoint, NodeId};
use egui::accesskit;
use egui::{Rect, Sense, vec2};

use super::scene::{Class, NodeView, Scene, SocketView};
use super::{Canvas, CanvasIo, widgets};
use crate::i18n::{tr, tr_args};
use crate::theme;

/// What has the canvas's keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Focus {
    Node(NodeId),
    Socket {
        node: NodeId,
        name: String,
        input: bool,
    },
}

impl Focus {
    /// The node the focus is on or in.
    pub fn node(&self) -> NodeId {
        match self {
            Focus::Node(n) | Focus::Socket { node: n, .. } => *n,
        }
    }

    /// The widget a focus is drawn as, for egui and AccessKit.
    pub fn widget_id(&self) -> egui::Id {
        let base = egui::Id::unique("canvas-a11y");
        match self {
            Focus::Node(n) => base.with(("node", n.0)),
            Focus::Socket { node, name, input } => {
                base.with(("socket", node.0, name.as_str(), *input))
            }
        }
    }
}

impl Canvas {
    /// Whether egui's keyboard focus is on one of the canvas's cards or
    /// sockets (last frame's), so single-key shortcuts still reach the
    /// canvas.
    pub fn owns_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.focused())
            .is_some_and(|id| self.a11y_ids.contains_key(&id))
    }

    /// Give the canvas the keyboard (F6, or Tab from the panels), on the
    /// node it had or the first one.
    pub fn take_keyboard(&mut self) {
        self.focus_visible = true;
    }

    /// Hide the focus ring and hand the keyboard back (Esc with nothing
    /// to cancel, or a click on the canvas).
    pub fn drop_keyboard(&mut self, ctx: &egui::Context) {
        self.focus_visible = false;
        if let Some(f) = &self.focus {
            let id = f.widget_id();
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }

    /// The canvas, its cards, and their sockets for AccessKit, and egui's
    /// focus kept in step with the canvas's.
    pub(super) fn accessibility(
        &mut self,
        ui: &mut egui::Ui,
        io: &mut CanvasIo,
        scene: &Scene,
        canvas: &egui::Response,
    ) {
        let label = if io.doc.scope.is_empty() {
            tr_args("a11y-canvas", &[("count", scene.nodes.len().into())])
        } else {
            tr_args(
                "a11y-canvas-group",
                &[
                    ("count", scene.nodes.len().into()),
                    ("group", io.doc.breadcrumb().join(" › ").into()),
                ],
            )
        };
        canvas.widget_info(|| egui::WidgetInfo::labeled(accesskit::Role::Canvas, true, &label));

        let t = self.transform();
        let mut ids = HashMap::new();
        let mut clicked = None;
        for n in &scene.nodes {
            let card = t.rect_to_screen(n.rect);
            let holds_focus = self.focus.as_ref().is_some_and(|f| f.node() == n.id);
            if !card.intersects(self.screen) && !holds_focus {
                continue;
            }
            let f = Focus::Node(n.id);
            let id = f.widget_id();
            let r = ui.interact(card, id, Sense::focusable_noninteractive());
            let name = node_label(io, n);
            let selected = n.selected;
            ui.ctx().accesskit_node_builder(id, |node| {
                node.set_role(accesskit::Role::Button);
                node.set_label(name);
                node.set_selected(selected);
                node.add_action(accesskit::Action::Click);
                if n.collapsed {
                    node.set_expanded(false);
                }
            });
            if ui.input(|i| i.has_accesskit_action_request(id, accesskit::Action::Click)) {
                clicked = Some(f.clone());
            }
            drop(r);
            ids.insert(id, f);
            if n.collapsed || matches!(n.class, Class::Comment) {
                continue;
            }
            for s in n.inputs.iter().chain(n.outputs.iter()) {
                if s.is_new_slot() {
                    continue;
                }
                let f = Focus::Socket {
                    node: n.id,
                    name: s.name.clone(),
                    input: s.input,
                };
                let id = f.widget_id();
                let hit = theme::SOCKET_HIT.max(theme::SOCKET_RADIUS * self.view.zoom + 3.0);
                let rect = Rect::from_center_size(t.to_screen(s.pos), vec2(hit, hit) * 2.0);
                let _ = ui.interact(rect, id, Sense::focusable_noninteractive());
                let name = socket_label(io, scene, n, s);
                ui.ctx().accesskit_node_builder(id, |node| {
                    node.set_role(accesskit::Role::Button);
                    node.set_label(name);
                });
                ids.insert(id, f);
            }
        }
        self.a11y_ids = ids;

        // A screen reader's "activate" on a card is a click: select it.
        if let Some(f) = clicked {
            io.doc.selection = [f.node()].into();
            self.focus = Some(f);
            self.focus_visible = true;
        }
        self.sync_focus(ui.ctx(), io);
    }

    /// egui's keyboard focus and the canvas's, in step. Whichever moved
    /// since last frame leads: the canvas's keys (and a node just added)
    /// move egui's focus, and Tab from the panels or a screen reader moves
    /// the canvas's.
    fn sync_focus(&mut self, ctx: &egui::Context, io: &mut CanvasIo) {
        let moved_here = self.focus != self.synced;
        let focused = ctx.memory(|m| m.focused());
        let want = self
            .focus
            .as_ref()
            .map(Focus::widget_id)
            .filter(|id| self.a11y_ids.contains_key(id));
        match focused.map(|id| (id, self.a11y_ids.get(&id).cloned())) {
            Some((id, Some(f))) => {
                if moved_here {
                    if let Some(want) = want
                        && want != id
                    {
                        ctx.memory_mut(|m| m.request_focus(want));
                    }
                } else if self.focus.as_ref() != Some(&f) {
                    if let Focus::Node(n) = f
                        && !self.keep_selection
                    {
                        io.doc.selection = [n].into();
                    }
                    self.focus = Some(f);
                }
                self.focus_visible = true;
                // Tab, the arrows, and Esc are the canvas's own while it
                // has the keyboard (keys.rs), not egui's to move focus
                // with.
                ctx.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });
            }
            Some((_, None)) => {
                if moved_here && self.focus_visible {
                    // A node added from the keyboard takes the keyboard
                    // from the field or button that added it.
                    if let Some(want) = want {
                        ctx.memory_mut(|m| m.request_focus(want));
                    }
                } else {
                    // A field or a button has the keyboard.
                    self.focus_visible = false;
                }
            }
            None => {
                if self.focus_visible
                    && let Some(want) = want
                {
                    ctx.memory_mut(|m| m.request_focus(want));
                }
            }
        }
        // A focus on something no longer drawn moves to its node, or goes.
        if let Some(f) = &self.focus
            && !self.a11y_ids.values().any(|x| x == f)
        {
            let node = f.node();
            self.focus = self
                .a11y_ids
                .values()
                .any(|x| *x == Focus::Node(node))
                .then_some(Focus::Node(node))
                .or_else(|| {
                    io.doc
                        .graph()
                        .node(node)
                        .map(|_| Focus::Node(node))
                        .filter(|_| matches!(f, Focus::Socket { .. }))
                });
        }
        self.synced = self.focus.clone();
    }
}

/// A card as a screen reader says it: its name, what it is, its sockets,
/// and its state.
pub fn node_label(io: &CanvasIo, n: &NodeView) -> String {
    let what = match &n.class {
        Class::Group(_) => tr("a11y-kind-group"),
        Class::Missing(_) => tr("a11y-kind-missing"),
        Class::Reroute => tr("a11y-kind-reroute"),
        Class::Portal(info) => {
            if info.send {
                tr("a11y-kind-portal-sender")
            } else {
                tr("a11y-kind-portal-receiver")
            }
        }
        Class::Comment => tr("a11y-kind-note"),
        Class::Interface { input_side: true } => tr("node-tab-group-input-title"),
        Class::Interface { input_side: false } => tr("node-tab-group-output-title"),
        Class::Normal => io
            .registry
            .get(&n.kind)
            .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string()),
    };
    let title = if n.class == Class::Comment {
        let text = super::scene::comment_text(io.doc, n.id);
        text.lines().next().unwrap_or_default().to_string()
    } else {
        n.title.clone()
    };
    let inputs = n.inputs.iter().filter(|s| !s.is_new_slot()).count();
    let outputs = n.outputs.iter().filter(|s| !s.is_new_slot()).count();
    // A card titled by its kind says the kind once.
    let id = if title == what {
        "a11y-node-plain"
    } else {
        "a11y-node"
    };
    let args = [
        ("title", title.into()),
        ("kind", what.into()),
        ("inputs", inputs.into()),
        ("outputs", outputs.into()),
    ];
    let mut label = if id == "a11y-node" {
        tr_args("a11y-node", &args)
    } else {
        tr_args("a11y-node-plain", &args)
    };
    let states = [
        n.selected.then(|| tr("a11y-selected")),
        n.muted.then(|| tr("a11y-muted")),
        n.collapsed.then(|| tr("a11y-collapsed")),
    ];
    for state in states.into_iter().flatten() {
        label.push_str(", ");
        label.push_str(&state);
    }
    label
}

/// A socket as a screen reader says it: its card and name, its type and
/// side, and what it's wired to or set to.
pub fn socket_label(io: &CanvasIo, scene: &Scene, n: &NodeView, s: &SocketView) -> String {
    let ep = Endpoint::new(n.id, s.name.clone());
    let name = super::paint::pretty(&s.name);
    let ty = s.ty.name();
    let title_of = |e: &Endpoint| {
        scene.node(e.node).map_or_else(
            || e.node.to_string(),
            |x| format!("{} {}", x.title, super::paint::pretty(&e.socket)),
        )
    };
    let state = if s.input {
        match scene.wires.iter().find(|w| w.wire.to == ep) {
            Some(w) => tr_args("a11y-from", &[("source", title_of(&w.wire.from).into())]),
            None => match (&s.spec, io.shown_node(n.id)) {
                (Some(spec), Some(node)) => match widgets::field(spec) {
                    widgets::Field::Number => widgets::format(widgets::number(&node, spec), spec),
                    widgets::Field::Choice => {
                        widgets::choice_label(spec, &widgets::choice(&node, spec))
                    }
                    widgets::Field::Check => {
                        if widgets::value(&node, spec).as_bool().unwrap_or(false) {
                            tr("canvas-tip-on")
                        } else {
                            tr("canvas-tip-off")
                        }
                    }
                    widgets::Field::Text => widgets::text(&node, spec),
                    widgets::Field::Fire | widgets::Field::Label => tr("a11y-not-connected"),
                },
                _ => tr("a11y-not-connected"),
            },
        }
    } else {
        let to: Vec<String> = scene
            .wires
            .iter()
            .filter(|w| w.wire.from == ep)
            .map(|w| title_of(&w.wire.to))
            .collect();
        if to.is_empty() {
            tr("a11y-not-connected")
        } else {
            tr_args("a11y-to", &[("targets", to.join(", ").into())])
        }
    };
    let side = if s.input {
        tr_args("a11y-input", &[("type", ty.into())])
    } else {
        tr_args("a11y-output", &[("type", ty.into())])
    };
    tr_args(
        "a11y-socket",
        &[
            ("node", (&n.title).into()),
            ("socket", name.into()),
            ("side", side.into()),
            ("state", state.into()),
        ],
    )
}
