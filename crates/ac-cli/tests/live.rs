//! Smoke tests for `audioc devices`, `audioc run`, and `audioc latency`
//! against the built binary. Debug builds of `audioc` abort on any
//! allocation in the real-time callbacks, so these double as hygiene tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LIVE_PROJECT: &str = r#"{
  "schema_version": 1,
  "name": "smoke",
  "graph": {
    "nodes": [
      { "id": "1", "kind": "audio_in", "config": { "channels": 2 } },
      { "id": "2", "kind": "record", "config": { "channels": 2 }, "ui": { "pos": [0, 0], "label": "Mic" } },
      { "id": "3", "kind": "audio_out", "config": { "channels": 2 } }
    ],
    "wires": [
      { "from": "1:out", "to": "2:in" },
      { "from": "1:out", "to": "3:in" }
    ]
  }
}
"#;

fn audioc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(args)
        .output()
        .expect("audioc runs")
}

fn text(o: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// A fresh directory with the smoke project in it.
fn project(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("audioc-live-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("smoke.acproj");
    std::fs::write(&p, LIVE_PROJECT).unwrap();
    (dir, p)
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn devices_lists_even_without_hardware() {
    let o = audioc(&["devices"]);
    let (out, err) = text(&o);
    assert!(o.status.success(), "{out}\n{err}");
    assert!(out.contains("MIDI inputs"), "{out}");
}

#[test]
fn run_null_plays_in_real_time_and_stops() {
    let (dir, p) = project("play");
    let t0 = std::time::Instant::now();
    let o = audioc(&[
        "run",
        path(&p),
        "--null",
        "--duration",
        "1.3",
        "--midi",
        "none",
    ]);
    let (out, err) = text(&o);
    assert!(o.status.success(), "{out}\n{err}");
    assert!(t0.elapsed().as_secs_f64() >= 1.2, "paced to real time");
    assert!(out.contains("audio: null"), "{out}");
    assert!(out.contains("playing"), "{out}");
    assert!(out.contains("pos 0:01."), "{out}");
    assert!(out.contains("xruns 0"), "{out}");
    // Nothing was recording: no takes.
    assert!(!dir.join("assets").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_null_no_play_leaves_the_transport_stopped() {
    let (dir, p) = project("noplay");
    let o = audioc(&["run", path(&p), "--null", "--no-play", "--duration", "1.1"]);
    let (out, err) = text(&o);
    assert!(o.status.success(), "{out}\n{err}");
    assert!(out.contains("stopped"), "{out}");
    assert!(out.contains("pos 0:00.000"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_null_record_writes_a_take_into_assets() {
    let (dir, p) = project("record");
    let o = audioc(&[
        "run",
        path(&p),
        "--null",
        "--record",
        "--duration",
        "1",
        "--sample-rate",
        "44100",
        "--buffer-size",
        "128",
    ]);
    let (out, err) = text(&o);
    assert!(o.status.success(), "{out}\n{err}");
    assert!(out.contains("recorded"), "{out}");
    let take = dir.join("assets").join("Mic-001.wav");
    let wav = ac_io::read_wav(&take).unwrap();
    assert_eq!(wav.sample_rate, 44_100);
    assert_eq!(wav.channels.len(), 2);
    let secs = wav.frames() as f64 / 44_100.0;
    assert!((0.8..1.6).contains(&secs), "{secs} s");
    // Silent input records silence.
    assert!(wav.channels.iter().flatten().all(|&s| s == 0.0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_on_a_missing_device_suggests_null() {
    let (dir, p) = project("nodevice");
    let o = audioc(&[
        "run",
        path(&p),
        "--output-device",
        "no-such-device-anywhere",
        "--duration",
        "0.5",
    ]);
    let (out, err) = text(&o);
    assert!(!o.status.success(), "{out}\n{err}");
    assert!(err.contains("--null"), "{err}");
    let o = audioc(&["run", path(&p), "--host", "no-such-host"]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("no audio host named `no-such-host`"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn latency_on_a_missing_device_fails_cleanly() {
    let o = audioc(&["latency", "--output-device", "no-such-device-anywhere"]);
    let (out, err) = text(&o);
    assert!(!o.status.success(), "{out}\n{err}");
    assert!(err.contains("no-such-device-anywhere"), "{err}");
}

#[test]
fn run_rejects_a_missing_project() {
    let o = audioc(&["run", "/definitely/not/here.acproj", "--null"]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("here.acproj"), "{err}");
}
