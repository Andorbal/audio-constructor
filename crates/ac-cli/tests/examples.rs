//! Every shipped example opens, is in canonical form, compiles without
//! errors, carries the shipped blocks as they ship, and makes sound when it
//! doesn't depend on a live input. The Reverse Delay example also checks
//! its Phase 2 acceptance criterion (docs/ROADMAP.md).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{
    AudioData, Block, BlockSource, Control, Endpoint, GamepadAxis, GamepadButton, Key, MidiMsg,
    NodeId, NodeInstance, NodePath, Origin, Project, Source, Target, TransportAction, Value, Wire,
};
use ac_engine::{Engine, EngineOptions, HostIo, RenderOptions, WholeState, render};
use ac_io::{AssetStore, FileResources, Library};

/// The examples that must ship (the M2.6 list in docs/ROADMAP.md, plus the
/// Phase 1 ones). Others may exist; they get every check below too.
const EXPECTED: &[&str] = &[
    "start",
    "guitar-pedalboard",
    "subtractive-synth",
    "four-tracks",
    "sidechain-pump",
    "drum-machine",
    "reverse-delay",
    "multiband-compressor",
    "vocoder",
    "generative-patch",
    "tape-loop",
    "guitar-to-synth",
    "build-a-keyboard",
    "gamepad-jam",
    // M3.4: single-sample feedback, Convolution, Chaos, Spectrogram.
    "plucked-strings",
];

/// Examples that only make sound from a live input.
const LIVE_ONLY: &[&str] = &["guitar-pedalboard"];

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn example(name: &str) -> PathBuf {
    examples_dir().join(format!("{name}.acproj"))
}

fn examples() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("examples/ exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "acproj"))
        .collect();
    v.sort();
    v
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn resources(
    project: &Project,
    path: &Path,
    registry: &Arc<ac_core::Registry>,
) -> Arc<FileResources> {
    let assets = AssetStore::load_for(project, path.parent().expect("dir"), registry);
    assert!(
        assets.issues.is_empty(),
        "{}: {:?}",
        path.display(),
        assets.issues
    );
    Arc::new(FileResources {
        assets,
        recorder: None,
    })
}

fn render_with(project: &Project, path: &Path, options: &RenderOptions) -> ac_engine::Rendered {
    let registry = Arc::new(ac_nodes::registry());
    let res = resources(project, path, &registry);
    render(project, registry, res, options)
}

fn render_project(project: &Project, path: &Path, seconds: f64) -> ac_engine::Rendered {
    render_with(
        project,
        path,
        &RenderOptions {
            to: Some(seconds),
            ..RenderOptions::default()
        },
    )
}

fn render_example(path: &Path, seconds: f64) -> ac_engine::Rendered {
    let project = ac_io::load_project(path).expect("example loads");
    render_project(&project, path, seconds)
}

fn peak(channels: &[Vec<f32>]) -> f32 {
    channels
        .iter()
        .map(|c| ac_core::testing::peak(c))
        .fold(0.0, f32::max)
}

/// The node with this label.
fn labeled(project: &Project, label: &str) -> NodeId {
    project
        .graph
        .nodes
        .iter()
        .find(|n| n.ui.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no node labeled {label:?}"))
        .id
}

#[test]
fn the_expected_examples_ship() {
    let found: Vec<String> = examples().iter().map(|p| stem(p)).collect();
    let missing: Vec<&&str> = EXPECTED
        .iter()
        .filter(|e| !found.iter().any(|f| f == *e))
        .collect();
    assert!(missing.is_empty(), "missing examples: {missing:?}");
}

#[test]
fn examples_are_canonical_and_compile_cleanly() {
    for path in examples() {
        let text = std::fs::read_to_string(&path).expect("reads");
        let project = ac_core::Project::from_json(&text).expect("parses");
        assert_eq!(
            project.to_json(),
            text,
            "{} is not canonical (run audioc fmt)",
            path.display()
        );
        let out = render_example(&path, 0.5);
        assert!(
            !out.report.has_errors(),
            "{}:\n{}",
            path.display(),
            out.report
        );
    }
}

/// An example's copy of a shipped block is the library's, exactly, so the
/// example shows the block as it ships and opening it offers no update.
#[test]
fn example_blocks_are_the_shipped_versions() {
    let library = Library::shipped_only();
    let mut problems = Vec::new();
    for path in examples() {
        let project = ac_io::load_project(&path).expect("example loads");
        for (id, def) in &project.group_defs {
            let Some(source) = def.source.as_deref().and_then(BlockSource::parse) else {
                continue;
            };
            if source.origin != Origin::Shipped {
                problems.push(format!("{}: {id} comes from {source}", path.display()));
                continue;
            }
            let Some(lib) = library.get(&source) else {
                problems.push(format!("{}: no shipped block {source}", path.display()));
                continue;
            };
            let copy = Block::from_project(&project, id).expect("definition exists");
            if copy != lib.block {
                problems.push(format!(
                    "{}: its copy of {source} differs from blocks/{}.acblock; paste the block's \
                     definition (and `uses`) into the project again",
                    path.display(),
                    source.id
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn examples_without_live_inputs_make_sound() {
    for path in examples() {
        let name = stem(&path);
        if LIVE_ONLY.contains(&name.as_str()) {
            continue;
        }
        let out = render_example(&path, 4.0);
        let peak = peak(&out.channels);
        assert!(peak > 0.01, "{name} is silent (peak {peak})");
        assert!(peak <= 1.0, "{name} clips (peak {peak})");
    }
}

/// Kinds that have a widget to pin to a performance panel (UX.md "Panels").
const PINNABLE: &[&str] = &[
    "button",
    "toggle",
    "slider",
    "knob",
    "xy_pad",
    "keyboard",
    "number",
    "dropdown",
    "meter",
    "scope",
    "analyzer",
    "readout",
    "clip",
    "record",
    "audio_out",
];

/// Panels pin top-level nodes that have widgets, and bindings drive
/// unwired inputs that exist (a wired input ignores its binding), so
/// what an example shows on its panel and plays from its controls works.
#[test]
fn example_panels_and_bindings_point_at_real_controls() {
    let registry = ac_nodes::registry();
    let mut problems = Vec::new();
    for path in examples() {
        let name = stem(&path);
        let project = ac_io::load_project(&path).expect("example loads");
        for panel in &project.panels {
            assert!(
                !panel.items.is_empty(),
                "{name}: panel {} is empty",
                panel.name
            );
            for item in &panel.items {
                match project.graph.node(item.node) {
                    Some(n) if PINNABLE.contains(&n.kind.as_str()) => {}
                    Some(n) => problems.push(format!(
                        "{name}: panel {} pins a {} (no widget)",
                        panel.name, n.kind
                    )),
                    None => problems.push(format!(
                        "{name}: panel {} pins missing node {}",
                        panel.name, item.node
                    )),
                }
            }
        }
        for b in &project.bindings {
            let Target::Input { node, socket } = &b.target else {
                continue;
            };
            let [id] = node.0.as_slice() else {
                problems.push(format!("{name}: binding into a group ({node})"));
                continue;
            };
            let Some(n) = project.graph.node(*id) else {
                problems.push(format!("{name}: binding to missing node {node}"));
                continue;
            };
            let inputs: Vec<String> = match n.group_def() {
                Some(def) => project.group_defs[&def]
                    .inputs
                    .iter()
                    .map(|i| i.name.clone())
                    .collect(),
                None => registry.get(&n.kind).map_or(Vec::new(), |e| {
                    (e.describe)(&n.config)
                        .inputs
                        .iter()
                        .map(|s| s.name.to_string())
                        .collect()
                }),
            };
            if !inputs.contains(socket) {
                problems.push(format!("{name}: {} has no input {socket}", n.kind));
            }
            let to = Endpoint::new(*id, socket.as_str());
            if project.graph.wire_into(&to).is_some() {
                problems.push(format!("{name}: {node}:{socket} is bound but wired"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn subtractive_synth_cli_render_is_bit_identical() {
    let path = example("subtractive-synth");
    let wav = std::env::temp_dir().join(format!("ac-synth-{}.wav", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(["render", "-q", "--bit-depth", "32f", "--to", "4", "-o"])
        .arg(&wav)
        .arg(&path)
        .status()
        .expect("audioc runs");
    assert!(status.success());
    let cli = ac_io::read_wav(&wav).expect("reads");
    let lib = render_example(&path, 4.0);
    assert_eq!(cli.channels, lib.channels);
    std::fs::remove_file(wav).ok();
}

/// The Generative Patch's random choices come from the project's seed: the
/// same seed plays the same notes every time, another seed other notes.
#[test]
fn generative_patch_follows_the_seed() {
    let path = example("generative-patch");
    let project = ac_io::load_project(&path).expect("loads");
    let a = render_project(&project, &path, 6.0);
    let b = render_project(&project, &path, 6.0);
    assert!(a.channels == b.channels, "two renders differ");
    let mut reseeded = project.clone();
    reseeded.settings.seed += 1;
    let c = render_project(&reseeded, &path, 6.0);
    assert!(a.channels != c.channels, "a new seed plays the same notes");
}

/// `project` with Audio Out fed from `from` instead.
fn listening_to(project: &Project, from: Endpoint) -> Project {
    let mut p = project.clone();
    let out = p
        .graph
        .nodes
        .iter()
        .find(|n| n.kind == "audio_out")
        .expect("an Audio Out")
        .id;
    p.graph.wires.retain(|w| w.to.node != out);
    p.graph
        .wires
        .push(Wire::new(from, Endpoint::new(out, "in")));
    p
}

/// Phase 2 acceptance: "The Reverse Delay example shows the Reverse node
/// disabled when fed from AudioIn and baked when fed from a Clip, and
/// renders correctly offline."
#[test]
fn reverse_delay_bakes_from_a_clip_and_is_disabled_from_audio_in() {
    let path = example("reverse-delay");
    let project = ac_io::load_project(&path).expect("loads");
    let registry = Arc::new(ac_nodes::registry());
    let from_clip = NodePath::root(labeled(&project, "Reverse (from Clip)"));
    let from_in = NodePath::root(labeled(&project, "Reverse (from Audio In)"));
    let block = project.settings.block_size as usize;
    let rate = project.settings.sample_rate;

    // Live: the Clip path bakes, the Audio In path is disabled and says why.
    let (mut engine, report) = Engine::new(
        project.clone(),
        registry.clone(),
        resources(&project, &path, &registry),
        EngineOptions {
            sample_rate: f64::from(rate),
            max_block: block,
            socket_telemetry: false,
            offline: false,
        },
    );
    let state = |e: &Engine, p: &NodePath| {
        e.handle
            .bakes()
            .iter()
            .find(|b| &b.path == p)
            .map(|b| b.state.clone())
    };
    assert_eq!(state(&engine, &from_clip), Some(WholeState::Pending));
    assert_eq!(
        state(&engine, &from_in),
        Some(WholeState::Disabled {
            sources: vec!["Guitar or mic".to_string()]
        })
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.node.as_ref() == Some(&from_in) && d.message.contains("disabled while live")),
        "{report}"
    );
    engine.handle.bake_all();
    assert_eq!(state(&engine, &from_clip), Some(WholeState::Baked));
    assert!(matches!(
        state(&engine, &from_in),
        Some(WholeState::Disabled { .. })
    ));

    // Once baked, live playback is the offline render, sample for sample.
    let seconds = 3.0;
    let offline = render_example(&path, seconds);
    let frames = offline.channels[0].len();
    engine.handle.play_from_start();
    let silence = vec![0.0f32; block];
    let mut out = vec![0.0f32; 2 * block];
    let mut live = Vec::with_capacity(frames);
    while live.len() < frames {
        let mut io = HostIo {
            frames: block,
            input: &silence,
            in_channels: 1,
            output: &mut out,
            out_channels: 2,
            midi: &[],
            midi_out: None,
            controls: &[],
            osc_out: None,
        };
        engine.process(&mut io);
        live.extend_from_slice(&out[..block]);
    }
    live.truncate(frames);
    assert!(live == offline.channels[0], "live and offline differ");
    let level = peak(&offline.channels);
    assert!(level > 0.05 && level <= 1.0, "peak {level}");

    // Offline nothing is live, so both paths bake.
    assert!(
        offline
            .report
            .stats
            .whole_input
            .iter()
            .all(|(_, s)| *s == WholeState::Baked),
        "{:?}",
        offline.report.stats.whole_input
    );

    // And the Clip path really plays backwards, half a second at a time.
    let clip = labeled(&project, "Melody");
    let forward = render_project(
        &listening_to(&project, Endpoint::new(clip, "out")),
        &path,
        seconds,
    );
    let reversed = render_project(
        &listening_to(
            &project,
            Endpoint::new(from_clip.leaf().expect("leaf"), "out"),
        ),
        &path,
        seconds,
    );
    let (fwd, rev) = (&forward.channels[0], &reversed.channels[0]);
    assert!(peak(&forward.channels) > 0.05);
    let slice = (0.5 * f64::from(rate)) as usize;
    for k in 0..frames / slice {
        for t in 0..slice {
            assert_eq!(
                rev[k * slice + t],
                fwd[k * slice + slice - 1 - t],
                "slice {k}, frame {t}"
            );
        }
    }
}

/// Guitar-to-Synth follows the pitch of whatever plays into it: the demo
/// riff by default, and Audio In once Guitar in is switched on.
#[test]
fn guitar_to_synth_follows_the_riff_and_the_guitar() {
    let path = example("guitar-to-synth");
    let project = ac_io::load_project(&path).expect("loads");
    let rate = f64::from(project.settings.sample_rate);
    let detector = labeled(&project, "Pitch Detect");
    let pitch = listening_to(&project, Endpoint::new(detector, "freq"));
    let near = |f: f32, hz: f64| (f64::from(f) / hz - 1.0).abs() < 0.02;

    // The riff: E5 D5 C5 B4 C5 D5 E5 G5, a quarter of a second each.
    let out = render_project(&pitch, &path, 2.0);
    let riff = [659.3, 587.3, 523.3, 493.9, 523.3, 587.3, 659.3, 784.0];
    for (i, hz) in riff.iter().enumerate() {
        let f = out.channels[0][((i as f64 + 0.75) * 0.25 * rate) as usize];
        assert!(near(f, *hz), "note {i}: {f} Hz, expected {hz}");
    }

    // A guitar's G3 into Audio In, with Guitar in on.
    let switch_on = |p: &Project| {
        let mut p = p.clone();
        let toggle = labeled(&p, "Guitar in");
        let node = p.graph.node_mut(toggle).expect("toggle");
        node.params.insert("value".into(), Value::Bool(true));
        p
    };
    let tone: Vec<f32> = (0..rate as usize)
        .map(|i| 0.3 * (std::f64::consts::TAU * 196.0 * i as f64 / rate).sin() as f32)
        .collect();
    let options = RenderOptions {
        to: Some(1.0),
        input: Some(AudioData {
            sample_rate: project.settings.sample_rate,
            channels: vec![tone],
        }),
        ..RenderOptions::default()
    };
    let out = render_with(&switch_on(&pitch), &path, &options);
    let f = out.channels[0][(0.75 * rate) as usize];
    assert!(near(f, 196.0), "{f} Hz from a 196 Hz guitar");
    let out = render_with(&switch_on(&project), &path, &options);
    let level = peak(&out.channels);
    assert!(level > 0.05 && level <= 1.0, "peak {level}");
}

/// `project` with the param `socket` of the node labeled `label` set.
fn set(project: &Project, label: &str, socket: &str, value: Value) -> Project {
    let mut p = project.clone();
    let id = labeled(&p, label);
    let node = p.graph.node_mut(id).expect("node");
    node.params.insert(socket.into(), value);
    p
}

/// Render `seconds` of `project` with control-surface events at the given
/// times (seconds).
fn render_controls(
    project: &Project,
    path: &Path,
    seconds: f64,
    controls: &[(f64, Control, f32)],
) -> ac_engine::Rendered {
    let rate = f64::from(project.settings.sample_rate);
    let options = RenderOptions {
        to: Some(seconds),
        controls: controls
            .iter()
            .map(|(t, c, v)| ((t * rate) as u64, *c, *v))
            .collect(),
        ..RenderOptions::default()
    };
    render_with(project, path, &options)
}

/// A slice of `x` between two times (seconds).
fn between(x: &[f32], rate: f64, from: f64, to: f64) -> &[f32] {
    &x[(from * rate) as usize..(to * rate) as usize]
}

/// Build a Keyboard plays its demo phrase, the piano row of Keyboard In,
/// and the on-screen Keyboard (shifted by Octave) all at once, through a
/// Merge: a note two of them hold sounds once and ends when the last lets
/// go. The Demo toggle mutes the phrase.
#[test]
fn build_a_keyboard_plays_the_demo_the_computer_keys_and_the_on_screen_keys_at_once() {
    let path = example("build-a-keyboard");
    let project = ac_io::load_project(&path).expect("loads");
    let rate = f64::from(project.settings.sample_rate);
    let synth = listening_to(&project, Endpoint::new(labeled(&project, "Synth"), "out"));
    let demo = render_project(&synth, &path, 2.0);
    assert!(peak(&demo.channels) > 0.05, "the demo plays");
    let notes = |played: &[Strum]| -> Vec<(f64, Vec<u8>)> {
        played.iter().map(|(t, n, _)| (*t, n.clone())).collect()
    };
    let tapped = tapping_midi(&synth, "Notes", "out");
    let out = render_project(&tapped, &path, 1.0);
    assert_eq!(
        notes(&chords(&out.midi, rate)),
        vec![
            (0.0, vec![57, 60, 64, 76]),
            (0.6, vec![74]),
            (0.9, vec![72])
        ],
        "the demo's first bar (100 bpm)"
    );

    // With Demo off, the computer keys alone: silent until A and G, which
    // play middle C and the G above it at the Velocity knob's 0.8.
    let solo = set(&synth, "Demo", "value", Value::Bool(false));
    let keys = tapping_midi(&solo, "Notes", "out");
    let key = |name| Control::Key(Key::from_name(name).expect("key"));
    let presses = [
        (0.5, key("A"), 1.0),
        (0.5, key("G"), 1.0),
        (1.5, key("A"), 0.0),
        (1.5, key("G"), 0.0),
    ];
    let out = render_controls(&keys, &path, 2.0, &presses);
    let ch = &out.channels[0];
    assert_eq!(ac_core::testing::peak(between(ch, rate, 0.0, 0.5)), 0.0);
    assert!(ac_core::testing::peak(between(ch, rate, 0.6, 1.4)) > 0.05);
    assert_eq!(chords(&out.midi, rate), vec![(0.5, vec![60, 67], 102)]);
    let silent = render_controls(&keys, &path, 1.0, &[]);
    assert_eq!(peak(&silent.channels), 0.0, "no demo and no keys, no notes");
    assert!(silent.midi.is_empty());

    // The on-screen keyboard alone: key 12 is C4 from the lowest C3, an
    // octave up C5.
    let board = set(&solo, "On-screen keyboard", "key_12", Value::Bool(true));
    let board = set(&board, "Octave", "value", Value::Int(1));
    let out = render_project(&tapping_midi(&board, "Notes", "out"), &path, 1.0);
    assert!(ac_core::testing::peak(between(&out.channels[0], rate, 0.1, 0.9)) > 0.05);
    assert_eq!(chords(&out.midi, rate), vec![(0.0, vec![72], 102)]);

    // All three at once. The demo's chord holds middle C until beat 3.8
    // (2.28 s), so pressing A adds nothing and releasing it ends nothing;
    // the on-screen C5 is held throughout, so the demo's C5 at 0.9 s
    // neither strikes it again nor ends it.
    let all = set(&synth, "On-screen keyboard", "key_12", Value::Bool(true));
    let all = tapping_midi(&set(&all, "Octave", "value", Value::Int(1)), "Notes", "out");
    let out = render_controls(&all, &path, 2.35, &presses);
    assert_eq!(
        notes(&chords(&out.midi, rate)),
        vec![
            (0.0, vec![57, 60, 64, 72, 76]),
            (0.5, vec![67]),
            (0.6, vec![74]),
            (1.2, vec![74]),
        ]
    );
    let offs = |note: u8| -> Vec<f64> {
        out.midi
            .iter()
            .filter(|(_, m)| matches!(m, MidiMsg::NoteOff { note: n, .. } if *n == note))
            .map(|(t, _)| (*t as f64 / rate * 100.0).round() / 100.0)
            .collect()
    };
    assert_eq!(offs(60), vec![2.28], "middle C ends with the demo's chord");
    assert_eq!(offs(67), vec![1.5], "G ends with its key");
    assert!(offs(72).is_empty(), "C5 rings while its key is held");

    // The Play panel holds the keyboard and the Demo toggle.
    let panel = project
        .panels
        .iter()
        .find(|p| p.name == "Play")
        .expect("a Play panel");
    for label in [
        "On-screen keyboard",
        "Demo",
        "Octave",
        "Velocity",
        "Cutoff (Hz)",
    ] {
        assert!(
            panel.contains(labeled(&project, label)),
            "{label} isn't pinned"
        );
    }
}

/// `project` with a MIDI Out fed from output `socket` of the node labeled
/// `label`, so a render reports the notes passing there.
fn tapping_midi(project: &Project, label: &str, socket: &str) -> Project {
    let mut p = project.clone();
    let from = labeled(&p, label);
    let id = NodeId(p.graph.ids().last().map_or(1, |n| n.0 + 1));
    p.graph.nodes.push(NodeInstance::new(id, "midi_out"));
    p.graph
        .connect(Endpoint::new(from, socket), Endpoint::new(id, "midi"));
    p
}

/// A chord: when it starts (seconds, to 10 ms), its notes in order, and
/// its first note's velocity.
type Strum = (f64, Vec<u8>, u8);

/// Note-ons in `midi`, grouped by when they start.
fn chords(midi: &[(u64, MidiMsg)], rate: f64) -> Vec<Strum> {
    let mut out: Vec<Strum> = Vec::new();
    for (t, m) in midi {
        if let MidiMsg::NoteOn { note, velocity, .. } = m
            && *velocity > 0
        {
            let at = (*t as f64 / rate * 100.0).round() / 100.0;
            match out.last_mut() {
                Some((s, notes, _)) if *s == at => notes.push(*note),
                _ => out.push((at, vec![*note], *velocity)),
            }
        }
    }
    for (_, notes, _) in &mut out {
        notes.sort_unstable();
    }
    out
}

/// Gamepad Jam: the demo strums Am, F, C, G, one a bar, and plays the drums;
/// Select on the controller takes over (a binding flips the Gamepad switch),
/// D-pad right strums the next chord and D-pad left starts over, the right
/// trigger strums harder, the left stick moves the Cutoff knob, and A plays
/// the kick.
#[test]
fn gamepad_jam_plays_the_demo_and_follows_the_controller() {
    let path = example("gamepad-jam");
    let project = ac_io::load_project(&path).expect("loads");
    let rate = f64::from(project.settings.sample_rate);
    let tapped = tapping_midi(&project, "In A minor", "midi");
    let am = vec![57, 60, 64];
    let f = vec![53, 57, 60];
    let c = vec![48, 52, 55];
    let g = vec![55, 59, 62];
    let notes = |played: &[Strum]| -> Vec<(f64, Vec<u8>)> {
        played.iter().map(|(t, n, _)| (*t, n.clone())).collect()
    };

    // The demo: a chord at the top of every bar (120 bpm: every 2 s).
    let out = render_project(&tapped, &path, 8.0);
    assert_eq!(
        notes(&chords(&out.midi, rate)),
        vec![(0.0, am.clone()), (2.0, f.clone()), (4.0, c), (6.0, g)]
    );

    // The controller takes over at 1 s; the demo's chord at 2 s is gone.
    let button = |b| Control::GamepadButton { pad: 0, button: b };
    let press = |t: f64, b| [(t, button(b), 1.0), (t + 0.05, button(b), 0.0)];
    let mut controls = Vec::new();
    controls.extend(press(1.0, GamepadButton::Select));
    let right_trigger = Control::GamepadAxis {
        pad: 0,
        axis: GamepadAxis::RightTrigger,
    };
    controls.push((2.9, right_trigger, 1.0));
    controls.extend(press(3.0, GamepadButton::DpadRight));
    controls.extend(press(4.0, GamepadButton::DpadLeft));
    controls.extend(press(5.0, GamepadButton::DpadRight));
    let out = render_controls(&tapped, &path, 7.0, &controls);
    let played = chords(&out.midi, rate);
    assert_eq!(notes(&played), vec![(0.0, am.clone()), (3.0, f), (5.0, am)]);
    assert!(played[0].2 < 100, "the demo strums at 0.7");
    assert_eq!(played[1].2, 127, "the right trigger pulled all the way");

    // The left stick is bound to the Cutoff knob: all the way right is its top.
    let knob = listening_to(
        &project,
        Endpoint::new(labeled(&project, "Cutoff (Hz)"), "out"),
    );
    let stick = Control::GamepadAxis {
        pad: 0,
        axis: GamepadAxis::LeftX,
    };
    let out = render_controls(&knob, &path, 1.0, &[(0.5, stick, 1.0)]);
    let ch = &out.channels[0];
    assert!(
        (ch[(0.25 * rate) as usize] - 2500.0).abs() < 1.0,
        "the knob's own value"
    );
    assert!(
        (ch[(0.75 * rate) as usize] - 12000.0).abs() < 1.0,
        "the stick's"
    );

    // With the controller playing, nothing sounds until A plays the kick.
    let drums = listening_to(&project, Endpoint::new(labeled(&project, "Drums"), "out"));
    let on = set(&drums, "Controller", "value", Value::Bool(true));
    let out = render_controls(&on, &path, 1.5, &press(0.5, GamepadButton::South));
    let ch = &out.channels[0];
    assert_eq!(ac_core::testing::peak(between(ch, rate, 0.0, 0.5)), 0.0);
    assert!(ac_core::testing::peak(between(ch, rate, 0.5, 0.8)) > 0.05);

    // Start plays and stops on any controller, as the Devices tab's preset
    // would on A (which is the kick here).
    assert!(project.bindings.iter().any(|b| {
        b.source
            == Source::GamepadButton {
                pad: None,
                button: GamepadButton::Start,
            }
            && b.target
                == Target::Transport {
                    action: TransportAction::PlayStop,
                }
    }));
    let panel = project
        .panels
        .iter()
        .find(|p| p.name == "Jam")
        .expect("a Jam panel");
    for label in ["Controller", "Cutoff (Hz)", "Resonance"] {
        assert!(
            panel.contains(labeled(&project, label)),
            "{label} isn't pinned"
        );
    }
}

#[test]
fn plucked_strings_runs_its_string_sample_by_sample() {
    let path = example("plucked-strings");
    let project = ac_io::load_project(&path).expect("example loads");
    let at = |block: usize| {
        render_with(
            &project,
            &path,
            &RenderOptions {
                to: Some(4.0),
                block_size: Some(block),
                ..RenderOptions::default()
            },
        )
    };
    let r = at(64);
    // The string's loop (Mix, Delay, Damping, Sustain, Feedback) runs one
    // sample at a time, and the hall's partition lines up the dry path.
    let loops = &r.report.stats.sample_loops;
    assert_eq!(loops.len(), 1, "{}", r.report);
    assert_eq!(loops[0].len(), 5);
    assert_eq!(r.report.stats.latency, 1024);
    let p = peak(&r.channels);
    assert!(p > 0.2 && p < 1.0, "peak {p}");
    // So it plays the same at any block size.
    assert!(at(1024).channels == r.channels);
}
