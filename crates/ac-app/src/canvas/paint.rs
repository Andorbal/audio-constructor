//! Painting the canvas: grid, frames, wires, node cards with their sockets,
//! inline widgets, badges and bodies, and gesture overlays.

use ac_core::{ConfigExt, SignalType};
use ac_engine::{Severity, WholeState};
use egui::{
    Align2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2,
};

use super::live::{Live, MeterChannel, source_path};
use super::scene::{Body, Class, NodeView, Scene, SocketView};
use super::view::Transform;
use super::wires::{self, WireLook};
use super::{Canvas, CanvasIo, Gesture, Hit, accepts, field_rect, frame_rect, widgets};
use crate::edit;
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
        painter.rect_filled(self.screen, 0.0, theme::CANVAS_BG);
        // One copy of the graph (with interface wires) for this frame's lookups.
        self.graph_cache = io.doc.display_graph();
        paint_grid(painter, &t);
        let tele = io.engine.telemetry();
        let live = Live::new(&tele, io.doc);
        let hover = match self.pointer {
            Some(p) if self.screen.contains(p) => self.hit(scene, io.doc, p),
            _ => Hit::None,
        };

        // Frames, behind everything.
        for (i, f) in io.doc.graph().frames.iter().enumerate() {
            let r = t.rect_to_screen(frame_rect(f));
            let hovered = matches!(hover, Hit::FrameHeader(h) | Hit::FrameResize(h) if h == i);
            painter.rect_filled(r, theme::radius(theme::FRAME_RADIUS * z), theme::FRAME_FILL);
            painter.rect_stroke(
                r,
                theme::radius(theme::FRAME_RADIUS * z),
                Stroke::new(
                    1.0,
                    if hovered {
                        theme::NODE_HOVER
                    } else {
                        theme::FRAME_BORDER
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
                    theme::FRAME_LABEL,
                );
            }
            super::bodies::grip(
                painter,
                r.max - vec2(4.0, 4.0),
                1.0,
                theme::fade(theme::FRAME_LABEL, 0.6),
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
            if io.calm {
                look.level = 0.0;
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
                theme::fade(theme::NODE_BG, 1.0 - k),
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
                    let k = super::ease(((now - since) as f32) / theme::SNAP_SECONDS);
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
                    painter.rect_filled(r, 3.0, theme::SELECT_FILL);
                    painter.rect_stroke(
                        r,
                        3.0,
                        Stroke::new(1.0, theme::SELECT_STROKE),
                        StrokeKind::Inside,
                    );
                }
            }
            Gesture::Cut { points } => paint_stroke(painter, points, theme::CUT_LINE, true),
            Gesture::Reroute { points } => {
                paint_stroke(painter, points, theme::REROUTE_LINE, false)
            }
            _ => {}
        }
        self.paint_overlay(painter, io);
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
            let galley = painter.layout_no_wrap(text, theme::ui_font(theme::FONT_UI), theme::TEXT);
            let pill =
                Rect::from_min_size(r.min + vec2(12.0, 10.0), galley.size() + vec2(20.0, 10.0));
            painter.rect_filled(
                pill,
                theme::radius(pill.height() / 2.0),
                Color32::from_rgba_premultiplied(0x2A, 0x3A, 0x36, 0xE8),
            );
            painter.galley(pill.min + vec2(10.0, 5.0), galley, theme::TEXT);
            painter.text(
                pill.right_center() + vec2(10.0, 0.0),
                Align2::LEFT_CENTER,
                "Tab to leave",
                theme::ui_font(theme::FONT_SMALL),
                theme::TEXT_FAINT,
            );
        }
        if let Some(h) = &self.hint {
            let galley =
                painter.layout_no_wrap(h.clone(), theme::ui_font(theme::FONT_SMALL), theme::TEXT);
            let pill = Rect::from_center_size(
                pos2(r.center().x, r.max.y - 24.0),
                galley.size() + vec2(24.0, 10.0),
            );
            painter.rect_filled(
                pill,
                theme::radius(pill.height() / 2.0),
                Color32::from_rgba_premultiplied(0x10, 0x11, 0x14, 0xD8),
            );
            painter.galley(pill.min + vec2(12.0, 5.0), galley, theme::TEXT);
        }
        painter.text(
            r.max - vec2(12.0, 10.0),
            Align2::RIGHT_BOTTOM,
            format!("{:.0} %", self.view.zoom * 100.0),
            theme::mono_font(theme::FONT_SMALL),
            theme::TEXT_FAINT,
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

        if n.class == Class::Reroute {
            let c = t.to_screen(n.rect.center());
            let ty = n.outputs.first().map_or(SignalType::Any, |s| s.ty);
            let r = theme::REROUTE_RADIUS * z.max(0.5);
            if n.selected {
                painter.circle_stroke(c, r + 3.0, Stroke::new(1.5, theme::NODE_SELECTED));
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
                theme::fade(theme::NOTE_TEXT, 0.4),
            );
            return;
        }

        // The card: shadow, body, header with its category strip, outline.
        let radius = theme::radius(theme::NODE_RADIUS * z);
        painter.add(theme::node_shadow().as_shape(card, radius));
        painter.rect_filled(card, radius, theme::fade(theme::NODE_BG, alpha));
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
            Stroke::new(1.6, theme::fade(theme::NODE_SELECTED, alpha))
        } else if hovered_node {
            Stroke::new(1.0, theme::fade(theme::NODE_HOVER, alpha))
        } else {
            Stroke::new(1.0, theme::fade(theme::NODE_OUTLINE, alpha))
        };
        if n.selected {
            painter.rect_stroke(
                card.expand(2.5),
                radius,
                Stroke::new(3.0, theme::fade(theme::NODE_SELECTED, 0.12 * alpha)),
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
            if io.show_timing
                && let Some(ns) = live.node_time_ns(n.id)
            {
                let ms = f64::from(ns) / 1e6;
                let heat = (ms / 0.2).clamp(0.0, 1.0) as f32;
                let color = theme::mix(theme::TEXT_FAINT, theme::WARNING, heat);
                let g = painter.layout_no_wrap(
                    format!("{ms:.2} ms"),
                    theme::canvas_font(theme::FONT_SMALL, z, true),
                    color,
                );
                let w = g.size().x;
                painter.galley(
                    pos2(right - w, header.center().y - g.size().y / 2.0),
                    g,
                    color,
                );
                right -= w + 6.0 * z;
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
                            theme::TEXT_STRONG
                        } else {
                            theme::TEXT_MUTED
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
                        theme::fade(theme::TEXT_STRONG, alpha),
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
                painter.galley(pos2(x0, y), galley, theme::TEXT_STRONG);
            }
        }

        // Sockets and rows.
        let doc_node = io.doc.graph().node(n.id).cloned();
        for s in &n.outputs {
            let p = t.to_screen(s.pos);
            if !n.collapsed && text_ok {
                let row = t.rect_to_screen(s.row);
                let label = if s.is_new_slot() {
                    "drop to add".to_string()
                } else {
                    pretty(&s.name)
                };
                let color = if s.is_new_slot() {
                    theme::TEXT_FAINT
                } else {
                    theme::TEXT
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
        }
        painter.extend(sockets);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_socket_dot(
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
            None => (alpha, hovered.then_some(theme::TEXT_STRONG)),
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
            "drop to add".to_string()
        } else {
            pretty(&s.name)
        };
        let x0 = row.min.x + theme::NODE_PAD_X * z;
        let font = theme::canvas_font(theme::FONT_NODE_LABEL, z, false);
        let controlled = n.body.is_some_and(|(b, _)| b.drives(&s.name));

        if s.wired {
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
                theme::fade(theme::TEXT, alpha),
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
                && !matches!(s.ty, SignalType::Trigger | SignalType::Midi)
            {
                let text = widgets::live_text(v, s.ty, s.spec.as_ref());
                let right = row.max.x - (theme::NODE_PAD_X + s.reserve) * z;
                painter.text(
                    pos2(right, row.center().y),
                    Align2::RIGHT_CENTER,
                    text,
                    theme::canvas_font(theme::FONT_VALUE, z, true),
                    theme::fade(theme::TEXT_MUTED, alpha),
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
                        Stroke::new(1.0, theme::fade(theme::WIDGET_BG, alpha)),
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
                theme::TEXT_FAINT
            } else {
                theme::TEXT_MUTED
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
        let active = matches!(&self.gesture, Gesture::DragValue { node, socket, .. } if *node == n.id && *socket == s.name)
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
            widgets::Field::Label => {
                painter.text(
                    pos2(x0, row.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    font,
                    theme::fade(theme::TEXT_MUTED, alpha),
                );
            }
        }
    }

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
        let z = self.view.zoom;
        let hovered = matches!(hover, Hit::Body { node } if *node == n.id);
        let path = live.path(n.id);
        match body {
            Body::Button => {
                let down = self.held_inputs(n.id).iter().any(|s| s == "pressed")
                    || live
                        .tele
                        .socket_value(&path, "gate")
                        .is_some_and(|v| v > 0.5);
                super::bodies::button(painter, rect, z, down, hovered, alpha);
            }
            Body::Toggle => {
                let on = live
                    .tele
                    .socket_value(&path, "out")
                    .map(|v| v > 0.5)
                    .or_else(|| {
                        io.doc
                            .graph()
                            .node(n.id)
                            .and_then(|x| x.params.get("value"))
                            .and_then(|v| v.as_bool())
                    })
                    .unwrap_or(false);
                super::bodies::toggle(painter, rect, z, on, hovered, alpha);
            }
            Body::Slider => {
                let spec = n.socket("value", true).and_then(|s| s.spec.clone());
                let node = io.doc.graph().node(n.id);
                if let (Some(spec), Some(node)) = (spec, node) {
                    let wired = n.socket("value", true).is_some_and(|s| s.wired);
                    let v = if wired {
                        live.tele.socket_value(&path, "out").map_or(0.0, f64::from)
                    } else {
                        widgets::number(node, &spec)
                    };
                    let active =
                        matches!(&self.gesture, Gesture::DragValue { node, .. } if *node == n.id);
                    super::bodies::slider(
                        painter,
                        rect,
                        z,
                        widgets::normalized(v, &spec).unwrap_or(0.0),
                        &widgets::format(v, &spec),
                        hovered,
                        active,
                        alpha,
                    );
                }
            }
            Body::Knob => {
                let input = n.socket("value", true);
                let spec = input.and_then(|s| s.spec.clone());
                let node = io.doc.graph().node(n.id);
                if let (Some(spec), Some(node)) = (spec, node) {
                    let wired = input.is_some_and(|s| s.wired);
                    let v = if wired {
                        live.tele.socket_value(&path, "out").map_or(0.0, f64::from)
                    } else {
                        widgets::number(node, &spec)
                    };
                    let active =
                        matches!(&self.gesture, Gesture::DragValue { node, .. } if *node == n.id);
                    super::bodies::knob(
                        painter,
                        rect,
                        z,
                        widgets::normalized(v, &spec).unwrap_or(0.0),
                        widgets::zero_point(&spec),
                        &widgets::format(v, &spec),
                        hovered,
                        active,
                        alpha,
                    );
                }
            }
            Body::XyPad => {
                let node = io.doc.graph().node(n.id);
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
                let touched = self.held_inputs(n.id).iter().any(|s| s == "touched")
                    || live
                        .tele
                        .socket_value(&path, "gate")
                        .is_some_and(|v| v > 0.5);
                super::bodies::xy_pad(painter, rect, z, pos, touched, hovered, alpha);
            }
            Body::Keyboard => {
                if let Some(node) = io.doc.graph().node(n.id) {
                    let (low, count) = super::piano::layout(io.registry, node);
                    let keys = super::piano::keys(low, count, rect);
                    // Keys played here, and keys a project holds down.
                    let latched = node
                        .params
                        .iter()
                        .filter(|(_, v)| v.as_bool() == Some(true))
                        .map(|(s, _)| s.clone());
                    let held: Vec<usize> = self
                        .held_inputs(n.id)
                        .into_iter()
                        .chain(latched)
                        .filter_map(|s| super::piano::key_index(&s))
                        .collect();
                    let over = match (hovered, self.pointer) {
                        (true, Some(p)) => super::piano::key_at(&keys, p),
                        _ => None,
                    };
                    // Name the keys for the octave they play in.
                    let octave = n
                        .socket("octave", true)
                        .filter(|s| !s.wired)
                        .and_then(|s| s.spec.as_ref())
                        .map_or(0, |spec| widgets::number(node, spec).round() as i32)
                        .clamp(-3, 3);
                    super::bodies::keyboard(painter, z, &keys, &held, over, 12 * octave, alpha);
                }
            }
            Body::Number => {
                let input = n.socket("value", true);
                let spec = input.and_then(|s| s.spec.clone());
                let node = io.doc.graph().node(n.id);
                if let (Some(spec), Some(node)) = (spec, node) {
                    let wired = input.is_some_and(|s| s.wired);
                    let editing = matches!(&self.edit, Some(e) if matches!(&e.target, super::EditTarget::Param { node, socket, .. } if *node == n.id && socket == "value"));
                    let active = editing
                        || matches!(&self.gesture, Gesture::DragValue { node, .. } if *node == n.id);
                    let text = if editing {
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
                    super::bodies::number(
                        painter,
                        rect,
                        z,
                        &text,
                        hovered && !wired,
                        active,
                        alpha,
                    );
                }
            }
            Body::Dropdown => {
                if let Some(node) = io.doc.graph().node(n.id) {
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
                    let open = matches!(&self.choice, Some((node, socket, _)) if *node == n.id && socket == "value");
                    super::bodies::dropdown(
                        painter,
                        rect,
                        z,
                        label,
                        hovered && !wired,
                        open,
                        alpha,
                    );
                }
            }
            Body::Meter => {
                let r = |k| live.node_value(n.id, k).unwrap_or(0.0);
                let ch = self
                    .live
                    .meter(&path, [(r(0), r(1)), (r(2), r(3))], now, dt);
                super::bodies::meter(painter, rect, z, ch, alpha);
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
                if super::bodies::scope(painter, rect, z, ring.as_deref(), start, alpha) {
                    self.live.active = true;
                }
            }
            Body::Analyzer => {
                let bands: Vec<f32> = (0..ac_nodes::display::analyzer::TELEMETRY_BANDS)
                    .map(|k| live.node_value(n.id, k).unwrap_or(f32::NEG_INFINITY))
                    .collect();
                let rate = io.engine.options().sample_rate;
                if super::bodies::analyzer(painter, rect, z, &bands, rate, alpha) {
                    self.live.active = true;
                }
            }
            Body::Readout => {
                let text = match live.node_value(n.id, 0) {
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
                };
                super::bodies::readout(painter, rect, z, &text, alpha);
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
                super::bodies::clip(painter, rect, z, &file, missing, hovered, alpha);
            }
            Body::Record => {
                let rec = live.node_value(n.id, 0).unwrap_or(0.0) > 0.5;
                if rec {
                    self.live.active = true;
                }
                if io.can_record {
                    super::bodies::record(
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
                    super::bodies::readout(painter, rect, z, "", alpha);
                    if z >= 0.4 {
                        painter.text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            "save the project to record",
                            theme::canvas_font(theme::FONT_SMALL, z, false),
                            theme::fade(theme::TEXT_MUTED, alpha),
                        );
                    }
                }
            }
            Body::AudioOut => {
                let r = |k| live.node_value(n.id, k).unwrap_or(0.0);
                let ch: [MeterChannel; 2] =
                    self.live
                        .meter(&path, [(r(0), r(0)), (r(1), r(1))], now, dt);
                super::bodies::audio_out(painter, rect, z, ch, alpha);
            }
        }
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
                color: theme::WARNING,
                tip: format!(
                    "Missing node: {m}. It's kept in the project and its wires are preserved."
                ),
            });
            warnings.clear();
        }
        if !errors.is_empty() {
            out.push(Badge {
                icon: icons::WARNING_CIRCLE,
                color: theme::ERROR,
                tip: errors.join("\n"),
            });
        }
        if !warnings.is_empty() {
            out.push(Badge {
                icon: icons::WARNING,
                color: theme::WARNING,
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
                    color: theme::ERROR,
                    tip: format!("File not found: {file}\nLocate it from the Project tab."),
                });
            }
        }
        if n.kind == ac_core::VOICES_KIND
            && let Some(node) = io.doc.graph().node(n.id)
        {
            let voices = node.config.get_u64_or("voices", 8).clamp(1, 64);
            out.push(Badge {
                icon: icons::STACK,
                color: theme::TEXT_MUTED,
                tip: format!(
                    "Polyphonic: the inside runs once per voice ({voices} voices), and notes \
                     from `midi` are shared out among them."
                ),
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
                color: theme::TEXT_MUTED,
                tip: format!(
                    "Adds {samples} samples ({ms:.1} ms) of latency; parallel paths are delayed to match."
                ),
            });
        }
        if n.muted {
            out.push(Badge {
                icon: icons::SPEAKER_SLASH,
                color: theme::TEXT_MUTED,
                tip: "Muted: passes its input through where the types match (M to unmute)".into(),
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
            color: theme::TEXT_MUTED,
            tip: format!(
                "Disabled: needs a known input, and {} is live. It plays silence while \
                 live; offline renders still work.",
                sources.join(", ")
            ),
        },
        WholeState::Pending | WholeState::Stale => Badge {
            icon: icons::CLOCK_COUNTER_CLOCKWISE,
            color: theme::WARNING,
            tip: match (state, running) {
                (WholeState::Stale, _) => {
                    "Baking: something upstream changed; the previous bake plays until \
                     the new one is ready."
                        .into()
                }
                (_, true) => "Baking: rendering its input in the background.".into(),
                _ => "Waiting to bake.".into(),
            },
        },
        WholeState::Baked => Badge {
            icon: icons::CHECK_CIRCLE,
            color: theme::OK,
            tip: "Baked: plays a render of its whole input, in time with the transport.".into(),
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
    painter.rect_filled(bg, 1.0, theme::fade(theme::METER_BG, alpha));
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
                theme::GRID_DOT_MAJOR
            } else {
                theme::GRID_DOT
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
