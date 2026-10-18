use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::onepole::{OnePole, prewarp};

node! {
    kind: "dc_block",
    name: "DC Block",
    category: Filters,
    keywords: ["dc offset", "highpass", "hpf", "rumble", "subsonic", "offset", "center"],
    /// Removes DC offset: a gentle one-pole highpass that pulls a signal
    /// back to center without touching anything you can hear.
    ///
    /// `cutoff` (hertz, default 10, kept between 1 Hz and 0.49 × the sample
    /// rate) is the -3 dB corner; the slope below it is 6 dB/oct and DC is
    /// removed completely, so a constant input decays to exactly 0 (with a
    /// time constant of 1 / (2π·`cutoff`) seconds: 16 ms at 10 Hz). Raise it
    /// towards 30..80 Hz to clear rumble as well.
    ///
    /// Use it after anything that can push a signal off center: asymmetric
    /// waveshaping, a `Math` add, `Unary` abs, rectified envelopes, or a long
    /// feedback loop that slowly creeps. The filter is the trapezoidal
    /// one-pole, prewarped so the corner is exact at any sample rate, and
    /// safe to modulate. Any number of channels is filtered (up to 64 with
    /// their own state; more pass through).
    ///
    /// **Try this:** put a DC Block inside a `Feedback` loop that runs
    /// through a `Shaper` or a drive stage, so the loop can't build up an
    /// offset and choke.
    pub struct DcBlock {
        states: Vec<OnePole>,
        /// Per-frame gains when `cutoff` moves within a block.
        gains: Vec<f64>,
        /// The last steady gain and the cutoff and rate it was computed from.
        gain: f64,
        key: Option<(f32, f64)>,
    }
}

const IN: usize = 0;
const CUTOFF: usize = 1;
const OUT: usize = 0;

/// Channels with their own filter state (one `f64` each).
const MAX_CHANNELS: usize = 64;

/// Default corner, also the fallback for a non-finite cutoff.
const DEFAULT_HZ: f64 = 10.0;

fn gain_for(cutoff: f32, rate: f64) -> f64 {
    OnePole::gain(prewarp(f64::from(cutoff), DEFAULT_HZ, rate))
}

impl Node for DcBlock {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to center."))
            .input(
                SocketSpec::float("cutoff", Unit::Hz, DEFAULT_HZ)
                    .range(1.0, 200.0)
                    .log()
                    .doc("Corner frequency; below it is removed."),
            )
            .output(SocketSpec::audio("out").doc("The signal without DC."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        DcBlock {
            states: vec![OnePole::default(); MAX_CHANNELS],
            gains: vec![0.0; ctx.max_block.max(1)],
            gain: gain_for(DEFAULT_HZ as f32, ctx.sample_rate),
            key: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let frames = io.frames();
        let input = io.inputs.audio(IN);
        let cutoff = io.inputs.float(CUTOFF);
        let varying = cutoff.is_varying();
        let key = (cutoff.first(), rate);
        if self.key != Some(key) {
            self.gain = gain_for(key.0, rate);
            self.key = Some(key);
        }
        if varying {
            for (i, g) in self.gains.iter_mut().take(frames).enumerate() {
                *g = gain_for(cutoff.at(i), rate);
            }
        }
        let steady = self.gain;
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            let Some(st) = self.states.get_mut(c) else {
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = *s;
                }
                continue;
            };
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let g = if varying {
                    self.gains.get(i).copied().unwrap_or(steady)
                } else {
                    steady
                };
                let x = if x.is_finite() { f64::from(x) } else { 0.0 };
                let y = st.highpass(g, x);
                if st.is_sane(1e12) {
                    *o = y as f32;
                } else {
                    st.reset();
                    *o = 0.0;
                }
            }
            st.flush();
        }
    }

    fn reset(&mut self) {
        for st in &mut self.states {
            st.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32)
            .collect()
    }

    #[test]
    fn a_constant_decays_to_zero() {
        let mut h = Harness::new::<DcBlock>(json!({}));
        h.set_audio("in", vec![vec![0.5; 48_000]]);
        let o = h.run(48_000).audio("out")[0].clone();
        assert!((o[0] - 0.5).abs() < 1e-3);
        // One time constant (16 ms) in: about e⁻¹ left.
        let tau = (RATE / (std::f64::consts::TAU * 10.0)) as usize;
        assert!((o[tau] / 0.5 - (-1.0f32).exp()).abs() < 0.01, "{}", o[tau]);
        assert!(o[47_999].abs() < 1e-9, "{}", o[47_999]);
    }

    #[test]
    fn passes_audio_and_removes_an_offset() {
        let x: Vec<f32> = sine(1000.0, 48_000).iter().map(|v| v * 0.5 + 0.3).collect();
        let mut h = Harness::new::<DcBlock>(json!({}));
        h.set_audio("in", vec![x]);
        let o = h.run(48_000).audio("out")[0].clone();
        let tail = &o[24_000..];
        let mean: f32 = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 1e-4, "{mean}");
        assert!((tone_magnitude(tail, 1000.0, RATE) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn cutoff_is_minus_three_db() {
        let mut h = Harness::new::<DcBlock>(json!({}));
        h.set_audio("in", vec![sine(100.0, 96_000)])
            .set("cutoff", 100.0);
        let o = h.run(96_000).audio("out")[0].clone();
        let m = tone_magnitude(&o[48_000..], 100.0, RATE);
        assert!((m - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.005, "{m}");
    }

    #[test]
    fn block_size_and_modulation() {
        let x: Vec<f32> = sine(50.0, 3_000).iter().map(|v| v + 0.2).collect();
        let sweep: Vec<f32> = (0..3_000).map(|i| 1.0 + (i % 200) as f32).collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<DcBlock>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()]);
            if wired {
                h.set_samples("cutoff", sweep.clone());
            }
            h.run(3_000).audio("out")[0].clone()
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert!(peak(&one) < 2.0);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn extreme_values_stay_finite_and_reset_clears() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        let mut h = Harness::with_resources::<DcBlock>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![wild.clone(), vec![1.0; 700]])
            .set_samples("cutoff", wild.iter().rev().copied().collect());
        let out = h.run(700);
        for c in out.audio("out") {
            assert!(c.iter().all(|v| v.is_finite()));
        }
        h.reset();
        h.set_audio("in", vec![vec![0.0; 1400], vec![0.0; 1400]]);
        assert_eq!(peak(&h.run(100).audio("out")[1]), 0.0);
    }
}
