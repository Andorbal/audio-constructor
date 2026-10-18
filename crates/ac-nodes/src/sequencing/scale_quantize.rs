use ac_core::{
    Config, ConfigExt, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::scale::{ROOT_NAMES, SCALE_NAMES};

use super::notes::{
    MAJOR_MASK, NoteMap, Sender, chosen_scale, is_all_notes_off, note_off_of, note_on_of,
};

node! {
    kind: "scale_quantize",
    name: "Scale Quantize",
    category: Sequencing,
    keywords: [
        "quantize", "quantizer", "scale", "key", "snap", "tune", "pentatonic", "generative",
        "pitch", "melody"
    ],
    /// Snaps notes to a scale: a `Float` note number, or the notes of a MIDI stream.
    ///
    /// The `type` config picks the form. `float` (the default) quantizes a
    /// number: `in` is a MIDI note number (60 is middle C; fractions are
    /// fine, and `Int` sources wire straight in), `out` is the nearest note
    /// of the scale as a whole number, at the input's rate, and `note` is
    /// the same as an `Int`, taken at the start of each block. A non-finite
    /// `in` gives 0, and inputs beyond ±10 000 are clamped first.
    ///
    /// `midi` quantizes the notes in `midi` instead. Each held note
    /// remembers the note it started, so its note-off ends that note even
    /// if the scale changes while it's held; two keys landing on the same
    /// note sound it once, until the last is released. Polyphonic pressure
    /// follows its note and other messages pass through. All-notes-off and
    /// all-sound-off release the notes on their channel, and a transport
    /// reset releases every sounding note at the start of the next block.
    ///
    /// Either way, `scale` and `root` choose the notes allowed (C major by
    /// default) and `custom` uses `mask`, the scale as 12 bits: bit 0 is
    /// the root, bit 1 a semitone above, and so on (2741 is the major
    /// scale; 0 counts as every note). Exactly halfway between two scale
    /// notes rounds down.
    ///
    /// **Try this:** wire a slow `Oscillator` (`pure`, 0.2 Hz) through a
    /// `Map` (−1..1 → 48..72) into `in`, and `out` through a `Convert` (MIDI
    /// note → Hz) into a second Oscillator's `freq`: with `scale`
    /// minor_pentatonic the sweep turns into a melody that never plays a
    /// wrong note. Set `type` to midi after `MIDI In` to keep any keyboard
    /// part in key.
    pub struct ScaleQuantize {
        /// The `midi` type.
        midi: bool,
        map: NoteMap<1>,
        /// A reset happened: release sounding notes at the next block.
        flush: bool,
    }
}

// Inputs: `in` (float) or `midi` (midi) first, then the scale.
const SOURCE: usize = 0;
const SCALE: usize = 1;
const ROOT: usize = 2;
const MASK: usize = 3;

// Float outputs.
const OUT: usize = 0;
const NOTE: usize = 1;
// MIDI output.
const MIDI_OUT: usize = 0;

/// Inputs are clamped to this many semitones either side of 0.
const LIMIT: f64 = 10_000.0;

fn is_midi(config: &Config) -> bool {
    config.get_str_or("type", "float") == "midi"
}

impl ScaleQuantize {
    fn process_midi(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(SOURCE);
        let scale = chosen_scale(io, SCALE, MASK);
        let root = io.inputs.choice(ROOT, ROOT_NAMES.len()) as i32;
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut tx = Sender::new(io.outputs.midi(MIDI_OUT));

        if std::mem::take(&mut self.flush) {
            self.map.release_all(&mut tx, 0);
        }
        for ev in events {
            let at = ev.offset.min(last);
            if let Some((ch, note, velocity)) = note_on_of(&ev.msg) {
                let out = scale.quantize(f64::from(note), root) as i32;
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
}

impl Node for ScaleQuantize {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let sig = if is_midi(config) {
            Signature::new().input(SocketSpec::midi("midi").doc("Notes to snap."))
        } else {
            Signature::new().input(
                SocketSpec::float("in", Unit::Generic, 60.0)
                    .range(0.0, 127.0)
                    .no_smooth()
                    .doc("A MIDI note number (fractions allowed)."),
            )
        };
        let sig = sig
            .input(SocketSpec::choice("scale", SCALE_NAMES, "major").doc("Scale to snap to."))
            .input(SocketSpec::choice("root", ROOT_NAMES, "C").doc("The scale's root note."))
            .input(
                SocketSpec::int("mask", MAJOR_MASK)
                    .range(0.0, 4095.0)
                    .doc("Custom scale: bit 0 the root, bit 1 a semitone up, and so on."),
            );
        if is_midi(config) {
            sig.output(SocketSpec::midi("midi").doc("The snapped notes and everything else."))
        } else {
            sig.output(
                SocketSpec::float("out", Unit::Generic, 60.0)
                    .poly()
                    .doc("The nearest scale note."),
            )
            .output(SocketSpec::int("note", 60).doc("The nearest scale note, per block."))
        }
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        ScaleQuantize {
            midi: is_midi(config),
            map: NoteMap::new(),
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        if self.midi {
            self.process_midi(ctx, io);
            return;
        }
        let input = io.inputs.float(SOURCE);
        let scale = chosen_scale(io, SCALE, MASK);
        let root = io.inputs.choice(ROOT, ROOT_NAMES.len()) as i32;
        let snap = |v: f32| {
            let v = if v.is_finite() {
                f64::from(v).clamp(-LIMIT, LIMIT)
            } else {
                0.0
            };
            scale.quantize(v, root)
        };
        let first = snap(input.first());
        let mut out = io.outputs.float(OUT);
        if input.is_varying() {
            out.fill_with(|i| snap(input.at(i)) as f32);
        } else {
            out.fill(first as f32);
        }
        io.outputs.set_int(NOTE, first as i32);
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

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<ScaleQuantize>(config, 48_000.0, block, &NoResources)
    }

    #[test]
    fn sockets_follow_the_type() {
        let f = ScaleQuantize::describe(&json!({}));
        let names: Vec<_> = f.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names, ["in", "scale", "root", "mask"]);
        assert_eq!(f.outputs.len(), 2);
        let m = ScaleQuantize::describe(&json!({ "type": "midi" }));
        assert_eq!(m.inputs[0].name, "midi");
        assert_eq!(m.outputs[0].name, "midi");
        assert_eq!(m.outputs.len(), 1);
    }

    #[test]
    fn floats_snap_to_the_nearest_scale_note() {
        let mut h = harness(json!({}), 4);
        // C major: 61 ties down to 60; 61.6 → 62; 66 → 65; 70.9 → 71.
        h.set_samples("in", vec![61.0, 61.6, 66.0, 70.9]);
        let out = h.run(4);
        assert_eq!(out.float("out"), &[60.0, 62.0, 65.0, 71.0]);
        assert_eq!(out.int("note")[0], 60);
    }

    #[test]
    fn root_scale_and_mask() {
        let mut h = harness(json!({}), 64);
        h.set("scale", "minor_pentatonic")
            .set("root", "A")
            .set("in", 70.0);
        assert_eq!(h.run(64).float("out")[0], 69.0);

        let mut h = harness(json!({}), 64);
        h.set("scale", "custom")
            .set("mask", 0b1001_0001)
            .set("in", 66.0);
        let out = h.run(64);
        assert_eq!(out.float("out")[0], 67.0);
        assert_eq!(out.int("note")[0], 67);
    }

    #[test]
    fn bad_inputs_stay_finite() {
        let mut h = harness(json!({}), 4);
        h.set_samples("in", vec![f32::NAN, f32::INFINITY, -1.0e30, 1.0e30]);
        let out = h.run(4);
        assert!(out.float("out").iter().all(|v| v.is_finite()));
        assert_eq!(out.float("out")[0], 0.0);
        assert!(out.float("out")[3] <= LIMIT as f32);
    }

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 90,
        }
    }
    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

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
    fn midi_notes_snap_and_note_offs_match_after_a_scale_change() {
        let mut h = harness(json!({ "type": "midi" }), 64);
        h.midi("midi", &[(3, on(61)), (4, on(66))]);
        assert_eq!(notes(h.run(64).midi("midi")), vec![(3, 60), (4, 65)]);
        h.set("scale", "lydian");
        h.midi(
            "midi",
            &[(64 + 1, off(66)), (64 + 2, on(66)), (64 + 9, off(61))],
        );
        assert_eq!(
            notes(h.run(64).midi("midi")),
            vec![(65, -65), (66, 66), (73, -60)]
        );
    }

    #[test]
    fn midi_shared_notes_and_reset() {
        let mut h = harness(json!({ "type": "midi" }), 64);
        h.midi("midi", &[(0, on(60)), (1, on(61)), (2, off(61))]);
        assert_eq!(notes(h.run(64).midi("midi")), vec![(0, 60)]);
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
        let samples: Vec<f32> = (0..2_400)
            .map(|i| 48.0 + (i as f32 * 0.01).sin() * 12.0)
            .collect();
        let render = |block| {
            let mut m = harness(json!({ "type": "midi" }), block);
            m.set("scale", "blues").set("root", "E");
            m.midi("midi", &events);
            let mut f = harness(json!({}), block);
            f.set("scale", "blues").set_samples("in", samples.clone());
            (
                m.run(2_400).midi("midi").to_vec(),
                f.run(2_400).float("out").to_vec(),
            )
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
