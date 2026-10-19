use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    WholeCtx, WholeInputs, WholeOutputs, WholeValue, node,
};
use ac_dsp::fft::{Fft, Window};
use ac_dsp::pvoc::wrap_phase;
use ac_dsp::stft::pow2_size;

node! {
    kind: "time_stretch",
    name: "Time Stretch",
    category: Time,
    keywords: ["stretch", "slow down", "speed up", "tempo", "phase vocoder", "timescale", "whole input"],
    /// Plays its input slower or faster without changing its pitch.
    ///
    /// `stretch` is how many times longer the sound lasts: 2 plays it at
    /// half speed (twice as long), 0.5 at double speed, 1 leaves it as it
    /// is; anything from 0.25 to 4. It can change over time: the sound
    /// moves through the input at `1 / stretch` of real time at each
    /// moment, so a rising `stretch` slows the sound down as it goes. The
    /// output runs as long as the project; a stretched sound longer than
    /// that is cut there, and after a shortened one comes silence.
    ///
    /// Like `Reverse`, it needs its whole input before it can play, so it
    /// doesn't run live: when everything feeding it is known ahead of time
    /// (a `Clip`, an `Oscillator`, a `Pattern`) the engine **bakes** it in
    /// the background and plays the result in time with the transport, and
    /// when a live source such as `Audio In` feeds it, it's **disabled**
    /// and silent. Offline renders always work.
    ///
    /// It's a phase vocoder: the input is cut into overlapping frames
    /// (`size` samples, config, a power of two from 256 to 16384, default
    /// 2048), each frame's partials keep their pitch while the frames are
    /// spread further apart or closer together, and the frames are added
    /// back up. Bigger frames keep tones and chords smoother; smaller ones
    /// keep drums and consonants sharper (a phase vocoder always softens
    /// attacks a little). Each channel is stretched on its own. NaN or
    /// infinite input samples count as silence.
    ///
    /// **Try this:** wire a drum or vocal `Clip` through a Time Stretch
    /// with `stretch` at 4 for a slow, smeared ambience; or drive `stretch`
    /// from a `Graph` that ramps from 1 to 3 for a tape-stop that keeps its
    /// pitch.
    pub struct TimeStretch {}
}

const IN: usize = 0;
const STRETCH: usize = 1;
const OUT: usize = 0;

/// The range `stretch` is held to.
const MIN_STRETCH: f64 = 0.25;
const MAX_STRETCH: f64 = 4.0;

fn frame_size(config: &Config) -> usize {
    pow2_size(config.get_f64_or("size", 2048.0), 256, 16_384, 2048)
}

/// Stretch one channel `x` over `frames` output samples, `stretch[t]` at
/// each output sample, with frames of `n` samples and a synthesis hop of
/// `n / 4`.
///
/// The phase vocoder of M. Dolson, "The phase vocoder: a tutorial"
/// (Computer Music Journal 10(4), 1986), with the identity phase locking
/// of J. Laroche and M. Dolson, "Improved phase vocoder time-scale
/// modification of audio" (IEEE Trans. Speech Audio Proc. 7(3), 1999):
/// each output frame reads the input at the position its center maps to,
/// measures every bin's frequency from that frame and the one a synthesis
/// hop before it, advances each spectral peak's phase by that much, and
/// keeps every other bin's phase where it stands relative to its peak, so
/// a partial's lobe stays coherent. Frames are added back up with the
/// least-squares weights of D. Griffin and J. Lim (IEEE Trans. ASSP 32(2),
/// 1984). At a stretch of 1 the output is the input.
fn stretch_channel(x: &[f32], frames: usize, stretch: &[f32], n: usize) -> Vec<f32> {
    let hop = n / 4;
    let bins = n / 2 + 1;
    let window = Window::Hann.table(n);
    let mut fft = Fft::new(n);
    let at = |t: f64| -> f64 {
        let i = (t.max(0.0) as usize).min(stretch.len().saturating_sub(1));
        match stretch.get(i) {
            Some(v) if v.is_finite() => f64::from(*v).clamp(MIN_STRETCH, MAX_STRETCH),
            _ => 1.0,
        }
    };
    let clean = |k: i64| -> f32 {
        if k < 0 {
            return 0.0;
        }
        match x.get(k as usize) {
            Some(v) if v.is_finite() => *v,
            _ => 0.0,
        }
    };
    let mut frame = vec![0.0f32; n];
    let mut cur = vec![0.0f32; 2 * bins];
    let mut prev = vec![0.0f32; 2 * bins];
    let mut mag = vec![0.0f32; bins];
    let mut phase = vec![0.0f32; bins];
    let mut psi = vec![0.0f32; bins];
    let mut out_bins = vec![0.0f32; 2 * bins];
    let mut peaks: Vec<usize> = Vec::with_capacity(bins);
    let mut acc = vec![0.0f64; frames];
    let mut norm = vec![0.0f64; frames];
    let expected = std::f64::consts::TAU * hop as f64 / n as f64;
    let half = (n / 2) as f64;
    // Start early enough that the first output samples are fully covered.
    let first = -((n / hop) as i64 - 1);
    let mut center_in = 0.0f64;
    for m in first.. {
        let o = m * hop as i64;
        if o >= frames as i64 {
            break;
        }
        let center_out = o as f64 + half;
        let s = at(center_out);
        center_in = if m == first {
            center_out / s
        } else {
            center_in + hop as f64 / s
        };
        let a = (center_in - half).round() as i64;
        for (back, dst) in [(0, &mut cur), (hop as i64, &mut prev)] {
            for (j, f) in frame.iter_mut().enumerate() {
                *f = clean(a - back + j as i64) * window[j];
            }
            fft.forward(&frame, dst);
        }
        for k in 0..bins {
            let (re, im) = (cur[2 * k], cur[2 * k + 1]);
            mag[k] = re.hypot(im);
            phase[k] = im.atan2(re);
        }
        if m == first {
            psi.copy_from_slice(&phase);
        } else {
            // Peaks: local maxima of the level.
            peaks.clear();
            for k in 1..bins - 1 {
                if mag[k] > mag[k - 1] && mag[k] >= mag[k + 1] {
                    peaks.push(k);
                }
            }
            let mut start = 0;
            for (i, &p) in peaks.iter().enumerate() {
                let end = match peaks.get(i + 1) {
                    Some(&q) => (p..q)
                        .min_by(|&u, &v| mag[u].total_cmp(&mag[v]))
                        .unwrap_or(p),
                    None => bins - 1,
                };
                let (pr, pi) = (prev[2 * p], prev[2 * p + 1]);
                let centre = expected * p as f64;
                let turned = f64::from(phase[p] - pi.atan2(pr));
                let step = centre + f64::from(wrap_phase((turned - centre) as f32));
                let peak = wrap_phase((f64::from(psi[p]) + step) as f32);
                for k in start..=end {
                    psi[k] = if k == p {
                        peak
                    } else {
                        wrap_phase(peak + phase[k] - phase[p])
                    };
                }
                start = end + 1;
            }
            if peaks.is_empty() {
                // Nothing tonal (silence): keep the measured phases.
                psi.copy_from_slice(&phase);
            }
        }
        for k in 0..bins {
            let (sn, cs) = psi[k].sin_cos();
            out_bins[2 * k] = mag[k] * cs;
            out_bins[2 * k + 1] = mag[k] * sn;
        }
        fft.inverse(&out_bins, &mut frame);
        for (j, (&y, &w)) in frame.iter().zip(&window).enumerate() {
            let t = o + j as i64;
            if t < 0 {
                continue;
            }
            let Some(slot) = acc.get_mut(t as usize) else {
                break;
            };
            *slot += f64::from(y) * f64::from(w);
            norm[t as usize] += f64::from(w) * f64::from(w);
        }
    }
    acc.iter()
        .zip(&norm)
        .map(|(&v, &w)| if w > 1e-6 { (v / w) as f32 } else { 0.0 })
        .collect()
}

impl Node for TimeStretch {
    fn meta() -> NodeMeta {
        Self::base_meta().whole_input()
    }

    fn describe(config: &Config) -> Signature {
        let _ = frame_size(config);
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to stretch."))
            .input(
                SocketSpec::float("stretch", Unit::Ratio, 1.0)
                    .range(MIN_STRETCH, MAX_STRETCH)
                    .log()
                    .doc("How many times longer it lasts (2: half speed)."),
            )
            .output(SocketSpec::audio("out").doc("The input, slower or faster, at its pitch."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TimeStretch {}
    }

    // The engine streams the bake; called directly (outside an engine) the
    // node is silent.
    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn causality(&self) -> ac_core::Causality {
        ac_core::Causality::WholeInput
    }

    fn render_whole(config: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs {
        let n = frame_size(config);
        let stretch = inputs.float(STRETCH);
        let chans: Vec<Vec<f32>> = inputs
            .audio(IN)
            .iter()
            .map(|c| stretch_channel(c, ctx.frames, stretch, n))
            .collect();
        let mut out = WholeOutputs::with_outputs(1);
        out.set(OUT, WholeValue::Audio(chans));
        out
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::{Harness, rms, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(f: f64, amp: f64, n: usize, rate: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * f * i as f64 / rate).sin()) as f32)
            .collect()
    }

    fn stretched(
        config: serde_json::Value,
        rate: f64,
        x: Vec<Vec<f32>>,
        stretch: WholeValue,
    ) -> Vec<Vec<f32>> {
        let out = Harness::render_whole::<TimeStretch>(
            &config,
            rate,
            vec![WholeValue::Audio(x), stretch],
        );
        match out.get(OUT) {
            WholeValue::Audio(c) => c.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stretch_of_one_gives_the_input_back() {
        let n = 24_000;
        let x: Vec<f32> = sine(440.0, 0.4, n, RATE)
            .iter()
            .zip(sine(1234.5, 0.2, n, RATE))
            .map(|(a, b)| a + b)
            .collect();
        let y = stretched(
            json!({}),
            RATE,
            vec![x.clone()],
            WholeValue::Float(vec![1.0; n]),
        );
        let err: Vec<f32> = x.iter().zip(&y[0]).map(|(a, b)| a - b).collect();
        let db = 20.0 * (rms(&err) / rms(&x)).log10();
        assert!(db < -60.0, "{db} dB");
        assert_eq!(y[0].len(), n);
    }

    #[test]
    fn stretching_changes_the_length_and_keeps_the_pitch() {
        // Half a second of 440 Hz in a two-second render.
        let total = 96_000;
        let mut x = sine(440.0, 0.5, 24_000, RATE);
        x.resize(total, 0.0);
        for (s, end) in [(2.0, 48_000usize), (0.5, 12_000), (3.0, 72_000)] {
            let y = &stretched(
                json!({}),
                RATE,
                vec![x.clone()],
                WholeValue::Float(vec![s; total]),
            )[0];
            // It sounds until `end`, then stops (within a frame).
            let during = &y[end / 4..end - 2_048];
            assert!(rms(&y[end + 2_048..]) < 0.01, "{s}: rings on");
            assert!(rms(during) > 0.3, "{s}: {}", rms(during));
            // At its pitch, at its level.
            let m = tone_magnitude(during, 440.0, RATE);
            assert!((m - 0.5).abs() < 0.06, "{s}: {m}");
            assert!(
                tone_magnitude(during, 440.0 * f64::from(s), RATE) < 0.05,
                "{s}"
            );
        }
    }

    #[test]
    fn a_changing_stretch_moves_through_the_input_at_its_rate() {
        // A tone that steps from 300 Hz to 900 Hz at 0.25 s; stretch 1 for
        // the first half second, then 2: the step lands at 0.25 s still.
        let n = 48_000;
        let mut x = sine(300.0, 0.5, 12_000, RATE);
        x.extend(sine(900.0, 0.5, n - 12_000, RATE));
        let s: Vec<f32> = (0..n).map(|i| if i < 24_000 { 1.0 } else { 2.0 }).collect();
        let y = &stretched(json!({}), RATE, vec![x.clone()], WholeValue::Float(s))[0];
        assert!(tone_magnitude(&y[4_000..10_000], 300.0, RATE) > 0.4);
        assert!(tone_magnitude(&y[14_000..22_000], 900.0, RATE) > 0.4);
        // Later, at half speed, it's still 900 Hz.
        assert!(tone_magnitude(&y[30_000..46_000], 900.0, RATE) > 0.4);
        // A constant stretch of 2 instead puts the step at 0.5 s.
        let y2 = &stretched(json!({}), RATE, vec![x], WholeValue::Float(vec![2.0; n]))[0];
        assert!(tone_magnitude(&y2[14_000..20_000], 300.0, RATE) > 0.4);
    }

    #[test]
    fn it_is_sample_rate_independent_and_stretches_each_channel() {
        let rate = 44_100.0;
        let n = 44_100;
        let mut l = sine(500.0, 0.5, n / 2, rate);
        l.resize(n, 0.0);
        let mut r = sine(700.0, 0.5, n / 2, rate);
        r.resize(n, 0.0);
        let y = stretched(
            json!({ "size": 4096 }),
            rate,
            vec![l, r],
            WholeValue::Float(vec![1.5; n]),
        );
        assert_eq!(y.len(), 2);
        let (a, b) = (&y[0][5_000..30_000], &y[1][5_000..30_000]);
        assert!(tone_magnitude(a, 500.0, rate) > 0.4);
        assert!(tone_magnitude(b, 700.0, rate) > 0.4);
        assert!(tone_magnitude(a, 700.0, rate) < 0.02);
    }

    #[test]
    fn nothing_in_and_bad_values_are_handled() {
        // Unwired: nothing to bake.
        let y = stretched(json!({}), RATE, vec![], WholeValue::None);
        assert!(y.is_empty());
        // An unwired stretch counts as 1; NaN samples as silence; absurd
        // stretches are held to 0.25..4; bad sizes are repaired.
        let mut x = sine(440.0, 0.5, 9_600, RATE);
        x[100] = f32::NAN;
        x[200] = f32::INFINITY;
        for (config, stretch) in [
            (json!({}), WholeValue::None),
            (
                json!({ "size": 3 }),
                WholeValue::Float(vec![f32::NAN; 9_600]),
            ),
            (
                json!({ "size": "big" }),
                WholeValue::Float(vec![1e9; 9_600]),
            ),
            (json!({ "size": 1e9 }), WholeValue::Float(vec![-5.0; 9_600])),
        ] {
            let y = stretched(config.clone(), RATE, vec![x.clone()], stretch);
            assert_eq!(y[0].len(), 9_600, "{config}");
            assert!(y[0].iter().all(|v| v.is_finite()), "{config}");
        }
        assert_eq!(frame_size(&json!({ "size": 3 })), 256);
        assert_eq!(frame_size(&json!({ "size": 5000 })), 4096);
    }

    #[test]
    fn it_is_a_whole_input_node_and_silent_when_run_directly() {
        assert!(TimeStretch::meta().whole_input);
        let mut h = Harness::new::<TimeStretch>(json!({}));
        h.set_audio("in", vec![vec![0.5; 128]]);
        assert!(h.run(128).audio("out")[0].iter().all(|v| *v == 0.0));
    }
}
