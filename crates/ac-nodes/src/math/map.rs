use ac_core::{
    BoolIn, Config, FloatIn, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, node,
};

use super::finite_or_zero;

node! {
    kind: "map",
    name: "Map",
    category: Math,
    keywords: ["range", "scale", "remap", "lerp"],
    /// Remaps `in` from one range (`in_min`..`in_max`) to another
    /// (`out_min`..`out_max`), along a straight line or a curve.
    ///
    /// - `linear`: equal steps in give equal steps out.
    /// - `exponential`: equal steps in multiply the output by equal ratios,
    ///   out_min·(out_max/out_min)^t where t runs 0..1 across the input
    ///   range. This is the natural curve for frequencies and times. It needs
    ///   `out_min` and `out_max` both above 0; otherwise it acts as `linear`.
    /// - `logarithmic`: the inverse, for an input that is spread
    ///   exponentially: t = log(in/in_min) / log(in_max/in_min), then a
    ///   straight line to the output. It needs `in_min` and `in_max` both
    ///   above 0; otherwise it acts as `linear`. A `logarithmic` Map with its
    ///   ranges swapped undoes an `exponential` one.
    ///
    /// With `clamp` on (the default) the output stays within the output
    /// range; with it off, inputs outside the input range extend the curve.
    /// Either range may run backwards (`out_min` above `out_max` flips the
    /// direction).
    ///
    /// The output runs at the fastest rate among the inputs and is always
    /// finite: NaN or infinite inputs count as 0, `in_min` equal to `in_max`
    /// gives `out_min`, an unclamped `logarithmic` input at or below 0 gives
    /// `out_min`, and a result too large to represent is 0.
    ///
    /// **Try this:** map a 0..1 `Slider` to 20..20000 `exponential` and wire
    /// it into a `Biquad` cutoff: the sweep feels even across the whole
    /// range. Or map an `Oscillator` (−1..1) to 200..2000 for a filter LFO
    /// with exactly the range you want.
    pub struct Map {}
}

const IN: usize = 0;
const IN_MIN: usize = 1;
const IN_MAX: usize = 2;
const OUT_MIN: usize = 3;
const OUT_MAX: usize = 4;
const CURVE: usize = 5;
const CLAMP: usize = 6;
const OUT: usize = 0;

const CURVES: &[&str] = &["linear", "exponential", "logarithmic"];

/// One frame's inputs, already made finite.
#[derive(Clone, Copy, Debug)]
struct Args {
    x: f32,
    in_min: f32,
    in_max: f32,
    out_min: f32,
    out_max: f32,
    clamp: bool,
}

impl Node for Map {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The value to remap."))
            .input(SocketSpec::float("in_min", Unit::Generic, 0.0).doc("Start of the input range."))
            .input(SocketSpec::float("in_max", Unit::Generic, 1.0).doc("End of the input range."))
            .input(
                SocketSpec::float("out_min", Unit::Generic, 0.0).doc("Start of the output range."),
            )
            .input(SocketSpec::float("out_max", Unit::Generic, 1.0).doc("End of the output range."))
            .input(SocketSpec::choice("curve", CURVES, "linear").doc("The shape of the mapping."))
            .input(
                SocketSpec::boolean("clamp", true).doc("Keep the output inside the output range."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("The remapped value."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Map {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let ins = MapInputs {
            x: io.inputs.float(IN),
            in_min: io.inputs.float(IN_MIN),
            in_max: io.inputs.float(IN_MAX),
            out_min: io.inputs.float(OUT_MIN),
            out_max: io.inputs.float(OUT_MAX),
            clamp: io.inputs.boolean(CLAMP),
        };
        let curve = io.inputs.choice(CURVE, CURVES.len());
        let mut out = io.outputs.float(OUT);
        match curve {
            0 => apply(&mut out, &ins, linear),
            1 => apply(&mut out, &ins, exponential),
            _ => apply(&mut out, &ins, logarithmic),
        }
    }

    fn reset(&mut self) {}
}

struct MapInputs<'a> {
    x: FloatIn<'a>,
    in_min: FloatIn<'a>,
    in_max: FloatIn<'a>,
    out_min: FloatIn<'a>,
    out_max: FloatIn<'a>,
    clamp: BoolIn<'a>,
}

impl MapInputs<'_> {
    #[inline]
    fn at(&self, i: usize) -> Args {
        Args {
            x: finite_or_zero(self.x.at(i)),
            in_min: finite_or_zero(self.in_min.at(i)),
            in_max: finite_or_zero(self.in_max.at(i)),
            out_min: finite_or_zero(self.out_min.at(i)),
            out_max: finite_or_zero(self.out_max.at(i)),
            clamp: self.clamp.at(i),
        }
    }
}

#[inline]
fn apply(out: &mut FloatOut, ins: &MapInputs, f: impl Fn(Args) -> f32) {
    out.fill_with(|i| finite_or_zero(f(ins.at(i))));
}

/// Position of `x` in the input range as 0..1 (clamped if asked).
#[inline]
fn position(f: &Args) -> f32 {
    let t = (f.x - f.in_min) / (f.in_max - f.in_min);
    if f.clamp { t.clamp(0.0, 1.0) } else { t }
}

#[inline]
fn lerp(f: &Args, t: f32) -> f32 {
    f.out_min + (f.out_max - f.out_min) * t
}

fn linear(f: Args) -> f32 {
    if f.in_min == f.in_max {
        return f.out_min;
    }
    lerp(&f, position(&f))
}

fn exponential(f: Args) -> f32 {
    if f.out_min <= 0.0 || f.out_max <= 0.0 {
        return linear(f);
    }
    if f.in_min == f.in_max {
        return f.out_min;
    }
    f.out_min * (f.out_max / f.out_min).powf(position(&f))
}

fn logarithmic(f: Args) -> f32 {
    if f.in_min <= 0.0 || f.in_max <= 0.0 {
        return linear(f);
    }
    if f.in_min == f.in_max {
        return f.out_min;
    }
    let x = if f.clamp {
        f.x.clamp(f.in_min.min(f.in_max), f.in_min.max(f.in_max))
    } else {
        f.x
    };
    if x <= 0.0 {
        return f.out_min;
    }
    let t = (x / f.in_min).ln() / (f.in_max / f.in_min).ln();
    lerp(&f, if f.clamp { t.clamp(0.0, 1.0) } else { t })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(curve: &str, ranges: [f32; 4]) -> Harness {
        let mut h = Harness::new::<Map>(json!({}));
        h.set("in_min", ranges[0])
            .set("in_max", ranges[1])
            .set("out_min", ranges[2])
            .set("out_max", ranges[3])
            .set("curve", curve);
        h
    }

    fn eval(curve: &str, x: f32, ranges: [f32; 4]) -> f32 {
        let mut h = harness(curve, ranges);
        h.set("in", x);
        h.run(1).float("out")[0]
    }

    #[test]
    fn defaults_are_identity_on_zero_to_one() {
        let mut h = Harness::new::<Map>(json!({}));
        h.set("in", 0.25);
        assert_eq!(h.run(1).float("out")[0], 0.25);
    }

    #[test]
    fn linear_remaps_and_clamps() {
        let r = [0.0, 1.0, -60.0, 0.0];
        assert_eq!(eval("linear", 0.5, r), -30.0);
        assert_eq!(eval("linear", 2.0, r), 0.0);
        assert_eq!(eval("linear", -1.0, r), -60.0);
        // Reversed ranges.
        assert_eq!(eval("linear", 0.25, [1.0, 0.0, 0.0, 100.0]), 75.0);
        assert_eq!(eval("linear", 0.25, [0.0, 1.0, 100.0, 0.0]), 75.0);
    }

    #[test]
    fn clamp_off_extrapolates() {
        let mut h = harness("linear", [0.0, 1.0, 0.0, 10.0]);
        h.set("in", 2.0).set("clamp", false);
        assert_eq!(h.run(1).float("out")[0], 20.0);
    }

    #[test]
    fn exponential_multiplies_evenly() {
        let r = [0.0, 1.0, 20.0, 20_000.0];
        assert!((eval("exponential", 0.0, r) - 20.0).abs() < 1e-3);
        assert!((eval("exponential", 0.5, r) - 632.455_5).abs() < 1e-2);
        assert!((eval("exponential", 1.0, r) - 20_000.0).abs() < 1e-1);
        // Non-positive output range falls back to linear.
        assert_eq!(eval("exponential", 0.5, [0.0, 1.0, 0.0, 10.0]), 5.0);
    }

    #[test]
    fn logarithmic_undoes_exponential() {
        for t in [0.0, 0.1, 0.3, 0.77, 1.0] {
            let hz = eval("exponential", t, [0.0, 1.0, 20.0, 20_000.0]);
            let back = eval("logarithmic", hz, [20.0, 20_000.0, 0.0, 1.0]);
            assert!((back - t).abs() < 1e-5, "{t} -> {hz} -> {back}");
        }
        // Non-positive input range falls back to linear.
        assert_eq!(eval("logarithmic", 0.5, [0.0, 1.0, 0.0, 10.0]), 5.0);
    }

    #[test]
    fn logarithmic_domain_guards() {
        let r = [1.0, 100.0, 0.0, 1.0];
        assert_eq!(eval("logarithmic", -5.0, r), 0.0);
        let mut h = harness("logarithmic", r);
        h.set("in", -5.0).set("clamp", false);
        assert_eq!(h.run(1).float("out")[0], 0.0);
        let mut h = harness("logarithmic", r);
        h.set("in", 1000.0).set("clamp", false);
        assert!((h.run(1).float("out")[0] - 1.5).abs() < 1e-5);
    }

    #[test]
    fn empty_input_range_gives_out_min() {
        for curve in CURVES {
            assert_eq!(eval(curve, 0.3, [2.0, 2.0, 5.0, 9.0]), 5.0, "{curve}");
        }
    }

    #[test]
    fn non_finite_inputs_count_as_zero() {
        let mut h = harness("linear", [0.0, 1.0, 10.0, 20.0]);
        h.set_samples("in", vec![f32::NAN, f32::INFINITY]);
        assert_eq!(h.run(2).float("out"), vec![10.0, 10.0]);
    }

    #[test]
    fn follows_the_fastest_input() {
        let mut h = harness("linear", [0.0, 1.0, 0.0, 2.0]);
        let x: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        h.set_samples("in", x.clone());
        let out = h.run(64);
        assert_eq!(out.float("out")[32], 1.0);
        assert_eq!(out.float("out")[63], x[63] * 2.0);
    }
}
