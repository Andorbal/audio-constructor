//! Extra outputs of an offline render: `File Out` files and `MIDI Out`
//! messages, through the engine and through `audioc render`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use ac_core::{Endpoint, MidiMsg, NodeId, NodeInstance, Project, Wire};
use ac_engine::{RenderOptions, render};
use serde_json::json;

fn wire(p: &mut Project, from: (u64, &str), to: (u64, &str)) {
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(from.0), from.1),
        Endpoint::new(NodeId(to.0), to.1),
    ));
}

/// Oscillator → Audio Out, Oscillator → File Out "tone", Oscillator → Gain
/// → File Out "stems/quiet" (mono); MIDI In → MIDI Out.
fn project() -> Project {
    let mut p = Project::default();
    p.settings.length = 0.25;
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 440.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out"));
    p.graph.nodes.push(
        NodeInstance::new(NodeId(3), "file_out")
            .with_config(json!({"path": "tone"}))
            .with_label("Tone"),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(4), "gain").with_param("gain", -6.0));
    p.graph.nodes.push(
        NodeInstance::new(NodeId(5), "file_out")
            .with_config(json!({"path": "stems/quiet.wav", "channels": 1})),
    );
    p.graph.nodes.push(NodeInstance::new(NodeId(6), "midi_in"));
    p.graph.nodes.push(NodeInstance::new(NodeId(7), "midi_out"));
    wire(&mut p, (1, "out"), (2, "in"));
    wire(&mut p, (1, "out"), (3, "in"));
    wire(&mut p, (1, "out"), (4, "in"));
    wire(&mut p, (4, "out"), (5, "in"));
    wire(&mut p, (6, "midi"), (7, "midi"));
    p
}

fn note(on: bool) -> MidiMsg {
    if on {
        MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        }
    } else {
        MidiMsg::NoteOff {
            channel: 0,
            note: 60,
            velocity: 64,
        }
    }
}

#[test]
fn file_out_nodes_render_alongside_the_main_output() {
    let out = render(
        &project(),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        &RenderOptions {
            midi: vec![(100, note(true)), (5_000, note(false))],
            ..RenderOptions::default()
        },
    );
    assert!(!out.report.has_errors(), "{}", out.report);
    assert_eq!(out.files.len(), 2);
    let tone = &out.files[0];
    assert_eq!(tone.path, "tone");
    assert_eq!(tone.name, "Tone");
    assert_eq!(tone.sample_rate, 48_000);
    // Sample for sample the same as the main mix.
    assert_eq!(tone.channels, out.channels);
    let quiet = &out.files[1];
    assert_eq!(quiet.channels.len(), 1);
    assert_eq!(quiet.channels[0].len(), out.frames());
    let ratio =
        ac_core::testing::peak(&quiet.channels[0]) / ac_core::testing::peak(&tone.channels[0]);
    assert!((ratio - 0.501).abs() < 0.01, "{ratio}");
    // MIDI Out passed the input through, timing intact.
    assert_eq!(out.midi, [(100, note(true)), (5_000, note(false))]);
}

#[test]
fn a_render_from_later_files_the_same_span() {
    let out = render(
        &project(),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        &RenderOptions {
            from: 0.1,
            ..RenderOptions::default()
        },
    );
    assert_eq!(out.files[0].channels, out.channels);
    assert_eq!(out.frames(), 7_200);
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ac-render-out-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn audioc_render_writes_file_out_files_next_to_the_output() {
    let dir = temp_dir("cli");
    let mut p = project();
    // One that tries to escape the output folder is refused.
    p.graph.nodes.push(
        NodeInstance::new(NodeId(8), "file_out")
            .with_config(json!({"path": "../escape.wav"}))
            .with_label("Escape"),
    );
    wire(&mut p, (1, "out"), (8, "in"));
    let proj = dir.join("p.acproj");
    ac_io::save_project(&proj, &p).unwrap();
    let out_dir = dir.join("renders");
    std::fs::create_dir_all(&out_dir).unwrap();
    let main = out_dir.join("mix.wav");
    let mid = dir.join("notes.mid");
    let o = Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args([
            "render",
            proj.to_str().unwrap(),
            "-o",
            main.to_str().unwrap(),
        ])
        .args(["--midi-out", mid.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{stdout}\n{stderr}");
    let tone = ac_io::read_wav(&out_dir.join("tone.wav")).unwrap();
    let mix = ac_io::read_wav(&main).unwrap();
    assert_eq!(tone.channels, mix.channels);
    let quiet = ac_io::read_wav(&out_dir.join("stems/quiet.wav")).unwrap();
    assert_eq!(quiet.channels.len(), 1);
    assert!(!dir.join("escape.wav").exists());
    assert!(stderr.contains("Escape"), "{stderr}");
    assert!(stdout.contains("File Out \"Tone\""), "{stdout}");
    // No MIDI came in, so the file holds only the tempo map.
    let bytes = std::fs::read(&mid).unwrap();
    assert_eq!(&bytes[..4], b"MThd");
}
