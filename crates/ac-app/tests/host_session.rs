//! The editor of a host that keeps the project itself (a plugin's session,
//! egui_kittest): each frame the host lends the editor its engine handle,
//! takes the edits back, and does what the editor asks of it. Its macros
//! move through the host, and its transport is the host's.

use std::sync::{Arc, Mutex};

use ac_app::audio::AudioMode;
use ac_app::host::{HostControls, HostParam, HostRequest};
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp, HostAudio};
use ac_core::{NodeId, NodeInstance, Project, Value};
use ac_engine::{Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};
use egui::{Event, Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Stands in for the plugin's session: the project lives in its engine.
struct Session {
    editor: EditorApp,
    handle: EngineHandle,
    processor: Processor,
    /// Every project the editor handed back.
    pushed: Vec<Project>,
    requests: Vec<HostRequest>,
}

/// What the macros heard.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

struct Macros {
    values: [f32; 2],
    log: Log,
}

impl HostControls for Macros {
    fn params(&self) -> Vec<HostParam> {
        (0..2)
            .map(|i| HostParam {
                name: format!("Macro {}", i + 1),
                target: (i == 0).then(|| "Gain › gain".to_string()),
                value: self.values[i],
                text: self.format(i, self.values[i]),
            })
            .collect()
    }
    fn begin(&mut self, index: usize) {
        self.log.0.lock().unwrap().push(format!("begin {index}"));
    }
    fn set(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        self.log.0.lock().unwrap().push(format!("set {index}"));
    }
    fn end(&mut self, index: usize) {
        self.log.0.lock().unwrap().push(format!("end {index}"));
    }
    fn format(&self, _index: usize, value: f32) -> String {
        format!("{:.0} %", value * 100.0)
    }
}

fn project() -> Project {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "gain").at(0.0, 0.0));
    p
}

fn harness(log: &Log) -> Harness<'static, Session> {
    let (engine, _) = Engine::new(
        project(),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        EngineOptions {
            sample_rate: 44_100.0,
            max_block: 64,
            socket_telemetry: true,
            offline: false,
        },
    );
    let (handle, processor) = engine.split();
    let mut editor = EditorApp::embedded(AppOptions {
        audio: AudioMode::Hosted(HostAudio::default()),
        ..AppOptions::headless()
    });
    editor.host_keeps_project();
    editor.set_host_controls(Box::new(Macros {
        values: [0.5, 0.0],
        log: log.clone(),
    }));
    editor.load_project_with(project(), None, ac_io::AssetStore::default());
    let session = Session {
        editor,
        handle,
        processor,
        pushed: Vec::new(),
        requests: Vec::new(),
    };
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, s: &mut Session| {
                let Session {
                    editor,
                    handle,
                    processor,
                    pushed,
                    requests,
                } = s;
                editor.with_engine(handle, |e| e.ui(ui));
                if let Some(p) = editor.take_engine_edit() {
                    let r = handle.edit(GraphEdit::Replace(Box::new(p.clone())));
                    editor.host_compiled(Some(r));
                    pushed.push(p);
                }
                requests.extend(editor.take_host_requests());
                // The host's audio thread: a block a frame.
                let mut out = [0.0f32; 128];
                processor.process(&mut HostIo {
                    frames: 64,
                    input: &[],
                    in_channels: 0,
                    output: &mut out,
                    out_channels: 2,
                    midi: &[],
                    midi_out: None,
                    controls: &[],
                    osc_out: None,
                });
                handle.collect_garbage();
            },
            session,
        );
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'static, Session>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

#[test]
fn edits_go_back_to_the_host_which_compiles_them() {
    let log = Log::default();
    let mut h = harness(&log);
    assert!(h.state().pushed.is_empty(), "loading isn't an edit");
    // The editor ran on the host's handle: the host's rate shows.
    assert_eq!(h.state().editor.engine.info.sample_rate, 44_100);
    // Add a node from the palette.
    h.hover_at(Pos2::new(600.0, 500.0));
    h.run_steps(2);
    press(&mut h, Modifiers::SHIFT, Key::A);
    h.event(Event::Text("oscillator".into()));
    h.run_steps(3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let s = h.state();
    let last = s.pushed.last().expect("the edit went to the host");
    assert!(last.graph.nodes.iter().any(|n| n.kind == "oscillator"));
    assert_eq!(
        s.handle.project().graph,
        last.graph,
        "the host's engine runs it"
    );
    assert!(s.editor.engine.report.diagnostics.is_empty());

    // An inline value moves on the host's engine at once (no recompile).
    let pushed = h.state().pushed.len();
    let path = h.state().editor.document().node_path(NodeId(1));
    let Session { editor, handle, .. } = h.state_mut();
    editor.with_engine(handle, |e| {
        assert!(e.engine.handle.set_param(&path, "gain", Value::Float(-6.0)));
    });
    assert_eq!(
        h.state()
            .handle
            .project()
            .graph
            .node(NodeId(1))
            .unwrap()
            .params["gain"],
        Value::Float(-6.0)
    );
    h.run_steps(2);
    assert_eq!(h.state().pushed.len(), pushed);
}

#[test]
fn the_host_runs_the_transport() {
    let log = Log::default();
    let mut h = harness(&log);
    // Space, Shift+Space, and R are the host's.
    press(&mut h, Modifiers::NONE, Key::Space);
    press(&mut h, Modifiers::SHIFT, Key::Space);
    press(&mut h, Modifiers::NONE, Key::R);
    assert!(!h.state().handle.is_playing());
    // Play keys can't reach a host that doesn't take the keys.
    press(&mut h, Modifiers::COMMAND, Key::K);
    assert!(!h.state().editor.play_keys());
    // The engine does play when the host says so.
    h.state_mut().handle.play();
    h.run_steps(2);
    assert!(h.state().handle.is_playing());
}

#[test]
fn macros_move_through_the_host_as_one_gesture() {
    let log = Log::default();
    let mut h = harness(&log);
    h.state_mut().editor.side.open = true;
    h.state_mut().editor.side.tab = Tab::Devices;
    h.run_steps(3);
    h.get_by_label("MACROS");
    h.get_by_label_contains("The host provides audio and MIDI");
    // The transport bindings are the editor's transport's, so they're gone.
    assert!(h.query_by_label_contains("TRANSPORT BUTTONS").is_none());
    // The macro's slider, named by its label for screen readers.
    let rect = h
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Macro 1")
        .rect();
    let y = rect.center().y;
    let from = Pos2::new(rect.min.x + rect.width() * 0.2, y);
    let to = Pos2::new(rect.min.x + rect.width() * 0.8, y);
    log.take();
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=4 {
        h.hover_at(from + (to - from) * (i as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let heard = log.take();
    assert_eq!(
        heard.first().map(String::as_str),
        Some("begin 0"),
        "{heard:?}"
    );
    assert_eq!(heard.last().map(String::as_str), Some("end 0"), "{heard:?}");
    assert!(
        heard.iter().filter(|e| *e == "set 0").count() >= 2,
        "{heard:?}"
    );
    assert_eq!(heard.iter().filter(|e| e.starts_with("begin")).count(), 1);
}

#[test]
fn new_open_and_picked_files_are_asked_of_the_host() {
    let log = Log::default();
    let mut h = harness(&log);
    // File ▸ New on an unchanged document: straight to the host.
    press(&mut h, Modifiers::COMMAND, Key::N);
    assert_eq!(h.state().requests, vec![HostRequest::New]);
    assert_eq!(
        h.state().editor.document().graph().nodes.len(),
        1,
        "the document waits for the host's project"
    );
    // A file picked for a Clip goes to the host under assets/.
    let dir = std::env::temp_dir().join(format!("ac-host-session-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("hit.wav");
    ac_io::write_wav(&wav, &[vec![0.25; 32]], 48_000, ac_io::BitDepth::Float32).unwrap();
    let mut p = project();
    let mut clip = NodeInstance::new(NodeId(5), "clip").at(300.0, 0.0);
    clip.config = json!({});
    p.graph.nodes.push(clip);
    h.state_mut()
        .editor
        .load_project_with(p, None, ac_io::AssetStore::default());
    h.state_mut().requests.clear();
    h.state_mut().editor.assign_clip(NodeId(5), &wav, 0.0);
    h.run_steps(2);
    let requests = std::mem::take(&mut h.state_mut().requests);
    let [HostRequest::AddAsset { path, bytes }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert!(
        path.starts_with("assets/hit-") && path.ends_with(".wav"),
        "{path}"
    );
    assert_eq!(bytes, &std::fs::read(&wav).unwrap());
    let doc = h.state().editor.document();
    assert_eq!(
        doc.graph().node(NodeId(5)).unwrap().config["file"],
        json!(path)
    );
    // The host's engine got the edit.
    assert!(
        h.state()
            .pushed
            .last()
            .is_some_and(|p| p.graph.node(NodeId(5)).unwrap().config["file"] == json!(path))
    );
    // Opening a file is the host's too: it loads the audio beside it.
    let file = dir.join("other.acproj");
    std::fs::write(&file, project().to_json()).unwrap();
    h.state_mut().editor.open(&file, 0.0);
    h.run_steps(1);
    assert_eq!(h.state().requests, vec![HostRequest::Open(file.clone())]);
    std::fs::remove_dir_all(&dir).ok();
}
