//! The M3.4 nodes inside the engine: Convolution's latency lines dry paths
//! up, Time Stretch bakes from known sources and is disabled by live ones,
//! and the new streaming nodes run inside a single-sample loop. Every
//! `Processor::process` here runs under the allocation guard.

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use serde_json::{Value as Json, json};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f64 = 48_000.0;

/// Resources holding one impulse response for every file.
struct Ir(Arc<AudioData>);

impl Resources for Ir {
    fn audio_asset(&self, _: &str) -> Option<Arc<AudioData>> {
        Some(self.0.clone())
    }
}

fn node(id: u64, kind: &str, config: Json) -> NodeInstance {
    NodeInstance::new(NodeId(id), kind).with_config(config)
}

fn wire(p: &mut Project, from: u64, out: &str, to: u64, inp: &str) {
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(from), out),
        Endpoint::new(NodeId(to), inp),
    ));
}

fn render_with(p: &Project, resources: Arc<dyn Resources>, block: usize) -> Rendered {
    render(
        p,
        Arc::new(ac_nodes::registry()),
        resources,
        &RenderOptions {
            block_size: Some(block),
            ..Default::default()
        },
    )
}

#[test]
fn convolution_latency_lines_up_a_dry_path() {
    // An impulse, dry and through a response that is a single click,
    // summed: with the dry path delayed to match, both arrive together.
    let mut click = vec![0.0; 64];
    click[0] = 1.0;
    let ir = Arc::new(Ir(Arc::new(AudioData {
        sample_rate: 48_000,
        channels: vec![click],
    })));
    let mut p = Project::default();
    p.settings.length = 0.05;
    p.graph.nodes.extend([
        node(1, "impulse", json!({})),
        node(
            2,
            "convolution",
            json!({ "file": "ir.wav", "channels": 1, "partition": 512 }),
        ),
        node(3, "mix", json!({ "inputs": 2 })),
        node(4, "audio_out", json!({})),
    ]);
    wire(&mut p, 1, "out", 2, "in");
    wire(&mut p, 1, "out", 3, "in_0");
    wire(&mut p, 2, "out", 3, "in_1");
    wire(&mut p, 3, "out", 4, "in");
    for block in [1, 64, 1024] {
        let r = render_with(&p, ir.clone(), block);
        assert_eq!(r.report.stats.latency, 512, "{}", r.report);
        let x = &r.channels[0];
        let hits: Vec<usize> = (0..x.len()).filter(|&i| x[i].abs() > 1e-6).collect();
        assert_eq!(hits, vec![512], "block {block}");
        assert!((x[512] - 2.0).abs() < 1e-5, "{}", x[512]);
    }
}

#[test]
fn time_stretch_bakes_from_a_known_source_and_waits_for_a_live_one() {
    let stretch = |source: &str| {
        let mut p = Project::default();
        p.settings.length = 0.5;
        p.graph.nodes.extend([
            node(1, source, json!({})),
            node(2, "time_stretch", json!({})),
            node(3, "audio_out", json!({})),
        ]);
        if let Some(n) = p.graph.node_mut(NodeId(2)) {
            n.params.insert("stretch".into(), Value::Float(2.0));
        }
        wire(&mut p, 1, "out", 2, "in");
        wire(&mut p, 2, "out", 3, "in");
        p
    };
    // Offline, an oscillator's sine comes out at its pitch.
    let r = render_with(&stretch("oscillator"), Arc::new(NoResources), 256);
    let x = &r.channels[0][4_800..19_200];
    let rms = (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    assert!(rms > 0.3, "{rms}");
    assert!(
        r.report
            .stats
            .whole_input
            .iter()
            .any(|(p, _)| *p == NodePath::root(NodeId(2)))
    );
    // Live, fed by Audio In, it's disabled and says why.
    let (engine, report) = Engine::new(
        stretch("audio_in"),
        Arc::new(ac_nodes::registry()),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: RATE,
            max_block: 256,
            socket_telemetry: false,
            offline: false,
        },
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.node == Some(NodePath::root(NodeId(2))) && d.message.contains("live")),
        "{report}"
    );
    assert!(
        matches!(
            report.stats.whole_input.first(),
            Some((_, WholeState::Disabled { .. }))
        ),
        "{:?}",
        report.stats.whole_input
    );
    drop(engine);
}

#[test]
fn the_new_streaming_nodes_run_in_a_single_sample_loop() {
    // Impulse → Mix → Formant → Gain (−3 dB) → Feedback (sample) → Mix,
    // with Chaos moving the vowel: a talking resonator, the same at every
    // block size.
    let mut p = Project::default();
    p.settings.length = 0.2;
    p.graph.nodes.extend([
        node(1, "impulse", json!({})),
        node(2, "mix", json!({ "inputs": 2 })),
        node(3, "formant", json!({})),
        node(4, "gain", json!({})),
        node(5, "feedback", json!({ "delay": "sample" })),
        node(6, "audio_out", json!({})),
        node(7, "chaos", json!({})),
        node(8, "map", json!({})),
    ]);
    if let Some(n) = p.graph.node_mut(NodeId(4)) {
        n.params.insert("gain".into(), Value::Float(-3.0));
    }
    for (k, v) in [
        ("in_min", -1.0),
        ("in_max", 1.0),
        ("out_min", 0.0),
        ("out_max", 4.0),
    ] {
        if let Some(n) = p.graph.node_mut(NodeId(8)) {
            n.params.insert(k.into(), Value::Float(v));
        }
    }
    wire(&mut p, 1, "out", 2, "in_0");
    wire(&mut p, 2, "out", 3, "in");
    wire(&mut p, 3, "out", 4, "in");
    wire(&mut p, 4, "out", 5, "in");
    wire(&mut p, 5, "out", 2, "in_1");
    wire(&mut p, 2, "out", 6, "in");
    wire(&mut p, 7, "x", 8, "in");
    wire(&mut p, 8, "out", 3, "vowel");
    let one = render_with(&p, Arc::new(NoResources), 1);
    assert_eq!(one.report.stats.sample_loops.len(), 1, "{}", one.report);
    assert_eq!(one.report.stats.sample_loops[0].len(), 4);
    assert!(one.channels[0].iter().all(|v| v.is_finite()));
    assert!(one.channels[0][100..].iter().any(|v| v.abs() > 1e-3));
    for block in [64, 1024] {
        assert!(render_with(&p, Arc::new(NoResources), block).channels == one.channels);
    }
    // A Convolution in the loop isn't refused (it works sample by sample),
    // but a Spectral Freeze is: it needs FFT frames.
    let mut q = p.clone();
    q.graph.nodes.push(node(9, "fft", json!({})));
    q.graph.nodes.push(node(10, "spectral_freeze", json!({})));
    q.graph.nodes.push(node(11, "ifft", json!({})));
    q.graph
        .wires
        .retain(|w| !(w.from.node == NodeId(4) && w.to.node == NodeId(5)));
    wire(&mut q, 4, "out", 9, "in");
    wire(&mut q, 9, "spectrum", 10, "spectrum");
    wire(&mut q, 10, "spectrum", 11, "spectrum");
    wire(&mut q, 11, "out", 5, "in");
    let r = render_with(&q, Arc::new(NoResources), 64);
    assert!(r.report.stats.sample_loops.is_empty());
    assert!(
        r.report.diagnostics.iter().any(|d| d.node == Some(NodePath::root(NodeId(10)))
            && d.message.contains("FFT frames")),
        "{}",
        r.report
    );
}
