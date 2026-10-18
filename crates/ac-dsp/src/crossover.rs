//! Multiband Linkwitz-Riley crossovers whose bands sum to an allpass.
//!
//! Each split is a 4th-order Linkwitz-Riley crossover: two cascaded
//! 2nd-order Butterworth sections per side (S. Linkwitz, "Active crossover
//! networks for noncoincident drivers", JAES 24(1), 1976). Its lowpass and
//! highpass outputs sum to the 2nd-order allpass with the same corner and
//! Q = 1/√2: in the analog prototype `(ω⁴ + s⁴) / D² = (s² − √2ωs + ω²) / D`
//! with `D = s² + √2ωs + ω²`, and because the bilinear transform is a
//! substitution, the Audio EQ Cookbook lowpass, highpass, and allpass
//! sections keep that identity exactly.
//!
//! Several splits are arranged as a balanced tree. A band must pass through
//! every split's response exactly once: the lowpass or highpass where its
//! path splits, and that split's allpass everywhere else. So at each split
//! the low branch gets the allpasses of the edges on the high side and the
//! high branch those of the low side (the usual allpass phase compensation
//! of multiway crossovers; see also D. Self, "The Design of Active
//! Crossovers", ch. on summing). The bands then sum to the input through the
//! product of every edge's allpass: flat magnitude, smooth phase.

use crate::biquad::{Coeffs, Mode, Tdf2};

/// The Q of every section: Butterworth.
const Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Audio EQ Cookbook 2nd-order allpass (R. Bristow-Johnson), with the same
/// sanitizing as [`Coeffs::rbj`].
pub fn allpass(freq: f64, q: f64, rate: f64) -> Coeffs {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        48_000.0
    };
    let freq = if freq.is_finite() { freq } else { 1000.0 }.clamp(1.0, 0.49 * rate);
    let q = if q.is_finite() { q } else { Q }.clamp(0.01, 100.0);
    let w0 = std::f64::consts::TAU * freq / rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a0 = 1.0 + alpha;
    Coeffs {
        b0: (1.0 - alpha) / a0,
        b1: -2.0 * cos / a0,
        b2: 1.0,
        a1: -2.0 * cos / a0,
        a2: (1.0 - alpha) / a0,
    }
}

/// One step of the program: filter buffer `src` into buffer `dst` (which
/// may be the same buffer).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Op {
    src: usize,
    dst: usize,
    coeffs: Coeffs,
}

/// A crossover for a fixed set of edges at a fixed rate. The per-channel
/// filter state lives outside (see [`Crossover::state`]), so one tree serves
/// every channel.
#[derive(Clone, Debug)]
pub struct Crossover {
    ops: Vec<Op>,
    bands: usize,
}

impl Crossover {
    /// A crossover splitting at `edges` (Hz, ascending) into
    /// `edges.len() + 1` bands. Edges are clamped below 0.49 × `rate`.
    pub fn new(edges: &[f64], rate: f64) -> Crossover {
        let mut ops = Vec::new();
        let bands = edges.len() + 1;
        build(edges, rate, 0, bands - 1, &mut ops);
        Crossover { ops, bands }
    }

    pub fn bands(&self) -> usize {
        self.bands
    }

    /// Fresh filter state for one channel.
    pub fn state(&self) -> Vec<Tdf2> {
        vec![Tdf2::default(); self.ops.len()]
    }

    /// Split `input` into `bands` (at least [`Crossover::bands`] buffers,
    /// each at least `input.len()` long). `state` is one channel's state.
    pub fn process(&self, state: &mut [Tdf2], input: &[f32], bands: &mut [Vec<f32>]) {
        let n = input.len();
        let Some(first) = bands.first_mut() else {
            return;
        };
        if let Some(dst) = first.get_mut(..n) {
            dst.copy_from_slice(input);
        }
        for (op, st) in self.ops.iter().zip(state.iter_mut()) {
            if op.src == op.dst {
                if let Some(buf) = bands.get_mut(op.dst).and_then(|b| b.get_mut(..n)) {
                    for v in buf {
                        *v = st.tick(&op.coeffs, *v);
                    }
                }
            } else if let Ok([src, dst]) = bands.get_disjoint_mut([op.src, op.dst]) {
                for (d, s) in dst.iter_mut().zip(src.iter()).take(n) {
                    *d = st.tick(&op.coeffs, *s);
                }
            }
        }
    }

    /// Flush tiny state values (see [`Tdf2::flush`]).
    pub fn flush(state: &mut [Tdf2]) {
        for st in state {
            st.flush();
        }
    }

    /// The complex response of band `band` at `freq` Hz, as (magnitude,
    /// phase in radians), for tests and displays.
    pub fn response(&self, band: usize, freq: f64, rate: f64) -> (f64, f64) {
        // Follow the program on a single complex value per buffer.
        let mut bufs = vec![(0.0f64, 0.0f64); self.bands];
        bufs[0] = (1.0, 0.0);
        let w = std::f64::consts::TAU * freq / rate;
        for op in &self.ops {
            let h = eval(&op.coeffs, w);
            let (re, im) = bufs[op.src];
            bufs[op.dst] = (re * h.0 - im * h.1, re * h.1 + im * h.0);
        }
        let (re, im) = bufs.get(band).copied().unwrap_or((0.0, 0.0));
        (re.hypot(im), im.atan2(re))
    }
}

/// Program for bands `a..=b`, whose signal is in buffer `a`. Edge `i` lies
/// between bands `i` and `i + 1`.
fn build(edges: &[f64], rate: f64, a: usize, b: usize, ops: &mut Vec<Op>) {
    if a >= b {
        return;
    }
    let m = (a + b - 1) / 2;
    let f = edges[m];
    let lp = Coeffs::rbj(Mode::Lowpass, f, Q, 0.0, rate);
    let hp = Coeffs::rbj(Mode::Highpass, f, Q, 0.0, rate);
    let hi = m + 1;
    // Highpass first: it reads buffer `a` before the lowpass overwrites it.
    ops.push(Op {
        src: a,
        dst: hi,
        coeffs: hp,
    });
    ops.push(Op {
        src: hi,
        dst: hi,
        coeffs: hp,
    });
    ops.push(Op {
        src: a,
        dst: a,
        coeffs: lp,
    });
    ops.push(Op {
        src: a,
        dst: a,
        coeffs: lp,
    });
    for &e in &edges[hi..b] {
        ops.push(Op {
            src: a,
            dst: a,
            coeffs: allpass(e, Q, rate),
        });
    }
    for &e in &edges[a..m] {
        ops.push(Op {
            src: hi,
            dst: hi,
            coeffs: allpass(e, Q, rate),
        });
    }
    build(edges, rate, a, m, ops);
    build(edges, rate, hi, b, ops);
}

/// `H(e^{jw})` of a biquad.
fn eval(k: &Coeffs, w: f64) -> (f64, f64) {
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let (nr, ni) = (k.b0 + k.b1 * c1 + k.b2 * c2, -(k.b1 * s1 + k.b2 * s2));
    let (dr, di) = (1.0 + k.a1 * c1 + k.a2 * c2, -(k.a1 * s1 + k.a2 * s2));
    let den = (dr * dr + di * di).max(1e-300);
    ((nr * dr + ni * di) / den, (ni * dr - nr * di) / den)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    fn sum_response(x: &Crossover, freq: f64) -> (f64, f64) {
        let (mut re, mut im) = (0.0, 0.0);
        for b in 0..x.bands() {
            let (m, p) = x.response(b, freq, RATE);
            re += m * p.cos();
            im += m * p.sin();
        }
        (re, im)
    }

    #[test]
    fn allpass_is_flat() {
        let ap = allpass(1000.0, Q, RATE);
        for f in [20.0, 500.0, 1000.0, 5000.0, 20_000.0] {
            assert!((ap.magnitude(f, RATE) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn bands_sum_to_flat_magnitude_for_many_edge_sets() {
        let sets: [&[f64]; 5] = [
            &[],
            &[1000.0],
            &[200.0, 2000.0],
            &[100.0, 400.0, 1600.0, 6400.0],
            &[
                30.0, 60.0, 100.0, 200.0, 300.0, 500.0, 800.0, 1200.0, 2000.0, 3000.0, 5000.0,
                7000.0, 10_000.0, 14_000.0, 18_000.0,
            ],
        ];
        for edges in sets {
            let x = Crossover::new(edges, RATE);
            assert_eq!(x.bands(), edges.len() + 1);
            for f in [
                25.0, 80.0, 200.0, 650.0, 1000.0, 2000.0, 4321.0, 9000.0, 19_000.0,
            ] {
                let (re, im) = sum_response(&x, f);
                let mag = re.hypot(im);
                assert!(db(mag).abs() < 1e-6, "{edges:?} at {f}: {} dB", db(mag));
            }
        }
    }

    #[test]
    fn each_band_passes_its_own_range() {
        let x = Crossover::new(&[200.0, 2000.0], RATE);
        // Band centers in the passband, -6 dB at the edges.
        assert!(db(x.response(0, 50.0, RATE).0).abs() < 0.1);
        assert!(db(x.response(1, 632.0, RATE).0).abs() < 0.5);
        assert!(db(x.response(2, 8000.0, RATE).0).abs() < 0.1);
        assert!((db(x.response(0, 200.0, RATE).0) + 6.02).abs() < 0.1);
        assert!(db(x.response(0, 2000.0, RATE).0) < -70.0);
        assert!(db(x.response(2, 200.0, RATE).0) < -70.0);
    }

    #[test]
    fn processing_matches_the_response_and_sums_flat() {
        let x = Crossover::new(&[150.0, 900.0, 4000.0], RATE);
        let mut state = x.state();
        let n = 48_000;
        for f in [60.0, 500.0, 2000.0, 11_000.0] {
            for s in state.iter_mut() {
                s.reset();
            }
            let input: Vec<f32> = (0..n)
                .map(|i| (std::f64::consts::TAU * f * i as f64 / RATE).sin() as f32)
                .collect();
            let mut bands = vec![vec![0.0; n]; x.bands()];
            x.process(&mut state, &input, &mut bands);
            let sum: Vec<f32> = (0..n).map(|i| bands.iter().map(|b| b[i]).sum()).collect();
            // RMS × √2 is the amplitude, however the samples fall.
            let tail = &sum[n / 2..];
            let ms = tail.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / tail.len() as f64;
            let amp = (2.0 * ms).sqrt();
            assert!((amp - 1.0).abs() < 1e-3, "{f}: {amp}");
        }
    }
}
