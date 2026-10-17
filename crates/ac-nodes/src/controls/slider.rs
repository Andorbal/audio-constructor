use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};

node! {
    kind: "slider",
    name: "Slider",
    category: Controls,
    keywords: ["fader", "knob", "parameter", "control"],
    /// A value you set by hand, with a range, a unit, and a linear or log
    /// scale.
    ///
    /// The `min`, `max`, `unit`, and `scale` config set the slider up: a
    /// frequency from 20 Hz to 20 kHz on a log scale, a level in dB, a time
    /// in beats. `out` is `value` kept within `min`..`max`. A log scale
    /// needs a positive range; otherwise the slider is linear. Right-click
    /// `value` to bind it to a MIDI controller or a gamepad axis. `value`
    /// can also be wired, which makes the slider a clamp with a readout.
    ///
    /// **Try this:** make a log slider from 20 to 20 000 Hz, wire `out` into
    /// a `Biquad`'s `cutoff`, and pin it to a panel; or wire one dB slider
    /// into several `Gain` nodes as a group fader.
    pub struct Slider {
        min: f32,
        max: f32,
    }
}

const VALUE: usize = 0;
const OUT: usize = 0;

/// The slider's setup, repaired: finite, `min <= max`, within `f32`.
struct Setup {
    min: f64,
    max: f64,
    unit: Unit,
    log: bool,
}

fn setup(config: &Config) -> Setup {
    let limit = f64::from(f32::MAX);
    let a = config.get_f64_or("min", 0.0).clamp(-limit, limit);
    let b = config.get_f64_or("max", 1.0).clamp(-limit, limit);
    let (min, max) = if a <= b { (a, b) } else { (b, a) };
    let unit_name = config.get_str_or("unit", "generic");
    let unit = serde_json::from_value(serde_json::Value::from(unit_name)).unwrap_or_default();
    let log = config.get_str_or("scale", "linear") == "log" && min > 0.0;
    Setup {
        min,
        max,
        unit,
        log,
    }
}

impl Node for Slider {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let s = setup(config);
        let mut value = SocketSpec::float("value", s.unit, s.min)
            .range(s.min, s.max)
            .doc("The slider's position.");
        if s.log {
            value = value.log();
        }
        Signature::new().input(value).output(
            SocketSpec::float("out", s.unit, s.min)
                .poly()
                .doc("`value`, kept within the range."),
        )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let s = setup(config);
        Slider {
            min: s.min as f32,
            max: s.max as f32,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let value = io.inputs.float(VALUE);
        let (min, max) = (self.min, self.max);
        // `max`/`min` return the other operand for NaN, so NaN reads as `min`.
        io.outputs
            .float(OUT)
            .fill_with(|i| value.at(i).max(min).min(max));
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{OutRate, Scale, Value};
    use serde_json::json;

    #[test]
    fn passes_the_value_through() {
        let mut h = Harness::new::<Slider>(json!({}));
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));
        h.set("value", 0.25);
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.25));
    }

    #[test]
    fn clamps_to_the_range() {
        let mut h = Harness::new::<Slider>(json!({"min": -1.0, "max": 1.0}));
        h.set("value", 2.0);
        assert_eq!(h.run(64).float("out")[0], 1.0);
        h.set("value", -5.0);
        assert_eq!(h.run(64).float("out")[0], -1.0);

        let mut h = Harness::new::<Slider>(json!({"min": 20.0, "max": 20_000.0}));
        h.set_samples(
            "value",
            vec![f32::NAN, f32::INFINITY, 440.0, f32::NEG_INFINITY],
        );
        assert_eq!(&h.run(4).float("out")[..4], &[20.0, 20_000.0, 440.0, 20.0]);
    }

    #[test]
    fn defaults_to_min() {
        let mut h = Harness::new::<Slider>(json!({"min": 20.0, "max": 20_000.0}));
        assert!(h.run(64).float("out").iter().all(|&v| v == 20.0));
    }

    #[test]
    fn follows_a_wired_signal_per_sample() {
        let mut h = Harness::new::<Slider>(json!({}));
        let ramp: Vec<f32> = (0..64).map(|i| i as f32 / 32.0 - 0.5).collect();
        h.set_samples("value", ramp.clone());
        let out = h.run(64);
        let expected: Vec<f32> = ramp.iter().map(|v| v.clamp(0.0, 1.0)).collect();
        assert_eq!(out.float("out"), expected.as_slice());
        assert_eq!(
            Slider::describe(&json!({})).outputs[OUT].rate,
            OutRate::Poly
        );
    }

    #[test]
    fn config_sets_unit_range_and_scale() {
        let sig = Slider::describe(&json!({"min": 20, "max": 20000, "unit": "hz", "scale": "log"}));
        let v = &sig.inputs[VALUE];
        assert_eq!(v.unit, Unit::Hz);
        assert_eq!(v.range, Some((20.0, 20_000.0)));
        assert_eq!(v.scale, Scale::Log);
        assert_eq!(v.default, Value::Float(20.0));
        assert_eq!(sig.outputs[OUT].unit, Unit::Hz);

        let sig = Slider::describe(&json!({"unit": "semitones"}));
        assert_eq!(sig.inputs[VALUE].unit, Unit::Semitones);
        assert_eq!(sig.inputs[VALUE].scale, Scale::Linear);
        assert_eq!(sig.inputs[VALUE].range, Some((0.0, 1.0)));
    }

    #[test]
    fn bad_config_is_repaired() {
        let sig = Slider::describe(&json!({"unit": "parsecs", "min": 5, "max": -5}));
        assert_eq!(sig.inputs[VALUE].unit, Unit::Generic);
        assert_eq!(sig.inputs[VALUE].range, Some((-5.0, 5.0)));
        // A log scale can't include zero or negatives.
        let sig = Slider::describe(&json!({"min": 0, "max": 10, "scale": "log"}));
        assert_eq!(sig.inputs[VALUE].scale, Scale::Linear);
        let sig = Slider::describe(&json!({"min": "low", "max": 1e300}));
        assert_eq!(sig.inputs[VALUE].range, Some((0.0, f64::from(f32::MAX))));
    }
}
