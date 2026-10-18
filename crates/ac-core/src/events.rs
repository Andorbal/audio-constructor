//! Event payloads: MIDI messages with sample offsets.

/// Events one `Midi` output holds per block, in the engine and the test
/// harness. Past it an output drops events, so nodes that owe note-offs
/// hold back optional messages to keep room for them.
pub const MIDI_CAPACITY: usize = 1024;

/// A MIDI message stamped with its sample offset inside the current block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    pub offset: u32,
    pub msg: MidiMsg,
}

impl MidiEvent {
    pub fn new(offset: u32, msg: MidiMsg) -> Self {
        MidiEvent { offset, msg }
    }
}

/// A channel voice message. Channels are 0..=15 (shown to people as 1..=16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMsg {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    /// -8192..=8191, centered at 0.
    PitchBend {
        channel: u8,
        value: i16,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    PolyPressure {
        channel: u8,
        note: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
}

impl MidiMsg {
    /// Decode a raw channel message. Note-on with velocity 0 becomes note-off.
    /// System messages and malformed input return `None`.
    pub fn from_bytes(bytes: &[u8]) -> Option<MidiMsg> {
        let status = *bytes.first()?;
        let channel = status & 0x0f;
        let d1 = bytes.get(1).copied().unwrap_or(0) & 0x7f;
        let d2 = bytes.get(2).copied().unwrap_or(0) & 0x7f;
        let need = |n: usize| bytes.len() >= n;
        match status & 0xf0 {
            0x80 if need(3) => Some(MidiMsg::NoteOff {
                channel,
                note: d1,
                velocity: d2,
            }),
            0x90 if need(3) => Some(if d2 == 0 {
                MidiMsg::NoteOff {
                    channel,
                    note: d1,
                    velocity: 64,
                }
            } else {
                MidiMsg::NoteOn {
                    channel,
                    note: d1,
                    velocity: d2,
                }
            }),
            0xa0 if need(3) => Some(MidiMsg::PolyPressure {
                channel,
                note: d1,
                value: d2,
            }),
            0xb0 if need(3) => Some(MidiMsg::ControlChange {
                channel,
                controller: d1,
                value: d2,
            }),
            0xc0 if need(2) => Some(MidiMsg::ProgramChange {
                channel,
                program: d1,
            }),
            0xd0 if need(2) => Some(MidiMsg::ChannelPressure { channel, value: d1 }),
            0xe0 if need(3) => Some(MidiMsg::PitchBend {
                channel,
                value: ((i16::from(d2) << 7) | i16::from(d1)) - 8192,
            }),
            _ => None,
        }
    }

    /// Encode as raw bytes; returns the buffer and the number of bytes used.
    pub fn to_bytes(&self) -> ([u8; 3], usize) {
        match *self {
            MidiMsg::NoteOff {
                channel,
                note,
                velocity,
            } => ([0x80 | channel, note, velocity], 3),
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } => ([0x90 | channel, note, velocity], 3),
            MidiMsg::PolyPressure {
                channel,
                note,
                value,
            } => ([0xa0 | channel, note, value], 3),
            MidiMsg::ControlChange {
                channel,
                controller,
                value,
            } => ([0xb0 | channel, controller, value], 3),
            MidiMsg::ProgramChange { channel, program } => ([0xc0 | channel, program, 0], 2),
            MidiMsg::ChannelPressure { channel, value } => ([0xd0 | channel, value, 0], 2),
            MidiMsg::PitchBend { channel, value } => {
                let v = (i32::from(value) + 8192).clamp(0, 16383) as u16;
                ([0xe0 | channel, (v & 0x7f) as u8, (v >> 7) as u8], 3)
            }
        }
    }

    pub fn channel(&self) -> u8 {
        match *self {
            MidiMsg::NoteOn { channel, .. }
            | MidiMsg::NoteOff { channel, .. }
            | MidiMsg::ControlChange { channel, .. }
            | MidiMsg::PitchBend { channel, .. }
            | MidiMsg::ChannelPressure { channel, .. }
            | MidiMsg::PolyPressure { channel, .. }
            | MidiMsg::ProgramChange { channel, .. } => channel,
        }
    }
}

/// Frequency in Hz of a MIDI note number, A4 (69) = 440 Hz, equal temperament.
pub fn note_to_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_encode_round_trip() {
        let msgs = [
            MidiMsg::NoteOn {
                channel: 3,
                note: 60,
                velocity: 100,
            },
            MidiMsg::NoteOff {
                channel: 0,
                note: 61,
                velocity: 0,
            },
            MidiMsg::ControlChange {
                channel: 15,
                controller: 74,
                value: 127,
            },
            MidiMsg::PitchBend {
                channel: 1,
                value: -8192,
            },
            MidiMsg::PitchBend {
                channel: 1,
                value: 8191,
            },
            MidiMsg::ProgramChange {
                channel: 2,
                program: 5,
            },
        ];
        for m in msgs {
            let (b, n) = m.to_bytes();
            assert_eq!(MidiMsg::from_bytes(&b[..n]), Some(m));
        }
    }

    #[test]
    fn note_on_zero_velocity_is_note_off() {
        assert!(matches!(
            MidiMsg::from_bytes(&[0x90, 60, 0]),
            Some(MidiMsg::NoteOff { note: 60, .. })
        ));
    }

    #[test]
    fn malformed_input_is_none() {
        assert_eq!(MidiMsg::from_bytes(&[]), None);
        assert_eq!(MidiMsg::from_bytes(&[0x90, 60]), None);
        assert_eq!(MidiMsg::from_bytes(&[0xf8]), None);
    }

    #[test]
    fn a4_is_440() {
        assert!((note_to_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((note_to_hz(81.0) - 880.0).abs() < 1e-2);
    }
}
