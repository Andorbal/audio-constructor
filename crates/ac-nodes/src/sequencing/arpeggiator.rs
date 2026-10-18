use ac_core::{
    Config, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::rng::Rng;

use super::notes::{RELEASE_VELOCITY, Sender, is_all_notes_off, note_off, note_off_of, note_on_of};

node! {
    kind: "arpeggiator",
    name: "Arpeggiator",
    category: Sequencing,
    keywords: ["arp", "arpeggio", "broken chord", "sequence", "latch", "pattern", "notes"],
    /// Plays the notes you hold one at a time, stepping on each clock tick.
    ///
    /// Hold a chord on `midi` and each `clock` trigger plays the next of
    /// its notes, by `mode`: `up` (low to high), `down`, `up_down` (up and
    /// back without repeating the ends), `random`, or `order` (the order
    /// the keys went down). `octaves` (1–4) repeats the notes that many
    /// octaves up before starting over. A chord pressed after every key
    /// was let go starts from the beginning. Up to 16 held keys count;
    /// beyond that the oldest is forgotten. Notes that `octaves` pushes
    /// past 127 are rests. Each note keeps the velocity and channel of its
    /// key.
    ///
    /// Each note lasts `gate` (0–1) of the time between the last two
    /// clocks, measured as they arrive (until two have arrived, a beat at
    /// the current tempo stands in), and at least one sample. A clock that
    /// comes while a note still sounds ends it first, on the same sample.
    /// At 1 the notes run into each other (legato): each ends at the next
    /// clock, or after two intervals if the clock stops. Letting go of
    /// every key ends the sounding note at once, unless `latch` is on: then
    /// the notes keep playing after you let go, until you press a new chord
    /// (which replaces them) or turn `latch` off.
    ///
    /// Nothing plays without a `clock`: wire a `Clock`. Messages other
    /// than notes (controllers, pitch bend, pressure, program changes) pass
    /// straight through. On the same sample, incoming notes apply first,
    /// then a note due to end, then the clock. All-notes-off clears the
    /// held keys and ends the sounding note, and a transport reset ends it
    /// at the start of the next block.
    ///
    /// **Try this:** `MIDI In` → Arpeggiator → a synth, with a `Clock`
    /// (`interval` 0.25) into `clock`, `mode` up_down, `octaves` 2, and
    /// `gate` 0.5. Hold a chord, turn on `latch`, and change chords on the
    /// beat while you shape the sound.
    pub struct Arpeggiator {
        /// Keys held down now.
        keys: Keys,
        /// The notes being played: the held keys, or with `latch` the last
        /// chord.
        playing: Keys,
        /// `latch` at the last event.
        latch: bool,
        /// Clocks since the notes started.
        step: u64,
        /// The sounding note: (channel, note).
        sounding: Option<(u8, u8)>,
        /// When the sounding note ends, on this node's own sample clock.
        off_at: Option<u64>,
        /// This node's sample clock at the start of the block.
        now: u64,
        last_clock: Option<u64>,
        /// Samples between the last two clocks.
        interval: Option<u64>,
        rng: Rng,
        seed: u64,
        /// A reset happened: end the sounding note at the next block.
        flush: bool,
    }
}

const MIDI: usize = 0;
const CLOCK: usize = 1;
const MODE: usize = 2;
const OCTAVES: usize = 3;
const GATE: usize = 4;
const LATCH: usize = 5;

const OUT: usize = 0;

const MODES: &[&str] = &["up", "down", "up_down", "random", "order"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Up,
    Down,
    UpDown,
    Random,
    Order,
}

impl Mode {
    fn from_index(i: usize) -> Mode {
        match i {
            1 => Mode::Down,
            2 => Mode::UpDown,
            3 => Mode::Random,
            4 => Mode::Order,
            _ => Mode::Up,
        }
    }
}

/// Most held keys remembered.
pub const MAX_KEYS: usize = 16;
const MAX_OCTAVES: i32 = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Key {
    channel: u8,
    note: u8,
    velocity: u8,
}

/// Keys in the order they went down, without allocation.
#[derive(Clone, Copy, Debug, Default)]
struct Keys {
    keys: [Key; MAX_KEYS],
    len: usize,
}

impl Keys {
    fn as_slice(&self) -> &[Key] {
        self.keys.get(..self.len).unwrap_or(&[])
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn press(&mut self, key: Key) {
        self.release(key.channel, key.note);
        if self.len >= MAX_KEYS {
            self.keys.copy_within(1.., 0);
            self.len = MAX_KEYS - 1;
        }
        if let Some(slot) = self.keys.get_mut(self.len) {
            *slot = key;
            self.len += 1;
        }
    }

    fn release(&mut self, channel: u8, note: u8) {
        if let Some(i) = self
            .as_slice()
            .iter()
            .position(|k| k.channel == channel && k.note == note)
        {
            self.keys.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// The keys from low to high.
    fn sorted(&self) -> Keys {
        let mut s = *self;
        if let Some(keys) = s.keys.get_mut(..s.len) {
            keys.sort_unstable_by_key(|k| (k.note, k.channel));
        }
        s
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Arpeggiator {
    fn end(&mut self, tx: &mut Sender, at: u32) {
        if let Some((channel, note)) = self.sounding.take() {
            note_off(tx, at, channel, note, RELEASE_VELOCITY);
        }
        self.off_at = None;
    }

    /// Turning `latch` off goes back to the keys actually held.
    fn follow_latch(&mut self, tx: &mut Sender, at: u32, latch: bool) {
        if self.latch && !latch {
            self.playing = self.keys;
            if self.playing.is_empty() {
                self.end(tx, at);
                self.step = 0;
            }
        }
        self.latch = latch;
    }

    fn handle(&mut self, tx: &mut Sender, at: u32, msg: MidiMsg) {
        if let Some((channel, note, velocity)) = note_on_of(&msg) {
            if self.latch && self.keys.is_empty() {
                // A new chord after letting go replaces the latched one.
                self.playing.clear();
            }
            if self.playing.is_empty() {
                self.step = 0;
            }
            let key = Key {
                channel,
                note,
                velocity,
            };
            self.keys.press(key);
            self.playing.press(key);
        } else if let Some((channel, note, _)) = note_off_of(&msg) {
            self.keys.release(channel, note);
            if !self.latch {
                self.playing.release(channel, note);
                if self.playing.is_empty() {
                    self.end(tx, at);
                    self.step = 0;
                }
            }
        } else {
            if is_all_notes_off(&msg) {
                self.keys.clear();
                self.playing.clear();
                self.end(tx, at);
                self.step = 0;
            }
            tx.send_if_room(at, msg, usize::from(self.sounding.is_some()));
        }
    }

    /// One clock at sample `now` (this node's clock), frame `at`.
    fn tick(&mut self, tx: &mut Sender, at: u32, now: u64, step: Step) {
        if let Some(last) = self.last_clock
            && now > last
        {
            self.interval = Some(now - last);
        }
        self.last_clock = Some(now);
        self.end(tx, at);

        let n = self.playing.len;
        if n == 0 {
            return;
        }
        let total = n * step.octaves;
        let s = (self.step % total as u64) as usize;
        let index = match step.mode {
            Mode::Up | Mode::Order => s,
            Mode::Down => total - 1 - s,
            Mode::UpDown if total < 2 => 0,
            Mode::UpDown => {
                let period = 2 * total - 2;
                let j = (self.step % period as u64) as usize;
                if j < total { j } else { period - j }
            }
            Mode::Random => self.rng.below(total as u64) as usize,
        };
        self.step += 1;
        let keys = if step.mode == Mode::Order {
            self.playing
        } else {
            self.playing.sorted()
        };
        let Some(key) = keys.as_slice().get(index % n).copied() else {
            return;
        };
        let note = usize::from(key.note) + 12 * (index / n);
        let Ok(note) = u8::try_from(note) else {
            return;
        };
        if note > 127 {
            return;
        }
        let msg = MidiMsg::NoteOn {
            channel: key.channel,
            note,
            velocity: key.velocity,
        };
        if !tx.send_if_room(at, msg, 1) {
            return;
        }
        self.sounding = Some((key.channel, note));
        let interval = self.interval.unwrap_or(step.fallback).max(1);
        let gate = f64::from(finite_or(step.gate, 0.5).clamp(0.0, 1.0));
        let length = if gate >= 1.0 {
            // Legato: the next clock ends it; this only covers a stopped clock.
            2 * interval
        } else {
            ((gate * interval as f64).round() as u64).max(1)
        };
        self.off_at = Some(now + length);
    }
}

/// What a clock needs to know besides the held notes.
#[derive(Clone, Copy, Debug)]
struct Step {
    mode: Mode,
    octaves: usize,
    gate: f32,
    /// Interval to assume before two clocks have arrived.
    fallback: u64,
}

impl Node for Arpeggiator {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("The notes to play."))
            .input(SocketSpec::trigger("clock").doc("Plays the next note."))
            .input(SocketSpec::choice("mode", MODES, "up").doc("The order notes play in."))
            .input(
                SocketSpec::int("octaves", 1)
                    .range(1.0, 4.0)
                    .doc("Octaves the notes repeat over."),
            )
            .input(
                SocketSpec::float("gate", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Note length as a fraction of the time between clocks."),
            )
            .input(
                SocketSpec::boolean("latch", false)
                    .doc("Keep playing after the keys are let go, until a new chord."),
            )
            .output(SocketSpec::midi("midi").doc("One note at a time, and everything else."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Arpeggiator {
            keys: Keys::default(),
            playing: Keys::default(),
            latch: false,
            step: 0,
            sounding: None,
            off_at: None,
            now: 0,
            last_clock: None,
            interval: None,
            rng: Rng::new(ctx.seed),
            seed: ctx.seed,
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let events = io.inputs.midi(MIDI);
        let clocks = io.inputs.triggers(CLOCK);
        let mode = Mode::from_index(io.inputs.choice(MODE, MODES.len()));
        let octaves = io.inputs.int(OCTAVES).clamp(1, MAX_OCTAVES) as usize;
        let gate = io.inputs.float(GATE);
        let latch = io.inputs.boolean(LATCH);
        let t = ctx.transport;
        let beat = t.tempo.samples_per_beat_at(t.position);
        let fallback = if beat.is_finite() {
            beat.round().max(1.0) as u64
        } else {
            1
        };
        let last = frames.saturating_sub(1);
        let end = self.now + frames as u64;
        let mut tx = Sender::new(io.outputs.midi(OUT));
        if std::mem::take(&mut self.flush) {
            self.end(&mut tx, 0);
        }

        let (mut e, mut c) = (0, 0);
        loop {
            let next_event = events.get(e).map(|ev| (ev.offset as usize).min(last));
            let next_clock = clocks.get(c).map(|&k| (k as usize).min(last));
            let next_off = self
                .off_at
                .filter(|&s| s < end)
                .map(|s| s.saturating_sub(self.now) as usize);
            let Some(i) = [next_event, next_clock, next_off]
                .into_iter()
                .flatten()
                .min()
            else {
                break;
            };
            let at = i as u32;
            self.follow_latch(&mut tx, at, latch.at(i));
            while let Some(ev) = events.get(e) {
                if (ev.offset as usize).min(last) > i {
                    break;
                }
                e += 1;
                self.handle(&mut tx, at, ev.msg);
            }
            if self.off_at.is_some_and(|s| s <= self.now + i as u64) {
                self.end(&mut tx, at);
            }
            let mut tick = false;
            while clocks.get(c).is_some_and(|&k| (k as usize).min(last) <= i) {
                c += 1;
                tick = true;
            }
            if tick {
                let step = Step {
                    mode,
                    octaves,
                    gate: gate.at(i),
                    fallback,
                };
                self.tick(&mut tx, at, self.now + i as u64, step);
            }
        }
        self.now = end;
    }

    fn reset(&mut self) {
        self.flush = self.sounding.is_some();
        self.keys.clear();
        self.playing.clear();
        self.latch = false;
        self.step = 0;
        self.off_at = None;
        self.now = 0;
        self.last_clock = None;
        self.interval = None;
        self.rng = Rng::new(self.seed);
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
        Harness::with_resources::<Arpeggiator>(json!({}), 48_000.0, block, &NoResources)
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

    /// The note-ons only.
    fn played(out: &[(u64, MidiMsg)]) -> Vec<i32> {
        notes(out)
            .into_iter()
            .filter(|&(_, n)| n > 0)
            .map(|(_, n)| n)
            .collect()
    }

    fn clocks(n: u64, every: u64) -> Vec<u64> {
        (0..n).map(|k| k * every).collect()
    }

    fn chord() -> Vec<(u64, MidiMsg)> {
        vec![(0, on(64)), (0, on(60)), (0, on(67))]
    }

    #[test]
    fn plays_up_with_gate_lengths_from_the_measured_interval() {
        let mut h = harness(64);
        h.midi("midi", &chord()).triggers("clock", &clocks(4, 100));
        let out = h.run(400);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 60),
                (100, -60),
                (100, 64),
                (150, -64),
                (200, 67),
                (250, -67),
                (300, 60),
                (350, -60),
            ]
        );
    }

    #[test]
    fn modes_and_octaves() {
        let run = |mode: &str, octaves: i64, n: u64| {
            let mut h = harness(64);
            h.set("mode", mode).set("octaves", octaves);
            h.midi("midi", &chord()).triggers("clock", &clocks(n, 50));
            played(h.run((n * 50) as usize).midi("midi"))
        };
        assert_eq!(run("down", 1, 4), vec![67, 64, 60, 67]);
        assert_eq!(run("up_down", 1, 6), vec![60, 64, 67, 64, 60, 64]);
        assert_eq!(run("order", 1, 4), vec![64, 60, 67, 64]);
        assert_eq!(run("up", 2, 7), vec![60, 64, 67, 72, 76, 79, 60]);
        assert_eq!(
            run("up_down", 2, 11),
            vec![60, 64, 67, 72, 76, 79, 76, 72, 67, 64, 60]
        );
        let random = run("random", 2, 40);
        assert!(random.iter().all(|n| [60, 64, 67, 72, 76, 79].contains(n)));
        assert_eq!(random, run("random", 2, 40), "seeded");
        assert!(random.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn letting_go_ends_the_note_and_a_new_chord_starts_over() {
        let mut h = harness(64);
        let mut ev = chord();
        ev.extend([
            (120, off(60)),
            (130, off(64)),
            (140, off(67)),
            (250, on(50)),
        ]);
        h.set("gate", 1.0)
            .midi("midi", &ev)
            .triggers("clock", &clocks(4, 100));
        let out = h.run(400);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(0, 60), (100, -60), (100, 64), (140, -64), (300, 50)]
        );
    }

    #[test]
    fn latch_keeps_playing_until_a_new_chord_or_latch_off() {
        let mut h = harness(64);
        let mut ev = chord();
        ev.extend([
            (10, off(60)),
            (10, off(64)),
            (10, off(67)),
            (250, on(48)),
            (260, off(48)),
        ]);
        h.set("latch", true)
            .midi("midi", &ev)
            .triggers("clock", &clocks(5, 100));
        let out = h.run(500);
        // C E G keep playing after release; the new key replaces them.
        assert_eq!(played(out.midi("midi")), vec![60, 64, 67, 48, 48]);

        let mut h = harness(64);
        let latch: Vec<f32> = (0..400).map(|i| if i < 150 { 1.0 } else { 0.0 }).collect();
        let mut ev = chord();
        ev.extend([
            (10, off(60)),
            (10, off(64)),
            (10, off(67)),
            (150, on(1)),
            (151, off(1)),
        ]);
        h.set_samples("latch", latch)
            .midi("midi", &ev)
            .triggers("clock", &clocks(4, 100));
        // Turning latch off with no keys down ends the note at once.
        let n = notes(h.run(400).midi("midi"));
        assert_eq!(&n[..4], &[(0, 60), (100, -60), (100, 64), (150, -64)]);
    }

    #[test]
    fn legato_gate_ends_each_note_on_the_next_clock() {
        let mut h = harness(64);
        h.set("gate", 1.0)
            .midi("midi", &chord())
            .triggers("clock", &clocks(3, 100));
        let out = h.run(300);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(0, 60), (100, -60), (100, 64), (200, -64), (200, 67)]
        );
        // The clock stopped: the last note ends two intervals later.
        let rest = notes(h.run(200).midi("midi"));
        assert_eq!(rest, vec![(400, -67)]);
    }

    #[test]
    fn first_note_uses_a_beat_until_an_interval_is_measured() {
        let mut h = harness(64);
        h.set("gate", 0.25)
            .midi("midi", &chord())
            .triggers("clock", &[0]);
        // 120 BPM: a beat is 24000 samples, a quarter of it 6000.
        assert_eq!(
            notes(h.run(7_000).midi("midi")),
            vec![(0, 60), (6_000, -60)]
        );
    }

    #[test]
    fn nothing_plays_without_a_clock_and_other_messages_pass() {
        let mut h = harness(64);
        let bend = MidiMsg::PitchBend {
            channel: 0,
            value: 5,
        };
        let mut ev = chord();
        ev.push((7, bend));
        h.midi("midi", &ev);
        assert_eq!(h.run(256).midi("midi"), &[(7, bend)]);
    }

    #[test]
    fn notes_past_127_rest_and_keys_keep_their_channel() {
        let mut h = harness(64);
        h.set("octaves", 2);
        h.midi(
            "midi",
            &[(
                0,
                MidiMsg::NoteOn {
                    channel: 4,
                    note: 120,
                    velocity: 33,
                },
            )],
        )
        .triggers("clock", &clocks(3, 50));
        let out = h.run(150);
        let m = out.midi("midi");
        assert_eq!(
            m[0],
            (
                0,
                MidiMsg::NoteOn {
                    channel: 4,
                    note: 120,
                    velocity: 33
                }
            )
        );
        assert_eq!(played(m), vec![120, 120]);
    }

    #[test]
    fn all_notes_off_and_reset_end_the_sounding_note() {
        let mut h = harness(64);
        let panic = MidiMsg::ControlChange {
            channel: 0,
            controller: 123,
            value: 0,
        };
        let mut ev = chord();
        ev.push((30, panic));
        h.set("latch", true)
            .midi("midi", &ev)
            .triggers("clock", &clocks(3, 50));
        let out = h.run(150);
        assert_eq!(notes(out.midi("midi")), vec![(0, 60), (30, -60)]);
        assert_eq!(out.midi("midi")[2], (30, panic));

        let mut h = harness(64);
        h.set("gate", 1.0)
            .midi("midi", &chord())
            .triggers("clock", &[0]);
        h.run(64);
        h.reset();
        assert_eq!(notes(h.run(64).midi("midi")), vec![(64, -60)]);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let mut ev: Vec<(u64, MidiMsg)> = Vec::new();
        for k in 0..12u64 {
            ev.push((k * 400, on(48 + (k * 5 % 24) as u8)));
            ev.push((k * 400 + 350, off(48 + (k * 5 % 24) as u8)));
            ev.push((k * 400 + 100, on(60 + (k % 7) as u8)));
            ev.push((k * 400 + 390, off(60 + (k % 7) as u8)));
        }
        let ticks: Vec<u64> = (0..120).map(|k| k * 41 + (k % 5)).collect();
        let render = |block, mode: &str| {
            let mut h = harness(block);
            h.set("mode", mode).set("octaves", 2).set("gate", 0.7);
            h.midi("midi", &ev).triggers("clock", &ticks);
            h.run(5_000).midi("midi").to_vec()
        };
        for mode in MODES {
            let one = render(1, mode);
            assert!(!one.is_empty());
            assert_eq!(one, render(64, mode), "{mode}");
            assert_eq!(one, render(512, mode), "{mode}");
        }
    }
}
