use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};

use super::finite_or_zero;

/// The kind of value a `Constant` holds, from its `type` config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueType {
    Float,
    Int,
    Bool,
}

impl ValueType {
    fn from_config(config: &Config) -> Self {
        match config.get_str_or("type", "float") {
            "int" => ValueType::Int,
            "bool" => ValueType::Bool,
            // Unknown types fall back to the default rather than failing to load.
            _ => ValueType::Float,
        }
    }
}

node! {
    kind: "constant",
    name: "Constant",
    category: Math,
    keywords: ["value", "number"],
    /// A fixed `Float`, `Int`, or `Bool` value, set with its inline widget.
    ///
    /// The `type` setting chooses the kind of value: `float` (the default),
    /// `int`, or `bool`. The `value` input and the `out` output both take that
    /// type. Leave `value` unwired and set it with its widget; that widget is
    /// the control.
    ///
    /// `out` is a block-rate signal. If you wire something into `value`, it is
    /// passed through once per block (the value at the start of the block), so
    /// a `Constant` also turns a fast signal into a block-rate one. A `float`
    /// value that is NaN or infinite comes out as 0.
    ///
    /// **Try this:** wire one `Constant` into the `freq` of several
    /// `Oscillator`s and retune them all from one widget. A `bool` Constant
    /// makes a project-wide switch: wire it into every `loop` input you want
    /// to flip at once.
    pub struct Constant {
        ty: ValueType,
    }
}

const VALUE: usize = 0;
const OUT: usize = 0;

impl Node for Constant {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let (input, output) = match ValueType::from_config(config) {
            ValueType::Float => (
                SocketSpec::float("value", Unit::Generic, 0.0),
                SocketSpec::float("out", Unit::Generic, 0.0),
            ),
            ValueType::Int => (SocketSpec::int("value", 0), SocketSpec::int("out", 0)),
            ValueType::Bool => (
                SocketSpec::boolean("value", false),
                SocketSpec::boolean("out", false),
            ),
        };
        Signature::new()
            .input(input.doc("The value to output."))
            .output(output.rate(Rate::Block).doc("The value, once per block."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Constant {
            ty: ValueType::from_config(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        match self.ty {
            ValueType::Float => {
                let v = finite_or_zero(io.inputs.float(VALUE).first());
                io.outputs.set_float(OUT, v);
            }
            ValueType::Int => {
                let v = io.inputs.int(VALUE);
                io.outputs.set_int(OUT, v);
            }
            ValueType::Bool => {
                let v = io.inputs.boolean(VALUE).first();
                io.outputs.set_bool(OUT, v);
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{OutRate, SignalType, testing::Harness};
    use serde_json::json;

    #[test]
    fn float_is_the_default_type() {
        let mut h = Harness::new::<Constant>(json!({}));
        let sig = h.signature().clone();
        assert_eq!(sig.inputs[VALUE].ty, SignalType::Float);
        assert_eq!(sig.outputs[OUT].ty, SignalType::Float);
        assert_eq!(sig.outputs[OUT].rate, OutRate::Fixed(Rate::Block));
        h.set("value", 2.5);
        let out = h.run(128);
        assert_eq!(out.float("out"), vec![2.5; 128]);
    }

    #[test]
    fn int_type() {
        let mut h = Harness::new::<Constant>(json!({"type": "int"}));
        assert_eq!(h.signature().outputs[OUT].ty, SignalType::Int);
        h.set("value", 7);
        assert_eq!(h.run(64).int("out"), vec![7; 64]);
    }

    #[test]
    fn bool_type() {
        let mut h = Harness::new::<Constant>(json!({"type": "bool"}));
        assert_eq!(h.signature().inputs[VALUE].ty, SignalType::Bool);
        assert_eq!(h.run(64).float("out"), vec![0.0; 64]);
        h.set("value", true);
        assert_eq!(h.run(64).float("out"), vec![1.0; 64]);
    }

    #[test]
    fn unknown_type_falls_back_to_float() {
        let h = Harness::new::<Constant>(json!({"type": "complex"}));
        assert_eq!(h.signature().outputs[OUT].ty, SignalType::Float);
    }

    #[test]
    fn wired_value_is_sampled_once_per_block() {
        let mut h = Harness::new::<Constant>(json!({}));
        let ramp: Vec<f32> = (0..128).map(|i| i as f32).collect();
        h.set_samples("value", ramp);
        let out = h.run(128);
        assert_eq!(out.float("out")[63], 0.0);
        assert_eq!(out.float("out")[64], 64.0);
    }

    #[test]
    fn non_finite_value_becomes_zero() {
        let mut h = Harness::new::<Constant>(json!({}));
        h.set_samples("value", vec![f32::NAN; 64]);
        assert_eq!(h.run(64).float("out"), vec![0.0; 64]);
    }
}
