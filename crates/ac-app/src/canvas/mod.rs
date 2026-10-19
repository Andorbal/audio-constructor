//! The node canvas (docs/UX.md "Canvas interactions"): pan, zoom, select,
//! move, wire, cut, reroute, frames, inline widgets, and live feedback.
//!
//! Each frame builds a [`scene::Scene`] from the document, handles the
//! pointer against it ([`interact`]), and paints it ([`paint`]). The
//! canvas hit-tests by hand rather than through egui widgets so that the
//! drawing order, the hit order, and the zoomed geometry always agree.

mod access;
mod arrange;
pub mod bodies;
pub mod controls;
mod interact;
mod keys;
pub mod live;
mod menu;
pub mod minimap;
mod paint;
pub mod piano;
mod portal;
pub mod scene;
mod timing;
pub mod view;
pub mod widgets;
pub mod wires;

pub use access::Focus;
pub use keys::{KeyCmd, NUDGE};
pub use menu::arrange_items;
pub use paint::pretty as paint_pretty;

use std::collections::{BTreeSet, HashMap};

use ac_core::{Endpoint, GroupId, NodeId, Registry, SignalType, SocketSpec, Value, Wire};
use ac_engine::{CompileReport, EngineHandle};
use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::document::Document;
use crate::edit::{self, GROUP_IN, GROUP_OUT};
use crate::i18n::{tr, tr_args};
use crate::theme;
use scene::{Body, Class, NodeView, Scene};
use view::{Transform, View};

/// A wire being dragged, or the half-made wire the palette will complete.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWire {
    /// Dragging from an output: the output.
    pub from: Option<Endpoint>,
    /// Dragging from an input: the input.
    pub to: Option<Endpoint>,
    pub ty: SignalType,
}

/// Things the canvas asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasEvent {
    /// Open the add-node palette at a screen position (canvas position for
    /// the node), optionally filtered to nodes that can complete a wire.
    OpenPalette {
        screen: Pos2,
        canvas: Pos2,
        pending: Option<PendingWire>,
    },
    /// Pick an audio file for a Clip node.
    ChooseClipFile(NodeId),
    /// Show a node's docs in the side panel.
    ShowHelp(NodeId),
    /// Something to tell the user in the status line.
    Status(String),
    /// Run an editor command (from a right-click menu).
    Run(crate::actions::Action),
    /// Pin nodes to a performance panel, or unpin them.
    Pin(crate::perform::PinRequest),
    /// Learn a binding for an unwired input (UX.md "Bindings").
    Bind { node: NodeId, socket: String },
}

/// What the canvas needs from the app each frame.
pub struct CanvasIo<'a> {
    pub doc: &'a mut Document,
    pub registry: &'a Registry,
    pub engine: &'a mut EngineHandle,
    pub report: &'a CompileReport,
    /// Asset paths that failed to load.
    pub missing_assets: &'a BTreeSet<String>,
    /// Skip animations while the audio thread is heavily loaded (UX.md).
    pub calm: bool,
    /// Per-node timing for the headers, while it's shown (`Ctrl+T`).
    pub timing: Option<&'a crate::timing::Timings>,
    /// Record nodes can write takes (the project is saved somewhere).
    pub can_record: bool,
    /// Bound inputs, with the values their bindings last set.
    pub bound: &'a crate::bindings::BoundInputs,
    /// View ▸ Reduce Motion: no fades, eases, or flowing wires.
    pub reduce_motion: bool,
    pub events: Vec<CanvasEvent>,
}

impl CanvasIo<'_> {
    /// A node of the current graph as drawn: bound inputs at the value
    /// their bindings last set while the engine runs.
    pub fn shown_node(&self, id: NodeId) -> Option<ac_core::NodeInstance> {
        let path = self.doc.node_path(id);
        self.doc
            .graph()
            .node(id)
            .map(|n| self.bound.shown(&path, n))
    }

    /// The bindings of a node's input in the current graph (through the
    /// instance the scope was entered by).
    pub fn bound_input(&self, id: NodeId, socket: &str) -> Option<&crate::bindings::BoundInput> {
        self.bound.get(&self.doc.node_path(id), socket)
    }
}

/// What the pointer is over.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    None,
    Socket {
        node: NodeId,
        name: String,
        input: bool,
    },
    /// The inline widget of an unwired input.
    Field {
        node: NodeId,
        socket: String,
    },
    /// The fire button of an unwired trigger input.
    Fire {
        node: NodeId,
        socket: String,
    },
    /// A control node's widget body.
    Body {
        node: NodeId,
    },
    /// The collapse caret in a header.
    Collapse {
        node: NodeId,
    },
    Header {
        node: NodeId,
    },
    Node {
        node: NodeId,
    },
    /// The resize grip of a comment.
    Resize {
        node: NodeId,
    },
    FrameHeader(usize),
    FrameResize(usize),
    Wire(Wire),
    /// The minimap in the corner.
    Minimap,
}

impl Hit {
    pub fn node(&self) -> Option<NodeId> {
        match self {
            Hit::Socket { node, .. }
            | Hit::Field { node, .. }
            | Hit::Fire { node, .. }
            | Hit::Body { node }
            | Hit::Collapse { node }
            | Hit::Header { node }
            | Hit::Node { node }
            | Hit::Resize { node } => Some(*node),
            _ => None,
        }
    }
}

/// An in-progress gesture.
#[derive(Clone, Debug)]
pub enum Gesture {
    Idle,
    Pan,
    BoxSelect {
        origin: Pos2,
        additive: bool,
    },
    /// Dragging nodes with the pointer, or moving them with `G` (no button).
    Move {
        ids: Vec<NodeId>,
        starts: Vec<Pos2>,
        origin: Pos2,
        grab: bool,
        /// A wire the single moved node would be spliced into on release.
        insert: Option<Wire>,
    },
    Wire {
        pending: PendingWire,
        /// The wire this drag picked up from an input, removed on release.
        detached: Option<Wire>,
        /// The socket the end has snapped to, and since when.
        snap: Option<(NodeId, String, f64)>,
    },
    Cut {
        points: Vec<Pos2>,
    },
    Reroute {
        points: Vec<Pos2>,
    },
    MoveFrame {
        index: usize,
        start: [f32; 4],
        ids: Vec<NodeId>,
        starts: Vec<Pos2>,
        origin: Pos2,
    },
    ResizeFrame {
        index: usize,
        start: [f32; 4],
        origin: Pos2,
    },
    ResizeNode {
        node: NodeId,
        start: Vec2,
        origin: Pos2,
    },
    /// Pressing or dragging in the minimap moves the view.
    Minimap {
        map: minimap::MiniMap,
    },
}

/// Typing into a field on the canvas.
#[derive(Clone, Debug)]
pub struct InlineEdit {
    pub target: EditTarget,
    pub text: String,
    /// Where the editor sits (canvas units).
    pub rect: Rect,
    pub multiline: bool,
    pub started: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditTarget {
    Param {
        node: NodeId,
        socket: String,
        spec: SocketSpec,
    },
    Label(NodeId),
    Comment(NodeId),
    /// The name of the portal a tag belongs to.
    Portal(NodeId),
    Frame(usize),
    Interface {
        input_side: bool,
        name: String,
    },
}

/// A node being removed, still shrinking away.
#[derive(Clone, Debug)]
struct Ghost {
    rect: Rect,
    accent: egui::Color32,
    died: f64,
}

/// The canvas's own state (everything else lives in the document).
pub struct Canvas {
    pub view: View,
    /// Where a smooth zoom or fit is heading.
    target: Option<View>,
    pub gesture: Gesture,
    /// What was under the pointer when the primary button went down.
    press: Option<(Hit, Pos2)>,
    pub live: live::LiveState,
    born: HashMap<NodeId, f64>,
    ghosts: Vec<Ghost>,
    /// Interface card positions moved by the user, per definition.
    iface_pos: HashMap<GroupId, [Pos2; 2]>,
    pub edit: Option<InlineEdit>,
    menu: Option<(menu::Target, Pos2)>,
    choice: Option<(NodeId, String, Pos2)>,
    /// Frames the menu / dropdown has been open (the opening click must
    /// not count as a click outside).
    menu_age: u32,
    choice_age: u32,
    /// Widget bodies and inline values being played (shared with panels).
    pub controls: controls::Controls,
    /// Space was used to pan, so its release doesn't toggle playback.
    space_panned: bool,
    /// Screen rectangle of the canvas last frame.
    pub screen: Rect,
    /// Pointer position over the canvas (screen), if any.
    pub pointer: Option<Pos2>,
    /// The scene drawn last frame (tests read socket positions from it).
    pub last_scene: Option<Scene>,
    /// The hint shown at the bottom of the canvas during a gesture.
    pub hint: Option<String>,
    /// Hover-sensitive rectangles (screen) with their tooltip text.
    tips: Vec<(Rect, String)>,
    /// The timing chips drawn in headers last frame (tests read them).
    pub header_timing: Vec<(NodeId, String)>,
    /// Portal tags lit this frame: the partners of a hovered or selected one.
    portal_lit: BTreeSet<NodeId>,
    /// Which receiver a double-click on each sender jumped to last.
    portal_jump: HashMap<NodeId, usize>,
    /// Show the minimap while the graph doesn't fit the view.
    pub minimap: bool,
    /// The minimap this frame, if it shows.
    minimap_now: Option<minimap::MiniMap>,
    /// The view was set from the document (load, scope change).
    view_scope: Option<Vec<NodeId>>,
    /// The view each group scope was last left at.
    scope_views: HashMap<Vec<NodeId>, View>,
    /// The current graph with interface wires, refreshed each paint.
    graph_cache: ac_core::Graph,
    /// Frame the graph once the canvas knows its real size: a project
    /// loaded before the first frame was framed against a guess, and the
    /// window and panels settle over the first few frames. Counts the
    /// frames left to follow size changes, with the rect last framed.
    fit_on_show: (u8, Rect),
    /// The keyboard's place on the canvas (Tab, Alt+arrows), and whether
    /// it shows (it hides when the pointer takes over).
    pub focus: Option<Focus>,
    pub focus_visible: bool,
    /// Shift+Enter started a selection: Tab moves only the focus.
    pub keep_selection: bool,
    /// A wire being made from the keyboard (Enter on a socket).
    kb_wire: Option<PendingWire>,
    /// Keyboard commands waiting for the next frame.
    keys: Vec<KeyCmd>,
    /// The widgets drawn for cards and sockets last frame (egui focus to
    /// canvas focus).
    a11y_ids: HashMap<egui::Id, Focus>,
    /// The focus as egui last saw it: when `focus` differs, the canvas
    /// moved it (access.rs).
    synced: Option<Focus>,
    /// The open menu came from Shift+F10: the canvas takes the keyboard
    /// back when it closes.
    menu_from_keys: bool,
    /// The arrows moved the grab: it splices into the wire under the
    /// moved card rather than under the pointer.
    grab_keys: bool,
    /// The value being typed came from F2 on a socket: the canvas takes
    /// the keyboard back when it's done.
    edit_from_keys: bool,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            view: View::default(),
            target: None,
            gesture: Gesture::Idle,
            press: None,
            live: live::LiveState::default(),
            born: HashMap::new(),
            ghosts: Vec::new(),
            iface_pos: HashMap::new(),
            edit: None,
            menu: None,
            choice: None,
            menu_age: 0,
            choice_age: 0,
            controls: controls::Controls::default(),
            space_panned: false,
            screen: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
            pointer: None,
            last_scene: None,
            hint: None,
            tips: Vec::new(),
            header_timing: Vec::new(),
            portal_lit: BTreeSet::new(),
            portal_jump: HashMap::new(),
            minimap: true,
            minimap_now: None,
            view_scope: None,
            scope_views: HashMap::new(),
            graph_cache: ac_core::Graph::default(),
            fit_on_show: (0, Rect::NOTHING),
            focus: None,
            focus_visible: false,
            keep_selection: false,
            kb_wire: None,
            keys: Vec::new(),
            a11y_ids: HashMap::new(),
            synced: None,
            menu_from_keys: false,
            grab_keys: false,
            edit_from_keys: false,
        }
    }
}

impl Canvas {
    pub fn transform(&self) -> Transform {
        Transform {
            view: self.view,
            screen: self.screen,
        }
    }

    /// Canvas position under the pointer, or the view center.
    pub fn pointer_canvas(&self) -> Pos2 {
        let t = self.transform();
        match self.pointer {
            Some(p) if self.screen.contains(p) => t.to_canvas(p),
            _ => self.view.center,
        }
    }

    /// Is the canvas busy with something keys shouldn't interrupt?
    pub fn is_typing(&self) -> bool {
        self.edit.is_some()
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.gesture, Gesture::Idle) && !self.controls.is_active()
    }

    /// A node was just added from the keyboard or a menu: it takes the
    /// focus, and the ring shows when the pointer isn't on the canvas
    /// (the palette's Enter, not a drop).
    pub fn focus_added(&mut self, id: NodeId) {
        self.focus = Some(Focus::Node(id));
        self.keep_selection = false;
        if self.pointer.is_none() {
            self.focus_visible = true;
        }
    }

    /// Note that nodes were just created (they fade in).
    pub fn mark_born(&mut self, ids: &[NodeId], now: f64) {
        for id in ids {
            self.born.insert(*id, now);
        }
    }

    /// Smoothly frame the given canvas bounds.
    pub fn fit(&mut self, bounds: Option<Rect>, max_zoom: f32) {
        if let Some(b) = bounds {
            self.target = Some(View::fit(b, self.screen, max_zoom));
        }
    }

    /// Canvas bounds of every node and frame in the current graph.
    fn everything(&self, doc: &Document, registry: &Registry) -> Option<Rect> {
        let frames = doc.graph().frames.iter().map(frame_rect);
        self.build(doc, registry)
            .bounds(None)
            .into_iter()
            .chain(frames)
            .reduce(|a, b| a.union(b))
    }

    /// Frame everything (Home).
    pub fn fit_all(&mut self, doc: &Document, registry: &Registry) {
        self.fit(self.everything(doc, registry), 1.0);
    }

    /// Frame the selection (`.`).
    pub fn fit_selection(&mut self, doc: &Document, registry: &Registry) {
        let scene = self.build(doc, registry);
        if doc.selection.is_empty() {
            self.fit(self.everything(doc, registry), 1.0);
        } else {
            self.fit(scene.bounds(Some(&doc.selection)), 1.5);
        }
    }

    /// Center on one node without changing the zoom much.
    pub fn focus_node(&mut self, doc: &Document, registry: &Registry, id: NodeId) {
        let scene = self.build(doc, registry);
        if let Some(n) = scene.node(id) {
            self.target = Some(View {
                center: n.rect.center(),
                zoom: self.view.zoom.max(0.8),
            });
        }
    }

    /// Restore the view stored in a project (or frame it if none).
    pub fn reset_for(&mut self, doc: &Document, registry: &Registry) {
        self.gesture = Gesture::Idle;
        self.controls.active = None;
        self.edit = None;
        self.menu = None;
        self.choice = None;
        self.iface_pos.clear();
        match &doc.project.view {
            Some(v) if v.zoom.is_finite() && v.zoom > 0.0 => {
                self.view = View {
                    center: pos2(v.center[0], v.center[1]),
                    zoom: v.zoom.clamp(theme::ZOOM_MIN, theme::ZOOM_MAX),
                };
                self.target = None;
                // A stored view wins over framing still pending from the
                // document this one replaced.
                self.fit_on_show = (0, Rect::NOTHING);
            }
            _ => {
                if let Some(b) = self.everything(doc, registry) {
                    self.view = View::fit(b, self.screen, 1.0);
                }
                self.target = None;
                self.fit_on_show = (6, Rect::NOTHING);
            }
        }
        self.view_scope = Some(doc.scope_prefix());
        self.scope_views.clear();
    }

    fn build(&self, doc: &Document, registry: &Registry) -> Scene {
        let ip = doc
            .current_def()
            .and_then(|d| self.iface_pos.get(d).copied());
        scene::build(doc, registry, ip)
    }

    /// Screen position of a socket in the last drawn frame (tests).
    pub fn socket_screen_pos(&self, node: NodeId, socket: &str, input: bool) -> Option<Pos2> {
        let s = self.last_scene.as_ref()?;
        let p = s.node(node)?.socket(socket, input)?.pos;
        Some(self.transform().to_screen(p))
    }

    /// Screen rectangle of a node's card in the last drawn frame (tests).
    pub fn node_screen_rect(&self, node: NodeId) -> Option<Rect> {
        let s = self.last_scene.as_ref()?;
        Some(self.transform().rect_to_screen(s.node(node)?.rect))
    }

    /// Start moving the selection with the pointer (`G`). `own_step`:
    /// record the move as its own undo step; a grab that follows a
    /// duplicate folds into the duplicate's step instead, so one undo
    /// removes the copies.
    pub fn start_grab(&mut self, doc: &mut Document, own_step: bool) {
        let ids: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        if ids.is_empty() {
            return;
        }
        let starts = ids
            .iter()
            .filter_map(|id| doc.graph().node(*id))
            .map(|n| pos2(n.ui.pos[0], n.ui.pos[1]))
            .collect();
        if own_step {
            doc.checkpoint(&tr("undo-move"));
        }
        self.gesture = Gesture::Move {
            ids,
            starts,
            origin: self.pointer_canvas(),
            grab: true,
            insert: None,
        };
        self.hint = Some(tr("hint-grab"));
    }

    /// Escape: cancel the current gesture (a grab returns nodes to where
    /// they were).
    pub fn cancel(&mut self, doc: &mut Document) -> bool {
        if self.menu.take().is_some() || self.choice.take().is_some() {
            return true;
        }
        if self.kb_wire.take().is_some() {
            self.hint = None;
            return true;
        }
        if self.edit.take().is_some() {
            return true;
        }
        if self.controls.cancel(doc) {
            self.hint = None;
            return true;
        }
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => false,
            Gesture::Move { ids, starts, .. } => {
                self.grab_keys = false;
                let moves: Vec<(NodeId, [f32; 2])> = ids
                    .iter()
                    .zip(&starts)
                    .map(|(i, p)| (*i, [p.x, p.y]))
                    .collect();
                doc.move_nodes(&moves);
                self.hint = None;
                true
            }
            _ => {
                self.hint = None;
                true
            }
        }
    }

    /// Delete the selection (with `reconnect`, keep the chain). The nodes
    /// shrink away.
    pub fn delete_selection(
        &mut self,
        doc: &mut Document,
        registry: &Registry,
        reconnect: bool,
        now: f64,
    ) {
        let ids: BTreeSet<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        if ids.is_empty() {
            return;
        }
        let scene = self.build(doc, registry);
        for id in &ids {
            if let Some(n) = scene.node(*id) {
                self.ghosts.push(Ghost {
                    rect: n.rect,
                    accent: n.accent,
                    died: now,
                });
            }
        }
        let label = if reconnect {
            tr("undo-delete-reconnect")
        } else {
            tr("undo-delete")
        };
        doc.edit(registry, &label, |g, ctx| {
            if reconnect {
                edit::delete_reconnect(g, ctx, &ids);
            } else {
                edit::delete_nodes(g, &ids);
            }
        });
        doc.selection.clear();
    }

    /// Put a labeled frame around the selection (`Ctrl+J`).
    pub fn frame_selection(&mut self, doc: &mut Document, registry: &Registry) {
        let scene = self.build(doc, registry);
        let ids: BTreeSet<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        let Some(b) = scene.bounds(Some(&ids)) else {
            return;
        };
        let r = Rect::from_min_max(
            b.min - vec2(theme::FRAME_PAD, theme::FRAME_PAD + theme::FRAME_HEADER),
            b.max + vec2(theme::FRAME_PAD, theme::FRAME_PAD),
        );
        let n = doc.graph().frames.len() + 1;
        doc.edit(registry, &tr("undo-add-frame"), |g, _| {
            g.frames.push(ac_core::Frame {
                label: if n == 1 {
                    tr("canvas-frame")
                } else {
                    tr_args("canvas-frame-numbered", &[("number", n.into())])
                },
                rect: [r.min.x, r.min.y, r.width(), r.height()],
                color: None,
            });
        });
        let index = doc.graph().frames.len() - 1;
        let header = Rect::from_min_size(r.min, vec2(r.width(), theme::FRAME_HEADER));
        self.edit = Some(InlineEdit {
            target: EditTarget::Frame(index),
            text: doc.graph().frames[index].label.clone(),
            rect: header.shrink2(vec2(8.0, 4.0)),
            multiline: false,
            started: false,
        });
    }

    /// Start renaming the selected node (double-click a header, or F2).
    pub fn rename_node(&mut self, doc: &Document, registry: &Registry, id: NodeId) {
        let scene = self.build(doc, registry);
        let Some(n) = scene.node(id) else { return };
        if edit::is_virtual(id) {
            return;
        }
        if let Class::Portal(info) = &n.class {
            self.edit = Some(InlineEdit {
                target: EditTarget::Portal(id),
                text: info.name.clone(),
                rect: n.rect.shrink2(vec2(4.0, 1.0)),
                multiline: false,
                started: false,
            });
            return;
        }
        if n.class == Class::Comment {
            self.edit = Some(InlineEdit {
                target: EditTarget::Comment(id),
                text: scene::comment_text(doc, id),
                rect: Rect::from_min_max(
                    n.header.left_bottom() + vec2(6.0, 4.0),
                    n.rect.max - vec2(6.0, 6.0),
                ),
                multiline: true,
                started: false,
            });
            return;
        }
        self.edit = Some(InlineEdit {
            target: EditTarget::Label(id),
            text: n.title.clone(),
            rect: n.header.shrink2(vec2(22.0, 3.0)),
            multiline: false,
            started: false,
        });
    }

    fn send_param(io: &mut CanvasIo, id: NodeId, socket: &str, value: Value, now: f64) {
        crate::params::set(io.doc, io.engine, id, socket, value, now);
    }

    fn fire(io: &mut CanvasIo, id: NodeId, socket: &str) {
        crate::params::fire(io.doc, io.engine, id, socket);
    }

    /// Inputs the canvas is holding down for the engine on `node` right
    /// now (a pressed Button, a played Keyboard key, a touched XY Pad),
    /// including releases still waiting out the minimum hold.
    pub fn held_inputs(&self, node: NodeId) -> Vec<String> {
        self.controls.held_inputs(node)
    }

    /// Screen rectangle of a node's widget body in the last drawn frame
    /// (tests).
    pub fn body_screen_rect(&self, node: NodeId) -> Option<Rect> {
        let s = self.last_scene.as_ref()?;
        Some(self.transform().rect_to_screen(s.node(node)?.body?.1))
    }

    /// Main entry: draw the canvas into `rect` and handle its input.
    pub fn show(&mut self, ui: &mut egui::Ui, io: &mut CanvasIo) {
        let rect = ui.available_rect_before_wrap();
        self.screen = rect;
        let (frames, framed) = self.fit_on_show;
        if frames > 0 && rect.width() > 1.0 && rect.height() > 1.0 {
            if rect != framed
                && let Some(b) = self.everything(io.doc, io.registry)
            {
                self.view = View::fit(b, rect, 1.0);
                self.target = None;
            }
            self.fit_on_show = (frames - 1, rect);
            ui.ctx().request_repaint();
        }
        let id = egui::Id::unique("canvas");
        let response = ui.interact(rect, id, egui::Sense::click_and_drag());
        let now = ui.input(|i| i.time);
        let dt = self.live.begin_frame(now);

        // A scope change (entering or leaving a group): come back to where
        // that graph was last seen, or frame it the first time.
        let prefix = io.doc.scope_prefix();
        if self.view_scope.as_ref() != Some(&prefix) {
            if let Some(old) = self.view_scope.take() {
                self.scope_views
                    .insert(old, self.target.unwrap_or(self.view));
                match self.scope_views.get(&prefix) {
                    Some(v) => self.target = Some(*v),
                    None => {
                        if let Some(b) = self.everything(io.doc, io.registry) {
                            self.target = Some(View::fit(b, rect, 1.0));
                        }
                    }
                }
            }
            self.view_scope = Some(prefix);
        }

        self.pointer = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p) || !self.is_idle());
        if io.reduce_motion
            && let Some(target) = self.target.take()
        {
            self.view = target;
        }
        self.handle_view_input(ui, &response, dt);

        // Releases held back so a quick tap still reaches the engine.
        if self.controls.release_due(io, now) {
            ui.ctx().request_repaint();
        }

        let mut scene = self.build(io.doc, io.registry);
        // Keyboard commands from the app (Tab, Enter, the arrows…).
        if !self.keys.is_empty() && self.run_keys(ui, io, &scene, now) {
            scene = self.build(io.doc, io.registry);
        }
        // The pointer takes over from the keyboard.
        if response.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
            self.focus_visible = false;
            self.keep_selection = false;
        }
        if io.reduce_motion {
            self.ghosts.clear();
            self.born.clear();
        }
        self.minimap_now = self.minimap_layout(&scene, io.doc);
        let changed = self.handle_pointer(ui, &response, io, &scene, now);
        if changed {
            scene = self.build(io.doc, io.registry);
        }
        self.minimap_now = self.minimap_layout(&scene, io.doc);
        self.ghosts
            .retain(|g| now - g.died < f64::from(theme::ANIM_SECONDS));
        self.born
            .retain(|_, t| now - *t < f64::from(theme::ANIM_SECONDS));

        let painter = ui.painter_at(rect);
        self.tips.clear();
        self.header_timing.clear();
        self.paint(ui, &painter, io, &scene, now, dt);
        self.show_inline_edit(ui, io, now);
        self.show_menus(ui, io, &scene, now);
        if self.menu_from_keys && self.menu.is_none() {
            // The menu Shift+F10 opened closed: the keyboard comes back.
            self.menu_from_keys = false;
            self.focus_visible = true;
        }
        if self.edit_from_keys && self.edit.is_none() {
            // The value F2 opened was set or dropped: the keyboard comes
            // back to the socket.
            self.edit_from_keys = false;
            self.focus_visible = true;
        }
        self.show_tooltips(ui, io, &scene);
        self.accessibility(ui, io, &scene, &response);

        if io.reduce_motion
            && let Some(target) = self.target.take()
        {
            // Jump where a smooth zoom or fit was heading.
            self.view = target;
        }
        let animating = !self.ghosts.is_empty() || !self.born.is_empty() || self.target.is_some();
        if animating || self.live.active || !self.is_idle() {
            ui.ctx().request_repaint();
        }
        self.last_scene = Some(scene);
    }

    /// Pan and zoom: middle-drag, space+drag, wheel, pinch, two-finger scroll.
    fn handle_view_input(&mut self, ui: &egui::Ui, response: &egui::Response, dt: f32) {
        let rect = self.screen;
        let hovered = response.hovered() || response.contains_pointer();
        if let Some(target) = self.target
            && !self.view.approach(target, dt)
        {
            self.target = None;
        }
        if !hovered {
            return;
        }
        let (events, zoom_delta, pointer) =
            ui.input(|i| (i.events.clone(), i.zoom_delta(), i.pointer.hover_pos()));
        let anchor = pointer.unwrap_or(rect.center());
        for e in &events {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                source,
                modifiers,
                ..
            } = e
            {
                // Ctrl+wheel arrives as zoom_delta (handled below).
                if modifiers.ctrl || modifiers.command {
                    continue;
                }
                let is_wheel = match source {
                    egui::MouseWheelSource::Wheel => true,
                    egui::MouseWheelSource::Trackpad | egui::MouseWheelSource::Momentum => false,
                    _ => *unit != egui::MouseWheelUnit::Point,
                };
                if is_wheel {
                    let steps = match unit {
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 3.0,
                        egui::MouseWheelUnit::Point => delta.y / 40.0,
                    };
                    let base = self.target.unwrap_or(self.view);
                    let mut v = base;
                    v.zoom_at(rect, anchor, 1.15f32.powf(steps));
                    self.target = Some(v);
                } else {
                    self.target = None;
                    self.view.pan(*delta);
                }
            }
        }
        if (zoom_delta - 1.0).abs() > 1e-4 {
            self.target = None;
            self.view.zoom_at(rect, anchor, zoom_delta);
        }
    }

    /// Delete key etc. are handled by the app; this lets a press of Space
    /// that was used to pan not toggle playback.
    pub fn take_space_panned(&mut self) -> bool {
        std::mem::take(&mut self.space_panned)
    }

    /// Hit test a screen point against the scene (topmost first).
    pub fn hit(&self, scene: &Scene, doc: &Document, p: Pos2) -> Hit {
        if self.minimap_now.is_some_and(|m| m.rect.contains(p)) {
            return Hit::Minimap;
        }
        let t = self.transform();
        let z = self.view.zoom;
        for n in scene.nodes.iter().rev() {
            let card = t.rect_to_screen(n.rect);
            let near = card.expand(theme::SOCKET_HIT + 2.0);
            if !near.contains(p) {
                continue;
            }
            // Sockets win over the card they sit on.
            let r = theme::SOCKET_HIT.max(theme::SOCKET_RADIUS * z + 3.0);
            if let Class::Portal(info) = &n.class {
                // Only the dot is a socket: the far side is the hidden link
                // (except on a receiver whose sender is gone).
                let open = n
                    .inputs
                    .iter()
                    .chain(n.outputs.iter())
                    .filter(|s| portal_side_open(info, s.input));
                for s in open {
                    if (t.to_screen(s.pos) - p).length() <= r {
                        return Hit::Socket {
                            node: n.id,
                            name: s.name.clone(),
                            input: s.input,
                        };
                    }
                }
                if card.expand(2.0).contains(p) {
                    return Hit::Node { node: n.id };
                }
                continue;
            }
            let mut best: Option<(f32, &scene::SocketView)> = None;
            for s in n.inputs.iter().chain(n.outputs.iter()) {
                let d = (t.to_screen(s.pos) - p).length();
                if d <= r && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, s));
                }
            }
            if let Some((_, s)) = best {
                if n.class == Class::Reroute {
                    // A reroute's dot moves it; its output starts a wire
                    // from the handle to its right (see paint).
                    let handle = t.to_screen(s.pos) + vec2(theme::REROUTE_RADIUS * z + 8.0, 0.0);
                    if (handle - p).length() <= 6.0 {
                        return Hit::Socket {
                            node: n.id,
                            name: "out".into(),
                            input: false,
                        };
                    }
                    if card.expand(3.0).contains(p) {
                        return Hit::Node { node: n.id };
                    }
                } else {
                    return Hit::Socket {
                        node: n.id,
                        name: s.name.clone(),
                        input: s.input,
                    };
                }
            }
            if n.class == Class::Reroute {
                let handle =
                    t.to_screen(n.rect.center()) + vec2(theme::REROUTE_RADIUS * z + 8.0, 0.0);
                if (handle - p).length() <= 6.0 {
                    return Hit::Socket {
                        node: n.id,
                        name: "out".into(),
                        input: false,
                    };
                }
                if card.expand(3.0).contains(p) {
                    return Hit::Node { node: n.id };
                }
                continue;
            }
            if !card.contains(p) {
                continue;
            }
            if n.class == Class::Comment {
                let grip = Rect::from_min_max(card.max - vec2(14.0, 14.0), card.max);
                if grip.contains(p) {
                    return Hit::Resize { node: n.id };
                }
                return Hit::Node { node: n.id };
            }
            let header = t.rect_to_screen(n.header);
            if header.contains(p) {
                let caret = Rect::from_min_size(header.min, vec2(20.0 * z, header.height()));
                if caret.contains(p) && !matches!(n.class, Class::Interface { .. }) {
                    return Hit::Collapse { node: n.id };
                }
                return Hit::Header { node: n.id };
            }
            if let Some((_, body)) = n.body {
                let b = t.rect_to_screen(body);
                if b.contains(p) {
                    return Hit::Body { node: n.id };
                }
            }
            if !n.collapsed
                && let Some(node) = doc.graph().node(n.id)
            {
                for s in &n.inputs {
                    if s.wired || !t.rect_to_screen(s.row).contains(p) {
                        continue;
                    }
                    let Some(spec) = &s.spec else { continue };
                    if n.body.is_some_and(|(b, _)| b.drives(&s.name)) {
                        continue;
                    }
                    let f = t.rect_to_screen(field_rect(s));
                    match widgets::field(spec) {
                        widgets::Field::Label => {}
                        widgets::Field::Fire => {
                            let b = Rect::from_min_max(pos2(f.max.x - 26.0 * z, f.min.y), f.max);
                            if b.contains(p) {
                                return Hit::Fire {
                                    node: n.id,
                                    socket: s.name.clone(),
                                };
                            }
                        }
                        _ if f.contains(p) => {
                            let _ = node;
                            return Hit::Field {
                                node: n.id,
                                socket: s.name.clone(),
                            };
                        }
                        _ => {}
                    }
                }
            }
            return Hit::Node { node: n.id };
        }
        // Frames: header to move, corner to resize.
        for (i, f) in doc.graph().frames.iter().enumerate().rev() {
            let r = t.rect_to_screen(frame_rect(f));
            let header = Rect::from_min_size(r.min, vec2(r.width(), theme::FRAME_HEADER * z));
            if header.contains(p) {
                return Hit::FrameHeader(i);
            }
            let grip = Rect::from_min_max(r.max - vec2(16.0, 16.0), r.max);
            if grip.contains(p) {
                return Hit::FrameResize(i);
            }
        }
        if let Some(w) = self.wire_at(scene, p) {
            return Hit::Wire(w);
        }
        Hit::None
    }

    /// The wire under a screen point in the last drawn frame.
    pub fn wire_under(&self, doc: &Document, p: Pos2) -> Option<Wire> {
        let _ = doc;
        self.wire_at(self.last_scene.as_ref()?, p)
    }

    /// The wire under a screen point.
    fn wire_at(&self, scene: &Scene, p: Pos2) -> Option<Wire> {
        let t = self.transform();
        let mut best: Option<(f32, &Wire)> = None;
        for w in &scene.wires {
            let c = wires::bezier(t.to_screen(w.from), t.to_screen(w.to), self.view.zoom);
            let bb = Rect::from_points(&c).expand(theme::WIRE_HIT);
            if !bb.contains(p) {
                continue;
            }
            let d = wires::distance(&wires::flatten(&c), p);
            if d <= theme::WIRE_HIT && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, &w.wire));
            }
        }
        best.map(|(_, w)| w.clone())
    }

    fn alpha_of(&self, id: NodeId, now: f64, calm: bool) -> f32 {
        match self.born.get(&id) {
            Some(t) if !calm => ease((now - t) as f32 / theme::ANIM_SECONDS),
            _ => 1.0,
        }
    }
}

/// Ease-out (no overshoot).
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

/// The inline widget rectangle of an input (canvas units), leaving room for
/// an output label on the same row.
pub fn field_rect(s: &scene::SocketView) -> Rect {
    let row = s.row;
    let right =
        (row.max.x - theme::NODE_PAD_X - s.reserve).max(row.min.x + theme::NODE_PAD_X + 30.0);
    Rect::from_min_max(
        pos2(row.min.x + theme::NODE_PAD_X, row.min.y + 2.0),
        pos2(right, row.max.y - 2.0),
    )
}

pub fn frame_rect(f: &ac_core::Frame) -> Rect {
    Rect::from_min_size(
        pos2(f.rect[0], f.rect[1]),
        vec2(f.rect[2].max(40.0), f.rect[3].max(40.0)),
    )
}

/// Is this side of a portal tag a socket you can wire? The dot always is
/// (a sender's input, a receiver's output); the other side is the portal's
/// hidden link, open only on a receiver whose sender is gone.
pub fn portal_side_open(info: &edit::portal::Info, input: bool) -> bool {
    let dot = input == info.send;
    dot || (!info.send && info.alone())
}

/// Can a socket accept the wire being dragged?
pub fn accepts(pending: &PendingWire, n: &NodeView, s: &scene::SocketView) -> bool {
    if matches!(n.class, Class::Missing(_)) {
        return false;
    }
    if let Class::Portal(info) = &n.class
        && !portal_side_open(info, s.input)
    {
        return false;
    }
    if s.input {
        if pending.from.is_none() {
            return false;
        }
        if pending.from.as_ref().is_some_and(|f| f.node == n.id) {
            return false;
        }
        // Group interface outputs only connect to inner nodes; the "add"
        // slot accepts anything with a type.
        if s.is_new_slot() {
            return pending.from.as_ref().is_some_and(|f| f.node != GROUP_IN);
        }
        edit::compatible(pending.ty, s.ty)
    } else {
        if pending.to.is_none() {
            return false;
        }
        if pending.to.as_ref().is_some_and(|t| t.node == n.id) {
            return false;
        }
        if s.is_new_slot() {
            return pending.to.as_ref().is_some_and(|t| t.node != GROUP_OUT);
        }
        edit::compatible(s.ty, pending.ty)
    }
}

/// Nodes of a body kind that need the pointer for themselves.
pub fn body_is_control(b: Body) -> bool {
    matches!(
        b,
        Body::Button
            | Body::Toggle
            | Body::Slider
            | Body::Knob
            | Body::XyPad
            | Body::Keyboard
            | Body::Number
            | Body::Dropdown
            | Body::Clip
    )
}
