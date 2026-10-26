//! Stop always stops (ROADMAP M4.1): the transport bar's Stop works with
//! the transport already stopped and reaches wired `stop`s, Panic (a
//! button and `Escape` twice) silences everything, and the status bar says
//! which outputs are sounding.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, NodePath, Project, Value, Wire};
use egui::{Key, Modifiers, vec2};
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

/// Step in real time, so the null stream runs blocks, until `done` holds.
fn wait_for(h: &mut Harness<'static, EditorApp>, mut done: impl FnMut(&EditorApp) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
        h.run_steps(1);
    }
    done(h.state())
}

/// An oscillator that only a wire starts (a button), into the output; its
/// `stop` is wired too, from a second button, as a launcher's would be.
fn drone() -> Project {
    let mut p = Project::default();
    let mut osc = NodeInstance::new(NodeId(1), "oscillator").at(300.0, 0.0);
    osc.params
        .insert("start".into(), Value::Text("never".into()));
    p.graph.nodes.push(osc);
    let mut out = NodeInstance::new(NodeId(2), "audio_out").at(600.0, 0.0);
    out.ui.label = Some("Speakers".into());
    p.graph.nodes.push(out);
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "button").at(0.0, 0.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(4), "button").at(0.0, 200.0));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(3), "trigger"),
        Endpoint::new(NodeId(1), "start"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(4), "trigger"),
        Endpoint::new(NodeId(1), "stop"),
    ));
    p
}

fn sounding(app: &EditorApp) -> bool {
    app.sounding().iter().any(|o| o.name == "Speakers")
}

/// Press the first button for a moment: the drone starts, transport stopped.
fn start_drone(h: &mut Harness<'static, EditorApp>) {
    let path = NodePath::root(NodeId(3));
    h.state_mut()
        .engine
        .handle
        .set_param(&path, "pressed", Value::Bool(true));
    std::thread::sleep(Duration::from_millis(30));
    h.state_mut()
        .engine
        .handle
        .set_param(&path, "pressed", Value::Bool(false));
    assert!(
        wait_for(h, sounding),
        "the drone sounds and the status bar says where"
    );
    assert!(!h.state().engine.handle.is_playing());
}

#[test]
fn stop_with_the_transport_stopped_ends_a_wired_generator() {
    let mut h = harness(drone());
    start_drone(&mut h);
    h.get_by_label("Stop").click();
    h.run_steps(2);
    assert!(wait_for(&mut h, |a| !sounding(a)), "Stop silenced it");
    assert!(!h.state().engine.handle.is_playing());
}

#[test]
fn escape_twice_is_panic() {
    let mut h = harness(drone());
    start_drone(&mut h);
    // The stop's dropdown says Never: transport Stop leaves it alone, but
    // Panic resets it.
    h.state_mut().engine.handle.set_param(
        &NodePath::root(NodeId(1)),
        "stop",
        Value::Text("never".into()),
    );
    h.key_press_modifiers(Modifiers::NONE, Key::Escape);
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::NONE, Key::Escape);
    h.run_steps(2);
    assert_eq!(
        h.state().status(),
        Some("Panic: everything stopped and reset")
    );
    assert!(wait_for(&mut h, |a| !sounding(a)), "Panic silenced it");
}

#[test]
fn one_escape_is_not_panic() {
    let mut h = harness(drone());
    start_drone(&mut h);
    h.key_press_modifiers(Modifiers::NONE, Key::Escape);
    // Twenty frames of the harness's clock: well over half a second.
    h.run_steps(20);
    h.key_press_modifiers(Modifiers::NONE, Key::Escape);
    h.run_steps(2);
    assert_ne!(
        h.state().status(),
        Some("Panic: everything stopped and reset")
    );
    std::thread::sleep(Duration::from_millis(100));
    h.run_steps(2);
    assert!(sounding(h.state()), "still sounding");
}

#[test]
fn the_panic_button_is_in_the_transport_bar() {
    let mut h = harness(drone());
    start_drone(&mut h);
    h.get_by_label("Panic").click();
    h.run_steps(2);
    assert!(wait_for(&mut h, |a| !sounding(a)));
}
