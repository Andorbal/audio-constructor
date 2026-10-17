use ac_core::{
    Config, FloatIn, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};

use super::finite_or_zero;

node! {
    kind: "unary",
    name: "Unary",
    category: Math,
    keywords: ["sin", "abs", "function"],
    /// Applies a one-input function to `in`: trigonometry, exponentials,
    /// rounding, and the like.
    ///
    /// | `op` | `out` |
    /// | --- | --- |
    /// | `sin`, `cos`, `tan` | trigonometry, with `in` in **radians** |
    /// | `exp` | e raised to `in` |
    /// | `log` | natural logarithm |
    /// | `abs` | absolute value |
    /// | `sqrt` | square root |
    /// | `floor`, `ceil` | round down, round up |
    /// | `round` | nearest whole number (halves round away from zero) |
    /// | `sign` | −1, 0, or 1 |
    /// | `negate` | −`in` |
    /// | `reciprocal` | 1 ÷ `in` |
    ///
    /// The output runs at the rate of `in`: block rate for controls, sample
    /// rate for audio-rate signals.
    ///
    /// The result is always finite. A NaN or infinite input counts as 0.
    /// `log` of 0 or less, `sqrt` of a negative number, and `reciprocal` of 0
    /// give 0; `exp` limits its input to ±87 so it can't overflow. Anything
    /// else too large to represent is 0.
    ///
    /// **Try this:** wire `Transport.time` through a `Math` (`multiply`) by
    /// 2π·f into `Unary` (`sin`): a sine at f Hz locked to song time, so it
    /// restarts in the same place on every play. It suits slow modulation
    /// best, because a long song's time loses precision at audio rates. Or
    /// put `abs` after an `Oscillator` for a full-wave rectifier.
    pub struct Unary {}
}

const IN: usize = 0;
const OP: usize = 1;
const OUT: usize = 0;

const OPS: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "exp",
    "log",
    "abs",
    "sqrt",
    "floor",
    "ceil",
    "round",
    "sign",
    "negate",
    "reciprocal",
];

/// `exp(87)` ≈ 6e37 and `exp(-87)` ≈ 1.6e-38 are both normal `f32`s, so the
/// clamp avoids overflow without producing slow denormals.
const EXP_LIMIT: f32 = 87.0;

impl Node for Unary {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The input value."))
            .input(SocketSpec::choice("op", OPS, "sin").doc("The function to apply."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("The result."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Unary {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let op = io.inputs.choice(OP, OPS.len());
        let mut out = io.outputs.float(OUT);
        // One monomorphized loop per op keeps the match out of the per-sample path.
        match op {
            0 => apply(&mut out, x, f32::sin),
            1 => apply(&mut out, x, f32::cos),
            2 => apply(&mut out, x, f32::tan),
            3 => apply(&mut out, x, |v| v.clamp(-EXP_LIMIT, EXP_LIMIT).exp()),
            4 => apply(&mut out, x, |v| if v > 0.0 { v.ln() } else { 0.0 }),
            5 => apply(&mut out, x, f32::abs),
            6 => apply(&mut out, x, |v| if v > 0.0 { v.sqrt() } else { 0.0 }),
            7 => apply(&mut out, x, f32::floor),
            8 => apply(&mut out, x, f32::ceil),
            9 => apply(&mut out, x, f32::round),
            10 => apply(&mut out, x, sign),
            11 => apply(&mut out, x, |v| -v),
            _ => apply(&mut out, x, |v| if v == 0.0 { 0.0 } else { 1.0 / v }),
        }
    }

    fn reset(&mut self) {}
}

#[inline]
fn apply(out: &mut FloatOut, x: FloatIn, f: impl Fn(f32) -> f32) {
    out.fill_with(|i| finite_or_zero(f(finite_or_zero(x.at(i)))));
}

/// Unlike `f32::signum`, zero (of either sign) maps to 0.
#[inline]
fn sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn eval(op: &str, x: f32) -> f32 {
        let mut h = Harness::new::<Unary>(json!({}));
        h.set("in", x).set("op", op);
        h.run(1).float("out")[0]
    }

    #[test]
    fn trig_is_in_radians() {
        assert!((eval("sin", FRAC_PI_2) - 1.0).abs() < 1e-6);
        assert!((eval("cos", PI) + 1.0).abs() < 1e-6);
        assert!((eval("tan", PI / 4.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn default_op_is_sin() {
        let mut h = Harness::new::<Unary>(json!({}));
        h.set("in", FRAC_PI_2);
        assert!((h.run(1).float("out")[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn other_ops() {
        assert!((eval("exp", 1.0) - std::f32::consts::E).abs() < 1e-5);
        assert!((eval("log", std::f32::consts::E) - 1.0).abs() < 1e-6);
        assert_eq!(eval("abs", -3.0), 3.0);
        assert_eq!(eval("sqrt", 9.0), 3.0);
        assert_eq!(eval("floor", -1.5), -2.0);
        assert_eq!(eval("ceil", -1.5), -1.0);
        assert_eq!(eval("round", 2.5), 3.0);
        assert_eq!(eval("round", -2.5), -3.0);
        assert_eq!(eval("sign", -0.1), -1.0);
        assert_eq!(eval("sign", 4.0), 1.0);
        assert_eq!(eval("sign", 0.0), 0.0);
        assert_eq!(eval("sign", -0.0), 0.0);
        assert_eq!(eval("negate", 2.0), -2.0);
        assert_eq!(eval("reciprocal", 4.0), 0.25);
    }

    #[test]
    fn domain_errors_give_zero() {
        assert_eq!(eval("log", 0.0), 0.0);
        assert_eq!(eval("log", -1.0), 0.0);
        assert_eq!(eval("sqrt", -4.0), 0.0);
        assert_eq!(eval("reciprocal", 0.0), 0.0);
    }

    #[test]
    fn exp_never_overflows() {
        let big = eval("exp", 1000.0);
        assert!(big.is_finite() && big > 1e37);
        let small = eval("exp", -1000.0);
        assert!(small.is_normal() && small > 0.0);
    }

    #[test]
    fn non_finite_input_counts_as_zero() {
        let mut h = Harness::new::<Unary>(json!({}));
        h.set_samples("in", vec![f32::NAN, f32::NEG_INFINITY]);
        h.set("op", "cos");
        assert_eq!(h.run(2).float("out"), vec![1.0, 1.0]);
    }

    #[test]
    fn follows_input_rate() {
        let phase: Vec<f32> = (0..256).map(|i| i as f32 * 0.1).collect();
        let mut h = Harness::new::<Unary>(json!({}));
        h.set_samples("in", phase.clone());
        let out = h.run(256);
        for (o, p) in out.float("out").iter().zip(&phase) {
            assert_eq!(*o, p.sin());
        }
        let mut h = Harness::new::<Unary>(json!({}));
        h.set_block("in", phase.clone());
        let out = h.run(128);
        assert_eq!(out.float("out")[127], phase[64].sin());
    }
}
