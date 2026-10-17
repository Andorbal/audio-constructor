//! Driving the editor without a window (egui_kittest): add a node from the
//! palette, wire two nodes, undo and redo, group and enter. The engine runs
//! on the null audio stream, the path used when there is no device.

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, GROUP_KIND, NodeId, NodeInstance, Project, Wire};
use egui::{Event, Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;

fn harness(project: Option<Project>) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            ..AppOptions::headless()
        },
    );
    if let Some(p) = project {
        app.open_unsaved(p);
    }
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

/// A constant feeding nothing, and a gain, side by side.
fn two_nodes() -> Project {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "constant").at(0.0, 0.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "gain").at(320.0, 0.0));
    p
}

fn steps(h: &mut Harness<'static, EditorApp>, n: usize) {
    h.run_steps(n);
}

fn press(h: &mut Harness<'static, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
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

#[test]
fn add_a_node_from_the_palette() {
    let mut h = harness(Some(Project::default()));
    let before = h.state().document().graph().nodes.len();
    h.hover_at(Pos2::new(400.0, 400.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::SHIFT, Key::A);
    assert!(h.state().palette.open, "Shift+A opens the palette");
    // Synonyms: "volume" finds Gain.
    h.event(Event::Text("volume".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    assert!(!h.state().palette.open);
    assert_eq!(doc.graph().nodes.len(), before + 1);
    let added = doc.graph().nodes.last().unwrap();
    assert_eq!(added.kind, "gain");
    assert!(doc.selection.contains(&added.id));
    // The engine runs on the null stream (no device here).
    assert!(h.state().engine.is_null());
}

#[test]
fn wire_two_nodes_by_dragging() {
    let mut h = harness(Some(two_nodes()));
    let from = h
        .state()
        .canvas
        .socket_screen_pos(NodeId(1), "out", false)
        .expect("constant out is drawn");
    let to = h
        .state()
        .canvas
        .socket_screen_pos(NodeId(2), "in", true)
        .expect("gain in is drawn");
    drag(&mut h, from, to);
    let wires = &h.state().document().graph().wires;
    assert_eq!(
        wires,
        &vec![Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in")
        )]
    );
    // Dragging the wire's end off the input disconnects it.
    let off = to + vec2(0.0, 160.0);
    drag(&mut h, to, off);
    assert!(h.state().document().graph().wires.is_empty());
}

#[test]
fn undo_and_redo() {
    let mut h = harness(Some(two_nodes()));
    // Select everything and delete it: one undo step.
    h.hover_at(Pos2::new(600.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::A);
    assert_eq!(h.state().document().selection.len(), 2);
    press(&mut h, Modifiers::NONE, Key::X);
    assert!(h.state().document().graph().nodes.is_empty());
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().document().graph().nodes.len(), 2);
    assert!(!h.state().document().is_dirty());
    let redo = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    press(&mut h, redo, Key::Z);
    assert!(h.state().document().graph().nodes.is_empty());
    assert!(h.state().document().is_dirty());
}

#[test]
fn group_and_enter() {
    let mut p = two_nodes();
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    );
    let mut h = harness(Some(p));
    h.hover_at(Pos2::new(600.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::COMMAND, Key::G);
    {
        let doc = h.state().document();
        assert_eq!(doc.project.group_defs.len(), 1);
        assert_eq!(doc.graph().nodes.len(), 1);
        assert_eq!(doc.graph().nodes[0].kind, GROUP_KIND);
        let def = doc.project.group_defs.values().next().unwrap();
        assert_eq!(def.graph.nodes.len(), 2);
        assert_eq!(def.graph.wires.len(), 1);
    }
    // Tab goes inside the selected group; Tab again comes back out.
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().document().scope.len(), 1);
    assert_eq!(h.state().document().graph().nodes.len(), 2);
    // Inside, the group's interface is drawn as cards with sockets.
    let scene = h.state().canvas.last_scene.as_ref().unwrap();
    assert!(scene.node(ac_app::edit::GROUP_IN).is_some());
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert!(h.state().document().scope.is_empty());
    // Ungroup restores the two nodes and their wire.
    press(
        &mut h,
        Modifiers {
            alt: true,
            ..Modifiers::COMMAND
        },
        Key::G,
    );
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), 2);
    assert_eq!(doc.graph().wires.len(), 1);
    assert!(doc.project.group_defs.is_empty());
}

#[test]
fn missing_nodes_and_bad_wires_still_draw() {
    let mut p = two_nodes();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "quantum_flux").at(0.0, 300.0));
    p.graph.connect(
        Endpoint::new(NodeId(3), "warp"),
        Endpoint::new(NodeId(2), "gain"),
    );
    // A wire to a node that doesn't exist.
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(99), "out"),
        Endpoint::new(NodeId(2), "in"),
    ));
    let mut h = harness(Some(p));
    steps(&mut h, 4);
    let scene = h.state().canvas.last_scene.as_ref().unwrap();
    assert!(scene.node(NodeId(3)).is_some());
    assert!(
        h.state()
            .canvas
            .socket_screen_pos(NodeId(3), "warp", false)
            .is_some()
    );
}

#[test]
fn dropping_a_wire_on_empty_canvas_adds_a_wired_node() {
    let mut h = harness(Some(two_nodes()));
    let from = h
        .state()
        .canvas
        .socket_screen_pos(NodeId(1), "out", false)
        .unwrap();
    drag(&mut h, from, from + vec2(120.0, 260.0));
    assert!(
        h.state().palette.open,
        "the palette opens where the wire dropped"
    );
    h.event(Event::Text("meter".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    let new = doc.graph().nodes.last().unwrap();
    assert_eq!(new.kind, "meter");
    assert!(
        doc.graph().wires.contains(&Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(new.id, "in")
        )),
        "the new node is wired to the dragged output"
    );
}

#[test]
fn dropping_a_node_on_a_wire_splices_it_in() {
    let mut p = two_nodes();
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "meter").at(120.0, 260.0));
    let mut h = harness(Some(p));
    let canvas = &h.state().canvas;
    let a = canvas.socket_screen_pos(NodeId(1), "out", false).unwrap();
    let b = canvas.socket_screen_pos(NodeId(2), "in", true).unwrap();
    let header = canvas.node_screen_rect(NodeId(3)).unwrap().center_top() + vec2(0.0, 10.0);
    // Drag the meter by its header so the pointer ends on the wire.
    let target = a + (b - a) * 0.5;
    drag(&mut h, header, target);
    let w = &h.state().document().graph().wires;
    assert!(w.contains(&Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(3), "in")
    )));
    assert!(w.contains(&Wire::new(
        Endpoint::new(NodeId(3), "out"),
        Endpoint::new(NodeId(2), "in")
    )));
    assert_eq!(w.len(), 2);
}
