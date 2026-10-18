//! Bindings in the editor (egui_kittest): learning a binding from MIDI the
//! engine receives on the null stream, listing and undoing it, bindings
//! leaving with their nodes, and the gamepad transport preset.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp};
use ac_core::{
    Binding, MidiMsg, NodeId, NodeInstance, NodePath, Project, Source, Target, TransportAction,
};
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

/// A constant and a gain, side by side, unwired.
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

fn right_click(h: &mut Harness<'static, EditorApp>, at: Pos2) {
    h.hover_at(at);
    steps(h, 2);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        steps(h, 1);
    }
    steps(h, 2);
}

fn cc(controller: u8, value: u8) -> MidiMsg {
    MidiMsg::ControlChange {
        channel: 0,
        controller,
        value,
    }
}

/// Step the editor (in real time, so the null stream runs blocks) until
/// `done` holds or a few seconds pass.
fn wait_for(h: &mut Harness<'static, EditorApp>, done: impl FnMut(&EditorApp) -> bool) -> bool {
    wait_up_to(h, Duration::from_secs(5), done)
}

fn wait_up_to(
    h: &mut Harness<'static, EditorApp>,
    limit: Duration,
    mut done: impl FnMut(&EditorApp) -> bool,
) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
        steps(h, 1);
    }
    done(h.state())
}

#[test]
fn bind_an_input_by_moving_a_midi_controller() {
    let mut h = harness(two_nodes());
    let socket = h
        .state()
        .canvas
        .socket_screen_pos(NodeId(2), "gain", true)
        .expect("gain's gain input is drawn");
    right_click(&mut h, socket);
    h.get_by_label_contains("Bind…").click();
    steps(&mut h, 3);
    assert!(h.state().learn.is_some(), "learning");
    assert!(
        h.query_by_label_contains("move a control").is_some(),
        "the status line says what to do"
    );
    let revision = h.state().document().revision;

    let mut midi = h
        .state_mut()
        .engine
        .virtual_midi("test controller")
        .expect("the null stream takes virtual MIDI");
    // Where the knob sits isn't a move; jitter isn't either.
    midi.send(cc(7, 60));
    midi.send(cc(7, 62));
    assert!(!wait_up_to(&mut h, Duration::from_millis(400), |a| {
        !a.document().project.bindings.is_empty()
    }));
    assert!(h.state().learn.is_some());
    // A real turn binds it, as one undo step.
    midi.send(cc(7, 100));
    assert!(wait_for(&mut h, |a| !a.document().project.bindings.is_empty()));
    let doc = h.state().document();
    assert!(h.state().learn.is_none());
    assert_eq!(
        doc.project.bindings,
        vec![Binding::input(
            Source::MidiCc {
                channel: Some(1),
                cc: 7
            },
            NodePath::root(NodeId(2)),
            "gain"
        )]
    );
    assert_eq!(doc.undo_label(), Some("Bind"));
    assert!(doc.revision > revision);

    // The Project tab lists it.
    h.state_mut().side.tab = Tab::Project;
    steps(&mut h, 3);
    assert!(h.query_by_label_contains("MIDI CC 7 · ch 1").is_some());
    assert!(h.query_by_label_contains("→ Gain · Gain").is_some());

    // The controller now moves the gain in the engine: the widget follows
    // without touching the document.
    let rev = h.state().document().revision;
    midi.send(cc(7, 127));
    assert!(wait_for(&mut h, |a| a
        .engine
        .handle
        .binding_values()
        .get(0)
        .is_some_and(|v| (v - 12.0).abs() < 1e-4)));
    let doc = h.state().document();
    assert_eq!(doc.revision, rev, "a bound control isn't an edit");
    assert!(
        doc.graph()
            .node(NodeId(2))
            .is_some_and(|n| n.params.get("gain").is_none())
    );

    // Undo takes the binding away.
    h.hover_at(Pos2::new(700.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().document().project.bindings.is_empty());
}

#[test]
fn escape_cancels_learning_and_keys_can_be_learned() {
    let mut h = harness(two_nodes());
    let socket = h
        .state()
        .canvas
        .socket_screen_pos(NodeId(2), "gain", true)
        .unwrap();
    right_click(&mut h, socket);
    h.get_by_label_contains("Bind…").click();
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().learn.is_none());
    assert!(h.state().document().project.bindings.is_empty());

    // A key pressed while learning binds (and runs no shortcut: X would
    // delete the selection).
    h.state_mut().doc.selection = [NodeId(2)].into();
    right_click(&mut h, socket);
    h.get_by_label_contains("Bind…").click();
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::X);
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), 2, "nothing deleted");
    assert_eq!(doc.project.bindings.len(), 1);
    assert_eq!(
        doc.project.bindings[0].source,
        Source::Key {
            key: ac_core::Key::from_char('x').unwrap()
        }
    );
}

#[test]
fn deleting_a_node_removes_its_bindings_in_the_same_undo_step() {
    let mut p = two_nodes();
    p.bindings.push(Binding::input(
        Source::MidiCc {
            channel: None,
            cc: 1,
        },
        NodePath::root(NodeId(2)),
        "gain",
    ));
    p.bindings.push(Binding::input(
        Source::MidiCc {
            channel: None,
            cc: 2,
        },
        NodePath::root(NodeId(1)),
        "value",
    ));
    p.bindings.push(Binding::transport(
        Source::MidiNote {
            channel: None,
            note: 36,
        },
        TransportAction::PlayStop,
    ));
    let mut h = harness(p);
    h.state_mut().doc.selection = [NodeId(2)].into();
    h.hover_at(Pos2::new(700.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::X);
    let doc = h.state().document();
    assert!(doc.graph().node(NodeId(2)).is_none());
    let left: Vec<&Target> = doc.project.bindings.iter().map(|b| &b.target).collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(
        doc.project
            .bindings
            .iter()
            .all(|b| b.node() != Some(&NodePath::root(NodeId(2))))
    );
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let doc = h.state().document();
    assert!(doc.graph().node(NodeId(2)).is_some());
    assert_eq!(doc.project.bindings.len(), 3, "one undo brings both back");
}

#[test]
fn the_transport_preset_adds_three_gamepad_buttons() {
    let mut h = harness(two_nodes());
    h.state_mut().side.tab = Tab::Devices;
    steps(&mut h, 3);
    h.get_by_label_contains("Add gamepad transport buttons").click();
    steps(&mut h, 3);
    let actions: Vec<TransportAction> = h
        .state()
        .document()
        .project
        .bindings
        .iter()
        .filter_map(|b| match b.target {
            Target::Transport { action } => Some(action),
            _ => None,
        })
        .collect();
    assert_eq!(
        actions,
        vec![
            TransportAction::PlayStop,
            TransportAction::Stop,
            TransportAction::Record
        ]
    );
    assert!(h.query_by_label_contains("Gamepad A (south)").is_some());
    // Again: nothing new, and one undo removes all three.
    h.get_by_label_contains("Add gamepad transport buttons").click();
    steps(&mut h, 3);
    assert_eq!(h.state().document().project.bindings.len(), 3);
    h.hover_at(Pos2::new(700.0, 600.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(h.state().document().project.bindings.is_empty());
}

#[test]
fn control_nodes_offer_bind_in_their_menu() {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(4), "xy_pad").at(0.0, 0.0));
    let mut h = harness(p);
    let rect = h.state().canvas.node_screen_rect(NodeId(4)).unwrap();
    // The title strip.
    right_click(&mut h, rect.left_top() + vec2(40.0, 10.0));
    assert!(h.query_by_label_contains("Bind X…").is_some());
    h.get_by_label_contains("Bind Y…").click();
    steps(&mut h, 3);
    let learn = h.state().learn.as_ref().expect("learning");
    assert_eq!(
        learn.target,
        ac_app::bindings::LearnTarget::Input {
            node: NodePath::root(NodeId(4)),
            socket: "y".into()
        }
    );
}
