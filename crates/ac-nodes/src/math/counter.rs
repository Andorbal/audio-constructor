use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use crate::segments::Runs;

node! {
    kind: "counter",
    name: "Counter",
    category: Math,
    keywords: ["count", "increment", "step", "index", "wrap", "modulo", "tally"],
    /// Counts triggers: each `trigger` adds `step` to `count`, wrapping
    /// around at `max`.
    ///
    /// With `max` above 0 the count runs from 0 to `max` − 1 and wraps (8
    /// becomes 0 when `max` is 8), and `wrapped` fires on the sample where
    /// it wraps. A negative `step` counts down and wraps from 0 to
    /// `max` − 1, which also fires `wrapped`. With `max` at 0 or below the
    /// count never wraps (it stops at the largest `Int`). Lowering `max`
    /// below the current count takes effect at the next trigger.
    ///
    /// `reset` sets the count back to 0. When `reset` and `trigger` land on
    /// the same sample, the reset happens first, so the count comes out as
    /// `step`: a `Clock` wired into both starts each bar from the top.
    ///
    /// `count` is an `Int` that changes once per block (it shows the count
    /// after the block's triggers); `wrapped` is sample-accurate. A
    /// transport restart sets the count back to 0.
    ///
    /// **Try this:** count `Clock` ticks with `max` 4 and wire `count` into
    /// a `Select`'s `index` to step through four values; wire `wrapped`
    /// into a second Counter to count bars.
    pub struct Counter {
        count: i64,
        /// Frames where `wrapped` fired this block.
        wraps: Vec<u32>,
    }
}

const TRIGGER: usize = 0;
const RESET: usize = 1;
const STEP: usize = 2;
const MAX: usize = 3;

const COUNT: usize = 0;
const WRAPPED: usize = 1;

impl Node for Counter {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("trigger").doc("Add `step` to the count."))
            .input(
                SocketSpec::trigger("reset")
                    .doc("Back to 0 (before a trigger on the same sample)."),
            )
            .input(
                SocketSpec::int("step", 1)
                    .range(-16.0, 16.0)
                    .doc("Added on each trigger; negative counts down."),
            )
            .input(
                SocketSpec::int("max", 0)
                    .range(0.0, 64.0)
                    .doc("Wraps at this count (0 = never)."),
            )
            .output(SocketSpec::int("count", 0).doc("The count, once per block."))
            .output(SocketSpec::trigger("wrapped").doc("Fires when the count wraps."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Counter {
            count: 0,
            wraps: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let step = i64::from(io.inputs.int(STEP));
        let max = i64::from(io.inputs.int(MAX));
        let events = [io.inputs.triggers(RESET), io.inputs.triggers(TRIGGER)];
        self.wraps.clear();
        for run in Runs::new(events, ctx.frames) {
            let [reset, trigger] = run.fired;
            if reset {
                self.count = 0;
            }
            if trigger {
                let next = self.count.saturating_add(step);
                if max > 0 {
                    if (next < 0 || next >= max) && self.wraps.len() < self.wraps.capacity() {
                        self.wraps.push(run.at as u32);
                    }
                    self.count = next.rem_euclid(max);
                } else {
                    self.count = next.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
                }
            }
        }
        io.outputs.set_int(
            COUNT,
            self.count.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        let mut wrapped = io.outputs.trigger(WRAPPED);
        for &at in &self.wraps {
            wrapped.push(at);
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.wraps.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn counts_triggers() {
        let mut h = Harness::new::<Counter>(json!({}));
        h.triggers("trigger", &[3, 10, 70]);
        let out = h.run(128);
        let c = out.int("count");
        assert_eq!(c[0], 2, "count after the first block's triggers");
        assert_eq!(c[100], 3);
        assert!(out.triggers("wrapped").is_empty());
    }

    #[test]
    fn wraps_at_max_and_fires_on_the_wrap_sample() {
        let at: Vec<u64> = (0..10).map(|k| k * 10).collect();
        let mut h = Harness::with_resources::<Counter>(json!({}), 48_000.0, 1, &NoResources);
        h.triggers("trigger", &at).set("max", 4);
        let out = h.run(100);
        let c = out.int("count");
        let counts: Vec<i32> = at.iter().map(|&t| c[t as usize]).collect();
        assert_eq!(counts, vec![1, 2, 3, 0, 1, 2, 3, 0, 1, 2]);
        assert_eq!(out.triggers("wrapped"), &[30, 70]);
    }

    #[test]
    fn negative_step_counts_down_and_wraps() {
        let at: Vec<u64> = (0..5).map(|k| k * 10).collect();
        let mut h = Harness::with_resources::<Counter>(json!({}), 48_000.0, 1, &NoResources);
        h.triggers("trigger", &at).set("max", 3).set("step", -1);
        let out = h.run(50);
        let c = out.int("count");
        let counts: Vec<i32> = at.iter().map(|&t| c[t as usize]).collect();
        assert_eq!(counts, vec![2, 1, 0, 2, 1]);
        assert_eq!(out.triggers("wrapped"), &[0, 30]);
    }

    #[test]
    fn big_steps_wrap_once_per_trigger() {
        let mut h = Harness::with_resources::<Counter>(json!({}), 48_000.0, 1, &NoResources);
        h.triggers("trigger", &[0, 1]).set("max", 3).set("step", 7);
        let out = h.run(2);
        assert_eq!(out.int("count"), &[1, 2]);
        assert_eq!(out.triggers("wrapped"), &[0, 1]);
    }

    #[test]
    fn reset_comes_before_a_trigger_on_the_same_sample() {
        let mut h = Harness::with_resources::<Counter>(json!({}), 48_000.0, 1, &NoResources);
        h.triggers("trigger", &[0, 1, 2, 3])
            .triggers("reset", &[2, 5])
            .set("step", 2);
        let out = h.run(6);
        assert_eq!(out.int("count"), &[2, 4, 2, 4, 4, 0]);
    }

    #[test]
    fn step_can_be_wired_and_max_zero_never_wraps() {
        let at: Vec<u64> = (0..100).collect();
        let mut h = Harness::new::<Counter>(json!({}));
        h.triggers("trigger", &at).set_int("step", 3);
        let out = h.run(128);
        assert_eq!(out.int("count")[127], 300);
        assert!(out.triggers("wrapped").is_empty());
    }

    #[test]
    fn same_output_at_any_block_size() {
        let at: Vec<u64> = (0..300).map(|k| k * 7 + k % 3).collect();
        let resets: Vec<u64> = vec![100, 1_000, 1_001];
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Counter>(json!({}), 48_000.0, block, &NoResources);
            h.triggers("trigger", &at)
                .triggers("reset", &resets)
                .set("max", 5);
            let out = h.run(2_048);
            (
                out.triggers("wrapped").to_vec(),
                *out.int("count").last().unwrap_or(&-1),
            )
        };
        let one = render(1);
        assert!(!one.0.is_empty());
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn transport_reset_clears_the_count() {
        let mut h = Harness::new::<Counter>(json!({}));
        h.triggers("trigger", &[0, 1, 2]);
        assert_eq!(h.run(64).int("count")[0], 3);
        h.reset();
        assert_eq!(h.run(64).int("count")[0], 0);
    }
}
