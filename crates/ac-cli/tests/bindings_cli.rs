//! Bindings from the command line: `audioc inspect` lists them and
//! `audioc validate` reports the ones that can't act.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ac_core::{Binding, NodeId, NodeInstance, NodePath, Project, Source, TransportAction};

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

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .join(name)
}

#[test]
fn inspect_lists_bindings() {
    let path = golden("bindings-midi-cc.acproj");
    let o = audioc(&["inspect", path.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("bindings (3):"), "{t}");
    assert!(t.contains("MIDI CC 74 · ch 1 → 1:freq  (110..880)"), "{t}");
    assert!(t.contains("Note C4 · any channel → 1:sync"), "{t}");
}

#[test]
fn validate_reports_bindings_that_cant_act() {
    let dir = std::env::temp_dir().join(format!("ac-bindings-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = Project::default();
    p.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
    p.bindings.push(Binding::input(
        Source::MidiCc {
            channel: None,
            cc: 1,
        },
        NodePath::root(NodeId(2)),
        "gain",
    ));
    p.bindings.push(Binding::transport(
        Source::MidiNote {
            channel: Some(10),
            note: 36,
        },
        TransportAction::PlayStop,
    ));
    let file = dir.join("broken-binding.acproj");
    std::fs::write(&file, p.to_json()).unwrap();
    let o = audioc(&["validate", file.to_str().unwrap()]);
    let t = text(&o);
    // A warning, not an error: the rest of the project runs.
    assert!(o.status.success(), "{t}");
    assert!(t.contains("warning [2:gain]"), "{t}");
    assert!(t.contains("its node isn't in the project"), "{t}");
    let o = audioc(&["inspect", file.to_str().unwrap()]);
    assert!(
        text(&o).contains("Note C2 · ch 10 → transport play / stop"),
        "{}",
        text(&o)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
