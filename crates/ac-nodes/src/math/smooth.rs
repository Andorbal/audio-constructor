use ac_core::{
    Config, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit,
    node,
};

use ac_dsp::util::snap_samples;

use super::{finite_f32, finite_or_zero};

node! {
    kind: "smooth",
    name: "Smooth",
    category: Math,
    keywords: ["slew", "lag", "glide", "portamento", "filter", "ease", "dezipper"],
    /// Smooths a changing value: a slew limiter or a one-pole smoother, with
    /// separate times for rising and falling.
    ///
    /// - `linear`: a slew limiter. The output moves toward `in` at a fixed
    ///   speed: at most 1 unit per `rise` seconds going up and 1 unit per
    ///   `fall` seconds going down, so a jump of 0.5 takes half the time.
    /// - `exponential` (default): a one-pole smoother. `rise` and `fall` are
    ///   time constants: the output covers about 63% of the remaining
    ///   distance in that time (99% in about 4.6 times as long), quickly at
    ///   first and then easing in.
    ///
    /// A time of 0 (or less) follows `in` immediately in that direction.
    /// The first value after loading or a transport restart is taken as-is,
    /// so the output never glides up from 0 at the start. The output is a
    /// sample-rate signal even when `in` changes once per block, which is
    /// what turns a stepped control into a smooth one. A NaN or infinite
    /// input counts as 0.
    ///
    /// **Try this:** put Smooth (`exponential`, rise and fall 0.05) between a
    /// `MidiNote` note and `Convert` (`note_to_hz`) for portamento, or set
    /// `rise` short and `fall` long on a level signal for a quick-attack,
    /// slow-release envelope.
    pub struct Smooth {
        y: f64,
        primed: bool,
    }
}

const IN: usize = 0;
const RISE: usize = 1;
const FALL: usize = 2;
const MODE: usize = 3;
const OUT: usize = 0;

const MODES: &[&str] = &["linear", "exponential"];

/// Per-sample step for a time in seconds: the slew per sample (linear) or
/// the one-pole coefficient (exponential). Times of 0 or less, or NaN,
/// mean "follow immediately".
#[inline]
fn coefficient(linear: bool, seconds: f32, rate: f64) -> f64 {
    let samples = snap_samples(f64::from(seconds) * rate);
    if linear {
        if samples > 0.0 {
            1.0 / samples
        } else {
            f64::INFINITY
        }
    } else if samples > 0.0 {
        (-1.0 / samples).exp()
    } else {
        0.0
    }
}

impl Node for Smooth {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The value to smooth."))
            .input(
                SocketSpec::float("rise", Unit::Seconds, 0.1)
                    .range(0.0, 10.0)
                    .doc("Time for rising changes."),
            )
            .input(
                SocketSpec::float("fall", Unit::Seconds, 0.1)
                    .range(0.0, 10.0)
                    .doc("Time for falling changes."),
            )
            .input(SocketSpec::choice("mode", MODES, "exponential").doc("Slew or one-pole."))
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The smoothed value."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Smooth {
            y: 0.0,
            primed: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let rise = io.inputs.float(RISE);
        let fall = io.inputs.float(FALL);
        let linear = io.inputs.choice(MODE, MODES.len()) == 0;
        let rate = ctx.sample_rate;
        if !self.primed {
            self.y = f64::from(finite_or_zero(x.first()));
            self.primed = true;
        }
        let varying = rise.is_varying() || fall.is_varying();
        let steady = (
            coefficient(linear, rise.first(), rate),
            coefficient(linear, fall.first(), rate),
        );
        let coefs = |i: usize| {
            if varying {
                (
                    coefficient(linear, rise.at(i), rate),
                    coefficient(linear, fall.at(i), rate),
                )
            } else {
                steady
            }
        };
        let mut out = io.outputs.float(OUT);
        let y = &mut self.y;
        if linear {
            run(&mut out, x, coefs, y, |y, target, step| {
                let d = target - y;
                // The slack absorbs rounding so a slew lands on its sample.
                if d.abs() <= step * (1.0 + 1e-9) {
                    target
                } else {
                    y + step.copysign(d)
                }
            });
        } else {
            run(&mut out, x, coefs, y, |y, target, c| {
                target + (y - target) * c
            });
        }
    }

    fn reset(&mut self) {
        self.y = 0.0;
        self.primed = false;
    }
}

/// The per-sample loop: `f(y, target, coef)` with the rise coefficient when
/// heading up and the fall coefficient when heading down.
#[inline]
fn run(
    out: &mut ac_core::FloatOut,
    x: FloatIn,
    coefs: impl Fn(usize) -> (f64, f64),
    y: &mut f64,
    f: impl Fn(f64, f64, f64) -> f64,
) {
    out.fill_with(|i| {
        let target = f64::from(finite_or_zero(x.at(i)));
        let (up, down) = coefs(i);
        let c = if target >= *y { up } else { down };
        *y = f(*y, target, c);
        if !y.is_finite() {
            *y = target;
        }
        finite_f32(*y)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn step_input(at: usize, n: usize, from: f32, to: f32) -> Vec<f32> {
        (0..n).map(|i| if i < at { from } else { to }).collect()
    }

    #[test]
    fn linear_slews_at_one_unit_per_rise_time() {
        let mut h = Harness::new::<Smooth>(json!({}));
        h.set_samples("in", step_input(10, 2_000, 0.0, 0.5))
            .set("mode", "linear")
            .set("rise", 0.02);
        let o = h.run(2_000).float("out").to_vec();
        // 1 unit per 0.02 s = 960 samples, so 0.5 takes 480 samples.
        assert_eq!(o[9], 0.0);
        assert!((o[10] - 1.0 / 960.0).abs() < 1e-6);
        assert!((o[10 + 239] - 0.25).abs() < 1e-5);
        assert!(o[10 + 478] < 0.5);
        assert_eq!(o[10 + 479], 0.5);
        assert!(o[10 + 480..].iter().all(|&v| v == 0.5));
    }

    #[test]
    fn rise_and_fall_are_separate() {
        let mut x = step_input(0, 3_000, 1.0, 1.0);
        x[..10].fill(0.0);
        x[1_000..].fill(0.0);
        let mut h = Harness::new::<Smooth>(json!({}));
        h.set_samples("in", x)
            .set("mode", "linear")
            .set("rise", 0.0)
            .set("fall", 0.01);
        let o = h.run(3_000).float("out").to_vec();
        assert_eq!(o[10], 1.0, "rise 0 jumps");
        // Falls 1 unit in 480 samples.
        assert!((o[1_000 + 239] - 0.5).abs() < 1e-4);
        assert_eq!(o[1_000 + 479], 0.0);
    }

    #[test]
    fn exponential_covers_63_percent_in_one_time_constant() {
        let mut h = Harness::new::<Smooth>(json!({}));
        h.set_samples("in", step_input(1, 10_000, 0.0, 1.0))
            .set("rise", 0.01);
        let o = h.run(10_000).float("out").to_vec();
        let after = o[480];
        assert!((after - (1.0 - (-1.0f32).exp())).abs() < 2e-3, "{after}");
        assert!(o.windows(2).all(|w| w[1] >= w[0]));
        assert!(o[9_999] > 0.999);
    }

    #[test]
    fn first_value_is_taken_as_is_and_reset_reprimes() {
        let mut h = Harness::new::<Smooth>(json!({}));
        h.set("in", 5.0).set("rise", 1.0);
        assert_eq!(h.run(64).float("out"), vec![5.0; 64]);
        h.set("in", 0.0);
        assert!(h.run(64).float("out")[63] > 4.0);
        h.reset();
        h.set("in", -3.0);
        assert_eq!(h.run(1).float("out")[0], -3.0);
    }

    #[test]
    fn block_rate_input_becomes_a_smooth_sample_rate_output() {
        let mut h = Harness::new::<Smooth>(json!({}));
        let x: Vec<f32> = (0..256).map(|i| if i < 64 { 0.0 } else { 1.0 }).collect();
        h.set_block("in", x)
            .set("mode", "linear")
            .set("rise", 0.004);
        let o = h.run(256).float("out").to_vec();
        // 1 unit per 192 samples, starting at frame 64.
        assert!((o[64 + 95] - 0.5).abs() < 1e-4);
        assert!(o[64..64 + 191].windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn same_output_at_any_block_size() {
        let x: Vec<f32> = (0..3_000).map(|i| ((i / 300) % 2) as f32).collect();
        let fall: Vec<f32> = (0..3_000).map(|i| 0.001 + i as f32 * 1e-6).collect();
        for mode in MODES {
            let render = |block: usize| {
                let mut h =
                    Harness::with_resources::<Smooth>(json!({}), 48_000.0, block, &NoResources);
                h.set_samples("in", x.clone())
                    .set_samples("fall", fall.clone())
                    .set("rise", 0.002)
                    .set("mode", *mode);
                h.run(3_000).float("out").to_vec()
            };
            let one = render(1);
            assert_eq!(one, render(64), "{mode}");
            assert_eq!(one, render(512), "{mode}");
        }
    }

    #[test]
    fn wild_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30, 3e38];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        for mode in MODES {
            let mut h = Harness::new::<Smooth>(json!({}));
            h.set_samples("in", wild.clone())
                .set_samples("rise", wild.iter().rev().copied().collect())
                .set_samples("fall", wild.clone())
                .set("mode", *mode);
            assert!(
                h.run(2_000).float("out").iter().all(|v| v.is_finite()),
                "{mode}"
            );
        }
    }
}
