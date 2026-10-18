//! Integrated loudness after ITU-R BS.1770-4, "Algorithms to measure audio
//! programme loudness and true-peak audio level" (2015), Annex 1.
//!
//! - **K-weighting**: a high shelf (+4 dB above about 1.5 kHz, modelling
//!   the head) followed by a high-pass (the "RLB" curve). The standard
//!   lists coefficients for 48 kHz only; these are built at any rate from
//!   analog prototype parameters (corner, gain, Q) chosen so the bilinear
//!   transform reproduces the 48 kHz coefficients, which the tests check.
//! - **Gating**: mean square over 400 ms blocks overlapping by 75 %; blocks
//!   below −70 LKFS are dropped (absolute gate), then blocks more than
//!   10 LU below the loudness of what is left (relative gate). Channels
//!   are weighted equally, which is the standard's weighting for mono,
//!   stereo, and front channels.
//!
//! Runs off the audio thread (it allocates), as whole-input nodes do.

use std::f64::consts::PI;

/// Normalized biquad coefficients `[b0, b1, b2, a1, a2]`.
pub type Biquad = [f64; 5];

/// The two K-weighting stages at `rate`: the shelf, then the high-pass.
pub fn k_weighting(rate: f64) -> (Biquad, Biquad) {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        48_000.0
    };
    // Stage 1: high shelf.
    let (f0, gain_db, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = [
        (vh + vb * k / q + k * k) / a0,
        2.0 * (k * k - vh) / a0,
        (vh - vb * k / q + k * k) / a0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    ];
    // Stage 2: high-pass, with the standard's unnormalized numerator.
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = [
        1.0,
        -2.0,
        1.0,
        2.0 * (k * k - 1.0) / a0,
        (1.0 - k / q + k * k) / a0,
    ];
    (shelf, high_pass)
}

/// Filter `x` through one biquad (direct form I, `f64`).
fn filter(k: &Biquad, x: &mut [f64]) {
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for v in x.iter_mut() {
        let x0 = *v;
        let y0 = k[0] * x0 + k[1] * x1 + k[2] * x2 - k[3] * y1 - k[4] * y2;
        (x2, x1, y2, y1) = (x1, x0, y1, y0);
        *v = y0;
    }
}

/// Loudness of a mean square, in LKFS.
fn lkfs(z: f64) -> f64 {
    -0.691 + 10.0 * z.log10()
}

/// Per-block mean squares (summed over channels) of the K-weighted
/// signal. A signal shorter than one block gives one block over all of it.
fn block_energies(channels: &[Vec<f32>], rate: f64) -> Vec<f64> {
    let rate = if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        48_000.0
    };
    let len = channels.iter().map(Vec::len).max().unwrap_or(0);
    if len == 0 {
        return Vec::new();
    }
    let step = ((0.1 * rate).round() as usize).max(1);
    let (shelf, high_pass) = k_weighting(rate);
    // Energy of each 100 ms step, summed over channels.
    let steps = len / step;
    let mut step_energy = vec![0.0; steps];
    let mut total = 0.0;
    for ch in channels {
        let mut y: Vec<f64> = ch
            .iter()
            .map(|v| if v.is_finite() { f64::from(*v) } else { 0.0 })
            .collect();
        y.resize(len, 0.0);
        filter(&shelf, &mut y);
        filter(&high_pass, &mut y);
        for (s, chunk) in step_energy.iter_mut().zip(y.chunks_exact(step)) {
            *s += chunk.iter().map(|v| v * v).sum::<f64>();
        }
        total += y.iter().map(|v| v * v).sum::<f64>();
    }
    if steps < 4 {
        return vec![total / len as f64];
    }
    step_energy
        .windows(4)
        .map(|w| w.iter().sum::<f64>() / (4 * step) as f64)
        .collect()
}

/// Gated integrated loudness in LUFS, or `None` when no block is louder
/// than the absolute gate (silence, or everything below −70 LKFS).
pub fn integrated(channels: &[Vec<f32>], rate: f64) -> Option<f64> {
    let blocks = block_energies(channels, rate);
    let gate = |z: &&f64| **z > 0.0 && lkfs(**z) > -70.0;
    let loud: Vec<f64> = blocks.iter().filter(gate).copied().collect();
    if loud.is_empty() {
        return None;
    }
    let relative = lkfs(loud.iter().sum::<f64>() / loud.len() as f64) - 10.0;
    let kept: Vec<f64> = loud.into_iter().filter(|z| lkfs(*z) > relative).collect();
    if kept.is_empty() {
        return None;
    }
    Some(lkfs(kept.iter().sum::<f64>() / kept.len() as f64))
}

/// Ungated loudness (K-weighted mean square over everything), or `None`
/// for silence.
pub fn ungated(channels: &[Vec<f32>], rate: f64) -> Option<f64> {
    let blocks = block_energies(channels, rate);
    if blocks.is_empty() {
        return None;
    }
    let z = blocks.iter().sum::<f64>() / blocks.len() as f64;
    (z > 0.0).then(|| lkfs(z))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f64, seconds: f64, rate: f64) -> Vec<f32> {
        (0..(seconds * rate) as usize)
            .map(|i| (amp * (2.0 * PI * freq * i as f64 / rate).sin()) as f32)
            .collect()
    }

    #[test]
    fn coefficients_match_the_standard_at_48k() {
        let (s, h) = k_weighting(48_000.0);
        let want_s = [
            1.535_124_859_586_97,
            -2.691_696_189_406_38,
            1.198_392_810_852_85,
            -1.690_659_293_182_41,
            0.732_480_774_215_85,
        ];
        let want_h = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];
        for (a, b) in s.iter().zip(&want_s).chain(h.iter().zip(&want_h)) {
            assert!((a - b).abs() < 1e-8, "{a} vs {b}");
        }
    }

    #[test]
    fn full_scale_1k_sine_reads_minus_3_lufs() {
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            let x = sine(997.0, 1.0, 5.0, rate);
            let l = integrated(&[x], rate).unwrap();
            assert!((l + 3.01).abs() < 0.05, "{rate}: {l}");
        }
        // Two identical channels read 3 dB louder.
        let x = sine(997.0, 1.0, 5.0, 48_000.0);
        let l = integrated(&[x.clone(), x], 48_000.0).unwrap();
        assert!((l - 0.0).abs() < 0.05, "{l}");
    }

    #[test]
    fn gates_drop_silence_and_quiet_passages() {
        let rate = 48_000.0;
        let loud = sine(997.0, 0.1, 5.0, rate);
        let alone = integrated(std::slice::from_ref(&loud), rate).unwrap();
        let mut with_silence = loud.clone();
        with_silence.extend(vec![0.0; 5 * 48_000]);
        // Blocks straddling the edge still count, which costs about 0.13 LU.
        let l = integrated(&[with_silence], rate).unwrap();
        assert!((l - alone).abs() < 0.2, "{l} vs {alone}");
        let mut with_quiet = loud;
        with_quiet.extend(sine(997.0, 0.003, 5.0, rate));
        let l = integrated(&[with_quiet], rate).unwrap();
        assert!((l - alone).abs() < 0.2, "{l} vs {alone}");
        // Without the gates the silence would pull it down by 3 LU.
        let mut half = sine(997.0, 0.1, 5.0, rate);
        half.extend(vec![0.0; 5 * 48_000]);
        let u = ungated(&[half], rate).unwrap();
        assert!((u - alone + 3.0).abs() < 0.2, "{u}");
    }

    #[test]
    fn silence_and_short_signals() {
        assert_eq!(integrated(&[vec![0.0; 48_000]], 48_000.0), None);
        assert_eq!(integrated(&[], 48_000.0), None);
        assert_eq!(ungated(&[vec![0.0; 100]], 48_000.0), None);
        // Shorter than a block: measured as one block.
        let x = sine(997.0, 1.0, 0.2, 48_000.0);
        let l = integrated(&[x], 48_000.0).unwrap();
        assert!((l + 3.01).abs() < 0.2, "{l}");
        // Below the absolute gate: only the ungated figure exists.
        let quiet = sine(997.0, 1e-4, 2.0, 48_000.0);
        assert_eq!(integrated(std::slice::from_ref(&quiet), 48_000.0), None);
        let u = ungated(&[quiet], 48_000.0).unwrap();
        assert!((u + 83.01).abs() < 0.1, "{u}");
    }
}
