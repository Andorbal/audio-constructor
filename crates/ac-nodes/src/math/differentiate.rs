use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::{finite_f32, finite_or_zero};

node! {
    kind: "differentiate",
    name: "Differentiate",
    category: Math,
    keywords: ["derivative", "slope", "rate of change", "velocity", "difference", "delta"],
    /// How fast `in` is changing, in units per second.
    ///
    /// A ramp rising by 2 every second gives 2; a steady value gives 0; a
    /// falling value gives a negative rate. At sample rate it is the
    /// difference between this sample and the last, times the sample rate.
    ///
    /// The output runs at the rate of `in`. When `in` changes once per
    /// block, the rate is the change since the previous block divided by
    /// that block's length, so it reads correctly at any block size (but
    /// stays one block behind the change). The first value after loading or
    /// a transport restart has nothing to compare with and gives 0.
    ///
    /// A NaN or infinite input counts as 0, and a rate too large to
    /// represent is 0. Steps in the input give spikes in the output: a jump
    /// of 1 in one sample reads as 48 000 per second at 48 kHz.
    ///
    /// **Try this:** differentiate a `Slider` and take `Unary` (`abs`) of
    /// the result to get how fast it is being moved, then `Compare` it with
    /// a threshold to fire on quick gestures. Differentiating an `ADSR`
    /// gives a positive bump on each attack and a negative one on each
    /// release.
    pub struct Differentiate {
        prev: f64,
        prev_frames: usize,
        primed: bool,
    }
}

const IN: usize = 0;
const OUT: usize = 0;

impl Node for Differentiate {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The changing value."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("Rate of change per second."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Differentiate {
            prev: 0.0,
            prev_frames: 0,
            primed: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let rate = ctx.sample_rate;
        let mut out = io.outputs.float(OUT);
        if out.is_sample() {
            out.fill_with(|i| {
                let v = f64::from(finite_or_zero(x.at(i)));
                let d = if self.primed {
                    (v - self.prev) * rate
                } else {
                    0.0
                };
                self.prev = v;
                self.primed = true;
                finite_f32(d)
            });
        } else {
            let v = f64::from(finite_or_zero(x.first()));
            let d = if self.primed && self.prev_frames > 0 {
                (v - self.prev) * rate / self.prev_frames as f64
            } else {
                0.0
            };
            self.prev = v;
            self.primed = true;
            out.fill(finite_f32(d));
        }
        self.prev_frames = ctx.frames;
    }

    fn reset(&mut self) {
        self.prev = 0.0;
        self.prev_frames = 0;
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn ramp_gives_its_slope() {
        // Rises 2 per second.
        let x: Vec<f32> = (0..1_000).map(|i| i as f32 * 2.0 / 48_000.0).collect();
        let mut h = Harness::new::<Differentiate>(json!({}));
        h.set_samples("in", x);
        let o = h.run(1_000).float("out").to_vec();
        assert_eq!(o[0], 0.0, "nothing to compare the first sample with");
        assert!(
            o[1..].iter().all(|&d| (d - 2.0).abs() < 1e-2),
            "{:?}",
            &o[..4]
        );
    }

    #[test]
    fn steady_is_zero_and_steps_spike() {
        let mut x = vec![0.5; 200];
        x[100..].fill(1.5);
        let mut h = Harness::new::<Differentiate>(json!({}));
        h.set_samples("in", x);
        let o = h.run(200).float("out").to_vec();
        assert!(o[..100].iter().all(|&d| d == 0.0));
        assert_eq!(o[100], 48_000.0);
        assert!(o[101..].iter().all(|&d| d == 0.0));
    }

    #[test]
    fn block_rate_divides_by_the_block_length() {
        // One value per 64-frame block, rising 64 per block = 48 000 per second.
        let x: Vec<f32> = (0..256).map(|i| (i / 64 * 64) as f32).collect();
        let mut h = Harness::new::<Differentiate>(json!({}));
        h.set_block("in", x);
        let o = h.run(256).float("out").to_vec();
        assert_eq!(o[0], 0.0);
        assert_eq!(o[64], 48_000.0);
        assert_eq!(o[255], 48_000.0);
    }

    #[test]
    fn same_output_at_any_block_size_for_sample_rate_input() {
        let x: Vec<f32> = (0..2_000).map(|i| ((i as f32) * 0.01).sin()).collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Differentiate>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", x.clone());
            h.run(2_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn wild_inputs_stay_finite_and_reset_forgets() {
        let mut h = Harness::new::<Differentiate>(json!({}));
        h.set_samples("in", vec![f32::NAN, 3e38, -3e38, f32::INFINITY, 1.0]);
        let o = h.run(5).float("out").to_vec();
        assert!(o.iter().all(|v| v.is_finite()));
        assert_eq!(o[1], 0.0, "too large to represent");

        let mut h = Harness::new::<Differentiate>(json!({}));
        h.set("in", 3.0);
        h.run(64);
        h.reset();
        h.set("in", 10.0);
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }
}
