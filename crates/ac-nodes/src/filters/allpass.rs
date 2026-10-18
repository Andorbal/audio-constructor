use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::allpass::Allpass as AllpassSection;

use super::{DelayTimes, line_len, max_delay};

node! {
    kind: "allpass",
    name: "Allpass",
    category: Filters,
    keywords: ["schroeder", "diffusion", "reverb", "phase", "smear", "dispersion"],
    /// A Schroeder allpass filter: smears a signal into a train of echoes
    /// without changing its level at any frequency.
    ///
    /// A click comes out as a dense, decaying train of echoes, which makes
    /// it a reverb ingredient.
    ///
    /// - `delay` (seconds) is the spacing of the echoes. Reads between
    ///   samples go through an allpass interpolator, which keeps the response
    ///   flat, so it can be modulated (slow wobble breaks up metallic
    ///   ringing); a block-rate `delay` glides across each block. It is held
    ///   between one sample and `max_delay` (delays under 1.5 samples are
    ///   held at 1).
    /// - `gain` (−0.99 to 0.99, clamped) sets how much echoes feed back and
    ///   forward: higher magnitudes give longer, denser smears. 0 is a plain
    ///   delay.
    ///
    /// The magnitude response is flat at any delay: what changes is only
    /// the phase, so on its own it sounds subtle on steady tones and obvious
    /// on clicks and drums. Several in series with unrelated delays
    /// (1 to 20 ms, gain around 0.6) make a diffuser; see also `Diffuser`,
    /// which does that in one node.
    ///
    /// Config: `max_delay`, the longest delay in seconds (default 0.1, 0.001
    /// to 2), which sizes the buffer. The first two channels are filtered;
    /// further channels pass through dry.
    ///
    /// **Try this:** chain four Allpasses with delays 4.7, 3.6, 12.7, and
    /// 9.3 ms and gains 0.75, 0.75, 0.625, 0.625 between a drum Clip and a
    /// feedback `Comb` for a small, home-made reverb.
    pub struct Allpass {
        sections: Vec<AllpassSection>,
        times: DelayTimes,
    }
}

const IN: usize = 0;
const DELAY: usize = 1;
const GAIN: usize = 2;
const OUT: usize = 0;

/// Channels with their own delay line; further channels pass dry.
const LINES: usize = 2;

/// `|gain|` is kept below this so the recursion decays audibly fast.
const MAX_GAIN: f32 = 0.99;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Node for Allpass {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to smear."))
            .input(
                SocketSpec::float("delay", Unit::Seconds, 0.01)
                    .range(0.0, max_delay(config))
                    .doc("Echo spacing."),
            )
            .input(
                SocketSpec::float("gain", Unit::Generic, 0.5)
                    .range(-0.99, 0.99)
                    .doc("Echo feedback and feedforward amount."),
            )
            .output(SocketSpec::audio("out").doc("The smeared signal, at full level."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let len = line_len(config, ctx.sample_rate);
        Allpass {
            sections: (0..LINES).map(|_| AllpassSection::new(len)).collect(),
            times: DelayTimes::new(ctx.max_block),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        self.times
            .fill(io.inputs.float(DELAY), ctx.sample_rate, frames);
        let input = io.inputs.audio(IN);
        let gain = io.inputs.float(GAIN);
        let g0 = finite_or(gain.first(), 0.0).clamp(-MAX_GAIN, MAX_GAIN);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            let Some(ap) = self.sections.get_mut(c) else {
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = *s;
                }
                continue;
            };
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let g = if gain.is_varying() {
                    finite_or(gain.at(i), 0.0).clamp(-MAX_GAIN, MAX_GAIN)
                } else {
                    g0
                };
                *o = ap.tick(x, self.times.at(i), g);
            }
        }
    }

    fn reset(&mut self) {
        for ap in &mut self.sections {
            ap.clear();
        }
        self.times.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn impulse(n: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[0] = 1.0;
        x
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32)
            .collect()
    }

    #[test]
    fn impulse_response_is_a_decaying_echo_train() {
        let mut h = Harness::new::<Allpass>(json!({}));
        h.set_audio("in", vec![impulse(2_000)])
            .set("delay", 0.01)
            .set("gain", 0.5);
        let o = h.run(2_000).audio("out")[0].clone();
        assert_eq!(o[0], -0.5);
        assert_eq!(o[480], 0.75);
        assert_eq!(o[960], 0.375);
        assert_eq!(o[1440], 0.1875);
        assert_eq!(o[1], 0.0);
        // Lossless: all the energy comes out.
        let mut h = Harness::new::<Allpass>(json!({}));
        h.set_audio("in", vec![impulse(48_000)])
            .set("delay", 0.0071)
            .set("gain", 0.7);
        let o = h.run(48_000).audio("out")[0].clone();
        let energy: f64 = o.iter().map(|v| f64::from(*v).powi(2)).sum();
        assert!((energy - 1.0).abs() < 1e-4, "{energy}");
    }

    fn magnitude(delay: f64, gain: f64, f: f64) -> f64 {
        let mut h = Harness::new::<Allpass>(json!({}));
        h.set_audio("in", vec![sine(f, 48_000)])
            .set("delay", delay)
            .set("gain", gain);
        let o = h.run(48_000).audio("out")[0].clone();
        tone_magnitude(&o[24_000..], f, RATE)
    }

    #[test]
    fn magnitude_is_unity_at_every_frequency() {
        // Whole-sample delays: exactly allpass.
        for (delay, gain) in [(0.007, 0.7), (0.001, -0.6), (0.002, 0.95)] {
            for f in [50.0, 333.0, 1000.0, 4321.0, 12_000.0, 19_000.0] {
                let m = magnitude(delay, gain, f);
                assert!(
                    (m - 1.0).abs() < 0.01,
                    "delay {delay} gain {gain} f {f}: {m}"
                );
            }
        }
        // Fractional delays (340.8 samples) are read through an allpass
        // interpolator, so they lose nothing either.
        for f in [100.0, 1000.0, 5000.0, 12_000.0, 19_000.0] {
            let m = magnitude(0.0071, 0.7, f);
            assert!((m - 1.0).abs() < 0.01, "f {f}: {m}");
        }
    }

    #[test]
    fn modulated_delay_stays_bounded() {
        let n = 48_000;
        let time: Vec<f32> = (0..n)
            .map(|i| 0.005 + 0.004 * (i as f32 * 0.03).sin())
            .collect();
        let mut h = Harness::new::<Allpass>(json!({}));
        h.set_audio("in", vec![noise(n, 3)])
            .set("gain", 0.99)
            .set_samples("delay", time);
        let o = h.run(n).audio("out")[0].clone();
        assert!(o.iter().all(|v| v.is_finite()));
        assert!(peak(&o) < 20.0, "{}", peak(&o));
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(3_000, 9);
        let time: Vec<f32> = (0..3_000)
            .map(|i| 0.003 + 0.002 * (i as f32 * 0.003).sin())
            .collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<Allpass>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()]).set("gain", 0.8);
            if wired {
                h.set_samples("delay", time.clone());
            }
            h.run(3_000).audio("out")[0].clone()
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let x: Vec<f32> = (0..2_000)
            .map(|i| [1.0, f32::NAN, -0.3, f32::INFINITY][i % 4])
            .collect();
        let mut h = Harness::with_resources::<Allpass>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![x])
            .set_samples("delay", wild.clone())
            .set_samples("gain", wild.iter().rev().copied().collect());
        assert!(h.run(2_000).audio("out")[0].iter().all(|v| v.is_finite()));
    }

    #[test]
    fn stereo_and_extra_channels() {
        let mut h = Harness::new::<Allpass>(json!({ "max_delay": 0.5 }));
        h.set_audio("in", vec![impulse(600), vec![0.0; 600], vec![0.25; 600]])
            .set("delay", 0.01)
            .set("gain", 0.5);
        let out = h.run(600);
        let o = out.audio("out");
        assert_eq!(o[0][480], 0.75);
        assert_eq!(peak(&o[1]), 0.0);
        assert!(o[2].iter().all(|v| *v == 0.25));
        let sig = Allpass::describe(&json!({ "max_delay": 0.5 }));
        assert_eq!(sig.inputs[DELAY].range, Some((0.0, 0.5)));
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut h = Harness::new::<Allpass>(json!({}));
        h.set_audio("in", vec![impulse(1_000)]).set("gain", 0.9);
        h.run(300);
        h.reset();
        assert_eq!(peak(&h.run(700).audio("out")[0]), 0.0);
    }
}
