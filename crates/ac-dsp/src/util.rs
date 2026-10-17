//! Level and pitch conversions.

/// Decibels to linear gain. -60 dB and below is treated as silence, the
/// usual floor for faders.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Linear gain to decibels, floored at -120 dB.
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 1e-6 {
        -120.0
    } else {
        20.0 * gain.log10()
    }
}

/// Semitones to a frequency ratio.
#[inline]
pub fn semitones_to_ratio(st: f32) -> f32 {
    (st / 12.0).exp2()
}

/// Snap a sample count to the nearest whole sample when it is off only by
/// `f32` rounding. Control values arrive as `f32`, so "0.01 s" is really
/// 0.00999999977 s, a hair off 480 samples at 48 kHz; snapping makes
/// round-number times land on exact samples. Values further from a whole
/// number (real fractions) pass through unchanged.
#[inline]
pub fn snap_samples(samples: f64) -> f64 {
    let whole = samples.round();
    if (samples - whole).abs() <= samples.abs() * f64::from(f32::EPSILON) {
        whole
    } else {
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert_eq!(db_to_gain(-60.0), 0.0);
        assert!((gain_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(gain_to_db(0.0), -120.0);
        assert!((semitones_to_ratio(12.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn snapping_only_absorbs_f32_rounding() {
        assert_eq!(snap_samples(f64::from(0.01f32) * 48_000.0), 480.0);
        assert_eq!(snap_samples(f64::from(0.1f32) * 48_000.0), 4800.0);
        assert_eq!(snap_samples(480.25), 480.25);
        assert_eq!(snap_samples(0.0), 0.0);
        assert!(snap_samples(f64::NAN).is_nan());
    }
}
