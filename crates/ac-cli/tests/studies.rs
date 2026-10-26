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
    "pattern",
    "midi_pattern",
    "step_sequencer",
    "graph",
    "arrangement",
    "waveform",
    "midi_monitor",
    "tuner",
    "sample",
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

/// Renders the recorder from `from` to `to` seconds and returns its stems
/// by name.
fn recorder_stems(from: f64, to: f64) -> Vec<(String, Vec<f32>)> {
    let path = studies_dir().join("multitrack-recorder.acproj");
    let project = ac_io::load_project(&path).expect("study loads");
    let registry = Arc::new(ac_nodes::registry());
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    let res = Arc::new(FileResources {
        assets,
        recorder: None,
        plugins: None,
    });
    let r = render(
        &project,
        registry,
        res,
        &RenderOptions {
            from,
            to: Some(to),
            ..RenderOptions::default()
        },
    );
    assert!(!r.report.has_errors(), "{}", r.report);
    r.files
        .into_iter()
        .map(|f| (f.name, f.channels.into_iter().next().unwrap_or_default()))
        .collect()
}

#[test]
fn the_recorder_plays_from_bar_9_as_it_does_from_the_top() {
    // Bar 9 is 16 s in. Every clip is placed in the song, so each track
    // from bar 9 is the same as the same stretch of the whole song.
    let whole = recorder_stems(0.0, 18.0);
    let from9 = recorder_stems(16.0, 18.0);
    let at = (16.0 * 48_000.0) as usize;
    let mut compared = 0;
    for (name, part) in &from9 {
        if !name.contains("Drums") && !name.contains("Bass") && !name.contains("Lead") {
            continue;
        }
        let full = &whole.iter().find(|(n, _)| n == name).expect("same stems").1;
        assert!(
            ac_core::testing::peak(part) > 0.01,
            "{name} plays from bar 9"
        );
        let diff = part
            .iter()
            .zip(&full[at..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-4, "{name} differs by {diff}");
        compared += 1;
    }
    assert_eq!(compared, 3);
}

#[test]
fn the_recorders_click_never_reaches_the_export() {
    let project = ac_io::load_project(&studies_dir().join("multitrack-recorder.acproj"))
        .expect("study loads");
    let outs: Vec<_> = project
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind == "audio_out")
        .collect();
    assert_eq!(outs.len(), 1, "the click is the transport's, not an output");
    assert!(
        !project
            .graph
            .nodes
            .iter()
            .any(|n| n.kind == "compare" || n.kind == "gate_to_trigger"),
        "the arrangement is the clips' places, not logic"
    );
}

/// Renders the pads study for `seconds` with MIDI `events` played in and
/// the demo beat silenced; channel 0.
fn pads_with(events: &[(u64, ac_core::MidiMsg)], seconds: f64) -> Vec<f32> {
    let path = studies_dir().join("pads-and-keys.acproj");
    let project = ac_io::load_project(&path).expect("study loads");
    let registry = Arc::new(ac_nodes::registry());
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    let res = Arc::new(FileResources {
        assets,
        recorder: None,
        plugins: None,
    });
    let demo = ac_core::NodePath::root(ac_core::NodeId(6));
    let r = render(
        &project,
        registry,
        res,
        &RenderOptions {
            to: Some(seconds),
            midi: events.to_vec(),
            set: vec![(demo, "start".into(), ac_core::Value::Text("never".into()))],
            ..RenderOptions::default()
        },
    );
    assert!(!r.report.has_errors(), "{}", r.report);
    r.channels.into_iter().next().unwrap_or_default()
}

#[test]
fn the_pads_play_slices_and_the_kit_with_velocity() {
    let on = |note, velocity| ac_core::MidiMsg::NoteOn {
        channel: 0,
        note,
        velocity,
    };
    let peak = |x: &[f32]| ac_core::testing::peak(x);
    // A slice of the loop, hard and soft: one Slicer, velocity from the note.
    let hard = pads_with(&[(0, on(36, 127))], 0.5);
    let soft = pads_with(&[(0, on(36, 30))], 0.5);
    assert!(peak(&hard) > 0.01, "slice 0 sounds");
    assert!(
        peak(&hard) > 2.0 * peak(&soft),
        "{} vs {}",
        peak(&hard),
        peak(&soft)
    );
    // The snare and the two hats come through Note Split (51, the
    // resampled pad, is silent until something is recorded).
    for note in [48, 49, 50] {
        assert!(
            peak(&pads_with(&[(0, on(note, 110))], 0.5)) > 0.01,
            "note {note}"
        );
    }
}

#[test]
fn the_kit_is_made_of_sampler_nodes_not_chains() {
    let project =
        ac_io::load_project(&studies_dir().join("pads-and-keys.acproj")).expect("study loads");
    let kinds: Vec<&str> = project
        .graph
        .nodes
        .iter()
        .map(|n| n.kind.as_str())
        .collect();
    for gone in ["midi_note", "map", "compare", "switch"] {
        assert!(!kinds.contains(&gone), "no {gone} left in the kit");
    }
    // The pads are one Pads grid; the only Button is Resample's.
    assert_eq!(kinds.iter().filter(|k| **k == "button").count(), 1);
    for needed in ["pads", "sample", "slicer", "note_split", "zones", "route"] {
        assert!(kinds.contains(&needed), "a {needed}");
    }
    let keys = &project.group_defs[&ac_core::GroupId::from("sampled-keys")];
    assert!(keys.graph.nodes.iter().any(|n| n.kind == "zones"));
    assert!(
        project.graph.nodes.len() <= 40,
        "{}",
        project.graph.nodes.len()
    );
    // Phase 4 acceptance: a 4 × 4 bank of pads, lit by the merged MIDI
    // (the demo pattern's notes too), in at most twenty nodes from the
    // players to the pads' Mix.
    let pads = project
        .graph
        .nodes
        .iter()
        .find(|n| n.kind == "pads")
        .unwrap();
    assert_eq!(
        (
            pads.config["rows"].as_u64(),
            pads.config["columns"].as_u64()
        ),
        (Some(4), Some(4))
    );
    let lit = project
        .graph
        .wire_into(&Endpoint::new(pads.id, "lit"))
        .unwrap();
    let players = lit.from.node;
    for feeds in project.graph.wires_to(players) {
        let k = project.graph.node(feeds.from.node).unwrap().kind.as_str();
        assert!(
            ["midi_in", "keyboard_in", "pads", "midi_pattern"].contains(&k),
            "{k}"
        );
    }
    let bank = project
        .graph
        .nodes
        .iter()
        .filter(|n| {
            !matches!(
                n.kind.as_str(),
                "comment"
                    | "audio_out"
                    | "meter"
                    | "button"
                    | "record"
                    | "voices"
                    | "midi_filter"
                    | "group"
            ) && n.ui.label.as_deref() != Some("Master")
                && n.ui.label.as_deref() != Some("Keys players")
                && n.ui.label.as_deref() != Some("Demo riff")
        })
        .count();
    assert!(bank <= 20, "the bank is {bank} nodes");
}

/// ROADMAP M4.5: the pattern study's one Pattern plays bank B in bars 5–8
/// of every eight, switching on the bar line, from the top or from a seek;
/// the chords follow; the hats carry accents.
#[test]
fn the_pattern_study_alternates_its_banks_on_the_bar() {
    let path = |id: u64| ac_core::NodePath(vec![ac_core::NodeId(id)]);
    let (beat, chords) = (path(0x7), path(0x13));
    let bank = |host: &mut ac_engine::NullHost, p: &ac_core::NodePath, k: usize| {
        host.handle().telemetry().node_value(p, k)
    };
    // 128 BPM: a bar is 1.875 s, so bar 5 begins 7.5 s in.
    let bar = |n: f64| (48_000.0 * 1.875 * (n - 1.0)) as usize;
    let mut host = live_study(&studies_dir().join("pattern-studio.acproj"), 256);
    host.handle().play();
    host.run(bar(5.0) - 2_400, 256);
    assert_eq!(bank(&mut host, &beat, 1), Some(0.0), "A in bar 4");
    host.run(4_800, 256);
    assert_eq!(bank(&mut host, &beat, 1), Some(1.0), "B from bar 5");
    assert_eq!(bank(&mut host, &chords, 1), Some(1.0), "the chords too");
    // From bar 9: A again.
    host.handle().seek(bar(9.0) as u64);
    host.run(4_800, 256);
    assert_eq!(bank(&mut host, &beat, 1), Some(0.0));
    let project = ac_io::load_project(&studies_dir().join("pattern-studio.acproj")).unwrap();
    let hats = project.graph.node(ac_core::NodeId(0x7)).unwrap().config["cells"][0][2].clone();
    assert_eq!(hats, "x.7.x.7.x.7.x.8.", "accented hats");
    let g = &project.graph;
    assert!(!g.nodes.iter().any(|n| n.kind == "switch"), "no Switches");
}

/// ROADMAP M4.5: the scenes study's looper is one Looper node, with record,
/// overdub, and undo on buttons, waiting for the bar.
#[test]
fn the_scenes_looper_is_one_node_that_overdubs() {
    let project = ac_io::load_project(&studies_dir().join("scenes.acproj")).unwrap();
    let g = &project.graph;
    let loopers: Vec<_> = g.nodes.iter().filter(|n| n.kind == "looper").collect();
    assert_eq!(loopers.len(), 1);
    let looper = loopers[0].id;
    for input in ["in", "record", "overdub", "undo"] {
        assert!(
            g.wire_into(&Endpoint::new(looper, input)).is_some(),
            "{input} is wired"
        );
    }
    assert_eq!(
        loopers[0].params["quantize"].as_f64(),
        Some(4.0),
        "on the bar"
    );
    assert!(
        !g.nodes.iter().any(|n| n.kind == "record"),
        "no Record and Sampler pair"
    );
    // Phase 4 acceptance: the three-track, two-scene grid is at most twelve
    // nodes besides the clips: a Clip Slot each, the scene and stop buttons.
    let slots: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.group_def().is_some_and(|d| d.0 == "clip-slot"))
        .collect();
    assert_eq!(slots.len(), 6);
    let feeds_a_slot = |id: ac_core::NodeId| {
        g.wires_from(id)
            .any(|w| slots.iter().any(|s| s.id == w.to.node))
    };
    let buttons = g
        .nodes
        .iter()
        .filter(|n| n.kind == "button")
        .filter(|n| {
            feeds_a_slot(n.id)
                || g.wires_from(n.id).any(|w| {
                    g.node(w.to.node)
                        .is_some_and(|m| m.kind == "merge" && feeds_a_slot(m.id))
                })
        })
        .count();
    assert_eq!(buttons, 5, "two scenes and three stops");
    assert!(slots.len() + buttons <= 12);
    // Each slot drives its clip and shows its state on the Session panel.
    for s in &slots {
        let starts = g
            .wires_from(s.id)
            .filter(|w| w.from.socket == "start")
            .filter_map(|w| g.node(w.to.node))
            .all(|n| n.kind == "clip");
        assert!(starts, "{:?} starts a clip", s.ui.label);
        assert!(
            project.panels[0].contains(s.id),
            "{:?} on the panel",
            s.ui.label
        );
    }
}

/// Phase 4 acceptance: the pattern studio is at most twenty nodes, with its
/// grid and piano roll on one panel and a channel strip per track.
#[test]
fn the_pattern_studio_is_twenty_nodes_with_its_editors_on_a_panel() {
    let project = ac_io::load_project(&studies_dir().join("pattern-studio.acproj")).unwrap();
    let g = &project.graph;
    assert!(g.nodes.len() <= 20, "{}", g.nodes.len());
    let beat = project
        .panels
        .iter()
        .find(|p| p.name == "Beat")
        .expect("Beat");
    let kinds: Vec<&str> = beat
        .items
        .iter()
        .filter_map(|i| g.node(i.node))
        .map(|n| n.kind.as_str())
        .collect();
    assert!(kinds.contains(&"pattern") && kinds.contains(&"midi_pattern"));
    let rack = project
        .panels
        .iter()
        .find(|p| p.name == "Rack")
        .expect("Rack");
    assert!(rack.items.iter().all(|i| {
        g.node(i.node)
            .is_some_and(|n| n.group_def().is_some_and(|d| d.0 == "track"))
    }));
    let clock = g
        .nodes
        .iter()
        .find(|n| n.ui.label.as_deref() == Some("Sixteenths"))
        .unwrap();
    assert!(clock.params["swing"].as_f64().unwrap() > 0.0, "swing");
}

/// Phase 4 acceptance: Stop in any study, playing or already stopped, and
/// nothing goes on sounding once the effects' tails have died away.
#[test]
fn stop_silences_every_study_playing_or_not() {
    for path in studies() {
        let name = stem(&path);
        let mut host = live_study(&path, 256);
        host.handle().play();
        let playing = host.run(48_000 * 3, 256);
        assert!(peak(&playing) > 0.01, "{name} plays");
        host.handle().stop();
        // Reverb and delay tails ring on for a while, as they would.
        host.run(48_000 * 4, 256);
        let after = host.run(48_000, 256);
        assert!(
            peak(&after) < 1e-3,
            "{name}: Stop ended it ({})",
            peak(&after)
        );
        host.handle().stop();
        let again = host.run(24_000, 256);
        assert!(peak(&again) < 1e-3, "{name}: still quiet");
    }
}

/// Phase 4 acceptance: the recorder's Mixer panel reads as one strip per
/// track, its Track block on top, with a label over each section.
#[test]
fn the_recorders_mixer_is_a_strip_per_track_under_labels() {
    let project = ac_io::load_project(&studies_dir().join("multitrack-recorder.acproj")).unwrap();
    let g = &project.graph;
    let mixer = project
        .panels
        .iter()
        .find(|p| p.name == "Mixer")
        .expect("Mixer");
    let tracks: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| n.group_def().is_some_and(|d| d.0 == "track"))
        .collect();
    assert_eq!(tracks.len(), 5);
    assert_eq!(mixer.strips.len(), tracks.len(), "a strip per track");
    for strip in &mixer.strips {
        let first = mixer
            .items
            .iter()
            .find(|i| i.strip == Some(strip.id))
            .and_then(|i| g.node(i.node))
            .unwrap_or_else(|| panic!("{} is empty", strip.name));
        assert!(
            first.group_def().is_some_and(|d| d.0 == "track"),
            "{} starts with its Track",
            strip.name
        );
    }
    let labels: Vec<&str> = mixer.labels.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(labels, ["Tracks", "Buses", "Master", "Recording"]);
}
