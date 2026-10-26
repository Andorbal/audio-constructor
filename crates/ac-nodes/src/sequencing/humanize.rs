use std::collections::VecDeque;

use ac_core::{
    Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, node,
};
use ac_dsp::rng::Rng;

use super::notes::MIDI_LIMIT;
use crate::triggers::frames_of;

node! {
    kind: "humanize",
    name: "Humanize",
    category: Sequencing,
    keywords: [
        "groove", "feel", "jitter", "random", "loose", "timing", "velocity", "drift", "human"
    ],
    /// Loosens a part the way a player would: each trigger or note comes a
    /// little late and a little louder or softer, differently every time.
    ///
    /// Set the `type` config to *trigger* (the default) or *midi*. Each
    /// event on `in` is delayed by a random time from 0 to `timing`
    /// (seconds, default 10 ms) and its level moved by up to `dynamics` of
    /// itself, up or down (0.1 is ±10%). MIDI note-ons have their velocity
    /// changed (never below 1), and each note-off is delayed as much as its
    /// note-on, so notes keep their length; other messages pass on time.
    /// For triggers, `level` (0 to 1, default 1) is read at each trigger,
    /// changed the same way, and held on `velocity` from where the trigger
    /// comes out, so a Pattern's `velocity_k` wired into `level` keeps its
    /// accents, loosened.
    ///
    /// The randomness comes from the project's seed, so a render sounds
    /// the same every time, and restarting the transport replays it. Every
    /// event uses the same two draws whatever `timing` and `dynamics` are,
    /// so changing them doesn't reshuffle the part. Up to 1024 events can
    /// wait at once; more are dropped.
    ///
    /// **Try this:** put Humanize (`timing` 0.008, `dynamics` 0.2) between
    /// a Pattern's hat row and its envelope, with `velocity_2` into `level`
    /// and Humanize's `velocity` into the hat's level: the same grid, but
    /// it breathes.
    pub struct Humanize {
        midi: bool,
        seed: u64,
        rng: Rng,
        /// The node's own clock: frames since loading or reset, at the
        /// start of the block.
        now: u64,
        /// Waiting events by when they come out, ascending.
        queue: VecDeque<Pending>,
        /// Each note's delay at its note-on, by channel and note, for its
        /// note-off.
        delays: [[u32; 128]; 16],
        /// The `velocity` output (trigger type).
        held: f32,
    }
}

const IN: usize = 0;
const TIMING: usize = 1;
const DYNAMICS: usize = 2;
/// Trigger type only.
const LEVEL: usize = 3;

const OUT: usize = 0;
const VELOCITY: usize = 1;

/// Most events waiting at once.
const CAPACITY: usize = 1024;
pub const TYPES: [&str; 2] = ["trigger", "midi"];

#[derive(Clone, Copy, Debug)]
struct Pending {
    due: u64,
    event: Event,
}

#[derive(Clone, Copy, Debug)]
enum Event {
    Trigger(f32),
    Midi(MidiMsg),
}

fn is_midi(config: &Config) -> bool {
    config.get_str_or("type", TYPES[0]) == TYPES[1]
}

/// `v` scaled by `1 + jitter × amount`, for a jitter in −1..1.
fn loosen(v: f32, jitter: f64, amount: f32) -> f32 {
    let amount = if amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    };
    v * (1.0 + jitter as f32 * amount)
}

impl Humanize {
    /// The two draws for one event: a delay in samples (up to `timing` at
    /// `rate`) and a jitter in −1..1.
    fn draw(&mut self, timing: f32, rate: f64) -> (u32, f64) {
        let (t, j) = (self.rng.uniform(), self.rng.bipolar());
        let timing = if timing.is_finite() {
            f64::from(timing).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((t * timing * rate).round() as u32, j)
    }

    fn wait(&mut self, due: u64, event: Event) {
        if self.queue.len() < CAPACITY.min(self.queue.capacity()) {
            let at = self.queue.partition_point(|p| p.due <= due);
            self.queue.insert(at, Pending { due, event });
        }
    }
}

impl Node for Humanize {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let midi = is_midi(config);
        let mut sig = Signature::new()
            .input(if midi {
                SocketSpec::midi("in").doc("The notes to loosen.")
            } else {
                SocketSpec::trigger("in").doc("The triggers to loosen.")
            })
            .input(
                SocketSpec::float("timing", Unit::Seconds, 0.01)
                    .range(0.0, 0.05)
                    .no_smooth()
                    .doc("Each event comes up to this much late."),
            )
            .input(
                SocketSpec::float("dynamics", Unit::Generic, 0.1)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("How far each level moves, up or down, as a share of itself."),
            );
        if midi {
            sig = sig.output(SocketSpec::midi("out").doc("The notes, loosened."));
        } else {
            sig = sig
                .input(
                    SocketSpec::float("level", Unit::Generic, 1.0)
                        .range(0.0, 1.0)
                        .doc("Each trigger's level, read as it arrives."),
                )
                .output(SocketSpec::trigger("out").doc("The triggers, a little late."))
                .output(
                    SocketSpec::float("velocity", Unit::Generic, 0.0)
                        .range(0.0, 1.0)
                        .doc("The last trigger's level, loosened, from where it comes out."),
                );
        }
        sig
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        Humanize {
            midi: is_midi(config),
            seed: ctx.seed,
            rng: Rng::new(ctx.seed),
            now: 0,
            queue: VecDeque::with_capacity(CAPACITY),
            delays: [[0; 128]; 16],
            held: 0.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let start = self.now;
        let frames = ctx.frames;
        let end = start.saturating_add(frames as u64);
        let rate = if ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            ctx.transport.tempo.sample_rate()
        };
        let timing = io.inputs.float(TIMING);
        let dynamics = io.inputs.float(DYNAMICS);
        if self.midi {
            for e in io.inputs.midi(IN) {
                let i = (e.offset as usize).min(frames.saturating_sub(1));
                let at = start + i as u64;
                let msg = match e.msg {
                    MidiMsg::NoteOn {
                        channel,
                        note,
                        velocity,
                    } if velocity > 0 => {
                        let (delay, jitter) = self.draw(timing.at(i), rate);
                        self.delays[usize::from(channel & 15)][usize::from(note & 127)] = delay;
                        let v = loosen(f32::from(velocity), jitter, dynamics.at(i));
                        let msg = MidiMsg::NoteOn {
                            channel,
                            note,
                            velocity: v.round().clamp(1.0, 127.0) as u8,
                        };
                        self.wait(at + u64::from(delay), Event::Midi(msg));
                        continue;
                    }
                    other => other,
                };
                let delay = match msg {
                    MidiMsg::NoteOff { channel, note, .. }
                    | MidiMsg::NoteOn { channel, note, .. } => {
                        self.delays[usize::from(channel & 15)][usize::from(note & 127)]
                    }
                    _ => 0,
                };
                self.wait(at + u64::from(delay), Event::Midi(msg));
            }
        } else {
            let level = io.inputs.float(LEVEL);
            for o in frames_of(io.inputs.triggers(IN), frames) {
                let i = o as usize;
                let (delay, jitter) = self.draw(timing.at(i), rate);
                let v = level.at(i);
                let v = if v.is_finite() { v } else { 1.0 };
                let v = loosen(v, jitter, dynamics.at(i)).clamp(0.0, 1.0);
                self.wait(start + o + u64::from(delay), Event::Trigger(v));
            }
        }

        if self.midi {
            let mut out = io.outputs.midi(OUT);
            let mut sent = 0;
            while let Some(p) = self.queue.front().copied() {
                if p.due >= end || sent >= MIDI_LIMIT {
                    break;
                }
                self.queue.pop_front();
                if let Event::Midi(msg) = p.event {
                    out.push(MidiEvent::new(p.due.saturating_sub(start) as u32, msg));
                    sent += 1;
                }
            }
        } else {
            let mut held = self.held;
            let (mut vel, mut trig) = io.outputs.float_trigger(VELOCITY, OUT);
            let mut from = 0usize;
            let mut last = None;
            while let Some(p) = self.queue.front().copied() {
                if p.due >= end {
                    break;
                }
                self.queue.pop_front();
                let Event::Trigger(v) = p.event else {
                    continue;
                };
                let at = p.due.saturating_sub(start) as usize;
                for k in from..at {
                    vel.set(k, held);
                }
                from = at;
                held = v;
                if last != Some(at) {
                    trig.push(at as u32);
                    last = Some(at);
                }
            }
            for k in from..frames {
                vel.set(k, held);
            }
            self.held = held;
        }
        self.now = end;
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        self.now = 0;
        self.queue.clear();
        self.held = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<Humanize>(config, 48_000.0, block, &ac_core::NoResources)
    }

    #[test]
    fn triggers_come_late_by_at_most_the_timing() {
        let at: Vec<u64> = (0..200).map(|k| k * 1_000).collect();
        let mut h = harness(json!({}), 64);
        h.triggers("in", &at).set("timing", 0.01);
        let out = h.run(201_000).triggers("out").to_vec();
        assert_eq!(out.len(), at.len());
        let late: Vec<u64> = out.iter().zip(&at).map(|(o, a)| o - a).collect();
        assert!(late.iter().all(|&d| d <= 480), "{late:?}");
        assert!(late.iter().any(|&d| d > 240) && late.iter().any(|&d| d < 240));
    }

    #[test]
    fn levels_move_by_up_to_the_dynamics_and_are_held() {
        let at: Vec<u64> = (0..100).map(|k| k * 100).collect();
        let mut h = harness(json!({}), 64);
        h.triggers("in", &at)
            .set("timing", 0.0)
            .set("dynamics", 0.2)
            .set("level", 0.5);
        let out = h.run(10_000);
        assert_eq!(out.triggers("out"), at.as_slice(), "no timing, on time");
        let vel = out.float("velocity");
        let levels: Vec<f32> = at.iter().map(|&a| vel[a as usize]).collect();
        assert!(
            levels.iter().all(|&v| (0.4..=0.6).contains(&v)),
            "{levels:?}"
        );
        assert!(levels.windows(2).any(|w| w[0] != w[1]));
        assert_eq!(vel[150], vel[100], "held between triggers");
        // No dynamics: the level passes as it is.
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[10])
            .set("dynamics", 0.0)
            .set("level", 0.3);
        assert_eq!(*h.run(2_000).float("velocity").last().unwrap(), 0.3);
    }

    #[test]
    fn note_offs_follow_their_note_ons() {
        let on = |note, velocity| MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity,
        };
        let off = |note| MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 64,
        };
        let mut h = harness(json!({ "type": "midi" }), 64);
        h.midi(
            "in",
            &[
                (0, on(60, 100)),
                (10, on(64, 100)),
                (1_000, off(60)),
                (1_010, off(64)),
            ],
        )
        .set("timing", 0.005)
        .set("dynamics", 0.5);
        let out = h.run(2_000).midi("out").to_vec();
        assert_eq!(out.len(), 4);
        let time = |pred: &dyn Fn(&MidiMsg) -> bool| out.iter().find(|(_, m)| pred(m)).unwrap().0;
        for (note, t) in [(60u8, 0u64), (64, 10)] {
            let t_on = time(&|m| matches!(m, MidiMsg::NoteOn { note: n, .. } if *n == note));
            let t_off = time(&|m| matches!(m, MidiMsg::NoteOff { note: n, .. } if *n == note));
            assert!(t_on >= t && t_on <= t + 240);
            assert_eq!(t_off - t_on, 1_000, "the note keeps its length");
        }
        let velocities: Vec<u8> = out
            .iter()
            .filter_map(|(_, m)| match m {
                MidiMsg::NoteOn { velocity, .. } => Some(*velocity),
                _ => None,
            })
            .collect();
        assert!(velocities.iter().all(|&v| (50..=127).contains(&v)));
        assert!(out.windows(2).all(|w| w[0].0 <= w[1].0), "in order");
    }

    #[test]
    fn the_same_seed_and_any_block_size_give_the_same_part() {
        let at: Vec<u64> = (0..300).map(|k| k * 97 + k % 7).collect();
        let run = |block| {
            let mut h = harness(json!({}), block);
            h.triggers("in", &at).set("timing", 0.004);
            let out = h.run(30_000);
            (out.triggers("out").to_vec(), out.float("velocity").to_vec())
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        // A restart replays it.
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[5, 900]).set("timing", 0.01);
        let first = h.run(2_000).triggers("out").to_vec();
        h.reset();
        h.triggers("in", &[2_000 + 5, 2_000 + 900]);
        let again: Vec<u64> = h
            .run(2_000)
            .triggers("out")
            .iter()
            .map(|t| t - 2_000)
            .collect();
        assert_eq!(first, again);
    }

    #[test]
    fn sockets_follow_the_type() {
        let t = Humanize::describe(&json!({}));
        let names = |s: &[SocketSpec]| s.iter().map(|s| s.name.to_string()).collect::<Vec<_>>();
        assert_eq!(names(&t.inputs), ["in", "timing", "dynamics", "level"]);
        assert_eq!(names(&t.outputs), ["out", "velocity"]);
        let m = Humanize::describe(&json!({ "type": "midi" }));
        assert_eq!(names(&m.inputs), ["in", "timing", "dynamics"]);
        assert_eq!(names(&m.outputs), ["out"]);
    }
}
