use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};

node! {
    kind: "number",
    name: "Number",
    category: Controls,
    keywords: ["value", "field", "type", "entry", "integer", "float", "parameter", "constant"],
    /// A number you type in: a `Float` or an `Int`, optionally kept within
    /// a range.
    ///
    /// The `type` config chooses `float` (the default) or `int`; `value`
    /// and `out` both take that type. `min` and `max` (optional) bound the
    /// number: `out` is `value` kept within them, and a missing bound
    /// leaves that side open. A reversed range is swapped, bounds that
    /// aren't numbers are ignored, and an `int` number's bounds round to
    /// whole numbers. The number starts at 0, or at the nearest bound if 0
    /// is outside the range.
    ///
    /// `value` can be wired, which makes the node a clamp with a readout.
    /// A float `out` follows the rate of `value`; NaN reads as the starting
    /// value and infinities as the bounds (or the largest float), so `out`
    /// is always finite. An int `out` changes once per block.
    ///
    /// **Try this:** make an `int` Number from 1 to 16 and wire it into a
    /// `Euclidean`'s `pulses`, then type new counts while it plays; or wire a
    /// float Number into an `Oscillator`'s `freq` to tune it to the exact
    /// hertz you type.
    pub struct Number {
        int: bool,
        min: f64,
        max: f64,
        start: f64,
    }
}

const VALUE: usize = 0;
const OUT: usize = 0;

/// The number's setup, repaired: the type, the bounds (open sides at the
/// type's limits), and the starting value.
struct Setup {
    int: bool,
    min: Option<f64>,
    max: Option<f64>,
}

impl Setup {
    fn from_config(config: &Config) -> Setup {
        let int = config.get_str_or("type", "float") == "int";
        let read = |key: &str| {
            let v = config.get(key)?.as_f64().filter(|v| v.is_finite())?;
            Some(if int {
                v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX))
            } else {
                v.clamp(-f64::from(f32::MAX), f64::from(f32::MAX))
            })
        };
        let (mut min, mut max) = (read("min"), read("max"));
        if let (Some(a), Some(b)) = (min, max)
            && a > b
        {
            (min, max) = (Some(b), Some(a));
        }
        Setup { int, min, max }
    }

    /// The bounds with open sides at the type's limits.
    fn limits(&self) -> (f64, f64) {
        let (lo, hi) = if self.int {
            (f64::from(i32::MIN), f64::from(i32::MAX))
        } else {
            (-f64::from(f32::MAX), f64::from(f32::MAX))
        };
        (self.min.unwrap_or(lo), self.max.unwrap_or(hi))
    }

    fn start(&self) -> f64 {
        let (lo, hi) = self.limits();
        0.0f64.clamp(lo, hi)
    }

    fn range(&self) -> Option<(f64, f64)> {
        Some((self.min?, self.max?))
    }
}

impl Node for Number {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let s = Setup::from_config(config);
        let start = s.start();
        let (mut value, out) = if s.int {
            (
                SocketSpec::int("value", start as i64),
                SocketSpec::int("out", start as i64),
            )
        } else {
            (
                SocketSpec::float("value", Unit::Generic, start).no_smooth(),
                SocketSpec::float("out", Unit::Generic, start).poly(),
            )
        };
        if let Some((lo, hi)) = s.range() {
            value = value.range(lo, hi);
        }
        Signature::new()
            .input(value.doc("The number."))
            .output(out.doc("`value`, kept within the range."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let s = Setup::from_config(config);
        let (min, max) = s.limits();
        Number {
            int: s.int,
            min,
            max,
            start: s.start(),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        if self.int {
            // `max`/`min` rather than `clamp`, which would assert on bad bounds.
            let v = f64::from(io.inputs.int(VALUE)).max(self.min).min(self.max);
            io.outputs.set_int(OUT, v as i32);
        } else {
            let value = io.inputs.float(VALUE);
            let (lo, hi, start) = (self.min as f32, self.max as f32, self.start as f32);
            io.outputs.float(OUT).fill_with(|i| {
                let v = value.at(i);
                if v.is_nan() { start } else { v.max(lo).min(hi) }
            });
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{SignalType, Value};
    use serde_json::json;

    #[test]
    fn a_float_number_passes_through_within_its_bounds() {
        let mut h = Harness::new::<Number>(json!({"min": -1.5, "max": 2.5}));
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));
        h.set("value", 1.25);
        assert!(h.run(64).float("out").iter().all(|&v| v == 1.25));
        h.set("value", 7.0);
        assert_eq!(h.run(64).float("out")[0], 2.5);
        let mut h = Harness::new::<Number>(json!({"min": -1.5, "max": 2.5}));
        h.set_samples(
            "value",
            vec![f32::NAN, f32::NEG_INFINITY, 0.5, f32::INFINITY],
        );
        assert_eq!(&h.run(4).float("out")[..4], &[0.0, -1.5, 0.5, 2.5]);
    }

    #[test]
    fn open_bounds_stay_finite() {
        let mut h = Harness::new::<Number>(json!({}));
        h.set_samples("value", vec![f32::INFINITY, -1.0e30, f32::NAN]);
        assert_eq!(&h.run(3).float("out")[..3], &[f32::MAX, -1.0e30, 0.0]);
        // Only a lower bound: the start moves up to it.
        let mut h = Harness::new::<Number>(json!({"min": 5}));
        assert_eq!(h.run(64).float("out")[0], 5.0);
        h.set("value", 1.0e9);
        assert_eq!(h.run(64).float("out")[0], 1.0e9);
    }

    #[test]
    fn an_int_number() {
        let sig = Number::describe(&json!({"type": "int", "min": 1, "max": 16}));
        assert_eq!(sig.inputs[VALUE].ty, SignalType::Int);
        assert_eq!(sig.outputs[OUT].ty, SignalType::Int);
        assert_eq!(sig.inputs[VALUE].default, Value::Int(1));
        assert_eq!(sig.inputs[VALUE].range, Some((1.0, 16.0)));
        let mut h = Harness::new::<Number>(json!({"type": "int", "min": 1, "max": 16}));
        assert_eq!(h.run(64).int("out")[0], 1);
        h.set("value", 9);
        assert_eq!(h.run(64).int("out")[0], 9);
        h.set("value", 40);
        assert_eq!(h.run(64).int("out")[0], 16);
        h.set_int("value", i32::MIN);
        assert_eq!(h.run(64).int("out")[0], 1);
        let mut h = Harness::new::<Number>(json!({"type": "int"}));
        h.set_int("value", i32::MAX);
        assert_eq!(h.run(64).int("out")[0], i32::MAX);
    }

    #[test]
    fn bad_config_is_repaired() {
        let sig = Number::describe(&json!({"min": 10, "max": -10}));
        assert_eq!(sig.inputs[VALUE].range, Some((-10.0, 10.0)));
        let sig = Number::describe(&json!({"min": "zero", "max": 3}));
        assert_eq!(sig.inputs[VALUE].range, None);
        assert_eq!(sig.inputs[VALUE].ty, SignalType::Float);
        let sig = Number::describe(&json!({"type": "int", "min": 0.4, "max": 2.6}));
        assert_eq!(sig.inputs[VALUE].range, Some((0.0, 3.0)));
        let sig = Number::describe(&json!({"type": "complex", "max": -1e300}));
        assert_eq!(sig.inputs[VALUE].ty, SignalType::Float);
        assert_eq!(
            sig.inputs[VALUE].default,
            Value::Float(-f64::from(f32::MAX))
        );
    }
}
