use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::notes::{RELEASE_VELOCITY, Sender, note_off};

node! {
    kind: "note_to_midi",
    name: "Note To MIDI",
    category: Sequencing,
    keywords: [
        "gate to midi", "trigger to midi", "keyboard", "button", "note", "cv to midi", "notes"
    ],
    /// Builds MIDI notes from signals: a gate or a trigger, a note number, and a velocity.
    ///
    /// With `gate` wired, a note starts when the gate rises and ends when
    /// it falls. Otherwise each `trigger` starts a note that ends `length`
    /// seconds later (at least one sample); the unwired gate's own toggle
    /// still plays a note while it's on. `note` (a MIDI note number, read
    /// when the note starts, rounded and kept within 0–127; `Int` sources
    /// wire straight in), `velocity` (0–1, at least the softest MIDI
    /// velocity), and `channel` (1–16) are taken when each note starts, and
    /// the note-off always matches the note that started.
    ///
    /// One note sounds at a time: a new note ends the previous one first,
    /// on the same sample (note-off before note-on). On the same sample a
    /// falling gate or an ending note comes first, then a rising gate, then
    /// a trigger. A transport reset ends the sounding note at the start of
    /// the next block, and a gate still high afterwards starts a new note.
    ///
    /// **Try this:** wire several `Button`s through `Select` into `note`
    /// and their gates through `Logic` (or) into `gate` to build a
    /// keyboard, then feed `midi` into a `Voices` synth; or drive `trigger`
    /// from a `Clock` and `note` from a `Random` for a generative line.
    pub struct NoteToMidi {
        /// Last frame's gate.
        gate: bool,
        /// The sounding note: (channel, note).
        sounding: Option<(u8, u8)>,
        /// Samples (on this node's own clock) when a triggered note ends.
        off_at: Option<u64>,
        /// This node's sample clock at the start of the block.
        now: u64,
        sample_rate: f64,
        /// A reset happened: end the sounding note at the next block.
        flush: bool,
    }
}

const GATE: usize = 0;
const TRIGGER: usize = 1;
const NOTE: usize = 2;
const VELOCITY: usize = 3;
const CHANNEL: usize = 4;
const LENGTH: usize = 5;

const OUT: usize = 0;

/// Longest triggered note, beyond the input's soft range.
const MAX_LENGTH_SECONDS: f32 = 60.0;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl NoteToMidi {
    fn end(&mut self, tx: &mut Sender, at: u32) {
        if let Some((channel, note)) = self.sounding.take() {
            note_off(tx, at, channel, note, RELEASE_VELOCITY);
        }
        self.off_at = None;
    }

    /// Ends the sounding note, then starts `note` (a note number) at
    /// `velocity` (0–1) on `channel` (1–16).
    fn start(&mut self, tx: &mut Sender, at: u32, note: f32, velocity: f32, channel: i32) {
        self.end(tx, at);
        let note = finite_or(note, 60.0).round().clamp(0.0, 127.0) as u8;
        let velocity = finite_or(velocity, 0.8).clamp(0.0, 1.0);
        let velocity = ((velocity * 127.0).round() as u8).clamp(1, 127);
        let channel = (channel.clamp(1, 16) - 1) as u8;
        tx.send(
            at,
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            },
        );
        self.sounding = Some((channel, note));
    }
}

impl Node for NoteToMidi {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("gate", false).doc("A note plays while this is high."))
            .input(
                SocketSpec::trigger("trigger")
                    .doc("Plays a note `length` long (when `gate` isn't wired)."),
            )
            .input(
                SocketSpec::float("note", Unit::Generic, 60.0)
                    .range(0.0, 127.0)
                    .no_smooth()
                    .doc("MIDI note number, read when a note starts."),
            )
            .input(
                SocketSpec::float("velocity", Unit::Generic, 0.8)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("Velocity, 0–1, read when a note starts."),
            )
            .input(
                SocketSpec::int("channel", 1)
                    .range(1.0, 16.0)
                    .doc("MIDI channel, 1–16."),
            )
            .input(
                SocketSpec::float("length", Unit::Seconds, 0.1)
                    .range(0.01, 2.0)
                    .no_smooth()
                    .doc("How long a triggered note lasts."),
            )
            .output(SocketSpec::midi("midi").doc("The notes."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        NoteToMidi {
            gate: false,
            sounding: None,
            off_at: None,
            now: 0,
            sample_rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let gate = io.inputs.boolean(GATE);
        let triggers: &[u32] = if io.inputs.is_wired(GATE) {
            &[]
        } else {
            io.inputs.triggers(TRIGGER)
        };
        let note = io.inputs.float(NOTE);
        let velocity = io.inputs.float(VELOCITY);
        let channel = io.inputs.int(CHANNEL);
        let length = io.inputs.float(LENGTH);
        let last = frames.saturating_sub(1);
        let mut tx = Sender::new(io.outputs.midi(OUT));

        if std::mem::take(&mut self.flush) {
            self.end(&mut tx, 0);
        }
        let mut trig = triggers.iter().map(|&t| (t as usize).min(last)).peekable();
        for i in 0..frames {
            let at = i as u32;
            let g = gate.at(i);
            if self.gate && !g {
                self.end(&mut tx, at);
            }
            if self.off_at == Some(self.now + i as u64) {
                self.end(&mut tx, at);
            }
            if !self.gate && g {
                self.start(&mut tx, at, note.at(i), velocity.at(i), channel);
            }
            self.gate = g;
            let mut fired = false;
            while trig.next_if(|&t| t <= i).is_some() {
                fired = true;
            }
            if fired {
                self.start(&mut tx, at, note.at(i), velocity.at(i), channel);
                let secs = finite_or(length.at(i), 0.1).clamp(0.0, MAX_LENGTH_SECONDS);
                let samples = ((f64::from(secs) * self.sample_rate).round() as u64).max(1);
                self.off_at = Some(self.now + i as u64 + samples);
            }
        }
        self.now += frames as u64;
    }

    fn reset(&mut self) {
        self.flush = self.sounding.is_some();
        self.gate = false;
        self.off_at = None;
        self.now = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<NoteToMidi>(json!({}), 48_000.0, block, &NoResources)
    }

    fn on(channel: u8, note: u8, velocity: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity,
        }
    }
    fn off(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel,
            note,
            velocity: RELEASE_VELOCITY,
        }
    }

    #[test]
    fn a_gate_plays_a_note_while_high() {
        let mut h = harness(64);
        let mut gate = vec![0.0; 300];
        gate[50..200].fill(1.0);
        h.set_samples("gate", gate)
            .set("note", 64.0)
            .set("velocity", 1.0);
        let out = h.run(300);
        assert_eq!(out.midi("midi"), &[(50, on(0, 64, 127)), (200, off(0, 64))]);
    }

    #[test]
    fn the_note_off_matches_even_if_the_note_changes() {
        let mut h = harness(64);
        let mut gate = vec![0.0; 200];
        gate[10..100].fill(1.0);
        let notes: Vec<f32> = (0..200).map(|i| if i < 50 { 60.4 } else { 72.0 }).collect();
        h.set_samples("gate", gate)
            .set_samples("note", notes)
            .set("channel", 3);
        let out = h.run(200);
        assert_eq!(out.midi("midi"), &[(10, on(2, 60, 102)), (100, off(2, 60))]);
    }

    #[test]
    fn triggers_play_notes_of_a_set_length_one_at_a_time() {
        let mut h = harness(64);
        h.set("length", 0.001); // 48 samples
        h.triggers("trigger", &[10, 100, 120]);
        let out = h.run(300);
        assert_eq!(
            out.midi("midi"),
            &[
                (10, on(0, 60, 102)),
                (58, off(0, 60)),
                (100, on(0, 60, 102)),
                (120, off(0, 60)),
                (120, on(0, 60, 102)),
                (168, off(0, 60)),
            ]
        );
    }

    #[test]
    fn triggers_are_ignored_while_the_gate_is_wired() {
        let mut h = harness(64);
        h.set_samples("gate", vec![0.0; 128]);
        h.triggers("trigger", &[10]);
        assert!(h.run(128).midi("midi").is_empty());
    }

    #[test]
    fn notes_and_velocities_are_kept_in_range() {
        let mut h = harness(64);
        h.set("note", 300.0).set("velocity", 0.0);
        h.triggers("trigger", &[0]);
        assert_eq!(h.run(64).midi("midi")[0], (0, on(0, 127, 1)));
        let mut h = harness(64);
        h.set("note", f64::NAN)
            .set("velocity", 9.0)
            .set("channel", 99);
        h.triggers("trigger", &[0]);
        assert_eq!(h.run(64).midi("midi")[0], (0, on(15, 60, 127)));
    }

    #[test]
    fn reset_ends_the_sounding_note() {
        let mut h = harness(64);
        h.set("gate", true);
        assert_eq!(h.run(64).midi("midi"), &[(0, on(0, 60, 102))]);
        h.reset();
        // The note ends, and the gate, still on, starts a new one.
        assert_eq!(
            h.run(64).midi("midi"),
            &[(64, off(0, 60)), (64, on(0, 60, 102))]
        );
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let gate: Vec<f32> = (0..3_000)
            .map(|i| if (i / 170) % 2 == 1 { 1.0 } else { 0.0 })
            .collect();
        let notes: Vec<f32> = (0..3_000).map(|i| 40.0 + (i % 37) as f32).collect();
        let render = |block, gated: bool| {
            let mut h = harness(block);
            if gated {
                h.set_samples("gate", gate.clone());
            } else {
                h.triggers("trigger", &[0, 5, 333, 1_000, 1_001, 2_047, 2_048]);
                h.set("length", 0.004);
            }
            h.set_samples("note", notes.clone());
            h.run(3_000).midi("midi").to_vec()
        };
        for gated in [true, false] {
            let one = render(1, gated);
            assert_eq!(one, render(64, gated));
            assert_eq!(one, render(512, gated));
        }
    }
}
