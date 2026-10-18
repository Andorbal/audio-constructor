use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::time_samples;
use crate::segments::Runs;

node! {
    kind: "timer",
    name: "Timer",
    category: Triggers,
    keywords: ["stopwatch", "elapsed", "countdown", "duration", "alarm", "time since", "one shot"],
    /// Counts the time since `start`, and fires `done` once `duration` has
    /// passed.
    ///
    /// A trigger on `start` sets `elapsed` (seconds) to 0 and starts it
    /// counting, sample by sample; a `start` while running starts again
    /// from 0. `stop` halts it, and `elapsed` holds where it stopped.
    /// `running` is high while it counts.
    ///
    /// When `elapsed` reaches `duration`, `done` fires on that exact sample
    /// and the timer stops by itself, with `elapsed` holding at `duration`,
    /// so `running` is a gate exactly `duration` long. `progress` goes from
    /// 0 to 1 over the duration. With `duration` at 0 (or negative or NaN)
    /// the timer is a plain stopwatch: it runs until stopped, `done` never
    /// fires, and `progress` stays 0. `duration` is read every sample, so
    /// shortening it below the time already counted ends the run at once.
    ///
    /// On one sample, reaching `duration` comes first (so `done` still
    /// fires when a `start` lands on the same sample), then `stop`, then
    /// `start`, so a `start` always wins. The timer counts whether or not
    /// the transport is playing; restarting the transport stops it and sets
    /// `elapsed` back to 0.
    ///
    /// **Try this:** wire a `Button` into `start` and `running` into an
    /// `ADSR`'s `gate` for a note that lasts exactly `duration` however you
    /// press, or wire `progress` through a `Map` into a `Biquad`'s `cutoff`
    /// for a sweep that takes as long as you like.
    pub struct Timer {
        rate: f64,
        running: bool,
        /// Samples counted since the last start.
        count: u64,
        /// Per-frame output values for the block.
        elapsed: Vec<f32>,
        on: Vec<f32>,
        progress: Vec<f32>,
    }
}

const START: usize = 0;
const STOP: usize = 1;
const DURATION: usize = 2;

const ELAPSED: usize = 0;
const RUNNING: usize = 1;
const DONE: usize = 2;
const PROGRESS: usize = 3;

impl Node for Timer {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("start").doc("Starts counting from 0."))
            .input(SocketSpec::trigger("stop").doc("Stops counting."))
            .input(
                SocketSpec::float("duration", Unit::Seconds, 1.0)
                    .range(0.0, 10.0)
                    .no_smooth()
                    .doc("When `done` fires; 0 runs until stopped."),
            )
            .output(SocketSpec::float("elapsed", Unit::Seconds, 0.0).doc("Time since the start."))
            .output(SocketSpec::boolean("running", false).doc("High while counting."))
            .output(SocketSpec::trigger("done").doc("Fires when `duration` has passed."))
            .output(
                SocketSpec::float("progress", Unit::Generic, 0.0)
                    .doc("`elapsed` over `duration`, 0–1."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let n = ctx.max_block.max(1);
        Timer {
            rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            running: false,
            count: 0,
            elapsed: vec![0.0; n],
            on: vec![0.0; n],
            progress: vec![0.0; n],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let duration = io.inputs.float(DURATION);
        let lists = [io.inputs.triggers(STOP), io.inputs.triggers(START)];
        let frames = ctx.frames.min(self.elapsed.len());
        {
            let mut done = io.outputs.trigger(DONE);
            for run in Runs::new(lists, frames) {
                for i in run.at..run.until {
                    let d = duration.at(i);
                    let limit = (d > 0.0).then(|| time_samples(d, false, ctx, i).max(1));
                    if let Some(limit) = limit
                        && self.running
                        && self.count >= limit
                    {
                        self.running = false;
                        done.push(i as u32);
                    }
                    if i == run.at {
                        let [stop, start] = run.fired;
                        if stop {
                            self.running = false;
                        }
                        if start {
                            self.running = true;
                            self.count = 0;
                        }
                    }
                    self.elapsed[i] = (self.count as f64 / self.rate) as f32;
                    self.on[i] = if self.running { 1.0 } else { 0.0 };
                    self.progress[i] =
                        limit.map_or(0.0, |l| (self.count as f64 / l as f64).min(1.0) as f32);
                    if self.running {
                        self.count = self.count.saturating_add(1);
                    }
                }
            }
        }
        let last = frames.saturating_sub(1);
        let pick = |v: &[f32], i: usize| v.get(i.min(last)).copied().unwrap_or(0.0);
        io.outputs
            .float(ELAPSED)
            .fill_with(|i| pick(&self.elapsed, i));
        io.outputs.float(RUNNING).fill_with(|i| pick(&self.on, i));
        io.outputs
            .float(PROGRESS)
            .fill_with(|i| pick(&self.progress, i));
    }

    fn reset(&mut self) {
        self.running = false;
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<Timer>(json!({}), 48_000.0, block, &ac_core::NoResources)
    }

    #[test]
    fn counts_from_start_and_fires_done_at_the_duration() {
        let mut h = harness(64);
        h.triggers("start", &[100]);
        h.set("duration", 0.01);
        let out = h.run(1_000);
        assert_eq!(out.triggers("done"), &[580]);
        let elapsed = out.float("elapsed");
        assert_eq!(elapsed[99], 0.0);
        assert_eq!(elapsed[100], 0.0);
        assert_eq!(elapsed[101], 1.0 / 48_000.0);
        assert_eq!(elapsed[580], (480.0f64 / 48_000.0) as f32);
        assert_eq!(elapsed[999], elapsed[580]);
        let running = out.float("running");
        assert!(running[..100].iter().all(|&r| r == 0.0));
        assert!(running[100..580].iter().all(|&r| r == 1.0));
        assert!(running[580..].iter().all(|&r| r == 0.0));
        let progress = out.float("progress");
        assert_eq!(progress[100], 0.0);
        assert_eq!(progress[340], 0.5);
        assert_eq!(progress[580], 1.0);
        assert_eq!(progress[900], 1.0);
    }

    #[test]
    fn stop_holds_and_start_restarts() {
        let mut h = harness(64);
        h.triggers("start", &[0, 300]);
        h.triggers("stop", &[200]);
        h.set("duration", 0.0);
        let out = h.run(400);
        let elapsed = out.float("elapsed");
        assert_eq!(elapsed[199], (199.0f64 / 48_000.0) as f32);
        assert_eq!(elapsed[200], (200.0f64 / 48_000.0) as f32);
        assert_eq!(elapsed[299], elapsed[200]);
        assert_eq!(elapsed[300], 0.0);
        assert_eq!(out.float("running")[250], 0.0);
        assert_eq!(out.float("running")[399], 1.0);
        // Duration 0: no done, progress stays 0.
        assert!(out.triggers("done").is_empty());
        assert!(out.float("progress").iter().all(|&p| p == 0.0));
    }

    #[test]
    fn same_sample_order() {
        // Done fires at 480 although a start lands there too; it restarts.
        let mut h = harness(64);
        h.triggers("start", &[0, 480]);
        h.set("duration", 0.01);
        let out = h.run(1_500);
        assert_eq!(out.triggers("done"), &[480, 960]);
        assert_eq!(out.float("running")[480], 1.0);
        // Stop and start together: start wins.
        let mut h = harness(64);
        h.triggers("start", &[0, 100]);
        h.triggers("stop", &[100]);
        h.set("duration", 0.0);
        let out = h.run(200);
        assert_eq!(out.float("running")[150], 1.0);
        assert_eq!(out.float("elapsed")[100], 0.0);
    }

    #[test]
    fn shortening_the_duration_ends_the_run() {
        let mut duration = vec![1.0; 256];
        duration[150..].fill(0.001);
        let mut h = harness(64);
        h.triggers("start", &[0]);
        h.set_samples("duration", duration);
        assert_eq!(h.run(256).triggers("done"), &[150]);
        // A tiny positive duration still stops after one sample.
        let mut h = harness(64);
        h.triggers("start", &[10]);
        h.set("duration", 1.0e-9);
        assert_eq!(h.run(64).triggers("done"), &[11]);
        let mut h = harness(64);
        h.triggers("start", &[10]);
        h.set_samples("duration", vec![f32::NAN; 64]);
        assert!(h.run(64).triggers("done").is_empty());
    }

    #[test]
    fn block_size_does_not_matter() {
        let start: Vec<u64> = (0..30).map(|k| k * 700 + k % 7).collect();
        let stop: Vec<u64> = (0..20).map(|k| k * 1_013 + 300).collect();
        let run = |block| {
            let mut h = harness(block);
            h.triggers("start", &start).triggers("stop", &stop);
            h.set("duration", 0.009);
            let out = h.run(22_000);
            (
                out.triggers("done").to_vec(),
                out.float("elapsed").to_vec(),
                out.float("running").to_vec(),
                out.float("progress").to_vec(),
            )
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(!a.0.is_empty());
    }

    #[test]
    fn reset_stops_and_clears() {
        let mut h = harness(64);
        h.triggers("start", &[0]);
        h.set("duration", 0.0);
        h.run(128);
        h.reset();
        let out = h.run(64);
        assert!(out.float("elapsed").iter().all(|&e| e == 0.0));
        assert!(out.float("running").iter().all(|&r| r == 0.0));
    }
}
