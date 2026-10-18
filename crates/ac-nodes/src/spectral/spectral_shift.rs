use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, SpectrumShape, Unit,
    node,
};
use ac_dsp::pvoc::BinShifter;

use super::{MAX_BINS, finite_or};

node! {
    kind: "spectral_shift",
    name: "Spectral Shift",
    category: Spectral,
    keywords: ["frequency shift", "pitch shift", "transpose", "inharmonic", "phase vocoder"],
    /// Moves every partial of a spectrum to a new frequency:
    /// `frequency × scale + shift`.
    ///
    /// - `scale` multiplies frequencies, keeping harmonics in tune with each
    ///   other: 2 is an octave up, 0.5 an octave down, 1.5 a fifth up.
    ///   Values are kept between 1/16 and 16.
    /// - `shift` then adds a fixed number of hertz to every partial, which
    ///   pulls harmonics out of tune with each other: small shifts sound
    ///   metallic and bell-like, larger ones robotic.
    ///
    /// It works as a phase vocoder: each bin's exact frequency is worked out
    /// from how far its phase moved since the previous frame, its energy
    /// moves to the bin nearest the new frequency, and the output phase
    /// advances at the new frequency, so moved partials stay smooth across
    /// overlapping frames. Use an `FFT` overlap of 4 or more; overlap 1 or 2
    /// can't follow phases and sounds rough. Bins that land on the same
    /// output bin add up; partials moved below 0 Hz or past half the sample
    /// rate are dropped. With `scale` 1 and `shift` 0 frames pass through
    /// exactly.
    ///
    /// Inputs are read at each frame's own sample. The output has the
    /// input's frame size and hop. Phase tracking starts over after a reset
    /// or a change of frame size. NaN or infinite bins count as zero.
    ///
    /// **Try this:** between `FFT` and `IFFT`, set `shift` to 5 Hz and mix
    /// the result with the dry signal for a slow, swirling beat; or set
    /// `scale` to 2 on a melody for an octave-up double.
    pub struct SpectralShift {
        shifter: BinShifter,
        shape: Option<SpectrumShape>,
    }
}

const SPECTRUM: usize = 0;
const SHIFT: usize = 1;
const SCALE: usize = 2;
const OUT: usize = 0;

impl Node for SpectralShift {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("spectrum").doc("Frames to shift."))
            .input(
                SocketSpec::float("shift", Unit::Hz, 0.0)
                    .range(-2000.0, 2000.0)
                    .doc("Hertz added to every partial."),
            )
            .input(
                SocketSpec::float("scale", Unit::Ratio, 1.0)
                    .range(0.25, 4.0)
                    .log()
                    .doc("Every frequency is multiplied by this."),
            )
            .output(SocketSpec::spectrum("spectrum").doc("The shifted frames."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        SpectralShift {
            shifter: BinShifter::new(MAX_BINS),
            shape: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(SPECTRUM);
        if input.shape() != self.shape {
            self.shape = input.shape();
            self.shifter.reset();
        }
        let (Some(shape), Some(out)) = (self.shape, io.outputs.spectrum(OUT)) else {
            return;
        };
        let shift = io.inputs.float(SHIFT);
        let scale = io.inputs.float(SCALE);
        let (size, hop) = (shape.size as usize, shape.hop as usize);
        let nyquist = (ctx.sample_rate / 2.0) as f32;
        let hz_to_bins = size as f32 / ctx.sample_rate as f32;
        for (offset, bins) in input.iter() {
            let i = offset as usize;
            let k = finite_or(scale.at(i), 1.0).clamp(1.0 / 16.0, 16.0);
            let s = finite_or(shift.at(i), 0.0).clamp(-nyquist, nyquist) * hz_to_bins;
            if let Some(dst) = out.push(offset) {
                self.shifter.process(bins, dst, size, hop, k, s);
            }
        }
    }

    fn reset(&mut self) {
        self.shifter.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::fft::Fft;
    use crate::spectral::ifft::Ifft;
    use ac_core::NoResources;
    use ac_core::testing::{Frame, Harness, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;
    const SHAPE: SpectrumShape = SpectrumShape {
        size: 2048,
        hop: 512,
    };

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.5 * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    fn analyze(x: &[f32]) -> Vec<Frame> {
        let mut h = Harness::new::<Fft>(json!({}));
        h.set_audio("in", vec![x.to_vec()]);
        h.run(x.len()).spectrum("spectrum").to_vec()
    }

    fn shift(frames: Vec<Frame>, n: usize, set: &[(&str, f64)], block: usize) -> Vec<Frame> {
        let mut h = Harness::with_resources::<SpectralShift>(json!({}), RATE, block, &NoResources);
        h.spectrum("spectrum", SHAPE, frames);
        for (k, v) in set {
            h.set(k, *v);
        }
        h.run(n).spectrum("spectrum").to_vec()
    }

    fn resynth(frames: Vec<Frame>, n: usize) -> Vec<f32> {
        let mut h = Harness::new::<Ifft>(json!({}));
        h.spectrum("spectrum", SHAPE, frames);
        h.run(n).audio("out")[0].clone()
    }

    #[test]
    fn identity_passes_frames_exactly() {
        let x = sine(440.0, 8192);
        let f = analyze(&x);
        assert_eq!(shift(f.clone(), 8192, &[], 64), f);
    }

    #[test]
    fn scale_and_shift_move_tones() {
        let n = 48_000;
        let f = analyze(&sine(500.0, n));
        let up = resynth(shift(f.clone(), n, &[("scale", 2.0)], 64), n);
        assert!(tone_magnitude(&up[12_000..], 1000.0, RATE) > 0.4);
        assert!(tone_magnitude(&up[12_000..], 500.0, RATE) < 0.02);
        let moved = resynth(shift(f, n, &[("shift", 330.0)], 64), n);
        assert!(tone_magnitude(&moved[12_000..], 830.0, RATE) > 0.4);
        assert!(tone_magnitude(&moved[12_000..], 500.0, RATE) < 0.02);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x: Vec<f32> = (0..12_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.3 + (i as f32 * 0.21).sin() * 0.2)
            .collect();
        let f = analyze(&x);
        let set = [("scale", 1.26), ("shift", -40.0)];
        let one = shift(f.clone(), 12_000, &set, 1);
        assert_eq!(one, shift(f.clone(), 12_000, &set, 64));
        assert_eq!(one, shift(f, 12_000, &set, 512));
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let mut f = analyze(&sine(1000.0, 8192));
        f[3].1[100] = f32::NAN;
        for set in [
            [("scale", f64::NAN), ("shift", f64::INFINITY)],
            [("scale", 1e9), ("shift", -1e9)],
            [("scale", -3.0), ("shift", 0.0)],
        ] {
            let out = shift(f.clone(), 8192, &set, 64);
            assert!(out.iter().all(|f| f.1.iter().all(|v| v.is_finite())));
        }
    }

    #[test]
    fn reset_and_unwired_are_harmless() {
        let mut h = Harness::new::<SpectralShift>(json!({}));
        assert!(h.run(256).spectra.is_empty());
        h.reset();
        h.run(64);
    }
}
