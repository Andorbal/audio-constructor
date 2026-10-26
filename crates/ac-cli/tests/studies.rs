//! The gap studies in `studies/` (docs/GAPS.md) are real projects: each
//! opens, is in canonical form, compiles without errors, makes sound, carries
//! the shipped blocks as they ship, and pins and binds controls that exist.
//! They measure how far today's nodes get, so they have to keep working as
//! the nodes change.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{Block, BlockSource, Endpoint, Origin, Project, Target};
use ac_engine::{RenderOptions, render};
use ac_io::{AssetStore, FileResources, Library};

/// The studies docs/GAPS.md describes. Others may exist; they get every
/// check below too.
const EXPECTED: &[&str] = &[
    "multitrack-recorder",
    "pattern-studio",
    "pads-and-keys",
    "scenes",
];

fn studies_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../studies")
}

fn studies() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(studies_dir())
        .expect("studies/ exists")
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

fn render_study(path: &Path, seconds: f64) -> ac_engine::Rendered {
    let project = ac_io::load_project(path).expect("study loads");
    let registry = Arc::new(ac_nodes::registry());
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    assert!(
        assets.issues.is_empty(),
        "{}: {:?}",
        path.display(),
        assets.issues
    );
    let res = Arc::new(FileResources {
        assets,
        recorder: None,
        plugins: None,
    });
    render(
        &project,
        registry,
        res,
        &RenderOptions {
            to: Some(seconds),
            ..RenderOptions::default()
        },
    )
}

fn peak(channels: &[Vec<f32>]) -> f32 {
    channels
        .iter()
        .map(|c| ac_core::testing::peak(c))
        .fold(0.0, f32::max)
}

#[test]
fn the_expected_studies_exist() {
    let found: Vec<String> = studies().iter().map(|p| stem(p)).collect();
    let missing: Vec<&&str> = EXPECTED
        .iter()
        .filter(|e| !found.iter().any(|f| f == *e))
        .collect();
    assert!(missing.is_empty(), "missing studies: {missing:?}");
}

#[test]
fn studies_are_canonical_and_compile_cleanly() {
    for path in studies() {
        let text = std::fs::read_to_string(&path).expect("reads");
        let project = Project::from_json(&text).expect("parses");
        assert_eq!(
            project.to_json(),
            text,
            "{} is not canonical (run audioc fmt)",
            path.display()
        );
        let out = render_study(&path, 0.5);
        assert!(
            !out.report.has_errors(),
            "{}:\n{}",
            path.display(),
            out.report
        );
    }
}

/// Every study plays a demo, so a render makes sound and doesn't clip.
#[test]
fn studies_make_sound() {
    for path in studies() {
        let name = stem(&path);
        let out = render_study(&path, 6.0);
        let peak = peak(&out.channels);
        assert!(peak > 0.01, "{name} is silent (peak {peak})");
        assert!(peak <= 1.0, "{name} clips (peak {peak})");
    }
}

#[test]
fn study_blocks_are_the_shipped_versions() {
    let library = Library::shipped_only();
    let mut problems = Vec::new();
    for path in studies() {
        let project = ac_io::load_project(&path).expect("study loads");
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
                    "{}: its copy of {source} differs from blocks/{}.acblock",
                    path.display(),
                    source.id
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
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
    "spectrogram",
    "readout",
    "clip",
    "record",
    "audio_out",
    "light",
    "fader",
    "pad",
    "pads",
    // A group or block instance pins as a card of its sockets (D203).
    "group",
    "voices",
];

#[test]
fn study_panels_and_bindings_point_at_real_controls() {
    let registry = ac_nodes::registry();
    let mut problems = Vec::new();
    for path in studies() {
        let name = stem(&path);
        let project = ac_io::load_project(&path).expect("study loads");
        assert!(
            !project.panels.is_empty(),
            "{name}: every study has a panel"
        );
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

/// A study running live on the null host (silent input), with its files.
fn live_study(path: &Path, block: usize) -> ac_engine::NullHost {
    let project = ac_io::load_project(path).expect("study loads");
    let registry = Arc::new(ac_nodes::registry());
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    let res = Arc::new(FileResources {
        assets,
        recorder: None,
        plugins: None,
    });
    let (engine, report) = ac_engine::Engine::new(
        project,
        registry,
        res,
        ac_engine::EngineOptions {
            sample_rate: 48_000.0,
            max_block: block,
            socket_telemetry: false,
            offline: false,
        },
    );
    assert!(!report.has_errors(), "{}", path.display());
    ac_engine::NullHost::new(engine, 2, 2)
}

/// ROADMAP M4.1: in the launcher study every clip's `stop` is wired (the
/// slot logic needs it), and transport Stop still ends the loops, and does
/// again with the transport already stopped.
#[test]
fn stop_ends_the_scene_loops_playing_or_not() {
    let mut host = live_study(&studies_dir().join("scenes.acproj"), 256);
    host.handle().play();
    // The demo arms scene 1 at the start; it launches on bar 2 (2 s in).
    let playing = host.run(48_000 * 3, 256);
    assert!(
        peak(&[playing[0][96_000..].to_vec()]) > 0.01,
        "scene 1 plays"
    );
    host.handle().stop();
    // A block for the stop, then nothing but a filter's last ring.
    host.run(4_800, 256);
    let after = host.run(24_000, 256);
    assert!(
        peak(&after) < 1e-3,
        "Stop ended the loops ({})",
        peak(&after)
    );
    host.handle().stop();
    let again = host.run(24_000, 256);
    assert!(peak(&again) < 1e-3);
}

/// ROADMAP M4.1: the recorder study's input monitor starts off, so a
/// microphone is quiet until someone turns it on.
#[test]
fn the_recorders_input_monitor_starts_off() {
    let project = ac_io::load_project(&studies_dir().join("multitrack-recorder.acproj"))
        .expect("study loads");
    let monitor = project
        .graph
        .nodes
        .iter()
        .find(|n| n.ui.label.as_deref() == Some("Monitor input"))
        .expect("the monitor toggle");
    assert_eq!(
        monitor.params.get("value").and_then(|v| v.as_bool()),
        Some(false)
    );
}
