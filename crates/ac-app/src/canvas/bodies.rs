//! Widget bodies of the control and display nodes: buttons, switches,
//! sliders, knobs, XY pads, keyboards, number fields, dropdowns, meters,
//! scopes, readouts, clip and record status, and comment sticky notes.

use std::f32::consts::PI;

use egui::{Align2, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::canvas::live::MeterChannel;
use crate::canvas::piano;
use crate::i18n::tr;
use crate::icons;
use crate::theme;

/// A knob turns through three quarters of a circle, from 7:30 round to
/// 4:30, like most hardware.
const KNOB_SWEEP: f32 = 1.5 * PI;

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

/// A lit control's glow: a ring in the lamp color around `rect`.
pub fn lit_glow(painter: &Painter, rect: Rect, zoom: f32, color: Color32, alpha: f32) {
    let r = theme::radius(7.0 * zoom);
    painter.rect_filled(rect.expand(3.0 * zoom), r, theme::fade(color, 0.25 * alpha));
    painter.rect_stroke(
        rect.expand(1.5 * zoom),
        r,
        Stroke::new(2.0 * zoom.max(0.5), theme::fade(color, alpha)),
        StrokeKind::Outside,
    );
}

/// The color a lit control glows in.
pub fn lit_color() -> Color32 {
    theme::warning()
}

#[allow(clippy::too_many_arguments)]
pub fn button(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    down: bool,
    lit: bool,
    hovered: bool,
    alpha: f32,
) {
    if lit {
        lit_glow(painter, rect, zoom, lit_color(), alpha);
    }
    let r = theme::radius(6.0 * zoom);
    let rect = if down { rect.shrink(1.0 * zoom) } else { rect };
    let base = if down {
        theme::category_color(ac_core::Category::Controls)
    } else if hovered {
        theme::raised_hover()
    } else {
        theme::raised()
    };
    if !down {
        // A lip under the button so it reads as raised.
        painter.rect_filled(
            rect.translate(vec2(0.0, 2.0 * zoom)),
            r,
            theme::fade(theme::raised_lip(), alpha),
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
                if down {
                    tr("canvas-button-held")
                } else {
                    tr("canvas-button-press")
                }
            ),
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(
                if down {
                    theme::text_strong()
                } else {
                    theme::text()
                },
                alpha,
            ),
        );
    }
}

pub fn toggle(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    on: bool,
    lit: bool,
    hovered: bool,
    alpha: f32,
) {
    let h = (rect.height() - 8.0 * zoom).clamp(4.0, 30.0 * zoom);
    let w = h * 1.9;
    let track = Rect::from_center_size(
        pos2(rect.min.x + w / 2.0 + 2.0 * zoom, rect.center().y),
        vec2(w, h),
    );
    if lit {
        lit_glow(painter, track, zoom, lit_color(), alpha);
    }
    let on_color = theme::category_color(ac_core::Category::Controls);
    let fill = if on {
        on_color
    } else if hovered {
        theme::toggle_hover()
    } else {
        theme::toggle_off()
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
        theme::fade(theme::text_strong(), alpha),
    );
    if zoom >= 0.4 {
        painter.text(
            pos2(track.max.x + 10.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            if on { "on" } else { "off" },
            theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
            theme::fade(
                if on {
                    theme::text_strong()
                } else {
                    theme::text_muted()
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
        theme::fade(theme::widget_bg(), alpha),
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
    painter.circle_filled(knob, kr + 1.0, theme::fade(theme::body_shadow(), alpha));
    painter.circle_filled(knob, kr, theme::fade(theme::text_strong(), alpha));
    if zoom >= 0.4 {
        painter.text(
            pos2(rect.center().x, rect.min.y + 6.0 * zoom),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::text_strong(), alpha),
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
        Stroke::new(w, theme::fade(theme::widget_bg(), alpha)),
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
        theme::fade(theme::body_shadow(), alpha),
    );
    let fill = if hovered || active {
        theme::raised_hover()
    } else {
        theme::raised()
    };
    painter.circle_filled(center, cap, theme::fade(fill, alpha));
    if active {
        painter.circle_stroke(
            center,
            cap,
            Stroke::new(1.0, theme::fade(theme::accent(), alpha)),
        );
    }
    painter.line_segment(
        [
            on_circle(center, cap * 0.25, b),
            on_circle(center, cap * 0.85, b),
        ],
        Stroke::new(
            (2.0 * zoom).max(1.0),
            theme::fade(theme::text_strong(), alpha),
        ),
    );
    if text_h > 0.0 {
        painter.text(
            pos2(rect.center().x, rect.max.y - text_h / 2.0),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::text_strong(), alpha),
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
    painter.rect_filled(rect, r, theme::fade(theme::scope_bg(), alpha));
    let grid = Stroke::new(1.0, theme::fade(theme::scope_grid(), alpha));
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
                    theme::node_hover()
                } else {
                    theme::node_outline()
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
    painter.circle_filled(p, dot + 1.0, theme::fade(theme::body_shadow(), alpha));
    painter.circle_filled(
        p,
        dot,
        theme::fade(if touched { color } else { theme::text_strong() }, alpha),
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
    let edge = Stroke::new(1.0, theme::fade(theme::body_shadow(), alpha));
    for k in keys {
        let down = held.contains(&k.index);
        let fill = match (k.black, down, hovered == Some(k.index)) {
            (false, true, _) => lit,
            (false, false, true) => theme::KEY_WHITE_HOVER,
            (false, false, false) => theme::KEY_WHITE,
            (true, true, _) => theme::mix(lit, theme::body_shadow(), 0.25),
            (true, false, true) => theme::KEY_BLACK_HOVER,
            (true, false, false) => theme::KEY_BLACK,
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
                        theme::text_strong()
                    } else {
                        theme::KEY_LABEL
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
        theme::widget_bg_hover()
    } else {
        theme::well_bg()
    };
    painter.rect_filled(rect, r, theme::fade(bg, alpha));
    if active {
        painter.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, theme::fade(theme::accent(), alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom >= 0.3 && !text.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(17.0, zoom, true),
            theme::fade(theme::text_strong(), alpha),
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
        theme::widget_bg_hover()
    } else {
        theme::widget_bg()
    };
    painter.rect_filled(rect, r, theme::fade(bg, alpha));
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(
            1.0,
            theme::fade(
                if open {
                    theme::accent()
                } else {
                    theme::node_outline()
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
        theme::fade(theme::text_muted(), alpha),
    );
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(
            theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
            theme::fade(theme::text_strong(), alpha),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 3.0 * pad).max(8.0));
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(rect.min.x + pad, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_strong(),
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
            theme::fade(theme::text_faint(), alpha),
        );
    }
    let peak = ch[0].hold.max(ch[1].hold);
    let (txt, color) = if peak < 1e-5 {
        ("-∞".to_string(), theme::text_faint())
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
    painter.rect_filled(bar, r, theme::fade(theme::meter_bg(), alpha));
    let w = bar.width();
    let p = meter_pos(c.peak);
    if p > 0.0 {
        // Color by zone so the bar reads like a classic meter.
        let zones = [
            (-60.0f32, -12.0f32, theme::meter_low()),
            (-12.0, -3.0, theme::meter_mid()),
            (-3.0, 3.0, theme::meter_high()),
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

/// Lowest level the spectrogram shows, in dB.
pub const SPECTROGRAM_FLOOR_DB: f32 = -96.0;

/// A spectrogram cell's brightness for a level: 0 at the floor, 1 at 0 dB.
pub fn spectrogram_level(db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    ((db - SPECTROGRAM_FLOOR_DB) / -SPECTROGRAM_FLOOR_DB).clamp(0.0, 1.0)
}

/// A spectrogram cell's color at brightness `t` (0..1): fading in from the
/// background through the meter's low color, then on to its mid and high
/// colors near full scale, so the picture reads like the meters.
pub fn spectrogram_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let low = theme::meter_low();
    if t < 0.5 {
        theme::fade(low, t * 2.0)
    } else if t < 0.8 {
        theme::mix(low, theme::meter_mid(), (t - 0.5) / 0.3)
    } else {
        theme::mix(theme::meter_mid(), theme::meter_high(), (t - 0.8) / 0.2)
    }
}

/// The Spectrogram's picture: `ring` holds columns of `bands` levels (dB,
/// lowest band first), oldest column first, drawn left to right so the
/// newest is at the right edge, each band a row from the bottom (the bands
/// are spaced evenly in pitch, so the rows make a log frequency axis from
/// 20 Hz to Nyquist), with lines at 100 Hz, 1 kHz, and 10 kHz. Cells at
/// the floor are left out, so the mesh is only as big as what sounded.
/// Returns whether anything is above the floor (so the canvas keeps
/// repainting).
pub fn spectrogram(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    ring: Option<&[f32]>,
    bands: usize,
    sample_rate: f64,
    alpha: f32,
) -> bool {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::scope_bg(), alpha),
    );
    let nyquist = (sample_rate / 2.0).max(40.0);
    let span = (nyquist / 20.0).ln();
    let grid = theme::fade(theme::scope_grid(), alpha * 0.7);
    for hz in [100.0, 1_000.0, 10_000.0] {
        if hz < nyquist {
            let y = rect.max.y - ((hz / 20.0f64).ln() / span) as f32 * rect.height();
            painter.line_segment(
                [pos2(rect.min.x, y), pos2(rect.max.x, y)],
                Stroke::new(1.0, grid),
            );
        }
    }
    let Some(ring) = ring else {
        return false;
    };
    let bands = bands.max(1);
    let columns = ring.len() / bands;
    if columns == 0 {
        return false;
    }
    let (w, h) = (rect.width() / columns as f32, rect.height() / bands as f32);
    let mut mesh = Mesh::default();
    for (c, col) in ring.chunks_exact(bands).enumerate() {
        let x0 = rect.min.x + c as f32 * w;
        for (k, &db) in col.iter().enumerate() {
            let t = spectrogram_level(db);
            if t <= 0.0 {
                continue;
            }
            let y1 = rect.max.y - k as f32 * h;
            mesh.add_colored_rect(
                Rect::from_min_max(pos2(x0, y1 - h), pos2(x0 + w, y1)),
                theme::fade(spectrogram_color(t), alpha),
            );
        }
    }
    let any = !mesh.is_empty();
    if any {
        painter.add(Shape::mesh(mesh));
    }
    any
}

/// The scope: the ring, aligned on the last trigger when there is one.
/// Lowest level an analyzer bar shows, in dB.
pub const ANALYZER_FLOOR_DB: f32 = -96.0;

/// Bar height for a band level: 0 at the floor, 1 at 0 dB.
pub fn analyzer_height(db: f32) -> f32 {
    if db.is_nan() {
        return 0.0;
    }
    ((db - ANALYZER_FLOOR_DB) / -ANALYZER_FLOOR_DB).clamp(0.0, 1.0)
}

/// The Analyzer's bands (dB, lowest first) as bars over a log frequency
/// axis from 20 Hz to Nyquist, with lines every 24 dB and at 100 Hz, 1 kHz,
/// and 10 kHz. Returns whether anything is above the floor (so the canvas
/// keeps repainting).
pub fn analyzer(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    bands: &[f32],
    sample_rate: f64,
    alpha: f32,
) -> bool {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::scope_bg(), alpha),
    );
    let grid = theme::fade(theme::scope_grid(), alpha);
    for db in [-24.0, -48.0, -72.0] {
        let y = rect.max.y - analyzer_height(db) * rect.height();
        painter.line_segment(
            [pos2(rect.min.x, y), pos2(rect.max.x, y)],
            Stroke::new(1.0, grid),
        );
    }
    let nyquist = (sample_rate / 2.0).max(40.0);
    let span = (nyquist / 20.0).ln();
    for hz in [100.0, 1_000.0, 10_000.0] {
        if hz < nyquist {
            let x = rect.min.x + ((hz / 20.0f64).ln() / span) as f32 * rect.width();
            painter.line_segment(
                [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                Stroke::new(1.0, theme::fade(grid, 0.7)),
            );
        }
    }
    let n = bands.len().max(1);
    let w = rect.width() / n as f32;
    let mut any = false;
    for (k, &db) in bands.iter().enumerate() {
        let h = analyzer_height(db);
        if h <= 0.0 {
            continue;
        }
        any = true;
        let x0 = rect.min.x + k as f32 * w;
        let bar = Rect::from_min_max(
            pos2(x0 + (w * 0.15).min(1.0), rect.max.y - h * rect.height()),
            pos2(x0 + w - (w * 0.15).min(1.0), rect.max.y),
        );
        // Meter colors near full scale.
        let c = if db > -6.0 {
            theme::meter_high()
        } else if db > -18.0 {
            theme::meter_mid()
        } else {
            theme::meter_low()
        };
        painter.rect_filled(bar, 0.0, theme::fade(c, alpha));
    }
    any
}

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
        theme::fade(theme::scope_bg(), alpha),
    );
    let grid = theme::fade(theme::scope_grid(), alpha);
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
        theme::fade(theme::well_bg(), alpha),
    );
    if zoom >= 0.3 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(17.0, zoom, true),
            theme::fade(theme::text_strong(), alpha),
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
        theme::widget_bg_hover()
    } else {
        theme::widget_bg()
    };
    painter.rect_filled(rect, theme::radius(4.0 * zoom), theme::fade(bg, alpha));
    if missing {
        painter.rect_stroke(
            rect,
            theme::radius(4.0 * zoom),
            Stroke::new(1.0, theme::fade(theme::error(), alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom < 0.4 {
        return;
    }
    let name = if file.is_empty() {
        tr("canvas-choose-a-file")
    } else {
        std::path::Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| file.to_string())
    };
    let (icon, color) = if missing {
        (icons::FILE_X, theme::error())
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
                theme::text_muted()
            } else {
                theme::text()
            },
            alpha,
        ),
        (rect.width() - 30.0 * zoom).max(10.0),
    );
    let row = galley.rows.first().map_or(0.0, |r| r.height());
    painter.galley(
        pos2(rect.min.x + 24.0 * zoom, rect.center().y - row / 2.0),
        galley,
        theme::text(),
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
        theme::fade(theme::well_bg(), alpha),
    );
    let lamp = pos2(rect.min.x + 12.0 * zoom, rect.center().y);
    let blink = if recording {
        0.65 + 0.35 * ((time * 4.0).sin() as f32)
    } else {
        0.25
    };
    painter.circle_filled(
        lamp,
        5.0 * zoom,
        theme::fade(theme::record(), blink * alpha),
    );
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
            if recording {
                tr("canvas-recording")
            } else {
                tr("canvas-record-idle")
            },
            mins,
            secs
        ),
        theme::canvas_font(theme::FONT_VALUE, zoom, true),
        theme::fade(
            if recording {
                theme::text_strong()
            } else {
                theme::text_muted()
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
            theme::fade(theme::warning(), alpha),
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
    painter.rect_filled(rect, r, theme::fade(theme::note_bg(), alpha));
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), 20.0 * zoom));
    painter.rect_filled(
        header,
        egui::CornerRadius {
            nw: r.nw,
            ne: r.ne,
            sw: 0,
            se: 0,
        },
        theme::fade(theme::note_header(), alpha),
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
        theme::fade(theme::note_header(), alpha),
        Stroke::NONE,
    ));
    let outline = if selected {
        theme::node_selected()
    } else {
        theme::fade(theme::note_header(), 1.0)
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
        format!(
            "{}  {}",
            icons::NOTE,
            title.map_or_else(|| tr("canvas-note"), str::to_string)
        ),
        theme::canvas_font(theme::FONT_SMALL, zoom, false),
        theme::fade(theme::note_text(), 0.8 * alpha),
    );
    let body = Rect::from_min_max(
        pos2(rect.min.x + 10.0 * zoom, header.max.y + 6.0 * zoom),
        pos2(rect.max.x - 10.0 * zoom, rect.max.y - 6.0 * zoom),
    );
    let empty = tr("canvas-note-empty");
    let shown = if text.is_empty() { &empty } else { text };
    let galley = painter.layout(
        shown.to_string(),
        theme::canvas_font(theme::FONT_NODE_TITLE, zoom, false),
        theme::fade(
            if text.is_empty() {
                theme::text_muted()
            } else {
                theme::note_text()
            },
            alpha,
        ),
        body.width().max(10.0),
    );
    let clip = painter.with_clip_rect(body.intersect(painter.clip_rect()));
    clip.galley(body.min, galley, theme::note_text());
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

#[cfg(test)]
mod spectrogram_tests {
    use super::*;

    #[test]
    fn levels_map_from_the_floor_to_full_scale() {
        assert_eq!(spectrogram_level(0.0), 1.0);
        assert_eq!(spectrogram_level(SPECTROGRAM_FLOOR_DB), 0.0);
        assert_eq!(spectrogram_level(-120.0), 0.0);
        assert_eq!(spectrogram_level(f32::NAN), 0.0);
        assert!((spectrogram_level(-48.0) - 0.5).abs() < 1e-6);
        // Quiet cells fade in; loud ones are opaque.
        assert_eq!(spectrogram_color(0.0).a(), 0);
        assert!(spectrogram_color(0.25).a() < spectrogram_color(0.45).a());
        assert_eq!(spectrogram_color(1.0), theme::meter_high());
    }

    #[test]
    fn the_picture_puts_each_column_and_band_in_its_place() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 30.0));
        // Four columns of three bands: one loud cell in the newest column's
        // top band, one quieter in the oldest column's bottom band.
        let mut ring = vec![-120.0f32; 12];
        ring[9 + 2] = 0.0;
        ring[0] = -48.0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            assert!(spectrogram(
                &painter,
                rect,
                1.0,
                Some(&ring),
                3,
                48_000.0,
                1.0
            ));
            assert!(!spectrogram(
                &painter,
                rect,
                1.0,
                Some(&[-120.0; 12]),
                3,
                48_000.0,
                1.0
            ));
            assert!(!spectrogram(&painter, rect, 1.0, None, 3, 48_000.0, 1.0));
            assert!(!spectrogram(
                &painter,
                rect,
                1.0,
                Some(&[]),
                3,
                48_000.0,
                1.0
            ));
        });
        let meshes: Vec<&Mesh> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Mesh(m) => Some(m.as_ref()),
                _ => None,
            })
            .collect();
        // Only the first call had anything to draw: two cells, 10 × 10.
        assert_eq!(meshes.len(), 1);
        let m = meshes[0];
        assert_eq!(m.vertices.len(), 8);
        let cell = |i: usize| Rect::from_two_pos(m.vertices[i].pos, m.vertices[i + 3].pos);
        assert_eq!(
            cell(0),
            Rect::from_min_max(pos2(0.0, 20.0), pos2(10.0, 30.0))
        );
        assert_eq!(
            cell(4),
            Rect::from_min_max(pos2(30.0, 0.0), pos2(40.0, 10.0))
        );
        assert_eq!(m.vertices[0].color, spectrogram_color(0.5));
        assert_eq!(m.vertices[4].color, theme::meter_high());
        // No renderer here to take the font atlas.
        out.textures_delta.clear();
    }
}

#[cfg(test)]
mod analyzer_tests {
    use super::*;

    #[test]
    fn bar_heights_span_the_floor_to_full_scale() {
        assert_eq!(analyzer_height(0.0), 1.0);
        assert_eq!(analyzer_height(6.0), 1.0);
        assert_eq!(analyzer_height(ANALYZER_FLOOR_DB), 0.0);
        assert_eq!(analyzer_height(-120.0), 0.0);
        assert_eq!(analyzer_height(f32::NAN), 0.0);
        assert_eq!(analyzer_height(f32::NEG_INFINITY), 0.0);
        assert!((analyzer_height(-48.0) - 0.5).abs() < 1e-6);
    }
}

/// A lamp: dark when `level` is 0, glowing in `color` up to full at 1.
pub fn light(painter: &Painter, rect: Rect, zoom: f32, level: f32, color: Color32, alpha: f32) {
    let level = if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let r = (rect.height().min(rect.width()) / 2.0 - 2.0 * zoom).max(2.0);
    let c = pos2(rect.min.x + r + 2.0 * zoom, rect.center().y);
    if level > 0.0 {
        painter.circle_filled(c, r * 1.35, theme::fade(color, 0.22 * level * alpha));
    }
    painter.circle_filled(c, r, theme::fade(theme::meter_bg(), alpha));
    painter.circle_filled(c, r, theme::fade(color, (0.15 + 0.85 * level) * alpha));
    painter.circle_stroke(
        c,
        r,
        Stroke::new(1.0, theme::fade(theme::node_outline(), alpha)),
    );
    if zoom >= 0.4 && rect.width() > 4.0 * r {
        painter.text(
            pos2(c.x + r + 8.0 * zoom, c.y),
            Align2::LEFT_CENTER,
            if level > 0.5 {
                tr("canvas-light-on")
            } else {
                tr("canvas-light-off")
            },
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(
                if level > 0.5 {
                    theme::text_strong()
                } else {
                    theme::text_muted()
                },
                alpha,
            ),
        );
    }
}

/// A vertical meter bar, filling upwards from −60 dBFS.
pub fn meter_column(painter: &Painter, bar: Rect, zoom: f32, c: MeterChannel, alpha: f32) {
    let r = theme::radius(2.0 * zoom);
    painter.rect_filled(bar, r, theme::fade(theme::meter_bg(), alpha));
    let h = bar.height();
    let p = meter_pos(c.peak.max(c.rms));
    if p > 0.0 {
        let top = bar.max.y - h * p;
        let fill = Rect::from_min_max(pos2(bar.min.x, top), bar.max);
        painter.rect_filled(fill, r, theme::fade(theme::meter_color(db(c.peak)), alpha));
    }
    let hold = meter_pos(c.hold);
    if hold > 0.0 {
        let y = bar.max.y - h * hold;
        painter.line_segment(
            [pos2(bar.min.x, y), pos2(bar.max.x, y)],
            Stroke::new(
                1.5 * zoom.max(0.5),
                theme::fade(theme::meter_color(db(c.hold)), alpha),
            ),
        );
    }
}

/// A fader: a vertical track with a handle at `t` (0 bottom, 1 top), the
/// value above it, and a two-bar meter beside it.
#[allow(clippy::too_many_arguments)]
pub fn fader(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    t: f32,
    text: &str,
    meter: [MeterChannel; 2],
    hovered: bool,
    active: bool,
    alpha: f32,
) {
    let t = t.clamp(0.0, 1.0);
    let label_h = if zoom >= 0.4 { 16.0 * zoom } else { 0.0 };
    let area = Rect::from_min_max(pos2(rect.min.x, rect.min.y + label_h), rect.max);
    let handle_h = (10.0 * zoom).max(4.0);
    let travel = Rect::from_min_max(
        pos2(area.min.x, area.min.y + handle_h / 2.0),
        pos2(area.max.x, area.max.y - handle_h / 2.0),
    );
    let track_w = 6.0 * zoom;
    let cx = area.min.x + (area.width() * 0.4).max(14.0 * zoom);
    let track = Rect::from_center_size(
        pos2(cx, travel.center().y),
        vec2(track_w, travel.height() + handle_h),
    );
    painter.rect_filled(
        track,
        theme::radius(track_w / 2.0),
        theme::fade(theme::widget_bg(), alpha),
    );
    let y = travel.max.y - travel.height() * t;
    let fill = Rect::from_min_max(pos2(track.min.x, y), track.max);
    painter.rect_filled(
        fill,
        theme::radius(track_w / 2.0),
        theme::fade(controls_color(), alpha),
    );
    // The 0 dB mark is the caller's to place; the handle is a wide cap.
    let cap_w = (22.0 * zoom).min(area.width() * 0.7);
    let cap = Rect::from_center_size(pos2(cx, y), vec2(cap_w, handle_h));
    painter.rect_filled(
        cap.translate(vec2(0.0, 1.0 * zoom)),
        theme::radius(3.0 * zoom),
        theme::fade(theme::body_shadow(), alpha),
    );
    painter.rect_filled(
        cap,
        theme::radius(3.0 * zoom),
        theme::fade(
            if hovered || active {
                theme::text_strong()
            } else {
                theme::text()
            },
            alpha,
        ),
    );
    // The meter: two thin bars to the right of the track.
    let bar_w = 4.0 * zoom;
    let mx = (cx + cap_w / 2.0 + 6.0 * zoom).min(area.max.x - 2.0 * bar_w - 2.0 * zoom);
    for (k, c) in meter.iter().enumerate() {
        let bar = Rect::from_min_max(
            pos2(mx + k as f32 * (bar_w + 2.0 * zoom), travel.min.y),
            pos2(mx + k as f32 * (bar_w + 2.0 * zoom) + bar_w, travel.max.y),
        );
        meter_column(painter, bar, zoom, *c, alpha);
    }
    if label_h > 0.0 {
        painter.text(
            pos2(rect.center().x, rect.min.y + label_h / 2.0),
            Align2::CENTER_CENTER,
            text,
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::text_strong(), alpha),
        );
    }
}

/// A drum pad: brighter while held, glowing while lit, its last velocity
/// as a bar along the bottom.
#[allow(clippy::too_many_arguments)]
pub fn pad(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    down: bool,
    lit: bool,
    velocity: f32,
    hovered: bool,
    alpha: f32,
) {
    let r = theme::radius(6.0 * zoom);
    let rect = if down { rect.shrink(1.0 * zoom) } else { rect };
    if lit {
        lit_glow(painter, rect, zoom, lit_color(), alpha);
    }
    let base = if down {
        controls_color()
    } else if hovered {
        theme::raised_hover()
    } else {
        theme::raised()
    };
    painter.rect_filled(rect, r, theme::fade(base, alpha));
    if lit && !down {
        painter.rect_filled(
            rect.shrink(3.0 * zoom),
            r,
            theme::fade(lit_color(), 0.35 * alpha),
        );
    }
    let v = if velocity.is_finite() {
        velocity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if v > 0.0 {
        let h = 3.0 * zoom;
        let bar = Rect::from_min_size(
            pos2(rect.min.x + 4.0 * zoom, rect.max.y - h - 4.0 * zoom),
            vec2((rect.width() - 8.0 * zoom) * v, h),
        );
        painter.rect_filled(
            bar,
            theme::radius(h / 2.0),
            theme::fade(theme::text_strong(), 0.7 * alpha),
        );
    }
}

/// The rectangle of each pad of a `rows` × `columns` grid drawn in
/// `rect`, by pad number (pads count from the bottom-left). The gaps
/// follow the grid's size, so drawing and hitting agree at any zoom.
pub fn pad_rects(rect: Rect, rows: usize, columns: usize) -> Vec<Rect> {
    let (rows, columns) = (rows.max(1), columns.max(1));
    let gap = (rect.width().min(rect.height()) / (rows.max(columns) as f32) * 0.08).clamp(1.0, 6.0);
    let w = (rect.width() - gap * (columns - 1) as f32) / columns as f32;
    let h = (rect.height() - gap * (rows - 1) as f32) / rows as f32;
    let mut out = vec![Rect::NOTHING; rows * columns];
    for row in 0..rows {
        for col in 0..columns {
            let k = ac_nodes::controls::pads::pad_at(rows, columns, row, col);
            out[k] = Rect::from_min_size(
                pos2(
                    rect.min.x + col as f32 * (w + gap),
                    rect.min.y + row as f32 * (h + gap),
                ),
                vec2(w.max(1.0), h.max(1.0)),
            );
        }
    }
    out
}

/// A grid of pads: `down` and `lit` by pad number, `over` the pad under
/// the pointer.
#[allow(clippy::too_many_arguments)]
pub fn pads(
    painter: &Painter,
    rects: &[Rect],
    zoom: f32,
    down: &[bool],
    lit: &[bool],
    over: Option<usize>,
    alpha: f32,
) {
    for (k, r) in rects.iter().enumerate() {
        let is = |v: &[bool]| v.get(k).copied().unwrap_or(false);
        pad(
            painter,
            *r,
            zoom,
            is(down),
            is(lit),
            0.0,
            over == Some(k),
            alpha,
        );
    }
}

/// What a Waveform shows: the sound's overview, the playhead, and the
/// handles (start, end, loop start, loop end), as fractions of the sound.
#[derive(Clone, Debug, Default)]
pub struct WaveView {
    /// Each column's lowest and highest sample.
    pub columns: Vec<(f32, f32)>,
    pub playhead: Option<f32>,
    pub handles: Option<[f32; 4]>,
    /// Slice starts, for a Slicer.
    pub slices: Vec<f32>,
}

/// A sound's waveform with its handles: the shaded region between start
/// and end, the loop bracketed, a line at the playhead. Shared by the
/// Waveform card and the Node tab's editor for Clip, Sampler, and Slicer.
pub fn waveform(painter: &Painter, rect: Rect, zoom: f32, w: &WaveView, alpha: f32) {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::well_bg(), alpha),
    );
    let x = |f: f32| rect.min.x + f.clamp(0.0, 1.0) * rect.width();
    if let Some([s, e, ls, le]) = w.handles {
        // Outside the region dims.
        for (a, b) in [(0.0, s), (e, 1.0)] {
            if b > a {
                painter.rect_filled(
                    Rect::from_x_y_ranges(x(a)..=x(b), rect.y_range()),
                    0.0,
                    theme::fade(theme::panel_bg(), 0.55 * alpha),
                );
            }
        }
        if le > ls && (ls > 0.0 || le < 1.0) {
            let band = Rect::from_x_y_ranges(x(ls)..=x(le), rect.top()..=rect.top() + 4.0 * zoom);
            painter.rect_filled(band, 0.0, theme::fade(theme::accent(), 0.7 * alpha));
        }
    }
    let mid = rect.center().y;
    let half = rect.height() / 2.0 - 2.0 * zoom;
    let n = w.columns.len().max(1);
    let col_w = rect.width() / n as f32;
    for (k, &(lo, hi)) in w.columns.iter().enumerate() {
        let cx = rect.min.x + (k as f32 + 0.5) * col_w;
        let (lo, hi) = (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0));
        painter.line_segment(
            [pos2(cx, mid - hi * half), pos2(cx, mid - lo * half + 0.5)],
            Stroke::new(
                col_w.max(1.0),
                theme::fade(theme::type_color(ac_core::SignalType::Audio), 0.85 * alpha),
            ),
        );
    }
    for &f in &w.slices {
        painter.line_segment(
            [pos2(x(f), rect.top()), pos2(x(f), rect.bottom())],
            Stroke::new(1.0, theme::fade(theme::warning(), alpha)),
        );
    }
    if let Some([s, e, ..]) = w.handles {
        for f in [s, e] {
            let hx = x(f);
            painter.line_segment(
                [pos2(hx, rect.top()), pos2(hx, rect.bottom())],
                Stroke::new(
                    2.0 * zoom.max(0.5),
                    theme::fade(theme::text_strong(), alpha),
                ),
            );
            painter.rect_filled(
                Rect::from_center_size(pos2(hx, rect.bottom() - 4.0 * zoom), vec2(6.0, 8.0) * zoom),
                theme::radius(1.0),
                theme::fade(theme::text_strong(), alpha),
            );
        }
    }
    if let Some(p) = w.playhead {
        let px = x(p);
        painter.line_segment(
            [pos2(px, rect.top()), pos2(px, rect.bottom())],
            Stroke::new(1.5 * zoom.max(0.5), theme::fade(theme::play(), alpha)),
        );
    }
}

/// Which handle (0 start, 1 end, 2 loop start, 3 loop end) is under `x`
/// on a waveform in `rect`, if one is near: the ends win ties, so a
/// handle at the far edge can still be pulled back.
pub fn waveform_handle_at(rect: Rect, handles: [f32; 4], x: f32, reach: f32) -> Option<usize> {
    let at = |f: f32| rect.min.x + f.clamp(0.0, 1.0) * rect.width();
    [1usize, 0, 3, 2]
        .into_iter()
        .map(|k| (k, (at(handles[k]) - x).abs()))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// MIDI Monitor: the last messages, newest at the top.
pub fn midi_monitor(painter: &Painter, rect: Rect, zoom: f32, lines: &[String], alpha: f32) {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::well_bg(), alpha),
    );
    if zoom < 0.4 {
        return;
    }
    let font = theme::canvas_font(theme::FONT_SMALL, zoom, true);
    let line = (theme::FONT_SMALL + 3.0) * zoom;
    for (k, text) in lines.iter().enumerate() {
        let y = rect.top() + 4.0 * zoom + k as f32 * line;
        if y + line > rect.bottom() {
            break;
        }
        painter.text(
            pos2(rect.min.x + 6.0 * zoom, y),
            Align2::LEFT_TOP,
            text,
            font.clone(),
            theme::fade(
                if k == 0 {
                    theme::text_strong()
                } else {
                    theme::text_muted()
                },
                alpha,
            ),
        );
    }
}

/// Tuner: the note's name, and a needle from −50 to +50 cents that turns
/// green within 5 cents.
pub fn tuner(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    note: &str,
    cents: f32,
    clear: bool,
    alpha: f32,
) {
    painter.rect_filled(
        rect,
        theme::radius(4.0 * zoom),
        theme::fade(theme::well_bg(), alpha),
    );
    let cents = if cents.is_finite() {
        cents.clamp(-50.0, 50.0)
    } else {
        0.0
    };
    let in_tune = clear && cents.abs() <= 5.0;
    let scale = Rect::from_min_max(
        pos2(rect.min.x + 10.0 * zoom, rect.bottom() - 12.0 * zoom),
        pos2(rect.max.x - 10.0 * zoom, rect.bottom() - 6.0 * zoom),
    );
    painter.line_segment(
        [scale.left_center(), scale.right_center()],
        Stroke::new(1.0, theme::fade(theme::text_faint(), alpha)),
    );
    for t in [-50.0f32, -25.0, 0.0, 25.0, 50.0] {
        let x = scale.min.x + (t + 50.0) / 100.0 * scale.width();
        let h = if t == 0.0 { 6.0 } else { 3.0 } * zoom;
        painter.line_segment(
            [pos2(x, scale.center().y - h), pos2(x, scale.center().y + h)],
            Stroke::new(1.0, theme::fade(theme::text_faint(), alpha)),
        );
    }
    let nx = scale.min.x + (cents + 50.0) / 100.0 * scale.width();
    let color = if in_tune {
        theme::play()
    } else {
        theme::warning()
    };
    painter.line_segment(
        [pos2(nx, rect.top() + 6.0 * zoom), pos2(nx, scale.bottom())],
        Stroke::new(
            2.0 * zoom.max(0.5),
            theme::fade(color, if clear { alpha } else { 0.4 * alpha }),
        ),
    );
    if zoom >= 0.3 {
        painter.text(
            pos2(
                rect.center().x,
                rect.top() + (rect.height() - 16.0 * zoom) / 2.0,
            ),
            Align2::CENTER_CENTER,
            note,
            theme::canvas_font(17.0, zoom, true),
            theme::fade(
                if in_tune {
                    theme::play()
                } else {
                    theme::text_strong()
                },
                alpha,
            ),
        );
    }
}
