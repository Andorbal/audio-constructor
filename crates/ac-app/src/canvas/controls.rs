//! Widget bodies as controls, shared by the canvas and the performance
//! panels: painting a body from the document and telemetry, and the
//! gestures that play it (holding a Button, a Keyboard key, or an XY Pad;
//! dragging a Slider, a Knob, a Number, or an inline value; clicking a
//! Toggle, a Dropdown, a Number, or a Clip). Both views drive these same
//! functions, so a control sets its inputs the same way wherever it's
//! played (see `params.rs`): values are coalesced document edits, held
//! inputs go straight to the engine.

use ac_core::{ConfigExt, NodeId, SignalType, SocketSpec, Value};
use egui::{Align2, Painter, Pos2, Rect};

use super::live::{Live, LiveState, MeterChannel};
use super::scene::{Body, NodeView};
use super::{CanvasEvent, CanvasIo, bodies, piano, widgets};
use crate::i18n::{tr, tr_args};
use crate::theme;

/// A held control (a Button, a Keyboard key, an XY Pad) stays down in the
/// engine for at least this long, so a quick tap is never lost between
/// blocks.
pub const MIN_HOLD: f64 = 0.04;

/// Pointer travel (points) that sweeps an inline value's whole soft range.
pub const DRAG_TRAVEL: f32 = 320.0;

/// Vertical pointer travel (points) that turns a knob through its range.
pub const KNOB_TRAVEL: f32 = 200.0;

/// A control gesture in progress.
#[derive(Clone, Debug)]
pub enum ControlGesture {
    /// Dragging a value: sideways for fields and sliders, up and down for
    /// knobs (`vertical`). `width` is the travel that sweeps the range.
    DragValue {
        node: NodeId,
        socket: String,
        spec: SocketSpec,
        start: f64,
        origin: Pos2,
        width: f32,
        vertical: bool,
    },
    /// A Button node held down.
    HoldButton { node: NodeId, since: f64 },
    /// A Keyboard key held down; dragging across the keys moves it.
    HoldKey {
        node: NodeId,
        key: usize,
        since: f64,
    },
    /// An XY Pad held down: the point follows the pointer. `touch`: the
    /// pad holds `touched` (it isn't wired).
    HoldPad {
        node: NodeId,
        since: f64,
        touch: bool,
    },
}

impl ControlGesture {
    /// The node the gesture plays.
    pub fn node(&self) -> NodeId {
        match self {
            ControlGesture::DragValue { node, .. }
            | ControlGesture::HoldButton { node, .. }
            | ControlGesture::HoldKey { node, .. }
            | ControlGesture::HoldPad { node, .. } => *node,
        }
    }

    /// The input a hold gesture keeps down, and since when.
    fn held(&self) -> Option<(NodeId, String, f64)> {
        match self {
            ControlGesture::HoldButton { node, since } => Some((*node, "pressed".into(), *since)),
            ControlGesture::HoldKey { node, key, since } => {
                Some((*node, piano::key_socket(*key), *since))
            }
            ControlGesture::HoldPad {
                node,
                since,
                touch: true,
            } => Some((*node, "touched".into(), *since)),
            _ => None,
        }
    }
}

/// A held input let go too soon after it was pressed: the release waits so
/// the engine sees the press for at least a block.
#[derive(Clone, Debug)]
struct PendingRelease {
    node: NodeId,
    socket: String,
    at: f64,
}

/// What a press-and-drag on a body turned into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    /// The control took the drag (a value is being dragged).
    Control,
    /// The body is a control that ignores drags (a Button).
    Nothing,
    /// Not something to drag: the view may move or reorder instead.
    Other,
}

/// What a click on a body asks the view to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Click {
    /// Handled, or nothing to do.
    Done,
    /// Not a control click: the view selects the node.
    Select,
    /// Open the choices of the body's `value` below it.
    Choose,
    /// Type a value for the body's `value`, starting from `text`.
    Type { spec: SocketSpec, text: String },
}

/// The control gesture under way, and holds waiting out [`MIN_HOLD`].
#[derive(Default)]
pub struct Controls {
    pub active: Option<ControlGesture>,
    releases: Vec<PendingRelease>,
}

impl Controls {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Whether `node`'s `socket` (any socket with `None`) is being dragged.
    pub fn dragging(&self, node: NodeId, socket: Option<&str>) -> bool {
        matches!(&self.active, Some(ControlGesture::DragValue { node: n, socket: s, .. })
            if *n == node && socket.is_none_or(|x| x == s))
    }

    /// The node a held Keyboard or XY Pad follows the pointer on.
    pub fn following(&self) -> Option<NodeId> {
        match &self.active {
            Some(ControlGesture::HoldKey { node, .. } | ControlGesture::HoldPad { node, .. }) => {
                Some(*node)
            }
            _ => None,
        }
    }

    /// Inputs held down for the engine on `node` right now (a pressed
    /// Button, a played Keyboard key, a touched XY Pad), including releases
    /// still waiting out the minimum hold.
    pub fn held_inputs(&self, node: NodeId) -> Vec<String> {
        let mut out: Vec<String> = self
            .active
            .as_ref()
            .and_then(ControlGesture::held)
            .filter(|h| h.0 == node)
            .map(|h| h.1)
            .into_iter()
            .collect();
        for r in &self.releases {
            if r.node == node && !out.contains(&r.socket) {
                out.push(r.socket.clone());
            }
        }
        out
    }

    /// Let go of holds that have waited out the minimum. Returns whether
    /// any are still waiting (keep repainting).
    pub fn release_due(&mut self, io: &mut CanvasIo, now: f64) -> bool {
        let (due, waiting): (Vec<PendingRelease>, Vec<PendingRelease>) =
            std::mem::take(&mut self.releases)
                .into_iter()
                .partition(|r| r.at <= now);
        self.releases = waiting;
        for r in due {
            hold(io, r.node, &r.socket, false);
        }
        !self.releases.is_empty()
    }

    /// The primary button went down on a body (`rect`, screen): controls
    /// that act on press (Button, Keyboard, XY Pad) start holding.
    pub fn press(&mut self, io: &mut CanvasIo, n: &NodeView, rect: Rect, pointer: Pos2, now: f64) {
        let Some((body, _)) = n.body else { return };
        let node = n.id;
        match body {
            Body::Button => {
                self.hold_down(io, node, "pressed");
                self.active = Some(ControlGesture::HoldButton { node, since: now });
            }
            Body::Keyboard => {
                if let Some(key) = key_under(io, node, rect, pointer) {
                    self.hold_down(io, node, &piano::key_socket(key));
                    self.active = Some(ControlGesture::HoldKey {
                        node,
                        key,
                        since: now,
                    });
                }
            }
            Body::XyPad => {
                let touch = n.socket("touched", true).is_some_and(|s| !s.wired);
                let values = pad_values(n, rect, pointer);
                if !touch && values.is_empty() {
                    // Every input is wired: the pad only shows them.
                    return;
                }
                crate::params::set_many(io.doc, io.engine, node, &values, now);
                if touch {
                    self.hold_down(io, node, "touched");
                }
                self.active = Some(ControlGesture::HoldPad {
                    node,
                    since: now,
                    touch,
                });
            }
            _ => {}
        }
    }

    /// While a Keyboard key or an XY Pad on `n` (drawn in `rect`) is held,
    /// follow the pointer: sliding onto another key plays it instead, and
    /// the pad's point moves.
    pub fn follow(&mut self, io: &mut CanvasIo, n: &NodeView, rect: Rect, p: Pos2, now: f64) {
        match self.active.clone() {
            Some(ControlGesture::HoldKey { node, key, since }) if node == n.id => {
                let Some(next) = key_under(io, node, rect, p) else {
                    return;
                };
                if next != key {
                    self.let_go(io, node, piano::key_socket(key), since, now);
                    self.hold_down(io, node, &piano::key_socket(next));
                    self.active = Some(ControlGesture::HoldKey {
                        node,
                        key: next,
                        since: now,
                    });
                }
            }
            Some(ControlGesture::HoldPad { node, .. }) if node == n.id => {
                let values = pad_values(n, rect, p);
                crate::params::set_many(io.doc, io.engine, node, &values, now);
            }
            _ => {}
        }
    }

    /// The primary button came up: a held control lets go.
    pub fn release(&mut self, io: &mut CanvasIo, now: f64) {
        let pad = match self.active {
            Some(ControlGesture::HoldPad { .. }) => true,
            Some(ControlGesture::HoldButton { .. } | ControlGesture::HoldKey { .. }) => false,
            _ => return,
        };
        if let Some((node, socket, since)) = self.active.as_ref().and_then(ControlGesture::held) {
            self.let_go(io, node, socket, since, now);
        }
        if pad {
            // The pad's moves were one undo step.
            io.doc.end_coalescing();
        }
        self.active = None;
    }

    /// A drag started on a body (`rect`, screen) at `origin`.
    pub fn begin_drag(
        &mut self,
        io: &mut CanvasIo,
        n: &NodeView,
        rect: Rect,
        origin: Pos2,
        now: f64,
    ) -> Grab {
        let node = n.id;
        // The unwired input a body drags, with its spec.
        let free = |socket: &str| {
            n.socket(socket, true)
                .filter(|s| !s.wired)
                .and_then(|s| s.spec.clone())
        };
        match n.body.map(|b| b.0) {
            Some(Body::Slider) => {
                let spec = n.socket("value", true).and_then(|s| s.spec.clone());
                if let (Some(_), Some(spec)) = (io.doc.graph().node(node), spec) {
                    // Jump to where the slider was grabbed, then drag.
                    let tpos = ((origin.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
                    let lo = spec.range.map_or(0.0, |r| r.0);
                    let start = widgets::dragged(
                        lo,
                        tpos * rect.width(),
                        rect.width(),
                        &spec,
                        false,
                        false,
                    );
                    crate::params::set(io.doc, io.engine, node, "value", Value::Float(start), now);
                    self.begin_value_drag(node, "value", spec, start, origin, rect.width(), false);
                }
                Grab::Control
            }
            // Knobs turn with an up-and-down drag; number fields drag
            // sideways like inline fields.
            Some(b @ (Body::Knob | Body::Number)) => {
                // From the value shown (a bound input's live value).
                match (free("value"), io.shown_node(node)) {
                    (Some(spec), Some(inst)) => {
                        let knob = b == Body::Knob;
                        let start = widgets::number(&inst, &spec);
                        let width = if knob { KNOB_TRAVEL } else { DRAG_TRAVEL };
                        self.begin_value_drag(node, "value", spec, start, origin, width, knob);
                        Grab::Control
                    }
                    _ => Grab::Other,
                }
            }
            Some(Body::Button) => Grab::Nothing,
            _ => Grab::Other,
        }
    }

    /// Start dragging an input's value from `start`.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_value_drag(
        &mut self,
        node: NodeId,
        socket: &str,
        spec: SocketSpec,
        start: f64,
        origin: Pos2,
        width: f32,
        vertical: bool,
    ) {
        self.active = Some(ControlGesture::DragValue {
            node,
            socket: socket.to_string(),
            spec,
            start,
            origin,
            width,
            vertical,
        });
    }

    /// The pointer moved while dragging a value.
    pub fn drag(&mut self, io: &mut CanvasIo, pointer: Pos2, modifiers: egui::Modifiers, now: f64) {
        let Some(ControlGesture::DragValue {
            node,
            socket,
            spec,
            start,
            origin,
            width,
            vertical,
        }) = &self.active
        else {
            return;
        };
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
        crate::params::set(io.doc, io.engine, node, &socket, value, now);
    }

    /// A value drag ended: the drag was one undo step.
    pub fn end_drag(&mut self, io: &mut CanvasIo) {
        if let Some(ControlGesture::DragValue { .. }) = self.active {
            io.doc.end_coalescing();
            self.active = None;
        }
    }

    /// Escape, or the view went away: a held control lets go (after its
    /// minimum hold) rather than staying down in the engine. Returns
    /// whether a gesture was cancelled.
    pub fn cancel(&mut self, doc: &mut crate::document::Document) -> bool {
        let Some(g) = self.active.take() else {
            return false;
        };
        if let Some((node, socket, since)) = g.held() {
            self.releases.push(PendingRelease {
                node,
                socket,
                at: since + MIN_HOLD,
            });
        }
        if matches!(
            g,
            ControlGesture::HoldPad { .. } | ControlGesture::DragValue { .. }
        ) {
            doc.end_coalescing();
        }
        true
    }

    /// Let go of everything now (the view is going away): the control
    /// being played, and holds still waiting out the minimum.
    pub fn flush(
        &mut self,
        doc: &mut crate::document::Document,
        engine: &mut ac_engine::EngineHandle,
    ) {
        self.cancel(doc);
        for r in std::mem::take(&mut self.releases) {
            crate::params::momentary(doc, engine, r.node, &r.socket, Value::Bool(false));
        }
    }

    /// Hold an input down in the engine, dropping a release of it that was
    /// still waiting (so it isn't let go while held again).
    fn hold_down(&mut self, io: &mut CanvasIo, node: NodeId, socket: &str) {
        self.releases
            .retain(|r| !(r.node == node && r.socket == socket));
        hold(io, node, socket, true);
    }

    /// Let go of an input held since `since`, but not before it has been
    /// down for [`MIN_HOLD`].
    fn let_go(&mut self, io: &mut CanvasIo, node: NodeId, socket: String, since: f64, now: f64) {
        if now - since >= MIN_HOLD {
            hold(io, node, &socket, false);
        } else {
            self.releases.push(PendingRelease {
                node,
                socket,
                at: since + MIN_HOLD,
            });
        }
    }
}

/// Held inputs (a Button's `pressed`, a Keyboard's keys, an XY Pad's
/// `touched`) go straight to the engine: holding a control down isn't a
/// project change, and playing a keyboard leaves no undo steps.
fn hold(io: &mut CanvasIo, id: NodeId, socket: &str, down: bool) {
    crate::params::momentary(io.doc, io.engine, id, socket, Value::Bool(down));
}

/// A click on a body.
pub fn click(io: &mut CanvasIo, n: &NodeView) -> Click {
    let node = n.id;
    let free = |socket: &str| {
        n.socket(socket, true)
            .filter(|s| !s.wired)
            .and_then(|s| s.spec.clone())
    };
    match n.body.map(|b| b.0) {
        Some(Body::Toggle) => {
            crate::params::fire(io.doc, io.engine, node, "flip");
            Click::Done
        }
        Some(Body::Clip) => {
            io.events.push(CanvasEvent::ChooseClipFile(node));
            Click::Done
        }
        // Pressing played them already.
        Some(Body::Button | Body::Keyboard | Body::XyPad) => Click::Done,
        Some(Body::Dropdown) if free("value").is_some() => Click::Choose,
        // The number itself is the field: click to type.
        Some(Body::Number) => match (free("value"), io.shown_node(node)) {
            (Some(spec), Some(inst)) => Click::Type {
                text: plain_number(widgets::number(&inst, &spec), &spec),
                spec,
            },
            _ => Click::Select,
        },
        _ => Click::Select,
    }
}

/// A double-click on a body: a Knob goes back to its default, a Slider
/// takes typing.
pub fn double_click(io: &mut CanvasIo, n: &NodeView, now: f64) -> Click {
    let node = n.id;
    match n.body.map(|b| b.0) {
        Some(Body::Knob) => {
            if let Some(s) = n.socket("value", true).filter(|s| !s.wired)
                && let Some(spec) = &s.spec
            {
                crate::params::set(io.doc, io.engine, node, "value", spec.default.clone(), now);
                io.doc.end_coalescing();
            }
            Click::Done
        }
        Some(Body::Slider) => {
            match (
                n.socket("value", true).and_then(|s| s.spec.clone()),
                io.shown_node(node),
            ) {
                (Some(spec), Some(inst)) => Click::Type {
                    text: plain_number(widgets::number(&inst, &spec), &spec),
                    spec,
                },
                _ => Click::Done,
            }
        }
        _ => Click::Done,
    }
}

/// Typed text for an input: set it (one undo step), or say why not.
pub fn commit_value(
    io: &mut CanvasIo,
    node: NodeId,
    socket: &str,
    spec: &SocketSpec,
    text: &str,
    now: f64,
) {
    match widgets::parse(text, spec) {
        Some(Value::Text(t)) => {
            crate::params::set_text(io.doc, node, socket, t, now);
            io.doc.end_coalescing();
        }
        Some(v) => {
            crate::params::set(io.doc, io.engine, node, socket, v, now);
            io.doc.end_coalescing();
        }
        None => io.events.push(CanvasEvent::Status(tr_args(
            "canvas-not-a-number",
            &[("text", text.trim().into())],
        ))),
    }
}

/// How a text field ended this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldResult {
    Editing,
    Cancel,
    Commit,
}

/// A text field over `rect` (screen) for typing a value, a label, or a
/// note. The first frame (`started` false) focuses it with everything
/// selected, so typing replaces the text. Enter or leaving the field
/// commits (a multiline field commits only on leaving); Escape cancels.
pub fn text_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    font: egui::FontId,
    text: &mut String,
    started: &mut bool,
    multiline: bool,
) -> FieldResult {
    let mut te = if multiline {
        egui::TextEdit::multiline(text)
    } else {
        egui::TextEdit::singleline(text)
    };
    te = te
        .id(id)
        .font(font)
        .margin(egui::Margin::symmetric(4, 1))
        .desired_width(rect.width());
    let out = ui.put(rect, te);
    if !*started {
        out.request_focus();
        // Select everything so typing replaces the value.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        *started = true;
        return FieldResult::Editing;
    }
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if escape {
        return FieldResult::Cancel;
    }
    let commit = if multiline {
        out.lost_focus()
    } else {
        enter || out.lost_focus()
    };
    if commit {
        FieldResult::Commit
    } else {
        FieldResult::Editing
    }
}

/// The choices of `n`'s input `socket` in a popup at `at` (screen). `age`:
/// frames it has been open (the opening click doesn't close it). Picking
/// one sets it as one undo step. Returns whether the popup should close.
#[allow(clippy::too_many_arguments)]
pub fn choice_popup(
    ctx: &egui::Context,
    id: egui::Id,
    io: &mut CanvasIo,
    n: &NodeView,
    socket: &str,
    at: Pos2,
    age: u32,
    now: f64,
) -> bool {
    let (items, current) = choice_items(io, n, socket);
    let mut close = false;
    let area = egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(140.0);
                if items.is_empty() {
                    close = true;
                }
                // A Dropdown can hold dozens of labels.
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (i, (label, value)) in items.iter().enumerate() {
                            if ui.selectable_label(current == Some(i), label).clicked() {
                                crate::params::set(
                                    io.doc,
                                    io.engine,
                                    n.id,
                                    socket,
                                    value.clone(),
                                    now,
                                );
                                io.doc.end_coalescing();
                                close = true;
                            }
                        }
                    });
            });
        });
    let clicked_outside =
        age > 0 && ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
    close || clicked_outside
}

/// What a choice popup offers for an input: each label with the value
/// picking it stores, and which one is chosen now. A Dropdown's `value`
/// lists the labels from its config and stores their index; other choice
/// inputs list their socket's named choices.
pub fn choice_items(
    io: &CanvasIo,
    n: &NodeView,
    socket: &str,
) -> (Vec<(String, Value)>, Option<usize>) {
    let spec = n.socket(socket, true).and_then(|s| s.spec.clone());
    let (Some(spec), Some(inst)) = (spec, io.shown_node(n.id)) else {
        return (Vec::new(), None);
    };
    let inst = &inst;
    let dropdown = n
        .body
        .is_some_and(|(b, _)| b == Body::Dropdown && b.drives(socket));
    if dropdown {
        let labels = ac_nodes::controls::dropdown::choices(&inst.config);
        let last = labels.len().saturating_sub(1) as f64;
        let current = widgets::number(inst, &spec).round().clamp(0.0, last) as usize;
        let items = labels
            .into_iter()
            .enumerate()
            .map(|(i, l)| (l, Value::Int(i as i64)))
            .collect();
        return (items, Some(current));
    }
    let chosen = widgets::choice(inst, &spec);
    let items = spec
        .choices
        .iter()
        .map(|c| (widgets::choice_label(&spec, c), Value::Text(c.to_string())))
        .collect();
    let current = spec.choices.iter().position(|c| *c == chosen);
    (items, current)
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

/// A Readout's number as it shows it: up to four decimals, no trailing
/// zeros, and a dash before the first reading.
pub fn readout_text(v: Option<f32>) -> String {
    match v {
        Some(v) if v.is_nan() => "NaN".to_string(),
        Some(v) if v.is_infinite() => {
            if v > 0.0 {
                "∞".into()
            } else {
                "-∞".into()
            }
        }
        Some(v) => {
            let s = format!("{v:.4}");
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            if s == "-0" { "0".into() } else { s }
        }
        None => "–".into(),
    }
}

/// A number as typed text, without its unit (units are accepted back).
pub fn plain_number(v: f64, spec: &SocketSpec) -> String {
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

// ---------------------------------------------------------------- painting

/// How a body looks this frame, from the view's own state.
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub zoom: f32,
    pub alpha: f32,
    pub hovered: bool,
    /// The pointer (screen), for the Keyboard's hovered key.
    pub pointer: Option<Pos2>,
    /// Inputs the view holds down (`Controls::held_inputs`).
    pub held: Vec<String>,
    /// The body's value is being dragged.
    pub dragging: bool,
    /// The body's value is being typed.
    pub editing: bool,
    /// The body's choices are open.
    pub choice_open: bool,
}

/// Paint `n`'s widget body `body` into `rect` (screen) from the document
/// and live telemetry. `state` keeps the meters' ballistics and notes
/// whether anything moved (keep repainting).
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &Painter,
    io: &CanvasIo,
    live: &Live,
    state: &mut LiveState,
    n: &NodeView,
    body: Body,
    rect: Rect,
    look: &Look,
    now: f64,
    dt: f32,
) {
    let z = look.zoom;
    let alpha = look.alpha;
    let hovered = look.hovered;
    let path = live.path(n.id);
    let held = |socket: &str| look.held.iter().any(|s| s == socket);
    // Bound inputs at the value their bindings last set (canvas and panels
    // alike).
    let shown = io.shown_node(n.id);
    match body {
        Body::Button => {
            let down = held("pressed")
                || live
                    .tele
                    .socket_value(&path, "gate")
                    .is_some_and(|v| v > 0.5);
            bodies::button(painter, rect, z, down, hovered, alpha);
        }
        Body::Toggle => {
            let on = live
                .tele
                .socket_value(&path, "out")
                .map(|v| v > 0.5)
                .or_else(|| {
                    shown
                        .as_ref()
                        .and_then(|x| x.params.get("value"))
                        .and_then(|v| v.as_bool())
                })
                .unwrap_or(false);
            bodies::toggle(painter, rect, z, on, hovered, alpha);
        }
        Body::Slider => {
            let spec = n.socket("value", true).and_then(|s| s.spec.clone());
            let node = shown.as_ref();
            if let (Some(spec), Some(node)) = (spec, node) {
                let wired = n.socket("value", true).is_some_and(|s| s.wired);
                let v = if wired {
                    live.tele.socket_value(&path, "out").map_or(0.0, f64::from)
                } else {
                    widgets::number(node, &spec)
                };
                bodies::slider(
                    painter,
                    rect,
                    z,
                    widgets::normalized(v, &spec).unwrap_or(0.0),
                    &widgets::format(v, &spec),
                    hovered,
                    look.dragging,
                    alpha,
                );
            }
        }
        Body::Knob => {
            let input = n.socket("value", true);
            let spec = input.and_then(|s| s.spec.clone());
            let node = shown.as_ref();
            if let (Some(spec), Some(node)) = (spec, node) {
                let wired = input.is_some_and(|s| s.wired);
                let v = if wired {
                    live.tele.socket_value(&path, "out").map_or(0.0, f64::from)
                } else {
                    widgets::number(node, &spec)
                };
                bodies::knob(
                    painter,
                    rect,
                    z,
                    widgets::normalized(v, &spec).unwrap_or(0.0),
                    widgets::zero_point(&spec),
                    &widgets::format(v, &spec),
                    hovered,
                    look.dragging,
                    alpha,
                );
            }
        }
        Body::XyPad => {
            let node = shown.as_ref();
            // Each axis from its input, or the live output when wired.
            let axis = |name: &str| -> f32 {
                let Some(s) = n.socket(name, true) else {
                    return 0.5;
                };
                let Some(spec) = &s.spec else { return 0.5 };
                let v = if s.wired {
                    live.tele.socket_value(&path, name).map(f64::from)
                } else {
                    node.map(|x| widgets::number(x, spec))
                };
                v.and_then(|v| widgets::normalized(v, spec)).unwrap_or(0.5)
            };
            let pos = (axis("x"), axis("y"));
            let touched = held("touched")
                || live
                    .tele
                    .socket_value(&path, "gate")
                    .is_some_and(|v| v > 0.5);
            bodies::xy_pad(painter, rect, z, pos, touched, hovered, alpha);
        }
        Body::Keyboard => {
            if let Some(node) = shown.as_ref() {
                let (low, count) = piano::layout(io.registry, node);
                let keys = piano::keys(low, count, rect);
                // Keys played here, and keys a project holds down.
                let latched = node
                    .params
                    .iter()
                    .filter(|(_, v)| v.as_bool() == Some(true))
                    .map(|(s, _)| s.clone());
                let down: Vec<usize> = look
                    .held
                    .iter()
                    .cloned()
                    .chain(latched)
                    .filter_map(|s| piano::key_index(&s))
                    .collect();
                let over = match (hovered, look.pointer) {
                    (true, Some(p)) => piano::key_at(&keys, p),
                    _ => None,
                };
                // Name the keys for the octave they play in.
                let octave = n
                    .socket("octave", true)
                    .filter(|s| !s.wired)
                    .and_then(|s| s.spec.as_ref())
                    .map_or(0, |spec| widgets::number(node, spec).round() as i32)
                    .clamp(-3, 3);
                bodies::keyboard(painter, z, &keys, &down, over, 12 * octave, alpha);
            }
        }
        Body::Number => {
            let input = n.socket("value", true);
            let spec = input.and_then(|s| s.spec.clone());
            let node = shown.as_ref();
            if let (Some(spec), Some(node)) = (spec, node) {
                let wired = input.is_some_and(|s| s.wired);
                let active = look.editing || look.dragging;
                let text = if look.editing {
                    String::new()
                } else if wired {
                    live.tele
                        .socket_value(&path, "out")
                        .map_or("–".into(), |v| {
                            widgets::live_text(v, spec.ty, Some(&spec))
                        })
                } else {
                    widgets::format(widgets::bounded_number(node, &spec), &spec)
                };
                bodies::number(painter, rect, z, &text, hovered && !wired, active, alpha);
            }
        }
        Body::Dropdown => {
            if let Some(node) = shown.as_ref() {
                let labels = ac_nodes::controls::dropdown::choices(&node.config);
                let input = n.socket("value", true);
                let wired = input.is_some_and(|s| s.wired);
                let index = if wired {
                    live.tele
                        .socket_value(&path, "out")
                        .map_or(0, |v| v.round() as i64)
                } else {
                    input
                        .and_then(|s| s.spec.as_ref())
                        .map_or(0.0, |spec| widgets::number(node, spec))
                        .round() as i64
                };
                let label = labels
                    .get(index.min(labels.len() as i64 - 1).max(0) as usize)
                    .map_or("", |s| s.as_str());
                bodies::dropdown(
                    painter,
                    rect,
                    z,
                    label,
                    hovered && !wired,
                    look.choice_open,
                    alpha,
                );
            }
        }
        Body::Meter => {
            let r = |k| live.node_value(n.id, k).unwrap_or(0.0);
            let ch = state.meter(&path, [(r(0), r(1)), (r(2), r(3))], now, dt);
            bodies::meter(painter, rect, z, ch, alpha);
        }
        Body::Scope => {
            let ring = live.scope(n.id);
            let trig = live.node_value(n.id, 0).filter(|v| *v >= 0.0);
            let start = match (trig, live.tele.scope_position(&path), ring.as_ref()) {
                (Some(tp), Some(wp), Some(r)) if !r.is_empty() => {
                    let len = r.len();
                    Some((tp as usize + len - wp % len) % len)
                }
                _ => None,
            };
            if bodies::scope(painter, rect, z, ring.as_deref(), start, alpha) {
                state.active = true;
            }
        }
        Body::Analyzer => {
            let bands: Vec<f32> = (0..ac_nodes::display::analyzer::TELEMETRY_BANDS)
                .map(|k| live.node_value(n.id, k).unwrap_or(f32::NEG_INFINITY))
                .collect();
            let rate = io.engine.options().sample_rate;
            if bodies::analyzer(painter, rect, z, &bands, rate, alpha) {
                state.active = true;
            }
        }
        Body::Spectrogram => {
            let ring = live.scope(n.id);
            let rate = io.engine.options().sample_rate;
            let bands = ac_nodes::display::analyzer::TELEMETRY_BANDS;
            if bodies::spectrogram(painter, rect, z, ring.as_deref(), bands, rate, alpha) {
                state.active = true;
            }
        }
        Body::Readout => {
            let text = readout_text(live.node_value(n.id, 0));
            bodies::readout(painter, rect, z, &text, alpha);
        }
        Body::Clip => {
            let file = io
                .doc
                .graph()
                .node(n.id)
                .map(|x| x.config.get_str_or("file", "").to_string())
                .unwrap_or_default();
            let missing = !file.is_empty()
                && (io.missing_assets.contains(&file)
                    || live.node_value(n.id, 0).is_some_and(|v| v >= 0.5));
            bodies::clip(painter, rect, z, &file, missing, hovered, alpha);
        }
        Body::Record => {
            let rec = live.node_value(n.id, 0).unwrap_or(0.0) > 0.5;
            if rec {
                state.active = true;
            }
            if io.can_record {
                bodies::record(
                    painter,
                    rect,
                    z,
                    rec,
                    live.node_value(n.id, 1).unwrap_or(0.0),
                    live.node_value(n.id, 2).unwrap_or(0.0),
                    now,
                    alpha,
                );
            } else {
                bodies::readout(painter, rect, z, "", alpha);
                if z >= 0.4 {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        tr("canvas-save-to-record"),
                        theme::canvas_font(theme::FONT_SMALL, z, false),
                        theme::fade(theme::text_muted(), alpha),
                    );
                }
            }
        }
        Body::AudioOut => {
            let r = |k| live.node_value(n.id, k).unwrap_or(0.0);
            let ch: [MeterChannel; 2] = state.meter(&path, [(r(0), r(0)), (r(1), r(1))], now, dt);
            bodies::audio_out(painter, rect, z, ch, alpha);
        }
    }
}
