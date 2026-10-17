use ac_core::{
    Config, ConfigExt, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::delay::DelayLine;
use ac_dsp::smooth::BlockRamp;

node! {
    kind: "delay",
    name: "Delay",
    category: Time,
    keywords: ["echo", "repeat", "slapback", "feedback", "tape", "tempo"],
    /// Echoes: the input comes back after `time`, repeating with
    /// `feedback`, blended with the dry signal by `mix` (0 dry, 1 wet).
    ///
    /// Config:
    /// - `unit`: `"seconds"` (default) or `"beats"`. In beats mode `time` is
    ///   in quarter notes and follows the project tempo.
    /// - `max_time`: the longest delay in seconds (default 4, 0.01 to 60).
    ///   This sizes the buffer; longer times are clamped to it, in beats
    ///   mode too.
    ///
    /// `time` can be modulated: reads are interpolated, so sweeping it bends
    /// the pitch of the repeats like tape instead of clicking. A block-rate
    /// `time` (a slider, a tempo change) glides across each block. Delays
    /// shorter than one sample are held at one sample. `feedback` is kept
    /// within ±0.99 so the echoes always die away; negative values flip the
    /// polarity of every other repeat.
    ///
    /// The first two channels are delayed; further channels pass through dry.
    ///
    /// **Try this:** set `unit` to beats with `time` 0.75 for a dotted-eighth
    /// echo, or wire a slow `pure` Oscillator through a `Map` into `time`
    /// (around 0.005..0.01 s) with `mix` 0.5 for a chorus.
    pub struct Delay {
        lines: Vec<DelayLine>,
        /// Delay in samples for each frame of the current block.
        delays: Vec<f64>,
        ramp: BlockRamp,
        beats: bool,
    }
}

const IN: usize = 0;
const TIME: usize = 1;
const FEEDBACK: usize = 2;
const MIX: usize = 3;
const OUT: usize = 0;

/// Channels with their own delay line. The node isn't told its channel
/// count before `process`, and each line can be megabytes, so this covers
/// stereo rather than reserving lines that are rarely used.
const LINES: usize = 2;

fn max_time(config: &Config) -> f64 {
    let t = config.get_f64_or("max_time", 4.0);
    if t.is_finite() {
        t.clamp(0.01, 60.0)
    } else {
        4.0
    }
}

fn is_beats(config: &Config) -> bool {
    config.get_str_or("unit", "seconds") == "beats"
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// Times arrive as `f32`, so "0.01 s" is really 0.00999999977 s, a hair off
/// 480 samples at 48 kHz. Snap to the whole sample when the difference is
/// within `f32` rounding, so round-number times give exact integer delays.
fn snap(samples: f64) -> f64 {
    let whole = samples.round();
    if (samples - whole).abs() <= samples.abs() * f64::from(f32::EPSILON) {
        whole
    } else {
        samples
    }
}

/// Bound on what the feedback path writes. A fixed delay with feedback
/// below 1 always decays (the interpolator never boosts), but audio-rate
/// jumps in `time` can briefly exceed unity loop gain; this keeps that
/// finite.
const LIMIT: f32 = 1e9;

impl Delay {
    /// Fill `self.delays` with the delay in samples for each frame.
    fn delays(&mut self, time: FloatIn, scale: f64, frames: usize) {
        let to_samples = |t: f32| snap(f64::from(finite_or(t, 0.0)) * scale);
        if time.is_varying() {
            let mut last = to_samples(time.first());
            for (i, d) in self.delays.iter_mut().take(frames).enumerate() {
                last = to_samples(time.at(i));
                *d = last;
            }
            self.ramp.set(last);
        } else {
            let (start, step) = self.ramp.begin(to_samples(time.first()), frames);
            for (i, d) in self.delays.iter_mut().take(frames).enumerate() {
                *d = start + step * (i + 1) as f64;
            }
        }
    }
}

impl Node for Delay {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let unit = if is_beats(config) {
            Unit::Beats
        } else {
            Unit::Seconds
        };
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to delay."))
            .input(
                SocketSpec::float("time", unit, 0.25)
                    .range(0.0, max_time(config))
                    .doc("Delay time."),
            )
            .input(
                SocketSpec::float("feedback", Unit::Generic, 0.3)
                    .range(0.0, 0.99)
                    .doc("How much of each echo feeds back into the next."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("0 is dry, 1 is only the echoes."),
            )
            .output(SocketSpec::audio("out").doc("Dry and delayed signal."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let max = (max_time(config) * ctx.sample_rate).ceil();
        let max = if max.is_finite() && max >= 1.0 {
            max as usize
        } else {
            1
        };
        Delay {
            lines: (0..LINES).map(|_| DelayLine::new(max)).collect(),
            delays: vec![0.0; ctx.max_block.max(1)],
            ramp: BlockRamp::new(),
            beats: is_beats(config),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let scale = if self.beats {
            ctx.transport
                .tempo
                .samples_per_beat_at(ctx.transport.position)
        } else {
            ctx.sample_rate
        };
        let frames = io.frames();
        self.delays(io.inputs.float(TIME), scale, frames);
        let last_delay = self.delays.last().copied().unwrap_or(1.0);

        let input = io.inputs.audio(IN);
        let feedback = io.inputs.float(FEEDBACK);
        let mix = io.inputs.float(MIX);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            let Some(line) = self.lines.get_mut(c) else {
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = *s;
                }
                continue;
            };
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let d = self.delays.get(i).copied().unwrap_or(last_delay);
                let fb = finite_or(feedback.at(i), 0.0).clamp(-0.99, 0.99);
                let m = finite_or(mix.at(i), 0.5).clamp(0.0, 1.0);
                let wet = line.read(d);
                let back = x + fb * wet;
                line.write(if back.is_finite() {
                    back.clamp(-LIMIT, LIMIT)
                } else {
                    0.0
                });
                *o = x * (1.0 - m) + wet * m;
            }
        }
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        self.ramp.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak};
    use serde_json::json;

    fn impulse(n: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[0] = 1.0;
        x
    }

    fn wet_only(config: Config, time: f64, feedback: f64, input: Vec<f32>) -> Vec<f32> {
        let n = input.len();
        let mut h = Harness::new::<Delay>(config);
        h.set_audio("in", vec![input])
            .set("time", time)
            .set("feedback", feedback)
            .set("mix", 1.0);
        h.run(n).audio("out")[0].clone()
    }

    #[test]
    fn impulse_arrives_exactly_at_time_times_rate() {
        let out = wet_only(json!({}), 0.25, 0.0, impulse(13_000));
        assert_eq!(out[12_000], 1.0);
        let others: f32 = out.iter().map(|v| v.abs()).sum::<f32>() - 1.0;
        assert_eq!(others, 0.0);
    }

    #[test]
    fn feedback_makes_decaying_repeats() {
        let out = wet_only(json!({}), 0.01, 0.5, impulse(2_000));
        assert_eq!(out[480], 1.0);
        assert_eq!(out[960], 0.5);
        assert_eq!(out[1440], 0.25);
        assert_eq!(out[1439], 0.0);
    }

    #[test]
    fn mix_zero_is_dry() {
        let x: Vec<f32> = (0..500).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut h = Harness::new::<Delay>(json!({}));
        h.set_audio("in", vec![x.clone()])
            .set("mix", 0.0)
            .set("time", 0.001);
        assert_eq!(h.run(500).audio("out")[0], x);
    }

    #[test]
    fn beats_follow_the_tempo() {
        let sig = Delay::describe(&json!({ "unit": "beats" }));
        assert_eq!(sig.inputs[TIME].unit, Unit::Beats);
        // The harness runs at 120 BPM: one beat is 0.5 s.
        let out = wet_only(json!({ "unit": "beats" }), 1.0, 0.0, impulse(25_000));
        assert_eq!(out[24_000], 1.0);
        let tempo = ac_core::TempoMap::constant(48_000.0, 60.0);
        let mut h = Harness::new::<Delay>(json!({ "unit": "beats" }));
        h.tempo(tempo)
            .set_audio("in", vec![impulse(49_000)])
            .set("time", 1.0)
            .set("mix", 1.0);
        let slow = h.run(49_000).audio("out")[0].clone();
        assert_eq!(slow[48_000], 1.0);
    }

    #[test]
    fn max_time_limits_the_delay() {
        let sig = Delay::describe(&json!({ "max_time": 1000.0 }));
        assert_eq!(sig.inputs[TIME].range, Some((0.0, 60.0)));
        let out = wet_only(json!({ "max_time": 0.01 }), 5.0, 0.0, impulse(1_000));
        assert_eq!(out[480], 1.0);
    }

    #[test]
    fn block_size_one_matches_larger_blocks() {
        let x: Vec<f32> = (0..3_000).map(|i| (i as f32 * 0.05).sin()).collect();
        let time: Vec<f32> = (0..3_000)
            .map(|i| 0.01 + 0.005 * (i as f32 * 0.002).sin())
            .collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<Delay>(json!({}), 48_000.0, block, &NoResources);
            h.set_audio("in", vec![x.clone()]).set("feedback", 0.6);
            if wired {
                h.set_samples("time", time.clone());
            }
            h.run(3_000).audio("out")[0].clone()
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn sweeping_time_does_not_click() {
        let n = 48_000;
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 200.0 * i as f32 / 48_000.0).sin())
            .collect();
        let max_step = |o: &[f32]| {
            o.windows(2)
                .skip(2_000)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        };
        // Per-sample sweep.
        let sweep: Vec<f32> = (0..n).map(|i| 0.01 + 0.01 * i as f32 / n as f32).collect();
        let mut h = Harness::new::<Delay>(json!({}));
        h.set_audio("in", vec![x.clone()])
            .set("mix", 1.0)
            .set("feedback", 0.0)
            .set_samples("time", sweep);
        let smooth = max_step(&h.run(n).audio("out")[0]);
        // A 200 Hz sine moves at most 2π·200/48000 ≈ 0.026 per sample.
        assert!(smooth < 0.03, "{smooth}");
        // Block-rate steps are glided too.
        let steps: Vec<f32> = (0..n).map(|i| 0.01 + 0.0002 * (i / 64) as f32).collect();
        let mut h = Harness::new::<Delay>(json!({}));
        h.set_audio("in", vec![x])
            .set("mix", 1.0)
            .set("feedback", 0.0)
            .set_block("time", steps);
        let glided = max_step(&h.run(n).audio("out")[0]);
        assert!(glided < 0.04, "{glided}");
    }

    #[test]
    fn stereo_lines_are_independent_and_extra_channels_pass_dry() {
        let mut h = Harness::new::<Delay>(json!({}));
        h.set_audio(
            "in",
            vec![impulse(1_000), vec![0.0; 1_000], vec![0.5; 1_000]],
        )
        .set("time", 0.01)
        .set("mix", 1.0);
        let out = h.run(1_000);
        let o = out.audio("out");
        assert_eq!(o[0][480], 1.0);
        assert_eq!(peak(&o[1]), 0.0);
        assert!(o[2].iter().all(|v| *v == 0.5));
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let x: Vec<f32> = (0..2_000)
            .map(|i| if i % 7 == 0 { 1.0 } else { -0.3 })
            .collect();
        let mut h = Harness::with_resources::<Delay>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![x])
            .set_samples("time", wild.clone())
            .set_samples("feedback", wild.iter().rev().copied().collect())
            .set_samples("mix", wild);
        let out = h.run(2_000);
        let o = &out.audio("out")[0];
        assert!(o.iter().all(|v| v.is_finite()));
        assert!(peak(o) < 1_000.0);
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut h = Harness::new::<Delay>(json!({}));
        h.set_audio("in", vec![impulse(1_000)])
            .set("time", 0.005)
            .set("mix", 1.0)
            .set("feedback", 0.9);
        h.run(300);
        h.reset();
        assert_eq!(peak(&h.run(700).audio("out")[0]), 0.0);
    }
}
