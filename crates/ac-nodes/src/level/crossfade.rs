use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "crossfade",
    name: "Crossfade",
    category: Level,
    keywords: ["blend", "xfade", "fader", "morph", "dry wet", "a/b", "switch", "mix two"],
    /// Blends two signals: `mix` 0 is all `a`, 1 is all `b`, and values in
    /// between mix them.
    ///
    /// - `curve` **linear** (default) adds `a·(1 − mix) + b·mix`: right for
    ///   two versions of the same signal (dry and wet) and for control
    ///   values, where the midpoint is the average.
    /// - `curve` **equal_power** uses `cos`/`sin` gains (−3 dB each at the
    ///   midpoint), so two unrelated sounds keep the same loudness all the
    ///   way across instead of dipping in the middle.
    ///
    /// `mix` is held within 0..1 and can be modulated at audio rate. The
    /// output has as many channels as the wider input; a mono input is
    /// copied to every channel. `Float` signals (an LFO, an envelope) can be
    /// wired into `a` and `b` too, through the automatic Float → Audio
    /// conversion, and the output wired into a `Float` parameter reads
    /// channel 0, so Crossfade blends control signals as well as audio.
    ///
    /// **Try this:** wire a dry signal into `a` and the same signal through
    /// a `Delay` or `Biquad` into `b` for a wet/dry control, or drive `mix`
    /// from a slow `Oscillator` (`pure`, mapped to 0..1) with `equal_power`
    /// to sweep between two loops.
    pub struct Crossfade {}
}

const A: usize = 0;
const B: usize = 1;
const MIX: usize = 2;
const CURVE: usize = 3;
const OUT: usize = 0;

const CURVES: &[&str] = &["linear", "equal_power"];

/// Gains for `a` and `b` at `mix`.
#[inline]
fn gains(mix: f32, equal_power: bool) -> (f32, f32) {
    let m = if mix.is_finite() {
        mix.clamp(0.0, 1.0)
    } else {
        0.5
    };
    if equal_power {
        let (s, c) = (m * std::f32::consts::FRAC_PI_2).sin_cos();
        (c, s)
    } else {
        (1.0 - m, m)
    }
}

impl Node for Crossfade {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("a").doc("Heard at `mix` 0."))
            .input(SocketSpec::audio("b").doc("Heard at `mix` 1."))
            .input(
                SocketSpec::float("mix", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("0 is all `a`, 1 is all `b`."),
            )
            .input(
                SocketSpec::choice("curve", CURVES, "linear")
                    .doc("Linear for related signals, equal power for unrelated ones."),
            )
            .output(SocketSpec::audio("out").doc("The blend."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Crossfade {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.audio(A);
        let b = io.inputs.audio(B);
        let mix = io.inputs.float(MIX);
        let equal_power = io.inputs.choice(CURVE, CURVES.len()) == 1;
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let (xa, xb) = (a.channel(c), b.channel(c));
            let dst = out.channel(c);
            if mix.is_varying() {
                for (i, ((o, va), vb)) in dst.iter_mut().zip(xa).zip(xb).enumerate() {
                    let (ga, gb) = gains(mix.at(i), equal_power);
                    *o = va * ga + vb * gb;
                }
            } else {
                let (ga, gb) = gains(mix.first(), equal_power);
                for ((o, va), vb) in dst.iter_mut().zip(xa).zip(xb) {
                    *o = va * ga + vb * gb;
                }
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, rms};
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<Crossfade>(json!({}), 48_000.0, block, &NoResources)
    }

    #[test]
    fn linear_blends_by_mix() {
        for (m, want) in [(0.0, 1.0), (1.0, 3.0), (0.25, 1.5), (-2.0, 1.0), (7.0, 3.0)] {
            let mut h = harness(64);
            h.set_audio("a", vec![vec![1.0; 64]])
                .set_audio("b", vec![vec![3.0; 64]])
                .set("mix", m);
            assert_eq!(h.run(64).audio("out")[0][10], want, "mix {m}");
        }
    }

    #[test]
    fn equal_power_keeps_unrelated_signals_level() {
        let tone = |f: f32| -> Vec<f32> { (0..4_800).map(|i| (i as f32 * f).sin()).collect() };
        let level = |curve: &str| {
            let mut h = harness(64);
            h.set_audio("a", vec![tone(0.05)])
                .set_audio("b", vec![tone(0.13)])
                .set("curve", curve);
            rms(&h.run(4_800).audio("out")[0])
        };
        // Two unrelated sines at the midpoint: equal power keeps the RMS of
        // one; linear dips by 3 dB.
        assert!((level("equal_power") - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert!((level("linear") - 0.5).abs() < 0.01);
        let (ga, gb) = gains(0.0, true);
        assert_eq!((ga, gb), (1.0, 0.0));
    }

    #[test]
    fn channels_follow_the_wider_input() {
        let mut h = harness(64);
        h.set_audio("a", vec![vec![1.0; 64]])
            .set_audio("b", vec![vec![0.0; 64], vec![2.0; 64]])
            .set("mix", 0.5);
        let out = h.run(64);
        let o = out.audio("out");
        assert_eq!(o.len(), 2);
        assert_eq!((o[0][3], o[1][3]), (0.5, 1.5));
    }

    #[test]
    fn audio_rate_mix_is_independent_of_block_size() {
        let mix: Vec<f32> = (0..1_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let a: Vec<f32> = (0..1_000).map(|i| (i as f32 * 0.3).sin()).collect();
        let render = |block: usize| {
            let mut h = harness(block);
            h.set_audio("a", vec![a.clone()])
                .set_samples("b", vec![0.25; 1_000])
                .set_samples("mix", mix.clone())
                .set("curve", "equal_power");
            h.run(1_000).audio("out")[0].clone()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn bad_mix_stays_finite() {
        let mut h = harness(64);
        let wild = vec![f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30];
        h.set_audio("a", vec![vec![1.0; 4]])
            .set_audio("b", vec![vec![1.0; 4]])
            .set_samples("mix", wild);
        let out = h.run(4);
        assert!(out.audio("out")[0].iter().all(|v| v.is_finite()));
        h.reset();
    }
}
