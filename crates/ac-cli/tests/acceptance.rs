//! The parts of the Phase 1 acceptance criteria that don't need hardware,
//! run end to end through the real nodes, engine, null host, disk recorder,
//! and project files (docs/ROADMAP.md, "Phase 1 acceptance").

use std::sync::Arc;
use std::time::{Duration, Instant};

use ac_core::{Endpoint, NodeId, NodeInstance, Project, Wire};
use ac_engine::{Engine, EngineOptions, NullHost, RenderOptions, render};
use ac_io::{AssetStore, DiskRecorder, FileResources};
use serde_json::json;

const RATE: u32 = 48_000;

fn signal(frames: usize) -> Vec<f32> {
    // A deterministic, non-periodic test signal: two tones plus a little noise.
    let mut s = 0x1234_5678u32;
    (0..frames)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let t = i as f32 / RATE as f32;
            0.4 * (std::f32::consts::TAU * 220.0 * t).sin()
                + 0.2 * (std::f32::consts::TAU * 331.0 * t).sin()
                + 0.05 * (s as f32 / u32::MAX as f32 - 0.5)
        })
        .collect()
}

fn wire(p: &mut Project, from: (u64, &str), to: (u64, &str)) {
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(from.0), from.1),
        Endpoint::new(NodeId(to.0), to.1),
    ));
}

/// "Record 30 s, play back from a Clip, save, close, reopen, same result."
#[test]
fn record_thirty_seconds_play_back_from_a_clip_after_save_and_reopen() {
    let dir = std::env::temp_dir().join(format!("ac-accept-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = Arc::new(ac_nodes::registry());
    let seconds = 30usize;
    let input = signal(seconds * RATE as usize);

    // 1. Record: Audio In → Record (armed), transport recording, through the
    //    live code path (null host = a device clocked by the test).
    let mut rec = Project::default();
    rec.graph.nodes.push(
        NodeInstance::new(NodeId(1), "audio_in").with_config(json!({"channel": 0, "channels": 1})),
    );
    rec.graph.nodes.push(
        NodeInstance::new(NodeId(2), "record")
            .with_config(json!({"channels": 1}))
            .with_label("Take"),
    );
    wire(&mut rec, (1, "out"), (2, "in"));
    let recorder = Arc::new(DiskRecorder::new(&dir).unwrap());
    let resources = Arc::new(FileResources {
        assets: AssetStore::new(&dir),
        recorder: Some(recorder.clone()),
        plugins: None,
    });
    let (engine, report) = Engine::new(
        rec,
        registry.clone(),
        resources,
        EngineOptions {
            sample_rate: f64::from(RATE),
            max_block: 256,
            socket_telemetry: true,
            offline: false,
        },
    );
    assert!(!report.has_errors(), "{report}");
    let mut host = NullHost::new(engine, 1, 2);
    host.handle().set_recording(true);
    host.handle().play();
    // Feed one second at a time and give the disk thread a moment between
    // seconds: still ~20x faster than real time, but within what its 5 s
    // buffer is designed for (a device delivers audio in real time).
    for second in input.chunks(RATE as usize) {
        for chunk in second.chunks(256) {
            host.tick_with(chunk.len(), chunk, &[]);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let dropped = host
        .handle()
        .telemetry()
        .node_value(&ac_core::NodePath::root(NodeId(2)), 2);
    assert_eq!(dropped, Some(0.0), "the recorder dropped samples");
    host.handle().set_recording(false);
    host.handle().stop();
    host.tick(256);
    let deadline = Instant::now() + Duration::from_secs(20);
    let take = loop {
        if let Some(t) = recorder.finished_takes().into_iter().next() {
            break t;
        }
        assert!(Instant::now() < deadline, "the take never finished");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(take.error.is_none(), "{:?}", take.error);
    assert_eq!(take.frames, input.len() as u64, "take length");
    assert!(
        take.relative_path.starts_with("assets/"),
        "{}",
        take.relative_path
    );

    // 2. Play it back from a Clip, save the project, "close" (drop
    //    everything), reopen it from disk, and render.
    let mut play = Project::default();
    play.settings.length = seconds as f64;
    play.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "clip").with_config(json!({
            "file": take.relative_path,
            "channels": 1,
        })));
    play.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out"));
    wire(&mut play, (1, "out"), (2, "in"));
    let path = dir.join("playback.acproj");
    ac_io::save_project(&path, &play).unwrap();
    drop(play);

    let reopened = ac_io::load_project(&path).unwrap();
    let assets = AssetStore::load_for(&reopened, &dir, &registry);
    assert!(assets.issues.is_empty(), "{:?}", assets.issues);
    let out = render(
        &reopened,
        registry,
        Arc::new(FileResources {
            assets,
            recorder: None,
            plugins: None,
        }),
        &RenderOptions::default(),
    );
    assert!(!out.report.has_errors(), "{}", out.report);
    assert_eq!(out.channels[0].len(), input.len());
    // Same result: the recorded input comes back sample for sample.
    assert!(
        out.channels[0] == input,
        "playback differs from what was recorded"
    );
    assert!(
        out.channels[1] == input,
        "mono clip upmixes to both outputs"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// "Change the drum machine's pattern while it plays and it plays the new
/// pattern, in time" (issue #14). Editing a cell rebuilds the Pattern node;
/// the new one takes over from the old, so it neither waits for the next
/// transport start nor starts over from its first step.
#[test]
fn a_pattern_edited_while_playing_plays_on_in_time() {
    let registry = Arc::new(ac_nodes::registry());
    let example = ac_io::load_project(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/drum-machine.acproj"
    )))
    .unwrap();
    let def = ac_core::GroupId::from("drum-machine");
    let mut p = Project {
        settings: example.settings.clone(),
        ..Project::default()
    };
    p.group_defs
        .insert(def.clone(), example.group_defs[&def].clone());
    p.graph.nodes.push(
        NodeInstance::new(NodeId(1), ac_core::GROUP_KIND)
            .with_config(json!({ "def": "drum-machine" })),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out"));
    wire(&mut p, (1, "out"), (2, "in"));
    // The same project with the kick on every beat.
    let mut edited = p.clone();
    let pattern = edited
        .group_defs
        .get_mut(&def)
        .unwrap()
        .graph
        .nodes
        .iter_mut()
        .find(|n| n.kind == "pattern")
        .unwrap();
    pattern.config["cells"][0] = json!("x...x...x...x...");

    let block = 256;
    let (engine, report) = Engine::new(
        p,
        registry.clone(),
        Arc::new(ac_core::NoResources),
        EngineOptions {
            sample_rate: f64::from(RATE),
            max_block: block,
            socket_telemetry: false,
            offline: false,
        },
    );
    assert!(!report.has_errors(), "{report}");
    let mut host = NullHost::new(engine, 0, 2);
    host.handle().play();
    // Edit a little way into the second bar (a bar is 2.18 s at 110 BPM).
    let before = 3 * RATE as usize;
    host.run(before, block);
    let report = host
        .handle()
        .edit(ac_engine::GraphEdit::Replace(Box::new(edited.clone())));
    assert!(!report.has_errors(), "{report}");
    let after = host.run(4 * RATE as usize, block);
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    assert!(
        rms(&after[0]) > 0.01,
        "the pattern went quiet after the edit"
    );

    // In time: once what played before the edit has died away, the live
    // output is the edited project's, sample for sample, as rendered from
    // the start.
    let fresh = render(
        &edited,
        registry,
        Arc::new(ac_core::NoResources),
        &RenderOptions {
            block_size: Some(block),
            to: Some(7.0),
            ..Default::default()
        },
    );
    assert!(!fresh.report.has_errors(), "{}", fresh.report);
    let from = 2 * RATE as usize;
    let live = &after[0][from..];
    let reference = &fresh.channels[0][before + from..before + after[0].len()];
    let worst = live
        .iter()
        .zip(reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(rms(reference) > 0.01);
    assert!(worst < 1e-3, "out of step with the edited pattern: {worst}");
}
