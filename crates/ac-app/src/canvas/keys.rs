//! The canvas from the keyboard (docs/UX.md "Canvas interactions"): Tab
//! and Shift+Tab move a focus through the cards (the selection follows
//! it), Shift+Enter keeps the focused card selected while the focus moves
//! on, Alt+Down and Alt+Up step through a card's sockets, Enter wires two
//! sockets or opens a group, the arrows move what's selected (or the view),
//! Shift+F10 opens the focused thing's menu, and F2 types a socket's
//! value. The app turns key presses into [`KeyCmd`]s; the canvas runs them
//! on its next frame, where it has the scene and the document.

use ac_core::Endpoint;
use egui::{Pos2, Vec2, vec2};

use super::access::Focus;
use super::scene::{Class, NodeView, Scene};
use super::{Canvas, CanvasEvent, CanvasIo, Gesture, Hit, PendingWire, accepts, menu};
use crate::edit;
use crate::i18n::{tr, tr_args};

/// A keyboard command for the canvas.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyCmd {
    /// Tab (+1) and Shift+Tab (−1): the next or previous card.
    Step(i32),
    /// Alt+Down (+1) and Alt+Up (−1): the focused card's sockets.
    SocketStep(i32),
    /// Enter: finish a move, wire, select the focused card, or open it.
    Activate,
    /// Shift+Enter: keep the focused card selected, or let it go.
    Extend,
    /// The arrows: move the selection (canvas units), or the view.
    Nudge(Vec2),
    /// Shift+F10: the focused card's or socket's right-click menu.
    Menu,
    /// F2 on an input socket: type its value.
    EditValue,
}

/// How far an arrow moves a card: a grid step (Shift: five, Ctrl: one
/// canvas unit).
pub const NUDGE: f32 = crate::theme::GRID;

impl Canvas {
    /// Queue a keyboard command for the next frame.
    pub fn key(&mut self, cmd: KeyCmd) {
        self.focus_visible = true;
        self.keys.push(cmd);
    }

    /// Whether the focus is on a socket (F2 types its value there).
    pub fn socket_focused(&self) -> bool {
        self.focus_visible && matches!(self.focus, Some(Focus::Socket { .. }))
    }

    /// The keyboard wire being made, if any (Esc cancels it).
    pub fn keyboard_wire(&self) -> Option<&PendingWire> {
        self.kb_wire.as_ref()
    }

    /// Run the queued commands. Returns whether the document changed.
    pub(super) fn run_keys(
        &mut self,
        ui: &egui::Ui,
        io: &mut CanvasIo,
        scene: &Scene,
        now: f64,
    ) -> bool {
        let rev = io.doc.revision;
        for cmd in std::mem::take(&mut self.keys) {
            match cmd {
                KeyCmd::Step(d) => self.step(io, scene, d),
                KeyCmd::SocketStep(d) => self.socket_step(scene, d),
                KeyCmd::Activate => self.activate(io, scene),
                KeyCmd::Extend => self.extend(io),
                KeyCmd::Nudge(d) => self.nudge(io, d, now),
                KeyCmd::Menu => self.menu_from_keys(ui, scene),
                KeyCmd::EditValue => self.edit_focused_value(io, scene, now),
            }
        }
        io.doc.revision != rev
    }

    /// F2 on an input socket: what clicking its inline value does (type a
    /// number or text, flip a checkbox, fire a trigger).
    fn edit_focused_value(&mut self, io: &mut CanvasIo, scene: &Scene, now: f64) {
        let Some(Focus::Socket {
            node,
            name,
            input: true,
        }) = self.focus.clone()
        else {
            return;
        };
        let hit = match scene
            .node(node)
            .and_then(|n| n.socket(&name, true))
            .and_then(|s| s.spec.as_ref())
            .map(super::widgets::field)
        {
            Some(super::widgets::Field::Fire) => Hit::Fire { node, socket: name },
            _ => Hit::Field { node, socket: name },
        };
        self.click(io, scene, hit, egui::Modifiers::NONE, now);
        // Typing takes the keyboard; it comes back after.
        self.edit_from_keys = self.edit.is_some();
    }

    /// Cards in the order Tab visits them: left to right in columns about
    /// half a card wide, top to bottom within one, which follows the
    /// signal through a patch laid out left to right.
    fn tab_order(scene: &Scene) -> Vec<&NodeView> {
        let mut cards: Vec<&NodeView> = scene.nodes.iter().collect();
        let column = |n: &NodeView| (n.rect.min.x / (crate::theme::NODE_WIDTH * 0.5)).floor();
        cards.sort_by(|a, b| {
            column(a)
                .total_cmp(&column(b))
                .then(a.rect.min.y.total_cmp(&b.rect.min.y))
                .then(a.rect.min.x.total_cmp(&b.rect.min.x))
                .then(a.id.cmp(&b.id))
        });
        cards
    }

    fn step(&mut self, io: &mut CanvasIo, scene: &Scene, d: i32) {
        let order = Self::tab_order(scene);
        if order.is_empty() {
            return;
        }
        let at = self
            .focus
            .as_ref()
            .and_then(|f| order.iter().position(|n| n.id == f.node()));
        let len = order.len() as i32;
        let next = match at {
            Some(i) => (i as i32 + d).rem_euclid(len),
            None if d >= 0 => 0,
            None => len - 1,
        };
        let n = order[next as usize];
        self.focus = Some(Focus::Node(n.id));
        if !self.keep_selection {
            io.doc.selection = [n.id].into();
        }
        self.reveal(n.rect);
    }

    fn socket_step(&mut self, scene: &Scene, d: i32) {
        let Some(node) = self.focus.as_ref().map(Focus::node) else {
            return;
        };
        let Some(n) = scene.node(node) else { return };
        if n.collapsed {
            return;
        }
        // The card, then its inputs, then its outputs.
        let mut stops = vec![Focus::Node(node)];
        for s in n.inputs.iter().chain(n.outputs.iter()) {
            if !s.is_new_slot() {
                stops.push(Focus::Socket {
                    node,
                    name: s.name.clone(),
                    input: s.input,
                });
            }
        }
        let at = self
            .focus
            .as_ref()
            .and_then(|f| stops.iter().position(|x| x == f))
            .unwrap_or(0) as i32;
        let next = (at + d).clamp(0, stops.len() as i32 - 1);
        self.focus = stops.get(next as usize).cloned();
    }

    fn activate(&mut self, io: &mut CanvasIo, scene: &Scene) {
        // Placing nodes moved with `G` and the arrows.
        if matches!(self.gesture, Gesture::Move { grab: true, .. }) {
            self.finish_move(io);
            return;
        }
        if self.kb_wire.is_some() {
            self.finish_keyboard_wire(io, scene);
            return;
        }
        match self.focus.clone() {
            Some(Focus::Socket { node, name, input }) => {
                let Some(s) = scene.node(node).and_then(|n| n.socket(&name, input)) else {
                    return;
                };
                let ep = Endpoint::new(node, name.clone());
                self.kb_wire = Some(PendingWire {
                    from: (!input).then(|| ep.clone()),
                    to: input.then_some(ep),
                    ty: s.ty,
                });
                let title = scene
                    .node(node)
                    .map_or_else(String::new, |n| n.title.clone());
                self.hint = Some(tr_args(
                    "hint-keyboard-wire",
                    &[
                        ("node", title.into()),
                        ("socket", super::paint::pretty(&name).into()),
                    ],
                ));
            }
            Some(Focus::Node(id)) => {
                let only = io.doc.selection.len() == 1 && io.doc.selection.contains(&id);
                let group = scene
                    .node(id)
                    .is_some_and(|n| matches!(n.class, Class::Group(_)));
                if only && group {
                    io.doc.enter_group(id);
                    self.focus = None;
                } else {
                    io.doc.selection = [id].into();
                }
                self.keep_selection = false;
            }
            None => {}
        }
    }

    /// Enter with a keyboard wire waiting: wire it to the focused socket,
    /// or to the focused card's first socket that fits.
    fn finish_keyboard_wire(&mut self, io: &mut CanvasIo, scene: &Scene) {
        let Some(pending) = self.kb_wire.take() else {
            return;
        };
        self.hint = None;
        let Some(focus) = self.focus.clone() else {
            return;
        };
        let Some(n) = scene.node(focus.node()) else {
            return;
        };
        let candidates: Vec<&super::scene::SocketView> = match &focus {
            Focus::Socket { name, input, .. } => n.socket(name, *input).into_iter().collect(),
            Focus::Node(_) => {
                let side = if pending.from.is_some() {
                    &n.inputs
                } else {
                    &n.outputs
                };
                side.iter().filter(|s| !s.wired || !s.input).collect()
            }
        };
        let Some(s) = candidates.into_iter().find(|s| accepts(&pending, n, s)) else {
            io.events
                .push(CanvasEvent::Status(tr("status-keyboard-wire-nothing")));
            return;
        };
        let here = Endpoint::new(n.id, s.name.clone());
        let (from, to) = match (pending.from, pending.to) {
            (Some(from), _) => (from, here),
            (None, Some(to)) => (here, to),
            (None, None) => return,
        };
        let mut error = None;
        io.doc.edit(io.registry, &tr("undo-connect"), |g, ctx| {
            if let Err(e) = edit::connect(g, ctx, from.clone(), to.clone()) {
                error = Some(e);
            }
        });
        let text = match error {
            Some(e) => tr_args("canvas-couldnt-connect", &[("error", e.to_string().into())]),
            None => {
                let name = |e: &Endpoint| {
                    scene
                        .node(e.node)
                        .map_or_else(|| e.node.to_string(), |x| x.title.clone())
                };
                tr_args(
                    "canvas-connected",
                    &[
                        ("from", name(&from).into()),
                        ("output", super::paint::pretty(&from.socket).into()),
                        ("to", name(&to).into()),
                        ("input", super::paint::pretty(&to.socket).into()),
                    ],
                )
            }
        };
        io.events.push(CanvasEvent::Status(text));
    }

    fn extend(&mut self, io: &mut CanvasIo) {
        let Some(id) = self.focus.as_ref().map(Focus::node) else {
            return;
        };
        let sel = &mut io.doc.selection;
        if self.keep_selection && sel.contains(&id) {
            sel.remove(&id);
        } else {
            sel.insert(id);
        }
        self.keep_selection = true;
    }

    fn nudge(&mut self, io: &mut CanvasIo, d: Vec2, now: f64) {
        if let Gesture::Move {
            grab: true, origin, ..
        } = &mut self.gesture
        {
            // The grab follows the pointer from `origin`; moving the origin
            // back moves the nodes on, whatever the pointer does.
            *origin -= d;
            self.grab_keys = true;
            return;
        }
        let ids: Vec<ac_core::NodeId> = io
            .doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        if ids.is_empty() {
            // Nothing to move: the arrows look around instead.
            self.target = None;
            self.view.pan(-d * self.view.zoom * 3.0);
            return;
        }
        let moves: Vec<(ac_core::NodeId, [f32; 2])> = ids
            .iter()
            .filter_map(|id| io.doc.graph().node(*id))
            .map(|n| (n.id, [n.ui.pos[0] + d.x, n.ui.pos[1] + d.y]))
            .collect();
        // A run of presses is one undo step.
        io.doc
            .checkpoint_coalesced(&tr("undo-move"), "keyboard-nudge", now);
        io.doc.move_nodes(&moves);
        if let Some(r) = self.selection_rect(io, &ids) {
            self.reveal(r);
        }
    }

    fn selection_rect(&self, io: &CanvasIo, ids: &[ac_core::NodeId]) -> Option<egui::Rect> {
        let scene = self.build(io.doc, io.registry);
        ids.iter()
            .filter_map(|id| scene.node(*id).map(|n| n.rect))
            .reduce(|a, b| a.union(b))
    }

    /// Shift+F10: the menu a right-click there would open, with the
    /// keyboard in its first item.
    fn menu_from_keys(&mut self, ui: &egui::Ui, scene: &Scene) {
        let t = self.transform();
        let (target, at) = match self.focus.clone() {
            Some(Focus::Socket { node, name, input }) => {
                let at = scene
                    .node(node)
                    .and_then(|n| n.socket(&name, input))
                    .map_or(self.screen.center(), |s| t.to_screen(s.pos));
                (menu::Target::Socket { node, name, input }, at)
            }
            Some(Focus::Node(id)) => {
                let at = scene
                    .node(id)
                    .map_or(self.screen.center(), |n| t.to_screen(n.rect.center()));
                (menu::Target::Node(id), at)
            }
            None => {
                let c = self.screen.center();
                (menu::Target::Canvas(t.to_canvas(c)), c)
            }
        };
        self.menu = Some((target, at + vec2(8.0, 8.0)));
        self.menu_age = 1;
        self.menu_from_keys = true;
        // Nothing has focus, so egui gives it to the first widget that
        // wants it from here on: the menu's first item.
        if let Some(f) = &self.focus {
            let id = f.widget_id();
            ui.memory_mut(|m| m.surrender_focus(id));
        }
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::Next));
    }

    /// Bring a card into view if it isn't (keyboard focus moved to it).
    fn reveal(&mut self, rect: egui::Rect) {
        let t = self.transform();
        let screen = t.rect_to_screen(rect);
        if self.screen.shrink(8.0).contains_rect(screen) {
            return;
        }
        self.target = Some(super::view::View {
            center: rect.center(),
            zoom: self.view.zoom.max(0.6),
        });
    }

    /// Where a keyboard wire is drawn to: the focused socket, or the
    /// middle of the focused card.
    pub(super) fn keyboard_wire_end(&self, scene: &Scene) -> Option<Pos2> {
        self.kb_wire.as_ref()?;
        let t = self.transform();
        match self.focus.as_ref()? {
            Focus::Socket { node, name, input } => scene
                .node(*node)
                .and_then(|n| n.socket(name, *input))
                .map(|s| t.to_screen(s.pos)),
            Focus::Node(id) => scene.node(*id).map(|n| t.to_screen(n.rect.center())),
        }
    }
}
