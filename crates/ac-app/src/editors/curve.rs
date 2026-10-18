//! The breakpoint curve editor, for `Graph` (a value over normalized time),
//! `Envelope` (levels over seconds from the gate), and `Shaper` (a transfer
//! function over −1..1).
//!
//! Drag a point to move it (it can't pass its neighbors), double-click empty
//! space to add one, double-click a point to remove it, and right-click a
//! point to choose how the segment after it bends.

use ac_core::Config;
use ac_dsp::curve::{Curve, Point, Shape};
use egui::{Align2, Pos2, Shape as Paint, Stroke, pos2};

use super::{Edit, area, grid, load, store};
use crate::theme;

const GRAPH_DEFAULT: &[(f64, f64)] = ac_nodes::generators::graph::DEFAULT_POINTS;
const ENVELOPE_DEFAULT: &[(f64, f64)] = ac_nodes::modulation::envelope::DEFAULT_POINTS;
const SHAPER_DEFAULT: &[(f64, f64)] = ac_nodes::level::shaper::DEFAULT_POINTS;
/// Pixels within which the pointer grabs a point.
const GRAB: f32 = 8.0;
/// Fewest points a curve keeps.
const MIN_POINTS: usize = 2;

/// The plane a curve lives in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    pub x: (f64, f64),
    /// `None`: fit the points, always including 0..1.
    pub y: Option<(f64, f64)>,
    pub default: &'static [(f64, f64)],
    pub x_label: &'static str,
    /// The point held while a gate stays high (envelopes).
    pub sustain: Option<usize>,
}

impl Space {
    pub fn graph() -> Space {
        Space {
            x: (0.0, 1.0),
            y: None,
            default: GRAPH_DEFAULT,
            x_label: "time",
            sustain: None,
        }
    }

    /// Seconds from the gate; the span grows in steps as points reach its end.
    pub fn envelope(config: &Config) -> Space {
        let last = Curve::from_config(config, "points", ENVELOPE_DEFAULT)
            .span()
            .1;
        let max = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 60.0, 120.0]
            .into_iter()
            .find(|&m| m >= last * 1.1)
            .unwrap_or(120.0);
        Space {
            x: (0.0, max),
            y: Some((0.0, 1.0)),
            default: ENVELOPE_DEFAULT,
            x_label: "seconds",
            // Negative: no sustain point.
            sustain: usize::try_from(
                config
                    .get("sustain")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(ac_nodes::modulation::envelope::DEFAULT_SUSTAIN),
            )
            .ok(),
        }
    }

    pub fn shaper() -> Space {
        Space {
            x: (-1.0, 1.0),
            y: Some((-1.0, 1.0)),
            default: SHAPER_DEFAULT,
            x_label: "input",
            sustain: None,
        }
    }

    /// The y range shown for `points`.
    pub fn y_range(&self, points: &[Point]) -> (f64, f64) {
        self.y.unwrap_or_else(|| {
            let lo = points.iter().map(|p| p.y).fold(0.0, f64::min);
            let hi = points.iter().map(|p| p.y).fold(1.0, f64::max);
            (lo, hi)
        })
    }
}

/// The points stored in `config` (or the node's default).
pub fn parse(config: &Config, space: &Space) -> Vec<Point> {
    Curve::from_config(config, "points", space.default)
        .points()
        .to_vec()
}

/// The config value for `points`.
pub fn to_value(points: &[Point]) -> serde_json::Value {
    Curve::new(points.to_vec()).to_json()
}

/// Move point `i`, keeping it between its neighbors and inside the space.
pub fn move_point(points: &mut [Point], i: usize, x: f64, y: f64, space: &Space) {
    let lo = if i == 0 { space.x.0 } else { points[i - 1].x };
    let hi = points.get(i + 1).map_or(space.x.1, |p| p.x);
    let (ylo, yhi) = space.y.unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
    if let Some(p) = points.get_mut(i) {
        p.x = x.clamp(lo.min(hi), hi.max(lo));
        p.y = y.clamp(ylo, yhi);
    }
}

/// Add a straight-segment point at (`x`, `y`); returns its index.
pub fn add_point(points: &mut Vec<Point>, x: f64, y: f64) -> usize {
    let i = points.partition_point(|p| p.x <= x);
    points.insert(
        i,
        Point {
            x,
            y,
            shape: Shape::Linear,
        },
    );
    i
}

/// Remove point `i` unless that would leave too few.
pub fn remove_point(points: &mut Vec<Point>, i: usize) -> bool {
    if points.len() <= MIN_POINTS || i >= points.len() {
        return false;
    }
    points.remove(i);
    true
}

/// The shapes offered for a segment, with their names.
pub const SHAPES: &[(&str, Shape)] = &[
    ("Straight", Shape::Linear),
    ("Hold (step)", Shape::Step),
    ("Smooth", Shape::Smooth),
    ("Slow start", Shape::Bend(0.5)),
    ("Fast start", Shape::Bend(-0.5)),
];

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
}

#[derive(Clone, Copy)]
struct MenuFor(usize);

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    space: Space,
    config: &Config,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let mut points = parse(config, &space);
    let (resp, rect) = area(ui, width, height);
    let plot = rect.shrink(10.0);
    let (ylo, yhi) = space.y_range(&points);
    let (xlo, xhi) = space.x;
    let xspan = (xhi - xlo).max(1e-9);
    let yspan = (yhi - ylo).max(1e-9);
    let to_screen = |x: f64, y: f64| {
        pos2(
            plot.left() + ((x - xlo) / xspan) as f32 * plot.width(),
            plot.bottom() - ((y - ylo) / yspan) as f32 * plot.height(),
        )
    };
    let from_screen = |p: Pos2| {
        (
            xlo + f64::from((p.x - plot.left()) / plot.width().max(1.0)) * xspan,
            ylo + f64::from((plot.bottom() - p.y) / plot.height().max(1.0)) * yspan,
        )
    };
    let nearest = |points: &[Point], at: Pos2| {
        points
            .iter()
            .enumerate()
            .map(|(i, p)| (i, to_screen(p.x, p.y).distance(at)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };

    let mut edit = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let hovered = pointer.and_then(|p| nearest(&points, p));
    let drag_id = id.with("drag");
    // Grab what was under the pointer when it went down, not where it is
    // once the drag registers (a fast drag has moved on by then).
    if resp.drag_started()
        && let Some(i) = ui
            .input(|i| i.pointer.press_origin())
            .and_then(|p| nearest(&points, p))
    {
        store(ui, drag_id, Some(Drag { index: i }));
    }
    if let (Some(d), Some(p)) = (load::<Drag>(ui, drag_id), pointer)
        && resp.dragged()
        && d.index < points.len()
    {
        let (x, y) = from_screen(p);
        move_point(&mut points, d.index, x, y, &space);
        edit = Some(Edit {
            value: to_value(&points),
            done: false,
        });
    }
    if resp.drag_stopped() && load::<Drag>(ui, drag_id).is_some() {
        store::<Drag>(ui, drag_id, None);
        edit = Some(Edit {
            value: to_value(&points),
            done: true,
        });
    }
    if resp.double_clicked()
        && let Some(p) = pointer
    {
        let changed = match nearest(&points, p) {
            Some(i) => remove_point(&mut points, i),
            None if plot.expand(GRAB).contains(p) => {
                let (x, y) = from_screen(p);
                let i = add_point(&mut points, x, y);
                move_point(&mut points, i, x, y, &space);
                true
            }
            None => false,
        };
        if changed {
            edit = Some(Edit {
                value: to_value(&points),
                done: true,
            });
        }
    }
    let menu_id = id.with("menu");
    if resp.secondary_clicked() {
        store(ui, menu_id, hovered.map(MenuFor));
    }
    if let Some(MenuFor(i)) = load::<MenuFor>(ui, menu_id) {
        resp.context_menu(|ui| {
            if i + 1 < points.len() {
                ui.label("Segment after this point");
                for (name, shape) in SHAPES {
                    let current = points.get(i).is_some_and(|p| p.shape == *shape);
                    if ui.selectable_label(current, *name).clicked() {
                        if let Some(p) = points.get_mut(i) {
                            p.shape = *shape;
                        }
                        edit = Some(Edit {
                            value: to_value(&points),
                            done: true,
                        });
                        ui.close();
                    }
                }
                ui.separator();
            }
            if ui
                .add_enabled(points.len() > MIN_POINTS, egui::Button::new("Remove point"))
                .clicked()
            {
                remove_point(&mut points, i);
                edit = Some(Edit {
                    value: to_value(&points),
                    done: true,
                });
                ui.close();
            }
        });
    }

    // Paint.
    let painter = ui.painter_at(rect);
    for k in 0..=4 {
        let t = k as f32 / 4.0;
        let x = plot.left() + t * plot.width();
        let y = plot.top() + t * plot.height();
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, grid(k == 0 || k == 4)),
        );
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(1.0, grid(k == 0 || k == 4)),
        );
    }
    if ylo < 0.0 && yhi > 0.0 {
        let y0 = to_screen(xlo, 0.0).y;
        painter.line_segment(
            [pos2(plot.left(), y0), pos2(plot.right(), y0)],
            Stroke::new(1.0, theme::GRID_DOT_MAJOR),
        );
    }
    let curve = Curve::new(points.clone());
    let n = (plot.width() / 2.0).max(16.0) as usize;
    let line: Vec<Pos2> = (0..=n)
        .map(|k| {
            let x = xlo + xspan * k as f64 / n as f64;
            let y = curve.eval(x).clamp(ylo - yspan, yhi + yspan);
            to_screen(x, y)
        })
        .collect();
    painter.add(Paint::line(line, Stroke::new(2.0, theme::ACCENT)));
    if let Some(p) = space
        .sustain
        .and_then(|i| points.get(i.min(points.len() - 1)))
    {
        let at = to_screen(p.x, p.y);
        painter.circle_stroke(at, 8.0, Stroke::new(1.5, theme::WARNING));
        painter.text(
            at + egui::vec2(0.0, -10.0),
            Align2::CENTER_BOTTOM,
            "sustain",
            egui::FontId::proportional(theme::FONT_SMALL),
            theme::WARNING,
        );
    }
    for (i, p) in points.iter().enumerate() {
        let hot = hovered == Some(i);
        painter.circle(
            to_screen(p.x, p.y),
            if hot { 5.5 } else { 4.0 },
            if hot { theme::TEXT_STRONG } else { theme::TEXT },
            Stroke::new(1.5, theme::WELL_BG),
        );
    }
    let small = egui::FontId::proportional(theme::FONT_SMALL);
    painter.text(
        rect.left_bottom() + egui::vec2(4.0, -3.0),
        Align2::LEFT_BOTTOM,
        format!("{} {}", fmt(xlo), space.x_label),
        small.clone(),
        theme::TEXT_FAINT,
    );
    painter.text(
        rect.right_bottom() + egui::vec2(-4.0, -3.0),
        Align2::RIGHT_BOTTOM,
        fmt(xhi),
        small.clone(),
        theme::TEXT_FAINT,
    );
    painter.text(
        rect.left_top() + egui::vec2(4.0, 3.0),
        Align2::LEFT_TOP,
        fmt(yhi),
        small,
        theme::TEXT_FAINT,
    );
    if let Some(i) = hovered
        && let Some(p) = points.get(i)
    {
        resp.clone()
            .on_hover_text_at_pointer(format!("{}, {}", fmt(p.x), fmt(p.y)));
    }
    edit
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pts(c: serde_json::Value, space: &Space) -> Vec<Point> {
        parse(&json!({ "points": c }), space)
    }

    #[test]
    fn reads_and_writes_the_node_format() {
        let s = Space::graph();
        let p = pts(json!([[0, 0], [0.5, 1, "step"], [1, 0.25, 0.5]]), &s);
        assert_eq!(p.len(), 3);
        assert_eq!(p[1].shape, Shape::Step);
        assert_eq!(
            to_value(&p),
            json!([[0.0, 0.0], [0.5, 1.0, "step"], [1.0, 0.25, 0.5]])
        );
        // Nothing stored: the node's default.
        assert_eq!(parse(&json!({}), &s).len(), 3);
    }

    #[test]
    fn points_stay_between_their_neighbors() {
        let s = Space::graph();
        let mut p = pts(json!([[0, 0], [0.5, 1], [1, 0]]), &s);
        move_point(&mut p, 1, 1.4, 0.3, &s);
        assert_eq!((p[1].x, p[1].y), (1.0, 0.3));
        move_point(&mut p, 1, -3.0, 0.3, &s);
        assert_eq!(p[1].x, 0.0);
        move_point(&mut p, 0, -1.0, 0.0, &s);
        assert_eq!(p[0].x, 0.0, "inside the space");
    }

    #[test]
    fn y_is_bounded_only_where_the_node_bounds_it() {
        let g = Space::graph();
        let mut p = pts(json!([[0, 0], [1, 1]]), &g);
        move_point(&mut p, 1, 1.0, 7.0, &g);
        assert_eq!(p[1].y, 7.0);
        assert_eq!(g.y_range(&p), (0.0, 7.0));
        let sh = Space::shaper();
        let mut p = pts(json!([[-1, -1], [1, 1]]), &sh);
        move_point(&mut p, 1, 1.0, 7.0, &sh);
        assert_eq!(p[1].y, 1.0);
    }

    #[test]
    fn adding_keeps_order_and_removing_keeps_two() {
        let s = Space::graph();
        let mut p = pts(json!([[0, 0], [1, 0]]), &s);
        assert_eq!(add_point(&mut p, 0.25, 1.0), 1);
        assert_eq!(add_point(&mut p, 0.75, 1.0), 2);
        assert!(p.windows(2).all(|w| w[0].x <= w[1].x));
        assert!(remove_point(&mut p, 1));
        assert!(remove_point(&mut p, 1));
        assert!(!remove_point(&mut p, 0), "two points stay");
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn the_envelope_span_grows_with_its_points() {
        assert_eq!(Space::envelope(&json!({})).x, (0.0, 1.0));
        let long = json!({"points": [[0, 0], [3, 1]]});
        assert_eq!(Space::envelope(&long).x, (0.0, 5.0));
        let edge = json!({"points": [[0, 0], [1, 1]]});
        assert_eq!(Space::envelope(&edge).x, (0.0, 2.0));
    }
}
