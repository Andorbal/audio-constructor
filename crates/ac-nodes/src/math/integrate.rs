use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

use super::{finite_f32, finite_or_zero};
use crate::segments::Runs;

node! {
    kind: "integrate",
    name: "Integrate",
    category: Math,
    keywords: ["integral", "accumulate", "sum", "ramp", "leaky", "area"],
    /// Adds up `in` over time: the running integral, per second.
    ///
    /// A constant `in` of 1 rises by 1 every second, so Integrate turns a
    /// rate into a position: a speed into a distance, a constant into a
    /// ramp. `out` is the integral up to (not including) the current
    /// sample: it starts at 0, and integrating 1 gives exactly the seconds
    /// since the start or the last `reset`.
    ///
    /// - `leak` is the fraction of the stored value that drains away each
    ///   second: 0 keeps it forever, 0.5 halves it every second, 1 forgets
    ///   immediately. A leaky integrator settles instead of drifting.
    /// - `reset` sets the integral back to 0 on its exact sample (the
    ///   output reads 0 there).
    /// - With `clamp` on, the integral stays between `min` and `max`
    ///   (swapped if reversed), and it stops at the bound rather than
    ///   winding up beyond it, so it leaves the bound as soon as `in` turns
    ///   around.
    ///
    /// The output is a sample-rate signal. NaN or infinite inputs count as
    /// 0, `leak` is limited to 0..1, and the integral stays finite. A
    /// transport restart clears it.
    ///
    /// **Try this:** integrate a `Slider` (−1..1) with `clamp` on and `min`
    /// 0, `max` 1 for a fader that moves at the slider's speed; wire a
    /// `Clock` trigger into `reset` and a constant `in` of 2 for a ramp that
    /// restarts on every beat.
    pub struct Integrate {
        y: f64,
    }
}

const IN: usize = 0;
const LEAK: usize = 1;
const RESET: usize = 2;
const CLAMP: usize = 3;
const MIN: usize = 4;
const MAX: usize = 5;
const OUT: usize = 0;

/// Keeps the integral far inside the `f64` range (and its `f32` view finite).
const LIMIT: f64 = 1.0e30;

/// Per-sample retention for a leak per second.
#[inline]
fn retention(leak: f32, rate: f64) -> f64 {
    let leak = f64::from(finite_or_zero(leak)).clamp(0.0, 1.0);
    if leak <= 0.0 {
        1.0
    } else {
        // (1 - leak)^(1/rate); ln(0) = -inf gives exp(-inf) = 0.
        ((1.0 - leak).ln() / rate).exp()
    }
}

impl Node for Integrate {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("in", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("The rate to add up, per second."),
            )
            .input(
                SocketSpec::float("leak", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Fraction of the value lost each second."),
            )
            .input(SocketSpec::trigger("reset").doc("Back to 0."))
            .input(SocketSpec::boolean("clamp", false).doc("Keep the value between min and max."))
            .input(SocketSpec::float("min", Unit::Generic, 0.0).doc("Lower bound when clamped."))
            .input(SocketSpec::float("max", Unit::Generic, 1.0).doc("Upper bound when clamped."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The integral."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Integrate { y: 0.0 }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let leak = io.inputs.float(LEAK);
        let clamp = io.inputs.boolean(CLAMP);
        let lo = io.inputs.float(MIN);
        let hi = io.inputs.float(MAX);
        let resets = io.inputs.triggers(RESET);
        let rate = ctx.sample_rate.max(1.0);
        let dt = 1.0 / rate;
        let steady = retention(leak.first(), rate);
        let varying = leak.is_varying();
        let mut out = io.outputs.float(OUT);
        for run in Runs::new([resets], ctx.frames) {
            if run.fired[0] {
                self.y = 0.0;
            }
            for i in run.at..run.until {
                if clamp.at(i) {
                    let a = f64::from(finite_or_zero(lo.at(i)));
                    let b = f64::from(finite_or_zero(hi.at(i)));
                    self.y = self.y.clamp(a.min(b), a.max(b));
                }
                out.set(i, finite_f32(self.y));
                let keep = if varying {
                    retention(leak.at(i), rate)
                } else {
                    steady
                };
                let next = self.y * keep + f64::from(finite_or_zero(x.at(i))) * dt;
                self.y = if next.is_finite() {
                    next.clamp(-LIMIT, LIMIT)
                } else {
                    0.0
                };
            }
        }
    }

    fn reset(&mut self) {
        self.y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn integrating_one_counts_seconds() {
        let mut h = Harness::new::<Integrate>(json!({}));
        h.set("in", 1.0);
        let o = h.run(48_001).float("out").to_vec();
        assert_eq!(o[0], 0.0);
        assert!((o[24_000] - 0.5).abs() < 1e-6);
        assert!((o[48_000] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reset_lands_on_its_sample() {
        let mut h = Harness::new::<Integrate>(json!({}));
        h.set("in", 2.0).triggers("reset", &[100, 1_000]);
        let o = h.run(1_200).float("out").to_vec();
        assert!(o[99] > 0.0);
        assert_eq!(o[100], 0.0);
        assert!((o[101] - 2.0 / 48_000.0).abs() < 1e-9);
        assert_eq!(o[1_000], 0.0);
        assert!((o[999] - 899.0 * 2.0 / 48_000.0).abs() < 1e-6);
    }

    #[test]
    fn leak_halves_the_value_every_second() {
        let mut h = Harness::new::<Integrate>(json!({}));
        let mut x = vec![0.0; 96_001];
        x[0] = 48_000.0; // An impulse that adds 1.
        h.set_samples("in", x).set("leak", 0.5);
        let o = h.run(96_001).float("out").to_vec();
        assert!((o[1] - 1.0).abs() < 1e-6);
        assert!((o[48_001] - 0.5).abs() < 1e-4, "{}", o[48_001]);
        assert!((o[96_000] - 0.25).abs() < 1e-4, "{}", o[96_000]);

        let mut h = Harness::new::<Integrate>(json!({}));
        h.set("in", 1.0).set("leak", 1.0);
        let o = h.run(100).float("out").to_vec();
        // Full leak forgets everything but the last sample's input.
        assert!((o[50] - 1.0 / 48_000.0).abs() < 1e-9);
    }

    #[test]
    fn clamp_stops_at_the_bounds_without_winding_up() {
        let mut x = vec![10.0; 9_600];
        x.extend(vec![-10.0; 2_400]);
        let mut h = Harness::new::<Integrate>(json!({}));
        h.set_samples("in", x)
            .set("clamp", true)
            .set("min", 1.0)
            .set("max", 0.0);
        let o = h.run(12_000).float("out").to_vec();
        // Reaches 1 after 0.1 s (4800 samples) and stays.
        assert_eq!(o[9_599], 1.0);
        // Turning around leaves the bound at once: 10 per second downward.
        assert!((o[9_600 + 480] - 0.9).abs() < 1e-3, "{}", o[9_600 + 480]);
    }

    #[test]
    fn same_output_at_any_block_size() {
        let x: Vec<f32> = (0..5_000).map(|i| ((i as f32) * 0.003).sin()).collect();
        let leak: Vec<f32> = (0..5_000).map(|i| (i % 7) as f32 * 0.1).collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Integrate>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", x.clone())
                .set_samples("leak", leak.clone())
                .triggers("reset", &[3, 777, 4_000]);
            h.run(5_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn wild_inputs_stay_finite_and_reset_clears() {
        let wild = [f32::NAN, f32::INFINITY, 3e38, -3e38, f32::NEG_INFINITY];
        let wild: Vec<f32> = wild.iter().cycle().take(1_000).copied().collect();
        let mut h = Harness::new::<Integrate>(json!({}));
        h.set_samples("in", vec![3e38; 1_000])
            .set_samples("leak", wild.clone())
            .set_samples("min", wild.clone())
            .set("clamp", true);
        assert!(h.run(1_000).float("out").iter().all(|v| v.is_finite()));

        let mut h = Harness::new::<Integrate>(json!({}));
        h.set("in", 1.0);
        h.run(1_000);
        h.reset();
        assert_eq!(h.run(1).float("out")[0], 0.0);
    }
}
