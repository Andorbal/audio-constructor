use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};
use ac_dsp::scale::{ROOT_NAMES, SCALE_NAMES};

use super::notes::{
    MAJOR_MASK, NoteMap, Sender, chosen_scale, is_all_notes_off, note_off_of, note_on_of,
};

node! {
    kind: "midi_transpose",
    name: "MIDI Transpose",
    category: Sequencing,
    keywords: ["transpose", "shift", "octave", "key", "pitch", "scale", "snap", "harmonize"],
    /// Shifts MIDI notes up or down by semitones, optionally snapping them into a scale.
    ///
    /// Every note moves by `semitones` (−48 to 48; 12 is an octave up).
    /// With `snap` on, the shifted note then moves to the nearest note of
    /// `scale` over `root` (exactly halfway rounds down), which pushes a
    /// part into a key. `custom` uses `mask`, the scale as 12 bits: bit 0 is
    /// the root, bit 1 a semitone above, and so on (2741 is the major
    /// scale). Notes that would land outside 0–127 are dropped, and so are
    /// their note-offs.
    ///
    /// Each held note remembers the note it started, so its note-off ends
    /// that note even if `semitones`, `snap`, or the scale change while it's
    /// held. When two held keys land on the same note (common with `snap`),
    /// it sounds once and ends when the last of them is released.
    /// Polyphonic pressure follows its note; controllers, pitch bend,
    /// channel pressure, and program changes pass through unchanged.
    /// All-notes-off and all-sound-off release the notes on their channel,
    /// and a transport reset releases every sounding note at the start of
    /// the next block.
    ///
    /// **Try this:** put MIDI Transpose with `semitones` 12 between `MIDI In`
    /// and a second synth to double a part an octave up; or turn on `snap`
    /// with `scale` minor_pentatonic and `root` A, and every key you play
    /// lands in A minor pentatonic.
    pub struct MidiTranspose {
        map: NoteMap<1>,
        /// A reset happened: release sounding notes at the next block.
        flush: bool,
    }
}

const MIDI: usize = 0;
const SEMITONES: usize = 1;
const SNAP: usize = 2;
const SCALE: usize = 3;
const ROOT: usize = 4;
const MASK: usize = 5;

const OUT: usize = 0;

impl Node for MidiTranspose {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Notes to shift."))
            .input(
                SocketSpec::int("semitones", 0)
                    .range(-48.0, 48.0)
                    .doc("How far to shift; 12 is an octave."),
            )
            .input(SocketSpec::boolean("snap", false).doc("Snap shifted notes into the scale."))
            .input(SocketSpec::choice("scale", SCALE_NAMES, "major").doc("Scale to snap to."))
            .input(SocketSpec::choice("root", ROOT_NAMES, "C").doc("The scale's root note."))
            .input(
                SocketSpec::int("mask", MAJOR_MASK)
                    .range(0.0, 4095.0)
                    .doc("Custom scale: bit 0 the root, bit 1 a semitone up, and so on."),
            )
            .output(SocketSpec::midi("midi").doc("The shifted notes and everything else."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MidiTranspose {
            map: NoteMap::new(),
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        let semitones = io.inputs.int(SEMITONES).clamp(-127, 127);
        let snap = io.inputs.boolean(SNAP);
        let scale = chosen_scale(io, SCALE, MASK);
        let root = io.inputs.choice(ROOT, ROOT_NAMES.len()) as i32;
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut tx = Sender::new(io.outputs.midi(OUT));

        if std::mem::take(&mut self.flush) {
            self.map.release_all(&mut tx, 0);
        }
        for ev in events {
            let at = ev.offset.min(last);
            if let Some((ch, note, velocity)) = note_on_of(&ev.msg) {
                let mut out = i32::from(note) + semitones;
                if snap.at(at as usize) {
                    out = scale.quantize(f64::from(out), root) as i32;
                }
                self.map.press(&mut tx, at, ch, note, velocity, &[out]);
            } else if let Some((ch, note, velocity)) = note_off_of(&ev.msg) {
                self.map.release(&mut tx, at, ch, note, velocity);
            } else if let MidiMsg::PolyPressure {
                channel,
                note,
                value,
            } = ev.msg
            {
                let reserve = self.map.sounding();
                for &out in self.map.started(channel, note) {
                    let msg = MidiMsg::PolyPressure {
                        channel,
                        note: out,
                        value,
                    };
                    tx.send_if_room(at, msg, reserve);
                }
            } else {
                if is_all_notes_off(&ev.msg) {
                    self.map.release_channel(&mut tx, at, ev.msg.channel());
                }
                tx.send_if_room(at, ev.msg, self.map.sounding());
            }
        }
    }

    fn reset(&mut self) {
        self.flush = self.map.sounding() > 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }
    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<MidiTranspose>(json!({}), 48_000.0, block, &NoResources)
    }

    /// (frame, +note for on / -note for off).
    fn notes(out: &[(u64, MidiMsg)]) -> Vec<(u64, i32)> {
        out.iter()
            .filter_map(|(t, m)| match m {
                MidiMsg::NoteOn { note, .. } => Some((*t, i32::from(*note))),
                MidiMsg::NoteOff { note, .. } => Some((*t, -i32::from(*note))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shifts_notes_on_their_samples() {
        let mut h = harness(64);
        h.set("semitones", 12);
        h.midi("midi", &[(5, on(60)), (100, off(60))]);
        let out = h.run(128);
        assert_eq!(notes(out.midi("midi")), vec![(5, 72), (100, -72)]);
        assert_eq!(
            out.midi("midi")[0].1,
            MidiMsg::NoteOn {
                channel: 0,
                note: 72,
                velocity: 100
            }
        );
    }

    #[test]
    fn note_offs_match_after_the_shift_changes() {
        let mut h = harness(64);
        h.set("semitones", 7);
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.set("semitones", -5);
        h.midi("midi", &[(70, off(60)), (80, on(60)), (90, off(60))]);
        let out = h.run(64);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(70, -67), (80, 55), (90, -55)]
        );
    }

    #[test]
    fn snap_quantizes_the_shifted_note() {
        let mut h = harness(64);
        h.set("snap", true)
            .set("scale", "minor_pentatonic")
            .set("root", "A")
            .set("semitones", 1);
        // A minor pentatonic: A C D E G. 60+1 = C# → C (tie goes down);
        // 63+1 = E stays; 65+1 = F# → G... F# is between E and G: 66 is 2
        // from 64 and 1 from 67, so G.
        h.midi("midi", &[(0, on(60)), (1, on(63)), (2, on(65))]);
        assert_eq!(
            notes(h.run(64).midi("midi")),
            vec![(0, 60), (1, 64), (2, 67)]
        );
    }

    #[test]
    fn custom_mask() {
        let mut h = harness(64);
        h.set("snap", true)
            .set("scale", "custom")
            .set("mask", 0b1001_0001);
        // C, E, G over C: 62 → 60 (tie down), 66 → 67.
        h.midi("midi", &[(0, on(62)), (1, on(66))]);
        assert_eq!(notes(h.run(64).midi("midi")), vec![(0, 60), (1, 67)]);
    }

    #[test]
    fn out_of_range_notes_are_dropped_with_their_note_offs() {
        let mut h = harness(64);
        h.set("semitones", 48);
        h.midi(
            "midi",
            &[(0, on(100)), (1, on(70)), (5, off(100)), (6, off(70))],
        );
        assert_eq!(notes(h.run(64).midi("midi")), vec![(1, 118), (6, -118)]);
    }

    #[test]
    fn keys_landing_on_one_note_sound_it_once() {
        let mut h = harness(64);
        h.set("snap", true); // C major
        h.midi(
            "midi",
            &[(0, on(60)), (1, on(61)), (2, off(60)), (3, off(61))],
        );
        assert_eq!(notes(h.run(64).midi("midi")), vec![(0, 60), (3, -60)]);
    }

    #[test]
    fn other_messages_pass_and_pressure_follows_its_note() {
        let mut h = harness(64);
        h.set("semitones", 2);
        let bend = MidiMsg::PitchBend {
            channel: 1,
            value: -3,
        };
        let cc = MidiMsg::ControlChange {
            channel: 0,
            controller: 1,
            value: 9,
        };
        h.midi(
            "midi",
            &[
                (0, on(60)),
                (1, bend),
                (
                    2,
                    MidiMsg::PolyPressure {
                        channel: 0,
                        note: 60,
                        value: 40,
                    },
                ),
                (3, cc),
            ],
        );
        let out = h.run(64);
        let m = out.midi("midi");
        assert_eq!(m[1], (1, bend));
        assert_eq!(
            m[2],
            (
                2,
                MidiMsg::PolyPressure {
                    channel: 0,
                    note: 62,
                    value: 40
                }
            )
        );
        assert_eq!(m[3], (3, cc));
    }

    #[test]
    fn all_notes_off_and_reset_release_sounding_notes() {
        let mut h = harness(64);
        h.set("semitones", 1);
        let panic = MidiMsg::ControlChange {
            channel: 0,
            controller: 123,
            value: 0,
        };
        h.midi(
            "midi",
            &[(0, on(60)), (1, on(62)), (4, panic), (9, off(60))],
        );
        let out = h.run(64);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(0, 61), (1, 63), (4, -61), (4, -63)]
        );
        assert_eq!(out.midi("midi")[4], (4, panic));

        let mut h = harness(64);
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.reset();
        h.midi("midi", &[]);
        assert_eq!(notes(h.run(64).midi("midi")), vec![(64, -60)]);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let events: Vec<(u64, MidiMsg)> = (0..80u64)
            .map(|k| {
                let note = 40 + (k % 30) as u8;
                (k * 29, if k % 4 == 3 { off(note - 3) } else { on(note) })
            })
            .collect();
        let render = |block| {
            let mut h = harness(block);
            h.set("semitones", 5)
                .set("snap", true)
                .set("scale", "dorian");
            h.midi("midi", &events);
            h.run(2_400).midi("midi").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
