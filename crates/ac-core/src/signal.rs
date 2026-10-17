//! Signal types, units, rates, channel rules, and socket signatures.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::value::Value;

/// What a socket carries. Each type has a fixed color and shape in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// 1..N channels of `f32` per sample.
    Audio,
    /// One `f32` with a unit hint; constant, per-block, or per-sample.
    Float,
    /// An `i32`; constant or per-block.
    Int,
    /// A gate; constant, per-block, or per-sample.
    Bool,
    /// Sample-stamped events with no payload.
    Trigger,
    /// Sample-stamped MIDI messages.
    Midi,
    /// Takes the type of whatever is connected (Reroute only).
    Any,
}

impl SignalType {
    pub fn name(self) -> &'static str {
        match self {
            SignalType::Audio => "Audio",
            SignalType::Float => "Float",
            SignalType::Int => "Int",
            SignalType::Bool => "Bool",
            SignalType::Trigger => "Trigger",
            SignalType::Midi => "Midi",
            SignalType::Any => "Any",
        }
    }

    pub fn is_event(self) -> bool {
        matches!(self, SignalType::Trigger | SignalType::Midi)
    }
}

impl fmt::Display for SignalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How a value of one type reaches an input of another. Only these
/// conversions are implicit (ARCHITECTURE §5, DECISIONS D13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conversion {
    /// Same type, nothing to do (mono → N upmix is handled by channel rules).
    None,
    /// `Float → Audio`: a mono stream of the value.
    FloatToAudio,
    /// `Int → Float`.
    IntToFloat,
    /// `Bool → Float` (0 or 1).
    BoolToFloat,
    /// `Audio → Float`: channel 0 as a sample-rate float.
    AudioToFloat,
}

/// The conversion used to wire `from` into `to`, or `None` if they can't connect.
pub fn conversion(from: SignalType, to: SignalType) -> Option<Conversion> {
    use SignalType::*;
    match (from, to) {
        (Any, _) | (_, Any) => Some(Conversion::None),
        (a, b) if a == b => Some(Conversion::None),
        (Float, Audio) => Some(Conversion::FloatToAudio),
        (Int, Float) => Some(Conversion::IntToFloat),
        (Bool, Float) => Some(Conversion::BoolToFloat),
        (Audio, Float) => Some(Conversion::AudioToFloat),
        _ => None,
    }
}

/// Unit hint for `Float` sockets: chooses the inline widget and formatting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Generic,
    Hz,
    Db,
    Seconds,
    Beats,
    Ratio,
    Semitones,
    Bpm,
    Samples,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Generic => "",
            Unit::Hz => "Hz",
            Unit::Db => "dB",
            Unit::Seconds => "s",
            Unit::Beats => "beats",
            Unit::Ratio => "×",
            Unit::Semitones => "st",
            Unit::Bpm => "BPM",
            Unit::Samples => "smp",
        }
    }
}

/// Inline widget scale for numeric inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    #[default]
    Linear,
    Log,
}

/// How often a `Float`/`Bool`/`Int` value can change. Ordered: a node whose
/// output follows its inputs runs at the highest input rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rate {
    /// Unwired: comes from the node's params (changes go through the parameter queue).
    Const,
    /// One value per block.
    Block,
    /// One value per sample.
    Sample,
}

/// Declared rate of an output socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutRate {
    Fixed(Rate),
    /// The highest effective rate among the node's `Float`/`Bool` inputs
    /// (at least `Block`). Used by rate-polymorphic nodes like Math.
    Poly,
}

/// Channel rules for `Audio` sockets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    /// Exactly this many channels. On an input, mono sources are upmixed.
    Fixed(u32),
    /// Inputs: any channel count. Outputs: the maximum channel count among the
    /// node's audio inputs (1 if none are wired).
    Follow,
    /// Inputs only: at most this many channels.
    UpTo(u32),
}

/// What an unwired trigger input does on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoTrigger {
    /// Only fires from a wire or the inline button.
    #[default]
    None,
    /// Generator `start`: dropdown of *On transport play* / *On load* / *Never*.
    Start,
    /// Generator `stop`: dropdown of *On transport stop* / *Never*.
    Stop,
}

/// Choices offered by an `AutoTrigger::Start` input, stored as the param value.
pub const START_CHOICES: &[&str] = &["play", "load", "never"];
/// Choices offered by an `AutoTrigger::Stop` input.
pub const STOP_CHOICES: &[&str] = &["stop", "never"];

/// One input or output socket.
#[derive(Clone, Debug, PartialEq)]
pub struct SocketSpec {
    /// snake_case, stable forever; files address sockets by name.
    pub name: Cow<'static, str>,
    pub ty: SignalType,
    pub unit: Unit,
    /// Value used when unwired and no param is stored.
    pub default: Value,
    /// Soft range for the inline widget (values outside are allowed).
    pub range: Option<(f64, f64)>,
    pub scale: Scale,
    /// Non-empty for enumerated `Int` inputs: shown as a dropdown, stored as
    /// the choice name.
    pub choices: &'static [&'static str],
    /// Outputs: the rate this socket produces.
    pub rate: OutRate,
    pub channels: Channels,
    /// Inputs: smooth parameter changes over a few milliseconds.
    pub smooth: bool,
    pub auto: AutoTrigger,
    /// One-line description shown on hover.
    pub doc: &'static str,
}

impl SocketSpec {
    fn base(name: impl Into<Cow<'static, str>>, ty: SignalType, default: Value) -> Self {
        SocketSpec {
            name: name.into(),
            ty,
            unit: Unit::Generic,
            default,
            range: None,
            scale: Scale::Linear,
            choices: &[],
            rate: OutRate::Fixed(match ty {
                SignalType::Int => Rate::Block,
                _ => Rate::Sample,
            }),
            channels: Channels::Follow,
            smooth: ty == SignalType::Float,
            auto: AutoTrigger::None,
            doc: "",
        }
    }

    pub fn audio(name: impl Into<Cow<'static, str>>) -> Self {
        Self::base(name, SignalType::Audio, Value::Float(0.0))
    }

    pub fn float(name: impl Into<Cow<'static, str>>, unit: Unit, default: f64) -> Self {
        let mut s = Self::base(name, SignalType::Float, Value::Float(default));
        s.unit = unit;
        s
    }

    pub fn int(name: impl Into<Cow<'static, str>>, default: i64) -> Self {
        Self::base(name, SignalType::Int, Value::Int(default))
    }

    pub fn boolean(name: impl Into<Cow<'static, str>>, default: bool) -> Self {
        Self::base(name, SignalType::Bool, Value::Bool(default))
    }

    pub fn trigger(name: impl Into<Cow<'static, str>>) -> Self {
        Self::base(name, SignalType::Trigger, Value::Bool(false))
    }

    pub fn midi(name: impl Into<Cow<'static, str>>) -> Self {
        Self::base(name, SignalType::Midi, Value::Bool(false))
    }

    pub fn any(name: impl Into<Cow<'static, str>>) -> Self {
        Self::base(name, SignalType::Any, Value::Float(0.0))
    }

    /// An enumerated `Int` input shown as a dropdown; `default` is a choice name.
    pub fn choice(
        name: impl Into<Cow<'static, str>>,
        choices: &'static [&'static str],
        default: &str,
    ) -> Self {
        let mut s = Self::base(name, SignalType::Int, Value::Text(default.to_string()));
        s.choices = choices;
        s
    }

    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.range = Some((min, max));
        self
    }

    pub fn log(mut self) -> Self {
        self.scale = Scale::Log;
        self
    }

    pub fn rate(mut self, rate: Rate) -> Self {
        self.rate = OutRate::Fixed(rate);
        self
    }

    /// Output follows the highest input rate.
    pub fn poly(mut self) -> Self {
        self.rate = OutRate::Poly;
        self
    }

    pub fn channels(mut self, ch: Channels) -> Self {
        self.channels = ch;
        self
    }

    pub fn no_smooth(mut self) -> Self {
        self.smooth = false;
        self
    }

    pub fn auto(mut self, auto: AutoTrigger) -> Self {
        self.auto = auto;
        self.default = Value::Text(
            match auto {
                AutoTrigger::None => "",
                AutoTrigger::Start => "play",
                AutoTrigger::Stop => "stop",
            }
            .to_string(),
        );
        self.choices = match auto {
            AutoTrigger::None => &[],
            AutoTrigger::Start => START_CHOICES,
            AutoTrigger::Stop => STOP_CHOICES,
        };
        self
    }

    pub fn doc(mut self, doc: &'static str) -> Self {
        self.doc = doc;
        self
    }

    pub fn default_value(mut self, v: impl Into<Value>) -> Self {
        self.default = v.into();
        self
    }

    /// Index of a choice name, for enumerated inputs.
    pub fn choice_index(&self, name: &str) -> Option<usize> {
        self.choices.iter().position(|c| *c == name)
    }
}

/// A node's sockets, in order. Inputs and outputs are addressed by name in
/// files and by index (their position here) at runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signature {
    pub inputs: Vec<SocketSpec>,
    pub outputs: Vec<SocketSpec>,
}

impl Signature {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn input(mut self, s: SocketSpec) -> Self {
        self.inputs.push(s);
        self
    }

    pub fn output(mut self, s: SocketSpec) -> Self {
        self.outputs.push(s);
        self
    }

    pub fn input_index(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|s| s.name == name)
    }

    pub fn output_index(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|s| s.name == name)
    }

    pub fn find_input(&self, name: &str) -> Option<&SocketSpec> {
        self.inputs.iter().find(|s| s.name == name)
    }

    pub fn find_output(&self, name: &str) -> Option<&SocketSpec> {
        self.outputs.iter().find(|s| s.name == name)
    }

    /// The generator contract inputs (NODES.md): `start`, `stop`, `reset`,
    /// and optionally `loop`, `reverse`, `rate`. Appended in that order.
    pub fn generator_inputs(mut self, finite: bool, reversible: bool) -> Self {
        self.inputs.push(
            SocketSpec::trigger("start")
                .auto(AutoTrigger::Start)
                .doc("Starts or restarts. Unwired: on transport play, on load, or never."),
        );
        self.inputs.push(
            SocketSpec::trigger("stop")
                .auto(AutoTrigger::Stop)
                .doc("Stops. Unwired: on transport stop, or never."),
        );
        self.inputs
            .push(SocketSpec::trigger("reset").doc("Returns to the beginning."));
        if finite {
            self.inputs
                .push(SocketSpec::boolean("loop", false).doc("Wrap around at the end."));
        }
        if reversible {
            self.inputs
                .push(SocketSpec::trigger("reverse").doc("Toggles playback direction."));
            self.inputs.push(
                SocketSpec::float("rate", Unit::Ratio, 1.0)
                    .range(0.0, 4.0)
                    .doc("Playback speed."),
            );
        }
        self
    }

    /// The generator contract outputs: `playing: Bool` and `end: Trigger`.
    pub fn generator_outputs(mut self) -> Self {
        self.outputs.push(
            SocketSpec::boolean("playing", false)
                .rate(Rate::Block)
                .doc("True while running."),
        );
        self.outputs
            .push(SocketSpec::trigger("end").doc("Fires on reaching the end or stopping."));
        self
    }
}

/// Snake-case check for socket names (lowercase ASCII, digits, underscores).
pub fn is_snake_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_conversions_are_exactly_the_documented_ones() {
        use SignalType::*;
        assert_eq!(conversion(Float, Audio), Some(Conversion::FloatToAudio));
        assert_eq!(conversion(Int, Float), Some(Conversion::IntToFloat));
        assert_eq!(conversion(Bool, Float), Some(Conversion::BoolToFloat));
        assert_eq!(conversion(Audio, Float), Some(Conversion::AudioToFloat));
        assert_eq!(conversion(Trigger, Bool), None);
        assert_eq!(conversion(Bool, Trigger), None);
        assert_eq!(conversion(Midi, Float), None);
        assert_eq!(conversion(Float, Int), None);
        assert_eq!(conversion(Float, Bool), None);
        assert_eq!(conversion(Audio, Audio), Some(Conversion::None));
    }

    #[test]
    fn generator_contract_sockets() {
        let s = Signature::new()
            .generator_inputs(true, true)
            .generator_outputs();
        let names: Vec<_> = s.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names, ["start", "stop", "reset", "loop", "reverse", "rate"]);
        assert_eq!(s.inputs[0].default, Value::Text("play".into()));
        assert_eq!(s.output_index("end"), Some(1));
    }

    #[test]
    fn snake_case_names() {
        assert!(is_snake_case("in_3"));
        assert!(!is_snake_case("In"));
        assert!(!is_snake_case("3in"));
        assert!(!is_snake_case(""));
    }
}
