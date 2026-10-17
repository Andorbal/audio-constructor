use ac_core::{
    Config, FloatIn, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};

use super::finite_or_zero;

node! {
    kind: "compare",
    name: "Compare",
    category: Math,
    keywords: ["greater", "less", "threshold", "if"],
    /// Compares `a` with `b` and outputs a `Bool`: true while the comparison
    /// holds.
    ///
    /// `hysteresis` stops the output chattering when a noisy signal hovers
    /// around the threshold: once the output is true, it stays true until the
    /// comparison fails by more than `hysteresis`. With `hysteresis` at 0 it
    /// is a plain comparison.
    ///
    /// | `op` | turns true when | stays true while |
    /// | --- | --- | --- |
    /// | `greater` | a > b | a > b − hysteresis |
    /// | `greater_equal` | a ≥ b | a ≥ b − hysteresis |
    /// | `less` | a < b | a < b + hysteresis |
    /// | `less_equal` | a ≤ b | a ≤ b + hysteresis |
    /// | `equal` | a = b | a is within hysteresis of b |
    /// | `not_equal` | a ≠ b | a ≠ b (hysteresis has no effect) |
    ///
    /// The output runs at the fastest rate of `a`, `b`, and `hysteresis`, and
    /// the held state carries from one sample (or block) to the next. NaN or
    /// infinite inputs count as 0, and a negative `hysteresis` counts as 0.
    /// Restarting the transport clears the held state.
    ///
    /// **Try this:** compare a slow `Oscillator` (or any level signal) with a
    /// `Slider` threshold, `hysteresis` 0.05, for a clean gate to drive an
    /// `ADSR`. Or compare `Transport.time` with a number to switch something
    /// on partway through a song.
    pub struct Compare {
        on: bool,
    }
}

const A: usize = 0;
const B: usize = 1;
const OP: usize = 2;
const HYSTERESIS: usize = 3;
const OUT: usize = 0;

const OPS: &[&str] = &[
    "greater",
    "greater_equal",
    "less",
    "less_equal",
    "equal",
    "not_equal",
];

impl Node for Compare {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("a", Unit::Generic, 0.0).doc("The value to test."))
            .input(SocketSpec::float("b", Unit::Generic, 0.0).doc("What to compare it with."))
            .input(SocketSpec::choice("op", OPS, "greater").doc("The comparison."))
            .input(
                SocketSpec::float("hysteresis", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("How far past the threshold a true result must fall to turn false."),
            )
            .output(
                SocketSpec::boolean("out", false)
                    .poly()
                    .doc("True while the comparison holds."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Compare { on: false }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.float(A);
        let b = io.inputs.float(B);
        let h = io.inputs.float(HYSTERESIS);
        let op = io.inputs.choice(OP, OPS.len());
        let mut out = io.outputs.float(OUT);
        let ins = [a, b, h];
        // Each op is a pair of tests on d = a - b: one to turn on, one to stay on.
        match op {
            0 => self.run(&mut out, ins, |d| d > 0.0, |d, h| d > -h),
            1 => self.run(&mut out, ins, |d| d >= 0.0, |d, h| d >= -h),
            2 => self.run(&mut out, ins, |d| d < 0.0, |d, h| d < h),
            3 => self.run(&mut out, ins, |d| d <= 0.0, |d, h| d <= h),
            4 => self.run(&mut out, ins, |d| d == 0.0, |d, h| d.abs() <= h),
            _ => self.run(&mut out, ins, |d| d != 0.0, |d, _| d != 0.0),
        }
    }

    fn reset(&mut self) {
        self.on = false;
    }
}

impl Compare {
    #[inline]
    fn run(
        &mut self,
        out: &mut FloatOut,
        [a, b, h]: [FloatIn; 3],
        turn_on: impl Fn(f32) -> bool,
        stay_on: impl Fn(f32, f32) -> bool,
    ) {
        let on = &mut self.on;
        out.fill_with(|i| {
            let d = finite_or_zero(a.at(i)) - finite_or_zero(b.at(i));
            let h = finite_or_zero(h.at(i)).max(0.0);
            *on = if *on { stay_on(d, h) } else { turn_on(d) };
            if *on { 1.0 } else { 0.0 }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn eval(op: &str, a: f32, b: f32) -> f32 {
        let mut h = Harness::new::<Compare>(json!({}));
        h.set("a", a).set("b", b).set("op", op);
        h.run(1).float("out")[0]
    }

    #[test]
    fn plain_comparisons() {
        let cases = [
            ("greater", [0.0, 0.0, 1.0]),
            ("greater_equal", [0.0, 1.0, 1.0]),
            ("less", [1.0, 0.0, 0.0]),
            ("less_equal", [1.0, 1.0, 0.0]),
            ("equal", [0.0, 1.0, 0.0]),
            ("not_equal", [1.0, 0.0, 1.0]),
        ];
        for (op, want) in cases {
            let got = [eval(op, 1.0, 2.0), eval(op, 2.0, 2.0), eval(op, 3.0, 2.0)];
            assert_eq!(got, want, "{op}");
        }
    }

    #[test]
    fn hysteresis_holds_a_true_result() {
        let mut h = Harness::new::<Compare>(json!({}));
        let a = vec![0.0, 0.6, 0.45, 0.41, 0.39, 0.45, 0.51];
        h.set_samples("a", a).set("b", 0.5).set("hysteresis", 0.1);
        let out = h.run(7);
        assert_eq!(out.float("out"), vec![0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn hysteresis_for_less_and_equal() {
        let mut h = Harness::new::<Compare>(json!({}));
        h.set_samples("a", vec![1.0, 0.4, 0.55, 0.61]);
        h.set("b", 0.5).set("hysteresis", 0.1).set("op", "less");
        assert_eq!(h.run(4).float("out"), vec![0.0, 1.0, 1.0, 0.0]);

        let mut h = Harness::new::<Compare>(json!({}));
        h.set_samples("a", vec![0.45, 0.5, 0.55, 0.65]);
        h.set("b", 0.5).set("hysteresis", 0.1).set("op", "equal");
        assert_eq!(h.run(4).float("out"), vec![0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn negative_hysteresis_counts_as_zero() {
        let mut h = Harness::new::<Compare>(json!({}));
        h.set_samples("a", vec![1.0, 0.5]);
        h.set("b", 0.5).set("hysteresis", -1.0);
        assert_eq!(h.run(2).float("out"), vec![1.0, 0.0]);
    }

    #[test]
    fn state_carries_across_blocks_at_block_rate() {
        let mut h = Harness::new::<Compare>(json!({}));
        let mut a = vec![1.0; 64];
        a.extend(vec![0.45; 64]);
        h.set_block("a", a).set("b", 0.5).set("hysteresis", 0.1);
        let out = h.run(128);
        assert_eq!(out.float("out")[0], 1.0);
        assert_eq!(out.float("out")[127], 1.0);
    }

    #[test]
    fn reset_clears_the_held_state() {
        let mut h = Harness::new::<Compare>(json!({}));
        h.set("a", 1.0).set("b", 0.5).set("hysteresis", 1.0);
        assert_eq!(h.run(1).float("out")[0], 1.0);
        h.set("a", 0.0);
        assert_eq!(h.run(1).float("out")[0], 1.0);
        h.reset();
        assert_eq!(h.run(1).float("out")[0], 0.0);
    }

    #[test]
    fn non_finite_inputs_count_as_zero() {
        let mut h = Harness::new::<Compare>(json!({}));
        h.set_samples("a", vec![f32::NAN, f32::INFINITY]);
        h.set("b", -1.0);
        assert_eq!(h.run(2).float("out"), vec![1.0, 1.0]);
    }
}
