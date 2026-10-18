//! Pointer gestures on the canvas: what a press, drag, click, or
//! double-click does depending on what it lands on.

use std::collections::BTreeSet;

use ac_core::{Endpoint, NodeId, SignalType, Value, Wire};
use egui::{PointerButton, Pos2, Rect, Response, pos2, vec2};

use super::scene::{Body, Class, NodeView, Scene};
use super::{
    Canvas, CanvasEvent, CanvasIo, EditTarget, Gesture, Hit, InlineEdit, PendingRelease,
    PendingWire, accepts, field_rect, piano, widgets,
};
use crate::edit::{self, GROUP_IN, GROUP_OUT};
use crate::theme;

/// A held control (a Button, a Keyboard key, an XY Pad) stays down in the
/// engine for at least this long, so a quick tap is never lost between
/// blocks.
pub(super) const MIN_HOLD: f64 = 0.04;

/// Pointer travel (points) that sweeps an inline value's whole soft range.
const DRAG_TRAVEL: f32 = 320.0;

/// Vertical pointer travel (points) that turns a knob through its range.
const KNOB_TRAVEL: f32 = 200.0;

impl Canvas {
    /// Handle the pointer for this frame. Returns whether the document changed.
    pub(super) fn handle_pointer(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        io: &mut CanvasIo,
        scene: &Scene,
        now: f64,
    ) -> bool {
        let rev = io.doc.revision;
        let (primary_pressed, primary_released, secondary_pressed, modifiers, space) =
            ui.input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_released(),
                    i.pointer.secondary_pressed(),
                    i.modifiers,
                    i.key_down(egui::Key::Space),
                )
            });
        let typing = ui.ctx().egui_wants_keyboard_input();
        let pointer = self.pointer.unwrap_or(self.screen.center());
        let hover = if response.hovered() || !self.is_idle() {
            self.hit(scene, io.doc, pointer)
        } else {
            Hit::None
        };

        // `G`: nodes follow the pointer until a click.
        if let Gesture::Move { grab: true, .. } = self.gesture {
            self.update_move(io, scene, pointer, modifiers.ctrl);
            if primary_pressed {
                self.finish_move(io);
            } else if secondary_pressed {
                self.cancel(io.doc);
            }
            return io.doc.revision != rev;
        }

        if primary_pressed && response.hovered() {
            self.press = Some((hover.clone(), pointer));
            self.menu = None;
            if let Hit::Body { node } = hover {
                self.press_body(io, scene, node, pointer, now);
            }
        }
        if primary_released {
            self.release_hold(io, now);
        } else {
            self.follow_hold(io, scene, now);
        }

        if response.drag_started_by(PointerButton::Primary) && self.is_idle() {
            let (hit, origin) = self.press.clone().unwrap_or((hover.clone(), pointer));
            self.begin_drag(io, scene, hit, origin, modifiers, space && !typing, now);
        }
        if response.drag_started_by(PointerButton::Middle) && self.is_idle() {
            self.gesture = Gesture::Pan;
        }
        if response.drag_started_by(PointerButton::Secondary) && self.is_idle() {
            if modifiers.shift {
                self.gesture = Gesture::Reroute {
                    points: vec![pointer],
                };
                self.hint = Some("Drag across wires to add reroutes".into());
            } else if modifiers.ctrl || modifiers.command {
                self.gesture = Gesture::Cut {
                    points: vec![pointer],
                };
                self.hint = Some("Drag across wires to cut them".into());
            }
        }

        if response.dragged() {
            self.update_drag(ui, response, io, scene, pointer, modifiers, now);
        }
        if response.drag_stopped() {
            self.finish_drag(io, scene, pointer, modifiers, now);
        }

        if response.clicked() && !matches!(self.gesture, Gesture::HoldButton { .. }) {
            let hit = self.press.clone().map_or(hover.clone(), |p| p.0);
            self.click(io, scene, hit, modifiers, now);
        }
        if response.double_clicked() {
            let hit = self.press.clone().map_or(hover.clone(), |p| p.0);
            self.double_click(io, scene, hit, pointer, now);
        }
        if response.secondary_clicked() {
            if let Some(n) = hover.node()
                && !io.doc.selection.contains(&n)
            {
                io.doc.selection = [n].into();
            }
            self.menu = Some((
                super::menu::Target::from_hit(&hover, self.pointer_canvas()),
                pointer,
            ));
        }
        io.doc.revision != rev
    }

    /// The primary button went down on a widget body: controls that act
    /// on press (Button, Keyboard, XY Pad) start holding.
    fn press_body(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        node: NodeId,
        pointer: Pos2,
        now: f64,
    ) {
        let Some(n) = scene.node(node) else { return };
        let Some((body, rect)) = n.body else { return };
        let r = self.transform().rect_to_screen(rect);
        match body {
            Body::Button => {
                self.hold_down(io, node, "pressed");
                self.gesture = Gesture::HoldButton { node, since: now };
            }
            Body::Keyboard => {
                if let Some(key) = key_under(io, node, r, pointer) {
                    self.hold_down(io, node, &piano::key_socket(key));
                    self.gesture = Gesture::HoldKey {
                        node,
                        key,
                        since: now,
                    };
                }
            }
            Body::XyPad => {
                let touch = n.socket("touched", true).is_some_and(|s| !s.wired);
                let values = pad_values(n, r, pointer);
                if !touch && values.is_empty() {
                    // Every input is wired: the pad only shows them.
                    return;
                }
                Self::send_params(io, node, &values, now);
                if touch {
                    self.hold_down(io, node, "touched");
                }
                self.gesture = Gesture::HoldPad {
                    node,
                    since: now,
                    touch,
                };
            }
            _ => {}
        }
    }

    /// While a Keyboard key or an XY Pad is held, follow the pointer:
    /// sliding onto another key plays it instead, and the pad's point moves.
    fn follow_hold(&mut self, io: &mut CanvasIo, scene: &Scene, now: f64) {
        let Some(p) = self.pointer else { return };
        let (node, held) = match self.gesture {
            Gesture::HoldKey { node, .. } | Gesture::HoldPad { node, .. } => {
                (node, self.gesture.clone())
            }
            _ => return,
        };
        let Some(n) = scene.node(node) else { return };
        let Some((_, rect)) = n.body else { return };
        let r = self.transform().rect_to_screen(rect);
        match held {
            Gesture::HoldKey { key, since, .. } => {
                let Some(next) = key_under(io, node, r, p) else {
                    return;
                };
                if next != key {
                    self.let_go(io, node, piano::key_socket(key), since, now);
                    self.hold_down(io, node, &piano::key_socket(next));
                    self.gesture = Gesture::HoldKey {
                        node,
                        key: next,
                        since: now,
                    };
                }
            }
            Gesture::HoldPad { .. } => {
                let values = pad_values(n, r, p);
                Self::send_params(io, node, &values, now);
            }
            _ => {}
        }
    }

    /// The primary button came up: a held control lets go.
    fn release_hold(&mut self, io: &mut CanvasIo, now: f64) {
        let pad = match self.gesture {
            Gesture::HoldPad { .. } => true,
            Gesture::HoldButton { .. } | Gesture::HoldKey { .. } => false,
            _ => return,
        };
        if let Some((node, socket, since)) = self.gesture.held() {
            self.let_go(io, node, socket, since, now);
        }
        if pad {
            // The pad's moves were one undo step.
            io.doc.end_coalescing();
        }
        self.gesture = Gesture::Idle;
    }

    /// Hold an input down in the engine, dropping a release of it that was
    /// still waiting (so it isn't let go while held again).
    fn hold_down(&mut self, io: &mut CanvasIo, node: NodeId, socket: &str) {
        self.releases
            .retain(|r| !(r.node == node && r.socket == socket));
        Self::hold(io, node, socket, true);
    }

    /// Let go of an input held since `since`, but not before it has been
    /// down for [`MIN_HOLD`].
    fn let_go(&mut self, io: &mut CanvasIo, node: NodeId, socket: String, since: f64, now: f64) {
        if now - since >= MIN_HOLD {
            Self::hold(io, node, &socket, false);
        } else {
            self.releases.push(PendingRelease {
                node,
                socket,
                at: since + MIN_HOLD,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_drag(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        hit: Hit,
        origin: Pos2,
        modifiers: egui::Modifiers,
        space: bool,
        now: f64,
    ) {
        let t = self.transform();
        if space {
            self.gesture = Gesture::Pan;
            self.space_panned = true;
            return;
        }
        match hit {
            Hit::Socket { node, name, input } => {
                let graph = io.doc.display_graph();
                let ctx_types = |ep: &Endpoint, out: bool| -> SignalType {
                    let n = scene.node(ep.node);
                    let s = n.and_then(|n| n.socket(&ep.socket, !out));
                    s.map_or(SignalType::Any, |s| s.ty)
                };
                let ep = Endpoint::new(node, name);
                if !input {
                    self.gesture = Gesture::Wire {
                        pending: PendingWire {
                            ty: ctx_types(&ep, true),
                            from: Some(ep),
                            to: None,
                        },
                        detached: None,
                        snap: None,
                    };
                } else if let Some(w) = graph.wire_into(&ep).cloned() {
                    // Pick up the wire's end; it reconnects or drops on release.
                    let ty = scene
                        .wires
                        .iter()
                        .find(|x| x.wire == w)
                        .map_or(SignalType::Any, |x| x.ty);
                    self.gesture = Gesture::Wire {
                        pending: PendingWire {
                            from: Some(w.from.clone()),
                            to: None,
                            ty,
                        },
                        detached: Some(w),
                        snap: None,
                    };
                } else {
                    self.gesture = Gesture::Wire {
                        pending: PendingWire {
                            ty: ctx_types(&ep, false),
                            from: None,
                            to: Some(ep),
                        },
                        detached: None,
                        snap: None,
                    };
                }
                self.hint =
                    Some("Drop on a socket to connect · on empty canvas to add a node".into());
            }
            Hit::Field { node, socket } => {
                let spec = scene
                    .node(node)
                    .and_then(|n| n.socket(&socket, true))
                    .and_then(|s| s.spec.clone().map(|spec| (spec, field_rect(s))));
                match spec {
                    Some((spec, _)) if widgets::field(&spec) == widgets::Field::Number => {
                        // From the value shown (a bound input's live value).
                        let start = io
                            .shown_node(node)
                            .map_or(0.0, |n| widgets::number(&n, &spec));
                        self.gesture = Gesture::DragValue {
                            node,
                            socket,
                            spec,
                            start,
                            origin,
                            // A fixed travel, not the field's width: narrow
                            // fields would be too touchy.
                            width: DRAG_TRAVEL,
                            vertical: false,
                        };
                        self.hint = Some("Ctrl: fine · Shift: coarse".into());
                    }
                    _ => self.begin_move(io, node, modifiers, origin),
                }
            }
            Hit::Body { node } => {
                let body = scene.node(node).and_then(|n| n.body);
                // The unwired input a body drags, with its spec.
                let free = |socket: &str| {
                    scene
                        .node(node)
                        .and_then(|n| n.socket(socket, true))
                        .filter(|s| !s.wired)
                        .and_then(|s| s.spec.clone())
                };
                match body {
                    Some((Body::Slider, rect)) => {
                        let n = io.doc.graph().node(node);
                        let spec = scene
                            .node(node)
                            .and_then(|n| n.socket("value", true))
                            .and_then(|s| s.spec.clone());
                        if let (Some(n), Some(spec)) = (n, spec) {
                            // Jump to where the slider was grabbed, then drag.
                            let r = t.rect_to_screen(rect);
                            let tpos = ((origin.x - r.min.x) / r.width().max(1.0)).clamp(0.0, 1.0);
                            let lo = spec.range.map_or(0.0, |r| r.0);
                            let start = widgets::dragged(
                                lo,
                                tpos * r.width(),
                                r.width(),
                                &spec,
                                false,
                                false,
                            );
                            let _ = n;
                            Self::send_param(io, node, "value", Value::Float(start), now);
                            self.gesture = Gesture::DragValue {
                                node,
                                socket: "value".into(),
                                spec,
                                start,
                                origin,
                                width: r.width(),
                                vertical: false,
                            };
                        }
                    }
                    // Knobs turn with an up-and-down drag; number fields
                    // drag sideways like inline fields.
                    Some((b @ (Body::Knob | Body::Number), _)) => {
                        let spec = free("value");
                        let inst = io.shown_node(node);
                        match (spec, inst) {
                            (Some(spec), Some(inst)) => {
                                let knob = b == Body::Knob;
                                self.gesture = Gesture::DragValue {
                                    node,
                                    socket: "value".into(),
                                    start: widgets::number(&inst, &spec),
                                    spec,
                                    origin,
                                    width: if knob { KNOB_TRAVEL } else { DRAG_TRAVEL },
                                    vertical: knob,
                                };
                                self.hint = Some("Ctrl: fine · Shift: coarse".into());
                            }
                            _ => self.begin_move(io, node, modifiers, origin),
                        }
                    }
                    Some((Body::Button, _)) => {}
                    _ => self.begin_move(io, node, modifiers, origin),
                }
            }
            Hit::Header { node } | Hit::Node { node } | Hit::Collapse { node } => {
                self.begin_move(io, node, modifiers, origin);
            }
            Hit::Fire { node, .. } => self.begin_move(io, node, modifiers, origin),
            Hit::Resize { node } => {
                let size = scene
                    .node(node)
                    .map_or(vec2(240.0, 120.0), |n| n.rect.size());
                io.doc.checkpoint("Resize");
                self.gesture = Gesture::ResizeNode {
                    node,
                    start: size,
                    origin: t.to_canvas(origin),
                };
            }
            Hit::FrameHeader(index) => {
                let Some(f) = io.doc.graph().frames.get(index).cloned() else {
                    return;
                };
                let r = super::frame_rect(&f);
                let ids: Vec<NodeId> = scene
                    .nodes
                    .iter()
                    .filter(|n| !edit::is_virtual(n.id) && r.contains(n.rect.center()))
                    .map(|n| n.id)
                    .collect();
                let starts = ids
                    .iter()
                    .filter_map(|id| io.doc.graph().node(*id))
                    .map(|n| pos2(n.ui.pos[0], n.ui.pos[1]))
                    .collect();
                io.doc.checkpoint("Move frame");
                self.gesture = Gesture::MoveFrame {
                    index,
                    start: f.rect,
                    ids,
                    starts,
                    origin: t.to_canvas(origin),
                };
            }
            Hit::FrameResize(index) => {
                let Some(start) = io.doc.graph().frames.get(index).map(|f| f.rect) else {
                    return;
                };
                io.doc.checkpoint("Resize frame");
                self.gesture = Gesture::ResizeFrame {
                    index,
                    start,
                    origin: t.to_canvas(origin),
                };
            }
            Hit::Wire(_) | Hit::None => {
                if modifiers.ctrl || modifiers.command {
                    self.gesture = Gesture::Cut {
                        points: vec![origin],
                    };
                    self.hint = Some("Drag across wires to cut them".into());
                } else {
                    self.gesture = Gesture::BoxSelect {
                        origin: t.to_canvas(origin),
                        additive: modifiers.shift,
                    };
                }
            }
        }
    }

    fn begin_move(
        &mut self,
        io: &mut CanvasIo,
        node: NodeId,
        modifiers: egui::Modifiers,
        origin: Pos2,
    ) {
        if !io.doc.selection.contains(&node) {
            if !modifiers.shift {
                io.doc.selection.clear();
            }
            io.doc.selection.insert(node);
        }
        let ids: Vec<NodeId> = io.doc.selection.iter().copied().collect();
        let iface = self.iface_cards(io);
        let starts = ids
            .iter()
            .map(|id| {
                if *id == GROUP_IN {
                    iface[0]
                } else if *id == GROUP_OUT {
                    iface[1]
                } else {
                    io.doc
                        .graph()
                        .node(*id)
                        .map_or(Pos2::ZERO, |n| pos2(n.ui.pos[0], n.ui.pos[1]))
                }
            })
            .collect();
        io.doc.checkpoint("Move");
        self.gesture = Gesture::Move {
            ids,
            starts,
            origin: self.transform().to_canvas(origin),
            grab: false,
            insert: None,
        };
    }

    fn iface_cards(&self, io: &CanvasIo) -> [Pos2; 2] {
        let moved = io
            .doc
            .current_def()
            .and_then(|d| self.iface_pos.get(d).copied());
        super::scene::interface_positions(io.doc.graph(), moved)
    }

    #[allow(clippy::too_many_arguments)]
    fn update_drag(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        io: &mut CanvasIo,
        scene: &Scene,
        pointer: Pos2,
        modifiers: egui::Modifiers,
        now: f64,
    ) {
        let t = self.transform();
        match &mut self.gesture {
            Gesture::Pan => {
                let d = response.drag_delta();
                self.target = None;
                self.view.pan(d);
            }
            Gesture::Move { .. } => self.update_move(io, scene, pointer, modifiers.ctrl),
            Gesture::Wire { pending, snap, .. } => {
                // Snap to the nearest compatible socket within reach.
                let mut best: Option<(f32, NodeId, String)> = None;
                for n in &scene.nodes {
                    let list = if pending.from.is_some() {
                        &n.inputs
                    } else {
                        &n.outputs
                    };
                    for s in list {
                        if !accepts(pending, n, s) {
                            continue;
                        }
                        let d = (t.to_screen(s.pos) - pointer).length();
                        if d <= theme::SOCKET_SNAP && best.as_ref().is_none_or(|b| d < b.0) {
                            best = Some((d, n.id, s.name.clone()));
                        }
                    }
                }
                *snap = match (best, snap.take()) {
                    (Some((_, n, s)), Some((pn, ps, since))) if pn == n && ps == s => {
                        Some((n, s, since))
                    }
                    (Some((_, n, s)), _) => Some((n, s, now)),
                    (None, _) => None,
                };
            }
            Gesture::DragValue {
                node,
                socket,
                spec,
                start,
                origin,
                width,
                vertical,
            } => {
                let fine = modifiers.ctrl || modifiers.command;
                // Up turns a knob up (screen y grows downwards).
                let travel = if *vertical {
                    origin.y - pointer.y
                } else {
                    pointer.x - origin.x
                };
                let v = widgets::dragged(*start, travel, *width, spec, fine, modifiers.shift);
                let value = if spec.ty == SignalType::Int {
                    Value::Int(v as i64)
                } else {
                    Value::Float(v)
                };
                let (node, socket) = (*node, socket.clone());
                Self::send_param(io, node, &socket, value, now);
            }
            Gesture::Cut { points } | Gesture::Reroute { points } => {
                if points.last().is_none_or(|l| (*l - pointer).length() > 3.0) {
                    points.push(pointer);
                }
            }
            Gesture::MoveFrame {
                index,
                start,
                ids,
                starts,
                origin,
            } => {
                let mut d = t.to_canvas(pointer) - *origin;
                if modifiers.ctrl {
                    d = snap_vec(d);
                }
                let (index, start, ids, starts) = (*index, *start, ids.clone(), starts.clone());
                let g = io.doc.graph_mut_raw();
                if let Some(f) = g.frames.get_mut(index) {
                    f.rect = [start[0] + d.x, start[1] + d.y, start[2], start[3]];
                }
                let moves: Vec<(NodeId, [f32; 2])> = ids
                    .iter()
                    .zip(&starts)
                    .map(|(id, p)| (*id, [p.x + d.x, p.y + d.y]))
                    .collect();
                io.doc.move_nodes(&moves);
            }
            Gesture::ResizeFrame {
                index,
                start,
                origin,
            } => {
                let d = t.to_canvas(pointer) - *origin;
                let (index, start) = (*index, *start);
                if let Some(f) = io.doc.graph_mut_raw().frames.get_mut(index) {
                    f.rect[2] = (start[2] + d.x).max(80.0);
                    f.rect[3] = (start[3] + d.y).max(60.0);
                }
                io.doc.changed(false);
            }
            Gesture::ResizeNode {
                node,
                start,
                origin,
            } => {
                let d = t.to_canvas(pointer) - *origin;
                let size = [(start.x + d.x).max(100.0), (start.y + d.y).max(50.0)];
                let node = *node;
                if let Some(n) = io.doc.graph_mut_raw().node_mut(node) {
                    n.ui.size = Some(size);
                }
                io.doc.changed(false);
            }
            // Held controls follow the pointer every frame (`follow_hold`).
            Gesture::BoxSelect { .. }
            | Gesture::Idle
            | Gesture::HoldButton { .. }
            | Gesture::HoldKey { .. }
            | Gesture::HoldPad { .. } => {}
        }
        let _ = ui;
    }

    fn update_move(&mut self, io: &mut CanvasIo, scene: &Scene, pointer: Pos2, snap: bool) {
        let t = self.transform();
        let Gesture::Move {
            ids,
            starts,
            origin,
            insert,
            ..
        } = &mut self.gesture
        else {
            return;
        };
        let d = t.to_canvas(pointer) - *origin;
        let mut moves = Vec::new();
        let mut iface = None;
        for (id, start) in ids.iter().zip(starts.iter()) {
            let mut p = *start + d;
            if snap {
                p = pos2(snap_f(p.x), snap_f(p.y));
            }
            if edit::is_virtual(*id) {
                let cur = iface.get_or_insert_with(|| {
                    let moved = io
                        .doc
                        .current_def()
                        .and_then(|d| self.iface_pos.get(d).copied());
                    super::scene::interface_positions(io.doc.graph(), moved)
                });
                cur[if *id == GROUP_IN { 0 } else { 1 }] = p;
            } else {
                moves.push((*id, [p.x, p.y]));
            }
        }
        if let (Some(cur), Some(def)) = (iface, io.doc.current_def().cloned()) {
            self.iface_pos.insert(def, cur);
        }
        io.doc.move_nodes(&moves);

        // Dropping one unwired node onto a wire splices it in.
        *insert = None;
        if ids.len() == 1 && !edit::is_virtual(ids[0]) {
            let id = ids[0];
            let graph = io.doc.display_graph();
            let unwired = !graph
                .wires
                .iter()
                .any(|w| w.from.node == id || w.to.node == id);
            if unwired {
                let mut best: Option<(f32, Wire)> = None;
                for w in &scene.wires {
                    let c = super::wires::bezier(
                        t.to_screen(w.from),
                        t.to_screen(w.to),
                        self.view.zoom,
                    );
                    let dist = super::wires::distance(&super::wires::flatten(&c), pointer);
                    if dist < 14.0 && best.as_ref().is_none_or(|b| dist < b.0) {
                        best = Some((dist, w.wire.clone()));
                    }
                }
                if let Some((_, w)) = best
                    && can_insert(io, id, &w)
                {
                    *insert = Some(w);
                }
            }
        }
    }

    fn finish_move(&mut self, io: &mut CanvasIo) {
        let Gesture::Move { ids, insert, .. } = std::mem::replace(&mut self.gesture, Gesture::Idle)
        else {
            return;
        };
        self.hint = None;
        if let Some(w) = insert {
            let id = ids[0];
            io.doc.edit(io.registry, "Insert on wire", |g, ctx| {
                edit::insert_on_wire(g, ctx, id, &w);
            });
        } else {
            io.doc.drop_noop_step();
        }
    }

    fn finish_drag(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        pointer: Pos2,
        modifiers: egui::Modifiers,
        now: f64,
    ) {
        let t = self.transform();
        let gesture = std::mem::replace(&mut self.gesture, Gesture::Idle);
        self.hint = None;
        match gesture {
            Gesture::BoxSelect { origin, additive } => {
                let r = Rect::from_two_pos(origin, t.to_canvas(pointer));
                if !additive {
                    io.doc.selection.clear();
                }
                for n in &scene.nodes {
                    if r.intersects(n.rect) {
                        io.doc.selection.insert(n.id);
                    }
                }
            }
            g @ Gesture::Move { .. } => {
                self.gesture = g;
                self.finish_move(io);
            }
            Gesture::Wire {
                pending,
                detached,
                snap,
            } => self.finish_wire(io, scene, pointer, pending, detached, snap),
            Gesture::DragValue { .. } => io.doc.end_coalescing(),
            Gesture::Cut { points } => {
                let cut: Vec<Wire> = scene
                    .wires
                    .iter()
                    .filter(|w| {
                        let c = super::wires::bezier(
                            t.to_screen(w.from),
                            t.to_screen(w.to),
                            self.view.zoom,
                        );
                        super::wires::crossing(&super::wires::flatten(&c), &points).is_some()
                    })
                    .map(|w| w.wire.clone())
                    .collect();
                if !cut.is_empty() {
                    io.doc.edit(io.registry, "Cut wires", |g, _| {
                        g.wires.retain(|w| !cut.contains(w));
                    });
                }
            }
            Gesture::Reroute { points } => {
                let crossings: Vec<edit::Crossing> = scene
                    .wires
                    .iter()
                    .filter_map(|w| {
                        let c = super::wires::bezier(
                            t.to_screen(w.from),
                            t.to_screen(w.to),
                            self.view.zoom,
                        );
                        super::wires::crossing(&super::wires::flatten(&c), &points).map(|p| {
                            let cp = t.to_canvas(p);
                            (w.wire.clone(), [cp.x, cp.y])
                        })
                    })
                    .collect();
                if !crossings.is_empty() {
                    let new = io.doc.edit(io.registry, "Add reroute", |g, ctx| {
                        edit::add_reroutes(g, ctx, &crossings)
                    });
                    self.mark_born(&new, now);
                }
            }
            Gesture::MoveFrame { .. }
            | Gesture::ResizeFrame { .. }
            | Gesture::ResizeNode { .. } => {
                io.doc.drop_noop_step();
            }
            // Held controls let go in `release_hold`, before this runs.
            Gesture::Pan
            | Gesture::Idle
            | Gesture::HoldButton { .. }
            | Gesture::HoldKey { .. }
            | Gesture::HoldPad { .. } => {}
        }
        let _ = modifiers;
    }

    fn finish_wire(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        pointer: Pos2,
        pending: PendingWire,
        detached: Option<Wire>,
        snap: Option<(NodeId, String, f64)>,
    ) {
        // Where did it land: a snapped socket, or a node (pick its best socket).
        let target = snap.map(|(n, s, _)| (n, s)).or_else(|| {
            let hit = self.hit(scene, io.doc, pointer);
            let n = scene.node(hit.node()?)?;
            let list = if pending.from.is_some() {
                &n.inputs
            } else {
                &n.outputs
            };
            let p = self.transform().to_canvas(pointer);
            list.iter()
                .filter(|s| accepts(&pending, n, s) && !s.is_new_slot())
                .min_by(|a, b| (a.pos.y - p.y).abs().total_cmp(&(b.pos.y - p.y).abs()))
                .map(|s| (n.id, s.name.clone()))
        });
        match target {
            Some((node, socket)) => {
                let (from, to) = match (&pending.from, &pending.to) {
                    (Some(f), _) => (f.clone(), Endpoint::new(node, socket)),
                    (None, Some(t)) => (Endpoint::new(node, socket), t.clone()),
                    _ => return,
                };
                let mut error = None;
                io.doc.edit(io.registry, "Connect", |g, ctx| {
                    let at = detached
                        .as_ref()
                        .and_then(|d| g.wires.iter().position(|w| w == d));
                    if let Some(i) = at {
                        g.wires.remove(i);
                    }
                    if let Err(e) = edit::connect(g, ctx, from, to) {
                        if let (Some(i), Some(d)) = (at, detached.clone()) {
                            g.wires.insert(i.min(g.wires.len()), d);
                        }
                        error = Some(e.to_string());
                    }
                });
                if let Some(e) = error {
                    io.events.push(CanvasEvent::Status(e));
                }
            }
            None => {
                if let Some(d) = detached {
                    io.doc.edit(io.registry, "Disconnect", |g, _| {
                        g.wires.retain(|w| *w != d);
                    });
                } else if !self.screen.contains(pointer) {
                    // Dropped outside the canvas: nothing.
                } else {
                    let is_iface = pending.from.as_ref().is_some_and(|f| f.node == GROUP_IN)
                        || pending.to.as_ref().is_some_and(|t| t.node == GROUP_OUT);
                    if !is_iface {
                        io.events.push(CanvasEvent::OpenPalette {
                            screen: pointer,
                            canvas: self.transform().to_canvas(pointer),
                            pending: Some(pending),
                        });
                    }
                }
            }
        }
    }

    fn click(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        hit: Hit,
        modifiers: egui::Modifiers,
        now: f64,
    ) {
        let t = self.transform();
        match hit {
            Hit::Field { node, socket } => {
                let Some(s) = scene.node(node).and_then(|n| n.socket(&socket, true)) else {
                    return;
                };
                let Some(spec) = s.spec.clone() else { return };
                let Some(shown) = io.shown_node(node) else {
                    return;
                };
                let n = &shown;
                match widgets::field(&spec) {
                    widgets::Field::Number => {
                        let v = widgets::number(n, &spec);
                        self.edit = Some(InlineEdit {
                            target: EditTarget::Param {
                                node,
                                socket,
                                spec: spec.clone(),
                            },
                            text: plain_number(v, &spec),
                            rect: field_rect(s),
                            multiline: false,
                            started: false,
                        });
                    }
                    widgets::Field::Choice => {
                        let r = t.rect_to_screen(field_rect(s));
                        self.choice = Some((node, socket, r.left_bottom()));
                    }
                    widgets::Field::Check => {
                        let on = widgets::value(n, &spec).as_bool().unwrap_or(false);
                        Self::send_param(io, node, &socket, Value::Bool(!on), now);
                        io.doc.end_coalescing();
                    }
                    _ => {}
                }
            }
            Hit::Fire { node, socket } => Self::fire(io, node, &socket),
            Hit::Body { node } => {
                let Some(n) = scene.node(node) else { return };
                let free = |socket: &str| {
                    n.socket(socket, true)
                        .filter(|s| !s.wired)
                        .and_then(|s| s.spec.clone())
                };
                match n.body {
                    Some((Body::Toggle, _)) => Self::fire(io, node, "flip"),
                    Some((Body::Clip, _)) => io.events.push(CanvasEvent::ChooseClipFile(node)),
                    // Pressing played them already.
                    Some((Body::Button | Body::Keyboard | Body::XyPad, _)) => {}
                    Some((Body::Dropdown, rect)) if free("value").is_some() => {
                        let r = t.rect_to_screen(rect);
                        self.choice = Some((node, "value".into(), r.left_bottom()));
                    }
                    // The number itself is the field: click to type.
                    Some((Body::Number, rect)) => {
                        match (free("value"), io.doc.graph().node(node)) {
                            (Some(spec), Some(inst)) => {
                                self.edit = Some(InlineEdit {
                                    target: EditTarget::Param {
                                        node,
                                        socket: "value".into(),
                                        spec: spec.clone(),
                                    },
                                    text: plain_number(widgets::number(inst, &spec), &spec),
                                    rect: rect.shrink2(vec2(6.0, 6.0)),
                                    multiline: false,
                                    started: false,
                                });
                            }
                            _ => self.select(io, node, modifiers),
                        }
                    }
                    _ => self.select(io, node, modifiers),
                }
            }
            Hit::Collapse { node } => {
                io.doc.edit(io.registry, "Collapse", |g, _| {
                    if let Some(n) = g.node_mut(node) {
                        n.ui.collapsed = !n.ui.collapsed;
                    }
                });
            }
            Hit::Socket { node, .. }
            | Hit::Header { node }
            | Hit::Node { node }
            | Hit::Resize { node } => self.select(io, node, modifiers),
            Hit::FrameHeader(i) => {
                if let Some(f) = io.doc.graph().frames.get(i) {
                    let r = super::frame_rect(f);
                    if !modifiers.shift {
                        io.doc.selection.clear();
                    }
                    for n in &scene.nodes {
                        if r.contains(n.rect.center()) && !edit::is_virtual(n.id) {
                            io.doc.selection.insert(n.id);
                        }
                    }
                }
            }
            Hit::Wire(_) | Hit::None | Hit::FrameResize(_) => {
                if !modifiers.shift {
                    io.doc.selection.clear();
                }
            }
        }
    }

    fn select(&mut self, io: &mut CanvasIo, node: NodeId, modifiers: egui::Modifiers) {
        if modifiers.shift {
            if !io.doc.selection.remove(&node) {
                io.doc.selection.insert(node);
            }
        } else {
            io.doc.selection = BTreeSet::from([node]);
        }
    }

    fn double_click(
        &mut self,
        io: &mut CanvasIo,
        scene: &Scene,
        hit: Hit,
        pointer: Pos2,
        now: f64,
    ) {
        let t = self.transform();
        match hit {
            Hit::None | Hit::Wire(_) => io.events.push(CanvasEvent::OpenPalette {
                screen: pointer,
                canvas: t.to_canvas(pointer),
                pending: None,
            }),
            Hit::Header { node } | Hit::Node { node } => {
                let Some(n) = scene.node(node) else { return };
                match &n.class {
                    Class::Group(_) => {
                        if io.doc.enter_group(node) {
                            self.edit = None;
                        }
                    }
                    Class::Interface { .. } => {}
                    _ => self.rename_node(io.doc, io.registry, node),
                }
            }
            Hit::Body { node } => {
                let Some(n) = scene.node(node) else { return };
                // Double-click a knob to put it back to its default.
                if let Some((Body::Knob, _)) = n.body
                    && let Some(s) = n.socket("value", true).filter(|s| !s.wired)
                    && let Some(spec) = &s.spec
                {
                    Self::send_param(io, node, "value", spec.default.clone(), now);
                    io.doc.end_coalescing();
                    return;
                }
                if let Some((Body::Slider, rect)) = n.body
                    && let Some(spec) = n.socket("value", true).and_then(|s| s.spec.clone())
                    && let Some(inst) = io.doc.graph().node(node)
                {
                    let v = widgets::number(inst, &spec);
                    self.edit = Some(InlineEdit {
                        target: EditTarget::Param {
                            node,
                            socket: "value".into(),
                            spec: spec.clone(),
                        },
                        text: plain_number(v, &spec),
                        rect: Rect::from_min_size(rect.min, vec2(rect.width(), 18.0)),
                        multiline: false,
                        started: false,
                    });
                }
            }
            Hit::FrameHeader(i) => {
                if let Some(f) = io.doc.graph().frames.get(i) {
                    let r = super::frame_rect(f);
                    self.edit = Some(InlineEdit {
                        target: EditTarget::Frame(i),
                        text: f.label.clone(),
                        rect: Rect::from_min_size(r.min, vec2(r.width(), theme::FRAME_HEADER))
                            .shrink2(vec2(8.0, 4.0)),
                        multiline: false,
                        started: false,
                    });
                }
            }
            _ => {}
        }
    }
}

/// The Keyboard key under screen point `p`, for the keyboard drawn in `r`.
fn key_under(io: &CanvasIo, node: NodeId, r: Rect, p: Pos2) -> Option<usize> {
    let inst = io.doc.graph().node(node)?;
    let (low, keys) = piano::layout(io.registry, inst);
    piano::key_at(&piano::keys(low, keys, r), p)
}

/// The XY Pad's unwired `x` and `y` for screen point `p` on the pad drawn
/// in `r` (clamped to the pad; y grows upwards).
fn pad_values(n: &NodeView, r: Rect, p: Pos2) -> Vec<(&'static str, Value)> {
    let tx = ((p.x - r.min.x) / r.width().max(1.0)).clamp(0.0, 1.0);
    let ty = ((r.max.y - p.y) / r.height().max(1.0)).clamp(0.0, 1.0);
    [("x", tx), ("y", ty)]
        .into_iter()
        .filter_map(|(name, t)| {
            let s = n.socket(name, true).filter(|s| !s.wired)?;
            let v = widgets::at_position(f64::from(t), s.spec.as_ref()?)?;
            Some((name, Value::Float(v)))
        })
        .collect()
}

/// Could `node` be spliced into `wire`? (The same rule as `insert_on_wire`.)
fn can_insert(io: &CanvasIo, node: NodeId, wire: &Wire) -> bool {
    let mut project = io.doc.project.clone();
    let mut ids = ac_core::IdGen::new(0);
    let scope = io.doc.current_def().cloned();
    edit::edit_scope(
        &mut project,
        scope.as_ref(),
        io.registry,
        &mut ids,
        |g, ctx| edit::insert_on_wire(g, ctx, node, wire),
    )
}

fn snap_f(v: f32) -> f32 {
    (v / theme::GRID).round() * theme::GRID
}

fn snap_vec(v: egui::Vec2) -> egui::Vec2 {
    vec2(snap_f(v.x), snap_f(v.y))
}

/// A number as typed text, without its unit (units are accepted back).
pub fn plain_number(v: f64, spec: &ac_core::SocketSpec) -> String {
    if spec.ty == SignalType::Int {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}
