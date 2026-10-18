//! Canvas aids for big patches, driven without a window (egui_kittest):
//! per-node timing in the headers (`Ctrl+T`), portals, and the minimap. The
//! engine runs on the null audio stream, as it does with no device.

use std::time::Duration;

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Project};
use egui::{Key, Modifiers, Pos2, vec2};
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
