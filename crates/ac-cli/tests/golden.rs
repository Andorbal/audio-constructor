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
//!   "midi": [[frame, [status, d1, d2]], ...],
//!   "controls": [[frame, {"key": "A"}, 1], [frame, {"gamepad": 0, "button": "south"}, 1],
//!                [frame, {"gamepad": 0, "axis": "left_x"}, -0.5],
//!                [frame, {"osc": "/fader/1", "arg": 0}, 0.25], ...]}`.
//! Controls name keys as `Key::name` spells them, gamepad buttons and axes
//! by their socket names (`gamepad` defaults to 0), and OSC by address
//! (`arg` defaults to 0). OSC Out messages are compared like MIDI Out.
//!
//! Regenerate expectations with `GOLDEN_UPDATE=1 cargo test -p ac-cli --test golden`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{
    AudioData, Control, GamepadAxis, GamepadButton, Key, MidiMsg, Project, osc_address_hash,
    util::Fnv64,
};
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
    controls: Vec<(u64, Control, f32)>,
}

/// A test control: `{"key": "A"}`, `{"gamepad": 0, "button": "south"}`,
/// `{"gamepad": 0, "axis": "left_x"}`, or `{"osc": "/fader/1", "arg": 0}`.
fn control(v: &Value) -> Option<Control> {
    if let Some(k) = v.get("key") {
        return Key::from_name(k.as_str()?).map(Control::Key);
    }
    if let Some(a) = v.get("osc") {
        let arg = v.get("arg").and_then(Value::as_u64).unwrap_or(0);
        return Some(Control::Osc {
            address: osc_address_hash(a.as_str()?),
            arg: u8::try_from(arg).ok()?,
        });
    }
    let pad = u8::try_from(v.get("gamepad").and_then(Value::as_u64).unwrap_or(0)).ok()?;
    if let Some(b) = v.get("button").and_then(Value::as_str) {
        let button = GamepadButton::ALL.into_iter().find(|x| x.name() == b)?;
        return Some(Control::GamepadButton { pad, button });
    }
    let a = v.get("axis").and_then(Value::as_str)?;
    let axis = GamepadAxis::ALL.into_iter().find(|x| x.name() == a)?;
    Some(Control::GamepadAxis { pad, axis })
}

/// The control events in `[done, done + n)`, as one callback's events.
fn block_controls(
    events: &[(u64, Control, f32)],
    done: usize,
    n: usize,
) -> Vec<ac_core::ControlEvent> {
    let mut v: Vec<ac_core::ControlEvent> = events
        .iter()
        .filter(|(at, _, _)| (*at as usize) >= done && (*at as usize) < done + n)
        .map(|&(at, c, x)| ac_core::ControlEvent::new((at as usize - done) as u32, c, x))
        .collect();
    // Stable, like the render: events on one frame keep their order.
    v.sort_by_key(|e| e.offset);
    v
}

/// `[[frame, control, value], ...]`; malformed entries fail the test.
fn controls(g: &Value) -> Vec<(u64, Control, f32)> {
    let Some(list) = g.get("controls").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|e| {
            let parsed = (|| {
                let at = e.get(0)?.as_u64()?;
                let c = control(e.get(1)?)?;
                let value = e.get(2)?.as_f64()? as f32;
                Some((at, c, value))
            })();
            parsed.unwrap_or_else(|| panic!("bad golden control {e}"))
        })
        .collect()
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
        controls: controls(&g),
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

/// The render's summary, plus its File Out files and MIDI Out messages
/// when it has any.
fn render_summary(out: &ac_engine::Rendered) -> Value {
    let mut v = summary(&out.channels, out.sample_rate);
    if !out.files.is_empty() {
        v["files"] = out
            .files
            .iter()
            .map(|f| (f.path.clone(), summary(&f.channels, f.sample_rate)))
            .collect::<serde_json::Map<_, _>>()
            .into();
    }
    if !out.midi.is_empty() {
        let mut h = Fnv64::new();
        for (at, msg) in &out.midi {
            let (bytes, len) = msg.to_bytes();
            h.write(&at.to_le_bytes());
            h.write(&bytes[..len]);
        }
        v["midi"] = json!({
            "messages": out.midi.len(),
            "hash": format!("fnv1a64:{:016x}", h.finish()),
        });
    }
    if !out.osc.is_empty() {
        let mut h = Fnv64::new();
        for (at, m) in &out.osc {
            h.write(&at.to_le_bytes());
            h.write(m.target.to_string().as_bytes());
            h.write(m.address().as_bytes());
            for a in m.args() {
                h.write(&a.to_bits().to_le_bytes());
            }
        }
        v["osc"] = json!({
            "messages": out.osc.len(),
            "hash": format!("fnv1a64:{:016x}", h.finish()),
        });
    }
    v
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
        plugins: None,
    });
    let i = inputs(&project);
    let opts = RenderOptions {
        block_size: block,
        to: i.seconds,
        input: i.input,
        midi: i.midi,
        controls: i.controls,
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
        let got = render_summary(&out);
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
        // MIDI and OSC are exact everywhere; files are compared like the
        // main mix.
        if got["midi"] != want["midi"] {
            failures.push(format!(
                "{}: MIDI Out sent {} != expected {}",
                path.display(),
                got["midi"],
                want["midi"]
            ));
        }
        if got["osc"] != want["osc"] {
            failures.push(format!(
                "{}: OSC Out sent {} != expected {}",
                path.display(),
                got["osc"],
                want["osc"]
            ));
        }
        let file_names = |v: &Value| -> Vec<String> {
            v["files"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        };
        if file_names(&got) != file_names(&want) {
            failures.push(format!("{}: File Out files differ", path.display()));
        } else if file_names(&got).iter().any(|f| {
            let (g, w) = (&got["files"][f], &want["files"][f]);
            g["frames"] != w["frames"] || (reference && g["hash"] != w["hash"])
        }) {
            failures.push(format!(
                "{}: File Out renders differ\n  got {}\n  want {}",
                path.display(),
                got["files"],
                want["files"]
            ));
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
                failures.push(format!(
                    "{}: levels differ beyond tolerance",
                    path.display()
                ));
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
                    offline: false,
                },
            );
            let in_ch = i.input.as_ref().map_or(0, |d| d.channels.len());
            let mut host = NullHost::new(engine, in_ch, off.channels.len());
            // Whole-input nodes play their bake; a live host bakes in the
            // background first.
            host.handle().bake_all();
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
                let controls = block_controls(&i.controls, done, n);
                let out = host
                    .tick_with_controls(n, &input, &midi, &controls)
                    .to_vec();
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
            assert_eq!(
                off.midi,
                host.take_sent_midi(),
                "{}: offline and live MIDI Out differ at block {block}",
                path.display()
            );
            assert_eq!(
                off.osc,
                host.take_sent_osc(),
                "{}: offline and live OSC Out differ at block {block}",
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
    // The CLI can't feed a golden's test MIDI or input: use one without.
    let path = projects()
        .into_iter()
        .find(|p| {
            let i = inputs(&ac_io::load_project(p).expect("loads"));
            i.midi.is_empty() && i.input.is_none() && i.controls.is_empty()
        })
        .expect("a golden project without test inputs");
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

/// Every shipped block (`blocks/*.acblock`) has a golden project,
/// `tests/golden/block-<id>.acproj`, that plays an exact copy of it (and of
/// the definitions it uses), so the goldens test the blocks as shipped.
#[test]
fn every_shipped_block_has_a_golden_project_with_its_current_copy() {
    let mut problems = Vec::new();
    for lib in ac_io::shipped_blocks() {
        let id = &lib.source.id;
        let path = golden_dir().join(format!("block-{id}.acproj"));
        let Ok(project) = ac_io::load_project(&path) else {
            problems.push(format!("{}: missing or unreadable", path.display()));
            continue;
        };
        let source = lib.source.to_string();
        let Some((def_id, _)) = project
            .group_defs
            .iter()
            .find(|(_, d)| d.source.as_deref() == Some(source.as_str()))
        else {
            problems.push(format!("{}: no definition from {source}", path.display()));
            continue;
        };
        let copy = ac_core::Block::from_project(&project, def_id).expect("definition exists");
        if copy != lib.block {
            problems.push(format!(
                "{}: its copy of {source} differs from blocks/{id}.acblock; paste the block's \
                 definition (and `uses`) into the project again",
                path.display()
            ));
        }
        let used = project
            .graph
            .nodes
            .iter()
            .any(|n| n.group_def().as_ref() == Some(def_id));
        if !used {
            problems.push(format!("{}: doesn't play the block", path.display()));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The `controls` test input names every kind of control, and a project
/// carrying them renders the same offline and through the null host
/// (golden projects for the control-surface nodes build on this).
#[test]
fn golden_control_inputs_parse_and_play_live_and_offline() {
    let path = projects()
        .into_iter()
        .find(|p| {
            let i = inputs(&ac_io::load_project(p).expect("loads"));
            i.midi.is_empty() && i.input.is_none()
        })
        .expect("a golden project without test inputs");
    let mut project = ac_io::load_project(&path).expect("loads");
    let g = project
        .extra
        .entry("golden".to_string())
        .or_insert_with(|| json!({}));
    g["controls"] = json!([
        [480, {"key": "space"}, 1],
        [0, {"key": "A"}, 1],
        [96, {"gamepad": 1, "button": "south"}, 1],
        [96, {"axis": "right_trigger_axis"}, 0.5],
        [2000, {"osc": "/fader/1"}, 0.25],
        [2000, {"osc": "/xy", "arg": 1}, -1]
    ]);
    let i = inputs(&project);
    assert_eq!(
        i.controls,
        vec![
            (480, Control::Key(Key::from_name("Space").unwrap()), 1.0),
            (0, Control::Key(Key::from_name("A").unwrap()), 1.0),
            (
                96,
                Control::GamepadButton {
                    pad: 1,
                    button: GamepadButton::South
                },
                1.0
            ),
            (
                96,
                Control::GamepadAxis {
                    pad: 0,
                    axis: GamepadAxis::RightTrigger
                },
                0.5
            ),
            (
                2000,
                Control::Osc {
                    address: osc_address_hash("/fader/1"),
                    arg: 0
                },
                0.25
            ),
            (
                2000,
                Control::Osc {
                    address: osc_address_hash("/xy"),
                    arg: 1
                },
                -1.0
            ),
        ]
    );
    // Split into callbacks of 64: rebased, and events on one frame in order.
    let second = block_controls(&i.controls, 64, 64);
    assert_eq!(second.len(), 2);
    assert_eq!((second[0].offset, second[1].offset), (32, 32));
    assert!(matches!(second[0].control, Control::GamepadButton { .. }));
    // Offline and live agree with the events fed in (nothing in this
    // project reads them; the plumbing must not disturb it either).
    let registry = Arc::new(ac_nodes::registry());
    let resources = Arc::new(FileResources {
        assets: AssetStore::load_for(&project, path.parent().expect("dir"), &registry),
        recorder: None,
        plugins: None,
    });
    let block = 64;
    let off = render(
        &project,
        registry.clone(),
        resources.clone(),
        &RenderOptions {
            block_size: Some(block),
            to: Some(0.1),
            controls: i.controls.clone(),
            ..RenderOptions::default()
        },
    );
    let (engine, _) = Engine::new(
        project.clone(),
        registry,
        resources,
        EngineOptions {
            sample_rate: f64::from(project.settings.sample_rate),
            max_block: block,
            socket_telemetry: false,
            offline: false,
        },
    );
    let mut host = NullHost::new(engine, 0, off.channels.len());
    host.handle().bake_all();
    host.handle().play();
    let mut live = vec![Vec::new(); off.channels.len()];
    let mut done = 0;
    while done < off.frames() {
        let n = block.min(off.frames() - done);
        let controls = block_controls(&i.controls, done, n);
        let out = host.tick_with_controls(n, &[], &[], &controls).to_vec();
        for (c, ch) in live.iter_mut().enumerate() {
            ch.extend_from_slice(&out[c * n..(c + 1) * n]);
        }
        done += n;
    }
    assert!(off.channels == live);
    assert!(off.osc.is_empty() && host.take_sent_osc().is_empty());
}

#[test]
#[should_panic(expected = "bad golden control")]
fn malformed_golden_controls_fail_loudly() {
    let mut project = Project::default();
    project.extra.insert(
        "golden".into(),
        json!({"controls": [[0, {"key": "NoSuchKey"}, 1]]}),
    );
    let _ = inputs(&project);
}

/// The Clip Slot block launches on the bar and stops on it: launched at
/// 0.3 s at 120 BPM, the clip starts at 2.0 s (bar 2); stopped at 2.5 s,
/// it ends at 4.0 s (bar 3).
#[test]
fn the_clip_slot_block_launches_and_stops_on_the_bar() {
    let (_, out, _) = render_with(&golden_dir().join("block-clip-slot.acproj"), Some(256));
    let ch = &out.channels[0];
    let peak = |a: usize, b: usize| ch[a..b].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert_eq!(peak(0, 95_900), 0.0, "silent until the bar");
    assert!(peak(96_000, 110_000) > 0.1, "playing from 2.0 s");
    assert!(peak(170_000, 191_000) > 0.1, "still playing before 4.0 s");
    assert!(peak(192_200, 240_000) < 1e-3, "stopped at 4.0 s");
}
