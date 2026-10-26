//! Performance panels (docs/UX.md "Panels"): the current panel's pinned
//! nodes as a grid of their live widgets, without the graph, and
//! full-screen performance mode.
//!
//! The widgets are the canvas's own bodies, painted and played through
//! [`crate::canvas::controls`], so a slider moved here is the same edit as
//! on the canvas and a button held here is the same momentary input.
//! Panels pin top-level nodes only, so the view runs with the document at
//! the root ([`crate::document::Document::at_root`]).
//!
//! A panel has structure (DECISIONS D202): cards sit at places of their
//! own on a grid of cells and lines (half cells), strips stack cards under
//! one header, labels name sections, rows, and columns, compact cards take
//! half a cell, and a pinned group shows its sockets as a channel strip
//! (D203). [`grid`] decides where everything goes; this module draws it and
//! plays it.

use ac_core::{NodeId, Panel, Registry, SocketSpec};
use ac_engine::EngineHandle;
use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::actions::Action;
use crate::audio::EngineHost;
use crate::canvas::CanvasIo;
use crate::canvas::controls::{self, Click, ControlGesture, Controls, FieldResult, Grab, Look};
use crate::canvas::live::{Live, LiveState, MeterChannel};
use crate::canvas::piano;
use crate::canvas::scene::{self, Body, NodeView};
use crate::document::Document;
use crate::edit::pins;
use crate::i18n::{tr, tr_args};
use crate::{icons, theme};

mod access;
pub mod grid;
pub mod group_card;

use grid::Element;

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
/// Columns a panel lays out for when it has never been shown.
const DEFAULT_COLUMNS: u32 = 6;

/// The card colors a person can pick, by the name the project stores.
pub const CARD_COLORS: &[&str] = &["amber", "green", "red", "blue", "purple", "grey"];

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
    /// Pin nodes to a panel as one new strip, in this order (`None`: the
    /// panel the panel view shows, or a new one when there's none).
    Strip {
        nodes: Vec<NodeId>,
        panel: Option<usize>,
    },
    /// Make a new panel from the selection (D204).
    FromSelection { nodes: Vec<NodeId>, kind: Generated },
}

/// What kind of panel "panel from selection" makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generated {
    /// A strip per group (a Track block's channel strip) and the loose
    /// controls after them.
    Mixer,
    /// A Pads grid playing the selected Samplers and Clips.
    Pads,
    /// A launch grid of the selected Clips: a column per track (by where
    /// they sit across the canvas), a row per scene.
    Grid,
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
    /// The strip it's stacked in.
    pub strip: Option<u32>,
    pub compact: bool,
    /// The header's color.
    pub accent: Color32,
    /// A pinned group's widgets and where each is drawn.
    pub group: Option<(Vec<group_card::Widget>, Vec<Rect>)>,
}

/// A strip drawn last frame: its header and everything it holds.
#[derive(Clone, Debug)]
pub struct StripCell {
    pub id: u32,
    pub name: String,
    pub header: Rect,
    pub area: Rect,
    /// Narrower, wider, and unpin (none in performance mode).
    pub buttons: Option<[Rect; 3]>,
    pub accent: Color32,
}

/// A label drawn last frame.
#[derive(Clone, Debug)]
pub struct LabelCell {
    pub index: usize,
    pub rect: Rect,
    pub text: String,
}

/// How grid places map to the screen this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Geometry {
    origin: Pos2,
    /// A cell's width and height (points).
    uw: f32,
    uh: f32,
    pub scale: f32,
}

impl Geometry {
    /// One line: half a cell and half a gap, so two lines are a cell.
    fn line(&self) -> f32 {
        (self.uh + GAP) / 2.0
    }

    /// The screen rectangle of `cols` × `lines` at `[col, line]`.
    pub fn rect(&self, col: u32, line: u32, cols: u32, lines: u32) -> Rect {
        let min = self.origin + vec2(col as f32 * (self.uw + GAP), line as f32 * self.line());
        Rect::from_min_size(
            min,
            vec2(
                cols as f32 * self.uw + cols.saturating_sub(1) as f32 * GAP,
                (lines as f32 * self.line() - GAP).max(4.0),
            ),
        )
    }

    /// The grid place under screen point `p` (negative off the grid).
    pub fn place_at(&self, p: Pos2) -> [i32; 2] {
        let w = (self.uw + GAP).max(1.0);
        let l = self.line().max(1.0);
        [
            ((p.x - self.origin.x) / w).floor() as i32,
            ((p.y - self.origin.y) / l).floor() as i32,
        ]
    }
}

/// What the pointer is over in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelHit {
    None,
    Body(usize),
    /// Anywhere else on a card: drag to move, click to select.
    Card(usize),
    Shrink(usize),
    Grow(usize),
    Unpin(usize),
    /// Widget `.1` of the pinned group on card `.0`.
    Widget(usize, usize),
    Strip(usize),
    StripShrink(usize),
    StripGrow(usize),
    StripUnpin(usize),
    Label(usize),
}

impl PanelHit {
    fn cell(self) -> Option<usize> {
        match self {
            PanelHit::Body(i)
            | PanelHit::Card(i)
            | PanelHit::Shrink(i)
            | PanelHit::Grow(i)
            | PanelHit::Unpin(i)
            | PanelHit::Widget(i, _) => Some(i),
            _ => None,
        }
    }
}

/// Typing a value into a pinned Number or Slider, or a group's input.
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

/// Typing a strip's name or a label's text.
struct TextEdit {
    what: Element,
    text: String,
    rect: Rect,
    started: bool,
}

/// Cards, strips, or labels being dragged to a new place.
#[derive(Clone, Debug)]
struct Moving {
    elements: Vec<Element>,
    /// The grid place the drag started on.
    from: [i32; 2],
}

/// A change a panel menu entry makes, as one undo step.
type PanelChange = Box<dyn FnOnce(&mut Panel) -> bool>;

/// What a right-click opened the menu for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuTarget {
    Card(usize),
    Strip(u32),
    Label(usize),
    /// An empty place on the grid.
    Place([u32; 2]),
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
    /// Performance mode fills the window instead of asking it to go full
    /// screen (a plugin window, whose host owns its size).
    pub fills_window: bool,
    /// Widgets being played.
    pub controls: Controls,
    live: LiveState,
    press: Option<PanelHit>,
    moving: Option<Moving>,
    /// Cards, strips, and labels selected (click, Ctrl- or Shift-click).
    pub selected: Vec<Element>,
    /// An open choice popup: the node, its input, and where.
    choice: Option<(NodeId, String, Pos2)>,
    choice_age: u32,
    edit: Option<ValueEdit>,
    rename: Option<Rename>,
    text_edit: Option<TextEdit>,
    menu: Option<MenuTarget>,
    scroll: Vec2,
    /// Columns the grid fit last frame (new pins are placed for it).
    pub columns: u32,
    /// The cards drawn last frame (tests read these).
    pub cells: Vec<Cell>,
    /// The strips and labels drawn last frame (tests read these).
    pub strips: Vec<StripCell>,
    pub labels: Vec<LabelCell>,
    /// The grid's geometry last frame.
    pub geometry: Geometry,
    /// The panel's layout last frame, in grid places.
    layout: grid::Layout,
    /// The Button that Space is holding down (access.rs).
    key_hold: Option<NodeId>,
}

impl PerformView {
    /// Is a text field taking the keyboard?
    pub fn is_typing(&self) -> bool {
        self.edit.is_some() || self.rename.is_some() || self.text_edit.is_some()
    }

    /// A new document: start from its first panel with nothing held.
    pub fn reset(&mut self) {
        self.current = 0;
        self.controls = Controls::default();
        self.press = None;
        self.moving = None;
        self.selected.clear();
        self.choice = None;
        self.edit = None;
        self.rename = None;
        self.text_edit = None;
        self.menu = None;
        self.scroll = Vec2::ZERO;
        self.cells.clear();
        self.strips.clear();
        self.labels.clear();
        self.key_hold = None;
    }

    /// Escape: close a popup or a field, let go of a control, or forget
    /// the selection. Returns whether there was something to cancel.
    pub fn cancel(&mut self, doc: &mut Document) -> bool {
        if self.choice.take().is_some() || self.edit.take().is_some() {
            return true;
        }
        if self.rename.take().is_some()
            || self.moving.take().is_some()
            || self.text_edit.take().is_some()
        {
            return true;
        }
        if self.controls.cancel(doc) {
            return true;
        }
        if !self.selected.is_empty() {
            self.selected.clear();
            return true;
        }
        false
    }

    /// The view is going away: let go of everything held, now.
    pub fn let_go(&mut self, doc: &mut Document, engine: &mut EngineHandle) {
        self.choice = None;
        self.edit = None;
        self.moving = None;
        self.text_edit = None;
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
        self.text_edit = None;
        if on {
            self.shown = true;
        }
        if !self.fills_window {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        }
    }

    /// Follow the window: leaving full screen some other way (the window's
    /// own controls) ends performance mode.
    pub fn follow_window(&mut self, ctx: &egui::Context) {
        if !self.fullscreen || self.fills_window {
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

    /// Columns to place new pins for: what the grid fit last time it was
    /// shown.
    pub fn place_columns(&self) -> u32 {
        if self.columns == 0 {
            DEFAULT_COLUMNS
        } else {
            self.columns
        }
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
            ui.painter().rect_filled(strip, 0.0, theme::panel_bg());
            ui.painter().line_segment(
                [strip.left_bottom(), strip.right_bottom()],
                Stroke::new(1.0, theme::border()),
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
                            io.doc.edit_panels(&tr("undo-rename-panel"), |p, _| {
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
                    theme::text_strong()
                } else {
                    theme::text_muted()
                });
            let tab = ui
                .add(
                    egui::Button::new(text)
                        .selected(selected)
                        .frame_when_inactive(false)
                        .min_size(vec2(0.0, 26.0)),
                )
                .on_hover_text(tr("perform-tab-tip"));
            tab.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, &name));
            if tab.clicked() {
                self.current = i;
            }
            if tab.double_clicked() {
                self.start_rename(i, &name);
            }
            tab.context_menu(|ui| {
                if icons::spoken(ui.button(format!("{}  {}", icons::PENCIL, tr("perform-rename"))))
                    .clicked()
                {
                    self.start_rename(i, &name);
                    ui.close();
                }
                let left = icons::spoken(ui.add_enabled(
                    i > 0,
                    egui::Button::new(format!(
                        "{}  {}",
                        icons::ARROW_LEFT,
                        tr("perform-move-left")
                    )),
                ));
                if left.clicked() {
                    self.move_panel(io, i, i - 1);
                    ui.close();
                }
                let right = icons::spoken(ui.add_enabled(
                    i + 1 < n,
                    egui::Button::new(format!(
                        "{}  {}",
                        icons::CARET_RIGHT,
                        tr("perform-move-right")
                    )),
                ));
                if right.clicked() {
                    self.move_panel(io, i, i + 1);
                    ui.close();
                }
                ui.separator();
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::TRASH,
                    tr("perform-delete-panel")
                )))
                .on_hover_text(tr("perform-delete-panel-tip"))
                .clicked()
                {
                    io.doc.edit_panels(&tr("undo-delete-panel"), |p, _| {
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
        let add = icons::spoken(ui.add(
            egui::Button::new(RichText::new(icons::PLUS).size(15.0)).frame_when_inactive(false),
        ))
        .on_hover_text(tr("perform-new-panel"))
        .accessible_name(tr("perform-new-panel"));
        if add.clicked() {
            let (i, name) = io.doc.edit_panels(&tr("undo-new-panel"), |p, _| {
                let i = pins::add_panel(p, "");
                (i, p.get(i).map_or(String::new(), |x| x.name.clone()))
            });
            self.current = i;
            self.start_rename(i, &name);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let perform = icons::spoken(ui.add(egui::Button::new(format!(
                "{}  {}",
                icons::CORNERS_OUT,
                tr("perform-perform")
            ))))
            .on_hover_text(tr("perform-perform-tip"));
            if perform.clicked() {
                self.set_fullscreen(ui.ctx(), true);
            }
            ui.label(
                RichText::new(tr("perform-hint"))
                    .small()
                    .color(theme::text_faint()),
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
            .edit_panels(&tr("undo-move-panel"), |p, _| pins::move_panel(p, from, to));
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
        play_keys: bool,
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
                            .color(if playing {
                                theme::play()
                            } else {
                                theme::text()
                            }),
                    )
                    .selected(playing)
                    .min_size(vec2(52.0, 36.0)),
                )
                // With Play keys on, Space goes to the patch.
                .on_hover_text(match (playing, play_keys) {
                    (true, false) => tr("transport-stop-tip"),
                    (false, false) => tr("perform-play-tip"),
                    (true, true) => tr("transport-stop"),
                    (false, true) => tr("transport-play"),
                })
                .accessible_name(if playing {
                    tr("transport-stop")
                } else {
                    tr("transport-play")
                });
            if play.clicked() {
                actions.push(Action::PlayStop);
            }
            crate::transport::panic_button(ui, &mut actions);
            let spoken = tr_args(
                "transport-position",
                &[("bars", (&bars).into()), ("time", (&time).into())],
            );
            ui.label(
                RichText::new(bars)
                    .font(theme::mono_font(20.0))
                    .color(theme::text_strong()),
            )
            .accessible_name(spoken);
            ui.label(
                RichText::new(time)
                    .font(theme::mono_font(theme::FONT_UI))
                    .color(theme::text_muted()),
            );
            if doc.project.panels.len() > 1 {
                ui.add_space(12.0);
                for (i, p) in doc.project.panels.iter().enumerate() {
                    let selected = i == self.current;
                    let b = egui::Button::new(RichText::new(&p.name).size(theme::FONT_UI))
                        .selected(selected)
                        .frame_when_inactive(false)
                        .min_size(vec2(0.0, 32.0));
                    let r = ui.add(b);
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::Tab, true, selected, &p.name)
                    });
                    if r.clicked() {
                        self.current = i;
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let exit = icons::spoken(
                    ui.add(
                        egui::Button::new(
                            RichText::new(format!("{}  {}", icons::CORNERS_IN, tr("perform-exit")))
                                .size(theme::FONT_UI),
                        )
                        .min_size(vec2(0.0, 32.0)),
                    ),
                )
                .on_hover_text(tr("perform-exit-tip"));
                if exit.clicked() {
                    actions.push(Action::PerformanceMode);
                }
                let keys = icons::spoken(
                    ui.add(
                        egui::Button::new(RichText::new(icons::KEYBOARD).size(20.0).color(
                            if play_keys {
                                theme::accent()
                            } else {
                                theme::text()
                            },
                        ))
                        .selected(play_keys)
                        .min_size(vec2(40.0, 32.0)),
                    ),
                )
                .on_hover_text(if play_keys {
                    tr("perform-play-keys-on")
                } else {
                    tr("perform-play-keys-off")
                })
                .accessible_name(tr("transport-play-keys"));
                if keys.clicked() {
                    actions.push(Action::PlayKeys);
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
        self.name_grid(io, &response);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::canvas_bg());
        let dt = self.live.begin_frame(now);
        if self.controls.release_due(io, now) {
            ctx.request_repaint();
        }
        let Some(panel) = io.doc.project.panels.get(self.current).cloned() else {
            self.cells.clear();
            self.strips.clear();
            self.labels.clear();
            empty_state(
                &painter,
                rect,
                &tr("perform-no-panels"),
                &tr("perform-no-panels-hint"),
            );
            return;
        };
        // Cards play widgets the canvas may show collapsed.
        let scene = scene::build_expanded(io.doc, io.registry);
        if response.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta);
            self.scroll -= d;
        }
        let content = self.layout_view(&panel, &scene, io, rect);
        self.scroll.x = self
            .scroll
            .x
            .clamp(0.0, (content.x - rect.width()).max(0.0));
        self.scroll.y = self
            .scroll
            .y
            .clamp(0.0, (content.y - rect.height()).max(0.0));
        if self.cells.is_empty() && self.labels.is_empty() {
            empty_state(
                &painter,
                rect,
                &tr_args("perform-nothing-pinned", &[("panel", (&panel.name).into())]),
                &tr("perform-nothing-pinned-hint"),
            );
        }

        let (pressed, released, modifiers, origin, secondary) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.modifiers,
                i.pointer.press_origin(),
                i.pointer.secondary_pressed(),
            )
        });
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hover = match pointer {
            Some(p) if rect.contains(p) && (response.hovered() || !self.idle()) => self.hit(p),
            _ => PanelHit::None,
        };

        if pressed && response.hovered() {
            self.press = Some(hover);
            match (hover, pointer) {
                (PanelHit::Body(i), Some(p)) => {
                    if let Some((n, r)) = cell_view(&self.cells, &scene, i) {
                        self.controls.press(io, n, r, p, now);
                    }
                }
                (PanelHit::Widget(i, k), Some(_)) => {
                    let w = self.widget(i, k);
                    if let (Some(n), Some(w)) =
                        (self.cells.get(i).and_then(|c| scene.node(c.node)), w)
                        && w.kind == group_card::Kind::Fire
                        && !w.wired
                    {
                        crate::params::fire(io.doc, io.engine, n.id, &w.socket);
                    }
                }
                _ => {}
            }
        }
        if secondary && response.hovered() {
            self.menu = Some(self.menu_target(hover, pointer));
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
            let start = origin.or(pointer);
            match self.press.unwrap_or(hover) {
                PanelHit::Body(i) => {
                    if let (Some((n, r)), Some(o)) = (cell_view(&self.cells, &scene, i), start)
                        && self.controls.begin_drag(io, n, r, o, now) == Grab::Other
                    {
                        self.start_move(Element::Item(self.cells[i].item), o);
                    }
                }
                PanelHit::Widget(i, k) => {
                    if let (Some(o), Some(c)) = (start, self.cells.get(i).cloned()) {
                        self.drag_widget(io, &scene, &c, k, o, now);
                    }
                }
                PanelHit::Card(i) => {
                    if let (Some(o), Some(c)) = (start, self.cells.get(i)) {
                        let e = Element::Item(c.item);
                        self.start_move(e, o);
                    }
                }
                PanelHit::Strip(s) => {
                    if let (Some(o), Some(st)) = (start, self.strips.get(s)) {
                        let e = Element::Strip(st.id);
                        self.start_move(e, o);
                    }
                }
                PanelHit::Label(l) => {
                    if let (Some(o), Some(lc)) = (start, self.labels.get(l)) {
                        let e = Element::Label(lc.index);
                        self.start_move(e, o);
                    }
                }
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
            if let (Some(m), Some(p)) = (self.moving.take(), pointer) {
                self.finish_move(io, &panel, m, p);
            }
        }
        if response.clicked()
            && !matches!(
                self.controls.active,
                Some(ControlGesture::HoldButton { .. } | ControlGesture::HoldDrum { .. })
            )
        {
            self.click(io, &scene, self.press.unwrap_or(hover), modifiers, now);
        }
        if response.double_clicked() {
            self.double_click(io, &scene, self.press.unwrap_or(hover), now);
        }
        if released {
            self.press = None;
        }

        // Paint.
        let tele = io.engine.telemetry();
        let live = Live::new(&tele, io.doc);
        for s in &self.strips {
            paint_strip(&painter, s, self.geometry.scale);
        }
        for l in &self.labels {
            paint_label(&painter, l, self.geometry.scale);
        }
        let target = self
            .moving
            .as_ref()
            .and(hover.cell())
            .filter(|_| self.moving.as_ref().is_some_and(|m| m.elements.len() == 1));
        for (i, c) in self.cells.iter().enumerate() {
            let n = scene.node(c.node);
            let here = match hover {
                h if h.cell() == Some(i) => h,
                _ => PanelHit::None,
            };
            let moving = self
                .moving
                .as_ref()
                .is_some_and(|m| m.elements.contains(&Element::Item(c.item)));
            let selected = self.selected.contains(&Element::Item(c.item));
            paint_card(&painter, c, n, here, moving, target == Some(i), selected);
            let Some(n) = n else { continue };
            if let Some((widgets, rects)) = &c.group {
                let look = group_card::Look {
                    zoom: c.zoom,
                    alpha: if n.muted { 0.5 } else { 1.0 },
                    hovered: match here {
                        PanelHit::Widget(_, k) => Some(k),
                        _ => None,
                    },
                    dragging: match &self.controls.active {
                        Some(ControlGesture::DragValue { node, socket, .. }) if *node == c.node => {
                            Some(socket.clone())
                        }
                        _ => None,
                    },
                };
                group_card::paint(
                    &painter,
                    io,
                    &live,
                    &mut self.live,
                    n,
                    widgets,
                    rects,
                    &look,
                    now,
                    dt,
                );
                continue;
            }
            let Some((body, br)) = c.body else { continue };
            let look = Look {
                zoom: c.zoom,
                alpha: if n.muted { 0.5 } else { 1.0 },
                hovered: here == PanelHit::Body(i),
                pointer,
                held: self.controls.held_inputs(c.node),
                dragging: self.controls.dragging(c.node, None),
                editing: self.edit.as_ref().is_some_and(|e| e.node == c.node),
                choice_open: self
                    .choice
                    .as_ref()
                    .is_some_and(|(node, _, _)| *node == c.node),
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
        for (k, l) in self.labels.iter().enumerate() {
            if self.selected.contains(&Element::Label(l.index)) {
                painter.rect_stroke(
                    l.rect.expand(2.0),
                    theme::radius(4.0),
                    Stroke::new(1.5, theme::accent()),
                    StrokeKind::Outside,
                );
            }
            let _ = k;
        }
        for s in &self.strips {
            if self.selected.contains(&Element::Strip(s.id)) {
                painter.rect_stroke(
                    s.area.expand(2.0),
                    theme::radius(theme::NODE_RADIUS + 2.0),
                    Stroke::new(1.5, theme::accent()),
                    StrokeKind::Outside,
                );
            }
        }
        if let (Some(m), Some(p)) = (&self.moving, pointer) {
            self.paint_ghosts(&painter, m, p);
        }

        self.accessibility(ui, io, &scene, &live, now);

        // Popups, menus, and fields over the grid.
        self.context_menu(&response, io, &panel);
        if let Some((node, socket, at)) = self.choice.clone() {
            let close = match scene.node(node) {
                Some(n) => controls::choice_popup(
                    &ctx,
                    egui::Id::unique("panel-choice"),
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
        if let Some(mut e) = self.text_edit.take() {
            let font = theme::ui_font(theme::FONT_UI);
            let id = egui::Id::unique("panel-text-edit");
            match controls::text_field(ui, id, e.rect, font, &mut e.text, &mut e.started, false) {
                FieldResult::Editing => self.text_edit = Some(e),
                FieldResult::Cancel => {}
                FieldResult::Commit => self.commit_text(io, &e),
            }
        }
        if self.live.active || !self.idle() || io.engine.is_playing() {
            ctx.request_repaint();
        }
    }

    fn idle(&self) -> bool {
        !self.controls.is_active() && self.moving.is_none()
    }

    /// Widget `k` of the pinned group on card `i`.
    fn widget(&self, i: usize, k: usize) -> Option<group_card::Widget> {
        self.cells
            .get(i)
            .and_then(|c| c.group.as_ref())
            .and_then(|(w, _)| w.get(k).cloned())
    }

    /// Start dragging `grabbed` (and the rest of the selection, when it's
    /// part of it) from screen point `origin`.
    fn start_move(&mut self, grabbed: Element, origin: Pos2) {
        let elements = if self.selected.contains(&grabbed) {
            self.selected.clone()
        } else {
            vec![grabbed]
        };
        self.moving = Some(Moving {
            elements,
            from: self.geometry.place_at(origin),
        });
    }

    /// A drag ended at screen point `p`: the dragged things go where it
    /// ended, or a card into the strip it was dropped on.
    fn finish_move(&mut self, io: &mut CanvasIo, panel: &Panel, m: Moving, p: Pos2) {
        let to = self.geometry.place_at(p);
        let delta = [to[0] - m.from[0], to[1] - m.from[1]];
        let drop = match m.elements.as_slice() {
            [Element::Item(item)] => self
                .strips
                .iter()
                .find(|s| s.area.contains(p))
                .map(|s| grid::Drop::Strip {
                    id: s.id,
                    before: self
                        .cells
                        .iter()
                        .find(|c| c.strip == Some(s.id) && c.rect.contains(p) && c.item != *item)
                        .map(|c| c.item),
                })
                .unwrap_or(grid::Drop::Place),
            _ => grid::Drop::Place,
        };
        if delta == [0, 0] && drop == grid::Drop::Place {
            return;
        }
        let layout = self.layout.clone();
        let index = self.current;
        let _ = panel;
        io.doc.edit_panels(&tr("undo-move-on-panel"), |panels, _| {
            panels
                .get_mut(index)
                .is_some_and(|pl| grid::move_elements(pl, &layout, &m.elements, delta, drop))
        });
    }

    /// Outlines where the dragged things would land.
    fn paint_ghosts(&self, painter: &egui::Painter, m: &Moving, p: Pos2) {
        let to = self.geometry.place_at(p);
        let delta = [to[0] - m.from[0], to[1] - m.from[1]];
        for e in &m.elements {
            let Some(pl) = self.layout.get(*e) else {
                continue;
            };
            let lines = match e {
                Element::Strip(id) => {
                    self.layout
                        .placed
                        .iter()
                        .filter(|q| q.strip == Some(*id))
                        .map(|q| q.lines)
                        .sum::<u32>()
                        + 1
                }
                _ => pl.lines,
            };
            let col = (pl.col as i32 + delta[0]).max(0) as u32;
            let line = (pl.line as i32 + delta[1]).max(0) as u32;
            let ghost = self.geometry.rect(col, line, pl.cols, lines);
            painter.rect_stroke(
                ghost,
                theme::radius(theme::NODE_RADIUS),
                Stroke::new(1.5, theme::accent()),
                StrokeKind::Inside,
            );
        }
    }

    /// What a right-click at `p` opened the menu for.
    fn menu_target(&self, hit: PanelHit, p: Option<Pos2>) -> MenuTarget {
        match hit {
            PanelHit::Body(i)
            | PanelHit::Card(i)
            | PanelHit::Shrink(i)
            | PanelHit::Grow(i)
            | PanelHit::Unpin(i)
            | PanelHit::Widget(i, _) => {
                MenuTarget::Card(self.cells.get(i).map_or(usize::MAX, |c| c.item))
            }
            PanelHit::Strip(s)
            | PanelHit::StripShrink(s)
            | PanelHit::StripGrow(s)
            | PanelHit::StripUnpin(s) => {
                MenuTarget::Strip(self.strips.get(s).map_or(u32::MAX, |x| x.id))
            }
            PanelHit::Label(l) => {
                MenuTarget::Label(self.labels.get(l).map_or(usize::MAX, |x| x.index))
            }
            PanelHit::None => {
                let [c, l] = p.map_or([0, 0], |p| self.geometry.place_at(p));
                MenuTarget::Place([c.max(0) as u32, l.max(0) as u32])
            }
        }
    }

    /// The grid's right-click menu, for what was clicked.
    fn context_menu(&mut self, response: &egui::Response, io: &mut CanvasIo, panel: &Panel) {
        if self.fullscreen {
            return;
        }
        let index = self.current;
        let layout = self.layout.clone();
        let columns = self.place_columns();
        let Some(target) = self.menu else {
            return;
        };
        let mut done: Option<PanelChange> = None;
        let mut label = String::new();
        let mut start_text: Option<Element> = None;
        response.context_menu(|ui| match target {
            MenuTarget::Card(item) => {
                let Some(it) = panel.items.get(item).cloned() else {
                    ui.close();
                    return;
                };
                let mut compact = it.compact;
                if ui.checkbox(&mut compact, tr("perform-compact")).clicked() {
                    label = tr("undo-panel-layout");
                    done = Some(Box::new(move |p: &mut Panel| {
                        p.items.get_mut(item).is_some_and(|x| {
                            x.compact = compact;
                            true
                        })
                    }));
                    ui.close();
                }
                if let Some(c) = color_menu(ui, it.color.as_deref()) {
                    label = tr("undo-panel-color");
                    done = Some(Box::new(move |p: &mut Panel| {
                        p.items.get_mut(item).is_some_and(|x| {
                            x.color = c;
                            true
                        })
                    }));
                    ui.close();
                }
                ui.separator();
                let together: Vec<usize> = if self.selected.contains(&Element::Item(item)) {
                    self.selected
                        .iter()
                        .filter_map(|e| match e {
                            Element::Item(i) => Some(*i),
                            _ => None,
                        })
                        .collect()
                } else {
                    vec![item]
                };
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::SQUARES_FOUR,
                    tr("perform-make-strip")
                )))
                .clicked()
                {
                    label = tr("undo-make-strip");
                    let name = tr("perform-new-strip");
                    let layout = layout.clone();
                    done = Some(Box::new(move |p: &mut Panel| {
                        grid::make_strip(p, &layout, &together, &name).is_some()
                    }));
                    ui.close();
                }
                if it.strip.is_some() && ui.button(tr("perform-out-of-strip")).clicked() {
                    label = tr("undo-panel-layout");
                    let layout = layout.clone();
                    done = Some(Box::new(move |p: &mut Panel| {
                        grid::take_out_of_strip(p, &layout, item)
                    }));
                    ui.close();
                }
                if icons::spoken(ui.button(format!("{}  {}", icons::X, tr("perform-unpin"))))
                    .clicked()
                {
                    label = tr("undo-unpin");
                    done = Some(Box::new(move |p: &mut Panel| {
                        if item < p.items.len() {
                            p.items.remove(item);
                            true
                        } else {
                            false
                        }
                    }));
                    ui.close();
                }
            }
            MenuTarget::Strip(id) => {
                let Some(st) = panel.strip(id).cloned() else {
                    ui.close();
                    return;
                };
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::PENCIL,
                    tr("perform-rename-strip")
                )))
                .clicked()
                {
                    start_text = Some(Element::Strip(id));
                    ui.close();
                }
                if let Some(c) = color_menu(ui, st.color.as_deref()) {
                    label = tr("undo-panel-color");
                    done = Some(Box::new(move |p: &mut Panel| {
                        p.strips.iter_mut().find(|s| s.id == id).is_some_and(|s| {
                            s.color = c;
                            true
                        })
                    }));
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("perform-ungroup-strip")).clicked() {
                    label = tr("undo-panel-layout");
                    let layout = layout.clone();
                    done = Some(Box::new(move |p: &mut Panel| {
                        grid::dissolve_strip(p, &layout, id)
                    }));
                    ui.close();
                }
                if icons::spoken(ui.button(format!("{}  {}", icons::X, tr("perform-unpin-strip"))))
                    .clicked()
                {
                    label = tr("undo-unpin");
                    done = Some(Box::new(move |p: &mut Panel| grid::remove_strip(p, id)));
                    ui.close();
                }
            }
            MenuTarget::Label(k) => {
                let is_text = panel.labels.get(k).is_some_and(|l| !l.is_separator());
                if is_text
                    && icons::spoken(ui.button(format!(
                        "{}  {}",
                        icons::PENCIL,
                        tr("perform-edit-label")
                    )))
                    .clicked()
                {
                    start_text = Some(Element::Label(k));
                    ui.close();
                }
                let wider = ui.button(tr("perform-label-wider"));
                let narrower = ui.button(tr("perform-label-narrower"));
                for (r, d) in [(wider, 1i32), (narrower, -1)] {
                    if r.clicked() {
                        label = tr("undo-panel-layout");
                        done = Some(Box::new(move |p: &mut Panel| {
                            p.labels.get_mut(k).is_some_and(|l| {
                                let next = (l.span() as i32 + d).max(1) as u32;
                                let changed = next != l.span();
                                l.span = next;
                                changed
                            })
                        }));
                        ui.close();
                    }
                }
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::TRASH,
                    tr("perform-remove-label")
                )))
                .clicked()
                {
                    label = tr("undo-panel-layout");
                    done = Some(Box::new(move |p: &mut Panel| {
                        if k < p.labels.len() {
                            p.labels.remove(k);
                            true
                        } else {
                            false
                        }
                    }));
                    ui.close();
                }
            }
            MenuTarget::Place(at) => {
                if ui.button(tr("perform-add-label")).clicked() {
                    label = tr("undo-panel-layout");
                    let layout = layout.clone();
                    let text = tr("perform-new-label");
                    done = Some(Box::new(move |p: &mut Panel| {
                        grid::add_label(p, &layout, at, &text, columns);
                        true
                    }));
                    start_text = Some(Element::Label(panel.labels.len()));
                    ui.close();
                }
                if ui.button(tr("perform-add-separator")).clicked() {
                    label = tr("undo-panel-layout");
                    let layout = layout.clone();
                    done = Some(Box::new(move |p: &mut Panel| {
                        grid::add_label(p, &layout, at, "", columns.max(1));
                        true
                    }));
                    ui.close();
                }
            }
        });
        if let Some(f) = done {
            io.doc
                .edit_panels(&label, |panels, _| panels.get_mut(index).is_some_and(f));
            self.selected.clear();
            self.menu = None;
        }
        if let Some(e) = start_text {
            self.menu = None;
            self.begin_text(io, e);
        }
    }

    /// Start typing a strip's name or a label's text.
    fn begin_text(&mut self, io: &CanvasIo, what: Element) {
        let Some(panel) = io.doc.project.panels.get(self.current) else {
            return;
        };
        let (text, rect) = match what {
            Element::Strip(id) => (
                panel.strip(id).map(|s| s.name.clone()),
                self.strips.iter().find(|s| s.id == id).map(|s| s.header),
            ),
            Element::Label(k) => (
                panel.labels.get(k).map(|l| l.text.clone()),
                self.labels
                    .iter()
                    .find(|l| l.index == k)
                    .map(|l| l.rect)
                    .or_else(|| {
                        panel
                            .labels
                            .get(k)
                            .map(|l| self.geometry.rect(l.at[0], l.at[1], l.span(), 1))
                    }),
            ),
            Element::Item(_) => (None, None),
        };
        if let (Some(text), Some(rect)) = (text, rect) {
            self.text_edit = Some(TextEdit {
                what,
                text,
                rect: Rect::from_min_size(rect.min, vec2(rect.width().max(140.0), 24.0)),
                started: false,
            });
        }
    }

    fn commit_text(&mut self, io: &mut CanvasIo, e: &TextEdit) {
        let index = self.current;
        let text = e.text.trim().to_string();
        let what = e.what;
        let label = match what {
            Element::Strip(_) => tr("undo-rename-strip"),
            _ => tr("undo-panel-layout"),
        };
        io.doc.edit_panels(&label, |panels, _| {
            let Some(p) = panels.get_mut(index) else {
                return false;
            };
            match what {
                Element::Strip(id) => match p.strips.iter_mut().find(|s| s.id == id) {
                    Some(s) if !text.is_empty() && s.name != text => {
                        s.name = text;
                        true
                    }
                    _ => false,
                },
                Element::Label(k) => match p.labels.get_mut(k) {
                    Some(l) if l.text != text => {
                        l.text = text;
                        true
                    }
                    _ => false,
                },
                Element::Item(_) => false,
            }
        });
    }

    /// Start dragging a pinned group's knob, fader, or number.
    fn drag_widget(
        &mut self,
        io: &mut CanvasIo,
        scene: &scene::Scene,
        c: &Cell,
        k: usize,
        origin: Pos2,
        now: f64,
    ) {
        let Some((widgets, rects)) = &c.group else {
            return;
        };
        let (Some(w), Some(r)) = (widgets.get(k), rects.get(k)) else {
            return;
        };
        if w.wired {
            return;
        }
        let Some(spec) = w.spec.clone() else { return };
        let Some(n) = scene.node(c.node) else { return };
        let Some(inst) = io.shown_node(n.id) else {
            return;
        };
        let start = crate::canvas::widgets::number(&inst, &spec);
        let (travel, vertical) = match w.kind {
            group_card::Kind::Fader { .. } => (r.height().max(40.0), true),
            group_card::Kind::Knob => (controls::KNOB_TRAVEL, true),
            group_card::Kind::Number => (controls::DRAG_TRAVEL, false),
            _ => return,
        };
        let _ = now;
        self.controls
            .begin_value_drag(n.id, &w.socket, spec, start, origin, travel, vertical);
    }

    /// A click on the grid.
    fn click(
        &mut self,
        io: &mut CanvasIo,
        scene: &scene::Scene,
        hit: PanelHit,
        modifiers: egui::Modifiers,
        now: f64,
    ) {
        let panel = self.current;
        let adding = modifiers.ctrl || modifiers.command || modifiers.shift;
        let select = |this: &mut Self, e: Element| {
            if adding {
                if let Some(at) = this.selected.iter().position(|x| *x == e) {
                    this.selected.remove(at);
                } else {
                    this.selected.push(e);
                }
            } else {
                this.selected = vec![e];
            }
        };
        match hit {
            PanelHit::None => {
                self.selected.clear();
                return;
            }
            PanelHit::Strip(s) => {
                if let Some(id) = self.strips.get(s).map(|x| x.id) {
                    select(self, Element::Strip(id));
                }
                return;
            }
            PanelHit::Label(l) => {
                if let Some(k) = self.labels.get(l).map(|x| x.index) {
                    select(self, Element::Label(k));
                }
                return;
            }
            PanelHit::StripShrink(s) | PanelHit::StripGrow(s) => {
                let Some(id) = self.strips.get(s).map(|x| x.id) else {
                    return;
                };
                let d: i32 = if matches!(hit, PanelHit::StripGrow(_)) {
                    1
                } else {
                    -1
                };
                io.doc.edit_panels(&tr("undo-resize-on-panel"), |p, _| {
                    p.get_mut(panel)
                        .is_some_and(|pl| grid::widen_strip(pl, id, d))
                });
                return;
            }
            PanelHit::StripUnpin(s) => {
                let Some(id) = self.strips.get(s).map(|x| x.id) else {
                    return;
                };
                io.doc.edit_panels(&tr("undo-unpin"), |p, _| {
                    p.get_mut(panel)
                        .is_some_and(|pl| grid::remove_strip(pl, id))
                });
                return;
            }
            _ => {}
        }
        let Some(c) = hit.cell().and_then(|i| self.cells.get(i)).cloned() else {
            return;
        };
        match hit {
            PanelHit::Card(_) => select(self, Element::Item(c.item)),
            PanelHit::Body(_) => {
                let Some(n) = scene.node(c.node) else { return };
                let Some((_, r)) = c.body else { return };
                match controls::click(io, n) {
                    Click::Done | Click::Select => {}
                    Click::Choose => self.choice = Some((n.id, "value".into(), r.left_bottom())),
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
            PanelHit::Widget(i, k) => {
                let (Some(w), Some(n)) = (self.widget(i, k), scene.node(c.node)) else {
                    return;
                };
                let rect = c.group.as_ref().and_then(|g| g.1.get(k).copied());
                match w.kind {
                    group_card::Kind::Toggle => group_card::flip(io, n, &w, now),
                    group_card::Kind::Choice if !w.wired => {
                        if let Some(r) = rect {
                            self.choice = Some((n.id, w.socket.clone(), r.left_bottom()));
                        }
                    }
                    _ => {}
                }
            }
            PanelHit::Unpin(_) => {
                let k = c.item;
                io.doc.edit_panels(&tr("undo-unpin"), |p, _| {
                    if let Some(pl) = p.get_mut(panel)
                        && k < pl.items.len()
                    {
                        pl.items.remove(k);
                    }
                });
                self.selected.clear();
            }
            PanelHit::Grow(_) | PanelHit::Shrink(_) => {
                let grow = matches!(hit, PanelHit::Grow(_));
                let wide_only =
                    c.compact || c.group.is_some() || c.body.is_some_and(|b| grows_wide(b.0));
                let k = c.item;
                let natural = io
                    .doc
                    .project
                    .graph
                    .node(c.node)
                    .map_or([1, 1], |inst| natural_span(io.registry, inst));
                io.doc.edit_panels(&tr("undo-resize-on-panel"), |p, _| {
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
            _ => {}
        }
    }

    /// A double-click on the grid: a value back to its default or typed,
    /// a strip or a label renamed.
    fn double_click(&mut self, io: &mut CanvasIo, scene: &scene::Scene, hit: PanelHit, now: f64) {
        match hit {
            PanelHit::Body(i) => {
                if let Some((n, r)) = cell_view(&self.cells, scene, i)
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
            }
            PanelHit::Widget(i, k) => {
                let (Some(w), Some(c)) = (self.widget(i, k), self.cells.get(i)) else {
                    return;
                };
                if w.wired
                    || !matches!(
                        w.kind,
                        group_card::Kind::Fader { .. } | group_card::Kind::Knob
                    )
                {
                    return;
                }
                if let Some(spec) = &w.spec {
                    crate::params::set(
                        io.doc,
                        io.engine,
                        c.node,
                        &w.socket,
                        spec.default.clone(),
                        now,
                    );
                    io.doc.end_coalescing();
                }
            }
            PanelHit::Strip(s) => {
                if let Some(id) = self.strips.get(s).map(|x| x.id) {
                    self.begin_text(io, Element::Strip(id));
                }
            }
            PanelHit::Label(l) => {
                if let Some(k) = self.labels.get(l).map(|x| x.index)
                    && io
                        .doc
                        .project
                        .panels
                        .get(self.current)
                        .and_then(|p| p.labels.get(k))
                        .is_some_and(|x| !x.is_separator())
                {
                    self.begin_text(io, Element::Label(k));
                }
            }
            _ => {}
        }
    }

    /// What's under screen point `p`, topmost first.
    fn hit(&self, p: Pos2) -> PanelHit {
        for (i, s) in self.strips.iter().enumerate() {
            if !s.header.contains(p) {
                continue;
            }
            if let Some([shrink, grow, unpin]) = s.buttons {
                if shrink.expand(2.0).contains(p) {
                    return PanelHit::StripShrink(i);
                }
                if grow.expand(2.0).contains(p) {
                    return PanelHit::StripGrow(i);
                }
                if unpin.expand(2.0).contains(p) {
                    return PanelHit::StripUnpin(i);
                }
            }
            return PanelHit::Strip(i);
        }
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
            if let Some((_, rects)) = &c.group
                && let Some(k) = group_card::hit(rects, p)
            {
                return PanelHit::Widget(i, k);
            }
            if let Some((_, b)) = c.body
                && b.contains(p)
            {
                return PanelHit::Body(i);
            }
            return PanelHit::Card(i);
        }
        for (i, l) in self.labels.iter().enumerate() {
            if l.rect.contains(p) {
                return PanelHit::Label(i);
            }
        }
        PanelHit::None
    }

    /// Lay the panel out in `rect`: where each card, strip, and label goes
    /// on screen this frame (into `cells`, `strips`, `labels`). Returns the
    /// size of the whole grid, for scrolling.
    fn layout_view(
        &mut self,
        panel: &Panel,
        scene: &scene::Scene,
        io: &CanvasIo,
        rect: Rect,
    ) -> Vec2 {
        let min_w = if self.fullscreen {
            CELL_WIDTH_FULL
        } else {
            CELL_WIDTH
        };
        let avail = (rect.width() - 2.0 * MARGIN).max(min_w);
        let columns = (((avail + GAP) / (min_w + GAP)).floor() as u32).max(1);
        self.columns = columns;
        let uw = (avail - GAP * (columns - 1) as f32) / columns as f32;
        let scale = (uw / CELL_WIDTH).clamp(1.0, 1.5);
        let uh = CELL_HEIGHT * scale;
        let geometry = Geometry {
            origin: rect.min + vec2(MARGIN, MARGIN) - self.scroll,
            uw,
            uh,
            scale,
        };
        self.geometry = geometry;
        let graph = &io.doc.project.graph;
        let defs = &io.doc.project.group_defs;
        let layout = grid::layout(
            panel,
            |i| item_size(io.registry, graph, defs, panel.items.get(i)?),
            columns,
        );
        let mut cells = Vec::new();
        let mut strips = Vec::new();
        let mut labels = Vec::new();
        for pl in &layout.placed {
            let r = geometry.rect(pl.col, pl.line, pl.cols, pl.lines);
            match pl.element {
                Element::Strip(id) => {
                    let Some(s) = panel.strip(id) else { continue };
                    let members = layout
                        .placed
                        .iter()
                        .filter(|q| q.strip == Some(id))
                        .map(|q| q.lines)
                        .sum::<u32>();
                    let area = geometry
                        .rect(pl.col, pl.line, pl.cols, 1 + members)
                        .expand(5.0);
                    let buttons = (!self.fullscreen).then(|| header_buttons(r));
                    strips.push(StripCell {
                        id,
                        name: s.name.clone(),
                        header: r,
                        area,
                        buttons,
                        accent: s
                            .color
                            .as_deref()
                            .and_then(card_color)
                            .unwrap_or_else(theme::accent),
                    });
                }
                Element::Label(k) => {
                    if let Some(l) = panel.labels.get(k) {
                        labels.push(LabelCell {
                            index: k,
                            rect: r,
                            text: l.text.clone(),
                        });
                    }
                }
                Element::Item(i) => {
                    let Some(it) = panel.items.get(i) else {
                        continue;
                    };
                    let n = scene.node(it.node);
                    let accent = it
                        .color
                        .as_deref()
                        .and_then(card_color)
                        .or_else(|| n.map(|n| n.accent))
                        .unwrap_or_else(theme::missing_header);
                    let compact = it.compact;
                    let header = if compact {
                        r
                    } else {
                        Rect::from_min_size(r.min, vec2(r.width(), HEADER * scale))
                    };
                    let content = if compact {
                        Rect::from_min_max(
                            pos2(r.min.x + r.width() * 0.42, r.min.y + 4.0),
                            r.max - vec2(6.0, 4.0),
                        )
                    } else {
                        Rect::from_min_max(
                            pos2(r.min.x + PAD, header.max.y + 2.0),
                            r.max - vec2(PAD, PAD),
                        )
                    };
                    let group = n
                        .filter(|n| graph.node(n.id).is_some_and(group_card::is_group))
                        .map(|n| {
                            let w = group_card::widgets(n);
                            let total: u32 = w.iter().map(|x| x.kind.lines()).sum();
                            let line = content.height() / total.max(1) as f32;
                            let rects = group_card::place(&w, content, line);
                            (w, rects)
                        });
                    let body = if group.is_some() {
                        None
                    } else {
                        n.and_then(|n| n.body)
                            .map(|(b, _)| (b, fit_body(b, content, scale)))
                    };
                    let buttons = (!self.fullscreen && !compact).then(|| header_buttons(header));
                    cells.push(Cell {
                        node: it.node,
                        item: i,
                        rect: r,
                        header,
                        body,
                        buttons,
                        zoom: scale,
                        strip: pl.strip,
                        compact,
                        accent,
                        group,
                    });
                }
            }
        }
        self.cells = cells;
        self.strips = strips;
        self.labels = labels;
        let size = vec2(
            layout.columns as f32 * (uw + GAP) + 2.0 * MARGIN,
            layout.lines as f32 * geometry.line() + 2.0 * MARGIN,
        );
        self.layout = layout;
        size
    }
}

/// The span a panel item takes, `[cells across, lines down]`: its widget's
/// size (or its own), half a cell when compact, and for a pinned group as
/// many lines as its widgets need. `None` when its node is gone.
pub fn item_size(
    registry: &Registry,
    graph: &ac_core::Graph,
    defs: &std::collections::BTreeMap<ac_core::GroupId, ac_core::GroupDef>,
    it: &ac_core::PanelItem,
) -> Option<[u32; 2]> {
    let inst = graph.node(it.node)?;
    if group_card::is_group(inst) {
        let sig = registry.signature_of(inst, defs)?;
        let w = group_card::widgets_of(&sig, |s| {
            graph
                .wire_into(&ac_core::Endpoint::new(it.node, s))
                .is_some()
        });
        let cols = it
            .size
            .map_or(1, |s| s[0])
            .clamp(1, ac_core::panel::MAX_SPAN);
        return Some([cols, group_card::card_lines(&w)]);
    }
    let [c, r] = it.span(natural_span(registry, inst));
    Some([c, grid::item_lines(r, it.compact)])
}

/// The header buttons of a card or strip: shrink, grow, unpin.
fn header_buttons(header: Rect) -> [Rect; 3] {
    let y = header.min.y + (HEADER / 2.0).min(header.height() / 2.0);
    let at = |k: f32| {
        Rect::from_center_size(
            pos2(header.max.x - 6.0 - BUTTON * (k + 0.5) - 2.0 * k, y),
            Vec2::splat(BUTTON),
        )
    };
    [at(2.0), at(1.0), at(0.0)]
}

/// A card or strip color by name or `#RRGGBB`.
pub fn card_color(name: &str) -> Option<Color32> {
    let rgb = match name.trim().to_ascii_lowercase().as_str() {
        "purple" => Some([0xA9, 0x8B, 0xF0]),
        "grey" | "gray" => Some([0x8C, 0x92, 0x9C]),
        other => ac_nodes::display::light::parse_color(other),
    }?;
    Some(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// The Color submenu: `Some(choice)` when one was picked (`Some(None)`
/// for the category's own color).
fn color_menu(ui: &mut egui::Ui, current: Option<&str>) -> Option<Option<String>> {
    let mut picked = None;
    ui.menu_button(
        format!("{}  {}", icons::PALETTE, tr("perform-color")),
        |ui| {
            if ui
                .radio(current.is_none(), tr("perform-color-default"))
                .clicked()
            {
                picked = Some(None);
                ui.close();
            }
            for c in CARD_COLORS {
                let chosen = current == Some(*c);
                let label = match *c {
                    "amber" => tr("perform-color-amber"),
                    "green" => tr("perform-color-green"),
                    "red" => tr("perform-color-red"),
                    "blue" => tr("perform-color-blue"),
                    "purple" => tr("perform-color-purple"),
                    _ => tr("perform-color-grey"),
                };
                if ui.radio(chosen, label).clicked() {
                    picked = Some(Some(c.to_string()));
                    ui.close();
                }
            }
        },
    );
    picked
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
            | Body::Light
    )
}

/// The cells a node's widget takes by default, `[columns, rows]`: bigger
/// widgets (the Keyboard, Scope, Analyzer, Spectrogram, XY Pad, Pads) get
/// bigger cells, and a Fader is a column.
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
        Some(Body::Scope | Body::Analyzer | Body::Spectrogram) => [3, 2],
        Some(Body::Slider | Body::Meter | Body::Clip | Body::Record | Body::AudioOut) => [2, 1],
        Some(Body::Fader) => [1, 2],
        Some(Body::Pads) => {
            let (rows, cols) = ac_nodes::controls::pads::shape(&node.config);
            [
                (cols as u32).clamp(1, ac_core::panel::MAX_SPAN),
                (rows as u32).clamp(1, ac_core::panel::MAX_SPAN),
            ]
        }
        Some(_) | None => [1, 1],
    }
}

/// Where a body sits in its card's content area: pads, keys, scopes, and
/// knobs fill it; widgets that read along a line keep their height and
/// sit in the middle.
fn fit_body(body: Body, content: Rect, zoom: f32) -> Rect {
    match body {
        Body::Button
        | Body::Knob
        | Body::XyPad
        | Body::Keyboard
        | Body::Scope
        | Body::Analyzer
        | Body::Spectrogram
        | Body::Fader
        | Body::Pad
        | Body::Pads => content,
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

/// A strip: a panel behind its cards and a header with its name and (out
/// of performance mode) its buttons.
fn paint_strip(painter: &egui::Painter, s: &StripCell, zoom: f32) {
    let radius = theme::radius(theme::NODE_RADIUS + 3.0);
    painter.rect_filled(s.area, radius, theme::panel_bg());
    painter.rect_stroke(
        s.area,
        radius,
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    let bar = Rect::from_min_size(
        s.header.left_bottom() - vec2(0.0, 2.0),
        vec2(s.header.width(), 2.0),
    );
    painter.rect_filled(bar, 0.0, s.accent);
    let right = s.buttons.map_or(s.header.max.x - 8.0, |b| b[0].min.x - 4.0);
    let mut job = egui::text::LayoutJob::single_section(
        s.name.clone(),
        egui::TextFormat::simple(
            theme::ui_font(theme::FONT_NODE_TITLE * zoom.min(1.25)),
            theme::text_strong(),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - s.header.min.x - 6.0).max(8.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(
            s.header.min.x + 4.0,
            s.header.center().y - galley.size().y / 2.0,
        ),
        galley,
        theme::text_strong(),
    );
    if let Some(buttons) = s.buttons {
        for (r, icon) in buttons.iter().zip([icons::MINUS, icons::PLUS, icons::X]) {
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                icon,
                theme::ui_font(theme::FONT_SMALL + 1.0),
                theme::text_faint(),
            );
        }
    }
}

/// A label: its text as a section heading, or a line when it has none.
fn paint_label(painter: &egui::Painter, l: &LabelCell, zoom: f32) {
    if l.text.trim().is_empty() {
        let y = l.rect.center().y;
        painter.line_segment(
            [pos2(l.rect.min.x, y), pos2(l.rect.max.x, y)],
            Stroke::new(1.5, theme::border()),
        );
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        l.text.clone(),
        egui::TextFormat::simple(
            theme::ui_font(theme::FONT_HEADING * zoom.min(1.2)),
            theme::text_strong(),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(l.rect.width().max(8.0));
    let galley = painter.layout_job(job);
    let y = l.rect.max.y - galley.size().y - 2.0;
    painter.galley(pos2(l.rect.min.x + 2.0, y), galley, theme::text_strong());
    painter.line_segment(
        [l.rect.left_bottom(), l.rect.right_bottom()],
        Stroke::new(1.0, theme::border()),
    );
}

/// A card: its frame, an accent strip in its color, the node's label, and
/// (outside performance mode) its buttons. `here`: what the pointer is
/// over on this card.
fn paint_card(
    painter: &egui::Painter,
    c: &Cell,
    n: Option<&NodeView>,
    here: PanelHit,
    moving: bool,
    target: bool,
    selected: bool,
) {
    let radius = theme::radius(theme::NODE_RADIUS);
    let alpha = if moving { 0.45 } else { 1.0 };
    if c.strip.is_none() {
        painter.add(theme::node_shadow().as_shape(c.rect, radius));
    }
    painter.rect_filled(c.rect, radius, theme::fade(theme::node_bg(), alpha));
    let accent = c.accent;
    if c.compact {
        let side = Rect::from_min_size(c.rect.min, vec2(4.0, c.rect.height()));
        painter.rect_filled(
            side,
            egui::CornerRadius {
                nw: radius.nw,
                sw: radius.sw,
                ne: 0,
                se: 0,
            },
            theme::fade(accent, alpha),
        );
    } else {
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
    }
    let outline = if target || selected {
        Stroke::new(2.0, theme::accent())
    } else if matches!(here, PanelHit::Card(_)) {
        Stroke::new(1.0, theme::node_hover())
    } else {
        Stroke::new(1.0, theme::fade(theme::node_outline(), alpha))
    };
    painter.rect_stroke(c.rect, radius, outline, StrokeKind::Outside);
    let title = n.map_or_else(|| tr("perform-missing-node"), |n| n.title.clone());
    let right = if c.compact {
        c.rect.min.x + c.rect.width() * 0.42 - 4.0
    } else {
        c.buttons.map_or(c.header.max.x - 8.0, |b| b[0].min.x - 4.0)
    };
    let mut job = egui::text::LayoutJob::single_section(
        title,
        egui::TextFormat::simple(
            theme::ui_font(theme::FONT_NODE_TITLE * c.zoom.min(1.25)),
            theme::fade(theme::text_strong(), alpha),
        ),
    );
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((right - c.header.min.x - 10.0).max(8.0));
    let galley = painter.layout_job(job);
    let y = if c.compact {
        c.rect.center().y
    } else {
        c.header.center().y + 1.5
    };
    painter.galley(
        pos2(c.header.min.x + 10.0, y - galley.size().y / 2.0),
        galley,
        theme::text_strong(),
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
                        theme::text_strong()
                    } else {
                        theme::text_faint()
                    },
                    alpha,
                ),
            );
        }
    }
    if c.body.is_none() && c.group.is_none() {
        painter.text(
            c.rect.center() + vec2(0.0, HEADER / 2.0),
            Align2::CENTER_CENTER,
            tr("perform-no-widget"),
            theme::ui_font(theme::FONT_SMALL),
            theme::text_muted(),
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
        theme::text(),
    );
    let galley = painter.layout(
        text.to_string(),
        theme::ui_font(theme::FONT_UI),
        theme::text_muted(),
        (rect.width() - 80.0).clamp(120.0, 520.0),
    );
    painter.galley(
        pos2(c.x - galley.size().x / 2.0, c.y - 8.0),
        galley,
        theme::text_muted(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(span("spectrogram", serde_json::json!({})), [3, 2]);
        assert_eq!(span("keyboard", serde_json::json!({})), [3, 1]);
        let long = span("keyboard", serde_json::json!({"low": 21, "keys": 88}));
        assert_eq!(long, [ac_core::panel::MAX_SPAN, 1]);
        assert_eq!(span("fader", serde_json::json!({})), [1, 2]);
        assert_eq!(
            span("pads", serde_json::json!({"rows": 2, "columns": 8})),
            [8, 2]
        );
        assert_eq!(span("gain", serde_json::json!({})), [1, 1]);
    }

    #[test]
    fn compact_items_take_a_line_and_groups_their_widgets() {
        let registry = ac_nodes::registry();
        let mut g = ac_core::Graph::default();
        g.nodes
            .push(ac_core::NodeInstance::new(NodeId(1), "toggle"));
        let defs = std::collections::BTreeMap::new();
        let mut it = ac_core::PanelItem::new(NodeId(1));
        assert_eq!(item_size(&registry, &g, &defs, &it), Some([1, 2]));
        it.compact = true;
        assert_eq!(item_size(&registry, &g, &defs, &it), Some([1, 1]));
        assert_eq!(
            item_size(&registry, &g, &defs, &ac_core::PanelItem::new(NodeId(9))),
            None
        );
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

    #[test]
    fn card_colors_by_name() {
        assert!(CARD_COLORS.iter().all(|c| card_color(c).is_some()));
        assert_eq!(card_color("#000000"), Some(Color32::BLACK));
        assert_eq!(card_color("plaid"), None);
    }
}
