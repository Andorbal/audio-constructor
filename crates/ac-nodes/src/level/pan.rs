use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

node! {
    kind: "pan",
    name: "Pan",
    category: Level,
    keywords: ["balance", "stereo", "panner", "position", "left", "right"],
    /// Places a mono signal in the stereo field, or shifts the balance of a
    /// stereo one. The output is always stereo.
    ///
    /// `pan` runs from -1 (left) through 0 (center) to 1 (right).
    ///
    /// - **Mono in:** constant-power (sine/cosine) panning, so the sound
    ///   keeps the same loudness as it moves. At the center each side is
    ///   -3 dB (×0.707).
    /// - **Stereo in:** balance. The side being panned toward stays at full
    ///   level and the other side fades out along a quarter cosine, so at
    ///   the center a stereo signal passes unchanged.
    ///
    /// **Try this:** wire a slow `pure` sine Oscillator (0.2 Hz) into `pan`
    /// for auto-pan, or put one Pan per track before a `Mix` to place a
    /// band across the stereo field.
    pub struct Pan {}
}

const IN: usize = 0;
const PAN: usize = 1;
const OUT: usize = 0;

/// Left and right gains for a pan position.
///
/// Mono uses the constant-power law `(cos θ, sin θ)` with θ from 0 to π/2
/// across the field (C. Roads, "The Computer Music Tutorial", MIT Press 1996,
/// ch. 10, constant-power panning). Stereo balance applies the cosine half of
/// the same law to the far side only.
#[inline]
fn gains(pan: f32, stereo: bool) -> (f32, f32) {
    let p = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    if stereo {
        if p > 0.0 {
            ((p * FRAC_PI_2).cos(), 1.0)
        } else {
            (1.0, (-p * FRAC_PI_2).cos())
        }
    } else {
        let theta = (p + 1.0) * FRAC_PI_4;
        (theta.cos(), theta.sin())
    }
}

impl Node for Pan {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(Channels::UpTo(2))
                    .doc("Mono or stereo signal."),
            )
            .input(
                SocketSpec::float("pan", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("-1 left, 0 center, 1 right."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(2))
                    .doc("Stereo output."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Pan {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let pan = io.inputs.float(PAN);
        let stereo = input.channels() >= 2;
        // For a mono input both read channel 0.
        let (src_l, src_r) = (input.channel(0), input.channel(1));
        let mut out = io.outputs.audio(OUT);
        let Some((l, r)) = out.stereo() else {
            out.fill(0.0);
            return;
        };
        let frames = l.iter_mut().zip(r.iter_mut()).zip(src_l.iter().zip(src_r));
        if pan.is_varying() {
            for (i, ((ol, or), (sl, sr))) in frames.enumerate() {
                let (gl, gr) = gains(pan.at(i), stereo);
                *ol = sl * gl;
                *or = sr * gr;
            }
        } else {
            let (gl, gr) = gains(pan.first(), stereo);
            for ((ol, or), (sl, sr)) in frames {
                *ol = sl * gl;
                *or = sr * gr;
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;
    use std::f32::consts::FRAC_1_SQRT_2;

    fn run(input: Vec<Vec<f32>>, pan: f64) -> Vec<Vec<f32>> {
        let mut h = Harness::new::<Pan>(json!({}));
        let n = input[0].len();
        h.set_audio("in", input).set("pan", pan);
        h.run(n).audio("out").to_vec()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn mono_center_is_minus_three_db_each_side() {
        let out = run(vec![vec![1.0; 64]], 0.0);
        assert_eq!(out.len(), 2);
        assert!(close(out[0][10], FRAC_1_SQRT_2));
        assert!(close(out[1][10], FRAC_1_SQRT_2));
    }

    #[test]
    fn mono_hard_left_and_right() {
        let left = run(vec![vec![1.0; 64]], -1.0);
        assert!(close(left[0][5], 1.0) && close(left[1][5], 0.0));
        let right = run(vec![vec![1.0; 64]], 1.0);
        assert!(close(right[0][5], 0.0) && close(right[1][5], 1.0));
    }

    #[test]
    fn mono_panning_keeps_constant_power() {
        for p in [-0.8, -0.3, 0.1, 0.6] {
            let out = run(vec![vec![1.0; 8]], p);
            let power = out[0][0].powi(2) + out[1][0].powi(2);
            assert!(close(power, 1.0), "{p}: {power}");
        }
    }

    #[test]
    fn stereo_balance() {
        let st = || vec![vec![0.5; 64], vec![-0.25; 64]];
        let center = run(st(), 0.0);
        assert!(close(center[0][3], 0.5) && close(center[1][3], -0.25));
        let half_right = run(st(), 0.5);
        assert!(close(half_right[0][3], 0.5 * FRAC_1_SQRT_2));
        assert!(close(half_right[1][3], -0.25));
        let hard_left = run(st(), -1.0);
        assert!(close(hard_left[0][3], 0.5) && close(hard_left[1][3], 0.0));
    }

    #[test]
    fn sample_rate_pan_modulation() {
        let mut h = Harness::with_resources::<Pan>(json!({}), 48_000.0, 16, &NoResources);
        let pan: Vec<f32> = (0..64).map(|i| if i < 32 { -1.0 } else { 1.0 }).collect();
        h.set_audio("in", vec![vec![1.0; 64]])
            .set_samples("pan", pan);
        let out = h.run(64);
        let o = out.audio("out");
        assert!(close(o[0][31], 1.0) && close(o[1][31], 0.0));
        assert!(close(o[0][32], 0.0) && close(o[1][32], 1.0));
    }

    #[test]
    fn unwired_input_and_wild_pan_are_silent_and_finite() {
        let mut h = Harness::new::<Pan>(json!({}));
        h.set_samples("pan", vec![f32::NAN, f32::INFINITY, -1e30, 3.0]);
        let out = h.run(4);
        assert_eq!(out.audio("out").len(), 2);
        assert!(out.audio("out").iter().flatten().all(|v| *v == 0.0));
        let gains = [f32::NAN, f32::INFINITY, -1e30].map(|p| gains(p, false));
        assert!(gains.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
    }
}
