use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

use super::{finite_f32, finite_or_zero};
use crate::segments::Runs;

node! {
    kind: "accumulate",
    name: "Accumulate",
    category: Math,
    keywords: ["add", "sum", "total", "increment", "counter", "running total"],
    /// Adds `in` to a running total on every `trigger`.
    ///
    /// `out` is `initial` plus every value of `in` added since the start or
    /// the last `reset`. `in` is read on the trigger's exact sample, and the
    /// total changes there (the output is a sample-rate signal). Since
    /// `initial` is added live, changing it shifts the whole total.
    ///
    /// `reset` clears the total, so `out` returns to `initial`. When `reset`
    /// and `trigger` land on the same sample the reset happens first, and
    /// the trigger's value starts the new total. A transport restart clears
    /// the total too.
    ///
    /// NaN or infinite inputs count as 0, and the total stays finite.
    ///
    /// **Try this:** add 0.1 on every `Clock` tick and `reset` every bar
    /// for a pitch that climbs in steps, or add a `Random` value (−1..1) on
    /// each tick for a melody that wanders as a random walk.
    pub struct Accumulate {
        sum: f64,
    }
}

const IN: usize = 0;
const INITIAL: usize = 1;
const TRIGGER: usize = 2;
const RESET: usize = 3;
const OUT: usize = 0;

/// Keeps the total within the `f32` range.
const LIMIT: f64 = 1.0e38;

impl Node for Accumulate {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("in", Unit::Generic, 1.0)
                    .range(-1.0, 1.0)
                    .doc("Added on each trigger."),
            )
            .input(SocketSpec::float("initial", Unit::Generic, 0.0).doc("Where the total starts."))
            .input(SocketSpec::trigger("trigger").doc("Add `in` to the total."))
            .input(SocketSpec::trigger("reset").doc("Back to `initial`."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("`initial` plus the total."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Accumulate { sum: 0.0 }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let initial = io.inputs.float(INITIAL);
        let events = [io.inputs.triggers(RESET), io.inputs.triggers(TRIGGER)];
        let mut out = io.outputs.float(OUT);
        for run in Runs::new(events, ctx.frames) {
            let [reset, trigger] = run.fired;
            if reset {
                self.sum = 0.0;
            }
            if trigger {
                let add = f64::from(finite_or_zero(x.at(run.at)));
                self.sum = (self.sum + add).clamp(-LIMIT, LIMIT);
            }
            for i in run.at..run.until {
                out.set(
                    i,
                    finite_f32(f64::from(finite_or_zero(initial.at(i))) + self.sum),
                );
            }
        }
    }

    fn reset(&mut self) {
        self.sum = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn adds_on_each_trigger_sample() {
        let mut h = Harness::new::<Accumulate>(json!({}));
        h.set("in", 0.25)
            .set("initial", 1.0)
            .triggers("trigger", &[10, 20, 100]);
        let o = h.run(128).float("out").to_vec();
        assert!(o[..10].iter().all(|&v| v == 1.0));
        assert!(o[10..20].iter().all(|&v| v == 1.25));
        assert!(o[20..100].iter().all(|&v| v == 1.5));
        assert_eq!(o[127], 1.75);
    }

    #[test]
    fn reset_comes_first_on_a_shared_sample() {
        let mut h = Harness::new::<Accumulate>(json!({}));
        h.triggers("trigger", &[0, 5, 9])
            .triggers("reset", &[5, 30]);
        let o = h.run(40).float("out").to_vec();
        assert_eq!(o[4], 1.0);
        assert_eq!(o[5], 1.0, "reset, then the trigger adds 1");
        assert_eq!(o[9], 2.0);
        assert_eq!(o[30], 0.0);
    }

    #[test]
    fn reads_in_at_the_trigger_and_initial_live() {
        let x: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let init: Vec<f32> = (0..128).map(|i| if i < 64 { 0.0 } else { 100.0 }).collect();
        let mut h = Harness::new::<Accumulate>(json!({}));
        h.set_samples("in", x)
            .set_block("initial", init)
            .triggers("trigger", &[3, 7]);
        let o = h.run(128).float("out").to_vec();
        assert_eq!(o[10], 10.0);
        assert_eq!(o[64], 110.0);
    }

    #[test]
    fn same_output_at_any_block_size() {
        let at: Vec<u64> = (0..200).map(|k| k * 5 + k % 2).collect();
        let x: Vec<f32> = (0..1_100).map(|i| ((i as f32) * 0.1).sin()).collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Accumulate>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("in", x.clone())
                .triggers("trigger", &at)
                .triggers("reset", &[64, 500]);
            h.run(1_100).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn stays_finite_and_reset_clears() {
        let at: Vec<u64> = (0..64).collect();
        let mut h = Harness::new::<Accumulate>(json!({}));
        h.set("in", 3e38).triggers("trigger", &at);
        assert!(h.run(64).float("out").iter().all(|v| v.is_finite()));

        let mut h = Harness::new::<Accumulate>(json!({}));
        h.set_samples("in", vec![f32::NAN; 64])
            .triggers("trigger", &[0]);
        assert_eq!(h.run(64).float("out")[63], 0.0);
        h.set("in", 1.0).triggers("trigger", &[70]);
        assert_eq!(h.run(64).float("out")[63], 1.0);
        h.reset();
        assert_eq!(h.run(64).float("out")[0], 0.0);
    }
}
