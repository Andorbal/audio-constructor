//! Golden renders (ARCHITECTURE §14): every `tests/golden/*.acproj` is
//! rendered offline and compared with its committed `.expected.json`.
//!
//! - On the reference platform (Linux x86-64) the sample hash must match
//!   bit for bit; elsewhere peak and RMS per channel must match within a
//!   small tolerance (floating-point library differences).
//! - Determinism: each project also renders through the null real-time host
//!   at block sizes 32 and 1024 and must equal the offline render at the
//!   same block size. Projects marked `block_independent` must also render
//!   identically at both block sizes.
//!
//! A project may carry test inputs in a top-level `"golden"` object:
//! `{"seconds": 1.0, "block_independent": true, "input": {"sine": [hz, amp], "channels": 1},
//!   "midi": [[frame, [status, d1, d2]], ...]}`.
//!
//! Regenerate expectations with `GOLDEN_UPDATE=1 cargo test -p ac-cli --test golden`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{AudioData, MidiMsg, Project, util::Fnv64};
use ac_engine::{Engine, EngineOptions, NullHost, RenderOptions, render};
use ac_io::{AssetStore, FileResources};
use serde_json::{Value, json};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn projects() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(golden_dir())
        .expect("tests/golden exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "acproj"))
        .collect();
    v.sort();
    v
}

struct Inputs {
    seconds: Option<f64>,
    block_independent: bool,
    input: Option<AudioData>,
    midi: Vec<(u64, MidiMsg)>,
}

fn inputs(project: &Project) -> Inputs {
    let g = project.extra.get("golden").cloned().unwrap_or(Value::Null);
    let rate = project.settings.sample_rate;
    let seconds = g.get("seconds").and_then(Value::as_f64);
    let length = seconds.unwrap_or(project.settings.length);
    let input = g.get("input").map(|i| {
        let sine = i
            .get("sine")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let hz = sine.first().and_then(Value::as_f64).unwrap_or(440.0);
        let amp = sine.get(1).and_then(Value::as_f64).unwrap_or(0.5);
        let channels = i.get("channels").and_then(Value::as_u64).unwrap_or(1) as usize;
        let frames = (length * f64::from(rate)).round() as usize;
        let ch: Vec<f32> = (0..frames)
            .map(|n| {
                (amp * (2.0 * std::f64::consts::PI * hz * n as f64 / f64::from(rate)).sin()) as f32
            })
            .collect();
        AudioData {
            sample_rate: rate,
            channels: vec![ch; channels],
        }
    });
    let midi = g
        .get("midi")
        .and_then(Value::as_array)
        .map(|evs| {
            evs.iter()
                .filter_map(|e| {
                    let at = e.get(0)?.as_u64()?;
                    let bytes: Vec<u8> = e
                        .get(1)?
                        .as_array()?
                        .iter()
                        .filter_map(|b| b.as_u64().map(|b| b as u8))
                        .collect();
                    Some((at, MidiMsg::from_bytes(&bytes)?))
                })
                .collect()
        })
        .unwrap_or_default();
    Inputs {
        seconds,
        block_independent: g
            .get("block_independent")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        input,
        midi,
    }
}

fn hash(channels: &[Vec<f32>]) -> String {
    let mut h = Fnv64::new();
    for ch in channels {
        for s in ch {
            h.write(&s.to_bits().to_le_bytes());
        }
    }
    format!("fnv1a64:{:016x}", h.finish())
}

fn summary(channels: &[Vec<f32>], rate: u32) -> Value {
    json!({
        "sample_rate": rate,
        "frames": channels.first().map_or(0, Vec::len),
        "hash": hash(channels),
        "channels": channels.iter().map(|c| json!({
            "peak": ac_core::testing::peak(c),
            "rms": ac_core::testing::rms(c),
        })).collect::<Vec<_>>(),
    })
}

fn render_with(
    path: &Path,
    block: Option<usize>,
) -> (Project, ac_engine::Rendered, Arc<FileResources>) {
    let registry = Arc::new(ac_nodes::registry());
    let project = ac_io::load_project(path).expect("golden project loads");
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    assert!(
        assets.issues.is_empty(),
        "{}: asset problems {:?}",
        path.display(),
        assets.issues
    );
    let resources = Arc::new(FileResources {
        assets,
        recorder: None,
    });
    let i = inputs(&project);
    let opts = RenderOptions {
        block_size: block,
        to: i.seconds,
        input: i.input,
        midi: i.midi,
        ..RenderOptions::default()
    };
    let out = render(&project, registry, resources.clone(), &opts);
    (project, out, resources)
}

#[test]
fn golden_renders_match() {
    let update = std::env::var_os("GOLDEN_UPDATE").is_some();
    let reference = cfg!(all(target_os = "linux", target_arch = "x86_64"));
    let list = projects();
    assert!(!list.is_empty(), "no golden projects found");
    let mut failures = Vec::new();
    for path in list {
        let (_, out, _) = render_with(&path, None);
        assert!(
            !out.report.has_errors(),
            "{} has compile errors:\n{}",
            path.display(),
            out.report
        );
        let got = summary(&out.channels, out.sample_rate);
        let expected_path = path.with_extension("expected.json");
        if update {
            let text = serde_json::to_string_pretty(&got).expect("json") + "\n";
            std::fs::write(&expected_path, text).expect("write expected");
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&expected_path) else {
            failures.push(format!(
                "{}: missing {}",
                path.display(),
                expected_path.display()
            ));
            continue;
        };
        let want: Value = serde_json::from_str(&text).expect("expected json parses");
        if got["frames"] != want["frames"] || got["sample_rate"] != want["sample_rate"] {
            failures.push(format!("{}: length or rate differs", path.display()));
            continue;
        }
        if reference {
            if got["hash"] != want["hash"] {
                failures.push(format!(
                    "{}: hash {} != expected {}\n  got {}\n  want {}",
                    path.display(),
                    got["hash"],
                    want["hash"],
                    got["channels"],
                    want["channels"]
                ));
            }
        } else {
            let (g, w) = (got["channels"].as_array(), want["channels"].as_array());
            let close = |a: f64, b: f64| (a - b).abs() <= 1e-4 + 1e-3 * b.abs();
            let ok = g.zip(w).is_some_and(|(g, w)| {
                g.len() == w.len()
                    && g.iter().zip(w).all(|(a, b)| {
                        close(
                            a["peak"].as_f64().unwrap_or(0.0),
                            b["peak"].as_f64().unwrap_or(0.0),
                        ) && close(
                            a["rms"].as_f64().unwrap_or(0.0),
                            b["rms"].as_f64().unwrap_or(0.0),
                        )
                    })
            });
            if !ok {
                failures.push(format!("{}: levels differ beyond tolerance", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn golden_renders_are_deterministic_live_and_offline() {
    for path in projects() {
        let mut by_block = Vec::new();
        for block in [32usize, 1024] {
            let (project, off, resources) = render_with(&path, Some(block));
            let i = inputs(&project);
            let rate = project.settings.sample_rate;
            let (engine, _) = Engine::new(
                project,
                Arc::new(ac_nodes::registry()),
                resources,
                EngineOptions {
                    sample_rate: f64::from(rate),
                    max_block: block,
                    socket_telemetry: true,
                },
            );
            let in_ch = i.input.as_ref().map_or(0, |d| d.channels.len());
            let mut host = NullHost::new(engine, in_ch, off.channels.len());
            host.handle().play();
            let total = off.frames();
            let mut live = vec![Vec::with_capacity(total); off.channels.len()];
            let mut done = 0;
            while done < total {
                let n = block.min(total - done);
                let mut input = vec![0.0f32; in_ch * n];
                if let Some(d) = &i.input {
                    for (c, ch) in d.channels.iter().enumerate() {
                        for k in 0..n {
                            input[c * n + k] = ch.get(done + k).copied().unwrap_or(0.0);
                        }
                    }
                }
                let midi: Vec<ac_core::MidiEvent> = i
                    .midi
                    .iter()
                    .filter(|(at, _)| (*at as usize) >= done && (*at as usize) < done + n)
                    .map(|(at, m)| ac_core::MidiEvent::new((*at as usize - done) as u32, *m))
                    .collect();
                let out = host.tick_with(n, &input, &midi).to_vec();
                for (c, ch) in live.iter_mut().enumerate() {
                    ch.extend_from_slice(&out[c * n..(c + 1) * n]);
                }
                done += n;
            }
            assert!(
                off.channels == live,
                "{}: offline and live renders differ at block {block}",
                path.display()
            );
            by_block.push(off.channels);
        }
        if inputs(&ac_io::load_project(&path).expect("loads")).block_independent {
            assert!(
                by_block[0] == by_block[1],
                "{}: marked block_independent but blocks 32 and 1024 differ",
                path.display()
            );
        }
    }
}

#[test]
fn audioc_render_matches_the_library() {
    let path = projects().into_iter().next().expect("a golden project");
    let out_wav = std::env::temp_dir().join(format!("ac-golden-cli-{}.wav", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(["render", "-q", "--bit-depth", "32f", "-o"])
        .arg(&out_wav)
        .arg(&path)
        .status()
        .expect("audioc runs");
    assert!(status.success());
    let wav = ac_io::read_wav(&out_wav).expect("wav reads");
    let (_, lib, _) = render_with(&path, None);
    // The CLI renders the project length; compare the overlapping part.
    for (a, b) in wav.channels.iter().zip(&lib.channels) {
        let n = a.len().min(b.len());
        assert_eq!(&a[..n], &b[..n]);
    }
    std::fs::remove_file(out_wav).ok();
}
