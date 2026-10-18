//! Nodes the compiler inserts itself. They never appear in the registry or
//! in project files:
//!
//! - [`CompDelay`] delays one input to line it up with slower inputs
//!   (latency compensation, ARCHITECTURE §7 step 6).
//! - [`Baked`] stands in for a whole-input node and streams its bake, or
//!   silence while disabled or baking (ARCHITECTURE §9).
//! - [`Capture`] stands in for the node being baked and records its inputs.
//! - [`VoicesSum`] adds up one output of every voice of a `Voices` node.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ac_core::{
    Category, Channels, Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx,
    ProcessCtx, Rate, SignalType, Signature, SocketSpec, WholeInputs, WholeOutputs, WholeValue,
};

/// Registry-style constructor for internal nodes.
pub(crate) fn create<N: Node>(config: &Config, ctx: &PrepareCtx) -> Box<dyn Node> {
    Box::new(N::new(config, ctx))
}

// --- CompDelay ---------------------------------------------------------------

/// What a compensation delay carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompType {
    Audio,
    Float,
    Trigger,
    Midi,
}

impl CompType {
    pub fn name(self) -> &'static str {
        match self {
            CompType::Audio => "audio",
            CompType::Float => "float",
            CompType::Trigger => "trigger",
            CompType::Midi => "midi",
        }
    }

    fn parse(s: &str) -> CompType {
        match s {
            "float" => CompType::Float,
            "trigger" => CompType::Trigger,
            "midi" => CompType::Midi,
            _ => CompType::Audio,
        }
    }
}

/// Most events a trigger or MIDI compensation delay holds; more are dropped
/// (a trigger every sample for a long delay is not a patch anyone means).
const EVENT_CAPACITY: usize = 4096;

/// Delays one signal by a fixed number of samples. Config: `type`
/// (`audio`/`float`/`trigger`/`midi`), `samples`, `channels`.
pub(crate) struct CompDelay {
    ty: CompType,
    delay: usize,
    /// One ring per channel (audio, float).
    rings: Vec<Vec<f32>>,
    write: usize,
    /// Events waiting to come out, at absolute frame positions.
    triggers: VecDeque<u64>,
    midi: VecDeque<(u64, MidiMsg)>,
    /// Frames processed since creation or reset.
    clock: u64,
}

pub(crate) const COMP_DELAY_KIND: &str = "comp_delay";

impl Node for CompDelay {
    fn meta() -> NodeMeta {
        NodeMeta::new(COMP_DELAY_KIND, "Latency compensation", Category::Utility)
    }

    fn describe(config: &Config) -> Signature {
        let channels = config.get_u64_or("channels", 1).clamp(1, 64) as u32;
        let (i, o) = match CompType::parse(config.get_str_or("type", "audio")) {
            CompType::Audio => (
                SocketSpec::audio("in").channels(Channels::Fixed(channels)),
                SocketSpec::audio("out").channels(Channels::Fixed(channels)),
            ),
            CompType::Float => (
                SocketSpec::float("in", ac_core::Unit::Generic, 0.0).no_smooth(),
                SocketSpec::float("out", ac_core::Unit::Generic, 0.0).rate(Rate::Sample),
            ),
            CompType::Trigger => (SocketSpec::trigger("in"), SocketSpec::trigger("out")),
            CompType::Midi => (SocketSpec::midi("in"), SocketSpec::midi("out")),
        };
        Signature::new().input(i).output(o)
    }

    fn new(config: &Config, _ctx: &PrepareCtx) -> Self {
        let ty = CompType::parse(config.get_str_or("type", "audio"));
        let delay = config.get_u64_or("samples", 0).min(1 << 24) as usize;
        let channels = config.get_u64_or("channels", 1).clamp(1, 64) as usize;
        let rings = match ty {
            CompType::Audio => vec![vec![0.0; delay.max(1)]; channels],
            CompType::Float => vec![vec![0.0; delay.max(1)]],
            _ => Vec::new(),
        };
        let events = match ty {
            CompType::Trigger | CompType::Midi => delay.clamp(64, EVENT_CAPACITY),
            _ => 0,
        };
        CompDelay {
            ty,
            delay,
            rings,
            write: 0,
            triggers: VecDeque::with_capacity(if ty == CompType::Trigger { events } else { 0 }),
            midi: VecDeque::with_capacity(if ty == CompType::Midi { events } else { 0 }),
            clock: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let delay = self.delay as u64;
        match self.ty {
            CompType::Audio => {
                let input = io.inputs.audio(0);
                let mut out = io.outputs.audio(0);
                let write = self.write;
                for (c, ring) in self.rings.iter_mut().enumerate() {
                    let src = input.channel(c);
                    let dst = out.channel(c);
                    delay_into(ring, write, self.delay, src, dst);
                }
            }
            CompType::Float => {
                let input = io.inputs.float(0);
                let mut out = io.outputs.float(0);
                if let (Some(ring), Some(dst)) = (self.rings.first_mut(), out.as_slice_mut()) {
                    let len = ring.len();
                    let mut w = self.write;
                    for (i, o) in dst.iter_mut().enumerate() {
                        if self.delay == 0 {
                            *o = input.at(i);
                        } else {
                            *o = ring[w];
                            ring[w] = input.at(i);
                            w = (w + 1) % len;
                        }
                    }
                }
            }
            CompType::Trigger => {
                for &t in io.inputs.triggers(0) {
                    if self.triggers.len() < self.triggers.capacity() {
                        self.triggers.push_back(self.clock + u64::from(t) + delay);
                    }
                }
                let mut out = io.outputs.trigger(0);
                let end = self.clock + frames as u64;
                while let Some(&at) = self.triggers.front() {
                    if at >= end {
                        break;
                    }
                    out.push(at.saturating_sub(self.clock) as u32);
                    self.triggers.pop_front();
                }
            }
            CompType::Midi => {
                for e in io.inputs.midi(0) {
                    if self.midi.len() < self.midi.capacity() {
                        self.midi
                            .push_back((self.clock + u64::from(e.offset) + delay, e.msg));
                    }
                }
                let mut out = io.outputs.midi(0);
                let end = self.clock + frames as u64;
                while let Some(&(at, msg)) = self.midi.front() {
                    if at >= end {
                        break;
                    }
                    out.push(MidiEvent::new(at.saturating_sub(self.clock) as u32, msg));
                    self.midi.pop_front();
                }
            }
        }
        if !self.rings.is_empty() && self.delay > 0 {
            self.write = (self.write + frames) % self.rings[0].len();
        }
        self.clock += frames as u64;
    }

    fn latency(&self) -> u32 {
        // The delay lines up a path; it adds nothing on top of the slowest one.
        0
    }

    fn reset(&mut self) {
        for r in &mut self.rings {
            r.fill(0.0);
        }
        self.write = 0;
        self.triggers.clear();
        self.midi.clear();
        self.clock = 0;
    }
}

/// Ring-buffer delay of `delay` samples starting at write index `write`.
fn delay_into(ring: &mut [f32], write: usize, delay: usize, src: &[f32], dst: &mut [f32]) {
    if delay == 0 {
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        return;
    }
    let len = ring.len();
    let mut w = write;
    for (i, o) in dst.iter_mut().enumerate() {
        *o = ring[w];
        ring[w] = src.get(i).copied().unwrap_or(0.0);
        w = (w + 1) % len;
    }
}

// --- Baked ---------------------------------------------------------------------

pub(crate) const BAKED_KIND: &str = "baked";

/// Streams a whole-input node's bake at the transport position. Silent while
/// the transport is stopped, past the end of the bake, while baking, and
/// while disabled.
pub(crate) struct Baked {
    result: Option<Arc<WholeOutputs>>,
    outputs: Vec<SignalType>,
}

impl Baked {
    pub fn new(sig: &Signature, result: Option<Arc<WholeOutputs>>) -> Self {
        Baked {
            result,
            outputs: sig.outputs.iter().map(|s| s.ty).collect(),
        }
    }
}

impl Node for Baked {
    fn meta() -> NodeMeta {
        NodeMeta::new(BAKED_KIND, "Baked", Category::Utility)
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Baked {
            result: None,
            outputs: Vec::new(),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let Some(result) = self.result.as_deref() else {
            return;
        };
        if !ctx.transport.playing {
            return;
        }
        let pos = usize::try_from(ctx.transport.position).unwrap_or(usize::MAX);
        for (o, ty) in self.outputs.iter().enumerate() {
            match (ty, result.get(o)) {
                (SignalType::Audio, WholeValue::Audio(chans)) if !chans.is_empty() => {
                    let mut out = io.outputs.audio(o);
                    for c in 0..out.channels() {
                        let src = &chans[c % chans.len()];
                        copy_from(src, pos, out.channel(c));
                    }
                }
                (SignalType::Float | SignalType::Bool, WholeValue::Float(v)) => {
                    let mut out = io.outputs.float(o);
                    out.fill_with(|i| v.get(pos.saturating_add(i)).copied().unwrap_or(0.0));
                }
                _ => {}
            }
        }
    }

    fn reset(&mut self) {}
}

fn copy_from(src: &[f32], pos: usize, dst: &mut [f32]) {
    let avail = src.get(pos..).unwrap_or(&[]);
    let n = avail.len().min(dst.len());
    dst[..n].copy_from_slice(&avail[..n]);
}

// --- Capture -------------------------------------------------------------------

pub(crate) const CAPTURE_KIND: &str = "capture";

/// Records every input of the node being baked. Only runs inside a bake,
/// never on the audio thread, so it may allocate.
pub(crate) struct Capture {
    sink: Arc<Mutex<WholeInputs>>,
    types: Vec<SignalType>,
    frames: u64,
}

impl Capture {
    pub fn new(sig: &Signature, sink: Arc<Mutex<WholeInputs>>, frames: usize) -> Self {
        if let Ok(mut s) = sink.lock() {
            s.values = sig
                .inputs
                .iter()
                .map(|spec| match spec.ty {
                    SignalType::Audio => WholeValue::Audio(Vec::new()),
                    SignalType::Float | SignalType::Bool => {
                        WholeValue::Float(Vec::with_capacity(frames))
                    }
                    SignalType::Int => WholeValue::Int(Vec::with_capacity(frames)),
                    SignalType::Trigger => WholeValue::Triggers(Vec::new()),
                    SignalType::Midi => WholeValue::Midi(Vec::new()),
                    _ => WholeValue::None,
                })
                .collect();
        }
        Capture {
            sink,
            types: sig.inputs.iter().map(|s| s.ty).collect(),
            frames: 0,
        }
    }
}

impl Node for Capture {
    fn meta() -> NodeMeta {
        NodeMeta::new(CAPTURE_KIND, "Capture", Category::Utility)
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Capture {
            sink: Arc::new(Mutex::new(WholeInputs::default())),
            types: Vec::new(),
            frames: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let start = self.frames;
        let Ok(mut sink) = self.sink.lock() else {
            return;
        };
        for (i, ty) in self.types.iter().enumerate() {
            let Some(value) = sink.values.get_mut(i) else {
                continue;
            };
            match (ty, value) {
                (SignalType::Audio, WholeValue::Audio(chans)) => {
                    let input = io.inputs.audio(i);
                    if chans.is_empty() {
                        chans.resize(input.channels(), Vec::new());
                    }
                    for (c, ch) in chans.iter_mut().enumerate() {
                        ch.extend_from_slice(input.channel(c));
                    }
                }
                (SignalType::Float | SignalType::Bool, WholeValue::Float(v)) => {
                    let input = io.inputs.float(i);
                    v.extend((0..frames).map(|k| input.at(k)));
                }
                (SignalType::Int, WholeValue::Int(v)) => {
                    let x = io.inputs.int(i);
                    v.extend(std::iter::repeat_n(x, frames));
                }
                (SignalType::Trigger, WholeValue::Triggers(v)) => {
                    v.extend(io.inputs.triggers(i).iter().map(|&t| start + u64::from(t)));
                }
                (SignalType::Midi, WholeValue::Midi(v)) => {
                    v.extend(
                        io.inputs
                            .midi(i)
                            .iter()
                            .map(|e| (start + u64::from(e.offset), e.msg)),
                    );
                }
                _ => {}
            }
        }
        self.frames += frames as u64;
    }

    fn reset(&mut self) {}
}
// --- VoicesSum -----------------------------------------------------------------

pub(crate) const VOICES_SUM_KIND: &str = "voices_sum";

/// Adds up `voices` inputs (`in_0`, `in_1`, ...) of one type (`type`:
/// `audio` or `float`), in voice order.
pub(crate) struct VoicesSum {
    inputs: usize,
    audio: bool,
}

impl Node for VoicesSum {
    fn meta() -> NodeMeta {
        NodeMeta::new(VOICES_SUM_KIND, "Voices sum", Category::Utility)
    }

    fn describe(config: &Config) -> Signature {
        let n = config.get_u64_or("voices", 1).clamp(1, 64);
        let audio = config.get_str_or("type", "audio") == "audio";
        let mut sig = Signature::new();
        for v in 0..n {
            sig = sig.input(if audio {
                SocketSpec::audio(format!("in_{v}"))
            } else {
                SocketSpec::float(format!("in_{v}"), ac_core::Unit::Generic, 0.0).no_smooth()
            });
        }
        sig.output(if audio {
            SocketSpec::audio("out")
        } else {
            SocketSpec::float("out", ac_core::Unit::Generic, 0.0).poly()
        })
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        VoicesSum {
            inputs: config.get_u64_or("voices", 1).clamp(1, 64) as usize,
            audio: config.get_str_or("type", "audio") == "audio",
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        if self.audio {
            let mut out = io.outputs.audio(0);
            for v in 0..self.inputs {
                let input = io.inputs.audio(v);
                for c in 0..out.channels() {
                    let src = input.channel(c);
                    for (o, s) in out.channel(c).iter_mut().zip(src) {
                        *o += s;
                    }
                }
            }
        } else {
            let ins = &io.inputs;
            let n = self.inputs;
            io.outputs
                .float(0)
                .fill_with(|i| (0..n).map(|v| ins.float(v).at(i)).sum());
        }
    }

    fn reset(&mut self) {}
}
