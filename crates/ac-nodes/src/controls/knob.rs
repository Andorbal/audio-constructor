use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, node};

use super::{Ranged, clamp_through};

node! {
    kind: "knob",
    name: "Knob",
    category: Controls,
    keywords: ["dial", "pot", "rotary", "encoder", "parameter", "control"],
    /// A round dial for a value you set by hand, with a range, a unit, and
    /// a linear or log scale.
    ///
    /// It works like `Slider` in a smaller space. The `min`, `max`, `unit`,
    /// and `scale` config set the knob up: a frequency from 20 Hz to 20 kHz
    /// on a log scale, a level in dB, a time in beats. `out` is `value`
    /// kept within `min`..`max` (NaN reads as `min`). A log scale needs a
    /// positive range; otherwise the knob is linear. A reversed range is
    /// swapped and an unknown unit shows no unit. Right-click `value` to
    /// bind it to a MIDI controller or a gamepad axis. `value` can also be
    /// wired, which makes the knob a clamp with a readout.
    ///
    /// **Try this:** pin a row of knobs to a panel for a synth's cutoff,
    /// resonance, and envelope times, or set `unit` to semitones with a
    /// range of −12 to 12 for a detune control.
    pub struct Knob {
        min: f32,
        max: f32,
    }
}

const VALUE: usize = 0;
const OUT: usize = 0;

impl Node for Knob {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        Ranged::from_config(config).signature("The knob's position.")
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let r = Ranged::from_config(config);
        Knob {
            min: r.min as f32,
            max: r.max as f32,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        clamp_through(io, VALUE, OUT, self.min, self.max);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{OutRate, Scale, Unit, Value};
    use serde_json::json;

    #[test]
    fn passes_the_value_through_within_the_range() {
        let mut h = Harness::new::<Knob>(json!({"min": -12, "max": 12, "unit": "semitones"}));
        assert!(h.run(64).float("out").iter().all(|&v| v == -12.0));
        h.set("value", 3.5);
        assert!(h.run(64).float("out").iter().all(|&v| v == 3.5));
        h.set("value", 40.0);
        assert_eq!(h.run(64).float("out")[0], 12.0);
        let mut h = Harness::new::<Knob>(json!({"min": -12, "max": 12}));
        h.set_samples(
            "value",
            vec![f32::NAN, f32::INFINITY, 1.0, f32::NEG_INFINITY],
        );
        assert_eq!(&h.run(4).float("out")[..4], &[-12.0, 12.0, 1.0, -12.0]);
    }

    #[test]
    fn config_sets_unit_range_and_scale() {
        let sig = Knob::describe(&json!({"min": 20, "max": 20000, "unit": "hz", "scale": "log"}));
        let v = &sig.inputs[VALUE];
        assert_eq!(v.unit, Unit::Hz);
        assert_eq!(v.range, Some((20.0, 20_000.0)));
        assert_eq!(v.scale, Scale::Log);
        assert_eq!(v.default, Value::Float(20.0));
        assert_eq!(sig.outputs[OUT].unit, Unit::Hz);
        assert_eq!(sig.outputs[OUT].rate, OutRate::Poly);
    }

    #[test]
    fn bad_config_is_repaired() {
        let sig = Knob::describe(&json!({"unit": "furlongs", "min": 5, "max": -5, "scale": "log"}));
        assert_eq!(sig.inputs[VALUE].unit, Unit::Generic);
        assert_eq!(sig.inputs[VALUE].range, Some((-5.0, 5.0)));
        assert_eq!(sig.inputs[VALUE].scale, Scale::Linear);
        let sig = Knob::describe(&json!({"min": [1], "max": -1e300}));
        assert_eq!(sig.inputs[VALUE].range, Some((-f64::from(f32::MAX), 0.0)));
    }
}
