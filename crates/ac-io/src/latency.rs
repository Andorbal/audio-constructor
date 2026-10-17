//! Round-trip latency measurement (ARCHITECTURE §11: "the settings panel
//! shows the measured round-trip latency").
//!
//! With a cable from an output back to an input, the running stream pauses
//! the engine for about a second, plays a burst of pseudo-random noise on
//! every output, and records every input (see [`Probe`]). [`find_delay`]
//! then locates the burst in the recording by cross-correlation: noise
//! correlates with itself only at one lag, so the peak is sharp even through
//! converter filters, a quiet level, or a noisy line. The result is what a
//! performer hears: output buffering, converters, cable, input buffering,
//! and the duplex ring in between.

use std::fmt;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// Below this normalized correlation the peak is indistinguishable from
/// chance (noise against noise sits near `1 / sqrt(len)`, about 0.01 for
/// the default burst).
pub const MIN_CONFIDENCE: f32 = 0.2;
/// Length of the noise burst, in seconds.
pub const BURST_SECONDS: f64 = 0.35;
/// The longest round trip we listen for, in seconds.
pub const MAX_LATENCY_SECONDS: f64 = 0.75;
/// Burst level: -12 dBFS, enough to stand out on a line input without being
/// alarming if a speaker is connected.
pub const BURST_LEVEL: f32 = 0.25;
/// Input channels captured at once (the loopback can come back on any).
pub const MAX_PROBE_CHANNELS: usize = 8;

/// Where the played signal was found in the capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayEstimate {
    pub samples: usize,
    /// Normalized correlation at the peak: 1 for a perfect (scaled) copy.
    pub confidence: f32,
}

/// A measured round trip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Latency {
    pub samples: usize,
    pub ms: f64,
    pub sample_rate: u32,
    /// The input the signal came back on (counting from 0).
    pub input_channel: usize,
    pub confidence: f32,
}

impl fmt::Display for Latency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:.2} ms ({} samples at {} Hz, input {}, confidence {:.2})",
            self.ms,
            self.samples,
            self.sample_rate,
            self.input_channel + 1,
            self.confidence
        )
    }
}

/// Why a measurement didn't produce a number.
#[derive(Clone, Debug, PartialEq)]
pub enum LatencyError {
    /// The stream has no input device.
    NoInput,
    /// A measurement is already running.
    Busy,
    /// The stream didn't finish the probe in time (stopped, or stalled).
    Timeout,
    /// No correlation peak: nothing is connected from an output to an input.
    NoSignal { best_confidence: f32 },
    /// This host can't measure (the null host has no hardware).
    Unsupported,
}

impl fmt::Display for LatencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LatencyError::NoInput => f.write_str("the stream has no input device to listen on"),
            LatencyError::Busy => f.write_str("a latency measurement is already running"),
            LatencyError::Timeout => f.write_str("the audio stream didn't finish the measurement"),
            LatencyError::NoSignal { best_confidence } => write!(
                f,
                "the test signal didn't come back (best match {best_confidence:.2}); connect an \
                 output to an input with a cable and try again"
            ),
            LatencyError::Unsupported => f.write_str("this host can't measure latency"),
        }
    }
}

impl std::error::Error for LatencyError {}

/// A burst of pseudo-random ±`level` noise (xorshift64*, Vigna 2016).
/// Binary noise has the flattest spectrum for its peak level, which gives
/// the sharpest correlation peak per unit of loudness.
pub fn test_signal(len: usize, level: f32, seed: u64) -> Vec<f32> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let r = x.wrapping_mul(0x2545_f491_4f6c_dd1d);
            if r >> 63 == 0 { level } else { -level }
        })
        .collect()
}

/// Find the lag in `0..=max_delay` at which `played` best matches
/// `captured`, by FFT cross-correlation. `None` if there's no clear match
/// (confidence below [`MIN_CONFIDENCE`]) or not enough audio to compare.
pub fn find_delay(played: &[f32], captured: &[f32], max_delay: usize) -> Option<DelayEstimate> {
    let best = correlate(played, captured, max_delay)?;
    (best.confidence >= MIN_CONFIDENCE).then_some(best)
}

/// The best lag and its confidence, however poor.
fn correlate(played: &[f32], captured: &[f32], max_delay: usize) -> Option<DelayEstimate> {
    let lp = played.len();
    if lp == 0 || captured.len() < lp {
        return None;
    }
    let max_delay = max_delay.min(captured.len() - lp);
    let n = (captured.len() + lp).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);

    let mut buf = fwd.make_input_vec();
    for (b, s) in buf.iter_mut().zip(captured) {
        *b = f64::from(*s);
    }
    let mut cap_spec = fwd.make_output_vec();
    fwd.process(&mut buf, &mut cap_spec).ok()?;

    buf.fill(0.0);
    for (b, s) in buf.iter_mut().zip(played) {
        *b = f64::from(*s);
    }
    let mut play_spec = fwd.make_output_vec();
    fwd.process(&mut buf, &mut play_spec).ok()?;

    // Correlation is multiplication by the conjugate in the frequency domain.
    for (c, p) in cap_spec.iter_mut().zip(&play_spec) {
        *c *= p.conj();
    }
    // The DC and Nyquist bins of a real signal's spectrum are real.
    let last = cap_spec.len() - 1;
    cap_spec[0].im = 0.0;
    cap_spec[last] = Complex::new(cap_spec[last].re, 0.0);
    let mut corr = inv.make_output_vec();
    inv.process(&mut cap_spec, &mut corr).ok()?;

    let e_played: f64 = played.iter().map(|s| f64::from(*s).powi(2)).sum();
    if e_played <= 0.0 {
        return None;
    }
    // Energy of each `lp`-long window of the capture, from prefix sums.
    let mut prefix = Vec::with_capacity(captured.len() + 1);
    prefix.push(0.0f64);
    let mut acc = 0.0;
    for s in captured {
        acc += f64::from(*s).powi(2);
        prefix.push(acc);
    }
    let mut best: Option<(usize, f64)> = None;
    for (d, r) in corr.iter().take(max_delay + 1).enumerate() {
        let mag = r.abs();
        if best.is_none_or(|(_, m)| mag > m) {
            best = Some((d, mag));
        }
    }
    let (d, mag) = best?;
    let e_window = prefix[d + lp] - prefix[d];
    // The inverse transform is unnormalized: scale by n.
    let rho = if e_window > 0.0 {
        mag / n as f64 / (e_played * e_window).sqrt()
    } else {
        0.0
    };
    Some(DelayEstimate {
        samples: d,
        confidence: rho.clamp(0.0, 1.0) as f32,
    })
}

/// A latency probe in flight. Built on the control thread, moved to the
/// audio thread through a ring, filled there by [`Probe::step`] (which
/// never allocates), and moved back to be analyzed.
#[derive(Debug)]
pub struct Probe {
    signal: Vec<f32>,
    /// Channel-major: channel `c` is `[c * len .. (c + 1) * len]`.
    captured: Vec<f32>,
    channels: usize,
    len: usize,
    pos: usize,
    sample_rate: u32,
}

impl Probe {
    /// A probe for a stream at `sample_rate` with `in_channels` inputs.
    pub fn new(sample_rate: u32, in_channels: usize) -> Probe {
        let rate = f64::from(sample_rate.max(1));
        let signal = test_signal((rate * BURST_SECONDS).round() as usize, BURST_LEVEL, 0x5eed);
        let len = signal.len() + (rate * MAX_LATENCY_SECONDS).round() as usize;
        let channels = in_channels.min(MAX_PROBE_CHANNELS);
        Probe {
            signal,
            captured: vec![0.0; len * channels],
            channels,
            len,
            pos: 0,
            sample_rate,
        }
    }

    /// Play the next `frames` of the burst on every output and record every
    /// input. Buffers are channel-major with stride `frames`. Real-time safe.
    pub fn step(
        &mut self,
        input: &[f32],
        in_channels: usize,
        output: &mut [f32],
        out_channels: usize,
        frames: usize,
    ) {
        for i in 0..frames {
            let p = self.pos + i;
            let s = self.signal.get(p).copied().unwrap_or(0.0);
            for c in 0..out_channels {
                if let Some(o) = output.get_mut(c * frames + i) {
                    *o = s;
                }
            }
            if p < self.len {
                for c in 0..self.channels.min(in_channels) {
                    let v = input.get(c * frames + i).copied().unwrap_or(0.0);
                    if let Some(d) = self.captured.get_mut(c * self.len + p) {
                        *d = v;
                    }
                }
            }
        }
        self.pos += frames;
    }

    pub fn done(&self) -> bool {
        self.pos >= self.len
    }

    /// Find the burst on the best input channel.
    pub fn analyze(&self) -> Result<Latency, LatencyError> {
        if self.channels == 0 {
            return Err(LatencyError::NoInput);
        }
        let max_delay = self.len.saturating_sub(self.signal.len());
        let mut best: Option<(usize, DelayEstimate)> = None;
        for c in 0..self.channels {
            let cap = &self.captured[c * self.len..(c + 1) * self.len];
            if let Some(e) = correlate(&self.signal, cap, max_delay)
                && best.is_none_or(|(_, b)| e.confidence > b.confidence)
            {
                best = Some((c, e));
            }
        }
        match best {
            Some((c, e)) if e.confidence >= MIN_CONFIDENCE => Ok(Latency {
                samples: e.samples,
                ms: e.samples as f64 * 1000.0 / f64::from(self.sample_rate.max(1)),
                sample_rate: self.sample_rate,
                input_channel: c,
                confidence: e.confidence,
            }),
            other => Err(LatencyError::NoSignal {
                best_confidence: other.map_or(0.0, |(_, e)| e.confidence),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loopback: delay, gain, a gentle low-pass (converter filters), an
    /// inverted polarity option, and additive noise.
    fn loopback(played: &[f32], delay: usize, gain: f32, noise: f32, total: usize) -> Vec<f32> {
        let mut out = vec![0.0; total];
        let mut lp = 0.0f32;
        for (i, o) in out.iter_mut().enumerate() {
            let x = if i >= delay {
                played.get(i - delay).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            lp += 0.6 * (x - lp);
            *o = lp * gain;
        }
        for (o, n) in out.iter_mut().zip(test_signal(total, noise, 99)) {
            *o += n;
        }
        out
    }

    #[test]
    fn finds_a_delayed_attenuated_noisy_copy() {
        let played = test_signal(8192, 0.25, 7);
        for &delay in &[0usize, 1, 37, 480, 2_047, 13_001] {
            let cap = loopback(&played, delay, 0.3, 0.05, 8192 + 16_000);
            let e = find_delay(&played, &cap, 16_000).expect("found");
            // The one-pole filter adds well under a sample of group delay.
            assert!(e.samples.abs_diff(delay) <= 1, "{delay}: {e:?}");
            assert!(e.confidence > 0.5, "{delay}: {e:?}");
        }
    }

    #[test]
    fn inverted_polarity_still_matches() {
        let played = test_signal(4096, 0.25, 3);
        let cap = loopback(&played, 300, -0.5, 0.01, 10_000);
        let e = find_delay(&played, &cap, 5_000).unwrap();
        assert_eq!(e.samples, 300);
    }

    #[test]
    fn no_loopback_is_no_signal() {
        let played = test_signal(8192, 0.25, 7);
        let silence = vec![0.0; 30_000];
        assert!(find_delay(&played, &silence, 20_000).is_none());
        let hiss = test_signal(30_000, 0.1, 1234);
        assert!(find_delay(&played, &hiss, 20_000).is_none());
    }

    #[test]
    fn degenerate_inputs() {
        assert!(find_delay(&[], &[1.0; 10], 5).is_none());
        assert!(find_delay(&[1.0; 10], &[1.0; 5], 5).is_none());
        assert!(find_delay(&[0.0; 10], &[1.0; 50], 5).is_none());
    }

    #[test]
    fn probe_measures_a_simulated_loopback() {
        // Run the probe as the audio thread would, with a device whose input
        // 2 hears output 1 after 1 234 samples.
        let rate = 48_000;
        let mut probe = Probe::new(rate, 2);
        let block = 256;
        let delay = 1_234;
        let mut line: Vec<f32> = vec![0.0; delay];
        let mut input = vec![0.0; 2 * block];
        let mut output = vec![0.0; 2 * block];
        while !probe.done() {
            for i in 0..block {
                input[i] = 0.0;
                input[block + i] = line.get(i).copied().unwrap_or(0.0) * 0.5;
            }
            line.drain(..block.min(line.len()));
            probe.step(&input, 2, &mut output, 2, block);
            line.extend_from_slice(&output[..block]);
        }
        let l = probe.analyze().unwrap();
        assert_eq!(l.samples, delay);
        assert_eq!(l.input_channel, 1);
        assert!((l.ms - 1234.0 / 48.0).abs() < 1e-9);
        assert!(l.to_string().contains("25.71 ms"), "{l}");
    }

    #[test]
    fn probe_without_loopback_explains_itself() {
        let mut probe = Probe::new(8_000, 1);
        let input = vec![0.0; 512];
        let mut output = vec![0.0; 512];
        while !probe.done() {
            probe.step(&input, 1, &mut output, 1, 512);
        }
        let err = probe.analyze().unwrap_err();
        assert!(matches!(err, LatencyError::NoSignal { .. }));
        assert!(err.to_string().contains("cable"));
        assert_eq!(Probe::new(8_000, 0).analyze(), Err(LatencyError::NoInput));
    }
}
