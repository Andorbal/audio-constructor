use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "readout",
    name: "Readout",
    category: Display,
    keywords: ["display", "value", "monitor", "debug"],
    /// Shows the current value of whatever is wired into it, as a number.
    ///
    /// `in` accepts a `Float`, `Int`, or `Bool` (a `Bool` reads as 0 or 1);
    /// audio shows its first channel. The node publishes the value at the
    /// last frame of each block, and the editor shows the latest one.
    ///
    /// The value is shown exactly as it arrives, including NaN or infinity,
    /// so a `Readout` is a quick way to find where a bad value starts.
    ///
    /// **Try this:** hang a `Readout` off `Transport.beat` to watch song
    /// position, or off a `MidiNote` `freq` to check what pitch a key
    /// produces.
    pub struct Readout {}
}

const IN: usize = 0;

/// Telemetry slot for the displayed value.
const VALUE: usize = 0;

impl Node for Readout {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new().input(SocketSpec::float("in", Unit::Generic, 0.0).doc("Value to show."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Readout {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let last = io.frames().saturating_sub(1);
        io.telemetry.set(VALUE, io.inputs.float(IN).at(last));
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn shows_the_last_frame() {
        let mut h = Harness::new::<Readout>(json!({}));
        let ramp: Vec<f32> = (0..100).map(|i| i as f32).collect();
        h.set_samples("in", ramp);
        h.run(64);
        assert_eq!(h.telemetry(VALUE), 63.0);
        // A short final block still reports its own last frame.
        h.run(36);
        assert_eq!(h.telemetry(VALUE), 99.0);
    }

    #[test]
    fn shows_unwired_value() {
        let mut h = Harness::new::<Readout>(json!({}));
        h.set("in", -2.5);
        h.run(64);
        assert_eq!(h.telemetry(VALUE), -2.5);
    }

    #[test]
    fn ints_arrive_as_floats() {
        let mut h = Harness::new::<Readout>(json!({}));
        h.set_int("in", 42);
        h.run(64);
        assert_eq!(h.telemetry(VALUE), 42.0);
    }

    #[test]
    fn shows_non_finite_values_as_they_are() {
        let mut h = Harness::new::<Readout>(json!({}));
        h.set_samples("in", vec![f32::NAN; 64]);
        h.run(64);
        assert!(h.telemetry(VALUE).is_nan());
    }
}
