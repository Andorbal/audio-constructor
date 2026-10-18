//! Editor conveniences, driven without a window (egui_kittest): auto-layout,
//! align, lazy connect, connect selected to a new node, and find.

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, GROUP_KIND, GroupDef, GroupId, NodeId, NodeInstance, Project, Wire};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

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

fn steps(h: &mut Harness<'static, EditorApp>, n: usize) {
    h.run_steps(n);
}

fn press(h: &mut Harness<'static, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    steps(h, 3);
}

/// Point at an empty spot of the canvas, so shortcuts go to it.
fn hover_canvas(h: &mut Harness<'static, EditorApp>) {
    h.hover_at(Pos2::new(1200.0, 820.0));
    steps(h, 2);
}

/// Click a node's header, with Shift held when `shift`.
fn click_header(h: &mut Harness<'static, EditorApp>, id: NodeId, shift: bool) {
    let p = h
        .state()
        .canvas
        .node_screen_rect(id)
        .expect("the node is drawn")
        .center_top()
        + vec2(0.0, 12.0);
    h.hover_at(p);
    steps(h, 2);
    let modifiers = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::NONE
    };
    h.event(Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    steps(h, 3);
}

fn pos(h: &Harness<'static, EditorApp>, id: u64) -> [f32; 2] {
    h.state()
        .document()
        .graph()
        .node(NodeId(id))
        .expect("the node exists")
        .ui
        .pos
}

fn node(id: u64, kind: &str, x: f32, y: f32) -> NodeInstance {
    NodeInstance::new(NodeId(id), kind).at(x, y)
}

fn wire(a: u64, out: &str, b: u64, inp: &str) -> Wire {
    Wire::new(Endpoint::new(NodeId(a), out), Endpoint::new(NodeId(b), inp))
}

#[test]
fn ctrl_l_lays_out_a_scrambled_chain_left_to_right_in_one_undo_step() {
    // oscillator → gain → audio out, drawn backwards and askew.
    let mut p = Project::default();
    p.graph.nodes.push(node(3, "audio_out", 0.0, 0.0));
    p.graph.nodes.push(node(2, "gain", 600.0, 300.0));
    p.graph.nodes.push(node(1, "oscillator", 300.0, -200.0));
    p.graph.wires.push(wire(1, "out", 2, "in"));
    p.graph.wires.push(wire(2, "out", 3, "in"));
    let mut h = harness(p);
    let before: Vec<[f32; 2]> = (1..=3).map(|i| pos(&h, i)).collect();
    let engine = h.state().document().engine_revision;
    hover_canvas(&mut h);
    // Nothing selected: the whole graph.
    press(&mut h, Modifiers::COMMAND, Key::L);
    let (a, b, c) = (pos(&h, 1), pos(&h, 2), pos(&h, 3));
    assert!(a[0] < b[0] && b[0] < c[0], "{a:?} {b:?} {c:?}");
    // No card overlaps the next.
    let rect = |id| h.state().canvas.node_screen_rect(NodeId(id)).unwrap();
    assert!(rect(1).max.x < rect(2).min.x && rect(2).max.x < rect(3).min.x);
    // The layout starts where the old top-left was.
    assert_eq!(a[0].min(b[0]).min(c[0]), 0.0);
    assert_eq!(a[1].min(b[1]).min(c[1]), -200.0);
    let doc = h.state().document();
    assert_eq!(doc.undo_label(), Some("Auto-layout"));
    assert_eq!(
        doc.engine_revision, engine,
        "moving cards isn't a recompile"
    );
    assert!(h.state().status().is_some_and(|s| s.contains("Laid out 3")));
    // One undo puts everything back.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let after: Vec<[f32; 2]> = (1..=3).map(|i| pos(&h, i)).collect();
    assert_eq!(after, before);
    assert!(!h.state().document().can_undo());
    assert!(!h.state().document().is_dirty());
}

#[test]
fn align_left_from_the_arrange_menu() {
    let mut p = Project::default();
    p.graph.nodes.push(node(1, "gain", 40.0, 0.0));
    p.graph.nodes.push(node(2, "gain", 300.0, 200.0));
    p.graph.nodes.push(node(3, "gain", 600.0, 420.0));
    let mut h = harness(p);
    hover_canvas(&mut h);
    press(&mut h, Modifiers::NONE, Key::A);
    // Alt+A is still Select None; Shift+Alt+A opens the arrange menu.
    let shift_alt = Modifiers {
        alt: true,
        shift: true,
        ..Modifiers::NONE
    };
    press(&mut h, shift_alt, Key::A);
    assert_eq!(h.state().document().selection.len(), 3);
    assert!(h.state().canvas.menu_open());
    h.get_by_label_contains("Align Left").click();
    steps(&mut h, 3);
    assert!(!h.state().canvas.menu_open());
    for id in 1..=3 {
        assert_eq!(pos(&h, id)[0], 40.0, "node {id}");
    }
    assert_eq!(pos(&h, 2)[1], 200.0, "only x moves");
    assert_eq!(h.state().document().undo_label(), Some("Align left"));
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(pos(&h, 3)[0], 600.0);
    // Alt+A still clears the selection.
    press(&mut h, Modifiers::ALT, Key::A);
    assert!(h.state().document().selection.is_empty());
}

#[test]
fn f_wires_the_two_selected_nodes() {
    let mut p = Project::default();
    p.graph.nodes.push(node(1, "gain", 400.0, 0.0));
    p.graph.nodes.push(node(2, "oscillator", 0.0, 60.0));
    let mut h = harness(p);
    hover_canvas(&mut h);
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::NONE, Key::F);
    // The oscillator is on the left: its output feeds the gain's input.
    assert_eq!(
        h.state().document().graph().wires,
        vec![wire(2, "out", 1, "in")]
    );
    let status = h.state().status().unwrap_or_default().to_string();
    assert!(status.contains("Oscillator out → Gain in"), "{status}");
    assert_eq!(h.state().document().undo_label(), Some("Connect"));
    // Again: the next free input that takes it.
    press(&mut h, Modifiers::NONE, Key::F);
    assert!(
        h.state()
            .document()
            .graph()
            .wires
            .contains(&wire(2, "out", 1, "gain"))
    );
    // And again: the gain is full, and wiring it back into the oscillator
    // would make a loop with no Feedback node.
    press(&mut h, Modifiers::NONE, Key::F);
    assert_eq!(h.state().document().graph().wires.len(), 2);
    assert!(
        h.state()
            .status()
            .is_some_and(|s| s.starts_with("Nothing to connect"))
    );
    // Each wire was its own undo step.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().document().graph().wires.len(), 1);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().document().graph().wires.is_empty());
}

#[test]
fn adding_a_mix_wires_the_selected_oscillators_in_selection_order() {
    let mut p = Project::default();
    p.graph.nodes.push(node(1, "oscillator", 0.0, 0.0));
    p.graph.nodes.push(node(2, "oscillator", 0.0, 300.0));
    let mut h = harness(p);
    // The lower one first, then the upper one.
    click_header(&mut h, NodeId(2), false);
    click_header(&mut h, NodeId(1), true);
    assert_eq!(h.state().document().selection.len(), 2);
    hover_canvas(&mut h);
    press(&mut h, Modifiers::SHIFT, Key::A);
    assert!(h.state().palette.open);
    assert!(h.state().palette.note.is_some(), "the palette says so");
    h.event(Event::Text("mix".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    let mix = doc.graph().nodes.last().unwrap();
    assert_eq!(mix.kind, "mix");
    let wires = &doc.graph().wires;
    assert!(wires.contains(&Wire::new(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(mix.id, "in_0")
    )));
    assert!(wires.contains(&Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(mix.id, "in_1")
    )));
    assert_eq!(wires.len(), 2);
    // Placed to the right of the oscillators.
    let osc_right = h.state().canvas.node_screen_rect(NodeId(1)).unwrap().max.x;
    let mix_left = h.state().canvas.node_screen_rect(mix.id).unwrap().min.x;
    assert!(mix_left > osc_right);
    assert!(doc.selection.contains(&mix.id));
    // One undo takes the Mix and its wires away.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), 2);
    assert!(doc.graph().wires.is_empty());
}

#[test]
fn adding_a_node_that_cant_take_them_all_leaves_it_unwired() {
    let mut p = Project::default();
    p.graph.nodes.push(node(1, "oscillator", 0.0, 0.0));
    p.graph.nodes.push(node(2, "oscillator", 0.0, 300.0));
    let mut h = harness(p);
    hover_canvas(&mut h);
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::SHIFT, Key::A);
    // A Gain has one audio input for two oscillators.
    h.event(Event::Text("gain".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.last().unwrap().kind, "gain");
    assert!(doc.graph().wires.is_empty());
    assert!(
        h.state()
            .status()
            .is_some_and(|s| s.contains("without wiring")),
        "{:?}",
        h.state().status()
    );
}

#[test]
fn ctrl_f_finds_a_node_inside_a_group_and_jumps_into_it() {
    let mut p = Project::default();
    let mut def = GroupDef::new("Voice");
    let mut lfo = NodeInstance::new(NodeId(10), "lfo").at(0.0, 0.0);
    lfo.ui.label = Some("Wobble".into());
    def.graph.nodes.push(lfo);
    def.graph.nodes.push(node(11, "gain", 300.0, 0.0));
    p.group_defs.insert(GroupId("voice".into()), def);
    p.graph.nodes.push(node(1, "oscillator", 0.0, 0.0));
    p.graph.nodes.push(
        NodeInstance::new(NodeId(2), GROUP_KIND)
            .with_config(serde_json::json!({"def": "voice"}))
            .at(400.0, 0.0),
    );
    let mut h = harness(p);
    hover_canvas(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::F);
    assert!(h.state().find.open);
    h.event(Event::Text("wobb".into()));
    steps(&mut h, 3);
    // The result shows where it is.
    let _ = h.get_by_label_contains("in Voice");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    assert_eq!(doc.scope.len(), 1, "inside the group");
    assert_eq!(doc.scope[0].instance, NodeId(2));
    assert_eq!(doc.selection, [NodeId(10)].into());
    // Typing still goes to the bar: shortcuts don't run while it's open.
    assert!(h.state().find.open);
    press(&mut h, Modifiers::NONE, Key::X);
    assert_eq!(h.state().document().graph().nodes.len(), 2);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().find.open);
    // Up and down choose among several.
    press(&mut h, Modifiers::COMMAND, Key::F);
    h.event(Event::Text("o".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(h.state().find.selected, 1);
    press(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(h.state().find.selected, 0);
}
