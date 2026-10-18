use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::curve::{Curve, Point, Shape};
use ac_dsp::util::snap_samples;

use crate::segments::Runs;

node! {
    kind: "envelope",
    name: "Envelope",
    category: Modulation,
    keywords: ["eg", "ahdsr", "msegs", "multi-stage", "breakpoint", "contour", "shape", "loop"],
    /// A multi-stage envelope drawn as breakpoints: it plays a curve of
    /// levels over time each time it starts, with an optional sustain point
    /// and loop.
    ///
    /// The `points` config is the curve: a list of `[time, level]` pairs
    /// (time in seconds from the start, level 0..1), each optionally with a
    /// third entry shaping the segment that starts there: `"step"`,
    /// `"smooth"`, or a bend from −1 to 1. The default is an AHDSR shape:
    /// up to 1 in 10 ms, hold 50 ms, down to 0.6 over 200 ms, sustain, and
    /// a 300 ms release to 0. Levels outside 0..1 are clamped, negative
    /// times count as 0, and if the first point comes after time 0 the
    /// envelope waits at its level until then.
    ///
    /// Three more config entries pick points by their position in the list
    /// (counting from 0; −1 means none, and a position past the end means
    /// the last point):
    ///
    /// - `sustain` (default 3): while `gate` is high, the envelope stops at
    ///   this point and holds its level.
    /// - `loop_start` and `loop_end` (default −1): while `gate` is high,
    ///   reaching `loop_end` jumps back to `loop_start` and plays that
    ///   stretch again. The loop needs `loop_start` before `loop_end`.
    ///   Whichever of the sustain point and the loop end comes first wins.
    ///
    /// A rising `gate` or a `trigger` starts the curve from its first point
    /// (jumping to its level). When `gate` falls before the envelope has
    /// passed the sustain point, it skips to the sustain point and plays the
    /// rest of the curve from there, with the first segment after the
    /// sustain point starting from the level it had reached, so a short
    /// note releases smoothly from mid-attack. Without a sustain point the
    /// gate only ends the loop, and the curve plays to its end. A `trigger`
    /// while `gate` is low plays the whole curve straight through.
    ///
    /// `time_scale` stretches every time: 2 plays the curve twice as
    /// slowly, 0.5 twice as fast (limited to 0.001..1000). `out` is a
    /// sample-rate signal; `active` is true from the start until the last
    /// point, and `end` fires on the sample the last point is reached.
    /// After the end, `out` holds the last point's level; before the first
    /// start it is 0. When a gate edge and a trigger land on the same
    /// sample, the release applies before the restart.
    ///
    /// **Try this:** wire a `Clock` gate into `gate` and the output into a
    /// `Gain` (through `Map` to dB) after an `Oscillator`, then set
    /// `loop_start` 1 and `loop_end` 2 with `sustain` −1 to make the
    /// attack-hold stretch flutter while the gate is held.
    pub struct Envelope {
        points: Vec<Point>,
        sustain: Option<usize>,
        looping: Option<(usize, usize)>,
        stage: Stage,
        /// Start point of the current segment.
        seg: usize,
        /// Samples into the current segment.
        pos: f64,
        /// Length of the current segment in samples, at `scale`.
        len: f64,
        scale: f64,
        /// Starting level of the current segment when it isn't the point's
        /// own (the segment after the sustain point, on release).
        from: Option<f64>,
        level: f64,
        gate: bool,
        rate: f64,
        /// Frames where `end` fired this block.
        ends: Vec<u32>,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Running,
    Holding,
}

const GATE: usize = 0;
const TRIGGER: usize = 1;
const TIME_SCALE: usize = 2;

const OUT: usize = 0;
const ACTIVE: usize = 1;
const END: usize = 2;

/// Attack 10 ms, hold 50 ms, decay 200 ms to 0.6, sustain, release 300 ms.
pub const DEFAULT_POINTS: &[(f64, f64)] = &[
    (0.0, 0.0),
    (0.01, 1.0),
    (0.06, 1.0),
    (0.26, 0.6),
    (0.56, 0.0),
];
pub const DEFAULT_SUSTAIN: i64 = 3;

const MIN_SCALE: f64 = 0.001;
const MAX_SCALE: f64 = 1000.0;

/// A point index from config: `None` for negative or missing values,
/// clamped to the last point otherwise.
fn index(config: &Config, key: &str, default: i64, n: usize) -> Option<usize> {
    let v = config
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
        .unwrap_or(default);
    (v >= 0).then(|| (v as usize).min(n.saturating_sub(1)))
}

fn time_scale(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v).clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

impl Node for Envelope {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::boolean("gate", false)
                    .doc("Start on the rising edge; release when it falls."),
            )
            .input(SocketSpec::trigger("trigger").doc("Start from the first point."))
            .input(
                SocketSpec::float("time_scale", Unit::Ratio, 1.0)
                    .range(0.1, 10.0)
                    .log()
                    .doc("Stretches every time."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The envelope level."),
            )
            .output(
                SocketSpec::boolean("active", false)
                    .rate(Rate::Sample)
                    .doc("True from the start until the last point."),
            )
            .output(SocketSpec::trigger("end").doc("Fires when the last point is reached."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let curve = Curve::from_config(config, "points", DEFAULT_POINTS);
        let mut points: Vec<Point> = curve
            .points()
            .iter()
            .map(|p| Point {
                x: p.x.max(0.0),
                y: p.y.clamp(0.0, 1.0),
                shape: p.shape,
            })
            .collect();
        let n = points.len();
        let mut sustain = index(config, "sustain", DEFAULT_SUSTAIN, n);
        let loop_start = index(config, "loop_start", -1, n);
        let loop_end = index(config, "loop_end", -1, n);
        // A first point after time 0 waits at its level: give it a lead-in.
        let lead = match points.first() {
            Some(first) if first.x > 0.0 => {
                let start = Point {
                    x: 0.0,
                    y: first.y,
                    shape: Shape::Linear,
                };
                points.insert(0, start);
                1
            }
            _ => 0,
        };
        sustain = sustain.map(|s| s + lead);
        let looping = match (loop_start, loop_end) {
            (Some(a), Some(b)) if a < b && points[b + lead].x > points[a + lead].x => {
                Some((a + lead, b + lead))
            }
            _ => None,
        };
        Envelope {
            points,
            sustain,
            looping,
            stage: Stage::Idle,
            seg: 0,
            pos: 0.0,
            len: 0.0,
            scale: 1.0,
            from: None,
            level: 0.0,
            gate: false,
            rate: ctx.sample_rate.max(1.0),
            ends: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let gate = io.inputs.boolean(GATE);
        let scale_in = io.inputs.float(TIME_SCALE);
        let triggers = io.inputs.triggers(TRIGGER);
        let varying = scale_in.is_varying();
        self.set_scale(time_scale(scale_in.first()));

        self.ends.clear();
        {
            let (mut out, mut active) = io.outputs.float2(OUT, ACTIVE);
            for run in Runs::new([triggers], ctx.frames) {
                for i in run.at..run.until {
                    if varying {
                        self.set_scale(time_scale(scale_in.at(i)));
                    }
                    let g = gate.at(i);
                    // Release before restart (D31).
                    if self.gate && !g {
                        self.release();
                    }
                    let restart = (!self.gate && g) || (i == run.at && run.fired[0]);
                    self.gate = g;
                    if restart {
                        self.start();
                    }
                    let ended = self.resolve();
                    if self.stage == Stage::Running {
                        self.level = self.value();
                    }
                    out.set(i, self.level as f32);
                    active.set(i, if self.stage == Stage::Idle { 0.0 } else { 1.0 });
                    if ended && self.ends.len() < self.ends.capacity() {
                        self.ends.push(i as u32);
                    }
                    if self.stage == Stage::Running {
                        self.pos += 1.0;
                    }
                }
            }
        }
        let mut end = io.outputs.trigger(END);
        for &at in &self.ends {
            end.push(at);
        }
    }

    fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.seg = 0;
        self.pos = 0.0;
        self.len = 0.0;
        self.from = None;
        self.level = 0.0;
        self.gate = false;
        self.ends.clear();
    }
}

impl Envelope {
    fn last(&self) -> usize {
        self.points.len().saturating_sub(1)
    }

    /// Length in samples of the segment starting at point `seg`.
    fn seg_len(&self, seg: usize) -> f64 {
        match (self.points.get(seg), self.points.get(seg + 1)) {
            (Some(a), Some(b)) => snap_samples((b.x - a.x) * self.rate * self.scale),
            _ => 0.0,
        }
    }

    /// Change the time scale, keeping the position within the segment.
    fn set_scale(&mut self, scale: f64) {
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        let len = self.seg_len(self.seg);
        if self.len > 0.0 {
            self.pos *= len / self.len;
        }
        self.len = len;
    }

    fn enter(&mut self, seg: usize) {
        self.seg = seg;
        self.len = self.seg_len(seg);
        self.from = None;
    }

    fn start(&mut self) {
        self.stage = Stage::Running;
        self.pos = 0.0;
        self.enter(0);
        self.level = self.points.first().map_or(0.0, |p| p.y);
    }

    fn release(&mut self) {
        let Some(s) = self.sustain else {
            return;
        };
        let before_sustain = match self.stage {
            Stage::Running => self.seg < s,
            Stage::Holding => true,
            Stage::Idle => false,
        };
        if before_sustain {
            self.stage = Stage::Running;
            self.pos = 0.0;
            self.enter(s);
            self.from = Some(self.level);
        }
    }

    /// Move past every point the position has reached. Returns true when
    /// the envelope reached its last point on this sample.
    fn resolve(&mut self) -> bool {
        // Bounds the work per sample even for degenerate loops.
        let mut budget = 2 * self.points.len() + 8;
        while self.stage == Stage::Running {
            let last = self.last();
            if self.seg >= last {
                self.stage = Stage::Idle;
                self.level = self.points.get(last).map_or(0.0, |p| p.y);
                self.from = None;
                return true;
            }
            if self.pos < self.len {
                return false;
            }
            budget -= 1;
            if budget == 0 {
                self.pos = 0.0;
                return false;
            }
            let next = self.seg + 1;
            let carry = self.pos - self.len;
            if self.gate {
                if let Some((a, b)) = self.looping
                    && next == b
                {
                    self.enter(a);
                    self.pos = carry;
                    continue;
                }
                if self.sustain == Some(next) {
                    self.stage = Stage::Holding;
                    self.enter(next);
                    self.pos = 0.0;
                    self.level = self.points[next].y;
                    return false;
                }
            }
            self.enter(next);
            self.pos = carry;
        }
        false
    }

    /// Level at the current position of a running segment.
    fn value(&self) -> f64 {
        let (Some(a), Some(b)) = (self.points.get(self.seg), self.points.get(self.seg + 1)) else {
            return self.level;
        };
        let start = self.from.unwrap_or(a.y);
        let u = if self.len > 0.0 {
            self.pos / self.len
        } else {
            1.0
        };
        start + (b.y - start) * a.shape.ease(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn gate(on: usize, off: usize, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if (on..off).contains(&i) { 1.0 } else { 0.0 })
            .collect()
    }

    /// Points at round millisecond times so segment ends land on samples.
    fn adsr() -> serde_json::Value {
        // 0→1 in 10 ms (480), down to 0.5 by 20 ms (480), release 10 ms (480).
        json!({ "points": [[0, 0], [0.01, 1], [0.02, 0.5], [0.03, 0]], "sustain": 2 })
    }

    #[test]
    fn plays_the_default_ahdsr_shape() {
        let mut h = Harness::new::<Envelope>(json!({}));
        h.set_samples("gate", gate(0, 24_000, 48_000));
        let out = h.run(48_000);
        let o = out.float("out");
        assert_eq!(o[0], 0.0);
        assert_eq!(o[480], 1.0, "attack 10 ms");
        assert_eq!(o[2_880], 1.0, "hold until 60 ms");
        assert!(
            (o[2_880 + 4_800] - 0.8).abs() < 1e-6,
            "halfway down the decay"
        );
        assert!((o[20_000] - 0.6).abs() < 1e-6, "sustain");
        // Release from 0.6 to 0 over 300 ms after the gate falls at 24 000.
        assert!((o[24_000 + 7_200] - 0.3).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[24_000 + 14_400]);
        assert_eq!(o[24_000 + 14_400], 0.0);
        let active = out.float("active");
        assert_eq!(active[24_000 + 14_399], 1.0);
        assert_eq!(active[24_000 + 14_400], 0.0);
    }

    #[test]
    fn segments_land_on_exact_samples() {
        let mut h = Harness::new::<Envelope>(adsr());
        h.set_samples("gate", gate(100, 5_000, 6_000));
        let out = h.run(6_000);
        let o = out.float("out");
        assert_eq!(o[99], 0.0);
        assert_eq!(o[100], 0.0);
        assert!((o[100 + 240] - 0.5).abs() < 1e-6);
        assert_eq!(o[100 + 480], 1.0);
        assert!((o[100 + 720] - 0.75).abs() < 1e-6);
        assert_eq!(o[100 + 960], 0.5);
        assert!(o[100 + 960..5_000].iter().all(|&v| v == 0.5));
        assert!((o[5_000 + 240] - 0.25).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[5_480]);
    }

    #[test]
    fn early_release_continues_from_the_current_level() {
        let mut h = Harness::new::<Envelope>(adsr());
        // Gate falls a quarter of the way up the attack (level 0.25).
        h.set_samples("gate", gate(0, 120, 2_000));
        let out = h.run(2_000);
        let o = out.float("out");
        assert!((o[119] - 119.0 / 480.0).abs() < 1e-6);
        // The release segment now runs 0.25 → 0 over its 480 samples.
        let jump = (o[120] - o[119]).abs();
        assert!(jump < 0.01, "{jump}");
        assert!((o[120 + 240] - o[119] * 0.5).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[600]);
        assert!(o[600..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn trigger_with_gate_low_plays_straight_through() {
        let mut h = Harness::new::<Envelope>(adsr());
        h.triggers("trigger", &[50]);
        let out = h.run(2_000);
        let o = out.float("out");
        assert_eq!(o[50 + 960], 0.5, "passes the sustain point");
        assert!((o[50 + 1_200] - 0.25).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[50 + 1_440]);
    }

    #[test]
    fn loops_while_the_gate_is_high() {
        // Loop between points 1 and 2 (1 → 0.5 → jump back to 1), no sustain.
        let config = json!({
            "points": [[0, 0], [0.01, 1], [0.02, 0.5], [0.03, 0]],
            "sustain": -1, "loop_start": 1, "loop_end": 2
        });
        let mut h = Harness::new::<Envelope>(config);
        h.set_samples("gate", gate(0, 2_200, 4_000));
        let out = h.run(4_000);
        let o = out.float("out");
        assert_eq!(o[480], 1.0);
        assert!((o[480 + 240] - 0.75).abs() < 1e-6);
        assert_eq!(o[960], 1.0, "looped back");
        assert_eq!(o[1_440], 1.0);
        assert!((o[1_440 + 240] - 0.75).abs() < 1e-6);
        // The gate falls at 2200, inside the fourth pass (which began at
        // 1920): that pass finishes, the loop ends, and the curve plays on.
        assert!((o[2_200] - (1.0 - 0.5 * 280.0 / 480.0)).abs() < 1e-6);
        assert_eq!(o[2_400], 0.5);
        assert!((o[2_400 + 240] - 0.25).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[2_400 + 480]);
    }

    #[test]
    fn loop_and_sustain_whichever_comes_first() {
        // Sustain at point 3 comes after the loop end at 2: it loops.
        let config = json!({
            "points": [[0, 0], [0.01, 1], [0.02, 0.5], [0.03, 0.2], [0.04, 0]],
            "sustain": 3, "loop_start": 1, "loop_end": 2
        });
        let mut h = Harness::new::<Envelope>(config);
        h.set_samples("gate", gate(0, 2_000, 4_000));
        let out = h.run(4_000);
        let o = out.float("out");
        assert_eq!(o[1_440], 1.0, "still looping");
        // Gate falls at 2000 mid-loop: skip to the sustain point (3) and
        // release from the current level to 0.
        let at = o[1_999];
        assert!((o[2_000] - at).abs() < 0.01);
        assert_eq!(out.triggers("end"), &[2_480]);
    }

    #[test]
    fn retrigger_restarts_and_release_comes_first() {
        let mut h = Harness::new::<Envelope>(adsr());
        // The gate drops and rises on the same sample via a trigger at 1000.
        h.set_samples("gate", gate(0, 1_000, 3_000))
            .triggers("trigger", &[1_000]);
        let out = h.run(3_000);
        let o = out.float("out");
        assert_eq!(o[999], 0.5);
        assert_eq!(o[1_000], 0.0, "restart jumps to the first point");
        // With the gate low the restart plays straight through.
        assert_eq!(out.triggers("end"), &[1_000 + 1_440]);
    }

    #[test]
    fn time_scale_stretches() {
        let mut h = Harness::new::<Envelope>(adsr());
        h.triggers("trigger", &[0]).set("time_scale", 2.0);
        let out = h.run(4_000);
        assert_eq!(out.float("out")[960], 1.0);
        assert_eq!(out.triggers("end"), &[2_880]);

        // Changing the scale mid-segment keeps the level continuous.
        let scale: Vec<f32> = (0..4_000)
            .map(|i| if i < 240 { 1.0 } else { 0.5 })
            .collect();
        let mut h = Harness::new::<Envelope>(adsr());
        h.triggers("trigger", &[0]).set_samples("time_scale", scale);
        let out = h.run(4_000);
        let o = out.float("out");
        assert!((o[240] - o[239]).abs() < 0.01);
        assert_eq!(out.triggers("end"), &[240 + 120 + 240 + 240]);
    }

    #[test]
    fn first_point_after_zero_waits_at_its_level() {
        let config = json!({ "points": [[0.01, 0.5], [0.02, 1]], "sustain": -1 });
        let mut h = Harness::new::<Envelope>(config);
        h.triggers("trigger", &[0]);
        let out = h.run(2_000);
        let o = out.float("out");
        assert!(o[..480].iter().all(|&v| v == 0.5));
        assert!((o[720] - 0.75).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[960]);
    }

    #[test]
    fn bad_config_is_repaired() {
        let config = json!({ "points": [[0.01, 0.5], [0.02, 3.0], [-1, 0]], "sustain": 99 });
        let mut h = Harness::new::<Envelope>(config);
        h.triggers("trigger", &[0]);
        let out = h.run(2_000);
        let o = out.float("out");
        // Sorted, negative time → 0, level 3 → 1, sustain past the end → last.
        // Points: (0,0) (0.01,0.5) (0.02,1). Gate is low, so no hold.
        assert!((o[240] - 0.25).abs() < 1e-6);
        assert_eq!(o[480], 0.5);
        assert_eq!(o[960], 1.0);
        assert_eq!(out.triggers("end"), &[960]);

        let config = json!({ "points": [[0.01, 0.5]], "loop_start": 3, "loop_end": 1 });
        let mut h = Harness::new::<Envelope>(config);
        h.set("gate", true);
        let out = h.run(1_000);
        let o = out.float("out");
        assert_eq!(o[0], 0.5);
        assert_eq!(o[999], 0.5);

        let config = json!({ "points": "nonsense", "sustain": "x" });
        let mut h = Harness::new::<Envelope>(config);
        h.set("gate", true);
        assert_eq!(h.run(500).float("out")[480], 1.0, "default shape");
    }

    #[test]
    fn same_output_at_any_block_size() {
        let g = gate(37, 3_001, 9_000);
        let scale: Vec<f32> = (0..9_000).map(|i| 0.5 + (i % 997) as f32 / 997.0).collect();
        let config = json!({
            "points": [[0, 0], [0.005, 1, "smooth"], [0.01, 0.3, -0.5], [0.02, 0.6], [0.03, 0, 0.4]],
            "sustain": 3, "loop_start": 1, "loop_end": 3
        });
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Envelope>(config.clone(), 48_000.0, block, &NoResources);
            h.set_samples("gate", g.clone())
                .set_samples("time_scale", scale.clone())
                .triggers("trigger", &[5_000, 7_777]);
            let out = h.run(9_000);
            (
                out.float("out").to_vec(),
                out.float("active").to_vec(),
                out.triggers("end").to_vec(),
            )
        };
        let one = render(1);
        assert!(!one.2.is_empty());
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn extreme_inputs_stay_finite_and_reset_returns_to_idle() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(5_000).copied().collect();
        let g: Vec<f32> = (0..5_000).map(|i| ((i / 37) % 2) as f32).collect();
        let mut h = Harness::new::<Envelope>(json!({ "loop_start": 0, "loop_end": 2 }));
        h.set_samples("gate", g).set_samples("time_scale", wild);
        let out = h.run(5_000);
        assert!(
            out.float("out")
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        );

        let mut h = Harness::new::<Envelope>(json!({}));
        h.set("gate", true);
        h.run(1_000);
        h.reset();
        h.set("gate", false);
        let out = h.run(64);
        assert!(out.float("out").iter().all(|&v| v == 0.0));
        assert!(out.triggers("end").is_empty());
    }
}
