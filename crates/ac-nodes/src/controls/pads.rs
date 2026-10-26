use ac_core::{
    Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate,
    Signature, SocketSpec, Unit, node,
};

use super::keyboard::midi_velocity;
use crate::sequencing::notes::{is_all_notes_off, note_off_of, note_on_of};

node! {
    kind: "pads",
    name: "Pads",
    category: Controls,
    keywords: [
        "drum pads", "pad bank", "grid", "4x4", "mpc", "launchpad", "velocity", "finger drum",
        "clip launcher", "buttons"
    ],
    /// A grid of drum pads in one node: rows × columns of pads that play
    /// notes with velocity, fire triggers, and light up.
    ///
    /// The `rows` and `columns` config (1–8 each, default 4 × 4) shape the
    /// grid. Pads count from the bottom-left, left to right and then
    /// upwards, as on a drum machine: pad *k* plays MIDI note `low` + *k*
    /// (config `low`, default 36, the usual kick) on channel `channel`
    /// (config, 1–16), so a 4 × 4 grid plays notes 36 to 51. Where you hit
    /// a pad sets the velocity, harder towards its top. Each pad also fires
    /// its own `trigger_k` when hit, `pad` says which was hit last (from 0)
    /// and `velocity` how hard (0–1).
    ///
    /// Pads light while their note is on in `lit`: wire the MIDI that plays
    /// your sounds (a pattern, a keyboard, a drum pad) into it and the grid
    /// shows what's playing. Each pad is a hidden `pad_k` input the editor
    /// holds down; restarting the transport releases them.
    ///
    /// **Try this:** wire `midi` and a `MIDI In` from a drum pad through a
    /// `Merge` into a `Slicer` of a drum loop, and the same Merge's `out`
    /// into `lit`; pin the Pads to a panel and play.
    pub struct Pads {
        pads: usize,
        low: u8,
        channel: u8,
        down: [bool; MAX_PADS],
        /// The note each pad started, if it sounds.
        sounding: [Option<u8>; MAX_PADS],
        /// Release every pad at the next block (after a reset).
        flush: bool,
        /// Notes on in `lit`, by note.
        lit: [u8; 128],
        /// Notes that started in `lit` this block (shown even if they end in it).
        flash: [bool; 128],
        last_pad: i32,
        last_velocity: f32,
    }
}

const LIT: usize = 0;
const HIT: usize = 1;
/// Input index of `pad_0`.
const FIRST_PAD: usize = 2;

const MIDI: usize = 0;
const VELOCITY: usize = 1;
const PAD: usize = 2;
/// Output index of `trigger_0`.
const FIRST_TRIGGER: usize = 3;

pub const MAX_SIDE: u64 = 8;
pub const MAX_PADS: usize = 64;
const RELEASE_VELOCITY: u8 = 64;

/// Telemetry: pad `k` is lit (1) or not, for `k` below [`MAX_PADS`].
pub const TELEMETRY_LIT: usize = 0;

/// `(rows, columns)` from config, each 1–8.
pub fn shape(config: &Config) -> (usize, usize) {
    let rows = config.get_u64_or("rows", 4).clamp(1, MAX_SIDE) as usize;
    let cols = config.get_u64_or("columns", 4).clamp(1, MAX_SIDE) as usize;
    (rows, cols)
}

/// The note of pad 0 and the channel (0–15), from config; the grid is
/// kept within MIDI's notes by clamping `low`.
fn notes(config: &Config, pads: usize) -> (u8, u8) {
    let low = config.get_u64_or("low", 36).min(128 - pads as u64) as u8;
    let channel = config.get_u64_or("channel", 1).clamp(1, 16) as u8 - 1;
    (low, channel)
}

/// The pad drawn at `row` (0 = the top row) and `column` of a grid of
/// `rows` rows: pads count from the bottom-left.
pub fn pad_at(rows: usize, columns: usize, row: usize, column: usize) -> usize {
    (rows - 1 - row.min(rows - 1)) * columns + column.min(columns - 1)
}

impl Pads {
    fn press(&mut self, k: usize, at: u32, velocity: f32, midi: &mut ac_core::MidiOut) {
        let note = self.low + k as u8;
        midi.push(MidiEvent::new(
            at,
            MidiMsg::NoteOn {
                channel: self.channel,
                note,
                velocity: midi_velocity(velocity),
            },
        ));
        self.sounding[k] = Some(note);
    }

    fn release(&mut self, k: usize, at: u32, midi: &mut ac_core::MidiOut) {
        if let Some(note) = self.sounding.get_mut(k).and_then(Option::take) {
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

impl Node for Pads {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(MAX_PADS)
    }

    fn describe(config: &Config) -> Signature {
        let (rows, cols) = shape(config);
        let pads = rows * cols;
        let mut sig = Signature::new()
            .input(SocketSpec::midi("lit").doc("Pads light while their note is on here."))
            .input(
                SocketSpec::float("hit", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .hidden()
                    .doc("How hard the next press is, 0–1 (set where a pad is hit)."),
            );
        for k in 0..pads {
            sig = sig.input(
                SocketSpec::boolean(format!("pad_{k}"), false)
                    .hidden()
                    .doc("True while the pad is held."),
            );
        }
        sig = sig
            .output(SocketSpec::midi("midi").doc("A note per pad, with velocity."))
            .output(
                SocketSpec::float("velocity", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("How hard the last hit was, 0–1."),
            )
            .output(SocketSpec::int("pad", -1).doc("The last pad hit, from 0 (−1 before any)."));
        for k in 0..pads {
            sig = sig.output(
                SocketSpec::trigger(format!("trigger_{k}")).doc("Fires when this pad is hit."),
            );
        }
        sig
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (rows, cols) = shape(config);
        let pads = rows * cols;
        let (low, channel) = notes(config, pads);
        Pads {
            pads,
            low,
            channel,
            down: [false; MAX_PADS],
            sounding: [None; MAX_PADS],
            flush: false,
            lit: [0; 128],
            flash: [false; 128],
            last_pad: -1,
            last_velocity: 0.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let hit = io.inputs.float(HIT).first();
        let hit = if hit.is_nan() {
            1.0
        } else {
            hit.clamp(0.0, 1.0)
        };
        let inputs = io.inputs;
        self.flash = [false; 128];
        for e in inputs.midi(LIT) {
            if is_all_notes_off(&e.msg) {
                self.lit = [0; 128];
            } else if let Some((_, note, _)) = note_on_of(&e.msg) {
                let n = usize::from(note);
                self.lit[n] = self.lit[n].saturating_add(1);
                self.flash[n] = true;
            } else if let Some((_, note, _)) = note_off_of(&e.msg) {
                let n = usize::from(note);
                self.lit[n] = self.lit[n].saturating_sub(1);
            }
        }
        let mut midi = io.outputs.midi(MIDI);
        if self.flush {
            self.flush = false;
            for k in 0..self.pads {
                self.release(k, 0, &mut midi);
                self.down[k] = false;
            }
        }
        let pad = |k: usize| inputs.boolean(FIRST_PAD + k);
        let varying = (0..self.pads).any(|k| pad(k).is_varying());
        let scan = if varying {
            ctx.frames
        } else {
            ctx.frames.min(1)
        };
        // Hits this block: (frame, pad), in order.
        let mut hits = [(0u32, 0usize); MAX_PADS];
        let mut n_hits = 0;
        for i in 0..scan {
            let at = i as u32;
            // Releases first, so a pad can come up and be hit on one sample.
            for k in 0..self.pads {
                let input = pad(k);
                if (i == 0 || input.is_varying()) && !input.at(i) && self.down[k] {
                    self.down[k] = false;
                    self.release(k, at, &mut midi);
                }
            }
            for k in 0..self.pads {
                let input = pad(k);
                if (i == 0 || input.is_varying()) && input.at(i) && !self.down[k] {
                    self.down[k] = true;
                    self.press(k, at, hit, &mut midi);
                    if n_hits < MAX_PADS {
                        hits[n_hits] = (at, k);
                        n_hits += 1;
                    }
                    self.last_pad = k as i32;
                    self.last_velocity = hit;
                }
            }
        }
        for &(at, k) in &hits[..n_hits] {
            io.outputs.trigger(FIRST_TRIGGER + k).push(at);
        }
        io.outputs.set_float(VELOCITY, self.last_velocity);
        io.outputs.set_int(PAD, self.last_pad);
        for k in 0..self.pads {
            let n = usize::from(self.low) + k;
            let on = self.lit.get(n).is_some_and(|&c| c > 0) || self.flash.get(n) == Some(&true);
            io.telemetry
                .set(TELEMETRY_LIT + k, if on { 1.0 } else { 0.0 });
        }
    }

    fn reset(&mut self) {
        self.flush = true;
        self.lit = [0; 128];
        self.last_pad = -1;
        self.last_velocity = 0.0;
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

    #[test]
    fn pads_play_notes_from_the_bottom_left_with_velocity() {
        let mut h = Harness::new::<Pads>(json!({}));
        assert_eq!(h.signature().outputs.len(), 3 + 16);
        h.set("hit", 0.5).set("pad_0", true);
        let out = h.run(64);
        assert_eq!(out.midi("midi"), &[(0, on(36, 64))]);
        assert_eq!(out.triggers("trigger_0"), &[0]);
        assert_eq!(out.int("pad")[0], 0);
        assert_eq!(out.float("velocity")[0], 0.5);
        h.set("hit", 1.0).set("pad_15", true);
        let out = h.run(64);
        assert_eq!(out.midi("midi"), &[(64, on(51, 127))]);
        assert_eq!(out.int("pad")[0], 15);
        h.set("pad_0", false);
        let out = h.run(64);
        assert_eq!(
            out.midi("midi"),
            &[(
                128,
                MidiMsg::NoteOff {
                    channel: 0,
                    note: 36,
                    velocity: RELEASE_VELOCITY
                }
            )]
        );
        // The top-left pad of a 4 × 4 grid is pad 12.
        assert_eq!(pad_at(4, 4, 0, 0), 12);
        assert_eq!(pad_at(4, 4, 3, 3), 3);
    }

    #[test]
    fn lit_notes_light_their_pads() {
        let mut h = Harness::new::<Pads>(json!({"rows": 2, "columns": 2, "low": 60}));
        h.midi("lit", &[(5, on(61, 100))]);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LIT + 1), 1.0);
        assert_eq!(h.telemetry(TELEMETRY_LIT), 0.0);
        // Still held next block; a note that starts and ends in one block
        // still shows.
        h.midi(
            "lit",
            &[
                (
                    70,
                    MidiMsg::NoteOff {
                        channel: 0,
                        note: 61,
                        velocity: 0,
                    },
                ),
                (80, on(60, 90)),
                (
                    90,
                    MidiMsg::NoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                ),
            ],
        );
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LIT + 1), 0.0);
        assert_eq!(h.telemetry(TELEMETRY_LIT), 1.0);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LIT), 0.0);
    }

    #[test]
    fn a_reset_releases_held_pads_and_the_grid_stays_in_midi() {
        let mut h = Harness::new::<Pads>(json!({"rows": 8, "columns": 8, "low": 120}));
        assert_eq!(h.signature().outputs.len(), 3 + 64);
        h.set("pad_63", true);
        let out = h.run(32);
        // `low` comes down so the top pad is note 127.
        assert_eq!(out.midi("midi")[0].1, on(127, 127));
        h.reset();
        h.set("pad_63", false);
        let out = h.run(32);
        assert!(matches!(
            out.midi("midi")[0].1,
            MidiMsg::NoteOff { note: 127, .. }
        ));
    }
}
