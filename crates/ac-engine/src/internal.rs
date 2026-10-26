//! Nodes the compiler inserts itself. They never appear in the registry or
//! in project files:
//!
//! - [`CompDelay`] delays one input to line it up with slower inputs
//!   (latency compensation, ARCHITECTURE §7 step 6).
//! - [`Baked`] stands in for a whole-input node and streams its bake, or
//!   silence while disabled or baking (ARCHITECTURE §9).
//! - [`Capture`] stands in for the node being baked and records its inputs.
//! - [`VoicesSum`] adds up one output of every voice of a `Voices` node.
//! - [`LateRead`] hands a late input last block's value (DECISIONS D205).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ac_core::{
    Category, Channels, Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, Phase,
    PrepareCtx, ProcessCtx, Rate, SignalType, Signature, SocketSpec, WholeInputs, WholeOutputs,
    WholeValue,
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

// --- LateRead ------------------------------------------------------------------

pub(crate) const LATE_READ_KIND: &str = "late_read";

/// Most events a late trigger or MIDI input carries over from one block to
/// the next; more are dropped (a light needs the last few, not thousands).
const LATE_EVENTS: usize = 256;

/// One block of delay in front of a late input (`type`: `audio`, `float`,
/// `bool`, `trigger` or `midi`; `channels` for audio). It's a Feedback node
/// for any of those types: the compiler splits it into a read step with no
/// dependencies and a write step after its source, which is what lets a
/// display input be wired from downstream without making a cycle. Unlike
/// Feedback, a value holds past the end of what was stored, so a light
/// doesn't flicker when blocks change size.
pub(crate) struct LateRead {
    ty: SignalType,
    /// One buffer per channel (audio), or one (float, bool).
    values: Vec<Vec<f32>>,
    stored: usize,
    triggers: Vec<u32>,
    midi: Vec<MidiEvent>,
}

impl LateRead {
    fn parse(config: &Config) -> (SignalType, u32) {
        let ty = match config.get_str_or("type", "float") {
            "audio" => SignalType::Audio,
            "bool" => SignalType::Bool,
            "trigger" => SignalType::Trigger,
            "midi" => SignalType::Midi,
            _ => SignalType::Float,
        };
        (ty, config.get_u64_or("channels", 1).clamp(1, 64) as u32)
    }

    /// The config for a late read in front of `spec`, if it can be late.
    pub fn config_for(spec: &SocketSpec) -> Option<Config> {
        let ty = match spec.ty {
            SignalType::Audio => "audio",
            SignalType::Float => "float",
            SignalType::Bool => "bool",
            SignalType::Trigger => "trigger",
            SignalType::Midi => "midi",
            _ => return None,
        };
        // The read side has no input to follow, so its channels are fixed:
        // as many as the input takes.
        let channels = match spec.channels {
            Channels::Fixed(n) | Channels::UpTo(n) => n,
            _ => 2,
        };
        Some(serde_json::json!({ "type": ty, "channels": channels }))
    }

    fn read(&self, io: &mut Io) {
        let last = io.frames().saturating_sub(1) as u32;
        match self.ty {
            SignalType::Trigger => {
                let mut out = io.outputs.trigger(0);
                for &t in &self.triggers {
                    out.push(t.min(last));
                }
            }
            SignalType::Midi => {
                let mut out = io.outputs.midi(0);
                for e in &self.midi {
                    out.push(MidiEvent::new(e.offset.min(last), e.msg));
                }
            }
            SignalType::Audio => {
                let mut out = io.outputs.audio(0);
                for (c, dst) in out.iter_mut().enumerate() {
                    let src = self
                        .values
                        .get(c)
                        .and_then(|v| v.get(..self.stored))
                        .unwrap_or(&[]);
                    let n = src.len().min(dst.len());
                    dst[..n].copy_from_slice(&src[..n]);
                    dst[n..].fill(0.0);
                }
            }
            _ => {
                let stored = self
                    .values
                    .first()
                    .and_then(|v| v.get(..self.stored))
                    .unwrap_or(&[]);
                let held = stored.last().copied().unwrap_or(0.0);
                io.outputs
                    .float(0)
                    .fill_with(|i| stored.get(i).copied().unwrap_or(held));
            }
        }
    }

    fn write(&mut self, io: &mut Io) {
        let capacity = self.values.first().map_or(0, Vec::len);
        let frames = io.frames().min(capacity);
        match self.ty {
            SignalType::Trigger => {
                self.triggers.clear();
                for &t in io.inputs.triggers(0).iter().take(LATE_EVENTS) {
                    self.triggers.push(t);
                }
            }
            SignalType::Midi => {
                self.midi.clear();
                for e in io.inputs.midi(0).iter().take(LATE_EVENTS) {
                    self.midi.push(*e);
                }
            }
            SignalType::Audio => {
                let input = io.inputs.audio(0);
                for (c, buf) in self.values.iter_mut().enumerate() {
                    let src = input.channel(c);
                    for (i, v) in buf[..frames].iter_mut().enumerate() {
                        let x = src.get(i).copied().unwrap_or(0.0);
                        *v = if x.is_finite() { x } else { 0.0 };
                    }
                }
                self.stored = frames;
            }
            _ => {
                let input = io.inputs.float(0);
                if let Some(buf) = self.values.first_mut() {
                    for (i, v) in buf[..frames].iter_mut().enumerate() {
                        let x = input.at(i);
                        *v = if x.is_finite() { x } else { 0.0 };
                    }
                }
                self.stored = frames;
            }
        }
    }
}

impl Node for LateRead {
    fn meta() -> NodeMeta {
        NodeMeta::new(LATE_READ_KIND, "Late read", Category::Utility).feedback()
    }

    fn describe(config: &Config) -> Signature {
        let (ty, channels) = Self::parse(config);
        let (i, o) = match ty {
            SignalType::Audio => (
                SocketSpec::audio("in").channels(Channels::Fixed(channels)),
                SocketSpec::audio("out").channels(Channels::Fixed(channels)),
            ),
            SignalType::Bool => (
                SocketSpec::boolean("in", false),
                SocketSpec::boolean("out", false),
            ),
            SignalType::Trigger => (SocketSpec::trigger("in"), SocketSpec::trigger("out")),
            SignalType::Midi => (SocketSpec::midi("in"), SocketSpec::midi("out")),
            _ => (
                SocketSpec::float("in", ac_core::Unit::Generic, 0.0).no_smooth(),
                SocketSpec::float("out", ac_core::Unit::Generic, 0.0).rate(Rate::Sample),
            ),
        };
        Signature::new().input(i).output(o)
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let (ty, channels) = Self::parse(config);
        let buffers = match ty {
            SignalType::Audio => channels as usize,
            SignalType::Float | SignalType::Bool => 1,
            _ => 0,
        };
        LateRead {
            ty,
            values: vec![vec![0.0; ctx.max_block.max(1)]; buffers],
            stored: 0,
            triggers: Vec::with_capacity(if ty == SignalType::Trigger {
                LATE_EVENTS
            } else {
                0
            }),
            midi: Vec::with_capacity(if ty == SignalType::Midi {
                LATE_EVENTS
            } else {
                0
            }),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        match ctx.phase {
            Phase::FeedbackRead => self.read(io),
            Phase::FeedbackWrite => self.write(io),
            Phase::Main => {
                self.read(io);
                self.write(io);
            }
        }
    }

    fn reset(&mut self) {
        for v in &mut self.values {
            v.fill(0.0);
        }
        self.stored = 0;
        self.triggers.clear();
        self.midi.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    const SPLIT: &[Phase] = &[Phase::FeedbackRead, Phase::FeedbackWrite];

    #[test]
    fn a_late_value_arrives_one_block_later_and_holds() {
        let mut h = Harness::new::<LateRead>(json!({ "type": "bool" }));
        h.phases(SPLIT);
        let mut x = vec![0.0; 128];
        x[100] = 1.0;
        x[127] = 1.0;
        h.set_samples("in", x);
        let out = h.run(192);
        let y = out.float("out");
        assert_eq!(y[64 + 100], 1.0);
        assert_eq!(y[64 + 99], 0.0);
        // Past what was stored, the last value holds rather than dropping.
        assert_eq!(y[191], 1.0);
    }

    #[test]
    fn late_audio_arrives_one_block_later() {
        let mut h = Harness::new::<LateRead>(json!({ "type": "audio", "channels": 2 }));
        h.phases(SPLIT);
        let x: Vec<f32> = (1..=128).map(|i| i as f32).collect();
        h.set_audio("in", vec![x.clone(), x.clone()]);
        let out = h.run(128);
        let y = &out.audio("out")[1];
        assert_eq!(&y[..64], &[0.0; 64]);
        assert_eq!(&y[64..], &x[..64]);
    }

    #[test]
    fn late_events_come_out_in_the_next_block() {
        // Main phase: read then write in one call, as when unsplit (the
        // harness clears event outputs between split phases).
        let mut h = Harness::new::<LateRead>(json!({ "type": "midi" }));
        let on = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        };
        h.midi("in", &[(10, on)]);
        let out = h.run(128);
        let events = out.midi("out");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 64 + 10);
    }
}
