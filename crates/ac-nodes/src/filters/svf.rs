use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::svf::{Svf as SvfState, SvfCoeffs};

node! {
    kind: "svf",
    name: "SVF",
    category: Filters,
    keywords: [
        "state variable", "filter", "lowpass", "highpass", "bandpass", "notch", "lpf", "hpf",
        "bpf", "resonance", "synth filter", "multimode"
    ],
    /// A resonant state-variable filter with lowpass, bandpass, highpass,
    /// and notch outputs at once: the synth filter.
    ///
    /// - `cutoff` is the corner (or center) frequency in hertz, kept between
    ///   1 Hz and 0.49 × the sample rate.
    /// - `resonance` (0 to 1) raises the peak at the cutoff. It maps to a Q
    ///   from 0.5 (no peak) to 100 evenly in decibels, so each step of the
    ///   knob raises the peak by the same amount; about 0.07 is a flat
    ///   (Butterworth) response, 0.5 a Q of 7, and near 1 the filter rings
    ///   like a struck bell but never runs away. Values outside 0..1 are
    ///   clamped.
    ///
    /// At the cutoff the lowpass and highpass are lifted by Q (up to +40 dB
    /// at full resonance), so a tone sitting on the cutoff gets loud.
    /// `bandpass` is normalized to 0 dB at its center (resonance narrows it
    /// without making it louder) and `notch` is the input minus `bandpass`.
    ///
    /// The filter is the trapezoidal (zero-delay feedback) state-variable
    /// design, whose state lives in its integrators, so `cutoff` and
    /// `resonance` can be swept at audio rate, by an envelope or an
    /// oscillator, without zipper noise or blowing up. Coefficients are
    /// worked out once per block for steady settings and every sample when
    /// either input is a sample-rate signal. Any number of channels is
    /// filtered (up to 64 with their own state; more pass through).
    ///
    /// **Try this:** wire an `ADSR` through a `Map` (0..1 → 200..4000 Hz)
    /// into `cutoff` with `resonance` at 0.6 for a plucky synth bass, or
    /// feed an Oscillator at 2 kHz into `cutoff` for growling audio-rate FM
    /// of the filter.
    pub struct Svf {
        states: Vec<SvfState>,
        /// Per-frame coefficients when parameters move within a block.
        scratch: Vec<SvfCoeffs>,
        /// One block of each response for the channel being filtered.
        outs: [Vec<f32>; 4],
        coeffs: SvfCoeffs,
        /// Inputs the current `coeffs` were computed from (cutoff, resonance, rate).
        key: Option<(f32, f32, f64)>,
    }
}

const IN: usize = 0;
const CUTOFF: usize = 1;
const RESONANCE: usize = 2;

/// Output indices, in the order of `SvfOut`'s fields.
const LOWPASS: usize = 0;
const BANDPASS: usize = 1;
const HIGHPASS: usize = 2;
const NOTCH: usize = 3;

/// Channels with their own filter state (two `f64`s each).
const MAX_CHANNELS: usize = 64;

/// Q at full resonance; the bottom is 0.5.
const MAX_Q: f64 = 100.0;

/// Resonance 0..1 to Q 0.5..100, exponential so the peak height (≈ Q) moves
/// evenly in decibels.
fn resonance_to_q(res: f32) -> f64 {
    let r = if res.is_finite() {
        f64::from(res).clamp(0.0, 1.0)
    } else {
        0.0
    };
    0.5 * (MAX_Q / 0.5).powf(r)
}

impl Svf {
    /// Coefficients for these inputs, recomputing only when they changed.
    fn update(&mut self, cutoff: f32, res: f32, rate: f64) -> SvfCoeffs {
        let key = (cutoff, res, rate);
        if self.key != Some(key) {
            self.coeffs = SvfCoeffs::new(f64::from(cutoff), resonance_to_q(res), rate);
            self.key = Some(key);
        }
        self.coeffs
    }
}

impl Node for Svf {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to filter."))
            .input(
                SocketSpec::float("cutoff", Unit::Hz, 1000.0)
                    .range(20.0, 20_000.0)
                    .log()
                    .doc("Corner or center frequency."),
            )
            .input(
                SocketSpec::float("resonance", Unit::Generic, 0.2)
                    .range(0.0, 1.0)
                    .doc("Peak at the cutoff: 0 none, 1 ringing."),
            )
            .output(SocketSpec::audio("lowpass").doc("Frequencies below the cutoff."))
            .output(
                SocketSpec::audio("bandpass").doc("A band around the cutoff, 0 dB at its center."),
            )
            .output(SocketSpec::audio("highpass").doc("Frequencies above the cutoff."))
            .output(SocketSpec::audio("notch").doc("Everything but a band around the cutoff."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let block = ctx.max_block.max(1);
        let identity = SvfCoeffs::new(1000.0, 0.707, ctx.sample_rate);
        Svf {
            states: vec![SvfState::default(); MAX_CHANNELS],
            scratch: vec![identity; block],
            outs: std::array::from_fn(|_| vec![0.0; block]),
            coeffs: identity,
            key: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let frames = io.frames();
        let input = io.inputs.audio(IN);
        let cutoff = io.inputs.float(CUTOFF);
        let res = io.inputs.float(RESONANCE);
        let varying = cutoff.is_varying() || res.is_varying();

        let block = self.update(cutoff.first(), res.first(), rate);
        if varying {
            for i in 0..self.scratch.len().min(frames) {
                let k = self.update(cutoff.at(i), res.at(i), rate);
                if let Some(s) = self.scratch.get_mut(i) {
                    *s = k;
                }
            }
        }

        let channels = io.outputs.audio(LOWPASS).channels();
        for c in 0..channels {
            let src = input.channel(c);
            let n = src.len().min(frames);
            let [lp, bp, hp, notch] = &mut self.outs;
            match self.states.get_mut(c) {
                Some(st) => {
                    for (i, &x) in src.iter().enumerate().take(n) {
                        let k = if varying {
                            self.scratch.get(i).unwrap_or(&block)
                        } else {
                            &block
                        };
                        let o = st.tick(k, x);
                        if let (Some(a), Some(b), Some(h), Some(m)) = (
                            lp.get_mut(i),
                            bp.get_mut(i),
                            hp.get_mut(i),
                            notch.get_mut(i),
                        ) {
                            (*a, *b, *h, *m) = (o.lowpass, o.bandpass, o.highpass, o.notch);
                        }
                    }
                    st.flush();
                }
                None => {
                    // Beyond the stateful channels: pass through.
                    for buf in [lp, bp, hp, notch] {
                        let m = n.min(buf.len());
                        buf[..m].copy_from_slice(&src[..m]);
                    }
                }
            }
            for (k, buf) in self.outs.iter().enumerate() {
                let mut out = io.outputs.audio([LOWPASS, BANDPASS, HIGHPASS, NOTCH][k]);
                let dst = out.channel(c);
                let m = dst.len().min(n).min(buf.len());
                dst[..m].copy_from_slice(&buf[..m]);
            }
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

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    /// Steady-state magnitude of a tone through each output.
    fn through(cutoff: f64, res: f64, freq: f64) -> [f64; 4] {
        let mut h = Harness::new::<Svf>(json!({}));
        h.set_audio("in", vec![sine(freq, 24_000)])
            .set("cutoff", cutoff)
            .set("resonance", res);
        let out = h.run(24_000);
        ["lowpass", "bandpass", "highpass", "notch"]
            .map(|name| tone_magnitude(&out.audio(name)[0][9_600..], freq, RATE))
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn responses_at_known_frequencies() {
        // Resonance 0 is Q 0.5: each pole pair is critically damped.
        let low = through(1000.0, 0.0, 100.0);
        assert!((low[0] - 1.0).abs() < 0.02, "{low:?}");
        // A decade below: highpass -40 dB, the broad bandpass about -14 dB.
        assert!(low[2] < 0.012, "{low:?}");
        assert!((db(low[1]) + 14.1).abs() < 0.3, "{low:?}");
        let high = through(1000.0, 0.0, 10_000.0);
        assert!((high[2] - 1.0).abs() < 0.02, "{high:?}");
        assert!(high[0] < 0.02);
        // At the cutoff with Q 0.5: lowpass and highpass at -6 dB, the
        // normalized bandpass at 0 dB, the notch silent.
        let at = through(1000.0, 0.0, 1000.0);
        assert!((db(at[0]) + 6.02).abs() < 0.1, "{at:?}");
        assert!((db(at[2]) + 6.02).abs() < 0.1, "{at:?}");
        assert!((at[1] - 1.0).abs() < 0.01, "{at:?}");
        assert!(at[3] < 0.01, "{at:?}");
    }

    #[test]
    fn resonance_lifts_the_peak_by_q() {
        let half = resonance_to_q(0.5);
        assert!((half - 0.5 * 200f64.sqrt()).abs() < 1e-9);
        assert_eq!(resonance_to_q(1.0), 100.0);
        assert_eq!(resonance_to_q(f32::NAN), 0.5);
        assert_eq!(resonance_to_q(7.0), 100.0);
        let at = through(1000.0, 0.5, 1000.0);
        assert!((at[0] / half - 1.0).abs() < 0.02, "{at:?}");
        // A narrow band: an octave away the bandpass is far down.
        let off = through(1000.0, 0.5, 2000.0);
        assert!(db(off[1]) < -15.0, "{off:?}");
    }

    #[test]
    fn impulse_response_rings_at_the_cutoff() {
        let mut x = vec![0.0; 24_000];
        x[0] = 1.0;
        let mut h = Harness::new::<Svf>(json!({}));
        h.set_audio("in", vec![x])
            .set("cutoff", 500.0)
            .set("resonance", 0.9);
        let out = h.run(24_000);
        let bp = &out.audio("bandpass")[0];
        let crossings = bp[..12_000]
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        // 0.25 s of ringing at 500 Hz.
        assert!((crossings as i64 - 125).abs() <= 2, "{crossings}");
        assert!(peak(&bp[12_000..]) < peak(&bp[..12_000]));
    }

    #[test]
    fn stable_and_smooth_under_audio_rate_sweeps() {
        let n = 48_000;
        let x = noise(n);
        let sweep: Vec<f32> = (0..n)
            .map(|i| 10_000.0 + 9_990.0 * (i as f32 * 0.05).sin())
            .collect();
        let res: Vec<f32> = (0..n)
            .map(|i| 0.5 + 0.5 * (i as f32 * 0.013).sin())
            .collect();
        let mut h = Harness::new::<Svf>(json!({}));
        h.set_audio("in", vec![x])
            .set_samples("cutoff", sweep)
            .set_samples("resonance", res);
        let out = h.run(n);
        for name in ["lowpass", "bandpass", "highpass", "notch"] {
            let o = &out.audio(name)[0];
            assert!(o.iter().all(|v| v.is_finite()), "{name}");
            assert!(peak(o) < 200.0, "{name}: {}", peak(o));
        }
        // A slow sweep of a steady tone has no steps beyond the tone's own slope.
        let tone = sine(220.0, n);
        let slow: Vec<f32> = (0..n).map(|i| 600.0 + i as f32 * 0.1).collect();
        let mut h = Harness::new::<Svf>(json!({}));
        h.set_audio("in", vec![tone])
            .set("resonance", 0.3)
            .set_samples("cutoff", slow);
        let o = h.run(n).audio("lowpass")[0].clone();
        let step = o
            .windows(2)
            .skip(100)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(step < 0.06, "{step}");
    }

    #[test]
    fn extreme_parameters_and_nan_input_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        let mut x = sine(1000.0, 700);
        x[100] = f32::NAN;
        x[200] = f32::INFINITY;
        let mut h = Harness::with_resources::<Svf>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![x])
            .set_samples("cutoff", wild.clone())
            .set_samples("resonance", wild.iter().rev().copied().collect());
        let out = h.run(700);
        for name in ["lowpass", "bandpass", "highpass", "notch"] {
            assert!(out.audio(name)[0].iter().all(|v| v.is_finite()), "{name}");
        }
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(3000);
        let sweep: Vec<f32> = (0..3000).map(|i| 300.0 + i as f32).collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<Svf>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()]).set("resonance", 0.7);
            if wired {
                h.set_samples("cutoff", sweep.clone());
            }
            let out = h.run(3000);
            (
                out.audio("lowpass")[0].clone(),
                out.audio("notch")[0].clone(),
            )
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn channels_are_independent_and_follow_the_input() {
        let tone = sine(440.0, 2048);
        let mut stereo = Harness::new::<Svf>(json!({}));
        stereo.set_audio("in", vec![tone.clone(), vec![0.0; 2048]]);
        let s = stereo.run(2048);
        let mut mono = Harness::new::<Svf>(json!({}));
        mono.set_audio("in", vec![tone]);
        let m = mono.run(2048);
        assert_eq!(s.audio("lowpass").len(), 2);
        assert_eq!(s.audio("lowpass")[0], m.audio("lowpass")[0]);
        assert_eq!(peak(&s.audio("highpass")[1]), 0.0);
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut h = Harness::new::<Svf>(json!({}));
        let mut x = vec![0.0; 256];
        x[0] = 1.0;
        h.set_audio("in", vec![x]).set("resonance", 1.0);
        h.run(64);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 256]]);
        let out = h.run(64);
        assert_eq!(peak(&out.audio("lowpass")[0]), 0.0);
        assert_eq!(peak(&out.audio("bandpass")[0]), 0.0);
    }
}
