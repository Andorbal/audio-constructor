use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::curve::Curve;
use ac_dsp::util::snap_samples;

use crate::segments::Runs;

node! {
    kind: "graph",
    name: "Graph",
    category: Generators,
    keywords: ["curve", "envelope", "automation", "breakpoint", "draw", "line", "ramp", "shape", "lfo"],
    /// A hand-drawn curve played over time: an envelope, an automation
    /// lane, or (looped) a custom LFO, as a sample-rate `Float`.
    ///
    /// The curve lives in the `points` config as `[x, y]` breakpoints:
    /// `x` is the fraction of the way through (0 at the start, 1 at the
    /// end) and `y` is the output value, any range. A third entry bends the
    /// segment that starts at that point: `"step"` holds `y` until the next
    /// point, `"smooth"` eases in and out, and a number from −1 to 1 bows
    /// the segment (positive rises late, negative rises early); without it
    /// the segment is a straight line. Before the first point and after
    /// the last the curve holds their values. The default rises from 0 to 1
    /// and falls back: `[[0, 0], [0.5, 1], [1, 0]]`.
    ///
    /// `duration` is how long one pass takes: seconds, or quarter-note
    /// beats that follow the tempo when the `unit` config is `"beats"`.
    /// Durations shorter than a sample take one sample.
    ///
    /// `out` is the curve at the playhead. Before the first `start` it
    /// holds the value at `x` = 0; after a pass finishes it holds the end
    /// value (the value at 1, or at 0 when reversed). `position` reports
    /// the playhead (0 to 1, once per block).
    ///
    /// Generator contract: `start` plays from the beginning (from the end
    /// when reversed). A pass lasts `duration`; on its last frame `out`
    /// reaches the end value, `end` fires, and the Graph stops, or wraps
    /// around and keeps going if `loop` is on (a looped pass repeats
    /// exactly every `duration`). `stop` freezes the playhead, holding the
    /// current value, and fires `end`; `reset` jumps back to the beginning
    /// without changing whether it plays. `reverse` flips the direction
    /// and `rate` scales the speed (negative plays backwards). Events at
    /// the same frame apply in the order reverse, stop, start, reset.
    ///
    /// **Try this:** wire a `Clock` into `start` and `out` into a `Gain`
    /// (through a `Map` into decibels) for a hand-drawn envelope on every
    /// beat, or set `loop` with `unit` beats and `duration` 4 for a
    /// one-bar modulation shape that stays in time.
    pub struct Graph {
        curve: Curve,
        beats: bool,
        /// Playhead in samples from the start of a pass (a hair outside
        /// `0..len` right after a loop wraps). Counting samples keeps a
        /// fixed duration exact; when `duration` changes the count is
        /// rescaled, so the playhead keeps its place on the curve.
        elapsed: f64,
        /// Samples in one pass, as of the last frame.
        len: f64,
        forward: bool,
        playing: bool,
    }
}

const DURATION: usize = 0;
const START: usize = 1;
const STOP: usize = 2;
const RESET: usize = 3;
const LOOP: usize = 4;
const REVERSE: usize = 5;
const RATE: usize = 6;

const OUT: usize = 0;
const POSITION: usize = 1;
const PLAYING: usize = 2;
const END: usize = 3;

/// The curve when `points` is missing or holds nothing usable.
pub const DEFAULT_POINTS: &[(f64, f64)] = &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)];
/// Fastest playback, either way.
const MAX_RATE: f32 = 16.0;

fn is_beats(config: &Config) -> bool {
    config.get_str_or("unit", "seconds") == "beats"
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Graph {
    fn beginning(&self) -> f64 {
        if self.forward { 0.0 } else { self.len }
    }

    /// The playhead as a fraction of the pass.
    fn position(&self) -> f64 {
        (self.elapsed / self.len).clamp(0.0, 1.0)
    }

    fn value(&self) -> f32 {
        self.curve.eval(self.position()) as f32
    }

    fn set_len(&mut self, len: f64) {
        if len != self.len {
            self.elapsed *= len / self.len;
            self.len = len;
        }
    }
}

impl Node for Graph {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let (unit, max) = if is_beats(config) {
            (Unit::Beats, 16.0)
        } else {
            (Unit::Seconds, 10.0)
        };
        Signature::new()
            .input(
                SocketSpec::float("duration", unit, 1.0)
                    .range(0.0, max)
                    .doc("How long one pass takes."),
            )
            .generator_inputs(true, true)
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The curve at the playhead."),
            )
            .output(
                SocketSpec::float("position", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("Playhead, 0 to 1."),
            )
            .generator_outputs()
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Graph {
            curve: Curve::from_config(config, "points", DEFAULT_POINTS),
            beats: is_beats(config),
            elapsed: 0.0,
            len: 1.0,
            forward: true,
            playing: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let per_unit = if self.beats {
            ctx.transport
                .tempo
                .samples_per_beat_at(ctx.transport.position)
        } else {
            ctx.sample_rate
        };
        let duration = io.inputs.float(DURATION);
        let looping = io.inputs.boolean(LOOP);
        let speed = io.inputs.float(RATE);
        let events = [
            io.inputs.triggers(REVERSE),
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
        ];
        let frames = io.frames();
        let len_at = |i: usize| {
            let len = snap_samples(f64::from(finite_or(duration.at(i), 1.0)) * per_unit);
            if len.is_finite() { len.max(1.0) } else { 1.0 }
        };
        {
            let (mut out, mut end) = io.outputs.float_trigger(OUT, END);
            for run in Runs::new(events, frames) {
                self.set_len(len_at(run.at));
                let [reverse, stop, start, reset] = run.fired;
                if reverse {
                    self.forward = !self.forward;
                }
                if stop && self.playing {
                    self.playing = false;
                    end.push(run.at as u32);
                }
                if start {
                    self.playing = true;
                    self.elapsed = self.beginning();
                }
                if reset {
                    self.elapsed = self.beginning();
                }
                for i in run.at..run.until {
                    self.set_len(len_at(i));
                    if !self.playing {
                        out.set(i, self.value());
                        continue;
                    }
                    let r = finite_or(speed.at(i), 1.0).clamp(-MAX_RATE, MAX_RATE);
                    let step = f64::from(if self.forward { r } else { -r });
                    // The pass ends on the frame nearest to its end, so a
                    // fractional length never adds or drops a frame.
                    let half = 0.5 * step.abs();
                    let at_end = (step > 0.0 && self.elapsed >= self.len - half)
                        || (step < 0.0 && self.elapsed <= half);
                    if at_end {
                        if looping.at(i) {
                            self.elapsed += if step > 0.0 { -self.len } else { self.len };
                        } else {
                            self.elapsed = if step > 0.0 { self.len } else { 0.0 };
                            self.playing = false;
                            end.push(i as u32);
                        }
                    }
                    out.set(i, self.value());
                    if self.playing {
                        self.elapsed += step;
                    }
                }
            }
        }
        io.outputs.set_float(POSITION, self.position() as f32);
        io.outputs.set_bool(PLAYING, self.playing);
    }

    fn reset(&mut self) {
        self.elapsed = 0.0;
        self.forward = true;
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::TempoMap;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(config: Config, block: usize) -> Harness {
        Harness::with_resources::<Graph>(config, 48_000.0, block, &NoResources)
    }

    /// 0.01 s = 480 frames at 48 kHz.
    fn short(config: Config) -> Harness {
        let mut h = harness(config, 64);
        h.set("duration", 0.01);
        h
    }

    #[test]
    fn default_curve_rises_and_falls_then_ends() {
        let mut h = short(json!({}));
        let out = h.run(1_000);
        let o = out.float("out");
        assert_eq!(o[0], 0.0);
        assert_eq!(o[240], 1.0);
        assert!((o[120] - 0.5).abs() < 1e-6);
        assert_eq!(o[480], 0.0);
        assert_eq!(out.triggers("end"), &[480]);
        assert!(o[481..].iter().all(|&v| v == 0.0));
        assert_eq!(out.float("playing")[999], 0.0);
        assert_eq!(out.float("position")[999], 1.0);
    }

    #[test]
    fn idle_holds_the_start_value_and_finishing_holds_the_end() {
        let points = json!({ "points": [[0, 0.3], [1, 0.9]] });
        let mut h = short(points.clone());
        h.stopped();
        assert!(h.run(256).float("out").iter().all(|&v| v == 0.3));
        let mut h = short(points);
        let o = h.run(1_000).float("out").to_vec();
        assert!((o[480] - 0.9).abs() < 1e-6);
        assert!(o[600..].iter().all(|&v| (v - 0.9).abs() < 1e-6));
    }

    #[test]
    fn start_is_sample_accurate_and_loops_repeat_exactly() {
        let mut h = short(json!({ "points": [[0, 0], [1, 1]] }));
        h.stopped().triggers("start", &[100]).set("loop", true);
        let out = h.run(2_000);
        let o = out.float("out");
        assert_eq!(o[100], 0.0);
        assert!((o[340] - 0.5).abs() < 1e-6);
        let drift = o[100..580]
            .iter()
            .zip(&o[580..1_060])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(drift < 1e-6, "{drift}");
        assert!(out.triggers("end").is_empty());
        assert_eq!(out.float("playing")[1_999], 1.0);
    }

    #[test]
    fn reverse_rate_stop_and_reset() {
        let ramp = json!({ "points": [[0, 0], [1, 1]] });
        // Reversed: from 1 down to 0.
        let mut h = short(ramp.clone());
        h.triggers("reverse", &[0]);
        let out = h.run(1_000);
        let o = out.float("out");
        assert_eq!(o[0], 1.0);
        assert!((o[240] - 0.5).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[480]);
        assert_eq!(o[999], 0.0);
        // Double speed ends in half the time.
        let mut h = short(ramp.clone());
        h.set("rate", 2.0);
        assert_eq!(h.run(1_000).triggers("end"), &[240]);
        // Stop holds the current value.
        let mut h = short(ramp.clone());
        h.triggers("stop", &[120]);
        let out = h.run(600);
        assert!(
            out.float("out")[120..]
                .iter()
                .all(|&v| (v - 0.25).abs() < 1e-6)
        );
        assert_eq!(out.triggers("end"), &[120]);
        // Reset jumps back and keeps playing.
        let mut h = short(ramp);
        h.triggers("reset", &[240]);
        let out = h.run(1_000);
        assert_eq!(out.float("out")[240], 0.0);
        assert_eq!(out.triggers("end"), &[720]);
    }

    #[test]
    fn beats_follow_the_tempo() {
        let mut h = harness(json!({ "unit": "beats", "points": [[0, 0], [1, 1]] }), 256);
        h.tempo(TempoMap::constant(48_000.0, 120.0))
            .set("duration", 1.0);
        let out = h.run(30_000);
        assert_eq!(out.triggers("end"), &[24_000]);
        assert!((out.float("out")[12_000] - 0.5).abs() < 1e-6);
        assert_eq!(
            Graph::describe(&json!({ "unit": "beats" })).inputs[0].unit,
            Unit::Beats
        );
    }

    #[test]
    fn shapes_from_the_points_config() {
        let mut h = short(json!({ "points": [[0, 0, "step"], [0.5, 1], [1, 1]] }));
        let o = h.run(480).float("out").to_vec();
        assert_eq!((o[100], o[239], o[240], o[300]), (0.0, 0.0, 1.0, 1.0));
        // Garbage falls back to the default curve.
        let mut h = short(json!({ "points": "nope" }));
        assert_eq!(h.run(480).float("out")[240], 1.0);
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let speed: Vec<f32> = (0..5_000)
            .map(|i| 1.0 + 0.7 * (i as f32 * 0.002).sin())
            .collect();
        let render = |block: usize| {
            let mut h = harness(
                json!({ "points": [[0, 0], [0.3, 1, "smooth"], [1, -1, 0.5]] }),
                block,
            );
            h.set("duration", 0.02)
                .set("loop", true)
                .set_samples("rate", speed.clone())
                .triggers("reverse", &[1_234])
                .triggers("stop", &[4_000]);
            let out = h.run(5_000);
            (out.float("out").to_vec(), out.triggers("end").to_vec())
        };
        let one = render(1);
        assert_eq!(one.1, vec![4_000]);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -3.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        let mut h = harness(json!({}), 64);
        h.set_samples("duration", wild.clone())
            .set_samples("rate", wild)
            .set("loop", true);
        let out = h.run(700);
        assert!(out.float("out").iter().all(|v| v.is_finite()));
        assert!(out.float("position").iter().all(|v| v.is_finite()));
    }

    #[test]
    fn transport_reset_starts_over() {
        let mut h = short(json!({ "points": [[0, 0], [1, 1]] }));
        h.triggers("reverse", &[10]);
        h.run(300);
        h.reset();
        h.play();
        let o = h.run(64).float("out").to_vec();
        assert_eq!(o[0], 0.0);
        assert!(o[63] > 0.0);
    }
}
