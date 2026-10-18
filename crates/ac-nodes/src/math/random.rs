use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::rng::Rng;

use super::finite_or_zero;
use crate::segments::Runs;

node! {
    kind: "random",
    name: "Random",
    category: Math,
    keywords: ["noise", "dice", "chance", "sample and hold", "s&h", "randomize", "gaussian"],
    /// A new random value on every `trigger`, held until the next.
    ///
    /// Each trigger picks a random position within the range and holds it;
    /// `out` is that position placed between `min` and `max`. Because the
    /// position is what is held, moving `min` or `max` moves the output
    /// with them (wire them from a `Slider` to open up the range live).
    /// Before the first trigger the position is the middle of the range.
    ///
    /// - `uniform`: every value in the range is equally likely.
    /// - `normal`: values cluster around the middle of the range in a bell
    ///   curve, with the range spanning ±3 standard deviations; the rare
    ///   values beyond are clamped to the range.
    ///
    /// The value changes on the trigger's exact sample (the output is a
    /// sample-rate signal). The sequence of values comes from the project's
    /// seed and this node's place in the patch, so a render repeats exactly,
    /// and a transport restart starts the sequence over. NaN or infinite
    /// `min`/`max` count as 0.
    ///
    /// **Try this:** trigger it from a `Clock`, set the range to 48..72, and
    /// wire it through `Quantize` (`step` 1) and `Convert` (`note_to_hz`)
    /// into an `Oscillator` for a random melody that repeats on every
    /// render.
    pub struct Random {
        seed: u64,
        rng: Rng,
        /// Held position in the range, 0..1.
        pos: f64,
    }
}

const TRIGGER: usize = 0;
const DISTRIBUTION: usize = 1;
const MIN: usize = 2;
const MAX: usize = 3;
const OUT: usize = 0;

const DISTRIBUTIONS: &[&str] = &["uniform", "normal"];

impl Node for Random {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("trigger").doc("Pick a new value."))
            .input(
                SocketSpec::choice("distribution", DISTRIBUTIONS, "uniform")
                    .doc("Flat, or clustered around the middle."),
            )
            .input(SocketSpec::float("min", Unit::Generic, 0.0).doc("Bottom of the range."))
            .input(SocketSpec::float("max", Unit::Generic, 1.0).doc("Top of the range."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.5)
                    .rate(Rate::Sample)
                    .doc("The held random value."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Random {
            seed: ctx.seed,
            rng: Rng::new(ctx.seed),
            pos: 0.5,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let triggers = io.inputs.triggers(TRIGGER);
        let normal = io.inputs.choice(DISTRIBUTION, DISTRIBUTIONS.len()) == 1;
        let lo = io.inputs.float(MIN);
        let hi = io.inputs.float(MAX);
        let mut out = io.outputs.float(OUT);
        for run in Runs::new([triggers], ctx.frames) {
            if run.fired[0] {
                self.pos = if normal {
                    (0.5 + self.rng.normal() / 6.0).clamp(0.0, 1.0)
                } else {
                    self.rng.uniform()
                };
            }
            for i in run.at..run.until {
                let a = f64::from(finite_or_zero(lo.at(i)));
                let b = f64::from(finite_or_zero(hi.at(i)));
                out.set(i, finite_or_zero((a + (b - a) * self.pos) as f32));
            }
        }
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        self.pos = 0.5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn held_values(out: &[f32], at: &[u64]) -> Vec<f32> {
        at.iter().map(|&t| out[t as usize]).collect()
    }

    #[test]
    fn changes_on_the_trigger_sample_and_holds() {
        let mut h = Harness::new::<Random>(json!({}));
        h.triggers("trigger", &[10, 100]);
        let o = h.run(200).float("out").to_vec();
        assert!(
            o[..10].iter().all(|&v| v == 0.5),
            "middle before the first trigger"
        );
        assert_ne!(o[10], 0.5);
        assert!(o[10..100].iter().all(|&v| v == o[10]));
        assert_ne!(o[100], o[10]);
        assert!(o[100..].iter().all(|&v| v == o[100]));
    }

    #[test]
    fn uniform_values_fill_the_range() {
        let at: Vec<u64> = (0..4_000).map(|k| k * 3).collect();
        let mut h = Harness::new::<Random>(json!({}));
        h.triggers("trigger", &at).set("min", 10.0).set("max", 20.0);
        let o = h.run(12_000).float("out").to_vec();
        let v = held_values(&o, &at);
        assert!(v.iter().all(|&x| (10.0..20.0).contains(&x)));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean - 15.0).abs() < 0.2, "{mean}");
        assert!(v.iter().any(|&x| x < 10.5) && v.iter().any(|&x| x > 19.5));
    }

    #[test]
    fn normal_values_cluster_in_the_middle() {
        let at: Vec<u64> = (0..4_000).map(|k| k * 3).collect();
        let mut h = Harness::new::<Random>(json!({}));
        h.triggers("trigger", &at)
            .set("distribution", "normal")
            .set("min", -3.0)
            .set("max", 3.0);
        let o = h.run(12_000).float("out").to_vec();
        let v = held_values(&o, &at);
        assert!(v.iter().all(|&x| (-3.0..=3.0).contains(&x)));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / v.len() as f32;
        // ±3σ spans the range, so σ is 1 here.
        assert!(mean.abs() < 0.1, "{mean}");
        assert!((var - 1.0).abs() < 0.1, "{var}");
        let within = v.iter().filter(|x| x.abs() < 1.0).count() as f32 / v.len() as f32;
        assert!((within - 0.68).abs() < 0.04, "{within}");
    }

    #[test]
    fn range_moves_the_held_value() {
        let mut h = Harness::new::<Random>(json!({}));
        let max: Vec<f32> = (0..128).map(|i| if i < 64 { 1.0 } else { 2.0 }).collect();
        h.triggers("trigger", &[0]).set_block("max", max);
        let o = h.run(128).float("out").to_vec();
        assert!((o[64] - 2.0 * o[0]).abs() < 1e-6);
    }

    #[test]
    fn renders_repeat_after_reset_and_ignore_block_size() {
        let at: Vec<u64> = vec![0, 5, 63, 64, 65, 300, 301, 999];
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Random>(json!({}), 48_000.0, block, &NoResources);
            h.triggers("trigger", &at).set("distribution", "normal");
            h.run(1_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));

        let mut h = Harness::new::<Random>(json!({}));
        h.triggers("trigger", &[0, 10, 20]);
        let first = h.run(64).float("out").to_vec();
        h.reset();
        // The harness counts frames from its start: the same pattern, 64 later.
        h.triggers("trigger", &[64, 74, 84]);
        let again = h.run(64).float("out").to_vec();
        assert_eq!(first[..30], again[..30]);
    }

    #[test]
    fn non_finite_range_counts_as_zero() {
        let mut h = Harness::new::<Random>(json!({}));
        h.triggers("trigger", &[0])
            .set_samples("min", vec![f32::NAN; 4])
            .set_samples("max", vec![f32::INFINITY; 4]);
        assert_eq!(h.run(4).float("out"), vec![0.0; 4]);
    }
}
