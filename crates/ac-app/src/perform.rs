//! Performance panels (docs/UX.md "Panels"): the current panel's pinned
//! nodes as a grid of their live widgets, without the graph, and
//! full-screen performance mode.
//!
//! The widgets are the canvas's own bodies, painted and played through
//! [`crate::canvas::controls`], so a slider moved here is the same edit as
//! on the canvas and a button held here is the same momentary input.
//! Panels pin top-level nodes only, so the view runs with the document at
//! the root ([`crate::document::Document::at_root`]).

use ac_core::{NodeId, Panel, Registry, SocketSpec};
use ac_engine::EngineHandle;
use egui::{Align2, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::actions::Action;
use crate::audio::EngineHost;
use crate::canvas::CanvasIo;
use crate::canvas::controls::{self, Click, ControlGesture, Controls, FieldResult, Grab, Look};
use crate::canvas::live::{Live, LiveState, MeterChannel};
use crate::canvas::piano;
use crate::canvas::scene::{self, Body, NodeView};
use crate::document::Document;
use crate::edit::pins;
use crate::{icons, theme};

/// Narrowest grid cell (points); the grid fits as many columns as the
/// width takes. Performance mode uses bigger cells, for touch.
const CELL_WIDTH: f32 = 150.0;
const CELL_WIDTH_FULL: f32 = 180.0;
/// Height of one grid row at the narrowest cell width.
const CELL_HEIGHT: f32 = 118.0;
const GAP: f32 = 12.0;
const MARGIN: f32 = 16.0;
const HEADER: f32 = 24.0;
const PAD: f32 = 10.0;
/// Height of the tab strip above the grid.
const TABS: f32 = 38.0;
/// Size of a card's header buttons.
const BUTTON: f32 = 18.0;

/// What the canvas (its right-click menu) or `P` asks of the panels.
#[derive(Clone, Debug, PartialEq)]
pub enum PinRequest {
    /// Pin nodes to a panel (`None`: a new one).
    Pin {
        nodes: Vec<NodeId>,
        panel: Option<usize>,
    },
    /// Unpin nodes from a panel (`None`: from every panel).
    Unpin {
        nodes: Vec<NodeId>,
        panel: Option<usize>,
    },
}

/// One card drawn last frame (screen coordinates).
#[derive(Clone, Debug)]
pub struct Cell {
    pub node: NodeId,
    /// The item's index in the panel.
    pub item: usize,
    pub rect: Rect,
    pub header: Rect,
    /// The widget body, when the node has one.
    pub body: Option<(Body, Rect)>,
    /// Shrink, grow, and unpin buttons (none in performance mode).
    pub buttons: Option<[Rect; 3]>,
    /// Zoom the widget is drawn at.
    pub zoom: f32,
}

/// What the pointer is over in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelHit {
    None,
    Body(usize),
    /// Anywhere else on a card: drag to reorder.
    Card(usize),
    Shrink(usize),
    Grow(usize),
    Unpin(usize),
}

impl PanelHit {
    fn cell(self) -> Option<usize> {
        match self {
            PanelHit::None => None,
            PanelHit::Body(i)
            | PanelHit::Card(i)
            | PanelHit::Shrink(i)
            | PanelHit::Grow(i)
            | PanelHit::Unpin(i) => Some(i),
        }
    }
}

/// Typing a value into a pinned Number or Slider.
struct ValueEdit {
    node: NodeId,
    socket: String,
    spec: SocketSpec,
    text: String,
    rect: Rect,
    started: bool,
}

/// Renaming a panel's tab.
struct Rename {
    panel: usize,
    text: String,
    started: bool,
}

/// The panel view's own state (the panels themselves live in the project).
#[derive(Default)]
pub struct PerformView {
    /// The panels are shown instead of the canvas.
    pub shown: bool,
    /// The panel shown.
    pub current: usize,
    /// Full-screen performance mode.
    pub fullscreen: bool,
    /// The window has reported being full screen since we asked.
    seen_fullscreen: bool,
    /// Widgets being played.
    pub controls: Controls,
    live: LiveState,
    press: Option<PanelHit>,
    /// The card being dragged to a new place.
    reorder: Option<usize>,
    choice: Option<(NodeId, Pos2)>,
    choice_age: u32,
    edit: Option<ValueEdit>,
    rename: Option<Rename>,
    scroll: f32,
    /// The cards drawn last frame (tests read these).
    pub cells: Vec<Cell>,
}

impl PerformView {
    /// Is a text field taking the keyboard?
    pub fn is_typing(&self) -> bool {
        self.edit.is_some() || self.rename.is_some()
    }

    /// A new document: start from its first panel with nothing held.
    pub fn reset(&mut self) {
        self.current = 0;
        self.controls = Controls::default();
        self.press = None;
        self.reorder = None;
        self.choice = None;
        self.edit = None;
        self.rename = None;
        self.scroll = 0.0;
        self.cells.clear();
    }

    /// Escape: close a popup or a field, or let go of a control. Returns
    /// whether there was something to cancel.
    pub fn cancel(&mut self, doc: &mut Document) -> bool {
        if self.choice.take().is_some() || self.edit.take().is_some() {
            return true;
        }
        if self.rename.take().is_some() || self.reorder.take().is_some() {
            return true;
        }
        self.controls.cancel(doc)
    }

    /// The view is going away: let go of everything held, now.
    pub fn let_go(&mut self, doc: &mut Document, engine: &mut EngineHandle) {
        self.choice = None;
        self.edit = None;
        self.reorder = None;
        doc.at_root(|doc| self.controls.flush(doc, engine));
    }

    /// Ask the window to enter or leave full screen.
    pub fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        if self.fullscreen == on {
            return;
        }
        self.fullscreen = on;
        self.seen_fullscreen = false;
        self.rename = None;
        if on {
            self.shown = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    /// Follow the window: leaving full screen some other way (the window's
    /// own controls) ends performance mode.
    pub fn follow_window(&mut self, ctx: &egui::Context) {
        if !self.fullscreen {
            return;
        }
        match ctx.input(|i| i.viewport().fullscreen) {
            Some(true) => self.seen_fullscreen = true,
            Some(false) if self.seen_fullscreen => {
                self.fullscreen = false;
                self.seen_fullscreen = false;
            }
            _ => {}
        }
    }

    /// The screen rectangle of a pinned node's widget last frame (tests).
    pub fn body_rect(&self, node: NodeId) -> Option<Rect> {
        self.cells
            .iter()
            .find(|c| c.node == node)
            .and_then(|c| c.body.map(|b| b.1))
    }

    /// A Meter's (or Audio Out's) displayed channels, with ballistics
    /// (tests).
    pub fn meter_reading(&self, node: NodeId) -> Option<[MeterChannel; 2]> {
        self.live.meter_reading(&ac_core::NodePath::root(node))
    }

    // ------------------------------------------------------------ drawing

    /// The panel view: tabs, then the current panel's grid. `io` must see
    /// the document at the root.
    pub fn show(&mut self, ui: &mut egui::Ui, io: &mut CanvasIo, now: f64) {
        let n = io.doc.project.panels.len();
        self.current = self.current.min(n.saturating_sub(1));
        let full = ui.available_rect_before_wrap();
        let mut grid = full;
        if !self.fullscreen {
            let strip = Rect::from_min_size(full.min, vec2(full.width(), TABS));
            ui.painter().rect_filled(strip, 0.0, theme::PANEL_BG);
            ui.painter().line_segment(
                [strip.left_bottom(), strip.right_bottom()],
                Stroke::new(1.0, theme::BORDER),
            );
            let inner = strip.shrink2(vec2(10.0, 4.0));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| self.tabs(ui, io),
            );
            grid.min.y = strip.max.y;
        }
        self.grid(ui, io, grid, now);
    }

    /// The tab strip: one tab per panel (click to show, double-click to
    /// rename, right-click for more), a button for a new panel, and the
    /// way into performance mode.
    fn tabs(&mut self, ui: &mut egui::Ui, io: &mut CanvasIo) {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = io.doc.project.panels.len();
        for i in 0..n {
            // A panel deleted from its menu this frame shortens the list.
            let Some(name) = io.doc.project.panels.get(i).map(|p| p.name.clone()) else {
                break;
            };
            if self.rename.as_ref().is_some_and(|r| r.panel == i) {
                if let Some(mut r) = self.rename.take() {
                    let (rect, _) = ui.allocate_exact_size(vec2(140.0, 24.0), Sense::hover());
                    let id = egui::Id::unique(("panel-rename", i));
                    let font = theme::ui_font(theme::FONT_UI);
                    let (text, started) = (&mut r.text, &mut r.started);
                    match controls::text_field(ui, id, rect, font, text, started, false) {
                        FieldResult::Editing => self.rename = Some(r),
                        FieldResult::Cancel => {}
                        FieldResult::Commit => {
                            let text = r.text.clone();
                            io.doc.edit_panels("Rename panel", |p, _| {
                                pins::rename_panel(p, i, &text);
                            });
                        }
                    }
                }
                continue;
            }
            let selected = i == self.current;
            let text = RichText::new(&name)
                .size(theme::FONT_UI)
                .color(if selected {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT_MUTED
                });
            let tab = ui
                .add(
                    egui::Button::new(text)
                        .selected(selected)
                        .frame_when_inactive(false)
                        .min_size(vec2(0.0, 26.0)),
                )
                .on_hover_text("Double-click to rename · right-click for more");
            if tab.clicked() {
                self.current = i;
            }
            if tab.double_clicked() {
                self.start_rename(i, &name);
            }
            tab.context_menu(|ui| {
                if ui.button(format!("{}  Rename", icons::PENCIL)).clicked() {
                    self.start_rename(i, &name);
                    ui.close();
                }
                let left = ui.add_enabled(
                    i > 0,
                    egui::Button::new(format!("{}  Move Left", icons::ARROW_LEFT)),
                );
                if left.clicked() {
                    self.move_panel(io, i, i - 1);
                    ui.close();
                }
                let right = ui.add_enabled(
                    i + 1 < n,
                    egui::Button::new(format!("{}  Move Right", icons::CARET_RIGHT)),
                );
                if right.clicked() {
                    self.move_panel(io, i, i + 1);
                    ui.close();
                }
                ui.separator();
                if ui
                    .button(format!("{}  Delete Panel", icons::TRASH))
                    .on_hover_text("The nodes stay in the patch; Ctrl+Z brings the panel back")
                    .clicked()
                {
                    io.doc.edit_panels("Delete panel", |p, _| {
                        pins::remove_panel(p, i);
                    });
                    if self.current >= i && self.current > 0 {
                        self.current -= 1;
                    }
                    self.rename = None;
                    ui.close();
                }
            });
        }
        let add = ui
            .add(
                egui::Button::new(RichText::new(icons::PLUS).size(15.0)).frame_when_inactive(false),
            )
            .on_hover_text("New panel");
        if add.clicked() {
            let (i, name) = io.doc.edit_panels("New panel", |p, _| {
                let i = pins::add_panel(p, "");
                (i, p.get(i).map_or(String::new(), |x| x.name.clone()))
            });
            self.current = i;
            self.start_rename(i, &name);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let perform = ui
                .add(egui::Button::new(format!(
                    "{}  Perform",
                    icons::CORNERS_OUT
                )))
                .on_hover_text("Show this panel full screen (F11; Esc leaves)");
            if perform.clicked() {
                self.set_fullscreen(ui.ctx(), true);
            }
            ui.label(
                RichText::new("P on the canvas pins the selected controls · Ctrl+Tab: canvas")
                    .small()
                    .color(theme::TEXT_FAINT),
            );
        });
    }

    fn start_rename(&mut self, panel: usize, name: &str) {
        self.rename = Some(Rename {
            panel,
            text: name.to_string(),
            started: false,
        });
    }

    fn move_panel(&mut self, io: &mut CanvasIo, from: usize, to: usize) {
        let moved = io
            .doc
            .edit_panels("Move panel", |p, _| pins::move_panel(p, from, to));
        if moved && self.current == from {
            self.current = to;
        } else if moved && self.current == to {
            self.current = from;
        }
    }

    /// The minimal transport of performance mode: play/stop, the
    /// position, the panels to switch between, and the way out.
    pub fn performance_bar(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        engine: &EngineHost,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let playing = engine.handle.is_playing();
        let rate = f64::from(engine.info.sample_rate);
        let tempo = ac_core::TempoMap::new(rate, &doc.project.settings.tempo);
        let (bars, time) = crate::transport::position_text(&tempo, engine.handle.position());
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let icon = if playing { icons::STOP } else { icons::PLAY };
            let play = ui
                .add(
                    egui::Button::new(
                        RichText::new(icon)
                            .family(theme::icons_fill_family())
                            .size(22.0)
                            .color(if playing { theme::PLAY } else { theme::TEXT }),
                    )
                    .selected(playing)
                    .min_size(vec2(52.0, 36.0)),
                )
                .on_hover_text(if playing {
                    "Stop (Space)"
                } else {
                    "Play (Space)"
                });
            if play.clicked() {
                actions.push(Action::PlayStop);
            }
            ui.label(
                RichText::new(bars)
                    .font(theme::mono_font(20.0))
                    .color(theme::TEXT_STRONG),
            );
            ui.label(
                RichText::new(time)
                    .font(theme::mono_font(theme::FONT_UI))
                    .color(theme::TEXT_MUTED),
            );
            if doc.project.panels.len() > 1 {
                ui.add_space(12.0);
                for (i, p) in doc.project.panels.iter().enumerate() {
                    let selected = i == self.current;
                    let b = egui::Button::new(RichText::new(&p.name).size(theme::FONT_UI))
                        .selected(selected)
                        .frame_when_inactive(false)
                        .min_size(vec2(0.0, 32.0));
                    if ui.add(b).clicked() {
                        self.current = i;
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let exit = ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("{}  Exit", icons::CORNERS_IN))
                                .size(theme::FONT_UI),
                        )
                        .min_size(vec2(0.0, 32.0)),
                    )
                    .on_hover_text("Leave performance mode (Esc or F11)");
                if exit.clicked() {
                    actions.push(Action::PerformanceMode);
                }
            });
        });
        actions
    }

    /// The grid of the current panel's cards, played with the pointer.
    fn grid(&mut self, ui: &mut egui::Ui, io: &mut CanvasIo, rect: Rect, now: f64) {
        let ctx = ui.ctx().clone();
        let response = ui.interact(
            rect,
            egui::Id::unique("panel-grid"),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::CANVAS_BG);
        let dt = self.live.begin_frame(now);
        if self.controls.release_due(io, now) {
            ctx.request_repaint();
        }
        let Some(panel) = io.doc.project.panels.get(self.current).cloned() else {
            self.cells.clear();
            empty_state(
                &painter,
                rect,
                "No panels yet",
                "Select sliders, buttons, meters, or other controls on the canvas and press P \
                 to pin them to a panel.",
            );
            return;
        };
        // Cards play widgets the canvas may show collapsed.
        let scene = scene::build_expanded(io.doc, io.registry);
        if response.hovered() {
            self.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        let (cells, height) = self.layout(&panel, &scene, io.registry, io.doc, rect);
        self.scroll = self.scroll.clamp(0.0, (height - rect.height()).max(0.0));
        self.cells = cells;
        if self.cells.is_empty() {
            empty_state(
                &painter,
                rect,
                &format!("Nothing pinned to {} yet", panel.name),
                "Select controls or displays on the canvas (sliders, knobs, buttons, the \
                 keyboard, meters, scopes) and press P, or right-click one and pin it here.",
            );
        }

        let (pressed, released, modifiers, origin) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.modifiers,
                i.pointer.press_origin(),
            )
        });
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hover = match pointer {
            Some(p) if rect.contains(p) && (response.hovered() || !self.idle()) => self.hit(p),
            _ => PanelHit::None,
        };

        if pressed && response.hovered() {
            self.press = Some(hover);
            if let (PanelHit::Body(i), Some(p)) = (hover, pointer)
                && let Some((n, r)) = cell_view(&self.cells, &scene, i)
            {
                self.controls.press(io, n, r, p, now);
            }
        }
        if released {
            self.controls.release(io, now);
        } else if let (Some(node), Some(p)) = (self.controls.following(), pointer)
            && let Some(i) = self.cells.iter().position(|c| c.node == node)
            && let Some((n, r)) = cell_view(&self.cells, &scene, i)
        {
            self.controls.follow(io, n, r, p, now);
        }
        if response.drag_started_by(egui::PointerButton::Primary) && self.idle() {
            match self.press.unwrap_or(hover) {
                PanelHit::Body(i) => {
                    if let (Some((n, r)), Some(o)) =
                        (cell_view(&self.cells, &scene, i), origin.or(pointer))
                        && self.controls.begin_drag(io, n, r, o, now) == Grab::Other
                    {
                        self.reorder = Some(i);
                    }
                }
                PanelHit::Card(i) => self.reorder = Some(i),
                _ => {}
            }
        }
        if response.dragged()
            && let Some(p) = pointer
        {
            self.controls.drag(io, p, modifiers, now);
        }
        if response.drag_stopped() {
            self.controls.end_drag(io);
            if let Some(from) = self.reorder.take()
                && let Some(to) = hover.cell()
                && let (Some(a), Some(b)) = (self.cells.get(from), self.cells.get(to))
            {
                let (a, b) = (a.item, b.item);
                let panel = self.current;
                io.doc
                    .edit_panels("Move on panel", |p, _| pins::move_item(p, panel, a, b));
            }
        }
        if response.clicked()
            && !matches!(
                self.controls.active,
                Some(ControlGesture::HoldButton { .. })
            )
        {
            self.click(io, &scene, self.press.unwrap_or(hover));
        }
        if response.double_clicked()
            && let PanelHit::Body(i) = self.press.unwrap_or(hover)
            && let Some((n, r)) = cell_view(&self.cells, &scene, i)
            && let Click::Type { spec, text } = controls::double_click(io, n, now)
        {
            let z = self.cells.get(i).map_or(1.0, |c| c.zoom);
            self.edit = Some(ValueEdit {
                node: n.id,
                socket: "value".into(),
                spec,
                text,
                rect: Rect::from_min_size(r.min, vec2(r.width(), 20.0 * z)),
                started: false,
            });
        }
        if released {
            self.press = None;
        }

        // Paint.
        let tele = io.engine.telemetry();
        let live = Live::new(&tele, io.doc);
        let target = self.reorder.and(hover.cell());
        for (i, c) in self.cells.iter().enumerate() {
            let n = scene.node(c.node);
            let here = if hover.cell() == Some(i) {
                hover
            } else {
                PanelHit::None
            };
            paint_card(
                &painter,
                c,
                n,
                here,
                self.reorder == Some(i),
                target == Some(i),
            );
            let (Some((body, br)), Some(n)) = (c.body, n) else {
                continue;
            };
            let look = Look {
                zoom: c.zoom,
                alpha: if n.muted { 0.5 } else { 1.0 },
                hovered: here == PanelHit::Body(i),
                pointer,
                held: self.controls.held_inputs(c.node),
                dragging: self.controls.dragging(c.node, None),
                editing: self.edit.as_ref().is_some_and(|e| e.node == c.node),
                choice_open: self.choice.is_some_and(|(node, _)| node == c.node),
            };
            controls::paint(
                &painter,
                io,
                &live,
                &mut self.live,
                n,
                body,
                br,
                &look,
                now,
                dt,
            );
        }
        if let Some(from) = self.reorder
            && let (Some(c), Some(p)) = (self.cells.get(from), pointer)
        {
            // The card being moved follows the pointer as an outline.
            let ghost = Rect::from_center_size(p, c.rect.size() * 0.5);
            painter.rect_stroke(
                ghost,
                theme::radius(theme::NODE_RADIUS),
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Inside,
            );
        }

        // Popups and fields over the grid.
        if let Some((node, at)) = self.choice {
            let close = match scene.node(node) {
                Some(n) => controls::choice_popup(
                    &ctx,
                    egui::Id::unique("panel-choice"),
                    io,
                    n,
                    "value",
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
        if let Some(mut e) = self.edit.take() {
            let font = theme::mono_font(theme::FONT_VALUE + 2.0);
            let id = egui::Id::unique("panel-value-edit");
            match controls::text_field(ui, id, e.rect, font, &mut e.text, &mut e.started, false) {
                FieldResult::Editing => self.edit = Some(e),
                FieldResult::Cancel => {}
                FieldResult::Commit => {
                    controls::commit_value(io, e.node, &e.socket, &e.spec, &e.text, now);
                }
            }
        }
        if self.live.active || !self.idle() || io.engine.is_playing() {
            ctx.request_repaint();
        }
    }

    fn idle(&self) -> bool {
        !self.controls.is_active() && self.reorder.is_none()
    }

    /// A click on the grid.
    fn click(&mut self, io: &mut CanvasIo, scene: &scene::Scene, hit: PanelHit) {
        let panel = self.current;
        let Some(c) = hit.cell().and_then(|i| self.cells.get(i)).cloned() else {
            return;
        };
        match hit {
            PanelHit::Body(_) => {
                let Some(n) = scene.node(c.node) else { return };
                let Some((_, r)) = c.body else { return };
                match controls::click(io, n) {
                    Click::Done | Click::Select => {}
                    Click::Choose => self.choice = Some((n.id, r.left_bottom())),
                    Click::Type { spec, text } => {
                        self.edit = Some(ValueEdit {
                            node: n.id,
                            socket: "value".into(),
                            spec,
                            text,
                            rect: r.shrink2(vec2(6.0, 6.0)),
                            started: false,
                        });
                    }
                }
            }
            PanelHit::Unpin(_) => {
                let k = c.item;
                io.doc.edit_panels("Unpin", |p, _| {
                    if let Some(pl) = p.get_mut(panel)
                        && k < pl.items.len()
                    {
                        pl.items.remove(k);
                    }
                });
            }
            PanelHit::Grow(_) | PanelHit::Shrink(_) => {
                let grow = matches!(hit, PanelHit::Grow(_));
                let wide_only = c.body.is_some_and(|b| grows_wide(b.0));
                let k = c.item;
                let natural = io
                    .doc
                    .project
                    .graph
                    .node(c.node)
                    .map_or([1, 1], |inst| natural_span(io.registry, inst));
                io.doc.edit_panels("Resize on panel", |p, _| {
                    let Some(it) = p.get(panel).and_then(|pl| pl.items.get(k)) else {
                        return;
                    };
                    let [cols, rows] = it.span(natural);
                    let next = if grow {
                        [cols + 1, if wide_only { rows } else { rows + 1 }]
                    } else {
                        let less = |v: u32| v.saturating_sub(1).max(1);
                        [less(cols), if wide_only { rows } else { less(rows) }]
                    };
                    let size = (next != natural).then_some(next);
                    pins::resize_item(p, panel, k, size);
                });
            }
            PanelHit::Card(_) | PanelHit::None => {}
        }
    }

    /// What's under screen point `p`, topmost first.
    fn hit(&self, p: Pos2) -> PanelHit {
        for (i, c) in self.cells.iter().enumerate().rev() {
            if !c.rect.contains(p) {
                continue;
            }
            if let Some([shrink, grow, unpin]) = c.buttons {
                if shrink.expand(2.0).contains(p) {
                    return PanelHit::Shrink(i);
                }
                if grow.expand(2.0).contains(p) {
                    return PanelHit::Grow(i);
                }
                if unpin.expand(2.0).contains(p) {
                    return PanelHit::Unpin(i);
                }
            }
            if let Some((_, b)) = c.body
                && b.contains(p)
            {
                return PanelHit::Body(i);
            }
            return PanelHit::Card(i);
        }
        PanelHit::None
    }

    /// Where each pinned node's card goes in `rect`, and the height of the
    /// whole grid.
    fn layout(
        &self,
        panel: &Panel,
        scene: &scene::Scene,
        registry: &Registry,
        doc: &Document,
        rect: Rect,
    ) -> (Vec<Cell>, f32) {
        let min_w = if self.fullscreen {
            CELL_WIDTH_FULL
        } else {
            CELL_WIDTH
        };
        let avail = (rect.width() - 2.0 * MARGIN).max(min_w);
        let columns = (((avail + GAP) / (min_w + GAP)).floor() as u32).max(1);
        let uw = (avail - GAP * (columns - 1) as f32) / columns as f32;
        let scale = (uw / CELL_WIDTH).clamp(1.0, 1.5);
        let uh = CELL_HEIGHT * scale;
        let items: Vec<(usize, NodeId, [u32; 2])> = panel
            .items
            .iter()
            .enumerate()
            .filter_map(|(k, it)| {
                let inst = doc.project.graph.node(it.node)?;
                let span = it.span(natural_span(registry, inst));
                Some((k, it.node, [span[0].min(columns), span[1]]))
            })
            .collect();
        let spans: Vec<[u32; 2]> = items.iter().map(|x| x.2).collect();
        let places = place(&spans, columns);
        let mut cells = Vec::new();
        let mut bottom: f32 = 0.0;
        let origin = rect.min + vec2(MARGIN, MARGIN - self.scroll);
        for ((k, node, [c, r]), (col, row)) in items.into_iter().zip(places) {
            let min = origin + vec2(col as f32 * (uw + GAP), row as f32 * (uh + GAP));
            let size = vec2(
                c as f32 * uw + (c - 1) as f32 * GAP,
                r as f32 * uh + (r - 1) as f32 * GAP,
            );
            let card = Rect::from_min_size(min, size);
            bottom = bottom.max((row + r) as f32 * (uh + GAP));
            let header = Rect::from_min_size(card.min, vec2(card.width(), HEADER * scale));
            let content = Rect::from_min_max(
                pos2(card.min.x + PAD, header.max.y + 2.0),
                card.max - vec2(PAD, PAD),
            );
            let zoom = scale;
            let body = scene
                .node(node)
                .and_then(|n| n.body)
                .map(|(b, _)| (b, fit_body(b, content, zoom)));
            let buttons = (!self.fullscreen).then(|| {
                let y = header.center().y;
                let at = |k: f32| {
                    Rect::from_center_size(
                        pos2(header.max.x - 6.0 - BUTTON * (k + 0.5) - 2.0 * k, y),
                        Vec2::splat(BUTTON),
                    )
                };
                [at(2.0), at(1.0), at(0.0)]
            });
            cells.push(Cell {
                node,
                item: k,
                rect: card,
                header,
                body,
                buttons,
                zoom,
            });
        }
        (cells, bottom + 2.0 * MARGIN)
    }
}

/// Bodies that grow sideways only: they read along a line.
fn grows_wide(b: Body) -> bool {
    matches!(
        b,
        Body::Slider
            | Body::Meter
            | Body::Toggle
            | Body::Number
            | Body::Dropdown
            | Body::Readout
            | Body::Clip
            | Body::Record
            | Body::AudioOut
    )
}

/// The cells a node's widget takes by default, `[columns, rows]`: bigger
/// widgets (the Keyboard, Scope, Analyzer, XY Pad) get bigger cells.
pub fn natural_span(registry: &Registry, node: &ac_core::NodeInstance) -> [u32; 2] {
    match Body::of(&node.kind) {
        Some(Body::Keyboard) => {
            let (low, keys) = piano::layout(registry, node);
            let white = piano::width_in_white_keys(low, keys);
            // Room for keys a finger can hit: about five white keys a cell.
            [
                ((white / 5.0).round() as u32).clamp(3, ac_core::panel::MAX_SPAN),
                1,
            ]
        }
        Some(Body::XyPad) => [2, 2],
        Some(Body::Scope | Body::Analyzer) => [3, 2],
        Some(Body::Slider | Body::Meter | Body::Clip | Body::Record | Body::AudioOut) => [2, 1],
        Some(_) | None => [1, 1],
    }
}

/// Place items of the given spans on a grid `columns` wide, in order,
/// each at the first free place at or after the previous one, row by row.
/// Returns each item's `(column, row)`.
pub fn place(spans: &[[u32; 2]], columns: u32) -> Vec<(u32, u32)> {
    let columns = columns.max(1);
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut cursor = (0u32, 0u32);
    let mut out = Vec::with_capacity(spans.len());
    for &[c, r] in spans {
        let (c, r) = (c.clamp(1, columns), r.max(1));
        let free = |taken: &Vec<Vec<bool>>, col: u32, row: u32| {
            (row..row + r).all(|y| {
                (col..col + c).all(|x| taken.get(y as usize).is_none_or(|line| !line[x as usize]))
            })
        };
        let (mut col, mut row) = cursor;
        loop {
            if col + c > columns {
                col = 0;
                row += 1;
                continue;
            }
            if free(&taken, col, row) {
                break;
            }
            col += 1;
        }
        for y in row..row + r {
            while taken.len() <= y as usize {
                taken.push(vec![false; columns as usize]);
            }
            for x in col..col + c {
                taken[y as usize][x as usize] = true;
            }
        }
        out.push((col, row));
        cursor = (col + c, row);
    }
    out
}

/// Where a body sits in its card's content area: pads, keys, scopes, and
/// knobs fill it; widgets that read along a line keep their height and
/// sit in the middle.
fn fit_body(body: Body, content: Rect, zoom: f32) -> Rect {
    match body {
        Body::Button | Body::Knob | Body::XyPad | Body::Keyboard | Body::Scope | Body::Analyzer => {
            content
        }
        _ => {
            let h = (body.height() * zoom).min(content.height());
            Rect::from_center_size(content.center(), vec2(content.width(), h))
        }
    }
}

/// A pinned node's body on screen, with its node view.
fn cell_view<'a>(
    cells: &[Cell],
    scene: &'a scene::Scene,
    i: usize,
) -> Option<(&'a NodeView, Rect)> {
    let c = cells.get(i)?;
    Some((scene.node(c.node)?, c.body?.1))
}

/// A card: its frame, an accent strip in the node's category color, the
/// node's label, and (outside performance mode) its buttons. `here`: what
/// the pointer is over on this card.
fn paint_card(
    painter: &egui::Painter,
    c: &Cell,
    n: Option<&NodeView>,
    here: PanelHit,
    moving: bool,
    target: bool,
) {
    let radius = theme::radius(theme::NODE_RADIUS);
    let alpha = if moving { 0.45 } else { 1.0 };
    painter.add(theme::node_shadow().as_shape(c.rect, radius));
    painter.rect_filled(c.rect, radius, theme::fade(theme::NODE_BG, alpha));
    let accent = n.map_or(theme::MISSING_HEADER, |n| n.accent);
    let strip = Rect::from_min_size(c.rect.min, vec2(c.rect.width(), theme::HEADER_STRIP));
    painter.rect_filled(
        strip,
        egui::CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            sw: 0,
            se: 0,
        },
        theme::fade(accent, alpha),
    );
    let outline = if target {
        Stroke::new(2.0, theme::ACCENT)
    } else if matches!(here, PanelHit::Card(_)) {
        Stroke::new(1.0, theme::NODE_HOVER)
    } else {
        Stroke::new(1.0, theme::fade(theme::NODE_OUTLINE, alpha))
    };
    painter.rect_stroke(c.rect, radius, outline, StrokeKind::Outside);
    let title = n.map_or_else(|| "Missing node".to_string(), |n| n.title.clone());
    let right = c.buttons.map_or(c.header.max.x - 8.0, |b| b[0].min.x - 4.0);
    let mut job = egui::text::LayoutJob::single_section(
        title,
        egui::TextFormat::simple(
            theme::ui_font(theme::FONT_NODE_TITLE * c.zoom.min(1.25)),
            theme::fade(theme::TEXT_STRONG, alpha),
        ),
    );
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((right - c.header.min.x - 10.0).max(8.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(
            c.header.min.x + 10.0,
            c.header.center().y + 1.5 - galley.size().y / 2.0,
        ),
        galley,
        theme::TEXT_STRONG,
    );
    if let Some(buttons) = c.buttons {
        let lit = [
            matches!(here, PanelHit::Shrink(_)),
            matches!(here, PanelHit::Grow(_)),
            matches!(here, PanelHit::Unpin(_)),
        ];
        let tips = [icons::MINUS, icons::PLUS, icons::X];
        for ((r, icon), lit) in buttons.iter().zip(tips).zip(lit) {
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                icon,
                theme::ui_font(theme::FONT_SMALL + 1.0),
                theme::fade(
                    if lit {
                        theme::TEXT_STRONG
                    } else {
                        theme::TEXT_FAINT
                    },
                    alpha,
                ),
            );
        }
    }
    if c.body.is_none() {
        painter.text(
            c.rect.center() + vec2(0.0, HEADER / 2.0),
            Align2::CENTER_CENTER,
            "no widget to show",
            theme::ui_font(theme::FONT_SMALL),
            theme::TEXT_MUTED,
        );
    }
}

fn empty_state(painter: &egui::Painter, rect: Rect, title: &str, text: &str) {
    let c = rect.center();
    painter.text(
        c - vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        format!("{}  {title}", icons::SQUARES_FOUR),
        theme::ui_font(theme::FONT_HEADING),
        theme::TEXT,
    );
    let galley = painter.layout(
        text.to_string(),
        theme::ui_font(theme::FONT_UI),
        theme::TEXT_MUTED,
        (rect.width() - 80.0).clamp(120.0, 520.0),
    );
    painter.galley(
        pos2(c.x - galley.size().x / 2.0, c.y - 8.0),
        galley,
        theme::TEXT_MUTED,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_flow_in_order_and_wrap() {
        // Four columns: a 2-wide slider, a 1×1 button, a 3×2 scope that
        // won't fit after them, then a button that goes after the scope.
        let p = place(&[[2, 1], [1, 1], [3, 2], [1, 1]], 4);
        assert_eq!(p, vec![(0, 0), (2, 0), (0, 1), (3, 1)]);
        // Wider than the grid: clamped to it.
        assert_eq!(place(&[[9, 1], [1, 1]], 3), vec![(0, 0), (0, 1)]);
        // One column: everything stacks.
        assert_eq!(place(&[[1, 2], [1, 1]], 1), vec![(0, 0), (0, 2)]);
        assert!(place(&[], 4).is_empty());
    }

    #[test]
    fn bigger_widgets_get_bigger_cells() {
        let registry = ac_nodes::registry();
        let span = |kind: &str, config: serde_json::Value| {
            let mut n = ac_core::NodeInstance::new(NodeId(1), kind);
            n.config = config;
            natural_span(&registry, &n)
        };
        assert_eq!(span("button", serde_json::json!({})), [1, 1]);
        assert_eq!(span("slider", serde_json::json!({})), [2, 1]);
        assert_eq!(span("xy_pad", serde_json::json!({})), [2, 2]);
        assert_eq!(span("scope", serde_json::json!({})), [3, 2]);
        assert_eq!(span("analyzer", serde_json::json!({})), [3, 2]);
        assert_eq!(span("keyboard", serde_json::json!({})), [3, 1]);
        let long = span("keyboard", serde_json::json!({"low": 21, "keys": 88}));
        assert_eq!(long, [ac_core::panel::MAX_SPAN, 1]);
        assert_eq!(span("gain", serde_json::json!({})), [1, 1]);
    }

    #[test]
    fn line_widgets_keep_their_height() {
        let content = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 90.0));
        let s = fit_body(Body::Slider, content, 1.0);
        assert_eq!(s.height(), Body::Slider.height());
        assert_eq!(s.center(), content.center());
        assert_eq!(fit_body(Body::XyPad, content, 1.0), content);
        // Never taller than the room.
        assert_eq!(fit_body(Body::Scope, content, 2.0), content);
        assert!(fit_body(Body::Meter, content, 9.0).height() <= 90.0);
    }
}
