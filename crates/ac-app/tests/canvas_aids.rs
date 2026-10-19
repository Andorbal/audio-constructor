//! Canvas aids for big patches, driven without a window (egui_kittest):
//! per-node timing in the headers (`Ctrl+T`), portals, and the minimap. The
//! engine runs on the null audio stream, as it does with no device.

use std::time::Duration;

use ac_app::audio::AudioMode;
use ac_app::canvas::scene::Class;
use ac_app::edit::portal;
use ac_app::icons;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Project, Wire};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
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

/// Hover the empty canvas so key presses go to it.
fn hover_canvas(h: &mut Harness<'static, EditorApp>) {
    h.hover_at(Pos2::new(500.0, 700.0));
    steps(h, 2);
}

/// constant → gain → audio out.
fn chain() -> Project {
    let mut p = Project::default();
    for (id, kind, x) in [
        (1, "constant", 0.0),
        (2, "gain", 260.0),
        (3, "audio_out", 520.0),
    ] {
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(id), kind).at(x, 0.0));
    }
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    );
    p.graph.connect(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(3), "in"),
    );
    p
}

/// Let the null stream run some blocks (it keeps real time), then draw.
fn let_audio_run(h: &mut Harness<'static, EditorApp>) {
    std::thread::sleep(Duration::from_millis(60));
    steps(h, 3);
}

fn chip(h: &Harness<'static, EditorApp>, id: NodeId) -> Option<String> {
    h.state()
        .canvas
        .header_timing
        .iter()
        .find(|(n, _)| *n == id)
        .map(|(_, t)| t.clone())
}

// ---------------------------------------------------------------- timing

#[test]
fn ctrl_t_shows_each_nodes_time_in_its_header() {
    let mut h = harness(chain());
    assert!(h.state().engine.is_null());
    let_audio_run(&mut h);
    assert!(h.state().canvas.header_timing.is_empty(), "off by default");
    assert!(h.query_by_label_contains("of budget").is_none());
    hover_canvas(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::T);
    let_audio_run(&mut h);
    for id in [1, 2, 3] {
        let text = chip(&h, NodeId(id)).unwrap_or_else(|| panic!("node {id} shows its time"));
        assert!(text.ends_with("µs") || text.ends_with("ms"), "{text}");
    }
    // The Node tab shows the selected node's time too, with its share of
    // the block's budget.
    h.state_mut().doc.selection = [NodeId(2)].into();
    steps(&mut h, 3);
    assert!(h.query_by_label_contains("of budget").is_some());

    // A group's header shows the sum of its insides.
    press(&mut h, Modifiers::COMMAND, Key::G);
    let group = *h.state().doc.selection.iter().next().unwrap();
    let_audio_run(&mut h);
    assert!(
        chip(&h, group).is_some(),
        "the group shows its insides' time"
    );

    // Ctrl+T again turns it off, in the engine too.
    hover_canvas(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::T);
    assert!(h.state().canvas.header_timing.is_empty());
    let on = h
        .state()
        .engine
        .handle
        .status()
        .timing_enabled
        .load(std::sync::atomic::Ordering::Relaxed);
    assert!(!on, "the engine stops reading the clock");
}

// ---------------------------------------------------------------- portals

fn button(h: &mut Harness<'static, EditorApp>, at: Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

fn click(h: &mut Harness<'static, EditorApp>, at: Pos2) {
    h.hover_at(at);
    steps(h, 2);
    button(h, at, PointerButton::Primary, true);
    steps(h, 1);
    button(h, at, PointerButton::Primary, false);
    steps(h, 2);
}

fn right_click(h: &mut Harness<'static, EditorApp>, at: Pos2) {
    h.hover_at(at);
    steps(h, 2);
    button(h, at, PointerButton::Secondary, true);
    steps(h, 1);
    button(h, at, PointerButton::Secondary, false);
    steps(h, 2);
}

fn double_click(h: &mut Harness<'static, EditorApp>, at: Pos2) {
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

/// The portal tags of the current graph: (senders, receivers).
fn tags(h: &Harness<'static, EditorApp>) -> (Vec<NodeId>, Vec<NodeId>) {
    let g = h.state().document().graph();
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for n in &g.nodes {
        match portal::tag(n) {
            Some(t) if t.send => senders.push(n.id),
            Some(_) => receivers.push(n.id),
            None => {}
        }
    }
    (senders, receivers)
}

fn wire_1_2() -> Wire {
    Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    )
}

#[test]
fn a_wire_becomes_a_portal_and_back_one_undo_step_each() {
    let mut h = harness(chain());
    let canvas = &h.state().canvas;
    let a = canvas.socket_screen_pos(NodeId(1), "out", false).unwrap();
    let b = canvas.socket_screen_pos(NodeId(2), "in", true).unwrap();
    // A wire's curve passes through the middle of its two ends.
    right_click(&mut h, a + (b - a) * 0.5);
    h.get_by_label_contains(&format!("{}  Make Portal", icons::TAG))
        .click();
    steps(&mut h, 3);

    let (senders, receivers) = tags(&h);
    let ([s], [r]) = (senders.as_slice(), receivers.as_slice()) else {
        panic!("one sender and one receiver: {senders:?} {receivers:?}");
    };
    let (s, r) = (*s, *r);
    {
        let doc = h.state().document();
        let g = doc.graph();
        assert!(!g.wires.contains(&wire_1_2()));
        let feed = g.wire_into(&Endpoint::new(NodeId(2), "in")).unwrap();
        assert_eq!(
            ac_app::edit::resolve_source(g, &feed.from),
            Endpoint::new(NodeId(1), "out"),
            "the gain still hears the constant"
        );
        assert_eq!(doc.undo_label(), Some("Make portal"));
        // The tags are drawn as portals, and no line joins them.
        let scene = h.state().canvas.last_scene.as_ref().unwrap();
        assert!(matches!(scene.node(s).unwrap().class, Class::Portal(_)));
        assert!(matches!(scene.node(r).unwrap().class, Class::Portal(_)));
        assert!(
            !scene
                .wires
                .iter()
                .any(|w| w.wire.from.node == s && w.wire.to.node == r),
            "the link isn't drawn"
        );
        assert!(scene.wires.iter().any(|w| w.wire.to.node == s));
        assert!(scene.wires.iter().any(|w| w.wire.from.node == r));
    }

    // One undo step each way.
    hover_canvas(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().document().graph().wires.contains(&wire_1_2()));
    assert_eq!(h.state().document().graph().nodes.len(), 3);
    let redo = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    press(&mut h, redo, Key::Z);
    assert_eq!(tags(&h), (vec![s], vec![r]));

    // Hovering a tag lights its partner; double-clicking jumps to it.
    let s_at = h.state().canvas.node_screen_rect(s).unwrap().center();
    h.hover_at(s_at);
    steps(&mut h, 3);
    assert!(h.state().canvas.portal_lit().contains(&r));
    double_click(&mut h, s_at);
    assert_eq!(h.state().document().selection, [r].into());
    steps(&mut h, 20);
    let r_center = h
        .state()
        .canvas
        .last_scene
        .as_ref()
        .and_then(|sc| sc.node(r))
        .unwrap()
        .rect
        .center();
    assert!((h.state().canvas.view.center - r_center).length() < 5.0);

    // Shift+P on a tag turns the portal back into a wire: one step.
    hover_canvas(&mut h);
    press(&mut h, Modifiers::SHIFT, Key::P);
    {
        let doc = h.state().document();
        assert!(doc.graph().wires.contains(&wire_1_2()));
        assert_eq!(doc.graph().nodes.len(), 3);
        assert_eq!(doc.undo_label(), Some("Back to wire"));
    }
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(tags(&h), (vec![s], vec![r]));
}

#[test]
fn portals_rename_and_paste_inside_groups() {
    let mut h = harness(chain());
    // Select the constant and the gain and make portals of the wires
    // between them, with the keyboard.
    h.state_mut().doc.selection = [NodeId(1), NodeId(2)].into();
    hover_canvas(&mut h);
    press(&mut h, Modifiers::SHIFT, Key::P);
    let (senders, receivers) = tags(&h);
    assert_eq!((senders.len(), receivers.len()), (1, 1));
    // Group everything and go inside: the portal still works there.
    press(&mut h, Modifiers::NONE, Key::A);
    press(&mut h, Modifiers::COMMAND, Key::G);
    press(&mut h, Modifiers::COMMAND, Key::Enter);
    assert_eq!(h.state().document().scope.len(), 1);
    let (senders, receivers) = tags(&h);
    let (s, r) = (senders[0], receivers[0]);
    let scene = h.state().canvas.last_scene.as_ref().unwrap();
    assert!(matches!(scene.node(s).unwrap().class, Class::Portal(_)));
    assert!(
        !scene
            .wires
            .iter()
            .any(|w| w.wire.from.node == s && w.wire.to.node == r)
    );

    // F2 renames the whole portal.
    h.state_mut().doc.selection = [r].into();
    hover_canvas(&mut h);
    press(&mut h, Modifiers::NONE, Key::F2);
    h.event(Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.event(Event::Text("Drone".into()));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let g = h.state().document().graph();
    assert_eq!(portal::tag(g.node(s).unwrap()).unwrap().name, "Drone");
    assert_eq!(portal::tag(g.node(r).unwrap()).unwrap().name, "Drone");

    // Copy the receiver and paste it: a second receiver of the same portal.
    h.state_mut().doc.selection = [r].into();
    hover_canvas(&mut h);
    press(&mut h, Modifiers::COMMAND, Key::C);
    press(&mut h, Modifiers::COMMAND, Key::V);
    let (senders, receivers) = tags(&h);
    assert_eq!(senders, vec![s]);
    assert_eq!(receivers.len(), 2);
    let g = h.state().document().graph();
    assert_eq!(portal::receivers_of(g, s).len(), 2);
}

// ---------------------------------------------------------------- minimap

/// Four nodes far apart, with the view stored on the first, so most of the
/// graph is out of view.
fn spread() -> Project {
    let mut p = Project::default();
    for (id, x, y) in [
        (1, 0.0, 0.0),
        (2, 3000.0, 0.0),
        (3, 3000.0, 2000.0),
        (4, 0.0, 2000.0),
    ] {
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(id), "constant").at(x, y));
    }
    p.view = Some(ac_core::View {
        center: [100.0, 50.0],
        zoom: 1.0,
    });
    p
}

#[test]
fn the_minimap_pans_the_view_and_hides_when_everything_fits() {
    let mut h = harness(spread());
    h.state_mut().doc.selection = [NodeId(1)].into();
    steps(&mut h, 2);
    // The project opens on its stored view, not framed to fit.
    assert_eq!(h.state().canvas.view.zoom, 1.0);
    let map = h
        .state()
        .canvas
        .minimap()
        .expect("the graph doesn't fit, so the minimap shows");
    // Clicking a point of the minimap centers the view there.
    let target = pos2(3050.0, 2040.0);
    click(&mut h, map.to_map(target));
    let center = h.state().canvas.view.center;
    assert!((center - target).length() < 20.0, "{center:?}");
    assert_eq!(
        h.state().document().selection,
        [NodeId(1)].into(),
        "a click on the minimap isn't a click on the canvas"
    );
    // Dragging in it moves the view along.
    let map = h.state().canvas.minimap().unwrap();
    let (a, b) = (
        map.to_map(pos2(100.0, 100.0)),
        map.to_map(pos2(2900.0, 100.0)),
    );
    h.hover_at(a);
    steps(&mut h, 2);
    h.drag_at(a);
    steps(&mut h, 2);
    for i in 1..=8 {
        h.hover_at(a + (b - a) * (i as f32 / 8.0));
        steps(&mut h, 1);
    }
    let center = h.state().canvas.view.center;
    assert!((center - map.to_canvas(b)).length() < 20.0, "{center:?}");
    h.drop_at(b);
    steps(&mut h, 3);
    assert!(h.state().canvas.is_idle());

    // Shift+M turns it off and on.
    hover_canvas(&mut h);
    press(&mut h, Modifiers::SHIFT, Key::M);
    assert!(h.state().canvas.minimap_rect().is_none());
    press(&mut h, Modifiers::SHIFT, Key::M);
    assert!(h.state().canvas.minimap_rect().is_some());

    // Once the whole graph fits the view, it hides itself.
    press(&mut h, Modifiers::NONE, Key::Home);
    steps(&mut h, 30);
    assert!(h.state().canvas.minimap_rect().is_none());
}
