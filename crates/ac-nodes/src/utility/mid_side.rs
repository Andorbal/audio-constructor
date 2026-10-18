use ac_core::{
    Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};

node! {
    kind: "mid_side",
    name: "Mid/Side",
    category: Utility,
    keywords: ["m/s", "ms", "stereo width", "stereo", "wide", "sum difference", "imager"],
    /// Converts stereo to mid (what both sides share) and side (how they
    /// differ), or back, so the center and the edges of a mix can be
    /// processed separately.
    ///
    /// Config `mode`:
    /// - `"encode"` (default): `in` (stereo; a mono input has no side) to
    ///   `mid` = (L + R) / 2 and `side` = (L − R) / 2, each mono.
    /// - `"decode"`: `mid` and `side` (mono each) to `out` (stereo),
    ///   L = mid + `width` × side and R = mid − `width` × side. `width` (0
    ///   to 2, clamped) scales the side: 0 collapses to mono, 1 restores the
    ///   original image, 2 doubles the stereo spread.
    ///
    /// Encoding and decoding at `width` 1 gives back exactly what went in
    /// (bit for bit for 16-bit sources; within the last bit of a 32-bit
    /// float otherwise). Unknown modes are treated as `"encode"`.
    ///
    /// **Try this:** encode a stereo mix, put a `Biquad` highpass at 150 Hz
    /// on `side` only (bass stays centered and punchy), and decode; or
    /// decode with `width` from a `Slider` for a stereo width control.
    pub struct MidSide {
        decode: bool,
    }
}

/// Encode: input and outputs.
const IN: usize = 0;
const MID_OUT: usize = 0;
const SIDE_OUT: usize = 1;
/// Decode: inputs and output.
const MID_IN: usize = 0;
const SIDE_IN: usize = 1;
const WIDTH: usize = 2;
const OUT: usize = 0;

const MAX_WIDTH: f32 = 2.0;

fn is_decode(config: &Config) -> bool {
    config.get_str_or("mode", "encode") == "decode"
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Node for MidSide {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mono = Channels::Fixed(1);
        let stereo = Channels::Fixed(2);
        if is_decode(config) {
            Signature::new()
                .input(
                    SocketSpec::audio("mid")
                        .channels(mono)
                        .doc("What both sides share."),
                )
                .input(
                    SocketSpec::audio("side")
                        .channels(mono)
                        .doc("How the sides differ."),
                )
                .input(
                    SocketSpec::float("width", Unit::Generic, 1.0)
                        .range(0.0, f64::from(MAX_WIDTH))
                        .doc("Side level: 0 mono, 1 original, 2 extra wide."),
                )
                .output(
                    SocketSpec::audio("out")
                        .channels(stereo)
                        .doc("Left and right."),
                )
        } else {
            Signature::new()
                .input(
                    SocketSpec::audio("in")
                        .channels(stereo)
                        .doc("Stereo signal to split."),
                )
                .output(SocketSpec::audio("mid").channels(mono).doc("(L + R) / 2."))
                .output(SocketSpec::audio("side").channels(mono).doc("(L − R) / 2."))
        }
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        MidSide {
            decode: is_decode(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        if self.decode {
            let mid = io.inputs.audio(MID_IN).channel(0);
            let side = io.inputs.audio(SIDE_IN).channel(0);
            let width = io.inputs.float(WIDTH);
            let w0 = finite_or(width.first(), 1.0).clamp(0.0, MAX_WIDTH);
            let mut out = io.outputs.audio(OUT);
            let Some((left, right)) = out.stereo() else {
                return;
            };
            for (i, ((l, r), (&m, &s))) in left
                .iter_mut()
                .zip(right.iter_mut())
                .zip(mid.iter().zip(side))
                .enumerate()
            {
                let w = if width.is_varying() {
                    finite_or(width.at(i), 1.0).clamp(0.0, MAX_WIDTH)
                } else {
                    w0
                };
                let (m, s) = (finite_or(m, 0.0), finite_or(s, 0.0) * w);
                *l = m + s;
                *r = m - s;
            }
        } else {
            let input = io.inputs.audio(IN);
            let (left, right) = (input.channel(0), input.channel(1));
            {
                let mut mid = io.outputs.audio(MID_OUT);
                for (o, (&l, &r)) in mid.channel(0).iter_mut().zip(left.iter().zip(right)) {
                    *o = (finite_or(l, 0.0) + finite_or(r, 0.0)) * 0.5;
                }
            }
            let mut side = io.outputs.audio(SIDE_OUT);
            for (o, (&l, &r)) in side.channel(0).iter_mut().zip(left.iter().zip(right)) {
                *o = (finite_or(l, 0.0) - finite_or(r, 0.0)) * 0.5;
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed.max(1);
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn encode(l: Vec<f32>, r: Vec<f32>) -> (Vec<f32>, Vec<f32>) {
        let n = l.len();
        let mut h = Harness::new::<MidSide>(json!({}));
        h.set_audio("in", vec![l, r]);
        let out = h.run(n);
        (out.audio("mid")[0].clone(), out.audio("side")[0].clone())
    }

    fn decode(m: Vec<f32>, s: Vec<f32>, width: f64) -> (Vec<f32>, Vec<f32>) {
        let n = m.len();
        let mut h = Harness::new::<MidSide>(json!({ "mode": "decode" }));
        h.set_audio("mid", vec![m])
            .set_audio("side", vec![s])
            .set("width", width);
        let out = h.run(n);
        (out.audio("out")[0].clone(), out.audio("out")[1].clone())
    }

    #[test]
    fn sockets_follow_the_mode() {
        let names = |c: Config| {
            let sig = MidSide::describe(&c);
            let ins: Vec<String> = sig.inputs.iter().map(|s| s.name.to_string()).collect();
            let outs: Vec<String> = sig.outputs.iter().map(|s| s.name.to_string()).collect();
            (ins, outs)
        };
        assert_eq!(
            names(json!({})),
            (vec!["in".into()], vec!["mid".into(), "side".into()])
        );
        assert_eq!(names(json!({ "mode": "nonsense" })), names(json!({})));
        assert_eq!(
            names(json!({ "mode": "decode" })),
            (
                vec!["mid".into(), "side".into(), "width".into()],
                vec!["out".into()]
            )
        );
    }

    #[test]
    fn encodes_and_decodes_by_the_formulas() {
        let (m, s) = encode(vec![1.0, 0.0, 0.5], vec![0.5, 1.0, 0.5]);
        assert_eq!(m, vec![0.75, 0.5, 0.5]);
        assert_eq!(s, vec![0.25, -0.5, 0.0]);
        let (l, r) = decode(vec![0.75, 0.5], vec![0.25, -0.5], 1.0);
        assert_eq!((l, r), (vec![1.0, 0.0], vec![0.5, 1.0]));
        // Width 0 is mono, 2 doubles the side.
        let (l, r) = decode(vec![0.5], vec![0.25], 0.0);
        assert_eq!((l, r), (vec![0.5], vec![0.5]));
        let (l, r) = decode(vec![0.5], vec![0.25], 2.0);
        assert_eq!((l, r), (vec![1.0], vec![0.0]));
    }

    #[test]
    fn a_mono_input_has_no_side() {
        let x = noise(256, 3);
        let mut h = Harness::new::<MidSide>(json!({}));
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(256);
        assert_eq!(out.audio("mid")[0], x);
        assert!(out.audio("side")[0].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn round_trip_is_exact() {
        // 16-bit sources come back bit for bit.
        let q = |x: Vec<f32>| -> Vec<f32> {
            x.iter().map(|v| (v * 32767.0).round() / 32768.0).collect()
        };
        let (l, r) = (q(noise(4_096, 1)), q(noise(4_096, 2)));
        let (m, s) = encode(l.clone(), r.clone());
        let (l2, r2) = decode(m, s, 1.0);
        assert_eq!(l2, l);
        assert_eq!(r2, r);
        // Arbitrary floats come back within the last bit.
        let (l, r) = (noise(4_096, 5), noise(4_096, 6));
        let (m, s) = encode(l.clone(), r.clone());
        let (l2, r2) = decode(m, s, 1.0);
        for (a, b) in l.iter().zip(&l2).chain(r.iter().zip(&r2)) {
            assert!((a - b).abs() <= f32::EPSILON * 2.0, "{a} {b}");
        }
    }

    #[test]
    fn bad_values_stay_finite() {
        let wild = vec![f32::NAN, f32::INFINITY, -1.0, 0.5];
        let (m, s) = encode(wild.clone(), wild.iter().rev().copied().collect());
        assert!(m.iter().chain(&s).all(|v| v.is_finite()));
        let n = wild.len();
        let mut h = Harness::new::<MidSide>(json!({ "mode": "decode" }));
        h.set_audio("mid", vec![wild.clone()])
            .set_audio("side", vec![wild.clone()])
            .set_samples("width", wild);
        let out = h.run(n);
        assert!(out.audio("out").iter().flatten().all(|v| v.is_finite()));
    }
}
