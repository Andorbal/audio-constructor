use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::fft::{Fft, Window};
use ac_dsp::onset::{PeakPicker, SpectralFlux, locate};
use ac_dsp::stft::{StftAnalysis, pow2_size};

node! {
    kind: "onset",
    name: "Onset",
    category: Triggers,
    keywords: ["transient", "attack", "beat", "hit", "drum", "detector", "spectral flux"],
    /// Fires a trigger at the start of each new sound: drum hits, plucked
    /// notes, consonants. A transient detector based on spectral flux.
    ///
    /// The first channel of `in` is analyzed in short overlapping frames
    /// (about 21 ms, a quarter of that apart). For each frame the node adds
    /// up how much louder every frequency got since the previous frame, on
    /// a logarithmic scale, so a new sound counts wherever it lands in the
    /// spectrum while fading sounds don't. A frame whose rise is a local
    /// peak and stands well out from the average of the last 40 ms or so is
    /// an onset, so steady noise and held notes don't fire.
    /// The trigger is then placed at the sample where the sound's energy
    /// jumps, not just at the frame.
    ///
    /// - `sensitivity` (0..1) sets how far a rise must stand out: higher
    ///   finds softer onsets (and more false ones). 0.5 suits drums and
    ///   plucked instruments; near 1, even busy noise fires.
    /// - `min_interval` (seconds) is the shortest gap between triggers;
    ///   onsets closer than that to the previous one are ignored.
    /// - `strength` is how strong the latest onset was, 0..1, held until
    ///   the next one: handy as a velocity.
    ///
    /// Finding an onset needs a little of what follows it, so triggers come
    /// out a fixed `frame / 2 + hop` samples after the onset (768 samples,
    /// 16 ms, at 44.1 or 48 kHz). The node reports that as its latency, so
    /// the engine delays a parallel dry path by the same amount and the
    /// triggers line up with the audio. `sensitivity` and `min_interval`
    /// are read at each frame. NaN or infinite samples count as silence.
    ///
    /// **Try this:** wire a drum loop into Onset and `trigger` into an
    /// `ADSR` gating a noise burst or a sampled clap to layer a sound on
    /// every hit; use `strength` to scale its level.
    pub struct Onset {
        analysis: StftAnalysis,
        fft: Fft,
        bins: Vec<f32>,
        flux: SpectralFlux,
        picker: PeakPicker,
        /// Magnitude scale: a full-scale sine reads 1.
        scale: f32,
        /// Scheduled triggers: (absolute sample, strength), oldest first.
        pending: [(u64, f32); QUEUE],
        head: usize,
        len: usize,
        /// Samples processed since the last reset.
        clock: u64,
        last_onset: Option<u64>,
        strength: f32,
        latency: u32,
    }
}

const IN: usize = 0;
const SENSITIVITY: usize = 1;
const MIN_INTERVAL: usize = 2;

const TRIGGER: usize = 0;
const STRENGTH: usize = 1;

/// Triggers that can wait for their time at once.
const QUEUE: usize = 32;
/// Frame length in seconds (about 1024 samples at 48 kHz).
const FRAME_SECONDS: f64 = 0.0213;

/// The peak picker's threshold for a sensitivity, as (ratio, offset) over
/// the recent mean flux: (2, 0.5) at sensitivity 0, (1.5, 0.05) at 0.5, and
/// (1, 0.005) at full sensitivity. The ratio rejects the frame-to-frame
/// wobble of steady noise, whose flux has a large mean; the offset rejects
/// near-silence.
fn threshold(sensitivity: f32) -> (f32, f32) {
    let s = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.5
    };
    (2.0 - s, 0.5 * 10f32.powf(-2.0 * s))
}

impl Onset {
    fn schedule(&mut self, at: u64, strength: f32) {
        if self.len < QUEUE {
            self.pending[(self.head + self.len) % QUEUE] = (at, strength);
            self.len += 1;
        }
    }

    /// The analysis frame that just completed at absolute sample `now`.
    fn frame(&mut self, now: u64, sensitivity: f32, min_interval: f32, rate: f64) {
        self.analysis.analyze(&mut self.fft, &mut self.bins);
        let f = self.flux.process(&self.bins, self.scale);
        let (ratio, offset) = threshold(sensitivity);
        let Some(peak) = self.picker.push(f, ratio, offset) else {
            return;
        };
        // The peak was the previous frame, which ended one hop ago. Look
        // for the jump in the newest half of that frame.
        let size = self.analysis.size();
        let hop = self.analysis.hop();
        let half = size / 2;
        let region = self
            .analysis
            .latest()
            .get(size - hop - half..size - hop)
            .unwrap_or(&[]);
        let start = now as i64 - (hop + half) as i64 + 1;
        let at = (start + locate(region) as i64).max(0) as u64;
        let gap = f64::from(if min_interval.is_finite() {
            min_interval.max(0.0)
        } else {
            0.0
        });
        let min_gap = (gap * rate).round() as u64;
        let clear = self
            .last_onset
            .is_none_or(|last| at > last && at - last >= min_gap);
        if clear {
            self.last_onset = Some(at);
            self.schedule(at + u64::from(self.latency), 1.0 - (-peak).exp());
        }
    }
}

impl Node for Onset {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to listen to (first channel)."))
            .input(
                SocketSpec::float("sensitivity", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Higher finds softer onsets."),
            )
            .input(
                SocketSpec::float("min_interval", Unit::Seconds, 0.05)
                    .range(0.0, 1.0)
                    .doc("Shortest time between triggers."),
            )
            .output(SocketSpec::trigger("trigger").doc("Fires at each onset."))
            .output(
                SocketSpec::float("strength", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("How strong the latest onset was, 0..1."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let size = pow2_size(FRAME_SECONDS * ctx.sample_rate, 256, 4096, 1024);
        let hop = size / 4;
        let fft = Fft::new(size);
        let bins = fft.bins();
        Onset {
            analysis: StftAnalysis::new(size, hop, Window::Hann),
            fft,
            bins: vec![0.0; 2 * bins],
            flux: SpectralFlux::new(bins),
            picker: PeakPicker::new(),
            scale: 4.0 / size as f32,
            pending: [(0, 0.0); QUEUE],
            head: 0,
            len: 0,
            clock: 0,
            last_onset: None,
            strength: 0.0,
            latency: (size / 2 + hop) as u32,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let x = input.channel(0);
        let sensitivity = io.inputs.float(SENSITIVITY);
        let min_interval = io.inputs.float(MIN_INTERVAL);
        let (mut strength, mut trigger) = io.outputs.float_trigger(STRENGTH, TRIGGER);
        for (i, &s) in x.iter().enumerate() {
            let now = self.clock + i as u64;
            if self.analysis.push(s) {
                self.frame(now, sensitivity.at(i), min_interval.at(i), ctx.sample_rate);
            }
            while self.len > 0 && self.pending[self.head].0 <= now {
                self.strength = self.pending[self.head].1;
                trigger.push(i as u32);
                self.head = (self.head + 1) % QUEUE;
                self.len -= 1;
            }
        }
        strength.fill(self.strength);
        self.clock += x.len() as u64;
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn reset(&mut self) {
        self.analysis.reset();
        self.flux.reset();
        self.picker.reset();
        self.head = 0;
        self.len = 0;
        self.clock = 0;
        self.last_onset = None;
        self.strength = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, Output};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect()
    }

    /// A decaying noise burst (a drum-like hit) of `amp` at `at`.
    fn add_burst(x: &mut [f32], at: usize, amp: f32, seed: u32) {
        for (i, n) in noise(4800, seed).into_iter().enumerate() {
            if let Some(v) = x.get_mut(at + i) {
                *v += amp * n * (-(i as f32) / 600.0).exp();
            }
        }
    }

    fn detect(x: &[f32], set: &[(&str, f64)], block: usize) -> Output {
        let mut h = Harness::with_resources::<Onset>(json!({}), RATE, block, &NoResources);
        h.set_audio("in", vec![x.to_vec()]);
        for (k, v) in set {
            h.set(k, *v);
        }
        h.run(x.len() + 2048)
    }

    const LATENCY: u64 = 768;

    #[test]
    fn clicks_in_silence_fire_once_each_on_time() {
        let mut x = vec![0.0; 48_000];
        let at = [1000usize, 7777, 15_000, 24_001, 40_000];
        for &a in &at {
            x[a] = 0.8;
        }
        let out = detect(&x, &[], 64);
        let t = out.triggers("trigger");
        assert_eq!(t.len(), at.len(), "{t:?}");
        for (&got, &a) in t.iter().zip(&at) {
            assert_eq!(got, a as u64 + LATENCY, "click at {a}");
        }
    }

    #[test]
    fn drum_like_bursts_fire_once_each_near_their_start() {
        let mut x = vec![0.0; 72_000];
        let at = [2000usize, 14_000, 26_500, 39_000, 51_234, 63_000];
        for (k, &a) in at.iter().enumerate() {
            add_burst(&mut x, a, 0.3 + 0.1 * k as f32, k as u32 + 3);
        }
        let out = detect(&x, &[], 64);
        let t = out.triggers("trigger");
        assert_eq!(t.len(), at.len(), "{t:?}");
        for (&got, &a) in t.iter().zip(&at) {
            let err = got as i64 - (a as u64 + LATENCY) as i64;
            assert!(err.abs() <= 16, "burst at {a}: trigger {got}");
        }
        let s = out.float("strength");
        assert!(s.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(*s.last().unwrap() > 0.0);
    }

    /// Triggers before the input ends (its abrupt end is a click too).
    fn during(x: &[f32]) -> Vec<u64> {
        let out = detect(x, &[], 64);
        let end = x.len() as u64;
        out.triggers("trigger")
            .iter()
            .copied()
            .filter(|t| *t < end)
            .collect()
    }

    #[test]
    fn steady_sounds_and_silence_do_not_fire() {
        assert!(during(&vec![0.0; 24_000]).is_empty());
        // A tone or hiss starting is one onset; holding it is none.
        let tone: Vec<f32> = (0..96_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        assert_eq!(during(&tone).len(), 1);
        for seed in [9, 10, 11] {
            let hiss: Vec<f32> = noise(96_000, seed).iter().map(|v| v * 0.1).collect();
            assert_eq!(during(&hiss), vec![LATENCY], "seed {seed}");
        }
    }

    #[test]
    fn sensitivity_and_min_interval() {
        // A loud hit and a soft one.
        let mut x = vec![0.0; 24_000];
        add_burst(&mut x, 2000, 0.8, 1);
        add_burst(&mut x, 12_000, 0.002, 2);
        assert_eq!(
            detect(&x, &[("sensitivity", 1.0)], 64)
                .triggers("trigger")
                .len(),
            2
        );
        assert_eq!(
            detect(&x, &[("sensitivity", 0.0)], 64)
                .triggers("trigger")
                .len(),
            1
        );
        // Two clicks 20 ms apart: one trigger with a 50 ms minimum.
        let mut y = vec![0.0; 24_000];
        y[3000] = 1.0;
        y[3960] = 1.0;
        assert_eq!(detect(&y, &[], 64).triggers("trigger").len(), 1);
        assert_eq!(
            detect(&y, &[("min_interval", 0.0)], 64)
                .triggers("trigger")
                .len(),
            2
        );
    }

    #[test]
    fn block_size_does_not_change_the_triggers() {
        let mut x = vec![0.0; 30_000];
        for (k, a) in [1500usize, 9000, 17_321, 25_000].into_iter().enumerate() {
            add_burst(&mut x, a, 0.5, k as u32 + 11);
        }
        let one = detect(&x, &[], 1);
        assert_eq!(one.triggers("trigger").len(), 4);
        for block in [64, 512] {
            let other = detect(&x, &[], block);
            assert_eq!(one.triggers("trigger"), other.triggers("trigger"));
        }
    }

    #[test]
    fn latency_scales_with_the_sample_rate() {
        let ctx = |rate: f64| PrepareCtx {
            sample_rate: rate,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "t",
            resources: &NoResources,
        };
        assert_eq!(Onset::new(&json!({}), &ctx(48_000.0)).latency(), 768);
        assert_eq!(Onset::new(&json!({}), &ctx(44_100.0)).latency(), 768);
        assert_eq!(Onset::new(&json!({}), &ctx(96_000.0)).latency(), 1536);
    }

    #[test]
    fn nan_input_and_reset() {
        let mut x = vec![0.0; 12_000];
        x[2000] = f32::NAN;
        x[2001] = f32::INFINITY;
        x[6000] = 0.9;
        let mut h = Harness::new::<Onset>(json!({}));
        h.set_audio("in", vec![x]);
        let out = h.run(8000);
        assert_eq!(out.triggers("trigger"), &[6000 + LATENCY]);
        assert!(out.float("strength").iter().all(|v| v.is_finite()));
        // A reset drops pending triggers and the held strength.
        let mut h = Harness::new::<Onset>(json!({}));
        let mut y = vec![0.0; 4000];
        y[1000] = 0.9;
        h.set_audio("in", vec![y]);
        h.run(1500);
        h.reset();
        let out = h.run(2000);
        assert!(out.triggers("trigger").is_empty());
        assert_eq!(out.float("strength")[0], 0.0);
    }
}
