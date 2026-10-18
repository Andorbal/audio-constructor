use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::finite_or_zero;

node! {
    kind: "clamp",
    name: "Clamp",
    category: Math,
    keywords: ["limit", "bound", "constrain", "range", "clip", "saturate"],
    /// Keeps `in` between `min` and `max`.
    ///
    /// Values below `min` come out as `min`, values above `max` come out as
    /// `max`, and anything in between passes unchanged. If `min` is above
    /// `max` the two are swapped, so the range always works whichever way
    /// round it is set.
    ///
    /// The output runs at the fastest rate among the inputs: block rate when
    /// all three are controls, sample rate when any carries audio-rate
    /// modulation. Audio wired into an input arrives as its first channel.
    /// A NaN or infinite input counts as 0, so the output is always finite.
    ///
    /// **Try this:** add an `Oscillator` to a `Slider` with `Math` and clamp
    /// the sum to 200..5000 before a `Biquad` cutoff, so deep modulation can
    /// never push the filter out of its useful range. With `min` −0.5 and
    /// `max` 0.5 on an audio-rate signal it is a hard clipper.
    pub struct Clamp {}
}

const IN: usize = 0;
const MIN: usize = 1;
const MAX: usize = 2;
const OUT: usize = 0;

impl Node for Clamp {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The value to limit."))
            .input(SocketSpec::float("min", Unit::Generic, 0.0).doc("Lowest value let through."))
            .input(SocketSpec::float("max", Unit::Generic, 1.0).doc("Highest value let through."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .poly()
                    .doc("`in`, kept within the range."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Clamp {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let lo = io.inputs.float(MIN);
        let hi = io.inputs.float(MAX);
        io.outputs.float(OUT).fill_with(|i| {
            clamp(
                finite_or_zero(x.at(i)),
                finite_or_zero(lo.at(i)),
                finite_or_zero(hi.at(i)),
            )
        });
    }

    fn reset(&mut self) {}
}

/// `x` within `a..b` or `b..a`, whichever is the right way round.
#[inline]
fn clamp(x: f32, a: f32, b: f32) -> f32 {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    x.clamp(lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn eval(x: f32, lo: f32, hi: f32) -> f32 {
        let mut h = Harness::new::<Clamp>(json!({}));
        h.set("in", x).set("min", lo).set("max", hi);
        h.run(1).float("out")[0]
    }

    #[test]
    fn limits_to_the_range() {
        assert_eq!(eval(0.5, 0.0, 1.0), 0.5);
        assert_eq!(eval(-3.0, 0.0, 1.0), 0.0);
        assert_eq!(eval(7.0, 0.0, 1.0), 1.0);
        assert_eq!(eval(250.0, 200.0, 5000.0), 250.0);
    }

    #[test]
    fn reversed_range_is_swapped() {
        assert_eq!(eval(7.0, 1.0, 0.0), 1.0);
        assert_eq!(eval(-7.0, 1.0, 0.0), 0.0);
        assert_eq!(eval(0.25, 1.0, 0.0), 0.25);
        assert_eq!(eval(9.0, 3.0, 3.0), 3.0);
    }

    #[test]
    fn non_finite_inputs_count_as_zero() {
        let mut h = Harness::new::<Clamp>(json!({}));
        h.set_samples("in", vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5]);
        h.set("min", -1.0).set("max", 1.0);
        assert_eq!(h.run(4).float("out"), vec![0.0, 0.0, 0.0, 0.5]);
        // A NaN bound counts as 0 too.
        let mut h = Harness::new::<Clamp>(json!({}));
        h.set("in", 5.0)
            .set_samples("max", vec![f32::NAN])
            .set("min", -1.0);
        assert_eq!(h.run(1).float("out")[0], 0.0);
    }

    #[test]
    fn rate_follows_the_fastest_input() {
        let ramp: Vec<f32> = (0..128).map(|i| i as f32 / 64.0).collect();
        let mut h = Harness::new::<Clamp>(json!({}));
        h.set_block("in", ramp.clone());
        let out = h.run(128);
        // Block rate: frame 70 shows the block's first value, 1.0.
        assert_eq!(out.float("out")[70], 1.0);
        assert_eq!(out.float("out")[10], 0.0);

        let mut h = Harness::new::<Clamp>(json!({}));
        h.set_samples("max", ramp.clone()).set("in", 10.0);
        let out = h.run(128);
        assert_eq!(out.float("out")[10], ramp[10]);
        assert_eq!(out.float("out")[127], ramp[127]);
    }

    #[test]
    fn same_output_at_any_block_size() {
        let x: Vec<f32> = (0..1_000)
            .map(|i| ((i as f32) * 0.37).sin() * 2.0)
            .collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Clamp>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", x.clone())
                .set("min", -0.5)
                .set("max", 0.75);
            h.run(1_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
