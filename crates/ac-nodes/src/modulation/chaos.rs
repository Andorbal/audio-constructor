use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::rng::Rng;

use super::random_walk::mixed_seed;

node! {
    kind: "chaos",
    name: "Chaos",
    category: Modulation,
    keywords: ["lorenz", "rossler", "rössler", "attractor", "strange", "chaotic", "wander", "organic"],
    /// Modulation from a strange attractor: three signals, `x`, `y`, and
    /// `z`, that swirl in a pattern that never quite repeats, each between
    /// about −1 and 1.
    ///
    /// The `system` picks the equations. **Lorenz** (E. Lorenz's 1963
    /// weather model) loops around one of two lobes, then flips to the
    /// other at moments you can't predict: `x` swings between positive and
    /// negative stretches, `z` rises and falls with each loop. **Rössler**
    /// (O. Rössler, 1976) circles one center: `x` and `y` are a wobbly
    /// sine and cosine pair, and `z` stays low with a sharp spike once in
    /// a while.
    ///
    /// `rate` sets the speed, about how many loops it makes a second.
    /// `chaos` bends the shape: at low values Lorenz settles to a steady
    /// point and Rössler to a plain, repeating loop; higher values wander
    /// further (Rössler doubles its loop, then doubles again, on its way to
    /// chaos). Both are read every sample, so they can be modulated.
    /// `reset` starts the motion over from its first point on that sample.
    ///
    /// The motion is computed, not random: the same project renders the
    /// same curves every time, at any block size. The starting point is
    /// nudged by the project's seed, this node's place in the patch, and
    /// the `seed` config (an integer, default 0), so two Chaos nodes drift
    /// apart. It runs freely, even with the transport stopped, and a
    /// transport restart starts the same motion over. NaN or infinite
    /// inputs count as their defaults.
    ///
    /// **Try this:** wire `x` through a `Map` (−1..1 → 300..3000 Hz) into
    /// a `Biquad`'s `cutoff` and `z` into a `Pan`, with `rate` around 0.3,
    /// for a filter that sweeps like weather; or set `system` to Rössler,
    /// `rate` to 2, and sweep `chaos` slowly from 0 to 1 to hear the loop
    /// split as it turns chaotic.
    pub struct Chaos {
        seed: u64,
        /// The start, nudged by the seed.
        start: [f64; 3],
        state: [f64; 3],
        /// This block's scaled `x, y, z`, so all three outputs come from
        /// one pass.
        scratch: Vec<[f32; 3]>,
    }
}

const SYSTEM: usize = 0;
const RATE: usize = 1;
const CHAOS: usize = 2;
const RESET: usize = 3;
const X: usize = 0;
const Y: usize = 1;
const Z: usize = 2;

/// Systems, in choice order.
pub const SYSTEMS: &[&str] = &["lorenz", "rossler"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum System {
    Lorenz,
    Rossler,
}

impl System {
    fn from_index(i: usize) -> System {
        if i == 1 {
            System::Rossler
        } else {
            System::Lorenz
        }
    }

    /// Attractor time per loop, so `rate` reads as loops per second.
    fn loop_time(self) -> f64 {
        match self {
            System::Lorenz => 0.75,
            System::Rossler => 6.1,
        }
    }

    /// Largest integration step that keeps fourth-order Runge-Kutta
    /// accurate on this system.
    fn max_step(self) -> f64 {
        match self {
            System::Lorenz => 0.005,
            System::Rossler => 0.05,
        }
    }

    /// The parameter `chaos` (0..1) sets: Lorenz's ρ, Rössler's c.
    fn param(self, chaos: f64) -> f64 {
        match self {
            System::Lorenz => 20.0 + 30.0 * chaos,
            System::Rossler => 2.0 + 7.0 * chaos,
        }
    }

    /// The flow at `p` (E. N. Lorenz, "Deterministic nonperiodic flow",
    /// J. Atmos. Sci. 20, 1963, with σ = 10, β = 8/3; O. E. Rössler, "An
    /// equation for continuous chaos", Phys. Lett. A 57, 1976, with
    /// a = b = 0.2).
    fn flow(self, k: f64, p: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = p;
        match self {
            System::Lorenz => [10.0 * (y - x), x * (k - z) - y, x * y - 8.0 / 3.0 * z],
            System::Rossler => [-y - z, x + 0.2 * y, 0.2 + z * (x - k)],
        }
    }

    /// `p` scaled to about −1..1. The attractors grow with their
    /// parameter; these spans fit them across `chaos` 0..1 (measured by
    /// simulation): Lorenz's lobes sit at ±√(β(ρ − 1)) with z around ρ − 1,
    /// Rössler's loop widens about as 1.9c and its z spikes reach 0.65c².
    fn scale(self, k: f64, p: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = p;
        match self {
            System::Lorenz => {
                let s = (8.0 / 3.0 * (k - 1.0)).max(1.0).sqrt();
                let zc = (k - 1.0).max(1.0);
                [x / (2.3 * s), y / (3.4 * s), (z - zc) / (0.8 * zc)]
            }
            System::Rossler => {
                let w = 1.9 * k + 1.0;
                [x / w, y / w, 2.0 * z / (0.65 * k * k).max(1.0) - 1.0]
            }
        }
    }
}

/// One fourth-order Runge-Kutta step of `h` (W. H. Press et al.,
/// "Numerical Recipes", §17.1).
fn rk4(sys: System, k: f64, p: [f64; 3], h: f64) -> [f64; 3] {
    let add =
        |a: [f64; 3], b: [f64; 3], s: f64| [a[0] + s * b[0], a[1] + s * b[1], a[2] + s * b[2]];
    let k1 = sys.flow(k, p);
    let k2 = sys.flow(k, add(p, k1, h / 2.0));
    let k3 = sys.flow(k, add(p, k2, h / 2.0));
    let k4 = sys.flow(k, add(p, k3, h));
    [
        p[0] + h / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
        p[1] + h / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
        p[2] + h / 6.0 * (k1[2] + 2.0 * k2[2] + 2.0 * k3[2] + k4[2]),
    ]
}

/// Fastest `rate` honoured, in loops per second.
const MAX_RATE: f64 = 200.0;
/// Most integration steps per sample (a fast rate at a low sample rate).
const MAX_STEPS: usize = 64;
/// A state further out than this has left the attractor (a jump in
/// `chaos` or `system`); start over.
const ESCAPE: f64 = 1e4;

fn finite_or(v: f32, default: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { default }
}

impl Chaos {
    /// Advance the state by `t` units of attractor time.
    fn advance(&mut self, sys: System, k: f64, t: f64) {
        if t <= 0.0 {
            return;
        }
        let steps = ((t / sys.max_step()).ceil() as usize).clamp(1, MAX_STEPS);
        let h = t / steps as f64;
        for _ in 0..steps {
            self.state = rk4(sys, k, self.state, h);
        }
        if !self.state.iter().all(|v| v.is_finite() && v.abs() < ESCAPE) {
            self.state = self.start;
        }
    }
}

impl Node for Chaos {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        let out = |name: &'static str, doc: &'static str| {
            SocketSpec::float(name, Unit::Generic, 0.0)
                .rate(Rate::Sample)
                .doc(doc)
        };
        Signature::new()
            .input(SocketSpec::choice("system", SYSTEMS, "lorenz").doc("Which attractor."))
            .input(
                SocketSpec::float("rate", Unit::Hz, 1.0)
                    .range(0.01, 50.0)
                    .log()
                    .doc("Speed: about how many loops a second."),
            )
            .input(
                SocketSpec::float("chaos", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Steady or looping at 0, wilder towards 1."),
            )
            .input(SocketSpec::trigger("reset").doc("Start over from the first point."))
            .output(out("x", "First coordinate, about −1..1."))
            .output(out("y", "Second coordinate, about −1..1."))
            .output(out("z", "Third coordinate, about −1..1."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let seed = mixed_seed(config, ctx.seed);
        let mut node = Chaos {
            seed,
            start: [0.0; 3],
            state: [0.0; 3],
            scratch: vec![[0.0; 3]; ctx.max_block.max(1)],
        };
        node.reset();
        node
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let system = io.inputs.int(SYSTEM).max(0) as usize;
        let sys = System::from_index(system);
        let rate = io.inputs.float(RATE);
        let chaos = io.inputs.float(CHAOS);
        let resets = io.inputs.triggers(RESET);
        let per_second = sys.loop_time() / ctx.sample_rate.max(1.0);
        let mut next_reset = resets.iter().peekable();
        let frames = ctx.frames.min(self.scratch.len());
        for i in 0..frames {
            while next_reset.next_if(|&&t| t as usize <= i).is_some() {
                self.state = self.start;
            }
            let k = sys.param(finite_or(chaos.at(i), 0.5).clamp(0.0, 1.0));
            let p = sys.scale(k, self.state);
            self.scratch[i] = p.map(|v| v.clamp(-1.0, 1.0) as f32);
            let r = finite_or(rate.at(i), 1.0).clamp(0.0, MAX_RATE);
            self.advance(sys, k, r * per_second);
        }
        for (c, o) in [X, Y, Z].into_iter().enumerate() {
            let mut out = io.outputs.float(o);
            for (i, p) in self.scratch.iter().take(frames).enumerate() {
                out.set(i, p[c]);
            }
        }
    }

    fn reset(&mut self) {
        // A small nudge from the seed, so nodes with different seeds or
        // places drift apart.
        let mut rng = Rng::new(self.seed);
        self.start = [
            1.0 + 0.1 * rng.bipolar(),
            1.0 + 0.1 * rng.bipolar(),
            1.0 + 0.1 * rng.bipolar(),
        ];
        self.state = self.start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn run(
        config: serde_json::Value,
        system: &str,
        rate: f64,
        chaos: f64,
        n: usize,
    ) -> Vec<Vec<f32>> {
        let mut h = Harness::new::<Chaos>(config);
        h.set("system", system)
            .set("rate", rate)
            .set("chaos", chaos);
        let out = h.run(n);
        vec![
            out.float("x").to_vec(),
            out.float("y").to_vec(),
            out.float("z").to_vec(),
        ]
    }

    #[test]
    fn outputs_stay_within_one_and_use_the_range() {
        for system in SYSTEMS {
            for chaos in [0.3, 0.5, 0.75, 1.0] {
                // A hundred loops, so Lorenz visits both lobes.
                let out = run(json!({}), system, 10.0, chaos, 480_000);
                for (name, x) in ["x", "y", "z"].iter().zip(&out) {
                    let (lo, hi) = x[48_000..]
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
                    assert!(x.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
                    assert!(hi - lo > 0.7, "{system} {chaos} {name}: {lo}..{hi}");
                }
            }
        }
    }

    #[test]
    fn lorenz_flips_between_its_lobes() {
        let x = &run(json!({}), "lorenz", 2.0, 0.5, 480_000)[0];
        // Count stretches of each sign of x: a chaotic Lorenz changes lobe
        // irregularly, many times in ten seconds of two loops a second.
        let flips = x
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        assert!(flips > 6, "{flips} flips");
        // Below the onset of chaos it spirals in towards one point.
        let calm = &run(json!({}), "lorenz", 4.0, 0.0, 480_000)[0];
        let spread = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max((v - x[0]).abs()));
        let (early, late) = (spread(&calm[..48_000]), spread(&calm[432_000..]));
        assert!(late < 0.02 && late < early / 10.0, "{early} → {late}");
    }

    #[test]
    fn rossler_at_zero_chaos_repeats_at_its_rate() {
        // c = 2: a single loop of period about 6.1 time units, which `rate`
        // maps to one loop a second.
        let rate = 4.0;
        let x = &run(json!({}), "rossler", rate, 0.0, 192_000)[0];
        let tail = &x[96_000..];
        let ups = tail
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        let loops_per_second = ups as f64 / 2.0;
        assert!(
            (loops_per_second - rate).abs() < 0.6,
            "{loops_per_second} loops a second"
        );
    }

    #[test]
    fn it_is_sample_rate_independent() {
        // The same motion over one second at two rates, compared where the
        // samples line up (every 160 ms).
        let at = |rate: f64| {
            let mut h = Harness::with_resources::<Chaos>(json!({}), rate, 64, &NoResources);
            h.set("rate", 1.0);
            let n = rate as usize;
            let x = h.run(n).float("x").to_vec();
            (0..6).map(|k| x[k * n * 16 / 100]).collect::<Vec<f32>>()
        };
        let (a, b) = (at(48_000.0), at(96_000.0));
        for (p, q) in a.iter().zip(&b) {
            assert!((p - q).abs() < 1e-3, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn block_size_and_reset_repeat_exactly() {
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Chaos>(json!({}), 48_000.0, block, &NoResources);
            h.set("rate", 3.0).triggers("reset", &[5_000]);
            h.run(20_000).float("y").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1024));
        // The reset at 5000 starts the same motion again.
        assert_eq!(&one[5_000..10_000], &one[..5_000]);
        let mut h = Harness::new::<Chaos>(json!({}));
        let a = h.run(3_000).float("z").to_vec();
        h.reset();
        assert_eq!(h.run(3_000).float("z"), a);
    }

    #[test]
    fn seeds_drift_apart_and_repeat() {
        // Nearby starts separate exponentially; after some thirty loops
        // they're unrelated.
        let a = run(json!({ "seed": 1 }), "lorenz", 4.0, 0.5, 480_000);
        let b = run(json!({ "seed": 2 }), "lorenz", 4.0, 0.5, 480_000);
        assert_eq!(a, run(json!({ "seed": 1 }), "lorenz", 4.0, 0.5, 480_000));
        let diff = a[0][360_000..]
            .iter()
            .zip(&b[0][360_000..])
            .fold(0.0f32, |m, (p, q)| m.max((p - q).abs()));
        assert!(diff > 0.5, "{diff}");
    }

    #[test]
    fn extreme_and_bad_inputs_stay_finite() {
        for (rate, chaos) in [
            (0.0, 0.5),
            (1e9, 1.0),
            (f64::NAN, f64::INFINITY),
            (-5.0, -3.0),
        ] {
            for system in SYSTEMS {
                let mut h = Harness::with_resources::<Chaos>(json!({}), 8_000.0, 1, &NoResources);
                h.set("system", *system)
                    .set("rate", rate)
                    .set("chaos", chaos);
                let out = h.run(4_000);
                for name in ["x", "y", "z"] {
                    assert!(
                        out.float(name)
                            .iter()
                            .all(|v| v.is_finite() && v.abs() <= 1.0)
                    );
                }
            }
        }
        // Rate 0 holds still.
        let x = &run(json!({}), "lorenz", 0.0, 0.5, 100)[0];
        assert!(x.iter().all(|v| *v == x[0]));
    }
}
