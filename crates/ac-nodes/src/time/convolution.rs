use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, node,
};
use ac_dsp::fft::Fft;
use ac_dsp::interp::hermite4;
use ac_dsp::util::db_to_gain;

use crate::generators::clip::prepare;

node! {
    kind: "convolution",
    name: "Convolution",
    category: Time,
    keywords: ["convolve", "impulse response", "ir", "reverb", "cabinet", "cab", "room", "fir"],
    /// Plays its input through a recorded *impulse response*: the sound of
    /// a room, a speaker cabinet, a spring, or any filter, captured as the
    /// echo of a single click. Every sample of the input sets off a copy
    /// of the response, scaled by that sample, and the copies add up
    /// (convolution), so the input sounds as if it were played in that
    /// room or through that cabinet.
    ///
    /// **Response.** A wired `buffer` (a `Record` node's take, say) is the
    /// response; with `buffer` unwired, the `file` config (an audio file in
    /// the project) is, converted to the engine's rate when the project
    /// loads. A buffer at another rate is converted with cubic
    /// interpolation as it comes in. Responses are used as they are,
    /// without normalizing (`gain` sets the level), and are cut at
    /// `max_length` seconds (config, default 5, up to 60). A mono response
    /// is used for every channel; a stereo one gives the left output its
    /// left channel and the right its right. Nothing to convolve with
    /// gives silence.
    ///
    /// **Latency.** The work is done in blocks of `partition` samples
    /// (config, a power of two from 64 to 4096, default 256), so the output
    /// comes `partition` samples late (5.3 ms at 48 kHz by default). The
    /// node reports it, and the engine delays parallel paths to match, so
    /// the dry signal mixed back in with a `Mix` lines up. Smaller
    /// partitions mean less delay and more CPU.
    ///
    /// **Changing the response.** A new buffer takes over one partition at
    /// a time, so the cost is spread out: sound that came in before the
    /// change finishes ringing with the old response, and sound after it
    /// uses the new one.
    ///
    /// The output has `channels` channels (config, 1 or 2, default 2); a
    /// mono input feeds both. NaN or infinite input samples count as
    /// silence.
    ///
    /// **Try this:** load a room's impulse response into `file`, feed a dry
    /// `Clip` into `in`, and mix the Convolution's output with the dry clip
    /// at about −12 dB in a `Mix` for a natural reverb; or record yourself
    /// clapping in a stairwell into a `Record` node and wire its `buffer`
    /// here.
    pub struct Convolution {
        rate: f64,
        /// Partition length (B); the FFT is 2B.
        part: usize,
        /// Partitions reserved (from `max_length`).
        max_parts: usize,
        fft: Fft,
        channels: usize,
        file: Option<Arc<AudioData>>,
        missing: bool,
        /// The response in use (or coming in), to notice a new one.
        current: Option<Arc<AudioData>>,
        /// The response's partitions, per channel (two, a mono response
        /// copied), each `B + 1` bins, real and imaginary parts apart so the
        /// multiply-accumulate runs on whole vectors.
        ir: [Planes; 2],
        /// Partitions with anything in them.
        active: usize,
        /// A response coming in partition by partition.
        loading: Option<Load>,
        /// Spectra of the last `max_parts` input blocks, per channel.
        fdl: [Planes; 2],
        head: usize,
        /// The last two input blocks (overlap-save), per channel.
        input: [Vec<f32>; 2],
        /// The output block being played out, per channel.
        output: [Vec<f32>; 2],
        fill: usize,
        time: Vec<f32>,
        /// One spectrum, interleaved as the FFT reads and writes it.
        spec: Vec<f32>,
        acc: Planes,
    }
}

/// Spectra stored as planes: `re[k * m + j]`, `im[k * m + j]` for bin `j`
/// of spectrum `k`, `m` bins each.
#[derive(Clone, Debug, Default)]
struct Planes {
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Planes {
    fn new(len: usize) -> Planes {
        Planes {
            re: vec![0.0; len],
            im: vec![0.0; len],
        }
    }

    /// Spectrum `k` of `m` bins, read-only.
    fn get(&self, k: usize, m: usize) -> Option<(&[f32], &[f32])> {
        Some((
            self.re.get(k * m..(k + 1) * m)?,
            self.im.get(k * m..(k + 1) * m)?,
        ))
    }

    /// Store the interleaved spectrum `spec` as spectrum `k`.
    fn put(&mut self, k: usize, m: usize, spec: &[f32]) {
        let (Some(re), Some(im)) = (
            self.re.get_mut(k * m..(k + 1) * m),
            self.im.get_mut(k * m..(k + 1) * m),
        ) else {
            return;
        };
        for ((r, i), pair) in re.iter_mut().zip(im.iter_mut()).zip(spec.chunks_exact(2)) {
            *r = pair[0];
            *i = pair[1];
        }
    }

    fn clear(&mut self) {
        self.re.fill(0.0);
        self.im.fill(0.0);
    }
}

const IN: usize = 0;
const BUFFER: usize = 1;
const GAIN: usize = 2;
const OUT: usize = 0;

/// Telemetry slot: 1.0 when the `file` config couldn't be loaded.
pub const TELEMETRY_MISSING: usize = 0;

const DEFAULT_PART: usize = 256;
const DEFAULT_MAX_LENGTH: f64 = 5.0;
const LONGEST: f64 = 60.0;
const MAX_GAIN_DB: f32 = 24.0;

/// A response being transformed in, one partition per partition period.
#[derive(Clone, Debug)]
struct Load {
    src: Option<Arc<AudioData>>,
    /// The next partition to transform.
    next: usize,
    /// Partitions the new response has.
    parts: usize,
}

fn partition(config: &Config) -> usize {
    let p = config.get_f64_or("partition", DEFAULT_PART as f64);
    if !p.is_finite() {
        return DEFAULT_PART;
    }
    (p.clamp(64.0, 4096.0) as usize)
        .next_power_of_two()
        .min(4096)
}

fn max_length(config: &Config) -> f64 {
    let s = config.get_f64_or("max_length", DEFAULT_MAX_LENGTH);
    if s.is_finite() {
        s.clamp(0.01, LONGEST)
    } else {
        DEFAULT_MAX_LENGTH
    }
}

fn channels(config: &Config) -> usize {
    config.get_u64_or("channels", 2).clamp(1, 2) as usize
}

/// Sample `k` of `ch`, silent outside it.
#[inline]
fn sample(ch: &[f32], k: isize) -> f32 {
    if k < 0 {
        return 0.0;
    }
    ch.get(k as usize).copied().unwrap_or(0.0)
}

impl Convolution {
    /// Bins per spectrum.
    fn bins(&self) -> usize {
        self.part + 1
    }

    /// Partitions a response needs at the engine rate, up to the reserve.
    fn parts_of(&self, src: Option<&AudioData>) -> usize {
        let Some(d) = src else {
            return 0;
        };
        let frames = d.frames() as f64;
        let ratio = self.rate / f64::from(d.sample_rate.max(1));
        let len = (frames * ratio).ceil();
        ((len / self.part as f64).ceil() as usize).min(self.max_parts)
    }

    /// Transform partition `k` of `src` (at the engine rate) into both
    /// channels' response spectra; past its end, the partition is silent.
    fn transform(&mut self, src: Option<&AudioData>, k: usize) {
        let (b, m) = (self.part, self.bins());
        for ch in 0..2 {
            let Some(d) = src.filter(|d| d.frames() > 0) else {
                self.spec.fill(0.0);
                self.ir[ch].put(k, m, &self.spec);
                continue;
            };
            let data = d
                .channels
                .get(ch)
                .or_else(|| d.channels.first())
                .map_or(&[][..], Vec::as_slice);
            let step = f64::from(d.sample_rate.max(1)) / self.rate;
            for (j, t) in self.time.iter_mut().enumerate() {
                *t = if j >= b {
                    0.0
                } else if step == 1.0 {
                    sample(data, (k * b + j) as isize)
                } else {
                    let p = (k * b + j) as f64 * step;
                    let whole = p.floor();
                    let f = (p - whole) as f32;
                    let i = whole as isize;
                    hermite4(
                        sample(data, i - 1),
                        sample(data, i),
                        sample(data, i + 1),
                        sample(data, i + 2),
                        f,
                    )
                };
            }
            self.fft.forward(&self.time, &mut self.spec);
            self.ir[ch].put(k, m, &self.spec);
        }
    }

    /// One partition period: bring in a partition of a new response if one
    /// is loading, transform the newest input block, and work out the next
    /// output block (uniformly partitioned overlap-save with a frequency-
    /// domain delay line: F. Wefers, "Partitioned convolution algorithms
    /// for real-time auralization", Logos, 2015, §5; after J. S. Soo and
    /// K. K. Pang, "Multidelay block frequency domain adaptive filter",
    /// IEEE Trans. ASSP 38(2), 1990).
    fn period(&mut self) {
        let mut sum = self.active;
        if let Some(mut load) = self.loading.take() {
            let k = load.next;
            self.transform(load.src.as_deref(), k);
            load.next += 1;
            sum = sum.max(load.parts);
            if load.next >= self.active.max(load.parts) {
                self.active = load.parts;
            } else {
                self.loading = Some(load);
            }
        }
        let (b, m, p) = (self.part, self.bins(), self.max_parts.max(1));
        self.head = (self.head + 1) % p;
        for c in 0..self.channels {
            self.fft.forward(&self.input[c], &mut self.spec);
            self.fdl[c].put(self.head, m, &self.spec);
            self.acc.clear();
            let (ar, ai) = (&mut self.acc.re[..], &mut self.acc.im[..]);
            for k in 0..sum.min(p) {
                let slot = (self.head + p - k) % p;
                let (Some((xr, xi)), Some((hr, hi))) =
                    (self.fdl[c].get(slot, m), self.ir[c].get(k, m))
                else {
                    continue;
                };
                for j in 0..m.min(ar.len()).min(ai.len()) {
                    ar[j] += xr[j] * hr[j] - xi[j] * hi[j];
                    ai[j] += xr[j] * hi[j] + xi[j] * hr[j];
                }
            }
            for ((pair, r), i) in self.spec.chunks_exact_mut(2).zip(ar.iter()).zip(ai.iter()) {
                pair[0] = *r;
                pair[1] = *i;
            }
            self.fft.inverse(&self.spec, &mut self.time);
            // The second half is the linear part of the circular result.
            self.output[c].copy_from_slice(&self.time[b..2 * b]);
            self.input[c].copy_within(b..2 * b, 0);
        }
    }
}

impl Node for Convolution {
    fn meta() -> NodeMeta {
        Self::base_meta().assets(&["file"]).telemetry(1)
    }

    fn describe(config: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to play through the response."))
            .input(
                SocketSpec::buffer("buffer")
                    .doc("Impulse response; unwired, the `file` config is used."),
            )
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Level of the result in decibels."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(channels(config) as u32))
                    .doc("The input through the response, `partition` samples late."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let rate = ctx.sample_rate.max(1.0);
        let part = partition(config);
        let max_parts = ((max_length(config) * rate / part as f64).ceil() as usize).max(1);
        let m = part + 1;
        let path = config.get_str_or("file", "");
        let asset = if path.is_empty() {
            None
        } else {
            ctx.resources.audio_asset(path)
        };
        let missing = !path.is_empty() && asset.is_none();
        let file = asset.map(|a| prepare(a, 2, rate));
        let mut node = Convolution {
            rate,
            part,
            max_parts,
            fft: Fft::new(2 * part),
            channels: channels(config),
            file: file.clone(),
            missing,
            current: file.clone(),
            ir: [Planes::new(max_parts * m), Planes::new(max_parts * m)],
            active: 0,
            loading: None,
            fdl: [Planes::new(max_parts * m), Planes::new(max_parts * m)],
            head: 0,
            input: [vec![0.0; 2 * part], vec![0.0; 2 * part]],
            output: [vec![0.0; part], vec![0.0; part]],
            fill: 0,
            time: vec![0.0; 2 * part],
            spec: vec![0.0; 2 * m],
            acc: Planes::new(m),
        };
        // The file's response is ready from the start.
        let parts = node.parts_of(file.as_deref());
        for k in 0..parts {
            node.transform(file.as_deref(), k);
        }
        node.active = parts;
        node
    }

    fn latency(&self) -> u32 {
        self.part as u32
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let wired = io.inputs.is_wired(BUFFER);
        io.telemetry.set(
            TELEMETRY_MISSING,
            if self.missing && !wired { 1.0 } else { 0.0 },
        );
        let want = if wired {
            io.inputs.buffer(BUFFER)
        } else {
            self.file.as_ref()
        };
        let same = match (want, self.current.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            // Handles are cloned and dropped here, never freed: whoever made
            // the buffer (or this node, for the file) keeps one.
            self.current = want.cloned();
            let parts = self.parts_of(want.map(|a| &**a));
            self.loading = Some(Load {
                src: want.cloned(),
                next: 0,
                parts,
            });
        }
        let input = io.inputs.audio(IN);
        let gain = io.inputs.float(GAIN);
        let frames = io.frames();
        let n_in = input.channels().max(1);
        let mut out = io.outputs.audio(OUT);
        let n_out = out.channels().min(self.channels);
        for i in 0..frames {
            let db = gain.at(i);
            let g = if db.is_finite() {
                db_to_gain(db.min(MAX_GAIN_DB))
            } else {
                0.0
            };
            let at = self.part + self.fill;
            for c in 0..n_out {
                let x = input.channel(c % n_in).get(i).copied().unwrap_or(0.0);
                if let Some(s) = self.input[c].get_mut(at) {
                    *s = if x.is_finite() { x } else { 0.0 };
                }
                let y = self.output[c].get(self.fill).copied().unwrap_or(0.0) * g;
                if let Some(o) = out.channel(c).get_mut(i) {
                    *o = if y.is_finite() { y } else { 0.0 };
                }
            }
            self.fill += 1;
            if self.fill >= self.part {
                self.fill = 0;
                self.period();
            }
        }
    }

    fn reset(&mut self) {
        for c in 0..2 {
            self.fdl[c].clear();
            self.input[c].fill(0.0);
            self.output[c].fill(0.0);
        }
        self.fill = 0;
        self.head = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, Resources};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn data(channels: Vec<Vec<f32>>, rate: u32) -> Arc<AudioData> {
        Arc::new(AudioData {
            sample_rate: rate,
            channels,
        })
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.wrapping_mul(2_654_435_761).max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect()
    }

    /// The plain sum `y[n] = Σ x[m] h[n − m]`.
    fn direct(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0f64; x.len()];
        for (n, o) in y.iter_mut().enumerate() {
            for (m, &hv) in h.iter().enumerate().take(n + 1) {
                *o += f64::from(x[n - m]) * f64::from(hv);
            }
        }
        y.into_iter().map(|v| v as f32).collect()
    }

    fn convolve(
        config: serde_json::Value,
        ir: Arc<AudioData>,
        x: Vec<f32>,
        block: usize,
    ) -> Vec<Vec<f32>> {
        let n = x.len();
        let mut h = Harness::with_resources::<Convolution>(config, RATE, block, &NoResources);
        h.buffer("buffer", ir).set_audio("in", vec![x]);
        h.run(n).audio("out").to_vec()
    }

    #[test]
    fn it_matches_the_direct_sum_late_by_one_partition() {
        let ir = noise(1_000, 3);
        let x = noise(6_000, 4);
        // The response arrives one partition at a time; once it's all in,
        // the output is the convolution `partition` samples late.
        let y = convolve(
            json!({ "partition": 128 }),
            data(vec![ir.clone()], 48_000),
            x.clone(),
            64,
        );
        let want = direct(&x, &ir);
        let start = 128 * 10;
        for n in start..x.len() {
            assert!(
                (y[0][n] - want[n - 128]).abs() < 1e-3,
                "{n}: {} vs {}",
                y[0][n],
                want[n - 128]
            );
        }
        assert_eq!(y[0], y[1], "a mono response feeds both channels");
    }

    #[test]
    fn a_file_response_is_ready_at_once_and_an_impulse_comes_back_as_it() {
        struct One(Arc<AudioData>);
        impl Resources for One {
            fn audio_asset(&self, _: &str) -> Option<Arc<AudioData>> {
                Some(self.0.clone())
            }
        }
        let ir = noise(700, 5);
        let file = One(data(vec![ir.clone()], 48_000));
        let mut h = Harness::with_resources::<Convolution>(
            json!({ "file": "assets/room.wav" }),
            RATE,
            64,
            &file,
        );
        let mut x = vec![0.0; 2_000];
        x[0] = 1.0;
        h.set_audio("in", vec![x]);
        let y = h.run(2_000).audio("out")[0].clone();
        assert!(y[..256].iter().all(|v| v.abs() < 1e-6));
        for (k, v) in ir.iter().enumerate() {
            assert!((y[256 + k] - v).abs() < 1e-5, "{k}");
        }
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }

    #[test]
    fn it_reports_its_partition_as_latency() {
        for (config, want) in [
            (json!({}), 256),
            (json!({ "partition": 1024 }), 1024),
            (json!({ "partition": 100 }), 128),
            (json!({ "partition": 1e9 }), 4096),
            (json!({ "partition": "big" }), 256),
        ] {
            let node = Convolution::new(
                &config,
                &PrepareCtx {
                    sample_rate: RATE,
                    max_block: 64,
                    seed: 0,
                    node_key: "c",
                    name: "c",
                    resources: &NoResources,
                },
            );
            assert_eq!(node.latency(), want, "{config}");
        }
    }

    #[test]
    fn a_stereo_response_gives_each_side_its_own() {
        let (l, r) = (noise(300, 7), noise(300, 8));
        let mut x = vec![0.0; 3_000];
        x[1_000] = 1.0;
        let y = convolve(
            json!({}),
            data(vec![l.clone(), r.clone()], 48_000),
            x.clone(),
            64,
        );
        for k in 0..300 {
            assert!((y[0][1_256 + k] - l[k]).abs() < 1e-5);
            assert!((y[1][1_256 + k] - r[k]).abs() < 1e-5);
        }
        // One channel: the left of the response.
        let y1 = convolve(
            json!({ "channels": 1 }),
            data(vec![l.clone(), r], 48_000),
            x,
            64,
        );
        assert_eq!(y1.len(), 1);
        assert!((y1[0][1_256] - l[0]).abs() < 1e-5);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let ir = data(vec![noise(2_000, 9)], 48_000);
        let x = noise(10_000, 10);
        let one = convolve(json!({}), ir.clone(), x.clone(), 1);
        assert_eq!(one, convolve(json!({}), ir.clone(), x.clone(), 64));
        assert_eq!(one, convolve(json!({}), ir, x, 1024));
    }

    #[test]
    fn a_new_response_takes_over_without_cutting_the_old_tail() {
        // Response A rings for 2 000 samples; response B is a single click.
        // The input strikes once before the change and once after.
        let a = data(vec![vec![0.5; 2_000]], 48_000);
        let b = data(
            vec![{
                let mut v = vec![0.0; 10];
                v[0] = 1.0;
                v
            }],
            48_000,
        );
        let mut x = vec![0.0; 12_000];
        x[100] = 1.0;
        x[6_000] = 1.0;
        let mut h = Harness::new::<Convolution>(json!({ "partition": 256 }));
        h.set_audio("in", vec![x]).buffer("buffer", a);
        let mut y = h.run(5_120).audio("out")[0].clone();
        h.buffer("buffer", b);
        y.extend(h.run(6_880).audio("out")[0].clone());
        // The first strike still rings with A, its whole length.
        assert!(y[356..2_356].iter().all(|v| (v - 0.5).abs() < 1e-5));
        // The second gets B: one click, nothing after.
        assert!((y[6_256] - 1.0).abs() < 1e-5, "{}", y[6_256]);
        assert!(y[6_257..].iter().all(|v| v.abs() < 1e-5));
    }

    #[test]
    fn a_response_at_another_rate_is_converted() {
        // A click 10 samples in at 24 kHz is 20 samples in at 48 kHz.
        let mut ir = vec![0.0; 100];
        ir[10] = 1.0;
        let mut x = vec![0.0; 4_000];
        x[2_000] = 1.0;
        let y = convolve(json!({}), data(vec![ir], 24_000), x, 64);
        let top = (0..4_000)
            .max_by(|&i, &j| y[0][i].abs().total_cmp(&y[0][j].abs()))
            .unwrap_or(0);
        assert_eq!(top, 2_000 + 256 + 20);
    }

    #[test]
    fn long_responses_are_cut_and_nothing_is_silence() {
        // max_length 0.01 s at 48 kHz: two partitions of 256 (512 samples).
        let ir = data(vec![vec![0.1; 5_000]], 48_000);
        let mut x = vec![0.0; 8_000];
        x[4_000] = 1.0;
        let y = convolve(json!({ "max_length": 0.01 }), ir, x.clone(), 64);
        assert!((y[0][4_256 + 400] - 0.1).abs() < 1e-5);
        assert!(y[0][4_256 + 600].abs() < 1e-6);
        // No response at all, an empty one, and bad input.
        let mut h = Harness::new::<Convolution>(json!({}));
        h.set_audio("in", vec![noise(2_000, 1)]);
        assert!(h.run(2_000).audio("out")[0].iter().all(|v| *v == 0.0));
        let mut bad = noise(4_000, 2);
        bad[10] = f32::NAN;
        bad[11] = f32::INFINITY;
        let y = convolve(json!({}), data(vec![], 48_000), bad.clone(), 64);
        assert!(y[0].iter().all(|v| *v == 0.0));
        let y = convolve(json!({}), data(vec![noise(500, 3)], 48_000), bad, 64);
        assert!(y.iter().flatten().all(|v| v.is_finite()));
        // A missing file is flagged.
        let mut h = Harness::new::<Convolution>(json!({ "file": "assets/none.wav" }));
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
    }

    #[test]
    fn reset_clears_what_is_ringing() {
        let ir = data(vec![vec![0.5; 3_000]], 48_000);
        let mut h = Harness::new::<Convolution>(json!({}));
        let mut x = vec![0.0; 4_000];
        x[3_000] = 1.0;
        h.buffer("buffer", ir).set_audio("in", vec![x]);
        h.run(4_000);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 4_000]]);
        assert!(h.run(4_000).audio("out")[0].iter().all(|v| *v == 0.0));
    }
}
