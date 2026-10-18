use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::button::for_each_edge;
use super::{bounds, clamp_through};

node! {
    kind: "xy_pad",
    name: "XY Pad",
    category: Controls,
    keywords: ["xy", "pad", "touch", "joystick", "2d", "kaoss", "surface", "trackpad"],
    /// A square pad you drag a point around on: two values at once, plus a
    /// gate while you touch it.
    ///
    /// Dragging the point in the node (or on a panel) sets `x` and `y`, and
    /// holding it down sets `touched`. The `x` and `y` outputs are the
    /// inputs kept within the ranges from the `x_min`, `x_max`, `y_min`, and
    /// `y_max` config (0 to 1 by default; a reversed range is swapped, and
    /// NaN reads as the low end). The point starts in the middle. `gate`
    /// follows `touched`, and `touch` fires when the pad is pressed, on the
    /// exact sample when `touched` is wired at audio rate and at the start
    /// of the block otherwise. The inputs can be wired too, which turns the
    /// pad into a two-way clamp with a picture of where the signals are.
    ///
    /// **Try this:** wire `x` into a `Biquad`'s `cutoff` (set `x_min` 200
    /// and `x_max` 8000) and `y` into its `q`, and `gate` into an `ADSR` on
    /// the output level, so touching the pad plays and shapes the sound at
    /// once.
    pub struct XyPad {
        x: (f32, f32),
        y: (f32, f32),
        /// `touched` at the end of the last block.
        touched: bool,
    }
}

const X: usize = 0;
const Y: usize = 1;
const TOUCHED: usize = 2;

const X_OUT: usize = 0;
const Y_OUT: usize = 1;
const GATE: usize = 2;
const TOUCH: usize = 3;

fn ranges(config: &Config) -> ((f64, f64), (f64, f64)) {
    (
        bounds(config, "x_min", "x_max", (0.0, 1.0)),
        bounds(config, "y_min", "y_max", (0.0, 1.0)),
    )
}

/// The middle of a range, as the pad's starting point.
fn middle((lo, hi): (f64, f64)) -> f64 {
    lo + (hi - lo) / 2.0
}

impl Node for XyPad {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let (x, y) = ranges(config);
        Signature::new()
            .input(
                SocketSpec::float("x", Unit::Generic, middle(x))
                    .range(x.0, x.1)
                    .doc("The point's horizontal position."),
            )
            .input(
                SocketSpec::float("y", Unit::Generic, middle(y))
                    .range(y.0, y.1)
                    .doc("The point's vertical position."),
            )
            .input(SocketSpec::boolean("touched", false).doc("True while the pad is held."))
            .output(
                SocketSpec::float("x", Unit::Generic, middle(x))
                    .poly()
                    .doc("`x`, kept within its range."),
            )
            .output(
                SocketSpec::float("y", Unit::Generic, middle(y))
                    .poly()
                    .doc("`y`, kept within its range."),
            )
            .output(
                SocketSpec::boolean("gate", false)
                    .poly()
                    .doc("High while the pad is held."),
            )
            .output(SocketSpec::trigger("touch").doc("Fires when the pad is pressed."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (x, y) = ranges(config);
        XyPad {
            x: (x.0 as f32, x.1 as f32),
            y: (y.0 as f32, y.1 as f32),
            touched: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        clamp_through(io, X, X_OUT, self.x.0, self.x.1);
        clamp_through(io, Y, Y_OUT, self.y.0, self.y.1);
        let touched = io.inputs.boolean(TOUCHED);
        let (mut gate, mut touch) = io.outputs.float_trigger(GATE, TOUCH);
        gate.fill_with(|i| if touched.at(i) { 1.0 } else { 0.0 });
        self.touched = for_each_edge(touched, self.touched, ctx.frames, |i, down| {
            if down {
                touch.push(i);
            }
        });
    }

    fn reset(&mut self) {
        self.touched = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Value;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn outputs_the_point_within_the_ranges() {
        let mut h =
            Harness::new::<XyPad>(json!({"x_min": 200, "x_max": 8000, "y_min": -1, "y_max": 1}));
        let out = h.run(64);
        assert!(out.float("x").iter().all(|&v| v == 4_100.0));
        assert!(out.float("y").iter().all(|&v| v == 0.0));
        h.set("x", 50.0).set("y", 0.25);
        let out = h.run(64);
        assert_eq!(out.float("x")[0], 200.0);
        assert_eq!(out.float("y")[0], 0.25);
        h.set_samples("x", vec![f32::NAN; 64]);
        h.set("y", 9.0);
        let out = h.run(64);
        assert_eq!(out.float("x")[0], 200.0);
        assert_eq!(out.float("y")[0], 1.0);
    }

    #[test]
    fn gate_follows_touched_and_touch_fires_on_press() {
        let mut h = Harness::new::<XyPad>(json!({}));
        let touched: Vec<f32> = (0..256)
            .map(|i| {
                if (10..100).contains(&i) || (150..300).contains(&i) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        h.set_samples("touched", touched.clone());
        let out = h.run(256);
        assert_eq!(out.triggers("touch"), &[10, 150]);
        assert_eq!(out.float("gate"), touched.as_slice());
        // A held press spans blocks without firing again; widget presses
        // land at the block start.
        let mut h = Harness::new::<XyPad>(json!({}));
        h.set("touched", true);
        assert_eq!(h.run(192).triggers("touch"), &[0]);
        h.reset();
        assert_eq!(h.run(64).triggers("touch"), &[192]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let touched: Vec<f32> = (0..3_000)
            .map(|i| ((i / 37) % 3 == 0) as u8 as f32)
            .collect();
        let x: Vec<f32> = (0..3_000).map(|i| (i as f32 * 0.01).sin() * 2.0).collect();
        let run = |block| {
            let mut h =
                Harness::with_resources::<XyPad>(json!({}), 48_000.0, block, &ac_core::NoResources);
            h.set_samples("touched", touched.clone())
                .set_samples("x", x.clone());
            let out = h.run(3_000);
            (
                out.triggers("touch").to_vec(),
                out.float("gate").to_vec(),
                out.float("x").to_vec(),
            )
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.0.len() > 20);
    }

    #[test]
    fn bad_config_is_repaired() {
        let sig = XyPad::describe(&json!({"x_min": 10, "x_max": -10, "y_min": "low"}));
        assert_eq!(sig.inputs[X].range, Some((-10.0, 10.0)));
        assert_eq!(sig.inputs[Y].range, Some((0.0, 1.0)));
        assert_eq!(sig.inputs[X].default, Value::Float(0.0));
        let sig = XyPad::describe(&json!({"x_min": -1e300, "x_max": 1e300}));
        assert_eq!(sig.inputs[X].default, Value::Float(0.0));
        assert_eq!(sig.outputs[X_OUT].name, "x");
    }
}
