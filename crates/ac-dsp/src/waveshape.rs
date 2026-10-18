//! Memoryless waveshaping curves (static nonlinearities) for saturation,
//! clipping, and folding.
//!
//! Every curve passes through 0 and has slope 1 there, so a quiet signal
//! comes through at its own level and only peaks are shaped:
//!
//! - `tanh`: the classic smooth saturator.
//! - `hard_clip`: `clamp(x, -1, 1)`.
//! - `soft_clip`: the cubic soft clipper `x − x³/3` (J. O. Smith,
//!   "Physical Audio Signal Processing", Cubic Soft Clipper), scaled to
//!   `x − 4x³/27` so it keeps slope 1 at zero and reaches ±1 (with zero
//!   slope) at |x| = 1.5, holding there.
//! - `fold`: an ideal triangle wavefolder: levels past ±1 fold back toward
//!   zero instead of clipping, again and again for higher drive (the
//!   idealized response of analog folders; F. Esqueda, H. Pöntynen, J. D.
//!   Parker, S. Bilbao, "Virtual Analog Models of the Lockhart and Serge
//!   Wavefolders", Applied Sciences 7(12), 2017).
//! - `tube`: an asymmetric saturator: `tanh(x)` on positive swings and the
//!   harder `tanh(2x)/2` on negative ones, a simple stand-in for a triode's
//!   uneven clipping that adds even harmonics (J. Pakarinen and D. T. Yeh,
//!   "A Review of Digital Techniques for Modeling Vacuum-Tube Guitar
//!   Amplifiers", Computer Music Journal 33(2), 2009, §4.1).

/// Built-in curves, in the order of the Shaper's `shape` choices (which
/// add `custom` after these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Tanh,
    HardClip,
    SoftClip,
    Fold,
    Tube,
}

impl Shape {
    pub const NAMES: &'static [&'static str] = &["tanh", "hard_clip", "soft_clip", "fold", "tube"];

    /// The shape for a choice index, or `None` past the built-ins.
    pub fn from_index(i: usize) -> Option<Shape> {
        Some(match i {
            0 => Shape::Tanh,
            1 => Shape::HardClip,
            2 => Shape::SoftClip,
            3 => Shape::Fold,
            4 => Shape::Tube,
            _ => return None,
        })
    }
}

/// Inputs are held within this range before shaping, so folding (which
/// works modulo 4) keeps its precision and nothing overflows.
const LIMIT: f32 = 1.0e6;

/// Apply `shape` to `x`. Non-finite input gives 0; the output is always
/// within ±1.
#[inline]
pub fn apply(shape: Shape, x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let x = x.clamp(-LIMIT, LIMIT);
    match shape {
        Shape::Tanh => x.tanh(),
        Shape::HardClip => x.clamp(-1.0, 1.0),
        Shape::SoftClip => {
            if x.abs() >= 1.5 {
                x.signum()
            } else {
                x - 4.0 / 27.0 * x * x * x
            }
        }
        Shape::Fold => {
            // A triangle wave of period 4 through the origin.
            let u = (x + 1.0).rem_euclid(4.0) - 1.0;
            if u <= 1.0 { u } else { 2.0 - u }
        }
        Shape::Tube => {
            if x >= 0.0 {
                x.tanh()
            } else {
                0.5 * (2.0 * x).tanh()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Shape; 5] = [
        Shape::Tanh,
        Shape::HardClip,
        Shape::SoftClip,
        Shape::Fold,
        Shape::Tube,
    ];

    #[test]
    fn unity_slope_at_zero_and_bounded() {
        for s in ALL {
            let slope = apply(s, 1e-3) / 1e-3;
            assert!((slope - 1.0).abs() < 1e-3, "{s:?}: {slope}");
            assert_eq!(apply(s, 0.0), 0.0);
            for x in [-1e30, -7.3, -1.0, 0.3, 2.0, 1e30, f32::NAN, f32::INFINITY] {
                let y = apply(s, x);
                assert!(y.is_finite() && y.abs() <= 1.0, "{s:?}({x}) = {y}");
            }
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(apply(Shape::HardClip, 3.0), 1.0);
        assert!((apply(Shape::SoftClip, 1.5) - 1.0).abs() < 1e-6);
        assert_eq!(apply(Shape::SoftClip, -5.0), -1.0);
        assert!((apply(Shape::SoftClip, 0.75) - 0.6875).abs() < 1e-6);
        // Folding: 1.5 comes back to 0.5, 3 to -1, 5 to 1.
        assert!((apply(Shape::Fold, 1.5) - 0.5).abs() < 1e-6);
        assert!((apply(Shape::Fold, 3.0) + 1.0).abs() < 1e-6);
        assert!((apply(Shape::Fold, 5.0) - 1.0).abs() < 1e-6);
        assert!((apply(Shape::Fold, -1.5) + 0.5).abs() < 1e-6);
        // Tube: negative swings clip at half level.
        assert!((apply(Shape::Tube, -100.0) + 0.5).abs() < 1e-6);
        assert!((apply(Shape::Tube, 100.0) - 1.0).abs() < 1e-6);
        assert_eq!(Shape::from_index(5), None);
        assert_eq!(Shape::from_index(3), Some(Shape::Fold));
    }
}
