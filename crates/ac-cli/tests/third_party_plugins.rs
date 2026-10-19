//! Phase 3 acceptance: "A third-party CLAP synth runs as a node inside a
//! project." CI downloads a pinned release of a synth we didn't write
//! (see the `third-party-plugin` job in `.github/workflows/ci.yml`),
//! checks its SHA-256, and runs this test with:
//!
//! - `AC_THIRD_PARTY_PLUGINS`: the folder holding its bundles (used as
//!   both `CLAP_PATH` and `VST3_PATH`);
//! - `AC_THIRD_PARTY_NAME`: the plugin's name as it reports it;
//! - `AC_THIRD_PARTY_FORMATS`: the formats to play (`clap`, `vst3`, or
//!   both, comma-separated).
//!
//! For each format it scans the folder with `audioc plugins`, puts the
//! plugin in a project as a Plugin node played by a MIDI Pattern, renders
//! it with `audioc render`, and checks the result isn't silent and has the
//! notes' pitch. Without the variables it's skipped (`--ignored` runs it).

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Goertzel magnitude of `f` in `x`.
fn magnitude(x: &[f32], f: f64, rate: f64) -> f64 {
    let k = 2.0 * (std::f64::consts::TAU * f / rate).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for v in x {
        let s0 = f64::from(*v) + k * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - k * s1 * s2).max(0.0).sqrt()
}

/// The strongest frequency from 50 Hz to 2 kHz, to half a hertz.
fn pitch(x: &[f32], rate: f64) -> f64 {
    let coarse = (100..4000)
        .map(|h| f64::from(h) / 2.0)
        .fold((0.0, 0.0), |best, f| {
            let m = magnitude(x, f, rate);
            if m > best.1 { (f, m) } else { best }
        });
    coarse.0
}

#[test]
#[ignore = "needs a downloaded third-party plugin (CI's third-party-plugin job)"]
fn a_third_party_synth_plays_as_a_node() {
    let (Some(dir), Some(name)) = (env("AC_THIRD_PARTY_PLUGINS"), env("AC_THIRD_PARTY_NAME"))
    else {
        eprintln!("skipped: set AC_THIRD_PARTY_PLUGINS and AC_THIRD_PARTY_NAME");
        return;
    };
    let formats = env("AC_THIRD_PARTY_FORMATS").unwrap_or_else(|| "clap".into());
    let work = std::env::temp_dir().join(format!("ac-third-party-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let audioc = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_audioc"));
        c.env("CLAP_PATH", &dir)
            .env("VST3_PATH", &dir)
            .env("AC_PLUGIN_CACHE", work.join("plugins.json"));
        c
    };
    let out = audioc().args(["plugins", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let catalog: Value = serde_json::from_slice(&out.stdout).unwrap();
    println!(
        "{}",
        String::from_utf8_lossy(&audioc().arg("plugins").output().unwrap().stdout)
    );
    for format in formats.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        let plugin = catalog["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name.as_str() && p["format"] == format)
            .cloned()
            .unwrap_or_else(|| panic!("no {format} plugin named {name} in {catalog}"));
        assert_eq!(plugin["midi_in"], true, "{name} takes notes");
        for (note, hz) in [(57u8, 220.0), (69, 440.0)] {
            let project = json!({
                "schema_version": 2,
                "name": format!("{name} ({format})"),
                "settings": { "sample_rate": 48000, "length": 2.0 },
                "graph": {
                    "nodes": [
                        { "id": "1", "kind": "midi_pattern",
                          "config": { "notes": [[0, 2, note, 100]], "length": 4 } },
                        { "id": "2", "kind": "plugin", "config": { "plugin": plugin } },
                        { "id": "3", "kind": "audio_out" }
                    ],
                    "wires": [
                        { "from": "1:midi", "to": "2:midi" },
                        { "from": "2:out", "to": "3:in" }
                    ]
                }
            });
            let file: PathBuf = work.join(format!("{format}-{note}.acproj"));
            std::fs::write(&file, serde_json::to_vec_pretty(&project).unwrap()).unwrap();
            let wav = file.with_extension("wav");
            let out = audioc()
                .arg("render")
                .arg(&file)
                .arg("-o")
                .arg(&wav)
                .args(["--bit-depth", "32f"])
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!("{}{stderr}", String::from_utf8_lossy(&out.stdout));
            assert!(out.status.success(), "{stderr}");
            assert!(!stderr.contains("plays silence"), "{stderr}");
            let audio = ac_io::read_wav(&wav).unwrap();
            // The note is held for one second (two beats at 120 BPM).
            let held = &audio.channels[0][4_800..43_200];
            let level = rms(held);
            let f = pitch(held, 48_000.0);
            println!("{name} ({format}) note {note}: rms {level:.4}, strongest at {f} Hz");
            assert!(level > 0.001, "{name} ({format}) is silent");
            assert!(
                (f - hz).abs() <= 3.0,
                "{name} ({format}) note {note}: strongest at {f} Hz, not {hz}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&work);
}
