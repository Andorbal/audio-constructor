//! triggers nodes.

use ac_core::{Config, ConfigExt, ProcessCtx, Registry, Unit};
use ac_dsp::util::snap_samples;

pub mod burst;
pub mod clock;
pub mod clock_divide;
pub mod clock_multiply;
pub mod euclidean;
pub mod frequency_trigger;
pub mod gate_to_trigger;
pub mod latch;
pub mod onset;
pub mod probability;
pub mod threshold_trigger;
pub mod timer;
pub mod trigger_delay;
pub mod trigger_to_gate;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clock::Clock>();
    r.register::<threshold_trigger::ThresholdTrigger>();
    r.register::<clock_divide::ClockDivide>();
    r.register::<clock_multiply::ClockMultiply>();
    r.register::<trigger_delay::TriggerDelay>();
    r.register::<gate_to_trigger::GateToTrigger>();
    r.register::<trigger_to_gate::TriggerToGate>();
    r.register::<euclidean::Euclidean>();
    r.register::<probability::Probability>();
    r.register::<burst::Burst>();
    r.register::<latch::Latch>();
    r.register::<timer::Timer>();
    r.register::<frequency_trigger::FrequencyTrigger>();
    r.register::<onset::Onset>();
}

/// Longest time a trigger node counts, in samples: about 265 days at
/// 48 kHz. Far beyond any musical use; it keeps sample arithmetic on `u64`
/// clear of overflow.
pub(crate) const MAX_SAMPLES: u64 = 1 << 40;

/// Whether a node's `unit` config asks for beats rather than seconds (the
/// default).
pub(crate) fn uses_beats(config: &Config) -> bool {
    config.get_str_or("unit", "seconds") == "beats"
}

/// The unit a time input shows, from the node's `unit` config.
pub(crate) fn time_unit(beats: bool) -> Unit {
    if beats { Unit::Beats } else { Unit::Seconds }
}

/// A time input read at frame `i` of the block, as a whole number of
/// samples. Beats follow the tempo at that frame's song position. Negative,
/// NaN, or infinite times count as 0; times within `f32` rounding of a whole
/// sample land on it (DECISIONS D32), and anything else rounds to the
/// nearest sample.
pub(crate) fn time_samples(t: f32, beats: bool, ctx: &ProcessCtx, i: usize) -> u64 {
    let t = if t.is_finite() {
        f64::from(t.max(0.0))
    } else {
        0.0
    };
    let per_unit = if beats {
        let at = ctx.transport.position.saturating_add(i as u64);
        ctx.transport.tempo.samples_per_beat_at(at)
    } else if ctx.sample_rate > 0.0 {
        ctx.sample_rate
    } else {
        ctx.transport.tempo.sample_rate()
    };
    let samples = snap_samples(t * per_unit).round();
    if samples.is_finite() {
        samples.clamp(0.0, MAX_SAMPLES as f64) as u64
    } else {
        0
    }
}

/// Trigger offsets of one block, clamped into the block and with repeats on
/// one frame merged, so a node handles each frame's trigger once. Offsets
/// past the end land on the last frame and out-of-order offsets on the
/// current one, as in `Runs`.
pub(crate) fn frames_of(list: &[u32], frames: usize) -> impl Iterator<Item = u64> + '_ {
    let last = frames.saturating_sub(1) as u64;
    let mut prev: Option<u64> = None;
    list.iter().filter_map(move |&o| {
        let o = u64::from(o).min(last);
        let o = prev.map_or(o, |p| o.max(p));
        if prev == Some(o) {
            None
        } else {
            prev = Some(o);
            Some(o)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_frames_are_merged_and_clamped() {
        let v: Vec<u64> = frames_of(&[0, 0, 3, 2, 9, 70, 80], 64).collect();
        assert_eq!(v, vec![0, 3, 9, 63]);
        assert_eq!(frames_of(&[5], 0).count(), 1);
    }
}
