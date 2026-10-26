//! Panels with structure (ROADMAP M4.2, DECISIONS D202–D204): cards keep
//! their places when something new is pinned, strips and labels, compact
//! cards, a pinned block as a channel strip, the new controls (Fader, Pad,
//! Pads, Light, `lit`), multi-select moves, and panels made from a
//! selection.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{
    Block, Endpoint, GroupId, NodeId, NodeInstance, NodePath, Panel, PanelItem, Project, Value,
    Wire,
};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2};
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

fn button(h: &Harness<'static, EditorApp>, pos: Pos2, b: PointerButton, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerButton {
        pos,
        button: b,
        pressed,
        modifiers,
    });
}

fn click_with(h: &mut Harness<'static, EditorApp>, p: Pos2, modifiers: Modifiers) {
    h.hover_at(p);
    steps(h, 2);
    h.event(Event::ModifiersChanged(modifiers));
    button(h, p, PointerButton::Primary, true, modifiers);
    button(h, p, PointerButton::Primary, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    steps(h, 3);
}

fn click(h: &mut Harness<'static, EditorApp>, p: Pos2) {
    click_with(h, p, Modifiers::NONE);
}

fn right_click(h: &mut Harness<'static, EditorApp>, p: Pos2) {
    h.hover_at(p);
    steps(h, 2);
    button(h, p, PointerButton::Secondary, true, Modifiers::NONE);
    button(h, p, PointerButton::Secondary, false, Modifiers::NONE);
    steps(h, 3);
}

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

/// Step in real time, so the null stream runs blocks, until `done` holds.
fn wait_for(h: &mut Harness<'static, EditorApp>, mut done: impl FnMut(&EditorApp) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
        steps(h, 1);
    }
    done(h.state())
}

fn nodes(list: &[(&str, serde_json::Value)]) -> Project {
    let mut p = Project::default();
    for (i, (kind, config)) in list.iter().enumerate() {
        let mut n = NodeInstance::new(NodeId(i as u64 + 1), *kind).at(i as f32 * 300.0, 0.0);
        n.config = config.clone();
        p.graph.nodes.push(n);
    }
    p
}

fn with_panel(mut p: Project, ids: &[u64]) -> Project {
    let mut panel = Panel::new("Live");
    for id in ids {
        panel.items.push(PanelItem::new(NodeId(*id)));
    }
    p.panels.push(panel);
    p
}

/// A Track block instance (id 50, "Drums") with a clip into it.
fn with_track(mut p: Project) -> Project {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../blocks/track.acblock"))
        .expect("the Track block");
    let block = Block::from_json(&text).expect("reads");
    p.group_defs.insert(GroupId("track".into()), block.def.clone());
    let mut inst = NodeInstance::new(NodeId(50), ac_core::GROUP_KIND).at(0.0, 400.0);
    inst.config = json!({"def": "track"});
    inst.ui.label = Some("Drums".into());
    p.graph.nodes.push(inst);
    p
}

fn cell(h: &Harness<'static, EditorApp>, id: u64) -> ac_app::perform::Cell {
    h.state()
        .perform
        .cells
        .iter()
        .find(|c| c.node == NodeId(id))
        .cloned()
        .expect("the card is drawn")
}

fn param(h: &Harness<'static, EditorApp>, id: u64, socket: &str) -> Option<Value> {
    h.state()
        .document()
        .project
        .graph
        .node(NodeId(id))
        .and_then(|n| n.params.get(socket).cloned())
}

#[test]
fn pinning_something_new_moves_nothing_already_there() {
    let mut p = with_panel(
        nodes(&[
            ("knob", json!({})),
            ("toggle", json!({})),
            ("readout", json!({})),
            ("slider", json!({})),
        ]),
        &[1, 2, 3],
    );
    // A placed panel: the knob and toggle have places, out of pin order.
    p.panels[0].items[0].at = Some([3, 0]);
    p.panels[0].items[1].at = Some([0, 2]);
    p.panels[0].items[2].at = Some([1, 0]);
    let mut h = harness(p);
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let before: Vec<Rect> = [1, 2, 3].iter().map(|&id| cell(&h, id).rect).collect();
    // Pin the slider from the canvas.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    h.state_mut().doc.selection = [NodeId(4)].into();
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::P);
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let after: Vec<Rect> = [1, 2, 3].iter().map(|&id| cell(&h, id).rect).collect();
    assert_eq!(before, after, "nothing moved");
    let items = &h.state().document().project.panels[0].items;
    assert_eq!(items[3].node, NodeId(4));
    assert!(items[3].at.is_some(), "the new card got a place of its own");
}

#[test]
fn a_track_block_pins_as_a_channel_strip_and_plays() {
    let p = with_track(Project::default());
    let mut h = harness(p);
    h.state_mut().doc.selection = [NodeId(50)].into();
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::P);
    assert_eq!(h.state().document().project.panels[0].items[0].node, NodeId(50));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let c = cell(&h, 50);
    let (widgets, rects) = c.group.clone().expect("a group card");
    let names: Vec<&str> = widgets.iter().map(|w| w.socket.as_str()).collect();
    assert_eq!(names, ["volume", "pan", "mute", "solo", "any_solo", "soloed"]);
    // Click Mute: one undo step that sets the block's own input.
    click(&mut h, rects[2].center());
    assert_eq!(param(&h, 50, "mute"), Some(Value::Bool(true)));
    assert_eq!(h.state().document().undo_label(), Some("Change value"));
    // Drag the fader down.
    let c = cell(&h, 50);
    let (_, rects) = c.group.clone().unwrap();
    let fader = rects[0];
    drag(&mut h, fader.center(), fader.center() + egui::vec2(0.0, fader.height() / 3.0));
    let v = param(&h, 50, "volume").and_then(|v| v.as_f64()).expect("set");
    assert!(v < -5.0, "the fader moved the volume down ({v})");
    // The engine got it.
    let path = NodePath::root(NodeId(50));
    assert!(h.state_mut().engine.handle.set_param(&path, "volume", Value::Float(v)));
}

#[test]
fn a_strip_from_the_canvas_menu_stacks_compact_cards_under_one_header() {
    let mut h = harness(nodes(&[
        ("fader", json!({})),
        ("toggle", json!({})),
        ("button", json!({})),
        ("knob", json!({})),
    ]));
    {
        let d = &mut h.state_mut().doc;
        d.selection = [NodeId(1), NodeId(2), NodeId(3)].into();
        if let Some(n) = d.project.graph.node_mut(NodeId(1)) {
            n.ui.label = Some("Vocal".into());
        }
    }
    let p = h.state().canvas.node_screen_rect(NodeId(1)).unwrap().center_top() + egui::vec2(0.0, 10.0);
    right_click(&mut h, p);
    h.get_by_label_contains("Pin as Strip").click();
    steps(&mut h, 3);
    let panel = h.state().document().project.panels[0].clone();
    assert_eq!(panel.strips.len(), 1);
    assert_eq!(panel.strips[0].name, "Vocal");
    let compact: Vec<bool> = panel.items.iter().map(|i| i.compact).collect();
    assert_eq!(compact, vec![false, true, true], "the toggle and button are compact");
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let s = h.state().perform.strips[0].clone();
    let (f, t, b) = (cell(&h, 1), cell(&h, 2), cell(&h, 3));
    assert!(s.header.max.y <= f.rect.min.y && f.rect.max.y <= t.rect.min.y);
    assert!(t.rect.max.y <= b.rect.min.y);
    assert!(
        t.rect.height() * 1.8 < f.rect.height() / 2.0 + 30.0,
        "two compact cards to a cell"
    );
    assert!(s.area.contains(b.rect.center()));
    // Dropping the knob onto the strip adds it; × unpins the whole strip.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    h.state_mut().doc.selection = [NodeId(4)].into();
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::P);
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let k = cell(&h, 4);
    let into = cell(&h, 2).rect.center();
    drag(&mut h, k.header.center(), into);
    let panel = &h.state().document().project.panels[0];
    let id = panel.strips[0].id;
    let members: Vec<u64> = panel.strip_items(id).map(|(_, i)| i.node.0).collect();
    assert_eq!(members, vec![1, 4, 2, 3]);
    let s = h.state().perform.strips[0].clone();
    let [_, _, unpin] = s.buttons.expect("strip buttons");
    click(&mut h, unpin.center());
    assert!(h.state().document().project.panels[0].items.is_empty());
}

#[test]
fn several_cards_move_together_and_labels_name_sections() {
    let mut h = harness(with_panel(
        nodes(&[("knob", json!({})), ("knob", json!({})), ("knob", json!({}))]),
        &[1, 2, 3],
    ));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let (a, b) = (cell(&h, 1), cell(&h, 2));
    click(&mut h, a.header.center());
    click_with(&mut h, b.header.center(), Modifiers::CTRL);
    assert_eq!(h.state().perform.selected.len(), 2);
    // Drag both down a cell.
    let g = h.state().perform.geometry;
    let down = a.rect.height() + 12.0;
    drag(&mut h, a.header.center(), a.header.center() + egui::vec2(0.0, down));
    let items = &h.state().document().project.panels[0].items;
    let (ya, yb, yc) = (items[0].at.unwrap()[1], items[1].at.unwrap()[1], items[2].at.unwrap()[1]);
    assert_eq!((ya, yb), (2, 2), "both moved a cell (two lines) down");
    assert_eq!(yc, 0, "the third stayed");
    let _ = g;
    // A label from the empty grid's menu.
    let empty = pos2(700.0, 750.0);
    right_click(&mut h, empty);
    h.get_by_label("Add Label Here").click();
    steps(&mut h, 3);
    // It opens for typing; Enter keeps it.
    h.event(Event::Text("Buses".into()));
    press(&mut h, Modifiers::NONE, Key::Enter);
    let panel = &h.state().document().project.panels[0];
    assert_eq!(panel.labels.len(), 1);
    assert_eq!(panel.labels[0].text, "Buses");
    assert_eq!(h.state().perform.labels[0].text, "Buses");
}

#[test]
fn a_card_takes_a_color_and_goes_compact_from_its_menu() {
    let mut h = harness(with_panel(nodes(&[("toggle", json!({}))]), &[1]));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let c = cell(&h, 1);
    right_click(&mut h, c.header.center());
    h.get_by_label("Compact (half height)").click();
    steps(&mut h, 3);
    assert!(h.state().document().project.panels[0].items[0].compact);
    let tall = c.rect.height();
    assert!(cell(&h, 1).rect.height() < tall * 0.6);
    let c = cell(&h, 1);
    right_click(&mut h, c.rect.center() - egui::vec2(c.rect.width() * 0.3, 0.0));
    h.get_by_label_contains("Color").hover();
    steps(&mut h, 3);
    h.get_by_label("Green").click();
    steps(&mut h, 3);
    assert_eq!(
        h.state().document().project.panels[0].items[0].color.as_deref(),
        Some("green")
    );
}

#[test]
fn a_fader_drags_in_db_and_a_pad_feels_where_it_is_hit() {
    let mut h = harness(with_panel(
        nodes(&[("fader", json!({})), ("pad", json!({})), ("pads", json!({"rows": 2, "columns": 2}))]),
        &[1, 2, 3],
    ));
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let f = cell(&h, 1).body.unwrap().1;
    // Grabbed near the bottom: the fader jumps low, in dB.
    drag(&mut h, pos2(f.center().x, f.max.y - 4.0), pos2(f.center().x, f.max.y - 24.0));
    let v = param(&h, 1, "value").and_then(|v| v.as_f64()).unwrap();
    assert!(v < -40.0, "{v}");
    // A pad hit near its top is loud, near its bottom soft.
    let pad = cell(&h, 2).body.unwrap().1;
    let path = NodePath::root(NodeId(2));
    let hit = |h: &mut Harness<'static, EditorApp>, y: f32| {
        let p = pos2(pad.center().x, y);
        h.hover_at(p);
        steps(h, 1);
        button(h, p, PointerButton::Primary, true, Modifiers::NONE);
        steps(h, 1);
        let held = h.state().perform.controls.held_inputs(NodeId(2));
        // Held for a few blocks of the null stream's real time.
        std::thread::sleep(Duration::from_millis(40));
        button(h, p, PointerButton::Primary, false, Modifiers::NONE);
        // Past the minimum hold, so the release reaches the engine.
        steps(h, 6);
        std::thread::sleep(Duration::from_millis(40));
        steps(h, 2);
        held
    };
    assert_eq!(hit(&mut h, pad.min.y + 3.0), vec!["pressed".to_string()]);
    assert!(
        wait_for(&mut h, |a| a
            .engine
            .handle
            .telemetry()
            .node_value(&path, 1)
            .is_some_and(|v| v > 0.9)),
        "a hard hit"
    );
    hit(&mut h, pad.max.y - 3.0);
    assert!(wait_for(&mut h, |a| a
        .engine
        .handle
        .telemetry()
        .node_value(&path, 1)
        .is_some_and(|v| v < 0.4)));
    // On a Pads grid the top-left pad of a 2 × 2 is pad 2.
    let grid = cell(&h, 3).body.unwrap().1;
    let p = grid.min + egui::vec2(8.0, 8.0);
    h.hover_at(p);
    steps(&mut h, 1);
    button(&h, p, PointerButton::Primary, true, Modifiers::NONE);
    steps(&mut h, 1);
    assert_eq!(
        h.state().perform.controls.held_inputs(NodeId(3)),
        vec!["pad_2".to_string()]
    );
    button(&h, p, PointerButton::Primary, false, Modifiers::NONE);
    steps(&mut h, 2);
}

#[test]
fn lit_buttons_and_lights_show_a_clip_playing() {
    let mut p = with_panel(
        nodes(&[("clip", json!({})), ("button", json!({})), ("light", json!({"color": "green"}))]),
        &[2, 3],
    );
    // An empty clip still "plays" until its end; use an oscillator's
    // playing instead, which runs until stopped.
    p.graph.nodes[0] = NodeInstance::new(NodeId(1), "oscillator").at(0.0, 0.0);
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "playing"),
        Endpoint::new(NodeId(2), "lit"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "playing"),
        Endpoint::new(NodeId(3), "in"),
    ));
    let mut h = harness(p);
    press(&mut h, Modifiers::CTRL, Key::Tab);
    h.state_mut().engine.handle.play();
    let lit = |a: &EditorApp, id: u64| {
        a.engine
            .handle
            .telemetry()
            .node_value(&NodePath::root(NodeId(id)), 0)
            .is_some_and(|v| v > 0.5)
    };
    assert!(wait_for(&mut h, |a| lit(a, 2) && lit(a, 3)));
    h.state_mut().engine.handle.stop();
    assert!(wait_for(&mut h, |a| !lit(a, 2) && !lit(a, 3)));
}

#[test]
fn pads_from_samplers_make_a_panel_in_one_step() {
    let mut h = harness(nodes(&[("sampler", json!({})), ("sampler", json!({}))]));
    h.state_mut().doc.selection = [NodeId(1), NodeId(2)].into();
    steps(&mut h, 2);
    let p = h.state().canvas.node_screen_rect(NodeId(1)).unwrap().center_top() + egui::vec2(0.0, 10.0);
    right_click(&mut h, p);
    h.get_by_label_contains("Pads from Selection").click();
    steps(&mut h, 3);
    let d = h.state().document();
    assert_eq!(d.project.panels.len(), 1);
    assert_eq!(d.project.panels[0].name, "Pads");
    let pads = d.project.panels[0].items[0].node;
    assert_eq!(d.project.graph.node(pads).unwrap().kind, "pads");
    assert!(h.state().panels_shown(), "the new panel shows");
    assert_eq!(d.undo_label(), Some("Pads from selection"));
}
