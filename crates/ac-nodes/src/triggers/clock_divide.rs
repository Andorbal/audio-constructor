use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use crate::segments::Runs;

node! {
    kind: "clock_divide",
    name: "Clock Divide",
    category: Triggers,
    keywords: ["divider", "every nth", "skip", "slower", "half time", "subdivide", "count"],
    /// Passes every Nth trigger.
    ///
    /// Counting from the last `reset` (or the start), the first trigger on
    /// `in` passes, then every `divide`th after it: with `divide` at 4,
    /// triggers 1, 5, 9, … come out on `out`, on the same sample they came
    /// in. `offset` shifts which ones pass by that many triggers (at 1 they
    /// are 2, 6, 10, …); negative offsets count back. `divide` below 1
    /// counts as 1, which passes everything.
    ///
    /// `divide` and `offset` can change at any time: the count since the
    /// last reset decides, so the pattern stays lined up with the reset.
    /// `reset` sets the count back to the start; a `reset` and a trigger on
    /// the same sample apply in that order, so the trigger counts as the
    /// first and passes (with `offset` 0). Several triggers on one sample
    /// count once. Restarting the transport resets the count too.
    ///
    /// **Try this:** wire a `Clock` at sixteenth notes into `in` and set
    /// `divide` to 4 for quarter notes; use several dividers with different
    /// `offset`s to spread one clock across a kick, a snare, and a hi-hat.
    pub struct ClockDivide {
        /// Triggers since the last reset.
        count: u64,
    }
}

const IN: usize = 0;
const DIVIDE: usize = 1;
const OFFSET: usize = 2;
const RESET: usize = 3;

const OUT: usize = 0;

/// Whether trigger number `n` (from 0) passes.
fn passes(n: u64, divide: i32, offset: i32) -> bool {
    let d = i64::from(divide.max(1));
    let n = i64::try_from(n).unwrap_or(i64::MAX);
    n.saturating_sub(i64::from(offset)).rem_euclid(d) == 0
}

impl Node for ClockDivide {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("in").doc("The triggers to divide."))
            .input(
                SocketSpec::int("divide", 2)
                    .range(1.0, 16.0)
                    .doc("Pass one trigger in this many."),
            )
            .input(
                SocketSpec::int("offset", 0)
                    .range(0.0, 15.0)
                    .doc("Shifts which triggers pass."),
            )
            .input(SocketSpec::trigger("reset").doc("Starts counting again."))
            .output(SocketSpec::trigger("out").doc("Every Nth trigger."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        ClockDivide { count: 0 }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let divide = io.inputs.int(DIVIDE);
        let offset = io.inputs.int(OFFSET);
        let lists = [io.inputs.triggers(RESET), io.inputs.triggers(IN)];
        let mut out = io.outputs.trigger(OUT);
        for run in Runs::new(lists, ctx.frames) {
            let [reset, fired] = run.fired;
            if reset {
                self.count = 0;
            }
            if fired {
                if passes(self.count, divide, offset) {
                    out.push(run.at as u32);
                }
                self.count = self.count.saturating_add(1);
            }
        }
    }

    fn reset(&mut self) {
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<ClockDivide>(json!({}), 48_000.0, block, &ac_core::NoResources)
    }

    fn every(step: u64, n: u64) -> Vec<u64> {
        (0..n).map(|k| k * step).collect()
    }

    #[test]
    fn passes_every_nth_on_the_same_sample() {
        let mut h = harness(64);
        h.triggers("in", &every(10, 12));
        h.set("divide", 4);
        assert_eq!(h.run(128).triggers("out"), &[0, 40, 80]);
        let mut h = harness(64);
        h.triggers("in", &every(10, 12));
        assert_eq!(h.run(128).triggers("out"), &[0, 20, 40, 60, 80, 100]);
    }

    #[test]
    fn offset_shifts_the_pattern() {
        let mut h = harness(64);
        h.triggers("in", &every(10, 12));
        h.set("divide", 4).set("offset", 1);
        assert_eq!(h.run(128).triggers("out"), &[10, 50, 90]);
        let mut h = harness(64);
        h.triggers("in", &every(10, 12));
        h.set("divide", 4).set("offset", -1);
        assert_eq!(h.run(128).triggers("out"), &[30, 70, 110]);
    }

    #[test]
    fn reset_comes_before_a_trigger_on_the_same_sample() {
        let mut h = harness(64);
        h.triggers("in", &every(10, 12));
        h.triggers("reset", &[30]);
        h.set("divide", 4);
        // Count: 0 at 0, 1 at 10, 2 at 20, reset, 0 at 30, ... 4 at 70.
        assert_eq!(h.run(128).triggers("out"), &[0, 30, 70, 110]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let at: Vec<u64> = (0..300).map(|k| k * 7 + (k % 3)).collect();
        let run = |block| {
            let mut h = harness(block);
            h.triggers("in", &at);
            h.triggers("reset", &[500, 1_234]);
            h.set("divide", 3).set("offset", 2);
            h.run(2_200).triggers("out").to_vec()
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(!a.is_empty());
    }

    #[test]
    fn extreme_settings() {
        let mut h = harness(64);
        h.triggers("in", &every(10, 6));
        h.set("divide", 0);
        assert_eq!(h.run(64).triggers("out"), &[0, 10, 20, 30, 40, 50]);
        let mut h = harness(64);
        h.triggers("in", &every(10, 6));
        h.set("divide", i32::MAX).set("offset", i32::MIN);
        assert!(h.run(64).triggers("out").is_empty());
        // Repeats on one sample count once.
        let mut h = harness(64);
        h.triggers("in", &[5, 5, 5, 9]);
        assert_eq!(h.run(64).triggers("out"), &[5]);
        // A count at the limit saturates instead of overflowing.
        assert!(!passes(u64::MAX, 2, i32::MIN));
    }

    #[test]
    fn transport_reset_starts_counting_again() {
        let mut h = harness(64);
        h.triggers("in", &[10, 20, 30, 74, 84]);
        h.set("divide", 3);
        assert_eq!(h.run(64).triggers("out"), &[10]);
        h.reset();
        assert_eq!(h.run(64).triggers("out"), &[74]);
    }
}
