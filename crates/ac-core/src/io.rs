//! Typed, allocation-free access to a node's inputs and outputs for one block.
//!
//! The engine (and the test harness) builds an [`Io`] per node per block from
//! compiled slot tables. Nodes read inputs through accessors that hide the
//! rate (`Const`/`Block`/`Sample`) and write outputs through typed handles.
//! Inputs and outputs are separate fields so a node can hold an input and an
//! output at the same time:
//!
//! ```ignore
//! let input = io.inputs.audio(IN);
//! let gain = io.inputs.float(GAIN);
//! let mut out = io.outputs.audio(OUT);
//! ```
//!
//! Every accessor tolerates a bad index or a short buffer by returning silence
//! or zero instead of panicking: this code runs on the audio thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::control::{ControlEvent, OscMessage};
use crate::events::MidiEvent;
use crate::node::AudioData;
use crate::signal::{AutoTrigger, Rate};
use crate::spectrum::{EMPTY_SPECTRUM, SpectrumBuf};
use crate::text::TextBuf;

/// The value a `Buffer` wire holds: a shared handle to audio, or nothing.
///
/// Cloning and dropping handles on the audio thread is fine (an atomic
/// count); whoever creates a buffer keeps a handle of its own off the audio
/// thread, so the last one is never dropped there (DECISIONS D53).
pub type BufferRef = Option<Arc<AudioData>>;

/// A buffer of zeros, for unwired inputs and out-of-range reads.
const SILENCE: &[f32] = &[0.0; 4096];
/// One trigger at frame 0.
const TRIGGER_AT_ZERO: &[u32] = &[0];

/// Where a `Float` or `Bool` input reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatSrc {
    /// Unwired: a parameter slot (possibly ramping).
    Param(u32),
    /// A block-rate value.
    Scalar(u32),
    /// A sample-rate buffer.
    Sample(u32),
    /// An `Int` value converted to float.
    Int(u32),
}

/// Where an `Int` input reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntSrc {
    Param(u32),
    Int(u32),
}

/// Where a `Text` input reads from: a slot in the text table, holding
/// either the typed value (fixed at compile time) or a wired output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSrc {
    Const(u32),
    Wired(u32),
}

/// Where a `Trigger` input reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerSrc {
    Wired(u32),
    /// Unwired: fires from the inline button (the param's `fired` flag) or,
    /// for generator start/stop sockets, from transport events chosen by the
    /// param's value.
    Unwired {
        param: u32,
        auto: AutoTrigger,
    },
}

/// Compiled binding of one input socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSlot {
    Float(FloatSrc),
    Int(IntSrc),
    /// A range in the channel table; each entry is a sample-buffer index.
    Audio {
        start: u32,
        count: u32,
    },
    Trigger(TriggerSrc),
    Midi(Option<u32>),
    Spectrum(Option<u32>),
    Buffer(Option<u32>),
    Text(TextSrc),
    /// Nothing (e.g. an unresolved `Any` socket).
    None,
}

/// Compiled binding of one output socket. Indices are local to the step:
/// the engine hands the node only its own output buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputSlot {
    /// `Float` or `Bool`; `sample` selects the local sample buffers vs. scalars.
    Float {
        local: u32,
        sample: bool,
    },
    Int(u32),
    Audio {
        start: u32,
        count: u32,
    },
    Trigger(u32),
    Midi(u32),
    Spectrum(u32),
    Buffer(u32),
    Text(u32),
    None,
}

/// Runtime state of an unwired input's value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParamSlot {
    /// Value at the start of this block (choice index for enumerated inputs,
    /// start/stop mode index for generator triggers).
    pub value: f32,
    /// Per-sample increment while smoothing.
    pub step: f32,
    /// Samples of smoothing left (0 = constant).
    pub remaining: u32,
    /// Where smoothing ends.
    pub target: f32,
    /// The inline trigger button was pressed for this block.
    pub fired: bool,
}

/// Transport events that drive unwired generator start/stop sockets this block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutoFlags {
    /// Transport started (or looped) at frame 0.
    pub play: bool,
    /// Transport stopped at frame 0.
    pub stop: bool,
    /// First block after this node was created or reset.
    pub load: bool,
}

/// A `Float` input for one block, whatever its rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloatIn<'a> {
    Const(f32),
    /// A smoothed parameter change from about `start` to `end` over `len`
    /// frames. Frame `i < len` is `end - step * (len - i)`: counted back
    /// from the end, so a ramp split across blocks of any size gives the
    /// same value at the same sample.
    Ramp {
        start: f32,
        step: f32,
        len: u32,
        end: f32,
    },
    Block(f32),
    Sample(&'a [f32]),
}

impl FloatIn<'_> {
    /// Value at frame `i`.
    #[inline]
    pub fn at(&self, i: usize) -> f32 {
        match *self {
            FloatIn::Const(v) | FloatIn::Block(v) => v,
            FloatIn::Ramp { step, len, end, .. } => {
                if i < len as usize {
                    end - step * (len as usize - i) as f32
                } else {
                    end
                }
            }
            FloatIn::Sample(s) => s.get(i).copied().unwrap_or(0.0),
        }
    }

    /// Value at the first frame (what a block-rate consumer uses).
    pub fn first(&self) -> f32 {
        self.at(0)
    }

    /// True when the value can change within the block.
    pub fn is_varying(&self) -> bool {
        matches!(self, FloatIn::Ramp { .. } | FloatIn::Sample(_))
    }

    /// The effective rate of this input.
    pub fn rate(&self) -> Rate {
        match self {
            FloatIn::Const(_) => Rate::Const,
            FloatIn::Block(_) => Rate::Block,
            FloatIn::Ramp { .. } | FloatIn::Sample(_) => Rate::Sample,
        }
    }

    /// Copy the block's values into `out` (up to `out.len()` frames).
    pub fn fill(&self, out: &mut [f32]) {
        match *self {
            FloatIn::Const(v) | FloatIn::Block(v) => out.fill(v),
            FloatIn::Sample(s) => {
                let n = out.len().min(s.len());
                out[..n].copy_from_slice(&s[..n]);
                out[n..].fill(0.0);
            }
            FloatIn::Ramp { .. } => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = self.at(i);
                }
            }
        }
    }
}

/// A `Bool` input: a float read with a 0.5 threshold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoolIn<'a>(pub FloatIn<'a>);

impl BoolIn<'_> {
    #[inline]
    pub fn at(&self, i: usize) -> bool {
        self.0.at(i) > 0.5
    }

    pub fn first(&self) -> bool {
        self.at(0)
    }

    pub fn is_varying(&self) -> bool {
        self.0.is_varying()
    }
}

/// An `Audio` input: 1..N channels of `frames` samples.
#[derive(Clone, Copy, Debug)]
pub struct AudioIn<'a> {
    samples: &'a [Vec<f32>],
    chans: &'a [u32],
    frames: usize,
}

impl<'a> AudioIn<'a> {
    pub fn new(samples: &'a [Vec<f32>], chans: &'a [u32], frames: usize) -> Self {
        AudioIn {
            samples,
            chans,
            frames,
        }
    }

    /// A one-channel silent input.
    pub fn silent(frames: usize) -> AudioIn<'static> {
        AudioIn {
            samples: &[],
            chans: &[],
            frames,
        }
    }

    /// Channel count (at least 1).
    pub fn channels(&self) -> usize {
        self.chans.len().max(1)
    }

    /// Channel `c`. Asking for more channels than exist wraps around, which is
    /// mono upmix for a one-channel input.
    #[inline]
    pub fn channel(&self, c: usize) -> &'a [f32] {
        if self.chans.is_empty() {
            return silence(self.frames);
        }
        let idx = self.chans[c % self.chans.len()] as usize;
        match self.samples.get(idx) {
            Some(buf) => buf.get(..self.frames).unwrap_or(buf),
            None => silence(self.frames),
        }
    }

    pub fn frames(&self) -> usize {
        self.frames
    }
}

fn silence(frames: usize) -> &'static [f32] {
    &SILENCE[..frames.min(SILENCE.len())]
}

/// Read access to a node's inputs for one block.
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    pub slots: &'a [InputSlot],
    pub frames: usize,
    pub params: &'a [ParamSlot],
    pub scalars: &'a [f32],
    pub ints: &'a [i32],
    pub samples: &'a [Vec<f32>],
    pub chan_table: &'a [u32],
    pub triggers: &'a [Vec<u32>],
    pub midi: &'a [Vec<MidiEvent>],
    pub spectra: &'a [SpectrumBuf],
    pub buffers: &'a [BufferRef],
    pub texts: &'a [TextBuf],
    pub auto: AutoFlags,
}

impl<'a> Inputs<'a> {
    fn slot(&self, i: usize) -> InputSlot {
        self.slots.get(i).copied().unwrap_or(InputSlot::None)
    }

    /// A `Float` (or `Bool`) input.
    pub fn float(&self, i: usize) -> FloatIn<'a> {
        match self.slot(i) {
            InputSlot::Float(src) => self.float_src(src),
            InputSlot::Int(IntSrc::Int(k)) => FloatIn::Block(self.int_at(k) as f32),
            InputSlot::Int(IntSrc::Param(p)) => FloatIn::Const(self.param(p).value),
            InputSlot::Audio { start, count } if count > 0 => {
                // Audio → Float: channel 0 at sample rate.
                FloatIn::Sample(
                    AudioIn::new(self.samples, self.chans(start, count), self.frames).channel(0),
                )
            }
            _ => FloatIn::Const(0.0),
        }
    }

    fn float_src(&self, src: FloatSrc) -> FloatIn<'a> {
        match src {
            FloatSrc::Param(p) => {
                let s = self.param(p);
                if s.remaining > 0 {
                    FloatIn::Ramp {
                        start: s.value,
                        step: s.step,
                        len: s.remaining,
                        end: s.target,
                    }
                } else {
                    FloatIn::Const(s.value)
                }
            }
            FloatSrc::Scalar(k) => {
                FloatIn::Block(self.scalars.get(k as usize).copied().unwrap_or(0.0))
            }
            FloatSrc::Sample(k) => match self.samples.get(k as usize) {
                Some(buf) => FloatIn::Sample(buf.get(..self.frames).unwrap_or(buf)),
                None => FloatIn::Const(0.0),
            },
            FloatSrc::Int(k) => FloatIn::Block(self.int_at(k) as f32),
        }
    }

    /// A `Bool` input.
    pub fn boolean(&self, i: usize) -> BoolIn<'a> {
        BoolIn(self.float(i))
    }

    /// An `Int` input (block rate).
    pub fn int(&self, i: usize) -> i32 {
        match self.slot(i) {
            InputSlot::Int(IntSrc::Param(p)) => self.param(p).value.round() as i32,
            InputSlot::Int(IntSrc::Int(k)) => self.int_at(k),
            InputSlot::Float(_) => self.float(i).first().round() as i32,
            _ => 0,
        }
    }

    /// An enumerated input as a choice index in `0..n`.
    pub fn choice(&self, i: usize, n: usize) -> usize {
        let v = self.int(i).max(0) as usize;
        v.min(n.saturating_sub(1))
    }

    /// An `Audio` input.
    pub fn audio(&self, i: usize) -> AudioIn<'a> {
        match self.slot(i) {
            InputSlot::Audio { start, count } => {
                AudioIn::new(self.samples, self.chans(start, count), self.frames)
            }
            _ => AudioIn::silent(self.frames),
        }
    }

    /// Trigger offsets in this block, ascending.
    pub fn triggers(&self, i: usize) -> &'a [u32] {
        match self.slot(i) {
            InputSlot::Trigger(TriggerSrc::Wired(k)) => {
                self.triggers.get(k as usize).map_or(&[], Vec::as_slice)
            }
            InputSlot::Trigger(TriggerSrc::Unwired { param, auto }) => {
                let p = self.param(param);
                let mode = p.value.round() as i32;
                let auto_fire = match auto {
                    AutoTrigger::None => false,
                    // START_CHOICES: play, load, never
                    AutoTrigger::Start => match mode {
                        0 => self.auto.play,
                        1 => self.auto.load,
                        _ => false,
                    },
                    // STOP_CHOICES: stop, never
                    AutoTrigger::Stop => mode == 0 && self.auto.stop,
                };
                if p.fired || auto_fire {
                    TRIGGER_AT_ZERO
                } else {
                    &[]
                }
            }
            _ => &[],
        }
    }

    /// MIDI events in this block, ascending by offset.
    pub fn midi(&self, i: usize) -> &'a [MidiEvent] {
        match self.slot(i) {
            InputSlot::Midi(Some(k)) => self.midi.get(k as usize).map_or(&[], Vec::as_slice),
            _ => &[],
        }
    }

    /// The spectrum frames in this block (empty when unwired).
    pub fn spectrum(&self, i: usize) -> &'a SpectrumBuf {
        match self.slot(i) {
            InputSlot::Spectrum(Some(k)) => self.spectra.get(k as usize).unwrap_or(&EMPTY_SPECTRUM),
            _ => &EMPTY_SPECTRUM,
        }
    }

    /// The buffer a `Buffer` input holds, if any.
    pub fn buffer(&self, i: usize) -> Option<&'a Arc<AudioData>> {
        match self.slot(i) {
            InputSlot::Buffer(Some(k)) => self.buffers.get(k as usize).and_then(Option::as_ref),
            _ => None,
        }
    }

    /// A `Text` input: the typed value, or the wired output's value this
    /// block. Empty for anything else.
    pub fn text(&self, i: usize) -> &'a TextBuf {
        match self.slot(i) {
            InputSlot::Text(TextSrc::Const(k) | TextSrc::Wired(k)) => {
                self.texts.get(k as usize).unwrap_or(&TextBuf::EMPTY)
            }
            _ => &TextBuf::EMPTY,
        }
    }

    /// True if a wire feeds this input.
    pub fn is_wired(&self, i: usize) -> bool {
        match self.slot(i) {
            InputSlot::Float(FloatSrc::Param(_)) | InputSlot::Int(IntSrc::Param(_)) => false,
            InputSlot::Text(TextSrc::Const(_)) => false,
            InputSlot::Trigger(TriggerSrc::Unwired { .. }) | InputSlot::Midi(None) => false,
            InputSlot::Spectrum(None) | InputSlot::Buffer(None) => false,
            InputSlot::Audio { count, .. } => count > 0 && !self.is_silence_audio(i),
            InputSlot::None => false,
            _ => true,
        }
    }

    fn is_silence_audio(&self, i: usize) -> bool {
        match self.slot(i) {
            InputSlot::Audio { start, count } => self.chans(start, count).iter().all(|&b| b == 0),
            _ => true,
        }
    }

    fn param(&self, p: u32) -> ParamSlot {
        self.params.get(p as usize).copied().unwrap_or_default()
    }

    fn int_at(&self, k: u32) -> i32 {
        self.ints.get(k as usize).copied().unwrap_or(0)
    }

    fn chans(&self, start: u32, count: u32) -> &'a [u32] {
        let s = start as usize;
        let e = s + count as usize;
        self.chan_table.get(s..e).unwrap_or(&[])
    }
}

/// A `Float`/`Bool` output for one block.
#[derive(Debug)]
pub enum FloatOut<'a> {
    Sample(&'a mut [f32]),
    Block(&'a mut f32),
    None,
}

impl FloatOut<'_> {
    /// Set every frame (or the block value) to `v`.
    pub fn fill(&mut self, v: f32) {
        match self {
            FloatOut::Sample(s) => s.fill(v),
            FloatOut::Block(b) => **b = v,
            FloatOut::None => {}
        }
    }

    /// Compute each frame with `f(i)`. A block-rate output calls `f(0)` once.
    #[inline]
    pub fn fill_with(&mut self, mut f: impl FnMut(usize) -> f32) {
        match self {
            FloatOut::Sample(s) => {
                for (i, o) in s.iter_mut().enumerate() {
                    *o = f(i);
                }
            }
            FloatOut::Block(b) => **b = f(0),
            FloatOut::None => {}
        }
    }

    /// Set frame `i`. On a block-rate output only frame 0 counts.
    #[inline]
    pub fn set(&mut self, i: usize, v: f32) {
        match self {
            FloatOut::Sample(s) => {
                if let Some(o) = s.get_mut(i) {
                    *o = v;
                }
            }
            FloatOut::Block(b) => {
                if i == 0 {
                    **b = v;
                }
            }
            FloatOut::None => {}
        }
    }

    pub fn is_sample(&self) -> bool {
        matches!(self, FloatOut::Sample(_))
    }

    /// The sample buffer, if this output runs at sample rate.
    pub fn as_slice_mut(&mut self) -> Option<&mut [f32]> {
        match self {
            FloatOut::Sample(s) => Some(s),
            _ => None,
        }
    }
}

/// An `Audio` output: `channels()` buffers of `frames` samples.
#[derive(Debug)]
pub struct AudioOut<'a> {
    chans: &'a mut [Vec<f32>],
    frames: usize,
}

impl AudioOut<'_> {
    pub fn channels(&self) -> usize {
        self.chans.len()
    }

    /// Channel `c`, or an empty slice if it doesn't exist.
    #[inline]
    pub fn channel(&mut self, c: usize) -> &mut [f32] {
        let frames = self.frames;
        match self.chans.get_mut(c) {
            Some(buf) => {
                let n = frames.min(buf.len());
                &mut buf[..n]
            }
            None => &mut [],
        }
    }

    /// Iterate over channel buffers.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        let frames = self.frames;
        self.chans.iter_mut().map(move |b| {
            let n = frames.min(b.len());
            &mut b[..n]
        })
    }

    /// The first two channels at once, if there are at least two.
    pub fn stereo(&mut self) -> Option<(&mut [f32], &mut [f32])> {
        let frames = self.frames;
        match self.chans {
            [l, r, ..] => {
                let nl = frames.min(l.len());
                let nr = frames.min(r.len());
                Some((&mut l[..nl], &mut r[..nr]))
            }
            _ => None,
        }
    }

    pub fn fill(&mut self, v: f32) {
        for c in self.iter_mut() {
            c.fill(v);
        }
    }
}

/// A `Trigger` output.
#[derive(Debug)]
pub struct TriggerOut<'a> {
    buf: Option<&'a mut Vec<u32>>,
}

impl TriggerOut<'_> {
    /// Emit a trigger at `offset`. Keeps offsets ascending; drops the event if
    /// the preallocated buffer is full (never reallocates).
    #[inline]
    pub fn push(&mut self, offset: u32) {
        if let Some(buf) = self.buf.as_deref_mut() {
            if buf.len() == buf.capacity() {
                return;
            }
            let pos = buf.partition_point(|&o| o <= offset);
            buf.insert(pos, offset);
        }
    }

    pub fn len(&self) -> usize {
        self.buf.as_ref().map_or(0, |b| b.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A `Midi` output.
#[derive(Debug)]
pub struct MidiOut<'a> {
    buf: Option<&'a mut Vec<MidiEvent>>,
}

impl MidiOut<'_> {
    /// Emit an event, keeping offsets ascending; drops it if the buffer is full.
    pub fn push(&mut self, ev: MidiEvent) {
        if let Some(buf) = self.buf.as_deref_mut() {
            if buf.len() == buf.capacity() {
                return;
            }
            let pos = buf.partition_point(|e| e.offset <= ev.offset);
            buf.insert(pos, ev);
        }
    }
}

/// Write access to a node's outputs for one block.
#[derive(Debug)]
pub struct Outputs<'a> {
    pub slots: &'a [OutputSlot],
    pub frames: usize,
    /// This step's sample buffers (float outputs and audio channels).
    pub samples: &'a mut [Vec<f32>],
    pub scalars: &'a mut [f32],
    pub ints: &'a mut [i32],
    pub triggers: &'a mut [Vec<u32>],
    pub midi: &'a mut [Vec<MidiEvent>],
    pub spectra: &'a mut [SpectrumBuf],
    pub buffers: &'a mut [BufferRef],
    pub texts: &'a mut [TextBuf],
}

impl Outputs<'_> {
    fn slot(&self, i: usize) -> OutputSlot {
        self.slots.get(i).copied().unwrap_or(OutputSlot::None)
    }

    /// A `Float` or `Bool` output.
    pub fn float(&mut self, i: usize) -> FloatOut<'_> {
        let frames = self.frames;
        match self.slot(i) {
            OutputSlot::Float {
                local,
                sample: true,
            } => match self.samples.get_mut(local as usize) {
                Some(b) => {
                    let n = frames.min(b.len());
                    FloatOut::Sample(&mut b[..n])
                }
                None => FloatOut::None,
            },
            OutputSlot::Float {
                local,
                sample: false,
            } => match self.scalars.get_mut(local as usize) {
                Some(v) => FloatOut::Block(v),
                None => FloatOut::None,
            },
            _ => FloatOut::None,
        }
    }

    /// Fill a `Float` output with one value.
    pub fn set_float(&mut self, i: usize, v: f32) {
        self.float(i).fill(v);
    }

    /// Fill a `Bool` output with one value.
    pub fn set_bool(&mut self, i: usize, v: bool) {
        self.float(i).fill(if v { 1.0 } else { 0.0 });
    }

    /// Set an `Int` output.
    pub fn set_int(&mut self, i: usize, v: i32) {
        if let OutputSlot::Int(k) = self.slot(i)
            && let Some(o) = self.ints.get_mut(k as usize)
        {
            *o = v;
        }
    }

    /// A `Text` output, to write into (it keeps last block's value until
    /// cleared or overwritten).
    pub fn text(&mut self, i: usize) -> Option<&mut TextBuf> {
        match self.slot(i) {
            OutputSlot::Text(k) => self.texts.get_mut(k as usize),
            _ => None,
        }
    }

    /// Set a `Text` output.
    pub fn set_text(&mut self, i: usize, s: &str) {
        if let Some(t) = self.text(i) {
            t.clear();
            t.push_str(s);
        }
    }

    /// An `Audio` output.
    pub fn audio(&mut self, i: usize) -> AudioOut<'_> {
        let frames = self.frames;
        match self.slot(i) {
            OutputSlot::Audio { start, count } => {
                let s = start as usize;
                let e = s + count as usize;
                match self.samples.get_mut(s..e) {
                    Some(chans) => AudioOut { chans, frames },
                    None => AudioOut {
                        chans: &mut [],
                        frames,
                    },
                }
            }
            _ => AudioOut {
                chans: &mut [],
                frames,
            },
        }
    }

    /// A `Trigger` output.
    pub fn trigger(&mut self, i: usize) -> TriggerOut<'_> {
        match self.slot(i) {
            OutputSlot::Trigger(k) => TriggerOut {
                buf: self.triggers.get_mut(k as usize),
            },
            _ => TriggerOut { buf: None },
        }
    }

    /// A `Midi` output.
    pub fn midi(&mut self, i: usize) -> MidiOut<'_> {
        match self.slot(i) {
            OutputSlot::Midi(k) => MidiOut {
                buf: self.midi.get_mut(k as usize),
            },
            _ => MidiOut { buf: None },
        }
    }

    /// A `Spectrum` output (cleared at the start of each block), if connected.
    pub fn spectrum(&mut self, i: usize) -> Option<&mut SpectrumBuf> {
        match self.slot(i) {
            OutputSlot::Spectrum(k) => self.spectra.get_mut(k as usize),
            _ => None,
        }
    }

    /// Set a `Buffer` output. The value persists from block to block, but a
    /// recompile starts from none, so set it every block.
    pub fn set_buffer(&mut self, i: usize, b: Option<&Arc<AudioData>>) {
        if let OutputSlot::Buffer(k) = self.slot(i)
            && let Some(slot) = self.buffers.get_mut(k as usize)
        {
            let same = match (slot.as_ref(), b) {
                (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                (None, None) => true,
                _ => false,
            };
            if !same {
                *slot = b.cloned();
            }
        }
    }

    /// Two `Float`/`Bool` outputs at once (for loops that write both per sample).
    pub fn float2(&mut self, a: usize, b: usize) -> (FloatOut<'_>, FloatOut<'_>) {
        let frames = self.frames;
        let (sa, sb) = (self.slot(a), self.slot(b));
        let (
            OutputSlot::Float {
                local: la,
                sample: pa,
            },
            OutputSlot::Float {
                local: lb,
                sample: pb,
            },
        ) = (sa, sb)
        else {
            return (FloatOut::None, FloatOut::None);
        };
        let (la, lb) = (la as usize, lb as usize);
        match (pa, pb) {
            (true, true) => match self.samples.get_disjoint_mut([la, lb]) {
                Ok([x, y]) => {
                    let (nx, ny) = (frames.min(x.len()), frames.min(y.len()));
                    (
                        FloatOut::Sample(&mut x[..nx]),
                        FloatOut::Sample(&mut y[..ny]),
                    )
                }
                Err(_) => (FloatOut::None, FloatOut::None),
            },
            (false, false) => match self.scalars.get_disjoint_mut([la, lb]) {
                Ok([x, y]) => (FloatOut::Block(x), FloatOut::Block(y)),
                Err(_) => (FloatOut::None, FloatOut::None),
            },
            (true, false) => (
                self.samples.get_mut(la).map_or(FloatOut::None, |x| {
                    let n = frames.min(x.len());
                    FloatOut::Sample(&mut x[..n])
                }),
                self.scalars
                    .get_mut(lb)
                    .map_or(FloatOut::None, FloatOut::Block),
            ),
            (false, true) => (
                self.scalars
                    .get_mut(la)
                    .map_or(FloatOut::None, FloatOut::Block),
                self.samples.get_mut(lb).map_or(FloatOut::None, |y| {
                    let n = frames.min(y.len());
                    FloatOut::Sample(&mut y[..n])
                }),
            ),
        }
    }

    /// A `Float`/`Bool` output and a `Trigger` output at once.
    pub fn float_trigger(&mut self, f: usize, t: usize) -> (FloatOut<'_>, TriggerOut<'_>) {
        let frames = self.frames;
        let fo = match self.slot(f) {
            OutputSlot::Float {
                local,
                sample: true,
            } => self
                .samples
                .get_mut(local as usize)
                .map_or(FloatOut::None, |x| {
                    let n = frames.min(x.len());
                    FloatOut::Sample(&mut x[..n])
                }),
            OutputSlot::Float {
                local,
                sample: false,
            } => self
                .scalars
                .get_mut(local as usize)
                .map_or(FloatOut::None, FloatOut::Block),
            _ => FloatOut::None,
        };
        let to = match self.slots.get(t).copied() {
            Some(OutputSlot::Trigger(k)) => TriggerOut {
                buf: self.triggers.get_mut(k as usize),
            },
            _ => TriggerOut { buf: None },
        };
        (fo, to)
    }

    /// True if anything downstream reads this output (the engine marks unread
    /// outputs `None` only when it can prove so; nodes must not rely on it).
    pub fn is_connected(&self, i: usize) -> bool {
        !matches!(self.slot(i), OutputSlot::None)
    }
}

/// The host's device buffers, for `AudioIn`, `AudioOut`, `MidiIn`, and
/// `MidiOut` nodes.
/// Channel-major: channel `c` of the host block lives at
/// `[c * stride .. (c + 1) * stride]`; this view covers `offset..offset + frames`.
#[derive(Debug)]
pub struct HostAccess<'a> {
    pub input: &'a [f32],
    pub in_channels: usize,
    pub output: &'a mut [f32],
    pub out_channels: usize,
    pub stride: usize,
    pub offset: usize,
    pub frames: usize,
    /// MIDI received for this block, offsets relative to this block.
    pub midi: &'a [MidiEvent],
    /// MIDI to send to the devices, offsets relative to the host block (so
    /// `offset` + the offset in this view). Only `host_output` nodes get it;
    /// write with [`HostAccess::send_midi`].
    pub midi_out: Option<&'a mut Vec<MidiEvent>>,
    /// Control-surface events (keys, gamepads, OSC) received for this
    /// block, offsets relative to this block.
    pub controls: &'a [ControlEvent],
    /// OSC messages to send, offsets relative to the host block like
    /// `midi_out`. Only `host_output` nodes get it; write with
    /// [`HostAccess::send_osc`].
    pub osc_out: Option<&'a mut Vec<OscMessage>>,
}

impl<'a> HostAccess<'a> {
    /// No device (offline renders without input, tests).
    pub fn none(frames: usize) -> HostAccess<'static> {
        HostAccess {
            input: &[],
            in_channels: 0,
            output: &mut [],
            out_channels: 0,
            stride: frames,
            offset: 0,
            frames,
            midi: &[],
            midi_out: None,
            controls: &[],
            osc_out: None,
        }
    }

    /// Device input channel `c` for this block (silence if absent).
    pub fn input_channel(&self, c: usize) -> &[f32] {
        if c >= self.in_channels {
            return silence(self.frames);
        }
        let s = c * self.stride + self.offset;
        self.input
            .get(s..s + self.frames)
            .unwrap_or_else(|| silence(self.frames))
    }

    /// Device output channel `c` for this block, if it exists.
    pub fn output_channel(&mut self, c: usize) -> Option<&mut [f32]> {
        if c >= self.out_channels {
            return None;
        }
        let s = c * self.stride + self.offset;
        self.output.get_mut(s..s + self.frames)
    }

    pub fn midi(&self) -> &'a [MidiEvent] {
        self.midi
    }

    /// Send a message to the MIDI devices at `ev.offset` in this block.
    /// Keeps offsets ascending; drops the message if the host has no MIDI
    /// output or its buffer is full.
    pub fn send_midi(&mut self, ev: MidiEvent) {
        let offset = self.offset as u32 + ev.offset.min(self.frames.saturating_sub(1) as u32);
        if let Some(buf) = self.midi_out.as_deref_mut() {
            if buf.len() == buf.capacity() {
                return;
            }
            let pos = buf.partition_point(|e| e.offset <= offset);
            buf.insert(pos, MidiEvent::new(offset, ev.msg));
        }
    }

    /// Whether [`HostAccess::send_midi`] goes anywhere.
    pub fn has_midi_out(&self) -> bool {
        self.midi_out.is_some()
    }

    pub fn controls(&self) -> &'a [ControlEvent] {
        self.controls
    }

    /// Send an OSC message at `msg.offset` in this block. Keeps offsets
    /// ascending; drops the message if the host has no OSC output or its
    /// buffer is full.
    pub fn send_osc(&mut self, mut msg: OscMessage) {
        msg.offset = self.offset as u32 + msg.offset.min(self.frames.saturating_sub(1) as u32);
        if let Some(buf) = self.osc_out.as_deref_mut() {
            if buf.len() == buf.capacity() {
                return;
            }
            let pos = buf.partition_point(|m| m.offset <= msg.offset);
            buf.insert(pos, msg);
        }
    }

    /// Whether [`HostAccess::send_osc`] goes anywhere.
    pub fn has_osc_out(&self) -> bool {
        self.osc_out.is_some()
    }
}

/// Lock-free telemetry slots a node can publish to (meters, scopes, readouts).
/// The UI reads the same atomics at frame rate; nothing waits.
#[derive(Debug, Clone, Copy)]
pub struct TelemetryAccess<'a> {
    pub values: &'a [AtomicU32],
    pub scope: &'a [AtomicU32],
    pub scope_pos: Option<&'a AtomicU32>,
}

impl<'a> TelemetryAccess<'a> {
    pub fn none() -> TelemetryAccess<'static> {
        TelemetryAccess {
            values: &[],
            scope: &[],
            scope_pos: None,
        }
    }

    /// Publish value `k`.
    #[inline]
    pub fn set(&self, k: usize, v: f32) {
        if let Some(a) = self.values.get(k) {
            a.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// Read back value `k` (tests, and nodes that keep state in telemetry).
    pub fn get(&self, k: usize) -> f32 {
        self.values
            .get(k)
            .map_or(0.0, |a| f32::from_bits(a.load(Ordering::Relaxed)))
    }

    /// Append samples to the scope ring.
    pub fn push_scope(&self, samples: impl IntoIterator<Item = f32>) {
        let (Some(pos), len) = (self.scope_pos, self.scope.len()) else {
            return;
        };
        if len == 0 {
            return;
        }
        let mut p = pos.load(Ordering::Relaxed) as usize % len;
        for s in samples {
            self.scope[p].store(s.to_bits(), Ordering::Relaxed);
            p = (p + 1) % len;
        }
        pos.store(p as u32, Ordering::Relaxed);
    }

    pub fn scope_len(&self) -> usize {
        self.scope.len()
    }
}

/// Everything a node touches during `process`.
#[derive(Debug)]
pub struct Io<'a> {
    pub inputs: Inputs<'a>,
    pub outputs: Outputs<'a>,
    pub host: HostAccess<'a>,
    pub telemetry: TelemetryAccess<'a>,
}

impl Io<'_> {
    pub fn frames(&self) -> usize {
        self.inputs.frames
    }
}
