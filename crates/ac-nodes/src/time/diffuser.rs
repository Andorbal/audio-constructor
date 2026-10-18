use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::allpass::Allpass;

node! {
    kind: "diffuser",
    name: "Diffuser",
    category: Time,
    keywords: [
        "reverb", "diffusion", "allpass", "smear", "wash", "space", "early reflections",
        "stereo", "decorrelate", "ambience"
    ],
    /// Smears a signal into a dense wash: eight modulated allpass filters
    /// in series per channel, the building block of a reverb.
    ///
    /// An impulse comes out as a cloud of echoes spread over tens of
    /// milliseconds, with nothing lost: the allpasses pass every frequency
    /// at full level, so the long-term level matches the input and the
    /// Diffuser never rings on or builds up by itself.
    ///
    /// - `size` (0 to 1) scales every delay: at 1 the chain spans about
    ///   60 ms per channel, at 0 it collapses to a few samples (almost no
    ///   effect). Changes are smoothed over about 20 ms, so moving it bends
    ///   the pitch of the tail slightly instead of crackling.
    /// - `diffusion` (0 to 0.9, clamped) is the allpass gain: higher values
    ///   give a smoother, longer smear; 0 just delays.
    /// - `mod_depth` (0 to 1) wobbles each delay by up to ±1 ms, each at
    ///   its own phase, which breaks up metallic ringing; a little goes a
    ///   long way. `mod_rate` (hertz) sets the wobble speed.
    /// - `mix` blends the dry input (0) with the diffused signal (1).
    ///
    /// The output is stereo: the left and right chains use different,
    /// mutually prime delays, so a mono input comes out wide and the two
    /// sides are decorrelated. A stereo input keeps its sides. Delays that
    /// fall between samples (any `size` but 1, and all modulation) are read
    /// through allpass interpolators, so even then nothing is lost.
    ///
    /// **Try this:** put a Diffuser before a `Delay` with high feedback and
    /// a little `damping`-style lowpass (a `Biquad`) in its path for a quick
    /// reverb, or after a snare with `size` 0.3 and `mix` 0.4 to thicken it.
    pub struct Diffuser {
        /// Stage `s` of channel `c` is `stages[c * STAGES + s]`.
        stages: Vec<Allpass>,
        /// Each stage's delay at `size` 1, in samples at the running rate.
        base: [f64; CHANNELS * STAGES],
        /// Each stage's modulation phase offset as (cos, sin).
        offsets: [(f64, f64); CHANNELS * STAGES],
        /// Modulation excursion at `mod_depth` 1, in samples.
        excursion: f64,
        /// Modulation phase in cycles, in [0, 1).
        phase: f64,
        /// The smoothed `size` (None until the first block).
        size: Option<f64>,
        /// One-pole smoothing coefficient for `size` per sample.
        size_coef: f64,
        /// Per-frame values shared by both channels.
        sizes: Vec<f64>,
        /// Per-frame modulation, as excursion-scaled (sin, cos) of the phase.
        lfo: Vec<(f64, f64)>,
        gains: Vec<f32>,
        mixes: Vec<f32>,
        /// One channel's block as it passes through the stages.
        work: Vec<f32>,
    }
}

const IN: usize = 0;
const SIZE: usize = 1;
const DIFFUSION: usize = 2;
const MOD_DEPTH: usize = 3;
const MOD_RATE: usize = 4;
const MIX: usize = 5;
const OUT: usize = 0;

const CHANNELS: usize = 2;
const STAGES: usize = 8;

/// Stage delays at `size` 1, in samples at 48 kHz: primes (so no two
/// stages' echoes line up), spread roughly geometrically from 1.3 to 19 ms,
/// different for each channel. Converted to seconds, so the timing is the
/// same at any rate. Delay sets in the spirit of J. Dattorro, "Effect
/// Design, Part 1: Reverberator and Other Filters" (JAES 45(9), 1997),
/// whose input diffusers are four allpasses of a few ms with gains of
/// 0.625 to 0.75.
const PRIMES_48K: [[u32; STAGES]; CHANNELS] = [
    [61, 103, 167, 241, 347, 461, 641, 863],
    [71, 113, 179, 257, 359, 487, 673, 911],
];
const REFERENCE_RATE: f64 = 48_000.0;

/// Modulation excursion at full depth, in seconds.
const EXCURSION: f64 = 0.001;
/// The allpass gain is kept at or below this.
const MAX_DIFFUSION: f32 = 0.9;
/// Modulation rates are kept within this range, in hertz.
const RATE_RANGE: (f64, f64) = (0.0, 20.0);
/// Time constant of the `size` smoothing, in seconds.
const SIZE_SMOOTHING: f64 = 0.02;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Diffuser {
    /// Fill the per-frame tables shared by both channels.
    fn fill_frames(&mut self, io: &Io, rate: f64, frames: usize) {
        let size = io.inputs.float(SIZE);
        let diffusion = io.inputs.float(DIFFUSION);
        let depth = io.inputs.float(MOD_DEPTH);
        let mod_rate = io.inputs.float(MOD_RATE);
        let mix = io.inputs.float(MIX);
        // `size` moves every delay at once, the longest by tens of samples
        // per step of the control, so it is smoothed per sample: a jump
        // becomes a gentle pitch bend instead of a scrub.
        for (i, s) in self.sizes.iter_mut().take(frames).enumerate() {
            let target = f64::from(finite_or(size.at(i), 0.5)).clamp(0.0, 1.0);
            let current = self.size.get_or_insert(target);
            *current += (target - *current) * self.size_coef;
            *s = *current;
        }
        for (i, (((l, g), m), _)) in self
            .lfo
            .iter_mut()
            .zip(self.gains.iter_mut())
            .zip(self.mixes.iter_mut())
            .zip(0..frames)
            .enumerate()
        {
            let d = f64::from(finite_or(depth.at(i), 0.0)).clamp(0.0, 1.0) * self.excursion;
            let (s, c) = (std::f64::consts::TAU * self.phase).sin_cos();
            *l = (d * s, d * c);
            let r = f64::from(finite_or(mod_rate.at(i), 0.0)).clamp(RATE_RANGE.0, RATE_RANGE.1);
            self.phase += r / rate;
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
            }
            *g = finite_or(diffusion.at(i), 0.0).clamp(0.0, MAX_DIFFUSION);
            *m = finite_or(mix.at(i), 1.0).clamp(0.0, 1.0);
        }
    }
}

impl Node for Diffuser {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(Channels::UpTo(2))
                    .doc("Mono or stereo signal to diffuse."),
            )
            .input(
                SocketSpec::float("size", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Scales every delay; 1 spans about 60 ms."),
            )
            .input(
                SocketSpec::float("diffusion", Unit::Generic, 0.7)
                    .range(0.0, 0.9)
                    .doc("Allpass gain: how smooth and long the smear is."),
            )
            .input(
                SocketSpec::float("mod_depth", Unit::Generic, 0.3)
                    .range(0.0, 1.0)
                    .doc("Delay wobble, up to ±1 ms."),
            )
            .input(
                SocketSpec::float("mod_rate", Unit::Hz, 0.5)
                    .range(0.05, 5.0)
                    .log()
                    .doc("Wobble speed."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("0 is dry, 1 is fully diffused."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(2))
                    .doc("The diffused signal, in stereo."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            REFERENCE_RATE
        };
        let excursion = EXCURSION * rate;
        let mut base = [0.0; CHANNELS * STAGES];
        let mut offsets = [(1.0, 0.0); CHANNELS * STAGES];
        for (c, primes) in PRIMES_48K.iter().enumerate() {
            for (s, &p) in primes.iter().enumerate() {
                let k = c * STAGES + s;
                base[k] = f64::from(p) / REFERENCE_RATE * rate;
                // Phases spread evenly over the stages, the right channel's
                // halfway between the left's.
                let phi = std::f64::consts::TAU * (s as f64 + 0.5 * c as f64) / STAGES as f64;
                offsets[k] = (phi.cos(), phi.sin());
            }
        }
        let block = ctx.max_block.max(1);
        Diffuser {
            stages: base
                .iter()
                .map(|b| Allpass::new((b + excursion).ceil() as usize + 4))
                .collect(),
            base,
            offsets,
            excursion,
            phase: 0.0,
            size: None,
            size_coef: 1.0 - (-1.0 / (SIZE_SMOOTHING * rate)).exp(),
            sizes: vec![0.5; block],
            lfo: vec![(0.0, 0.0); block],
            gains: vec![0.0; block],
            mixes: vec![1.0; block],
            work: vec![0.0; block],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        self.fill_frames(io, ctx.sample_rate, frames);
        let input = io.inputs.audio(IN);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels().min(CHANNELS) {
            let src = input.channel(c);
            let dst = out.channel(c);
            let first = c * STAGES;
            let (Some(stages), Some(base), Some(offsets)) = (
                self.stages.get_mut(first..first + STAGES),
                self.base.get(first..first + STAGES),
                self.offsets.get(first..first + STAGES),
            ) else {
                continue;
            };
            for (w, &x) in self.work.iter_mut().zip(src) {
                *w = finite_or(x, 0.0);
            }
            // The stages are in series with no feedback between them, so
            // each runs over the whole block in turn: the same arithmetic as
            // sample by sample, with one delay line in cache at a time.
            for ((ap, &b), &(oc, os)) in stages.iter_mut().zip(base).zip(offsets) {
                for (((w, &scale), &(ls, lc)), &g) in self
                    .work
                    .iter_mut()
                    .zip(&self.sizes)
                    .zip(&self.lfo)
                    .zip(&self.gains)
                    .take(frames)
                {
                    // sin(θ + φ) = sin θ·cos φ + cos θ·sin φ.
                    let d = b * scale + ls * oc + lc * os;
                    *w = ap.tick(*w, d, g);
                }
            }
            for (((o, &x), &y), &m) in dst.iter_mut().zip(src).zip(&self.work).zip(&self.mixes) {
                *o = finite_or(x, 0.0) * (1.0 - m) + y * m;
            }
        }
    }

    fn reset(&mut self) {
        for ap in &mut self.stages {
            ap.clear();
        }
        self.phase = 0.0;
        self.size = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms};
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

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|v| f64::from(*v).powi(2)).sum()
    }

    fn impulse_response(size: f64, depth: f64) -> Vec<Vec<f32>> {
        let n = 96_000;
        let mut h = Harness::new::<Diffuser>(json!({}));
        h.set_audio("in", vec![impulse(n)])
            .set("size", size)
            .set("diffusion", 0.7)
            .set("mod_depth", depth);
        h.run(n).audio("out").to_vec()
    }

    /// Samples until 90 % of the energy has come out.
    fn t90(x: &[f32]) -> usize {
        let e = energy(x);
        let mut acc = 0.0;
        x.iter()
            .position(|v| {
                acc += f64::from(*v).powi(2);
                acc > 0.9 * e
            })
            .unwrap_or(0)
    }

    #[test]
    fn an_impulse_is_smeared_over_tens_of_ms_and_nothing_is_lost() {
        let out = impulse_response(1.0, 0.0);
        for ch in &out {
            let e = energy(ch);
            assert!((e - 1.0).abs() < 1e-3, "{e}");
            assert!(peak(ch) < 0.5, "{}", peak(ch));
            // Under half the energy in the first 5 ms; 90 % takes over 20 ms.
            let early = energy(&ch[..240]);
            assert!(early < 0.5, "{early}");
            assert!(t90(ch) > 960, "{}", t90(ch));
        }
        // Fractional and modulated delays lose nothing either.
        for (size, depth) in [(0.537, 0.0), (0.8, 1.0)] {
            for ch in impulse_response(size, depth) {
                let e = energy(&ch);
                // Modulation makes the chain time-varying, so energy is
                // conserved only approximately there.
                assert!((e - 1.0).abs() < 1e-2, "{size} {depth}: {e}");
            }
        }
        // Size scales the smear.
        let small = impulse_response(0.2, 0.0);
        let ratio = t90(&small[0]) as f64 / t90(&out[0]) as f64;
        assert!(ratio > 0.1 && ratio < 0.35, "{ratio}");
    }

    #[test]
    fn a_mono_input_comes_out_decorrelated() {
        let out = impulse_response(1.0, 0.3);
        let (l, r) = (&out[0], &out[1]);
        let dot: f64 = l
            .iter()
            .zip(r)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum();
        let corr = dot / (energy(l) * energy(r)).sqrt();
        assert!(corr.abs() < 0.3, "{corr}");
    }

    #[test]
    fn long_term_level_matches_the_input() {
        let n = 240_000;
        let x = noise(n, 5);
        let mut h = Harness::new::<Diffuser>(json!({}));
        h.set_audio("in", vec![x.clone(), noise(n, 6)])
            .set("size", 0.8)
            .set("diffusion", 0.9)
            .set("mod_depth", 1.0)
            .set("mod_rate", 3.0);
        let out = h.run(n);
        for c in out.audio("out") {
            assert!(c.iter().all(|v| v.is_finite()));
            let ratio = rms(&c[48_000..]) / rms(&x[48_000..]);
            assert!(ratio > 0.85 && ratio < 1.05, "{ratio}");
        }
    }

    #[test]
    fn mix_zero_is_dry_in_both_channels() {
        let x = noise(1_000, 3);
        let mut h = Harness::new::<Diffuser>(json!({}));
        h.set_audio("in", vec![x.clone()]).set("mix", 0.0);
        let out = h.run(1_000);
        assert_eq!(out.audio("out")[0], x);
        assert_eq!(out.audio("out")[1], x);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(4_000, 9);
        let size: Vec<f32> = (0..4_000)
            .map(|i| 0.5 + 0.4 * (i as f32 * 0.002).sin())
            .collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<Diffuser>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()])
                .set("mod_depth", 0.8)
                .set("mod_rate", 2.0);
            if wired {
                h.set_samples("size", size.clone());
            }
            h.run(4_000).audio("out").to_vec()
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn size_changes_glide_without_clicks() {
        let n = 48_000;
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 200.0 * i as f32 / 48_000.0).sin())
            .collect();
        let steps: Vec<f32> = (0..n).map(|i| if i < n / 2 { 0.3 } else { 0.9 }).collect();
        let mut h = Harness::new::<Diffuser>(json!({}));
        h.set_audio("in", vec![x])
            .set("mod_depth", 0.0)
            .set_block("size", steps);
        let out = h.run(n);
        let o = &out.audio("out")[0];
        let jump = o[n / 2 - 64..n / 2 + 128]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        // The sine's own steepest step is 0.026.
        assert!(jump < 0.08, "{jump}");
    }

    #[test]
    fn extreme_parameters_stay_finite_and_reset_clears() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let x: Vec<f32> = (0..2_000)
            .map(|i| [1.0, f32::NAN, -0.3, f32::INFINITY][i % 4])
            .collect();
        let mut h = Harness::with_resources::<Diffuser>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![x, vec![0.5; 2_000]])
            .set_samples("size", wild.clone())
            .set_samples("diffusion", wild.iter().rev().copied().collect())
            .set_samples("mod_depth", wild.clone())
            .set_samples("mod_rate", wild.clone())
            .set_samples("mix", wild);
        let out = h.run(2_000);
        for c in out.audio("out") {
            assert!(c.iter().all(|v| v.is_finite()));
        }
        h.reset();
        h.set_audio("in", vec![vec![0.0; 3_000]]);
        let out = h.run(500);
        assert_eq!(peak(&out.audio("out")[0]), 0.0);
        assert_eq!(peak(&out.audio("out")[1]), 0.0);
    }
}
