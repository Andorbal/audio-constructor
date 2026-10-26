use ac_core::{
    Config, FloatIn, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};

use super::finite_or_zero;
use super::logic::{input_count, input_name};

node! {
    kind: "math",
    name: "Math",
    category: Math,
    keywords: ["add", "multiply", "subtract", "divide", "arithmetic", "plus", "times"],
    /// Combines two values, `a` and `b`, with an arithmetic operation.
    ///
    /// | `op` | `out` |
    /// | --- | --- |
    /// | `add` | a + b |
    /// | `subtract` | a − b |
    /// | `multiply` | a × b |
    /// | `divide` | a ÷ b |
    /// | `min`, `max` | the smaller or larger of a and b |
    /// | `power` | a raised to the power b |
    /// | `modulo` | the remainder of a ÷ b, always from 0 up to \|b\| (Euclidean: −0.25 modulo 1 is 0.75) |
    /// | `atan2` | the angle in radians (−π..π) of the point with x = b and y = a |
    ///
    /// The `inputs` config adds more operands, `c`, `d`, and on (2 to 16,
    /// default 2). `add`, `multiply`, `min`, and `max` take them all (the
    /// sum, the product, the smallest, the largest); the other operations
    /// use `a` and `b` only.
    ///
    /// The output runs at the fastest rate of its inputs: block rate when
    /// both are controls, sample rate when either carries audio-rate
    /// modulation. Audio wired into `a` or `b` arrives as its first channel.
    ///
    /// The result is always finite. A NaN or infinite input counts as 0, and
    /// anything undefined or too large is 0: dividing by 0, modulo 0, and a
    /// negative number to a fractional power all give 0.
    ///
    /// **Try this:** `multiply` an `Oscillator` by a `Slider` for a depth
    /// control, then `add` a second `Slider` to set the centre, and wire the
    /// result into a `Biquad` cutoff. `modulo` with `b` = 1 wraps a rising
    /// ramp (such as `Transport.time`) into a repeating 0..1 sawtooth.
    pub struct Math {
        inputs: usize,
    }
}

const OUT: usize = 0;
const MAX_INPUTS: usize = super::logic::MAX_INPUTS;

const OPS: &[&str] = &[
    "add", "subtract", "multiply", "divide", "min", "max", "power", "modulo", "atan2",
];

impl Node for Math {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::float("a", Unit::Generic, 0.0).doc("First operand."))
            .input(SocketSpec::float("b", Unit::Generic, 0.0).doc("Second operand."));
        for k in 2..input_count(config) {
            sig = sig.input(
                SocketSpec::float(input_name(k), Unit::Generic, 0.0)
                    .doc("Another operand, for add, multiply, min, and max."),
            );
        }
        sig.input(SocketSpec::choice("op", OPS, "add").doc("The operation."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("The result."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Math {
            inputs: input_count(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let n = self.inputs;
        let op = io.inputs.choice(n, OPS.len());
        let mut out = io.outputs.float(OUT);
        if n > 2 && matches!(op, 0 | 2 | 4 | 5) {
            let ins: [FloatIn; MAX_INPUTS] = std::array::from_fn(|k| io.inputs.float(k.min(n - 1)));
            let ins = &ins[..n];
            match op {
                0 => fold(&mut out, ins, |x, y| x + y),
                2 => fold(&mut out, ins, |x, y| x * y),
                4 => fold(&mut out, ins, f32::min),
                _ => fold(&mut out, ins, f32::max),
            }
            return;
        }
        let a = io.inputs.float(0);
        let b = io.inputs.float(1);
        // One monomorphized loop per op keeps the match out of the per-sample path.
        match op {
            0 => apply(&mut out, a, b, |x, y| x + y),
            1 => apply(&mut out, a, b, |x, y| x - y),
            2 => apply(&mut out, a, b, |x, y| x * y),
            3 => apply(&mut out, a, b, divide),
            4 => apply(&mut out, a, b, f32::min),
            5 => apply(&mut out, a, b, f32::max),
            6 => apply(&mut out, a, b, f32::powf),
            7 => apply(&mut out, a, b, modulo),
            _ => apply(&mut out, a, b, f32::atan2),
        }
    }

    fn reset(&mut self) {}
}

#[inline]
fn apply(out: &mut FloatOut, a: FloatIn, b: FloatIn, f: impl Fn(f32, f32) -> f32) {
    out.fill_with(|i| finite_or_zero(f(finite_or_zero(a.at(i)), finite_or_zero(b.at(i)))));
}

/// `out` is every input combined by `f`, left to right.
#[inline]
fn fold(out: &mut FloatOut, ins: &[FloatIn], f: impl Fn(f32, f32) -> f32) {
    out.fill_with(|i| {
        let mut v = finite_or_zero(ins[0].at(i));
        for x in &ins[1..] {
            v = finite_or_zero(f(v, finite_or_zero(x.at(i))));
        }
        v
    });
}

#[inline]
fn divide(a: f32, b: f32) -> f32 {
    if b == 0.0 { 0.0 } else { a / b }
}

/// Euclidean remainder, in `0..|b|`.
#[inline]
fn modulo(a: f32, b: f32) -> f32 {
    if b == 0.0 {
        return 0.0;
    }
    let r = a.rem_euclid(b);
    // `rem_euclid` can round up to exactly |b| for tiny negative `a`; wrap it
    // so phase-wrapping patches never see a value outside the range.
    if r >= b.abs() { 0.0 } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn eval(op: &str, a: f32, b: f32) -> f32 {
        let mut h = Harness::new::<Math>(json!({}));
        h.set("a", a).set("b", b).set("op", op);
        h.run(1).float("out")[0]
    }

    #[test]
    fn arithmetic_ops() {
        assert_eq!(eval("add", 2.0, 3.0), 5.0);
        assert_eq!(eval("subtract", 2.0, 3.0), -1.0);
        assert_eq!(eval("multiply", 2.0, 3.0), 6.0);
        assert_eq!(eval("divide", 3.0, 2.0), 1.5);
        assert_eq!(eval("min", 2.0, 3.0), 2.0);
        assert_eq!(eval("max", 2.0, 3.0), 3.0);
        assert_eq!(eval("power", 2.0, 3.0), 8.0);
    }

    #[test]
    fn more_operands_for_add_multiply_min_and_max() {
        let run = |op: &str| {
            let mut h = Harness::new::<Math>(json!({ "inputs": 4 }));
            h.set("a", 2.0).set("b", 3.0).set("c", -1.0).set("d", 4.0);
            h.set("op", op);
            h.run(1).float("out")[0]
        };
        assert_eq!(run("add"), 8.0);
        assert_eq!(run("multiply"), -24.0);
        assert_eq!(run("min"), -1.0);
        assert_eq!(run("max"), 4.0);
        assert_eq!(run("subtract"), -1.0, "a and b only");
        let names: Vec<_> = Math::describe(&json!({ "inputs": 3 }))
            .inputs
            .iter()
            .map(|s| s.name.to_string())
            .collect();
        assert_eq!(names, ["a", "b", "c", "op"]);
    }

    #[test]
    fn default_op_is_add() {
        let mut h = Harness::new::<Math>(json!({}));
        h.set("a", 1.0).set("b", 2.0);
        assert_eq!(h.run(1).float("out")[0], 3.0);
    }

    #[test]
    fn modulo_is_euclidean() {
        assert_eq!(eval("modulo", 5.5, 2.0), 1.5);
        assert_eq!(eval("modulo", -0.25, 1.0), 0.75);
        assert_eq!(eval("modulo", -0.25, -1.0), 0.75);
        let r = eval("modulo", -1e-9, 1.0);
        assert!((0.0..1.0).contains(&r), "{r}");
    }

    #[test]
    fn atan2_takes_a_as_y() {
        let r = eval("atan2", 1.0, 0.0);
        assert!((r - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(eval("atan2", 0.0, 1.0), 0.0);
        assert!((eval("atan2", 0.0, -1.0) - std::f32::consts::PI).abs() < 1e-6);
    }

    #[test]
    fn domain_errors_give_zero() {
        assert_eq!(eval("divide", 1.0, 0.0), 0.0);
        assert_eq!(eval("modulo", 1.0, 0.0), 0.0);
        assert_eq!(eval("power", -2.0, 0.5), 0.0);
        assert_eq!(eval("power", 0.0, -1.0), 0.0);
        assert_eq!(eval("multiply", 3e38, 10.0), 0.0);
    }

    #[test]
    fn non_finite_inputs_count_as_zero() {
        let mut h = Harness::new::<Math>(json!({}));
        h.set_samples("a", vec![f32::NAN, f32::INFINITY, 1.0]);
        h.set("b", 2.0);
        assert_eq!(h.run(3).float("out"), vec![2.0, 2.0, 3.0]);
    }

    #[test]
    fn control_inputs_give_block_rate_output() {
        let mut h = Harness::new::<Math>(json!({}));
        let ramp: Vec<f32> = (0..128).map(|i| i as f32).collect();
        h.set_block("a", ramp).set("b", 1.0);
        let out = h.run(128);
        // Block-rate: one value per 64-frame block.
        assert_eq!(out.float("out")[10], 1.0);
        assert_eq!(out.float("out")[70], 65.0);
    }

    #[test]
    fn sample_rate_input_gives_sample_rate_output() {
        let mut h = Harness::new::<Math>(json!({}));
        let ramp: Vec<f32> = (0..128).map(|i| i as f32).collect();
        h.set_samples("b", ramp).set("a", 1.0).set("op", "multiply");
        let out = h.run(128);
        assert_eq!(out.float("out")[10], 10.0);
        assert_eq!(out.float("out")[127], 127.0);
    }

    #[test]
    fn op_can_be_wired_as_an_int() {
        let mut h = Harness::new::<Math>(json!({}));
        h.set("a", 6.0).set("b", 3.0).set_int("op", 3);
        assert_eq!(h.run(1).float("out")[0], 2.0);
        // Out-of-range indices clamp to the last op instead of failing.
        h.set_int("op", 99);
        assert_eq!(h.run(1).float("out")[0], 6f32.atan2(3.0));
    }
}
