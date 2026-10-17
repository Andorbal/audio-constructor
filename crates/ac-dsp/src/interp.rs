//! Interpolation between samples, for fractional delays and playback rates.

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation: the value between
/// `x0` (t = 0) and `x1` (t = 1), using one neighbour on each side.
///
/// From O. Niemitalo, "Polynomial Interpolators for High-Quality Resampling
/// of Oversampled Audio" (2001), the 4-point 3rd-order Hermite x-form. It
/// passes exactly through the samples, so integer positions read back the
/// input unchanged.
#[inline]
pub fn hermite4(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_the_samples() {
        assert_eq!(hermite4(3.0, -1.0, 2.0, 7.0, 0.0), -1.0);
        assert!((hermite4(3.0, -1.0, 2.0, 7.0, 1.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn reproduces_lines_and_parabolas() {
        // Cubic Hermite is exact for polynomials up to degree 2.
        let f = |x: f32| 0.5 * x * x - 2.0 * x + 1.0;
        for k in 0..=10 {
            let t = k as f32 / 10.0;
            let v = hermite4(f(-1.0), f(0.0), f(1.0), f(2.0), t);
            assert!((v - f(t)).abs() < 1e-5, "t {t}");
        }
    }
}
