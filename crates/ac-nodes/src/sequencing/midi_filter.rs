use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::notes::{RELEASE_VELOCITY, Sender, is_all_notes_off, note_off, note_off_of, note_on_of};
use crate::io::midi_in::channel_matches;

node! {
    kind: "midi_filter",
    name: "MIDI Filter",
    category: Sequencing,
    keywords: ["split", "keyboard split", "zone", "range", "channel", "block", "thru", "layer"],
    /// Passes only the MIDI you want: one channel, a range of notes, and chosen kinds of message.
    ///
    /// `channel` 0 passes every channel, 1–16 just one. Notes pass when
    /// their number is from `low` to `high` (inclusive; the two swap if
    /// `low` is above `high`), so two MIDI Filters with ranges 0–59 and
    /// 60–127 split a keyboard between two sounds. The toggles pass or
    /// block whole kinds of message: `notes` (note-ons and note-offs),
    /// `controllers` (control changes), `pitch_bend`, `pressure` (channel
    /// and polyphonic aftertouch), and `program` (program changes).
    /// Polyphonic pressure belongs to a note, so it also has to be in the
    /// range. The toggles are read on the sample each message arrives.
    ///
    /// A note-off always passes when its note-on did, so changing the
    /// range, the channel, or a toggle while keys are held never leaves a
    /// note hanging. All-notes-off and all-sound-off (controllers 123 and
    /// 120) release the notes that passed on their channel even when they
    /// are filtered out themselves: those notes get note-offs instead. A
    /// transport reset releases the notes that passed at the start of the
    /// next block.
    ///
    /// **Try this:** wire `MIDI In` into two MIDI Filters, one with `high`
    /// 59 feeding a bass synth and one with `low` 60 feeding a lead, for a
    /// split keyboard; or turn `notes` off to pass only the knobs on to a
    /// `MIDI CC`.
    pub struct MidiFilter {
        /// Notes whose note-on passed and whose note-off hasn't, per channel.
        passed: [u128; 16],
        /// A reset happened: release what passed at the next block.
        flush: bool,
    }
}

const MIDI: usize = 0;
const CHANNEL: usize = 1;
const LOW: usize = 2;
const HIGH: usize = 3;
const NOTES: usize = 4;
const CONTROLLERS: usize = 5;
const PITCH_BEND: usize = 6;
const PRESSURE: usize = 7;
const PROGRAM: usize = 8;

const OUT: usize = 0;

fn bit(note: u8) -> u128 {
    1u128 << (note & 127)
}

impl MidiFilter {
    fn release_channel(&mut self, tx: &mut Sender, offset: u32, channel: u8) {
        let ch = usize::from(channel & 15);
        let mut m = self.passed[ch];
        while m != 0 {
            let note = m.trailing_zeros() as u8;
            m &= m - 1;
            note_off(tx, offset, channel & 15, note, RELEASE_VELOCITY);
        }
        self.passed[ch] = 0;
    }
}

impl Node for MidiFilter {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Messages to filter."))
            .input(
                SocketSpec::int("channel", 0)
                    .range(0.0, 16.0)
                    .doc("MIDI channel to pass, 1–16; 0 passes all."),
            )
            .input(
                SocketSpec::int("low", 0)
                    .range(0.0, 127.0)
                    .doc("Lowest note that passes."),
            )
            .input(
                SocketSpec::int("high", 127)
                    .range(0.0, 127.0)
                    .doc("Highest note that passes."),
            )
            .input(SocketSpec::boolean("notes", true).doc("Pass note-ons and note-offs."))
            .input(SocketSpec::boolean("controllers", true).doc("Pass control changes."))
            .input(SocketSpec::boolean("pitch_bend", true).doc("Pass pitch bend."))
            .input(SocketSpec::boolean("pressure", true).doc("Pass channel and key pressure."))
            .input(SocketSpec::boolean("program", true).doc("Pass program changes."))
            .output(SocketSpec::midi("midi").doc("The messages that passed."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MidiFilter {
            passed: [0; 16],
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        let filter = io.inputs.int(CHANNEL);
        let a = io.inputs.int(LOW).clamp(0, 127) as u8;
        let b = io.inputs.int(HIGH).clamp(0, 127) as u8;
        let (low, high) = (a.min(b), a.max(b));
        let notes = io.inputs.boolean(NOTES);
        let controllers = io.inputs.boolean(CONTROLLERS);
        let bend = io.inputs.boolean(PITCH_BEND);
        let pressure = io.inputs.boolean(PRESSURE);
        let program = io.inputs.boolean(PROGRAM);
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut tx = Sender::new(io.outputs.midi(OUT));

        if std::mem::take(&mut self.flush) {
            for ch in 0..16 {
                self.release_channel(&mut tx, 0, ch);
            }
        }

        for ev in events {
            let at = ev.offset.min(last);
            let i = at as usize;
            let msg = ev.msg;
            let on_channel = channel_matches(filter, ev);
            let in_range = |note: u8| (low..=high).contains(&note);
            if let Some((ch, note, _)) = note_on_of(&msg) {
                if on_channel && notes.at(i) && in_range(note) {
                    self.passed[usize::from(ch)] |= bit(note);
                    tx.send(at, msg);
                }
            } else if let Some((ch, note, _)) = note_off_of(&msg) {
                let held = self.passed[usize::from(ch)] & bit(note) != 0;
                if held || (on_channel && notes.at(i) && in_range(note)) {
                    self.passed[usize::from(ch)] &= !bit(note);
                    tx.send(at, msg);
                }
            } else if is_all_notes_off(&msg) {
                if on_channel && controllers.at(i) {
                    // Whatever receives it releases the notes itself.
                    self.passed[usize::from(msg.channel() & 15)] = 0;
                    tx.send(at, msg);
                } else {
                    self.release_channel(&mut tx, at, msg.channel());
                }
            } else if on_channel {
                let pass = match msg {
                    MidiMsg::ControlChange { .. } => controllers.at(i),
                    MidiMsg::PitchBend { .. } => bend.at(i),
                    MidiMsg::ChannelPressure { .. } => pressure.at(i),
                    MidiMsg::PolyPressure { note, .. } => pressure.at(i) && in_range(note),
                    MidiMsg::ProgramChange { .. } => program.at(i),
                    MidiMsg::NoteOn { .. } | MidiMsg::NoteOff { .. } => false,
                };
                if pass {
                    tx.send(at, msg);
                }
            }
        }
    }

    fn reset(&mut self) {
        self.flush = self.passed.iter().any(|&m| m != 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity: 100,
        }
    }
    fn off(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel,
            note,
            velocity: 0,
        }
    }
    fn cc(channel: u8, controller: u8) -> MidiMsg {
        MidiMsg::ControlChange {
            channel,
            controller,
            value: 1,
        }
    }

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<MidiFilter>(json!({}), 48_000.0, block, &NoResources)
    }

    #[test]
    fn passes_everything_by_default() {
        let events = [
            (0, on(0, 60)),
            (1, cc(1, 7)),
            (
                2,
                MidiMsg::PitchBend {
                    channel: 2,
                    value: 100,
                },
            ),
            (
                3,
                MidiMsg::ChannelPressure {
                    channel: 3,
                    value: 9,
                },
            ),
            (
                4,
                MidiMsg::ProgramChange {
                    channel: 4,
                    program: 5,
                },
            ),
            (5, off(0, 60)),
        ];
        let mut h = harness(64);
        h.midi("midi", &events);
        assert_eq!(h.run(64).midi("midi"), &events);
    }

    #[test]
    fn note_range_and_channel() {
        let mut h = harness(64);
        h.set("low", 60).set("high", 64).set("channel", 2);
        h.midi(
            "midi",
            &[
                (0, on(1, 59)),
                (1, on(1, 60)),
                (2, on(1, 64)),
                (3, on(1, 65)),
                (4, on(0, 62)),
                (5, cc(0, 7)),
                (6, cc(1, 7)),
            ],
        );
        let out = h.run(64);
        assert_eq!(
            out.midi("midi"),
            &[(1, on(1, 60)), (2, on(1, 64)), (6, cc(1, 7))]
        );
    }

    #[test]
    fn a_reversed_range_is_swapped() {
        let mut h = harness(64);
        h.set("low", 64).set("high", 60);
        h.midi("midi", &[(0, on(0, 59)), (1, on(0, 62))]);
        assert_eq!(h.run(64).midi("midi"), &[(1, on(0, 62))]);
    }

    #[test]
    fn type_toggles() {
        let mut h = harness(64);
        h.set("notes", false)
            .set("controllers", false)
            .set("pitch_bend", false)
            .set("pressure", false);
        let program = MidiMsg::ProgramChange {
            channel: 0,
            program: 3,
        };
        h.midi(
            "midi",
            &[
                (0, on(0, 60)),
                (1, cc(0, 1)),
                (
                    2,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: 1,
                    },
                ),
                (
                    3,
                    MidiMsg::PolyPressure {
                        channel: 0,
                        note: 60,
                        value: 3,
                    },
                ),
                (4, program),
                (5, off(0, 60)),
            ],
        );
        assert_eq!(h.run(64).midi("midi"), &[(4, program)]);
    }

    #[test]
    fn a_note_off_follows_its_note_on_through_filter_changes() {
        let mut h = harness(64);
        h.midi("midi", &[(10, on(0, 60))]);
        h.run(64);
        // The range moves away and notes are switched off: the note-off
        // still passes, a new note-on doesn't.
        h.set("low", 70).set("notes", false).set("channel", 5);
        h.midi("midi", &[(64 + 5, off(0, 60)), (64 + 6, on(0, 60))]);
        assert_eq!(h.run(64).midi("midi"), &[(69, off(0, 60))]);
    }

    #[test]
    fn all_notes_off_releases_what_passed_even_when_blocked() {
        let mut h = harness(64);
        h.set("controllers", false);
        h.midi("midi", &[(0, on(0, 60)), (1, on(0, 67)), (9, cc(0, 123))]);
        let out = h.run(64);
        assert_eq!(
            out.midi("midi"),
            &[
                (0, on(0, 60)),
                (1, on(0, 67)),
                (9, off(0, 60).with_velocity(RELEASE_VELOCITY)),
                (9, off(0, 67).with_velocity(RELEASE_VELOCITY)),
            ]
        );
    }

    #[test]
    fn reset_releases_what_passed() {
        let mut h = harness(64);
        h.midi("midi", &[(0, on(3, 40))]);
        h.run(64);
        h.reset();
        h.midi("midi", &[]);
        assert_eq!(
            h.run(64).midi("midi"),
            &[(64, off(3, 40).with_velocity(RELEASE_VELOCITY))]
        );
        h.reset();
        assert!(h.run(64).midi("midi").is_empty());
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let events: Vec<(u64, MidiMsg)> = (0..60u64)
            .map(|k| {
                let note = 50 + (k % 20) as u8;
                (
                    k * 37,
                    if k % 3 == 2 {
                        off(0, note - 2)
                    } else {
                        on(0, note)
                    },
                )
            })
            .collect();
        let render = |block| {
            let mut h = harness(block);
            h.set("low", 55).set("high", 65);
            h.midi("midi", &events);
            h.run(2_400).midi("midi").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    trait WithVelocity {
        fn with_velocity(self, v: u8) -> MidiMsg;
    }
    impl WithVelocity for MidiMsg {
        fn with_velocity(self, v: u8) -> MidiMsg {
            match self {
                MidiMsg::NoteOff { channel, note, .. } => MidiMsg::NoteOff {
                    channel,
                    note,
                    velocity: v,
                },
                other => other,
            }
        }
    }
}
