use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::{frames_of, time_samples};

node! {
    kind: "burst",
    name: "Burst",
    category: Triggers,
    keywords: ["ratchet", "repeat", "roll", "flam", "stutter", "retrigger", "train"],
    /// Fires a quick run of triggers after each trigger.
    ///
    /// A trigger on `trigger` starts a burst of `count` (1–256) triggers on
    /// `out`, `interval` seconds apart, the first on the incoming trigger's
    /// own sample. Every one lands on its exact sample, even several blocks
    /// later. `done` fires on the sample of the burst's last trigger, so
    /// chained bursts follow on without a gap.
    ///
    /// `count` and `interval` are taken when a burst starts and hold for
    /// that burst. The spacing is at least one sample, so an `interval` of
    /// 0 (or a negative or NaN one) fires the burst on consecutive samples.
    /// A new trigger during a burst starts a new burst from that sample and
    /// the old one ends without `done`. Several triggers on one sample count
    /// once. Restarting the transport stops the burst.
    ///
    /// **Try this:** feed a `Clock` through a `Probability` into `trigger`
    /// with `count` at 3 and `interval` at 0.04 for drum rolls that crop up
    /// now and then, or wire `done` into an `ADSR` to mark the end of each
    /// roll.
    pub struct Burst {
        /// The node's own clock: frames since loading or reset, at the
        /// start of the block.
        now: u64,
        /// When the next trigger of the burst is due.
        next_at: u64,
        /// Triggers of the burst still to fire.
        left: u32,
        /// Samples between the burst's triggers.
        spacing: u64,
        /// `done` offsets of the current block.
        dones: Vec<u32>,
    }
}

const TRIGGER: usize = 0;
const COUNT: usize = 1;
const INTERVAL: usize = 2;

const OUT: usize = 0;
const DONE: usize = 1;

const MAX_COUNT: i32 = 256;

impl Node for Burst {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("trigger").doc("Starts a burst."))
            .input(
                SocketSpec::int("count", 4)
                    .range(1.0, 16.0)
                    .doc("Triggers in each burst (1–256)."),
            )
            .input(
                SocketSpec::float("interval", Unit::Seconds, 0.05)
                    .range(0.0, 0.5)
                    .no_smooth()
                    .doc("Time between the burst's triggers."),
            )
            .output(SocketSpec::trigger("out").doc("The burst's triggers."))
            .output(SocketSpec::trigger("done").doc("Fires with the burst's last trigger."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Burst {
            now: 0,
            next_at: 0,
            left: 0,
            spacing: 1,
            dones: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let start = self.now;
        let end = start.saturating_add(ctx.frames as u64);
        let count = io.inputs.int(COUNT).clamp(1, MAX_COUNT) as u32;
        let interval = io.inputs.float(INTERVAL);
        let mut ins = frames_of(io.inputs.triggers(TRIGGER), ctx.frames)
            .map(|o| start + o)
            .peekable();
        self.dones.clear();
        {
            let mut out = io.outputs.trigger(OUT);
            loop {
                let due = (self.left > 0).then_some(self.next_at);
                match (ins.peek().copied(), due) {
                    // A new burst replaces the old one; its first trigger
                    // fires on the next turn of the loop.
                    (Some(t), d) if d.is_none_or(|d| t <= d) => {
                        ins.next();
                        let i = (t - start) as usize;
                        self.spacing = time_samples(interval.at(i), false, ctx, i).max(1);
                        self.left = count;
                        self.next_at = t;
                    }
                    (_, Some(d)) if d < end => {
                        let offset = (d - start) as u32;
                        out.push(offset);
                        self.left -= 1;
                        if self.left == 0 {
                            if self.dones.len() < self.dones.capacity() {
                                self.dones.push(offset);
                            }
                        } else {
                            self.next_at = d.saturating_add(self.spacing);
                        }
                    }
                    _ => break,
                }
            }
        }
        let mut done = io.outputs.trigger(DONE);
        for &d in &self.dones {
            done.push(d);
        }
        self.now = end;
    }

    fn reset(&mut self) {
        self.now = 0;
        self.left = 0;
        self.dones.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn run(
        block: usize,
        at: &[u64],
        count: i32,
        interval: f32,
        frames: usize,
    ) -> (Vec<u64>, Vec<u64>) {
        let mut h =
            Harness::with_resources::<Burst>(json!({}), 48_000.0, block, &ac_core::NoResources);
        h.triggers("trigger", at);
        h.set("count", count).set("interval", interval);
        let out = h.run(frames);
        (out.triggers("out").to_vec(), out.triggers("done").to_vec())
    }

    #[test]
    fn fires_count_triggers_from_the_trigger_itself() {
        let (out, done) = run(64, &[10], 4, 0.01, 2_000);
        assert_eq!(out, &[10, 490, 970, 1_450]);
        assert_eq!(done, &[1_450]);
        let (out, done) = run(64, &[10], 1, 0.01, 2_000);
        assert_eq!(out, &[10]);
        assert_eq!(done, &[10]);
    }

    #[test]
    fn a_new_trigger_restarts_the_burst_without_done() {
        let (out, done) = run(64, &[0, 250], 3, 0.0025, 1_000);
        // 0, 120, 240 would finish at 240 -- but the burst from 0 is
        // done at 240, before 250 arrives.
        assert_eq!(out, &[0, 120, 240, 250, 370, 490]);
        assert_eq!(done, &[240, 490]);
        let (out, done) = run(64, &[0, 200], 3, 0.0025, 1_000);
        assert_eq!(out, &[0, 120, 200, 320, 440]);
        assert_eq!(done, &[440]);
        // A trigger on the sample a burst trigger is due fires once.
        let (out, _) = run(64, &[0, 120], 3, 0.0025, 1_000);
        assert_eq!(out, &[0, 120, 240, 360]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let at: Vec<u64> = (0..60).map(|k| k * 333 + (k * 17) % 200).collect();
        let a = run(1, &at, 5, 0.001, 21_000);
        assert_eq!(a, run(64, &at, 5, 0.001, 21_000));
        assert_eq!(a, run(512, &at, 5, 0.001, 21_000));
        assert!(a.0.len() > 200 && !a.1.is_empty());
    }

    #[test]
    fn extremes() {
        // Interval 0: one sample apart. Count clamps to 1..256.
        let (out, done) = run(64, &[5], 3, 0.0, 64);
        assert_eq!(out, &[5, 6, 7]);
        assert_eq!(done, &[7]);
        let (out, _) = run(64, &[5], 0, 0.0, 64);
        assert_eq!(out, &[5]);
        let (out, done) = run(64, &[0], 1_000, 0.0, 1_000);
        assert_eq!(out.len(), 256);
        assert_eq!(done, &[255]);
        let mut h = Harness::new::<Burst>(json!({}));
        h.triggers("trigger", &[2]);
        h.set_samples("interval", vec![f32::NAN; 64]);
        h.set("count", 2);
        assert_eq!(h.run(64).triggers("out"), &[2, 3]);
        // A huge interval leaves the rest of the burst waiting forever.
        let (out, done) = run(64, &[1], 3, 1.0e30, 256);
        assert_eq!(out, &[1]);
        assert!(done.is_empty());
    }

    #[test]
    fn reset_stops_the_burst() {
        let mut h = Harness::new::<Burst>(json!({}));
        h.triggers("trigger", &[0]);
        h.set("count", 8).set("interval", 0.0005);
        assert_eq!(h.run(64).triggers("out"), &[0, 24, 48]);
        h.reset();
        assert!(h.run(512).triggers("out").is_empty());
    }
}
