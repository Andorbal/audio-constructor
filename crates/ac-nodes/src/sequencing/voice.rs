use ac_core::{
    Config, ConfigExt, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature,
    SocketSpec, Unit, node, note_to_hz,
};

node! {
    kind: "voice",
    name: "Voice",
    category: Sequencing,
    keywords: ["polyphony", "poly", "note", "pitch", "gate", "velocity", "voice index"],
    /// One voice's note, inside the body of a `Voices` node.
    ///
    /// Each copy of a Voices body gets its own notes, and its Voice node
    /// turns them into signals: `freq` (A4 = 440 Hz, shifted by pitch bend
    /// up to `bend_range` semitones), `gate` (high while the note is held or
    /// sustained), `velocity` (0–1), `note`, a `trigger` on each new note,
    /// and `index`, which voice this is (0, 1, 2, ...), handy for spreading
    /// voices across the stereo field or detuning them. `midi` passes the
    /// voice's own notes on, for nodes that want MIDI (a `MIDI Note` for
    /// glide, a `MIDI CC`).
    ///
    /// When a voice is stolen for a new note, or the same note is played
    /// again, `gate` drops for one sample before the new note so envelopes
    /// start again. After a release, `freq` and `note` hold their values so
    /// the tail stays in tune. Outside a Voices body the node does nothing
    /// and the Problems tab says so.
    ///
    /// **Try this:** wire `freq` into an Oscillator and `gate` into an ADSR
    /// that drives a Gain, then multiply `index` by 0.3 and wire it into a
    /// `Pan` for voices that fan out left to right.
    pub struct Voice {
        index: i32,
        note: u8,
        velocity: f32,
        on: bool,
        bend: f32,
        triggers: Vec<u32>,
        /// Frames where the gate must dip for a retrigger.
        dips: Vec<u32>,
    }
}

const VOICE_MIDI: usize = 0;
const BEND_RANGE: usize = 1;

const FREQ: usize = 0;
const GATE: usize = 1;
const VELOCITY: usize = 2;
const NOTE: usize = 3;
const TRIGGER: usize = 4;
const INDEX: usize = 5;
const MIDI_OUT: usize = 6;

const A4: u8 = 69;
const MAX_BEND_SEMITONES: f32 = 48.0;

fn bend_amount(value: i16) -> f32 {
    let v = f32::from(value);
    let scaled = if v >= 0.0 { v / 8191.0 } else { v / 8192.0 };
    scaled.clamp(-1.0, 1.0)
}

impl Node for Voice {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::midi("voice_midi")
                    .hidden()
                    .doc("This voice's notes, from the Voices node."),
            )
            .input(
                SocketSpec::float("bend_range", Unit::Semitones, 2.0)
                    .range(0.0, 24.0)
                    .doc("Pitch bend depth."),
            )
            .output(
                SocketSpec::float("freq", Unit::Hz, 440.0)
                    .rate(Rate::Sample)
                    .doc("Pitch of this voice's note."),
            )
            .output(SocketSpec::boolean("gate", false).doc("High while the note is held."))
            .output(
                SocketSpec::float("velocity", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("Velocity of the note, 0–1."),
            )
            .output(SocketSpec::int("note", i64::from(A4)).doc("The note number."))
            .output(SocketSpec::trigger("trigger").doc("Fires on each new note."))
            .output(SocketSpec::int("index", 0).doc("Which voice this is, from 0."))
            .output(SocketSpec::midi("midi").doc("This voice's MIDI, passed on."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        Voice {
            index: config.get_u64_or("voice_index", 0).min(1024) as i32,
            note: A4,
            velocity: 0.0,
            on: false,
            bend: 0.0,
            triggers: Vec::with_capacity(ctx.max_block.max(1)),
            dips: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(VOICE_MIDI);
        let bend_range = io.inputs.float(BEND_RANGE);
        self.triggers.clear();
        self.dips.clear();
        let last = ctx.frames.saturating_sub(1);
        {
            let mut passthrough = io.outputs.midi(MIDI_OUT);
            for e in events {
                passthrough.push(*e);
            }
        }
        let mut events = events.iter().peekable();
        let (mut freq, mut gate) = io.outputs.float2(FREQ, GATE);
        for i in 0..ctx.frames {
            let mut dip = false;
            while let Some(ev) = events.next_if(|e| (e.offset as usize).min(last) <= i) {
                match ev.msg {
                    MidiMsg::NoteOn { note, velocity, .. } if velocity > 0 => {
                        if self.on || self.dips.last() == Some(&(i as u32)) {
                            dip = true;
                        }
                        self.on = true;
                        self.note = note;
                        self.velocity = f32::from(velocity.min(127)) / 127.0;
                        if self.triggers.last() != Some(&(i as u32))
                            && self.triggers.len() < self.triggers.capacity()
                        {
                            self.triggers.push(i as u32);
                        }
                    }
                    MidiMsg::NoteOn { note, .. } | MidiMsg::NoteOff { note, .. } => {
                        if note == self.note && self.on {
                            self.on = false;
                            // A note-off and a new note-on at one frame: dip.
                            if self.dips.len() < self.dips.capacity() {
                                self.dips.push(i as u32);
                            }
                        }
                    }
                    MidiMsg::PitchBend { value, .. } => self.bend = bend_amount(value),
                    MidiMsg::ControlChange {
                        controller: 120 | 123,
                        ..
                    } => self.on = false,
                    _ => {}
                }
            }
            let range = bend_range.at(i);
            let range = if range.is_finite() {
                range.clamp(0.0, MAX_BEND_SEMITONES)
            } else {
                0.0
            };
            freq.set(i, note_to_hz(f32::from(self.note) + self.bend * range));
            gate.set(i, if self.on && !dip { 1.0 } else { 0.0 });
        }
        io.outputs.set_float(VELOCITY, self.velocity);
        io.outputs.set_int(NOTE, i32::from(self.note));
        io.outputs.set_int(INDEX, self.index);
        let mut trigger = io.outputs.trigger(TRIGGER);
        for &t in &self.triggers {
            trigger.push(t);
        }
    }

    fn reset(&mut self) {
        self.note = A4;
        self.velocity = 0.0;
        self.on = false;
        self.bend = 0.0;
        self.triggers.clear();
        self.dips.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 127,
        }
    }
    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

    #[test]
    fn notes_become_pitch_gate_velocity_and_triggers() {
        let mut h = Harness::new::<Voice>(json!({ "voice_index": 3 }));
        h.midi("voice_midi", &[(10, on(57)), (40, off(57))]);
        let out = h.run(64);
        assert!(out.float("gate")[..10].iter().all(|&g| g == 0.0));
        assert!(out.float("gate")[10..40].iter().all(|&g| g == 1.0));
        assert!(out.float("gate")[40..].iter().all(|&g| g == 0.0));
        assert!((out.float("freq")[20] - 220.0).abs() < 1e-3);
        // After the release, pitch holds.
        assert!((out.float("freq")[63] - 220.0).abs() < 1e-3);
        assert_eq!(out.triggers("trigger"), [10]);
        assert_eq!(out.int("index")[0], 3);
        assert_eq!(out.int("note")[0], 57);
        assert_eq!(out.float("velocity")[0], 1.0);
        assert_eq!(out.midi("midi").len(), 2);
    }

    #[test]
    fn a_stolen_voice_dips_its_gate_for_one_sample() {
        let mut h = Harness::new::<Voice>(json!({}));
        h.midi("voice_midi", &[(5, on(60)), (20, off(60)), (20, on(64))]);
        let out = h.run(32);
        let g = out.float("gate");
        assert_eq!(g[19], 1.0);
        assert_eq!(g[20], 0.0, "dip");
        assert_eq!(g[21], 1.0);
        assert_eq!(out.triggers("trigger"), [5, 20]);
        // A repeated note-on without an off also dips.
        let mut h = Harness::new::<Voice>(json!({}));
        h.midi("voice_midi", &[(5, on(60)), (9, on(62))]);
        let g = h.run(16).float("gate").to_vec();
        assert_eq!((g[8], g[9], g[10]), (1.0, 0.0, 1.0));
    }

    #[test]
    fn pitch_bend_shifts_by_the_range() {
        let mut h = Harness::new::<Voice>(json!({}));
        h.set("bend_range", 12.0).midi(
            "voice_midi",
            &[
                (0, on(57)),
                (
                    1,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: 8191,
                    },
                ),
            ],
        );
        let out = h.run(8);
        assert!((out.float("freq")[4] - 440.0).abs() < 1e-2);
    }

    #[test]
    fn unwired_it_idles() {
        let mut h = Harness::new::<Voice>(json!({}));
        let out = h.run(64);
        assert!(out.float("gate").iter().all(|&g| g == 0.0));
        assert!((out.float("freq")[0] - 440.0).abs() < 1e-3);
    }
}
