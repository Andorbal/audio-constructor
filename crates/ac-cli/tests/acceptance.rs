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
