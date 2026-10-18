//! Driving the editor without a window (egui_kittest): add a node from the
//! palette, wire two nodes, undo and redo, group and enter. The engine runs
//! on the null audio stream, the path used when there is no device.

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, GROUP_KIND, NodeId, NodeInstance, Project, Value, Wire};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

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
fn duplicate_and_place_is_one_undo_step() {
    let mut h = harness(Some(two_nodes()));
    h.hover_at(Pos2::new(600.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::SHIFT, Key::D);
    // The copies follow the pointer until a click places them.
    let place = Pos2::new(700.0, 700.0);
    h.hover_at(place);
    steps(&mut h, 2);
    h.drag_at(place);
    steps(&mut h, 2);
    h.drop_at(place);
    steps(&mut h, 3);
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), 4);
    let copy = doc.graph().nodes.last().unwrap().ui.pos;
    assert!(
        copy[1] > 60.0,
        "the copies moved with the pointer: {copy:?}"
    );
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().document().graph().nodes.len(), 2);
    assert!(!h.state().document().is_dirty());
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

// ---------------------------------------------------------------- controls

/// One control node, alone on the canvas.
fn control(kind: &str, config: serde_json::Value) -> Project {
    let mut p = Project::default();
    let mut n = NodeInstance::new(NodeId(1), kind).at(0.0, 0.0);
    n.config = config;
    p.graph.nodes.push(n);
    p
}

/// The control's widget body on screen.
fn body(h: &Harness<'static, EditorApp>) -> Rect {
    h.state()
        .canvas
        .body_screen_rect(NodeId(1))
        .expect("the control's body is drawn")
}

/// An input's value in the document.
fn param(h: &Harness<'static, EditorApp>, socket: &str) -> Option<Value> {
    h.state()
        .document()
        .graph()
        .node(NodeId(1))
        .and_then(|n| n.params.get(socket).cloned())
}

fn float(v: Option<Value>) -> f64 {
    v.and_then(|v| v.as_f64()).expect("a number")
}

/// What the engine was last told for an input (held controls only go to
/// the engine, not the document).
fn engine_param(h: &Harness<'static, EditorApp>, socket: &str) -> Option<Value> {
    h.state()
        .engine
        .handle
        .project()
        .graph
        .node(NodeId(1))
        .and_then(|n| n.params.get(socket).cloned())
}

fn button(h: &Harness<'static, EditorApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

/// A plain click (the pointer stays where it was).
fn click(h: &mut Harness<'static, EditorApp>, p: Pos2) {
    h.hover_at(p);
    steps(h, 2);
    button(h, p, true);
    button(h, p, false);
    steps(h, 3);
}

#[test]
fn a_knob_turns_with_an_upward_drag() {
    let mut h = harness(Some(control("knob", json!({"min": 0, "max": 10}))));
    let c = body(&h).center();
    // Half the travel up turns it halfway.
    drag(&mut h, c, c - vec2(0.0, 100.0));
    let v = float(param(&h, "value"));
    assert!((v - 5.0).abs() < 1e-6, "{v}");
    // Down again, past the bottom: it stops at the bottom of the range.
    drag(&mut h, c, c + vec2(0.0, 300.0));
    assert_eq!(float(param(&h, "value")), 0.0);
    // Each drag is one undo step.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!((float(param(&h, "value")) - 5.0).abs() < 1e-6);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(param(&h, "value"), None);
    // Double-click puts it back to its default.
    drag(&mut h, c, c - vec2(0.0, 50.0));
    assert!(float(param(&h, "value")) > 2.0);
    h.hover_at(c);
    steps(&mut h, 2);
    for _ in 0..2 {
        button(&h, c, true);
        button(&h, c, false);
    }
    steps(&mut h, 3);
    assert_eq!(float(param(&h, "value")), 0.0);
}

#[test]
fn an_xy_pad_follows_the_pointer_while_touched() {
    let mut h = harness(Some(control("xy_pad", json!({"x_min": 0, "x_max": 100}))));
    let b = body(&h);
    let at = |fx: f32, fy: f32| pos2(b.min.x + b.width() * fx, b.max.y - b.height() * fy);
    h.hover_at(at(0.25, 0.75));
    steps(&mut h, 2);
    h.drag_at(at(0.25, 0.75));
    steps(&mut h, 2);
    // Pressing touches the pad and puts the point under the pointer.
    assert!((float(param(&h, "x")) - 25.0).abs() < 0.01);
    assert!((float(param(&h, "y")) - 0.75).abs() < 1e-3);
    assert_eq!(h.state().canvas.held_inputs(NodeId(1)), vec!["touched"]);
    assert_eq!(engine_param(&h, "touched"), Some(Value::Bool(true)));
    // Dragging moves it.
    for i in 1..=6 {
        let k = i as f32 / 6.0;
        h.hover_at(at(0.25 + 0.5 * k, 0.75 - 0.25 * k));
        steps(&mut h, 1);
    }
    assert!((float(param(&h, "x")) - 75.0).abs() < 0.01);
    assert!((float(param(&h, "y")) - 0.5).abs() < 1e-3);
    // Letting go clears `touched`; the point stays.
    h.drop_at(at(0.75, 0.5));
    steps(&mut h, 4);
    assert!(h.state().canvas.held_inputs(NodeId(1)).is_empty());
    assert_eq!(engine_param(&h, "touched"), Some(Value::Bool(false)));
    assert!((float(param(&h, "x")) - 75.0).abs() < 0.01);
    assert_eq!(
        param(&h, "touched"),
        None,
        "touching isn't a project change"
    );
    // The whole gesture is one undo step.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(param(&h, "x"), None);
    assert_eq!(param(&h, "y"), None);
    assert!(!h.state().document().is_dirty());
}

#[test]
fn keyboard_keys_play_while_held_and_slide() {
    let mut h = harness(Some(control("keyboard", json!({}))));
    let b = body(&h);
    // Low on the keys only white keys are hit: C3 is key 0, D3 key 2.
    let white = |i: f32| pos2(b.min.x + b.width() * (i + 0.5) / 15.0, b.max.y - 5.0);
    h.hover_at(white(0.0));
    steps(&mut h, 2);
    h.drag_at(white(0.0));
    steps(&mut h, 2);
    assert_eq!(engine_param(&h, "key_0"), Some(Value::Bool(true)));
    assert_eq!(h.state().canvas.held_inputs(NodeId(1)), vec!["key_0"]);
    // Sliding onto D lets go of C and plays D.
    for i in 1..=4 {
        h.hover_at(white(0.0) + (white(1.0) - white(0.0)) * (i as f32 / 4.0));
        steps(&mut h, 1);
    }
    steps(&mut h, 2);
    assert_eq!(engine_param(&h, "key_0"), Some(Value::Bool(false)));
    assert_eq!(engine_param(&h, "key_2"), Some(Value::Bool(true)));
    h.drop_at(white(1.0));
    steps(&mut h, 4);
    assert_eq!(engine_param(&h, "key_2"), Some(Value::Bool(false)));
    assert!(h.state().canvas.held_inputs(NodeId(1)).is_empty());
    // Playing isn't an edit: nothing to undo or save.
    let doc = h.state().document();
    assert!(!doc.can_undo() && !doc.is_dirty());
    assert_eq!(param(&h, "key_0"), None);
}

#[test]
fn a_quick_tap_on_a_key_still_reaches_the_engine() {
    let mut h = harness(Some(control("keyboard", json!({}))));
    let b = body(&h);
    // Near the top, between C and D: the black key C#.
    let p = pos2(b.min.x + b.width() / 15.0, b.min.y + 6.0);
    h.hover_at(p);
    steps(&mut h, 2);
    button(&h, p, true);
    h.step();
    assert_eq!(engine_param(&h, "key_1"), Some(Value::Bool(true)));
    // Released a frame later, sooner than the minimum hold: it waits.
    button(&h, p, false);
    h.step();
    assert_eq!(engine_param(&h, "key_1"), Some(Value::Bool(true)));
    assert_eq!(h.state().canvas.held_inputs(NodeId(1)), vec!["key_1"]);
    steps(&mut h, 3);
    assert_eq!(engine_param(&h, "key_1"), Some(Value::Bool(false)));
    assert!(h.state().canvas.held_inputs(NodeId(1)).is_empty());
}

#[test]
fn a_dropdown_picks_from_its_labels() {
    let config = json!({"choices": ["sine", "triangle", "saw", "square"]});
    let mut h = harness(Some(control("dropdown", config)));
    let c = body(&h).center();
    click(&mut h, c);
    h.get_by_label("saw").click();
    steps(&mut h, 3);
    assert_eq!(param(&h, "value"), Some(Value::Int(2)));
    // The menu closed; picking again works the same way.
    assert!(h.query_by_label("triangle").is_none());
    click(&mut h, c);
    h.get_by_label("square").click();
    steps(&mut h, 3);
    assert_eq!(param(&h, "value"), Some(Value::Int(3)));
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(param(&h, "value"), Some(Value::Int(2)));
}

#[test]
fn a_number_body_drags_and_takes_typing() {
    let config = json!({"type": "int", "min": 0, "max": 100});
    let mut h = harness(Some(control("number", config)));
    let c = body(&h).center();
    // Sideways, like an inline field: half the travel is half the range.
    drag(&mut h, c, c + vec2(160.0, 0.0));
    assert_eq!(param(&h, "value"), Some(Value::Int(50)));
    // Click to type a number.
    click(&mut h, c);
    assert!(h.state().canvas.is_typing());
    h.event(Event::Text("42".into()));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!h.state().canvas.is_typing());
    assert_eq!(param(&h, "value"), Some(Value::Int(42)));
}

#[test]
fn play_keys_send_keys_to_the_patch_instead_of_running_shortcuts() {
    let mut h = harness(Some(two_nodes()));
    h.hover_at(Pos2::new(600.0, 600.0));
    steps(&mut h, 2);
    let sent = |h: &Harness<'static, EditorApp>| h.state().engine.keys_sent();
    assert!(!h.state().play_keys(), "off by default");

    // The transport bar's keyboard button turns it on.
    h.get_by_label(ac_app::icons::KEYBOARD).click();
    steps(&mut h, 3);
    assert!(h.state().play_keys());
    // A goes to the patch (down and up); Select All doesn't run.
    press(&mut h, Modifiers::NONE, Key::A);
    assert!(h.state().document().selection.is_empty());
    assert_eq!(sent(&h), 2);
    // Space doesn't start the transport.
    press(&mut h, Modifiers::NONE, Key::Space);
    std::thread::sleep(std::time::Duration::from_millis(50));
    steps(&mut h, 2);
    assert!(!h.state().engine.handle.is_playing());
    assert_eq!(sent(&h), 4);
    // Escape keeps its job and isn't played; Ctrl shortcuts still work.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(sent(&h), 4);
    press(&mut h, Modifiers::COMMAND, Key::A);
    assert_eq!(sent(&h), 4, "a modified key is a shortcut");
    // A key held when Play keys turns off comes up with it.
    h.key_down(Key::Q);
    steps(&mut h, 2);
    assert_eq!(sent(&h), 5);
    press(&mut h, Modifiers::COMMAND, Key::K);
    assert!(!h.state().play_keys(), "Ctrl+K turns it off");
    assert_eq!(sent(&h), 6, "Q was released");
    h.key_up(Key::Q);
    steps(&mut h, 2);
    assert_eq!(sent(&h), 6);

    // Off: the shortcut works again and nothing is played.
    press(&mut h, Modifiers::NONE, Key::A);
    assert_eq!(h.state().document().selection.len(), 2);
    assert_eq!(sent(&h), 6);
    // The app can turn it on from code (a performance view does).
    h.state_mut().set_play_keys(true);
    press(&mut h, Modifiers::NONE, Key::X);
    assert_eq!(
        h.state().document().graph().nodes.len(),
        2,
        "X didn't delete"
    );
    assert_eq!(sent(&h), 8);
}

#[test]
fn the_devices_tab_turns_osc_listening_on_and_off() {
    let mut h = harness(Some(two_nodes()));
    let free = std::net::UdpSocket::bind("0.0.0.0:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    {
        let app = h.state_mut();
        app.engine.osc_port = free;
        app.side.open = true;
        app.side.tab = ac_app::panels::Tab::Devices;
    }
    steps(&mut h, 3);
    assert!(h.state().engine.osc_in.is_none(), "off until asked");
    let _ = h.get_by_label("GAMEPADS");
    let _ = h.get_by_label("COMPUTER KEYS");
    h.get_by_label("Listen on UDP port").click();
    steps(&mut h, 3);
    assert_eq!(h.state().engine.osc_in.as_ref().map(|l| l.port()), Some(free));
    let _ = h.get_by_label_contains(&format!(":{free}"));
    // Something arrives: the tab counts it.
    let tx = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    tx.send_to(b"/go\0,\0\0\0", ("127.0.0.1", free)).unwrap();
    let t0 = std::time::Instant::now();
    while h.state().engine.osc_in.as_ref().unwrap().stats().messages < 1
        && t0.elapsed() < std::time::Duration::from_secs(5)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    steps(&mut h, 2);
    let _ = h.get_by_label_contains("1 messages");
    h.get_by_label("Listen on UDP port").click();
    steps(&mut h, 3);
    assert!(h.state().engine.osc_in.is_none());
}

#[test]
fn play_keys_leave_text_fields_their_typing() {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "constant").at(0.0, 0.0));
    let mut h = harness(Some(p));
    h.hover_at(Pos2::new(600.0, 600.0));
    steps(&mut h, 2);
    // Start renaming the node, then turn Play keys on: the field keeps
    // getting the typing, and nothing reaches the patch.
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::NONE, Key::F2);
    assert!(h.state().canvas.is_typing(), "renaming");
    h.state_mut().set_play_keys(true);
    press(&mut h, Modifiers::NONE, Key::B);
    h.event(Event::Text("b".into()));
    steps(&mut h, 2);
    assert_eq!(h.state().engine.keys_sent(), 0, "typing isn't played");
    assert!(h.state().canvas.is_typing());
}
