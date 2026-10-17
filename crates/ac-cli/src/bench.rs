//! `audioc bench`: render stress projects as fast as possible and report the
//! real-time ratio overall and the cost per node kind (ARCHITECTURE §14).
//!
//! The stress projects are generated in code so they stay in step with the
//! node library without committing thousand-node files. `--project` benches
//! any project file instead. `--json` prints machine-readable results;
//! `--compare <file>` fails when a project got slower than the given results
//! by more than `--threshold` (CI compares a merge request against `main`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use ac_core::{Endpoint, NoResources, NodeId, NodeInstance, NodePath, Project, Resources, Wire};
use ac_engine::{Engine, EngineOptions, HostIo};
use anyhow::{Result, anyhow};
use clap::Args;
use serde_json::{Value, json};

#[derive(Debug, Args)]
pub struct BenchArgs {
    /// Bench this project instead of the built-in stress projects.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Seconds of audio to render per project.
    #[arg(long, default_value = "10")]
    seconds: f64,
    /// Block size.
    #[arg(long, default_value = "128")]
    block_size: usize,
    /// Print results as JSON.
    #[arg(long)]
    json: bool,
    /// Compare with earlier JSON results and fail on regressions.
    #[arg(long, value_name = "FILE")]
    compare: Option<PathBuf>,
    /// Allowed slowdown before --compare fails (0.2 = 20 %).
    #[arg(long, default_value = "0.2")]
    threshold: f64,
    /// Append the results as one JSON line to this history file.
    #[arg(long, value_name = "FILE")]
    history: Option<PathBuf>,
}

fn now_ns() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Result of benching one project.
struct Outcome {
    name: String,
    /// Seconds of audio per second of CPU time.
    realtime: f64,
    nodes: usize,
    blocks: u64,
    /// Total nanoseconds per node kind.
    per_kind: BTreeMap<String, (u64, usize)>,
}

pub fn run(args: BenchArgs) -> Result<ExitCode> {
    let registry = Arc::new(ac_nodes::registry());
    let mut projects: Vec<(String, Project, Arc<dyn Resources>)> = Vec::new();
    match &args.project {
        Some(p) => {
            let (project, res) = crate::util::load(p, &registry)?;
            let name = p
                .file_stem()
                .map_or("project".into(), |s| s.to_string_lossy().to_string());
            projects.push((name, project, res));
        }
        None => {
            for (name, project) in stress_projects() {
                projects.push((name, project, Arc::new(NoResources)));
            }
        }
    }
    let mut results = Vec::new();
    for (name, project, resources) in projects {
        results.push(bench_one(
            &name,
            project,
            registry.clone(),
            resources,
            args.seconds,
            args.block_size,
        ));
    }
    let json = results_json(&results, args.block_size);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        print_table(&results);
    }
    if let Some(h) = &args.history {
        append_history(h, &json)?;
    }
    if let Some(c) = &args.compare {
        return compare(&json, c, args.threshold);
    }
    Ok(ExitCode::SUCCESS)
}

fn bench_one(
    name: &str,
    project: Project,
    registry: Arc<ac_core::Registry>,
    resources: Arc<dyn Resources>,
    seconds: f64,
    block: usize,
) -> Outcome {
    let rate = f64::from(project.settings.sample_rate);
    let kinds: BTreeMap<NodePath, String> = project
        .graph
        .nodes
        .iter()
        .map(|n| (NodePath::root(n.id), n.kind.clone()))
        .collect();
    let (mut engine, _) = Engine::new(
        project,
        registry,
        resources,
        EngineOptions {
            sample_rate: rate,
            max_block: block,
            socket_telemetry: false,
        },
    );
    let telemetry = engine.handle.telemetry();
    let timing_paths: Vec<NodePath> = kinds.keys().cloned().collect();
    engine.processor.set_clock(Some(now_ns));
    engine
        .handle
        .status()
        .timing_enabled
        .store(true, Ordering::Relaxed);
    engine.handle.play();
    let total = (seconds * rate) as usize;
    let mut out = vec![0.0f32; 2 * block];
    let mut per_kind: BTreeMap<String, (u64, usize)> = BTreeMap::new();
    for kind in kinds.values() {
        per_kind.entry(kind.clone()).or_default().1 += 1;
    }
    let mut done = 0usize;
    let mut n_blocks = 0u64;
    while done < total {
        let n = block.min(total - done);
        let mut io = HostIo {
            frames: n,
            input: &[],
            in_channels: 0,
            output: &mut out[..2 * n],
            out_channels: 2,
            midi: &[],
        };
        engine.process(&mut io);
        n_blocks += 1;
        for p in &timing_paths {
            if let (Some(ns), Some(kind)) = (telemetry.node_time_ns(p), kinds.get(p)) {
                per_kind.entry(kind.clone()).or_default().0 += u64::from(ns);
            }
        }
        done += n;
    }
    // Timing itself costs a little; measure the real-time ratio separately
    // without it so the headline number isn't inflated.
    let realtime = {
        engine
            .handle
            .status()
            .timing_enabled
            .store(false, Ordering::Relaxed);
        engine.handle.play_from_start();
        let t0 = now_ns();
        let mut done = 0usize;
        while done < total {
            let n = block.min(total - done);
            let mut io = HostIo {
                frames: n,
                input: &[],
                in_channels: 0,
                output: &mut out[..2 * n],
                out_channels: 2,
                midi: &[],
            };
            engine.process(&mut io);
            done += n;
        }
        let elapsed = (now_ns() - t0).max(1) as f64 / 1e9;
        seconds / elapsed
    };
    Outcome {
        name: name.to_string(),
        realtime,
        nodes: kinds.len(),
        blocks: n_blocks,
        per_kind,
    }
}

fn print_table(results: &[Outcome]) {
    for r in results {
        println!(
            "{:<28} {:>6} nodes  {:>9.1}× real time",
            r.name, r.nodes, r.realtime
        );
        let total: u64 = r.per_kind.values().map(|v| v.0).sum::<u64>().max(1);
        let mut kinds: Vec<_> = r.per_kind.iter().collect();
        kinds.sort_by_key(|(_, v)| std::cmp::Reverse(v.0));
        for (kind, (ns, count)) in kinds.iter().take(6) {
            println!(
                "    {:<16} ×{:<5} {:>5.1} %  {:>8.0} ns/node/block",
                kind,
                count,
                *ns as f64 * 100.0 / total as f64,
                *ns as f64 / (*count).max(1) as f64 / r.blocks.max(1) as f64
            );
        }
    }
}

fn results_json(results: &[Outcome], block: usize) -> Value {
    json!({
        "block_size": block,
        "projects": results.iter().map(|r| (r.name.clone(), json!({
            "realtime": r.realtime,
            "nodes": r.nodes,
        }))).collect::<serde_json::Map<String, Value>>(),
    })
}

fn append_history(path: &Path, results: &Value) -> Result<()> {
    use std::io::Write;
    let line = json!({
        "time": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        "version": env!("CARGO_PKG_VERSION"),
        "results": results,
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| anyhow!("{}: {e}", path.display()))?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn compare(current: &Value, baseline: &Path, threshold: f64) -> Result<ExitCode> {
    let text =
        std::fs::read_to_string(baseline).map_err(|e| anyhow!("{}: {e}", baseline.display()))?;
    let base: Value = serde_json::from_str(&text)?;
    let mut failed = false;
    if let Some(projects) = current["projects"].as_object() {
        for (name, cur) in projects {
            let (Some(c), Some(b)) = (
                cur["realtime"].as_f64(),
                base["projects"][name]["realtime"].as_f64(),
            ) else {
                println!("{name}: no baseline");
                continue;
            };
            let change = c / b - 1.0;
            let verdict = if change < -threshold {
                failed = true;
                "REGRESSION"
            } else {
                "ok"
            };
            println!(
                "{name}: {c:.1}× vs {b:.1}× ({:+.1} %) {verdict}",
                change * 100.0
            );
        }
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn node(id: u64, kind: &str) -> NodeInstance {
    NodeInstance::new(NodeId(id), kind)
}

fn wire(project: &mut Project, from: u64, out: &str, to: u64, inp: &str) {
    project.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(from), out),
        Endpoint::new(NodeId(to), inp),
    ));
}

/// The built-in stress projects (Phase 1 nodes only).
pub fn stress_projects() -> Vec<(String, Project)> {
    vec![
        ("oscillators-1000".into(), oscillators(1000)),
        ("effect-chain-200".into(), effect_chain(200)),
        ("mono-synths-16".into(), synths(16)),
    ]
}

/// 1000 band-limited saw oscillators summed through a tree of 8-input mixers.
fn oscillators(count: u64) -> Project {
    let mut p = Project {
        name: format!("{count} oscillators into mixers"),
        ..Project::default()
    };
    let mut id = 1u64;
    let mut layer: Vec<u64> = Vec::new();
    for i in 0..count {
        p.graph.nodes.push(
            node(id, "oscillator")
                .with_param("freq", 55.0 + i as f64 * 3.7)
                .with_param("shape", "saw")
                .with_param("start", "load"),
        );
        layer.push(id);
        id += 1;
    }
    while layer.len() > 1 {
        let mut next = Vec::new();
        for chunk in layer.chunks(8) {
            p.graph
                .nodes
                .push(node(id, "mix").with_config(json!({ "inputs": chunk.len() })));
            for (k, src) in chunk.iter().enumerate() {
                wire(&mut p, *src, "out", id, &format!("in_{k}"));
                p.graph
                    .node_mut(NodeId(id))
                    .map(|n| n.params.insert(format!("gain_{k}"), (-18.0).into()));
            }
            next.push(id);
            id += 1;
        }
        layer = next;
    }
    p.graph.nodes.push(node(id, "audio_out"));
    if let Some(&last) = layer.first() {
        wire(&mut p, last, "out", id, "in");
    }
    p
}

/// A guitar-pedal style chain: oscillator → (biquad → delay → gain) × n → out.
fn effect_chain(stages: u64) -> Project {
    let mut p = Project {
        name: format!("{stages}-stage effect chain"),
        ..Project::default()
    };
    p.graph.nodes.push(
        node(1, "oscillator")
            .with_param("shape", "saw")
            .with_param("start", "load"),
    );
    let mut prev = 1u64;
    let mut id = 2u64;
    for i in 0..stages {
        p.graph.nodes.push(
            node(id, "biquad")
                .with_param("cutoff", 400.0 + (i % 20) as f64 * 300.0)
                .with_param("q", 0.9),
        );
        wire(&mut p, prev, "out", id, "in");
        p.graph.nodes.push(
            node(id + 1, "delay")
                .with_param("time", 0.01 + (i % 7) as f64 * 0.003)
                .with_param("feedback", 0.2)
                .with_param("mix", 0.3),
        );
        wire(&mut p, id, "out", id + 1, "in");
        p.graph
            .nodes
            .push(node(id + 2, "gain").with_param("gain", -0.1));
        wire(&mut p, id + 1, "out", id + 2, "in");
        prev = id + 2;
        id += 3;
    }
    p.graph.nodes.push(node(id, "audio_out"));
    wire(&mut p, prev, "out", id, "in");
    p
}

/// N copies of the subtractive mono synth (clock-driven, since there's no
/// MIDI offline): clock → ADSR → gain; oscillator → biquad → gain → mix.
fn synths(voices: u64) -> Project {
    let mut p = Project {
        name: format!("{voices} mono synths"),
        ..Project::default()
    };
    let mix = 1u64;
    p.graph
        .nodes
        .push(node(mix, "mix").with_config(json!({ "inputs": voices })));
    let mut id = 2u64;
    for v in 0..voices {
        let (clock, adsr, map, osc, filt, gain, sweep) =
            (id, id + 1, id + 2, id + 3, id + 4, id + 5, id + 6);
        id += 7;
        p.graph
            .nodes
            .push(node(clock, "clock").with_param("interval", 0.25 + (v % 4) as f64 * 0.25));
        p.graph.nodes.push(node(adsr, "adsr"));
        p.graph.nodes.push(
            node(map, "map")
                .with_param("out_min", -60.0)
                .with_param("out_max", 0.0),
        );
        p.graph.nodes.push(
            node(osc, "oscillator")
                .with_param("freq", 110.0 * (1.0 + v as f64 * 0.25))
                .with_param("shape", "saw")
                .with_param("start", "load"),
        );
        p.graph.nodes.push(
            node(filt, "biquad")
                .with_param("cutoff", 1200.0)
                .with_param("q", 2.0),
        );
        p.graph.nodes.push(node(gain, "gain"));
        p.graph.nodes.push(
            node(sweep, "map")
                .with_param("out_min", 200.0)
                .with_param("out_max", 3000.0),
        );
        wire(&mut p, clock, "gate", adsr, "gate");
        wire(&mut p, adsr, "out", map, "in");
        wire(&mut p, map, "out", gain, "gain");
        wire(&mut p, adsr, "out", sweep, "in");
        wire(&mut p, sweep, "out", filt, "cutoff");
        wire(&mut p, osc, "out", filt, "in");
        wire(&mut p, filt, "out", gain, "in");
        wire(&mut p, gain, "out", mix, &format!("in_{v}"));
    }
    p.graph.nodes.push(node(id, "audio_out"));
    wire(&mut p, mix, "out", id, "in");
    p
}
