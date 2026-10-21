//! Blocks and sharing in the editor, without a window (egui_kittest):
//! adding a library block from the palette, saving a group to the library
//! and updating older copies, opening packs and block files, exporting
//! packs and blocks, playing previews, and importing blocks, packs, and
//! shared libraries from links (served on this machine). The library lives in memory unless a test gives it a
//! folder.

use std::path::{Path, PathBuf};

use ac_app::actions::Action;
use ac_app::audio::AudioMode;
use ac_app::dialogs::{BlockToExport, FilePurpose};
use ac_app::online::{Playing, PreviewKey};
use ac_app::palette::{self, Candidate};
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp};
use ac_core::{
    Block, BlockSource, Endpoint, GROUP_KIND, GroupDef, GroupId, GroupInput, GroupOutput, NodeId,
    NodeInstance, Origin, Project, SignalType, Unit, VOICES_KIND,
};
use egui::{Event, Key, Modifiers, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
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
        .with_size(egui::vec2(1400.0, 900.0))
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

fn status(h: &Harness<'static, EditorApp>) -> String {
    h.state().status().unwrap_or_default().to_string()
}

/// A fresh folder under the system temp directory.
fn temp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = std::env::temp_dir().join(format!("ac-app-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn audio_input(name: &str, target: u64) -> GroupInput {
    GroupInput {
        name: name.into(),
        ty: SignalType::Audio,
        unit: Unit::Generic,
        default: None,
        range: None,
        targets: vec![Endpoint::new(NodeId(target), "in")],
    }
}

/// "My Fx": a gain inside, `in` → gain → `out`, plus inputs named in
/// `extra` (all feeding the gain).
fn fx_def(extra: &[&str]) -> GroupDef {
    let mut d = GroupDef::new("My Fx");
    d.description = "Turns it down. Mostly.".into();
    d.tags = vec!["utility".into()];
    d.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "gain").at(0.0, 0.0));
    d.inputs.push(audio_input("in", 1));
    for e in extra {
        d.inputs.push(audio_input(e, 1));
    }
    d.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(1), "out"),
    });
    d
}

/// One instance (node 1) of `def`, stored as `fx`, fed by a constant
/// (node 2) on each of its inputs.
fn with_instance(def: GroupDef) -> Project {
    let mut p = Project::default();
    let inputs: Vec<String> = def.inputs.iter().map(|i| i.name.clone()).collect();
    p.group_defs.insert(GroupId("fx".into()), def);
    p.graph.nodes.push(
        NodeInstance::new(NodeId(1), GROUP_KIND)
            .with_config(json!({ "def": "fx" }))
            .at(320.0, 0.0),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "constant").at(0.0, 0.0));
    for i in inputs {
        p.graph
            .connect(Endpoint::new(NodeId(2), "out"), Endpoint::new(NodeId(1), i));
    }
    p
}

fn fx(h: &Harness<'static, EditorApp>) -> GroupDef {
    h.state().document().project.group_defs[&GroupId("fx".into())].clone()
}

/// Select node 1 and show the Node tab.
fn select_instance(h: &mut Harness<'static, EditorApp>) {
    h.state_mut().doc.selection = [NodeId(1)].into();
    h.state_mut().side.tab = Tab::Node;
    steps(h, 3);
}

// ---------------------------------------------------------------- adding

#[test]
fn the_palette_adds_a_shipped_block() {
    let mut h = harness(Some(Project::default()));
    h.hover_at(Pos2::new(500.0, 400.0));
    steps(&mut h, 2);
    press(&mut h, Modifiers::SHIFT, Key::A);
    h.event(Event::Text("tremolo".into()));
    steps(&mut h, 3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), 1, "one instance");
    let node = &doc.graph().nodes[0];
    assert_eq!(node.kind, GROUP_KIND);
    assert_eq!(node.ui.label.as_deref(), Some("Tremolo"));
    let def = &doc.project.group_defs[&node.group_def().unwrap()];
    assert_eq!(def.source.as_deref(), Some("shipped:tremolo"));
    assert_eq!(def.name, "Tremolo");
    assert!(doc.selection.contains(&node.id));
    assert_eq!(doc.undo_label(), Some("Add Tremolo"));
    // A second one shares the definition.
    let c = palette::candidates(
        &h.state().registry,
        &h.state().library,
        h.state().document(),
        "tremolo",
        None,
        &[],
    )[0]
    .clone();
    h.state_mut()
        .add_candidate(&c, egui::pos2(0.0, 200.0), None, 0.0)
        .unwrap();
    assert_eq!(h.state().document().project.group_defs.len(), 1);
    // Undo takes each away, definition and all.
    press(&mut h, Modifiers::COMMAND, Key::Z);
    press(&mut h, Modifiers::COMMAND, Key::Z);
    let doc = h.state().document();
    assert!(doc.graph().nodes.is_empty());
    assert!(doc.project.group_defs.is_empty());
}

#[test]
fn a_block_with_a_voice_inside_comes_polyphonic() {
    let mut h = harness(Some(Project::default()));
    let mut def = GroupDef::new("Pluck");
    def.graph.nodes.push(NodeInstance::new(NodeId(1), "voice"));
    def.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "oscillator"));
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(2), "out"),
    });
    let block = Block {
        def,
        uses: Default::default(),
        preview: None,
    };
    let source = h.state_mut().library.save_user(block).unwrap();
    let b = h.state().library.get(&source).unwrap().clone();
    let c: Candidate = palette::block_candidate(&b, None);
    assert_eq!(c.kind, VOICES_KIND);
    let id = h
        .state_mut()
        .add_candidate(&c, egui::pos2(0.0, 0.0), None, 0.0)
        .unwrap();
    let doc = h.state().document();
    let node = doc.graph().node(id).unwrap();
    assert_eq!(node.kind, VOICES_KIND);
    let def = &doc.project.group_defs[&node.group_def().unwrap()];
    assert_eq!(def.source.as_deref(), Some("user:pluck"));
}

// ---------------------------------------------------------------- saving

#[test]
fn saving_a_group_to_the_library_and_updating_an_older_copy() {
    let mut h = harness(Some(with_instance(fx_def(&["side"]))));
    select_instance(&mut h);
    h.get_by_label_contains("Save to library").click();
    steps(&mut h, 3);
    let source = BlockSource::new(Origin::User, "my-fx");
    let def = fx(&h);
    assert_eq!(def.source.as_deref(), Some("user:my-fx"));
    assert_eq!(def.version, 1);
    assert_eq!(status(&h), "Saved My Fx to your library (v1)");
    assert_eq!(h.state().document().undo_label(), Some("Save block"));
    let saved = h.state().library.get(&source).unwrap().block.clone();
    assert_eq!(saved.def.source, None, "the library's copy names no source");
    assert_eq!(saved.def.graph, def.graph);
    // Saving again makes a new version, and the project's copy follows.
    h.get_by_label_contains("Save to library").click();
    steps(&mut h, 3);
    assert_eq!(fx(&h).version, 2);
    assert_eq!(h.state().library.get(&source).unwrap().block.def.version, 2);
    assert!(h.state().library.updates(&h.state().doc.project).is_empty());
    assert!(h.query_by_label_contains("Update from library").is_none());

    // Change the library's copy: v3 has no `side` input.
    let mut v3 = Block::from_project(&with_instance(fx_def(&[])), &GroupId("fx".into())).unwrap();
    v3.def.description = "Turns it down, now without a side input.".into();
    h.state_mut().library.save_user(v3).unwrap();

    // A project with the old copy (v2, with `side`) is offered the update.
    let mut old = fx_def(&["side"]);
    old.source = Some("user:my-fx".into());
    old.version = 2;
    h.state_mut().open_unsaved(with_instance(old.clone()));
    select_instance(&mut h);
    h.get_by_label_contains("Update from library (v2 → v3)")
        .click();
    steps(&mut h, 3);
    let def = fx(&h);
    assert_eq!(def.version, 3);
    assert_eq!(def.source.as_deref(), Some("user:my-fx"));
    assert_eq!(def.inputs.len(), 1, "the new version's interface");
    assert!(def.description.contains("without a side input"));
    let s = status(&h);
    assert!(s.starts_with("Updated My Fx to v3"), "{s}");
    assert!(s.contains("removed 1 wire"), "{s}");
    let wires = &h.state().document().project.graph.wires;
    assert_eq!(wires.len(), 1, "the wire into `side` went: {wires:?}");
    // One undo step puts the old copy back.
    assert_eq!(h.state().document().undo_label(), Some("Update block"));
    press(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(fx(&h), old);
    assert_eq!(h.state().document().project.graph.wires.len(), 2);

    // The Project tab lists it too, and updates everything at once.
    h.state_mut().side.tab = Tab::Project;
    steps(&mut h, 3);
    assert!(h.query_by_label("v2 → v3").is_some());
    h.get_by_label_contains("Update all").click();
    steps(&mut h, 3);
    assert_eq!(fx(&h).version, 3);
    assert_eq!(h.state().document().undo_label(), Some("Update blocks"));
    assert!(h.state().library.updates(&h.state().doc.project).is_empty());
}

#[test]
fn removing_a_saved_block_leaves_projects_alone() {
    let mut h = harness(Some(with_instance(fx_def(&[]))));
    h.state_mut().save_block(&GroupId("fx".into()), 0.0);
    let source = BlockSource::new(Origin::User, "my-fx");
    assert!(h.state().library.get(&source).is_some());
    h.state_mut().remove_block(&source, 0.0);
    assert!(h.state().library.get(&source).is_none());
    assert!(status(&h).contains("keep their copy"));
    assert_eq!(fx(&h).source.as_deref(), Some("user:my-fx"));
    // Shipped blocks stay.
    h.state_mut()
        .remove_block(&BlockSource::new(Origin::Shipped, "tremolo"), 0.0);
    assert!(
        h.state()
            .library
            .get(&BlockSource::new(Origin::Shipped, "tremolo"))
            .is_some()
    );
}

// ---------------------------------------------------------------- files

fn save(path: &Path, p: &Project) {
    ac_io::save_project(path, p).unwrap();
}

#[test]
fn opening_a_pack_opens_the_project_inside() {
    let dir = temp("open-pack");
    // Someone else's project, using one of their own blocks.
    let mut theirs = fx_def(&[]);
    theirs.name = "Their Fx".into();
    theirs.source = Some("user:their-fx".into());
    let mut p = with_instance(theirs);
    p.name = "Their Song".into();
    let proj = dir.join("song.acproj");
    save(&proj, &p);
    let pack = dir.join("shared.acpack");
    ac_io::write_pack(&proj, &pack, &ac_nodes::registry()).unwrap();

    let mut h = harness(None);
    h.state_mut().open(&pack, 0.0);
    steps(&mut h, 2);
    let doc = h.state().document();
    let opened = doc
        .path
        .clone()
        .expect("the unpacked project is the document");
    assert_eq!(opened, dir.join("shared").join("song.acproj"));
    assert!(opened.is_file());
    assert_eq!(doc.project.name, "Their Song");
    assert!(!doc.is_dirty());
    let s = status(&h);
    assert!(s.starts_with("Unpacked shared.acpack into"), "{s}");
    assert!(s.contains("added 1 block"), "{s}");
    // Their block is in the library now, under Imported.
    let imported = BlockSource::new(Origin::Imported, "their-fx");
    assert_eq!(
        h.state().library.get(&imported).map(|b| b.name()),
        Some("Their Fx")
    );
    // Opening it again unpacks into a fresh folder.
    h.state_mut().open(&pack, 0.0);
    assert_eq!(
        h.state().document().path.clone().unwrap(),
        dir.join("shared-2").join("song.acproj")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn opening_a_block_file_imports_it() {
    let dir = temp("open-block");
    let block = Block::from_project(&with_instance(fx_def(&[])), &GroupId("fx".into())).unwrap();
    let file = dir.join("gift.acblock");
    std::fs::write(&file, block.to_json()).unwrap();
    // The document has unsaved changes; importing doesn't touch it.
    let mut h = harness(Some(with_instance(fx_def(&[]))));
    let gain = Candidate::node("Gain", "gain", None, String::new());
    h.state_mut()
        .add_candidate(&gain, egui::pos2(0.0, 300.0), None, 0.0);
    assert!(h.state().document().is_dirty());
    // File ▸ Open, typing the path (no system dialog in tests).
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, Action::Open, 0.0);
    steps(&mut h, 2);
    let prompt = h.state_mut().prompt_mut().expect("a path prompt");
    assert_eq!(prompt.purpose, FilePurpose::Open);
    prompt.text = file.display().to_string();
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state_mut().prompt_mut().is_none());
    let source = BlockSource::new(Origin::Imported, "gift");
    assert_eq!(
        h.state().library.get(&source).map(|b| b.block.clone()),
        Some(block.clone())
    );
    assert_eq!(status(&h), "Imported My Fx into your library");
    assert_eq!(h.state().side.tab, Tab::Library);
    assert_eq!(
        h.state().side.block_filter.origin,
        Some(Origin::Imported),
        "the Library tab shows where it landed"
    );
    let doc = h.state().document();
    assert!(doc.is_dirty() && doc.path.is_none(), "the document stays");
    assert_eq!(doc.graph().nodes.len(), 3);
    // The same file again adds nothing.
    let n = h.state().library.blocks().len();
    h.state_mut().open(&file, 0.0);
    assert_eq!(h.state().library.blocks().len(), n);
    assert_eq!(status(&h), "My Fx is already in your library");
    // A broken one is reported.
    let bad = dir.join("bad.acblock");
    std::fs::write(&bad, "{").unwrap();
    h.state_mut().open(&bad, 0.0);
    assert!(status(&h).starts_with("Couldn't read"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_pack_saves_first_then_writes_a_pack() {
    let dir = temp("export-pack");
    let proj = dir.join("beat.acproj");
    save(&proj, &with_instance(fx_def(&[])));
    let mut h = harness(None);
    h.state_mut().open(&proj, 0.0);
    // An unsaved change: Export Pack saves it before packing.
    let gain = Candidate::node("Gain", "gain", None, String::new());
    h.state_mut()
        .add_candidate(&gain, egui::pos2(0.0, 300.0), None, 0.0);
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, Action::ExportPack, 0.0);
    steps(&mut h, 2);
    assert!(!h.state().document().is_dirty(), "saved first");
    let prompt = h.state_mut().prompt_mut().expect("a path prompt");
    assert_eq!(prompt.purpose, FilePurpose::ExportPack);
    let out = dir.join("beat.acpack");
    assert_eq!(prompt.text, out.display().to_string());
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let s = status(&h);
    assert!(
        s.starts_with("Packed the project and 0 audio files into"),
        "{s}"
    );
    let back = ac_io::unpack(&std::fs::read(&out).unwrap(), &dir.join("check"), false).unwrap();
    let q = ac_io::load_project(&back).unwrap();
    assert_eq!(q.graph.nodes.len(), 3, "with the change");
    assert_eq!(q.group_defs.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_pack_asks_where_to_save_an_untitled_project() {
    let dir = temp("export-untitled");
    let mut h = harness(Some(with_instance(fx_def(&[]))));
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, Action::ExportPack, 0.0);
    steps(&mut h, 2);
    let prompt = h.state_mut().prompt_mut().expect("save first");
    assert_eq!(prompt.purpose, FilePurpose::SaveAs);
    let proj = dir.join("untitled.acproj");
    prompt.text = proj.display().to_string();
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().document().path.as_deref(), Some(proj.as_path()));
    // Then the pack's own dialog, next to the project.
    let prompt = h.state_mut().prompt_mut().expect("then the pack");
    assert_eq!(prompt.purpose, FilePurpose::ExportPack);
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let out = dir.join("untitled.acpack");
    assert!(out.is_file(), "{}", status(&h));
    // Cancelling the save cancels the pack.
    h.state_mut().open_unsaved(Project::default());
    h.state_mut().run(&ctx, Action::ExportPack, 0.0);
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state_mut().prompt_mut().is_none());
    h.state_mut().run(&ctx, Action::SaveAs, 0.0);
    steps(&mut h, 2);
    let prompt = h.state_mut().prompt_mut().unwrap();
    prompt.text = dir.join("other.acproj").display().to_string();
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        h.state_mut().prompt_mut().is_none(),
        "a plain Save As doesn't go on to pack"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_library_folder_keeps_saved_blocks_between_runs() {
    let dir = temp("library");
    let open = || {
        EditorApp::new(
            &egui::Context::default(),
            AppOptions {
                library: Some(dir.join("blocks")),
                ..AppOptions::headless()
            },
        )
    };
    let mut app = open();
    app.open_unsaved(with_instance(fx_def(&[])));
    app.save_block(&GroupId("fx".into()), 0.0);
    assert!(dir.join("blocks/my-fx.acblock").is_file());
    std::fs::write(dir.join("blocks/broken.acblock"), "{").unwrap();
    let app = open();
    let source = BlockSource::new(Origin::User, "my-fx");
    assert_eq!(app.library.get(&source).map(|b| b.name()), Some("My Fx"));
    let s = app.status().unwrap_or_default().to_string();
    assert!(
        s.starts_with("1 block file in your library couldn't be read"),
        "{s}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------- previews and links

/// Step the editor until `done` holds (background work runs in real time),
/// for at most 30 s.
fn wait_for(h: &mut Harness<'static, EditorApp>, what: &str, done: impl Fn(&EditorApp) -> bool) {
    let t0 = std::time::Instant::now();
    while !done(h.state()) {
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(30),
            "timed out waiting for {what} (status: {})",
            status(h)
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(1);
    }
}

/// An editor with its library in `dir`, and downloads and cached previews
/// in folders of the test's own.
fn harness_with_library(dir: &Path) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            library: Some(dir.join("blocks")),
            ..AppOptions::headless()
        },
    );
    app.online.download_dir = Some(dir.join("downloads"));
    app.online.cache_dir = Some(dir.join("cache"));
    // Tall, so the Library tab's shared libraries fit without scrolling.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1600.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

#[test]
fn a_block_preview_plays_over_the_patch_and_stops() {
    let mut h = harness(None);
    let key = PreviewKey::Library(BlockSource::new(Origin::Shipped, "tremolo"));
    h.state_mut().toggle_preview(key.clone(), 0.0);
    assert_eq!(*h.state().preview_state(), Playing::Loading(key.clone()));
    wait_for(&mut h, "the preview to render", |a| {
        matches!(a.preview_state(), Playing::Playing(..))
    });
    let Playing::Playing(_, id) = h.state().preview_state().clone() else {
        unreachable!()
    };
    wait_for(&mut h, "the stream to play it", |a| {
        a.engine.audition_playing() == Some(id)
    });
    // The same button stops it.
    h.state_mut().toggle_preview(key.clone(), 0.0);
    assert_eq!(*h.state().preview_state(), Playing::Idle);
    wait_for(&mut h, "it to stop", |a| {
        a.engine.audition_playing().is_none()
    });
    // Again: the rendered preview was kept, so it starts at once.
    h.state_mut().toggle_preview(key.clone(), 0.0);
    assert!(matches!(h.state().preview_state(), Playing::Playing(k, _) if *k == key));
    h.state_mut().toggle_preview(key, 0.0);

    // A block with nothing to hear says so.
    let mut def = GroupDef::new("Silent");
    def.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
    let quiet = h
        .state_mut()
        .library
        .save_user(Block {
            def,
            uses: Default::default(),
            preview: None,
        })
        .unwrap();
    h.state_mut()
        .toggle_preview(PreviewKey::Library(quiet), 0.0);
    assert!(status(&h).contains("no audio output"), "{}", status(&h));
    assert_eq!(*h.state().preview_state(), Playing::Idle);
}

#[test]
fn export_block_file_writes_the_block_and_its_preview() {
    let dir = temp("export-block");
    let mut h = harness(Some(with_instance(fx_def(&[]))));
    select_instance(&mut h);
    h.get_by_label_contains("Export block file").click();
    steps(&mut h, 2);
    let prompt = h.state_mut().prompt_mut().expect("a path prompt");
    assert_eq!(
        prompt.purpose,
        FilePurpose::ExportBlock(BlockToExport::Def(GroupId("fx".into())))
    );
    assert!(prompt.text.ends_with("my-fx.acblock"), "{}", prompt.text);
    let out = dir.join("my-fx.acblock");
    prompt.text = out.display().to_string();
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::Enter);
    wait_for(&mut h, "the export", |a| a.online.exporting == 0);
    let s = status(&h);
    assert!(
        s.starts_with("Exported My Fx to") && s.ends_with("with its preview"),
        "{s}"
    );
    let block = Block::from_json(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let wav = std::fs::read(dir.join("my-fx.preview.wav")).unwrap();
    assert_eq!(
        block.preview.unwrap().sha256,
        ac_io::hash::sha256_hex(&wav),
        "the block records its preview"
    );
    // A library block exports from its row's menu the same way.
    h.state_mut().export_block_to(
        &BlockToExport::Library(BlockSource::new(Origin::Shipped, "chorus")),
        dir.join("chorus"),
        0.0,
    );
    wait_for(&mut h, "the second export", |a| a.online.exporting == 0);
    assert!(dir.join("chorus.acblock").is_file() && dir.join("chorus.preview.wav").is_file());
    let _ = std::fs::remove_dir_all(dir);
}

/// Serve the files in `dir` under `/lib/` on this machine; returns the base.
fn serve(dir: &Path) -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let body = first
                .split(' ')
                .nth(1)
                .and_then(|p| p.strip_prefix("/lib/"))
                .filter(|f| !f.contains(['/', '\\']) && !f.starts_with('.'))
                .and_then(|f| std::fs::read(dir.join(f)).ok());
            let response = match body {
                Some(b) => {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        b.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&b);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    base
}

/// A shared folder of two of someone's blocks (with previews), its index,
/// and a pack.
fn shared_folder(dir: &Path) {
    let registry = std::sync::Arc::new(ac_nodes::registry());
    for (id, name) in [("their-fx", "Their Fx"), ("their-echo", "Their Echo")] {
        let mut d = fx_def(&[]);
        d.name = name.into();
        let block = Block::from_project(&with_instance(d), &GroupId("fx".into())).unwrap();
        ac_io::export_block(
            block,
            &dir.join(format!("{id}.acblock")),
            Some(registry.clone()),
        )
        .unwrap();
    }
    let b = ac_io::index::build_index(dir, &registry, Some("Shared Things"), None).unwrap();
    assert!(b.problems.is_empty(), "{:?}", b.problems);
    ac_io::index::write_index(dir, &b.index).unwrap();
    let mut p = with_instance(fx_def(&[]));
    p.name = "Their Song".into();
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    ac_io::save_project(&work.join("song.acproj"), &p).unwrap();
    ac_io::write_pack(
        &work.join("song.acproj"),
        &dir.join("song.acpack"),
        &registry,
    )
    .unwrap();
}

#[test]
fn blocks_packs_and_libraries_come_from_links() {
    let dir = temp("links");
    let shared = dir.join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    shared_folder(&shared);
    let base = serve(&shared);
    let mut h = harness_with_library(&dir);

    // File ▸ Import from Link… with a block's link.
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, Action::ImportFromLink, 0.0);
    steps(&mut h, 2);
    h.state_mut()
        .online
        .link_dialog
        .as_mut()
        .expect("the dialog")
        .text = format!("{base}/lib/their-fx.acblock");
    steps(&mut h, 1);
    h.get_by_label("Import").click();
    steps(&mut h, 2);
    assert!(h.state().online.link_dialog.is_none());
    wait_for(&mut h, "the block", |a| a.online.importing.is_empty());
    assert_eq!(
        status(&h),
        "Imported Their Fx into your library with its preview"
    );
    let fx = BlockSource::new(Origin::Imported, "their-fx");
    let got = h.state().library.get(&fx).expect("imported").clone();
    assert_eq!(
        std::fs::read(got.preview_path().unwrap()).unwrap(),
        std::fs::read(shared.join("their-fx.preview.wav")).unwrap()
    );

    // A library's link (its folder) adds it to the Library tab.
    h.state_mut().import_link(&format!("{base}/lib/"), 0.0);
    wait_for(&mut h, "the library", |a| a.online.importing.is_empty());
    let link = format!("{base}/lib/index.json");
    assert_eq!(
        h.state().settings().prefs.libraries,
        std::slice::from_ref(&link)
    );
    assert!(status(&h).starts_with("Added the library Shared Things (2 blocks)"));
    assert_eq!(h.state().side.tab, Tab::Library);
    // Searching narrows shared libraries too (and keeps them on screen).
    h.state_mut().side.library_query = "their".into();
    steps(&mut h, 3);
    assert!(h.query_by_label_contains("Shared Things  (2)").is_some());
    // The block already imported shows as installed; the other installs.
    assert!(h.query_by_label_contains("Installed").is_some());
    // The Install button says which block it installs.
    h.query_all_by_role(egui::accesskit::Role::Button)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Install "))
        })
        .expect("an Install button")
        .click();
    steps(&mut h, 2);
    wait_for(&mut h, "the install", |a| {
        a.online
            .library(&link)
            .is_some_and(|l| l.installing.is_empty())
            && a.library
                .get(&BlockSource::new(Origin::Imported, "their-echo"))
                .is_some()
    });
    let echo = BlockSource::new(Origin::Imported, "their-echo");
    assert!(
        h.state()
            .library
            .get(&echo)
            .unwrap()
            .preview_path()
            .is_some()
    );

    // Its previews download once into the cache folder, and play.
    let key = PreviewKey::Remote {
        link: link.clone(),
        id: "their-echo".into(),
    };
    h.state_mut().toggle_preview(key, 0.0);
    wait_for(&mut h, "the remote preview", |a| {
        matches!(a.preview_state(), Playing::Playing(..))
    });
    let cached = std::fs::read_dir(dir.join("cache")).unwrap().count();
    assert_eq!(cached, 1);

    // A pack's link is saved to the download folder and opened.
    h.state_mut()
        .import_link(&format!("{base}/lib/song.acpack"), 0.0);
    wait_for(&mut h, "the pack", |a| {
        a.document().project.name == "Their Song"
    });
    let opened = h.state().document().path.clone().unwrap();
    let downloads = dir.join("downloads");
    assert_eq!(opened, downloads.join("song").join("song.acproj"));
    assert!(downloads.join("song.acpack").is_file());

    // Insecure and broken links are reported.
    h.state_mut()
        .import_link("http://example.org/x.acblock", 0.0);
    wait_for(&mut h, "the refusal", |a| a.online.importing.is_empty());
    let s = status(&h);
    assert!(
        s.starts_with("Couldn't import from the link") && s.contains("https"),
        "{s}"
    );

    // Removing the library forgets its link; its blocks stay.
    h.state_mut().side.tab = Tab::Library;
    steps(&mut h, 2);
    h.get_by_label_contains("Remove").click();
    steps(&mut h, 2);
    assert!(h.state().online.libraries.is_empty(), "{}", status(&h));
    assert!(h.state().settings().prefs.libraries.is_empty());
    assert!(h.state().library.get(&echo).is_some());
    let _ = std::fs::remove_dir_all(dir);
}
