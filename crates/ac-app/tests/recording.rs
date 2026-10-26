//! Recording in the editor (ROADMAP M4.6): a MIDI take becomes a MIDI
//! Pattern beside its MIDI Record node.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Project};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
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

#[test]
fn a_midi_take_becomes_a_pattern_beside_its_record_node() {
    // A riff plays into a MIDI Record and a MIDI Note while recording.
    let mut p = Project::default();
    p.graph.nodes.push(
        NodeInstance::new(NodeId(1), "midi_pattern").with_config(
            json!({ "length": 1, "notes": [[0, 0.25, 60, 100], [0.5, 0.25, 64, 90]] }),
        ),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "midi_record").at(300.0, 0.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "midi_note").at(300.0, 300.0));
    p.graph.connect(
        Endpoint::new(NodeId(1), "midi"),
        Endpoint::new(NodeId(2), "midi"),
    );
    p.graph.connect(
        Endpoint::new(NodeId(1), "midi"),
        Endpoint::new(NodeId(3), "midi"),
    );
    let mut h = harness(p);
    h.hover_at(egui::pos2(700.0, 500.0));
    h.run_steps(2);
    // R records (and plays); a beat at 120 BPM is half a second.
    h.key_press_modifiers(Modifiers::NONE, Key::R);
    h.run_steps(2);
    std::thread::sleep(Duration::from_millis(700));
    h.key_press_modifiers(Modifiers::NONE, Key::Space);
    let made = wait_for(&mut h, |app| {
        app.document()
            .project
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == "midi_pattern")
            .count()
            == 2
    });
    assert!(made, "a pattern from the take");
    let g = &h.state().document().project.graph;
    let take = g
        .nodes
        .iter()
        .find(|n| n.kind == "midi_pattern" && n.id != NodeId(1))
        .unwrap();
    let notes = take.config["notes"].as_array().unwrap();
    assert!(notes.len() >= 2, "{notes:?}");
    assert_eq!(notes[0][2], 60);
    assert_eq!(h.state().document().undo_label(), Some("MIDI take"));
    // Merged into the MIDI Note with the riff.
    let into = g.wire_into(&Endpoint::new(NodeId(3), "midi")).unwrap();
    assert_eq!(g.node(into.from.node).unwrap().kind, "merge");
}

fn drag(h: &mut Harness<'static, EditorApp>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=10 {
        h.hover_at(from + (to - from) * (i as f32 / 10.0));
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }
    h.drop_at(to);
    h.run_steps(2);
}

fn graphs(app: &EditorApp) -> Vec<NodeInstance> {
    app.document()
        .project
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind == "graph")
        .cloned()
        .collect()
}

#[test]
fn moving_a_knob_while_recording_writes_automation_into_it() {
    for (mode, back) in [
        (ac_core::AutomationMode::Write, false),
        (ac_core::AutomationMode::Touch, true),
    ] {
        let mut p = Project::default();
        p.settings.automation = mode;
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "knob").at(200.0, 100.0));
        let mut h = harness(p);
        h.hover_at(egui::pos2(900.0, 700.0));
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::NONE, Key::R);
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(150));
        h.run_steps(1);
        let knob = h
            .state()
            .canvas
            .node_screen_rect(NodeId(1))
            .expect("drawn")
            .center();
        drag(&mut h, knob, knob - vec2(0.0, 60.0));
        std::thread::sleep(Duration::from_millis(150));
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::NONE, Key::Space);
        assert!(
            wait_for(&mut h, |app| graphs(app).len() == 1),
            "{mode:?}: a Graph"
        );
        let g = graphs(h.state()).remove(0);
        assert_eq!(g.config["unit"], "beats");
        let points = g.config["points"].as_array().unwrap().clone();
        assert!(points.len() >= 3, "{points:?}");
        let first = points[0][1].as_f64().unwrap();
        let top = points
            .iter()
            .map(|p| p[1].as_f64().unwrap())
            .fold(f64::MIN, f64::max);
        assert!(top > first + 0.05, "the knob went up: {points:?}");
        let last = points.last().unwrap()[1].as_f64().unwrap();
        assert_eq!((last - first).abs() < 1e-9, back, "{mode:?}: {points:?}");
        let doc = h.state().document();
        assert_eq!(doc.undo_label(), Some("Write automation"));
        let into = doc
            .project
            .graph
            .wire_into(&Endpoint::new(NodeId(1), "value"))
            .map(|w| w.from.node);
        assert_eq!(into, Some(g.id), "wired into the knob");
    }
}

#[test]
fn a_tempo_node_sets_the_songs_tempo() {
    let mut p = Project::default();
    p.graph.nodes.push(NodeInstance::new(NodeId(1), "tempo"));
    let mut h = harness(p);
    // Let the engine run so the editor has seen the node.
    std::thread::sleep(Duration::from_millis(100));
    h.run_steps(3);
    {
        let app = h.state_mut();
        ac_app::params::set(
            &mut app.doc,
            &mut app.engine.handle,
            NodeId(1),
            "bpm",
            ac_core::Value::Float(96.0),
            1.0,
        );
    }
    let set = wait_for(&mut h, |app| {
        (app.document().project.settings.tempo[0].bpm - 96.0).abs() < 1e-9
    });
    assert!(set, "the tempo map follows the node");
}
