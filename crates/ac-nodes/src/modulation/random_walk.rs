use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::osc;
use ac_dsp::rng::Rng;

node! {
    kind: "random_walk",
    name: "Random Walk",
    category: Modulation,
    keywords: ["drift", "wander", "brownian", "smooth random", "jitter", "organic", "noise"],
    /// Smooth random modulation that wanders around `center`, never further
    /// than `range` from it.
    ///
    /// About `rate` times a second it picks a new point to head for, a
    /// random step away from the last one (bouncing back off the edges), and
    /// a smooth curve passes through the points, so the output drifts like
    /// something alive rather than jumping. Over time it roams the whole
    /// span, spending most of its time within four fifths of `range` of
    /// `center` (the curve only reaches the edges when it swings past a
    /// point near them). A negative `rate` counts as positive; 0 freezes
    /// it.
    ///
    /// The `seed` config (an integer, default 0) is mixed with the
    /// project's seed and this node's place in the patch: two RandomWalks
    /// with different seeds wander differently, and every render repeats
    /// exactly. It runs freely, even with the transport stopped, and a
    /// transport restart starts the same walk over. The output is a
    /// sample-rate signal; NaN or infinite inputs count as 0.
    ///
    /// **Try this:** add a RandomWalk (`rate` 0.5, `range` 3) to an
    /// `Oscillator`'s frequency for slow analog-style drift, or wire one
    /// (`center` 0, `range` 0.8) into a `Pan` for a sound that wanders
    /// around the stereo field.
    pub struct RandomWalk {
        seed: u64,
        rng: Rng,
        phase: f64,
        /// Four consecutive points (Catmull-Rom control points), −1..1;
        /// the curve runs from `pts[1]` to `pts[2]` over one cycle.
        pts: [f64; 4],
    }
}

const RATE: usize = 0;
const RANGE: usize = 1;
const CENTER: usize = 2;
const OUT: usize = 0;

/// Largest move between successive points, as a fraction of the half-span.
const STEP: f64 = 0.5;
/// A Catmull-Rom curve overshoots its points by at most 25%, so scaling
/// points within ±1 by 0.8 keeps the curve within ±1 without clipping it
/// (which would put corners in it).
const SCALE: f64 = 0.8;

fn finite(v: f32) -> f64 {
    if v.is_finite() { f64::from(v) } else { 0.0 }
}

/// The node's seed: the project/path seed mixed with the `seed` config
/// (any integer; fractions are truncated, anything else counts as 0).
fn mixed_seed(config: &Config, seed: u64) -> u64 {
    let mut s = config
        .get("seed")
        .and_then(|v| {
            v.as_i64()
                .map(|i| i as u64)
                .or_else(|| v.as_u64())
                .or_else(|| {
                    v.as_f64()
                        .filter(|f| f.is_finite())
                        .map(|f| f as i64 as u64)
                })
        })
        .unwrap_or(0);
    seed ^ ac_core::util::splitmix64(&mut s)
}

impl Node for RandomWalk {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("rate", Unit::Hz, 1.0)
                    .range(0.01, 20.0)
                    .log()
                    .doc("How often it picks a new direction."),
            )
            .input(
                SocketSpec::float("range", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("Furthest it strays from the center."),
            )
            .input(
                SocketSpec::float("center", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("What it wanders around."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The wandering value."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let seed = mixed_seed(config, ctx.seed);
        let mut walk = RandomWalk {
            seed,
            rng: Rng::new(seed),
            phase: 0.0,
            pts: [0.0; 4],
        };
        walk.start();
        walk
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = io.inputs.float(RATE);
        let range = io.inputs.float(RANGE);
        let center = io.inputs.float(CENTER);
        let sr = ctx.sample_rate;
        let steady = osc::increment(finite(rate.first()).abs(), sr);
        let varying = rate.is_varying();
        let mut out = io.outputs.float(OUT);
        for i in 0..ctx.frames {
            let v = (SCALE * catmull_rom(self.pts, self.phase)).clamp(-1.0, 1.0);
            let y = finite(center.at(i)) + finite(range.at(i)) * v;
            let y = y as f32;
            out.set(i, if y.is_finite() { y } else { 0.0 });
            let inc = if varying {
                osc::increment(finite(rate.at(i)).abs(), sr)
            } else {
                steady
            };
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                let next = self.walk(self.pts[3]);
                self.pts = [self.pts[1], self.pts[2], self.pts[3], next];
            }
        }
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        self.start();
    }
}

impl RandomWalk {
    fn start(&mut self) {
        let first = self.rng.bipolar();
        let second = self.walk(first);
        let third = self.walk(second);
        self.pts = [first, first, second, third];
        self.phase = 0.0;
    }

    /// A random step from `x`, folded back into −1..1.
    fn walk(&mut self, x: f64) -> f64 {
        let y = x + STEP * self.rng.bipolar();
        if y > 1.0 {
            2.0 - y
        } else if y < -1.0 {
            -2.0 - y
        } else {
            y
        }
    }
}

/// Catmull-Rom spline through `p[1]` (t = 0) and `p[2]` (t = 1) (E. Catmull
/// and R. Rom, "A class of local interpolating splines", 1974).
#[inline]
fn catmull_rom(p: [f64; 4], t: f64) -> f64 {
    let [p0, p1, p2, p3] = p;
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn stays_within_center_plus_minus_range() {
        let mut h = Harness::new::<RandomWalk>(json!({}));
        h.set("rate", 50.0).set("range", 0.3).set("center", 2.0);
        let o = h.run(96_000).float("out").to_vec();
        assert!(o.iter().all(|&v| (1.7 - 1e-6..=2.3 + 1e-6).contains(&v)));
        // It does wander across most of the span.
        let lo = o.iter().copied().fold(f32::MAX, f32::min);
        let hi = o.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi - lo > 0.2, "{lo}..{hi}");
    }

    #[test]
    fn output_is_smooth() {
        let mut h = Harness::new::<RandomWalk>(json!({}));
        h.set("rate", 10.0);
        let o = h.run(48_000).float("out").to_vec();
        // At 10 Hz no sample-to-sample step comes near a jump.
        let max_step = o
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step < 0.01, "{max_step}");
        // And the slope is continuous: second differences stay tiny.
        let max_curve = o
            .windows(3)
            .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_curve < 1e-4, "{max_curve}");
    }

    #[test]
    fn rate_sets_how_fast_it_moves() {
        let travel = |rate: f32| {
            let mut h = Harness::new::<RandomWalk>(json!({}));
            h.set("rate", rate);
            let o = h.run(48_000).float("out").to_vec();
            o.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
        };
        assert!(travel(8.0) > 3.0 * travel(1.0));
        let mut h = Harness::new::<RandomWalk>(json!({}));
        h.set("rate", 0.0);
        let o = h.run(1_000).float("out").to_vec();
        assert!(o.iter().all(|&v| v == o[0]), "rate 0 freezes");
    }

    #[test]
    fn seeds_repeat_and_differ() {
        let render = |config: serde_json::Value| {
            let mut h = Harness::new::<RandomWalk>(config);
            h.set("rate", 5.0);
            h.run(10_000).float("out").to_vec()
        };
        assert_eq!(render(json!({})), render(json!({})));
        assert_eq!(render(json!({"seed": 3})), render(json!({"seed": 3})));
        assert_ne!(render(json!({"seed": 3})), render(json!({"seed": 4})));
        assert_ne!(render(json!({"seed": -1})), render(json!({})));
    }

    #[test]
    fn same_output_at_any_block_size_and_after_reset() {
        let rate: Vec<f32> = (0..6_000)
            .map(|i| 2.0 + (i % 1_000) as f32 * 0.01)
            .collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<RandomWalk>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("rate", rate.clone()).set("range", 0.5);
            h.run(6_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));

        let mut h = Harness::new::<RandomWalk>(json!({}));
        h.set("rate", 7.0);
        let first = h.run(3_000).float("out").to_vec();
        h.reset();
        assert_eq!(h.run(3_000).float("out"), first.as_slice());
    }

    #[test]
    fn wild_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 3e38];
        let w: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let mut h = Harness::new::<RandomWalk>(json!({}));
        h.set_samples("rate", w.clone())
            .set_samples("range", w.iter().rev().copied().collect())
            .set_samples("center", w);
        assert!(h.run(2_000).float("out").iter().all(|v| v.is_finite()));
    }
}
