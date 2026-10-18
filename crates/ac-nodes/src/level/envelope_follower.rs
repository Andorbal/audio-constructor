use ac_core::{
    Config, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit,
    node,
};
use ac_dsp::dynamics::{Detector, time_coef};

node! {
    kind: "envelope_follower",
    name: "Envelope Follower",
    category: Level,
    keywords: ["detector", "level", "envelope", "sidechain", "peak", "rms", "amplitude", "follower"],
    /// Measures how loud a signal is, moment to moment: `level` (linear,
    /// 0 to 1 for full scale) and `level_db` (decibels, never below −120),
    /// both at sample rate.
    ///
    /// - `mode` **peak** follows the signal's peaks; **rms** follows its
    ///   power (root mean square), which tracks perceived loudness more
    ///   closely and reads 3 dB below the peak for a sine.
    /// - `attack` is how quickly the level rises when the signal gets
    ///   louder, `release` how quickly it falls when it gets quieter, both
    ///   in seconds: the time to cover about 63 % of a jump. 0 follows
    ///   instantly.
    ///
    /// Multichannel input is linked: the level follows the loudest
    /// channel, so a stereo compressor built from it turns both sides down
    /// together and the image doesn't shift.
    ///
    /// The detector is the smooth branching peak detector from Giannoulis,
    /// Massberg & Reiss's compressor design tutorial (JAES 2012); in rms
    /// mode it smooths the squared signal.
    ///
    /// **Try this:** wire `level_db` into a `Gain Computer` and its
    /// `gain_reduction` into a `Gain` on the same signal for a compressor,
    /// or feed a drum loop in and wire `level` through a `Map` into a
    /// `Biquad`'s `cutoff` for an envelope-following filter.
    pub struct EnvelopeFollower {
        detector: Detector,
        /// Loudest channel's |x| (or x²) for each frame of the block.
        scratch: Vec<f64>,
    }
}

const IN: usize = 0;
const MODE: usize = 1;
const ATTACK: usize = 2;
const RELEASE: usize = 3;

const LEVEL: usize = 0;
const LEVEL_DB: usize = 1;

const MODES: &[&str] = &["peak", "rms"];
/// Inputs are held below this (+180 dB) so the detector stays finite.
const LIMIT: f64 = 1e9;

#[inline]
fn coef(t: FloatIn, i: usize, rate: f64) -> f64 {
    time_coef(f64::from(t.at(i)), rate)
}

#[inline]
fn to_db(level: f64) -> f32 {
    if level <= 1e-6 {
        -120.0
    } else {
        (20.0 * level.log10()) as f32
    }
}

impl Node for EnvelopeFollower {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to measure."))
            .input(SocketSpec::choice("mode", MODES, "peak").doc("Follow peaks or power (RMS)."))
            .input(
                SocketSpec::float("attack", Unit::Seconds, 0.01)
                    .range(0.0, 0.5)
                    .doc("How fast the level rises."),
            )
            .input(
                SocketSpec::float("release", Unit::Seconds, 0.1)
                    .range(0.0, 2.0)
                    .doc("How fast the level falls."),
            )
            .output(
                SocketSpec::float("level", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("Level, linear (1 is full scale)."),
            )
            .output(
                SocketSpec::float("level_db", Unit::Db, -120.0)
                    .rate(Rate::Sample)
                    .doc("Level in decibels (floor −120)."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        EnvelopeFollower {
            detector: Detector::new(),
            scratch: vec![0.0; ctx.max_block.max(1)],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames().min(self.scratch.len());
        let rms = io.inputs.choice(MODE, MODES.len()) == 1;
        let input = io.inputs.audio(IN);
        let scratch = &mut self.scratch[..frames];
        scratch.fill(0.0);
        for c in 0..input.channels() {
            for (s, x) in scratch.iter_mut().zip(input.channel(c)) {
                let x = f64::from(x.abs());
                let x = if x.is_finite() { x.min(LIMIT) } else { 0.0 };
                *s = s.max(if rms { x * x } else { x });
            }
        }
        let attack = io.inputs.float(ATTACK);
        let release = io.inputs.float(RELEASE);
        let varying = attack.is_varying() || release.is_varying();
        let (mut a, mut r) = (
            coef(attack, 0, ctx.sample_rate),
            coef(release, 0, ctx.sample_rate),
        );
        let (mut level, mut level_db) = io.outputs.float2(LEVEL, LEVEL_DB);
        for (i, &x) in scratch.iter().enumerate() {
            if varying {
                a = coef(attack, i, ctx.sample_rate);
                r = coef(release, i, ctx.sample_rate);
            }
            let y = self.detector.tick(x, a, r);
            let v = if rms { y.sqrt() } else { y };
            level.set(i, v as f32);
            level_db.set(i, to_db(v));
        }
    }

    fn reset(&mut self) {
        self.detector.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<EnvelopeFollower>(json!({}), 48_000.0, block, &NoResources)
    }

    fn step(n: usize, at: usize, lo: f32, hi: f32) -> Vec<f32> {
        (0..n).map(|i| if i < at { lo } else { hi }).collect()
    }

    #[test]
    fn attack_and_release_times() {
        let mut h = harness(64);
        h.set_audio("in", vec![step(10_000, 0, 0.0, 1.0)])
            .set("attack", 0.01)
            .set("release", 0.1);
        let out = h.run(10_000);
        let l = out.float("level");
        // One time constant (480 samples) covers 63 % of the way up.
        assert!((l[479] - 0.632).abs() < 0.01, "{}", l[479]);
        assert!((l[9_999] - 1.0).abs() < 1e-6);
        assert!(out.float("level_db")[9_999].abs() < 1e-4);
        let mut h = harness(64);
        h.set_audio("in", vec![step(20_000, 10_000, 1.0, 0.0)])
            .set("attack", 0.0)
            .set("release", 0.1);
        let l = h.run(20_000).float("level").to_vec();
        assert_eq!(l[0], 1.0, "zero attack is instant");
        assert!((l[10_000 + 4_799] - 0.368).abs() < 0.01, "{}", l[14_799]);
    }

    #[test]
    fn peak_and_rms_of_a_sine() {
        let sine: Vec<f32> = (0..48_000)
            .map(|i| (std::f32::consts::TAU * 100.0 * i as f32 / 48_000.0).sin())
            .collect();
        let settle = |mode: &str| {
            let mut h = harness(64);
            h.set_audio("in", vec![sine.clone()])
                .set("mode", mode)
                .set("attack", 0.0005)
                .set("release", 0.5);
            let l = h.run(48_000).float("level").to_vec();
            let tail = &l[24_000..];
            tail.iter().sum::<f32>() / tail.len() as f32
        };
        assert!((settle("peak") - 1.0).abs() < 0.02);
        let mut h = harness(64);
        h.set_audio("in", vec![sine])
            .set("mode", "rms")
            .set("attack", 0.2)
            .set("release", 0.2);
        let l = h.run(48_000).float("level").to_vec();
        let tail = &l[36_000..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(
            (mean - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02,
            "{mean}"
        );
    }

    #[test]
    fn channels_are_linked_and_silence_floors() {
        let mut h = harness(64);
        h.set_audio("in", vec![vec![0.1; 2_000], vec![-0.5; 2_000]])
            .set("attack", 0.0);
        let out = h.run(2_000);
        assert_eq!(out.float("level")[1_000], 0.5);
        let mut h = harness(64);
        h.set_audio("in", vec![vec![0.0; 256]]);
        let out = h.run(256);
        assert!(out.float("level").iter().all(|&v| v == 0.0));
        assert!(out.float("level_db").iter().all(|&v| v == -120.0));
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let x: Vec<f32> = (0..5_000)
            .map(|i| (i as f32 * 0.01).sin() * (i as f32 * 0.0007).sin())
            .collect();
        let attack: Vec<f32> = (0..5_000)
            .map(|i| 0.001 + (i % 700) as f32 * 1e-5)
            .collect();
        for mode in MODES {
            let render = |block: usize| {
                let mut h = harness(block);
                h.set_audio("in", vec![x.clone(), x.iter().map(|v| v * 0.3).collect()])
                    .set("mode", *mode)
                    .set_samples("attack", attack.clone());
                let out = h.run(5_000);
                (out.float("level").to_vec(), out.float("level_db").to_vec())
            };
            let one = render(1);
            assert_eq!(one, render(64), "{mode}");
            assert_eq!(one, render(512), "{mode}");
        }
    }

    #[test]
    fn extreme_inputs_stay_finite_and_reset_clears() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(600).copied().collect();
        for mode in MODES {
            let mut h = harness(64);
            h.set_audio("in", vec![wild.clone()])
                .set("mode", *mode)
                .set_samples("attack", wild.clone())
                .set_samples("release", wild.iter().rev().copied().collect());
            let out = h.run(600);
            assert!(out.float("level").iter().all(|v| v.is_finite()), "{mode}");
            assert!(
                out.float("level_db").iter().all(|v| v.is_finite()),
                "{mode}"
            );
        }
        let mut h = harness(64);
        h.set_audio("in", vec![vec![1.0; 64]]);
        h.run(64);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 128]]);
        assert_eq!(h.run(64).float("level")[0], 0.0);
    }
}
