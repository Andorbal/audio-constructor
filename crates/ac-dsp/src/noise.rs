//! Colored noise from uniform white noise: pink (−3 dB per octave) and
//! brown (−6 dB per octave), each scaled to the RMS level of the white
//! noise it is made from, so switching color keeps roughly the same level.
//!
//! - Pink is the Voss-McCartney algorithm: a sum of random values held for
//!   1, 2, 4, 8, ... samples, plus fresh white noise every sample. R. F.
//!   Voss and J. Clarke, "1/f noise in music: Music from 1/f noise", JASA
//!   63(1), 1978 (popularized by M. Gardner, Scientific American, April
//!   1978), with J. McCartney's staggered update (one row per sample,
//!   chosen by the trailing zeros of a counter), described on the music-dsp
//!   list in 1999. The row count follows the sample rate so the slowest row
//!   sits below 10 Hz at any rate, which keeps the spectrum the same in
//!   hertz.
//! - Brown is leaky-integrated white noise: a one-pole lowpass with its
//!   corner at 10 Hz (so it has no DC), scaled by `sqrt(1 - a²)` so its
//!   variance equals the input's. With the corner fixed in hertz the power
//!   spectrum above it, `2σ²f_c / (π f²)`, doesn't depend on the rate.

use crate::rng::Rng;

/// Most rows the pink generator uses (enough for 10 Hz at any sane rate).
pub const MAX_ROWS: usize = 24;
/// The slowest pink row and the brown leak both sit at or below this.
const LOWEST_HZ: f64 = 10.0;

/// Voss-McCartney pink noise.
#[derive(Clone, Debug, PartialEq)]
pub struct Pink {
    rows: [f64; MAX_ROWS],
    n_rows: usize,
    sum: f64,
    counter: u32,
    norm: f64,
}

impl Pink {
    pub fn new(rate: f64) -> Pink {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            48_000.0
        };
        let n_rows = ((rate / LOWEST_HZ).log2().ceil() as usize).clamp(4, MAX_ROWS);
        Pink {
            rows: [0.0; MAX_ROWS],
            n_rows,
            sum: 0.0,
            counter: 0,
            // n_rows held values plus one white value, each with the same
            // variance: dividing by the square root keeps white's RMS.
            norm: 1.0 / ((n_rows + 1) as f64).sqrt(),
        }
    }

    /// Rows in use (for tests).
    pub fn rows(&self) -> usize {
        self.n_rows
    }

    /// All rows to zero (the output starts quiet and fills in).
    pub fn reset(&mut self) {
        self.rows = [0.0; MAX_ROWS];
        self.sum = 0.0;
        self.counter = 0;
    }

    /// Fill every row with a fresh value so the output starts at full
    /// level instead of fading in over the slow rows' periods.
    pub fn randomize(&mut self, rng: &mut Rng) {
        self.sum = 0.0;
        for r in self.rows.iter_mut().take(self.n_rows) {
            *r = rng.bipolar();
            self.sum += *r;
        }
        self.counter = 0;
    }

    /// The next sample. `white` is added this sample; `row` replaces the
    /// row that is due (if any). Both should be uniform in `[-1, 1)`.
    #[inline]
    pub fn next(&mut self, white: f64, row: f64) -> f64 {
        self.counter = self.counter.wrapping_add(1);
        let k = self.counter.trailing_zeros() as usize;
        if k < self.n_rows {
            self.sum += row - self.rows[k];
            self.rows[k] = row;
        }
        (self.sum + white) * self.norm
    }
}

/// Leaky-integrated (brown) noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brown {
    y: f64,
    a: f64,
    g: f64,
}

impl Brown {
    pub fn new(rate: f64) -> Brown {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            48_000.0
        };
        let a = (-std::f64::consts::TAU * LOWEST_HZ / rate).exp();
        Brown {
            y: 0.0,
            a,
            g: (1.0 - a * a).sqrt(),
        }
    }

    pub fn reset(&mut self) {
        self.y = 0.0;
    }

    /// Start from a random value with the steady-state variance, so the
    /// output doesn't fade in over the leak's time constant.
    pub fn randomize(&mut self, rng: &mut Rng) {
        self.y = rng.bipolar();
    }

    /// The next sample from one white value in `[-1, 1)`.
    #[inline]
    pub fn next(&mut self, white: f64) -> f64 {
        self.y = self.a * self.y + self.g * white;
        self.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f64]) -> f64 {
        (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// Power in a band, averaged over 4096-point FFT frames.
    fn band_power(x: &[f64], lo_hz: f64, hi_hz: f64, rate: f64) -> f64 {
        let n = 4_096;
        let mut fft = crate::fft::Fft::new(n);
        let mut bins = vec![0.0; 2 * fft.bins()];
        let (k0, k1) = (
            (lo_hz * n as f64 / rate) as usize,
            (hi_hz * n as f64 / rate) as usize,
        );
        let mut p = 0.0;
        for frame in x.chunks_exact(n) {
            let f: Vec<f32> = frame.iter().map(|v| *v as f32).collect();
            fft.forward(&f, &mut bins);
            p += (k0..k1)
                .map(|k| f64::from(crate::fft::magnitude(&bins, k)).powi(2))
                .sum::<f64>();
        }
        p
    }

    fn render(rate: f64, n: usize, pink: bool) -> Vec<f64> {
        let mut rng = Rng::new(7);
        let mut p = Pink::new(rate);
        let mut b = Brown::new(rate);
        p.randomize(&mut rng);
        b.randomize(&mut rng);
        (0..n)
            .map(|_| {
                let (w, r) = (rng.bipolar(), rng.bipolar());
                if pink { p.next(w, r) } else { b.next(w) }
            })
            .collect()
    }

    #[test]
    fn levels_match_white() {
        let white = 1.0 / 3f64.sqrt();
        for pink in [true, false] {
            let x = render(48_000.0, 480_000, pink);
            let r = rms(&x);
            assert!((r - white).abs() < 0.06 * white, "pink {pink}: {r}");
        }
    }

    #[test]
    fn rows_follow_the_rate() {
        assert_eq!(Pink::new(48_000.0).rows(), 13);
        assert_eq!(Pink::new(96_000.0).rows(), 14);
        assert_eq!(Pink::new(f64::NAN).rows(), 13);
        assert_eq!(Pink::new(1e30).rows(), MAX_ROWS);
    }

    #[test]
    fn slopes_are_three_and_six_db_per_octave() {
        // Compare octaves 500-1000 Hz and 4-8 kHz: three octaves apart, so
        // pink should drop about 9 dB and brown about 18 dB.
        let rate = 48_000.0;
        for (pink, expect) in [(true, 9.0), (false, 18.0)] {
            let x = render(rate, 4_096 * 32, pink);
            let lo = band_power(&x, 500.0, 1_000.0, rate) / 500.0;
            let hi = band_power(&x, 4_000.0, 8_000.0, rate) / 4_000.0;
            let drop = 10.0 * (lo / hi).log10();
            assert!((drop - expect).abs() < 2.5, "pink {pink}: {drop} dB");
        }
    }

    #[test]
    fn same_seed_same_noise_and_reset_is_quiet() {
        assert_eq!(render(44_100.0, 1_000, true), render(44_100.0, 1_000, true));
        let mut p = Pink::new(48_000.0);
        let mut b = Brown::new(48_000.0);
        p.reset();
        b.reset();
        assert_eq!(p.next(0.0, 0.0), 0.0);
        assert_eq!(b.next(0.0), 0.0);
    }
}
