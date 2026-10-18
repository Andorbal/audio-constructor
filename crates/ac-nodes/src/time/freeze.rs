use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};
use ac_dsp::delay::DelayLine;
use ac_dsp::util::snap_samples;

node! {
    kind: "freeze",
    name: "Freeze",
    category: Time,
    keywords: [
        "hold", "loop", "stutter", "sustain", "infinite", "capture", "buffer", "glitch",
        "repeat", "looper"
    ],
    /// Holds a short loop of the input while `hold` is on: catch a moment
    /// and let it ring forever.
    ///
    /// Freeze keeps recording its input. When `hold` goes high it takes the
    /// last `size` seconds, ending `position` seconds before that moment,
    /// and loops them until `hold` goes low; otherwise the input passes
    /// straight through, untouched.
    ///
    /// - `hold` (gate): loop while high.
    /// - `size` (seconds, 0.01 to 2, default 0.25): the loop length. Short
    ///   loops buzz at 1 / `size` Hz; long ones repeat a phrase.
    /// - `position` (seconds back): how long before the hold the loop ends;
    ///   0 loops the most recent audio, larger values reach back (up to
    ///   `max_time` − `size`).
    ///
    /// `size` and `position` are read at the moment `hold` rises and kept
    /// for that loop, so the loop never shifts under you. Nothing clicks:
    /// the loop's seam is crossfaded (equal-power, 10 ms or half the loop if
    /// shorter, blending the loop's end into the audio just before its
    /// start), and the switch between input and loop fades over 5 ms each
    /// way. While holding, recording pauses so the loop stays intact; it
    /// resumes on release. Raising `hold` again before the 5 ms fade-out
    /// ends picks the same loop back up instead of taking a new one.
    ///
    /// Config: `max_time`, how far back Freeze can reach in seconds
    /// (default 4, 0.02 to 60), which sizes its buffer. The first two
    /// channels are frozen; further channels pass through.
    ///
    /// **Try this:** wire a `Clock`'s `gate` (interval one beat, `duty` 0.5)
    /// into `hold` on a drum loop for rhythmic stutters, or hold a sustained
    /// chord with `size` 0.3 and feed it into a `Diffuser` for an endless pad.
    pub struct Freeze {
        lines: Vec<DelayLine>,
        rate: f64,
        /// `max_time` in samples: the furthest back a loop can end + start.
        reach: usize,
        /// Fade step per sample for the input/loop switch.
        toggle_step: f64,
        /// Longest seam crossfade, in samples.
        seam_max: usize,
        /// The hold input at the previous frame.
        held: bool,
        /// Position of the input ↔ loop fade, 0 (input) to 1 (loop).
        fade: f64,
        /// The current loop: length, distance back from the freeze point to
        /// its end, seam length, and playback position (all in samples).
        len: usize,
        back: usize,
        seam: usize,
        pos: usize,
        /// Samples recorded since the loop was taken (during fade-outs).
        since: usize,
        /// The current block's plan, shared by both channels.
        plan: Vec<Step>,
    }
}

const IN: usize = 0;
const HOLD: usize = 1;
const SIZE: usize = 2;
const POSITION: usize = 3;
const OUT: usize = 0;

/// Channels with their own buffer; further channels pass through.
const LINES: usize = 2;

const DEFAULT_MAX_TIME: f64 = 4.0;
/// Shortest and longest loops, in seconds.
const MIN_SIZE: f64 = 0.01;
const MAX_SIZE: f64 = 2.0;
/// Seam crossfade (at most half the loop) and input ↔ loop fade, in seconds.
const SEAM: f64 = 0.01;
const TOGGLE: f64 = 0.005;

/// What one frame does, for every frozen channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Step {
    /// Record this frame's input.
    write: bool,
    /// Ages (samples back from the write head) of the loop's two taps and
    /// their weights; `wet` 0 skips reading them.
    age_a: usize,
    age_b: usize,
    w_a: f32,
    w_b: f32,
    dry: f32,
    wet: f32,
}

fn max_time(config: &Config) -> f64 {
    let t = config.get_f64_or("max_time", DEFAULT_MAX_TIME);
    if t.is_finite() {
        t.clamp(0.02, 60.0)
    } else {
        DEFAULT_MAX_TIME
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// Seconds to whole samples (D32 snapping), at least `min`.
fn samples(seconds: f64, rate: f64, min: usize) -> usize {
    let s = snap_samples(seconds * rate).round();
    if s.is_finite() && s >= min as f64 {
        s as usize
    } else {
        min
    }
}

impl Freeze {
    /// Take a new loop from the recorded history.
    fn capture(&mut self, size: f32, position: f32) {
        let size = f64::from(finite_or(size, 0.25)).clamp(MIN_SIZE, MAX_SIZE);
        let len = samples(size, self.rate, 2).min(self.reach.max(2));
        let position = f64::from(finite_or(position, 0.0)).max(0.0);
        let back = samples(position, self.rate, 0).min(self.reach.saturating_sub(len));
        self.len = len;
        self.back = back;
        self.seam = self.seam_max.min(len / 2).max(1);
        self.pos = 0;
        self.since = 0;
    }

    /// Work out every frame of this block.
    fn plan(&mut self, io: &Io, frames: usize) {
        let hold = io.inputs.boolean(HOLD);
        let size = io.inputs.float(SIZE);
        let position = io.inputs.float(POSITION);
        for i in 0..frames.min(self.plan.len()) {
            let h = hold.at(i);
            if h && !self.held && self.fade <= 0.0 {
                self.capture(size.at(i), position.at(i));
            }
            self.held = h;
            let target = if h { 1.0 } else { 0.0 };
            if self.fade < target {
                self.fade = (self.fade + self.toggle_step).min(1.0);
            } else if self.fade > target {
                self.fade = (self.fade - self.toggle_step).max(0.0);
            }
            let mut step = Step {
                write: !h,
                dry: 1.0,
                ..Step::default()
            };
            if self.fade > 0.0 {
                // Equal-power fade; at full level exactly loop only.
                (step.dry, step.wet) = if self.fade >= 1.0 {
                    (0.0, 1.0)
                } else {
                    let angle = std::f64::consts::FRAC_PI_2 * self.fade;
                    (angle.cos() as f32, angle.sin() as f32)
                };
                // Loop position q is `back + len − q` samples before the
                // freeze point, which is now `since` further back.
                let q = self.pos;
                let base = self.back + self.since;
                step.age_a = base + self.len - q;
                step.w_a = 1.0;
                let seam_start = self.len - self.seam;
                if q >= seam_start {
                    // Blend the loop's end into the audio just before its
                    // start (position q − len), which runs into position 0.
                    let t = (q - seam_start) as f64 / self.seam as f64;
                    let angle = std::f64::consts::FRAC_PI_2 * t;
                    step.w_a = angle.cos() as f32;
                    step.w_b = angle.sin() as f32;
                    step.age_b = base + 2 * self.len - q;
                }
                self.pos += 1;
                if self.pos >= self.len {
                    self.pos = 0;
                }
            }
            if step.write && self.fade > 0.0 {
                self.since += 1;
            }
            if let Some(p) = self.plan.get_mut(i) {
                *p = step;
            }
        }
    }
}

impl Node for Freeze {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let max = max_time(config);
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to freeze."))
            .input(SocketSpec::boolean("hold", false).doc("Loop while high."))
            .input(
                SocketSpec::float("size", Unit::Seconds, 0.25)
                    .range(MIN_SIZE, MAX_SIZE.min(max))
                    .doc("Loop length, read when hold rises."),
            )
            .input(
                SocketSpec::float("position", Unit::Seconds, 0.0)
                    .range(0.0, max)
                    .doc("How long before the hold the loop ends."),
            )
            .output(SocketSpec::audio("out").doc("The input, or the loop while holding."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            48_000.0
        };
        let reach = samples(max_time(config), rate, 2);
        let seam_max = samples(SEAM, rate, 1);
        let toggle = samples(TOGGLE, rate, 1);
        // History for the furthest loop, the audio before its start for the
        // seam, and what a fade-out records on top.
        let len = reach + seam_max + toggle + 2;
        Freeze {
            lines: (0..LINES).map(|_| DelayLine::new(len)).collect(),
            rate,
            reach,
            toggle_step: 1.0 / toggle as f64,
            seam_max,
            held: false,
            fade: 0.0,
            len: 2,
            back: 0,
            seam: 1,
            pos: 0,
            since: 0,
            plan: vec![Step::default(); ctx.max_block.max(1)],
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        self.plan(io, frames);
        let input = io.inputs.audio(IN);
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
                let x = finite_or(x, 0.0);
                let Some(step) = self.plan.get(i) else {
                    *o = x;
                    continue;
                };
                *o = if step.wet > 0.0 {
                    let looped = step.w_a * line.tap(step.age_a) + step.w_b * line.tap(step.age_b);
                    step.dry * x + step.wet * looped
                } else {
                    x
                };
                if step.write {
                    line.write(x);
                }
            }
        }
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        self.held = false;
        self.fade = 0.0;
        self.pos = 0;
        self.since = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32)
            .collect()
    }

    /// A gate that is high over `[on, off)`.
    fn gate(n: usize, on: usize, off: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if i >= on && i < off { 1.0 } else { 0.0 })
            .collect()
    }

    fn max_step(x: &[f32]) -> f32 {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn passes_through_untouched_when_not_holding() {
        let x = sine(440.0, 5_000);
        let mut h = Harness::new::<Freeze>(json!({}));
        h.set_audio("in", vec![x.clone()]);
        assert_eq!(h.run(5_000).audio("out")[0], x);
    }

    #[test]
    fn loops_the_slice_that_ended_position_before_the_hold() {
        // Every input sample distinct, so the loop's source is unambiguous.
        let n = 20_000;
        let x: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let t = 10_000;
        let mut h = Harness::new::<Freeze>(json!({}));
        h.set_audio("in", vec![x])
            .set_samples("hold", gate(n, t, n))
            .set("size", 0.02)
            .set("position", 0.05);
        let o = h.run(n).audio("out")[0].clone();
        let (len, back, seam) = (960, 2_400, 480);
        let start = t - back - len;
        // From the second cycle (fade-in done), outside the seam, the loop
        // plays its source exactly.
        for cycle in 1..5 {
            for q in 0..len - seam {
                let got = o[t + cycle * len + q];
                let want = (start + q) as f32 / n as f32;
                assert_eq!(got, want, "cycle {cycle} q {q}");
            }
        }
        // And it repeats exactly.
        for k in t + len..n - len {
            assert_eq!(o[k], o[k + len], "{k}");
        }
    }

    #[test]
    fn nothing_clicks_on_hold_toggles_or_at_the_seam() {
        let n = 48_000;
        let x = sine(440.0, n);
        // 440 Hz in 0.0123 s is a fractional number of periods: a raw loop
        // would jump at every seam.
        let mut hold = gate(n, 6_000, 20_000);
        hold.iter_mut()
            .zip(gate(n, 26_000, 40_000))
            .for_each(|(a, b)| *a += b);
        let mut h = Harness::new::<Freeze>(json!({}));
        h.set_audio("in", vec![x])
            .set_samples("hold", hold)
            .set("size", 0.0123)
            .set("position", 0.003);
        let o = h.run(n).audio("out")[0].clone();
        // The sine's own steepest step is 0.058.
        let step = max_step(&o);
        assert!(step < 0.12, "{step}");
        assert!(peak(&o) < 1.5);
    }

    #[test]
    fn release_returns_to_the_input() {
        let n = 20_000;
        let x = sine(300.0, n);
        let mut h = Harness::new::<Freeze>(json!({}));
        h.set_audio("in", vec![x.clone()])
            .set_samples("hold", gate(n, 5_000, 9_000))
            .set("size", 0.05);
        let o = h.run(n).audio("out")[0].clone();
        // 5 ms (240 samples) of fade-out after the release.
        assert_eq!(&o[9_240..], &x[9_240..]);
        assert_ne!(&o[6_000..9_000], &x[6_000..9_000]);
    }

    #[test]
    fn rehold_during_the_fade_out_resumes_the_same_loop() {
        let n = 20_000;
        let x: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut hold = gate(n, 5_000, 9_000);
        hold.iter_mut()
            .zip(gate(n, 9_100, n))
            .for_each(|(a, b)| *a += b);
        let render = |hold: Vec<f32>| {
            let mut h = Harness::new::<Freeze>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set_samples("hold", hold)
                .set("size", 0.02);
            h.run(n).audio("out")[0].clone()
        };
        let o = render(hold);
        let steady = render(gate(n, 5_000, n));
        // Once back at full level, the same loop at the same place.
        assert_eq!(&o[9_400..], &steady[9_400..]);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let n = 12_000;
        let x = sine(523.0, n);
        let hold: Vec<f32> = (0..n)
            .map(|i| if (i / 1_700) % 2 == 1 { 1.0 } else { 0.0 })
            .collect();
        let size: Vec<f32> = (0..n).map(|i| 0.01 + (i % 900) as f32 * 1e-4).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Freeze>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone(), sine(97.0, n)])
                .set_samples("hold", hold.clone())
                .set_samples("size", size.clone())
                .set("position", 0.004);
            h.run(n).audio("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn configuration_and_parameters_are_clamped() {
        let sig = Freeze::describe(&json!({ "max_time": 1.0 }));
        assert_eq!(sig.inputs[SIZE].range, Some((0.01, 1.0)));
        assert_eq!(sig.inputs[POSITION].range, Some((0.0, 1.0)));
        assert_eq!(
            Freeze::describe(&json!({ "max_time": 1e9 })).inputs[POSITION].range,
            Some((0.0, 60.0))
        );
        // A loop bigger than the buffer, reaching too far back, still works.
        let n = 8_000;
        let mut h = Harness::new::<Freeze>(json!({ "max_time": 0.05 }));
        h.set_audio("in", vec![sine(200.0, n)])
            .set_samples("hold", gate(n, 4_000, n))
            .set("size", 5.0)
            .set("position", 9.0);
        let o = h.run(n).audio("out")[0].clone();
        assert!(o.iter().all(|v| v.is_finite()));
        assert!(peak(&o[5_000..]) > 0.1);
    }

    #[test]
    fn extreme_values_stay_finite_and_extra_channels_pass() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(3_000).copied().collect();
        let mut h = Harness::with_resources::<Freeze>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio(
            "in",
            vec![wild.clone(), vec![0.5; 3_000], vec![0.25; 3_000]],
        )
        .set_samples("hold", wild.iter().rev().copied().collect())
        .set_samples("size", wild.clone())
        .set_samples("position", wild);
        let out = h.run(3_000);
        for c in out.audio("out") {
            assert!(c.iter().all(|v| v.is_finite()));
        }
        assert!(out.audio("out")[2].iter().all(|&v| v == 0.25));
    }

    #[test]
    fn reset_forgets_the_recording() {
        let mut h = Harness::new::<Freeze>(json!({}));
        h.set_audio("in", vec![vec![0.8; 4_000]]);
        h.run(2_000);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 8_000]]).set("hold", true);
        assert_eq!(peak(&h.run(2_000).audio("out")[0]), 0.0);
    }
}
