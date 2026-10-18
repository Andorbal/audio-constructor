use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::curve::Curve;
use ac_dsp::oversample::{LATENCY, Oversampler2x};
use ac_dsp::waveshape::{self, Shape};

node! {
    kind: "shaper",
    name: "Shaper",
    category: Level,
    keywords: ["distortion", "saturation", "overdrive", "waveshaper", "clipper", "fold", "fuzz", "tube", "transfer function"],
    /// Waveshaping: bends the signal through a curve for saturation,
    /// clipping, folding, or any transfer function you draw.
    ///
    /// `shape` picks the curve. All of them pass quiet signals unchanged
    /// and shape only what gets near or past ±1:
    /// - **tanh**: smooth, warm saturation.
    /// - **hard_clip**: flat tops at ±1.
    /// - **soft_clip**: a cubic curve that rounds into ±1 by 1.5.
    /// - **fold**: peaks past ±1 fold back down instead of clipping, again
    ///   and again as the drive rises (a West-coast style folder).
    /// - **tube**: soft on positive swings, clipping at half level on
    ///   negative ones; the asymmetry adds even harmonics.
    /// - **custom**: the `points` config as a transfer function: `x` is the
    ///   input (−1 to 1), `y` the output, in the breakpoint format the
    ///   `Graph` node uses (`[[x, y], [x, y, "smooth"], ...]`). Past the
    ///   first and last points the curve holds their values. The default
    ///   `[[-1, -1], [1, 1]]` passes −1..1 through and clips beyond.
    ///
    /// `drive` (dB) amplifies the input before the curve, so more drive
    /// means more shaping; `bias` (added after the drive) shifts the signal
    /// along the curve, making symmetric curves asymmetric. The constant
    /// offset the bias would add to the output is subtracted again, so
    /// silence stays silent. `mix` blends the dry signal (0) with the
    /// shaped one (1, the default).
    ///
    /// The curve runs at twice the sample rate, between 51-tap halfband
    /// filters, so the new harmonics it creates fold back as aliasing far
    /// less. That costs 25 samples of delay (about half a millisecond),
    /// which the node reports so the engine keeps parallel paths aligned;
    /// the dry part of `mix` is delayed to match. Up to 64 channels are
    /// processed; the output has as many channels as the input. A `Float`
    /// wired through the automatic Float → Audio conversion is shaped too,
    /// which makes the custom curve a mapping for control signals.
    ///
    /// **Try this:** put a `tanh` Shaper with 12 dB of `drive` after an
    /// `Oscillator` sine for a warm overdrive, or `fold` a sine with `drive`
    /// swept by an `ADSR` (through a `Map` into dB) for a metallic pluck.
    pub struct Shaper {
        curve: Curve,
        over: Vec<Oversampler2x>,
        /// `LINES` rings of `LATENCY` samples: the dry signal, delayed to
        /// line up with the shaped one.
        dry: Vec<f32>,
        write: usize,
    }
}

const IN: usize = 0;
const SHAPE: usize = 1;
const DRIVE: usize = 2;
const BIAS: usize = 3;
const MIX: usize = 4;
const OUT: usize = 0;

const SHAPES: &[&str] = &["tanh", "hard_clip", "soft_clip", "fold", "tube", "custom"];
/// The identity over −1..1, clipping beyond.
pub const DEFAULT_POINTS: &[(f64, f64)] = &[(-1.0, -1.0), (1.0, 1.0)];

/// Channels with their own filters. The node isn't told its channel count
/// before `process`; the state is small, so this covers any patch.
const LINES: usize = 64;
const DELAY: usize = LATENCY as usize;

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// One curve: a built-in shape, or the custom points.
#[derive(Clone, Copy)]
enum Transfer<'a> {
    Builtin(Shape),
    Custom(&'a Curve),
}

impl Transfer<'_> {
    #[inline]
    fn apply(self, x: f32) -> f32 {
        match self {
            Transfer::Builtin(s) => waveshape::apply(s, x),
            Transfer::Custom(c) => {
                if x.is_finite() {
                    c.eval(f64::from(x)) as f32
                } else {
                    0.0
                }
            }
        }
    }
}

/// Drive as a linear gain, bias, and mix for one frame.
#[inline]
fn params(drive: f32, bias: f32, mix: f32) -> (f32, f32, f32) {
    let drive = finite_or(drive, 0.0).clamp(-60.0, 60.0);
    (
        10f32.powf(drive / 20.0),
        finite_or(bias, 0.0).clamp(-10.0, 10.0),
        finite_or(mix, 1.0).clamp(0.0, 1.0),
    )
}

impl Node for Shaper {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to shape."))
            .input(SocketSpec::choice("shape", SHAPES, "tanh").doc("The curve."))
            .input(
                SocketSpec::float("drive", Unit::Db, 0.0)
                    .range(0.0, 48.0)
                    .doc("Gain into the curve."),
            )
            .input(
                SocketSpec::float("bias", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("Offset added before the curve."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("0 is dry, 1 is fully shaped."),
            )
            .output(SocketSpec::audio("out").doc("The shaped signal."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Shaper {
            curve: Curve::from_config(config, "points", DEFAULT_POINTS),
            over: (0..LINES).map(|_| Oversampler2x::new()).collect(),
            dry: vec![0.0; LINES * DELAY],
            write: 0,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let shape = io.inputs.choice(SHAPE, SHAPES.len());
        let transfer = match Shape::from_index(shape) {
            Some(s) => Transfer::Builtin(s),
            None => Transfer::Custom(&self.curve),
        };
        let input = io.inputs.audio(IN);
        let (drive, bias, mix) = (
            io.inputs.float(DRIVE),
            io.inputs.float(BIAS),
            io.inputs.float(MIX),
        );
        let varying = drive.is_varying() || bias.is_varying() || mix.is_varying();
        let zero = transfer.apply(0.0);
        let frame = |i: usize| {
            let (g, b, m) = params(drive.at(i), bias.at(i), mix.at(i));
            // What the bias alone would add at the output.
            (g, b, m, transfer.apply(b) - zero)
        };
        let fixed = frame(0);
        let frames = io.frames();
        let start = self.write;
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            let (Some(over), Some(ring)) = (
                self.over.get_mut(c),
                self.dry.get_mut(c * DELAY..(c + 1) * DELAY),
            ) else {
                dst.fill(0.0);
                continue;
            };
            let mut w = start;
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let (g, b, m, offset) = if varying { frame(i) } else { fixed };
                let x = if x.is_finite() { x } else { 0.0 };
                let wet = over.process(x, |v| transfer.apply(g * v + b) - offset);
                let dry = ring[w];
                ring[w] = x;
                w += 1;
                if w == DELAY {
                    w = 0;
                }
                *o = dry + m * (wet - dry);
            }
        }
        self.write = (start + frames) % DELAY;
    }

    fn latency(&self) -> u32 {
        LATENCY
    }

    fn reset(&mut self) {
        for o in &mut self.over {
            o.reset();
        }
        self.dry.fill(0.0);
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms, tone_magnitude};
    use serde_json::json;

    fn harness(config: Config, block: usize) -> Harness {
        Harness::with_resources::<Shaper>(config, 48_000.0, block, &NoResources)
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn shape(config: Config, x: &[f32], set: &[(&str, ac_core::Value)]) -> Vec<f32> {
        let mut h = harness(config, 64);
        h.set_audio("in", vec![x.to_vec()]);
        for (k, v) in set {
            h.set(k, v.clone());
        }
        h.run(x.len()).audio("out")[0].clone()
    }

    #[test]
    fn quiet_signals_pass_25_samples_late() {
        let x = sine(1_000.0, 0.01, 2_000);
        for s in ["tanh", "hard_clip", "soft_clip", "fold", "tube", "custom"] {
            let y = shape(json!({}), &x, &[("shape", s.into())]);
            let err = x[..1_900]
                .iter()
                .zip(&y[DELAY..])
                .skip(60)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(err < 1e-4, "{s}: {err}");
        }
        assert_eq!(Shaper::new(&json!({}), &ctx()).latency(), 25);
    }

    fn ctx() -> PrepareCtx<'static> {
        PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Shaper",
            resources: &NoResources,
        }
    }

    #[test]
    fn mix_zero_is_the_exact_dry_signal_delayed() {
        let x = sine(3_000.0, 0.9, 500);
        let y = shape(
            json!({}),
            &x,
            &[("mix", 0.0.into()), ("drive", 24.0.into())],
        );
        assert_eq!(&y[DELAY..], &x[..500 - DELAY]);
        assert!(y[..DELAY].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn drive_saturates_and_curves_differ() {
        let x = sine(200.0, 0.5, 9_600);
        let clipped = shape(
            json!({}),
            &x,
            &[("shape", "hard_clip".into()), ("drive", 18.0.into())],
        );
        let tail = &clipped[1_000..];
        // Nearly a square wave: RMS close to the peak.
        assert!(peak(tail) > 0.95 && peak(tail) < 1.15, "{}", peak(tail));
        assert!(rms(tail) > 0.9, "{}", rms(tail));
        // Folding a DC level past 1 brings it back down.
        let dc = vec![1.5; 400];
        let folded = shape(json!({}), &dc, &[("shape", "fold".into())]);
        assert!((folded[300] - 0.5).abs() < 1e-4, "{}", folded[300]);
        // Tube clips negative swings at half level.
        let neg = vec![-10.0; 400];
        let tube = shape(json!({}), &neg, &[("shape", "tube".into())]);
        assert!((tube[300] + 0.5).abs() < 1e-4, "{}", tube[300]);
    }

    #[test]
    fn bias_offset_is_removed_and_adds_even_harmonics() {
        let silence = vec![0.0; 300];
        for s in ["tanh", "soft_clip", "tube", "custom"] {
            let y = shape(
                json!({}),
                &silence,
                &[("shape", s.into()), ("bias", 0.5.into())],
            );
            assert!(y.iter().all(|&v| v.abs() < 1e-6), "{s}");
        }
        let x = sine(1_000.0, 0.8, 48_000);
        let even = |bias: f64| {
            let y = shape(
                json!({}),
                &x,
                &[("drive", 6.0.into()), ("bias", bias.into())],
            );
            tone_magnitude(&y[1_000..], 2_000.0, 48_000.0)
        };
        assert!(even(0.0) < 1e-3);
        assert!(even(0.4) > 0.05);
    }

    #[test]
    fn custom_points_are_a_transfer_function() {
        // A full-wave rectifier doubles the frequency.
        let rect = json!({ "points": [[-1, 1], [0, 0], [1, 1]] });
        let x = sine(1_000.0, 0.5, 48_000);
        let y = shape(rect, &x, &[("shape", "custom".into())]);
        let t = &y[1_000..];
        assert!(tone_magnitude(t, 1_000.0, 48_000.0) < 1e-3);
        assert!(tone_magnitude(t, 2_000.0, 48_000.0) > 0.15);
        // Garbage points fall back to the identity.
        let y = shape(
            json!({ "points": 3 }),
            &[0.25; 200],
            &[("shape", "custom".into())],
        );
        assert!((y[150] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn oversampling_reduces_aliasing() {
        // Harmonics of a hard-driven 7 kHz tone that would fold to 13 kHz
        // and 1 kHz without oversampling.
        let x = sine(7_000.0, 1.0, 48_000);
        let y = shape(json!({}), &x, &[("drive", 6.0.into())]);
        let g = 10f32.powf(6.0 / 20.0);
        let naive: Vec<f32> = x.iter().map(|v| (g * v).tanh()).collect();
        for f in [13_000.0, 1_000.0] {
            let (a, b) = (
                tone_magnitude(&naive, f, 48_000.0),
                tone_magnitude(&y, f, 48_000.0),
            );
            assert!(b < 0.1 * a, "{f} Hz: {b} vs {a}");
        }
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let x = sine(440.0, 0.9, 3_000);
        let drive: Vec<f32> = (0..3_000)
            .map(|i| 12.0 + 12.0 * (i as f32 * 0.002).sin())
            .collect();
        for s in SHAPES {
            let render = |block: usize| {
                let mut h = harness(json!({}), block);
                h.set_audio("in", vec![x.clone(), x.iter().map(|v| -v).collect()])
                    .set("shape", *s)
                    .set_samples("drive", drive.clone())
                    .set("bias", 0.2)
                    .set("mix", 0.7);
                h.run(3_000).audio("out").to_vec()
            };
            let one = render(1);
            assert_eq!(one, render(64), "{s}");
            assert_eq!(one, render(512), "{s}");
        }
    }

    #[test]
    fn extreme_inputs_stay_finite_and_reset_clears() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 0.5];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        for s in SHAPES {
            let mut h = harness(json!({}), 64);
            h.set_audio("in", vec![wild.clone()])
                .set("shape", *s)
                .set_samples("drive", wild.iter().rev().copied().collect())
                .set_samples("bias", wild.iter().skip(1).copied().collect())
                .set_samples("mix", wild.iter().skip(2).copied().collect());
            let out = h.run(700);
            assert!(out.audio("out")[0].iter().all(|v| v.is_finite()), "{s}");
        }
        let mut h = harness(json!({}), 64);
        h.set_audio("in", vec![vec![0.7; 100]]);
        h.run(100);
        h.reset();
        assert!(h.run(100).audio("out")[0].iter().all(|&v| v == 0.0));
    }
}
