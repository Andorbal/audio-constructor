use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::delay::DelayLine;

use super::{DelayTimes, line_len, max_delay};

node! {
    kind: "comb",
    name: "Comb",
    category: Filters,
    keywords: [
        "comb filter", "karplus strong", "pluck", "string", "resonator", "flanger",
        "reverb", "metallic", "physical model"
    ],
    /// A comb filter: a copy of the input delayed by `delay`, fed back (a
    /// resonator ringing at 1 / `delay`) or added once (evenly spaced
    /// notches).
    ///
    /// - `mode`: `feedback` (default) recirculates the delayed signal, so an
    ///   impulse comes back every `delay`, scaled by `feedback` each time.
    ///   `feedforward` adds a single delayed copy, scaled by `feedback`, and
    ///   nothing recirculates.
    /// - `delay` (seconds) sets the spacing: peaks (feedback) or notches
    ///   (feedforward) every 1 / `delay` Hz. Reads are interpolated, so it
    ///   can be swept or modulated smoothly; a block-rate `delay` glides
    ///   across each block. It is held between one sample and `max_delay`.
    /// - `feedback` (−0.999 to 0.999, clamped): positive values put peaks at
    ///   multiples of 1 / `delay`; negative values put them halfway between
    ///   and drop the pitch an octave. In feedback mode, peaks reach
    ///   1 / (1 − |`feedback`|), so a steady tone on a peak gets loud at
    ///   high settings.
    /// - `damping` (0 to 1) is a one-pole lowpass inside the loop (on the
    ///   delayed copy in feedforward mode): 0 is off, and it sweeps the
    ///   corner from 20 kHz down to 200 Hz, so highs die away faster than
    ///   lows, like a real string or room.
    /// - `mix` blends the dry input (0) with what comes out of the delay
    ///   line (1). At 1 in feedback mode you hear only the resonance; in
    ///   feedforward mode, 0.5 with `feedback` at ±1 gives the deepest
    ///   notches.
    ///
    /// Config: `max_delay`, the longest delay in seconds (default 0.1, 0.001
    /// to 2), which sizes the buffer. The first two channels are filtered;
    /// further channels pass through dry.
    ///
    /// **Try this:** for a plucked string (Karplus-Strong), gate a short
    /// burst of noise or a bright Oscillator with a `Clock` into a feedback
    /// Comb with `delay` = 1 / pitch (0.00454 s for A 220), `feedback` 0.99,
    /// `damping` 0.3, and `mix` 1.
    pub struct Comb {
        lines: Vec<DelayLine>,
        /// The damping lowpass state per line.
        damp: Vec<f32>,
        times: DelayTimes,
    }
}

const IN: usize = 0;
const MODE: usize = 1;
const DELAY: usize = 2;
const FEEDBACK: usize = 3;
const DAMPING: usize = 4;
const MIX: usize = 5;
const OUT: usize = 0;

const MODES: &[&str] = &["feedback", "feedforward"];

/// Channels with their own delay line; further channels pass dry. The node
/// isn't told its channel count before `process`, and lines can be large.
const LINES: usize = 2;

/// `|feedback|` is kept below 1 so the resonance always dies away.
const MAX_FEEDBACK: f32 = 0.999;

/// Bound on what the loop writes: interpolation never boosts, but
/// audio-rate jumps in `delay` can briefly exceed unity loop gain.
const LIMIT: f32 = 1e9;

/// Damping corner at `damping` 0+ and 1, in hertz.
const DAMP_TOP_HZ: f64 = 20_000.0;
const DAMP_BOTTOM_HZ: f64 = 200.0;
/// Below this much damping the lowpass fades out, so 0 is exactly off.
const DAMP_FADE: f64 = 0.05;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// The damping lowpass pole for `damping` at `rate`: the corner sweeps
/// exponentially from 20 kHz to 200 Hz, the pole being `exp(−2π·fc/rate)`
/// so the corner is the same in hertz at any rate. Over the first 5 % the
/// pole fades to 0 (no filtering).
fn damping_pole(damping: f32, rate: f64) -> f32 {
    let d = f64::from(finite_or(damping, 0.0)).clamp(0.0, 1.0);
    if d <= 0.0 {
        return 0.0;
    }
    let fc = DAMP_TOP_HZ * (DAMP_BOTTOM_HZ / DAMP_TOP_HZ).powf(d);
    let pole = (-std::f64::consts::TAU * fc / rate).exp();
    (pole * (d / DAMP_FADE).min(1.0)) as f32
}

impl Node for Comb {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to filter."))
            .input(
                SocketSpec::choice("mode", MODES, "feedback").doc("Recirculate, or add one copy."),
            )
            .input(
                SocketSpec::float("delay", Unit::Seconds, 0.005)
                    .range(0.0, max_delay(config))
                    .doc("Delay time; the comb's spacing is 1 / delay."),
            )
            .input(
                SocketSpec::float("feedback", Unit::Generic, 0.7)
                    .range(-0.999, 0.999)
                    .doc("Gain of the delayed copy."),
            )
            .input(
                SocketSpec::float("damping", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Lowpass in the loop: highs die away faster."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("0 is dry, 1 is only the delay line's output."),
            )
            .output(SocketSpec::audio("out").doc("The filtered signal."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let len = line_len(config, ctx.sample_rate);
        Comb {
            lines: (0..LINES).map(|_| DelayLine::new(len)).collect(),
            damp: vec![0.0; LINES],
            times: DelayTimes::new(ctx.max_block),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let frames = io.frames();
        self.times.fill(io.inputs.float(DELAY), rate, frames);
        let feedforward = io.inputs.choice(MODE, MODES.len()) == 1;
        let input = io.inputs.audio(IN);
        let feedback = io.inputs.float(FEEDBACK);
        let damping = io.inputs.float(DAMPING);
        let mix = io.inputs.float(MIX);
        // Steady values (the usual case) are worked out once.
        let fb0 = finite_or(feedback.first(), 0.0).clamp(-MAX_FEEDBACK, MAX_FEEDBACK);
        let p0 = damping_pole(damping.first(), rate);
        let m0 = finite_or(mix.first(), 0.5).clamp(0.0, 1.0);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            let (Some(line), Some(lp)) = (self.lines.get_mut(c), self.damp.get_mut(c)) else {
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = *s;
                }
                continue;
            };
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let fb = if feedback.is_varying() {
                    finite_or(feedback.at(i), 0.0).clamp(-MAX_FEEDBACK, MAX_FEEDBACK)
                } else {
                    fb0
                };
                let p = if damping.is_varying() {
                    damping_pole(damping.at(i), rate)
                } else {
                    p0
                };
                let m = if mix.is_varying() {
                    finite_or(mix.at(i), 0.5).clamp(0.0, 1.0)
                } else {
                    m0
                };
                let x = finite_or(x, 0.0);
                let delayed = line.read(self.times.at(i));
                // One-pole lowpass: y = (1 − p)·x + p·y[n−1].
                *lp = delayed + p * (*lp - delayed);
                let wet = if feedforward {
                    line.write(x);
                    fb * *lp
                } else {
                    let back = (x + fb * *lp).clamp(-LIMIT, LIMIT);
                    line.write(back);
                    delayed
                };
                *o = x * (1.0 - m) + wet * m;
            }
            if lp.abs() < 1e-30 {
                *lp = 0.0;
            }
        }
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        self.damp.fill(0.0);
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

    fn wet(mode: &str, delay: f64, feedback: f64, damping: f64, x: Vec<f32>) -> Vec<f32> {
        let n = x.len();
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![x])
            .set("mode", mode)
            .set("delay", delay)
            .set("feedback", feedback)
            .set("damping", damping)
            .set("mix", 1.0);
        h.run(n).audio("out")[0].clone()
    }

    #[test]
    fn feedback_mode_repeats_every_delay() {
        let o = wet("feedback", 0.001, 0.5, 0.0, impulse(400));
        assert_eq!(o[0], 0.0);
        assert_eq!(o[48], 1.0);
        assert_eq!(o[96], 0.5);
        assert_eq!(o[144], 0.25);
        assert_eq!(o[145], 0.0);
        // Negative feedback alternates the sign.
        let o = wet("feedback", 0.001, -0.5, 0.0, impulse(400));
        assert_eq!((o[48], o[96], o[144]), (1.0, -0.5, 0.25));
    }

    #[test]
    fn feedforward_mode_adds_one_scaled_copy() {
        let o = wet("feedforward", 0.001, 0.8, 0.0, impulse(400));
        assert_eq!(o[48], 0.8);
        assert_eq!(peak(&o[49..]), 0.0);
        // Mix 0.5 with feedback 1: notches at odd multiples of 1 / (2·delay).
        let tone = |f: f64| -> f64 {
            let x: Vec<f32> = (0..9600)
                .map(|i| (std::f64::consts::TAU * f * i as f64 / RATE).sin() as f32)
                .collect();
            let mut h = Harness::new::<Comb>(json!({}));
            h.set_audio("in", vec![x])
                .set("mode", "feedforward")
                .set("delay", 0.001)
                .set("feedback", 1.0)
                .set("mix", 0.5);
            tone_magnitude(&h.run(9600).audio("out")[0][480..], f, RATE)
        };
        assert!(tone(500.0) < 1e-3, "{}", tone(500.0));
        assert!((tone(1000.0) - 0.999).abs() < 0.01, "{}", tone(1000.0));
    }

    #[test]
    fn karplus_strong_burst_rings_at_one_over_delay() {
        let n = 48_000;
        let mut x = noise(n, 7);
        for v in &mut x[480..] {
            *v = 0.0;
        }
        let delay = 1.0 / 220.0;
        let o = wet("feedback", delay, 0.995, 0.2, x);
        let seg = &o[9_600..43_200];
        // The strongest frequency between 150 and 300 Hz.
        let (best, _) = (0..=300)
            .map(|k| 150.0 + k as f64 * 0.5)
            .map(|f| (f, tone_magnitude(seg, f, RATE)))
            .fold((0.0, 0.0), |b, (f, m)| if m > b.1 { (f, m) } else { b });
        assert!((best - 220.0).abs() < 2.0, "{best}");
        // It decays but is still sounding after half a second.
        assert!(peak(&o[24_000..26_400]) > 0.01);
        assert!(peak(&o[45_000..]) < peak(&o[2_000..4_400]));
    }

    #[test]
    fn damping_darkens_the_repeats() {
        let bright = wet("feedback", 0.002, 0.9, 0.0, noise(24_000, 3));
        let dark = wet("feedback", 0.002, 0.9, 0.8, noise(24_000, 3));
        let hf = |o: &[f32]| tone_magnitude(&o[4_800..], 9_000.0, RATE);
        assert!(
            hf(&dark) < 0.3 * hf(&bright),
            "{} {}",
            hf(&dark),
            hf(&bright)
        );
        assert_eq!(damping_pole(0.0, RATE), 0.0);
        assert!(damping_pole(0.01, RATE) < 0.03);
        let top = damping_pole(1.0, RATE);
        assert!((top - (-std::f64::consts::TAU * 200.0 / RATE).exp() as f32).abs() < 1e-6);
        // The same corner in hertz at another rate.
        let at96 = damping_pole(1.0, 96_000.0);
        assert!((f64::from(at96).ln() * 96_000.0 - f64::from(top).ln() * RATE).abs() < 1.0);
    }

    #[test]
    fn mix_zero_is_dry_and_max_delay_limits() {
        let x = noise(500, 11);
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![x.clone()]).set("mix", 0.0);
        assert_eq!(h.run(500).audio("out")[0], x);
        let sig = Comb::describe(&json!({ "max_delay": 9.0 }));
        assert_eq!(sig.inputs[DELAY].range, Some((0.0, 2.0)));
        let mut h = Harness::new::<Comb>(json!({ "max_delay": 0.001 }));
        h.set_audio("in", vec![impulse(200)])
            .set("delay", 1.0)
            .set("feedback", 0.0)
            .set("mix", 1.0);
        assert_eq!(h.run(200).audio("out")[0][48], 1.0);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(3_000, 5);
        let time: Vec<f32> = (0..3_000)
            .map(|i| 0.003 + 0.002 * (i as f32 * 0.003).sin())
            .collect();
        let damp: Vec<f32> = (0..3_000).map(|i| (i % 100) as f32 / 100.0).collect();
        for mode in MODES {
            let render = |block: usize, wired: bool| {
                let mut h = Harness::with_resources::<Comb>(json!({}), RATE, block, &NoResources);
                h.set_audio("in", vec![x.clone()])
                    .set("mode", *mode)
                    .set("feedback", 0.9);
                if wired {
                    h.set_samples("delay", time.clone())
                        .set_samples("damping", damp.clone());
                }
                h.run(3_000).audio("out")[0].clone()
            };
            for wired in [false, true] {
                let one = render(1, wired);
                assert_eq!(one, render(64, wired), "{mode}");
                assert_eq!(one, render(512, wired), "{mode}");
            }
        }
    }

    #[test]
    fn modulated_delay_stays_bounded_and_smooth() {
        let n = 48_000;
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 150.0 * i as f32 / 48_000.0).sin() * 0.1)
            .collect();
        // A fast audio-rate wobble with near-unity feedback.
        let time: Vec<f32> = (0..n)
            .map(|i| 0.004 + 0.003 * (i as f32 * 0.02).sin())
            .collect();
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![x.clone()])
            .set("feedback", -0.999)
            .set("mix", 1.0)
            .set_samples("delay", time);
        let o = h.run(n).audio("out")[0].clone();
        assert!(o.iter().all(|v| v.is_finite()));
        assert!(peak(&o) < 1e3, "{}", peak(&o));
        // A slow sweep without feedback has no steps.
        let slow: Vec<f32> = (0..n)
            .map(|i| 0.002 + 0.004 * i as f32 / n as f32)
            .collect();
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![x])
            .set("feedback", 0.0)
            .set("mode", "feedforward")
            .set("mix", 0.5)
            .set_samples("delay", slow);
        let o = h.run(n).audio("out")[0].clone();
        let step = o
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(step < 0.003, "{step}");
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let x: Vec<f32> = (0..2_000)
            .map(|i| [1.0, f32::NAN, -0.3, f32::INFINITY][i % 4])
            .collect();
        for mode in MODES {
            let mut h = Harness::with_resources::<Comb>(json!({}), 44_100.0, 1, &NoResources);
            h.set_audio("in", vec![x.clone()])
                .set("mode", *mode)
                .set_samples("delay", wild.clone())
                .set_samples("feedback", wild.iter().rev().copied().collect())
                .set_samples("damping", wild.clone())
                .set_samples("mix", wild.clone());
            let out = h.run(2_000);
            assert!(out.audio("out")[0].iter().all(|v| v.is_finite()), "{mode}");
        }
    }

    #[test]
    fn stereo_lines_are_independent_and_extra_channels_pass_dry() {
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![impulse(500), vec![0.0; 500], vec![0.5; 500]])
            .set("delay", 0.001)
            .set("mix", 1.0);
        let out = h.run(500);
        let o = out.audio("out");
        assert_eq!(o[0][48], 1.0);
        assert_eq!(peak(&o[1]), 0.0);
        assert!(o[2].iter().all(|v| *v == 0.5));
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut h = Harness::new::<Comb>(json!({}));
        h.set_audio("in", vec![impulse(1_000)])
            .set("feedback", 0.99)
            .set("mix", 1.0);
        h.run(300);
        h.reset();
        assert_eq!(peak(&h.run(700).audio("out")[0]), 0.0);
    }
}
