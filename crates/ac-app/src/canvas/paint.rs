//! Painting the canvas: grid, frames, wires, node cards with their sockets,
//! inline widgets, badges and bodies, and gesture overlays.

use ac_core::{ConfigExt, SignalType};
use ac_engine::{Severity, WholeState};
use egui::{
    Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2,
};

use super::controls;
use super::live::{Live, source_path};
use super::scene::{Body, Class, NodeView, Scene, SocketView};
use super::view::Transform;
use super::wires::{self, WireLook};
use super::{Canvas, CanvasIo, Gesture, Hit, accepts, field_rect, frame_rect, widgets};
use crate::edit;
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::theme;

impl Canvas {
    pub(super) fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &Painter,
        io: &mut CanvasIo,
        scene: &Scene,
        now: f64,
        dt: f32,
    ) {
        let t = self.transform();
        let z = self.view.zoom;
        painter.rect_filled(self.screen, 0.0, theme::canvas_bg());
        // One copy of the graph (with interface wires) for this frame's lookups.
        self.graph_cache = io.doc.display_graph();
        paint_grid(painter, &t);
        let tele = io.engine.telemetry();
        let live = Live::new(&tele, io.doc);
        let hover = match self.pointer {
            Some(p) if self.screen.contains(p) => self.hit(scene, io.doc, p),
            _ => Hit::None,
        };
        self.portal_lit = super::portal::lit(scene, hover.node(), &io.doc.selection);

        // Frames, behind everything.
        for (i, f) in io.doc.graph().frames.iter().enumerate() {
            let r = t.rect_to_screen(frame_rect(f));
            let hovered = matches!(hover, Hit::FrameHeader(h) | Hit::FrameResize(h) if h == i);
            painter.rect_filled(
                r,
                theme::radius(theme::FRAME_RADIUS * z),
                theme::frame_fill(),
            );
            painter.rect_stroke(
                r,
                theme::radius(theme::FRAME_RADIUS * z),
                Stroke::new(
                    1.0,
                    if hovered {
                        theme::node_hover()
                    } else {
                        theme::frame_border()
                    },
                ),
                StrokeKind::Inside,
            );
            let editing = matches!(&self.edit, Some(e) if e.target == super::EditTarget::Frame(i));
            if z >= 0.25 && !editing {
                painter.text(
                    r.min + vec2(12.0 * z, theme::FRAME_HEADER * z / 2.0),
                    Align2::LEFT_CENTER,
                    format!("{}  {}", icons::FRAME_CORNERS, f.label),
                    theme::canvas_font(theme::FONT_HEADING, z, false),
                    theme::frame_label(),
                );
            }
            super::bodies::grip(
                painter,
                r.max - vec2(4.0, 4.0),
                1.0,
                theme::fade(theme::frame_label(), 0.6),
            );
        }

        // Wires, behind the nodes (cards are slightly translucent, so wires
        // passing behind them show dimmed).
        let wiring = match &self.gesture {
            Gesture::Wire {
                pending, detached, ..
            } => Some((pending.clone(), detached.clone())),
            _ => None,
        };
        let insert = match &self.gesture {
            Gesture::Move { insert, .. } => insert.clone(),
            _ => None,
        };
        let mut shapes = Vec::new();
        for w in &scene.wires {
            if wiring.as_ref().and_then(|(_, d)| d.as_ref()) == Some(&w.wire) {
                continue;
            }
            let hovered = hover == Hit::Wire(w.wire.clone()) || insert.as_ref() == Some(&w.wire);
            let mut look = WireLook {
                color: theme::type_color(w.ty),
                width: (theme::WIRE_WIDTH * z.sqrt()).max(1.0),
                glow: hovered || w.highlight,
                alpha: if wiring.is_some() { 0.55 } else { 1.0 },
                level: 0.0,
                pulse_age: None,
                time: now as f32,
            };
            if let Some((path, socket)) = source_path(io.doc, &w.wire.from) {
                match w.ty {
                    SignalType::Audio => {
                        look.level = tele
                            .socket_level(&path, &socket)
                            .filter(|l| l.is_finite())
                            .unwrap_or(0.0);
                        if look.level > 0.0005 {
                            self.live.active = true;
                        }
                    }
                    SignalType::Trigger | SignalType::Midi => {
                        if let Some(count) = tele.socket_events(&path, &socket) {
                            look.pulse_age = self.live.pulse_age((path, socket), count, now);
                        }
                    }
                    _ => {}
                }
            }
            if io.calm || io.reduce_motion {
                look.level = 0.0;
            }
            if io.reduce_motion {
                // No flashing on event wires either.
                look.pulse_age = None;
            }
            wires::paint(
                &mut shapes,
                wires::bezier(t.to_screen(w.from), t.to_screen(w.to), z),
                look,
            );
        }
        painter.extend(shapes);

        // Nodes being deleted shrink away.
        for g in &self.ghosts {
            let k = super::ease(((now - g.died) as f32) / theme::ANIM_SECONDS);
            let r = t.rect_to_screen(g.rect);
            let r = Rect::from_center_size(r.center(), r.size() * (1.0 - 0.35 * k));
            painter.rect_filled(
                r,
                theme::radius(theme::NODE_RADIUS * z),
                theme::fade(theme::node_bg(), 1.0 - k),
            );
            painter.rect_filled(
                Rect::from_min_size(
                    r.min,
                    vec2(
                        r.width(),
                        (theme::HEADER_HEIGHT * z * (1.0 - 0.35 * k)).min(r.height()),
                    ),
                ),
                theme::radius(theme::NODE_RADIUS * z),
                theme::fade(theme::header_fill(g.accent), 1.0 - k),
            );
        }

        for n in &scene.nodes {
            let alpha = self.alpha_of(n.id, now, io.calm);
            self.paint_node(
                ui,
                painter,
                io,
                &live,
                n,
                &hover,
                wiring.as_ref().map(|w| &w.0),
                alpha,
                now,
                dt,
            );
        }

        self.paint_focus(painter, scene);
        self.paint_keyboard_wire(painter, scene, now);

        // The wire being dragged, on top.
        if let Gesture::Wire { pending, snap, .. } = &self.gesture
            && let Some(p) = self.pointer
        {
            let fixed = pending
                .from
                .as_ref()
                .or(pending.to.as_ref())
                .and_then(|e| {
                    scene
                        .node(e.node)
                        .and_then(|n| n.socket(&e.socket, pending.from.is_none()))
                })
                .map(|s| t.to_screen(s.pos));
            if let Some(fixed) = fixed {
                let mut end = p;
                if let Some((node, socket, since)) = snap
                    && let Some(s) = scene
                        .node(*node)
                        .and_then(|n| n.socket(socket, pending.from.is_some()))
                {
                    let k = if io.reduce_motion {
                        1.0
                    } else {
                        super::ease(((now - since) as f32) / theme::SNAP_SECONDS)
                    };
                    end = p + (t.to_screen(s.pos) - p) * k;
                }
                let (a, b) = if pending.from.is_some() {
                    (fixed, end)
                } else {
                    (end, fixed)
                };
                let mut out = Vec::new();
                wires::paint(
                    &mut out,
                    wires::bezier(a, b, z),
                    WireLook {
                        color: theme::type_color(pending.ty),
                        width: (theme::WIRE_WIDTH * z.sqrt()).max(1.5),
                        glow: snap.is_some(),
                        alpha: 1.0,
                        level: 0.0,
                        pulse_age: None,
                        time: now as f32,
                    },
                );
                painter.extend(out);
            }
        }

        match &self.gesture {
            Gesture::BoxSelect { origin, .. } => {
                if let Some(p) = self.pointer {
                    let r = Rect::from_two_pos(t.to_screen(*origin), p);
                    painter.rect_filled(r, 3.0, theme::select_fill());
                    painter.rect_stroke(
                        r,
                        3.0,
                        Stroke::new(1.0, theme::select_stroke()),
                        StrokeKind::Inside,
                    );
                }
            }
            Gesture::Cut { points } => paint_stroke(painter, points, theme::cut_line(), true),
            Gesture::Reroute { points } => {
                paint_stroke(painter, points, theme::reroute_line(), false)
            }
            _ => {}
        }
        self.paint_minimap(painter, scene, io.doc);
        self.paint_overlay(painter, io);
    }

    /// The keyboard's focus ring: around the focused card, or the focused
    /// socket, drawn over everything so nothing hides it, in the theme's
    /// focus color (twice as thick in High Contrast).
    fn paint_focus(&self, painter: &Painter, scene: &Scene) {
        if !self.focus_visible {
            return;
        }
        let Some(f) = &self.focus else { return };
        let t = self.transform();
        let width = 2.0 * theme::outline_width();
        let stroke = Stroke::new(width, theme::focus_ring());
        match f {
            super::Focus::Node(id) => {
                let Some(n) = scene.node(*id) else { return };
                let card = t.rect_to_screen(n.rect);
                let r = if n.class == Class::Reroute {
                    theme::REROUTE_RADIUS * self.view.zoom.max(0.5) + 4.0
                } else {
                    theme::NODE_RADIUS * self.view.zoom + 4.0
                };
                painter.rect_stroke(
                    card.expand(4.0),
                    theme::radius(r),
                    stroke,
                    StrokeKind::Outside,
                );
            }
            super::Focus::Socket { node, name, input } => {
                let Some(s) = scene.node(*node).and_then(|n| n.socket(name, *input)) else {
                    return;
                };
                let r = theme::SOCKET_RADIUS * self.view.zoom.max(0.6) + 4.0;
                painter.circle_stroke(t.to_screen(s.pos), r, stroke);
            }
        }
    }

    /// A wire started from the keyboard, from its socket to wherever the
    /// focus is.
    fn paint_keyboard_wire(&self, painter: &Painter, scene: &Scene, now: f64) {
        let Some(pending) = self.keyboard_wire() else {
            return;
        };
        let t = self.transform();
        let z = self.view.zoom;
        let fixed = pending
            .from
            .as_ref()
            .or(pending.to.as_ref())
            .and_then(|e| {
                scene
                    .node(e.node)
                    .and_then(|n| n.socket(&e.socket, pending.from.is_none()))
            })
            .map(|s| t.to_screen(s.pos));
        let (Some(fixed), Some(end)) = (fixed, self.keyboard_wire_end(scene)) else {
            return;
        };
        let (a, b) = if pending.from.is_some() {
            (fixed, end)
        } else {
            (end, fixed)
        };
        let mut out = Vec::new();
        wires::paint(
            &mut out,
            wires::bezier(a, b, z),
            WireLook {
                color: theme::type_color(pending.ty),
                width: (theme::WIRE_WIDTH * z.sqrt()).max(1.5),
                glow: true,
                alpha: 0.8,
                level: 0.0,
                pulse_age: None,
                time: now as f32,
            },
        );
        painter.extend(out);
    }

    /// Breadcrumb, gesture hint, zoom readout.
    fn paint_overlay(&self, painter: &Painter, io: &CanvasIo) {
        let r = self.screen;
        if !io.doc.scope.is_empty() {
            let crumbs = io
                .doc
                .breadcrumb()
                .join(&format!("  {}  ", icons::CARET_RIGHT));
            let text = format!("{}  {crumbs}", icons::STACK);
            let galley =
                painter.layout_no_wrap(text, theme::ui_font(theme::FONT_UI), theme::text());
            let pill =
                Rect::from_min_size(r.min + vec2(12.0, 10.0), galley.size() + vec2(20.0, 10.0));
            painter.rect_filled(
                pill,
                theme::radius(pill.height() / 2.0),
                theme::breadcrumb_bg(),
            );
            painter.galley(pill.min + vec2(10.0, 5.0), galley, theme::text());
            painter.text(
                pill.right_center() + vec2(10.0, 0.0),
                Align2::LEFT_CENTER,
                tr("canvas-leave-hint"),
                theme::ui_font(theme::FONT_SMALL),
                theme::text_faint(),
            );
        }
        if let Some(h) = &self.hint {
            let galley =
                painter.layout_no_wrap(h.clone(), theme::ui_font(theme::FONT_SMALL), theme::text());
            let pill = Rect::from_center_size(
                pos2(r.center().x, r.max.y - 24.0),
                galley.size() + vec2(24.0, 10.0),
            );
            painter.rect_filled(pill, theme::radius(pill.height() / 2.0), theme::hint_bg());
            painter.galley(pill.min + vec2(12.0, 5.0), galley, theme::text());
        }
        painter.text(
            r.max - vec2(12.0, 10.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} %", self.view.zoom * 100.0),
            theme::mono_font(theme::FONT_SMALL),
            theme::text_faint(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_node(
        &mut self,
        ui: &egui::Ui,
        painter: &Painter,
        io: &mut CanvasIo,
        live: &Live,
        n: &NodeView,
        hover: &Hit,
        wiring: Option<&super::PendingWire>,
        alpha: f32,
        now: f64,
        dt: f32,
    ) {
        let t = self.transform();
        let z = self.view.zoom;
        let card = t.rect_to_screen(n.rect);
        if !card.expand(20.0).intersects(self.screen) {
            return;
        }
        let alpha = if n.muted { alpha * 0.5 } else { alpha };
        let hovered_node = hover.node() == Some(n.id);
        let mut sockets: Vec<Shape> = Vec::new();

        if let Class::Portal(info) = &n.class {
            self.paint_portal(painter, n, info, hover, wiring, alpha);
            return;
        }
        if n.class == Class::Reroute {
            let c = t.to_screen(n.rect.center());
            let ty = n.outputs.first().map_or(SignalType::Any, |s| s.ty);
            let r = theme::REROUTE_RADIUS * z.max(0.5);
            if n.selected {
                painter.circle_stroke(c, r + 3.0, Stroke::new(1.5, theme::node_selected()));
            }
            widgets::paint_socket(&mut sockets, c, r, ty, true, alpha, None);
            if hovered_node
                || n.selected
                || matches!(hover, Hit::Socket { node, .. } if *node == n.id)
            {
                // The output handle to drag a new wire from.
                let h = c + vec2(r + 8.0, 0.0);
                widgets::paint_socket(&mut sockets, h, 3.5, ty, false, alpha, None);
            }
            painter.extend(sockets);
            return;
        }

        if n.class == Class::Comment {
            let text = super::scene::comment_text(io.doc, n.id);
            let editing =
                matches!(&self.edit, Some(e) if e.target == super::EditTarget::Comment(n.id));
            let label = io.doc.graph().node(n.id).and_then(|x| x.ui.label.clone());
            super::bodies::comment(
                painter,
                card,
                z,
                label.as_deref(),
                if editing { "" } else { &text },
                n.selected,
                alpha,
            );
            super::bodies::grip(
                painter,
                card.max - vec2(3.0, 3.0),
                z.max(0.6),
                theme::fade(theme::note_text(), 0.4),
            );
            return;
        }

        // The card: shadow, body, header with its category strip, outline.
        let radius = theme::radius(theme::NODE_RADIUS * z);
        painter.add(theme::node_shadow().as_shape(card, radius));
        painter.rect_filled(card, radius, theme::fade(theme::node_bg(), alpha));
        let header = t.rect_to_screen(n.header);
        let top = if n.collapsed {
            radius
        } else {
            CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                sw: 0,
                se: 0,
            }
        };
        painter.rect_filled(
            header,
            top,
            theme::fade(theme::header_fill(n.accent), alpha),
        );
        if !n.collapsed {
            let strip = Rect::from_min_size(
                header.min,
                vec2(header.width(), (theme::HEADER_STRIP * z).max(1.0)),
            );
            painter.rect_filled(strip, top, theme::fade(n.accent, alpha));
        }
        let outline = if n.selected {
            Stroke::new(1.6, theme::fade(theme::node_selected(), alpha))
        } else if hovered_node {
            Stroke::new(1.0, theme::fade(theme::node_hover(), alpha))
        } else {
            Stroke::new(1.0, theme::fade(theme::node_outline(), alpha))
        };
        if n.selected {
            painter.rect_stroke(
                card.expand(2.5),
                radius,
                Stroke::new(3.0, theme::fade(theme::node_selected(), 0.12 * alpha)),
                StrokeKind::Outside,
            );
        }
        painter.rect_stroke(card, radius, outline, StrokeKind::Outside);

        // Header content: caret, title, badges, timing.
        let text_ok = z >= 0.3;
        let mut right = header.max.x - 8.0 * z;
        if text_ok {
            let badges = self.badges(io, live, n);
            for b in badges.iter().rev() {
                let galley = painter.layout_no_wrap(
                    b.icon.to_string(),
                    theme::canvas_font(theme::FONT_ICON, z, false),
                    theme::fade(b.color, alpha),
                );
                let w = galley.size().x;
                let pos = pos2(right - w, header.center().y - galley.size().y / 2.0);
                let r = Rect::from_min_size(pos, galley.size());
                painter.galley(pos, galley, b.color);
                self.tips.push((r.expand(2.0), b.tip.clone()));
                right -= w + 5.0 * z;
            }
            if let Some(timing) = io.timing
                && let Some(r) = timing.node(n.id)
            {
                right = self.paint_timing(painter, header, right, &r, timing, n, alpha);
            }
            let caret_w = if matches!(n.class, Class::Interface { .. }) {
                0.0
            } else {
                18.0 * z
            };
            if caret_w > 0.0 {
                let caret_hover = matches!(hover, Hit::Collapse { node } if *node == n.id);
                painter.text(
                    pos2(header.min.x + 10.0 * z, header.center().y),
                    Align2::CENTER_CENTER,
                    if n.collapsed {
                        icons::CARET_RIGHT
                    } else {
                        icons::CARET_DOWN
                    },
                    theme::canvas_font(theme::FONT_SMALL, z, false),
                    theme::fade(
                        if caret_hover {
                            theme::text_strong()
                        } else {
                            theme::text_muted()
                        },
                        alpha,
                    ),
                );
            }
            let editing_label =
                matches!(&self.edit, Some(e) if e.target == super::EditTarget::Label(n.id));
            if !editing_label {
                let prefix = match n.class {
                    Class::Group(_) => format!("{}  ", icons::STACK),
                    Class::Interface { input_side: true } => {
                        format!("{}  ", icons::ARROW_SQUARE_IN)
                    }
                    Class::Interface { input_side: false } => {
                        format!("{}  ", icons::ARROW_SQUARE_OUT)
                    }
                    _ => String::new(),
                };
                let x0 = header.min.x + caret_w + if caret_w > 0.0 { 2.0 * z } else { 10.0 * z };
                let max_w = (right - x0 - 4.0 * z).max(8.0);
                let mut job = egui::text::LayoutJob::single_section(
                    format!("{prefix}{}", n.title),
                    egui::TextFormat::simple(
                        theme::canvas_font(theme::FONT_NODE_TITLE, z, false),
                        theme::fade(theme::text_strong(), alpha),
                    ),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
                let galley = painter.layout_job(job);
                let y = header.center().y
                    + if n.collapsed {
                        0.0
                    } else {
                        theme::HEADER_STRIP * z * 0.5
                    }
                    - galley.size().y / 2.0;
                painter.galley(pos2(x0, y), galley, theme::text_strong());
            }
        }

        // Sockets and rows (bound inputs at their live values).
        let doc_node = io.shown_node(n.id);
        for s in &n.outputs {
            let p = t.to_screen(s.pos);
            if !n.collapsed && text_ok {
                let row = t.rect_to_screen(s.row);
                let label = if s.is_new_slot() {
                    tr("canvas-drop-to-add")
                } else {
                    pretty(&s.name)
                };
                let color = if s.is_new_slot() {
                    theme::text_faint()
                } else {
                    theme::text()
                };
                // Audio outputs carry a small level bar between the label
                // and the socket.
                let audio = s.ty == SignalType::Audio;
                let label_x = row.max.x - (theme::NODE_PAD_X + if audio { 7.0 } else { 0.0 }) * z;
                painter.text(
                    pos2(label_x, row.center().y),
                    Align2::RIGHT_CENTER,
                    label,
                    theme::canvas_font(theme::FONT_NODE_LABEL, z, false),
                    theme::fade(color, alpha),
                );
                if audio && let Some(level) = live.tele.socket_level(&live.path(n.id), &s.name) {
                    level_bar(
                        painter,
                        pos2(row.max.x - 10.0 * z, row.center().y),
                        z,
                        level,
                        alpha,
                    );
                    if level > 1e-4 {
                        self.live.active = true;
                    }
                }
            }
            self.paint_socket_dot(&mut sockets, p, n, s, hover, wiring, alpha);
        }

        if let Some((body, rect)) = n.body
            && !n.collapsed
        {
            self.paint_body(
                painter,
                io,
                live,
                n,
                body,
                t.rect_to_screen(rect),
                hover,
                alpha,
                now,
                dt,
            );
        }

        for s in &n.inputs {
            let p = t.to_screen(s.pos);
            if !n.collapsed && text_ok {
                self.paint_input_row(
                    ui,
                    painter,
                    io,
                    live,
                    n,
                    s,
                    doc_node.as_ref(),
                    hover,
                    alpha,
                    now,
                );
            }
            self.paint_socket_dot(&mut sockets, p, n, s, hover, wiring, alpha);
            if text_ok && let Some(b) = io.bound_input(n.id, &s.name) {
                self.paint_bound_badge(painter, p, s.wired, &b.sources, alpha);
            }
        }
        if let Some((count, row)) = n.more
            && !n.collapsed
            && text_ok
        {
            let row = t.rect_to_screen(row);
            painter.text(
                pos2(row.min.x + theme::NODE_PAD_X * z, row.center().y),
                Align2::LEFT_CENTER,
                tr_args("canvas-more-knobs", &[("count", count.into())]),
                theme::canvas_font(theme::FONT_NODE_LABEL, z, false),
                theme::fade(theme::text_faint(), alpha),
            );
            self.tips.push((row, tr("canvas-more-knobs-tip")));
        }
        painter.extend(sockets);
    }

    /// A small link left of a bound input's socket (where its wire would
    /// come in), naming what drives it on hover. A wired input ignores its
    /// binding, so the link shows in the warning color.
    fn paint_bound_badge(
        &mut self,
        painter: &Painter,
        socket: Pos2,
        wired: bool,
        sources: &[String],
        alpha: f32,
    ) {
        let z = self.view.zoom;
        let color = if wired {
            theme::warning()
        } else {
            theme::accent()
        };
        let galley = painter.layout_no_wrap(
            icons::LINK.to_string(),
            theme::canvas_font(theme::FONT_SMALL, z, false),
            theme::fade(color, alpha),
        );
        let size = galley.size();
        let pos = pos2(
            socket.x - (theme::SOCKET_RADIUS + 3.0) * z.sqrt() - size.x,
            socket.y - size.y / 2.0,
        );
        let r = Rect::from_min_size(pos, size);
        painter.galley(pos, galley, color);
        let args = [("sources", sources.join(", ").into())];
        let tip = if wired {
            tr_args("canvas-bound-but-wired", &args)
        } else {
            tr_args("canvas-bound", &args)
        };
        self.tips.push((r.expand(2.0), tip));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_socket_dot(
        &self,
        out: &mut Vec<Shape>,
        p: Pos2,
        n: &NodeView,
        s: &SocketView,
        hover: &Hit,
        wiring: Option<&super::PendingWire>,
        alpha: f32,
    ) {
        let z = self.view.zoom;
        let r = (theme::SOCKET_RADIUS * z.sqrt()).clamp(2.5, 8.0);
        let hovered = matches!(hover, Hit::Socket { node, name, input } if *node == n.id && *name == s.name && *input == s.input);
        let (alpha, ring) = match wiring {
            Some(pw) => {
                let fits = accepts(pw, n, s);
                let own = pw
                    .from
                    .as_ref()
                    .or(pw.to.as_ref())
                    .is_some_and(|e| e.node == n.id && e.socket == s.name);
                if fits {
                    (alpha, Some(theme::type_color(s.ty)))
                } else if own {
                    (alpha, None)
                } else {
                    (alpha * 0.25, None)
                }
            }
            None => (alpha, hovered.then_some(theme::text_strong())),
        };
        // Every socket is a solid dot of its type; the interface's "add"
        // slot is hollow.
        let ty = if s.is_new_slot() {
            SignalType::Any
        } else {
            s.ty
        };
        widgets::paint_socket(out, p, r, ty, !s.is_new_slot(), alpha, ring);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_input_row(
        &mut self,
        ui: &egui::Ui,
        painter: &Painter,
        io: &CanvasIo,
        live: &Live,
        n: &NodeView,
        s: &SocketView,
        node: Option<&ac_core::NodeInstance>,
        hover: &Hit,
        alpha: f32,
        now: f64,
    ) {
        let _ = (ui, now);
        let t = self.transform();
        let z = self.view.zoom;
        let row = t.rect_to_screen(s.row);
        let label = if s.is_new_slot() {
            tr("canvas-drop-to-add")
        } else {
            pretty(&s.name)
        };
        let x0 = row.min.x + theme::NODE_PAD_X * z;
        let font = theme::canvas_font(theme::FONT_NODE_LABEL, z, false);
        let controlled = n.body.is_some_and(|(b, _)| b.drives(&s.name));
        let keeps_field = s.spec.as_ref().is_some_and(widgets::shown_beside_wire);

        if s.wired && !keeps_field {
            let x0 = if s.ty == SignalType::Audio {
                x0 + 7.0 * z
            } else {
                x0
            };
            painter.text(
                pos2(x0, row.center().y),
                Align2::LEFT_CENTER,
                &label,
                font,
                theme::fade(theme::text(), alpha),
            );
            // The live value arriving on the wire, muted.
            let src = self
                .graph_cache
                .wire_into(&s.endpoint(n.id))
                .map(|w| w.from.clone());
            let reading = src.as_ref().and_then(|ep| source_path(io.doc, ep));
            let value = reading
                .as_ref()
                .and_then(|(p, sock)| live.tele.socket_value(p, sock));
            if s.ty == SignalType::Audio {
                if let Some(level) = reading
                    .as_ref()
                    .and_then(|(p, sock)| live.tele.socket_level(p, sock))
                {
                    level_bar(
                        painter,
                        pos2(row.min.x + 10.0 * z, row.center().y),
                        z,
                        level,
                        alpha,
                    );
                }
            } else if let Some(v) = value
                && !matches!(
                    s.ty,
                    SignalType::Trigger | SignalType::Midi | SignalType::Text
                )
            {
                let text = widgets::live_text(v, s.ty, s.spec.as_ref());
                let right = row.max.x - (theme::NODE_PAD_X + s.reserve) * z;
                painter.text(
                    pos2(right, row.center().y),
                    Align2::RIGHT_CENTER,
                    text,
                    theme::canvas_font(theme::FONT_VALUE, z, true),
                    theme::fade(theme::text_muted(), alpha),
                );
                // A tick on the socket's range.
                if let Some(spec) = &s.spec
                    && let Some(tpos) = widgets::normalized(f64::from(v), spec)
                {
                    let y = row.max.y - 2.5 * z;
                    let a = x0;
                    let b = right;
                    painter.line_segment(
                        [pos2(a, y), pos2(b, y)],
                        Stroke::new(1.0, theme::fade(theme::widget_bg(), alpha)),
                    );
                    let x = a + (b - a) * tpos;
                    painter.line_segment(
                        [pos2(x, y - 2.0 * z), pos2(x, y + 1.0 * z)],
                        Stroke::new(
                            1.5,
                            theme::fade(theme::type_color(SignalType::Float), alpha),
                        ),
                    );
                }
            }
            return;
        }
        let Some(spec) = s.spec.as_ref().filter(|_| !controlled) else {
            let color = if s.is_new_slot() {
                theme::text_faint()
            } else {
                theme::text_muted()
            };
            painter.text(
                pos2(x0, row.center().y),
                Align2::LEFT_CENTER,
                label,
                font,
                theme::fade(color, alpha),
            );
            return;
        };
        let Some(node) = node else { return };
        let f = t.rect_to_screen(field_rect(s));
        let field_hovered = matches!(hover, Hit::Field { node, socket } | Hit::Fire { node, socket } if *node == n.id && *socket == s.name);
        let active = self.controls.dragging(n.id, Some(&s.name))
            || matches!(&self.edit, Some(e) if matches!(&e.target, super::EditTarget::Param { node, socket, .. } if *node == n.id && *socket == s.name));
        let state = widgets::State {
            hovered: field_hovered,
            active,
            alpha,
        };
        let editing = matches!(&self.edit, Some(e) if matches!(&e.target, super::EditTarget::Param { node, socket, .. } if *node == n.id && *socket == s.name));
        match widgets::field(spec) {
            widgets::Field::Number => {
                let v = widgets::number(node, spec);
                let text = if editing {
                    String::new()
                } else {
                    widgets::format(v, spec)
                };
                widgets::paint_number(
                    painter,
                    f,
                    z,
                    &label,
                    &text,
                    widgets::normalized(v, spec),
                    state,
                );
            }
            widgets::Field::Choice => {
                let c = widgets::choice(node, spec);
                widgets::paint_choice(
                    painter,
                    f,
                    z,
                    &label,
                    &widgets::choice_label(spec, &c),
                    state,
                );
            }
            widgets::Field::Check => {
                let on = widgets::value(node, spec).as_bool().unwrap_or(false);
                widgets::paint_check(painter, f, z, &label, on, state);
            }
            widgets::Field::Fire => {
                widgets::paint_fire(painter, f, z, &label, state, 0.0);
            }
            widgets::Field::Text => {
                let text = if editing {
                    String::new()
                } else {
                    widgets::text(node, spec)
                };
                widgets::paint_text(painter, f, z, &label, &text, state);
            }
            widgets::Field::Label => {
                painter.text(
                    pos2(x0, row.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    font,
                    theme::fade(theme::text_muted(), alpha),
                );
            }
        }
    }

    /// A widget body, drawn by the code the panels share.
    #[allow(clippy::too_many_arguments)]
    fn paint_body(
        &mut self,
        painter: &Painter,
        io: &CanvasIo,
        live: &Live,
        n: &NodeView,
        body: Body,
        rect: Rect,
        hover: &Hit,
        alpha: f32,
        now: f64,
        dt: f32,
    ) {
        let editing = matches!(&self.edit, Some(e) if matches!(&e.target, super::EditTarget::Param { node, socket, .. } if *node == n.id && socket == "value"));
        let look = controls::Look {
            zoom: self.view.zoom,
            alpha,
            hovered: matches!(hover, Hit::Body { node } if *node == n.id),
            pointer: self.pointer,
            held: self.held_inputs(n.id),
            dragging: self.controls.dragging(n.id, None),
            editing,
            choice_open: matches!(&self.choice, Some((node, socket, _)) if *node == n.id && socket == "value"),
        };
        controls::paint(
            painter,
            io,
            live,
            &mut self.live,
            n,
            body,
            rect,
            &look,
            now,
            dt,
        );
    }

    /// Header badges for a node: errors, warnings, muted, missing, files.
    fn badges(&self, io: &CanvasIo, live: &Live, n: &NodeView) -> Vec<Badge> {
        let mut out = Vec::new();
        if edit::is_virtual(n.id) {
            return out;
        }
        let path = live.path(n.id);
        let (mut errors, mut warnings) = (Vec::new(), Vec::new());
        for d in io.report.for_node(&path) {
            let msg = match &d.socket {
                Some(s) => format!("{s}: {}", d.message),
                None => d.message.clone(),
            };
            match d.severity {
                Severity::Error => errors.push(msg),
                Severity::Warning => warnings.push(msg),
                Severity::Info => {}
            }
        }
        if let Class::Missing(m) = &n.class {
            out.push(Badge {
                icon: icons::QUESTION,
                color: theme::warning(),
                tip: tr_args("canvas-badge-missing", &[("kind", m.as_str().into())]),
            });
            warnings.clear();
        }
        if !errors.is_empty() {
            out.push(Badge {
                icon: icons::WARNING_CIRCLE,
                color: theme::error(),
                tip: errors.join("\n"),
            });
        }
        if !warnings.is_empty() {
            out.push(Badge {
                icon: icons::WARNING,
                color: theme::warning(),
                tip: warnings.join("\n"),
            });
        }
        if n.kind == "clip"
            && let Some(node) = io.doc.graph().node(n.id)
        {
            let file = node.config.get_str_or("file", "");
            if !file.is_empty()
                && (io.missing_assets.contains(file)
                    || live.node_value(n.id, 0).is_some_and(|v| v >= 0.5))
            {
                out.push(Badge {
                    icon: icons::FILE_X,
                    color: theme::error(),
                    tip: tr_args("canvas-badge-file-missing", &[("file", file.into())]),
                });
            }
        }
        if n.kind == ac_core::VOICES_KIND
            && let Some(node) = io.doc.graph().node(n.id)
        {
            let voices = node.config.get_u64_or("voices", 8).clamp(1, 64);
            out.push(Badge {
                icon: icons::STACK,
                color: theme::text_muted(),
                tip: tr_args("canvas-badge-voices", &[("count", voices.into())]),
            });
        }
        if let Some((_, state)) = io.report.stats.whole_input.iter().find(|(p, _)| *p == path) {
            out.push(whole_input_badge(state, io.engine.is_baking(&path)));
        }
        if let Some((_, samples)) = io
            .report
            .stats
            .node_latency
            .iter()
            .find(|(p, _)| *p == path)
        {
            let ms = f64::from(*samples) * 1000.0 / io.engine.options().sample_rate.max(1.0);
            out.push(Badge {
                icon: icons::TIMER,
                color: theme::text_muted(),
                tip: tr_args(
                    "canvas-badge-latency",
                    &[
                        ("samples", samples.to_string().into()),
                        ("ms", format!("{ms:.1}").into()),
                    ],
                ),
            });
        }
        if io.doc.scope.is_empty() {
            let on = crate::edit::pins::panels_of(&io.doc.project.panels, n.id);
            if !on.is_empty() {
                out.push(Badge {
                    icon: icons::PUSH_PIN,
                    color: theme::accent(),
                    tip: tr_args("canvas-badge-pinned", &[("panels", on.join(", ").into())]),
                });
            }
        }
        if n.muted {
            out.push(Badge {
                icon: icons::SPEAKER_SLASH,
                color: theme::text_muted(),
                tip: tr("canvas-badge-muted"),
            });
        }
        out
    }
}

struct Badge {
    icon: &'static str,
    color: Color32,
    tip: String,
}

/// The badge for a whole-input node: disabled, baking, or baked (UX.md).
fn whole_input_badge(state: &WholeState, running: bool) -> Badge {
    match state {
        WholeState::Disabled { sources } => Badge {
            icon: icons::X_CIRCLE,
            color: theme::text_muted(),
            tip: tr_args(
                "canvas-badge-disabled",
                &[("sources", sources.join(", ").into())],
            ),
        },
        WholeState::Pending | WholeState::Stale => Badge {
            icon: icons::CLOCK_COUNTER_CLOCKWISE,
            color: theme::warning(),
            tip: match (state, running) {
                (WholeState::Stale, _) => tr("canvas-badge-rebaking"),
                (_, true) => tr("canvas-badge-baking"),
                _ => tr("canvas-badge-waiting"),
            },
        },
        WholeState::Baked => Badge {
            icon: icons::CHECK_CIRCLE,
            color: theme::ok(),
            tip: tr("canvas-badge-baked"),
        },
    }
}

/// `cutoff_hz` → `cutoff hz`: socket names read better without underscores.
pub fn pretty(name: &str) -> String {
    name.replace('_', " ")
}

/// A small vertical level bar centered at `c` (screen).
fn level_bar(painter: &Painter, c: Pos2, z: f32, level: f32, alpha: f32) {
    let h = 12.0 * z;
    let w = (2.5 * z).max(1.5);
    let bg = Rect::from_center_size(c, vec2(w, h));
    painter.rect_filled(bg, 1.0, theme::fade(theme::meter_bg(), alpha));
    let db = 20.0 * level.max(1e-6).log10();
    let k = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    if k > 0.0 {
        let fill = Rect::from_min_max(pos2(bg.min.x, bg.max.y - h * k), bg.max);
        painter.rect_filled(fill, 1.0, theme::fade(theme::meter_color(db), alpha));
    }
}

fn paint_grid(painter: &Painter, t: &Transform) {
    let z = t.view.zoom;
    let mut step = theme::GRID;
    let mut major_every = 5;
    while step * z < 14.0 {
        step *= 5.0;
        major_every = 4;
    }
    let screen = t.screen;
    let tl = t.to_canvas(screen.min);
    let br = t.to_canvas(screen.max);
    let x0 = (tl.x / step).floor() as i64;
    let x1 = (br.x / step).ceil() as i64;
    let y0 = (tl.y / step).floor() as i64;
    let y1 = (br.y / step).ceil() as i64;
    if (x1 - x0) * (y1 - y0) > 20_000 {
        return;
    }
    let r = (1.1 * z.sqrt()).clamp(0.7, 1.6);
    let mut shapes = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for gx in x0..=x1 {
        for gy in y0..=y1 {
            let major = gx % major_every == 0 && gy % major_every == 0;
            let p = t.to_screen(pos2(gx as f32 * step, gy as f32 * step));
            let c = if major {
                theme::grid_dot_major()
            } else {
                theme::grid_dot()
            };
            shapes.push(Shape::circle_filled(p, if major { r * 1.35 } else { r }, c));
        }
    }
    painter.extend(shapes);
}

fn paint_stroke(painter: &Painter, points: &[Pos2], color: Color32, dashed: bool) {
    if points.len() < 2 {
        return;
    }
    if dashed {
        painter.extend(Shape::dashed_line(
            points,
            Stroke::new(1.5, color),
            6.0,
            4.0,
        ));
    } else {
        painter.add(Shape::line(points.to_vec(), Stroke::new(1.5, color)));
    }
}
