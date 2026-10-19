use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, SpectrumShape, Unit,
    node,
};
use ac_dsp::pvoc::wrap_phase;

use super::{MAX_BINS, finite_or};

node! {
    kind: "spectral_freeze",
    name: "Spectral Freeze",
    category: Spectral,
    keywords: ["freeze", "hold", "sustain", "drone", "infinite", "spectrum", "pad", "stretch"],
    /// Holds one moment of a sound and keeps it ringing: while `hold` is
    /// on, the spectrum stops changing and plays the frame it caught,
    /// for as long as you like.
    ///
    /// When `hold` turns on, the next frame is caught: each bin's level,
    /// and how fast its phase was turning (the frequency of the partial in
    /// it, measured against the frame before). While held, every output
    /// frame repeats those levels with each bin's phase still turning at
    /// its own frequency, so through an `IFFT` a frozen chord sustains as
    /// a smooth drone rather than buzzing at the frame rate. When `hold`
    /// turns off the live spectrum comes back. `glide` (seconds) crossfades
    /// between live and frozen each way; 0 switches on the frame.
    ///
    /// `hold` is read at each frame's own sample. The output has the
    /// input's frame size and hop; a change of frame size, or a reset,
    /// lets go of the frozen frame. Frames that hold NaN or infinite bins
    /// count those bins as silent.
    ///
    /// **Try this:** `FFT` → Spectral Freeze → `IFFT`, with a `Button`'s
    /// `gate` on `hold`: play a chord, press the button, and the chord hangs
    /// in the air while you play over it. With `glide` at 1 s, the freeze
    /// swells in and fades out.
    pub struct SpectralFreeze {
        shape: Option<SpectrumShape>,
        /// Phases of the last input frame (for measuring frequencies).
        last_phase: Vec<f32>,
        have_last: bool,
        /// The caught frame: levels and per-frame phase steps.
        level: Vec<f32>,
        step: Vec<f32>,
        /// The frozen frame's phases as they turn.
        phase: Vec<f32>,
        /// Peaks of the caught frame (room for every bin, so it never grows).
        peaks: Vec<usize>,
        holding: bool,
        caught: bool,
        /// How far towards the frozen frame the output is (0..1).
        blend: f32,
    }
}

const SPECTRUM: usize = 0;
const HOLD: usize = 1;
const GLIDE: usize = 2;
const OUT: usize = 0;

impl SpectralFreeze {
    /// Catch `bins`: levels, phases, and each bin's phase step per hop,
    /// estimated from the phase it turned since the last frame beyond what
    /// its center frequency explains (M. Dolson, "The phase vocoder: a
    /// tutorial", Computer Music Journal 10(4), 1986). Without a frame
    /// before it, a bin turns at its center frequency. Then every bin turns
    /// at the rate of the peak whose region it's in, so each partial's lobe
    /// turns as one and keeps its shape and level (rigid phase locking: J.
    /// Laroche and M. Dolson, "Improved phase vocoder time-scale
    /// modification of audio", IEEE Trans. Speech Audio Proc. 7(3), 1999).
    fn catch(&mut self, shape: SpectrumShape, bins: &[f32]) {
        let n = bins.len() / 2;
        let expected = std::f32::consts::TAU * shape.hop as f32 / shape.size.max(1) as f32;
        for k in 0..n.min(MAX_BINS) {
            let (re, im) = clean(bins[2 * k], bins[2 * k + 1]);
            let p = im.atan2(re);
            let center = expected * k as f32;
            self.level[k] = re.hypot(im);
            self.phase[k] = p;
            self.step[k] = if self.have_last {
                center + wrap_phase(p - self.last_phase[k] - center)
            } else {
                center
            };
        }
        self.lock(n.min(MAX_BINS));
        self.caught = true;
    }

    /// Give every bin of the first `n` the phase step of its peak: peaks
    /// are local maxima of the level, and regions split at the quietest bin
    /// between two peaks.
    fn lock(&mut self, n: usize) {
        self.peaks.clear();
        for k in 1..n.saturating_sub(1) {
            let l = self.level[k];
            if l > self.level[k - 1]
                && l >= self.level[k + 1]
                && self.peaks.len() < self.peaks.capacity()
            {
                self.peaks.push(k);
            }
        }
        let mut start = 0;
        for (i, &p) in self.peaks.iter().enumerate() {
            let end = match self.peaks.get(i + 1) {
                Some(&q) => (p..q)
                    .min_by(|&a, &b| self.level[a].total_cmp(&self.level[b]))
                    .unwrap_or(p),
                None => n.saturating_sub(1),
            };
            let step = self.step[p];
            for s in self.step.iter_mut().take(end + 1).skip(start) {
                *s = step;
            }
            start = end + 1;
        }
    }
}

/// A bin with NaN or infinite parts reads as silent.
fn clean(re: f32, im: f32) -> (f32, f32) {
    if re.is_finite() && im.is_finite() {
        (re, im)
    } else {
        (0.0, 0.0)
    }
}

impl Node for SpectralFreeze {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("spectrum").doc("Frames to freeze."))
            .input(SocketSpec::boolean("hold", false).doc("Freeze while on."))
            .input(
                SocketSpec::float("glide", Unit::Seconds, 0.05)
                    .range(0.0, 2.0)
                    .doc("Crossfade between live and frozen."),
            )
            .output(SocketSpec::spectrum("spectrum").doc("Live, or the frozen frame."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        SpectralFreeze {
            shape: None,
            last_phase: vec![0.0; MAX_BINS],
            have_last: false,
            level: vec![0.0; MAX_BINS],
            step: vec![0.0; MAX_BINS],
            phase: vec![0.0; MAX_BINS],
            peaks: Vec::with_capacity(MAX_BINS),
            holding: false,
            caught: false,
            blend: 0.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(SPECTRUM);
        if input.shape() != self.shape {
            self.shape = input.shape();
            self.reset();
        }
        let (Some(shape), Some(out)) = (self.shape, io.outputs.spectrum(OUT)) else {
            return;
        };
        let hold = io.inputs.boolean(HOLD);
        let glide = io.inputs.float(GLIDE);
        let hop = f64::from(shape.hop);
        for (offset, bins) in input.iter() {
            let i = offset as usize;
            let on = hold.at(i);
            if on && !self.holding {
                self.catch(shape, bins);
            }
            self.holding = on;
            let time = f64::from(finite_or(glide.at(i), 0.05)).max(0.0);
            let move_by = if time > 0.0 {
                (hop / (time * ctx.sample_rate)) as f32
            } else {
                1.0
            };
            let target = if on { 1.0 } else { 0.0 };
            self.blend = if self.blend < target {
                (self.blend + move_by).min(target)
            } else {
                (self.blend - move_by).max(target)
            };
            let Some(dst) = out.push(offset) else {
                continue;
            };
            let g = if self.caught { self.blend } else { 0.0 };
            for (k, (d, s)) in dst
                .chunks_exact_mut(2)
                .zip(bins.chunks_exact(2))
                .take(MAX_BINS)
                .enumerate()
            {
                let (re, im) = clean(s[0], s[1]);
                self.last_phase[k] = im.atan2(re);
                if g <= 0.0 {
                    d[0] = re;
                    d[1] = im;
                    continue;
                }
                let (fs, fc) = self.phase[k].sin_cos();
                let (fr, fi) = (self.level[k] * fc, self.level[k] * fs);
                d[0] = re * (1.0 - g) + fr * g;
                d[1] = im * (1.0 - g) + fi * g;
            }
            self.have_last = true;
            // The frozen frame turns on to the next one, while it's heard.
            if self.caught && self.blend > 0.0 {
                let n = bins.len() / 2;
                for k in 0..n.min(MAX_BINS) {
                    self.phase[k] = wrap_phase(self.phase[k] + self.step[k]);
                }
            } else if !on {
                self.caught = false;
            }
        }
    }

    fn reset(&mut self) {
        self.have_last = false;
        self.holding = false;
        self.caught = false;
        self.blend = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::fft::Fft;
    use crate::spectral::ifft::Ifft;
    use ac_core::NoResources;
    use ac_core::testing::{Frame, Harness, rms, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * f * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    /// `x` through FFT (2048, overlap 4) → Spectral Freeze → IFFT, with
    /// `hold` as given per sample.
    fn chain(x: &[f32], hold: Vec<f32>, glide: f64, block: usize) -> Vec<f32> {
        let n = x.len();
        let shape = SpectrumShape::new(2048, 512);
        let mut fft = Harness::with_resources::<Fft>(json!({}), RATE, block, &NoResources);
        fft.set_audio("in", vec![x.to_vec()]);
        let frames = fft.run(n).spectrum("spectrum").to_vec();
        let mut f = Harness::with_resources::<SpectralFreeze>(json!({}), RATE, block, &NoResources);
        f.spectrum("spectrum", shape, frames)
            .set_samples("hold", hold)
            .set("glide", glide);
        let frozen = f.run(n).spectrum("spectrum").to_vec();
        let mut ifft = Harness::with_resources::<Ifft>(json!({}), RATE, block, &NoResources);
        ifft.spectrum("spectrum", shape, frozen);
        ifft.run(n).audio("out")[0].clone()
    }

    #[test]
    fn a_held_tone_keeps_sounding_after_its_input_stops() {
        // A 1 kHz tone for half a second, then silence; hold from 0.3 s on.
        let n = 72_000;
        let mut x = sine(1000.0, 0.5, 24_000);
        x.resize(n, 0.0);
        let hold: Vec<f32> = (0..n)
            .map(|i| if i >= 14_400 { 1.0 } else { 0.0 })
            .collect();
        let y = chain(&x, hold, 0.0, 256);
        // A second after the input stopped, the tone is still there at
        // its level and its pitch, and nothing else is.
        let tail = &y[48_000..72_000];
        let m = tone_magnitude(tail, 1000.0, RATE);
        assert!((m - 0.5).abs() < 0.05, "{m}");
        let off = tone_magnitude(tail, 1100.0, RATE);
        assert!(off < 0.01, "{off}");
        // Without hold, it stops with the input.
        let y = chain(&x, vec![0.0; n], 0.0, 256);
        assert!(rms(&y[48_000..]) < 1e-4);
    }

    #[test]
    fn a_frozen_glide_holds_its_level() {
        // A saw gliding up 600 Hz a second, caught at 0.4 s: its frozen
        // partials neither drift apart nor fade.
        let n = 96_000;
        let mut phase = 0.0f64;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                phase = (phase + (300.0 + 600.0 * i as f64 / RATE) / RATE).fract();
                (0.5 * (2.0 * phase - 1.0)) as f32
            })
            .collect();
        let hold: Vec<f32> = (0..n)
            .map(|i| if i >= 19_200 { 1.0 } else { 0.0 })
            .collect();
        let y = chain(&x, hold, 0.0, 256);
        let (a, b) = (rms(&y[28_800..48_000]), rms(&y[76_800..96_000]));
        assert!((a / b - 1.0).abs() < 0.05, "{a} then {b}");
        assert!(a > 0.1, "{a}");
    }

    #[test]
    fn hold_off_passes_frames_through_unchanged() {
        let frames: Vec<Frame> = (0..4)
            .map(|k| {
                (
                    k * 16 + 3,
                    (0..18)
                        .map(|b| (b as f32 - 9.0) * 0.1 * (k + 1) as f32)
                        .collect(),
                )
            })
            .collect();
        let mut h = Harness::new::<SpectralFreeze>(json!({}));
        h.spectrum(
            "spectrum",
            SpectrumShape { size: 16, hop: 16 },
            frames.clone(),
        );
        assert_eq!(h.run(64).spectrum("spectrum"), &frames[..]);
    }

    #[test]
    fn frozen_frames_keep_their_levels_and_turn_their_phases() {
        // Bin 1 of a 16-point frame turns by 2π·hop/size per hop at its
        // center frequency; with hop 4, a quarter turn a frame.
        let shape = SpectrumShape { size: 16, hop: 4 };
        let frame = |at: u64, phase: f32| {
            let mut v = vec![0.0; 18];
            v[2] = 2.0 * phase.cos();
            v[3] = 2.0 * phase.sin();
            (at, v)
        };
        let quarter = std::f32::consts::FRAC_PI_2;
        // Live frames turn a quarter turn each; hold from the third.
        let frames: Vec<Frame> = (0..6)
            .map(|k| frame(4 * k + 3, quarter * k as f32))
            .collect();
        // Then the input changes completely; the output doesn't.
        let mut wild = frames.clone();
        for f in wild.iter_mut().skip(3) {
            f.1 = vec![9.0; 18];
        }
        let hold: Vec<f32> = (0..32).map(|i| if i >= 11 { 1.0 } else { 0.0 }).collect();
        let mut h = Harness::new::<SpectralFreeze>(json!({}));
        h.spectrum("spectrum", shape, wild)
            .set_samples("hold", hold)
            .set("glide", 0.0);
        let out = h.run(32).spectrum("spectrum").to_vec();
        // Frame 2 is caught as it is; frames 3..5 keep turning.
        for (k, (at, b)) in out.iter().enumerate().skip(2) {
            assert_eq!(*at, 4 * k as u64 + 3);
            let want = frame(0, quarter * k as f32).1;
            for (x, y) in b.iter().zip(&want) {
                assert!((x - y).abs() < 1e-4, "frame {k}: {b:?}");
            }
        }
    }

    #[test]
    fn glide_crossfades_and_block_size_does_not_matter() {
        let n = 48_000;
        let x = sine(440.0, 0.5, n);
        let hold: Vec<f32> = (0..n)
            .map(|i| {
                if (12_000..30_000).contains(&i) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let a = chain(&x, hold.clone(), 0.2, 64);
        assert_eq!(a, chain(&x, hold.clone(), 0.2, 1));
        assert_eq!(a, chain(&x, hold, 0.2, 1024));
        assert!(a.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn bad_bins_shape_changes_and_resets_let_go() {
        let shape = SpectrumShape { size: 16, hop: 16 };
        let mut bad = vec![1.0; 18];
        bad[4] = f32::NAN;
        bad[7] = f32::INFINITY;
        let mut h = Harness::new::<SpectralFreeze>(json!({}));
        h.spectrum("spectrum", shape, vec![(0, bad.clone()), (16, bad.clone())])
            .set("hold", true)
            .set("glide", 0.0);
        let out = h.run(32).spectrum("spectrum").to_vec();
        for (_, b) in &out {
            assert!(b.iter().all(|v| v.is_finite()));
            assert_eq!((b[4], b[5], b[6], b[7]), (0.0, 0.0, 0.0, 0.0));
        }
        // After a reset with hold still on, the next frame is caught
        // afresh: it comes out as it went in.
        let fresh: Vec<f32> = (0..18).map(|b| b as f32 * 0.25).collect();
        let mut h = Harness::new::<SpectralFreeze>(json!({}));
        h.spectrum(
            "spectrum",
            shape,
            vec![(0, vec![1.0; 18]), (16, fresh.clone())],
        )
        .set("hold", true)
        .set("glide", 0.0);
        h.run(16);
        h.reset();
        let out = h.run(16).spectrum("spectrum").to_vec();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 16);
        for (x, y) in out[0].1.iter().zip(&fresh) {
            assert!((x - y).abs() < 1e-5, "{out:?}");
        }
        // Unwired: nothing out, nothing breaks.
        let mut h = Harness::new::<SpectralFreeze>(json!({}));
        assert!(h.run(64).spectrum("spectrum").is_empty());
    }
}
