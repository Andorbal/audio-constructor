//! Wires: cubic béziers colored by type, with hover glow, flowing texture
//! on audio wires (brightness follows level), and pulses on event wires.

use egui::epaint::{CubicBezierShape, PathStroke};
use egui::{Color32, Pos2, Shape, Stroke, pos2};

use crate::theme;

/// Control points of the wire from an output at `a` to an input at `b`
/// (screen points): horizontal tangents whose reach grows with distance, so
/// short wires stay tight and backward wires loop around.
pub fn bezier(a: Pos2, b: Pos2, zoom: f32) -> [Pos2; 4] {
    let dx = (b.x - a.x).abs();
    // Short wires between close sockets stay a gentle S instead of looping.
    let min_reach = (40.0 * zoom.max(0.3)).min((b - a).length() * 0.5);
    let reach = (dx * 0.5).max(min_reach).min(260.0 * zoom.max(0.3));
    let back = if b.x < a.x {
        // Going backwards: push the tangents out further so the wire doesn't
        // cut straight through the nodes.
        reach + (a.x - b.x) * 0.15
    } else {
        reach
    };
    [a, pos2(a.x + back, a.y), pos2(b.x - back, b.y), b]
}

/// Points along the curve, roughly every few screen points.
pub fn flatten(c: &[Pos2; 4]) -> Vec<Pos2> {
    let len = (c[1] - c[0]).length() + (c[2] - c[1]).length() + (c[3] - c[2]).length();
    let n = ((len / 6.0) as usize).clamp(8, 160);
    (0..=n).map(|i| eval(c, i as f32 / n as f32)).collect()
}

pub fn eval(c: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    pos2(
        w[0] * c[0].x + w[1] * c[1].x + w[2] * c[2].x + w[3] * c[3].x,
        w[0] * c[0].y + w[1] * c[1].y + w[2] * c[2].y + w[3] * c[3].y,
    )
}

/// Distance from `p` to a polyline.
pub fn distance(points: &[Pos2], p: Pos2) -> f32 {
    points
        .windows(2)
        .map(|s| segment_distance(s[0], s[1], p))
        .fold(f32::MAX, f32::min)
}

fn segment_distance(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Where two segments cross, if they do.
pub fn segment_intersection(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> Option<Pos2> {
    let r = b - a;
    let s = d - c;
    let denom = r.x * s.y - r.y * s.x;
    if denom.abs() < 1e-6 {
        return None;
    }
    let qp = c - a;
    let t = (qp.x * s.y - qp.y * s.x) / denom;
    let u = (qp.x * r.y - qp.y * r.x) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// First point where a gesture polyline crosses a wire polyline.
pub fn crossing(wire: &[Pos2], stroke: &[Pos2]) -> Option<Pos2> {
    for s in stroke.windows(2) {
        for w in wire.windows(2) {
            if let Some(p) = segment_intersection(s[0], s[1], w[0], w[1]) {
                return Some(p);
            }
        }
    }
    None
}

/// How to draw one wire this frame.
#[derive(Clone, Copy, Debug)]
pub struct WireLook {
    pub color: Color32,
    pub width: f32,
    /// Hovered or attached to the selection.
    pub glow: bool,
    /// Dimmed (incompatible while dragging, or muted).
    pub alpha: f32,
    /// Audio level 0..1 (peak), for the flowing texture.
    pub level: f32,
    /// Seconds since the last event on this wire, for the pulse.
    pub pulse_age: Option<f32>,
    /// Animation clock, seconds.
    pub time: f32,
}

/// Shapes for one wire.
pub fn paint(out: &mut Vec<Shape>, c: [Pos2; 4], look: WireLook) {
    let base = theme::fade(look.color, look.alpha * if look.glow { 1.0 } else { 0.78 });
    if look.glow {
        out.push(curve(
            c,
            look.width + theme::WIRE_GLOW,
            theme::fade(look.color, 0.16 * look.alpha),
        ));
        out.push(curve(
            c,
            look.width + 3.0,
            theme::fade(look.color, 0.28 * look.alpha),
        ));
    }
    // A thin dark casing keeps wires readable where they cross bright ones.
    out.push(curve(
        c,
        look.width + 1.5,
        Color32::from_black_alpha((70.0 * look.alpha) as u8),
    ));
    out.push(curve(c, look.width, base));

    if look.level > 0.0005 {
        flow(out, &c, look);
    }
    if let Some(age) = look.pulse_age
        && age < theme::ANIM_SECONDS
    {
        pulse(out, &c, look, age / theme::ANIM_SECONDS);
    }
}

fn curve(c: [Pos2; 4], width: f32, color: Color32) -> Shape {
    Shape::CubicBezier(CubicBezierShape::from_points_stroke(
        c,
        false,
        Color32::TRANSPARENT,
        PathStroke::new(width, color),
    ))
}

/// Short bright dashes moving from output to input; their brightness
/// follows the signal level (mapped from dBFS so quiet signals still show).
fn flow(out: &mut Vec<Shape>, c: &[Pos2; 4], look: WireLook) {
    let pts = flatten(c);
    let db = 20.0 * look.level.max(1e-6).log10();
    let strength = ((db + 60.0) / 60.0).clamp(0.08, 1.0);
    let bright = theme::mix(look.color, Color32::WHITE, 0.55);
    let color = theme::fade(bright, strength * look.alpha * 0.9);
    let spacing = 22.0;
    let dash: f32 = 7.0;
    let phase = (look.time * 46.0) % spacing;
    let mut acc = 0.0;
    let mut next = phase;
    for s in pts.windows(2) {
        let len = (s[1] - s[0]).length();
        while next < acc + len {
            let t0 = (next - acc) / len.max(1e-3);
            let a = s[0] + (s[1] - s[0]) * t0;
            let dir = (s[1] - s[0]) / len.max(1e-3);
            let b = a + dir * dash.min(len);
            out.push(Shape::line_segment(
                [a, b],
                Stroke::new(look.width * 0.9, color),
            ));
            next += spacing;
        }
        acc += len;
    }
}

/// An event pulse: the wire flashes and a bright dot runs to the input.
fn pulse(out: &mut Vec<Shape>, c: &[Pos2; 4], look: WireLook, t: f32) {
    let ease = 1.0 - (1.0 - t) * (1.0 - t);
    let flash = theme::fade(Color32::WHITE, (1.0 - t) * 0.5 * look.alpha);
    out.push(curve(*c, look.width + 2.0, flash));
    let p = eval(c, ease);
    out.push(Shape::circle_filled(
        p,
        3.5 + 2.0 * (1.0 - t),
        theme::fade(theme::mix(look.color, Color32::WHITE, 0.6), look.alpha),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_testing_finds_points_on_the_curve() {
        let c = bezier(pos2(0.0, 0.0), pos2(200.0, 100.0), 1.0);
        let pts = flatten(&c);
        assert!(distance(&pts, eval(&c, 0.5)) < 1.0);
        assert!(distance(&pts, pos2(100.0, -80.0)) > 20.0);
    }

    #[test]
    fn crossings() {
        let wire = [pos2(0.0, 0.0), pos2(100.0, 0.0)];
        let stroke = [pos2(50.0, -10.0), pos2(50.0, 10.0)];
        let p = crossing(&wire, &stroke).unwrap();
        assert!((p - pos2(50.0, 0.0)).length() < 1e-3);
        assert!(crossing(&wire, &[pos2(0.0, 5.0), pos2(100.0, 5.0)]).is_none());
    }
}
