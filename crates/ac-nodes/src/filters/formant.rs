use ac_core::{
    Config, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::svf::{Svf, SvfCoeffs};

node! {
    kind: "formant",
    name: "Formant",
    category: Filters,
    keywords: ["vowel", "voice", "talk", "talkbox", "speech", "aeiou", "mouth", "wah", "filter bank"],
    /// A vowel filter: three resonances placed where a voice puts them for
    /// "a", "e", "i", "o", and "u", so a bright sound fed through it seems
    /// to speak.
    ///
    /// `vowel` morphs through the five vowels in that order: 0 is "a" (as
    /// in *father*), 1 "e" (*bed*), 2 "i" (*see*), 3 "o" (*law*), 4 "u"
    /// (*food*); in between, the resonances glide from one vowel's to the
    /// next. `voice` picks the measured resonances of a man, a woman, or a
    /// child, and `shift` moves them all up or down in semitones (a
    /// smaller or bigger "mouth"). `resonance` sharpens (above 1) or
    /// widens (below 1) the peaks, and `mix` blends the filtered sound with
    /// the input (1: only the vowel). Everything is read every sample, so
    /// an LFO or an envelope on `vowel` makes it talk.
    ///
    /// The filter works on each channel of `in` (up to 64) separately.
    /// Resonances above about 45 % of the sample rate are held there.
    /// Since only three narrow bands pass (each at most at the input's
    /// level), the vowel is quieter than its input: a 110 Hz sawtooth
    /// comes out 11 to 18 dB down, depending on the vowel, so a `Gain`
    /// after it is usual. NaN or infinite input clears
    /// the filter and gives silence until the input is finite again.
    ///
    /// **Try this:** feed a saw `Oscillator` at 110 Hz into a Formant and
    /// wire a slow `LFO` (0..4) into `vowel` for a choir that sings
    /// "a-e-i-o-u"; or put it after a `Noise` with `vowel` on a `Knob` for
    /// whispered vowels.
    pub struct Formant {
        rate: f64,
        /// Three filters per channel.
        states: Vec<[Svf; 3]>,
    }
}

const IN: usize = 0;
const VOWEL: usize = 1;
const VOICE: usize = 2;
const SHIFT: usize = 3;
const RESONANCE: usize = 4;
const MIX: usize = 5;
const OUT: usize = 0;

const MAX_CHANNELS: usize = 64;

/// Voices, in choice order.
pub const VOICES: &[&str] = &["man", "woman", "child"];

/// Vowels in `vowel` order, as the IPA symbols of the measurements used:
/// ɑ (hod), ɛ (head), i (heed), ɔ (hawed), u (who'd).
const VOWELS: usize = 5;

/// Average first three formant frequencies (Hz) of men, women, and
/// children, from G. E. Peterson and H. L. Barney, "Control methods used
/// in a study of the vowels", J. Acoust. Soc. Am. 24(2), 1952, table II.
const FORMANTS: [[[f64; 3]; VOWELS]; 3] = [
    // Men.
    [
        [730.0, 1090.0, 2440.0],
        [530.0, 1840.0, 2480.0],
        [270.0, 2290.0, 3010.0],
        [570.0, 840.0, 2410.0],
        [300.0, 870.0, 2240.0],
    ],
    // Women.
    [
        [850.0, 1220.0, 2810.0],
        [610.0, 2330.0, 2990.0],
        [310.0, 2790.0, 3310.0],
        [590.0, 920.0, 2710.0],
        [370.0, 950.0, 2670.0],
    ],
    // Children.
    [
        [1030.0, 1370.0, 3170.0],
        [690.0, 2610.0, 3570.0],
        [370.0, 3200.0, 3730.0],
        [680.0, 1060.0, 3180.0],
        [430.0, 1170.0, 3260.0],
    ],
];

/// Bandwidths (Hz) of the three formants, typical of voiced vowels
/// (J. L. Flanagan, "Speech Analysis, Synthesis and Perception", 1972,
/// §3.4: about 50–130 Hz for F1–F3). They widen with `shift`, so a
/// shifted vowel keeps its shape.
const BANDWIDTH: [f64; 3] = [70.0, 100.0, 130.0];

/// Peak gain of each formant: the higher ones a little weaker, roughly the
/// fall of a voice's harmonics, so a flat input comes out vowel-shaped.
const GAIN: [f32; 3] = [1.0, 0.63, 0.4];

/// Q bounds: wide enough to stay a resonance, narrow enough not to ring
/// for seconds.
const MIN_Q: f64 = 0.7;
const MAX_Q: f64 = 60.0;

fn finite_or(v: f32, default: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { default }
}

/// The three formants' (frequency, Q) for a voice, a vowel position
/// (0..4, fractional morphs), a shift in semitones, and a resonance
/// factor, at `rate`. Frequencies glide in pitch (geometrically) between
/// vowels, which is how they're heard.
fn formants(voice: usize, vowel: f64, shift: f64, resonance: f64, rate: f64) -> [(f64, f64); 3] {
    let table = &FORMANTS[voice.min(FORMANTS.len() - 1)];
    let v = vowel.clamp(0.0, (VOWELS - 1) as f64);
    let i = (v.floor() as usize).min(VOWELS - 2);
    let t = v - i as f64;
    let scale = (shift.clamp(-24.0, 24.0) / 12.0).exp2();
    let top = 0.45 * rate;
    let mut out = [(0.0, 0.0); 3];
    for (k, o) in out.iter_mut().enumerate() {
        let (a, b) = (table[i][k], table[i + 1][k]);
        let f = (a.ln() * (1.0 - t) + b.ln() * t).exp() * scale;
        let f = f.clamp(20.0, top.max(20.0));
        let q = (f / (BANDWIDTH[k] * scale) * resonance).clamp(MIN_Q, MAX_Q);
        *o = (f, q);
    }
    out
}

/// Coefficients for frame `i` of the block's inputs.
#[inline]
fn coeffs(
    voice: usize,
    vowel: FloatIn,
    shift: FloatIn,
    res: FloatIn,
    i: usize,
    rate: f64,
) -> [SvfCoeffs; 3] {
    let f = formants(
        voice,
        finite_or(vowel.at(i), 0.0),
        finite_or(shift.at(i), 0.0),
        finite_or(res.at(i), 1.0).clamp(0.25, 4.0),
        rate,
    );
    f.map(|(freq, q)| SvfCoeffs::new(freq, q, rate))
}

impl Node for Formant {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Sound to shape (bright ones work best)."))
            .input(
                SocketSpec::float("vowel", Unit::Generic, 0.0)
                    .range(0.0, 4.0)
                    .doc("0 a, 1 e, 2 i, 3 o, 4 u; fractions morph."),
            )
            .input(SocketSpec::choice("voice", VOICES, "man").doc("Whose vowels."))
            .input(
                SocketSpec::float("shift", Unit::Semitones, 0.0)
                    .range(-12.0, 12.0)
                    .doc("Moves every resonance up or down."),
            )
            .input(
                SocketSpec::float("resonance", Unit::Ratio, 1.0)
                    .range(0.25, 4.0)
                    .log()
                    .doc("Sharper peaks above 1, wider below."),
            )
            .input(
                SocketSpec::float("mix", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("Input (0) to vowel (1)."),
            )
            .output(SocketSpec::audio("out").doc("The vowel-shaped sound."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Formant {
            rate: ctx.sample_rate,
            states: vec![[Svf::default(); 3]; MAX_CHANNELS],
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let vowel = io.inputs.float(VOWEL);
        let voice = io.inputs.choice(VOICE, VOICES.len());
        let shift = io.inputs.float(SHIFT);
        let res = io.inputs.float(RESONANCE);
        let mix = io.inputs.float(MIX);
        let rate = self.rate;
        let varying = vowel.is_varying() || shift.is_varying() || res.is_varying();
        let steady = coeffs(voice, vowel, shift, res, 0, rate);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels().min(MAX_CHANNELS) {
            let src = input.channel(c);
            let dst = out.channel(c);
            let st = &mut self.states[c];
            for (i, (o, &x)) in dst.iter_mut().zip(src).enumerate() {
                let k = if varying {
                    coeffs(voice, vowel, shift, res, i, rate)
                } else {
                    steady
                };
                let mut wet = 0.0f32;
                for b in 0..3 {
                    wet += GAIN[b] * st[b].tick(&k[b], x).bandpass;
                }
                let m = finite_or(mix.at(i), 1.0).clamp(0.0, 1.0) as f32;
                let y = x * (1.0 - m) + wet * m;
                *o = if y.is_finite() { y } else { 0.0 };
            }
            for s in st.iter_mut() {
                s.flush();
            }
        }
    }

    fn reset(&mut self) {
        for st in &mut self.states {
            for s in st.iter_mut() {
                s.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, rms, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn impulse(n: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[0] = 1.0;
        x
    }

    /// The impulse response's level at `f` Hz.
    fn response(vowel: f64, voice: &str, shift: f64, rate: f64, freqs: &[f64]) -> Vec<f64> {
        let n = rate as usize / 2;
        let mut h = Harness::with_resources::<Formant>(json!({}), rate, 256, &NoResources);
        h.set_audio("in", vec![impulse(n)])
            .set("vowel", vowel)
            .set("voice", voice)
            .set("shift", shift);
        let y = h.run(n).audio("out")[0].clone();
        freqs.iter().map(|&f| tone_magnitude(&y, f, rate)).collect()
    }

    #[test]
    fn each_vowel_peaks_at_its_formants() {
        for (v, vowel) in FORMANTS[0].iter().enumerate() {
            // At each formant, and halfway between F1 and F2 (a valley,
            // when they're far apart).
            let mid = (vowel[0] * vowel[1]).sqrt();
            let r = response(
                v as f64,
                "man",
                0.0,
                RATE,
                &[vowel[0], vowel[1], vowel[2], mid],
            );
            assert!(r[0] > r[3] * 2.0, "vowel {v}: {r:?}");
            assert!(r[1] > r[3], "vowel {v}: {r:?}");
            // F3 sits well above the response an octave above it.
            let above = response(v as f64, "man", 0.0, RATE, &[vowel[2] * 1.6])[0];
            assert!(r[2] > above * 2.0, "vowel {v}: {r:?} vs {above}");
        }
    }

    #[test]
    fn a_and_i_differ_where_vowels_should() {
        // "a" has a high F1 and low F2; "i" the reverse.
        let a = response(0.0, "man", 0.0, RATE, &[730.0, 2290.0]);
        let i = response(2.0, "man", 0.0, RATE, &[730.0, 2290.0]);
        assert!(a[0] > i[0] * 3.0, "{a:?} {i:?}");
        assert!(i[1] > a[1] * 3.0, "{a:?} {i:?}");
    }

    #[test]
    fn morphing_glides_between_vowels() {
        // Halfway from "a" (F1 730) to "e" (F1 530) is their geometric mean.
        let f = formants(0, 0.5, 0.0, 1.0, RATE);
        assert!((f[0].0 - (730.0f64 * 530.0).sqrt()).abs() < 1e-6);
        assert_eq!(formants(0, 0.0, 0.0, 1.0, RATE)[1].0, 1090.0);
        assert_eq!(formants(0, 4.0, 0.0, 1.0, RATE)[2].0, 2240.0);
        // Out of range clamps to the ends.
        assert_eq!(
            formants(0, -3.0, 0.0, 1.0, RATE),
            formants(0, 0.0, 0.0, 1.0, RATE)
        );
        assert_eq!(
            formants(0, 9.0, 0.0, 1.0, RATE),
            formants(0, 4.0, 0.0, 1.0, RATE)
        );
    }

    #[test]
    fn shift_and_voice_move_the_formants() {
        let up = formants(0, 1.0, 12.0, 1.0, RATE);
        let base = formants(0, 1.0, 0.0, 1.0, RATE);
        for k in 0..3 {
            assert!((up[k].0 - 2.0 * base[k].0).abs() < 1e-6);
            assert!((up[k].1 - base[k].1).abs() < 1e-9, "Q follows the shift");
        }
        let child = response(0.0, "child", 0.0, RATE, &[1030.0, 730.0]);
        assert!(child[0] > child[1], "{child:?}");
    }

    #[test]
    fn it_is_sample_rate_independent() {
        let f = [730.0, 1090.0, 2440.0, 900.0];
        // The same shape at both rates (the measure divides by the length,
        // which doubles with the rate, so compare levels relative to F1).
        let a = response(0.0, "man", 0.0, 48_000.0, &f);
        let b = response(0.0, "man", 0.0, 96_000.0, &f);
        for (x, y) in a.iter().zip(&b) {
            let db = 20.0 * ((x / a[0]) / (y / b[0])).log10();
            assert!(db.abs() < 1.0, "{a:?} vs {b:?}");
        }
        // Formants above 45 % of a low rate are held below Nyquist.
        let f = formants(2, 2.0, 12.0, 1.0, 8_000.0);
        assert!(f.iter().all(|(freq, _)| *freq <= 3_600.0));
    }

    #[test]
    fn mix_zero_passes_the_input_and_block_size_does_not_matter() {
        let x: Vec<f32> = (0..4000)
            .map(|i| ((i * 37 % 101) as f32 / 50.0) - 1.0)
            .collect();
        let mut h = Harness::new::<Formant>(json!({}));
        h.set_audio("in", vec![x.clone(), x.clone()])
            .set("mix", 0.0);
        let out = h.run(4000);
        assert_eq!(out.audio("out")[0], x);
        assert_eq!(out.audio("out")[1], x);
        let vowel: Vec<f32> = (0..4000).map(|i| 4.0 * i as f32 / 4000.0).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Formant>(json!({}), RATE, block, &NoResources);
            h.set_audio("in", vec![x.clone()])
                .set_samples("vowel", vowel.clone());
            h.run(4000).audio("out")[0].clone()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1024));
        assert!(rms(&one) > 0.01);
    }

    #[test]
    fn a_saw_comes_out_11_to_18_db_quieter() {
        let saw: Vec<f32> = (0..48_000)
            .map(|i| 2.0 * ((i as f64 * 110.0 / RATE).fract() as f32) - 1.0)
            .collect();
        for vowel in 0..5 {
            let mut h = Harness::new::<Formant>(json!({}));
            h.set_audio("in", vec![saw.clone()])
                .set("vowel", vowel as f64);
            let y = h.run(48_000).audio("out")[0].clone();
            let loss = 20.0 * (rms(&saw[4800..]) / rms(&y[4800..])).log10();
            assert!((10.5..18.5).contains(&loss), "vowel {vowel}: {loss} dB");
        }
    }

    #[test]
    fn bad_input_and_extreme_settings_stay_finite() {
        let mut x = vec![0.5f32; 2000];
        x[100] = f32::NAN;
        x[101] = f32::INFINITY;
        for (vowel, shift, res) in [(f64::NAN, 1e9, 0.0), (1e9, -1e9, 1e9), (2.0, 0.0, 4.0)] {
            let mut h = Harness::new::<Formant>(json!({}));
            h.set_audio("in", vec![x.clone()])
                .set("vowel", vowel)
                .set("shift", shift)
                .set("resonance", res);
            let y = h.run(2000).audio("out")[0].clone();
            assert!(y.iter().all(|v| v.is_finite()));
        }
        // Reset clears the ringing.
        let mut h = Harness::new::<Formant>(json!({}));
        h.set_audio("in", vec![impulse(64)]);
        h.run(64);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 64]]);
        assert!(h.run(64).audio("out")[0].iter().all(|v| *v == 0.0));
    }
}
