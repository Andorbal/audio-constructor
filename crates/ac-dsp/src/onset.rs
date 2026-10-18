//! Onset detection by spectral flux, after J. P. Bello et al., "A tutorial
//! on onset detection in music signals" (IEEE Trans. Speech and Audio
//! Processing 13(5), 2005), with the log-compressed flux and the
//! mean-plus-offset peak picking that S. Dixon found most reliable in
//! "Onset detection revisited" (DAFx 2006), made causal with one frame of
//! lookahead.
//!
//! - [`SpectralFlux`] turns each STFT frame into one number: the mean over
//!   bins of the rise in log magnitude since the previous frame (falls are
//!   ignored), so new energy anywhere in the spectrum counts and decays
//!   don't.
//! - [`PeakPicker`] reports a frame as an onset when its flux is a local
//!   maximum and exceeds the mean of the preceding frames by `delta`.
//! - [`locate`] finds the sample where an onset starts inside a frame, so a
//!   trigger can be placed more precisely than one hop.

/// Log compression constant: magnitudes are `ln(1 + GAMMA × |X|)` with
/// `|X|` scaled so a full-scale sine reads 1. The knee sits near −80 dBFS.
const GAMMA: f32 = 1e4;

/// Frames of history the adaptive threshold averages.
pub const MEAN_FRAMES: usize = 8;

/// Spectral flux of successive frames.
#[derive(Clone, Debug)]
pub struct SpectralFlux {
    prev: Vec<f32>,
}

impl SpectralFlux {
    /// State for frames of up to `bins` bins.
    pub fn new(bins: usize) -> SpectralFlux {
        SpectralFlux {
            prev: vec![0.0; bins],
        }
    }

    /// Flux of `frame` (interleaved `re, im` bins) against the previous
    /// frame. `scale` multiplies magnitudes so that a full-scale sine reads
    /// about 1 (for a Hann window of size N, `4 / N`).
    pub fn process(&mut self, frame: &[f32], scale: f32) -> f32 {
        let bins = (frame.len() / 2).min(self.prev.len());
        if bins == 0 {
            return 0.0;
        }
        let mut sum = 0.0f32;
        for (pair, prev) in frame.chunks_exact(2).zip(self.prev.iter_mut()) {
            let (re, im) = (pair[0], pair[1]);
            let m = (re * re + im * im).sqrt() * scale;
            let l = if m.is_finite() {
                (GAMMA * m).ln_1p()
            } else {
                0.0
            };
            sum += (l - *prev).max(0.0);
            *prev = l;
        }
        sum / bins as f32
    }

    pub fn reset(&mut self) {
        self.prev.fill(0.0);
    }
}

/// Causal peak picking on an onset detection function.
#[derive(Clone, Debug, Default)]
pub struct PeakPicker {
    /// The values before the candidate, oldest overwritten first.
    history: [f32; MEAN_FRAMES],
    at: usize,
    before: f32,
    candidate: f32,
}

impl PeakPicker {
    pub fn new() -> PeakPicker {
        PeakPicker::default()
    }

    /// Add the next value. Returns the previous value if it was an onset: a
    /// local maximum (above the value before it, at least the new one) that
    /// is at least `delta` above the mean of the [`MEAN_FRAMES`] values
    /// before it.
    pub fn push(&mut self, value: f32, delta: f32) -> Option<f32> {
        let value = if value.is_finite() { value } else { 0.0 };
        let c = self.candidate;
        let mean = self.history.iter().sum::<f32>() / MEAN_FRAMES as f32;
        let onset = c > self.before && c >= value && c >= mean + delta;
        self.history[self.at] = c;
        self.at = (self.at + 1) % MEAN_FRAMES;
        self.before = c;
        self.candidate = value;
        onset.then_some(c)
    }

    pub fn reset(&mut self) {
        *self = PeakPicker::default();
    }
}

/// Where an onset starts in `x`: the 16-sample stretch whose energy rises
/// most over the stretches before it, then the first sample there that
/// reaches half that stretch's peak. Returns an index into `x`.
pub fn locate(x: &[f32]) -> usize {
    const SUB: usize = 16;
    const FLOOR: f32 = 1e-9;
    let n = x.len() / SUB;
    if n == 0 {
        return 0;
    }
    let energy = |s: usize| -> f32 {
        x.get(s * SUB..(s + 1) * SUB)
            .map_or(0.0, |c| c.iter().map(|v| v * v).sum())
    };
    let mut best = 0;
    let mut best_rise = f32::MIN;
    // Start from the first stretch's own level, so steady background
    // doesn't count as a rise.
    let mut prev = energy(0);
    for s in 0..n {
        let e = energy(s);
        let rise = (e + FLOOR) / (prev + FLOOR);
        if rise > best_rise && e > FLOOR {
            best_rise = rise;
            best = s;
        }
        // Background: the louder of the last stretch and a slow decay, so
        // the tail of an onset doesn't look like a new rise.
        prev = e.max(prev * 0.5);
    }
    let chunk = x.get(best * SUB..(best + 1) * SUB).unwrap_or(&[]);
    let peak = chunk.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let within = chunk
        .iter()
        .position(|v| v.abs() >= 0.5 * peak)
        .unwrap_or(0);
    best * SUB + within
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flux_counts_rises_not_falls() {
        let mut f = SpectralFlux::new(4);
        assert_eq!(f.process(&[0.0; 8], 1.0), 0.0);
        let loud = [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        let up = f.process(&loud, 1.0);
        assert!((up - (1e4f32).ln_1p()).abs() < 1e-3, "{up}");
        assert_eq!(f.process(&loud, 1.0), 0.0);
        assert_eq!(f.process(&[0.0; 8], 1.0), 0.0);
        assert_eq!(f.process(&[f32::NAN; 8], 1.0), 0.0);
    }

    #[test]
    fn peaks_need_a_local_maximum_above_the_mean() {
        let mut p = PeakPicker::new();
        let seq = [0.0, 0.1, 0.0, 0.0, 2.0, 1.0, 0.5, 0.0, 0.3, 0.31, 0.32, 0.0];
        let hits: Vec<usize> = seq
            .iter()
            .enumerate()
            .filter_map(|(i, &v)| p.push(v, 0.5).map(|_| i - 1))
            .collect();
        assert_eq!(hits, vec![4]);
    }

    #[test]
    fn locate_finds_clicks_and_bursts() {
        let mut x = vec![0.0; 512];
        x[300] = 1.0;
        assert_eq!(locate(&x), 300);
        let mut y = vec![0.001; 512];
        for (i, v) in y.iter_mut().enumerate().skip(200) {
            *v = 0.8 * (-((i - 200) as f32) / 100.0).exp() * if i % 2 == 0 { 1.0 } else { -1.0 };
        }
        assert_eq!(locate(&y), 200);
        assert_eq!(locate(&[]), 0);
    }
}
