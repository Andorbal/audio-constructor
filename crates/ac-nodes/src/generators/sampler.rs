use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx,
    Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::interp::hermite4;
use ac_dsp::util::{db_to_gain, snap_samples};

use super::clip::prepare;
use crate::segments::Runs;

node! {
    kind: "sampler",
    name: "Sampler",
    category: Generators,
    keywords: ["sample", "one shot", "drum", "pitch", "keyboard", "instrument", "loop", "buffer"],
    /// Plays a recorded sound at any pitch, with start and end points,
    /// a sustain loop, and the generator contract: the building block for
    /// drum machines and sampled instruments.
    ///
    /// **Source.** A wired `buffer` (for example a `Record` node's last
    /// take) is played; with `buffer` unwired, the `file` config (an audio
    /// file in the project, assigned in the side panel) is played instead,
    /// converted to the engine's rate once when the project loads. A wired
    /// `buffer` that holds nothing yet, or a missing file, plays silence
    /// (a missing file is flagged in the editor). A new buffer takes effect
    /// at the next `start`, so a sound never changes under a playing note.
    /// The output has `channels` channels (config, 1 to 64, default 2): a
    /// mono sound goes to every channel and extra channels are dropped.
    ///
    /// **Pitch.** `note` is a MIDI note number (fractions bend it); at the
    /// `root` config note (default 60, middle C) the sound plays at its own
    /// speed, and each semitone above multiplies the speed by 2^(1/12).
    /// `rate` multiplies on top. Fractional speeds are interpolated (cubic
    /// Hermite); a buffer recorded at another sample rate still plays at
    /// its own pitch. `gain` is in decibels (silent at −60 and below).
    ///
    /// **Region and loop.** `start_point` and `end_point` (0 to 1, fractions
    /// of the whole sound) mark the part that plays; if `end_point` is
    /// before `start_point` the two swap. With `loop` on, playback runs
    /// from `start_point` into the loop (`loop_start` to `loop_end`, also
    /// fractions of the whole sound, kept inside the region) and repeats
    /// it until stopped, or until `loop` turns off, when it plays on to
    /// `end_point`. A loop shorter than one sample loops the whole region.
    /// `crossfade` (seconds of the sound) blends the end of each pass into
    /// the audio just before `loop_start` (equal-power), hiding the seam;
    /// it is shortened to fit the audio available there and the loop's
    /// length. Points are followed sample by sample, so they can be
    /// modulated.
    ///
    /// **Generator contract.** `start` (re)triggers from `start_point`
    /// (from `end_point` when reversed). Reaching the end stops it and
    /// fires `end`; `stop` silences it and fires `end`; `reset` jumps back
    /// to the beginning without changing whether it plays. `reverse` flips
    /// the direction (backward loops cross-fade into the audio just after
    /// `loop_end`). Events at the same frame apply in the order reverse,
    /// stop, start, reset. Nothing to play (an empty sound or region)
    /// fires `end` as soon as it starts. `position` reports where playback
    /// is, in seconds from the beginning of the sound.
    ///
    /// **Try this:** wire a `Voices` body's `Voice` `trigger` into `start`
    /// and its note (through a `Constant` or the MIDI note) into `note` for
    /// a playable sampled instrument; or wire a `Record` node's `buffer`
    /// here and a `Clock` into `start` to loop a phrase you just played.
    pub struct Sampler {
        /// The `file` config's audio at the engine rate, if any.
        file: Option<Arc<AudioData>>,
        /// A file is configured but couldn't be loaded.
        missing: bool,
        root: f32,
        rate: f64,
        /// What the current note plays (latched at `start`).
        current: Option<Arc<AudioData>>,
        /// What played before the last `start` that changed the sound,
        /// for the frames of that block before the start.
        previous: Option<Arc<AudioData>>,
        /// The current sound's rate relative to the engine's.
        speed: f64,
        /// Read position in frames of the current sound.
        pos: f64,
        last_step: f64,
        forward: bool,
        playing: bool,
        /// Per frame of the block: the read position (NaN where silent),
        /// the crossfade partner's position (NaN for none), and the
        /// partner's share (0 to 1).
        positions: Vec<f64>,
        partners: Vec<f64>,
        shares: Vec<f32>,
    }
}

const BUFFER: usize = 0;
const NOTE: usize = 1;
const GAIN: usize = 2;
const START_POINT: usize = 3;
const END_POINT: usize = 4;
const LOOP_START: usize = 5;
const LOOP_END: usize = 6;
const CROSSFADE: usize = 7;
const START: usize = 8;
const STOP: usize = 9;
const RESET: usize = 10;
const LOOP: usize = 11;
const REVERSE: usize = 12;
const RATE: usize = 13;

const OUT: usize = 0;
const POSITION: usize = 1;
const PLAYING: usize = 2;
const END: usize = 3;

/// Telemetry slot: 1.0 when the `file` config couldn't be loaded.
pub const TELEMETRY_MISSING: usize = 0;

const MAX_CHANNELS: u64 = 64;
/// Fastest `rate`, either way.
const MAX_RATE: f32 = 16.0;
/// Furthest `note` can bend from `root`, in semitones.
const MAX_SEMITONES: f32 = 96.0;
/// Loudest `gain` honoured.
const MAX_GAIN_DB: f32 = 48.0;

fn channels(config: &Config) -> usize {
    config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as usize
}

fn root(config: &Config) -> f32 {
    let r = config.get_f64_or("root", 60.0);
    if r.is_finite() {
        r.clamp(0.0, 127.0) as f32
    } else {
        60.0
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// Channel `c` of a sound (the first channel for mono), or nothing.
fn channel(d: Option<&AudioData>, c: usize) -> &[f32] {
    d.and_then(|d| d.channels.get(c).or_else(|| d.channels.first()))
        .map_or(&[][..], Vec::as_slice)
}

/// A fraction input at frame `i`, held to 0..1.
#[inline]
fn fraction(x: FloatIn, i: usize, default: f32) -> f64 {
    f64::from(finite_or(x.at(i), default).clamp(0.0, 1.0))
}

/// Where to play within a sound of `len` frames, in frames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Region {
    lo: f64,
    hi: f64,
    loop_lo: f64,
    loop_hi: f64,
}

impl Region {
    #[inline]
    fn loop_len(&self) -> f64 {
        self.loop_hi - self.loop_lo
    }
}

/// The inputs that shape the region, read frame by frame.
#[derive(Clone, Copy)]
struct Points<'a> {
    start: FloatIn<'a>,
    end: FloatIn<'a>,
    loop_start: FloatIn<'a>,
    loop_end: FloatIn<'a>,
}

impl Points<'_> {
    #[inline]
    fn region(&self, i: usize, len: f64) -> Region {
        // Points typed as round fractions land on whole samples.
        let at = |x: FloatIn, default| snap_samples(fraction(x, i, default) * len);
        let (s, e) = (at(self.start, 0.0), at(self.end, 1.0));
        let (lo, hi) = if e < s { (e, s) } else { (s, e) };
        let (ls, le) = (at(self.loop_start, 0.0), at(self.loop_end, 1.0));
        let (ls, le) = if le < ls { (le, ls) } else { (ls, le) };
        let (loop_lo, loop_hi) = (ls.clamp(lo, hi), le.clamp(lo, hi));
        if loop_hi - loop_lo < 1.0 {
            Region {
                lo,
                hi,
                loop_lo: lo,
                loop_hi: hi,
            }
        } else {
            Region {
                lo,
                hi,
                loop_lo,
                loop_hi,
            }
        }
    }
}

impl Sampler {
    fn len(&self) -> f64 {
        self.current.as_ref().map_or(0, |d| d.frames()) as f64
    }

    fn beginning(&self, r: &Region) -> f64 {
        if self.forward { r.lo } else { r.hi - 1.0 }
    }

    /// Sample `k` of `ch`, silent outside it.
    #[inline]
    fn sample(ch: &[f32], k: isize) -> f32 {
        if k < 0 {
            return 0.0;
        }
        ch.get(k as usize).copied().unwrap_or(0.0)
    }

    #[inline]
    fn read(ch: &[f32], p: f64) -> f32 {
        let whole = p.floor();
        let t = (p - whole) as f32;
        let k = whole as isize;
        hermite4(
            Self::sample(ch, k - 1),
            Self::sample(ch, k),
            Self::sample(ch, k + 1),
            Self::sample(ch, k + 2),
            t,
        )
    }
}

impl Node for Sampler {
    fn meta() -> NodeMeta {
        Self::base_meta().assets(&["file"]).telemetry(1)
    }

    fn describe(config: &Config) -> Signature {
        let fraction = |name: &'static str, default: f64, doc: &'static str| {
            SocketSpec::float(name, Unit::Generic, default)
                .range(0.0, 1.0)
                .doc(doc)
        };
        Signature::new()
            .input(
                SocketSpec::buffer("buffer")
                    .doc("Sound to play; unwired, the `file` config plays."),
            )
            .input(
                SocketSpec::float("note", Unit::Generic, 60.0)
                    .range(0.0, 127.0)
                    .doc("MIDI note; the `root` note plays at the sound's own pitch."),
            )
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Level in decibels."),
            )
            .input(fraction(
                "start_point",
                0.0,
                "Where playback starts (fraction of the sound).",
            ))
            .input(fraction(
                "end_point",
                1.0,
                "Where playback ends (fraction of the sound).",
            ))
            .input(fraction(
                "loop_start",
                0.0,
                "Start of the loop (fraction of the sound).",
            ))
            .input(fraction(
                "loop_end",
                1.0,
                "End of the loop (fraction of the sound).",
            ))
            .input(
                SocketSpec::float("crossfade", Unit::Seconds, 0.0)
                    .range(0.0, 0.5)
                    .doc("Blend across the loop seam."),
            )
            .generator_inputs(true, true)
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(channels(config) as u32))
                    .doc("The sound."),
            )
            .output(
                SocketSpec::float("position", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("Playback position in the sound."),
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
        let block = ctx.max_block.max(1);
        Sampler {
            file: asset.map(|a| prepare(a, channels(config), ctx.sample_rate)),
            missing,
            root: root(config),
            rate: ctx.sample_rate,
            current: None,
            previous: None,
            speed: 1.0,
            pos: 0.0,
            last_step: 0.0,
            forward: true,
            playing: false,
            positions: vec![f64::NAN; block],
            partners: vec![f64::NAN; block],
            shares: vec![0.0; block],
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let buffer_wired = io.inputs.is_wired(BUFFER);
        io.telemetry.set(
            TELEMETRY_MISSING,
            if self.missing && !buffer_wired {
                1.0
            } else {
                0.0
            },
        );
        let frames = io.frames();
        let note = io.inputs.float(NOTE);
        let gain = io.inputs.float(GAIN);
        let points = Points {
            start: io.inputs.float(START_POINT),
            end: io.inputs.float(END_POINT),
            loop_start: io.inputs.float(LOOP_START),
            loop_end: io.inputs.float(LOOP_END),
        };
        let crossfade = io.inputs.float(CROSSFADE);
        let looping = io.inputs.boolean(LOOP);
        let speed = io.inputs.float(RATE);
        let events = [
            io.inputs.triggers(REVERSE),
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
        ];

        // A buffer input is constant within a block, so every start in
        // this block latches the same sound; frames before the first one
        // still read the sound that was playing.
        let mut switch_at = 0;
        let mut switched = false;

        // Pass 1: transport and read positions, frame by frame.
        {
            let mut end = io.outputs.trigger(END);
            for run in Runs::new(events, frames) {
                let [reverse, stop, start, reset] = run.fired;
                if reverse {
                    self.forward = !self.forward;
                }
                if stop && self.playing {
                    self.playing = false;
                    end.push(run.at as u32);
                }
                if start {
                    let next = if buffer_wired {
                        io.inputs.buffer(BUFFER)
                    } else {
                        self.file.as_ref()
                    };
                    let same = match (next, self.current.as_ref()) {
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    };
                    if !same {
                        if !switched {
                            switched = true;
                            switch_at = run.at;
                            self.previous = self.current.take();
                        }
                        self.current = next.cloned();
                    }
                    let data_rate = self.current.as_ref().map_or(0, |d| d.sample_rate);
                    self.speed = if data_rate > 0 && self.rate > 0.0 {
                        f64::from(data_rate) / self.rate
                    } else {
                        1.0
                    };
                    let r = points.region(run.at, self.len());
                    if r.hi - r.lo <= 0.0 {
                        self.playing = false;
                        end.push(run.at as u32);
                    } else {
                        self.playing = true;
                        self.pos = self.beginning(&r);
                        self.last_step = 0.0;
                    }
                }
                if reset {
                    let r = points.region(run.at, self.len());
                    self.pos = self.beginning(&r);
                    self.last_step = 0.0;
                }
                let len = self.len();
                let data_rate = self.speed * self.rate;
                for i in run.at..run.until {
                    let (mut at, mut partner, mut share) = (f64::NAN, f64::NAN, 0.0);
                    if self.playing {
                        let r = points.region(i, len);
                        let bend = (finite_or(note.at(i), self.root) - self.root)
                            .clamp(-MAX_SEMITONES, MAX_SEMITONES);
                        let rt = finite_or(speed.at(i), 1.0).clamp(-MAX_RATE, MAX_RATE);
                        let rt = if self.forward { rt } else { -rt };
                        let step = f64::from(rt) * f64::from((bend / 12.0).exp2()) * self.speed;
                        let lp = looping.at(i);
                        // Wrap only when this step crossed the loop's edge
                        // (a loop switched on later plays on to the end).
                        let wrapped = lp
                            && ((step >= 0.0
                                && self.pos >= r.loop_hi
                                && self.pos - self.last_step < r.loop_hi)
                                || (step < 0.0
                                    && self.pos < r.loop_lo
                                    && self.pos - self.last_step >= r.loop_lo));
                        if wrapped {
                            self.pos = r.loop_lo + (self.pos - r.loop_lo).rem_euclid(r.loop_len());
                        } else if (step >= 0.0 && self.pos >= r.hi)
                            || (step < 0.0 && self.pos < r.lo)
                        {
                            self.playing = false;
                            end.push(i as u32);
                        }
                        if self.playing {
                            at = self.pos;
                            let fade =
                                f64::from(finite_or(crossfade.at(i), 0.0).max(0.0)) * data_rate;
                            if lp && fade >= 1.0 {
                                let ll = r.loop_len();
                                if step >= 0.0 {
                                    let x = fade.min(r.loop_lo - r.lo).min(ll);
                                    let d = r.loop_hi - self.pos;
                                    if x >= 1.0 && d > 0.0 && d <= x {
                                        share = (1.0 - d / x) as f32;
                                        partner = self.pos - ll;
                                    }
                                } else {
                                    let x = fade.min(r.hi - r.loop_hi).min(ll);
                                    let d = self.pos - r.loop_lo;
                                    if x >= 1.0 && d > 0.0 && d <= x {
                                        share = (1.0 - d / x) as f32;
                                        partner = self.pos + ll;
                                    }
                                }
                            }
                            self.pos += step;
                            self.last_step = step;
                        }
                    }
                    if let Some(p) = self.positions.get_mut(i) {
                        *p = at;
                    }
                    if let Some(p) = self.partners.get_mut(i) {
                        *p = partner;
                    }
                    if let Some(s) = self.shares.get_mut(i) {
                        *s = share;
                    }
                }
            }
        }

        // Pass 2: render each channel from the positions.
        let mut out = io.outputs.audio(OUT);
        let gain_at = |i: usize| {
            let db = gain.at(i);
            if db.is_finite() {
                db_to_gain(db.min(MAX_GAIN_DB))
            } else {
                0.0
            }
        };
        for c in 0..out.channels() {
            let dst = out.channel(c);
            let (old, new) = (
                channel(self.previous.as_deref(), c),
                channel(self.current.as_deref(), c),
            );
            for (i, o) in dst.iter_mut().enumerate() {
                let p = self.positions.get(i).copied().unwrap_or(f64::NAN);
                if p.is_nan() {
                    *o = 0.0;
                    continue;
                }
                let src = if i < switch_at { old } else { new };
                let main = Self::read(src, p);
                let q = self.partners.get(i).copied().unwrap_or(f64::NAN);
                let v = if q.is_nan() {
                    main
                } else {
                    // Equal-power blend into the partner.
                    let w = self.shares.get(i).copied().unwrap_or(0.0);
                    let (s, k) = (w * std::f32::consts::FRAC_PI_2).sin_cos();
                    main * k + Self::read(src, q) * s
                };
                *o = v * gain_at(i);
            }
        }

        let data_rate = self.speed * self.rate;
        let seconds = if data_rate > 0.0 {
            self.pos.clamp(0.0, self.len()) / data_rate
        } else {
            0.0
        };
        io.outputs.set_float(
            POSITION,
            if seconds.is_finite() {
                seconds as f32
            } else {
                0.0
            },
        );
        io.outputs.set_bool(PLAYING, self.playing);
    }

    fn reset(&mut self) {
        self.pos = 0.0;
        self.last_step = 0.0;
        self.forward = true;
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Resources;
    use ac_core::testing::{Harness, peak, zero_crossing_freq};
    use serde_json::json;

    struct Assets(Arc<AudioData>);

    impl Resources for Assets {
        fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
            (path == "assets/hit.wav").then(|| self.0.clone())
        }
    }

    fn data(channels: Vec<Vec<f32>>, rate: u32) -> Arc<AudioData> {
        Arc::new(AudioData {
            sample_rate: rate,
            channels,
        })
    }

    /// 1/100, 2/100, ... 100/100.
    fn ramp() -> Vec<f32> {
        (1..=100).map(|i| i as f32 / 100.0).collect()
    }

    fn with_file(res: &Assets, block: usize) -> Harness {
        Harness::with_resources::<Sampler>(
            json!({ "file": "assets/hit.wav", "channels": 1 }),
            48_000.0,
            block,
            res,
        )
    }

    fn with_buffer(buf: Arc<AudioData>, block: usize) -> Harness {
        let mut h = Harness::with_resources::<Sampler>(
            json!({ "channels": 1 }),
            48_000.0,
            block,
            &ac_core::NoResources,
        );
        h.buffer("buffer", buf);
        h
    }

    #[test]
    fn declares_its_asset_and_sockets() {
        assert_eq!(Sampler::meta().asset_keys, &["file"]);
        let sig = Sampler::describe(&json!({ "channels": 3 }));
        assert_eq!(sig.outputs[OUT].channels, Channels::Fixed(3));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "buffer",
                "note",
                "gain",
                "start_point",
                "end_point",
                "loop_start",
                "loop_end",
                "crossfade",
                "start",
                "stop",
                "reset",
                "loop",
                "reverse",
                "rate"
            ]
        );
    }

    #[test]
    fn plays_the_file_at_root_then_ends() {
        let res = Assets(data(vec![ramp()], 48_000));
        let mut h = with_file(&res, 64);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!(&o[..100], ramp().as_slice());
        assert_eq!(peak(&o[100..]), 0.0);
        assert_eq!(out.triggers("end"), &[100]);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }

    #[test]
    fn note_sets_the_pitch() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32)
            .collect();
        let buf = data(vec![tone], 48_000);
        let mut h = with_buffer(buf.clone(), 256);
        h.set("note", 72.0);
        let o = h.run(20_000).audio("out")[0].clone();
        let f = zero_crossing_freq(&o[..20_000], 48_000.0);
        assert!((f - 880.0).abs() < 1.0, "{f}");
        // A buffer at another rate keeps its pitch.
        let tone: Vec<f32> = (0..24_000)
            .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 24_000.0).sin() as f32)
            .collect();
        let mut h = with_buffer(data(vec![tone], 24_000), 256);
        let out = h.run(50_000);
        let f = zero_crossing_freq(&out.audio("out")[0][..40_000], 48_000.0);
        assert!((f - 440.0).abs() < 1.0, "{f}");
        assert_eq!(out.triggers("end"), &[48_000]);
    }

    #[test]
    fn region_points_and_reverse() {
        let buf = data(vec![ramp()], 48_000);
        let mut h = with_buffer(buf.clone(), 64);
        h.set("start_point", 0.2).set("end_point", 0.5);
        let out = h.run(128);
        let o = &out.audio("out")[0];
        assert_eq!(&o[..30], &ramp()[20..50]);
        assert_eq!(out.triggers("end"), &[30]);
        // Swapped points are put in order; reversed playback starts at the end.
        let mut h = with_buffer(buf, 64);
        h.set("start_point", 0.5)
            .set("end_point", 0.2)
            .triggers("reverse", &[0]);
        let out = h.run(128);
        let o = &out.audio("out")[0];
        let back: Vec<f32> = ramp()[20..50].iter().rev().copied().collect();
        assert_eq!(&o[..30], back.as_slice());
        assert_eq!(out.triggers("end"), &[30]);
    }

    #[test]
    fn loops_between_the_loop_points() {
        let buf = data(vec![ramp()], 48_000);
        let mut h = with_buffer(buf.clone(), 64);
        h.set("loop", true)
            .set("loop_start", 0.5)
            .set("loop_end", 0.6);
        let out = h.run(300);
        let o = &out.audio("out")[0];
        assert_eq!(&o[..60], &ramp()[..60]);
        assert_eq!(&o[60..70], &ramp()[50..60]);
        assert_eq!(&o[70..80], &ramp()[50..60]);
        assert!(out.triggers("end").is_empty());
        // Turning the loop off plays on to the end.
        let mut h = with_buffer(buf, 64);
        let lp: Vec<f32> = (0..300).map(|i| if i < 85 { 1.0 } else { 0.0 }).collect();
        h.set_samples("loop", lp)
            .set("loop_start", 0.5)
            .set("loop_end", 0.6);
        let out = h.run(300);
        let o = &out.audio("out")[0];
        assert_eq!(&o[85..90], &ramp()[55..60]);
        assert_eq!(o[90], ramp()[60]);
        assert_eq!(out.triggers("end"), &[130]);
    }

    #[test]
    fn crossfade_hides_the_seam() {
        // A ramp has a jump at the loop seam; a crossfade smooths it.
        let long: Vec<f32> = (0..1_000).map(|i| i as f32 / 1_000.0).collect();
        let buf = data(vec![long], 48_000);
        let jump = |fade: f64| {
            let mut h = with_buffer(buf.clone(), 64);
            h.set("loop", true)
                .set("loop_start", 0.4)
                .set("loop_end", 0.6)
                .set("crossfade", fade);
            let o = h.run(2_000).audio("out")[0].clone();
            o.windows(2)
                .skip(10)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        };
        let (hard, soft) = (jump(0.0), jump(100.0 / 48_000.0));
        assert!(hard > 0.15, "{hard}");
        assert!(soft < 0.03, "{soft}");
    }

    #[test]
    fn new_buffers_wait_for_the_next_start() {
        let a = data(vec![vec![0.25; 200]], 48_000);
        let b = data(vec![vec![0.75; 200]], 48_000);
        let mut h = with_buffer(a, 64);
        h.triggers("start", &[0, 100]);
        let o = h.run(64).audio("out")[0].clone();
        assert_eq!(o[10], 0.25);
        h.buffer("buffer", b);
        let o = h.run(64).audio("out")[0].clone();
        assert_eq!(o[0], 0.25, "still the old sound");
        assert_eq!(o[63], 0.75, "retriggered at 100 with the new one");
    }

    #[test]
    fn empty_or_missing_sounds_end_at_once() {
        let mut h = Harness::with_resources::<Sampler>(
            json!({ "file": "assets/nope.wav" }),
            48_000.0,
            64,
            &ac_core::NoResources,
        );
        let out = h.run(128);
        assert!(out.audio("out").iter().flatten().all(|v| *v == 0.0));
        assert_eq!(out.triggers("end"), &[0]);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
        // An empty buffer, and an empty region, do the same.
        let mut h = with_buffer(data(vec![], 48_000), 64);
        assert_eq!(h.run(64).triggers("end"), &[0]);
        let mut h = with_buffer(data(vec![ramp()], 48_000), 64);
        h.set("start_point", 0.3).set("end_point", 0.3);
        assert_eq!(h.run(64).triggers("end"), &[0]);
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let tone: Vec<f32> = (0..3_000).map(|i| (i as f32 * 0.05).sin()).collect();
        let buf = data(vec![tone], 44_100);
        let note: Vec<f32> = (0..6_000)
            .map(|i| 60.0 + 5.0 * (i as f32 * 0.001).sin())
            .collect();
        let render = |block: usize| {
            let mut h = with_buffer(buf.clone(), block);
            h.set_samples("note", note.clone())
                .set("loop", true)
                .set("loop_start", 0.3)
                .set("loop_end", 0.7)
                .set("crossfade", 0.002)
                .triggers("reverse", &[2_000])
                .triggers("start", &[3_000])
                .triggers("stop", &[5_500]);
            let out = h.run(6_000);
            (out.audio("out")[0].clone(), out.triggers("end").to_vec())
        };
        let one = render(1);
        assert_eq!(one.1, vec![5_500]);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let buf = data(vec![ramp(), ramp()], 48_000);
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 0.5];
        let wild: Vec<f32> = wild.iter().cycle().take(1_000).copied().collect();
        let mut h = Harness::with_resources::<Sampler>(
            json!({ "channels": 2, "root": "x" }),
            48_000.0,
            64,
            &ac_core::NoResources,
        );
        h.buffer("buffer", buf);
        for name in [
            "note",
            "gain",
            "start_point",
            "end_point",
            "loop_start",
            "loop_end",
            "crossfade",
            "rate",
        ] {
            h.set_samples(name, wild.clone());
        }
        h.set("loop", true).triggers("start", &[0, 100, 333, 700]);
        let out = h.run(1_000);
        assert!(out.audio("out").iter().flatten().all(|v| v.is_finite()));
        assert!(out.float("position").iter().all(|v| v.is_finite()));
    }

    #[test]
    fn transport_reset_starts_over() {
        let res = Assets(data(vec![ramp()], 48_000));
        let mut h = with_file(&res, 64);
        h.triggers("reverse", &[5]);
        h.run(64);
        h.reset();
        h.play();
        assert_eq!(h.run(64).audio("out")[0][0], 0.01);
    }
}
