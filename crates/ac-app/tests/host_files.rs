//! The editor when its host keeps the files (the web build, DECISIONS
//! D153): file commands become requests the host answers later, and audio
//! files live in memory. Nothing touches the disk.

use std::path::{Path, PathBuf};

use ac_app::actions::Action;
use ac_app::audio::AudioMode;
use ac_app::settings::AppSettings;
use ac_app::{AppOptions, AudioPurpose, EditorApp, FileRequest, HostAudio, SaveKind};
use ac_core::{NodeId, NodeInstance, Project};
use serde_json::json;

fn editor() -> EditorApp {
    let audio = HostAudio {
        host: "Web Audio".into(),
        sample_rate: 48_000,
        max_block: 128,
        in_channels: 2,
        out_channels: 2,
    };
    let opts = AppOptions::web(audio, 7, AppSettings::in_memory());
    assert!(opts.host_files);
    assert!(matches!(opts.audio, AudioMode::Hosted(_)));
    EditorApp::embedded(opts)
}

fn run(app: &mut EditorApp, a: Action) {
    let ctx = egui::Context::default();
    app.run(&ctx, a, 0.0);
}

fn wav(v: f32) -> Vec<u8> {
    ac_io::wav_bytes(&[vec![v; 480]], 48_000, ac_io::BitDepth::Float32).unwrap()
}

fn with_clip(file: &str) -> Project {
    let mut p = Project {
        name: "Clips".into(),
        ..Project::default()
    };
    p.graph.nodes.push(
        NodeInstance::new(NodeId(1), "clip").with_config(json!({"file": file, "channels": 1})),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out").at(300.0, 0.0));
    p
}

fn save_request(app: &mut EditorApp) -> (String, Vec<u8>, SaveKind, bool, u64) {
    let mut requests = app.take_file_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    match requests.remove(0) {
        FileRequest::Save {
            name,
            bytes,
            kind,
            same_place,
            revision,
        } => (name, bytes, kind, same_place, revision),
        other => panic!("not a save: {other:?}"),
    }
}

#[test]
fn opening_asks_the_host_and_takes_the_bytes() {
    let mut app = editor();
    run(&mut app, Action::Open);
    assert_eq!(app.take_file_requests(), vec![FileRequest::Open]);
    assert!(app.take_file_requests().is_empty(), "taken once");
    let project = Project {
        name: "From Disk".into(),
        ..Project::default()
    };
    app.open_file("song.acproj", project.to_json().as_bytes());
    assert_eq!(app.document().project.name, "From Disk");
    assert_eq!(
        app.document().path.as_deref(),
        Some(Path::new("song.acproj"))
    );
    assert!(!app.document().is_dirty());
    // Something that isn't a project is reported, and nothing changes.
    app.open_file("broken.acproj", b"{ nope");
    assert_eq!(app.document().project.name, "From Disk");
    assert!(
        app.status()
            .unwrap()
            .contains("Couldn't open broken.acproj")
    );
}

#[test]
fn saving_hands_over_the_project_and_marks_it_saved() {
    let mut app = editor();
    run(&mut app, Action::Save);
    let (name, bytes, kind, same_place, revision) = save_request(&mut app);
    assert_eq!(kind, SaveKind::Project);
    assert!(!same_place, "an untitled project asks where");
    assert!(name.ends_with(".acproj"), "{name}");
    let saved = Project::from_json(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert_eq!(saved.graph, app.document().project.graph);
    assert!(saved.view.is_some(), "the view is kept");
    app.file_saved(SaveKind::Project, "mine.acproj", revision);
    assert!(!app.document().is_dirty());
    assert_eq!(
        app.document().path.as_deref(),
        Some(Path::new("mine.acproj"))
    );
    // Saved again: the host may write to the same place.
    run(&mut app, Action::Save);
    let (name, _, _, same_place, _) = save_request(&mut app);
    assert_eq!(name, "mine.acproj");
    assert!(same_place);
    // Save As always asks.
    run(&mut app, Action::SaveAs);
    assert!(!save_request(&mut app).3);
    // Cancelled or failed: said, and nothing else changes.
    app.file_not_saved(Some("the disk is full"));
    assert!(app.status().unwrap().contains("the disk is full"));
}

#[test]
fn edits_made_while_a_save_is_written_stay_unsaved() {
    let mut app = editor();
    run(&mut app, Action::Save);
    let (_, _, _, _, revision) = save_request(&mut app);
    let p = with_clip("x.wav");
    app.open_file("other.acproj", p.to_json().as_bytes());
    run(&mut app, Action::SelectAll);
    run(&mut app, Action::Delete);
    assert!(app.document().is_dirty());
    let edited = app.document().revision;
    assert!(edited > revision);
    app.file_saved(SaveKind::Project, "other.acproj", revision);
    assert!(
        app.document().is_dirty(),
        "the deletion came after the save"
    );
}

#[test]
fn audio_files_live_in_memory_and_go_into_packs() {
    let mut app = editor();
    // A project on its own: its audio file is missing until located.
    app.open_file("clips.acproj", with_clip("drums.wav").to_json().as_bytes());
    assert!(app.assets().get("drums.wav").is_none());
    assert_eq!(app.assets().issues.len(), 1);
    // Locate it with a file the person picks.
    app.audio_file(
        AudioPurpose::Locate("drums.wav".into()),
        "drums.wav",
        wav(0.5),
    );
    let node = app.document().project.graph.node(NodeId(1)).unwrap();
    assert_eq!(node.config["file"], "assets/drums.wav");
    assert!(app.assets().get("assets/drums.wav").is_some());
    // A Clip's own file, picked the same way; one that isn't audio is refused.
    app.audio_file(
        AudioPurpose::Clip(NodeId(1)),
        "notes.txt",
        b"hello".to_vec(),
    );
    assert!(app.status().unwrap().contains("Couldn't use notes.txt"));
    app.audio_file(AudioPurpose::Clip(NodeId(1)), "drums.wav", wav(0.25));
    let node = app.document().project.graph.node(NodeId(1)).unwrap();
    assert_eq!(
        node.config["file"], "assets/drums-2.wav",
        "another file of the same name gets a new one"
    );

    // Export Pack carries the audio; opening the pack brings it back.
    run(&mut app, Action::ExportPack);
    let (name, bytes, kind, _, _) = save_request(&mut app);
    assert_eq!(kind, SaveKind::Pack);
    assert!(name.ends_with(".acpack"), "{name}");
    let u = ac_io::unpack_in_memory(&bytes).unwrap();
    assert!(u.files.iter().any(|(p, _)| p == "assets/drums-2.wav"));

    let mut other = editor();
    other.open_file(&name, &bytes);
    assert!(
        other.assets().issues.is_empty(),
        "{:?}",
        other.assets().issues
    );
    assert!(other.assets().get("assets/drums-2.wav").is_some());
    assert!(
        other.document().path.is_none(),
        "a pack opens as a new project"
    );
}

#[test]
fn examples_come_from_the_host() {
    let mut app = editor();
    let start = Project {
        name: "Start".into(),
        ..with_clip("assets/drums.wav")
    };
    app.set_examples(vec![(
        PathBuf::from("examples/start.acproj"),
        start.clone(),
    )]);
    app.request_example(PathBuf::from("examples/start.acproj"));
    assert_eq!(
        app.take_file_requests(),
        vec![FileRequest::Example(PathBuf::from("examples/start.acproj"))]
    );
    app.open_example_files(
        Path::new("examples/start.acproj"),
        &start.to_json(),
        vec![("assets/drums.wav".into(), wav(0.1))],
    );
    assert_eq!(app.document().project.name, "Start");
    assert!(app.document().path.is_none(), "an example opens as a copy");
    assert!(app.assets().issues.is_empty());
    assert!(app.status().unwrap().contains("Opened the example “Start”"));
}

#[test]
fn exporting_renders_into_a_wav_for_the_host() {
    let mut app = editor();
    let mut p = Project {
        name: "Tone".into(),
        ..Project::default()
    };
    p.settings.length = 0.1;
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "oscillator"));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out").at(300.0, 0.0));
    p.graph.wires.push(ac_core::Wire::new(
        ac_core::Endpoint::new(NodeId(1), "out"),
        ac_core::Endpoint::new(NodeId(2), "in"),
    ));
    app.load_project(p, None);
    run(&mut app, Action::ExportWav);
    let (name, bytes, kind, _, _) = save_request(&mut app);
    assert_eq!(kind, SaveKind::Wav);
    assert_eq!(name, "Tone.wav");
    let audio = ac_io::wav::read_wav_bytes(&bytes).unwrap();
    assert_eq!(audio.frames(), 4_800);
}

#[test]
fn block_files_join_the_library() {
    let mut app = editor();
    let block = ac_io::shipped_blocks()[0].block.clone();
    let mut b = block.clone();
    b.def.name = "Shared Thing".into();
    app.open_file("shared.acblock", b.to_json().as_bytes());
    assert!(app.status().unwrap().contains("Imported Shared Thing"));
    run(&mut app, Action::Open);
    assert_eq!(app.take_file_requests(), vec![FileRequest::Open]);
}
