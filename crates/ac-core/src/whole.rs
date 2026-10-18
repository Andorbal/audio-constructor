//! Whole-input processing (ARCHITECTURE §9): nodes that must see all of
//! their input before producing any output (Reverse, Normalize) implement
//! [`Node::render_whole`](crate::Node::render_whole) instead of streaming.
//!
//! The engine captures every input over the project length, calls
//! `render_whole` off the audio thread, and streams the result back at the
//! transport position. The node never runs on the audio thread itself.

use crate::events::MidiMsg;

/// One input or output over the whole render.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum WholeValue {
    /// Unwired, or a type the engine doesn't capture.
    #[default]
    None,
    /// Audio channels, each `frames` long.
    Audio(Vec<Vec<f32>>),
    /// A `Float` or `Bool` value at every frame (bools are 0 or 1).
    Float(Vec<f32>),
    /// An `Int` value at every frame.
    Int(Vec<i32>),
    /// Trigger positions in frames, ascending.
    Triggers(Vec<u64>),
    /// MIDI messages at frame positions, ascending.
    Midi(Vec<(u64, MidiMsg)>),
}

/// What a whole-input node sees.
#[derive(Clone, Debug, PartialEq)]
pub struct WholeCtx {
    pub sample_rate: f64,
    /// Length of every captured input and expected output.
    pub frames: usize,
    /// PRNG seed derived from the project seed and the node's path.
    pub seed: u64,
}

/// Every input of a whole-input node, in signature order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WholeInputs {
    pub values: Vec<WholeValue>,
}

impl WholeInputs {
    pub fn get(&self, i: usize) -> &WholeValue {
        self.values.get(i).unwrap_or(&WholeValue::None)
    }

    /// An audio input's channels (empty if unwired).
    pub fn audio(&self, i: usize) -> &[Vec<f32>] {
        match self.get(i) {
            WholeValue::Audio(c) => c,
            _ => &[],
        }
    }

    /// A float input's value at every frame (empty if not captured).
    pub fn float(&self, i: usize) -> &[f32] {
        match self.get(i) {
            WholeValue::Float(v) => v,
            _ => &[],
        }
    }

    /// A float input's value at frame `at`, or `default` past the end.
    pub fn float_at(&self, i: usize, at: usize, default: f32) -> f32 {
        self.float(i).get(at).copied().unwrap_or(default)
    }

    pub fn int(&self, i: usize) -> &[i32] {
        match self.get(i) {
            WholeValue::Int(v) => v,
            _ => &[],
        }
    }

    pub fn triggers(&self, i: usize) -> &[u64] {
        match self.get(i) {
            WholeValue::Triggers(v) => v,
            _ => &[],
        }
    }
}

/// What a whole-input node produces, in signature order. The engine streams
/// `Audio` and `Float` outputs; anything else reads as silence or zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WholeOutputs {
    pub values: Vec<WholeValue>,
}

impl WholeOutputs {
    pub fn with_outputs(n: usize) -> Self {
        WholeOutputs {
            values: vec![WholeValue::None; n],
        }
    }

    pub fn set(&mut self, i: usize, v: WholeValue) {
        if self.values.len() <= i {
            self.values.resize(i + 1, WholeValue::None);
        }
        self.values[i] = v;
    }

    pub fn get(&self, i: usize) -> &WholeValue {
        self.values.get(i).unwrap_or(&WholeValue::None)
    }

    /// Bytes held, for reporting the memory a bake uses.
    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .map(|v| match v {
                WholeValue::Audio(c) => c.iter().map(|x| x.len() * 4).sum(),
                WholeValue::Float(x) => x.len() * 4,
                WholeValue::Int(x) => x.len() * 4,
                WholeValue::Triggers(x) => x.len() * 8,
                WholeValue::Midi(x) => x.len() * 16,
                WholeValue::None => 0,
            })
            .sum()
    }
}
