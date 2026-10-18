//! A harness for unit-testing a single node without the engine.
//!
//! ```ignore
//! let mut h = Harness::new::<Gain>(serde_json::json!({}));
//! h.set_audio("in", vec![vec![1.0; 256]]);
//! h.set("gain", -6.0);
//! let out = h.run(256);
//! assert!((out.audio("out")[0][100] - 0.501).abs() < 1e-3);
//! ```
//!
//! The harness builds the same [`Io`] accessors the engine does, runs the node
//! block by block (64 frames by default), and concatenates the outputs. The
//! transport plays from zero unless [`Harness::stopped`] is called, so
//! generators with the default start mode begin on the first block.

// A test helper: a typo in a socket name should fail the test loudly.
#![allow(clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::events::{MidiEvent, MidiMsg};
use crate::io::BufferRef;
use crate::io::{
    AutoFlags, FloatSrc, HostAccess, InputSlot, Inputs, IntSrc, Io, OutputSlot, Outputs, ParamSlot,
    TelemetryAccess, TriggerSrc,
};
use crate::node::{
    AudioData, EngineInfo, NoResources, Node, NodeMeta, Phase, PrepareCtx, ProcessCtx, Resources,
};
use crate::param::param_value;
use crate::signal::{Channels, OutRate, Rate, SignalType, Signature, SpectrumShape};
use crate::spectrum::SpectrumBuf;
use crate::time::{TempoMap, TransportView};
use crate::value::{Config, Value};

#[derive(Clone, Debug)]
enum Feed {
    Unwired,
    Const(Value),
    Samples(Vec<f32>),
    Block(Vec<f32>),
    Int(i32),
    Audio(Vec<Vec<f32>>),
    Triggers(Vec<u64>),
    Midi(Vec<(u64, MidiMsg)>),
    Spectrum(SpectrumShape, Vec<(u64, Vec<f32>)>),
    Buffer(Arc<AudioData>),
}

/// A spectrum frame as the harness collects it: absolute sample position and
/// interleaved `re, im` bins.
pub type Frame = (u64, Vec<f32>);

/// Outputs collected over one [`Harness::run`].
#[derive(Clone, Debug, Default)]
pub struct Output {
    /// `Float`/`Bool` outputs, one value per frame (block-rate values repeated).
    pub floats: BTreeMap<String, Vec<f32>>,
    /// `Int` outputs, one value per frame.
    pub ints: BTreeMap<String, Vec<i32>>,
    /// `Audio` outputs, per channel.
    pub audio: BTreeMap<String, Vec<Vec<f32>>>,
    /// Trigger outputs as absolute sample positions.
    pub triggers: BTreeMap<String, Vec<u64>>,
    /// MIDI outputs at absolute sample positions.
    pub midi: BTreeMap<String, Vec<(u64, MidiMsg)>>,
    /// Spectrum outputs: every frame, at absolute sample positions.
    pub spectra: BTreeMap<String, Vec<Frame>>,
    /// Buffer outputs: the value after the last block.
    pub buffers: BTreeMap<String, BufferRef>,
    /// Device output channels written by the node.
    pub host_out: Vec<Vec<f32>>,
    /// MIDI the node sent to the devices, at absolute sample positions
    /// (`host_output` nodes only).
    pub host_midi_out: Vec<(u64, MidiMsg)>,
}

impl Output {
    pub fn float(&self, name: &str) -> &[f32] {
        self.floats.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn int(&self, name: &str) -> &[i32] {
        self.ints.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn audio(&self, name: &str) -> &[Vec<f32>] {
        self.audio.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn triggers(&self, name: &str) -> &[u64] {
        self.triggers.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn midi(&self, name: &str) -> &[(u64, MidiMsg)] {
        self.midi.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn spectrum(&self, name: &str) -> &[Frame] {
        self.spectra.get(name).map_or(&[], Vec::as_slice)
    }
    pub fn buffer(&self, name: &str) -> Option<&Arc<AudioData>> {
        self.buffers.get(name).and_then(Option::as_ref)
    }
}

/// Drives one node through blocks with chosen inputs.
pub struct Harness {
    node: Box<dyn Node>,
    meta: NodeMeta,
    sig: Signature,
    sample_rate: f64,
    block: usize,
    feeds: Vec<Feed>,
    fire: Vec<Vec<u64>>,
    tempo: TempoMap,
    playing: bool,
    recording: bool,
    position: u64,
    frame: u64,
    pending_play: bool,
    pending_stop: bool,
    pending_load: bool,
    host_in: Vec<Vec<f32>>,
    host_out_channels: usize,
    host_midi: Vec<(u64, MidiMsg)>,
    telemetry: Vec<AtomicU32>,
    scope: Vec<AtomicU32>,
    scope_pos: AtomicU32,
    phases: Vec<Phase>,
    guard: Option<fn(&mut dyn FnMut())>,
    /// Buffer outputs persist from block to block, as in the engine.
    buffer_state: Vec<BufferRef>,
    engine: EngineInfo,
}

impl Harness {
    /// A harness at 48 kHz with 64-frame blocks and no host resources.
    pub fn new<N: Node>(config: Config) -> Harness {
        Self::with_resources::<N>(config, 48_000.0, 64, &NoResources)
    }

    /// A harness at a given rate and block size, with resources for `new()`.
    pub fn with_resources<N: Node>(
        config: Config,
        sample_rate: f64,
        block: usize,
        resources: &dyn Resources,
    ) -> Harness {
        let entry = crate::registry::NodeEntry {
            meta: N::meta(),
            describe: N::describe,
            create: |c, ctx| Box::new(N::new(c, ctx)),
            render_whole: N::render_whole,
        };
        Self::from_entry(&entry, config, sample_rate, block, resources)
    }

    /// A harness for a registry entry (for tests that sweep every node kind).
    pub fn from_entry(
        entry: &crate::registry::NodeEntry,
        config: Config,
        sample_rate: f64,
        block: usize,
        resources: &dyn Resources,
    ) -> Harness {
        let meta = entry.meta.clone();
        let sig = (entry.describe)(&config);
        let block = block.max(1);
        let ctx = PrepareCtx {
            sample_rate,
            max_block: block,
            seed: 0x5eed,
            node_key: "test",
            name: meta.name,
            resources,
        };
        let node: Box<dyn Node> = (entry.create)(&config, &ctx);
        let n_in = sig.inputs.len();
        Harness {
            telemetry: (0..meta.telemetry).map(|_| AtomicU32::new(0)).collect(),
            scope: (0..meta.scope).map(|_| AtomicU32::new(0)).collect(),
            scope_pos: AtomicU32::new(0),
            node,
            meta,
            sig,
            sample_rate,
            block,
            feeds: vec![Feed::Unwired; n_in],
            fire: vec![Vec::new(); n_in],
            tempo: TempoMap::constant(sample_rate, 120.0),
            playing: true,
            recording: false,
            position: 0,
            frame: 0,
            pending_play: true,
            pending_stop: false,
            pending_load: true,
            host_in: Vec::new(),
            host_out_channels: 0,
            host_midi: Vec::new(),
            phases: vec![Phase::Main],
            guard: None,
            buffer_state: Vec::new(),
            engine: EngineInfo::default(),
        }
    }

    pub fn signature(&self) -> &Signature {
        &self.sig
    }

    pub fn meta(&self) -> &NodeMeta {
        &self.meta
    }

    /// Run with the transport stopped (no `play` start trigger).
    pub fn stopped(&mut self) -> &mut Self {
        self.playing = false;
        self.pending_play = false;
        self
    }

    /// Start the transport at the next block.
    pub fn play(&mut self) -> &mut Self {
        self.playing = true;
        self.pending_play = true;
        self
    }

    /// Stop the transport at the next block.
    pub fn stop(&mut self) -> &mut Self {
        self.playing = false;
        self.pending_stop = true;
        self
    }

    /// Set the transport's recording flag (for `Record`).
    pub fn recording(&mut self, on: bool) -> &mut Self {
        self.recording = on;
        self
    }

    /// Move the transport to `position` before the next block (as a seek
    /// while playing, this also fires `play` start triggers).
    pub fn seek(&mut self, position: u64) -> &mut Self {
        self.position = position;
        if self.playing {
            self.pending_play = true;
        }
        self
    }

    /// Engine figures nodes see in `ProcessCtx::engine` (load, xruns).
    pub fn engine_info(&mut self, info: EngineInfo) -> &mut Self {
        self.engine = info;
        self
    }

    /// Run `render_whole` for a whole-input node over these inputs.
    pub fn render_whole<N: Node>(
        config: &Config,
        sample_rate: f64,
        inputs: Vec<crate::whole::WholeValue>,
    ) -> crate::whole::WholeOutputs {
        let frames = inputs
            .iter()
            .map(|v| match v {
                crate::whole::WholeValue::Audio(c) => c.first().map_or(0, Vec::len),
                crate::whole::WholeValue::Float(x) => x.len(),
                crate::whole::WholeValue::Int(x) => x.len(),
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        N::render_whole(
            config,
            &crate::whole::WholeCtx {
                sample_rate,
                frames,
                seed: 0x5eed,
            },
            &crate::whole::WholeInputs { values: inputs },
        )
    }

    pub fn tempo(&mut self, tempo: TempoMap) -> &mut Self {
        self.tempo = tempo;
        self
    }

    /// Call `process` once per phase per block (for `Feedback`).
    pub fn phases(&mut self, phases: &[Phase]) -> &mut Self {
        self.phases = phases.to_vec();
        self
    }

    /// Wrap every `process` call (e.g. with an allocation guard).
    pub fn guard(&mut self, g: fn(&mut dyn FnMut())) -> &mut Self {
        self.guard = Some(g);
        self
    }

    fn idx(&self, name: &str) -> usize {
        match self.sig.input_index(name) {
            Some(i) => i,
            None => panic!("{} has no input `{name}`", self.meta.kind),
        }
    }

    /// Set an unwired input's param (numbers, bools, choice names).
    pub fn set(&mut self, name: &str, v: impl Into<Value>) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Const(v.into());
        self
    }

    /// Wire a sample-rate signal into a `Float`/`Bool` input. Index = absolute frame.
    pub fn set_samples(&mut self, name: &str, v: Vec<f32>) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Samples(v);
        self
    }

    /// Wire a block-rate signal (sampled at each block's first frame).
    pub fn set_block(&mut self, name: &str, v: Vec<f32>) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Block(v);
        self
    }

    /// Wire a block-rate `Int`.
    pub fn set_int(&mut self, name: &str, v: i32) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Int(v);
        self
    }

    /// Wire audio channels into an input. Index = absolute frame.
    pub fn set_audio(&mut self, name: &str, channels: Vec<Vec<f32>>) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Audio(channels);
        self
    }

    /// Wire triggers at absolute frames.
    pub fn triggers(&mut self, name: &str, at: &[u64]) -> &mut Self {
        let i = self.idx(name);
        let mut v = at.to_vec();
        v.sort_unstable();
        self.feeds[i] = Feed::Triggers(v);
        self
    }

    /// Press an unwired trigger's inline button at the block containing `at`.
    pub fn fire(&mut self, name: &str, at: u64) -> &mut Self {
        let i = self.idx(name);
        self.fire[i].push(at);
        self
    }

    /// Wire MIDI events at absolute frames.
    pub fn midi(&mut self, name: &str, events: &[(u64, MidiMsg)]) -> &mut Self {
        let i = self.idx(name);
        let mut v = events.to_vec();
        v.sort_by_key(|e| e.0);
        self.feeds[i] = Feed::Midi(v);
        self
    }

    /// Wire spectrum frames (at absolute sample positions) into an input.
    pub fn spectrum(&mut self, name: &str, shape: SpectrumShape, frames: Vec<Frame>) -> &mut Self {
        let i = self.idx(name);
        let mut v = frames;
        v.sort_by_key(|f| f.0);
        self.feeds[i] = Feed::Spectrum(shape, v);
        self
    }

    /// Wire a buffer into a `Buffer` input.
    pub fn buffer(&mut self, name: &str, data: Arc<AudioData>) -> &mut Self {
        let i = self.idx(name);
        self.feeds[i] = Feed::Buffer(data);
        self
    }

    /// Device input channels (for `AudioIn`). Index = absolute frame.
    pub fn host_input(&mut self, channels: Vec<Vec<f32>>) -> &mut Self {
        self.host_in = channels;
        self
    }

    /// Number of device output channels (for `AudioOut`).
    pub fn host_outputs(&mut self, n: usize) -> &mut Self {
        self.host_out_channels = n;
        self
    }

    /// Device MIDI input (for `MidiIn`), at absolute frames.
    pub fn host_midi(&mut self, events: &[(u64, MidiMsg)]) -> &mut Self {
        self.host_midi = events.to_vec();
        self.host_midi.sort_by_key(|e| e.0);
        self
    }

    /// Reset the node (as on a transport restart) before the next block.
    pub fn reset(&mut self) -> &mut Self {
        self.node.reset();
        self.pending_load = true;
        self
    }

    /// Telemetry value `k` as last published.
    pub fn telemetry(&self, k: usize) -> f32 {
        self.telemetry
            .get(k)
            .map_or(0.0, |a| f32::from_bits(a.load(Ordering::Relaxed)))
    }

    /// The scope ring, oldest sample first.
    pub fn scope(&self) -> Vec<f32> {
        let n = self.scope.len();
        let p = self.scope_pos.load(Ordering::Relaxed) as usize;
        (0..n)
            .map(|i| f32::from_bits(self.scope[(p + i) % n.max(1)].load(Ordering::Relaxed)))
            .collect()
    }

    fn input_rate(&self, i: usize) -> Rate {
        match &self.feeds[i] {
            Feed::Samples(_) | Feed::Audio(_) => Rate::Sample,
            Feed::Block(_) | Feed::Int(_) => Rate::Block,
            _ => Rate::Const,
        }
    }

    fn input_channels(&self, i: usize) -> usize {
        match &self.feeds[i] {
            Feed::Audio(c) => c.len().max(1),
            _ => 1,
        }
    }

    /// Process `frames` more frames and collect the outputs.
    pub fn run(&mut self, frames: usize) -> Output {
        let mut out = Output::default();
        let mut done = 0;
        while done < frames {
            let n = self.block.min(frames - done);
            self.run_block(n, &mut out);
            done += n;
        }
        out
    }

    fn run_block(&mut self, frames: usize, out: &mut Output) {
        let start = self.frame;
        let end = start + frames as u64;
        let n_in = self.sig.inputs.len();

        // --- inputs ---
        let mut params: Vec<ParamSlot> = Vec::new();
        let mut scalars: Vec<f32> = Vec::new();
        let mut ints: Vec<i32> = Vec::new();
        let mut samples: Vec<Vec<f32>> = vec![vec![0.0; frames]]; // buffer 0: silence
        let mut chan_table: Vec<u32> = Vec::new();
        let mut trig: Vec<Vec<u32>> = Vec::new();
        let mut midi: Vec<Vec<MidiEvent>> = Vec::new();
        let mut spectra: Vec<SpectrumBuf> = Vec::new();
        let mut buffers: Vec<BufferRef> = Vec::new();
        let mut slots: Vec<InputSlot> = Vec::with_capacity(n_in);
        for i in 0..n_in {
            let spec = self.sig.inputs[i].clone();
            let fired = self.fire[i].iter().any(|&f| f >= start && f < end);
            let mut param = |v: Option<&Value>| {
                params.push(ParamSlot {
                    value: param_value(&spec, v),
                    fired,
                    ..ParamSlot::default()
                });
                (params.len() - 1) as u32
            };
            let slot = match (&self.feeds[i], spec.ty) {
                (Feed::Triggers(at), SignalType::Trigger) => {
                    trig.push(
                        at.iter()
                            .filter(|&&t| t >= start && t < end)
                            .map(|&t| (t - start) as u32)
                            .collect(),
                    );
                    InputSlot::Trigger(TriggerSrc::Wired((trig.len() - 1) as u32))
                }
                (Feed::Const(v), SignalType::Trigger) => InputSlot::Trigger(TriggerSrc::Unwired {
                    param: param(Some(v)),
                    auto: spec.auto,
                }),
                (_, SignalType::Trigger) => InputSlot::Trigger(TriggerSrc::Unwired {
                    param: param(None),
                    auto: spec.auto,
                }),
                (Feed::Midi(evs), SignalType::Midi) => {
                    midi.push(
                        evs.iter()
                            .filter(|e| e.0 >= start && e.0 < end)
                            .map(|e| MidiEvent::new((e.0 - start) as u32, e.1))
                            .collect(),
                    );
                    InputSlot::Midi(Some((midi.len() - 1) as u32))
                }
                (_, SignalType::Midi) => InputSlot::Midi(None),
                (Feed::Spectrum(shape, frames), SignalType::Spectrum) => {
                    let inside: Vec<_> = frames
                        .iter()
                        .filter(|f| f.0 >= start && f.0 < end)
                        .collect();
                    let mut buf = SpectrumBuf::new(*shape, inside.len().max(1));
                    for f in inside {
                        buf.push_copy((f.0 - start) as u32, &f.1);
                    }
                    spectra.push(buf);
                    InputSlot::Spectrum(Some((spectra.len() - 1) as u32))
                }
                (_, SignalType::Spectrum) => InputSlot::Spectrum(None),
                (Feed::Buffer(b), SignalType::Buffer) => {
                    buffers.push(Some(b.clone()));
                    InputSlot::Buffer(Some((buffers.len() - 1) as u32))
                }
                (_, SignalType::Buffer) => InputSlot::Buffer(None),
                (Feed::Audio(chs), SignalType::Audio) => {
                    let first = chan_table.len() as u32;
                    let want = match spec.channels {
                        Channels::Fixed(n) => n as usize,
                        _ => chs.len().max(1),
                    };
                    let bufs: Vec<u32> = chs
                        .iter()
                        .map(|c| {
                            samples.push(slice_block(c, start, frames));
                            (samples.len() - 1) as u32
                        })
                        .collect();
                    for k in 0..want {
                        chan_table.push(bufs.get(k % bufs.len().max(1)).copied().unwrap_or(0));
                    }
                    InputSlot::Audio {
                        start: first,
                        count: want as u32,
                    }
                }
                (Feed::Samples(v), SignalType::Audio) => {
                    samples.push(slice_block(v, start, frames));
                    let first = chan_table.len() as u32;
                    let want = match spec.channels {
                        Channels::Fixed(n) => n as usize,
                        _ => 1,
                    };
                    chan_table.extend(std::iter::repeat_n((samples.len() - 1) as u32, want));
                    InputSlot::Audio {
                        start: first,
                        count: want as u32,
                    }
                }
                (_, SignalType::Audio) => {
                    let first = chan_table.len() as u32;
                    let want = match spec.channels {
                        Channels::Fixed(n) => n as usize,
                        _ => 1,
                    };
                    chan_table.extend(std::iter::repeat_n(0, want));
                    InputSlot::Audio {
                        start: first,
                        count: want as u32,
                    }
                }
                (Feed::Samples(v), _) => {
                    samples.push(slice_block(v, start, frames));
                    InputSlot::Float(FloatSrc::Sample((samples.len() - 1) as u32))
                }
                (Feed::Audio(chs), _) => {
                    samples.push(slice_block(
                        chs.first().map_or(&[][..], Vec::as_slice),
                        start,
                        frames,
                    ));
                    InputSlot::Float(FloatSrc::Sample((samples.len() - 1) as u32))
                }
                (Feed::Block(v), _) => {
                    scalars.push(v.get(start as usize).copied().unwrap_or(0.0));
                    InputSlot::Float(FloatSrc::Scalar((scalars.len() - 1) as u32))
                }
                (Feed::Int(k), SignalType::Int) => {
                    ints.push(*k);
                    InputSlot::Int(IntSrc::Int((ints.len() - 1) as u32))
                }
                (Feed::Int(k), _) => {
                    ints.push(*k);
                    InputSlot::Float(FloatSrc::Int((ints.len() - 1) as u32))
                }
                (Feed::Const(v), SignalType::Int) => InputSlot::Int(IntSrc::Param(param(Some(v)))),
                (Feed::Const(v), _) => InputSlot::Float(FloatSrc::Param(param(Some(v)))),
                (_, SignalType::Int) => InputSlot::Int(IntSrc::Param(param(None))),
                (_, _) => InputSlot::Float(FloatSrc::Param(param(None))),
            };
            slots.push(slot);
        }

        // --- outputs ---
        let poly_rate = (0..n_in)
            .filter(|&i| matches!(self.sig.inputs[i].ty, SignalType::Float | SignalType::Bool))
            .map(|i| self.input_rate(i))
            .max()
            .unwrap_or(Rate::Const)
            .max(Rate::Block);
        let follow_ch = (0..n_in)
            .filter(|&i| self.sig.inputs[i].ty == SignalType::Audio)
            .map(|i| self.input_channels(i))
            .max()
            .unwrap_or(1);
        let follow_shape = self.feeds.iter().find_map(|f| match f {
            Feed::Spectrum(shape, _) => Some(*shape),
            _ => None,
        });
        let mut o_samples: Vec<Vec<f32>> = Vec::new();
        let mut o_scalars: Vec<f32> = Vec::new();
        let mut o_ints: Vec<i32> = Vec::new();
        let mut o_trig: Vec<Vec<u32>> = Vec::new();
        let mut o_midi: Vec<Vec<MidiEvent>> = Vec::new();
        let mut o_spectra: Vec<SpectrumBuf> = Vec::new();
        let mut o_buffers = std::mem::take(&mut self.buffer_state);
        let mut o_slots = Vec::new();
        for spec in &self.sig.outputs {
            let slot = match spec.ty {
                SignalType::Audio | SignalType::Any => {
                    let n = match spec.channels {
                        Channels::Fixed(n) => n as usize,
                        _ => follow_ch,
                    };
                    let first = o_samples.len() as u32;
                    for _ in 0..n {
                        o_samples.push(vec![0.0; frames]);
                    }
                    OutputSlot::Audio {
                        start: first,
                        count: n as u32,
                    }
                }
                SignalType::Float | SignalType::Bool => {
                    let rate = match spec.rate {
                        OutRate::Fixed(r) => r,
                        OutRate::Poly => poly_rate,
                    };
                    if rate == Rate::Sample {
                        o_samples.push(vec![0.0; frames]);
                        OutputSlot::Float {
                            local: (o_samples.len() - 1) as u32,
                            sample: true,
                        }
                    } else {
                        o_scalars.push(0.0);
                        OutputSlot::Float {
                            local: (o_scalars.len() - 1) as u32,
                            sample: false,
                        }
                    }
                }
                SignalType::Int => {
                    o_ints.push(0);
                    OutputSlot::Int((o_ints.len() - 1) as u32)
                }
                SignalType::Trigger => {
                    o_trig.push(Vec::with_capacity(self.block.max(256) + 64));
                    OutputSlot::Trigger((o_trig.len() - 1) as u32)
                }
                SignalType::Midi => {
                    o_midi.push(Vec::with_capacity(crate::MIDI_CAPACITY));
                    OutputSlot::Midi((o_midi.len() - 1) as u32)
                }
                SignalType::Spectrum => match spec.spectrum.or(follow_shape) {
                    Some(shape) => {
                        o_spectra.push(SpectrumBuf::new(shape, shape.max_frames(self.block)));
                        OutputSlot::Spectrum((o_spectra.len() - 1) as u32)
                    }
                    None => OutputSlot::None,
                },
                SignalType::Buffer => {
                    let k = o_slots
                        .iter()
                        .filter(|s| matches!(s, OutputSlot::Buffer(_)))
                        .count();
                    if o_buffers.len() <= k {
                        o_buffers.resize(k + 1, None);
                    }
                    OutputSlot::Buffer(k as u32)
                }
            };
            o_slots.push(slot);
        }

        // --- host ---
        let in_ch = self.host_in.len();
        let mut host_input = vec![0.0; in_ch * frames];
        for (c, ch) in self.host_in.iter().enumerate() {
            host_input[c * frames..(c + 1) * frames]
                .copy_from_slice(&slice_block(ch, start, frames));
        }
        let mut host_output = vec![0.0; self.host_out_channels * frames];
        // As in the engine, only device writers can send MIDI.
        let mut host_midi_out: Vec<MidiEvent> = Vec::with_capacity(crate::MIDI_CAPACITY);
        let host_midi: Vec<MidiEvent> = self
            .host_midi
            .iter()
            .filter(|e| e.0 >= start && e.0 < end)
            .map(|e| MidiEvent::new((e.0 - start) as u32, e.1))
            .collect();

        let auto = AutoFlags {
            play: std::mem::take(&mut self.pending_play),
            stop: std::mem::take(&mut self.pending_stop),
            load: std::mem::take(&mut self.pending_load),
        };
        let transport = TransportView {
            playing: self.playing,
            recording: self.recording,
            position: self.position,
            tempo: &self.tempo,
            started: auto.play,
            stopped: auto.stop,
        };

        for &phase in &self.phases.clone() {
            for t in &mut o_trig {
                t.clear();
            }
            for m in &mut o_midi {
                m.clear();
            }
            for sp in &mut o_spectra {
                sp.clear();
            }
            let ctx = ProcessCtx {
                frames,
                sample_rate: self.sample_rate,
                transport,
                phase,
                engine: self.engine,
            };
            let mut io = Io {
                inputs: Inputs {
                    slots: &slots,
                    frames,
                    params: &params,
                    scalars: &scalars,
                    ints: &ints,
                    samples: &samples,
                    chan_table: &chan_table,
                    triggers: &trig,
                    midi: &midi,
                    spectra: &spectra,
                    buffers: &buffers,
                    auto,
                },
                outputs: Outputs {
                    slots: &o_slots,
                    frames,
                    samples: &mut o_samples,
                    scalars: &mut o_scalars,
                    ints: &mut o_ints,
                    triggers: &mut o_trig,
                    midi: &mut o_midi,
                    spectra: &mut o_spectra,
                    buffers: &mut o_buffers,
                },
                host: HostAccess {
                    input: &host_input,
                    in_channels: in_ch,
                    output: &mut host_output,
                    out_channels: self.host_out_channels,
                    stride: frames,
                    offset: 0,
                    frames,
                    midi: &host_midi,
                    midi_out: self.meta.host_output.then_some(&mut host_midi_out),
                },
                telemetry: TelemetryAccess {
                    values: &self.telemetry,
                    scope: &self.scope,
                    scope_pos: Some(&self.scope_pos),
                },
            };
            let node = &mut self.node;
            match self.guard {
                Some(g) => g(&mut || node.process(&ctx, &mut io)),
                None => node.process(&ctx, &mut io),
            }
        }

        // --- collect ---
        for (spec, slot) in self.sig.outputs.iter().zip(&o_slots) {
            let name = spec.name.to_string();
            match *slot {
                OutputSlot::Audio { start: s, count } => {
                    let entry = out
                        .audio
                        .entry(name)
                        .or_insert_with(|| vec![Vec::new(); count as usize]);
                    for c in 0..count as usize {
                        entry[c].extend_from_slice(&o_samples[s as usize + c]);
                    }
                }
                OutputSlot::Float { local, sample } => {
                    let e = out.floats.entry(name).or_default();
                    if sample {
                        e.extend_from_slice(&o_samples[local as usize]);
                    } else {
                        e.extend(std::iter::repeat_n(o_scalars[local as usize], frames));
                    }
                }
                OutputSlot::Int(k) => {
                    out.ints
                        .entry(name)
                        .or_default()
                        .extend(std::iter::repeat_n(o_ints[k as usize], frames));
                }
                OutputSlot::Trigger(k) => {
                    out.triggers
                        .entry(name)
                        .or_default()
                        .extend(o_trig[k as usize].iter().map(|&o| start + u64::from(o)));
                }
                OutputSlot::Midi(k) => {
                    out.midi.entry(name).or_default().extend(
                        o_midi[k as usize]
                            .iter()
                            .map(|e| (start + u64::from(e.offset), e.msg)),
                    );
                }
                OutputSlot::Spectrum(k) => {
                    out.spectra.entry(name).or_default().extend(
                        o_spectra[k as usize]
                            .iter()
                            .map(|(o, bins)| (start + u64::from(o), bins.to_vec())),
                    );
                }
                OutputSlot::Buffer(k) => {
                    out.buffers
                        .insert(name, o_buffers.get(k as usize).cloned().flatten());
                }
                OutputSlot::None => {}
            }
        }
        self.buffer_state = o_buffers;
        if out.host_out.len() < self.host_out_channels {
            out.host_out.resize(self.host_out_channels, Vec::new());
        }
        for c in 0..self.host_out_channels {
            out.host_out[c].extend_from_slice(&host_output[c * frames..(c + 1) * frames]);
        }
        out.host_midi_out.extend(
            host_midi_out
                .iter()
                .map(|e| (start + u64::from(e.offset), e.msg)),
        );

        self.frame = end;
        if self.playing {
            self.position += frames as u64;
        }
    }
}

fn slice_block(v: &[f32], start: u64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| v.get(start as usize + i).copied().unwrap_or(0.0))
        .collect()
}

/// Peak absolute value of a signal.
pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

/// Root-mean-square of a signal.
pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// Estimate the dominant frequency of a signal by counting rising zero
/// crossings (adequate for clean periodic test signals).
pub fn zero_crossing_freq(x: &[f32], sample_rate: f64) -> f64 {
    let mut first = None;
    let mut last = 0usize;
    let mut count = 0usize;
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            if first.is_none() {
                first = Some(i);
            } else {
                count += 1;
            }
            last = i;
        }
    }
    match first {
        Some(f) if count > 0 => count as f64 * sample_rate / (last - f) as f64,
        _ => 0.0,
    }
}

/// Magnitude of the DFT of `x` at `freq` (Goertzel), normalized so a
/// full-scale sine reads about 1.0.
pub fn tone_magnitude(x: &[f32], freq: f64, sample_rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / sample_rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.max(0.0).sqrt() / x.len().max(1) as f64
}
