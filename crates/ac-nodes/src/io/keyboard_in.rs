use ac_core::{
    Config, ConfigExt, Control, Determinism, Io, Key, MidiMsg, Node, NodeMeta, PrepareCtx,
    ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::{fill_range, gate, stamped};
use crate::controls::keyboard::midi_velocity;
use crate::sequencing::notes::{RELEASE_VELOCITY, Sender};

node! {
    kind: "keyboard_in",
    name: "Keyboard In",
    category: Io,
    keywords: ["computer keyboard", "keys", "typing", "qwerty", "hotkey", "notes", "midi", "play", "input"],
    /// Brings the computer keyboard into the patch: chosen keys as gates
    /// and press triggers, and a row of letter keys played like a piano.
    ///
    /// The `keys` config lists the keys to bring in by name, separated by
    /// commas (default `1, 2, 3, 4`). Each gets a `key_<name>` output that
    /// is high while it's held and a `key_<name>_press` trigger, so `Space`
    /// becomes `key_space` and `key_space_press`. Keys go by their names:
    /// letters, digits, `Space`, `Enter`, `Tab`, `Backspace`, `Escape`,
    /// `Up`, `Down`, `Left`, `Right`, `Insert`, `Delete`, `Home`, `End`,
    /// `PageUp`, `PageDown`, `F1`–`F12`, and `Minus`, `Equals`,
    /// `LeftBracket`, `RightBracket`, `Backslash`, `Semicolon`, `Quote`,
    /// `Backtick`, `Comma`, `Period`, `Slash`. Unknown names are skipped,
    /// and a node takes up to 32 keys.
    ///
    /// The `midi` output plays the middle row of letters like a piano:
    /// `A S D F G H J K L ; '` are white keys from C up, and
    /// `W E T Y U O P` the black keys between them. `Z` and `X` shift the
    /// row down and up an octave, up to three each way. `A` plays MIDI note
    /// `low` (config, 0–127, default 60, middle C), on channel `channel`
    /// (1–16, default 1), with the `velocity` of the moment it's pressed. A
    /// key keeps the note it started, so shifting octaves while holding a
    /// key still releases the right note. Two keys that land on one note
    /// sound it once, until both are up, and notes outside MIDI's range
    /// don't sound. A key listed in `keys` that is also on the piano row
    /// (or is `Z` or `X`) does both jobs.
    ///
    /// Every press and release lands on the sample it arrived on, and a
    /// key that repeats while held doesn't strike again. Restarting the
    /// transport releases every note; keys still held sound again from
    /// there, and the octave shift stays where it was.
    ///
    /// Keyboard In is a live source: nodes that need their whole input
    /// before they can play are disabled downstream of it.
    ///
    /// **Try this:** wire `midi` into a `Voices` synth and play it from
    /// the laptop keyboard, or wire `key_1_press` into a `Sampler`'s
    /// `start` to fire a sample with the 1 key.
    pub struct KeyboardIn {
        /// The keys with outputs of their own, in socket order.
        keys: Vec<Key>,
        /// Each of those keys' state at the end of the last block.
        key_down: [bool; MAX_KEYS],
        /// The piano row's keys, as in `PIANO_ROW`.
        row: [Option<Key>; ROW],
        /// Each piano key's state.
        row_down: [bool; ROW],
        /// The note each piano key started, if it sounds.
        sounding: [Option<u8>; ROW],
        /// How many piano keys hold each note.
        holders: [u8; 128],
        /// Notes with holders: each owes a note-off.
        notes_on: usize,
        /// The octave keys, down then up.
        shift: [Option<Key>; 2],
        shift_down: [bool; 2],
        octave: i32,
        low: u8,
        channel: u8,
        /// Release every note at the start of the next block (after reset).
        flush: bool,
    }
}

const VELOCITY: usize = 0;

const MIDI: usize = 0;
/// Output index of the first key's gate; key `k`'s press is one after it.
const FIRST_KEY: usize = 1;

const MAX_KEYS: usize = 32;
const DEFAULT_KEYS: &str = "1, 2, 3, 4";
const DEFAULT_LOW: u64 = 60;
const DEFAULT_VELOCITY: f64 = 0.8;
const MAX_OCTAVE: i32 = 3;

/// The piano row: each key and its semitones above `low`. White keys on
/// the home row, black keys on the row above, where a piano has them.
const PIANO_ROW: [(&str, u8); 18] = [
    ("A", 0),
    ("W", 1),
    ("S", 2),
    ("E", 3),
    ("D", 4),
    ("F", 5),
    ("T", 6),
    ("G", 7),
    ("Y", 8),
    ("H", 9),
    ("U", 10),
    ("J", 11),
    ("K", 12),
    ("O", 13),
    ("L", 14),
    ("P", 15),
    ("Semicolon", 16),
    ("Quote", 17),
];
const ROW: usize = PIANO_ROW.len();
/// Octave down, octave up.
const SHIFT_KEYS: [&str; 2] = ["Z", "X"];

/// The `keys` config: a comma-separated string or a list of names. Unknown
/// names and repeats are skipped, and the list stops at `MAX_KEYS`.
fn keys_of(config: &Config) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut add = |text: &str| {
        for name in text
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|n| !n.is_empty())
        {
            if let Some(k) = Key::from_name(name)
                && !keys.contains(&k)
                && keys.len() < MAX_KEYS
            {
                keys.push(k);
            }
        }
    };
    match config.get("keys") {
        Some(serde_json::Value::String(s)) => add(s),
        Some(serde_json::Value::Array(list)) => {
            for name in list.iter().filter_map(|v| v.as_str()) {
                add(name);
            }
        }
        _ => add(DEFAULT_KEYS),
    }
    keys
}

/// The piano row's lowest note and MIDI channel (0–15), from config,
/// repaired.
fn notes_of(config: &Config) -> (u8, u8) {
    let low = config.get_u64_or("low", DEFAULT_LOW).min(127) as u8;
    let channel = config.get_u64_or("channel", 1).clamp(1, 16) as u8 - 1;
    (low, channel)
}

/// A key's socket name: `key_` and its name in snake case (`PageUp` →
/// `key_page_up`, `1` → `key_1`).
fn socket_name(key: Key) -> String {
    let mut out = String::from("key_");
    let mut prev_lower = false;
    for c in key.name().chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase();
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// A key event's state: down above 0.5, up otherwise; NaN says nothing.
fn key_down(v: f32) -> Option<bool> {
    (!v.is_nan()).then_some(v > 0.5)
}

impl KeyboardIn {
    /// Strikes piano key `n` at the current octave, unless its note is out
    /// of range or there's no room left this block for the note-offs owed.
    fn press(&mut self, n: usize, at: u32, velocity: u8, tx: &mut Sender) {
        let Some(&(_, semitones)) = PIANO_ROW.get(n) else {
            return;
        };
        let note = i32::from(self.low) + i32::from(semitones) + 12 * self.octave;
        let Some(note) = u8::try_from(note).ok().filter(|&n| n <= 127) else {
            return;
        };
        let (Some(slot), Some(holders)) = (
            self.sounding.get_mut(n),
            self.holders.get_mut(usize::from(note)),
        ) else {
            return;
        };
        if *holders == 0 {
            let on = MidiMsg::NoteOn {
                channel: self.channel,
                note,
                velocity,
            };
            if !tx.send_if_room(at, on, self.notes_on + 1) {
                return;
            }
            self.notes_on += 1;
        }
        *holders = holders.saturating_add(1);
        *slot = Some(note);
    }

    /// Releases the note piano key `n` started, if any.
    fn release(&mut self, n: usize, at: u32, tx: &mut Sender) {
        let Some(note) = self.sounding.get_mut(n).and_then(Option::take) else {
            return;
        };
        let Some(holders) = self.holders.get_mut(usize::from(note)) else {
            return;
        };
        *holders = holders.saturating_sub(1);
        if *holders == 0 {
            self.notes_on = self.notes_on.saturating_sub(1);
            tx.send(
                at,
                MidiMsg::NoteOff {
                    channel: self.channel,
                    note,
                    velocity: RELEASE_VELOCITY,
                },
            );
        }
    }
}

impl Node for KeyboardIn {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(
                SocketSpec::float("velocity", Unit::Generic, DEFAULT_VELOCITY)
                    .range(0.0, 1.0)
                    .doc("How hard the piano row plays, 0–1."),
            )
            .output(SocketSpec::midi("midi").doc("Notes from the piano row."));
        for key in keys_of(config) {
            let name = socket_name(key);
            sig = sig
                .output(SocketSpec::boolean(name.clone(), false).doc("High while the key is held."))
                .output(
                    SocketSpec::trigger(format!("{name}_press"))
                        .doc("Fires when the key goes down."),
                );
        }
        sig
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (low, channel) = notes_of(config);
        KeyboardIn {
            keys: keys_of(config),
            key_down: [false; MAX_KEYS],
            row: PIANO_ROW.map(|(name, _)| Key::from_name(name)),
            row_down: [false; ROW],
            sounding: [None; ROW],
            holders: [0; 128],
            notes_on: 0,
            shift: SHIFT_KEYS.map(Key::from_name),
            shift_down: [false; 2],
            octave: 0,
            low,
            channel,
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let events = io.host.controls();
        let velocity = io.inputs.float(VELOCITY);

        for (k, &key) in self.keys.iter().enumerate() {
            let Some(down) = self.key_down.get_mut(k) else {
                break;
            };
            let first = FIRST_KEY + 2 * k;
            let (mut gate_out, mut press) = io.outputs.float_trigger(first, first + 1);
            let mut from = 0;
            for (at, ev) in stamped(events, frames) {
                if ev.control != Control::Key(key) {
                    continue;
                }
                let Some(now) = key_down(ev.value).filter(|&now| now != *down) else {
                    continue;
                };
                fill_range(&mut gate_out, from, at, false, |_| gate(*down));
                if now {
                    press.push(at as u32);
                }
                *down = now;
                from = at;
            }
            fill_range(&mut gate_out, from, frames, false, |_| gate(*down));
        }

        let mut tx = Sender::new(io.outputs.midi(MIDI));
        if self.flush {
            self.flush = false;
            for n in 0..ROW {
                self.release(n, 0, &mut tx);
            }
            let v = midi_velocity(velocity.at(0));
            for n in 0..ROW {
                if self.row_down[n] {
                    self.press(n, 0, v, &mut tx);
                }
            }
        }
        for (at, ev) in stamped(events, frames) {
            let (Control::Key(key), Some(down)) = (ev.control, key_down(ev.value)) else {
                continue;
            };
            if let Some(n) = self.row.iter().position(|&k| k == Some(key))
                && self.row_down[n] != down
            {
                self.row_down[n] = down;
                if down {
                    self.press(n, at as u32, midi_velocity(velocity.at(at)), &mut tx);
                } else {
                    self.release(n, at as u32, &mut tx);
                }
            }
            if let Some(s) = self.shift.iter().position(|&k| k == Some(key))
                && self.shift_down[s] != down
            {
                self.shift_down[s] = down;
                if down {
                    let step = if s == 0 { -1 } else { 1 };
                    self.octave = (self.octave + step).clamp(-MAX_OCTAVE, MAX_OCTAVE);
                }
            }
        }
    }

    fn determinism(&self) -> Determinism {
        Determinism::Live
    }

    fn reset(&mut self) {
        // Notes end at the start of the next block; which keys are held
        // and the octave shift mirror the keyboard (DECISIONS D90).
        self.flush = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{MIDI_CAPACITY, NoResources};
    use serde_json::json;

    fn key(name: &str) -> Control {
        Control::Key(Key::from_name(name).unwrap())
    }

    fn on(note: u8, velocity: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity,
        }
    }

    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: RELEASE_VELOCITY,
        }
    }

    /// Presses and releases `name` at the two frames.
    fn tap(name: &str, down: u64, up: u64) -> [(u64, Control, f32); 2] {
        [(down, key(name), 1.0), (up, key(name), 0.0)]
    }

    #[test]
    fn keys_are_gates_and_triggers_on_the_exact_sample() {
        let mut h = Harness::new::<KeyboardIn>(json!({"keys": "Space, Z"}));
        h.host_controls(&[
            (10, key("Space"), 1.0),
            (40, key("Space"), 1.0),
            (100, key("Space"), 0.0),
            (130, key("Q"), 1.0),
        ]);
        let out = h.run(256);
        let g = out.float("key_space");
        assert!(g[..10].iter().all(|&v| v == 0.0));
        assert!(g[10..100].iter().all(|&v| v == 1.0));
        assert!(g[100..].iter().all(|&v| v == 0.0));
        // A repeat while held doesn't strike again.
        assert_eq!(out.triggers("key_space_press"), &[10]);
        assert!(out.float("key_z").iter().all(|&v| v == 0.0));
        assert!(out.triggers("key_z_press").is_empty());
        assert!(out.midi("midi").is_empty());
    }

    #[test]
    fn held_keys_span_blocks() {
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        h.host_controls(&[(50, key("2"), 1.0), (300, key("2"), 0.0)]);
        let out = h.run(512);
        let g = out.float("key_2");
        assert_eq!(g[49], 0.0);
        assert!(g[50..300].iter().all(|&v| v == 1.0));
        assert_eq!(g[300], 0.0);
        assert_eq!(out.triggers("key_2_press"), &[50]);
    }

    #[test]
    fn the_piano_row_plays_notes() {
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        let mut evs = Vec::new();
        evs.extend(tap("A", 3, 50));
        evs.extend(tap("W", 60, 70));
        evs.extend(tap("Quote", 80, 90));
        evs.extend(tap("Semicolon", 100, 110));
        h.host_controls(&evs);
        assert_eq!(
            h.run(128).midi("midi"),
            &[
                (3, on(60, 102)),
                (50, off(60)),
                (60, on(61, 102)),
                (70, off(61)),
                (80, on(77, 102)),
                (90, off(77)),
                (100, on(76, 102)),
                (110, off(76)),
            ]
        );
        // Every row key plays its own note, in order.
        let mut h = Harness::new::<KeyboardIn>(json!({"low": 48}));
        let evs: Vec<_> = PIANO_ROW
            .iter()
            .enumerate()
            .flat_map(|(i, (name, _))| tap(name, 2 * i as u64, 2 * i as u64 + 1))
            .collect();
        h.host_controls(&evs);
        let notes: Vec<u8> = h
            .run(64)
            .midi("midi")
            .iter()
            .filter_map(|(_, m)| match m {
                MidiMsg::NoteOn { note, .. } => Some(*note),
                _ => None,
            })
            .collect();
        assert_eq!(notes, (48..66).collect::<Vec<u8>>());
    }

    #[test]
    fn an_octave_shift_releases_the_note_that_was_started() {
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        let mut evs = vec![(0, key("A"), 1.0)];
        evs.extend(tap("X", 10, 11));
        evs.push((20, key("A"), 0.0));
        evs.push((30, key("A"), 1.0));
        h.host_controls(&evs);
        assert_eq!(
            h.run(64).midi("midi"),
            &[(0, on(60, 102)), (20, off(60)), (30, on(72, 102))]
        );
        // Shifts stop at three octaves each way.
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        let mut evs: Vec<_> = (0..5).flat_map(|i| tap("X", 2 * i, 2 * i + 1)).collect();
        evs.extend(tap("H", 20, 21));
        evs.extend((0..9).flat_map(|i| tap("Z", 30 + 2 * i, 31 + 2 * i)));
        evs.extend(tap("H", 60, 61));
        h.host_controls(&evs);
        assert_eq!(
            h.run(64).midi("midi"),
            &[
                (20, on(105, 102)),
                (21, off(105)),
                (60, on(33, 102)),
                (61, off(33))
            ]
        );
    }

    #[test]
    fn two_keys_on_one_note_sound_it_once() {
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        let mut evs = vec![(0, key("K"), 1.0)];
        evs.extend(tap("X", 2, 3));
        evs.push((5, key("A"), 1.0));
        evs.push((8, key("K"), 0.0));
        evs.push((12, key("A"), 0.0));
        h.host_controls(&evs);
        assert_eq!(h.run(64).midi("midi"), &[(0, on(72, 102)), (12, off(72))]);
    }

    #[test]
    fn notes_outside_midi_dont_sound() {
        let mut h = Harness::new::<KeyboardIn>(json!({"low": 120}));
        let mut evs = Vec::new();
        evs.extend(tap("L", 0, 5));
        evs.extend(tap("G", 6, 7));
        h.host_controls(&evs);
        assert_eq!(h.run(16).midi("midi"), &[(6, on(127, 102)), (7, off(127))]);
        let mut h = Harness::new::<KeyboardIn>(json!({"low": 0}));
        let mut evs: Vec<_> = tap("Z", 0, 1).into();
        evs.extend(tap("A", 2, 3));
        h.host_controls(&evs);
        assert!(h.run(16).midi("midi").is_empty());
    }

    #[test]
    fn velocity_is_read_at_the_press() {
        let mut h = Harness::new::<KeyboardIn>(json!({"channel": 3}));
        let ramp: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        h.set_samples("velocity", ramp);
        h.host_controls(&[(0, key("D"), 1.0), (63, key("G"), 1.0)]);
        let ch = |note, velocity| MidiMsg::NoteOn {
            channel: 2,
            note,
            velocity,
        };
        assert_eq!(h.run(64).midi("midi"), &[(0, ch(64, 1)), (63, ch(67, 127))]);
    }

    #[test]
    fn nan_values_change_nothing() {
        let mut h = Harness::new::<KeyboardIn>(json!({"keys": "A"}));
        h.host_controls(&[
            (1, key("A"), 1.0),
            (2, key("A"), f32::NAN),
            (3, key("X"), f32::NAN),
        ]);
        let out = h.run(8);
        assert!(out.float("key_a")[1..].iter().all(|&v| v == 1.0));
        assert_eq!(out.midi("midi"), &[(1, on(60, 102))]);
    }

    #[test]
    fn reset_releases_notes_and_strikes_held_keys_again() {
        let mut h = Harness::new::<KeyboardIn>(json!({"keys": "A"}));
        let mut evs = vec![(0, key("S"), 1.0)];
        evs.extend(tap("X", 1, 2));
        evs.extend(tap("F", 3, 4));
        h.host_controls(&evs);
        assert_eq!(
            h.run(64).midi("midi"),
            &[(0, on(62, 102)), (3, on(77, 102)), (4, off(77))]
        );
        h.reset();
        // S is still held: released, then struck again an octave up.
        let out = h.run(64);
        assert_eq!(out.midi("midi"), &[(64, off(62)), (64, on(74, 102))]);
        assert!(h.run(64).midi("midi").is_empty());
        h.host_controls(&[(200, key("S"), 0.0)]);
        assert_eq!(h.run(64).midi("midi"), &[(200, off(74))]);
        // Nothing held: a reset sends nothing.
        h.reset();
        assert!(h.run(64).midi("midi").is_empty());
    }

    #[test]
    fn a_flood_never_leaves_a_note_hanging() {
        // Thousands of presses and releases in one block: more MIDI than
        // an output holds.
        let mut evs = Vec::new();
        for i in 0..3_000u64 {
            let (name, _) = PIANO_ROW[(i * 7 % 18) as usize];
            evs.push((i / 64, key(name), ((i / 18) % 2 == 0) as u8 as f32));
            if i % 97 == 0 {
                evs.push((i / 64, key(if i % 2 == 0 { "X" } else { "Z" }), 1.0));
                evs.push((i / 64, key(if i % 2 == 0 { "X" } else { "Z" }), 0.0));
            }
        }
        for (name, _) in PIANO_ROW {
            evs.push((60, key(name), 1.0));
        }
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        h.host_controls(&evs);
        let first = h.run(64).midi("midi").to_vec();
        assert!(first.len() <= MIDI_CAPACITY);
        assert!(first.len() > MIDI_CAPACITY / 2);
        let ups: Vec<_> = PIANO_ROW.iter().map(|(n, _)| (64, key(n), 0.0)).collect();
        h.host_controls(&ups);
        let second = h.run(64).midi("midi").to_vec();
        let mut held = [0i32; 128];
        for (_, m) in first.iter().chain(&second) {
            match *m {
                MidiMsg::NoteOn { note, .. } => {
                    held[usize::from(note)] += 1;
                    assert_eq!(held[usize::from(note)], 1, "note {note} struck twice");
                }
                MidiMsg::NoteOff { note, .. } => {
                    held[usize::from(note)] -= 1;
                    assert_eq!(held[usize::from(note)], 0, "note {note} ended twice");
                }
                _ => {}
            }
        }
        assert!(held.iter().all(|&n| n == 0), "a note was left hanging");
        // The next block has room again.
        h.host_controls(&tap("A", 130, 140));
        let third = h.run(64).midi("midi").to_vec();
        assert!(matches!(
            third.as_slice(),
            [(130, MidiMsg::NoteOn { note: a, .. }), (140, MidiMsg::NoteOff { note: b, .. })] if a == b
        ));
    }

    #[test]
    fn notes_held_through_a_flood_still_end() {
        // Half the row held, the other half hammered past the output's
        // capacity, then everything let go in the next block.
        let mut evs: Vec<_> = PIANO_ROW[..9]
            .iter()
            .map(|(n, _)| (0, key(n), 1.0))
            .collect();
        for i in 0..4_000u64 {
            let (name, _) = PIANO_ROW[9 + (i % 9) as usize];
            evs.push((1 + i / 80, key(name), ((i / 9) % 2 == 0) as u8 as f32));
        }
        let mut h = Harness::new::<KeyboardIn>(json!({}));
        h.host_controls(&evs);
        let first = h.run(64).midi("midi").to_vec();
        assert!(first.len() <= MIDI_CAPACITY);
        // The held half: A W S E D F T G Y, notes 60 to 68.
        for n in 60..69 {
            assert!(first.contains(&(0, on(n, 102))), "{n} didn't start");
        }
        let ups: Vec<_> = PIANO_ROW.iter().map(|(n, _)| (64, key(n), 0.0)).collect();
        h.host_controls(&ups);
        let second = h.run(64).midi("midi").to_vec();
        let ons = first
            .iter()
            .filter(|(_, m)| matches!(m, MidiMsg::NoteOn { .. }))
            .count();
        assert!(ons > 100);
        assert_eq!(ons, first.len() - ons + second.len(), "unpaired notes");
        for n in 60..69 {
            assert!(second.contains(&(64, off(n))), "{n} was left hanging");
        }
    }

    #[test]
    fn block_size_does_not_matter() {
        let names: Vec<&str> = PIANO_ROW
            .iter()
            .map(|(n, _)| *n)
            .chain(["Z", "X", "1", "2", "Space"])
            .collect();
        let mut evs = Vec::new();
        let mut s = 11u32;
        for i in 0..600u64 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let name = names[(s >> 8) as usize % names.len()];
            evs.push((i * 6 + u64::from(s % 5), key(name), ((s >> 4) % 2) as f32));
        }
        let run = |block| {
            let mut h = Harness::with_resources::<KeyboardIn>(
                json!({"keys": "1, 2, Space, A"}),
                48_000.0,
                block,
                &NoResources,
            );
            let vel: Vec<f32> = (0..4_000).map(|i| (i % 300) as f32 / 300.0).collect();
            h.set_samples("velocity", vel);
            h.host_controls(&evs);
            let out = h.run(4_000);
            (out.midi, out.floats, out.triggers)
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.0["midi"].len() > 100);
    }

    #[test]
    fn config_is_read_and_repaired() {
        let names = |c| keys_of(&c).into_iter().map(socket_name).collect::<Vec<_>>();
        assert_eq!(names(json!({})), ["key_1", "key_2", "key_3", "key_4"]);
        assert_eq!(
            names(json!({"keys": "space,PageUp  leftbracket, F12, nope, SPACE"})),
            ["key_space", "key_page_up", "key_left_bracket", "key_f12"]
        );
        assert_eq!(
            names(json!({"keys": ["Up", 3, "Down, Left"]})),
            ["key_up", "key_down", "key_left"]
        );
        assert!(names(json!({"keys": ""})).is_empty());
        assert_eq!(names(json!({"keys": 7})).len(), 4);
        let every: Vec<&str> = Key::all().map(Key::name).collect();
        assert_eq!(keys_of(&json!({ "keys": every.join(",") })).len(), MAX_KEYS);
        assert_eq!(notes_of(&json!({})), (60, 0));
        assert_eq!(notes_of(&json!({"low": 300, "channel": 0})), (127, 0));
        assert_eq!(notes_of(&json!({"low": "high", "channel": 99})), (60, 15));
        let sig = KeyboardIn::describe(&json!({"keys": "Backtick, 0"}));
        let outs: Vec<&str> = sig.outputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            outs,
            [
                "midi",
                "key_backtick",
                "key_backtick_press",
                "key_0",
                "key_0_press"
            ]
        );
        for k in Key::all() {
            assert!(ac_core::signal::is_snake_case(&socket_name(k)), "{k}");
        }
    }

    #[test]
    fn is_a_live_source() {
        assert!(KeyboardIn::meta().live);
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Keyboard In",
            resources: &NoResources,
        };
        assert_eq!(
            KeyboardIn::new(&json!({}), &ctx).determinism(),
            Determinism::Live
        );
    }
}
