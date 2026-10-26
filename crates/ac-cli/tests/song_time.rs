//! Song-locked time (DECISIONS D206): generators placed at a beat land where
//! they belong however the transport gets there, so a live seek and a
//! render that starts at the same place (`audioc render --from`) give the
//! same samples.

use std::path::Path;
use std::sync::Arc;

use ac_core::Project;
use ac_engine::{Engine, EngineOptions, HostIo, RenderOptions, render};
use ac_io::{AssetStore, FileResources};

/// A placed drum loop, a placed riff into a saw, a placed curve, and a
/// placed step pattern on a clock, all into one output.
const SONG: &str = r#"{
  "schema_version": 2,
  "name": "song",
  "settings": { "sample_rate": 48000, "length": 8.0, "block_size": 128 },
  "graph": {
    "nodes": [
      { "id": "1", "kind": "clip", "config": { "file": "examples/assets/drums.wav", "channels": 2 },
        "params": { "start": "at", "at": 4.0, "loop": true } },
      { "id": "2", "kind": "midi_pattern",
        "config": { "length": 4, "notes": [[0, 1, 60, 100], [1, 1, 64, 100], [2, 2, 67, 100]] },
        "params": { "start": "at", "at": 2.0 } },
      { "id": "3", "kind": "midi_note" },
      { "id": "4", "kind": "oscillator", "params": { "shape": "saw" } },
      { "id": "5", "kind": "adsr" },
      { "id": "6", "kind": "math", "params": { "op": "multiply" } },
      { "id": "7", "kind": "graph", "config": { "unit": "beats", "points": [[0, 0], [1, 0.2]] },
        "params": { "start": "at", "at": 1.0, "duration": 8.0 } },
      { "id": "8", "kind": "clock", "params": { "interval": 0.25 } },
      { "id": "9", "kind": "pattern", "config": { "steps": 8, "rows": 1, "cells": ["x..x..x."] },
        "params": { "start": "at", "at": 0.5 } },
      { "id": "a", "kind": "math", "params": { "op": "multiply", "a": 0.1 } },
      { "id": "b", "kind": "mix", "config": { "inputs": 4 } },
      { "id": "c", "kind": "audio_out" }
    ],
    "wires": [
      { "from": "2:midi", "to": "3:midi" },
      { "from": "3:freq", "to": "4:freq" },
      { "from": "3:gate", "to": "5:gate" },
      { "from": "4:out", "to": "6:a" },
      { "from": "5:out", "to": "6:b" },
      { "from": "8:trigger", "to": "9:clock" },
      { "from": "9:gate_0", "to": "a:b" },
      { "from": "1:out", "to": "b:in_0" },
      { "from": "6:out", "to": "b:in_1" },
      { "from": "7:out", "to": "b:in_2" },
      { "from": "a:out", "to": "b:in_3" },
      { "from": "b:out", "to": "c:in" }
    ]
  }
}"#;

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn setup() -> (Project, Arc<ac_core::Registry>, Arc<FileResources>) {
    let project = Project::from_json(SONG).expect("the song parses");
    let registry = Arc::new(ac_nodes::registry());
    let assets = AssetStore::load_for(&project, root(), &registry);
    assert!(assets.issues.is_empty(), "{:?}", assets.issues);
    let res = Arc::new(FileResources {
        assets,
        recorder: None,
        plugins: None,
    });
    (project, registry, res)
}

/// Plays `blocks` blocks of the engine, appending channel 0 and 1 to `out`.
fn play(engine: &mut Engine, blocks: usize, out: &mut [Vec<f32>; 2]) {
    let mut buf = vec![0.0f32; 2 * BLOCK];
    for _ in 0..blocks {
        let mut io = HostIo {
            frames: BLOCK,
            input: &[],
            in_channels: 0,
            output: &mut buf,
            out_channels: 2,
            midi: &[],
            midi_out: None,
            controls: &[],
            osc_out: None,
        };
        engine.process(&mut io);
        out[0].extend_from_slice(&buf[..BLOCK]);
        out[1].extend_from_slice(&buf[BLOCK..]);
    }
}

#[test]
fn a_live_seek_and_a_render_from_there_agree() {
    let (project, registry, res) = setup();
    // 5 s is beat 10: every placed generator is under way.
    let from = 5.0;
    let seconds = 2.0;
    let rendered = render(
        &project,
        registry.clone(),
        res.clone(),
        &RenderOptions {
            from,
            to: Some(from + seconds),
            block_size: Some(BLOCK),
            ..RenderOptions::default()
        },
    );
    assert!(!rendered.report.has_errors(), "{}", rendered.report);
    assert!(ac_core::testing::peak(&rendered.channels[0]) > 0.05);

    let (mut engine, report) = Engine::new(
        project,
        registry,
        res,
        EngineOptions {
            sample_rate: RATE,
            max_block: BLOCK,
            socket_telemetry: false,
            offline: true,
        },
    );
    assert!(!report.has_errors(), "{report}");
    // Play a second and a half from the top, then jump.
    engine.handle.play();
    let mut before = [Vec::new(), Vec::new()];
    play(&mut engine, (1.5 * RATE) as usize / BLOCK, &mut before);
    engine.handle.seek((from * RATE) as u64);
    let mut after = [Vec::new(), Vec::new()];
    play(&mut engine, (seconds * RATE) as usize / BLOCK, &mut after);
    for (c, live) in after.iter().enumerate() {
        assert_eq!(live[..], rendered.channels[c][..live.len()], "channel {c}");
    }
}

#[test]
fn placed_generators_are_silent_before_their_beats() {
    let (project, registry, res) = setup();
    let rendered = render(
        &project,
        registry,
        res,
        &RenderOptions {
            to: Some(0.25),
            block_size: Some(BLOCK),
            ..RenderOptions::default()
        },
    );
    // Beat 0.5 is 0.25 s: nothing has started yet.
    assert_eq!(ac_core::testing::peak(&rendered.channels[0]), 0.0);
}
