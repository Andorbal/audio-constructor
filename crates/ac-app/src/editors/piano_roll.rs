//! The piano roll of `MidiPattern`: notes over beats.
//!
//! Click or drag in empty space to draw a note, drag a note to move it,
//! drag its right edge to change its length, and right-click (or
//! double-click) it to delete it. Positions snap to a sixteenth note; hold
//! Shift to place freely. The tabs above are the clip's banks, as on the
//! Pattern grid.
//!
//! The node stores `"notes": [[start_beat, length_beats, note, velocity],
//! ...]` (a list of such lists for several banks) and the clip's `length`
//! in beats.

use ac_core::{Config, ConfigExt};
use ac_nodes::sequencing::midi_pattern::{MAX_BANKS, note_banks};
use egui::{Align2, CursorIcon, Pos2, Rect, Stroke, pos2, vec2};

use super::banks::{self, Tabs};
use super::{Edit, area, grid, load, snap, store};
use crate::theme;

/// Snap grid in beats (a sixteenth note).
pub const GRID: f64 = 0.25;
/// Velocity of drawn notes.
pub const DEFAULT_VELOCITY: u8 = 100;
/// Notes a clip shows at least, in semitones.
const MIN_RANGE: i32 = 24;
const KEYS_W: f32 = 26.0;
/// Pixels at a note's right end that resize it.
const EDGE: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: f64,
    pub len: f64,
    pub note: u8,
    pub vel: u8,
}

/// The clip length and the notes of its first bank in `config`.
pub fn parse(config: &Config) -> (f64, Vec<Note>) {
    let (length, mut banks) = parse_banks(config);
    (length, banks.swap_remove(0))
}

/// The clip length and every bank's notes in `config`. Entries that aren't
/// four numbers are skipped; values are repaired into range.
pub fn parse_banks(config: &Config) -> (f64, Vec<Vec<Note>>) {
    let length = config.get_f64_or("length", 4.0).clamp(0.25, 1024.0);
    let banks = note_banks(config)
        .into_iter()
        .map(|list| {
            list.iter()
                .filter_map(|n| {
                    let a = n.as_array()?;
                    let num = |k: usize| a.get(k).and_then(serde_json::Value::as_f64);
                    let (start, len, note) = (num(0)?, num(1)?, num(2)?);
                    let vel = num(3).unwrap_or(f64::from(DEFAULT_VELOCITY));
                    (start.is_finite() && len.is_finite() && len > 0.0).then(|| Note {
                        start: start.max(0.0),
                        len,
                        note: note.round().clamp(0.0, 127.0) as u8,
                        vel: vel.round().clamp(1.0, 127.0) as u8,
                    })
                })
                .collect()
        })
        .collect();
    (length, banks)
}

/// The config value for every bank: one bank's notes alone, several as a
/// list of lists.
pub fn banks_value(banks: &[Vec<Note>]) -> serde_json::Value {
    match banks {
        [one] => to_value(one),
        _ => serde_json::Value::Array(banks.iter().map(|b| to_value(b)).collect()),
    }
}

/// The config value for `notes`, sorted by start then pitch.
pub fn to_value(notes: &[Note]) -> serde_json::Value {
    let mut sorted = notes.to_vec();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.note.cmp(&b.note)));
    serde_json::Value::Array(
        sorted
            .iter()
            .map(|n| serde_json::json!([round(n.start), round(n.len), n.note, n.vel]))
            .collect(),
    )
}

/// Six decimals is far finer than any grid and keeps the file readable.
fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// The note under (`beat`, `pitch`), the one drawn last if several overlap.
pub fn note_at(notes: &[Note], beat: f64, pitch: i32) -> Option<usize> {
    notes
        .iter()
        .rposition(|n| i32::from(n.note) == pitch && beat >= n.start && beat < n.start + n.len)
}

/// Add a note; returns its index.
pub fn add(notes: &mut Vec<Note>, start: f64, len: f64, pitch: i32) -> usize {
    notes.push(Note {
        start: start.max(0.0),
        len: len.max(1e-3),
        note: pitch.clamp(0, 127) as u8,
        vel: DEFAULT_VELOCITY,
    });
    notes.len() - 1
}

/// Move note `i` to start at `start` (inside the clip) on `pitch`.
pub fn move_note(notes: &mut [Note], i: usize, start: f64, pitch: i32, length: f64) {
    if let Some(n) = notes.get_mut(i) {
        n.start = start.clamp(0.0, (length - n.len).max(0.0));
        n.note = pitch.clamp(0, 127) as u8;
    }
}

/// Set note `i`'s length, at least `min`.
pub fn resize(notes: &mut [Note], i: usize, len: f64, min: f64) {
    if let Some(n) = notes.get_mut(i) {
        n.len = len.max(min.max(1e-3));
    }
}

/// The pitches shown: the notes with a little room, at least two octaves,
/// two octaves from C3 when empty.
pub fn pitch_range(notes: &[Note]) -> (i32, i32) {
    let (lo, hi) = notes.iter().fold((i32::MAX, i32::MIN), |(lo, hi), n| {
        (lo.min(i32::from(n.note)), hi.max(i32::from(n.note)))
    });
    let (lo, hi) = if lo > hi { (48, 72) } else { (lo - 3, hi + 3) };
    let grow = (MIN_RANGE - (hi - lo)).max(0);
    let lo = (lo - grow / 2).clamp(0, 127 - MIN_RANGE);
    let hi = (hi + (grow - grow / 2)).max(lo + MIN_RANGE).min(127);
    (lo, hi)
}

#[derive(Clone, Copy)]
enum Drag {
    Move {
        index: usize,
        /// Beats from the note's start to where it was grabbed.
        grab: f64,
    },
    Resize {
        index: usize,
    },
}

pub(super) fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let (length, mut all) = parse_banks(config);
    let bank_id = id.with("bank");
    let mut bank = load::<usize>(ui, bank_id).unwrap_or(0).min(all.len() - 1);
    let (resp, rect) = area(ui, width, height);
    let tabs = Tabs::new(
        Rect::from_min_size(
            rect.min + vec2(KEYS_W, 4.0),
            vec2(rect.width() - KEYS_W - 6.0, banks::HEIGHT),
        ),
        all.len(),
        MAX_BANKS,
    );
    let mut edit = None;
    if let Some(action) = tabs.action(&resp) {
        let (shown, changed) = banks::apply(&mut all, bank, action, MAX_BANKS);
        bank = shown;
        if changed {
            edit = Some(Edit {
                value: banks_value(&all),
                done: true,
            });
        }
    }
    store(ui, bank_id, Some(bank));
    let mut notes = all[bank].clone();
    // Every bank, with this one's notes as edited.
    let value = |notes: &[Note]| {
        let mut all = all.clone();
        all[bank] = notes.to_vec();
        banks_value(&all)
    };
    let body = Rect::from_min_max(
        pos2(rect.left(), rect.top() + 4.0 + banks::HEIGHT + banks::GAP),
        rect.max,
    );
    let drag_id = id.with("drag");
    // Keep the view still while a drag moves notes past its edge.
    let range_id = id.with("range");
    let (lo, hi) = match load::<(i32, i32)>(ui, range_id) {
        Some(r) if load::<Drag>(ui, drag_id).is_some() => r,
        _ => pitch_range(&notes),
    };
    store(ui, range_id, Some((lo, hi)));
    let roll = Rect::from_min_max(body.min + vec2(KEYS_W, 0.0), body.max);
    let rows = (hi - lo + 1).max(1);
    let row_h = roll.height() / rows as f32;
    let beat_w = roll.width() / length as f32;
    let fine = ui.input(|i| i.modifiers.shift);
    let step = if fine { 0.0 } else { GRID };
    let at = |p: Pos2| {
        let beat = f64::from((p.x - roll.left()) / beat_w);
        let pitch = hi - ((p.y - roll.top()) / row_h).floor() as i32;
        (beat, pitch)
    };
    let note_rect = |n: &Note| {
        let x0 = roll.left() + n.start as f32 * beat_w;
        let x1 = roll.left() + ((n.start + n.len).min(length)) as f32 * beat_w;
        let y = roll.top() + (hi - i32::from(n.note)) as f32 * row_h;
        Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 2.0), y + row_h))
    };
    let on_edge = |n: &Note, p: Pos2| {
        let r = note_rect(n);
        r.y_range().contains(p.y) && (p.x - r.right()).abs() <= EDGE
    };

    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    if let Some(p) = resp.hover_pos()
        && notes.iter().any(|n| on_edge(n, p))
    {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    // Press on a note: move or resize it. Press on empty space: draw one.
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
        && roll.contains(p)
    {
        let (beat, pitch) = at(p);
        let drag = if let Some(i) = notes.iter().rposition(|n| on_edge(n, p)) {
            Drag::Resize { index: i }
        } else if let Some(i) = note_at(&notes, beat, pitch) {
            Drag::Move {
                index: i,
                grab: beat - notes[i].start,
            }
        } else {
            let start = if fine {
                beat
            } else {
                (beat / GRID).floor() * GRID
            };
            let i = add(&mut notes, start, GRID, pitch);
            Drag::Resize { index: i }
        };
        store(ui, drag_id, Some(drag));
    }
    if resp.dragged()
        && let (Some(d), Some(p)) = (load::<Drag>(ui, drag_id), pointer)
    {
        let (beat, pitch) = at(p);
        match d {
            Drag::Move { index, grab } => {
                move_note(&mut notes, index, snap(beat - grab, step), pitch, length);
            }
            Drag::Resize { index } => {
                if let Some(n) = notes.get(index) {
                    let end = snap(beat, step).min(length);
                    let len = end - n.start;
                    resize(&mut notes, index, len, if fine { 1e-3 } else { GRID });
                }
            }
        }
        edit = Some(Edit {
            value: value(&notes),
            done: false,
        });
    }
    if resp.drag_stopped() && load::<Drag>(ui, drag_id).is_some() {
        store::<Drag>(ui, drag_id, None);
        edit = Some(Edit {
            value: value(&notes),
            done: true,
        });
    }
    if (resp.clicked() || resp.double_clicked() || resp.secondary_clicked())
        && let Some(p) = pointer
        && roll.contains(p)
    {
        let (beat, pitch) = at(p);
        match note_at(&notes, beat, pitch) {
            Some(i) if resp.secondary_clicked() || resp.double_clicked() => {
                notes.remove(i);
                edit = Some(Edit {
                    value: value(&notes),
                    done: true,
                });
            }
            None if resp.clicked() => {
                let start = if fine {
                    beat
                } else {
                    (beat / GRID).floor() * GRID
                };
                add(
                    &mut notes,
                    start.min(length - GRID).max(0.0),
                    2.0 * GRID,
                    pitch,
                );
                edit = Some(Edit {
                    value: value(&notes),
                    done: true,
                });
            }
            _ => {}
        }
    }

    // Paint.
    let painter = ui.painter_at(rect);
    let small = egui::FontId::proportional(theme::FONT_SMALL.min(row_h * 1.1).max(7.0));
    for k in 0..rows {
        let pitch = hi - k;
        let y = roll.top() + k as f32 * row_h;
        let black = matches!(pitch.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
        painter.rect_filled(
            Rect::from_min_size(pos2(body.left(), y), vec2(KEYS_W - 2.0, row_h)),
            0.0,
            if black {
                theme::meter_bg()
            } else {
                theme::text_muted()
            },
        );
        if black {
            painter.rect_filled(
                Rect::from_min_size(pos2(roll.left(), y), vec2(roll.width(), row_h)),
                0.0,
                theme::panel_bg(),
            );
        }
        if pitch.rem_euclid(12) == 0 {
            painter.line_segment(
                [pos2(roll.left(), y + row_h), pos2(roll.right(), y + row_h)],
                Stroke::new(1.0, grid(true)),
            );
            if row_h >= 7.0 {
                painter.text(
                    pos2(body.left() + 2.0, y + row_h / 2.0),
                    Align2::LEFT_CENTER,
                    format!("C{}", pitch / 12 - 1),
                    small.clone(),
                    theme::well_bg(),
                );
            }
        }
    }
    let quarter_lines = (length / GRID).round() as usize;
    for k in 0..=quarter_lines {
        let beat = k as f64 * GRID;
        let x = roll.left() + beat as f32 * beat_w;
        let bar = (beat % 4.0).abs() < 1e-9;
        let whole = (beat % 1.0).abs() < 1e-9;
        if !whole && beat_w * (GRID as f32) < 6.0 {
            continue;
        }
        painter.line_segment(
            [pos2(x, roll.top()), pos2(x, roll.bottom())],
            Stroke::new(
                if bar { 1.5 } else { 1.0 },
                if whole {
                    grid(bar)
                } else {
                    theme::fade(grid(false), 0.5)
                },
            ),
        );
    }
    tabs.paint(&painter, bank, resp.hover_pos());
    let color = theme::category_color(ac_core::Category::Sequencing);
    for n in &notes {
        let r = note_rect(n);
        let alpha = 0.45 + 0.55 * f32::from(n.vel) / 127.0;
        painter.rect_filled(
            r.shrink2(vec2(0.5, 1.0)),
            theme::radius(2.0),
            theme::fade(color, alpha),
        );
        painter.rect_stroke(
            r.shrink2(vec2(0.5, 1.0)),
            theme::radius(2.0),
            Stroke::new(1.0, theme::fade(theme::text_strong(), 0.4)),
            egui::StrokeKind::Inside,
        );
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_the_node_format() {
        let (len, n) = parse(&json!({
            "length": 8,
            "notes": [[1, 0.5, 64, 90], [0, 1, 60, 100], "junk", [2, 0, 60, 1], [3, 1, 200, 0]]
        }));
        assert_eq!(len, 8.0);
        assert_eq!(n.len(), 3, "the zero-length note and junk are skipped");
        assert_eq!(n[2].note, 127);
        assert_eq!(n[2].vel, 1);
        assert_eq!(
            to_value(&n[..2]),
            json!([[0.0, 1.0, 60, 100], [1.0, 0.5, 64, 90]])
        );
        assert_eq!(parse(&json!({})), (4.0, Vec::new()));
    }

    #[test]
    fn banks_read_and_write_as_lists_of_lists() {
        let c = json!({ "notes": [[[0, 1, 60, 100]], []] });
        let (_, b) = parse_banks(&c);
        assert_eq!((b.len(), b[0].len(), b[1].len()), (2, 1, 0));
        assert_eq!(banks_value(&b), json!([[[0.0, 1.0, 60, 100]], []]));
        assert_eq!(banks_value(&b[..1]), json!([[0.0, 1.0, 60, 100]]));
        assert_eq!(parse(&c).1.len(), 1, "the first bank");
    }

    #[test]
    fn finds_moves_and_resizes_notes() {
        let mut n = Vec::new();
        let i = add(&mut n, 1.0, 0.5, 60);
        assert_eq!(note_at(&n, 1.25, 60), Some(i));
        assert_eq!(note_at(&n, 1.5, 60), None, "end is exclusive");
        assert_eq!(note_at(&n, 1.25, 61), None);
        move_note(&mut n, i, 3.8, 62, 4.0);
        assert_eq!((n[0].start, n[0].note), (3.5, 62), "stays inside the clip");
        move_note(&mut n, i, -1.0, 500, 4.0);
        assert_eq!((n[0].start, n[0].note), (0.0, 127));
        resize(&mut n, i, 0.01, GRID);
        assert_eq!(n[0].len, GRID);
    }

    #[test]
    fn the_view_fits_the_notes() {
        assert_eq!(pitch_range(&[]), (48, 72));
        let mut n = Vec::new();
        add(&mut n, 0.0, 1.0, 60);
        let (lo, hi) = pitch_range(&n);
        assert!(lo <= 57 && hi >= 63 && hi - lo >= MIN_RANGE);
        add(&mut n, 0.0, 1.0, 100);
        let (lo, hi) = pitch_range(&n);
        assert!(lo <= 57 && hi >= 103);
        add(&mut n, 0.0, 1.0, 127);
        assert_eq!(pitch_range(&n).1, 127);
    }
}
