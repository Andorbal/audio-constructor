//! Bindings: MIDI controllers and notes, computer keys, gamepad controls,
//! and OSC mapped onto node inputs and transport actions (UX.md
//! "Bindings", ARCHITECTURE §8 "Parameters in").
//!
//! A [`Binding`] is stored in the project. The engine resolves its target
//! to parameter slots when it compiles and applies matching events on the
//! audio thread, so the editor, `audioc live`, and offline renders all
//! honor the same bindings. This module holds the parts that don't need
//! the engine: the file format, how an event reads as a position (0–1) and
//! a press ([`SourceKey`], [`Reading`]), how a position lands on an input
//! ([`Mapping`], [`Effect`]), and learning a source from incoming events
//! ([`Learner`]).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::control::{Control, ControlEvent, GamepadAxis, GamepadButton, Key, osc_address_hash};
use crate::events::MidiMsg;
use crate::id::NodePath;
use crate::signal::{Scale, SignalType, SocketSpec, Unit};

/// A control mapped onto a node input or a transport action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub source: Source,
    pub target: Target,
    /// The value the control's low end gives (default: the bottom of the
    /// input's range). Float and Int inputs only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// The value the control's high end gives (default: the top of the
    /// input's range).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Turn the control around: its low end gives `max`, a press releases.
    #[serde(default, skip_serializing_if = "is_false")]
    pub invert: bool,
    /// Bool inputs: follow the control, or flip on each press.
    #[serde(default, skip_serializing_if = "GateMode::is_momentary")]
    pub mode: GateMode,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Binding {
    pub fn new(source: Source, target: Target) -> Binding {
        Binding {
            source,
            target,
            min: None,
            max: None,
            invert: false,
            mode: GateMode::Momentary,
        }
    }

    /// A binding of `source` to a node input.
    pub fn input(source: Source, node: NodePath, socket: impl Into<String>) -> Binding {
        Binding::new(
            source,
            Target::Input {
                node,
                socket: socket.into(),
            },
        )
    }

    /// A binding of `source` to a transport action.
    pub fn transport(source: Source, action: TransportAction) -> Binding {
        Binding::new(source, Target::Transport { action })
    }

    /// The values the control's low and high ends give on an input
    /// described by `spec`: this binding's own `min`/`max` where set
    /// (and finite), else [`default_range`].
    pub fn range(&self, spec: &SocketSpec) -> (f64, f64) {
        let (lo, hi) = default_range(spec);
        let pick = |v: Option<f64>, d: f64| v.filter(|v| v.is_finite()).unwrap_or(d);
        (pick(self.min, lo), pick(self.max, hi))
    }

    /// The node path this binding targets, if it targets an input.
    pub fn node(&self) -> Option<&NodePath> {
        match &self.target {
            Target::Input { node, .. } => Some(node),
            Target::Transport { .. } => None,
        }
    }
}

/// Where a binding's events come from.
///
/// MIDI channels are 1–16 as people number them (absent: any channel);
/// gamepads are numbered from 0 in the order they were connected (absent:
/// any gamepad).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    MidiCc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<u8>,
        cc: u8,
    },
    MidiNote {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<u8>,
        note: u8,
    },
    Key {
        key: Key,
    },
    GamepadButton {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pad: Option<u8>,
        button: GamepadButton,
    },
    GamepadAxis {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pad: Option<u8>,
        axis: GamepadAxis,
    },
    /// Argument `arg` (from 0) of messages to exactly this address.
    Osc {
        address: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        arg: u8,
    },
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// A MIDI channel as stored (1–16) to the wire's 0–15. Out-of-range
/// channels are repaired (DECISIONS D29): 0 and below mean 1, above 16
/// means 16.
fn wire_channel(channel: Option<u8>) -> Option<u8> {
    channel.map(|c| c.clamp(1, 16) - 1)
}

impl Source {
    /// The form the audio thread matches events against.
    pub fn key(&self) -> SourceKey {
        match self {
            Source::MidiCc { channel, cc } => SourceKey::MidiCc {
                channel: wire_channel(*channel),
                cc: *cc & 0x7f,
            },
            Source::MidiNote { channel, note } => SourceKey::MidiNote {
                channel: wire_channel(*channel),
                note: *note & 0x7f,
            },
            Source::Key { key } => SourceKey::Key(*key),
            Source::GamepadButton { pad, button } => SourceKey::GamepadButton {
                pad: *pad,
                button: *button,
            },
            Source::GamepadAxis { pad, axis } => SourceKey::GamepadAxis {
                pad: *pad,
                axis: *axis,
            },
            Source::Osc { address, arg } => SourceKey::Osc {
                address: osc_address_hash(address),
                arg: *arg,
            },
        }
    }

    /// A short description for people: "MIDI CC 74 · ch 1", "Key Space".
    pub fn label(&self) -> String {
        let ch = |c: &Option<u8>| match c {
            Some(c) => format!("ch {}", (*c).clamp(1, 16)),
            None => "any channel".to_string(),
        };
        let pad = |p: &Option<u8>| match p {
            Some(p) => format!("Pad {}", u16::from(*p) + 1),
            None => "Gamepad".to_string(),
        };
        match self {
            Source::MidiCc { channel, cc } => format!("MIDI CC {cc} · {}", ch(channel)),
            Source::MidiNote { channel, note } => {
                format!("Note {} · {}", note_name(*note), ch(channel))
            }
            Source::Key { key } => format!("Key {key}"),
            Source::GamepadButton { pad: p, button } => format!("{} {}", pad(p), button.label()),
            Source::GamepadAxis { pad: p, axis } => format!("{} {}", pad(p), axis_label(*axis)),
            Source::Osc { address, arg } => {
                if *arg == 0 {
                    format!("OSC {address}")
                } else {
                    format!("OSC {address} [{arg}]")
                }
            }
        }
    }

    /// Whether the source sweeps a range (a controller, a stick, an OSC
    /// fader) rather than being pressed and released.
    pub fn is_continuous(&self) -> bool {
        matches!(
            self,
            Source::MidiCc { .. } | Source::GamepadAxis { .. } | Source::Osc { .. }
        )
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// A note number as people read it (60 is C4).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let n = note & 0x7f;
    let octave = i32::from(n / 12) - 1;
    format!("{}{octave}", NAMES[usize::from(n % 12)])
}

fn axis_label(axis: GamepadAxis) -> &'static str {
    match axis {
        GamepadAxis::LeftX => "left stick X",
        GamepadAxis::LeftY => "left stick Y",
        GamepadAxis::RightX => "right stick X",
        GamepadAxis::RightY => "right stick Y",
        GamepadAxis::LeftTrigger => "left trigger",
        GamepadAxis::RightTrigger => "right trigger",
    }
}

/// What a binding drives.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    /// An unwired input: the node's path from the root graph (group
    /// instance ids, then the node's id) and the socket's name.
    Input {
        node: NodePath,
        socket: String,
    },
    Transport {
        action: TransportAction,
    },
}

/// Transport actions a control can press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportAction {
    Play,
    /// Stop playback and recording.
    Stop,
    /// Play when stopped, stop when playing.
    PlayStop,
    /// Turn recording on (starting playback) or off.
    Record,
    PlayFromStart,
}

impl TransportAction {
    pub const ALL: [TransportAction; 5] = [
        TransportAction::Play,
        TransportAction::Stop,
        TransportAction::PlayStop,
        TransportAction::Record,
        TransportAction::PlayFromStart,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TransportAction::Play => "Play",
            TransportAction::Stop => "Stop",
            TransportAction::PlayStop => "Play / stop",
            TransportAction::Record => "Record",
            TransportAction::PlayFromStart => "Play from start",
        }
    }
}

/// Bool inputs: what a press does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// On while the control is held (a key, a pad, a sustain pedal).
    #[default]
    Momentary,
    /// Each press flips the input.
    Toggle,
}

impl GateMode {
    fn is_momentary(&self) -> bool {
        *self == GateMode::Momentary
    }
}

/// The input sockets a binding can drive: unwired Float, Int, Bool, and
/// Trigger inputs that people can see.
pub fn bindable(spec: &SocketSpec) -> bool {
    !spec.hidden
        && matches!(
            spec.ty,
            SignalType::Float | SignalType::Int | SignalType::Bool | SignalType::Trigger
        )
}

/// The range a control sweeps on an input when the binding doesn't say:
/// the input's soft range, else every choice of an enumerated input,
/// 0–127 for other Int inputs (one step per MIDI value), and 0–1 for the
/// rest.
pub fn default_range(spec: &SocketSpec) -> (f64, f64) {
    if !spec.choices.is_empty() {
        return (0.0, spec.choices.len().saturating_sub(1) as f64);
    }
    if let Some((a, b)) = spec.range.filter(|(a, b)| a.is_finite() && b.is_finite()) {
        return (a, b);
    }
    match spec.ty {
        SignalType::Int => (0.0, 127.0),
        _ => (0.0, 1.0),
    }
}

/// Whether an input's inline slider (and so a binding) sweeps its range on
/// a log scale: log-scaled and Hz inputs.
pub fn log_scaled(spec: &SocketSpec) -> bool {
    spec.scale == Scale::Log || spec.unit == Unit::Hz
}

/// A [`Source`] as the audio thread matches it: `Copy`, MIDI channels 0–15,
/// OSC addresses hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKey {
    MidiCc {
        channel: Option<u8>,
        cc: u8,
    },
    MidiNote {
        channel: Option<u8>,
        note: u8,
    },
    Key(Key),
    GamepadButton {
        pad: Option<u8>,
        button: GamepadButton,
    },
    GamepadAxis {
        pad: Option<u8>,
        axis: GamepadAxis,
    },
    Osc {
        address: u64,
        arg: u8,
    },
}

/// How one event reads on a source: where the control sits (0–1) and
/// whether it counts as pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub value: f32,
    pub on: bool,
}

impl Reading {
    /// A continuous position, pressed from halfway up.
    fn level(v: f32) -> Reading {
        let value = if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Reading {
            value,
            on: value >= 0.5,
        }
    }
}

impl SourceKey {
    /// The reading of a MIDI message, if it comes from this source.
    ///
    /// A controller reads its value over 127. A note reads its velocity
    /// over 127 and is pressed while held, however softly; its release
    /// reads 0.
    pub fn read_midi(&self, msg: &MidiMsg) -> Option<Reading> {
        let ch_ok = |want: Option<u8>, got: u8| want.is_none_or(|c| c == got);
        match (*self, *msg) {
            (
                SourceKey::MidiCc { channel, cc },
                MidiMsg::ControlChange {
                    channel: c,
                    controller,
                    value,
                },
            ) if controller == cc && ch_ok(channel, c) => {
                Some(Reading::level(f32::from(value.min(127)) / 127.0))
            }
            (
                SourceKey::MidiNote { channel, note },
                MidiMsg::NoteOn {
                    channel: c,
                    note: n,
                    velocity,
                },
            ) if n == note && ch_ok(channel, c) => Some(if velocity == 0 {
                Reading {
                    value: 0.0,
                    on: false,
                }
            } else {
                Reading {
                    value: f32::from(velocity.min(127)) / 127.0,
                    on: true,
                }
            }),
            (
                SourceKey::MidiNote { channel, note },
                MidiMsg::NoteOff {
                    channel: c,
                    note: n,
                    ..
                },
            ) if n == note && ch_ok(channel, c) => Some(Reading {
                value: 0.0,
                on: false,
            }),
            _ => None,
        }
    }

    /// The reading of a control event, if it comes from this source.
    ///
    /// Keys and buttons read 0–1 (pressed from halfway); sticks read
    /// `(v + 1) / 2` and count as pressed when pushed at least halfway right
    /// or up; triggers and OSC values read 0–1.
    pub fn read_control(&self, e: &ControlEvent) -> Option<Reading> {
        let pad_ok = |want: Option<u8>, got: u8| want.is_none_or(|p| p == got);
        match (*self, e.control) {
            (SourceKey::Key(k), Control::Key(got)) if k == got => Some(Reading::level(e.value)),
            (
                SourceKey::GamepadButton { pad, button },
                Control::GamepadButton { pad: p, button: b },
            ) if b == button && pad_ok(pad, p) => Some(Reading::level(e.value)),
            (SourceKey::GamepadAxis { pad, axis }, Control::GamepadAxis { pad: p, axis: a })
                if a == axis && pad_ok(pad, p) =>
            {
                let v = if e.value.is_finite() { e.value } else { 0.0 };
                if axis.is_trigger() {
                    Some(Reading::level(v))
                } else {
                    let v = v.clamp(-1.0, 1.0);
                    Some(Reading {
                        value: (v + 1.0) / 2.0,
                        on: v >= 0.5,
                    })
                }
            }
            (SourceKey::Osc { address, arg }, Control::Osc { address: a, arg: g })
                if a == address && g == arg =>
            {
                Some(Reading::level(e.value))
            }
            _ => None,
        }
    }
}

/// How a binding's readings land on its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mapping {
    /// Float and Int inputs: the position placed on `min..max` (on a log
    /// scale between positive ends when `log`), rounded for Int inputs.
    Range {
        min: f32,
        max: f32,
        log: bool,
        round: bool,
        invert: bool,
    },
    /// Bool inputs: on while pressed (momentary) or flipped by each press.
    Gate { invert: bool, toggle: bool },
    /// Trigger inputs and transport actions: once per press.
    Press,
}

/// What applying one reading does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Set the input to this value.
    Set(f32),
    /// Flip a Bool input.
    Flip,
    /// Fire a trigger (or press a transport action).
    Fire,
    None,
}

impl Mapping {
    /// How `binding` lands on an input described by `spec`, or `None` if
    /// the input can't be bound ([`bindable`]).
    pub fn for_input(binding: &Binding, spec: &SocketSpec) -> Option<Mapping> {
        if !bindable(spec) {
            return None;
        }
        Some(match spec.ty {
            SignalType::Trigger => Mapping::Press,
            SignalType::Bool => Mapping::Gate {
                invert: binding.invert,
                toggle: binding.mode == GateMode::Toggle,
            },
            _ => {
                let (min, max) = binding.range(spec);
                let (min, max) = (min as f32, max as f32);
                let round = spec.ty == SignalType::Int;
                Mapping::Range {
                    min,
                    max,
                    log: log_scaled(spec) && min > 0.0 && max > 0.0,
                    round,
                    invert: binding.invert,
                }
            }
        })
    }

    /// The effect of `reading`, given whether the control was pressed
    /// before it. Presses and toggles act on the rising edge only.
    pub fn effect(&self, reading: Reading, was_on: bool) -> Effect {
        let rising = reading.on && !was_on;
        match *self {
            Mapping::Range {
                min,
                max,
                log,
                round,
                invert,
            } => {
                let t = if invert {
                    1.0 - reading.value
                } else {
                    reading.value
                };
                Effect::Set(map_range(t, min, max, log, round))
            }
            Mapping::Gate {
                invert,
                toggle: false,
            } => Effect::Set(if reading.on != invert { 1.0 } else { 0.0 }),
            Mapping::Gate { toggle: true, .. } | Mapping::Press => {
                if rising {
                    if matches!(self, Mapping::Press) {
                        Effect::Fire
                    } else {
                        Effect::Flip
                    }
                } else {
                    Effect::None
                }
            }
        }
    }
}

/// Position `t` (0–1) of `min..max`, on a log scale when `log` (both ends
/// must be positive), rounded when `round`. The ends land exactly on
/// `min` and `max`.
pub fn map_range(t: f32, min: f32, max: f32, log: bool, round: bool) -> f32 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let v = if t <= 0.0 {
        min
    } else if t >= 1.0 {
        max
    } else if log && min > 0.0 && max > 0.0 {
        let (a, b) = (f64::from(min).ln(), f64::from(max).ln());
        (a + f64::from(t) * (b - a)).exp() as f32
    } else {
        min + t * (max - min)
    };
    if round { v.round() } else { v }
}

/// A raw event the engine received, as it hands them to the editor for
/// learning (`EngineHandle::take_input_events`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    Midi(MidiMsg),
    Control(Control, f32),
}

/// How far (0–1) a controller, stick, or OSC value must move from where
/// it was first seen before learning takes it, so jitter doesn't bind.
pub const LEARN_THRESHOLD: f32 = 0.08;

/// Learns a binding source from incoming events: the first key, note,
/// button, or press, or the first controller, stick, or OSC value that
/// moves by [`LEARN_THRESHOLD`].
#[derive(Clone, Debug, Default)]
pub struct Learner {
    /// Where each continuous control was first seen.
    seen: Vec<(Control, f32)>,
    seen_cc: Vec<((u8, u8), f32)>,
}

impl Learner {
    pub fn new() -> Learner {
        Learner::default()
    }

    /// Feed one event; returns the source to bind once one qualifies. An
    /// OSC address is known to the engine only by its hash: `osc_name`
    /// turns a hash back into the address (events it can't name are
    /// skipped).
    pub fn feed(
        &mut self,
        event: &InputEvent,
        osc_name: &dyn Fn(u64) -> Option<String>,
    ) -> Option<Source> {
        match *event {
            InputEvent::Midi(MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            }) if velocity > 0 => Some(Source::MidiNote {
                channel: Some(channel.min(15) + 1),
                note,
            }),
            InputEvent::Midi(MidiMsg::ControlChange {
                channel,
                controller,
                value,
            }) => {
                let v = f32::from(value.min(127)) / 127.0;
                let k = (channel, controller);
                match self.seen_cc.iter().find(|(c, _)| *c == k) {
                    Some((_, first)) if (v - first).abs() >= LEARN_THRESHOLD => {
                        Some(Source::MidiCc {
                            channel: Some(channel.min(15) + 1),
                            cc: controller,
                        })
                    }
                    Some(_) => None,
                    None => {
                        self.seen_cc.push((k, v));
                        None
                    }
                }
            }
            InputEvent::Midi(_) => None,
            InputEvent::Control(Control::Key(key), v) => (v >= 0.5).then_some(Source::Key { key }),
            InputEvent::Control(Control::GamepadButton { pad, button }, v) => {
                (v >= 0.5).then_some(Source::GamepadButton {
                    pad: Some(pad),
                    button,
                })
            }
            InputEvent::Control(control @ Control::GamepadAxis { pad, axis }, v) => {
                self.moved(control, v).then_some(Source::GamepadAxis {
                    pad: Some(pad),
                    axis,
                })
            }
            InputEvent::Control(control @ Control::Osc { address, arg }, v) => {
                if !self.moved(control, v) {
                    return None;
                }
                osc_name(address).map(|address| Source::Osc { address, arg })
            }
        }
    }

    /// Whether a continuous control moved far enough from where it was
    /// first seen (remembering it the first time).
    fn moved(&mut self, control: Control, v: f32) -> bool {
        let v = if v.is_finite() { v } else { 0.0 };
        let scale = match control {
            // Sticks span −1..1.
            Control::GamepadAxis { axis, .. } if !axis.is_trigger() => 0.5,
            _ => 1.0,
        };
        match self.seen.iter().find(|(c, _)| *c == control) {
            Some((_, first)) => (v - first).abs() * scale >= LEARN_THRESHOLD,
            None => {
                self.seen.push((control, v));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::NodeId;
    use crate::signal::SocketSpec;

    fn cc(channel: u8, controller: u8, value: u8) -> MidiMsg {
        MidiMsg::ControlChange {
            channel,
            controller,
            value,
        }
    }

    fn path(ids: &[u64]) -> NodePath {
        NodePath(ids.iter().map(|&i| NodeId(i)).collect())
    }

    #[test]
    fn bindings_round_trip_through_json() {
        let all = vec![
            Binding::input(
                Source::MidiCc {
                    channel: Some(1),
                    cc: 74,
                },
                path(&[0x10, 0x2]),
                "cutoff",
            ),
            Binding {
                min: Some(-24.0),
                max: Some(0.0),
                invert: true,
                ..Binding::input(
                    Source::MidiNote {
                        channel: None,
                        note: 60,
                    },
                    path(&[3]),
                    "gain",
                )
            },
            Binding {
                mode: GateMode::Toggle,
                ..Binding::input(
                    Source::Key {
                        key: Key::from_name("Space").unwrap(),
                    },
                    path(&[4]),
                    "on",
                )
            },
            Binding::transport(
                Source::GamepadButton {
                    pad: None,
                    button: GamepadButton::South,
                },
                TransportAction::PlayStop,
            ),
            Binding::input(
                Source::GamepadAxis {
                    pad: Some(1),
                    axis: GamepadAxis::LeftX,
                },
                path(&[5]),
                "pan",
            ),
            Binding::input(
                Source::Osc {
                    address: "/fader/1".into(),
                    arg: 2,
                },
                path(&[6]),
                "x",
            ),
        ];
        let text = serde_json::to_string_pretty(&all).unwrap();
        let back: Vec<Binding> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, all);
        // Defaults stay out of the file.
        let plain = serde_json::to_value(&all[0]).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({
                "source": {"type": "midi_cc", "channel": 1, "cc": 74},
                "target": {"type": "input", "node": "10/2", "socket": "cutoff"},
            })
        );
        let t = serde_json::to_value(&all[3]).unwrap();
        assert_eq!(
            t["target"],
            serde_json::json!({"type": "transport", "action": "play_stop"})
        );
        assert_eq!(
            t["source"],
            serde_json::json!({"type": "gamepad_button", "button": "south"})
        );
        // Bad entries are errors, not panics.
        assert!(serde_json::from_str::<Binding>(r#"{"source": {"type": "theremin"}}"#).is_err());
        assert!(
            serde_json::from_str::<Binding>(
                r#"{"source": {"type": "key", "key": "Hyper"}, "target": {"type": "transport", "action": "play"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn controllers_read_their_value_over_127() {
        let k = Source::MidiCc {
            channel: Some(1),
            cc: 7,
        }
        .key();
        assert_eq!(
            k.read_midi(&cc(0, 7, 127)),
            Some(Reading {
                value: 1.0,
                on: true
            })
        );
        let mid = k.read_midi(&cc(0, 7, 63)).unwrap();
        assert!((mid.value - 63.0 / 127.0).abs() < 1e-6 && !mid.on);
        assert!(k.read_midi(&cc(0, 7, 64)).unwrap().on);
        assert_eq!(k.read_midi(&cc(1, 7, 10)), None, "other channel");
        assert_eq!(k.read_midi(&cc(0, 8, 10)), None, "other controller");
        let any = Source::MidiCc {
            channel: None,
            cc: 7,
        }
        .key();
        assert!(any.read_midi(&cc(9, 7, 0)).is_some());
        // Channels outside 1–16 are repaired rather than matching nothing.
        let bad = Source::MidiCc {
            channel: Some(40),
            cc: 7,
        }
        .key();
        assert!(bad.read_midi(&cc(15, 7, 0)).is_some());
    }

    #[test]
    fn notes_press_however_softly_and_release_to_zero() {
        let k = Source::MidiNote {
            channel: None,
            note: 60,
        }
        .key();
        let soft = k
            .read_midi(&MidiMsg::NoteOn {
                channel: 3,
                note: 60,
                velocity: 20,
            })
            .unwrap();
        assert!(soft.on);
        assert!((soft.value - 20.0 / 127.0).abs() < 1e-6);
        let off = MidiMsg::NoteOff {
            channel: 3,
            note: 60,
            velocity: 64,
        };
        assert_eq!(
            k.read_midi(&off),
            Some(Reading {
                value: 0.0,
                on: false
            })
        );
        let zero_vel = MidiMsg::NoteOn {
            channel: 3,
            note: 60,
            velocity: 0,
        };
        assert!(!k.read_midi(&zero_vel).unwrap().on);
        assert_eq!(k.read_midi(&cc(0, 60, 1)), None);
    }

    #[test]
    fn controls_read_keys_buttons_sticks_triggers_and_osc() {
        let space = Key::from_name("Space").unwrap();
        let k = Source::Key { key: space }.key();
        let ev = |control, value| ControlEvent::new(0, control, value);
        assert!(k.read_control(&ev(Control::Key(space), 1.0)).unwrap().on);
        assert!(!k.read_control(&ev(Control::Key(space), 0.0)).unwrap().on);
        assert_eq!(
            k.read_control(&ev(Control::Key(Key::from_char('a').unwrap()), 1.0)),
            None
        );

        let stick = Source::GamepadAxis {
            pad: None,
            axis: GamepadAxis::LeftX,
        }
        .key();
        let at = |v| {
            stick.read_control(&ev(
                Control::GamepadAxis {
                    pad: 2,
                    axis: GamepadAxis::LeftX,
                },
                v,
            ))
        };
        assert_eq!(at(-1.0).unwrap().value, 0.0);
        assert_eq!(at(0.0).unwrap().value, 0.5);
        assert_eq!(at(1.0).unwrap().value, 1.0);
        assert!(!at(0.0).unwrap().on, "a stick at rest isn't pressed");
        assert!(at(0.6).unwrap().on);
        assert_eq!(at(f32::NAN).unwrap().value, 0.5);

        let trig = Source::GamepadAxis {
            pad: Some(0),
            axis: GamepadAxis::RightTrigger,
        }
        .key();
        let t = trig
            .read_control(&ev(
                Control::GamepadAxis {
                    pad: 0,
                    axis: GamepadAxis::RightTrigger,
                },
                0.25,
            ))
            .unwrap();
        assert_eq!(t.value, 0.25);
        assert_eq!(
            trig.read_control(&ev(
                Control::GamepadAxis {
                    pad: 1,
                    axis: GamepadAxis::RightTrigger,
                },
                1.0,
            )),
            None,
            "other pad"
        );

        let button = Source::GamepadButton {
            pad: None,
            button: GamepadButton::East,
        }
        .key();
        let b = button
            .read_control(&ev(
                Control::GamepadButton {
                    pad: 3,
                    button: GamepadButton::East,
                },
                1.0,
            ))
            .unwrap();
        assert!(b.on);

        let osc = Source::Osc {
            address: "/fader/1".into(),
            arg: 0,
        }
        .key();
        let o = |v, arg| {
            osc.read_control(&ev(
                Control::Osc {
                    address: osc_address_hash("/fader/1"),
                    arg,
                },
                v,
            ))
        };
        assert_eq!(o(0.3, 0).unwrap().value, 0.3);
        assert_eq!(o(7.0, 0).unwrap().value, 1.0, "clamped");
        assert_eq!(o(-2.0, 0).unwrap().value, 0.0);
        assert_eq!(o(0.5, 1), None, "other argument");
    }

    #[test]
    fn ranges_map_linearly_log_inverted_and_rounded() {
        let gain = SocketSpec::float("gain", Unit::Db, 0.0).range(-60.0, 12.0);
        let b = Binding::input(
            Source::MidiCc {
                channel: None,
                cc: 1,
            },
            path(&[1]),
            "gain",
        );
        let m = Mapping::for_input(&b, &gain).unwrap();
        let set = |m: &Mapping, v: f32| match m.effect(
            Reading {
                value: v,
                on: v >= 0.5,
            },
            false,
        ) {
            Effect::Set(x) => x,
            other => panic!("expected a value, got {other:?}"),
        };
        assert_eq!(set(&m, 0.0), -60.0);
        assert_eq!(set(&m, 1.0), 12.0);
        assert!((set(&m, 0.5) + 24.0).abs() < 1e-4);

        let inv = Binding {
            invert: true,
            ..b.clone()
        };
        let m = Mapping::for_input(&inv, &gain).unwrap();
        assert_eq!(set(&m, 0.0), 12.0);

        // Hz inputs sweep on a log scale, like their sliders.
        let freq = SocketSpec::float("freq", Unit::Hz, 440.0).range(20.0, 20_000.0);
        let m = Mapping::for_input(&b, &freq).unwrap();
        assert_eq!(set(&m, 0.0), 20.0);
        assert_eq!(set(&m, 1.0), 20_000.0);
        assert!((set(&m, 0.5) - 632.4555).abs() < 0.01);
        // Custom ends, still log; a range through zero can't be.
        let narrow = Binding {
            min: Some(100.0),
            max: Some(1000.0),
            ..b.clone()
        };
        let m = Mapping::for_input(&narrow, &freq).unwrap();
        assert!((set(&m, 0.5) - 316.2278).abs() < 0.01);
        let through_zero = Binding {
            min: Some(-1.0),
            ..b.clone()
        };
        assert!(matches!(
            Mapping::for_input(&through_zero, &freq),
            Some(Mapping::Range { log: false, .. })
        ));

        // Int inputs round; choices sweep every choice.
        let count = SocketSpec::int("steps", 1).range(1.0, 16.0);
        let m = Mapping::for_input(&b, &count).unwrap();
        assert_eq!(set(&m, 0.5), 9.0);
        let shape = SocketSpec::choice("shape", &["sine", "saw", "square"], "sine");
        let m = Mapping::for_input(&b, &shape).unwrap();
        assert_eq!(set(&m, 0.74), 1.0);
        assert_eq!(set(&m, 1.0), 2.0);
        // No range: Float 0–1, Int 0–127.
        assert_eq!(
            default_range(&SocketSpec::float("x", Unit::Generic, 0.0)),
            (0.0, 1.0)
        );
        assert_eq!(default_range(&SocketSpec::int("n", 0)), (0.0, 127.0));
        // A non-finite end falls back to the default.
        let nan = Binding {
            max: Some(f64::NAN),
            ..b.clone()
        };
        assert_eq!(nan.range(&gain), (-60.0, 12.0));
        assert!(Mapping::for_input(&b, &SocketSpec::audio("in")).is_none());
        assert!(
            Mapping::for_input(&b, &SocketSpec::float("h", Unit::Generic, 0.0).hidden()).is_none()
        );
    }

    #[test]
    fn gates_follow_or_toggle_and_presses_fire_on_the_rising_edge() {
        let on = Reading {
            value: 1.0,
            on: true,
        };
        let off = Reading {
            value: 0.0,
            on: false,
        };
        let src = Source::Key {
            key: Key::from_char('k').unwrap(),
        };
        let gate_spec = SocketSpec::boolean("on", false);
        let mut b = Binding::input(src, path(&[1]), "on");
        let momentary = Mapping::for_input(&b, &gate_spec).unwrap();
        assert_eq!(momentary.effect(on, false), Effect::Set(1.0));
        assert_eq!(momentary.effect(on, true), Effect::Set(1.0));
        assert_eq!(momentary.effect(off, true), Effect::Set(0.0));
        b.invert = true;
        let inverted = Mapping::for_input(&b, &gate_spec).unwrap();
        assert_eq!(inverted.effect(on, false), Effect::Set(0.0));
        b.invert = false;
        b.mode = GateMode::Toggle;
        let toggle = Mapping::for_input(&b, &gate_spec).unwrap();
        assert_eq!(toggle.effect(on, false), Effect::Flip);
        assert_eq!(
            toggle.effect(on, true),
            Effect::None,
            "held, not pressed again"
        );
        assert_eq!(toggle.effect(off, true), Effect::None);

        let press = Mapping::for_input(&b, &SocketSpec::trigger("go")).unwrap();
        assert_eq!(press, Mapping::Press);
        assert_eq!(press.effect(on, false), Effect::Fire);
        assert_eq!(press.effect(on, true), Effect::None);
        assert_eq!(press.effect(off, true), Effect::None);
        // A controller crossing halfway presses once.
        let k = Source::MidiCc {
            channel: None,
            cc: 64,
        }
        .key();
        let mut was = false;
        let mut fired = 0;
        for v in [0u8, 40, 70, 90, 127, 30, 100] {
            let r = k.read_midi(&cc(0, 64, v)).unwrap();
            if press.effect(r, was) == Effect::Fire {
                fired += 1;
            }
            was = r.on;
        }
        assert_eq!(fired, 2);
    }

    #[test]
    fn learning_waits_for_real_movement() {
        let none = |_: u64| None;
        let mut l = Learner::new();
        // A controller has to move, not just report where it is.
        assert_eq!(l.feed(&InputEvent::Midi(cc(2, 21, 64)), &none), None);
        assert_eq!(l.feed(&InputEvent::Midi(cc(2, 21, 66)), &none), None);
        assert_eq!(
            l.feed(&InputEvent::Midi(cc(2, 21, 80)), &none),
            Some(Source::MidiCc {
                channel: Some(3),
                cc: 21
            })
        );
        // Notes, keys, and buttons bind on press, not release.
        let mut l = Learner::new();
        let off = MidiMsg::NoteOff {
            channel: 0,
            note: 61,
            velocity: 0,
        };
        assert_eq!(l.feed(&InputEvent::Midi(off), &none), None);
        let on = MidiMsg::NoteOn {
            channel: 0,
            note: 61,
            velocity: 5,
        };
        assert_eq!(
            l.feed(&InputEvent::Midi(on), &none),
            Some(Source::MidiNote {
                channel: Some(1),
                note: 61
            })
        );
        let x = Key::from_char('x').unwrap();
        assert_eq!(
            l.feed(&InputEvent::Control(Control::Key(x), 0.0), &none),
            None
        );
        assert_eq!(
            l.feed(&InputEvent::Control(Control::Key(x), 1.0), &none),
            Some(Source::Key { key: x })
        );
        // A resting stick that drifts a little doesn't bind.
        let axis = Control::GamepadAxis {
            pad: 1,
            axis: GamepadAxis::RightY,
        };
        let mut l = Learner::new();
        for v in [0.0, 0.05, -0.08, 0.1] {
            assert_eq!(l.feed(&InputEvent::Control(axis, v), &none), None);
        }
        assert_eq!(
            l.feed(&InputEvent::Control(axis, 0.6), &none),
            Some(Source::GamepadAxis {
                pad: Some(1),
                axis: GamepadAxis::RightY
            })
        );
        // OSC needs its address back from the hash.
        let h = osc_address_hash("/xy/x");
        let osc = Control::Osc { address: h, arg: 0 };
        let mut l = Learner::new();
        assert_eq!(l.feed(&InputEvent::Control(osc, 0.1), &none), None);
        assert_eq!(l.feed(&InputEvent::Control(osc, 0.9), &none), None);
        let named = |a: u64| (a == h).then(|| "/xy/x".to_string());
        assert_eq!(
            l.feed(&InputEvent::Control(osc, 0.95), &named),
            Some(Source::Osc {
                address: "/xy/x".into(),
                arg: 0
            })
        );
    }

    #[test]
    fn labels_read_like_people_talk() {
        assert_eq!(
            Source::MidiCc {
                channel: Some(1),
                cc: 74
            }
            .label(),
            "MIDI CC 74 · ch 1"
        );
        assert_eq!(
            Source::MidiNote {
                channel: None,
                note: 60
            }
            .label(),
            "Note C4 · any channel"
        );
        assert_eq!(
            Source::GamepadButton {
                pad: None,
                button: GamepadButton::South
            }
            .label(),
            "Gamepad A (south)"
        );
        assert_eq!(
            Source::GamepadAxis {
                pad: Some(0),
                axis: GamepadAxis::LeftY
            }
            .label(),
            "Pad 1 left stick Y"
        );
        assert_eq!(note_name(0), "C-1");
        assert_eq!(note_name(69), "A4");
    }
}
