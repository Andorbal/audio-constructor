use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate,
    Signature, SocketSpec, Unit, node,
};
use ac_dsp::interp::hermite4;
use ac_dsp::resample::Resampler;
use ac_dsp::util::snap_samples;

use crate::segments::Runs;

node! {
    kind: "clip",
    name: "Clip",
    category: Generators,
    keywords: ["sample", "audio file", "wav", "player", "playback", "loop", "take", "file"],
    /// Plays an audio file, with looping, reverse, and variable speed.
    ///
    /// Assign a file in the side panel (the `file` config, a path relative
    /// to the project). The output has `channels` channels (set from the
    /// file when you assign it): a mono file is copied to every channel, and
    /// extra file channels are dropped. Files at another sample rate are
    /// converted once when the project loads. A missing file plays silence
    /// and reports itself as missing so the editor can flag it.
    ///
    /// Generator contract: silent until `start`, which plays from the
    /// beginning (the last frame when reversed). At the end it stops and
    /// fires `end`, or wraps around if `loop` is on. `stop` silences it and
    /// fires `end`; `reset` jumps back to the beginning without changing
    /// whether it plays. `reverse` flips the direction, and `rate` sets the
    /// speed (2 is double speed and an octave up; negative values play
    /// backwards); fractional rates are interpolated. Events at the same
    /// frame apply in the order reverse, stop, start, reset. A missing or
    /// empty file fires `end` as soon as it starts, so chains keep moving.
    ///
    /// Changing `position` (seconds) seeks there, at the start of the next
    /// block. After loading or a transport reset the clip counts as being at
    /// 0, so a fixed nonzero `position` acts as the point transport playback
    /// starts from. The `position` output reports where playback is.
    ///
    /// **Try this:** wire a `Clock` into `start` to retrigger a drum loop
    /// on every bar, or chain two Clips by wiring the first one's `end` into
    /// the second one's `start`.
    pub struct Clip {
        /// The file's channels (those the output uses) at the engine rate.
        data: Option<Arc<AudioData>>,
        missing: bool,
        len: usize,
        rate: f64,
        /// Read position in frames.
        pos: f64,
        forward: bool,
        playing: bool,
        /// Last block's `position` input, to detect seeks.
        last_position: f32,
        /// Read position for each frame of the block (NaN where silent).
        positions: Vec<f64>,
    }
}

const POSITION: usize = 0;
const START: usize = 1;
const STOP: usize = 2;
const RESET: usize = 3;
const LOOP: usize = 4;
const REVERSE: usize = 5;
const RATE: usize = 6;

const OUT: usize = 0;
const POSITION_OUT: usize = 1;
const PLAYING: usize = 2;
const END: usize = 3;

/// Telemetry slot: 1.0 when the file couldn't be loaded.
pub const TELEMETRY_MISSING: usize = 0;

const MAX_CHANNELS: u64 = 64;
/// Fastest playback, either way.
const MAX_RATE: f32 = 16.0;

fn channels(config: &Config) -> usize {
    config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as usize
}

/// The file's first `channels` channels at `rate`, sharing the host's
/// buffer when no conversion is needed.
fn prepare(asset: Arc<AudioData>, channels: usize, rate: f64) -> Arc<AudioData> {
    let file_rate = f64::from(asset.sample_rate);
    let used = asset.channels.len().min(channels);
    if file_rate <= 0.0 || file_rate == rate || asset.frames() == 0 {
        return asset;
    }
    match Resampler::new(file_rate, rate) {
        Some(r) => Arc::new(AudioData {
            sample_rate: rate.round() as u32,
            channels: asset
                .channels
                .iter()
                .take(used)
                .map(|c| r.process(c))
                .collect(),
        }),
        None => asset,
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Clip {
    fn beginning(&self) -> f64 {
        if self.forward {
            0.0
        } else {
            self.len.saturating_sub(1) as f64
        }
    }

    /// Sample `k` of `ch`, wrapping around when looping and silent outside
    /// the clip otherwise.
    #[inline]
    fn sample(ch: &[f32], k: isize, wrap: bool) -> f32 {
        let len = ch.len() as isize;
        let k = if wrap && len > 0 {
            k.rem_euclid(len)
        } else {
            k
        };
        if k < 0 {
            return 0.0;
        }
        ch.get(k as usize).copied().unwrap_or(0.0)
    }
}

impl Node for Clip {
    fn meta() -> NodeMeta {
        Self::base_meta().assets(&["file"]).telemetry(1)
    }

    fn describe(config: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("position", Unit::Seconds, 0.0)
                    .range(0.0, 600.0)
                    .doc("Seeks here when it changes."),
            )
            .generator_inputs(true, true)
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(channels(config) as u32))
                    .doc("The clip's audio."),
            )
            .output(
                SocketSpec::float("position", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("Playback position."),
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
        let missing = asset.is_none();
        let data = asset.map(|a| prepare(a, channels(config), ctx.sample_rate));
        let len = data.as_ref().map_or(0, |d| d.frames());
        Clip {
            data,
            missing,
            len,
            rate: ctx.sample_rate,
            pos: 0.0,
            forward: true,
            playing: false,
            last_position: 0.0,
            positions: vec![f64::NAN; ctx.max_block.max(1)],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        io.telemetry
            .set(TELEMETRY_MISSING, if self.missing { 1.0 } else { 0.0 });
        let frames = io.frames();
        let len = self.len as f64;
        let looping = io.inputs.boolean(LOOP);
        let speed = io.inputs.float(RATE);
        let events = [
            io.inputs.triggers(REVERSE),
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
        ];

        let position = io.inputs.float(POSITION).first();
        let seek = if position.is_finite() && position != self.last_position {
            self.last_position = position;
            Some(snap_samples(f64::from(position.max(0.0)) * ctx.sample_rate).min(len))
        } else {
            None
        };

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
                    if self.len == 0 {
                        self.playing = false;
                        end.push(run.at as u32);
                    } else {
                        self.playing = true;
                        self.pos = self.beginning();
                    }
                }
                if reset {
                    self.pos = self.beginning();
                }
                if let (0, Some(to)) = (run.at, seek) {
                    self.pos = to;
                }
                for i in run.at..run.until {
                    let mut at = f64::NAN;
                    if self.playing {
                        let r = finite_or(speed.at(i), 1.0).clamp(-MAX_RATE, MAX_RATE);
                        let step = f64::from(if self.forward { r } else { -r });
                        let past_end = if step >= 0.0 {
                            self.pos >= len
                        } else {
                            self.pos < 0.0
                        };
                        if past_end && looping.at(i) && self.len > 0 {
                            self.pos = self.pos.rem_euclid(len);
                        } else if past_end {
                            self.playing = false;
                            end.push(i as u32);
                        }
                        if self.playing {
                            at = self.pos;
                            self.pos += step;
                        }
                    }
                    if let Some(p) = self.positions.get_mut(i) {
                        *p = at;
                    }
                }
            }
        }

        // Pass 2: render each channel from the positions.
        let mut out = io.outputs.audio(OUT);
        let data = self.data.as_deref();
        for c in 0..out.channels() {
            let dst = out.channel(c);
            let src = data.and_then(|d| d.channels.get(c).or_else(|| d.channels.first()));
            let Some(src) = src else {
                dst.fill(0.0);
                continue;
            };
            for (i, o) in dst.iter_mut().enumerate() {
                let p = self.positions.get(i).copied().unwrap_or(f64::NAN);
                *o = if p.is_nan() {
                    0.0
                } else {
                    let whole = p.floor();
                    let t = (p - whole) as f32;
                    let k = whole as isize;
                    let wrap = looping.at(i);
                    hermite4(
                        Self::sample(src, k - 1, wrap),
                        Self::sample(src, k, wrap),
                        Self::sample(src, k + 1, wrap),
                        Self::sample(src, k + 2, wrap),
                        t,
                    )
                };
            }
        }

        let seconds = if self.rate > 0.0 {
            self.pos.clamp(0.0, len) / self.rate
        } else {
            0.0
        };
        io.outputs.set_float(POSITION_OUT, seconds as f32);
        io.outputs.set_bool(PLAYING, self.playing);
    }

    fn reset(&mut self) {
        self.pos = 0.0;
        self.forward = true;
        self.playing = false;
        self.last_position = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::{Harness, peak, tone_magnitude, zero_crossing_freq};
    use ac_core::{NoResources, Resources};
    use serde_json::json;

    /// Resources that serve fixed buffers by path.
    struct Assets(Vec<(&'static str, Arc<AudioData>)>);

    impl Resources for Assets {
        fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
            self.0
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, a)| a.clone())
        }
    }

    fn assets(channels: Vec<Vec<f32>>, rate: u32) -> Assets {
        Assets(vec![(
            "assets/take.wav",
            Arc::new(AudioData {
                sample_rate: rate,
                channels,
            }),
        )])
    }

    /// 1/100, 2/100, ... 100/100.
    fn ramp() -> Vec<f32> {
        (1..=100).map(|i| i as f32 / 100.0).collect()
    }

    fn mono(res: &Assets, block: usize) -> Harness {
        Harness::with_resources::<Clip>(
            json!({ "file": "assets/take.wav", "channels": 1 }),
            48_000.0,
            block,
            res,
        )
    }

    #[test]
    fn declares_its_asset_and_telemetry() {
        let meta = Clip::meta();
        assert_eq!(meta.asset_keys, &["file"]);
        assert_eq!(meta.telemetry, 1);
        let sig = Clip::describe(&json!({ "channels": 3 }));
        assert_eq!(sig.outputs[OUT].channels, Channels::Fixed(3));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "position", "start", "stop", "reset", "loop", "reverse", "rate"
            ]
        );
    }

    #[test]
    fn plays_samples_exactly_at_rate_one_then_ends() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!(&o[..100], ramp().as_slice());
        assert_eq!(peak(&o[100..]), 0.0);
        assert_eq!(out.triggers("end"), &[100]);
        assert_eq!(out.float("playing")[255], 0.0);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }

    #[test]
    fn start_is_sample_accurate() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.stopped().triggers("start", &[37]);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!(peak(&o[..37]), 0.0);
        assert_eq!(&o[37..137], ramp().as_slice());
        assert_eq!(out.triggers("end"), &[137]);
    }

    #[test]
    fn loops_without_ending() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.set("loop", true);
        let out = h.run(300);
        let o = &out.audio("out")[0];
        assert_eq!(&o[100..200], ramp().as_slice());
        assert_eq!(&o[200..300], ramp().as_slice());
        assert!(out.triggers("end").is_empty());
        assert_eq!(out.float("playing")[299], 1.0);
    }

    #[test]
    fn reverse_plays_backwards() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.triggers("reverse", &[0]);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        let backwards: Vec<f32> = ramp().into_iter().rev().collect();
        assert_eq!(&o[..100], backwards.as_slice());
        assert_eq!(out.triggers("end"), &[100]);

        // Flipping mid-play turns around on the spot.
        let mut h = mono(&res, 64);
        h.triggers("reverse", &[50]);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!((o[49], o[50], o[51]), (0.5, 0.51, 0.5));
        assert_eq!(out.triggers("end"), &[101]);
    }

    #[test]
    fn rate_two_halves_the_duration_and_half_interpolates() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.set("rate", 2.0);
        let out = h.run(128);
        let o = &out.audio("out")[0];
        assert_eq!(o[10], 0.21);
        assert_eq!(out.triggers("end"), &[50]);

        let mut h = mono(&res, 64);
        h.set("rate", 0.5);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!(o[20], 0.11);
        // Halfway between 0.11 and 0.12 (Hermite is exact on a straight line).
        assert!((o[21] - 0.115).abs() < 1e-6);
        assert_eq!(out.triggers("end"), &[200]);
    }

    #[test]
    fn stop_silences_and_fires_end() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.set("loop", true).triggers("stop", &[150]);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!(o[149], 0.5);
        assert_eq!(peak(&o[150..]), 0.0);
        assert_eq!(out.triggers("end"), &[150]);
    }

    #[test]
    fn reset_returns_to_the_beginning_and_keeps_playing() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.triggers("reset", &[30]);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!((o[29], o[30]), (0.3, 0.01));
        assert_eq!(out.triggers("end"), &[130]);
    }

    #[test]
    fn position_seeks_and_reports() {
        let long: Vec<f32> = (0..48_000).map(|i| i as f32).collect();
        let res = assets(vec![long], 48_000);
        // A fixed position is where playback starts.
        let mut h = mono(&res, 64);
        h.set("position", 0.25);
        let out = h.run(128);
        assert_eq!(out.audio("out")[0][0], 12_000.0);
        // Changing it seeks at the next block.
        let mut h = mono(&res, 64);
        let pos: Vec<f32> = (0..256).map(|i| if i < 128 { 0.0 } else { 0.5 }).collect();
        h.set_block("position", pos);
        let out = h.run(256);
        let o = &out.audio("out")[0];
        assert_eq!((o[127], o[128], o[129]), (127.0, 24_000.0, 24_001.0));
        let reported = out.float("position")[255];
        assert!(
            (reported - (24_128.0 / 48_000.0)).abs() < 1e-6,
            "{reported}"
        );
    }

    #[test]
    fn resampling_keeps_pitch_and_length() {
        let tone: Vec<f32> = (0..44_100)
            .map(|i| (std::f64::consts::TAU * 1_000.0 * i as f64 / 44_100.0).sin() as f32)
            .collect();
        let res = assets(vec![tone], 44_100);
        let mut h = mono(&res, 512);
        let out = h.run(50_000);
        let o = &out.audio("out")[0];
        let f = zero_crossing_freq(&o[1_000..47_000], 48_000.0);
        assert!((f - 1_000.0).abs() < 0.5, "{f}");
        assert!((tone_magnitude(&o[4_800..43_200], 1_000.0, 48_000.0) - 1.0).abs() < 0.01);
        assert_eq!(out.triggers("end"), &[48_000]);
    }

    #[test]
    fn channels_follow_the_config() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = Harness::with_resources::<Clip>(
            json!({ "file": "assets/take.wav", "channels": 2 }),
            48_000.0,
            64,
            &res,
        );
        let out = h.run(128);
        let o = out.audio("out");
        assert_eq!(o.len(), 2);
        assert_eq!(o[0], o[1]);

        let res = assets(vec![vec![1.0; 10], vec![2.0; 10], vec![3.0; 10]], 48_000);
        let mut h = Harness::with_resources::<Clip>(
            json!({ "file": "assets/take.wav", "channels": 2 }),
            48_000.0,
            64,
            &res,
        );
        let out = h.run(64);
        let o = out.audio("out");
        assert_eq!((o[0][5], o[1][5]), (1.0, 2.0));
    }

    #[test]
    fn missing_file_is_silent_flagged_and_ends_at_once() {
        for config in [
            json!({ "file": "assets/nope.wav" }),
            json!({}),
            json!({ "file": 7 }),
        ] {
            let mut h = Harness::with_resources::<Clip>(
                config,
                48_000.0,
                64,
                &assets(vec![ramp()], 48_000),
            );
            let out = h.run(128);
            assert_eq!(out.audio("out").len(), 2);
            assert!(out.audio("out").iter().flatten().all(|v| *v == 0.0));
            assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
            assert_eq!(out.triggers("end"), &[0]);
            assert_eq!(out.float("playing")[127], 0.0);
        }
        let mut h = Harness::with_resources::<Clip>(json!({}), 48_000.0, 64, &NoResources);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let tone: Vec<f32> = (0..4_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let res = assets(vec![tone], 44_100);
        let speed: Vec<f32> = (0..6_000)
            .map(|i| 1.0 + 0.5 * (i as f32 * 0.001).sin())
            .collect();
        let render = |block: usize| {
            let mut h = mono(&res, block);
            h.set_samples("rate", speed.clone())
                .set("loop", true)
                .triggers("reverse", &[1_000, 2_500])
                .triggers("stop", &[5_000]);
            let out = h.run(6_000);
            (out.audio("out")[0].clone(), out.triggers("end").to_vec())
        };
        let one = render(1);
        assert_eq!(one.1, vec![5_000]);
        assert_eq!(one, render(64));
        assert_eq!(one, render(1_024));
    }

    #[test]
    fn extreme_rates_stay_finite() {
        let res = assets(vec![ramp()], 48_000);
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 3.7];
        let wild: Vec<f32> = wild.iter().cycle().take(1_000).copied().collect();
        let mut h = mono(&res, 64);
        h.set_samples("rate", wild.clone())
            .set_samples("position", wild)
            .set("loop", true);
        let out = h.run(1_000);
        assert!(out.audio("out")[0].iter().all(|v| v.is_finite()));
        assert!(out.float("position").iter().all(|v| v.is_finite()));
    }

    #[test]
    fn transport_reset_starts_over() {
        let res = assets(vec![ramp()], 48_000);
        let mut h = mono(&res, 64);
        h.run(64);
        h.reset();
        h.play();
        let out = h.run(64);
        assert_eq!(out.audio("out")[0][0], 0.01);
    }
}
