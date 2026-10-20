//! The Phase 2 acceptance criteria that need neither hardware nor a
//! stopwatch, run end to end through project files, a block library on
//! disk, and `audioc` (docs/ROADMAP.md, "Phase 2 acceptance").
//!
//! The others are checked elsewhere: the Reverse Delay example in
//! `examples.rs` (`reverse_delay_bakes_from_a_clip_and_is_disabled_from_audio_in`),
//! and the CPU budget of a 16-voice synth and the multi-core scaling with
//! `audioc bench`, measured in `bench/README.md`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use ac_core::{
    Block, BlockSource, Endpoint, GroupDef, GroupId, GroupInput, GroupOutput, IdGen, NodeId,
    NodeInstance, Origin, Project, SignalType, Unit, Value, Wire,
};
use ac_engine::{RenderOptions, render};
use ac_io::{AssetStore, FileResources, Library};
use serde_json::json;

fn audioc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(args)
        .output()
        .expect("audioc runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ac-phase2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

fn rms_of(path: &Path, seconds: f64) -> f32 {
    let project = ac_io::load_project(path).expect("loads");
    let registry = Arc::new(ac_nodes::registry());
    let resources = Arc::new(FileResources {
        assets: AssetStore::load_for(&project, path.parent().expect("dir"), &registry),
        recorder: None,
        plugins: None,
    });
    let out = render(
        &project,
        registry,
        resources,
        &RenderOptions {
            to: Some(seconds),
            ..RenderOptions::default()
        },
    );
    assert!(!out.report.has_errors(), "{}", out.report);
    ac_core::testing::rms(&out.channels[0])
}

fn wire(graph: &mut ac_core::Graph, from: (NodeId, &str), to: (NodeId, &str)) {
    graph.wires.push(Wire::new(
        Endpoint::new(from.0, from.1),
        Endpoint::new(to.0, to.1),
    ));
}

fn input(name: &str, ty: SignalType, unit: Unit, targets: Vec<Endpoint>) -> GroupInput {
    GroupInput {
        name: name.into(),
        ty,
        unit,
        default: None,
        range: None,
        targets,
    }
}

/// "Trim", version 1: `in` → a Gain at −6 dB → `out`.
fn trim_v1() -> GroupDef {
    let mut d = GroupDef::new("Trim");
    d.description = "Turns a signal down by 6 dB.".into();
    d.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "gain").with_param("gain", -6.0));
    d.inputs.push(input(
        "in",
        SignalType::Audio,
        Unit::Generic,
        vec![Endpoint::new(NodeId(1), "in")],
    ));
    d.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(1), "out"),
    });
    d
}

/// The same block changed: the Gain at −12 dB, then a lowpass whose cutoff
/// is a new `tone` input.
fn trim_v2(mut d: GroupDef) -> GroupDef {
    d.description = "Turns a signal down by 12 dB, with a tone control.".into();
    if let Some(g) = d.graph.node_mut(NodeId(1)) {
        g.params.insert("gain".into(), Value::Float(-12.0));
    }
    d.graph.nodes.push(
        NodeInstance::new(NodeId(2), "biquad")
            .with_param("mode", "lowpass")
            .with_param("cutoff", 18000.0),
    );
    wire(&mut d.graph, (NodeId(1), "out"), (NodeId(2), "in"));
    d.inputs.push(input(
        "tone",
        SignalType::Float,
        Unit::Hz,
        vec![Endpoint::new(NodeId(2), "cutoff")],
    ));
    d.outputs[0].source = Endpoint::new(NodeId(2), "out");
    d
}

/// A sine at `freq` (node `id`), always running.
fn sine(id: u64, freq: f64) -> NodeInstance {
    NodeInstance::new(NodeId(id), "oscillator")
        .with_param("freq", freq)
        .with_param("shape", "sine")
        .with_param("start", "load")
}

/// "A block exported from one project imports into another and updates its
/// instances when the library copy changes."
///
/// Project A saves its group to a library folder as one of the person's own
/// blocks (as the editor's Save to Library does); project B adds two
/// instances of it from the library (as the Library tab does); the block
/// changes in A and is saved again, which raises its version; and
/// `audioc block-update --library` brings B's instances up to date, which
/// halves its level as the new version says.
#[test]
fn a_block_exported_from_one_project_updates_its_instances_in_another() {
    let dir = temp("block");
    let library = dir.join("library");
    let trim = GroupId("trim".into());

    // Project A: a sine through Trim.
    let mut a = Project::default();
    a.group_defs.insert(trim.clone(), trim_v1());
    a.graph.nodes.push(sine(1, 220.0));
    a.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "group").with_config(json!({ "def": "trim" })));
    a.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "audio_out"));
    wire(&mut a.graph, (NodeId(1), "out"), (NodeId(2), "in"));
    wire(&mut a.graph, (NodeId(2), "out"), (NodeId(3), "in"));

    // Export it: a `.acblock` file in the library, and A's copy remembers it.
    let source = {
        let mut lib = Library::open(&library);
        let block = Block::from_project(&a, &trim).expect("A has Trim");
        let source = lib.save_user(block).expect("saves");
        let def = a.group_defs.get_mut(&trim).expect("def");
        def.source = Some(source.to_string());
        def.version = lib.get(&source).expect("saved").block.def.version;
        source
    };
    assert_eq!(source, BlockSource::new(Origin::User, "trim"));
    assert!(library.join("trim.acblock").exists());
    let a_path = dir.join("a.acproj");
    ac_io::save_project(&a_path, &a).expect("saves A");

    // Project B, another session: two instances of Trim from the library,
    // on two sines, mixed.
    let mut b = Project::default();
    b.graph.nodes.push(sine(1, 220.0));
    b.graph.nodes.push(sine(2, 330.0));
    b.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "mix").with_config(json!({ "inputs": 2 })));
    b.graph
        .nodes
        .push(NodeInstance::new(NodeId(4), "audio_out"));
    wire(&mut b.graph, (NodeId(3), "out"), (NodeId(4), "in"));
    let lib = Library::open(&library);
    assert!(lib.issues.is_empty(), "{:?}", lib.issues);
    let from_library = lib.get(&source).expect("the library has Trim");
    let clip = from_library
        .block
        .instance(&GroupId(ac_core::slug("Trim")), &source, false);
    let mut ids = IdGen::new(7);
    let mut instances = Vec::new();
    for (k, osc) in [NodeId(1), NodeId(2)].into_iter().enumerate() {
        let pasted = clip.paste(
            &mut b.graph,
            &mut b.group_defs,
            &mut ids,
            [0.0, 120.0 * k as f32],
        );
        let id = pasted[0];
        wire(&mut b.graph, (osc, "out"), (id, "in"));
        wire(&mut b.graph, (id, "out"), (NodeId(3), &format!("in_{k}")));
        instances.push(id);
    }
    assert_eq!(b.group_defs.len(), 1, "both instances share one definition");
    let b_path = dir.join("b.acproj");
    ac_io::save_project(&b_path, &b).expect("saves B");
    let before = rms_of(&b_path, 1.0);
    assert!(before > 0.1, "B plays ({before})");
    let o = audioc(&[
        "block-update",
        "--check",
        s(&b_path),
        "--library",
        s(&library),
    ]);
    assert!(o.status.success(), "B starts up to date: {}", text(&o));

    // Change the block in A and save it to the library again.
    let def = a.group_defs.remove(&trim).expect("def");
    a.group_defs.insert(trim.clone(), trim_v2(def));
    {
        let mut lib = Library::open(&library);
        let block = Block::from_project(&a, &trim).expect("A has Trim");
        let source = lib.save_user(block).expect("saves again");
        assert_eq!(lib.get(&source).expect("saved").block.def.version, 2);
    }
    ac_io::save_project(&a_path, &a).expect("saves A");

    // Without the library, `audioc` knows only the shipped blocks.
    let o = audioc(&["block-update", "--check", s(&b_path)]);
    assert!(o.status.success(), "{}", text(&o));
    // With it, B's copy is out of date until updated.
    let o = audioc(&[
        "block-update",
        "--check",
        s(&b_path),
        "--library",
        s(&library),
    ]);
    assert!(!o.status.success(), "the update is reported");
    assert!(text(&o).contains("user:trim v1 → v2"), "{}", text(&o));
    let o = audioc(&["block-update", s(&b_path), "--library", s(&library)]);
    assert!(o.status.success(), "{}", text(&o));
    let o = audioc(&[
        "block-update",
        "--check",
        s(&b_path),
        "--library",
        s(&library),
    ]);
    assert!(o.status.success(), "now current: {}", text(&o));

    // B's instances are the new version, still wired, and 6 dB quieter.
    let updated = ac_io::load_project(&b_path).expect("loads B");
    assert_eq!(updated.group_defs.len(), 1);
    let (id, def) = updated.group_defs.iter().next().expect("one def");
    assert_eq!(def.version, 2);
    assert_eq!(def.source.as_deref(), Some("user:trim"));
    let names: Vec<&str> = def.inputs.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["in", "tone"], "the new version's interface");
    for inst in &instances {
        let node = updated.graph.node(*inst).expect("instance");
        assert_eq!(node.group_def().as_ref(), Some(id));
        assert!(
            updated
                .graph
                .wire_into(&Endpoint::new(*inst, "in"))
                .is_some()
        );
    }
    let after = rms_of(&b_path, 1.0);
    let ratio = after / before;
    assert!(
        (ratio - 0.5).abs() < 0.01,
        "−12 dB instead of −6 dB halves the level: {before} → {after} ({ratio})"
    );
    let _ = std::fs::remove_dir_all(dir);
}
