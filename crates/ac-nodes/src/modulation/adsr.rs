use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::envelope::{Adsr as Envelope, Curve, Params};

use crate::segments::Runs;

node! {
    kind: "adsr",
    name: "ADSR",
    category: Modulation,
    keywords: ["envelope", "eg", "adsr", "contour", "attack", "release"],
    /// An attack-decay-sustain-release envelope: rises to 1 while the gate
    /// opens, falls to `sustain`, holds, and falls to 0 when the gate closes.
    /// The output is a sample-rate `Float` from 0 to 1.
    ///
    /// Drive it with `gate` (a `Bool`: attack on the rising edge, release on
    /// the falling edge, detected per sample) or with the `on` and `off`
    /// triggers, or both. Retriggering starts the attack from the current
    /// level, so it never clicks.
    ///
    /// - `attack` is the time from 0 to 1 (sooner when retriggered from
    ///   higher up), `decay` the time from 1 down to `sustain`, `release`
    ///   the time from wherever the level is down to 0. A time of 0 jumps on
    ///   the next sample.
    /// - `curve`: `linear` ramps, or `exponential` (default) analog-style
    ///   curves: a bowed attack and decays that drop fast and then ease out.
    ///
    /// `active` is true (per sample) whenever the envelope isn't idle, from
    /// the attack until the release reaches 0; `end` fires on the sample the
    /// release reaches 0. When a gate edge and a trigger land on the same
    /// sample, releases (gate falling, `off`) apply before attacks (gate
    /// rising, `on`), so a note-off and the next note-on retrigger cleanly.
    ///
    /// **Try this:** wire `MidiNote`'s gate into `gate` and the output
    /// through a `Map` (0..1 → -60..0 dB) into a `Gain` after an Oscillator
    /// for a playable synth voice; wire `end` into a `Clip`'s `start` to
    /// chain a sound after the note dies away.
    pub struct Adsr {
        env: Envelope,
        gate: bool,
        /// Frames where `end` fired this block.
        ends: Vec<u32>,
    }
}

const GATE: usize = 0;
const ON: usize = 1;
const OFF: usize = 2;
const ATTACK: usize = 3;
const DECAY: usize = 4;
const SUSTAIN: usize = 5;
const RELEASE: usize = 6;
const CURVE: usize = 7;

const OUT: usize = 0;
const ACTIVE: usize = 1;
const END: usize = 2;

impl Node for Adsr {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::boolean("gate", false).doc("Attack while true, release when it falls."),
            )
            .input(SocketSpec::trigger("on").doc("Start the attack from the current level."))
            .input(SocketSpec::trigger("off").doc("Start the release."))
            .input(
                SocketSpec::float("attack", Unit::Seconds, 0.01)
                    .range(0.0, 10.0)
                    .doc("Time from 0 to 1."),
            )
            .input(
                SocketSpec::float("decay", Unit::Seconds, 0.2)
                    .range(0.0, 10.0)
                    .doc("Time from 1 down to the sustain level."),
            )
            .input(
                SocketSpec::float("sustain", Unit::Generic, 0.7)
                    .range(0.0, 1.0)
                    .doc("Level held while the gate is open."),
            )
            .input(
                SocketSpec::float("release", Unit::Seconds, 0.3)
                    .range(0.0, 20.0)
                    .doc("Time from the current level down to 0."),
            )
            .input(SocketSpec::choice("curve", Curve::NAMES, "exponential").doc("Stage shape."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The envelope, 0 to 1."),
            )
            .output(
                SocketSpec::boolean("active", false)
                    .rate(Rate::Sample)
                    .doc("True from the attack until the release ends."),
            )
            .output(SocketSpec::trigger("end").doc("Fires when the release reaches 0."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Adsr {
            env: Envelope::new(),
            gate: false,
            ends: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let gate = io.inputs.boolean(GATE);
        let attack = io.inputs.float(ATTACK);
        let decay = io.inputs.float(DECAY);
        let sustain = io.inputs.float(SUSTAIN);
        let release = io.inputs.float(RELEASE);
        let curve = Curve::from_index(io.inputs.choice(CURVE, Curve::NAMES.len()));
        let events = [io.inputs.triggers(OFF), io.inputs.triggers(ON)];
        let varying = attack.is_varying()
            || decay.is_varying()
            || sustain.is_varying()
            || release.is_varying();
        let params_at = |i: usize| {
            Params::from_seconds(
                f64::from(attack.at(i)),
                f64::from(decay.at(i)),
                f64::from(sustain.at(i)),
                f64::from(release.at(i)),
                curve,
                rate,
            )
        };
        let steady = params_at(0);

        self.ends.clear();
        {
            let (mut out, mut active) = io.outputs.float2(OUT, ACTIVE);
            for run in Runs::new(events, io.inputs.frames) {
                let [off, on] = run.fired;
                for i in run.at..run.until {
                    let g = gate.at(i);
                    let first = i == run.at;
                    if (self.gate && !g) || (first && off) {
                        self.env.gate_off();
                    }
                    if (!self.gate && g) || (first && on) {
                        self.env.gate_on();
                    }
                    self.gate = g;
                    let p = if varying { params_at(i) } else { steady };
                    let (level, ended) = self.env.tick(&p);
                    out.set(i, level as f32);
                    active.set(i, if self.env.is_active() { 1.0 } else { 0.0 });
                    if ended && self.ends.len() < self.ends.capacity() {
                        self.ends.push(i as u32);
                    }
                }
            }
        }
        let mut end = io.outputs.trigger(END);
        for &at in &self.ends {
            end.push(at);
        }
    }

    fn reset(&mut self) {
        self.env.reset();
        self.gate = false;
        self.ends.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    /// A gate high over `on..off` (absolute frames), `n` frames long.
    fn gate(on: usize, off: usize, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if (on..off).contains(&i) { 1.0 } else { 0.0 })
            .collect()
    }

    fn harness(curve: &str) -> Harness {
        let mut h = Harness::new::<Adsr>(json!({}));
        h.set("curve", curve);
        h
    }

    #[test]
    fn attack_reaches_one_at_the_attack_time() {
        for curve in ["linear", "exponential"] {
            let mut h = harness(curve);
            h.set_samples("gate", gate(100, 10_000, 10_000))
                .set("attack", 0.01)
                .set("decay", 0.1);
            let out = h.run(10_000);
            let o = out.float("out");
            assert_eq!(o[99], 0.0);
            // 0.01 s is 480 samples: the 480th sample of the attack is the peak.
            assert_eq!(o[100 + 479], 1.0, "{curve}");
            assert!(o[100 + 470] < 1.0, "{curve}");
            if curve == "linear" {
                assert!((o[100 + 239] - 0.5).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn decays_to_and_holds_the_sustain_level() {
        for curve in ["linear", "exponential"] {
            let mut h = harness(curve);
            h.set("gate", true)
                .set("attack", 0.001)
                .set("decay", 0.05)
                .set("sustain", 0.4);
            let out = h.run(9_600);
            let o = out.float("out");
            // Attack 48 samples, decay 2400.
            assert!(o[48 + 1200] > 0.4 && o[48 + 1200] < 1.0, "{curve}");
            assert!(
                o[48 + 2400 + 2..].iter().all(|&v| (v - 0.4).abs() < 1e-6),
                "{curve}"
            );
            assert!(out.float("active").iter().all(|&a| a == 1.0));
        }
    }

    #[test]
    fn release_reaches_zero_and_end_fires_once() {
        for curve in ["linear", "exponential"] {
            let mut h = harness(curve);
            h.set_samples("gate", gate(0, 4_800, 12_000))
                .set("release", 0.05);
            let out = h.run(12_000);
            let o = out.float("out");
            let ends = out.triggers("end");
            assert_eq!(ends.len(), 1, "{curve}: {ends:?}");
            // Release starts at 4800 and takes 2400 samples.
            let at = ends[0] as usize;
            assert!(
                (4_800 + 2_398..=4_800 + 2_401).contains(&at),
                "{curve}: {at}"
            );
            assert_eq!(o[at], 0.0);
            assert!(o[at - 1] > 0.0);
            assert_eq!(peak(&o[at..]), 0.0);
            let active = out.float("active");
            assert_eq!(active[at - 1], 1.0);
            assert!(active[at..].iter().all(|&a| a == 0.0));
        }
    }

    #[test]
    fn gate_shorter_than_attack_releases_from_where_it_got() {
        let mut h = harness("linear");
        h.set_samples("gate", gate(0, 960, 12_000))
            .set("attack", 0.1)
            .set("release", 0.1);
        let out = h.run(12_000);
        let o = out.float("out");
        // 960 of 4800 attack samples: 0.2.
        assert!((peak(o) - 0.2).abs() < 1e-3, "{}", peak(o));
        assert_eq!(out.triggers("end"), &[960 + 4_800 - 1]);
    }

    #[test]
    fn zero_times_are_instant() {
        let mut h = harness("exponential");
        h.set_samples("gate", gate(10, 20, 64))
            .set("attack", 0.0)
            .set("decay", 0.0)
            .set("sustain", 0.5)
            .set("release", 0.0);
        let out = h.run(64);
        let o = out.float("out");
        assert_eq!(&o[9..13], &[0.0, 1.0, 0.5, 0.5]);
        assert_eq!(o[19], 0.5);
        assert_eq!(o[20], 0.0);
        assert_eq!(out.triggers("end"), &[20]);
    }

    #[test]
    fn on_and_off_triggers_drive_it_too() {
        let mut h = harness("linear");
        h.triggers("on", &[100])
            .triggers("off", &[3_000])
            .set("release", 0.01);
        let out = h.run(4_000);
        let o = out.float("out");
        assert_eq!(o[99], 0.0);
        assert!(o[100] > 0.0);
        assert_eq!(out.triggers("end"), &[3_000 + 479]);
    }

    #[test]
    fn retrigger_during_release_does_not_click() {
        let mut h = harness("exponential");
        let g: Vec<f32> = (0..20_000)
            .map(|i| {
                if (5_000..6_000).contains(&i) {
                    0.0
                } else {
                    1.0
                }
            })
            .collect();
        h.set_samples("gate", g).set("release", 0.5);
        let out = h.run(20_000);
        let o = out.float("out");
        let jump = (o[6_000] - o[5_999]).abs();
        assert!(jump < 0.01, "{jump}");
        assert!(out.triggers("end").is_empty());
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let g = gate(37, 3_001, 8_000);
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Adsr>(json!({}), RATE, block, &NoResources);
            h.set_samples("gate", g.clone())
                .triggers("on", &[5_000])
                .triggers("off", &[6_500])
                .set("attack", 0.005)
                .set("decay", 0.01)
                .set("release", 0.02);
            let out = h.run(8_000);
            (
                out.float("out").to_vec(),
                out.float("active").to_vec(),
                out.triggers("end").to_vec(),
            )
        };
        let one = render(1);
        assert_eq!(one.2.len(), 2);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1_000));
    }

    #[test]
    fn extreme_parameters_stay_finite_and_in_range() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(5_000).copied().collect();
        let g: Vec<f32> = (0..5_000).map(|i| ((i / 37) % 2) as f32).collect();
        for curve in ["linear", "exponential"] {
            let mut h = harness(curve);
            h.set_samples("gate", g.clone())
                .set_samples("attack", wild.clone())
                .set_samples("decay", wild.iter().rev().copied().collect())
                .set_samples("sustain", wild.clone())
                .set_samples("release", wild.clone());
            let out = h.run(5_000);
            assert!(
                out.float("out")
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "{curve}"
            );
        }
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut h = harness("linear");
        h.set("gate", true);
        h.run(1_000);
        h.reset();
        h.set("gate", false);
        let out = h.run(64);
        assert_eq!(peak(out.float("out")), 0.0);
        assert!(out.triggers("end").is_empty());
    }
}
