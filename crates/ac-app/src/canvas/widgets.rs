//! Inline widgets for unwired inputs (UX.md "Node anatomy"): the value
//! logic (formatting, parsing, drag mapping) and how each widget is painted.
//! Drawn by hand at the canvas zoom so text stays crisp at every scale.

use ac_core::{AutoTrigger, NodeInstance, SignalType, SocketSpec, Unit, Value};
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::icons;
use crate::theme;

/// Which widget an unwired input gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Drag-slider for `Float` and plain `Int` inputs.
    Number,
    /// Dropdown of named choices (enumerated `Int`s, start/stop modes).
    Choice,
    /// Checkbox for `Bool` inputs.
    Check,
    /// A small button that fires the trigger.
    Fire,
    /// A text field for `Text` inputs (an address, a template).
    Text,
    /// No widget (audio, MIDI, or unknown sockets).
    Label,
}

pub fn field(spec: &SocketSpec) -> Field {
    if !spec.choices.is_empty() {
        return Field::Choice;
    }
    match spec.ty {
        SignalType::Float | SignalType::Int => Field::Number,
        SignalType::Bool => Field::Check,
        SignalType::Trigger => Field::Fire,
        SignalType::Text => Field::Text,
        _ => Field::Label,
    }
}

/// The stored value of an input, or its default.
pub fn value(node: &NodeInstance, spec: &SocketSpec) -> Value {
    node.params
        .get(spec.name.as_ref())
        .cloned()
        .unwrap_or_else(|| spec.default.clone())
}

pub fn number(node: &NodeInstance, spec: &SocketSpec) -> f64 {
    let v = value(node, spec);
    v.as_f64()
        .filter(|f| f.is_finite())
        .or_else(|| spec.default.as_f64())
        .unwrap_or(0.0)
}

/// A stored number kept within the node's `min`/`max` config, the way a
/// Number node puts it out. Either bound may be missing (so the socket's
/// range, which needs both, can't say); a reversed pair is swapped, and an
/// `Int` number's bounds round to whole numbers.
pub fn bounded_number(node: &NodeInstance, spec: &SocketSpec) -> f64 {
    let v = number(node, spec);
    let int = spec.ty == SignalType::Int;
    let bound = |key: &str| {
        let b = node.config.get(key)?.as_f64().filter(|b| b.is_finite())?;
        Some(if int { b.round() } else { b })
    };
    let (mut lo, mut hi) = (bound("min"), bound("max"));
    if let (Some(a), Some(b)) = (lo, hi)
        && a > b
    {
        (lo, hi) = (Some(b), Some(a));
    }
    let v = lo.map_or(v, |lo| v.max(lo));
    hi.map_or(v, |hi| v.min(hi))
}

/// The text of a `Text` input, as the engine reads it.
pub fn text(node: &NodeInstance, spec: &SocketSpec) -> String {
    ac_core::param_text(spec, node.params.get(spec.name.as_ref()))
        .as_str()
        .to_string()
}

/// The selected choice's name.
pub fn choice(node: &NodeInstance, spec: &SocketSpec) -> String {
    let idx = ac_core::param_value(spec, node.params.get(spec.name.as_ref()));
    spec.choices
        .get(idx.max(0.0) as usize)
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Display text for a choice: start/stop modes read as sentences.
pub fn choice_label(spec: &SocketSpec, name: &str) -> String {
    match (spec.auto, name) {
        (AutoTrigger::Start, "play") => "On play".into(),
        (AutoTrigger::Start, "load") => "On load".into(),
        (AutoTrigger::Stop, "stop") => "On stop".into(),
        (_, "never") => "Never".into(),
        _ => {
            let mut s = name.replace('_', " ");
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            s
        }
    }
}

/// Log mapping for a socket's range (Hz ranges and explicit log scales;
/// bindings sweep the same way).
fn is_log(spec: &SocketSpec) -> Option<(f64, f64)> {
    let (a, b) = spec.range?;
    let log = ac_core::binding::log_scaled(spec);
    (log && a > 0.0 && b > a).then_some((a, b))
}

/// Position of `v` within the socket's soft range, 0..1.
pub fn normalized(v: f64, spec: &SocketSpec) -> Option<f32> {
    let (a, b) = spec.range?;
    if b <= a {
        return None;
    }
    let t = match is_log(spec) {
        Some((a, b)) => (v.max(a).ln() - a.ln()) / (b.ln() - a.ln()),
        None => (v - a) / (b - a),
    };
    Some(t.clamp(0.0, 1.0) as f32)
}

/// Where a value's fill starts within its range, 0..1: at zero for a
/// linear range that spans it (a −12..12 detune fills from the middle),
/// otherwise at the bottom.
pub fn zero_point(spec: &SocketSpec) -> f32 {
    match spec.range {
        Some((a, b)) if a < 0.0 && b > 0.0 && is_log(spec).is_none() => {
            normalized(0.0, spec).unwrap_or(0.0)
        }
        _ => 0.0,
    }
}

/// A value at position `t` (0..1) of a socket's range, linear or log as
/// the socket is drawn; `None` without a range.
pub fn at_position(t: f64, spec: &SocketSpec) -> Option<f64> {
    denormalize(t, spec)
}

fn denormalize(t: f64, spec: &SocketSpec) -> Option<f64> {
    let (a, b) = spec.range?;
    // The ends land exactly on the range limits (no exp/ln round-off).
    if t <= 0.0 {
        return Some(a);
    }
    if t >= 1.0 {
        return Some(b);
    }
    Some(match is_log(spec) {
        Some((a, b)) => (a.ln() + t * (b.ln() - a.ln())).exp(),
        None => a + t * (b - a),
    })
}

/// The value after dragging `dx` screen points from `start` on a widget
/// `width` points wide. A full-width drag sweeps the soft range; Ctrl is
/// ten times finer, Shift five times coarser. Unbounded values move in
/// proportion to their size.
pub fn dragged(
    start: f64,
    dx: f32,
    width: f32,
    spec: &SocketSpec,
    fine: bool,
    coarse: bool,
) -> f64 {
    let speed = if fine {
        0.1
    } else if coarse {
        5.0
    } else {
        1.0
    };
    let dx = f64::from(dx) * speed;
    let v = match (normalized(start, spec), spec.range) {
        (Some(t), Some((a, b))) => {
            let t = f64::from(t) + dx / f64::from(width.max(20.0));
            denormalize(t.clamp(0.0, 1.0), spec)
                .unwrap_or(start)
                .clamp(a, b)
        }
        _ => {
            let step = if spec.ty == SignalType::Int {
                0.1
            } else {
                0.005 * (start.abs() + 1.0)
            };
            start + dx * step
        }
    };
    if spec.ty == SignalType::Int {
        v.round()
    } else {
        v
    }
}

/// A float as the user would like to read it, with its unit.
pub fn format(v: f64, spec: &SocketSpec) -> String {
    if spec.ty == SignalType::Int {
        return format!("{}", v.round() as i64);
    }
    if !v.is_finite() {
        return if v.is_nan() {
            "NaN".into()
        } else if v > 0.0 {
            "∞".into()
        } else {
            "-∞".into()
        };
    }
    match spec.unit {
        Unit::Hz if v.abs() >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
        Unit::Hz => format!("{} Hz", trim(v, if v.abs() < 10.0 { 2 } else { 1 })),
        Unit::Db if v <= -120.0 => "-∞ dB".into(),
        Unit::Db => format!("{} dB", trim(v, 1)),
        Unit::Seconds if v.abs() < 1.0 => format!("{} ms", trim(v * 1000.0, 1)),
        Unit::Seconds => format!("{} s", trim(v, 2)),
        Unit::Beats => format!("{} beats", trim(v, 2)),
        Unit::Ratio => format!("{}×", trim(v, 2)),
        Unit::Semitones => format!("{:+} st", trim_f(v, 1)),
        Unit::Bpm => format!("{} BPM", trim(v, 1)),
        Unit::Samples => format!("{} smp", v.round() as i64),
        Unit::Generic => trim(v, 3),
    }
}

fn trim_f(v: f64, decimals: usize) -> f64 {
    let p = 10f64.powi(decimals as i32);
    (v * p).round() / p
}

fn trim(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" { "0".into() } else { t.to_string() }
    } else {
        s
    }
}

/// Parse typed text: a number with an optional unit (`440`, `1.2k`,
/// `1.2 kHz`, `-6 dB`, `250 ms`, `0.5 s`). `None` if it isn't a number.
/// A `Text` input takes the text as typed, trimmed.
pub fn parse(text: &str, spec: &SocketSpec) -> Option<Value> {
    if spec.ty == SignalType::Text {
        return Some(Value::Text(text.trim().to_string()));
    }
    let t = text.trim().to_lowercase().replace(',', ".");
    if t.is_empty() {
        return None;
    }
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e'))
        .unwrap_or(t.len());
    // `e` belongs to the number only when followed by a digit or sign.
    let (num, unit) = t.split_at(split);
    let mut v: f64 = num.trim().parse().ok()?;
    let unit = unit.trim();
    match unit {
        "" => {}
        "k" | "khz" => v *= 1000.0,
        "ms" => v /= 1000.0,
        "hz" | "db" | "s" | "st" | "bpm" | "smp" | "x" | "×" | "beats" | "beat" => {}
        _ => return None,
    }
    if !v.is_finite() {
        return None;
    }
    Some(if spec.ty == SignalType::Int {
        Value::Int(v.round() as i64)
    } else {
        Value::Float(v)
    })
}

/// Text for a live value on a wired input or a readout.
pub fn live_text(v: f32, ty: SignalType, spec: Option<&SocketSpec>) -> String {
    match ty {
        SignalType::Bool => {
            if v > 0.5 {
                "on".into()
            } else {
                "off".into()
            }
        }
        SignalType::Int => format!("{}", v.round() as i64),
        _ => match spec {
            Some(s) if s.ty == SignalType::Float => format(f64::from(v), s),
            _ => trim(f64::from(v), 3),
        },
    }
}

// ---------------------------------------------------------------- painting

/// Interaction state of a widget, for its look.
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    pub hovered: bool,
    pub active: bool,
    pub alpha: f32,
}

fn bg(state: State) -> Color32 {
    let c = if state.hovered || state.active {
        theme::widget_bg_hover()
    } else {
        theme::widget_bg()
    };
    theme::fade(c, state.alpha)
}

/// A drag-slider: label on the left, value on the right, the fill showing
/// the position within the soft range.
pub fn paint_number(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    value: &str,
    fill: Option<f32>,
    state: State,
) {
    let r = theme::radius(theme::WIDGET_RADIUS * zoom);
    painter.rect_filled(rect, r, bg(state));
    if let Some(t) = fill {
        let w = rect.width() * t;
        if w > 0.5 {
            let fr = Rect::from_min_size(rect.min, vec2(w, rect.height()));
            let c = if state.hovered || state.active {
                theme::widget_fill_hover()
            } else {
                theme::widget_fill()
            };
            painter.rect_filled(fr, r, theme::fade(c, state.alpha));
        }
    }
    if state.active {
        painter.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, theme::fade(theme::accent(), state.alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom < 0.45 {
        return;
    }
    label_and_value(painter, rect, zoom, label, value, 0.0, true, state.alpha);
}

/// The value right-aligned, and the label left-aligned in whatever room is
/// left (truncated, or dropped when there's none: the value matters more).
#[allow(clippy::too_many_arguments)]
fn label_and_value(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    value: &str,
    right_pad: f32,
    mono: bool,
    alpha: f32,
) {
    let pad = 6.0 * zoom;
    let size = if mono {
        theme::FONT_VALUE
    } else {
        theme::FONT_NODE_LABEL
    };
    let vg = painter.layout_no_wrap(
        value.to_string(),
        theme::canvas_font(size, zoom, mono),
        theme::fade(theme::text_strong(), alpha),
    );
    let vx = rect.max.x - pad - right_pad - vg.size().x;
    painter.galley(
        pos2(vx, rect.center().y - vg.size().y / 2.0),
        vg,
        theme::text_strong(),
    );
    let room = vx - rect.min.x - 2.0 * pad;
    if room < 14.0 * zoom {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(theme::text_muted(), alpha),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let lg = painter.layout_job(job);
    painter.galley(
        pos2(rect.min.x + pad, rect.center().y - lg.size().y / 2.0),
        lg,
        theme::text_muted(),
    );
}

/// A text field: label and the text, cut to fit (the end matters less
/// than the start of an address, so it's cut on the right).
pub fn paint_text(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    value: &str,
    state: State,
) {
    let r = theme::radius(theme::WIDGET_RADIUS * zoom);
    painter.rect_filled(rect, r, bg(state));
    if state.active {
        painter.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, theme::fade(theme::accent(), state.alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom < 0.45 {
        return;
    }
    let pad = 6.0 * zoom;
    let mut job = egui::text::LayoutJob::single_section(
        value.to_string(),
        egui::TextFormat::simple(
            theme::canvas_font(theme::FONT_VALUE, zoom, true),
            theme::fade(theme::text_strong(), state.alpha),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 2.0 * pad);
    let vg = painter.layout_job(job);
    let vx = rect.max.x - pad - vg.size().x;
    painter.galley(
        pos2(vx, rect.center().y - vg.size().y / 2.0),
        vg,
        theme::text_strong(),
    );
    let room = vx - rect.min.x - 2.0 * pad;
    if room < 14.0 * zoom {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat::simple(
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(theme::text_muted(), state.alpha),
        ),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let lg = painter.layout_job(job);
    painter.galley(
        pos2(rect.min.x + pad, rect.center().y - lg.size().y / 2.0),
        lg,
        theme::text_muted(),
    );
}

/// A dropdown: label, current choice, caret.
pub fn paint_choice(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    value: &str,
    state: State,
) {
    let r = theme::radius(theme::WIDGET_RADIUS * zoom);
    painter.rect_filled(rect, r, bg(state));
    if zoom < 0.45 {
        return;
    }
    let pad = 6.0 * zoom;
    let caret = theme::canvas_font(theme::FONT_SMALL, zoom, false);
    painter.text(
        pos2(rect.max.x - pad, rect.center().y),
        Align2::RIGHT_CENTER,
        icons::CARET_DOWN,
        caret,
        theme::fade(theme::text_muted(), state.alpha),
    );
    label_and_value(
        painter,
        rect,
        zoom,
        label,
        value,
        14.0 * zoom,
        false,
        state.alpha,
    );
}

/// A checkbox with its label.
pub fn paint_check(painter: &Painter, rect: Rect, zoom: f32, label: &str, on: bool, state: State) {
    let s = (rect.height() - 4.0 * zoom).max(4.0);
    let b = Rect::from_min_size(
        pos2(rect.min.x + 4.0 * zoom, rect.center().y - s / 2.0),
        vec2(s, s),
    );
    let r = theme::radius(3.0 * zoom);
    if on {
        painter.rect_filled(b, r, theme::fade(theme::accent(), state.alpha));
        if zoom >= 0.35 {
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                icons::CHECK,
                theme::canvas_font(theme::FONT_SMALL, zoom, false),
                theme::fade(theme::on_accent(), state.alpha),
            );
        }
    } else {
        painter.rect_filled(b, r, bg(state));
        painter.rect_stroke(
            b,
            r,
            Stroke::new(1.0, theme::fade(theme::node_outline(), state.alpha)),
            StrokeKind::Inside,
        );
    }
    if zoom >= 0.45 {
        painter.text(
            pos2(b.max.x + 6.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(theme::text(), state.alpha),
        );
    }
}

/// A small fire button after the label.
pub fn paint_fire(
    painter: &Painter,
    rect: Rect,
    zoom: f32,
    label: &str,
    state: State,
    lit: f32,
) -> Rect {
    if zoom >= 0.45 {
        painter.text(
            pos2(rect.min.x + 6.0 * zoom, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            theme::canvas_font(theme::FONT_NODE_LABEL, zoom, false),
            theme::fade(theme::text_muted(), state.alpha),
        );
    }
    let bw = 26.0 * zoom;
    let b = Rect::from_min_max(pos2(rect.max.x - bw, rect.min.y), rect.max);
    let base = bg(state);
    let fill = theme::mix(
        base,
        theme::type_color(SignalType::Trigger),
        0.25 + 0.6 * lit,
    );
    painter.rect_filled(
        b,
        theme::radius(theme::WIDGET_RADIUS * zoom),
        theme::fade(fill, state.alpha),
    );
    if zoom >= 0.35 {
        painter.text(
            b.center(),
            Align2::CENTER_CENTER,
            icons::LIGHTNING,
            theme::canvas_font(theme::FONT_SMALL, zoom, false),
            theme::fade(theme::text_strong(), state.alpha),
        );
    }
    b
}

/// A socket dot in its type's color and shape.
pub fn paint_socket(
    out: &mut Vec<Shape>,
    center: Pos2,
    r: f32,
    ty: SignalType,
    filled: bool,
    alpha: f32,
    ring: Option<Color32>,
) {
    let color = theme::fade(theme::type_color(ty), alpha);
    let edge = Stroke::new(
        (1.2 * (r / 5.0)).max(0.8),
        theme::fade(theme::socket_edge(), alpha),
    );
    let fill = if filled {
        color
    } else {
        theme::fade(theme::node_bg(), alpha)
    };
    match theme::socket_shape(ty) {
        theme::SocketShape::Circle => {
            out.push(Shape::circle_filled(center, r + 1.0, edge.color));
            out.push(Shape::circle_filled(center, r, fill));
            if !filled {
                out.push(Shape::circle_stroke(
                    center,
                    r - 0.8,
                    Stroke::new(1.4, color),
                ));
            }
        }
        theme::SocketShape::Diamond => {
            let d = r * 1.25;
            let pts = |d: f32| {
                vec![
                    center + vec2(0.0, -d),
                    center + vec2(d, 0.0),
                    center + vec2(0.0, d),
                    center + vec2(-d, 0.0),
                ]
            };
            out.push(Shape::convex_polygon(
                pts(d + 1.0),
                edge.color,
                Stroke::NONE,
            ));
            out.push(Shape::convex_polygon(
                pts(d),
                fill,
                if filled {
                    Stroke::NONE
                } else {
                    Stroke::new(1.4, color)
                },
            ));
        }
        theme::SocketShape::Square => {
            let rect = Rect::from_center_size(center, vec2(r * 2.0, r * 2.0));
            out.push(Shape::rect_filled(rect.expand(1.0), 1.0, edge.color));
            out.push(Shape::rect_filled(rect, 1.0, fill));
        }
    }
    if let Some(c) = ring {
        out.push(Shape::circle_stroke(
            center,
            r + 3.5,
            Stroke::new(1.5, theme::fade(c, alpha)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn freq() -> SocketSpec {
        SocketSpec::float("freq", Unit::Hz, 440.0).range(20.0, 20_000.0)
    }

    #[test]
    fn formats_by_unit() {
        assert_eq!(format(440.0, &freq()), "440 Hz");
        assert_eq!(format(1200.0, &freq()), "1.20 kHz");
        let db = SocketSpec::float("gain", Unit::Db, 0.0);
        assert_eq!(format(-6.0, &db), "-6 dB");
        assert_eq!(format(-200.0, &db), "-∞ dB");
        let s = SocketSpec::float("t", Unit::Seconds, 0.0);
        assert_eq!(format(0.25, &s), "250 ms");
        assert_eq!(format(1.5, &s), "1.5 s");
        assert_eq!(format(f64::NAN, &s), "NaN");
        let g = SocketSpec::float("x", Unit::Generic, 0.0);
        assert_eq!(format(0.5, &g), "0.5");
        assert_eq!(format(-0.0001, &g), "0");
    }

    #[test]
    fn parses_numbers_with_units() {
        let f = freq();
        assert_eq!(parse("1.2k", &f), Some(Value::Float(1200.0)));
        assert_eq!(parse("1.2 kHz", &f), Some(Value::Float(1200.0)));
        assert_eq!(parse(" 440 ", &f), Some(Value::Float(440.0)));
        let s = SocketSpec::float("t", Unit::Seconds, 0.0);
        assert_eq!(parse("250ms", &s), Some(Value::Float(0.25)));
        assert_eq!(parse("loud", &s), None);
        assert_eq!(parse("", &s), None);
        let i = SocketSpec::int("n", 0);
        assert_eq!(parse("3.7", &i), Some(Value::Int(4)));
    }

    #[test]
    fn hz_drags_on_a_log_scale() {
        let f = freq();
        // Half the width from the bottom of a 20..20k log range is ~632 Hz.
        let v = dragged(20.0, 50.0, 100.0, &f, false, false);
        assert!((v - 632.45).abs() < 1.0, "{v}");
        // Fine drags move a tenth as far; drags stay inside the soft range.
        let fine = dragged(20.0, 50.0, 100.0, &f, true, false);
        assert!(fine < 40.0);
        assert_eq!(dragged(440.0, 10_000.0, 100.0, &f, false, false), 20_000.0);
        assert_eq!(normalized(20_000.0, &f), Some(1.0));
    }

    #[test]
    fn fills_start_at_zero_for_bipolar_ranges() {
        let detune = SocketSpec::float("d", Unit::Semitones, 0.0).range(-12.0, 12.0);
        assert_eq!(zero_point(&detune), 0.5);
        assert_eq!(zero_point(&freq()), 0.0);
        let level = SocketSpec::float("l", Unit::Generic, 0.0).range(0.0, 1.0);
        assert_eq!(zero_point(&level), 0.0);
        assert_eq!(zero_point(&SocketSpec::float("u", Unit::Generic, 0.0)), 0.0);
        // Positions map back through the same scale.
        assert_eq!(at_position(0.25, &detune), Some(-6.0));
        let mid = at_position(0.5, &freq()).unwrap();
        assert!((mid - 632.45).abs() < 1.0, "{mid}");
        assert_eq!(
            at_position(0.5, &SocketSpec::float("u", Unit::Generic, 0.0)),
            None
        );
    }

    #[test]
    fn numbers_read_within_their_bounds() {
        let node = |config: serde_json::Value, v: Value| {
            let mut n = NodeInstance::new(ac_core::NodeId(1), "number");
            n.config = config;
            n.params.insert("value".into(), v);
            n
        };
        let f = SocketSpec::float("value", Unit::Generic, 0.0);
        let n = node(serde_json::json!({"min": 5}), Value::Float(1.0));
        assert_eq!(bounded_number(&n, &f), 5.0);
        let n = node(serde_json::json!({"min": 5}), Value::Float(1e9));
        assert_eq!(bounded_number(&n, &f), 1e9);
        let n = node(serde_json::json!({"min": 3, "max": -3}), Value::Float(7.0));
        assert_eq!(bounded_number(&n, &f), 3.0);
        let n = node(serde_json::json!({"max": "big"}), Value::Float(7.0));
        assert_eq!(bounded_number(&n, &f), 7.0);
        let i = SocketSpec::int("value", 0);
        let n = node(
            serde_json::json!({"type": "int", "max": 2.6}),
            Value::Int(9),
        );
        assert_eq!(bounded_number(&n, &i), 3.0);
    }

    #[test]
    fn unbounded_and_int_drags() {
        let g = SocketSpec::float("x", Unit::Generic, 0.0);
        assert!(dragged(0.0, 100.0, 100.0, &g, false, false) > 0.0);
        let i = SocketSpec::int("n", 0);
        assert_eq!(dragged(0.0, 25.0, 100.0, &i, false, false), 3.0);
    }

    #[test]
    fn widget_per_type() {
        assert_eq!(field(&freq()), Field::Number);
        assert_eq!(field(&SocketSpec::boolean("b", false)), Field::Check);
        assert_eq!(field(&SocketSpec::trigger("t")), Field::Fire);
        assert_eq!(
            field(&SocketSpec::trigger("start").auto(AutoTrigger::Start)),
            Field::Choice
        );
        assert_eq!(field(&SocketSpec::audio("in")), Field::Label);
        let start = SocketSpec::trigger("start").auto(AutoTrigger::Start);
        assert_eq!(choice_label(&start, "load"), "On load");
        assert_eq!(
            choice_label(
                &SocketSpec::choice("shape", &["low_shelf"], "low_shelf"),
                "low_shelf"
            ),
            "Low shelf"
        );
    }
}
