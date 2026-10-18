use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
    util::derive_seed,
};
use ac_dsp::noise::{Brown, Pink};
use ac_dsp::rng::Rng;
use ac_dsp::util::db_to_gain;

use crate::segments::Runs;

node! {
    kind: "noise",
    name: "Noise",
    category: Generators,
    keywords: ["white noise", "pink noise", "brown noise", "hiss", "random", "static", "rumble"],
    /// Random noise in three colors: white, pink, or brown, as a
    /// sample-rate `Float` that wires into audio or into any parameter.
    ///
    /// - `color`: **white** has equal energy at every frequency (bright
    ///   hiss); **pink** loses 3 dB per octave, so every octave carries the
    ///   same energy (closer to rain or a waterfall, and the usual test
    ///   signal for mixes); **brown** loses 6 dB per octave (a deep rumble).
    ///   All three come out at about the same level, roughly −5 dBFS RMS
    ///   before `gain`; pink and brown can peak above 1 at 0 dB `gain`.
    /// - `gain`: output level in decibels (default −12). At −60 dB and below
    ///   the output is silent.
    ///
    /// The noise is seeded, so a project renders the same every time. Each
    /// node draws its own sequence (from the project seed and the node's
    /// place in the patch); the `seed` config (a whole number, default 0)
    /// picks a different one, so two Noise nodes can be made to differ or
    /// match exactly. Changing `color` while playing switches smoothly: all
    /// three colors are computed from the same random values.
    ///
    /// Generator contract: silent until `start`. Each start continues the
    /// random sequence, so repeated bursts are all different; `reset`
    /// re-seeds without changing whether it plays, so the sequence starts
    /// over, as it does when the transport returns to zero. `stop` silences
    /// it and fires `end`. Events at the same frame apply in the order
    /// stop, start, reset.
    ///
    /// **Try this:** wire a `Clock` into an `ADSR` (short decay) and
    /// multiply it with white Noise through a `Biquad` bandpass for a
    /// snare or hi-hat, or feed pink Noise into a `Meter` and a `Biquad` to
    /// hear what a filter does across the whole spectrum.
    pub struct Noise {
        seed: u64,
        rng: Rng,
        pink: Pink,
        brown: Brown,
        playing: bool,
    }
}

const COLOR: usize = 0;
const GAIN: usize = 1;
const START: usize = 2;
const STOP: usize = 3;
const RESET: usize = 4;

const OUT: usize = 0;
const PLAYING: usize = 1;
const END: usize = 2;

const COLORS: &[&str] = &["white", "pink", "brown"];
/// Loudest `gain` honoured; higher values are held here.
const MAX_GAIN_DB: f32 = 48.0;

/// The node's seed: the engine's per-node seed mixed with the config's.
fn seed(config: &Config, ctx_seed: u64) -> u64 {
    let user = match config.get("seed") {
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
            .unwrap_or(0),
        None => 0,
    };
    derive_seed(ctx_seed, user as u64)
}

impl Noise {
    /// Start each color from a steady-state level (no fade-in).
    fn prime(&mut self) {
        self.pink.randomize(&mut self.rng);
        self.brown.randomize(&mut self.rng);
    }

    fn reseed(&mut self) {
        self.rng = Rng::new(self.seed);
        self.pink.reset();
        self.brown.reset();
    }
}

impl Node for Noise {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::choice("color", COLORS, "white").doc("White, pink, or brown."))
            .input(
                SocketSpec::float("gain", Unit::Db, -12.0)
                    .range(-60.0, 12.0)
                    .doc("Output level in decibels."),
            )
            .generator_inputs(false, false)
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The noise."),
            )
            .generator_outputs()
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let seed = seed(config, ctx.seed);
        Noise {
            seed,
            rng: Rng::new(seed),
            pink: Pink::new(ctx.sample_rate),
            brown: Brown::new(ctx.sample_rate),
            playing: false,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let color = io.inputs.choice(COLOR, COLORS.len());
        let gain = io.inputs.float(GAIN);
        let events = [
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
        ];
        let frames = io.frames();
        {
            let (mut out, mut end) = io.outputs.float_trigger(OUT, END);
            for run in Runs::new(events, frames) {
                let [stop, start, reset] = run.fired;
                if stop && self.playing {
                    self.playing = false;
                    end.push(run.at as u32);
                }
                if start {
                    self.playing = true;
                    self.prime();
                }
                if reset {
                    self.reseed();
                    if self.playing {
                        self.prime();
                    }
                }
                for i in run.at..run.until {
                    if !self.playing {
                        out.set(i, 0.0);
                        continue;
                    }
                    // Two values every sample whatever the color, so
                    // switching color never shifts the random sequence.
                    let (w, r) = (self.rng.bipolar(), self.rng.bipolar());
                    let p = self.pink.next(w, r);
                    let b = self.brown.next(w);
                    let v = match color {
                        0 => w,
                        1 => p,
                        _ => b,
                    };
                    let db = gain.at(i);
                    let g = if db.is_finite() {
                        db_to_gain(db.min(MAX_GAIN_DB))
                    } else {
                        0.0
                    };
                    out.set(i, v as f32 * g);
                }
            }
        }
        io.outputs.set_bool(PLAYING, self.playing);
    }

    fn reset(&mut self) {
        self.reseed();
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms, tone_magnitude};
    use serde_json::json;

    fn harness(config: Config, block: usize) -> Harness {
        Harness::with_resources::<Noise>(config, 48_000.0, block, &NoResources)
    }

    fn render(config: Config, color: &str, block: usize, n: usize) -> Vec<f32> {
        let mut h = harness(config, block);
        h.set("color", color).set("gain", 0.0);
        h.run(n).float("out").to_vec()
    }

    #[test]
    fn white_is_uniform_at_the_documented_level() {
        let x = render(json!({}), "white", 64, 48_000);
        assert!(peak(&x) <= 1.0);
        assert!((rms(&x) - 1.0 / 3f32.sqrt()).abs() < 0.01, "{}", rms(&x));
        let mean = x.iter().sum::<f32>() / x.len() as f32;
        assert!(mean.abs() < 0.01);
        // Default gain is -12 dB.
        let mut h = harness(json!({}), 64);
        let quiet = h.run(48_000).float("out").to_vec();
        assert!((rms(&quiet) / rms(&x) - db_to_gain(-12.0)).abs() < 1e-3);
    }

    #[test]
    fn colors_tilt_the_spectrum() {
        let n = 96_000;
        let white = render(json!({}), "white", 64, n);
        let pink = render(json!({}), "pink", 64, n);
        let brown = render(json!({}), "brown", 64, n);
        // Energy near 100 Hz relative to near 5 kHz, averaged over a few
        // nearby frequencies to tame the variance.
        let ratio = |x: &[f32]| {
            let band = |f0: f64| -> f64 {
                (0..20)
                    .map(|k| tone_magnitude(x, f0 + k as f64 * 3.0, 48_000.0).powi(2))
                    .sum()
            };
            10.0 * (band(100.0) / band(5_000.0)).log10()
        };
        let (w, p, b) = (ratio(&white), ratio(&pink), ratio(&brown));
        // log2(5000 / 100) ≈ 5.6 octaves: 0, ~17, and ~34 dB.
        assert!(w.abs() < 4.0, "white {w}");
        assert!((p - 17.0).abs() < 5.0, "pink {p}");
        assert!((b - 34.0).abs() < 6.0, "brown {b}");
        // All three at about the same level.
        for x in [&pink, &brown] {
            assert!((rms(x) / rms(&white) - 1.0).abs() < 0.15, "{}", rms(x));
        }
    }

    #[test]
    fn seeded_and_independent_of_block_size() {
        for color in COLORS {
            let one = render(json!({}), color, 1, 3_000);
            assert_eq!(one, render(json!({}), color, 64, 3_000), "{color}");
            assert_eq!(one, render(json!({}), color, 512, 3_000), "{color}");
        }
        let a = render(json!({ "seed": 1 }), "white", 64, 256);
        let b = render(json!({ "seed": 2 }), "white", 64, 256);
        assert_ne!(a, b);
        assert_eq!(a, render(json!({ "seed": 1.0 }), "white", 64, 256));
        // A malformed seed is repaired, not rejected.
        assert_eq!(
            render(json!({ "seed": "x" }), "pink", 64, 256),
            render(json!({}), "pink", 64, 256)
        );
    }

    #[test]
    fn generator_contract() {
        let mut h = harness(json!({}), 64);
        h.stopped()
            .triggers("start", &[100])
            .triggers("stop", &[300]);
        let out = h.run(512);
        let o = out.float("out");
        assert_eq!(peak(&o[..100]), 0.0);
        assert!(peak(&o[100..300]) > 0.05);
        assert_eq!(peak(&o[300..]), 0.0);
        assert_eq!(out.triggers("end"), &[300]);
        assert_eq!(out.float("playing")[511], 0.0);
        // A second start continues the sequence rather than repeating it.
        let mut h = harness(json!({}), 64);
        h.set("gain", 0.0).triggers("start", &[0, 200]);
        let o = h.run(400).float("out").to_vec();
        assert_ne!(&o[..100], &o[200..300]);
    }

    #[test]
    fn reset_reseeds() {
        let first = render(json!({}), "pink", 64, 256);
        let mut h = harness(json!({}), 64);
        h.set("color", "pink").set("gain", 0.0);
        h.run(1_000);
        h.reset();
        h.play();
        assert_eq!(h.run(256).float("out"), first.as_slice());
        // The reset trigger does the same without stopping.
        let mut h = harness(json!({}), 64);
        h.set("gain", 0.0).triggers("reset", &[128]);
        let o = h.run(256).float("out").to_vec();
        assert_eq!(&o[128..], &o[..128]);
    }

    #[test]
    fn extreme_gain_stays_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -100.0];
        let wild: Vec<f32> = wild.iter().cycle().take(600).copied().collect();
        for color in COLORS {
            let mut h = harness(json!({}), 64);
            h.set("color", *color).set_samples("gain", wild.clone());
            let out = h.run(600);
            assert!(out.float("out").iter().all(|v| v.is_finite()), "{color}");
        }
    }
}
