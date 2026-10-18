//! Per-source event rings (ARCHITECTURE §8 "Events in"), shared by MIDI
//! input ([`crate::midi`]) and control-surface input ([`crate::control`]).
//!
//! Every source (a MIDI port, the computer keyboard, the gamepad thread, the
//! OSC listener) owns the producer end of its own lock-free ring, so no
//! producer ever waits on another. It stamps each item with
//! [`crate::clock::now_ns`] as it arrives; a full ring drops the item and
//! counts it.
//!
//! The control side, [`RingSet`], hands each new ring's consumer to the
//! audio side through a swap ring. The audio side, [`RingDrain`], installs
//! it at the start of a callback; a consumer it replaces is drained one last
//! time and goes back through a garbage ring to be dropped on the control
//! side, so the audio thread never frees memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::midi::block_offset;

/// Sources that can be open at once, per [`RingSet`].
pub const MAX_SOURCES: usize = 16;
/// Items each source can queue between two callbacks.
pub(crate) const SOURCE_RING: usize = 1024;
const SWAP_RING: usize = 64;

/// An item with its arrival time on [`crate::clock`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamped<T> {
    pub time_ns: u64,
    pub item: T,
}

/// The producer end one source owns.
pub struct SourceTx<T> {
    tx: Producer<Stamped<T>>,
    dropped: Arc<AtomicU64>,
}

impl<T> SourceTx<T> {
    /// Queue `item`, received at `time_ns`. Never blocks: a full ring drops
    /// it, counts it, and returns false.
    pub fn push(&mut self, time_ns: u64, item: T) -> bool {
        if self.tx.push(Stamped { time_ns, item }).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }
}

/// Why a ring couldn't be claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// Every slot is taken.
    Full,
    /// The audio side hasn't picked up earlier swaps yet.
    Busy,
}

/// The control-thread end: claims fresh rings for new sources.
pub struct RingSet<T> {
    swap_tx: Producer<(usize, Consumer<Stamped<T>>)>,
    garbage_rx: Consumer<Consumer<Stamped<T>>>,
    dropped: Arc<AtomicU64>,
}

/// The audio-thread end: drains every source's ring.
pub struct RingDrain<T> {
    slots: Vec<Option<Consumer<Stamped<T>>>>,
    swap_rx: Consumer<(usize, Consumer<Stamped<T>>)>,
    garbage_tx: Producer<Consumer<Stamped<T>>>,
}

/// A ring set with nothing connected, and its audio-thread end.
pub fn ring_set<T>() -> (RingSet<T>, RingDrain<T>) {
    let (swap_tx, swap_rx) = RingBuffer::new(SWAP_RING);
    let (garbage_tx, garbage_rx) = RingBuffer::new(SWAP_RING);
    (
        RingSet {
            swap_tx,
            garbage_rx,
            dropped: Arc::new(AtomicU64::new(0)),
        },
        RingDrain {
            slots: (0..MAX_SOURCES).map(|_| None).collect(),
            swap_rx,
            garbage_tx,
        },
    )
}

impl<T> RingSet<T> {
    /// A fresh ring for the first slot `in_use` says is free; its consumer
    /// goes to the audio side through the swap ring.
    pub fn claim(
        &mut self,
        in_use: impl Fn(usize) -> bool,
    ) -> Result<(usize, SourceTx<T>), ClaimError> {
        let slot = (0..MAX_SOURCES)
            .find(|i| !in_use(*i))
            .ok_or(ClaimError::Full)?;
        let (tx, rx) = RingBuffer::new(SOURCE_RING);
        self.swap_tx
            .push((slot, rx))
            .map_err(|_| ClaimError::Busy)?;
        Ok((
            slot,
            SourceTx {
                tx,
                dropped: self.dropped.clone(),
            },
        ))
    }

    /// Drop rings the audio side has finished with.
    pub fn collect_garbage(&mut self) {
        while let Ok(r) = self.garbage_rx.pop() {
            drop(r);
        }
    }

    /// Items dropped because a source's ring was full, over every source.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl<T: Copy> RingDrain<T> {
    /// Collect everything received since the last call into `out` (cleared
    /// first, filled up to its capacity; the rest waits), stamped for a
    /// callback of `frames` frames that started at `t_callback_ns`, in
    /// offset order (arrival order within a source and at equal offsets).
    /// `make` builds an event from an offset and an item; `offset` reads it
    /// back. Real-time safe as long as `out` has capacity.
    pub fn drain<E>(
        &mut self,
        t_callback_ns: u64,
        frames: usize,
        sample_rate: f64,
        out: &mut Vec<E>,
        make: impl Fn(u32, T) -> E,
        offset: impl Fn(&E) -> u32,
    ) {
        out.clear();
        let place = |s: Stamped<T>| {
            make(
                block_offset(s.time_ns, t_callback_ns, frames, sample_rate),
                s.item,
            )
        };
        self.apply_swaps(out, &place);
        for ring in self.slots.iter_mut().flatten() {
            drain_ring(ring, out, &place);
        }
        sort_by_offset(out, offset);
    }

    /// Install consumers for newly connected sources. The consumer being
    /// replaced is drained one last time and sent back to be dropped; a swap
    /// waits while the garbage ring is full rather than freeing here.
    fn apply_swaps<E>(&mut self, out: &mut Vec<E>, place: &impl Fn(Stamped<T>) -> E) {
        while self.garbage_tx.slots() > 0 {
            let Ok((slot, new)) = self.swap_rx.pop() else {
                break;
            };
            let Some(cell) = self.slots.get_mut(slot) else {
                // A bad slot index can't come from RingSet; return the ring.
                let _ = self.garbage_tx.push(new);
                continue;
            };
            if let Some(mut old) = cell.replace(new) {
                drain_ring(&mut old, out, place);
                let _ = self.garbage_tx.push(old);
            }
        }
    }
}

fn drain_ring<T, E>(
    ring: &mut Consumer<Stamped<T>>,
    out: &mut Vec<E>,
    place: &impl Fn(Stamped<T>) -> E,
) {
    while out.len() < out.capacity() {
        let Ok(e) = ring.pop() else {
            return;
        };
        out.push(place(e));
    }
}

/// Stable insertion sort: each source is already in time order, so this is
/// close to linear, and unlike `sort_by_key` it never allocates. Stability
/// keeps a note-off before a note-on (or a key release before a press) at
/// the same offset.
fn sort_by_offset<E>(v: &mut [E], offset: impl Fn(&E) -> u32) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && offset(&v[j - 1]) > offset(&v[j]) {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_swaps_and_drains_in_offset_order() {
        let (mut set, mut drain) = ring_set::<u8>();
        let (a_slot, mut a) = set.claim(|_| false).unwrap();
        let (b_slot, mut b) = set.claim(|i| i == a_slot).unwrap();
        assert_ne!(a_slot, b_slot);
        // 480 frames at 48 kHz is 10 ms; the callback starts at 100 ms.
        let ms = 1_000_000;
        a.push(95 * ms, 2);
        b.push(91 * ms, 1);
        let mut out: Vec<(u32, u8)> = Vec::with_capacity(8);
        drain.drain(100 * ms, 480, 48_000.0, &mut out, |o, x| (o, x), |e| e.0);
        assert_eq!(out, [(48, 1), (240, 2)]);
        assert_eq!(set.dropped(), 0);
    }

    #[test]
    fn a_full_set_is_an_error() {
        let (mut set, _drain) = ring_set::<u8>();
        assert_eq!(set.claim(|_| true).err(), Some(ClaimError::Full));
    }
}
