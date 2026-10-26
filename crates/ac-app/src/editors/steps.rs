//! The step values of `StepSequencer`: one bar per step. Click or drag
//! across the bars to draw values; hold Shift for fine values (otherwise
//! they snap to a hundredth of the range, or to whole numbers for note
//! ranges).
//!
//! The node stores `"values": [0.0, 0.5, ...]`, one number per step
//! (`steps` of them; missing ones are 0). The range shown fits the values:
//! 0..1, −1..1, a note range, or wider, and can be changed in the editor.

use ac_core::{Config, ConfigExt};
use egui::{Align2, Pos2, Rect, Stroke, pos2, vec2};

use super::{Edit, LiveView, area, grid, load, snap, store};
use crate::theme;

pub const MAX_STEPS: usize = 64;

/// The ranges the editor offers, with names.
pub const RANGES: &[(&str, (f64, f64))] = &[
    ("0 to 1", (0.0, 1.0)),
    ("−1 to 1", (-1.0, 1.0)),
    ("−12 to 12 (semitones)", (-12.0, 12.0)),
    ("−24 to 24 (semitones)", (-24.0, 24.0)),
    ("0 to 127 (notes)", (0.0, 127.0)),
];

/// The values in `config`, `steps` of them.
pub fn parse(config: &Config) -> Vec<f64> {
    let steps = config.get_u64_or("steps", 8).clamp(1, MAX_STEPS as u64) as usize;
    let stored = config.get("values").and_then(|v| v.as_array());
    (0..steps)
        .map(|k| {
            stored
                .and_then(|a| a.get(k))
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
                .unwrap_or(0.0)
        })
        .collect()
}

pub fn to_value(values: &[f64]) -> serde_json::Value {
    serde_json::Value::Array(
        values
            .iter()
            .map(|v| ((v * 1e6).round() / 1e6).into())
            .collect(),
    )
}

/// The smallest offered range holding every value, or one fitted to them.
pub fn fit_range(values: &[f64]) -> (f64, f64) {
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo > hi {
        return RANGES[0].1;
    }
    RANGES
        .iter()
        .map(|r| r.1)
        .find(|&(a, b)| lo >= a && hi <= b)
        .unwrap_or((lo.min(0.0), hi.max(lo + 1.0)))
}

/// The snap step for a range: whole numbers for semitone and note ranges.
pub fn snap_step(range: (f64, f64)) -> f64 {
    let span = range.1 - range.0;
    if span >= 24.0 { 1.0 } else { span / 100.0 }
}

/// Set steps `from..=to` (either order) along a line from `a` to `b`.
pub fn draw(values: &mut [f64], (from, a): (usize, f64), (to, b): (usize, f64)) {
    let (s0, v0, s1, v1) = if from <= to {
        (from, a, to, b)
    } else {
        (to, b, from, a)
    };
    for s in s0..=s1.min(values.len().saturating_sub(1)) {
        let t = if s1 > s0 {
            (s - s0) as f64 / (s1 - s0) as f64
        } else {
            0.0
        };
        values[s] = v0 + (v1 - v0) * t;
    }
}

/// The step and value at the last pointer position of a drag.
#[derive(Clone, Copy)]
struct Last(usize, f64);

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    live: &LiveView,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let mut values = parse(config);
    let range_id = id.with("range");
    let drag_id = id.with("last");
    let range = load::<(f64, f64)>(ui, range_id).unwrap_or_else(|| fit_range(&values));
    let mut edit = None;
    ui.horizontal(|ui| {
        let name = RANGES.iter().find(|r| r.1 == range).map_or_else(
            || format!("{} to {}", range.0, range.1),
            |r| r.0.to_string(),
        );
        egui::ComboBox::new(id.with("range-menu"), "range")
            .selected_text(name)
            .show_ui(ui, |ui| {
                for (label, r) in RANGES {
                    if ui.selectable_label(*r == range, *label).clicked() {
                        store(ui, range_id, Some(*r));
                    }
                }
            });
    });
    let (resp, rect) = area(ui, width, height);
    let plot = rect.shrink2(vec2(6.0, 8.0));
    let steps = values.len().max(1);
    let bw = plot.width() / steps as f32;
    let (lo, hi) = range;
    let span = (hi - lo).max(1e-9);
    let to_y = |v: f64| plot.bottom() - ((v - lo) / span).clamp(0.0, 1.0) as f32 * plot.height();
    let fine = ui.input(|i| i.modifiers.shift);
    let at = |p: Pos2| {
        let s = (((p.x - plot.left()) / bw).floor().max(0.0) as usize).min(steps - 1);
        let v = lo + f64::from((plot.bottom() - p.y) / plot.height().max(1.0)) * span;
        let v = if fine { v } else { snap(v, snap_step(range)) };
        (s, v.clamp(lo, hi))
    };

    let pointer = resp.interact_pointer_pos();
    if resp.clicked()
        && let Some(p) = pointer
    {
        let (s, v) = at(p);
        values[s] = v;
        edit = Some(Edit {
            value: to_value(&values),
            done: true,
        });
    }
    if resp.dragged()
        && let Some(p) = pointer
    {
        let (s, v) = at(p);
        let from = load::<Last>(ui, drag_id).map_or((s, v), |l| (l.0, l.1));
        draw(&mut values, from, (s, v));
        store(ui, drag_id, Some(Last(s, v)));
        edit = Some(Edit {
            value: to_value(&values),
            done: false,
        });
    }
    if resp.drag_stopped() {
        store::<Last>(ui, drag_id, None);
        edit = Some(Edit {
            value: to_value(&values),
            done: true,
        });
    }

    let painter = ui.painter_at(rect);
    let color = theme::category_color(ac_core::Category::Modulation);
    let base = to_y(0.0_f64.clamp(lo, hi));
    painter.line_segment(
        [pos2(plot.left(), base), pos2(plot.right(), base)],
        Stroke::new(1.0, grid(true)),
    );
    let hover = resp.hover_pos().map(|p| at(p).0);
    for (s, &v) in values.iter().enumerate() {
        let x0 = plot.left() + s as f32 * bw;
        let bar = Rect::from_min_max(
            pos2(x0 + (bw * 0.1).min(2.0), to_y(v).min(base)),
            pos2(x0 + bw - (bw * 0.1).min(2.0), to_y(v).max(base)),
        );
        let lit = hover == Some(s) || live.step == Some(s);
        if live.step == Some(s) {
            painter.rect_stroke(
                Rect::from_min_max(pos2(x0, plot.top()), pos2(x0 + bw, plot.bottom())),
                theme::radius(1.5),
                Stroke::new(1.5, theme::play()),
                egui::StrokeKind::Inside,
            );
        }
        painter.rect_filled(
            bar,
            theme::radius(1.5),
            theme::fade(color, if lit { 1.0 } else { 0.8 }),
        );
        if s % 4 == 0 && s > 0 {
            painter.line_segment(
                [pos2(x0, plot.top()), pos2(x0, plot.bottom())],
                Stroke::new(1.0, grid(false)),
            );
        }
    }
    if let Some(s) = hover
        && let Some(v) = values.get(s)
    {
        painter.text(
            rect.right_top() + vec2(-4.0, 2.0),
            Align2::RIGHT_TOP,
            format!("step {}: {}", s + 1, (v * 1000.0).round() / 1000.0),
            egui::FontId::proportional(theme::FONT_SMALL),
            theme::text_muted(),
        );
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_node_format_and_pads_with_zero() {
        assert_eq!(
            parse(&json!({"steps": 4, "values": [1, 0.5]})),
            [1.0, 0.5, 0.0, 0.0]
        );
        assert_eq!(parse(&json!({})).len(), 8);
        assert_eq!(to_value(&[0.1234567]), json!([0.123457]));
    }

    #[test]
    fn the_range_fits_the_values() {
        assert_eq!(fit_range(&[0.0, 0.7]), (0.0, 1.0));
        assert_eq!(fit_range(&[-0.5, 0.7]), (-1.0, 1.0));
        assert_eq!(fit_range(&[-7.0, 5.0]), (-12.0, 12.0));
        assert_eq!(fit_range(&[48.0, 72.0]), (0.0, 127.0));
        assert_eq!(fit_range(&[-500.0, 900.0]), (-500.0, 900.0));
        assert_eq!(snap_step((0.0, 127.0)), 1.0);
        assert_eq!(snap_step((0.0, 1.0)), 0.01);
    }

    #[test]
    fn drawing_fills_the_steps_between() {
        let mut v = vec![0.0; 5];
        draw(&mut v, (4, 1.0), (0, 0.0));
        assert_eq!(v, [0.0, 0.25, 0.5, 0.75, 1.0]);
        draw(&mut v, (2, 9.0), (2, 9.0));
        assert_eq!(v[2], 9.0);
    }
}
