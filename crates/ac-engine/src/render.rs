//! Offline rendering (ARCHITECTURE §10): run the engine as fast as the CPU
//! allows over the project length and collect the device outputs.

use std::sync::{Arc, Mutex};

use ac_core::{
    AudioData, AudioIn, ConfigExt, FileRequest, MidiEvent, MidiMsg, NodePath, Project,
    RecordRequest, RecordSink, Registry, Resources, Value,
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
    /// What each `FileOut` node rendered, by node path. The host writes them.
    pub files: Vec<RenderedFile>,
    /// MIDI that `MidiOut` nodes sent, at frames from the render's start.
    pub midi: Vec<(u64, MidiMsg)>,
}

/// The audio a `FileOut` node received while the render played.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedFile {
    /// The node's flattened path.
    pub node: String,
    /// The node's label or kind name.
    pub name: String,
    /// The path from the node's config, as written; the host decides where
    /// it goes.
    pub path: String,
    pub sample_rate: u32,
    /// Non-interleaved channels.
    pub channels: Vec<Vec<f32>>,
}

/// MIDI out messages one block can carry.
const MIDI_OUT_CAPACITY: usize = 4096;

/// The caller's resources, plus in-memory sinks for `FileOut` nodes.
struct RenderResources {
    inner: Arc<dyn Resources>,
    /// Frames the render covers: each file's capacity.
    frames: usize,
    files: Arc<Mutex<Vec<RenderedFile>>>,
}

impl Resources for RenderResources {
    fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
        self.inner.audio_asset(path)
    }

    fn recorder(&self, req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
        self.inner.recorder(req)
    }

    fn file_out(&self, req: &FileRequest<'_>) -> Option<Box<dyn RecordSink>> {
        // Allocated here, while the node is built, so writing never does.
        Some(Box::new(MemoryFile {
            file: RenderedFile {
                node: req.node_key.to_string(),
                name: req.name.to_string(),
                path: req.path.to_string(),
                sample_rate: req.sample_rate,
                // Not `vec![v; n]`: clones don't keep the capacity.
                channels: (0..req.channels.max(1))
                    .map(|_| Vec::with_capacity(self.frames))
                    .collect(),
            },
            writing: false,
            started: false,
            done: self.files.clone(),
        }))
    }
}

/// Collects a `FileOut` node's input; hands it over when the node is dropped
/// (after the render, off the processing path).
struct MemoryFile {
    file: RenderedFile,
    writing: bool,
    started: bool,
    done: Arc<Mutex<Vec<RenderedFile>>>,
}

impl RecordSink for MemoryFile {
    fn start(&mut self) {
        self.writing = true;
        self.started = true;
    }

    fn write(&mut self, audio: &AudioIn<'_>) {
        if !self.writing {
            return;
        }
        for (c, ch) in self.file.channels.iter_mut().enumerate() {
            let src = audio.channel(c.min(audio.channels().saturating_sub(1)));
            // Never past the capacity reserved up front.
            let n = src.len().min(ch.capacity() - ch.len());
            ch.extend_from_slice(&src[..n]);
        }
    }

    fn stop(&mut self) {
        self.writing = false;
    }
}

impl Drop for MemoryFile {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        if let Ok(mut files) = self.done.lock() {
            let file = std::mem::replace(
                &mut self.file,
                RenderedFile {
                    node: String::new(),
                    name: String::new(),
                    path: String::new(),
                    sample_rate: 0,
                    channels: Vec::new(),
                },
            );
            // A node rebuilt during the render replaces its earlier sink.
            files.retain(|f| f.node != file.node);
            files.push(file);
        }
    }
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
    let files = Arc::new(Mutex::new(Vec::new()));
    let resources: Arc<dyn Resources> = Arc::new(RenderResources {
        inner: resources,
        frames: total,
        files: files.clone(),
    });

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
    let mut midi_out: Vec<MidiEvent> = Vec::with_capacity(MIDI_OUT_CAPACITY);
    let mut sent = Vec::new();
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
            midi_out: Some(&mut midi_out),
        };
        engine.process(&mut io);
        for (c, ch) in channels.iter_mut().enumerate() {
            ch.extend_from_slice(&out[c * n..(c + 1) * n]);
        }
        sent.extend(
            midi_out
                .drain(..)
                .map(|e| ((done as u64) + u64::from(e.offset), e.msg)),
        );
        done += n;
    }
    // Dropping the engine drops the FileOut nodes, which hand over their audio.
    drop(engine);
    let mut files = files
        .lock()
        .map(|mut f| std::mem::take(&mut *f))
        .unwrap_or_default();
    files.sort_by(|a, b| a.node.cmp(&b.node));
    Rendered {
        sample_rate,
        channels,
        report,
        files,
        midi: sent,
    }
}
