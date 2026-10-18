use std::collections::VecDeque;

use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use super::{frames_of, time_samples, time_unit, uses_beats};

node! {
    kind: "trigger_delay",
    name: "Trigger Delay",
    category: Triggers,
    keywords: ["delay", "late", "offset", "shift", "echo", "swing", "nudge"],
    /// Delays each trigger by a time in seconds or beats.
    ///
    /// Every trigger on `in` comes out of `out` `time` later, on its exact
    /// sample, even when that falls several blocks later. The `unit` config
    /// chooses seconds (the default) or beats, which follow the tempo at
    /// the moment the trigger arrives. Each trigger keeps the delay it
    /// arrived with: changing `time` moves only triggers still to come, so
    /// a shorter time can let a later trigger overtake an earlier one. A
    /// `time` of 0 passes triggers through on the same sample; negative or
    /// NaN times count as 0.
    ///
    /// Up to 4096 triggers can be waiting at once; triggers arriving while
    /// the queue is full are dropped. Triggers that come due on the same
    /// sample come out once. Restarting the transport drops every waiting
    /// trigger.
    ///
    /// **Try this:** wire a `Clock` into a `Trigger Delay` with `unit` set
    /// to beats and `time` at 0.75, and send both the clock and the delayed
    /// trigger into envelopes for a dotted-eighth echo rhythm.
    pub struct TriggerDelay {
        beats: bool,
        /// The node's own clock: frames since loading or reset, at the
        /// start of the block.
        now: u64,
        /// When each waiting trigger comes due, on the node's clock,
        /// ascending.
        queue: VecDeque<u64>,
    }
}

const IN: usize = 0;
const TIME: usize = 1;

const OUT: usize = 0;

/// Most triggers waiting at once.
const CAPACITY: usize = 4096;

impl Node for TriggerDelay {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let beats = uses_beats(config);
        let max = if beats { 4.0 } else { 2.0 };
        Signature::new()
            .input(SocketSpec::trigger("in").doc("The triggers to delay."))
            .input(
                SocketSpec::float("time", time_unit(beats), 0.25)
                    .range(0.0, max)
                    .no_smooth()
                    .doc("How long each trigger waits."),
            )
            .output(SocketSpec::trigger("out").doc("The delayed triggers."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TriggerDelay {
            beats: uses_beats(config),
            now: 0,
            queue: VecDeque::with_capacity(CAPACITY),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let start = self.now;
        let end = start.saturating_add(ctx.frames as u64);
        let time = io.inputs.float(TIME);
        for o in frames_of(io.inputs.triggers(IN), ctx.frames) {
            let i = o as usize;
            let due = start + o + time_samples(time.at(i), self.beats, ctx, i);
            if self.queue.len() < CAPACITY.min(self.queue.capacity()) {
                let at = self.queue.partition_point(|&q| q <= due);
                self.queue.insert(at, due);
            }
        }
        let mut out = io.outputs.trigger(OUT);
        let mut last = None;
        while let Some(&due) = self.queue.front() {
            if due >= end {
                break;
            }
            self.queue.pop_front();
            let offset = due.saturating_sub(start) as u32;
            if last != Some(offset) {
                out.push(offset);
                last = Some(offset);
            }
        }
        self.now = end;
    }

    fn reset(&mut self) {
        self.now = 0;
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{TempoMap, Unit};
    use serde_json::json;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<TriggerDelay>(config, 48_000.0, block, &ac_core::NoResources)
    }

    #[test]
    fn delays_by_seconds_across_blocks() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[0, 10, 1_000]);
        h.set("time", 0.01);
        assert_eq!(h.run(2_000).triggers("out"), &[480, 490, 1_480]);
    }

    #[test]
    fn beats_follow_the_tempo() {
        let mut h = harness(json!({"unit": "beats"}), 64);
        h.triggers("in", &[100]);
        h.set("time", 0.5);
        assert_eq!(h.run(20_000).triggers("out"), &[12_100]);

        let mut h = harness(json!({"unit": "beats"}), 64);
        h.tempo(TempoMap::constant(48_000.0, 60.0));
        h.triggers("in", &[100]);
        h.set("time", 0.5);
        assert_eq!(h.run(30_000).triggers("out"), &[24_100]);
        let sig = TriggerDelay::describe(&json!({"unit": "beats"}));
        assert_eq!(sig.inputs[TIME].unit, Unit::Beats);
        assert_eq!(
            TriggerDelay::describe(&json!({})).inputs[TIME].unit,
            Unit::Seconds
        );
    }

    #[test]
    fn each_trigger_keeps_the_time_it_arrived_with() {
        // 100 samples for the first trigger, 10 for the second: it overtakes.
        let mut time = vec![100.0 / 48_000.0; 64];
        time[20..].fill(10.0 / 48_000.0);
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[5, 30]);
        h.set_samples("time", time);
        assert_eq!(h.run(256).triggers("out"), &[40, 105]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let at: Vec<u64> = (0..200).map(|k| k * 53 + (k % 5)).collect();
        let time: Vec<f32> = (0..20_000).map(|i| ((i % 700) as f32) / 48_000.0).collect();
        let run = |block| {
            let mut h = harness(json!({}), block);
            h.triggers("in", &at);
            h.set_samples("time", time.clone());
            h.run(20_000).triggers("out").to_vec()
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.len() > 150);
    }

    #[test]
    fn zero_and_bad_times_pass_through_and_coinciding_triggers_merge() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3, 9]);
        h.set("time", 0.0);
        assert_eq!(h.run(64).triggers("out"), &[3, 9]);
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3, 9]);
        h.set_samples("time", vec![f32::NAN; 64]);
        assert_eq!(h.run(64).triggers("out"), &[3, 9]);
        // 3 + 20 and 13 + 10 both come due at 23.
        let mut time = vec![20.0 / 48_000.0; 64];
        time[13] = 10.0 / 48_000.0;
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3, 13]);
        h.set_samples("time", time);
        assert_eq!(h.run(64).triggers("out"), &[23]);
        // A huge time never comes due (and doesn't overflow).
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3]);
        h.set("time", 1.0e30);
        assert!(h.run(256).triggers("out").is_empty());
    }

    #[test]
    fn a_full_queue_drops_new_triggers() {
        let at: Vec<u64> = (0..5_000).collect();
        let mut h = harness(json!({}), 64);
        h.triggers("in", &at);
        h.set("time", 1.0);
        let out = h.run(60_000);
        assert_eq!(out.triggers("out").len(), CAPACITY);
        assert_eq!(out.triggers("out")[0], 48_000);
    }

    #[test]
    fn reset_drops_waiting_triggers() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[10]);
        h.set("time", 0.01);
        h.run(64);
        h.reset();
        assert!(h.run(1_000).triggers("out").is_empty());
    }
}
