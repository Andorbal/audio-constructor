//! Blocks and sharing in the editor, without a window (egui_kittest):
//! adding a library block from the palette, saving a group to the library
//! and updating older copies, opening packs and block files, and
//! exporting a pack. The library lives in memory unless a test gives it a
//! folder.

use std::path::{Path, PathBuf};

use ac_app::actions::Action;
use ac_app::audio::AudioMode;
use ac_app::dialogs::FilePurpose;
use ac_app::palette::{self, Candidate};
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp};
use ac_core::{
    Block, BlockSource, Endpoint, GROUP_KIND, GroupDef, GroupId, GroupInput, GroupOutput, NodeId,
    NodeInstance, Origin, Project, SignalType, Unit, VOICES_KIND,
};
use egui::{Event, Key, Modifiers, Pos2};
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
