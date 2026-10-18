use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::rng::Rng;
use ac_dsp::util::snap_samples;

use crate::segments::Runs;

node! {
    kind: "step_sequencer",
    name: "StepSequencer",
    category: Modulation,
    keywords: ["sequencer", "steps", "pattern", "melody", "arp", "cv", "analog sequencer"],
    /// A row of step values played one after another, moving on with each
    /// `clock` trigger.
    ///
    /// The `steps` config sets how many steps there are (1 to 64, default
    /// 8) and `values` holds one number per step (missing values are 0,
    /// extra ones are ignored). `length` plays only the first `length`
    /// steps (1 up to `steps`).
    ///
    /// `direction` sets the order: `forward`, `backward`, `pingpong` (up
    /// and back down, without repeating the end steps), or `random` (any
    /// active step, picked with the project's seeded generator so renders
    /// repeat).
    ///
    /// Before the first clock, and after a `reset`, the sequencer waits on
    /// step 0 with `value` showing step 0's value. The next clock then plays
    /// the first step of the pattern (step 0 forward or pingpong, the last
    /// active step backward, a random one in `random`) rather than moving
    /// past it; every later clock advances. When `reset` and `clock` land on
    /// the same sample the reset comes first, so a `Clock` wired into
    /// `reset` once a bar and into `clock` every beat restarts the pattern
    /// cleanly on the downbeat.
    ///
    /// `value` is a sample-rate signal that changes on the clock's exact
    /// sample; with `glide` above 0 it slides from where it was to the new
    /// step's value over that many seconds instead. `step` is the current
    /// step (an `Int`, once per block, after the block's clocks), and
    /// `step_trigger` fires on every clock. A transport restart goes back
    /// to waiting on step 0.
    ///
    /// **Try this:** set `values` to note numbers like 48, 55, 60, 63, wire
    /// a `Clock` (interval 0.25) into `clock`, and `value` through `Convert`
    /// (`note_to_hz`) into an `Oscillator`; use `step_trigger` to fire an
    /// `ADSR` and a small `glide` for a classic sequenced bass line.
    pub struct StepSequencer {
        values: Vec<f32>,
        seed: u64,
        rng: Rng,
        step: usize,
        /// Waiting for the first clock after a start or reset.
        armed: bool,
        /// Pingpong is heading up.
        up: bool,
        current: f64,
        target: f64,
        glide_step: f64,
        glide_left: u64,
        /// Frames where `step_trigger` fired this block.
        fired: Vec<u32>,
    }
}

const CLOCK: usize = 0;
const RESET: usize = 1;
const DIRECTION: usize = 2;
const LENGTH: usize = 3;
const GLIDE: usize = 4;

const VALUE: usize = 0;
const STEP: usize = 1;
const STEP_TRIGGER: usize = 2;

const MAX_STEPS: u64 = 64;
const DIRECTIONS: &[&str] = &["forward", "backward", "pingpong", "random"];

fn step_count(config: &Config) -> usize {
    config.get_u64_or("steps", 8).clamp(1, MAX_STEPS) as usize
}

/// `values` from config, one per step: non-numbers and missing entries are
/// 0, values too large for `f32` are 0.
fn step_values(config: &Config, steps: usize) -> Vec<f32> {
    let list = config.get("values").and_then(|v| v.as_array());
    (0..steps)
        .map(|i| {
            let v = list
                .and_then(|l| l.get(i))
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0) as f32;
            if v.is_finite() { v } else { 0.0 }
        })
        .collect()
}

impl Node for StepSequencer {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let steps = step_count(config);
        Signature::new()
            .input(SocketSpec::trigger("clock").doc("Advance to the next step."))
            .input(SocketSpec::trigger("reset").doc("Wait on step 0 again."))
            .input(SocketSpec::choice("direction", DIRECTIONS, "forward").doc("Step order."))
            .input(
                SocketSpec::int("length", steps as i64)
                    .range(1.0, steps as f64)
                    .doc("How many steps play."),
            )
            .input(
                SocketSpec::float("glide", Unit::Seconds, 0.0)
                    .range(0.0, 1.0)
                    .doc("Slide time between values."),
            )
            .output(
                SocketSpec::float("value", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The current step's value."),
            )
            .output(SocketSpec::int("step", 0).doc("The current step, from 0."))
            .output(SocketSpec::trigger("step_trigger").doc("Fires on each clock."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let values = step_values(config, step_count(config));
        let first = f64::from(values.first().copied().unwrap_or(0.0));
        StepSequencer {
            values,
            seed: ctx.seed,
            rng: Rng::new(ctx.seed),
            step: 0,
            armed: true,
            up: true,
            current: first,
            target: first,
            glide_step: 0.0,
            glide_left: 0,
            fired: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = [io.inputs.triggers(RESET), io.inputs.triggers(CLOCK)];
        let direction = io.inputs.choice(DIRECTION, DIRECTIONS.len());
        let steps = self.values.len();
        let length = (io.inputs.int(LENGTH).max(1) as usize).min(steps);
        let glide = io.inputs.float(GLIDE);
        let rate = ctx.sample_rate;
        self.fired.clear();
        {
            let mut value = io.outputs.float(VALUE);
            for run in Runs::new(events, ctx.frames) {
                let [reset, clock] = run.fired;
                if reset {
                    self.step = 0;
                    self.armed = true;
                    self.up = true;
                    self.go(0, glide.at(run.at), rate);
                }
                if clock {
                    let next = self.next_step(direction, length);
                    self.step = next;
                    self.go(next, glide.at(run.at), rate);
                    if self.fired.len() < self.fired.capacity() {
                        self.fired.push(run.at as u32);
                    }
                }
                for i in run.at..run.until {
                    if self.glide_left > 0 {
                        self.glide_left -= 1;
                        self.current = if self.glide_left == 0 {
                            self.target
                        } else {
                            self.current + self.glide_step
                        };
                    }
                    value.set(i, self.current as f32);
                }
            }
        }
        io.outputs.set_int(STEP, self.step as i32);
        let mut trig = io.outputs.trigger(STEP_TRIGGER);
        for &at in &self.fired {
            trig.push(at);
        }
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        self.step = 0;
        self.armed = true;
        self.up = true;
        self.current = f64::from(self.values.first().copied().unwrap_or(0.0));
        self.target = self.current;
        self.glide_step = 0.0;
        self.glide_left = 0;
        self.fired.clear();
    }
}

impl StepSequencer {
    /// The step a clock moves to.
    fn next_step(&mut self, direction: usize, length: usize) -> usize {
        let last = length - 1;
        let s = self.step.min(last);
        if self.armed {
            self.armed = false;
            self.up = true;
            return match direction {
                1 => last,
                3 => self.rng.below(length as u64) as usize,
                _ => 0,
            };
        }
        match direction {
            0 => {
                if self.step >= last {
                    0
                } else {
                    self.step + 1
                }
            }
            1 => {
                if self.step == 0 || self.step > last {
                    last
                } else {
                    self.step - 1
                }
            }
            2 => {
                if last == 0 {
                    0
                } else if self.up {
                    if s < last {
                        s + 1
                    } else {
                        self.up = false;
                        s - 1
                    }
                } else if s > 0 {
                    s - 1
                } else {
                    self.up = true;
                    1
                }
            }
            _ => self.rng.below(length as u64) as usize,
        }
    }

    /// Head for step `k`'s value, sliding over `glide` seconds.
    fn go(&mut self, k: usize, glide: f32, rate: f64) {
        self.target = f64::from(self.values.get(k).copied().unwrap_or(0.0));
        let samples = if glide.is_finite() {
            snap_samples((f64::from(glide) * rate).clamp(0.0, 3600.0 * rate)).round()
        } else {
            0.0
        };
        if samples >= 1.0 {
            self.glide_left = samples as u64;
            self.glide_step = (self.target - self.current) / samples;
        } else {
            self.glide_left = 0;
            self.glide_step = 0.0;
            self.current = self.target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn seq(values: &[f32]) -> serde_json::Value {
        json!({ "steps": values.len(), "values": values })
    }

    /// The value just after each clock.
    fn played(out: &[f32], clocks: &[u64]) -> Vec<f32> {
        clocks.iter().map(|&c| out[c as usize]).collect()
    }

    fn clocks(n: u64, every: u64) -> Vec<u64> {
        (0..n).map(|k| 10 + k * every).collect()
    }

    #[test]
    fn waits_on_step_zero_then_plays_from_the_top() {
        let mut h = Harness::new::<StepSequencer>(seq(&[1.0, 2.0, 3.0, 4.0]));
        let c = clocks(6, 20);
        h.triggers("clock", &c);
        let out = h.run(200);
        let o = out.float("value");
        assert!(o[..10].iter().all(|&v| v == 1.0));
        assert_eq!(played(o, &c), vec![1.0, 2.0, 3.0, 4.0, 1.0, 2.0]);
        assert_eq!(out.triggers("step_trigger"), c.as_slice());
        assert_eq!(out.int("step")[199], 1);
    }

    #[test]
    fn value_changes_on_the_exact_clock_sample() {
        let mut h = Harness::new::<StepSequencer>(seq(&[5.0, 7.0]));
        h.triggers("clock", &[3, 37]);
        let o = h.run(64).float("value").to_vec();
        assert!(o[..37].iter().all(|&v| v == 5.0));
        assert!(o[37..].iter().all(|&v| v == 7.0));
    }

    #[test]
    fn directions() {
        let v = [0.0, 1.0, 2.0, 3.0];
        let c = clocks(9, 10);
        let run = |dir: &str| {
            let mut h = Harness::new::<StepSequencer>(seq(&v));
            h.triggers("clock", &c).set("direction", dir);
            played(h.run(128).float("value"), &c)
        };
        assert_eq!(
            run("forward"),
            vec![0.0, 1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 3.0, 0.0]
        );
        assert_eq!(
            run("backward"),
            vec![3.0, 2.0, 1.0, 0.0, 3.0, 2.0, 1.0, 0.0, 3.0]
        );
        assert_eq!(
            run("pingpong"),
            vec![0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 0.0, 1.0, 2.0]
        );
        let r = run("random");
        assert!(r.iter().all(|x| v.contains(x)));
        assert_eq!(r, run("random"), "seeded");
    }

    #[test]
    fn length_limits_the_active_steps() {
        let c = clocks(6, 10);
        let mut h = Harness::new::<StepSequencer>(seq(&[0.0, 1.0, 2.0, 3.0, 4.0]));
        h.triggers("clock", &c).set("length", 3);
        assert_eq!(
            played(h.run(128).float("value"), &c),
            vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0]
        );

        let mut h = Harness::new::<StepSequencer>(seq(&[0.0, 1.0, 2.0]));
        h.triggers("clock", &c)
            .set("length", 1)
            .set("direction", "pingpong");
        assert!(
            played(h.run(128).float("value"), &c)
                .iter()
                .all(|&x| x == 0.0)
        );
        assert_eq!(
            StepSequencer::describe(&seq(&[0.0; 5])).inputs[LENGTH].default,
            5.into()
        );
    }

    #[test]
    fn reset_comes_before_a_clock_on_the_same_sample() {
        let mut h = Harness::new::<StepSequencer>(seq(&[1.0, 2.0, 3.0, 4.0]));
        let c = clocks(6, 10);
        h.triggers("clock", &c).triggers("reset", &[30]);
        let out = h.run(128);
        // Clocks at 10, 20 play 1, 2; reset with the clock at 30 plays 1 again.
        assert_eq!(
            played(out.float("value"), &c),
            vec![1.0, 2.0, 1.0, 2.0, 3.0, 4.0]
        );

        let mut h = Harness::new::<StepSequencer>(seq(&[1.0, 2.0, 3.0]));
        h.triggers("clock", &[5, 10]).triggers("reset", &[15]);
        let out = h.run(64);
        let o = out.float("value");
        assert_eq!(o[14], 2.0);
        assert_eq!(o[15], 1.0, "reset alone shows step 0");
        assert_eq!(out.int("step")[0], 0);
    }

    #[test]
    fn glide_slides_to_the_new_value() {
        let mut h = Harness::new::<StepSequencer>(seq(&[0.0, 1.0]));
        h.triggers("clock", &[0, 100]).set("glide", 0.001); // 48 samples.
        let o = h.run(400).float("value").to_vec();
        assert_eq!(o[99], 0.0);
        assert!((o[100] - 1.0 / 48.0).abs() < 1e-6);
        assert!((o[100 + 23] - 0.5).abs() < 1e-6);
        assert_eq!(o[100 + 47], 1.0);
        assert!(o[100 + 47..].iter().all(|&v| v == 1.0));
    }

    #[test]
    fn values_are_padded_and_steps_clamped() {
        let config = json!({ "steps": 3, "values": [9, "x"] });
        let c = clocks(3, 10);
        let mut h = Harness::new::<StepSequencer>(config);
        h.triggers("clock", &c);
        assert_eq!(played(h.run(64).float("value"), &c), vec![9.0, 0.0, 0.0]);
        assert_eq!(step_count(&json!({ "steps": 0 })), 1);
        assert_eq!(step_count(&json!({ "steps": 1000 })), 64);
        assert_eq!(step_values(&json!({}), 8), vec![0.0; 8]);
        assert_eq!(step_values(&json!({ "values": [1e300] }), 1), vec![0.0]);
    }

    #[test]
    fn same_output_at_any_block_size() {
        let c: Vec<u64> = (0..120).map(|k| k * 37 + k % 5).collect();
        let config = seq(&[0.0, 3.0, -2.0, 7.0, 1.5, 4.0, 2.0, -1.0]);
        for dir in DIRECTIONS {
            let render = |block: usize| {
                let mut h = Harness::with_resources::<StepSequencer>(
                    config.clone(),
                    48_000.0,
                    block,
                    &NoResources,
                );
                h.triggers("clock", &c)
                    .triggers("reset", &[500, 2_000])
                    .set("direction", *dir)
                    .set("glide", 0.0005);
                let out = h.run(4_608);
                (
                    out.float("value").to_vec(),
                    out.triggers("step_trigger").to_vec(),
                )
            };
            let one = render(1);
            assert_eq!(one, render(64), "{dir}");
            assert_eq!(one, render(512), "{dir}");
        }
    }

    #[test]
    fn transport_reset_starts_over_and_wild_glide_is_safe() {
        let mut h = Harness::new::<StepSequencer>(seq(&[1.0, 2.0, 3.0]));
        h.triggers("clock", &[0, 10]);
        assert_eq!(h.run(64).float("value")[63], 2.0);
        h.reset();
        assert_eq!(h.run(64).float("value")[0], 1.0);

        let wild = [f32::NAN, f32::INFINITY, -1.0, 1e30];
        let g: Vec<f32> = wild.iter().cycle().take(256).copied().collect();
        let c: Vec<u64> = (0..256).step_by(3).collect();
        let mut h = Harness::new::<StepSequencer>(seq(&[1.0, 2.0, 3.0]));
        h.triggers("clock", &c).set_samples("glide", g);
        assert!(h.run(256).float("value").iter().all(|v| v.is_finite()));
    }
}
