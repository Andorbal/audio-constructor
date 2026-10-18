//! The editor as a plugin embeds it (egui_kittest): the host runs the
//! engine's processor on its own thread at its own rate, and the editor
//! keeps the control half. No devices open; edits reach the host's
//! processor; the host saves and restores the project; and the editor moves
//! between contexts (a plugin window opening and closing) and threads.

use std::sync::{Arc, Mutex};

use ac_app::audio::AudioMode;
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp, HostAudio};
use ac_core::{Endpoint, NodeId, NodeInstance, Panel, PanelItem, Project};
use ac_engine::{HostIo, Processor};
use egui::{Event, Key, Modifiers, Pos2, ViewportCommand, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn host_audio(rate: u32) -> HostAudio {
    HostAudio {
        host: "Test Host".into(),
        sample_rate: rate,
        max_block: BLOCK,
        in_channels: 2,
        out_channels: 2,
    }
}

fn hosted_editor(project: Option<Project>) -> EditorApp {
    let mut app = EditorApp::embedded(AppOptions {
        audio: AudioMode::Hosted(host_audio(RATE)),
        ..AppOptions::headless()
    });
    if let Some(p) = project {
        app.load_project(p, None);
    }
    app
}

fn harness_for(app: EditorApp) -> Harness<'static, EditorApp> {
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn harness(project: Option<Project>) -> Harness<'static, EditorApp> {
    harness_for(hosted_editor(project))
}

/// The plugin's side: it runs the processor in blocks, as a host's audio
/// callback would.
struct FakeHost {
    processor: Processor,
    out: Vec<f32>,
}

impl FakeHost {
    fn take_from(app: &mut EditorApp) -> FakeHost {
        FakeHost {
            processor: app.take_processor().expect("the host takes the processor"),
            out: vec![0.0; BLOCK * 2],
        }
    }

    /// Run `blocks` blocks; the last sample of the left channel.
    fn run(&mut self, blocks: usize) -> f32 {
        for _ in 0..blocks {
            let mut io = HostIo {
                frames: BLOCK,
                input: &[],
                in_channels: 0,
                output: &mut self.out,
                out_channels: 2,
                midi: &[],
                midi_out: None,
                controls: &[],
                osc_out: None,
            };
            self.processor.process(&mut io);
        }
        self.out[(BLOCK - 1) * 2]
    }
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

fn viewport_commands(h: &Harness<'static, EditorApp>) -> Vec<ViewportCommand> {
    h.output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|v| v.commands.clone())
        .unwrap_or_default()
}

/// A slider (0..10) into the output, pinned to a panel.
fn slider_to_out() -> Project {
    let mut p = Project::default();
    let mut s = NodeInstance::new(NodeId(1), "slider").at(0.0, 0.0);
    s.config = json!({"min": 0, "max": 10});
    p.graph.nodes.push(s);
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out").at(320.0, 0.0));
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    );
    let mut panel = Panel::new("Controls");
    panel.items = vec![PanelItem::new(NodeId(1))];
    p.panels.push(panel);
    p
}

#[test]
fn a_hosted_editor_opens_no_devices() {
    let mut h = harness(None);
    {
        let app = h.state();
        assert!(app.is_hosted());
        let e = &app.engine;
        assert!(e.midi.is_none() && e.midi_out.is_none());
        assert!(e.gamepads.is_none(), "no gamepad thread");
        assert!(e.osc_in.is_none() && e.osc_out.is_none(), "no OSC sockets");
        assert!(e.is_running() && !e.is_null());
        assert_eq!(e.info.sample_rate, RATE);
        assert_eq!(e.handle.options().sample_rate, f64::from(RATE));
        assert!(e.describe().contains("Test Host"), "{}", e.describe());
    }
    // The Devices tab says who provides the audio and offers no devices.
    h.state_mut().side.open = true;
    h.state_mut().side.tab = Tab::Devices;
    steps(&mut h, 3);
    h.get_by_label_contains("The host provides audio and MIDI");
    h.get_by_label("Test Host");
    assert!(h.query_by_label_contains("MIDI INPUTS").is_none());
    assert!(h.query_by_label_contains("Apply").is_none());
    assert!(h.query_by_label_contains("Measure latency").is_none());

    // No Quit: not in the File menu, and Ctrl+Q asks nothing.
    h.get_by_label("File").click();
    steps(&mut h, 3);
    h.get_by_label_contains("Save As");
    assert!(h.query_by_label_contains("Quit").is_none());
    press(&mut h, Modifiers::NONE, Key::Escape);
    let dirty = Project {
        name: "Unsaved".into(),
        ..Project::default()
    };
    h.state_mut().open_unsaved(dirty);
    h.state_mut().doc.changed(true);
    // The host owns the window: no title, close, or full screen asked of
    // it, even with unsaved changes to show.
    for _ in 0..3 {
        h.step();
        let cmds = viewport_commands(&h);
        assert!(
            !cmds.iter().any(|c| matches!(
                c,
                ViewportCommand::Title(_) | ViewportCommand::Close | ViewportCommand::Fullscreen(_)
            )),
            "{cmds:?}"
        );
    }
    press(&mut h, Modifiers::COMMAND, Key::Q);
    assert!(
        h.query_by_label_contains("Save changes").is_none(),
        "nothing to quit in a plugin"
    );
    // Performance mode fills the plugin window instead.
    press(&mut h, Modifiers::NONE, Key::F11);
    assert!(h.state().performance_mode());
    assert!(
        !viewport_commands(&h)
            .iter()
            .any(|c| matches!(c, ViewportCommand::Fullscreen(_)))
    );
    steps(&mut h, 3);
    assert!(h.state().performance_mode(), "the window doesn't end it");
}

/// What the hosted editor leaves out, the desktop editor shows (so the
/// test above isn't passing because nothing was drawn).
#[test]
fn a_desktop_editor_still_offers_devices_and_quit() {
    let mut h = harness_for(EditorApp::embedded(AppOptions::headless()));
    assert!(!h.state().is_hosted());
    h.state_mut().side.open = true;
    h.state_mut().side.tab = Tab::Devices;
    steps(&mut h, 3);
    h.get_by_label_contains("MIDI INPUTS");
    assert!(
        h.query_by_label_contains("The host provides audio and MIDI")
            .is_none()
    );
    h.get_by_label("File").click();
    steps(&mut h, 3);
    h.get_by_label_contains("Quit");
    press(&mut h, Modifiers::NONE, Key::Escape);
    // An unsaved change shows in the window's title.
    h.state_mut().doc.changed(true);
    h.step();
    assert!(
        viewport_commands(&h)
            .iter()
            .any(|c| matches!(c, ViewportCommand::Title(t) if t.starts_with('●')))
    );
    press(&mut h, Modifiers::COMMAND, Key::Q);
    h.get_by_label_contains("Save changes");
}

#[test]
fn edits_reach_the_processor_the_host_runs() {
    let mut h = harness(Some(slider_to_out()));
    let mut host = FakeHost::take_from(h.state_mut());
    assert!(h.state_mut().take_processor().is_none(), "taken once");
    h.state_mut().engine.handle.play();
    assert_eq!(host.run(4), 0.0, "the slider starts at 0");

    // Drag the slider on its panel to three quarters: 7.5 reaches the
    // host's processor through the editor's engine handle.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    let b = h
        .state()
        .perform
        .body_rect(NodeId(1))
        .expect("the slider is on the panel");
    let y = b.center().y;
    drag(
        &mut h,
        pos2(b.min.x + b.width() * 0.25, y),
        pos2(b.min.x + b.width() * 0.75, y),
    );
    let out = host.run(40);
    assert!((out - 7.5).abs() < 0.05, "{out}");
    // And a direct parameter change through the handle, as host
    // automation could send.
    let path = h.state().document().node_path(NodeId(1));
    assert!(
        h.state_mut()
            .engine
            .handle
            .set_param(&path, "value", ac_core::Value::Float(2.0))
    );
    let out = host.run(40);
    assert!((out - 2.0).abs() < 0.05, "{out}");

    // A structural edit recompiles on the editor's side and the host's
    // processor picks the new graph up: deleting the wire silences it.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    h.state_mut().doc.edit_project("Unwire", |p| {
        p.graph.wires.clear();
    });
    steps(&mut h, 2);
    let out = host.run(8);
    assert_eq!(out, 0.0);
    assert!(h.state().engine.handle.project().graph.wires.is_empty());
}

#[test]
fn take_changes_reports_edits_and_load_project_replaces_the_document() {
    let mut h = harness(Some(slider_to_out()));
    assert!(
        h.state_mut().take_changes().is_none(),
        "a loaded project isn't a change"
    );
    // Add a node from the palette.
    h.hover_at(Pos2::new(500.0, 500.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::SHIFT, Key::A);
    h.event(Event::Text("gain".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let changed = h.state_mut().take_changes().expect("an edit is a change");
    assert_eq!(changed.graph.nodes.len(), 3);
    assert!(changed.graph.nodes.iter().any(|n| n.kind == "gain"));
    assert!(changed.view.is_some(), "the view travels with it");
    assert!(h.state_mut().take_changes().is_none(), "reported once");
    // Undo is a change too.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let undone = h.state_mut().take_changes().expect("undo is a change");
    assert_eq!(undone.graph.nodes.len(), 2);
    // So is moving the canvas's nodes, but not panning the view.
    h.state_mut().doc.move_nodes(&[(NodeId(1), [40.0, 40.0])]);
    assert!(h.state_mut().take_changes().is_some());
    h.state_mut().canvas.view.center += vec2(100.0, 0.0);
    steps(&mut h, 2);
    assert!(h.state_mut().take_changes().is_none());

    // The host restores other state: the document is replaced at once,
    // with nothing to undo and nothing unsaved, and the engine runs it.
    let mut other = Project {
        name: "Restored".into(),
        ..Project::default()
    };
    other
        .graph
        .nodes
        .push(NodeInstance::new(NodeId(7), "oscillator").at(0.0, 0.0));
    h.state_mut().load_project(other.clone(), None);
    steps(&mut h, 3);
    {
        let app = h.state();
        let doc = app.document();
        assert_eq!(doc.project.name, "Restored");
        assert_eq!(doc.graph().nodes.len(), 1);
        assert!(!doc.is_dirty() && !doc.can_undo());
        assert_eq!(app.engine.handle.project().graph, other.graph);
        assert_eq!(app.project_path(), None);
    }
    assert!(
        h.state_mut().take_changes().is_none(),
        "the host knows what it loaded"
    );
    assert_eq!(h.state().project().graph, other.graph);
}

#[test]
fn restart_engine_follows_the_hosts_new_rate() {
    let mut h = harness(Some(slider_to_out()));
    let first = FakeHost::take_from(h.state_mut());
    let mut audio = host_audio(96_000);
    audio.max_block = 64;
    let processor = h
        .state_mut()
        .restart_engine(audio)
        .expect("a hosted editor restarts");
    drop(first);
    let mut host = FakeHost {
        processor,
        out: vec![0.0; BLOCK * 2],
    };
    steps(&mut h, 2);
    let app = h.state();
    assert_eq!(app.engine.info.sample_rate, 96_000);
    assert_eq!(app.engine.handle.options().sample_rate, 96_000.0);
    assert_eq!(app.engine.handle.options().max_block, 64);
    assert_eq!(
        app.engine.handle.project().graph,
        app.document().project.graph
    );
    assert!(app.engine.describe().contains("96000 Hz"));
    // The new processor runs (blocks longer than its largest split up).
    h.state_mut().engine.handle.play();
    host.run(2);
    assert_eq!(h.state().engine.handle.position(), 2 * BLOCK as u64);

    // A desktop editor has no host to restart for.
    let mut desktop = EditorApp::embedded(AppOptions::headless());
    assert!(!desktop.is_hosted());
    assert!(desktop.restart_engine(host_audio(RATE)).is_none());
}

#[test]
fn an_engine_the_host_built_can_be_attached() {
    let mut h = harness(Some(slider_to_out()));
    let registry = Arc::new(ac_nodes::registry());
    // The host builds its engine from the editor's project, at its rate.
    let (engine, _) = ac_engine::Engine::new(
        h.state().project(),
        registry,
        Arc::new(ac_core::NoResources),
        ac_engine::EngineOptions {
            sample_rate: 48_000.0,
            max_block: BLOCK,
            socket_telemetry: true,
            offline: false,
        },
    );
    let (handle, processor) = engine.split();
    h.state_mut().attach_engine(handle, host_audio(48_000));
    let mut host = FakeHost {
        processor,
        out: vec![0.0; BLOCK * 2],
    };
    h.state_mut().engine.handle.play();
    host.run(3);
    steps(&mut h, 2);
    assert_eq!(h.state().engine.handle.position(), 3 * BLOCK as u64);
    assert_eq!(h.state().engine.info.sample_rate, 48_000);
}

#[test]
fn the_editor_moves_between_contexts_and_threads() {
    // A plugin keeps one editor for its lifetime and shares it with each
    // window it opens; on X11 that window draws on its own thread.
    let shared = Arc::new(Mutex::new(hosted_editor(Some(slider_to_out()))));
    let window = |shared: Arc<Mutex<EditorApp>>, frames: usize| {
        let mut h = Harness::builder()
            .with_size(vec2(1200.0, 800.0))
            .with_step_dt(1.0 / 30.0)
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, app: &mut Arc<Mutex<EditorApp>>| {
                    let mut app = app.lock().unwrap_or_else(|e| e.into_inner());
                    app.ui(ui);
                },
                shared,
            );
        h.run_steps(frames);
        // The editor's look was applied to this new context.
        let visuals = h.ctx.global_style().visuals.clone();
        assert_eq!(visuals.panel_fill, ac_app::theme::DARK.panel_bg);
        h.get_by_label("File");
    };
    let s = shared.clone();
    std::thread::spawn(move || window(s, 6))
        .join()
        .expect("the first window draws on its own thread");
    {
        let mut app = shared.lock().unwrap();
        app.doc.move_nodes(&[(NodeId(1), [20.0, 20.0])]);
        assert!(app.take_changes().is_some());
        // While no window is open the host keeps the engine in step.
        app.doc.edit_project("Unwire", |p| p.graph.wires.clear());
        app.upkeep();
        assert!(app.engine.handle.project().graph.wires.is_empty());
    }
    // Reopened: a fresh context (whose clock starts at zero again), the
    // same document and undo history.
    window(shared.clone(), 6);
    let app = shared.lock().unwrap();
    assert_eq!(app.document().undo_label(), Some("Unwire"));
    assert_eq!(app.document().graph().nodes.len(), 2);
}
