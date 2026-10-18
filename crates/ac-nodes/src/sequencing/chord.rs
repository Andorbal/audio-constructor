use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::notes::{NoteMap, Sender, is_all_notes_off, note_off_of, note_on_of};

node! {
    kind: "chord",
    name: "Chord",
    category: Sequencing,
    keywords: [
        "harmony", "triad", "seventh", "interval", "harmonizer", "voicing", "power chord",
        "inversion", "notes"
    ],
    /// Plays a whole chord for every incoming note.
    ///
    /// `chord` picks the shape built on each note: major, minor, sus2,
    /// sus4, dim, aug, maj7, min7, dom7, power (root and fifth), octave
    /// (root and the octave above), or custom, which uses the `intervals`
    /// config: up to 8 semitone offsets from the played note, −48 to 48,
    /// such as `[0, 3, 7, 10]` (an empty list plays just the note).
    /// `inversion` (0–3) raises the lowest note by an octave that many
    /// times, so 1 is the first inversion. `spread` (0–3 octaves) opens the
    /// voicing by raising every second note, counted from the bottom, by
    /// that many octaves.
    ///
    /// Every chord note gets the played note's velocity and channel. Each
    /// note-off releases exactly the notes its note-on started, even if the
    /// settings changed in between. Notes that would fall outside 0–127 are
    /// left out. When two held chords share a note, it sounds once and ends
    /// with the last chord that holds it. Polyphonic pressure goes to every
    /// note of its chord; other messages pass through. All-notes-off and
    /// all-sound-off release the chords on their channel, and a transport
    /// reset releases every sounding note at the start of the next block.
    ///
    /// **Try this:** `MIDI In` → Chord (min7) → a `Voices` synth, and every
    /// key plays a minor seventh; set `chord` to power with `spread` 1 for
    /// wide fifths under a bass line.
    pub struct Chord {
        /// The `intervals` config, repaired.
        custom: [i32; MAX_NOTES],
        custom_len: usize,
        map: NoteMap<MAX_NOTES>,
        /// A reset happened: release sounding notes at the next block.
        flush: bool,
    }
}

const MIDI: usize = 0;
const CHORD: usize = 1;
const INVERSION: usize = 2;
const SPREAD: usize = 3;

const OUT: usize = 0;

/// Most notes a chord can have (custom intervals beyond this are ignored).
pub const MAX_NOTES: usize = 8;

const CHORDS: &[&str] = &[
    "major", "minor", "sus2", "sus4", "dim", "aug", "maj7", "min7", "dom7", "power", "octave",
    "custom",
];

/// Semitones above the played note for each built-in chord.
const SHAPES: [&[i32]; 11] = [
    &[0, 4, 7],
    &[0, 3, 7],
    &[0, 2, 7],
    &[0, 5, 7],
    &[0, 3, 6],
    &[0, 4, 8],
    &[0, 4, 7, 11],
    &[0, 3, 7, 10],
    &[0, 4, 7, 10],
    &[0, 7],
    &[0, 12],
];

const MAX_INVERSION: i32 = 3;
const MAX_SPREAD: i32 = 3;
const MAX_INTERVAL: f64 = 48.0;

/// The `intervals` config: numbers rounded and clamped to ±48, others
/// skipped, at most `MAX_NOTES`; empty means just the note.
fn custom_intervals(config: &Config) -> ([i32; MAX_NOTES], usize) {
    let mut out = [0; MAX_NOTES];
    let mut len = 0;
    if let Some(list) = config.get("intervals").and_then(|v| v.as_array()) {
        for v in list {
            if len >= MAX_NOTES {
                break;
            }
            if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                out[len] = x.round().clamp(-MAX_INTERVAL, MAX_INTERVAL) as i32;
                len += 1;
            }
        }
    }
    if len == 0 {
        len = 1;
    }
    (out, len)
}

/// Semitone offsets of the voiced chord, lowest first: `inversion` times
/// the lowest note goes up an octave, then every second note goes up
/// `spread` octaves.
fn voicing(shape: &[i32], inversion: i32, spread: i32) -> ([i32; MAX_NOTES], usize) {
    let mut v = [0; MAX_NOTES];
    let n = shape.len().min(MAX_NOTES);
    v[..n].copy_from_slice(&shape[..n]);
    let notes = &mut v[..n];
    for _ in 0..inversion.clamp(0, MAX_INVERSION) {
        if let Some(low) = notes.iter_mut().min() {
            *low += 12;
        }
    }
    notes.sort_unstable();
    let spread = spread.clamp(0, MAX_SPREAD);
    for x in notes.iter_mut().skip(1).step_by(2) {
        *x += 12 * spread;
    }
    notes.sort_unstable();
    (v, n)
}

impl Node for Chord {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Notes to build chords on."))
            .input(SocketSpec::choice("chord", CHORDS, "major").doc("The chord shape."))
            .input(
                SocketSpec::int("inversion", 0)
                    .range(0.0, 3.0)
                    .doc("Raise the lowest note an octave this many times."),
            )
            .input(
                SocketSpec::int("spread", 0)
                    .range(0.0, 3.0)
                    .doc("Octaves to raise every second note by."),
            )
            .output(SocketSpec::midi("midi").doc("The chords and everything else."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (custom, custom_len) = custom_intervals(config);
        Chord {
            custom,
            custom_len,
            map: NoteMap::new(),
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        let shape: &[i32] = match SHAPES.get(io.inputs.choice(CHORD, CHORDS.len())) {
            Some(s) => s,
            None => &self.custom[..self.custom_len],
        };
        let (offsets, n) = voicing(shape, io.inputs.int(INVERSION), io.inputs.int(SPREAD));
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut tx = Sender::new(io.outputs.midi(OUT));

        if std::mem::take(&mut self.flush) {
            self.map.release_all(&mut tx, 0);
        }
        for ev in events {
            let at = ev.offset.min(last);
            if let Some((ch, note, velocity)) = note_on_of(&ev.msg) {
                let mut outs = [0i32; MAX_NOTES];
                for (o, &d) in outs.iter_mut().zip(&offsets[..n]) {
                    *o = i32::from(note) + d;
                }
                self.map.press(&mut tx, at, ch, note, velocity, &outs[..n]);
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
            channel: 2,
            note,
            velocity: 77,
        }
    }
    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 2,
            note,
            velocity: 0,
        }
    }

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<Chord>(config, 48_000.0, block, &NoResources)
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

    fn chord_of(
        params: &[(&str, ac_core::Value)],
        config: serde_json::Value,
        note: u8,
    ) -> Vec<i32> {
        let mut h = harness(config, 64);
        for (k, v) in params {
            h.set(k, v.clone());
        }
        h.midi("midi", &[(0, on(note))]);
        notes(h.run(64).midi("midi"))
            .into_iter()
            .map(|(_, n)| n)
            .collect()
    }

    #[test]
    fn built_in_shapes() {
        let c = |name: &str| chord_of(&[("chord", name.into())], json!({}), 60);
        assert_eq!(c("major"), vec![60, 64, 67]);
        assert_eq!(c("minor"), vec![60, 63, 67]);
        assert_eq!(c("sus2"), vec![60, 62, 67]);
        assert_eq!(c("sus4"), vec![60, 65, 67]);
        assert_eq!(c("dim"), vec![60, 63, 66]);
        assert_eq!(c("aug"), vec![60, 64, 68]);
        assert_eq!(c("maj7"), vec![60, 64, 67, 71]);
        assert_eq!(c("min7"), vec![60, 63, 67, 70]);
        assert_eq!(c("dom7"), vec![60, 64, 67, 70]);
        assert_eq!(c("power"), vec![60, 67]);
        assert_eq!(c("octave"), vec![60, 72]);
    }

    #[test]
    fn inversions_and_spread() {
        let c = |inv: i64, spread: i64| {
            chord_of(
                &[("inversion", inv.into()), ("spread", spread.into())],
                json!({}),
                60,
            )
        };
        assert_eq!(c(1, 0), vec![64, 67, 72]);
        assert_eq!(c(2, 0), vec![67, 72, 76]);
        assert_eq!(c(3, 0), vec![72, 76, 79]);
        assert_eq!(c(9, 0), vec![72, 76, 79], "clamped to 3");
        assert_eq!(c(0, 1), vec![60, 67, 76]);
        assert_eq!(c(1, 2), vec![64, 72, 91]);
    }

    #[test]
    fn custom_intervals_are_repaired() {
        let c = |cfg| chord_of(&[("chord", "custom".into())], cfg, 60);
        assert_eq!(
            c(json!({ "intervals": [0, 3, 7, 10] })),
            vec![60, 63, 67, 70]
        );
        assert_eq!(c(json!({ "intervals": [] })), vec![60]);
        assert_eq!(c(json!({})), vec![60]);
        assert_eq!(
            c(json!({ "intervals": [7, "x", -12.4, 99, 0] })),
            vec![48, 60, 67, 108]
        );
        let many: Vec<i32> = (0..12).collect();
        assert_eq!(c(json!({ "intervals": many })).len(), MAX_NOTES);
    }

    #[test]
    fn note_offs_release_exactly_what_started() {
        let mut h = harness(json!({}), 64);
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.set("chord", "min7").set("inversion", 2);
        h.midi("midi", &[(64 + 10, off(60))]);
        let out = h.run(64);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(74, -60), (74, -64), (74, -67)]
        );
        assert!(out.midi("midi").iter().all(|(_, m)| m.channel() == 2));
    }

    #[test]
    fn shared_notes_sound_once_and_out_of_range_notes_are_left_out() {
        let mut h = harness(json!({}), 64);
        // C major (60 64 67) and A major (57 61 64) share E.
        h.midi(
            "midi",
            &[(0, on(60)), (1, on(57)), (2, off(60)), (3, off(57))],
        );
        assert_eq!(
            notes(h.run(64).midi("midi")),
            vec![
                (0, 60),
                (0, 64),
                (0, 67),
                (1, 57),
                (1, 61),
                (2, -60),
                (2, -67),
                (3, -57),
                (3, -61),
                (3, -64)
            ]
        );

        let mut h = harness(json!({}), 64);
        h.midi("midi", &[(0, on(122)), (5, off(122))]);
        assert_eq!(
            notes(h.run(64).midi("midi")),
            vec![(0, 122), (0, 126), (5, -122), (5, -126)]
        );
    }

    #[test]
    fn velocity_and_pass_through() {
        let mut h = harness(json!({}), 64);
        let bend = MidiMsg::PitchBend {
            channel: 2,
            value: 77,
        };
        h.midi("midi", &[(0, on(60)), (1, bend)]);
        let out = h.run(64);
        let m = out.midi("midi");
        assert!(m[..3].iter().all(|(_, msg)| matches!(
            msg,
            MidiMsg::NoteOn {
                velocity: 77,
                channel: 2,
                ..
            }
        )));
        assert_eq!(m[3], (1, bend));
    }

    #[test]
    fn reset_releases_sounding_chords() {
        let mut h = harness(json!({}), 64);
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.reset();
        h.midi("midi", &[]);
        assert_eq!(
            notes(h.run(64).midi("midi")),
            vec![(64, -60), (64, -64), (64, -67)]
        );
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
            let mut h = harness(json!({}), block);
            h.set("chord", "dom7").set("spread", 1);
            h.midi("midi", &events);
            h.run(2_400).midi("midi").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
