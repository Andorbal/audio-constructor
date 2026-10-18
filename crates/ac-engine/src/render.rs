//! Offline rendering (ARCHITECTURE §10): run the engine as fast as the CPU
//! allows over the project length and collect the device outputs.

use std::sync::Arc;

use ac_core::{
    AudioData, ConfigExt, MidiEvent, MidiMsg, NodePath, Project, Registry, Resources, Value,
};

use crate::bake::BakeInput;
use crate::engine::{Engine, EngineOptions, HostIo};
use crate::report::CompileReport;

/// What to render and how.
#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    /// Override the project's sample rate.
    pub sample_rate: Option<u32>,
    /// Override the project's render block size.
    pub block_size: Option<usize>,
    /// Start time in seconds.
    pub from: f64,
    /// End time in seconds (default: the project length).
    pub to: Option<f64>,
    /// Audio fed to `AudioIn` nodes as if from a device (must match the rate).
    pub input: Option<AudioData>,
    /// MIDI fed to `MidiIn` nodes, at absolute sample positions from `from`.
    pub midi: Vec<(u64, MidiMsg)>,
    /// `--set node.socket=value` overrides, applied before compiling.
    pub set: Vec<(NodePath, String, Value)>,
    /// Number of device output channels (default: what the AudioOut nodes use, at least 2).
    pub out_channels: Option<usize>,
}

/// A finished render.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub sample_rate: u32,
    /// Non-interleaved device output channels.
    pub channels: Vec<Vec<f32>>,
    pub report: CompileReport,
}

impl Rendered {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// Device output channels a project writes to: the highest `channel + channels`
/// over its AudioOut nodes (inside groups too), at least 2.
pub fn required_output_channels(project: &Project) -> usize {
    let mut max = 2usize;
    let mut visit = |g: &ac_core::Graph| {
        for n in &g.nodes {
            if n.kind == "audio_out" {
                let first = n.config.get_u64_or("channel", 0) as usize;
                let count = n.config.get_u64_or("channels", 2) as usize;
                max = max.max((first + count).min(64));
            }
        }
    };
    visit(&project.graph);
    for def in project.group_defs.values() {
        visit(&def.graph);
    }
    max
}

/// Apply `--set` overrides to root-level node params.
pub fn apply_overrides(
    project: &mut Project,
    set: &[(NodePath, String, Value)],
    report: &mut CompileReport,
) {
    for (path, socket, value) in set {
        let target = (path.depth() == 1)
            .then(|| path.leaf())
            .flatten()
            .and_then(|id| project.graph.node_mut(id));
        match target {
            Some(node) => {
                node.params.insert(socket.clone(), value.clone());
            }
            None => report.warning(
                Some(path),
                Some(socket),
                "--set target is not a root-level node; ignored",
            ),
        }
    }
}

/// Render a project offline. Same project, rate, and block size → same samples.
pub fn render(
    project: &Project,
    registry: Arc<Registry>,
    resources: Arc<dyn Resources>,
    opts: &RenderOptions,
) -> Rendered {
    let mut project = project.clone();
    let mut pre = CompileReport::default();
    apply_overrides(&mut project, &opts.set, &mut pre);
    let sample_rate = opts
        .sample_rate
        .unwrap_or(project.settings.sample_rate)
        .clamp(1_000, 768_000);
    let block = opts
        .block_size
        .unwrap_or(project.settings.block_size as usize)
        .clamp(1, 16_384);
    let out_channels = opts
        .out_channels
        .unwrap_or_else(|| required_output_channels(&project))
        .max(1);
    let to = opts.to.unwrap_or(project.settings.length).max(opts.from);
    let start = (opts.from.max(0.0) * f64::from(sample_rate)).round() as u64;
    let end = (to * f64::from(sample_rate)).round() as u64;
    let total = end.saturating_sub(start) as usize;

    let (mut engine, mut report) = Engine::new(
        project,
        registry,
        resources,
        EngineOptions {
            sample_rate: f64::from(sample_rate),
            max_block: block,
            socket_telemetry: false,
            offline: true,
        },
    );
    // Whole-input nodes: bake them first, over the whole render, from the
    // same input (ARCHITECTURE §10, multi-pass rendering).
    if !engine.handle.bakes().is_empty() {
        engine.handle.set_bake_frames(Some(end));
        if opts.input.is_some() || !opts.midi.is_empty() {
            // Bakes render from zero; the input starts at `from`.
            let audio = opts.input.as_ref().map(|a| {
                let lead = start as usize;
                Arc::new(AudioData {
                    sample_rate: a.sample_rate,
                    channels: a
                        .channels
                        .iter()
                        .map(|c| {
                            let mut v = vec![0.0; lead];
                            v.extend_from_slice(c);
                            v
                        })
                        .collect(),
                })
            });
            engine.handle.set_bake_input(BakeInput {
                audio,
                midi: Arc::new(opts.midi.iter().map(|&(at, m)| (at + start, m)).collect()),
            });
        }
        engine.handle.bake_all();
        report = engine.handle.report().clone();
    }
    report.diagnostics.splice(0..0, pre.diagnostics);
    if let Some(input) = &opts.input
        && input.sample_rate != sample_rate
    {
        report.warning(
            None,
            None,
            format!(
                "input is {} Hz but the render runs at {} Hz; it will play at the wrong speed",
                input.sample_rate, sample_rate
            ),
        );
    }
    if start > 0 {
        engine.handle.seek(start);
    }
    engine.handle.play();

    let in_channels = opts.input.as_ref().map_or(0, |i| i.channels.len());
    let mut channels = vec![Vec::with_capacity(total); out_channels];
    let mut out = vec![0.0f32; out_channels * block];
    let mut inp = vec![0.0f32; in_channels * block];
    let mut midi: Vec<MidiEvent> = Vec::new();
    let mut midi_events = opts.midi.clone();
    midi_events.sort_by_key(|e| e.0);
    let mut done = 0usize;
    while done < total {
        let n = block.min(total - done);
        if let Some(input) = &opts.input {
            for (c, ch) in input.channels.iter().enumerate() {
                for i in 0..n {
                    inp[c * n + i] = ch.get(done + i).copied().unwrap_or(0.0);
                }
            }
        }
        midi.clear();
        for &(at, msg) in &midi_events {
            let at = at as usize;
            if at >= done && at < done + n {
                midi.push(MidiEvent::new((at - done) as u32, msg));
            }
        }
        let mut io = HostIo {
            frames: n,
            input: &inp[..in_channels * n],
            in_channels,
            output: &mut out[..out_channels * n],
            out_channels,
            midi: &midi,
        };
        engine.process(&mut io);
        for (c, ch) in channels.iter_mut().enumerate() {
            ch.extend_from_slice(&out[c * n..(c + 1) * n]);
        }
        done += n;
    }
    Rendered {
        sample_rate,
        channels,
        report,
    }
}
