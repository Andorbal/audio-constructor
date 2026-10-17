use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::biquad::{Coeffs, Mode, Tdf2};

node! {
    kind: "biquad",
    name: "Biquad",
    category: Filters,
    keywords: [
        "lpf", "hpf", "bpf", "filter", "eq", "lowpass", "highpass", "bandpass", "notch",
        "peak", "shelf", "tone"
    ],
    /// A two-pole filter: lowpass, highpass, bandpass, notch, peak, low
    /// shelf, or high shelf. Works on any number of channels, each with its
    /// own state.
    ///
    /// - `cutoff` is the corner (or center) frequency in hertz.
    /// - `q` sets the resonance (lowpass/highpass), the width
    ///   (bandpass/notch/peak), or the shelf slope; 0.707 is flat.
    /// - `gain` is the boost or cut in dB for peak and shelf modes; other
    ///   modes ignore it.
    ///
    /// Coefficients come from the Audio EQ Cookbook. They are recomputed once
    /// per block for steady settings and every sample when `cutoff`, `q`, or
    /// `gain` is wired to a sample-rate signal, so a filter swept by an
    /// envelope stays smooth. The cutoff is kept below 0.49 × the sample rate
    /// and `q` above zero, so the filter stays stable whatever is wired in.
    /// Up to 64 channels are filtered; any beyond that pass through.
    ///
    /// **Try this:** sweep `cutoff` with an `ADSR` through a `Map`
    /// (0..1 → 200..5000 Hz) for a classic synth filter envelope, or chain
    /// three Biquads (lowshelf, peak, highshelf) for a tone-control EQ.
    pub struct Biquad {
        states: Vec<Tdf2>,
        /// Per-frame coefficients when parameters move within a block.
        scratch: Vec<Coeffs>,
        coeffs: Coeffs,
        /// Inputs the current `coeffs` were computed from (mode, cutoff, q, gain, rate).
        key: Option<(usize, f32, f32, f32, f64)>,
    }
}

const IN: usize = 0;
const MODE: usize = 1;
const CUTOFF: usize = 2;
const Q: usize = 3;
const GAIN: usize = 4;
const OUT: usize = 0;

/// Channels with their own filter state.
const MAX_CHANNELS: usize = 64;

impl Biquad {
    /// Coefficients for these inputs, recomputing only when they changed.
    fn update(&mut self, mode: usize, cutoff: f32, q: f32, gain: f32, rate: f64) -> Coeffs {
        let key = (mode, cutoff, q, gain, rate);
        if self.key != Some(key) {
            self.coeffs = Coeffs::rbj(
                Mode::from_index(mode),
                f64::from(cutoff),
                f64::from(q),
                f64::from(gain),
                rate,
            );
            self.key = Some(key);
        }
        self.coeffs
    }
}

impl Node for Biquad {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to filter."))
            .input(SocketSpec::choice("mode", Mode::NAMES, "lowpass").doc("Filter response."))
            .input(
                SocketSpec::float("cutoff", Unit::Hz, 1000.0)
                    .range(20.0, 20_000.0)
                    .log()
                    .doc("Corner or center frequency."),
            )
            .input(
                SocketSpec::float("q", Unit::Generic, 0.707)
                    .range(0.1, 20.0)
                    .log()
                    .doc("Resonance or bandwidth; 0.707 is flat."),
            )
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-24.0, 24.0)
                    .doc("Boost or cut for peak and shelf modes."),
            )
            .output(SocketSpec::audio("out").doc("The filtered signal."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Biquad {
            states: vec![Tdf2::default(); MAX_CHANNELS],
            scratch: vec![Coeffs::IDENTITY; ctx.max_block.max(1)],
            coeffs: Coeffs::IDENTITY,
            key: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let input = io.inputs.audio(IN);
        let mode = io.inputs.choice(MODE, Mode::NAMES.len());
        let cutoff = io.inputs.float(CUTOFF);
        let q = io.inputs.float(Q);
        let gain = io.inputs.float(GAIN);
        let varying = cutoff.is_varying() || q.is_varying() || gain.is_varying();

        let block = self.update(mode, cutoff.first(), q.first(), gain.first(), rate);
        if varying {
            for i in 0..self.scratch.len().min(io.frames()) {
                let k = self.update(mode, cutoff.at(i), q.at(i), gain.at(i), rate);
                if let Some(s) = self.scratch.get_mut(i) {
                    *s = k;
                }
            }
        }

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
            if varying {
                for (i, (o, s)) in dst.iter_mut().zip(src).enumerate() {
                    *o = st.tick(self.scratch.get(i).unwrap_or(&self.coeffs), *s);
                }
            } else {
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = st.tick(&block, *s);
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

    /// Steady-state magnitude of a tone through the filter.
    fn through(mode: &str, cutoff: f64, q: f64, gain: f64, freq: f64) -> f64 {
        let mut h = Harness::new::<Biquad>(json!({}));
        h.set_audio("in", vec![sine(freq, 24_000)])
            .set("mode", mode)
            .set("cutoff", cutoff)
            .set("q", q)
            .set("gain", gain);
        let out = h.run(24_000);
        tone_magnitude(&out.audio("out")[0][4_800..], freq, RATE)
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn lowpass_passes_lows_and_cuts_highs() {
        assert!((through("lowpass", 1000.0, 0.707, 0.0, 100.0) - 1.0).abs() < 0.01);
        let above = db(through("lowpass", 1000.0, 0.707, 0.0, 2000.0));
        assert!((-14.0..-11.0).contains(&above), "{above}");
    }

    #[test]
    fn highpass_passes_highs_and_cuts_lows() {
        assert!((through("highpass", 1000.0, 0.707, 0.0, 10_000.0) - 1.0).abs() < 0.01);
        let below = db(through("highpass", 1000.0, 0.707, 0.0, 500.0));
        assert!((-14.0..-11.0).contains(&below), "{below}");
    }

    #[test]
    fn notch_kills_its_center() {
        assert!(through("notch", 1500.0, 1.0, 0.0, 1500.0) < 0.01);
        assert!(through("notch", 1500.0, 1.0, 0.0, 150.0) > 0.98);
    }

    #[test]
    fn bandpass_peaks_at_center() {
        assert!((through("bandpass", 1500.0, 2.0, 0.0, 1500.0) - 1.0).abs() < 0.01);
        assert!(through("bandpass", 1500.0, 2.0, 0.0, 150.0) < 0.1);
    }

    #[test]
    fn peak_boosts_its_center_by_the_gain() {
        let g = db(through("peak", 2000.0, 1.0, 12.0, 2000.0));
        assert!((g - 12.0).abs() < 0.1, "{g}");
        let off = db(through("peak", 2000.0, 1.0, 12.0, 50.0));
        assert!(off.abs() < 0.2, "{off}");
    }

    #[test]
    fn shelves_lift_their_side() {
        let low = db(through("lowshelf", 300.0, 0.707, 6.0, 40.0));
        assert!((low - 6.0).abs() < 0.2, "{low}");
        let high = db(through("highshelf", 3000.0, 0.707, -6.0, 15_000.0));
        assert!((high + 6.0).abs() < 0.3, "{high}");
        assert!(db(through("highshelf", 3000.0, 0.707, -6.0, 100.0)).abs() < 0.1);
    }

    #[test]
    fn stereo_channels_have_independent_state() {
        let tone = sine(440.0, 2048);
        let mut stereo = Harness::new::<Biquad>(json!({}));
        stereo.set_audio("in", vec![tone.clone(), vec![0.0; 2048]]);
        let s = stereo.run(2048);
        let mut mono = Harness::new::<Biquad>(json!({}));
        mono.set_audio("in", vec![tone]);
        let m = mono.run(2048);
        assert_eq!(s.audio("out")[0], m.audio("out")[0]);
        assert_eq!(peak(&s.audio("out")[1]), 0.0);
    }

    #[test]
    fn sample_rate_constant_matches_block_rate() {
        let tone = sine(3000.0, 1024);
        let mut a = Harness::new::<Biquad>(json!({}));
        a.set_audio("in", vec![tone.clone()]).set("cutoff", 800.0);
        let mut b = Harness::new::<Biquad>(json!({}));
        b.set_audio("in", vec![tone])
            .set_samples("cutoff", vec![800.0; 1024]);
        assert_eq!(a.run(1024).audio("out"), b.run(1024).audio("out"));
    }

    #[test]
    fn stable_under_a_fast_sample_rate_sweep() {
        let n = 48_000;
        let mut s = 0x1234_5678u32;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let sweep: Vec<f32> = (0..n)
            .map(|i| 7_500.0 + 7_450.0 * (i as f32 * 0.003).sin())
            .collect();
        for mode in Mode::NAMES {
            let mut h = Harness::new::<Biquad>(json!({}));
            h.set_audio("in", vec![noise.clone()])
                .set("mode", *mode)
                .set("q", 10.0)
                .set("gain", 12.0)
                .set_samples("cutoff", sweep.clone());
            let out = h.run(n);
            let o = &out.audio("out")[0];
            assert!(o.iter().all(|v| v.is_finite()), "{mode}");
            assert!(peak(o) < 100.0, "{mode}: {}", peak(o));
        }
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        let tone = sine(1000.0, 700);
        for mode in Mode::NAMES {
            let mut h = Harness::with_resources::<Biquad>(json!({}), 44_100.0, 1, &NoResources);
            h.set_audio("in", vec![tone.clone()])
                .set("mode", *mode)
                .set_samples("cutoff", wild.clone())
                .set_samples("q", wild.iter().rev().copied().collect())
                .set_samples("gain", wild.clone());
            let out = h.run(700);
            assert!(out.audio("out")[0].iter().all(|v| v.is_finite()), "{mode}");
        }
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let tone = sine(700.0, 3000);
        let sweep: Vec<f32> = (0..3000).map(|i| 300.0 + i as f32).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Biquad>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![tone.clone()])
                .set_samples("cutoff", sweep.clone());
            h.run(3000).audio("out")[0].clone()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1024));
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut h = Harness::new::<Biquad>(json!({}));
        let mut x = vec![0.0; 256];
        x[0] = 1.0;
        h.set_audio("in", vec![x]).set("q", 15.0);
        h.run(64);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 256]]);
        assert_eq!(peak(&h.run(64).audio("out")[0]), 0.0);
    }
}
