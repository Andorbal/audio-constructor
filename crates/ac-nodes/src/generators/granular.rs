use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx,
    Signature, SocketSpec, Unit, node,
};
use ac_dsp::interp::hermite4;
use ac_dsp::rng::Rng;
use ac_dsp::util::db_to_gain;

use super::clip::prepare;
use crate::segments::Runs;

node! {
    kind: "granular",
    name: "Granular",
    category: Generators,
    keywords: ["grain", "granular synthesis", "cloud", "texture", "microsound", "scatter", "buffer", "freeze", "pad"],
    /// A cloud of grains: tiny, overlapping slices of a recorded sound,
    /// each a few milliseconds to a second long, played over and over from
    /// around one spot. A plain recording turns into a texture, a pad, or
    /// a shimmer, and it can stand still in time.
    ///
    /// **Source.** A wired `buffer` (a `Record` node's last take, say) is
    /// played; with `buffer` unwired, the `file` config (an audio file in
    /// the project) is used, converted to the engine's rate when the
    /// project loads. Nothing to play gives silence; a missing file is
    /// flagged in the editor. Each grain keeps the sound it started with,
    /// so a new buffer comes in grain by grain.
    ///
    /// **Grains.** About `density` grains start each second, at random
    /// moments around that rate. Each lasts `size` seconds under a smooth
    /// (Hann) fade in and out, and reads the sound from `position` (0 to
    /// 1, a fraction of the whole sound), moved by a random amount of up
    /// to `spray` seconds either way so the grains don't all repeat the
    /// same slice. `pitch` (semitones) sets how fast each grain plays, so
    /// the cloud can be transposed while `position` holds still. Each grain
    /// sits somewhere in the stereo field, randomly within `spread` (0: all
    /// in the middle, 1: anywhere from left to right). Every input is read
    /// when a grain starts, so the cloud follows modulation grain by grain.
    /// As grains overlap more, each is turned down (by the square root of
    /// how many overlap), so the cloud stays about as loud as one grain;
    /// `gain` is in decibels. At most 128 grains sound at once.
    ///
    /// **Generator contract.** `start` begins the cloud, `stop` lets the
    /// grains that are playing finish and starts no more (`end` fires),
    /// and `reset` starts the random choices over, so a cloud repeats
    /// exactly. The randomness comes from the project's seed, this node's
    /// place, and the `seed` config, so renders repeat; output doesn't
    /// depend on the block size.
    ///
    /// **Try this:** record a phrase into a `Record` node and wire its
    /// `buffer` here with `size` at 0.08, `density` 30, and `spray` 0.05;
    /// sweep `position` slowly with an `LFO` to drift through the phrase,
    /// or hold it still and play `pitch` from a `Keyboard` for a frozen,
    /// tuned texture.
    pub struct Granular {
        file: Option<Arc<AudioData>>,
        missing: bool,
        rate: f64,
        seed: u64,
        rng: Rng,
        grains: Vec<Grain>,
        playing: bool,
        /// Samples until the next grain starts.
        wait: f64,
    }
}

const BUFFER: usize = 0;
const POSITION: usize = 1;
const SIZE: usize = 2;
const DENSITY: usize = 3;
const SPRAY: usize = 4;
const PITCH: usize = 5;
const SPREAD: usize = 6;
const GAIN: usize = 7;
const START: usize = 8;
const STOP: usize = 9;
const RESET: usize = 10;

const OUT: usize = 0;
const PLAYING: usize = 1;
const END: usize = 2;

/// Telemetry slot: 1.0 when the `file` config couldn't be loaded.
pub const TELEMETRY_MISSING: usize = 0;

/// Most grains sounding at once.
pub const MAX_GRAINS: usize = 128;
/// Bounds on the inputs, so absurd values stay cheap and finite.
const MAX_DENSITY: f64 = 1000.0;
const MIN_SIZE: f64 = 0.001;
const MAX_SIZE: f64 = 10.0;
const MAX_SPRAY: f64 = 60.0;
const MAX_SEMITONES: f64 = 48.0;
const MAX_GAIN_DB: f32 = 24.0;

/// One grain: where it reads, how fast, how long, and where it sits.
#[derive(Clone, Debug, Default)]
struct Grain {
    src: Option<Arc<AudioData>>,
    /// Read position in frames of `src`, and the step per output sample.
    pos: f64,
    step: f64,
    /// Samples played and the grain's length in samples.
    age: f64,
    len: f64,
    /// Left and right gains (pan and overlap).
    gl: f32,
    gr: f32,
}

impl Grain {
    fn active(&self) -> bool {
        self.len > 0.0 && self.age < self.len
    }
}

/// The grain envelope at `t` (0..1): a raised cosine (Hann), as in C.
/// Roads, "Microsound" (MIT Press, 2001), §3.
#[inline]
fn envelope(t: f64) -> f32 {
    if (0.0..=1.0).contains(&t) {
        (0.5 - 0.5 * (std::f64::consts::TAU * t).cos()) as f32
    } else {
        0.0
    }
}

/// Sample `k` of `ch`, silent outside it.
#[inline]
fn sample(ch: &[f32], k: isize) -> f32 {
    if k < 0 {
        return 0.0;
    }
    ch.get(k as usize).copied().unwrap_or(0.0)
}

/// `ch` at fractional frame `p` (cubic Hermite).
#[inline]
fn read(ch: &[f32], p: f64) -> f32 {
    let whole = p.floor();
    let t = (p - whole) as f32;
    let k = whole as isize;
    hermite4(
        sample(ch, k - 1),
        sample(ch, k),
        sample(ch, k + 1),
        sample(ch, k + 2),
        t,
    )
}

fn finite_or(v: f32, default: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { default }
}

/// The node's seed: the project/path seed mixed with the `seed` config.
fn mixed_seed(config: &Config, seed: u64) -> u64 {
    let mut s = config.get_u64_or("seed", 0);
    seed ^ ac_core::util::splitmix64(&mut s)
}

impl Granular {
    /// Start a grain at frame `i` of the block, if there's a free slot and
    /// something to play.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        src: Option<&Arc<AudioData>>,
        i: usize,
        position: FloatIn,
        size: FloatIn,
        density: FloatIn,
        spray: FloatIn,
        pitch: FloatIn,
        spread: FloatIn,
    ) {
        let Some(src) = src else {
            return;
        };
        let frames = src.frames() as f64;
        if frames <= 0.0 {
            return;
        }
        let Some(slot) = self.grains.iter_mut().position(|g| !g.active()) else {
            return;
        };
        let len_s = finite_or(size.at(i), 0.1).clamp(MIN_SIZE, MAX_SIZE);
        let src_rate = f64::from(src.sample_rate.max(1));
        let jitter = finite_or(spray.at(i), 0.0).clamp(0.0, MAX_SPRAY) * self.rng.bipolar();
        let start = finite_or(position.at(i), 0.0).clamp(0.0, 1.0) * frames + jitter * src_rate;
        let semis = finite_or(pitch.at(i), 0.0).clamp(-MAX_SEMITONES, MAX_SEMITONES);
        let step = (semis / 12.0).exp2() * src_rate / self.rate;
        let pan = finite_or(spread.at(i), 0.0).clamp(0.0, 1.0) * self.rng.bipolar();
        let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
        // Overlapping grains add like uncorrelated signals: scale by one
        // over the square root of how many sound at once on average.
        let overlap = (finite_or(density.at(i), 20.0).clamp(0.0, MAX_DENSITY) * len_s).max(1.0);
        let level = std::f64::consts::SQRT_2 / overlap.sqrt();
        let g = &mut self.grains[slot];
        g.src = Some(src.clone());
        g.pos = start.clamp(0.0, frames);
        g.step = step;
        g.age = 0.0;
        g.len = (len_s * self.rate).max(1.0);
        g.gl = (angle.cos() * level) as f32;
        g.gr = (angle.sin() * level) as f32;
    }

    /// Samples to wait for the next grain at `density` grains a second:
    /// on average one over the density, spread from half to one and a half
    /// times that so grains don't line up into a buzz.
    fn next_wait(&mut self, density: f64) -> f64 {
        (self.rate / density.max(1e-9)) * (0.5 + self.rng.uniform())
    }
}

impl Node for Granular {
    fn meta() -> NodeMeta {
        Self::base_meta().assets(&["file"]).telemetry(1)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::buffer("buffer")
                    .doc("Sound to granulate; unwired, the `file` config plays."),
            )
            .input(
                SocketSpec::float("position", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Where grains read from (fraction of the sound)."),
            )
            .input(
                SocketSpec::float("size", Unit::Seconds, 0.1)
                    .range(0.005, 1.0)
                    .log()
                    .doc("Length of each grain."),
            )
            .input(
                SocketSpec::float("density", Unit::Hz, 20.0)
                    .range(0.5, 200.0)
                    .log()
                    .doc("Grains started per second."),
            )
            .input(
                SocketSpec::float("spray", Unit::Seconds, 0.02)
                    .range(0.0, 1.0)
                    .doc("Random offset of each grain's start, either way."),
            )
            .input(
                SocketSpec::float("pitch", Unit::Semitones, 0.0)
                    .range(-24.0, 24.0)
                    .doc("Transposes every grain."),
            )
            .input(
                SocketSpec::float("spread", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("How far grains scatter across the stereo field."),
            )
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Level in decibels."),
            )
            .generator_inputs(false, false)
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(2))
                    .doc("The cloud, in stereo."),
            )
            .generator_outputs()
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let path = config.get_str_or("file", "");
        let asset = if path.is_empty() {
            None
        } else {
            ctx.resources.audio_asset(path)
        };
        let missing = !path.is_empty() && asset.is_none();
        let seed = mixed_seed(config, ctx.seed);
        Granular {
            file: asset.map(|a| prepare(a, 2, ctx.sample_rate)),
            missing,
            rate: ctx.sample_rate.max(1.0),
            seed,
            rng: Rng::new(seed),
            grains: vec![Grain::default(); MAX_GRAINS],
            playing: false,
            wait: 0.0,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let wired = io.inputs.is_wired(BUFFER);
        io.telemetry.set(
            TELEMETRY_MISSING,
            if self.missing && !wired { 1.0 } else { 0.0 },
        );
        let src = if wired {
            io.inputs.buffer(BUFFER).cloned()
        } else {
            self.file.clone()
        };
        let frames = io.frames();
        let position = io.inputs.float(POSITION);
        let size = io.inputs.float(SIZE);
        let density = io.inputs.float(DENSITY);
        let spray = io.inputs.float(SPRAY);
        let pitch = io.inputs.float(PITCH);
        let spread = io.inputs.float(SPREAD);
        let gain = io.inputs.float(GAIN);
        let events = [
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
        ];
        let mut out = io.outputs.audio(OUT);
        let Some((left, right)) = out.stereo() else {
            return;
        };
        let mut ends = [0u32; 4];
        let mut n_ends = 0;
        for run in Runs::new(events, frames) {
            let [stop, start, reset] = run.fired;
            if stop && self.playing {
                self.playing = false;
                if n_ends < ends.len() {
                    ends[n_ends] = run.at as u32;
                    n_ends += 1;
                }
            }
            if reset {
                self.rng = Rng::new(self.seed);
                self.wait = 0.0;
            }
            if start {
                self.playing = true;
                self.wait = 0.0;
            }
            for i in run.at..run.until {
                let d = finite_or(density.at(i), 20.0).clamp(0.0, MAX_DENSITY);
                if self.playing && d > 0.0 {
                    while self.wait <= 0.0 {
                        self.spawn(
                            src.as_ref(),
                            i,
                            position,
                            size,
                            density,
                            spray,
                            pitch,
                            spread,
                        );
                        self.wait += self.next_wait(d).max(1.0);
                    }
                    self.wait -= 1.0;
                }
                let (mut l, mut r) = (0.0f32, 0.0f32);
                for g in self.grains.iter_mut().filter(|g| g.active()) {
                    let Some(data) = g.src.as_deref() else {
                        g.len = 0.0;
                        continue;
                    };
                    let e = envelope(g.age / g.len);
                    let cl = data.channels.first().map_or(&[][..], Vec::as_slice);
                    let cr = data.channels.get(1).map_or(cl, Vec::as_slice);
                    let (sl, sr) = if data.channels.len() > 1 {
                        (read(cl, g.pos), read(cr, g.pos))
                    } else {
                        let s = read(cl, g.pos);
                        (s, s)
                    };
                    l += sl * e * g.gl;
                    r += sr * e * g.gr;
                    g.pos += g.step;
                    g.age += 1.0;
                    if !g.active() {
                        // Done: let go of the sound (never the last handle;
                        // the node or the buffer's maker keeps one).
                        g.src = None;
                    }
                }
                let db = gain.at(i);
                let k = if db.is_finite() {
                    db_to_gain(db.min(MAX_GAIN_DB))
                } else {
                    0.0
                };
                let (l, r) = (l * k, r * k);
                if let (Some(a), Some(b)) = (left.get_mut(i), right.get_mut(i)) {
                    *a = if l.is_finite() { l } else { 0.0 };
                    *b = if r.is_finite() { r } else { 0.0 };
                }
            }
        }
        io.outputs.set_bool(PLAYING, self.playing);
        let mut end = io.outputs.trigger(END);
        for &at in ends.iter().take(n_ends) {
            end.push(at);
        }
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        for g in &mut self.grains {
            *g = Grain::default();
        }
        self.playing = false;
        self.wait = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::{Harness, rms, tone_magnitude};
    use ac_core::{NoResources, Resources};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn data(channels: Vec<Vec<f32>>, rate: u32) -> Arc<AudioData> {
        Arc::new(AudioData {
            sample_rate: rate,
            channels,
        })
    }

    fn sine(f: f64, n: usize, rate: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * f * i as f64 / rate).sin() as f32)
            .collect()
    }

    fn cloud(buf: Arc<AudioData>, set: &[(&str, f64)], n: usize, block: usize) -> Vec<Vec<f32>> {
        let mut h = Harness::with_resources::<Granular>(json!({}), RATE, block, &NoResources);
        h.buffer("buffer", buf);
        for (k, v) in set {
            h.set(k, *v);
        }
        h.run(n).audio("out").to_vec()
    }

    #[test]
    fn grains_start_about_density_times_a_second() {
        // A buffer of ones: each grain is a Hann bump. Short, sparse grains
        // don't overlap, so bumps can be counted.
        let buf = data(vec![vec![1.0; 48_000]], 48_000);
        let out = cloud(
            buf,
            &[
                ("density", 10.0),
                ("size", 0.01),
                ("spray", 0.0),
                ("spread", 0.0),
            ],
            480_000,
            256,
        );
        let x = &out[0];
        let starts = x.windows(2).filter(|w| w[0] == 0.0 && w[1] > 0.0).count();
        // Ten seconds at ten a second.
        assert!((85..=115).contains(&starts), "{starts} grains");
        // Centered: both sides alike, each grain peaking at one.
        assert_eq!(out[0], out[1]);
        let peak = x.iter().fold(0.0f32, |m, v| m.max(*v));
        assert!((peak - 1.0).abs() < 1e-3, "{peak}");
    }

    #[test]
    fn pitch_transposes_and_position_picks_the_slice() {
        // First half 220 Hz, second half 880 Hz.
        let mut x = sine(220.0, 48_000, RATE);
        x.extend(sine(880.0, 48_000, RATE));
        let buf = data(vec![x], 48_000);
        let at = |position: f64, pitch: f64| {
            let out = cloud(
                buf.clone(),
                &[
                    ("position", position),
                    ("pitch", pitch),
                    ("spray", 0.01),
                    ("size", 0.05),
                    ("density", 50.0),
                ],
                48_000,
                64,
            );
            let y = &out[0][4800..];
            (
                tone_magnitude(y, 220.0, RATE),
                tone_magnitude(y, 440.0, RATE),
                tone_magnitude(y, 880.0, RATE),
            )
        };
        let (a, b, c) = at(0.25, 0.0);
        assert!(a > 5.0 * b.max(c), "{a} {b} {c}");
        let (a, b, c) = at(0.75, 0.0);
        assert!(c > 5.0 * a.max(b), "{a} {b} {c}");
        // An octave up from the 220 Hz half is 440 Hz.
        let (a, b, c) = at(0.25, 12.0);
        assert!(b > 5.0 * a.max(c), "{a} {b} {c}");
    }

    #[test]
    fn a_buffer_at_another_rate_keeps_its_pitch() {
        let buf = data(vec![sine(440.0, 44_100, 44_100.0)], 44_100);
        let out = cloud(
            buf,
            &[("position", 0.5), ("spray", 0.0), ("size", 0.1)],
            48_000,
            128,
        );
        let y = &out[0][4800..];
        assert!(tone_magnitude(y, 440.0, RATE) > 10.0 * tone_magnitude(y, 479.0, RATE));
    }

    #[test]
    fn overlap_keeps_the_level_about_even() {
        let mut s = 3u32;
        let noise: Vec<f32> = (0..96_000)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect();
        let buf = data(vec![noise], 48_000);
        let level = |density: f64| {
            let out = cloud(
                buf.clone(),
                &[
                    ("density", density),
                    ("size", 0.2),
                    ("spray", 0.5),
                    ("position", 0.5),
                    ("spread", 0.0),
                ],
                96_000,
                256,
            );
            rms(&out[0][24_000..])
        };
        let (thin, thick) = (level(10.0), level(150.0));
        let db = 20.0 * (thick / thin).log10();
        assert!(db.abs() < 3.0, "{thin} → {thick} ({db} dB)");
    }

    #[test]
    fn stop_lets_grains_finish_and_start_and_reset_repeat() {
        let buf = data(vec![vec![1.0; 48_000]], 48_000);
        let mut h = Harness::new::<Granular>(json!({}));
        h.buffer("buffer", buf.clone())
            .set("size", 0.05)
            .set("density", 40.0)
            .triggers("stop", &[24_000]);
        let out = h.run(48_000);
        assert_eq!(out.triggers("end"), &[24_000]);
        let x = &out.audio("out")[0];
        // Grains ring past the stop by at most one grain (2400 samples).
        assert!(rms(&x[24_000..26_400]) > 0.0);
        assert!(x[26_401..].iter().all(|v| *v == 0.0));
        // Reset starts the same random choices over.
        let mut h = Harness::new::<Granular>(json!({ "seed": 3 }));
        h.buffer("buffer", buf.clone());
        let a = h.run(9_600).audio("out")[1].clone();
        h.reset();
        h.play();
        let b = h.run(9_600).audio("out")[1].clone();
        assert_eq!(a, b);
        // Another seed scatters differently.
        let mut h = Harness::new::<Granular>(json!({ "seed": 4 }));
        h.buffer("buffer", buf);
        assert_ne!(h.run(9_600).audio("out")[1], a);
    }

    #[test]
    fn block_size_does_not_change_the_cloud() {
        let buf = data(
            vec![sine(330.0, 48_000, RATE), sine(331.0, 48_000, RATE)],
            48_000,
        );
        let pos: Vec<f32> = (0..20_000).map(|i| i as f32 / 20_000.0).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Granular>(json!({}), RATE, block, &NoResources);
            h.buffer("buffer", buf.clone())
                .set_samples("position", pos.clone());
            h.run(20_000).audio("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1024));
        assert!(rms(&one[0]) > 0.01);
    }

    #[test]
    fn nothing_to_play_is_silent_and_bad_inputs_stay_finite() {
        // Unwired, no file.
        let mut h = Harness::new::<Granular>(json!({}));
        assert!(h.run(4_800).audio("out")[0].iter().all(|v| *v == 0.0));
        // An empty buffer.
        let out = cloud(data(vec![], 48_000), &[], 4_800, 64);
        assert!(out[0].iter().all(|v| *v == 0.0));
        // A missing file is flagged.
        let mut h = Harness::new::<Granular>(json!({ "file": "assets/none.wav" }));
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
        // Absurd settings: finite, and never more grains than slots.
        let buf = data(vec![vec![0.5; 1_000]], 48_000);
        for set in [
            [
                ("density", 1e9),
                ("size", 1e9),
                ("spray", f64::NAN),
                ("pitch", 1e9),
            ],
            [
                ("density", 0.0),
                ("size", 0.0),
                ("spray", -1.0),
                ("pitch", f64::NAN),
            ],
            [
                ("density", f64::INFINITY),
                ("size", f64::NAN),
                ("spray", 1e9),
                ("pitch", -1e9),
            ],
        ] {
            let out = cloud(buf.clone(), &set, 9_600, 1);
            assert!(out.iter().flatten().all(|v| v.is_finite()), "{set:?}");
        }
    }

    #[test]
    fn the_file_config_plays_when_the_buffer_is_unwired() {
        struct One(Arc<AudioData>);
        impl Resources for One {
            fn audio_asset(&self, _: &str) -> Option<Arc<AudioData>> {
                Some(self.0.clone())
            }
        }
        let file = One(data(vec![vec![0.5; 24_000]], 24_000));
        let mut h =
            Harness::with_resources::<Granular>(json!({ "file": "assets/x.wav" }), RATE, 64, &file);
        let out = h.run(9_600);
        assert!(rms(&out.audio("out")[0]) > 0.05);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }
}
