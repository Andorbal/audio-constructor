//! Breakpoint curves: the hand-drawn shapes of `Graph` (a curve over time),
//! `Shaper` (a transfer function), and `Envelope` (stages between
//! breakpoints).
//!
//! Stored in node config as `"points": [[x, y], [x, y, shape], ...]`, sorted
//! by `x`. A point's shape bends the segment that *starts* at it:
//! `"linear"` (the default), `"step"` (hold `y` until the next point),
//! `"smooth"` (an S-shaped cosine ease), or a number from -1 to 1 that bows
//! the segment like a power curve (0 is straight; positive rises late,
//! negative rises early). Evaluation is a binary search: no allocation,
//! safe on the audio thread.

use ac_core::Config;

/// How a segment gets from its start point to the next one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Linear,
    Step,
    Smooth,
    /// -1..1: the exponent of `u^e` is `2^(4 * bend)`.
    Bend(f64),
}

impl Shape {
    fn parse(v: Option<&serde_json::Value>) -> Shape {
        match v {
            Some(serde_json::Value::String(s)) => match s.as_str() {
                "step" => Shape::Step,
                "smooth" => Shape::Smooth,
                _ => Shape::Linear,
            },
            Some(serde_json::Value::Number(n)) => {
                let b = n.as_f64().unwrap_or(0.0);
                if b.is_finite() && b != 0.0 {
                    Shape::Bend(b.clamp(-1.0, 1.0))
                } else {
                    Shape::Linear
                }
            }
            _ => Shape::Linear,
        }
    }

    fn to_json(self) -> Option<serde_json::Value> {
        match self {
            Shape::Linear => None,
            Shape::Step => Some("step".into()),
            Shape::Smooth => Some("smooth".into()),
            Shape::Bend(b) => Some(b.into()),
        }
    }

    /// Fraction of the way from start to end at `u` (0..1) along the segment.
    #[inline]
    pub fn ease(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Shape::Linear => u,
            Shape::Step => 0.0,
            Shape::Smooth => 0.5 - 0.5 * (std::f64::consts::PI * u).cos(),
            Shape::Bend(b) => u.powf((4.0 * b).exp2()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    /// The segment from this point to the next.
    pub shape: Shape,
}

/// A curve through breakpoints; flat before the first and after the last.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    points: Vec<Point>,
}

impl Curve {
    pub fn new(mut points: Vec<Point>) -> Curve {
        points.retain(|p| p.x.is_finite() && p.y.is_finite());
        points.sort_by(|a, b| a.x.total_cmp(&b.x));
        Curve { points }
    }

    /// From a JSON array of `[x, y]` / `[x, y, shape]`; entries that aren't
    /// numbers are skipped. `default` is used when nothing valid is left.
    pub fn from_json(v: Option<&serde_json::Value>, default: &[(f64, f64)]) -> Curve {
        let mut points = Vec::new();
        if let Some(list) = v.and_then(|v| v.as_array()) {
            for p in list {
                let Some(a) = p.as_array() else { continue };
                let (Some(x), Some(y)) = (
                    a.first().and_then(serde_json::Value::as_f64),
                    a.get(1).and_then(serde_json::Value::as_f64),
                ) else {
                    continue;
                };
                points.push(Point {
                    x,
                    y,
                    shape: Shape::parse(a.get(2)),
                });
            }
        }
        let curve = Curve::new(points);
        if curve.points.is_empty() {
            Curve::new(
                default
                    .iter()
                    .map(|&(x, y)| Point {
                        x,
                        y,
                        shape: Shape::Linear,
                    })
                    .collect(),
            )
        } else {
            curve
        }
    }

    /// The curve stored under `key` in a node's config.
    pub fn from_config(config: &Config, key: &str, default: &[(f64, f64)]) -> Curve {
        Curve::from_json(config.get(key), default)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.points
                .iter()
                .map(|p| {
                    let mut a = vec![p.x.into(), p.y.into()];
                    if let Some(s) = p.shape.to_json() {
                        a.push(s);
                    }
                    serde_json::Value::Array(a)
                })
                .collect(),
        )
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// `x` of the first and last points.
    pub fn span(&self) -> (f64, f64) {
        match (self.points.first(), self.points.last()) {
            (Some(a), Some(b)) => (a.x, b.x),
            _ => (0.0, 0.0),
        }
    }

    /// The value at `x`.
    pub fn eval(&self, x: f64) -> f64 {
        let p = &self.points;
        let Some(first) = p.first() else {
            return 0.0;
        };
        if x.is_nan() || x <= first.x {
            return first.y;
        }
        // The last point at or before x.
        let i = p.partition_point(|q| q.x <= x).saturating_sub(1);
        let a = p[i];
        let Some(b) = p.get(i + 1) else {
            return a.y;
        };
        let w = b.x - a.x;
        if w <= 0.0 {
            return b.y;
        }
        a.y + (b.y - a.y) * a.shape.ease((x - a.x) / w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evaluates_segments_and_holds_outside() {
        let c = Curve::from_json(
            Some(&json!([
                [0.0, 0.0],
                [1.0, 1.0, "step"],
                [2.0, 0.0, "smooth"],
                [3.0, 1.0]
            ])),
            &[],
        );
        assert_eq!(c.eval(-1.0), 0.0);
        assert_eq!(c.eval(0.5), 0.5);
        assert_eq!(c.eval(1.5), 1.0, "step holds");
        assert!((c.eval(2.5) - 0.5).abs() < 1e-12);
        assert!(c.eval(2.25) < 0.25, "smooth starts slow");
        assert_eq!(c.eval(10.0), 1.0);
        assert_eq!(c.span(), (0.0, 3.0));
    }

    #[test]
    fn bends_bow_the_segment() {
        let late = Curve::from_json(Some(&json!([[0, 0, 0.5], [1, 1]])), &[]);
        let early = Curve::from_json(Some(&json!([[0, 0, -0.5], [1, 1]])), &[]);
        assert!(late.eval(0.5) < 0.5 && early.eval(0.5) > 0.5);
        assert_eq!(late.eval(1.0), 1.0);
    }

    #[test]
    fn bad_input_falls_back_and_round_trips() {
        let c = Curve::from_json(Some(&json!(["x", [1], [2, "y"]])), &[(0.0, 1.0)]);
        assert_eq!(c.eval(5.0), 1.0);
        assert_eq!(Curve::new(vec![]).eval(1.0), 0.0);
        // Unsorted points are sorted; NaN is dropped.
        let c = Curve::from_json(Some(&json!([[1, 1], [0, 0, "smooth"], [0.5, null]])), &[]);
        assert_eq!(c.points().len(), 2);
        let back = Curve::from_json(Some(&c.to_json()), &[]);
        assert_eq!(back, c);
    }
}
