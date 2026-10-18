use ac_core::{
    Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::cepstrum::{CepstralEnvelope, interpolate};
use ac_dsp::delay::DelayLine;
use ac_dsp::fft::{Fft, Window, ola_gain};

node! {
    kind: "pitch_shift",
    name: "Pitch Shift",
    category: Time,
    keywords: [
        "pitch", "transpose", "harmonizer", "octave", "formant", "chipmunk", "detune",
        "shifter", "voice"
    ],
    /// Shifts pitch up or down without changing speed, with the formants
    /// (the vowel color of a voice) kept in place or moved on their own.
    ///
    /// - `semitones` (−24 to 24, clamped): the shift; 12 is an octave up.
    /// - `formant` (semitones, −12 to 12, clamped): where the spectral
    ///   envelope goes. 0 keeps it where the input had it, so a shifted voice
    ///   still sounds like the same person; positive values move it up
    ///   (smaller, brighter), negative down (bigger, darker), whatever the
    ///   pitch does. Set it equal to `semitones` to let the formants move
    ///   with the pitch (the classic sped-up-tape sound), which skips the
    ///   correction entirely.
    /// - `grain` (seconds, 0.01 up to `max_grain`): the length of the
    ///   overlapping grains. Short grains suit speech and drums but buzz on
    ///   low notes; long grains are smoother on sustained sounds but smear
    ///   attacks into slight echoes.
    /// - `mix` blends the input (0) with the shifted signal (1). The dry
    ///   signal is delayed to line up with the shifted one, so 0.5 makes a
    ///   clean two-voice harmony.
    ///
    /// Pitch is shifted in the time domain: two read heads sweep through a
    /// short delay line faster (up) or slower (down) than it fills, each
    /// faded in and out with a raised-cosine window, and they take turns so
    /// one is always at full level while the other jumps back. The formant
    /// correction then works on overlapping FFT frames (about 11 ms, hop of a
    /// quarter frame): it estimates the input's spectral envelope by
    /// cepstral smoothing and reshapes the shifted signal's spectrum towards
    /// the envelope moved by `formant`, by at most ±24 dB per frequency.
    ///
    /// Limits: shifting up passes whatever the input has near the top
    /// unfiltered, so bright material can alias slightly. The envelope
    /// estimate assumes a pitched, harmonic-rich sound (voice, strings,
    /// synths); a very sparse sound such as a pure sine has no real envelope,
    /// so with `formant` at 0 it loses level when shifted far. Large shifts
    /// of complex mixes warble, as with any real-time shifter.
    ///
    /// Latency: the output runs half of `max_grain` plus one FFT frame
    /// behind the input (2913 samples, about 61 ms, at 48 kHz with the
    /// default `max_grain` of 0.1 s). The node reports it, so the engine
    /// delays parallel paths to match; for live playing, lower `max_grain`
    /// (0.02 gives about 21 ms).
    ///
    /// Config: `max_grain`, the longest grain in seconds (default 0.1, 0.01
    /// to 0.2), which sets the latency. Mono or stereo; both channels share
    /// one set of read heads, so the stereo image holds.
    ///
    /// **Try this:** mix a voice with itself at `semitones` 4 and 7 through
    /// two Pitch Shifts and a `Mix` for an instant major chord, or set
    /// `semitones` 0 and sweep `formant` from −6 to 6 to morph one voice
    /// from giant to elf.
    pub struct PitchShift {
        channels: Vec<Channel>,
        spectral: Spectral,
        /// Read heads' phase in cycles, in [0, 1).
        phase: f64,
        /// Delay of a head at full level, in samples.
        center: usize,
        /// Total latency: `center` plus the frame delay.
        latency: usize,
        /// Grain length limits, in samples.
        grain_range: (f64, f64),
        rate: f64,
        /// Position in the frame rings, shared by both channels.
        pos: usize,
        /// The current block's head positions and shift, shared by both channels.
        plan: Vec<Step>,
    }
}

const IN: usize = 0;
const SEMITONES: usize = 1;
const FORMANT: usize = 2;
const GRAIN: usize = 3;
const MIX: usize = 4;
const OUT: usize = 0;

const CHANNELS: usize = 2;

const DEFAULT_MAX_GRAIN: f64 = 0.1;
const MIN_GRAIN: f64 = 0.01;
/// The shortest head delay, in samples: room for the interpolator.
const MIN_DELAY: usize = 2;
/// FFT frame length for the formant correction, in seconds (rounded to a
/// power of two: 512 samples at 44.1 and 48 kHz).
const FRAME: f64 = 512.0 / 48_000.0;
/// Lifter cutoff: envelope detail finer than 1.5 ms of quefrency is
/// treated as pitch. That is below the period of voices up to ~650 Hz.
const LIFTER: f64 = 0.0015;
/// Most the correction lifts or cuts a frequency: ±24 dB, as a natural log.
const MAX_LOG_GAIN: f32 = 2.772_588_7;

/// One frame's read heads and shift.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Step {
    delay_a: f64,
    delay_b: f64,
    /// Head A's window; head B's is `1 − weight_a`.
    weight_a: f32,
    semitones: f32,
    formant: f32,
}

/// One channel's input history and frame rings.
#[derive(Clone, Debug)]
struct Channel {
    line: DelayLine,
    /// The input delayed to line up with the heads, for the envelope.
    dry_ring: Vec<f32>,
    /// The shifted signal.
    shifted_ring: Vec<f32>,
    /// Overlap-add accumulator.
    acc: Vec<f32>,
}

/// The FFT machinery and scratch shared by both channels.
#[derive(Clone, Debug)]
struct Spectral {
    fft: Fft,
    envelope: CepstralEnvelope,
    window: Vec<f32>,
    /// The synthesis window: the analysis window scaled so analysis ×
    /// synthesis windows overlap-add to 1.
    synthesis: Vec<f32>,
    hop: usize,
    frame: Vec<f32>,
    spec_dry: Vec<f32>,
    spec_shifted: Vec<f32>,
    log_env: Vec<f32>,
    out: Vec<f32>,
}

fn max_grain(config: &Config) -> f64 {
    let g = config.get_f64_or("max_grain", DEFAULT_MAX_GRAIN);
    if g.is_finite() {
        g.clamp(MIN_GRAIN, 0.2)
    } else {
        DEFAULT_MAX_GRAIN
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// The FFT size for a rate: `FRAME` seconds rounded to a power of two.
fn frame_size(rate: f64) -> usize {
    let target = (FRAME * rate).clamp(128.0, 4096.0);
    let exp = target.log2().round() as u32;
    1usize << exp.clamp(7, 12)
}

impl Spectral {
    fn new(rate: f64) -> Self {
        let n = frame_size(rate);
        let window = Window::Hann.table(n);
        let hop = n / 4;
        let bins = n / 2 + 1;
        let lifter = ((LIFTER * rate).round() as usize).clamp(4, n / 4);
        let gain = ola_gain(&window, hop);
        Spectral {
            fft: Fft::new(n),
            envelope: CepstralEnvelope::new(n, lifter),
            synthesis: window.iter().map(|w| w * gain).collect(),
            window,
            hop,
            frame: vec![0.0; n],
            spec_dry: vec![0.0; 2 * bins],
            spec_shifted: vec![0.0; 2 * bins],
            log_env: vec![0.0; bins],
            out: vec![0.0; n],
        }
    }

    fn size(&self) -> usize {
        self.window.len()
    }

    /// Process the frame that ends at ring position `pos` and overlap-add
    /// it into the channel's accumulator.
    fn frame(&mut self, ch: &mut Channel, pos: usize, semitones: f32, formant: f32) {
        // The oldest sample of the frame; the ring is read as two
        // contiguous runs from there.
        let first = (pos + 1) % self.size().max(1);
        // Formants moved exactly with the pitch need no correction: the
        // frame passes through (windowed twice and overlap-added, which
        // reconstructs it).
        let identity = (semitones - formant).abs() < 1e-4;
        windowed(&ch.shifted_ring, first, &self.window, &mut self.frame);
        if identity {
            self.out.copy_from_slice(&self.frame);
        } else {
            self.fft.forward(&self.frame, &mut self.spec_shifted);
            windowed(&ch.dry_ring, first, &self.window, &mut self.frame);
            self.fft.forward(&self.frame, &mut self.spec_dry);
            self.envelope
                .log_envelope(&mut self.fft, &self.spec_dry, &mut self.log_env);
            // The shifted signal's envelope is the input's scaled by the
            // pitch ratio r; the target is the input's scaled by the formant
            // ratio φ. Gain at bin k: E(k / φ) / E(k / r).
            let inv_r = (-semitones / 12.0).exp2();
            let inv_phi = (-formant / 12.0).exp2();
            for (k, pair) in self.spec_shifted.chunks_exact_mut(2).enumerate() {
                let kf = k as f32;
                let log_gain = interpolate(&self.log_env, kf * inv_phi)
                    - interpolate(&self.log_env, kf * inv_r);
                let g = log_gain.clamp(-MAX_LOG_GAIN, MAX_LOG_GAIN).exp();
                pair[0] *= g;
                pair[1] *= g;
            }
            self.fft.inverse(&self.spec_shifted, &mut self.out);
        }
        let split = first.min(ch.acc.len());
        let (head, tail) = ch.acc.split_at_mut(split);
        for ((a, o), w) in tail
            .iter_mut()
            .chain(head.iter_mut())
            .zip(&self.out)
            .zip(&self.synthesis)
        {
            *a += o * w;
        }
    }
}

/// `ring` read from `first` round to `first − 1`, times `window`, into `out`.
fn windowed(ring: &[f32], first: usize, window: &[f32], out: &mut [f32]) {
    let (head, tail) = ring.split_at(first.min(ring.len()));
    for ((o, r), w) in out.iter_mut().zip(tail.iter().chain(head)).zip(window) {
        *o = r * w;
    }
}

impl PitchShift {
    /// Work out the read heads for every frame of this block.
    fn plan(&mut self, io: &Io, frames: usize) {
        let semitones = io.inputs.float(SEMITONES);
        let formant = io.inputs.float(FORMANT);
        let grain = io.inputs.float(GRAIN);
        let center = self.center as f64;
        // The pitch ratio for the last `semitones` seen (usually steady).
        let mut ratio = (f32::NAN, 1.0f64);
        for (i, step) in self.plan.iter_mut().take(frames).enumerate() {
            let st = finite_or(semitones.at(i), 0.0).clamp(-24.0, 24.0);
            let fm = finite_or(formant.at(i), 0.0).clamp(-12.0, 12.0);
            let w = (f64::from(finite_or(grain.at(i), 0.05)) * self.rate)
                .clamp(self.grain_range.0, self.grain_range.1);
            let a = self.phase;
            let b = if a < 0.5 { a + 0.5 } else { a - 0.5 };
            // Each head's delay sweeps `w` samples around the center; at
            // phase 0.5 a head sits on the center at full level, at 0 and 1
            // it is silent while it jumps.
            let s = (std::f32::consts::PI * a as f32).sin();
            *step = Step {
                delay_a: center + (a - 0.5) * w,
                delay_b: center + (b - 0.5) * w,
                weight_a: s * s,
                semitones: st,
                formant: fm,
            };
            // The delay changes by (1 − r) samples per sample.
            if st != ratio.0 {
                ratio = (st, f64::from(st / 12.0).exp2());
            }
            self.phase += (1.0 - ratio.1) / w;
            self.phase -= self.phase.floor();
        }
    }
}

impl Node for PitchShift {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let max = max_grain(config);
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(Channels::UpTo(2))
                    .doc("Mono or stereo signal to shift."),
            )
            .input(
                SocketSpec::float("semitones", Unit::Semitones, 12.0)
                    .range(-24.0, 24.0)
                    .doc("Pitch shift; 12 is an octave up."),
            )
            .input(
                SocketSpec::float("formant", Unit::Semitones, 0.0)
                    .range(-12.0, 12.0)
                    .doc("Formant shift; 0 keeps the input's."),
            )
            .input(
                SocketSpec::float("grain", Unit::Seconds, 0.05f64.min(max))
                    .range(MIN_GRAIN, max)
                    .doc("Grain length."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("0 is the (delayed) input, 1 only the shifted signal."),
            )
            .output(SocketSpec::audio("out").doc("The shifted signal."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            48_000.0
        };
        let max_w = (max_grain(config) * rate).round().max(4.0);
        let min_w = (MIN_GRAIN * rate).round().clamp(4.0, max_w);
        let center = MIN_DELAY + (max_w / 2.0).ceil() as usize;
        let spectral = Spectral::new(rate);
        let n = spectral.size();
        let latency = center + n - 1;
        // Room for the furthest head (plus interpolator neighbours) and the
        // dry signal at the full latency.
        let line_len = (center + (max_w / 2.0).ceil() as usize + 2).max(latency) + 1;
        let channel = Channel {
            line: DelayLine::new(line_len),
            dry_ring: vec![0.0; n],
            shifted_ring: vec![0.0; n],
            acc: vec![0.0; n],
        };
        PitchShift {
            channels: vec![channel; CHANNELS],
            spectral,
            phase: 0.5,
            center,
            latency,
            grain_range: (min_w, max_w),
            rate,
            pos: 0,
            plan: vec![Step::default(); ctx.max_block.max(1)],
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        self.plan(io, frames);
        let input = io.inputs.audio(IN);
        let mix = io.inputs.float(MIX);
        // Frame size and hop are powers of two, so positions wrap by masking.
        let mask = self.spectral.size() - 1;
        let hop_mask = self.spectral.hop - 1;
        let (center, latency) = (self.center, self.latency);
        let start = self.pos;
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels().min(CHANNELS) {
            let src = input.channel(c);
            let dst = out.channel(c);
            let Some(ch) = self.channels.get_mut(c) else {
                continue;
            };
            let mut pos = start;
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let Some(step) = self.plan.get(i).copied() else {
                    break;
                };
                let x = finite_or(x, 0.0);
                let aligned = ch.line.tap(center);
                let dry = ch.line.tap(latency);
                let shifted = step.weight_a * ch.line.read(step.delay_a)
                    + (1.0 - step.weight_a) * ch.line.read(step.delay_b);
                ch.line.write(x);
                if let (Some(d), Some(s)) = (ch.dry_ring.get_mut(pos), ch.shifted_ring.get_mut(pos))
                {
                    *d = aligned;
                    *s = shifted;
                }
                if pos & hop_mask == hop_mask {
                    self.spectral.frame(ch, pos, step.semitones, step.formant);
                }
                pos = (pos + 1) & mask;
                let wet = ch.acc.get_mut(pos).map_or(0.0, std::mem::take);
                let m = finite_or(mix.at(i), 1.0).clamp(0.0, 1.0);
                *o = dry * (1.0 - m) + wet * m;
            }
        }
        self.pos = (start + frames) & mask;
    }

    fn latency(&self) -> u32 {
        self.latency.min(u32::MAX as usize) as u32
    }

    fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.line.clear();
            ch.dry_ring.fill(0.0);
            ch.shifted_ring.fill(0.0);
            ch.acc.fill(0.0);
        }
        self.phase = 0.5;
        self.pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32 * 0.5)
            .collect()
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) - 0.5
            })
            .collect()
    }

    /// A pulse train at `f0` through a resonator at `formant`: a one-formant
    /// vowel.
    fn vowel(f0: f64, formant: f64, n: usize) -> Vec<f32> {
        let period = (RATE / f0).round() as usize;
        let r = 0.995f64;
        let w = std::f64::consts::TAU * formant / RATE;
        let (a1, a2) = (-2.0 * r * w.cos(), r * r);
        let (mut y1, mut y2) = (0.0f64, 0.0f64);
        (0..n)
            .map(|i| {
                let x = if i % period == 0 { 0.05 } else { 0.0 };
                let y = x - a1 * y1 - a2 * y2;
                y2 = y1;
                y1 = y;
                y as f32
            })
            .collect()
    }

    fn prepare(config: serde_json::Value) -> PitchShift {
        let ctx = PrepareCtx {
            sample_rate: RATE,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Pitch Shift",
            resources: &NoResources,
        };
        PitchShift::new(&config, &ctx)
    }

    #[test]
    fn latency_is_half_the_longest_grain_plus_a_frame() {
        assert_eq!(prepare(json!({})).latency(), 2 + 2_400 + 511);
        assert_eq!(
            prepare(json!({ "max_grain": 0.02 })).latency(),
            2 + 480 + 511
        );
        assert_eq!(
            prepare(json!({ "max_grain": 9.0 })).latency(),
            2 + 4_800 + 511
        );
        assert_eq!(frame_size(44_100.0), 512);
        assert_eq!(frame_size(96_000.0), 1024);
        let sig = PitchShift::describe(&json!({ "max_grain": 0.03 }));
        assert_eq!(sig.inputs[GRAIN].range, Some((0.01, 0.03)));
    }

    #[test]
    fn an_impulse_lines_up_with_the_reported_latency() {
        let lat = prepare(json!({})).latency() as usize;
        let n = lat + 2_000;
        let mut x = vec![0.0; n];
        x[100] = 1.0;
        for mix in [0.0, 1.0] {
            let mut h = Harness::new::<PitchShift>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set("semitones", 0.0)
                .set("formant", 0.0)
                .set("mix", mix);
            let o = h.run(n).audio("out")[0].clone();
            assert!(
                (o[100 + lat] - 1.0).abs() < 1e-5,
                "mix {mix}: {}",
                o[100 + lat]
            );
            let rest: f32 = o.iter().map(|v| v.abs()).sum::<f32>() - o[100 + lat].abs();
            assert!(rest < 1e-4, "mix {mix}: {rest}");
        }
        // Shifted, the impulse's copies cluster around the same point.
        let mut h = Harness::new::<PitchShift>(json!({}));
        h.set_audio("in", vec![x])
            .set("semitones", 5.0)
            .set("formant", 5.0);
        let o = h.run(n).audio("out")[0].clone();
        let (at, _) =
            o.iter().enumerate().fold(
                (0, 0.0f32),
                |b, (i, v)| if v.abs() > b.1 { (i, v.abs()) } else { b },
            );
        assert!((at as i64 - (100 + lat) as i64).abs() < 1_300, "{at}");
    }

    #[test]
    fn shifts_a_tone_by_the_ratio() {
        let n = 48_000;
        for (st, want) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 440.0 * 1.4983)] {
            let mut h = Harness::new::<PitchShift>(json!({}));
            h.set_audio("in", vec![sine(440.0, n)])
                .set("semitones", st)
                .set("formant", st);
            let o = h.run(n).audio("out")[0].clone();
            let seg = &o[8_000..];
            let shifted = tone_magnitude(seg, want, RATE);
            let original = tone_magnitude(seg, 440.0, RATE);
            assert!(shifted > 0.3, "{st}: {shifted}");
            assert!(original < 0.05 * shifted, "{st}: {original} vs {shifted}");
        }
    }

    /// Energy in harmonics of `f0` within `[lo, hi]` Hz.
    fn band(x: &[f32], f0: f64, lo: f64, hi: f64) -> f64 {
        (1..200)
            .map(|k| k as f64 * f0)
            .filter(|f| *f >= lo && *f <= hi)
            .map(|f| tone_magnitude(x, f, RATE).powi(2))
            .sum()
    }

    #[test]
    fn formants_stay_put_or_move_on_their_own() {
        let n = 48_000;
        let x = vowel(150.0, 1000.0, n);
        let render = |st: f64, fm: f64| {
            let mut h = Harness::new::<PitchShift>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set("semitones", st)
                .set("formant", fm)
                .set("grain", 0.04);
            h.run(n).audio("out")[0][8_000..].to_vec()
        };
        // Energy around the formant (1 kHz) and an octave above it.
        let bands = |o: &[f32], f0: f64| (band(o, f0, 700.0, 1300.0), band(o, f0, 1700.0, 2300.0));
        let input = x[8_000..].to_vec();
        let (low, high) = bands(&input, 150.0);
        assert!(low > 100.0 * high, "{low} {high}");
        // Octave up with the formants moving too: the peak goes to 2 kHz.
        let (low, high) = bands(&render(12.0, 12.0), 300.0);
        assert!(high > 10.0 * low, "{low} {high}");
        // Octave up with the formants kept: the peak stays near 1 kHz, and
        // the level stays close to the input's.
        let kept = render(12.0, 0.0);
        let (low, high) = bands(&kept, 300.0);
        assert!(low > 50.0 * high, "{low} {high}");
        let level = rms(&kept) / rms(&input);
        assert!(level > 0.7 && level < 1.3, "{level}");
        // An octave down, kept too.
        let (low, high) = bands(&render(-12.0, 0.0), 75.0);
        assert!(low > 50.0 * high, "{low} {high}");
        // Same pitch, formants up an octave.
        let (low, high) = bands(&render(0.0, 12.0), 150.0);
        assert!(high > 5.0 * low, "{low} {high}");
    }

    #[test]
    fn stereo_channels_follow_the_input_and_share_heads() {
        let n = 6_000;
        let x = sine(330.0, n);
        let mut stereo = Harness::new::<PitchShift>(json!({}));
        stereo.set_audio("in", vec![x.clone(), vec![0.0; n]]);
        let s = stereo.run(n);
        let mut mono = Harness::new::<PitchShift>(json!({}));
        mono.set_audio("in", vec![x]);
        let m = mono.run(n);
        assert_eq!(s.audio("out").len(), 2);
        assert_eq!(m.audio("out").len(), 1);
        assert_eq!(s.audio("out")[0], m.audio("out")[0]);
        assert_eq!(peak(&s.audio("out")[1]), 0.0);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let n = 8_000;
        let x = noise(n, 3);
        let st: Vec<f32> = (0..n).map(|i| 12.0 * (i as f32 * 0.001).sin()).collect();
        let fm: Vec<f32> = (0..n).map(|i| -6.0 + (i % 1000) as f32 * 0.012).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<PitchShift>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone(), noise(n, 4)])
                .set_samples("semitones", st.clone())
                .set_samples("formant", fm.clone())
                .set("mix", 0.7);
            h.run(n).audio("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn sweeps_and_extreme_values_stay_finite_and_bounded() {
        let n = 24_000;
        let st: Vec<f32> = (0..n).map(|i| 30.0 * (i as f32 * 0.01).sin()).collect();
        let grain: Vec<f32> = (0..n)
            .map(|i| 0.01 + 0.09 * ((i / 300) % 2) as f32)
            .collect();
        let mut h = Harness::new::<PitchShift>(json!({}));
        h.set_audio("in", vec![noise(n, 9)])
            .set_samples("semitones", st)
            .set_samples("grain", grain)
            .set("formant", 3.0);
        let o = h.run(n).audio("out")[0].clone();
        assert!(o.iter().all(|v| v.is_finite()));
        assert!(rms(&o[4_000..]) < 1.0, "{}", rms(&o[4_000..]));

        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 5.0];
        let wild: Vec<f32> = wild.iter().cycle().take(2_000).copied().collect();
        let mut h = Harness::with_resources::<PitchShift>(json!({}), 44_100.0, 1, &NoResources);
        h.set_audio("in", vec![wild.clone(), vec![0.5; 2_000]])
            .set_samples("semitones", wild.iter().rev().copied().collect())
            .set_samples("formant", wild.clone())
            .set_samples("grain", wild.clone())
            .set_samples("mix", wild);
        let out = h.run(2_000);
        for c in out.audio("out") {
            assert!(c.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn reset_clears_everything() {
        let mut h = Harness::new::<PitchShift>(json!({}));
        h.set_audio("in", vec![noise(6_000, 2)]);
        h.run(3_000);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 12_000]]);
        assert_eq!(peak(&h.run(4_000).audio("out")[0]), 0.0);
    }
}
