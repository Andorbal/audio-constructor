//! The step grid of `Pattern`: one row per output, one column per step.
//! Click a cell to switch it; drag across cells to paint them all the way
//! the first one went.
//!
//! The node stores the grid as `"cells": ["x...x...", "....x..."]`, one
//! string per row, `x` for on and `.` for off. `rows` and `steps` size it.

use ac_core::{Config, ConfigExt};
use egui::{Align2, Pos2, Rect, pos2, vec2};

use super::{Edit, area, load, store};
use crate::theme;

pub const MAX_ROWS: usize = 16;
pub const MAX_STEPS: usize = 64;

/// The grid in `config`: `rows` × `steps` cells.
pub fn parse(config: &Config) -> Vec<Vec<bool>> {
    let rows = config.get_u64_or("rows", 4).clamp(1, MAX_ROWS as u64) as usize;
    let steps = config.get_u64_or("steps", 16).clamp(1, MAX_STEPS as u64) as usize;
    let cells = config.get("cells").and_then(|v| v.as_array());
    (0..rows)
        .map(|r| {
            let row = cells
                .and_then(|c| c.get(r))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut chars = row.chars();
            (0..steps)
                .map(|_| matches!(chars.next(), Some('x' | 'X')))
                .collect()
        })
        .collect()
}

/// The config value for `grid`.
pub fn to_value(grid: &[Vec<bool>]) -> serde_json::Value {
    serde_json::Value::Array(
        grid.iter()
            .map(|row| {
                serde_json::Value::String(
                    row.iter().map(|&on| if on { 'x' } else { '.' }).collect(),
                )
            })
            .collect(),
    )
}

/// Set one cell; returns whether it changed.
pub fn set(grid: &mut [Vec<bool>], row: usize, step: usize, on: bool) -> bool {
    match grid.get_mut(row).and_then(|r| r.get_mut(step)) {
        Some(c) if *c != on => {
            *c = on;
            true
        }
        _ => false,
    }
}

/// Paint state: what a drag sets cells to.
#[derive(Clone, Copy)]
struct Paint(bool);

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let mut grid = parse(config);
    let rows = grid.len();
    let steps = grid.first().map_or(0, Vec::len);
    let (resp, rect) = area(ui, width, height);
    let label_w = 18.0;
    let cells = Rect::from_min_max(
        rect.min + vec2(label_w + 4.0, 6.0),
        rect.max - vec2(6.0, 6.0),
    );
    let cw = cells.width() / steps.max(1) as f32;
    let ch = cells.height() / rows.max(1) as f32;
    let cell_at = |p: Pos2| -> Option<(usize, usize)> {
        if !cells.contains(p) {
            return None;
        }
        let s = ((p.x - cells.left()) / cw) as usize;
        let r = ((p.y - cells.top()) / ch) as usize;
        (r < rows && s < steps).then_some((r, s))
    };

    let mut edit = None;
    let paint_id = id.with("paint");
    if resp.clicked()
        && let Some((r, s)) = resp.interact_pointer_pos().and_then(cell_at)
    {
        let on = !grid[r][s];
        set(&mut grid, r, s, on);
        edit = Some(Edit {
            value: to_value(&grid),
            done: true,
        });
    }
    if resp.drag_started()
        && let Some((r, s)) = ui.input(|i| i.pointer.press_origin()).and_then(cell_at)
    {
        store(ui, paint_id, Some(Paint(!grid[r][s])));
    }
    if resp.dragged()
        && let Some(Paint(on)) = load::<Paint>(ui, paint_id)
    {
        let mut changed = false;
        if let Some((r, s)) = ui.input(|i| i.pointer.press_origin()).and_then(cell_at) {
            changed |= set(&mut grid, r, s, on);
        }
        if let Some((r, s)) = resp.interact_pointer_pos().and_then(cell_at) {
            changed |= set(&mut grid, r, s, on);
        }
        if changed {
            edit = Some(Edit {
                value: to_value(&grid),
                done: false,
            });
        }
    }
    if resp.drag_stopped() && load::<Paint>(ui, paint_id).is_some() {
        store::<Paint>(ui, paint_id, None);
        edit = Some(Edit {
            value: to_value(&grid),
            done: true,
        });
    }

    let painter = ui.painter_at(rect);
    let on_color = theme::category_color(ac_core::Category::Sequencing);
    let small = egui::FontId::proportional(theme::FONT_SMALL);
    let hover = resp.hover_pos().and_then(cell_at);
    for (r, row) in grid.iter().enumerate() {
        painter.text(
            pos2(
                rect.left() + 4.0 + label_w / 2.0,
                cells.top() + (r as f32 + 0.5) * ch,
            ),
            Align2::CENTER_CENTER,
            format!("{}", r + 1),
            small.clone(),
            theme::TEXT_FAINT,
        );
        for (s, &on) in row.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(cells.left() + s as f32 * cw, cells.top() + r as f32 * ch),
                vec2(cw, ch),
            )
            .shrink((cw.min(ch) * 0.12).clamp(0.5, 2.0));
            let beat = s / 4 % 2 == 0;
            let fill = if on {
                on_color
            } else if hover == Some((r, s)) {
                theme::WIDGET_BG_HOVER
            } else if beat {
                theme::WIDGET_BG
            } else {
                theme::PANEL_BG
            };
            painter.rect_filled(cell, theme::radius(2.0), fill);
        }
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_node_format_and_pads_what_is_missing() {
        let g = parse(&json!({"rows": 2, "steps": 4, "cells": ["x.x", "X"]}));
        assert_eq!(
            g,
            [
                vec![true, false, true, false],
                vec![true, false, false, false]
            ]
        );
        let g = parse(&json!({}));
        assert_eq!((g.len(), g[0].len()), (4, 16));
        assert!(g.iter().flatten().all(|c| !c));
    }

    #[test]
    fn writes_one_string_per_row() {
        let mut g = parse(&json!({"rows": 2, "steps": 4}));
        assert!(set(&mut g, 1, 2, true));
        assert!(!set(&mut g, 1, 2, true), "already on");
        assert!(!set(&mut g, 5, 0, true), "outside");
        assert_eq!(to_value(&g), json!(["....", "..x."]));
    }

    #[test]
    fn sizes_are_repaired() {
        let g = parse(&json!({"rows": 0, "steps": 1000}));
        assert_eq!((g.len(), g[0].len()), (1, MAX_STEPS));
    }
}
