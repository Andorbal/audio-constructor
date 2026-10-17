//! Widget bodies of the control and display nodes: buttons, switches,
//! sliders, meters, scopes, readouts, clip and record status, and comment
//! sticky notes.

use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::canvas::live::MeterChannel;
use crate::icons;
use crate::theme;

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
    for mark in [-48.0f32, -24.0, -12.0, -6.0, 0.0] {
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
