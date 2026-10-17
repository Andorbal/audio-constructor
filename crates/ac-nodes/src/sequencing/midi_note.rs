use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit,
    node, note_to_hz,
};

use crate::io::midi_in::channel_matches;

node! {
    kind: "midi_note",
    name: "MIDI Note",
    category: Sequencing,
    keywords: ["note", "pitch", "monophonic", "mono", "keyboard", "portamento", "glide", "cv"],
    /// Turns MIDI notes into a pitch, a gate, and a velocity for one voice.
    ///
    /// `freq` follows the sounding note (A4 = 440 Hz), `gate` is high while
    /// any key is held, and `trigger` fires when a key press becomes the
    /// sounding note. `velocity` (0–1) and `note` belong to the sounding
    /// note. When several keys are held, `priority` picks which one sounds:
    /// the last pressed, the lowest, or the highest. Releasing the sounding
    /// key falls back to another held key by the same rule, keeping the gate
    /// high and without a new trigger, so envelopes carry on (legato).
    /// After the last key is released, `freq`, `note`, and `velocity` keep
    /// their values so a release tail stays at the right pitch. Before the
    /// first note, `freq` is 440 Hz and `note` is 69.
    ///
    /// `glide` slides from one note to the next: the pitch moves in a
    /// straight line in semitones and arrives after `glide` seconds,
    /// whatever the interval. The first note starts at its own pitch. Pitch
    /// bend shifts `freq` by up to `bend_range` semitones. `channel` 0
    /// listens to every MIDI channel, 1–16 to one. All-notes-off and
    /// all-sound-off messages release every key. Up to 16 held keys are
    /// remembered; beyond that the oldest is forgotten.
    ///
    /// **Try this:** `MIDI In` → MIDI Note, then `freq` into an `Oscillator`
    /// and `gate` into an `ADSR` driving a `Gain` for a complete monosynth;
    /// set `priority` to low and add a little `glide` for a sliding bass.
    pub struct MidiNote {
        held: NoteStack,
        /// The sounding note; `None` when no key is held.
        active: Option<Held>,
        /// The last sounding note, held after release.
        note: u8,
        velocity: f32,
        /// Current pitch as a fractional MIDI note number.
        pitch: f64,
        target: f64,
        step: f64,
        glide_left: u64,
        /// Pitch bend, -1..1.
        bend: f32,
        sample_rate: f64,
        /// A note has sounded since the last reset (the first one doesn't glide).
        started: bool,
        /// Trigger offsets for this block, preallocated to `max_block`.
        triggers: Vec<u32>,
    }
}

const MIDI: usize = 0;
const PRIORITY: usize = 1;
const GLIDE: usize = 2;
const BEND_RANGE: usize = 3;
const CHANNEL: usize = 4;

const FREQ: usize = 0;
const GATE: usize = 1;
const VELOCITY: usize = 2;
const NOTE: usize = 3;
const TRIGGER: usize = 4;

const PRIORITIES: &[&str] = &["last", "low", "high"];

/// Hard limits beyond the inputs' soft ranges, so pitch stays finite.
const MAX_GLIDE_SECONDS: f32 = 60.0;
const MAX_BEND_SEMITONES: f32 = 48.0;

const A4: u8 = 69;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Priority {
    Last,
    Low,
    High,
}

impl Priority {
    fn from_index(i: usize) -> Priority {
        match i {
            1 => Priority::Low,
            2 => Priority::High,
            _ => Priority::Last,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    note: u8,
    velocity: u8,
}

const CAPACITY: usize = 16;

/// Held keys in the order they were pressed, without allocation.
#[derive(Clone, Copy, Debug, Default)]
struct NoteStack {
    notes: [Held; CAPACITY],
    len: usize,
}

impl NoteStack {
    fn held(&self) -> &[Held] {
        self.notes.get(..self.len).unwrap_or(&[])
    }

    fn press(&mut self, note: u8, velocity: u8) {
        self.release(note);
        if self.len >= CAPACITY {
            self.notes.copy_within(1.., 0);
            self.len = CAPACITY - 1;
        }
        if let Some(slot) = self.notes.get_mut(self.len) {
            *slot = Held { note, velocity };
            self.len += 1;
        }
    }

    /// Forgets `note`; returns whether it was held.
    fn release(&mut self, note: u8) -> bool {
        match self.held().iter().position(|h| h.note == note) {
            Some(i) => {
                self.notes.copy_within(i + 1..self.len, i);
                self.len -= 1;
                true
            }
            None => false,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn pick(&self, priority: Priority) -> Option<Held> {
        let held = self.held().iter().copied();
        match priority {
            Priority::Last => self.held().last().copied(),
            Priority::Low => held.min_by_key(|h| h.note),
            Priority::High => held.max_by_key(|h| h.note),
        }
    }
}

/// Pitch bend as -1..1, reaching both ends exactly.
fn bend_amount(value: i16) -> f32 {
    let v = f32::from(value);
    let scaled = if v >= 0.0 { v / 8191.0 } else { v / 8192.0 };
    scaled.clamp(-1.0, 1.0)
}

fn limited(v: f32, max: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, max)
    } else {
        0.0
    }
}

impl MidiNote {
    /// Makes `h` the sounding note and starts the glide toward it.
    ///
    /// The glide is linear in semitones over a fixed time (constant-time
    /// portamento, as on most digital monosynths): every interval takes the
    /// same time and the pitch arrives exactly. The exponential lag of an
    /// analog portamento circuit never quite arrives, which makes "glide
    /// time" harder to set to a tempo.
    fn activate(&mut self, h: Held, glide_seconds: f32) {
        self.active = Some(h);
        self.note = h.note;
        self.velocity = f32::from(h.velocity.min(127)) / 127.0;
        self.target = f64::from(h.note);
        let samples = if self.started {
            (f64::from(limited(glide_seconds, MAX_GLIDE_SECONDS)) * self.sample_rate).round() as u64
        } else {
            0
        };
        self.started = true;
        if samples == 0 {
            self.pitch = self.target;
            self.glide_left = 0;
        } else {
            self.step = (self.target - self.pitch) / samples as f64;
            self.glide_left = samples;
        }
    }

    /// Re-picks the sounding note after keys change, without a trigger.
    fn follow(&mut self, priority: Priority, glide_seconds: f32) {
        match self.held.pick(priority) {
            None => self.active = None,
            Some(h) if self.active.map(|a| a.note) != Some(h.note) => {
                self.activate(h, glide_seconds);
            }
            Some(_) => {}
        }
    }

    fn handle(&mut self, msg: MidiMsg, offset: u32, priority: Priority, glide_seconds: f32) {
        match msg {
            MidiMsg::NoteOn { note, velocity, .. } if velocity > 0 => {
                self.held.press(note, velocity);
                match self.held.pick(priority) {
                    Some(h) if h.note == note => {
                        self.activate(h, glide_seconds);
                        self.push_trigger(offset);
                    }
                    _ => self.follow(priority, glide_seconds),
                }
            }
            MidiMsg::NoteOn { note, .. } | MidiMsg::NoteOff { note, .. } => {
                if self.held.release(note) {
                    self.follow(priority, glide_seconds);
                }
            }
            MidiMsg::PitchBend { value, .. } => self.bend = bend_amount(value),
            // All sound off, all notes off.
            MidiMsg::ControlChange {
                controller: 120 | 123,
                ..
            } => {
                self.held.clear();
                self.follow(priority, glide_seconds);
            }
            _ => {}
        }
    }

    fn push_trigger(&mut self, offset: u32) {
        if self.triggers.last() != Some(&offset) && self.triggers.len() < self.triggers.capacity() {
            self.triggers.push(offset);
        }
    }

    fn advance_glide(&mut self) {
        if self.glide_left > 0 {
            self.glide_left -= 1;
            self.pitch = if self.glide_left == 0 {
                self.target
            } else {
                self.pitch + self.step
            };
        }
    }
}

impl Node for MidiNote {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Notes to follow."))
            .input(
                SocketSpec::choice("priority", PRIORITIES, "last")
                    .doc("Which held key sounds: last pressed, lowest, or highest."),
            )
            .input(
                SocketSpec::float("glide", Unit::Seconds, 0.0)
                    .range(0.0, 2.0)
                    .doc("Time to slide from one note to the next."),
            )
            .input(
                SocketSpec::float("bend_range", Unit::Semitones, 2.0)
                    .range(0.0, 24.0)
                    .doc("Pitch bend depth."),
            )
            .input(
                SocketSpec::int("channel", 0)
                    .range(0.0, 16.0)
                    .doc("MIDI channel, 1–16; 0 listens to all."),
            )
            .output(
                SocketSpec::float("freq", Unit::Hz, 440.0)
                    .rate(Rate::Sample)
                    .doc("Pitch of the sounding note."),
            )
            .output(SocketSpec::boolean("gate", false).doc("High while a key is held."))
            .output(
                SocketSpec::float("velocity", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("Velocity of the sounding note, 0–1."),
            )
            .output(SocketSpec::int("note", i64::from(A4)).doc("The sounding note number."))
            .output(SocketSpec::trigger("trigger").doc("Fires on each new note."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        MidiNote {
            held: NoteStack::default(),
            active: None,
            note: A4,
            velocity: 0.0,
            pitch: f64::from(A4),
            target: f64::from(A4),
            step: 0.0,
            glide_left: 0,
            bend: 0.0,
            sample_rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            started: false,
            triggers: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        let priority = Priority::from_index(io.inputs.choice(PRIORITY, PRIORITIES.len()));
        let glide = io.inputs.float(GLIDE);
        let bend_range = io.inputs.float(BEND_RANGE);
        let filter = io.inputs.int(CHANNEL);
        self.triggers.clear();

        // A priority change while keys are held re-picks the sounding note.
        self.follow(priority, glide.first());

        let last = ctx.frames.saturating_sub(1);
        let mut events = events
            .iter()
            .filter(|e| channel_matches(filter, e))
            .peekable();
        let (mut freq, mut gate) = io.outputs.float2(FREQ, GATE);
        for i in 0..ctx.frames {
            // Late events are applied on the last frame rather than dropped.
            while let Some(ev) = events.next_if(|e| (e.offset as usize).min(last) <= i) {
                self.handle(ev.msg, i as u32, priority, glide.at(i));
            }
            let bend = self.bend * limited(bend_range.at(i), MAX_BEND_SEMITONES);
            freq.set(i, note_to_hz((self.pitch + f64::from(bend)) as f32));
            gate.set(i, if self.active.is_some() { 1.0 } else { 0.0 });
            self.advance_glide();
        }

        io.outputs.set_float(VELOCITY, self.velocity);
        io.outputs.set_int(NOTE, i32::from(self.note));
        let mut trigger = io.outputs.trigger(TRIGGER);
        for &t in &self.triggers {
            trigger.push(t);
        }
    }

    fn reset(&mut self) {
        self.held.clear();
        self.active = None;
        self.note = A4;
        self.velocity = 0.0;
        self.pitch = f64::from(A4);
        self.target = self.pitch;
        self.step = 0.0;
        self.glide_left = 0;
        self.bend = 0.0;
        self.started = false;
        self.triggers.clear();
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

    fn hz(note: f32) -> f32 {
        note_to_hz(note)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= b.abs() * 1e-5
    }

    #[test]
    fn before_any_note_it_rests_on_a4() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        let out = h.run(128);
        assert!(out.float("freq").iter().all(|&f| f == 440.0));
        assert!(out.float("gate").iter().all(|&g| g == 0.0));
        assert!(out.int("note").iter().all(|&n| n == 69));
        assert!(out.float("velocity").iter().all(|&v| v == 0.0));
        assert!(out.triggers("trigger").is_empty());
    }

    #[test]
    fn note_on_and_off_are_sample_accurate() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi("midi", &[(100, on(60)), (300, off(60))]);
        let out = h.run(512);
        let (freq, gate) = (out.float("freq"), out.float("gate"));
        assert_eq!(gate[99], 0.0);
        assert_eq!(gate[100], 1.0);
        assert_eq!(gate[299], 1.0);
        assert_eq!(gate[300], 0.0);
        assert_eq!(freq[99], 440.0);
        assert!(close(freq[100], hz(60.0)));
        // The pitch holds after release.
        assert!(close(freq[511], hz(60.0)));
        assert_eq!(out.triggers("trigger"), &[100]);
        assert_eq!(out.int("note")[511], 60);
        assert_eq!(out.float("velocity")[511], 1.0);
    }

    #[test]
    fn velocity_is_scaled_to_one() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi(
            "midi",
            &[(
                0,
                MidiMsg::NoteOn {
                    channel: 0,
                    note: 50,
                    velocity: 64,
                },
            )],
        );
        let out = h.run(64);
        assert!(close(out.float("velocity")[0], 64.0 / 127.0));
    }

    #[test]
    fn last_priority_falls_back_without_a_trigger() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi("midi", &[(0, on(60)), (10, on(64)), (20, off(64))]);
        let out = h.run(64);
        let freq = out.float("freq");
        assert!(close(freq[5], hz(60.0)));
        assert!(close(freq[15], hz(64.0)));
        assert!(close(freq[25], hz(60.0)));
        assert!(out.float("gate").iter().all(|&g| g == 1.0));
        assert_eq!(out.triggers("trigger"), &[0, 10]);
    }

    #[test]
    fn low_priority() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("priority", "low");
        h.midi(
            "midi",
            &[(0, on(60)), (10, on(64)), (20, on(55)), (30, off(55))],
        );
        let out = h.run(64);
        let freq = out.float("freq");
        assert!(close(freq[15], hz(60.0)));
        assert!(close(freq[25], hz(55.0)));
        assert!(close(freq[35], hz(60.0)));
        assert_eq!(out.triggers("trigger"), &[0, 20]);
    }

    #[test]
    fn high_priority() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("priority", "high");
        h.midi(
            "midi",
            &[(0, on(60)), (10, on(64)), (20, on(55)), (30, off(64))],
        );
        let out = h.run(64);
        let freq = out.float("freq");
        assert!(close(freq[15], hz(64.0)));
        assert!(close(freq[25], hz(64.0)));
        assert!(close(freq[35], hz(60.0)));
        assert_eq!(out.triggers("trigger"), &[0, 10]);
        assert_eq!(out.int("note")[63], 60);
    }

    #[test]
    fn gate_falls_when_the_last_key_is_released() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi(
            "midi",
            &[(0, on(60)), (10, on(64)), (20, off(60)), (30, off(64))],
        );
        let out = h.run(64);
        let gate = out.float("gate");
        assert!(gate[..30].iter().all(|&g| g == 1.0));
        assert!(gate[30..].iter().all(|&g| g == 0.0));
        assert!(close(out.float("freq")[40], hz(64.0)));
    }

    #[test]
    fn glide_reaches_the_target_on_time() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("glide", 0.01); // 480 samples at 48 kHz
        h.midi("midi", &[(0, on(60)), (1_000, on(72))]);
        let out = h.run(2_048);
        let freq = out.float("freq");
        // The first note doesn't glide in from A4.
        assert!(close(freq[0], hz(60.0)));
        assert!(close(freq[1_000], hz(60.0)));
        assert!(close(freq[1_240], hz(66.0)));
        assert!(freq[1_000..1_480].windows(2).all(|w| w[1] > w[0]));
        assert!(close(freq[1_480], hz(72.0)));
        assert!(close(freq[2_047], hz(72.0)));
    }

    #[test]
    fn pitch_bend_shifts_by_the_range() {
        let bend = |value| MidiMsg::PitchBend { channel: 0, value };
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi("midi", &[(0, on(69)), (10, bend(8191)), (20, bend(-8192))]);
        let out = h.run(64);
        let freq = out.float("freq");
        assert!(close(freq[5], 440.0));
        assert!(close(freq[15], 440.0 * 2f32.powf(2.0 / 12.0)));
        assert!(close(freq[25], 440.0 * 2f32.powf(-2.0 / 12.0)));

        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("bend_range", 12.0);
        h.midi("midi", &[(0, on(69)), (0, bend(8191))]);
        assert!(close(h.run(64).float("freq")[10], 880.0));
    }

    #[test]
    fn channel_filter() {
        let on_ch = |channel, note| MidiMsg::NoteOn {
            channel,
            note,
            velocity: 100,
        };
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("channel", 2);
        h.midi(
            "midi",
            &[(0, on_ch(0, 60)), (10, on_ch(1, 62)), (20, on_ch(2, 64))],
        );
        let out = h.run(64);
        assert_eq!(out.triggers("trigger"), &[10]);
        assert_eq!(out.int("note")[0], 62);
    }

    #[test]
    fn zero_velocity_note_on_and_panic_messages_release() {
        let zero = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 0,
        };
        let all_off = MidiMsg::ControlChange {
            channel: 0,
            controller: 123,
            value: 0,
        };
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi(
            "midi",
            &[
                (0, on(60)),
                (10, zero),
                (20, on(61)),
                (21, on(62)),
                (30, all_off),
            ],
        );
        let gate = h.run(64).float("gate").to_vec();
        assert_eq!(gate[5], 1.0);
        assert_eq!(gate[15], 0.0);
        assert_eq!(gate[25], 1.0);
        assert!(gate[30..].iter().all(|&g| g == 0.0));
    }

    #[test]
    fn many_held_keys_drop_the_oldest() {
        let mut s = NoteStack::default();
        for n in 0..20 {
            s.press(40 + n, 100);
        }
        assert_eq!(s.held().len(), CAPACITY);
        assert_eq!(s.held()[0].note, 44);
        assert!(!s.release(40));
        assert!(s.release(59));
        assert_eq!(s.pick(Priority::Last).map(|h| h.note), Some(58));
        // Pressing a held key again moves it to the top.
        s.press(44, 1);
        assert_eq!(
            s.pick(Priority::Last),
            Some(Held {
                note: 44,
                velocity: 1
            })
        );
        assert_eq!(s.held().len(), CAPACITY - 1);
    }

    #[test]
    fn reset_returns_to_rest() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.reset();
        let out = h.run(64);
        assert!(out.float("gate").iter().all(|&g| g == 0.0));
        assert!(out.float("freq").iter().all(|&f| f == 440.0));
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let mut h = Harness::new::<MidiNote>(json!({}));
        h.set("bend_range", 1.0e9).set("glide", f64::NAN);
        h.midi(
            "midi",
            &[
                (0, on(127)),
                (
                    1,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: i16::MAX,
                    },
                ),
            ],
        );
        assert!(h.run(64).float("freq").iter().all(|f| f.is_finite()));
    }
}
