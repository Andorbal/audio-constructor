//! Every shipped example opens, is in canonical form, compiles without
//! errors, carries the shipped blocks as they ship, and makes sound when it
//! doesn't depend on a live input. The Reverse Delay example also checks
//! its Phase 2 acceptance criterion (docs/ROADMAP.md).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{Block, BlockSource, Endpoint, NodeId, NodePath, Origin, Project, Wire};
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

fn render_project(project: &Project, path: &Path, seconds: f64) -> ac_engine::Rendered {
    let registry = Arc::new(ac_nodes::registry());
    let res = resources(project, path, &registry);
    render(
        project,
        registry,
        res,
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
