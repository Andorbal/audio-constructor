//! Spectral nodes: short-time Fourier analysis and resynthesis, processing
//! on spectra, band splitting, and pitch detection.

use ac_core::{Config, ConfigExt, Registry};
use ac_dsp::fft::Window;
use ac_dsp::stft::{self, MAX_SIZE, MIN_SIZE};

pub mod band_split;
pub mod fft;
pub mod ifft;
pub mod pitch_detect;
pub mod spectral_gate;
pub mod spectral_shift;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<fft::Fft>();
    r.register::<ifft::Ifft>();
    r.register::<band_split::BandSplit>();
    r.register::<spectral_gate::SpectralGate>();
    r.register::<spectral_shift::SpectralShift>();
    r.register::<pitch_detect::PitchDetect>();
}

/// The most bins a `Spectrum` frame can have (`SpectrumShape` allows sizes
/// up to 65536). Nodes that keep per-bin state for frames from any
/// producer size it for this.
pub(crate) const MAX_BINS: usize = 65_536 / 2 + 1;

/// Config `size`: an FFT size, repaired to a power of two in 64..=16384.
pub(crate) fn fft_size(config: &Config, default: usize) -> usize {
    stft::pow2_size(
        config.get_f64_or("size", default as f64),
        MIN_SIZE,
        MAX_SIZE,
        default,
    )
}

/// Config `overlap`: frames per window, repaired to 1, 2, 4, or 8.
pub(crate) fn overlap(config: &Config) -> usize {
    stft::overlap_factor(config.get_f64_or("overlap", 4.0))
}

/// Config `window`: a window name; unknown names are Hann.
pub(crate) fn window(config: &Config) -> Window {
    Window::from_name(config.get_str_or("window", "hann"))
}

/// `v`, or `default` when `v` is NaN or infinite.
#[inline]
pub(crate) fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}
