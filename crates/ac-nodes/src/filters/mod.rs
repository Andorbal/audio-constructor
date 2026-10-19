//! filters nodes.

use ac_core::{Config, ConfigExt, FloatIn, Registry};
use ac_dsp::smooth::BlockRamp;
use ac_dsp::util::snap_samples;

pub mod allpass;
pub mod biquad;
pub mod comb;
pub mod dc_block;
pub mod formant;
pub mod ladder;
pub mod svf;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<biquad::Biquad>();
    r.register::<svf::Svf>();
    r.register::<ladder::Ladder>();
    r.register::<comb::Comb>();
    r.register::<allpass::Allpass>();
    r.register::<dc_block::DcBlock>();
    r.register::<formant::Formant>();
}

/// Default `max_delay` for the delay-based filters, in seconds.
const DEFAULT_MAX_DELAY: f64 = 0.1;
/// Longest `max_delay` accepted, in seconds.
const LONGEST_MAX_DELAY: f64 = 2.0;

/// The `max_delay` config in seconds: default 0.1, clamped to 1 ms..2 s.
pub(crate) fn max_delay(config: &Config) -> f64 {
    let t = config.get_f64_or("max_delay", DEFAULT_MAX_DELAY);
    if t.is_finite() {
        t.clamp(0.001, LONGEST_MAX_DELAY)
    } else {
        DEFAULT_MAX_DELAY
    }
}

/// Samples a line needs for `max_delay(config)` at `rate` (at least 1).
pub(crate) fn line_len(config: &Config, rate: f64) -> usize {
    let n = (max_delay(config) * rate).ceil();
    if n.is_finite() && n >= 1.0 {
        n as usize
    } else {
        1
    }
}

/// Per-frame delay times in samples, from a `Seconds` input: followed
/// every sample when the input moves within the block, otherwise glided
/// linearly from the last block's value (so a block-rate control doesn't
/// step), as the Delay node does. Times within `f32` rounding of a whole
/// sample snap to it (DECISIONS D32).
#[derive(Clone, Debug)]
pub(crate) struct DelayTimes {
    delays: Vec<f64>,
    ramp: BlockRamp,
}

impl DelayTimes {
    pub(crate) fn new(max_block: usize) -> Self {
        DelayTimes {
            delays: vec![1.0; max_block.max(1)],
            ramp: BlockRamp::new(),
        }
    }

    pub(crate) fn fill(&mut self, time: FloatIn, rate: f64, frames: usize) {
        let to_samples = |t: f32| {
            let t = if t.is_finite() { f64::from(t) } else { 0.0 };
            snap_samples(t * rate)
        };
        if time.is_varying() {
            let mut last = to_samples(time.first());
            for (i, d) in self.delays.iter_mut().take(frames).enumerate() {
                last = to_samples(time.at(i));
                *d = last;
            }
            self.ramp.set(last);
        } else {
            let (start, step) = self.ramp.begin(to_samples(time.first()), frames);
            for (i, d) in self.delays.iter_mut().take(frames).enumerate() {
                *d = start + step * (i + 1) as f64;
            }
        }
    }

    /// The delay at frame `i` of the block last filled.
    #[inline]
    pub(crate) fn at(&self, i: usize) -> f64 {
        self.delays
            .get(i)
            .or(self.delays.last())
            .copied()
            .unwrap_or(1.0)
    }

    pub(crate) fn reset(&mut self) {
        self.ramp.reset();
    }
}
