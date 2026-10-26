//! The transport's click (DECISIONS D207): a short sine blip on every beat,
//! higher on the first beat of a bar, added to the device's first two
//! outputs. It's on the monitor path: live only, never in a render or a
//! File Out, since it isn't part of the graph.

/// How long a blip rings.
const SECONDS: f64 = 0.03;
/// Peak level, about −9 dBFS.
const LEVEL: f32 = 0.35;
const ACCENT_HZ: f64 = 1_600.0;
const BEAT_HZ: f64 = 1_000.0;

/// One blip at a time; a new beat cuts the last one short.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Click {
    /// Phase in cycles, and its step per sample.
    phase: f64,
    step: f64,
    /// Samples left of the blip, and its full length.
    left: u32,
    len: u32,
}

impl Click {
    /// Start a blip (accented on a bar's first beat) at `rate` Hz.
    pub fn strike(&mut self, accent: bool, rate: f64) {
        let rate = if rate > 0.0 { rate } else { 48_000.0 };
        self.phase = 0.0;
        self.step = if accent { ACCENT_HZ } else { BEAT_HZ } / rate;
        self.len = (SECONDS * rate).round().max(1.0) as u32;
        self.left = self.len;
    }

    /// Add frames `from..to` of the blip to the first two of `channels`
    /// channels of `out`, laid out channel by channel `stride` apart.
    pub fn render(
        &mut self,
        out: &mut [f32],
        channels: usize,
        stride: usize,
        from: usize,
        to: usize,
    ) {
        for i in from..to {
            if self.left == 0 {
                return;
            }
            // A cubic fade: bright at the strike, gone by the end.
            let fade = self.left as f32 / self.len as f32;
            let s = LEVEL * fade * fade * fade * (std::f64::consts::TAU * self.phase).sin() as f32;
            for c in 0..channels.min(2) {
                if let Some(o) = out.get_mut(c * stride + i) {
                    *o += s;
                }
            }
            self.phase = (self.phase + self.step).fract();
            self.left -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blip_rings_briefly_and_the_accent_is_higher() {
        let mut out = vec![0.0f32; 2 * 4_800];
        let mut c = Click::default();
        c.strike(false, 48_000.0);
        c.render(&mut out, 2, 4_800, 0, 4_800);
        let rung = out[..4_800].iter().rposition(|s| *s != 0.0).unwrap_or(0);
        assert!((1_000..=1_440).contains(&rung), "{rung}");
        assert_eq!(out[..4_800], out[4_800..]);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2 && peak <= LEVEL, "{peak}");
        // Zero crossings in the first millisecond: 1 kHz has two, 1.6 kHz three.
        let crossings = |accent: bool| {
            let mut out = vec![0.0f32; 96];
            let mut c = Click::default();
            c.strike(accent, 48_000.0);
            c.render(&mut out, 1, 96, 0, 96);
            out.windows(2).filter(|w| w[0] > 0.0 && w[1] <= 0.0).count()
                + out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
        };
        assert!(crossings(true) > crossings(false));
    }
}
