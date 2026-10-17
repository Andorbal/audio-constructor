//! Oscillator waveforms: a drift-free `f64` phase accumulator plus naive and
//! band-limited sine, triangle, saw, square, and pulse shapes.
//!
//! Band-limiting uses the two-point polynomial band-limited step (PolyBLEP)
//! for jumps and its integral, the band-limited ramp (PolyBLAMP), for slope
//! changes (the triangle's corners):
//!
//! - V. Välimäki and A. Huovilainen, "Antialiasing Oscillators in Subtractive
//!   Synthesis", IEEE Signal Processing Magazine 24(2), 2007.
//! - F. Esqueda, V. Välimäki, S. Bilbao, "Rounding Corners with BLAMP",
//!   Proc. DAFx-16, 2016.
//!
//! The corrections depend only on the distance (in samples) to each
//! discontinuity, so they are the same whether the phase runs forwards or
//! backwards; through-zero FM needs no special case.

use std::f64::consts::TAU;

/// Waveform shapes, in the order of the Oscillator's `shape` choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Triangle,
    Saw,
    Square,
    Pulse,
}

impl Shape {
    /// Choice names, in index order.
    pub const NAMES: &'static [&'static str] = &["sine", "triangle", "saw", "square", "pulse"];

    /// The shape for a choice index (out-of-range indices clamp to the last).
    pub fn from_index(i: usize) -> Shape {
        match i {
            0 => Shape::Sine,
            1 => Shape::Triangle,
            2 => Shape::Saw,
            3 => Shape::Square,
            _ => Shape::Pulse,
        }
    }
}

/// Wrap a phase into `[0, 1)`.
#[inline]
pub fn wrap01(p: f64) -> f64 {
    let w = p - p.floor();
    // `p - floor(p)` can round up to exactly 1.0 for tiny negative `p`.
    if w >= 1.0 { 0.0 } else { w }
}

/// PolyBLEP residual for a unit upward step at phase 0.
///
/// `t` is the phase distance past the step in `[0, 1)`, `a` the phase
/// increment per sample (`|freq| / rate`, at most 0.5). Add `jump * blep(...)`
/// to the naive waveform.
#[inline]
pub fn poly_blep(t: f64, a: f64) -> f64 {
    if t < a {
        // Just after the step: pull the naive (high) value down toward the midpoint.
        let d = t / a;
        -0.5 * (1.0 - d) * (1.0 - d)
    } else if t > 1.0 - a {
        // Just before: push the naive (low) value up.
        let d = (1.0 - t) / a;
        0.5 * (1.0 - d) * (1.0 - d)
    } else {
        0.0
    }
}

/// PolyBLAMP residual for a unit change of slope (per sample) at phase 0: the
/// integral of [`poly_blep`]. Add `slope_change_per_sample * blamp(...)`.
#[inline]
pub fn poly_blamp(t: f64, a: f64) -> f64 {
    let d = if t < a {
        t / a
    } else if t > 1.0 - a {
        (1.0 - t) / a
    } else {
        return 0.0;
    };
    let r = 1.0 - d;
    r * r * r / 6.0
}

/// Value of `shape` at phase `p` in `[0, 1)`.
///
/// `a` is the absolute phase increment per sample, used to size the
/// band-limiting corrections; `width` is the pulse width in `(0, 1)`. With
/// `bandlimited` false the naive shape is returned (for LFOs).
#[inline]
pub fn wave(shape: Shape, p: f64, a: f64, width: f64, bandlimited: bool) -> f64 {
    let a = a.clamp(0.0, 0.5);
    match shape {
        Shape::Sine => (TAU * p).sin(),
        Shape::Saw => {
            let v = 2.0 * p - 1.0;
            if bandlimited {
                v - 2.0 * poly_blep(p, a)
            } else {
                v
            }
        }
        Shape::Square => pulse(p, 0.5, a, bandlimited),
        Shape::Pulse => pulse(p, width, a, bandlimited),
        Shape::Triangle => {
            // Starts at 0 and rises, like the sine.
            let v = if p < 0.25 {
                4.0 * p
            } else if p < 0.75 {
                2.0 - 4.0 * p
            } else {
                4.0 * p - 4.0
            };
            if bandlimited {
                // Slope goes +4 → -4 per cycle at 0.25 and back at 0.75.
                let k = 8.0 * a;
                v - k * poly_blamp(wrap01(p - 0.25), a) + k * poly_blamp(wrap01(p - 0.75), a)
            } else {
                v
            }
        }
    }
}

fn pulse(p: f64, width: f64, a: f64, bandlimited: bool) -> f64 {
    let v = if p < width { 1.0 } else { -1.0 };
    if bandlimited {
        v + 2.0 * poly_blep(p, a) - 2.0 * poly_blep(wrap01(p - width), a)
    } else {
        v
    }
}

/// A phase accumulator in cycles. `f64` so hours of running don't drift.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Phasor {
    phase: f64,
}

impl Phasor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current phase in `[0, 1)`.
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Back to phase 0 (start, reset, hard sync).
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Advance by `inc` cycles (may be negative).
    #[inline]
    pub fn advance(&mut self, inc: f64) {
        self.phase = wrap01(self.phase + inc);
    }
}

/// Phase increment per sample for `freq` Hz at `rate`, clamped to Nyquist.
/// Non-finite input gives 0 (the oscillator holds still).
#[inline]
pub fn increment(freq: f64, rate: f64) -> f64 {
    let inc = freq / rate;
    if inc.is_finite() {
        inc.clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_stays_in_unit_interval() {
        for p in [-3.25, -1e-18, 0.0, 0.5, 1.0, 7.75, 1e9 + 0.5] {
            let w = wrap01(p);
            assert!((0.0..1.0).contains(&w), "{p} -> {w}");
        }
        assert_eq!(wrap01(-0.25), 0.75);
    }

    #[test]
    fn blep_is_odd_and_blamp_is_even_about_the_step() {
        let a = 0.1;
        for d in [0.0, 0.3, 0.7, 0.99] {
            let after = poly_blep(d * a, a);
            let before = poly_blep(1.0 - d * a, a);
            assert!((after + before).abs() < 1e-12, "d {d}");
            let ra = poly_blamp(d * a, a);
            let rb = poly_blamp(1.0 - d * a, a);
            assert!((ra - rb).abs() < 1e-12);
        }
        assert_eq!(poly_blep(0.5, 0.1), 0.0);
        assert_eq!(poly_blamp(0.5, 0.1), 0.0);
        // Zero increment: no correction and no division by zero.
        assert_eq!(poly_blep(0.0, 0.0), 0.0);
        assert_eq!(poly_blamp(0.0, 0.0), 0.0);
    }

    #[test]
    fn naive_shapes_hit_their_landmarks() {
        let w = |s, p| wave(s, p, 0.0, 0.25, false);
        assert_eq!(w(Shape::Saw, 0.0), -1.0);
        assert_eq!(w(Shape::Saw, 0.5), 0.0);
        assert_eq!(w(Shape::Square, 0.25), 1.0);
        assert_eq!(w(Shape::Square, 0.75), -1.0);
        assert_eq!(w(Shape::Pulse, 0.2), 1.0);
        assert_eq!(w(Shape::Pulse, 0.3), -1.0);
        assert_eq!(w(Shape::Triangle, 0.0), 0.0);
        assert_eq!(w(Shape::Triangle, 0.25), 1.0);
        assert_eq!(w(Shape::Triangle, 0.75), -1.0);
        assert!((w(Shape::Sine, 0.25) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bandlimited_shapes_stay_bounded_at_any_increment() {
        for shape in [Shape::Triangle, Shape::Saw, Shape::Square, Shape::Pulse] {
            for a in [0.0, 1e-6, 0.01, 0.2, 0.5, 3.0] {
                for k in 0..1000 {
                    let p = k as f64 / 1000.0;
                    let v = wave(shape, p, a, 0.3, true);
                    assert!(
                        v.is_finite() && v.abs() <= 2.0,
                        "{shape:?} a {a} p {p}: {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn step_is_split_at_the_midpoint() {
        // A sample exactly on the saw's reset reads the midpoint of the jump.
        let v = wave(Shape::Saw, 0.0, 0.05, 0.5, true);
        assert!(v.abs() < 1e-12);
    }

    #[test]
    fn phasor_wraps_both_ways() {
        let mut p = Phasor::new();
        p.advance(0.75);
        p.advance(0.5);
        assert!((p.phase() - 0.25).abs() < 1e-12);
        p.advance(-0.5);
        assert!((p.phase() - 0.75).abs() < 1e-12);
        p.reset();
        assert_eq!(p.phase(), 0.0);
    }

    #[test]
    fn increment_is_clamped_and_sanitized() {
        assert_eq!(increment(24_000.0, 48_000.0), 0.5);
        assert_eq!(increment(-1e9, 48_000.0), -0.5);
        assert_eq!(increment(f64::NAN, 48_000.0), 0.0);
        assert_eq!(increment(440.0, 0.0), 0.0);
        assert!((increment(480.0, 48_000.0) - 0.01).abs() < 1e-15);
    }
}
