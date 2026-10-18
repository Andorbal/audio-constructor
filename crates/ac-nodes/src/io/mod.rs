//! io nodes.

use ac_core::{ControlEvent, FloatOut, Registry};

pub mod audio_in;
pub mod audio_out;
pub mod file_out;
pub mod gamepad;
pub mod midi_in;
pub mod midi_out;
pub mod record;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<audio_in::AudioIn>();
    r.register::<audio_out::AudioOut>();
    r.register::<midi_in::MidiIn>();
    r.register::<midi_out::MidiOut>();
    r.register::<record::Record>();
    r.register::<file_out::FileOut>();
    r.register::<gamepad::Gamepad>();
}

/// The block's control events in order, each with the frame it lands on.
///
/// An offset past the end of the block lands on its last frame, and one
/// earlier than the event before it lands with that event, so a late or
/// out-of-order event (a key coming up, say) is never lost. Every walk over
/// the same events sees the same frames.
pub(crate) fn stamped(
    events: &[ControlEvent],
    frames: usize,
) -> impl Iterator<Item = (usize, &ControlEvent)> {
    let last = frames.saturating_sub(1);
    events.iter().scan(0usize, move |prev, ev| {
        let at = (ev.offset as usize).min(last).max(*prev);
        *prev = at;
        Some((at, ev))
    })
}

/// Sets frames `from..to` of `out` to `f(frame)`, calling `f` once for the
/// whole run unless `varying`. A block-rate output takes frame 0's value.
pub(crate) fn fill_range(
    out: &mut FloatOut,
    from: usize,
    to: usize,
    varying: bool,
    f: impl Fn(usize) -> f32,
) {
    if from >= to {
        return;
    }
    match out.as_slice_mut() {
        Some(samples) => {
            let to = to.min(samples.len());
            let Some(run) = samples.get_mut(from..to) else {
                return;
            };
            if varying {
                for (k, o) in run.iter_mut().enumerate() {
                    *o = f(from + k);
                }
            } else {
                run.fill(f(from));
            }
        }
        None => {
            if from == 0 {
                out.set(0, f(0));
            }
        }
    }
}

/// A gate as a sample value.
pub(crate) fn gate(on: bool) -> f32 {
    if on { 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Control, Key};

    #[test]
    fn late_and_out_of_order_events_are_kept_in_the_block() {
        let k = Control::Key(Key::from_char('a').unwrap());
        let evs = [
            ControlEvent::new(5, k, 1.0),
            ControlEvent::new(2, k, 0.0),
            ControlEvent::new(9, k, 1.0),
            ControlEvent::new(400, k, 0.0),
        ];
        let at: Vec<usize> = stamped(&evs, 64).map(|(at, _)| at).collect();
        assert_eq!(at, [5, 5, 9, 63]);
        assert_eq!(stamped(&evs, 1).map(|(at, _)| at).max(), Some(0));
    }

    #[test]
    fn fill_range_writes_only_its_run() {
        let mut buf = vec![9.0f32; 8];
        let mut out = FloatOut::Sample(&mut buf);
        fill_range(&mut out, 2, 5, false, |_| 1.0);
        fill_range(&mut out, 5, 99, true, |i| i as f32);
        fill_range(&mut out, 4, 2, false, |_| -1.0);
        assert_eq!(buf, [9.0, 9.0, 1.0, 1.0, 1.0, 5.0, 6.0, 7.0]);
        let mut v = 0.0f32;
        let mut block = FloatOut::Block(&mut v);
        fill_range(&mut block, 3, 8, false, |_| 2.0);
        fill_range(&mut block, 0, 3, false, |_| 4.0);
        assert_eq!(v, 4.0);
    }
}
