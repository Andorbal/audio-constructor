use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

use super::finite_or_zero;
use crate::segments::Runs;

node! {
    kind: "sample_hold",
    name: "SampleHold",
    category: Math,
    keywords: ["sample and hold", "s&h", "latch", "freeze", "snapshot", "track"],
    /// Samples `in` at each `trigger` and holds that value until the next.
    ///
    /// The value is read on the trigger's exact sample, so an audio-rate
    /// `in` is caught precisely; the output is a sample-rate signal that
    /// only changes at triggers. Before the first trigger (and after a
    /// transport restart) it holds 0. A NaN or infinite sample is held
    /// as 0.
    ///
    /// **Try this:** sample a slow `Oscillator` (or `RandomWalk`) on every
    /// `Clock` tick and wire the result into a `Biquad` cutoff for a
    /// stepped filter pattern, or sample `Transport.time` on a `Button`
    /// press to remember when it was pressed.
    pub struct SampleHold {
        held: f32,
    }
}

const IN: usize = 0;
const TRIGGER: usize = 1;
const OUT: usize = 0;

impl Node for SampleHold {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The signal to sample."))
            .input(SocketSpec::trigger("trigger").doc("Take a sample."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The last sample taken."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        SampleHold { held: 0.0 }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let triggers = io.inputs.triggers(TRIGGER);
        let mut out = io.outputs.float(OUT);
        for run in Runs::new([triggers], ctx.frames) {
            if run.fired[0] {
                self.held = finite_or_zero(x.at(run.at));
            }
            for i in run.at..run.until {
                out.set(i, self.held);
            }
        }
    }

    fn reset(&mut self) {
        self.held = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn samples_on_the_exact_trigger_sample() {
        let mut h = Harness::new::<SampleHold>(json!({}));
        h.set_samples("in", ramp(300))
            .triggers("trigger", &[7, 64, 200]);
        let o = h.run(300).float("out").to_vec();
        assert!(o[..7].iter().all(|&v| v == 0.0));
        assert!(o[7..64].iter().all(|&v| v == 7.0));
        assert!(o[64..200].iter().all(|&v| v == 64.0));
        assert!(o[200..].iter().all(|&v| v == 200.0));
    }

    #[test]
    fn same_output_at_any_block_size() {
        let at = [0, 1, 63, 64, 100, 511, 512, 513, 999];
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<SampleHold>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", ramp(1_000)).triggers("trigger", &at);
            h.run(1_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one[700], 513.0);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn block_rate_input_is_read_at_the_trigger() {
        let mut h = Harness::new::<SampleHold>(json!({}));
        h.set_block("in", ramp(128)).triggers("trigger", &[70]);
        let o = h.run(128).float("out").to_vec();
        assert_eq!(o[127], 64.0);
    }

    #[test]
    fn non_finite_samples_hold_zero_and_reset_clears() {
        let mut h = Harness::new::<SampleHold>(json!({}));
        h.set_samples("in", vec![f32::NAN; 64])
            .triggers("trigger", &[3]);
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));

        let mut h = Harness::new::<SampleHold>(json!({}));
        h.set("in", 4.0).triggers("trigger", &[0]);
        assert_eq!(h.run(64).float("out")[63], 4.0);
        h.reset();
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }
}
