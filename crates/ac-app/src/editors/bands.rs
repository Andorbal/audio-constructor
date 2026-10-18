//! The band editor of `BandSplit`: crossover frequencies on a log axis.
//! Drag an edge to move it, double-click empty space to split a band, and
//! double-click (or right-click) an edge to merge the bands beside it.
//!
//! The node stores `"edges": [200, 2000]` in Hz, sorted, 20 Hz to 20 kHz,
//! at most 15 (so at most 16 bands).

use ac_core::Config;
use egui::{Align2, CursorIcon, Pos2, Rect, Stroke, pos2, vec2};

use super::{Edit, area, grid, load, store};
use crate::theme;

pub const MIN_HZ: f64 = 20.0;
pub const MAX_HZ: f64 = 20_000.0;
pub const MAX_EDGES: usize = 15;
/// Neighboring edges stay at least this ratio apart (about a semitone).
pub const MIN_RATIO: f64 = 1.06;
const DEFAULT: [f64; 2] = [200.0, 2000.0];
const GRAB: f32 = 6.0;

/// The edges in `config`: repaired, sorted, and spaced.
pub fn parse(config: &Config) -> Vec<f64> {
    let mut edges: Vec<f64> = config
        .get("edges")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_f64)
                .filter(|f| f.is_finite())
                .map(|f| f.clamp(MIN_HZ, MAX_HZ))
                .collect()
        })
        .unwrap_or_else(|| DEFAULT.to_vec());
    edges.sort_by(f64::total_cmp);
    edges.dedup_by(|b, a| *b < *a * MIN_RATIO);
    edges.truncate(MAX_EDGES);
    if edges.is_empty() {
        edges = DEFAULT.to_vec();
    }
    edges
}

/// The config value for `edges`, rounded to a tenth of a hertz.
pub fn to_value(edges: &[f64]) -> serde_json::Value {
    serde_json::Value::Array(
        edges
            .iter()
            .map(|f| ((f * 10.0).round() / 10.0).into())
            .collect(),
    )
}

/// Move edge `i` to `hz`, staying between its neighbors.
pub fn move_edge(edges: &mut [f64], i: usize, hz: f64) {
    let lo = if i == 0 {
        MIN_HZ
    } else {
        edges[i - 1] * MIN_RATIO
    };
    let hi = edges.get(i + 1).map_or(MAX_HZ, |f| f / MIN_RATIO);
    if let Some(e) = edges.get_mut(i) {
        *e = hz.clamp(lo.min(hi), hi.max(lo));
    }
}

/// Split the band at `hz`; returns the new edge's index, or `None` when
/// there are already as many bands as the node allows or `hz` is too close
/// to an edge.
pub fn add_edge(edges: &mut Vec<f64>, hz: f64) -> Option<usize> {
    if edges.len() >= MAX_EDGES || !(MIN_HZ..=MAX_HZ).contains(&hz) {
        return None;
    }
    if edges
        .iter()
        .any(|&e| hz < e * MIN_RATIO && hz > e / MIN_RATIO)
    {
        return None;
    }
    let i = edges.partition_point(|&e| e < hz);
    edges.insert(i, hz);
    Some(i)
}

/// Merge the bands beside edge `i`; one edge always stays.
pub fn remove_edge(edges: &mut Vec<f64>, i: usize) -> bool {
    if edges.len() <= 1 || i >= edges.len() {
        return false;
    }
    edges.remove(i);
    true
}

#[derive(Clone, Copy)]
struct Drag(usize);

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let mut edges = parse(config);
    let (resp, rect) = area(ui, width, height);
    let plot = rect.shrink2(vec2(8.0, 6.0));
    let span = (MAX_HZ / MIN_HZ).ln();
    let to_x = |hz: f64| plot.left() + ((hz / MIN_HZ).ln() / span) as f32 * plot.width();
    let to_hz =
        |x: f32| MIN_HZ * (f64::from((x - plot.left()) / plot.width().max(1.0)) * span).exp();
    let nearest = |edges: &[f64], p: Pos2| {
        edges
            .iter()
            .enumerate()
            .map(|(i, &e)| (i, (to_x(e) - p.x).abs()))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };

    let mut edit = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let hovered = resp.hover_pos().and_then(|p| nearest(&edges, p));
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let drag_id = id.with("drag");
    if resp.drag_started()
        && let Some(i) = ui
            .input(|i| i.pointer.press_origin())
            .and_then(|p| nearest(&edges, p))
    {
        store(ui, drag_id, Some(Drag(i)));
    }
    if resp.dragged()
        && let (Some(Drag(i)), Some(p)) = (load::<Drag>(ui, drag_id), pointer)
    {
        move_edge(&mut edges, i, to_hz(p.x));
        edit = Some(Edit {
            value: to_value(&edges),
            done: false,
        });
    }
    if resp.drag_stopped() && load::<Drag>(ui, drag_id).is_some() {
        store::<Drag>(ui, drag_id, None);
        edit = Some(Edit {
            value: to_value(&edges),
            done: true,
        });
    }
    if (resp.double_clicked() || resp.secondary_clicked())
        && let Some(p) = pointer
    {
        let changed = match nearest(&edges, p) {
            Some(i) => remove_edge(&mut edges, i),
            None if resp.double_clicked() => add_edge(&mut edges, to_hz(p.x)).is_some(),
            None => false,
        };
        if changed {
            edit = Some(Edit {
                value: to_value(&edges),
                done: true,
            });
        }
    }

    let painter = ui.painter_at(rect);
    let small = egui::FontId::proportional(theme::FONT_SMALL);
    let color = theme::category_color(ac_core::Category::Spectral);
    let mut bounds = vec![MIN_HZ];
    bounds.extend(edges.iter().copied());
    bounds.push(MAX_HZ);
    for (k, w) in bounds.windows(2).enumerate() {
        let r = Rect::from_min_max(
            pos2(to_x(w[0]), plot.top()),
            pos2(to_x(w[1]), plot.bottom()),
        );
        let alpha = if k % 2 == 0 { 0.22 } else { 0.12 };
        painter.rect_filled(r, 0.0, theme::fade(color, alpha));
        if r.width() > 22.0 {
            painter.text(
                pos2(r.center().x, plot.top() + 2.0),
                Align2::CENTER_TOP,
                format!("{k}"),
                small.clone(),
                theme::TEXT_MUTED,
            );
        }
    }
    for hz in [100.0, 1_000.0, 10_000.0] {
        let x = to_x(hz);
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, grid(true)),
        );
        painter.text(
            pos2(x + 2.0, plot.bottom()),
            Align2::LEFT_BOTTOM,
            if hz >= 1_000.0 {
                format!("{}k", hz / 1_000.0)
            } else {
                format!("{hz}")
            },
            small.clone(),
            theme::TEXT_FAINT,
        );
    }
    for (i, &e) in edges.iter().enumerate() {
        let x = to_x(e);
        let hot = hovered == Some(i) || load::<Drag>(ui, drag_id).is_some_and(|d| d.0 == i);
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(
                if hot { 3.0 } else { 2.0 },
                if hot { theme::TEXT_STRONG } else { theme::TEXT },
            ),
        );
        if hot {
            painter.text(
                pos2(x, plot.center().y),
                Align2::CENTER_CENTER,
                if e >= 1_000.0 {
                    format!("{:.2} kHz", e / 1_000.0)
                } else {
                    format!("{e:.0} Hz")
                },
                small.clone(),
                theme::TEXT_STRONG,
            );
        }
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_repairs_the_node_format() {
        assert_eq!(parse(&json!({})), DEFAULT);
        assert_eq!(
            parse(&json!({"edges": [5000, 3, 400]})),
            [20.0, 400.0, 5000.0]
        );
        assert_eq!(parse(&json!({"edges": []})), DEFAULT);
        let many: Vec<f64> = (0..30).map(|k| 30.0 * 1.2f64.powi(k)).collect();
        assert_eq!(parse(&json!({ "edges": many })).len(), MAX_EDGES);
        assert_eq!(to_value(&[123.456]), json!([123.5]));
    }

    #[test]
    fn edges_keep_their_order() {
        let mut e = vec![200.0, 2000.0];
        move_edge(&mut e, 0, 5000.0);
        assert!(e[0] < e[1] && (e[0] - 2000.0 / MIN_RATIO).abs() < 1e-9);
        move_edge(&mut e, 1, 1e9);
        assert_eq!(e[1], MAX_HZ);
        move_edge(&mut e, 0, 1.0);
        assert_eq!(e[0], MIN_HZ);
    }

    #[test]
    fn splitting_and_merging() {
        let mut e = vec![200.0, 2000.0];
        assert_eq!(add_edge(&mut e, 600.0), Some(1));
        assert_eq!(add_edge(&mut e, 610.0), None, "too close");
        assert!(remove_edge(&mut e, 1));
        assert!(remove_edge(&mut e, 0));
        assert!(!remove_edge(&mut e, 0), "one edge stays");
        let mut full: Vec<f64> = (0..MAX_EDGES)
            .map(|k| 30.0 * 1.5f64.powi(k as i32))
            .collect();
        assert_eq!(add_edge(&mut full, 19_000.0), None);
    }
}
