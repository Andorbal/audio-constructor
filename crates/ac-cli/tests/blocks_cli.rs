//! `audioc blocks`, `block-update`, `pack`, and `unpack`, run as a person
//! would run them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ac_core::{GroupDef, GroupId, NodeId, NodeInstance, Project};
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
    let d = std::env::temp_dir().join(format!("ac-blocks-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn blocks_lists_the_shipped_library() {
    let o = audioc(&["blocks"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("shipped:tremolo"), "{}", text(&o));
    let o = audioc(&["blocks", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let trem = v
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["source"] == "shipped:tremolo")
        .unwrap();
    assert_eq!(trem["inputs"], json!(["in", "rate", "depth"]));
}

#[test]
fn block_update_brings_old_copies_up_to_date() {
    let dir = temp("update");
    let mut p = Project::default();
    let mut old = GroupDef::new("Tremolo");
    old.version = 0;
    old.source = Some("shipped:tremolo".into());
    p.group_defs.insert(GroupId("t".into()), old);
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "group").with_config(json!({"def": "t"})));
    let path = dir.join("p.acproj");
    ac_io::save_project(&path, &p).unwrap();
    let o = audioc(&["block-update", s(&path), "--check"]);
    assert!(!o.status.success(), "--check reports stale copies");
    assert!(text(&o).contains("shipped:tremolo v0 → v"), "{}", text(&o));
    assert_eq!(
        ac_io::load_project(&path).unwrap(),
        p,
        "--check writes nothing"
    );
    let o = audioc(&["block-update", s(&path)]);
    assert!(o.status.success(), "{}", text(&o));
    let q = ac_io::load_project(&path).unwrap();
    assert!(!q.group_defs[&GroupId("t".into())].graph.nodes.is_empty());
    let o = audioc(&["block-update", s(&path), "--check"]);
    assert!(o.status.success(), "now current: {}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pack_and_unpack_round_trip_a_project_with_audio() {
    let dir = temp("pack");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    ac_io::write_wav(
        &dir.join("assets/tone.wav"),
        &[vec![0.5; 480]],
        48_000,
        ac_io::BitDepth::Pcm16,
    )
    .unwrap();
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "clip").with_config(json!({"file": "assets/tone.wav"})));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out"));
    let proj = dir.join("song.acproj");
    ac_io::save_project(&proj, &p).unwrap();
    let o = audioc(&["pack", s(&proj)]);
    assert!(o.status.success(), "{}", text(&o));
    let pack = dir.join("song.acpack");
    assert!(pack.exists());
    let out = dir.join("unpacked");
    let o = audioc(&["unpack", s(&pack), "-o", s(&out)]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(ac_io::load_project(&out.join("song.acproj")).unwrap(), p);
    assert!(out.join("assets/tone.wav").exists());
    // Again into the same folder: refused unless forced.
    assert!(
        !audioc(&["unpack", s(&pack), "-o", s(&out)])
            .status
            .success()
    );
    assert!(
        audioc(&["unpack", s(&pack), "-o", s(&out), "--force"])
            .status
            .success()
    );
    // The unpacked project renders.
    let wav = dir.join("out.wav");
    let o = audioc(&["render", s(&out.join("song.acproj")), "-o", s(&wav)]);
    assert!(o.status.success(), "{}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}
