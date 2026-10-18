use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::finite_or_zero;

node! {
    kind: "quantize",
    name: "Quantize",
    category: Math,
    keywords: ["round", "snap", "step", "grid", "bitcrush", "staircase"],
    /// Rounds `in` to the nearest step of a grid: multiples of `step`,
    /// shifted by `offset`.
    ///
    /// The grid is `offset + k × step` for every whole number k, and `out`
    /// is the grid point nearest to `in`; a value exactly halfway between two
    /// points rounds up. A negative `step` counts as positive, and a `step`
    /// of 0 passes `in` through unchanged.
    ///
    /// The output runs at the fastest rate among the inputs and is always
    /// finite: a NaN or infinite input counts as 0.
    ///
    /// **Try this:** quantize a `Slider` (or an `LFO` scaled to 48..72) with
    /// `step` 1 and feed it through `Convert` (`note_to_hz`) into an
    /// `Oscillator`: it now plays only semitones. On audio, a `step` of 0.1
    /// is a crude bit-crusher.
    pub struct Quantize {}
}

const IN: usize = 0;
const STEP: usize = 1;
const OFFSET: usize = 2;
const OUT: usize = 0;

impl Node for Quantize {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The value to round."))
            .input(
                SocketSpec::float("step", Unit::Generic, 1.0)
                    .range(0.0, 12.0)
                    .doc("Grid spacing (0 passes through)."),
            )
            .input(
                SocketSpec::float("offset", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("Shifts the grid."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("The nearest grid point."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Quantize {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let step = io.inputs.float(STEP);
        let offset = io.inputs.float(OFFSET);
        io.outputs.float(OUT).fill_with(|i| {
            quantize(
                finite_or_zero(x.at(i)),
                finite_or_zero(step.at(i)),
                finite_or_zero(offset.at(i)),
            )
        });
    }

    fn reset(&mut self) {}
}

#[inline]
fn quantize(x: f32, step: f32, offset: f32) -> f32 {
    if step == 0.0 {
        return x;
    }
    let s = f64::from(step.abs());
    let o = f64::from(offset);
    let q = (f64::from(x) - o) / s;
    if !q.is_finite() {
        // A step so small the grid is finer than any f32: nothing to round.
        return x;
    }
    finite_or_zero(((q + 0.5).floor() * s + o) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn eval(x: f32, step: f32, offset: f32) -> f32 {
        let mut h = Harness::new::<Quantize>(json!({}));
        h.set("in", x).set("step", step).set("offset", offset);
        h.run(1).float("out")[0]
    }

    #[test]
    fn rounds_to_the_nearest_step() {
        assert_eq!(eval(0.4, 1.0, 0.0), 0.0);
        assert_eq!(eval(0.6, 1.0, 0.0), 1.0);
        assert_eq!(eval(-0.6, 1.0, 0.0), -1.0);
        assert_eq!(eval(7.3, 0.5, 0.0), 7.5);
        assert_eq!(eval(61.7, 1.0, 0.0), 62.0);
        assert_eq!(eval(0.26, 0.25, 0.0), 0.25);
    }

    #[test]
    fn halfway_rounds_up() {
        assert_eq!(eval(0.5, 1.0, 0.0), 1.0);
        assert_eq!(eval(-0.5, 1.0, 0.0), 0.0);
        assert_eq!(eval(1.5, 1.0, 0.0), 2.0);
    }

    #[test]
    fn offset_shifts_the_grid() {
        assert_eq!(eval(0.0, 1.0, 0.5), 0.5);
        assert_eq!(eval(1.2, 1.0, 0.25), 1.25);
        assert_eq!(eval(-0.3, 1.0, 0.25), -0.75);
    }

    #[test]
    fn zero_step_passes_through_and_negative_step_counts_as_positive() {
        assert_eq!(eval(0.37, 0.0, 0.5), 0.37);
        assert_eq!(eval(7.3, -0.5, 0.0), 7.5);
    }

    #[test]
    fn extreme_and_non_finite_inputs() {
        assert_eq!(eval(1.0e30, 1e-30, 0.0), 1.0e30);
        assert!(eval(3.0e38, 0.1, 0.0).is_finite());
        let mut h = Harness::new::<Quantize>(json!({}));
        h.set_samples("in", vec![f32::NAN, f32::INFINITY, 2.2]);
        h.set_samples("step", vec![1.0, 1.0, f32::NAN]);
        assert_eq!(h.run(3).float("out"), vec![0.0, 0.0, 2.2]);
    }

    #[test]
    fn rate_follows_the_fastest_input_and_ignores_block_size() {
        let x: Vec<f32> = (0..700).map(|i| (i as f32 * 0.01).sin() * 3.0).collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Quantize>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", x.clone()).set("step", 0.5);
            h.run(700).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one[300], quantize(x[300], 0.5, 0.0));
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));

        let mut h = Harness::new::<Quantize>(json!({}));
        h.set_block("in", x.clone()).set("step", 0.5);
        let out = h.run(128);
        assert_eq!(out.float("out")[100], quantize(x[64], 0.5, 0.0));
    }
}
