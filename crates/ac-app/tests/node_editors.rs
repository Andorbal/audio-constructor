//! The node editors driven by pointer input without a window
//! (egui_kittest): each test shows one editor over a config, plays a
//! gesture, and checks the config value the editor hands back.

use ac_app::editors::{self, Editor, curve::Space};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

const W: f32 = 320.0;
const H: f32 = 160.0;

struct State {
    editor: Editor,
    config: Value,
    /// Where the editor's area was drawn.
    rect: Rect,
    edits: usize,
    done: usize,
}

fn harness(editor: Editor, config: Value) -> Harness<'static, State> {
    let state = State {
        editor,
        config,
        rect: Rect::NOTHING,
        edits: 0,
        done: 0,
    };
    let mut h = Harness::builder()
        .with_size(vec2(W + 40.0, H + 80.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, s: &mut State| {
                let before = ui.cursor().min;
                let edit = editors::show(ui, s.editor, &s.config, "test", W, H);
                // The area is the last thing the editor allocated.
                let after = ui.min_rect();
                s.rect = Rect::from_min_size(pos2(before.x, after.max.y - H), vec2(W, H));
                if let Some(e) = edit {
                    s.edits += 1;
                    s.done += usize::from(e.done);
                    if let Some(o) = s.config.as_object_mut() {
                        o.insert(s.editor.key().into(), e.value);
                    }
                }
            },
            state,
        );
    h.run_steps(2);
    h
}

fn steps(h: &mut Harness<'static, State>, n: usize) {
    h.run_steps(n);
}

fn click(h: &mut Harness<'static, State>, at: Pos2) {
    h.hover_at(at);
    steps(h, 1);
    h.drag_at(at);
    steps(h, 1);
    h.drop_at(at);
    steps(h, 2);
}

fn button(h: &mut Harness<'static, State>, at: Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn right_click(h: &mut Harness<'static, State>, at: Pos2) {
    h.hover_at(at);
    steps(h, 1);
    button(h, at, PointerButton::Secondary, true);
    steps(h, 1);
    button(h, at, PointerButton::Secondary, false);
    steps(h, 2);
}

fn double_click(h: &mut Harness<'static, State>, at: Pos2) {
    h.hover_at(at);
    steps(h, 1);
    for _ in 0..2 {
        button(h, at, PointerButton::Primary, true);
        steps(h, 1);
        button(h, at, PointerButton::Primary, false);
        steps(h, 1);
    }
    steps(h, 2);
}

/// Press at `from`, move to `to` in small steps, release.
fn drag(h: &mut Harness<'static, State>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    steps(h, 1);
    h.drag_at(from);
    steps(h, 1);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        steps(h, 1);
    }
    h.drop_at(to);
    steps(h, 2);
}

/// A point at fractions (`fx`, `fy`) of the area.
fn at(h: &Harness<'static, State>, fx: f32, fy: f32) -> Pos2 {
    let r = h.state().rect;
    pos2(r.left() + fx * r.width(), r.top() + fy * r.height())
}

/// Centre of cell (`row`, `step`) of a Pattern grid of `rows` × `steps`
/// (the grid inset matches the editor's: an 18 pt row label and margins).
fn cell(h: &Harness<'static, State>, rows: usize, steps: usize, row: usize, step: usize) -> Pos2 {
    let r = h.state().rect;
    let cells = Rect::from_min_max(r.min + vec2(22.0, 6.0), r.max - vec2(6.0, 6.0));
    pos2(
        cells.left() + (step as f32 + 0.5) * cells.width() / steps as f32,
        cells.top() + (row as f32 + 0.5) * cells.height() / rows as f32,
    )
}

#[test]
fn pattern_cells_switch_on_click_and_paint_on_drag() {
    let mut h = harness(
        Editor::Pattern,
        json!({"rows": 2, "steps": 4, "cells": ["....", "...."]}),
    );
    let p = cell(&h, 2, 4, 1, 2);
    click(&mut h, p);
    assert_eq!(h.state().config["cells"], json!(["....", "..x."]));
    click(&mut h, p);
    assert_eq!(h.state().config["cells"], json!(["....", "...."]));
    // A drag along the top row paints every cell it crosses.
    let (a, b) = (cell(&h, 2, 4, 0, 0), cell(&h, 2, 4, 0, 3));
    drag(&mut h, a, b);
    assert_eq!(h.state().config["cells"], json!(["xxxx", "...."]));
    assert!(h.state().done >= 3, "each gesture ends");
}

#[test]
fn piano_roll_draws_moves_and_deletes_notes() {
    let mut h = harness(Editor::PianoRoll, json!({"length": 4, "notes": []}));
    // An empty clip shows C3..C5 (48..72); click a row a quarter of the way in.
    let p = at(&h, 0.3, 0.5);
    click(&mut h, p);
    let notes = h.state().config["notes"].as_array().cloned().unwrap();
    assert_eq!(notes.len(), 1);
    let start = notes[0][0].as_f64().unwrap();
    assert_eq!(start % 0.25, 0.0, "on the grid");
    // Right-click deletes it.
    let note_at = at(&h, 0.3 + 0.02, 0.5);
    right_click(&mut h, note_at);
    assert_eq!(h.state().config["notes"], json!([]));
    // Dragging in empty space draws a longer note.
    let (a, b) = (at(&h, 0.1, 0.3), at(&h, 0.6, 0.3));
    drag(&mut h, a, b);
    let notes = h.state().config["notes"].as_array().cloned().unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0][1].as_f64().unwrap() >= 1.5, "{notes:?}");
}

#[test]
fn curve_points_move_and_new_ones_appear_on_double_click() {
    let mut h = harness(
        Editor::Curve(Space::graph()),
        json!({"points": [[0, 0], [0.5, 1], [1, 0]]}),
    );
    // The middle point sits at the top centre of the plot (10 pt inset).
    let r = h.state().rect.shrink(10.0);
    let top = pos2(r.center().x, r.top());
    let lower = pos2(r.left() + 0.7 * r.width(), r.top() + 0.5 * r.height());
    drag(&mut h, top, lower);
    let p = &h.state().config["points"];
    let (x, y) = (p[1][0].as_f64().unwrap(), p[1][1].as_f64().unwrap());
    assert!((x - 0.7).abs() < 0.02 && (y - 0.5).abs() < 0.02, "{p}");
    double_click(
        &mut h,
        pos2(r.left() + 0.25 * r.width(), r.bottom() - 0.2 * r.height()),
    );
    assert_eq!(h.state().config["points"].as_array().unwrap().len(), 4);
}

#[test]
fn band_edges_split_and_merge() {
    let mut h = harness(Editor::Bands, json!({"edges": [200, 2000]}));
    // 20 Hz .. 20 kHz across the plot (8 pt inset): 632 Hz is the middle.
    let p = at(&h, 0.5, 0.5);
    double_click(&mut h, p);
    let edges = h.state().config["edges"].as_array().cloned().unwrap();
    assert_eq!(edges.len(), 3, "{edges:?}");
    let mid = edges[1].as_f64().unwrap();
    assert!((400.0..1000.0).contains(&mid), "{mid}");
    let p = at(&h, 0.5, 0.5);
    right_click(&mut h, p);
    assert_eq!(h.state().config["edges"], json!([200.0, 2000.0]));
}

#[test]
fn step_values_follow_the_pointer() {
    let mut h = harness(Editor::Steps, json!({"steps": 4, "values": [0, 0, 0, 0]}));
    // The range is 0..1; a click near the top of the second bar sets it high.
    let p = at(&h, 0.375, 0.1);
    click(&mut h, p);
    let v = h.state().config["values"].as_array().cloned().unwrap();
    assert!(v[1].as_f64().unwrap() > 0.8, "{v:?}");
    assert_eq!(v[0], json!(0.0));
}

#[test]
fn zones_move_resize_appear_and_go() {
    // 320 points over 128 notes: 2.5 points a note.
    let mut h = harness(Editor::Zones, json!({ "zones": [[60, 71, 0, 1]] }));
    let zones = |h: &Harness<'static, State>| h.state().config["zones"].clone();
    // Drag the zone's body a dozen notes right.
    let mid = at(&h, 66.0 / 128.0, 0.5);
    drag(&mut h, mid, mid + vec2(30.0, 0.0));
    assert_eq!(zones(&h), json!([[72.0, 83.0, 0.0, 1.0]]));
    // Pull its top edge down to half velocity.
    let top = at(&h, 78.0 / 128.0, 0.0) + vec2(0.0, 1.0);
    drag(&mut h, top, top + vec2(0.0, H / 2.0));
    assert_eq!(zones(&h)[0][3], json!(0.5));
    // Double-click empty space adds one; right-click removes it.
    let empty = at(&h, 20.0 / 128.0, 0.5);
    double_click(&mut h, empty);
    assert_eq!(zones(&h).as_array().map(Vec::len), Some(2));
    right_click(&mut h, empty);
    assert_eq!(zones(&h).as_array().map(Vec::len), Some(1));
}
