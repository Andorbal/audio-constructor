use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::ladder::{Ladder as LadderState, LadderCoeffs, LadderMode};

node! {
    kind: "ladder",
    name: "Ladder",
    category: Filters,
    keywords: [
        "transistor ladder", "filter", "lowpass", "lpf", "24db", "4 pole", "resonance",
        "self-oscillating", "drive", "saturation", "synth filter", "warm"
    ],
    /// A four-pole transistor-ladder filter with drive: warm, squelchy, and
    /// able to self-oscillate.
    ///
    /// - `mode` picks the response, mixed from the ladder's four stages:
    ///   `lp24` (24 dB/oct lowpass, the classic), `lp12` (12 dB/oct
    ///   lowpass), `bp` (bandpass, 12 dB/oct each side, 0 dB at the cutoff
    ///   before resonance), or `hp` (24 dB/oct highpass).
    /// - `cutoff` in hertz, kept between 1 Hz and 0.49 × the sample rate.
    /// - `resonance` (0 to 1, clamped) feeds the output back to the input.
    ///   The peak grows towards the top of the range and, from about 0.95,
    ///   the filter self-oscillates: a sine at `cutoff` that keeps going with
    ///   no input, so it can be played as an oscillator.
    /// - `drive` (dB) boosts the input into the ladder's `tanh` input stage.
    ///   At 0 dB a full-scale signal already rounds off a little; more drive
    ///   adds harmonics and compresses, less (negative values) keeps the
    ///   filter clean and quieter. The saturation keeps the output bounded
    ///   (around ±1) however hard it is driven.
    ///
    /// Like the circuit it models, resonance thins out the bass of the
    /// lowpass modes (feedback divides their passband by up to 5). Half of
    /// that loss is made up by raising the input level with resonance, so
    /// full resonance costs about 4.5 dB of passband instead of 14 (and
    /// drives the input stage a little harder); the bandpass and highpass
    /// need no make-up.
    ///
    /// The ladder is solved with zero-delay feedback, so cutoff stays in
    /// tune and resonance stays stable when either is swept at audio rate.
    /// Coefficients are worked out once per block for steady settings and
    /// every sample when `cutoff` or `resonance` is a sample-rate signal.
    /// Any number of channels is filtered (up to 64 with their own state;
    /// more pass through).
    ///
    /// **Try this:** set `resonance` to 1 with no input and play `cutoff`
    /// from a `MIDI Note`'s `freq` for a pure sine lead, or push `drive` to
    /// +12 dB on a sawtooth bass and sweep `cutoff` with an `ADSR`.
    pub struct Ladder {
        states: Vec<LadderState>,
        /// Per-frame coefficients when parameters move within a block.
        scratch: Vec<LadderCoeffs>,
        /// Per-frame drive gains when `drive` moves within a block.
        drives: Vec<f32>,
        coeffs: LadderCoeffs,
        /// Inputs the current `coeffs` were computed from (cutoff, resonance, rate).
        key: Option<(f32, f32, f64)>,
    }
}

const IN: usize = 0;
const MODE: usize = 1;
const CUTOFF: usize = 2;
const RESONANCE: usize = 3;
const DRIVE: usize = 4;
const OUT: usize = 0;

/// Channels with their own filter state (four `f64`s each).
const MAX_CHANNELS: usize = 64;

/// Feedback at full resonance. The loop self-oscillates from 4, so this
/// starts the oscillation just below the top of the knob (at 4 / 4.2 ≈
/// 0.95) and gives it a usable level.
const MAX_FEEDBACK: f64 = 4.2;

/// How much of the lowpass passband loss to resonance is made up: the input
/// gain is `1 + COMPENSATION · k` against a loss of `1 + k`.
const COMPENSATION: f64 = 0.5;

/// Drive is kept within ±48 dB.
const MAX_DRIVE_DB: f32 = 48.0;

fn feedback(res: f32) -> f64 {
    if res.is_finite() {
        f64::from(res).clamp(0.0, 1.0) * MAX_FEEDBACK
    } else {
        0.0
    }
}

fn drive_gain(db: f32) -> f32 {
    let db = if db.is_finite() {
        db.clamp(-MAX_DRIVE_DB, MAX_DRIVE_DB)
    } else {
        0.0
    };
    10f32.powf(db / 20.0)
}

impl Ladder {
    /// Coefficients for these inputs, recomputing only when they changed.
    fn update(&mut self, cutoff: f32, res: f32, rate: f64) -> LadderCoeffs {
        let key = (cutoff, res, rate);
        if self.key != Some(key) {
            self.coeffs = LadderCoeffs::new(f64::from(cutoff), feedback(res), rate);
            self.key = Some(key);
        }
        self.coeffs
    }
}

impl Node for Ladder {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to filter."))
            .input(SocketSpec::choice("mode", LadderMode::NAMES, "lp24").doc("Filter response."))
            .input(
                SocketSpec::float("cutoff", Unit::Hz, 1000.0)
                    .range(20.0, 20_000.0)
                    .log()
                    .doc("Corner frequency."),
            )
            .input(
                SocketSpec::float("resonance", Unit::Generic, 0.3)
                    .range(0.0, 1.0)
                    .doc("Feedback; self-oscillates near 1."),
            )
            .input(
                SocketSpec::float("drive", Unit::Db, 0.0)
                    .range(-12.0, 24.0)
                    .doc("Input gain into the saturating ladder."),
            )
            .output(SocketSpec::audio("out").doc("The filtered signal."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let block = ctx.max_block.max(1);
        let neutral = LadderCoeffs::new(1000.0, 0.0, ctx.sample_rate);
        Ladder {
            states: vec![LadderState::default(); MAX_CHANNELS],
            scratch: vec![neutral; block],
            drives: vec![1.0; block],
            coeffs: neutral,
            key: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let frames = io.frames();
        let input = io.inputs.audio(IN);
        let mode = LadderMode::from_index(io.inputs.choice(MODE, LadderMode::NAMES.len()));
        let cutoff = io.inputs.float(CUTOFF);
        let res = io.inputs.float(RESONANCE);
        let drive = io.inputs.float(DRIVE);
        let varying = cutoff.is_varying() || res.is_varying();
        let drive_varies = drive.is_varying();
        let comp = if mode.loses_passband() {
            COMPENSATION
        } else {
            0.0
        };

        let block = self.update(cutoff.first(), res.first(), rate);
        if varying {
            for i in 0..self.scratch.len().min(frames) {
                let k = self.update(cutoff.at(i), res.at(i), rate);
                if let Some(s) = self.scratch.get_mut(i) {
                    *s = k;
                }
            }
        }
        let drive0 = drive_gain(drive.first());
        if drive_varies {
            for (i, d) in self.drives.iter_mut().take(frames).enumerate() {
                *d = drive_gain(drive.at(i));
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
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let k = if varying {
                    self.scratch.get(i).unwrap_or(&block)
                } else {
                    &block
                };
                let d = if drive_varies {
                    self.drives.get(i).copied().unwrap_or(drive0)
                } else {
                    drive0
                };
                // The make-up goes in ahead of the saturator, so it restores
                // the small-signal passband without lifting the ceiling.
                let makeup = 1.0 + comp * k.feedback();
                let taps = st.tick(k, f64::from(x * d) * makeup);
                *o = mode.mix(&taps) as f32;
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
    use ac_core::testing::{Harness, peak, tone_magnitude, zero_crossing_freq};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 0x9e37_79b9u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    /// Small-signal gain of a tone through a mode (small, so the
    /// saturator stays linear).
    fn gain(mode: &str, cutoff: f64, res: f64, freq: f64) -> f64 {
        let amp = 1e-3;
        let mut h = Harness::new::<Ladder>(json!({}));
        h.set_audio("in", vec![sine(freq, amp, 24_000)])
            .set("mode", mode)
            .set("cutoff", cutoff)
            .set("resonance", res);
        let out = h.run(24_000);
        tone_magnitude(&out.audio("out")[0][12_000..], freq, RATE) / amp
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn modes_at_known_frequencies() {
        assert!((gain("lp24", 1000.0, 0.0, 50.0) - 1.0).abs() < 0.01);
        assert!((db(gain("lp24", 1000.0, 0.0, 1000.0)) + 12.04).abs() < 0.1);
        assert!(db(gain("lp24", 1000.0, 0.0, 4000.0)) < -46.0);
        assert!((db(gain("lp12", 1000.0, 0.0, 1000.0)) + 6.02).abs() < 0.1);
        assert!(db(gain("lp12", 1000.0, 0.0, 4000.0)) < -23.0);
        assert!(db(gain("bp", 1000.0, 0.0, 1000.0)).abs() < 0.1);
        // A decade below, 4·Ω² is -28 dB.
        assert!((db(gain("bp", 1000.0, 0.0, 100.0)) + 28.1).abs() < 0.2);
        assert!((gain("hp", 300.0, 0.0, 12_000.0) - 1.0).abs() < 0.02);
        assert!(db(gain("hp", 2000.0, 0.0, 500.0)) < -46.0);
    }

    #[test]
    fn resonance_peaks_and_half_the_bass_loss_is_made_up() {
        // k = 2.1: the loop divides DC by 3.1, the make-up multiplies by 2.05.
        let dc = gain("lp24", 1000.0, 0.5, 30.0);
        assert!((dc - 2.05 / 3.1).abs() < 0.01, "{dc}");
        // The highpass passband is untouched by resonance.
        assert!((gain("hp", 300.0, 0.5, 12_000.0) - 1.0).abs() < 0.03);
        // At the cutoff the loop divides by 1 - k/4.
        let at = gain("lp24", 1000.0, 0.5, 1000.0);
        let want = 0.25 / (1.0 - 2.1 / 4.0) * 2.05;
        assert!((at - want).abs() < 0.02 * want, "{at} vs {want}");
    }

    #[test]
    fn self_oscillates_in_tune_at_full_resonance() {
        for cutoff in [110.0, 440.0, 2000.0] {
            let mut x = vec![0.0; 48_000];
            x[0] = 0.5;
            let mut h = Harness::new::<Ladder>(json!({}));
            h.set_audio("in", vec![x])
                .set("cutoff", cutoff)
                .set("resonance", 1.0);
            let out = h.run(48_000);
            let tail = &out.audio("out")[0][24_000..];
            let p = peak(tail);
            assert!(p > 0.1 && p < 1.5, "{cutoff}: {p}");
            let f = zero_crossing_freq(tail, RATE);
            assert!((f / cutoff - 1.0).abs() < 0.01, "{cutoff}: {f}");
        }
        // Just below the threshold it dies away.
        let mut x = vec![0.0; 48_000];
        x[0] = 0.5;
        let mut h = Harness::new::<Ladder>(json!({}));
        h.set_audio("in", vec![x])
            .set("cutoff", 440.0)
            .set("resonance", 0.9);
        assert!(peak(&h.run(48_000).audio("out")[0][40_000..]) < 1e-3);
    }

    #[test]
    fn drive_saturates_and_stays_bounded() {
        let x = sine(110.0, 1.0, 24_000);
        let render = |drive: f64| {
            let mut h = Harness::new::<Ladder>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set("cutoff", 15_000.0)
                .set("resonance", 0.0)
                .set("drive", drive);
            h.run(24_000).audio("out")[0].clone()
        };
        let clean = render(-24.0);
        let hot = render(24.0);
        assert!(peak(&hot) < 1.2, "{}", peak(&hot));
        // The resonance make-up doesn't lift the ceiling either.
        for mode in LadderMode::NAMES {
            let mut h = Harness::new::<Ladder>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set("mode", *mode)
                .set("cutoff", 800.0)
                .set("resonance", 1.0)
                .set("drive", 24.0);
            let p = peak(&h.run(24_000).audio("out")[0]);
            assert!(p < 1.2, "{mode}: {p}");
        }
        // Clean is a scaled copy of the input; hot has a big third harmonic.
        let h3 = |o: &[f32]| {
            tone_magnitude(&o[4_800..], 330.0, RATE) / tone_magnitude(&o[4_800..], 110.0, RATE)
        };
        assert!(h3(&clean) < 1e-3, "{}", h3(&clean));
        assert!(h3(&hot) > 0.1, "{}", h3(&hot));
    }

    #[test]
    fn stable_under_audio_rate_sweeps() {
        let n = 48_000;
        let sweep: Vec<f32> = (0..n)
            .map(|i| 10_000.0 + 9_990.0 * (i as f32 * 0.07).sin())
            .collect();
        let res: Vec<f32> = (0..n)
            .map(|i| 0.6 + 0.6 * (i as f32 * 0.011).sin())
            .collect();
        for mode in LadderMode::NAMES {
            let mut h = Harness::new::<Ladder>(json!({}));
            h.set_audio("in", vec![noise(n)])
                .set("mode", *mode)
                .set("drive", 12.0)
                .set_samples("cutoff", sweep.clone())
                .set_samples("resonance", res.clone());
            let out = h.run(n);
            let o = &out.audio("out")[0];
            assert!(o.iter().all(|v| v.is_finite()), "{mode}");
            assert!(peak(o) < 50.0, "{mode}: {}", peak(o));
        }
    }

    #[test]
    fn extreme_parameters_and_nan_input_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(700).copied().collect();
        let mut x = sine(1000.0, 1.0, 700);
        x[100] = f32::NAN;
        x[300] = f32::NEG_INFINITY;
        for mode in LadderMode::NAMES {
            let mut h = Harness::with_resources::<Ladder>(json!({}), 44_100.0, 1, &NoResources);
            h.set_audio("in", vec![x.clone()])
                .set("mode", *mode)
                .set_samples("cutoff", wild.clone())
                .set_samples("resonance", wild.iter().rev().copied().collect())
                .set_samples("drive", wild.clone());
            let out = h.run(700);
            assert!(out.audio("out")[0].iter().all(|v| v.is_finite()), "{mode}");
        }
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = noise(3000);
        let sweep: Vec<f32> = (0..3000).map(|i| 300.0 + i as f32).collect();
        let drive: Vec<f32> = (0..3000).map(|i| (i % 37) as f32 * 0.5).collect();
        let render = |block: usize, wired: bool| {
            let mut h = Harness::with_resources::<Ladder>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()]).set("resonance", 0.8);
            if wired {
                h.set_samples("cutoff", sweep.clone())
                    .set_samples("drive", drive.clone());
            }
            h.run(3000).audio("out")[0].clone()
        };
        for wired in [false, true] {
            let one = render(1, wired);
            assert_eq!(one, render(64, wired));
            assert_eq!(one, render(512, wired));
        }
    }

    #[test]
    fn channels_are_independent() {
        let tone = sine(440.0, 0.5, 2048);
        let mut stereo = Harness::new::<Ladder>(json!({}));
        stereo.set_audio("in", vec![tone.clone(), vec![0.0; 2048]]);
        let s = stereo.run(2048);
        let mut mono = Harness::new::<Ladder>(json!({}));
        mono.set_audio("in", vec![tone]);
        assert_eq!(s.audio("out")[0], mono.run(2048).audio("out")[0]);
        assert_eq!(peak(&s.audio("out")[1]), 0.0);
    }

    #[test]
    fn reset_silences_self_oscillation() {
        let mut x = vec![0.0; 4096];
        x[0] = 1.0;
        let mut h = Harness::new::<Ladder>(json!({}));
        h.set_audio("in", vec![x]).set("resonance", 1.0);
        h.run(2048);
        h.reset();
        assert_eq!(peak(&h.run(2048).audio("out")[0]), 0.0);
    }
}
