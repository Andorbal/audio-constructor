//! Background baking of whole-input nodes (ARCHITECTURE §9).
//!
//! A whole-input node (Reverse, Normalize) can't stream: it needs all of its
//! input first. The compiler decides, per node, whether it is **disabled**
//! (a live source such as Audio In feeds it) or bakeable. A bake renders the
//! node's upstream subgraph over the project length, captures the node's
//! inputs, calls the node's `render_whole`, and keeps the result; the running
//! graph then streams the result at the transport position.
//!
//! Bakes are keyed by a hash of everything upstream (nodes, params, wires,
//! settings), so editing anything upstream invalidates them while edits
//! elsewhere don't. Until a fresh bake arrives, the previous one keeps
//! playing. Jobs are self-contained and run on any thread; the engine crate
//! itself spawns nothing (it builds for wasm).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use ac_core::{
    AudioData, Control, ControlEvent, HostAccess, MidiEvent, MidiMsg, NodePath, Project,
    RecordRequest, RecordSink, Registry, Resources, WholeCtx, WholeInputs, WholeOutputs,
};

use crate::compile::{CaptureTarget, CompileOptions, compile_with};
use crate::graph::BlockTransport;

/// Where a whole-input node stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WholeState {
    /// A live source feeds it; it outputs silence. Holds the live sources'
    /// names.
    Disabled { sources: Vec<String> },
    /// Waiting for its first bake; silent meanwhile.
    Pending,
    /// Something upstream changed; the previous bake plays until the new one
    /// is ready.
    Stale,
    /// Playing a bake of its current input.
    Baked,
}

impl WholeState {
    /// Short label for badges and reports.
    pub fn label(&self) -> &'static str {
        match self {
            WholeState::Disabled { .. } => "disabled",
            WholeState::Pending | WholeState::Stale => "baking",
            WholeState::Baked => "baked",
        }
    }
}

/// One whole-input node in a compiled graph.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeSpec {
    pub path: NodePath,
    /// Hash of everything the bake depends on.
    pub key: u64,
    pub state: WholeState,
    /// Whole-input nodes upstream of this one; their bakes come first.
    pub deps: Vec<NodePath>,
    /// Every node upstream (flattened paths), for invalidation on parameter
    /// changes.
    pub upstream: Vec<NodePath>,
}

/// A finished bake.
#[derive(Clone)]
pub struct BakeResult {
    pub path: NodePath,
    pub key: u64,
    pub outputs: Arc<WholeOutputs>,
}

impl fmt::Debug for BakeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakeResult")
            .field("path", &self.path)
            .field("key", &format_args!("{:016x}", self.key))
            .field("bytes", &self.outputs.bytes())
            .finish()
    }
}

/// The latest bake of every whole-input node, by path. A result stays (as
/// stale) after its key changes, until a fresh one replaces it.
#[derive(Clone, Debug, Default)]
pub struct BakeStore {
    results: HashMap<NodePath, BakeResult>,
}

impl BakeStore {
    pub fn get(&self, path: &NodePath) -> Option<&BakeResult> {
        self.results.get(path)
    }

    pub fn insert(&mut self, r: BakeResult) {
        self.results.insert(r.path.clone(), r);
    }

    /// Forget results for nodes that no longer exist.
    pub fn retain(&mut self, keep: &HashSet<NodePath>) {
        self.results.retain(|p, _| keep.contains(p));
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

/// Input a bake replays into `Audio In`, `MIDI In`, and control-surface
/// nodes (offline renders, where those are known in advance).
#[derive(Clone, Debug, Default)]
pub struct BakeInput {
    pub audio: Option<Arc<AudioData>>,
    pub midi: Arc<Vec<(u64, MidiMsg)>>,
    /// Key, gamepad, and OSC events at absolute frames, in time order.
    pub controls: Arc<Vec<(u64, Control, f32)>>,
}

/// Everything needed to bake one node, on any thread.
#[derive(Clone)]
pub struct BakeJob {
    pub path: NodePath,
    pub key: u64,
    pub(crate) project: Arc<Project>,
    pub(crate) registry: Arc<Registry>,
    pub(crate) resources: Arc<dyn Resources>,
    pub(crate) options: CompileOptions,
    /// Finished bakes of whole-input nodes upstream.
    pub(crate) store: BakeStore,
    pub(crate) input: BakeInput,
}

impl fmt::Debug for BakeJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BakeJob")
            .field("path", &self.path)
            .field("key", &format_args!("{:016x}", self.key))
            .field("frames", &self.options.bake_frames)
            .finish()
    }
}

/// Resources for a bake: the same assets, but no recording and no `FileOut`
/// files (a bake must not write takes or files).
struct BakeResources(Arc<dyn Resources>);

impl Resources for BakeResources {
    fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
        self.0.audio_asset(path)
    }
    fn recorder(&self, _req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
        None
    }
}

impl BakeJob {
    /// Frames this bake covers.
    pub fn frames(&self) -> u64 {
        self.options.bake_frames
    }

    /// Render the node's upstream, capture its inputs, and compute its
    /// output. CPU-bound; may take a while for long projects. Never panics
    /// on bad projects (they bake silence).
    pub fn run(&self) -> BakeResult {
        let frames = usize::try_from(self.options.bake_frames).unwrap_or(0);
        let sink = Arc::new(Mutex::new(WholeInputs::default()));
        let resources = BakeResources(self.resources.clone());
        let mut opts = self.options.clone();
        opts.max_block = opts.bake_block.max(1);
        opts.socket_telemetry = false;
        let compiled = compile_with(
            &self.project,
            &self.registry,
            &resources,
            &opts,
            None,
            &self.store,
            Some(&CaptureTarget {
                path: self.path.clone(),
                sink: sink.clone(),
                frames,
            }),
        );
        let entry = compiled.capture_entry;
        let mut graph = compiled.graph;
        let block = opts.max_block;
        let audio = self.input.audio.as_deref();
        let in_channels = audio.map_or(0, |a| a.channels.len());
        let mut inp = vec![0.0f32; in_channels * block];
        let mut midi: Vec<MidiEvent> = Vec::new();
        let mut controls: Vec<ControlEvent> = Vec::new();
        let mut done = 0usize;
        let mut started = true;
        while done < frames {
            let n = block.min(frames - done);
            if let Some(a) = audio {
                for (c, ch) in a.channels.iter().enumerate() {
                    for i in 0..n {
                        inp[c * n + i] = ch.get(done + i).copied().unwrap_or(0.0);
                    }
                }
            }
            midi.clear();
            for &(at, msg) in self.input.midi.iter() {
                let at = at as usize;
                if at >= done && at < done + n {
                    midi.push(MidiEvent::new((at - done) as u32, msg));
                }
            }
            controls.clear();
            for &(at, control, value) in self.input.controls.iter() {
                let at = at as usize;
                if at >= done && at < done + n {
                    controls.push(ControlEvent::new((at - done) as u32, control, value));
                }
            }
            let mut host = HostAccess {
                input: &inp[..in_channels * n],
                in_channels,
                output: &mut [],
                out_channels: 0,
                stride: n,
                offset: 0,
                frames: n,
                midi: &midi,
                midi_out: None,
                controls: &controls,
                osc_out: None,
            };
            let transport = BlockTransport {
                playing: true,
                recording: false,
                position: done as u64,
                started,
                stopped: false,
            };
            if started {
                graph.reset_nodes();
            }
            graph.run(n, transport, &mut host, None, None, Default::default());
            graph.advance_params(n);
            started = false;
            done += n;
        }
        let inputs = sink
            .lock()
            .map(|mut s| std::mem::take(&mut *s))
            .unwrap_or_default();
        let outputs = match entry {
            Some((render, config, seed)) => render(
                &config,
                &WholeCtx {
                    sample_rate: opts.sample_rate,
                    frames,
                    seed,
                },
                &inputs,
            ),
            None => WholeOutputs::default(),
        };
        BakeResult {
            path: self.path.clone(),
            key: self.key,
            outputs: Arc::new(outputs),
        }
    }
}
