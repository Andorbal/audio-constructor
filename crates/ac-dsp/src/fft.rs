//! Real FFTs for spectral nodes, planned once (off the audio thread) and run
//! without allocation, plus analysis windows.
//!
//! Spectra are `size / 2 + 1` complex bins stored as interleaved `re, im`
//! pairs, the layout `Spectrum` wires carry (`ac_core::SpectrumBuf`). The
//! forward transform is unnormalized and the inverse divides by `size`, so
//! `inverse(forward(x)) == x`.

use std::fmt;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Analysis windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Rectangular,
    Hann,
    Hamming,
    Blackman,
}

impl Window {
    pub const NAMES: &'static [&'static str] = &["hann", "hamming", "blackman", "rectangular"];

    pub fn from_name(name: &str) -> Window {
        match name {
            "hamming" => Window::Hamming,
            "blackman" => Window::Blackman,
            "rectangular" | "rect" | "none" => Window::Rectangular,
            _ => Window::Hann,
        }
    }

    /// The periodic window of length `n` (the right form for overlap-add).
    pub fn table(self, n: usize) -> Vec<f32> {
        let tau = std::f64::consts::TAU;
        (0..n)
            .map(|i| {
                let x = i as f64 / n.max(1) as f64;
                (match self {
                    Window::Rectangular => 1.0,
                    Window::Hann => 0.5 - 0.5 * (tau * x).cos(),
                    Window::Hamming => 0.54 - 0.46 * (tau * x).cos(),
                    Window::Blackman => 0.42 - 0.5 * (tau * x).cos() + 0.08 * (2.0 * tau * x).cos(),
                }) as f32
            })
            .collect()
    }
}

/// Gain that makes overlap-adding `window²` (analysis and synthesis
/// windows both applied) at `hop` sum to 1: `1 / Σ w²[k·hop]`, averaged
/// over one hop.
pub fn ola_gain(window: &[f32], hop: usize) -> f32 {
    let n = window.len();
    let hop = hop.clamp(1, n.max(1));
    let mut sum = 0.0f64;
    for offset in 0..hop {
        let mut s = 0.0f64;
        let mut k = offset;
        while k < n {
            s += f64::from(window[k]) * f64::from(window[k]);
            k += hop;
        }
        sum += s;
    }
    let avg = sum / hop as f64;
    if avg > 1e-12 { (1.0 / avg) as f32 } else { 1.0 }
}

/// A planned forward and inverse real FFT of one size, with scratch space.
#[derive(Clone)]
pub struct Fft {
    size: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    time: Vec<f32>,
    bins: Vec<Complex<f32>>,
    scratch_f: Vec<Complex<f32>>,
    scratch_i: Vec<Complex<f32>>,
}

impl fmt::Debug for Fft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fft").field("size", &self.size).finish()
    }
}

impl Fft {
    /// Plan transforms of `size` samples (an even number, at least 2).
    pub fn new(size: usize) -> Fft {
        let size = size.max(2) & !1;
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(size);
        let inverse = planner.plan_fft_inverse(size);
        Fft {
            size,
            time: forward.make_input_vec(),
            bins: forward.make_output_vec(),
            scratch_f: forward.make_scratch_vec(),
            scratch_i: inverse.make_scratch_vec(),
            forward,
            inverse,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn bins(&self) -> usize {
        self.size / 2 + 1
    }

    /// Forward transform of `input` (zero-padded or truncated to the size)
    /// into interleaved `re, im` pairs in `out` (at least `2 * bins()` long).
    pub fn forward(&mut self, input: &[f32], out: &mut [f32]) {
        let n = input.len().min(self.size);
        self.time[..n].copy_from_slice(&input[..n]);
        self.time[n..].fill(0.0);
        if self
            .forward
            .process_with_scratch(&mut self.time, &mut self.bins, &mut self.scratch_f)
            .is_err()
        {
            out.fill(0.0);
            return;
        }
        for (k, c) in self.bins.iter().enumerate() {
            if let Some(pair) = out.get_mut(2 * k..2 * k + 2) {
                pair[0] = c.re;
                pair[1] = c.im;
            }
        }
    }

    /// Inverse transform of interleaved bins into `out` (the first `size`
    /// samples), scaled so that it undoes [`Fft::forward`].
    pub fn inverse(&mut self, bins: &[f32], out: &mut [f32]) {
        for (k, c) in self.bins.iter_mut().enumerate() {
            let re = bins.get(2 * k).copied().unwrap_or(0.0);
            let im = bins.get(2 * k + 1).copied().unwrap_or(0.0);
            *c = Complex::new(re, im);
        }
        // A real signal's DC and Nyquist bins have no imaginary part.
        if let Some(c) = self.bins.first_mut() {
            c.im = 0.0;
        }
        if let Some(c) = self.bins.last_mut() {
            c.im = 0.0;
        }
        if self
            .inverse
            .process_with_scratch(&mut self.bins, &mut self.time, &mut self.scratch_i)
            .is_err()
        {
            out.fill(0.0);
            return;
        }
        let scale = 1.0 / self.size as f32;
        for (o, t) in out.iter_mut().zip(&self.time) {
            *o = t * scale;
        }
    }
}

/// Magnitude of bin `k` of an interleaved spectrum.
#[inline]
pub fn magnitude(bins: &[f32], k: usize) -> f32 {
    let re = bins.get(2 * k).copied().unwrap_or(0.0);
    let im = bins.get(2 * k + 1).copied().unwrap_or(0.0);
    (re * re + im * im).sqrt()
}

/// Center frequency of bin `k` for an FFT of `size` at `rate`.
#[inline]
pub fn bin_hz(k: usize, size: usize, rate: f64) -> f64 {
    k as f64 * rate / size.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_exact_enough() {
        let mut f = Fft::new(64);
        let x: Vec<f32> = (0..64)
            .map(|i| ((i * 7) % 13) as f32 / 13.0 - 0.5)
            .collect();
        let mut bins = vec![0.0; 2 * f.bins()];
        f.forward(&x, &mut bins);
        let mut y = vec![0.0; 64];
        f.inverse(&bins, &mut y);
        for (a, b) in x.iter().zip(&y) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn a_sine_lands_in_its_bin() {
        let n = 256;
        let mut f = Fft::new(n);
        let x: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 8.0 * i as f32 / n as f32).sin())
            .collect();
        let mut bins = vec![0.0; 2 * f.bins()];
        f.forward(&x, &mut bins);
        let peak = (0..f.bins())
            .max_by(|a, b| magnitude(&bins, *a).total_cmp(&magnitude(&bins, *b)))
            .unwrap();
        assert_eq!(peak, 8);
        assert!((magnitude(&bins, 8) - n as f32 / 2.0).abs() < 1e-2);
        assert_eq!(bin_hz(8, 256, 48_000.0), 1500.0);
    }

    #[test]
    fn hann_squared_at_quarter_hop_overlap_adds_flat() {
        let w = Window::Hann.table(512);
        let g = ola_gain(&w, 128);
        // Σ hann² at 75 % overlap is 1.5.
        assert!((g - 1.0 / 1.5).abs() < 1e-4, "{g}");
        assert_eq!(Window::from_name("bogus"), Window::Hann);
        assert_eq!(Window::Rectangular.table(4), vec![1.0; 4]);
    }

    #[test]
    fn odd_sizes_are_made_even() {
        assert_eq!(Fft::new(1).size(), 2);
        assert_eq!(Fft::new(101).size(), 100);
    }
}
