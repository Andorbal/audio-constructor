//! The drawn editors outside the Node tab (ROADMAP M4.5): on a panel card,
//! live and editable; in several windows at once; docked under the canvas.

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{NodeId, NodeInstance, Panel, PanelItem, Project};
use egui::{Key, Modifiers, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness(project: Project) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            ..AppOptions::headless()
        },
    );
    app.open_unsaved(project);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, EditorApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn pattern() -> NodeInstance {
    NodeInstance::new(NodeId(1), "pattern")
        .with_config(json!({ "steps": 4, "rows": 2, "cells": ["....", "...."] }))
}

fn cells(h: &Harness<'static, EditorApp>) -> serde_json::Value {
    h.state().document().project.graph.nodes[0].config["cells"].clone()
}

/// The centre of cell (`row`, `step`) of a 2 × 4 grid drawn in `r` (the
/// editor's insets: an 18 pt label column, the bank tabs, margins).
fn cell_in(r: Rect, row: usize, step: usize) -> Pos2 {
    let cells = Rect::from_min_max(r.min + vec2(22.0, 26.0), r.max - vec2(6.0, 6.0));
    Pos2::new(
        cells.left() + (step as f32 + 0.5) * cells.width() / 4.0,
        cells.top() + (row as f32 + 0.5) * cells.height() / 2.0,
    )
}

#[test]
fn a_pinned_pattern_is_its_grid_and_a_click_switches_a_cell() {
    let mut p = Project::default();
    p.graph.nodes.push(pattern());
    let mut panel = Panel::new("Beat");
    panel.items.push(PanelItem::new(NodeId(1)));
    p.panels.push(panel);
    let mut h = harness(p);
    h.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    h.run_steps(3);
    let cell = h.state().perform.cells[0].clone();
    let (editor, r) = cell.editor.expect("an editor on the card");
    assert_eq!(editor, ac_app::editors::Editor::Pattern);
    assert!(cell.body.is_none());
    let before = cell.rect;
    click(&mut h, cell_in(r, 1, 2));
    assert_eq!(cells(&h), json!(["....", "..x."]));
    assert_eq!(
        h.state().perform.cells[0].rect,
        before,
        "the card stays put"
    );
    // A drag on the grid paints; it doesn't move the card.
    let (a, b) = (cell_in(r, 0, 0), cell_in(r, 0, 3));
    h.hover_at(a);
    h.run_steps(2);
    h.drag_at(a);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(a + (b - a) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(3);
    assert_eq!(cells(&h), json!(["xxxx", "..x."]));
    assert_eq!(h.state().perform.cells[0].rect, before);
}

#[test]
fn editors_open_in_several_windows_and_one_docks_under_the_canvas() {
    let mut p = Project::default();
    p.graph.nodes.push(pattern().at(0.0, 0.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "midi_pattern").at(0.0, 300.0));
    let mut h = harness(p);
    {
        let app = h.state_mut();
        app.editor_windows.toggle(&app.doc, NodeId(1));
        app.editor_windows.toggle(&app.doc, NodeId(2));
    }
    h.run_steps(4);
    assert_eq!(h.state().editor_windows.open.len(), 2);
    assert!(h.query_by_label("Pattern").is_some());
    assert!(h.query_by_label("MIDI Pattern").is_some());
    // Docking the Pattern takes it out of its window.
    {
        let app = h.state_mut();
        app.editor_windows.dock(&app.doc, NodeId(1));
    }
    h.run_steps(4);
    let w = &h.state().editor_windows;
    assert_eq!(w.open.len(), 1);
    assert_eq!(w.docked.as_ref().map(|o| o.node), Some(NodeId(1)));
    // Removing the node forgets its editor.
    h.state_mut()
        .doc
        .graph_mut_raw()
        .nodes
        .retain(|n| n.id != NodeId(1));
    h.run_steps(2);
    assert!(h.state().editor_windows.docked.is_none());
}
