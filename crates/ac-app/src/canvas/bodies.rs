//! Widget bodies of the control and display nodes: buttons, switches,
//! sliders, knobs, XY pads, keyboards, number fields, dropdowns, meters,
//! scopes, readouts, clip and record status, and comment sticky notes.

use std::f32::consts::PI;

use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::canvas::live::MeterChannel;
use crate::canvas::piano;
use crate::icons;
use crate::theme;

/// A knob turns through three quarters of a circle, from 7:30 round to
/// 4:30, like most hardware.
const KNOB_SWEEP: f32 = 1.5 * PI;

/// Raised surfaces (button tops, knob caps) and their hovered shade.
const RAISED: Color32 = Color32::from_rgb(0x32, 0x36, 0x3F);
const RAISED_HOVER: Color32 = Color32::from_rgb(0x3A, 0x3F, 0x49);
const SHADOW: Color32 = Color32::from_rgb(0x15, 0x16, 0x1A);

fn controls_color() -> Color32 {
    theme::category_color(ac_core::Category::Controls)
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

/// Meter position for a linear level: -60..0 dBFS (and a little over).
fn meter_pos(x: f32) -> f32 {
    ((db(x) + 60.0) / 63.0).clamp(0.0, 1.0)
}

pub fn button(painter: &Painter, rect: Rect, zoom: f32, down: bool, hovered: bool, alpha: f32) {
    let r = theme::radius(6.0 * zoom);
    let rect = if down { rect.shrink(1.0 * zoom) } else { rect };
    let base = if down {
        theme::category_color(ac_core::Category::Controls)
    } else if hovered {
        Color32::from_rgb(0x3A, 0x3F, 0x49)
    } else {
        Color32::from_rgb(0x32, 0x36, 0x3F)
    };
    if !down {
        // A lip under the button so it reads as raised.
        painter.rect_filled(
            rect.translate(vec2(0.0, 2.0 * zoom)),
            r,
            theme::fade(Color32::from_rgb(0x1A, 0x1C, 0x21), alpha),
        );
    }
    painter.rect_filled(rect, r, theme::fade(base, alpha));
    if zoom >= 0.4 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            format!(
                "{}  {}",
                icons::HAND_TAP,
                if down { "held" } else { "press" }
            ),
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(
                if down {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT
                },
                alpha,
            ),
        );
    }
}

pub fn toggle(painter: &Painter, rect: Rect, zoom: f32, on: bool, hovered: bool, alpha: f32) {
    let h = (rect.height() - 8.0 * zoom).max(4.0);
    let w = h * 1.9;
    let track = Rect::from_center_size(
        pos2(rect.min.x + w / 2.0 + 2.0 * zoom, rect.center().y),
        vec2(w, h),
    );
    let on_color = theme::category_color(ac_core::Category::Controls);
    let fill = if on {
        on_color
    } else if hovered {
        Color32::from_rgb(0x3A, 0x3F, 0x49)
    } else {
        Color32::from_rgb(0x30, 0x34, 0x3C)
    };
    painter.rect_filled(track, theme::radius(h / 2.0), theme::fade(fill, alpha));
    let knob_x = if on {
        track.max.x - h / 2.0
    } else {
        track.min.x + h / 2.0
    };
    painter.circle_filled(
        pos2(knob_x, track.center().y),
        h / 2.0 - 2.5 * zoom,
        theme::fade(theme::TEXT_STRONG, alpha),
    );
    if zoom >= 0.4 {
        painter.text(
            pos2(track.max.x + 10.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            if on { "on" } else { "off" },
            theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
            theme::fade(
                if on {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT_MUTED
                },
                alpha,
            ),
        );
    }
}

/// A slider: a track with the value's position, and the value as text.
#[allow(clippy::too_many_arguments)]
pub fn slider(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    t: f32,
    text: &str,
    hovered: bool,
    active: bool,
    alpha: f32,
) {
    let track_h = 6.0 * zoom;
    let track = Rect::from_min_size(
        pos2(rect.min.x, rect.max.y - track_h - 3.0 * zoom),
        vec2(rect.width(), track_h),
    );
    painter.rect_filled(
        track,
        theme::radius(track_h / 2.0),
        theme::fade(theme::WIDGET_BG, alpha),
    );
    let fill = Rect::from_min_size(track.min, vec2(track.width() * t.clamp(0.0, 1.0), track_h));
    let color = theme::category_color(ac_core::Category::Controls);
    painter.rect_filled(
        fill,
        theme::radius(track_h / 2.0),
        theme::fade(color, alpha),
    );
    let knob = pos2(
        track.min.x + track.width() * t.clamp(0.0, 1.0),
        track.center().y,
    );
    let kr = if hovered || active { 6.5 } else { 5.5 } * zoom;
    painter.circle_filled(
        knob,
        kr + 1.0,
        theme::fade(Color32::from_rgb(0x15, 0x16, 0x1A), alpha),
    );
    painter.circle_filled(knob, kr, theme::fade(theme::TEXT_STRONG, alpha));
    if zoom >= 0.4 {
        painter.text(
            pos2(rect.center().x, rect.min.y + 6.0 * zoom),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::TEXT_STRONG, alpha),
        );
    }
}

/// The angle of a knob at position `t` (0..1 of its range), in radians
/// clockwise from straight up: −135° at the bottom of the range, 0 in the
/// middle, +135° at the top.
pub fn knob_angle(t: f32) -> f32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (t - 0.5) * KNOB_SWEEP
}

/// The point at `angle` (clockwise from up) on a circle.
fn on_circle(center: Pos2, r: f32, angle: f32) -> Pos2 {
    pos2(center.x + r * angle.sin(), center.y - r * angle.cos())
}

/// An arc from angle `a` to `b` as a polyline, fine enough to look round.
fn arc(center: Pos2, r: f32, a: f32, b: f32) -> Vec<Pos2> {
    let n = (((b - a).abs() / (PI / 24.0)).ceil() as usize).max(1);
    (0..=n)
        .map(|i| on_circle(center, r, a + (b - a) * i as f32 / n as f32))
        .collect()
}

/// A knob: a track arc round a cap, the value arc filled from `origin` (the
/// range's bottom, or its zero for a range that spans it) to `t`, a pointer
/// line on the cap, and the value as text underneath.
#[allow(clippy::too_many_arguments)]
pub fn knob(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    t: f32,
    origin: f32,
    text: &str,
    hovered: bool,
    active: bool,
    alpha: f32,
) {
    let text_h = if zoom >= 0.4 { 16.0 * zoom } else { 0.0 };
    let d = (rect.height() - text_h).min(rect.width()).max(4.0);
    let center = pos2(rect.center().x, rect.min.y + d / 2.0);
    let r = d / 2.0 - 2.5 * zoom;
    let w = (4.0 * zoom).max(1.0);
    painter.add(Shape::line(
        arc(center, r, knob_angle(0.0), knob_angle(1.0)),
        Stroke::new(w, theme::fade(theme::WIDGET_BG, alpha)),
    ));
    let (a, b) = (knob_angle(origin), knob_angle(t));
    if (b - a).abs() > 1e-3 {
        painter.add(Shape::line(
            arc(center, r, a, b),
            Stroke::new(w, theme::fade(controls_color(), alpha)),
        ));
    }
    let cap = (r - 6.0 * zoom).max(2.0);
    painter.circle_filled(
        center + vec2(0.0, 1.5 * zoom),
        cap,
        theme::fade(SHADOW, alpha),
    );
    let fill = if hovered || active {
        RAISED_HOVER
    } else {
        RAISED
    };
    painter.circle_filled(center, cap, theme::fade(fill, alpha));
    if active {
        painter.circle_stroke(
            center,
            cap,
            Stroke::new(1.0, theme::fade(theme::ACCENT, alpha)),
        );
    }
    painter.line_segment(
        [
            on_circle(center, cap * 0.25, b),
            on_circle(center, cap * 0.85, b),
        ],
        Stroke::new(
            (2.0 * zoom).max(1.0),
            theme::fade(theme::TEXT_STRONG, alpha),
        ),
    );
    if text_h > 0.0 {
        painter.text(
            pos2(rect.center().x, rect.max.y - text_h / 2.0),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::TEXT_STRONG, alpha),
        );
    }
}

/// An XY pad: a well with a faint grid, crosshair lines through the point
/// at `pos` (0..1 each way, y up), and the point itself, lit while touched.
pub fn xy_pad(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    pos: (f32, f32),
    touched: bool,
    hovered: bool,
    alpha: f32,
) {
    let r = theme::radius(4.0 * zoom);
    painter.rect_filled(rect, r, theme::fade(theme::SCOPE_BG, alpha));
    let grid = Stroke::new(1.0, theme::fade(theme::SCOPE_GRID, alpha));
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let x = rect.min.x + rect.width() * f;
        let y = rect.min.y + rect.height() * f;
        painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], grid);
        painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], grid);
    }
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(
            1.0,
            theme::fade(
                if hovered || touched {
                    theme::NODE_HOVER
                } else {
                    theme::NODE_OUTLINE
                },
                alpha,
            ),
        ),
        StrokeKind::Inside,
    );
    let clamp = |v: f32| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.5
        }
    };
    let p = pos2(
        rect.min.x + rect.width() * clamp(pos.0),
        rect.max.y - rect.height() * clamp(pos.1),
    );
    let color = controls_color();
    let cross = Stroke::new(1.0, theme::fade(color, 0.45 * alpha));
    painter.line_segment([pos2(p.x, rect.min.y), pos2(p.x, rect.max.y)], cross);
    painter.line_segment([pos2(rect.min.x, p.y), pos2(rect.max.x, p.y)], cross);
    let dot = if touched { 7.0 } else { 5.5 } * zoom;
    if touched {
        painter.circle_filled(p, dot * 2.0, theme::fade(color, 0.2 * alpha));
    }
    painter.circle_filled(p, dot + 1.0, theme::fade(SHADOW, alpha));
    painter.circle_filled(
        p,
        dot,
        theme::fade(if touched { color } else { theme::TEXT_STRONG }, alpha),
    );
}

/// A piano keyboard: white keys, then black keys on top, held keys lit in
/// the Controls color and the hovered key shaded. C keys carry their
/// octave name when there's room, `shift` semitones (the node's octave
/// setting) away from the key's own note.
#[allow(clippy::too_many_arguments)]
pub fn keyboard(
    painter: &Painter,
    zoom: f32,
    keys: &[piano::Key],
    held: &[usize],
    hovered: Option<usize>,
    shift: i32,
    alpha: f32,
) {
    let lit = controls_color();
    let r = theme::radius(2.0 * zoom);
    let edge = Stroke::new(1.0, theme::fade(SHADOW, alpha));
    for k in keys {
        let down = held.contains(&k.index);
        let fill = match (k.black, down, hovered == Some(k.index)) {
            (false, true, _) => lit,
            (false, false, true) => Color32::from_rgb(0xC9, 0xCD, 0xD4),
            (false, false, false) => Color32::from_rgb(0xE6, 0xE8, 0xEC),
            (true, true, _) => theme::mix(lit, SHADOW, 0.25),
            (true, false, true) => Color32::from_rgb(0x44, 0x48, 0x52),
            (true, false, false) => Color32::from_rgb(0x1E, 0x20, 0x25),
        };
        let rect = if k.black {
            k.rect
        } else {
            k.rect.shrink2(vec2(0.5, 0.0))
        };
        painter.rect_filled(
            rect,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: r.sw,
                se: r.se,
            },
            theme::fade(fill, alpha),
        );
        if !k.black {
            painter.rect_stroke(rect, 0.0, edge, StrokeKind::Outside);
        }
        if !k.black
            && zoom >= 0.6
            && k.rect.width() >= 10.0
            && let Some(name) = piano::c_label(i32::from(k.note) + shift)
        {
            painter.text(
                pos2(k.rect.center().x, k.rect.max.y - 7.0 * zoom),
                Align2::CENTER_CENTER,
                name,
                theme::canvas_font(8.5, zoom, false),
                theme::fade(
                    if down {
                        theme::TEXT_STRONG
                    } else {
                        Color32::from_rgb(0x6A, 0x70, 0x7A)
                    },
                    alpha,
                ),
            );
        }
    }
}

/// A number field: the value in large monospace in a well that reads as
/// editable (lighter when hovered, outlined while dragged).
pub fn number(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    text: &str,
    hovered: bool,
    active: bool,
    alpha: f32,
) {
    let r = theme::radius(4.0 * zoom);
    let bg = if hovered || active {
        theme::WIDGET_BG_HOVER
    } else {
        theme::WELL_BG
    };
    painter.rect_filled(rect, r, theme::fade(bg, alpha));
    if active {
        painter.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, theme::fade(theme::ACCENT, alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom >= 0.3 && !text.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(17.0, zoom, true),
            theme::fade(theme::TEXT_STRONG, alpha),
        );
    }
}

/// A dropdown: the chosen label and a caret, outlined while its menu is
/// open.
pub fn dropdown(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    hovered: bool,
    open: bool,
    alpha: f32,
) {
    let r = theme::radius(theme::WIDGET_RADIUS * zoom);
    let bg = if hovered || open {
        theme::WIDGET_BG_HOVER
    } else {
        theme::WIDGET_BG
    };
    painter.rect_filled(rect, r, theme::fade(bg, alpha));
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(
            1.0,
            theme::fade(
                if open {
                    theme::ACCENT
                } else {
                    theme::NODE_OUTLINE
                },
                alpha,
            ),
        ),
        StrokeKind::Inside,
    );
    if zoom < 0.4 {
        return;
    }
    let pad = 8.0 * zoom;
    painter.text(
        pos2(rect.max.x - pad, rect.center().y),
        Align2::RIGHT_CENTER,
        icons::CARET_DOWN,
        theme::canvas_font(theme::FONT_SMALL, zoom, false),
        theme::fade(theme::TEXT_MUTED, alpha),
    );
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(
            theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
            theme::fade(theme::TEXT_STRONG, alpha),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 3.0 * pad).max(8.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.min.x + pad, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::TEXT_STRONG,
    );
}

/// Two horizontal bars (peak with RMS inside, and a peak-hold tick) over a
/// dB scale, with the held peak as a number.
pub fn meter(painter: &Painter, rect: Rect, zoom: f32, ch: [MeterChannel; 2], alpha: f32) {
    let bar_h = 9.0 * zoom;
    let gap = 3.0 * zoom;
    let readout_w = if zoom >= 0.5 { 44.0 * zoom } else { 0.0 };
    let bars = Rect::from_min_max(rect.min, pos2(rect.max.x - readout_w, rect.max.y));
    for (i, c) in ch.iter().enumerate() {
        let y = bars.min.y + i as f32 * (bar_h + gap);
        let bar = Rect::from_min_size(pos2(bars.min.x, y), vec2(bars.width(), bar_h));
        meter_bar(painter, bar, zoom, *c, alpha);
    }
    if zoom < 0.5 {
        return;
    }
    let y = bars.min.y + 2.0 * (bar_h + gap) + 6.0 * zoom;
    for mark in [-48.0f32, -24.0, -12.0, 0.0] {
        let x = bars.min.x + bars.width() * ((mark + 60.0) / 63.0);
        painter.text(
            pos2(x, y),
            Align2::CENTER_CENTER,
            format!("{}", mark as i32),
            theme::canvas_font(9.5, zoom, true),
            theme::fade(theme::TEXT_FAINT, alpha),
        );
    }
    let peak = ch[0].hold.max(ch[1].hold);
    let (txt, color) = if peak < 1e-5 {
        ("-∞".to_string(), theme::TEXT_FAINT)
    } else {
        (format!("{:.1}", db(peak)), theme::meter_color(db(peak)))
    };
    painter.text(
        pos2(rect.max.x, bars.min.y + bar_h + gap / 2.0),
        Align2::RIGHT_CENTER,
        txt,
        theme::canvas_font(theme::FONT_VALUE, zoom, true),
        theme::fade(color, alpha),
    );
}

/// One meter bar.
pub fn meter_bar(painter: &Painter, bar: Rect, zoom: f32, c: MeterChannel, alpha: f32) {
    let r = theme::radius(2.0 * zoom);
    painter.rect_filled(bar, r, theme::fade(theme::METER_BG, alpha));
    let w = bar.width();
    let p = meter_pos(c.peak);
    if p > 0.0 {
        // Color by zone so the bar reads like a classic meter.
        let zones = [
            (-60.0f32, -12.0f32, theme::METER_LOW),
            (-12.0, -3.0, theme::METER_MID),
            (-3.0, 3.0, theme::METER_HIGH),
        ];
        for (lo, hi, color) in zones {
            let a = ((lo + 60.0) / 63.0).max(0.0);
            let b = ((hi + 60.0) / 63.0).min(p);
            if b > a {
                let zr = Rect::from_min_max(
                    pos2(bar.min.x + w * a, bar.min.y),
                    pos2(bar.min.x + w * b, bar.max.y),
                );
                painter.rect_filled(zr, 0.0, theme::fade(color, 0.55 * alpha));
            }
        }
        let rp = meter_pos(c.rms);
        if rp > 0.0 {
            let rr = Rect::from_min_max(bar.min, pos2(bar.min.x + w * rp, bar.max.y));
            painter.rect_filled(rr, r, theme::fade(theme::meter_color(db(c.rms)), alpha));
        }
    }
    let h = meter_pos(c.hold);
    if h > 0.0 {
        let x = bar.min.x + w * h;
        painter.line_segment(
            [pos2(x, bar.min.y), pos2(x, bar.max.y)],
            Stroke::new(
                1.5 * zoom.max(0.5),
                theme::fade(theme::meter_color(db(c.hold)), alpha),
            ),
        );
    }
}

/// The scope: the ring, aligned on the last trigger when there is one.
pub fn scope(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    ring: Option<&[f32]>,
    trigger_start: Option<usize>,
    alpha: f32,
) -> bool {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::SCOPE_BG, alpha),
    );
    let grid = theme::fade(theme::SCOPE_GRID, alpha);
    painter.line_segment(
        [
            pos2(rect.min.x, rect.center().y),
            pos2(rect.max.x, rect.center().y),
        ],
        Stroke::new(1.0, grid),
    );
    for i in 1..4 {
        let x = rect.min.x + rect.width() * i as f32 / 4.0;
        painter.line_segment(
            [pos2(x, rect.min.y), pos2(x, rect.max.y)],
            Stroke::new(1.0, theme::fade(grid, 0.6)),
        );
    }
    let Some(ring) = ring.filter(|r| r.len() > 1) else {
        return false;
    };
    let n = ring.len();
    // The ring arrives oldest first. With a trigger, the display starts at
    // the trigger, which keeps a periodic shape still.
    // `start`: where the last trigger sits in the oldest-first ring.
    let start = trigger_start.map_or(0, |k| k % n);
    let peak = ring.iter().fold(
        0.0f32,
        |m, v| if v.is_finite() { m.max(v.abs()) } else { m },
    );
    let scale = if peak > 1.0 { 1.0 / peak } else { 1.0 };
    let h = rect.height() * 0.45;
    let mut pts = Vec::with_capacity(n.min(512));
    let step = (n / (rect.width() as usize).max(32)).max(1);
    for i in (0..n).step_by(step) {
        let v = ring[(start + i) % n];
        let v = if v.is_finite() { v } else { 0.0 };
        pts.push(pos2(
            rect.min.x + rect.width() * i as f32 / (n - 1) as f32,
            rect.center().y - v * scale * h,
        ));
    }
    let color = theme::type_color(ac_core::SignalType::Audio);
    painter.add(Shape::line(
        pts.clone(),
        Stroke::new(3.0 * zoom.max(0.5), theme::fade(color, 0.18 * alpha)),
    ));
    painter.add(Shape::line(
        pts,
        Stroke::new(1.3 * zoom.max(0.6), theme::fade(color, alpha)),
    ));
    peak > 1e-5
}

/// A readout: the value in large monospace.
pub fn readout(painter: &Painter, rect: Rect, zoom: f32, text: &str, alpha: f32) {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::WELL_BG, alpha),
    );
    if zoom >= 0.3 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(17.0, zoom, true),
            theme::fade(theme::TEXT_STRONG, alpha),
        );
    }
}

/// Clip: the file it plays, or a prompt to choose one.
pub fn clip(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    file: &str,
    missing: bool,
    hovered: bool,
    alpha: f32,
) {
    let bg = if hovered {
        theme::WIDGET_BG_HOVER
    } else {
        theme::WIDGET_BG
    };
    painter.rect_filled(rect, theme::radius(4.0 * zoom), theme::fade(bg, alpha));
    if missing {
        painter.rect_stroke(
            rect,
            theme::radius(4.0 * zoom),
            Stroke::new(1.0, theme::fade(theme::ERROR, alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom < 0.4 {
        return;
    }
    let name = if file.is_empty() {
        "Choose a file…".to_string()
    } else {
        std::path::Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| file.to_string())
    };
    let (icon, color) = if missing {
        (icons::FILE_X, theme::ERROR)
    } else {
        (
            icons::FILE_AUDIO,
            theme::type_color(ac_core::SignalType::Audio),
        )
    };
    painter.text(
        pos2(rect.min.x + 6.0 * zoom, rect.center().y),
        Align2::LEFT_CENTER,
        icon,
        theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
        theme::fade(color, alpha),
    );
    let galley = painter.layout(
        name,
        theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
        theme::fade(
            if file.is_empty() {
                theme::TEXT_MUTED
            } else {
                theme::TEXT
            },
            alpha,
        ),
        (rect.width() - 30.0 * zoom).max(10.0),
    );
    let row = galley.rows.first().map_or(0.0, |r| r.height());
    painter.galley(
        pos2(rect.min.x + 24.0 * zoom, rect.center().y - row / 2.0),
        galley,
        theme::TEXT,
    );
}

/// Record: a REC lamp, take length, and dropped samples.
#[allow(clippy::too_many_arguments)]
pub fn record(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    recording: bool,
    length: f32,
    dropped: f32,
    time: f64,
    alpha: f32,
) {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::WELL_BG, alpha),
    );
    let lamp = pos2(rect.min.x + 12.0 * zoom, rect.center().y);
    let blink = if recording {
        0.65 + 0.35 * ((time * 4.0).sin() as f32)
    } else {
        0.25
    };
    painter.circle_filled(lamp, 5.0 * zoom, theme::fade(theme::RECORD, blink * alpha));
    if zoom < 0.4 {
        return;
    }
    let mins = (length / 60.0).floor();
    let secs = length - mins * 60.0;
    painter.text(
        pos2(lamp.x + 12.0 * zoom, rect.center().y),
        Align2::LEFT_CENTER,
        format!(
            "{}  {:.0}:{:04.1}",
            if recording { "REC" } else { "idle" },
            mins,
            secs
        ),
        theme::canvas_font(theme::FONT_VALUE, zoom, true),
        theme::fade(
            if recording {
                theme::TEXT_STRONG
            } else {
                theme::TEXT_MUTED
            },
            alpha,
        ),
    );
    if dropped > 0.0 {
        painter.text(
            pos2(rect.max.x - 6.0 * zoom, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{} {}", icons::WARNING, dropped as u64),
            theme::canvas_font(theme::FONT_SMALL, zoom, true),
            theme::fade(theme::WARNING, alpha),
        );
    }
}

/// Audio Out: two thin peak bars.
pub fn audio_out(painter: &Painter, rect: Rect, zoom: f32, ch: [MeterChannel; 2], alpha: f32) {
    let h = (rect.height() - 2.0 * zoom) / 2.0;
    for (i, c) in ch.iter().enumerate() {
        let bar = Rect::from_min_size(
            pos2(rect.min.x, rect.min.y + i as f32 * (h + 2.0 * zoom)),
            vec2(rect.width(), h),
        );
        meter_bar(painter, bar, zoom, *c, alpha);
    }
}

/// A sticky note for Comment nodes.
pub fn comment(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    title: Option<&str>,
    text: &str,
    selected: bool,
    alpha: f32,
) {
    let r = theme::radius(6.0 * zoom);
    painter.add(theme::node_shadow().as_shape(rect, r));
    painter.rect_filled(rect, r, theme::fade(theme::NOTE_BG, alpha));
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 20.0 * zoom));
    painter.rect_filled(
        header,
        egui::CornerRadius {
            nw: r.nw,
            ne: r.ne,
            sw: 0,
            se: 0,
        },
        theme::fade(theme::NOTE_HEADER, alpha),
    );
    // A folded corner, bottom right.
    let f = 14.0 * zoom;
    let c = rect.max;
    painter.add(Shape::convex_polygon(
        vec![
            pos2(c.x - f, c.y),
            pos2(c.x, c.y - f),
            pos2(c.x - f, c.y - f),
        ],
        theme::fade(theme::NOTE_HEADER, alpha),
        Stroke::NONE,
    ));
    let outline = if selected {
        theme::NODE_SELECTED
    } else {
        theme::fade(theme::NOTE_HEADER, 1.0)
    };
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(
            if selected { 1.5 } else { 1.0 },
            theme::fade(outline, alpha),
        ),
        StrokeKind::Outside,
    );
    if zoom < 0.35 {
        return;
    }
    painter.text(
        pos2(header.min.x + 8.0 * zoom, header.center().y),
        Align2::LEFT_CENTER,
        format!("{}  {}", icons::NOTE, title.unwrap_or("Note")),
        theme::canvas_font(theme::FONT_SMALL, zoom, false),
        theme::fade(theme::NOTE_TEXT, 0.8 * alpha),
    );
    let body = Rect::from_min_max(
        pos2(rect.min.x + 10.0 * zoom, header.max.y + 6.0 * zoom),
        pos2(rect.max.x - 10.0 * zoom, rect.max.y - 6.0 * zoom),
    );
    let shown = if text.is_empty() {
        "Double-click to write a note."
    } else {
        text
    };
    let galley = painter.layout(
        shown.to_string(),
        theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
        theme::fade(
            if text.is_empty() {
                theme::TEXT_MUTED
            } else {
                theme::NOTE_TEXT
            },
            alpha,
        ),
        body.width().max(10.0),
    );
    let clip = painter.with_clip_rect(body.intersect(painter.clip_rect()));
    clip.galley(body.min, galley, theme::NOTE_TEXT);
}

/// The resize grip of a resizable card (comments, frames).
pub fn grip(painter: &Painter, corner: Pos2, zoom: f32, color: Color32) {
    for i in 1..=3 {
        let d = i as f32 * 3.5 * zoom;
        painter.line_segment(
            [
                pos2(corner.x - d, corner.y - 1.5 * zoom),
                pos2(corner.x - 1.5 * zoom, corner.y - d),
            ],
            Stroke::new(1.0, color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_angles_sweep_three_quarters_of_a_turn() {
        assert!((knob_angle(0.0) + 0.75 * PI).abs() < 1e-6);
        assert_eq!(knob_angle(0.5), 0.0);
        assert!((knob_angle(1.0) - 0.75 * PI).abs() < 1e-6);
        assert_eq!(knob_angle(-2.0), knob_angle(0.0));
        assert_eq!(knob_angle(7.0), knob_angle(1.0));
        assert_eq!(knob_angle(f32::NAN), knob_angle(0.0));
        // Screen y points down: the bottom of the range sits low on the
        // left, the middle straight up, the top low on the right.
        let lo = on_circle(Pos2::ZERO, 1.0, knob_angle(0.0));
        let mid = on_circle(Pos2::ZERO, 1.0, knob_angle(0.5));
        let hi = on_circle(Pos2::ZERO, 1.0, knob_angle(1.0));
        assert!(lo.x < 0.0 && lo.y > 0.0, "{lo:?}");
        assert!(mid.x.abs() < 1e-6 && (mid.y + 1.0).abs() < 1e-6, "{mid:?}");
        assert!(hi.x > 0.0 && hi.y > 0.0, "{hi:?}");
    }

    #[test]
    fn arcs_stay_on_the_circle() {
        let c = pos2(5.0, 5.0);
        let pts = arc(c, 10.0, knob_angle(0.0), knob_angle(1.0));
        assert!(pts.len() > 20);
        assert!(pts.iter().all(|p| ((*p - c).length() - 10.0).abs() < 1e-3));
        assert_eq!(pts[0], on_circle(c, 10.0, knob_angle(0.0)));
        // A zero-length arc is still a valid line.
        assert_eq!(arc(c, 10.0, 1.0, 1.0).len(), 2);
    }
}
