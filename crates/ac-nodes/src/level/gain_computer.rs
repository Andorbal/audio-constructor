use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::dynamics::{Mode, gain_change_db};

node! {
    kind: "gain_computer",
    name: "Gain Computer",
    category: Level,
    keywords: ["compressor", "limiter", "expander", "gate", "dynamics", "ratio", "threshold", "knee", "sidechain"],
    /// Turns a level into a gain change: the decision-making half of a
    /// compressor, limiter, expander, or gate.
    ///
    /// Wire a level in decibels into `level` (usually an `Envelope
    /// Follower`'s `level_db`). Out come `gain_reduction`, the change in
    /// decibels (0, or negative when turning down), and `gain`, the same
    /// as a linear factor (1 for no change) to multiply a signal by.
    ///
    /// - `mode` **compress**: levels above `threshold` are turned down so
    ///   that every `ratio` dB over the threshold comes out as 1 dB over
    ///   (4 means 4:1). Very high ratios (20 and up) make a limiter.
    /// - `mode` **expand**: levels below `threshold` are turned down
    ///   further, every 1 dB under becoming `ratio` dB under: a downward
    ///   expander, and a gate at high ratios.
    /// - `knee` (dB) rounds the corner at the threshold over that width so
    ///   the change sets in gradually; 0 is a hard knee.
    ///
    /// `ratio` is held between 1 (no change) and 1000. `gain_reduction`
    /// stops at −120 dB, where `gain` is exactly 0. The outputs run at the
    /// rate of the fastest input, so a sample-rate level gives a
    /// sample-rate gain. The curve is the soft-knee gain computer from
    /// Giannoulis, Massberg & Reiss's compressor design tutorial (JAES
    /// 2012), mirrored below the threshold for expansion.
    ///
    /// The computer has no attack or release of its own: that's the
    /// follower's job. Detecting a different signal from the one being
    /// turned down makes a sidechain compressor (a kick drum ducking a
    /// bass line).
    ///
    /// **Try this:** chain Envelope Follower `level_db` → Gain Computer
    /// `level`, and `gain_reduction` → a `Gain`'s `gain` on the same audio
    /// for a compressor; add a `Lookahead` in front (follower on `early`,
    /// Gain on `out`) with ratio 100 for a limiter.
    pub struct GainComputer {}
}

const LEVEL: usize = 0;
const THRESHOLD: usize = 1;
const RATIO: usize = 2;
const KNEE: usize = 3;
const MODE: usize = 4;

const GAIN_REDUCTION: usize = 0;
const GAIN: usize = 1;

/// Floor for `gain_reduction`; `gain` is 0 there.
const FLOOR_DB: f64 = -120.0;
const MAX_RATIO: f32 = 1000.0;

/// `(gain_reduction_db, gain)` for one frame's inputs.
#[inline]
fn compute(level: f32, threshold: f32, ratio: f32, knee: f32, mode: Mode) -> (f32, f32) {
    let level = if level.is_nan() {
        FLOOR_DB as f32
    } else {
        level.clamp(-1e4, 1e4)
    };
    let threshold = if threshold.is_finite() {
        threshold.clamp(-1e4, 1e4)
    } else {
        -20.0
    };
    // +inf means an infinite ratio: held at the top of the range.
    let ratio = if ratio.is_nan() {
        4.0
    } else {
        ratio.clamp(1.0, MAX_RATIO)
    };
    let knee = if knee.is_finite() {
        knee.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let gr = gain_change_db(
        f64::from(level),
        f64::from(threshold),
        f64::from(ratio),
        f64::from(knee),
        mode,
    )
    .clamp(FLOOR_DB, 0.0);
    let gain = if gr <= FLOOR_DB {
        0.0
    } else {
        10f64.powf(gr / 20.0)
    };
    (gr as f32, gain as f32)
}

impl Node for GainComputer {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("level", Unit::Db, 0.0)
                    .range(-60.0, 6.0)
                    .doc("Input level in decibels (from an Envelope Follower)."),
            )
            .input(
                SocketSpec::float("threshold", Unit::Db, -20.0)
                    .range(-60.0, 0.0)
                    .doc("Where compression (or expansion) begins."),
            )
            .input(
                SocketSpec::float("ratio", Unit::Ratio, 4.0)
                    .range(1.0, 20.0)
                    .log()
                    .doc("How strongly levels past the threshold are changed."),
            )
            .input(
                SocketSpec::float("knee", Unit::Db, 6.0)
                    .range(0.0, 24.0)
                    .doc("Width of the soft corner at the threshold."),
            )
            .input(
                SocketSpec::choice("mode", Mode::NAMES, "compress")
                    .doc("Compress above the threshold or expand below it."),
            )
            .output(
                SocketSpec::float("gain_reduction", Unit::Db, 0.0)
                    .poly()
                    .doc("Gain change in decibels (0 or negative)."),
            )
            .output(
                SocketSpec::float("gain", Unit::Generic, 1.0)
                    .poly()
                    .doc("The same change as a linear factor."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        GainComputer {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let mode = Mode::from_index(io.inputs.choice(MODE, Mode::NAMES.len()));
        let level = io.inputs.float(LEVEL);
        let threshold = io.inputs.float(THRESHOLD);
        let ratio = io.inputs.float(RATIO);
        let knee = io.inputs.float(KNEE);
        let frames = io.frames();
        let (mut gr_out, mut gain_out) = io.outputs.float2(GAIN_REDUCTION, GAIN);
        let per_sample = gr_out.is_sample() || gain_out.is_sample();
        let n = if per_sample { frames } else { 1 };
        for i in 0..n {
            let (gr, g) = compute(level.at(i), threshold.at(i), ratio.at(i), knee.at(i), mode);
            gr_out.set(i, gr);
            gain_out.set(i, g);
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<GainComputer>(json!({}), 48_000.0, block, &NoResources)
    }

    fn at(level: f64, threshold: f64, ratio: f64, knee: f64, mode: &str) -> (f32, f32) {
        let mut h = harness(64);
        h.set("level", level)
            .set("threshold", threshold)
            .set("ratio", ratio)
            .set("knee", knee)
            .set("mode", mode);
        let out = h.run(64);
        (out.float("gain_reduction")[0], out.float("gain")[0])
    }

    #[test]
    fn compression_above_the_threshold() {
        assert_eq!(at(-30.0, -20.0, 4.0, 0.0, "compress"), (0.0, 1.0));
        let (gr, g) = at(0.0, -20.0, 4.0, 0.0, "compress");
        assert_eq!(gr, -15.0);
        assert!((g - 10f32.powf(-0.75)).abs() < 1e-6);
        // Knee: the threshold itself is already turned down a little.
        let (gr, _) = at(-20.0, -20.0, 4.0, 10.0, "compress");
        assert!((gr - (-0.9375)).abs() < 1e-5, "{gr}");
        // A huge ratio limits: everything over the threshold comes out at it.
        let (gr, _) = at(-5.0, -20.0, 1000.0, 0.0, "compress");
        assert!((gr - (-15.0)).abs() < 0.02, "{gr}");
        // A wired infinite ratio is held at the top of the range.
        let mut h = harness(64);
        h.set("level", -5.0)
            .set("knee", 0.0)
            .set_samples("ratio", vec![f32::INFINITY; 8]);
        let gr = h.run(8).float("gain_reduction")[3];
        assert!((gr - (-15.0)).abs() < 0.02, "{gr}");
    }

    #[test]
    fn expansion_below_the_threshold() {
        assert_eq!(at(-30.0, -40.0, 3.0, 0.0, "expand").0, 0.0);
        assert_eq!(at(-50.0, -40.0, 3.0, 0.0, "expand").0, -20.0);
        // A gate: far below the threshold the gain is exactly 0.
        let (gr, g) = at(-90.0, -40.0, 1000.0, 0.0, "expand");
        assert_eq!((gr, g), (-120.0, 0.0));
    }

    #[test]
    fn outputs_follow_the_level_rate() {
        let level: Vec<f32> = (0..256).map(|i| -60.0 + i as f32 * 0.25).collect();
        let render = |block: usize| {
            let mut h = harness(block);
            h.set_samples("level", level.clone());
            let out = h.run(256);
            (
                out.float("gain_reduction").to_vec(),
                out.float("gain").to_vec(),
            )
        };
        let one = render(1);
        assert_eq!(one, render(64));
        // Per sample: rising level means falling gain once past -23 dB.
        assert_eq!(one.0[0], 0.0);
        assert!(one.0[255] < one.0[200]);
        // A constant level gives block-rate outputs.
        let mut h = harness(64);
        h.set("level", -10.0);
        let sig = h.signature().clone();
        assert!(matches!(sig.outputs[0].rate, ac_core::OutRate::Poly));
        let out = h.run(64);
        assert!(out.float("gain").iter().all(|&g| g == out.float("gain")[0]));
    }

    #[test]
    fn bad_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 0.0];
        let wild: Vec<f32> = wild.iter().cycle().take(36).copied().collect();
        for mode in Mode::NAMES {
            let mut h = harness(64);
            h.set_samples("level", wild.clone())
                .set_samples("threshold", wild.iter().rev().copied().collect())
                .set_samples("ratio", wild.iter().skip(1).copied().collect())
                .set_samples("knee", wild.iter().skip(2).copied().collect())
                .set("mode", *mode);
            let out = h.run(36);
            for name in ["gain_reduction", "gain"] {
                assert!(
                    out.float(name).iter().all(|v| v.is_finite()),
                    "{mode} {name}"
                );
            }
            assert!(out.float("gain").iter().all(|v| (0.0..=1.0).contains(v)));
        }
        let mut h = harness(64);
        h.reset();
    }
}
