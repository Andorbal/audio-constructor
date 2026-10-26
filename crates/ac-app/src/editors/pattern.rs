//! The step grid of `Pattern`: one row per output, one column per step,
//! with a tab per bank above it.
//!
//! Click a cell to switch it; drag across cells to paint them all the way
//! the first one went; drag up or down on a cell to set its level, drawn as
//! how full the cell is (DECISIONS D212). The tabs pick the bank shown;
//! `+` adds a copy of it, and right-clicking a tab removes that bank.
//!
//! The node stores the grid as `"cells": ["x...5...", "....x..."]`, one
//! string per row (`x` full, `1`–`9` tenths, `.` off), or a list of such
//! lists for several banks; the node's own parser reads it here.

use ac_core::Config;
use ac_nodes::sequencing::pattern::{FULL, MAX_BANKS, cells_value, grids};
use egui::{Align2, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

use super::banks::{self, Tabs};
use super::{Edit, LiveView, area, load, store};
use crate::theme;

/// Banks, each rows of step levels.
pub type Grids = Vec<Vec<Vec<u8>>>;

/// The grids in `config`.
pub fn parse(config: &Config) -> Grids {
    grids(config)
}

/// The config value for `grids`.
pub fn to_value(grids: &[Vec<Vec<u8>>]) -> serde_json::Value {
    cells_value(grids)
}

/// Set one cell of `bank` to `level`; returns whether it changed.
pub fn set(grids: &mut Grids, bank: usize, row: usize, step: usize, level: u8) -> bool {
    match grids
        .get_mut(bank)
        .and_then(|g| g.get_mut(row))
        .and_then(|r| r.get_mut(step))
    {
        Some(c) if *c != level => {
            *c = level;
            true
        }
        _ => false,
    }
}

/// A cell's level after dragging `dy` points from `start` (up is louder):
/// the full range in [`LEVEL_SPAN`] points, never below a tenth.
pub fn dragged_level(start: u8, dy: f32) -> u8 {
    let steps = (-dy / LEVEL_SPAN * f32::from(FULL)).round();
    (f32::from(start.max(1)) + steps).clamp(1.0, f32::from(FULL)) as u8
}

/// Points of vertical drag from a tenth to full.
pub const LEVEL_SPAN: f32 = 80.0;
/// Points the pointer moves before a drag picks painting or level.
const DECIDE: f32 = 4.0;

/// What a drag in progress does.
#[derive(Clone, Copy)]
enum Drag {
    /// Not yet moved far enough to tell.
    Undecided { row: usize, step: usize },
    /// Painting cells on (at full) or off.
    Paint(bool),
    /// Setting one cell's level, from where it started.
    Level { row: usize, step: usize, start: u8 },
}

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    live: &LiveView,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let mut grids = parse(config);
    let bank_id = id.with("bank");
    let mut bank = load::<usize>(ui, bank_id).unwrap_or(0).min(grids.len() - 1);
    let rows = grids[0].len();
    let steps = grids[0].first().map_or(0, Vec::len);
    let (resp, rect) = area(ui, width, height);
    let label_w = 18.0;
    let strip = Rect::from_min_size(
        rect.min + vec2(label_w + 4.0, 4.0),
        vec2(rect.width() - label_w - 10.0, banks::HEIGHT),
    );
    let tabs = Tabs::new(strip, grids.len(), MAX_BANKS);
    let cells = Rect::from_min_max(
        pos2(strip.left(), strip.bottom() + banks::GAP),
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
    let drag_id = id.with("drag");
    let pointer = resp.interact_pointer_pos();
    if let Some(action) = tabs.action(&resp) {
        let (shown, changed) = banks::apply(&mut grids, bank, action, MAX_BANKS);
        bank = shown;
        if changed {
            edit = Some(Edit {
                value: to_value(&grids),
                done: true,
            });
        }
    } else if resp.clicked()
        && let Some((r, s)) = pointer.and_then(cell_at)
    {
        let level = if grids[bank][r][s] > 0 { 0 } else { FULL };
        set(&mut grids, bank, r, s, level);
        edit = Some(Edit {
            value: to_value(&grids),
            done: true,
        });
    }
    let origin = ui.input(|i| i.pointer.press_origin());
    if resp.drag_started()
        && let Some((row, step)) = origin.and_then(cell_at)
    {
        store(ui, drag_id, Some(Drag::Undecided { row, step }));
    }
    if resp.dragged()
        && let (Some(drag), Some(o), Some(now)) = (load::<Drag>(ui, drag_id), origin, pointer)
    {
        let d = now - o;
        let drag = match drag {
            Drag::Undecided { row, step } if d.length() >= DECIDE => {
                let level = grids[bank][row][step];
                // Up or down on an on cell sets its level; otherwise paint.
                if level > 0 && d.y.abs() > d.x.abs() {
                    Drag::Level {
                        row,
                        step,
                        start: level,
                    }
                } else {
                    Drag::Paint(level == 0)
                }
            }
            other => other,
        };
        store(ui, drag_id, Some(drag));
        let changed = match drag {
            Drag::Undecided { .. } => false,
            Drag::Paint(on) => {
                let level = if on { FULL } else { 0 };
                let mut changed = false;
                for (r, s) in [cell_at(o), cell_at(now)].into_iter().flatten() {
                    // Painting on keeps a cell's level.
                    if !(on && grids[bank][r][s] > 0) {
                        changed |= set(&mut grids, bank, r, s, level);
                    }
                }
                changed
            }
            Drag::Level { row, step, start } => {
                set(&mut grids, bank, row, step, dragged_level(start, d.y))
            }
        };
        if changed {
            edit = Some(Edit {
                value: to_value(&grids),
                done: false,
            });
        }
    }
    if resp.drag_stopped() && load::<Drag>(ui, drag_id).is_some() {
        store::<Drag>(ui, drag_id, None);
        edit = Some(Edit {
            value: to_value(&grids),
            done: true,
        });
    }
    store(ui, bank_id, Some(bank));

    let painter = ui.painter_at(rect);
    let on_color = theme::category_color(ac_core::Category::Sequencing);
    let small = egui::FontId::proportional(theme::FONT_SMALL);
    let hover = resp.hover_pos();
    tabs.paint(&painter, bank, live.bank, hover);
    let hover_cell = hover.and_then(cell_at);
    for (r, row) in grids[bank].iter().enumerate() {
        painter.text(
            pos2(
                rect.left() + 4.0 + label_w / 2.0,
                cells.top() + (r as f32 + 0.5) * ch,
            ),
            Align2::CENTER_CENTER,
            // Rows count from 0, like the node's `trigger_0`, `gate_0` outputs.
            format!("{r}"),
            small.clone(),
            theme::text_faint(),
        );
        for (s, &level) in row.iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(cells.left() + s as f32 * cw, cells.top() + r as f32 * ch),
                vec2(cw, ch),
            )
            .shrink((cw.min(ch) * 0.12).clamp(0.5, 2.0));
            let beat = s / 4 % 2 == 0;
            let well = if hover_cell == Some((r, s)) {
                theme::widget_bg_hover()
            } else if beat {
                theme::widget_bg()
            } else {
                theme::panel_bg()
            };
            painter.rect_filled(cell, theme::radius(2.0), well);
            if level > 0 {
                // The level fills the cell from the bottom.
                let f = f32::from(level) / f32::from(FULL);
                let mut lit = cell;
                lit.min.y = cell.max.y - cell.height() * f;
                painter.rect_filled(lit, theme::radius(2.0), on_color);
                if level < FULL {
                    painter.rect_stroke(
                        cell,
                        theme::radius(2.0),
                        Stroke::new(1.0, theme::fade(on_color, 0.6)),
                        StrokeKind::Inside,
                    );
                }
            }
        }
    }
    // The step playing, in the bank playing.
    if let Some(s) = live.step.filter(|&s| s < steps)
        && live.bank.is_none_or(|b| b == bank)
    {
        let column = Rect::from_min_size(
            pos2(cells.left() + s as f32 * cw, cells.top()),
            vec2(cw, ch * rows as f32),
        );
        painter.rect_stroke(
            column,
            theme::radius(2.0),
            Stroke::new(1.5, theme::play()),
            StrokeKind::Inside,
        );
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_node_format_and_pads_what_is_missing() {
        let g = parse(&json!({"rows": 2, "steps": 4, "cells": ["x.5", "X"]}));
        assert_eq!(g, [vec![vec![10, 0, 5, 0], vec![10, 0, 0, 0]]]);
        let g = parse(&json!({}));
        assert_eq!((g.len(), g[0].len(), g[0][0].len()), (1, 4, 16));
        assert!(g.iter().flatten().flatten().all(|&c| c == 0));
    }

    #[test]
    fn writes_one_string_per_row() {
        let mut g = parse(&json!({"rows": 2, "steps": 4}));
        assert!(set(&mut g, 0, 1, 2, FULL));
        assert!(!set(&mut g, 0, 1, 2, FULL), "already on");
        assert!(!set(&mut g, 0, 5, 0, FULL), "outside");
        assert!(set(&mut g, 0, 1, 3, 3));
        assert_eq!(to_value(&g), json!(["....", "..x3"]));
    }

    #[test]
    fn dragging_up_is_louder() {
        assert_eq!(dragged_level(FULL, 16.0), 8);
        assert_eq!(dragged_level(5, -LEVEL_SPAN), FULL);
        assert_eq!(dragged_level(5, 500.0), 1, "never off");
    }

    #[test]
    fn sizes_are_repaired() {
        let g = parse(&json!({"rows": 0, "steps": 1000}));
        assert_eq!((g[0].len(), g[0][0].len()), (1, 64));
    }
}
