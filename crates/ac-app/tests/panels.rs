//! Performance panels in the editor (egui_kittest, null audio stream):
//! pinning with `P`, playing pinned controls on a panel, several panels,
//! and full-screen performance mode.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::icons;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Panel, PanelItem, Project, Value};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, ViewportCommand, ViewportId, pos2};
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
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn steps(h: &mut Harness<'static, EditorApp>, n: usize) {
    h.run_steps(n);
}

fn press(h: &mut Harness<'static, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    steps(h, 3);
}

/// A key going down or up as its own event (one frame each).
fn key(h: &Harness<'static, EditorApp>, key: Key, pressed: bool) {
    h.event(Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}

fn button(h: &Harness<'static, EditorApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn click(h: &mut Harness<'static, EditorApp>, p: Pos2) {
    h.hover_at(p);
    steps(h, 2);
    button(h, p, true);
    button(h, p, false);
    steps(h, 3);
}

/// Drag with the primary button in small moves, as a hand would.
fn drag(h: &mut Harness<'static, EditorApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    steps(h, 2);
    h.drag_at(from);
    steps(h, 2);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        h.hover_at(from + (to - from) * t);
        steps(h, 1);
    }
    h.drop_at(to);
    steps(h, 4);
}

/// Select a node on the canvas by clicking its header.
fn select(h: &mut Harness<'static, EditorApp>, id: NodeId) {
    let r = h
        .state()
        .canvas
        .node_screen_rect(id)
        .expect("the node is drawn");
    click(h, r.center_top() + egui::vec2(0.0, 10.0));
    assert!(h.state().document().selection.contains(&id));
}

fn param(h: &Harness<'static, EditorApp>, id: NodeId, socket: &str) -> Option<Value> {
    h.state()
        .document()
        .project
        .graph
        .node(id)
        .and_then(|n| n.params.get(socket).cloned())
}

/// What the engine was last told for an input.
fn engine_param(h: &Harness<'static, EditorApp>, id: NodeId, socket: &str) -> Option<Value> {
    h.state()
        .engine
        .handle
        .project()
        .graph
        .node(id)
        .and_then(|n| n.params.get(socket).cloned())
}

fn float(v: Option<Value>) -> f64 {
    v.and_then(|v| v.as_f64()).expect("a number")
}

/// A pinned node's widget on the panel.
fn panel_body(h: &Harness<'static, EditorApp>, id: NodeId) -> Rect {
    h.state()
        .perform
        .body_rect(id)
        .expect("the node's widget is on the panel")
}

fn pinned(h: &Harness<'static, EditorApp>, panel: usize) -> Vec<NodeId> {
    h.state().document().project.panels[panel]
        .items
        .iter()
        .map(|i| i.node)
        .collect()
}

fn viewport_commands(h: &Harness<'static, EditorApp>) -> Vec<ViewportCommand> {
    h.output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

/// Nodes in a row: each kind with its config.
fn nodes(list: &[(&str, serde_json::Value)]) -> Project {
    let mut p = Project::default();
    for (i, (kind, config)) in list.iter().enumerate() {
        let mut n = NodeInstance::new(NodeId(i as u64 + 1), *kind).at(260.0 * i as f32, 0.0);
        n.config = config.clone();
        p.graph.nodes.push(n);
    }
    p
}

fn with_panel(mut p: Project, name: &str, ids: &[u64]) -> Project {
    let mut panel = Panel::new(name);
    panel.items = ids.iter().map(|i| PanelItem::new(NodeId(*i))).collect();
    p.panels.push(panel);
    p
}

#[test]
fn pin_a_slider_and_play_it_on_a_panel() {
    let slider = NodeId(1);
    let mut h = harness(nodes(&[
        ("slider", json!({"min": 0, "max": 10})),
        ("gain", json!({})),
    ]));
    select(&mut h, slider);
    press(&mut h, Modifiers::NONE, Key::P);
    {
        let doc = h.state().document();
        assert_eq!(doc.project.panels.len(), 1);
        assert_eq!(doc.project.panels[0].name, "Panel 1");
        assert_eq!(doc.undo_label(), Some("Pin to panel"));
    }
    assert_eq!(pinned(&h, 0), vec![slider]);
    // Only controls and displays pin: the gain doesn't.
    select(&mut h, NodeId(2));
    press(&mut h, Modifiers::NONE, Key::P);
    assert_eq!(pinned(&h, 0), vec![slider]);

    // Ctrl+Tab shows the panel, with the slider's widget and no graph.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    assert!(h.state().panels_shown());
    assert_eq!(h.state().perform.cells.len(), 1);
    let b = panel_body(&h, slider);
    // Grab it a quarter along and drag to three quarters: the value
    // follows, in the document and the engine alike.
    let y = b.center().y;
    drag(
        &mut h,
        pos2(b.min.x + b.width() * 0.25, y),
        pos2(b.min.x + b.width() * 0.75, y),
    );
    let v = float(param(&h, slider, "value"));
    assert!((v - 7.5).abs() < 0.05, "{v}");
    assert_eq!(
        engine_param(&h, slider, "value"),
        param(&h, slider, "value")
    );
    // The drag was one undo step, and undo works from the panel.
    assert_eq!(h.state().document().undo_label(), Some("Change value"));
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(param(&h, slider, "value"), None);
    assert_eq!(pinned(&h, 0), vec![slider], "the pin stays");

    // Canvas shortcuts don't reach the hidden graph: X deletes nothing.
    press(&mut h, Modifiers::NONE, Key::X);
    assert_eq!(h.state().document().project.graph.nodes.len(), 2);
    // Ctrl+Tab goes back to the canvas.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    assert!(!h.state().panels_shown());
    // P on a pinned node unpins it from the current panel.
    select(&mut h, slider);
    press(&mut h, Modifiers::NONE, Key::P);
    assert!(pinned(&h, 0).is_empty());
}

#[test]
fn a_button_on_a_panel_is_momentary() {
    let mut h = harness(with_panel(nodes(&[("button", json!({}))]), "Pads", &[1]));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let c = panel_body(&h, NodeId(1)).center();
    h.hover_at(c);
    steps(&mut h, 2);
    button(&h, c, true);
    steps(&mut h, 2);
    assert_eq!(
        engine_param(&h, NodeId(1), "pressed"),
        Some(Value::Bool(true))
    );
    assert_eq!(
        h.state().perform.controls.held_inputs(NodeId(1)),
        vec!["pressed"]
    );
    button(&h, c, false);
    steps(&mut h, 4);
    assert_eq!(
        engine_param(&h, NodeId(1), "pressed"),
        Some(Value::Bool(false))
    );
    assert!(h.state().perform.controls.held_inputs(NodeId(1)).is_empty());
    // Holding a button isn't an edit.
    assert_eq!(param(&h, NodeId(1), "pressed"), None);
    assert!(!h.state().document().is_dirty());
    assert!(!h.state().document().can_undo());
}

#[test]
fn several_panels_add_rename_and_switch() {
    let mut h = harness(with_panel(
        nodes(&[("knob", json!({})), ("toggle", json!({}))]),
        "Panel 1",
        &[1],
    ));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    // The + tab adds a panel, shows it, and starts naming it.
    h.get_by_label(icons::PLUS).click();
    steps(&mut h, 3);
    assert_eq!(h.state().document().project.panels.len(), 2);
    assert_eq!(h.state().perform.current, 1);
    assert!(h.state().perform.is_typing());
    h.event(Event::Text("Mixer".into()));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!h.state().perform.is_typing());
    assert_eq!(h.state().document().project.panels[1].name, "Mixer");
    assert!(h.state().perform.cells.is_empty(), "nothing on Mixer yet");

    // Click a tab to show it.
    h.get_by_label("Panel 1").click();
    steps(&mut h, 3);
    assert_eq!(h.state().perform.current, 0);
    assert_eq!(h.state().perform.cells.len(), 1);
    // Double-click a tab to rename it (well after the click above, so the
    // two don't make a double-click of their own).
    let tab = h.get_by_label("Panel 1").rect().center();
    h.hover_at(tab);
    steps(&mut h, 12);
    for _ in 0..2 {
        button(&h, tab, true);
        button(&h, tab, false);
    }
    steps(&mut h, 3);
    assert!(h.state().perform.is_typing());
    h.event(Event::Text("Live".into()));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let names: Vec<String> = h
        .state()
        .document()
        .project
        .panels
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["Live", "Mixer"]);

    // Back on the canvas, P pins to the panel shown last.
    h.get_by_label("Mixer").click();
    steps(&mut h, 3);
    press(&mut h, Modifiers::CTRL, Key::Tab);
    select(&mut h, NodeId(2));
    press(&mut h, Modifiers::NONE, Key::P);
    assert_eq!(pinned(&h, 1), vec![NodeId(2)]);
    assert_eq!(pinned(&h, 0), vec![NodeId(1)]);
}

#[test]
fn the_node_menu_pins_and_unpins() {
    let mut h = harness(with_panel(
        nodes(&[("knob", json!({})), ("meter", json!({}))]),
        "Live",
        &[1],
    ));
    let header = |h: &Harness<'static, EditorApp>, id| {
        h.state().canvas.node_screen_rect(id).unwrap().center_top() + egui::vec2(0.0, 10.0)
    };
    let p = header(&h, NodeId(2));
    h.hover_at(p);
    steps(&mut h, 2);
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 3);
    h.get_by_label_contains("Pin to Live").click();
    steps(&mut h, 3);
    assert_eq!(pinned(&h, 0), vec![NodeId(1), NodeId(2)]);
    // And unpin from it.
    let p = header(&h, NodeId(1));
    h.hover_at(p);
    steps(&mut h, 2);
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: p,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 3);
    h.get_by_label_contains("Unpin from Live").click();
    steps(&mut h, 3);
    assert_eq!(pinned(&h, 0), vec![NodeId(2)]);
}

#[test]
fn f11_enters_full_screen_and_esc_leaves() {
    let mut h = harness(with_panel(nodes(&[("slider", json!({}))]), "Live", &[1]));
    assert!(h.query_by_label("File").is_some(), "the menu bar shows");
    key(&h, Key::F11, true);
    h.step();
    assert!(
        viewport_commands(&h).contains(&ViewportCommand::Fullscreen(true)),
        "{:?}",
        viewport_commands(&h)
    );
    key(&h, Key::F11, false);
    steps(&mut h, 3);
    assert!(h.state().performance_mode());
    assert!(h.state().panels_shown());
    // The chrome is gone; the panel, a minimal transport, and a way out
    // stay.
    assert!(h.query_by_label("File").is_none());
    assert!(h.query_by_label_contains("Library").is_none());
    assert!(h.query_by_label_contains("Exit").is_some());
    assert!(h.state().perform.body_rect(NodeId(1)).is_some());

    key(&h, Key::Escape, true);
    h.step();
    assert!(viewport_commands(&h).contains(&ViewportCommand::Fullscreen(false)));
    key(&h, Key::Escape, false);
    steps(&mut h, 3);
    assert!(!h.state().performance_mode());
    assert!(h.query_by_label("File").is_some());
    // Still on the panel it showed.
    assert!(h.state().panels_shown());

    // F11 leaves too, and the Exit button.
    press(&mut h, Modifiers::NONE, Key::F11);
    assert!(h.state().performance_mode());
    press(&mut h, Modifiers::NONE, Key::F11);
    assert!(!h.state().performance_mode());
    press(&mut h, Modifiers::NONE, Key::F11);
    h.get_by_label_contains("Exit").click();
    steps(&mut h, 3);
    assert!(!h.state().performance_mode());
}

#[test]
fn a_meter_on_a_panel_follows_live_audio() {
    let mut p = nodes(&[
        ("oscillator", json!({})),
        ("meter", json!({})),
        ("audio_out", json!({})),
    ]);
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    );
    p.graph.connect(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(3), "in"),
    );
    let mut h = harness(with_panel(p, "Mixer", &[2, 3]));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    assert!(h.state().engine.is_null());
    h.state_mut().engine.handle.play();
    // The null stream runs in real time: wait for the meter to move.
    let t0 = Instant::now();
    let mut peak = 0.0f32;
    while t0.elapsed() < Duration::from_secs(10) {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
        peak = h
            .state()
            .perform
            .meter_reading(NodeId(2))
            .map_or(0.0, |m| m[0].peak.max(m[1].peak));
        if peak > 0.01 {
            break;
        }
    }
    assert!(
        peak > 0.01,
        "the panel's meter shows the oscillator: {peak}"
    );
    assert!(h.state().perform.body_rect(NodeId(2)).is_some());
    assert!(h.state().perform.body_rect(NodeId(3)).is_some());
    h.state_mut().engine.handle.stop();
}

#[test]
fn a_keyboard_on_a_panel_plays_and_slides() {
    let mut h = harness(with_panel(nodes(&[("keyboard", json!({}))]), "Keys", &[1]));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let b = panel_body(&h, NodeId(1));
    // Low on the keys only white keys are hit: C3 is key 0, D3 key 2.
    let white = |i: f32| pos2(b.min.x + b.width() * (i + 0.5) / 15.0, b.max.y - 5.0);
    h.hover_at(white(0.0));
    steps(&mut h, 2);
    h.drag_at(white(0.0));
    steps(&mut h, 2);
    assert_eq!(
        engine_param(&h, NodeId(1), "key_0"),
        Some(Value::Bool(true))
    );
    for i in 1..=4 {
        h.hover_at(white(0.0) + (white(1.0) - white(0.0)) * (i as f32 / 4.0));
        steps(&mut h, 1);
    }
    steps(&mut h, 2);
    assert_eq!(
        engine_param(&h, NodeId(1), "key_0"),
        Some(Value::Bool(false))
    );
    assert_eq!(
        engine_param(&h, NodeId(1), "key_2"),
        Some(Value::Bool(true))
    );
    h.drop_at(white(1.0));
    steps(&mut h, 4);
    assert_eq!(
        engine_param(&h, NodeId(1), "key_2"),
        Some(Value::Bool(false))
    );
    let doc = h.state().document();
    assert!(!doc.can_undo() && !doc.is_dirty(), "playing isn't an edit");
}

#[test]
fn cards_move_resize_and_unpin_on_the_panel() {
    let mut h = harness(with_panel(
        nodes(&[
            ("knob", json!({})),
            ("toggle", json!({})),
            ("readout", json!({})),
        ]),
        "Live",
        &[1, 2, 3],
    ));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let cell = |h: &Harness<'static, EditorApp>, id: u64| {
        h.state()
            .perform
            .cells
            .iter()
            .find(|c| c.node == NodeId(id))
            .cloned()
            .expect("the card is drawn")
    };
    // Drag the knob's card by its header onto the readout's card.
    let from = cell(&h, 1).header.center();
    let to = cell(&h, 3).rect.center();
    drag(&mut h, from, to);
    assert_eq!(pinned(&h, 0), vec![NodeId(2), NodeId(3), NodeId(1)]);
    assert_eq!(h.state().document().undo_label(), Some("Move on panel"));
    // + makes the toggle's card wider; − puts it back.
    let wide = cell(&h, 2).rect.width();
    let [_, grow, _] = cell(&h, 2)
        .buttons
        .expect("buttons outside performance mode");
    click(&mut h, grow.center());
    assert!(cell(&h, 2).rect.width() > wide * 1.5);
    assert_eq!(
        h.state().document().project.panels[0].items[0].size,
        Some([2, 1])
    );
    let [shrink, _, _] = cell(&h, 2).buttons.unwrap();
    click(&mut h, shrink.center());
    assert_eq!(h.state().document().project.panels[0].items[0].size, None);
    // × unpins; the node stays in the patch.
    let [_, _, unpin] = cell(&h, 3).buttons.unwrap();
    click(&mut h, unpin.center());
    assert_eq!(pinned(&h, 0), vec![NodeId(2), NodeId(1)]);
    assert_eq!(h.state().document().project.graph.nodes.len(), 3);
    // Each was one step: undo them all, back to the start.
    for _ in 0..4 {
        press(&mut h, Modifiers::COMMAND, Key::Z);
    }
    assert_eq!(pinned(&h, 0), vec![NodeId(1), NodeId(2), NodeId(3)]);
    assert!(!h.state().document().is_dirty());
}

#[test]
fn a_dropdown_on_a_panel_picks_from_its_labels() {
    let config = json!({"choices": ["sine", "triangle", "saw"]});
    let mut h = harness(with_panel(nodes(&[("dropdown", config)]), "Live", &[1]));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let c = panel_body(&h, NodeId(1)).center();
    click(&mut h, c);
    h.get_by_label("saw").click();
    steps(&mut h, 3);
    assert_eq!(param(&h, NodeId(1), "value"), Some(Value::Int(2)));
    assert_eq!(engine_param(&h, NodeId(1), "value"), Some(Value::Int(2)));
    assert!(h.query_by_label("triangle").is_none(), "the menu closed");
}
