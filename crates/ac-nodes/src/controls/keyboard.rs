use ac_core::{
    Config, ConfigExt, Io, MidiEvent, MidiMsg, MidiOut, Node, NodeMeta, PrepareCtx, ProcessCtx,
    Signature, SocketSpec, Unit, node,
};

node! {
    kind: "keyboard",
    name: "Keyboard",
    category: Controls,
    keywords: ["piano", "keys", "notes", "midi", "play", "virtual keyboard", "synth"],
    /// A piano keyboard you play with the mouse, on the canvas or on a
    /// panel; it sends the notes out as MIDI.
    ///
    /// To play notes from the computer keyboard, use a `Keyboard In` node
    /// and turn on Play keys.
    ///
    /// The `keys` config (1–88, default 25) sets how many keys it has and
    /// `low` the MIDI note of the leftmost one (default 48, the C below
    /// middle C). The keyboard is kept within MIDI's 128 notes, so a high
    /// `low` gets fewer keys. Notes go out on MIDI channel `channel`
    /// (1–16, default 1). Out-of-range config values are clamped.
    ///
    /// Pressing a key sends a note-on with the `velocity` of that moment
    /// (0–1, at least the softest MIDI velocity; NaN plays at 0.8), and
    /// releasing it sends the matching note-off. `octave` (−3 to 3) shifts
    /// the keys by octaves; a key keeps the note it started with, so
    /// changing `octave` while holding a key releases the right note later.
    /// Notes shifted outside MIDI's range don't sound. Two keys that land on
    /// one note (after an octave change) sound it once, until both are up.
    /// On one sample, releases go out before presses.
    ///
    /// Each key is a hidden `key_N` input the editor holds down. Keys the
    /// editor plays change at the start of a block; a key input wired to a
    /// gate plays on the exact sample. Restarting the transport releases
    /// every note; keys still held sound again from there.
    ///
    /// **Try this:** wire `midi` into a `MIDI Note` and its `freq` and
    /// `gate` into an `Oscillator` and an `ADSR`, pin the keyboard to a
    /// panel, and play; or wire it into a `Voices` node for chords.
    pub struct Keyboard {
        low: u8,
        keys: usize,
        channel: u8,
        /// Each key's input at the end of the last block.
        down: [bool; MAX_KEYS],
        /// The note each key started, if it sounds.
        sounding: [Option<u8>; MAX_KEYS],
        /// How many keys hold each note.
        holders: [u8; 128],
        /// Release everything at the start of the next block (after reset).
        flush: bool,
    }
}

const VELOCITY: usize = 0;
const OCTAVE: usize = 1;
/// Input index of `key_0`; key `k` is `FIRST_KEY + k`.
const FIRST_KEY: usize = 2;

const MIDI: usize = 0;

const MAX_KEYS: usize = 88;
const MAX_OCTAVE: i32 = 3;
const DEFAULT_VELOCITY: f32 = 0.8;
/// Note-off velocity for keys that don't sense release speed (the MIDI
/// specification's default).
const RELEASE_VELOCITY: u8 = 64;

/// The keyboard's layout from config, repaired: (low note, keys, channel
/// 0–15).
fn layout(config: &Config) -> (u8, usize, u8) {
    let low = config.get_u64_or("low", 48).min(127) as u8;
    let keys = (config.get_u64_or("keys", 25).clamp(1, MAX_KEYS as u64) as usize)
        .min(128 - usize::from(low));
    let channel = config.get_u64_or("channel", 1).clamp(1, 16) as u8 - 1;
    (low, keys, channel)
}

/// A velocity input as a MIDI velocity, 1–127.
pub(crate) fn midi_velocity(v: f32) -> u8 {
    let v = if v.is_nan() { DEFAULT_VELOCITY } else { v };
    (v.clamp(0.0, 1.0) * 127.0).round().max(1.0) as u8
}

impl Keyboard {
    fn press(&mut self, k: usize, at: u32, octave: i32, velocity: u8, midi: &mut MidiOut) {
        let note = i32::from(self.low) + k as i32 + 12 * octave;
        let Ok(note) = u8::try_from(note) else {
            return;
        };
        let (Some(slot), Some(holders)) = (
            self.sounding.get_mut(k),
            self.holders.get_mut(usize::from(note)),
        ) else {
            return;
        };
        if *holders == 0 {
            midi.push(MidiEvent::new(
                at,
                MidiMsg::NoteOn {
                    channel: self.channel,
                    note,
                    velocity,
                },
            ));
        }
        *holders = holders.saturating_add(1);
        *slot = Some(note);
    }

    fn release(&mut self, k: usize, at: u32, midi: &mut MidiOut) {
        let Some(note) = self.sounding.get_mut(k).and_then(Option::take) else {
            return;
        };
        let Some(holders) = self.holders.get_mut(usize::from(note)) else {
            return;
        };
        *holders = holders.saturating_sub(1);
        if *holders == 0 {
            midi.push(MidiEvent::new(
                at,
                MidiMsg::NoteOff {
                    channel: self.channel,
                    note,
                    velocity: RELEASE_VELOCITY,
                },
            ));
        }
    }
}

impl Node for Keyboard {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let (_, keys, _) = layout(config);
        let mut sig = Signature::new()
            .input(
                SocketSpec::float("velocity", Unit::Generic, f64::from(DEFAULT_VELOCITY))
                    .range(0.0, 1.0)
                    .doc("How hard keys are played, 0–1."),
            )
            .input(
                SocketSpec::int("octave", 0)
                    .range(-f64::from(MAX_OCTAVE), f64::from(MAX_OCTAVE))
                    .doc("Shifts the keys by octaves."),
            );
        for k in 0..keys {
            sig = sig.input(
                SocketSpec::boolean(format!("key_{k}"), false)
                    .hidden()
                    .doc("True while the key is held."),
            );
        }
        sig.output(SocketSpec::midi("midi").doc("The notes played."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (low, keys, channel) = layout(config);
        Keyboard {
            low,
            keys,
            channel,
            down: [false; MAX_KEYS],
            sounding: [None; MAX_KEYS],
            holders: [0; 128],
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let octave = io.inputs.int(OCTAVE).clamp(-MAX_OCTAVE, MAX_OCTAVE);
        let velocity = io.inputs.float(VELOCITY);
        let inputs = io.inputs;
        let mut midi = io.outputs.midi(MIDI);
        if self.flush {
            self.flush = false;
            for k in 0..self.keys {
                self.release(k, 0, &mut midi);
                self.down[k] = false;
            }
        }
        let key = |k: usize| inputs.boolean(FIRST_KEY + k);
        let any_varying = (0..self.keys).any(|k| key(k).is_varying());
        let scan = if any_varying {
            ctx.frames
        } else {
            ctx.frames.min(1)
        };
        for i in 0..scan {
            let at = i as u32;
            // Releases first, so a note can be released and struck again on
            // one sample.
            for k in 0..self.keys {
                let input = key(k);
                if (i == 0 || input.is_varying()) && !input.at(i) && self.down[k] {
                    self.down[k] = false;
                    self.release(k, at, &mut midi);
                }
            }
            for k in 0..self.keys {
                let input = key(k);
                if (i == 0 || input.is_varying()) && input.at(i) && !self.down[k] {
                    self.down[k] = true;
                    self.press(k, at, octave, midi_velocity(velocity.at(i)), &mut midi);
                }
            }
        }
    }

    fn reset(&mut self) {
        self.flush = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

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

    #[test]
    fn keys_play_notes_at_the_block_start() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        assert!(h.run(64).midi("midi").is_empty());
        h.set("key_0", true).set("key_4", true);
        assert_eq!(
            h.run(128).midi("midi"),
            &[(64, on(48, 102)), (64, on(52, 102))]
        );
        h.set("key_0", false);
        assert_eq!(h.run(64).midi("midi"), &[(192, off(48))]);
    }

    #[test]
    fn an_octave_change_releases_the_note_that_was_started() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        h.set("key_2", true);
        assert_eq!(h.run(64).midi("midi"), &[(0, on(50, 102))]);
        h.set("octave", 1);
        assert!(h.run(64).midi("midi").is_empty());
        h.set("key_2", false);
        assert_eq!(h.run(64).midi("midi"), &[(128, off(50))]);
        h.set("key_2", true);
        assert_eq!(h.run(64).midi("midi"), &[(192, on(62, 102))]);
        // Octave clamps to ±3, and notes outside MIDI's range don't sound.
        let mut h = Harness::new::<Keyboard>(json!({"low": 0}));
        h.set("octave", -9).set("key_0", true).set("key_24", true);
        assert!(h.run(64).midi("midi").is_empty());
    }

    #[test]
    fn wired_keys_play_on_the_exact_sample() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        let gate: Vec<f32> = (0..256)
            .map(|i| ((10..100).contains(&i)) as u8 as f32)
            .collect();
        h.set_samples("key_3", gate);
        h.set_samples("velocity", vec![1.0; 256]);
        assert_eq!(
            h.run(256).midi("midi"),
            &[(10, on(51, 127)), (100, off(51))]
        );
    }

    #[test]
    fn releases_go_before_presses_on_one_sample() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        let a: Vec<f32> = (0..64).map(|i| (i < 20) as u8 as f32).collect();
        let b: Vec<f32> = (0..64).map(|i| (i >= 20) as u8 as f32).collect();
        h.set_samples("key_1", b).set_samples("key_0", a);
        assert_eq!(
            h.run(64).midi("midi"),
            &[(0, on(48, 102)), (20, off(48)), (20, on(49, 102))]
        );
    }

    #[test]
    fn two_keys_on_one_note_sound_it_once() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        h.set("key_12", true);
        assert_eq!(h.run(64).midi("midi"), &[(0, on(60, 102))]);
        h.set("octave", 1).set("key_0", true);
        assert!(h.run(64).midi("midi").is_empty());
        h.set("key_12", false);
        assert!(h.run(64).midi("midi").is_empty());
        h.set("key_0", false);
        assert_eq!(h.run(64).midi("midi"), &[(192, off(60))]);
    }

    #[test]
    fn velocity_extremes() {
        assert_eq!(midi_velocity(0.0), 1);
        assert_eq!(midi_velocity(-3.0), 1);
        assert_eq!(midi_velocity(5.0), 127);
        assert_eq!(midi_velocity(f32::NAN), 102);
        assert_eq!(midi_velocity(f32::INFINITY), 127);
    }

    #[test]
    fn config_sets_the_layout_and_is_repaired() {
        let sig = Keyboard::describe(&json!({}));
        assert_eq!(sig.inputs.len(), FIRST_KEY + 25);
        assert!(sig.inputs[FIRST_KEY..].iter().all(|s| s.hidden));
        assert_eq!(sig.inputs[FIRST_KEY + 24].name, "key_24");
        assert!(!sig.inputs[VELOCITY].hidden && !sig.inputs[OCTAVE].hidden);
        assert_eq!(layout(&json!({"keys": 88, "low": 21})), (21, 88, 0));
        // 88 keys from note 48 would run past 127.
        assert_eq!(layout(&json!({"keys": 200})), (48, 80, 0));
        assert_eq!(layout(&json!({"keys": 0, "channel": 0})), (48, 1, 0));
        assert_eq!(layout(&json!({"low": 120, "channel": 99})), (120, 8, 15));
        assert_eq!(layout(&json!({"low": 1000, "keys": "many"})), (127, 1, 0));
        let mut h = Harness::new::<Keyboard>(json!({"channel": 10, "low": 60, "keys": 12}));
        h.set("key_11", true);
        assert_eq!(
            h.run(64).midi("midi"),
            &[(
                0,
                MidiMsg::NoteOn {
                    channel: 9,
                    note: 71,
                    velocity: 102
                }
            )]
        );
    }

    #[test]
    fn block_size_does_not_matter() {
        let gate = |period: usize, len: usize| -> Vec<f32> {
            (0..4_000)
                .map(|i| ((i % period) < len) as u8 as f32)
                .collect()
        };
        let run = |block| {
            let mut h = Harness::with_resources::<Keyboard>(
                json!({}),
                48_000.0,
                block,
                &ac_core::NoResources,
            );
            h.set_samples("key_0", gate(301, 120));
            h.set_samples("key_7", gate(173, 40));
            h.set_samples("key_24", gate(997, 500));
            h.set("key_12", true);
            h.run(4_000).midi("midi").to_vec()
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.len() > 50);
    }

    #[test]
    fn reset_releases_held_notes() {
        let mut h = Harness::new::<Keyboard>(json!({}));
        h.set("key_5", true);
        h.run(64);
        h.reset();
        h.set("key_5", false);
        assert_eq!(h.run(64).midi("midi"), &[(64, off(53))]);
        assert!(h.run(64).midi("midi").is_empty());
        // A key still held after the reset sounds again.
        h.set("key_5", true);
        h.run(64);
        h.reset();
        assert_eq!(
            h.run(64).midi("midi"),
            &[(256, off(53)), (256, on(53, 102))]
        );
    }
}
