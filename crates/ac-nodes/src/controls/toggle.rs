use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, node,
};

node! {
    kind: "toggle",
    name: "Toggle",
    category: Controls,
    keywords: ["switch", "on off", "checkbox", "latch", "flip flop"],
    /// An on/off switch.
    ///
    /// Clicking the switch in the node (or on a panel) sets `value`, and
    /// `out` follows it. `flip` turns the switch over from a wire, so a
    /// `Button` or a `Clock` can toggle it. Whenever `value` changes, the
    /// switch follows it again, whatever `flip` did before. Right-click
    /// `value` to bind it to a key or a MIDI control.
    ///
    /// `lit` lights the switch, whichever way it's set: wire a clip's
    /// `playing` into it so a mute switch also shows its track is sounding.
    ///
    /// **Try this:** wire a `Button`'s `trigger` into `flip` for a latching
    /// footswitch, or multiply a signal by `out` with a `Math` node to
    /// switch it on and off.
    pub struct Toggle {
        state: bool,
        /// `value` as last seen; `None` until the first block.
        last_value: Option<bool>,
    }
}

const VALUE: usize = 0;
const FLIP: usize = 1;
const LIT: usize = 2;

/// Telemetry: 1 while `lit` was high in the last block.
pub const TELEMETRY_LIT: usize = 0;

const OUT: usize = 0;

impl Toggle {
    fn follow(&mut self, value: bool) {
        if self.last_value != Some(value) {
            self.state = value;
            self.last_value = Some(value);
        }
    }
}

impl Node for Toggle {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("value", false).doc("Sets the switch."))
            .input(SocketSpec::trigger("flip").doc("Turns the switch over."))
            .input(
                SocketSpec::boolean("lit", false)
                    .late()
                    .doc("Lights the switch, whichever way it's set."),
            )
            .output(
                SocketSpec::boolean("out", false)
                    .rate(Rate::Block)
                    .doc("The switch's state."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Toggle {
            state: false,
            last_value: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let value = io.inputs.boolean(VALUE);
        let flips = io.inputs.triggers(FLIP);
        if value.is_varying() {
            // Changes and flips apply in time order; at the same frame the
            // value change comes first.
            let mut pending = flips.iter().peekable();
            for i in 0..ctx.frames {
                self.follow(value.at(i));
                while pending.next_if(|&&o| o as usize <= i).is_some() {
                    self.state = !self.state;
                }
            }
            for _ in pending {
                self.state = !self.state;
            }
        } else {
            self.follow(value.first());
            if flips.len() % 2 == 1 {
                self.state = !self.state;
            }
        }
        io.outputs.set_bool(OUT, self.state);
        let lit = super::lit_level(io.inputs.boolean(LIT), ctx.frames);
        io.telemetry.set(TELEMETRY_LIT, lit);
    }

    fn reset(&mut self) {
        self.state = false;
        self.last_value = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn follows_value() {
        let mut h = Harness::new::<Toggle>(json!({}));
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));
        h.set("value", true);
        assert!(h.run(64).float("out").iter().all(|&v| v == 1.0));
        h.set("value", false);
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn flip_turns_it_over() {
        let mut h = Harness::new::<Toggle>(json!({}));
        h.triggers("flip", &[10, 100, 130, 140]);
        let out = h.run(192);
        let v = out.float("out");
        assert_eq!(v[0], 1.0);
        assert_eq!(v[64], 0.0);
        // Two flips in one block cancel out.
        assert_eq!(v[128], 0.0);
    }

    #[test]
    fn a_value_change_overrides_flips() {
        let mut h = Harness::new::<Toggle>(json!({}));
        h.triggers("flip", &[10]);
        assert_eq!(h.run(64).float("out")[0], 1.0);
        // Value stays false: the flipped state holds.
        assert_eq!(h.run(64).float("out")[0], 1.0);
        h.set("value", true);
        assert_eq!(h.run(64).float("out")[0], 1.0);
        h.set("value", false);
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }

    #[test]
    fn wired_value_and_flips_apply_in_order() {
        let mut h = Harness::new::<Toggle>(json!({}));
        // Value rises at 20 (on), then flips at 30, 40, and 50 leave it off.
        let value: Vec<f32> = (0..64).map(|i| if i >= 20 { 1.0 } else { 0.0 }).collect();
        h.set_samples("value", value);
        h.triggers("flip", &[30, 40, 50]);
        assert_eq!(h.run(64).float("out")[0], 0.0);

        // At the same frame the value change applies before the flip.
        let mut h = Harness::new::<Toggle>(json!({}));
        let value: Vec<f32> = (0..64).map(|i| if i >= 30 { 1.0 } else { 0.0 }).collect();
        h.set_samples("value", value);
        h.triggers("flip", &[30]);
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }

    #[test]
    fn reset_returns_to_value() {
        let mut h = Harness::new::<Toggle>(json!({}));
        h.triggers("flip", &[0]);
        assert_eq!(h.run(64).float("out")[0], 1.0);
        h.reset();
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }
}
