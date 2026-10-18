//! Control-surface input: computer keys, gamepads, and OSC (ARCHITECTURE §8
//! "Events in", §11).
//!
//! Works like MIDI input ([`crate::midi`]): every source owns its own
//! lock-free ring ([`crate::rings`]), stamps each event with
//! [`crate::clock::now_ns`] as it arrives, and drops (and counts) events
//! when its ring is full. The audio side, [`ControlReceiver`], drains every
//! ring at the start of a device callback and places events one block late
//! with [`crate::midi::block_offset`].
//!
//! Sources are [`ControlSender`]s handed out by [`ControlInputs`]: the
//! editor keeps one for the computer keys, the gamepad thread
//! ([`crate::gamepad`]) and the OSC listener ([`crate::osc`]) each own one.
//! A sender's ring is reused after the sender is dropped.

use std::fmt;
use std::sync::{Arc, Weak};

use ac_core::{Control, ControlEvent};

use crate::clock;
use crate::rings::{ClaimError, MAX_SOURCES, RingDrain, RingSet, SourceTx, ring_set};

/// Control events delivered per callback (the engine's own capacity); the
/// rest wait for the next callback.
pub const MAX_CONTROLS_PER_CALLBACK: usize = ac_core::CONTROL_CAPACITY;

/// A control and its value, as a source queues it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlMsg {
    pub control: Control,
    pub value: f32,
}

/// No more control sources can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlError(pub String);

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ControlError {}

/// One source of control events: the computer keys, a gamepad thread, the
/// OSC listener, a test. Sending never blocks.
pub struct ControlSender {
    inner: SourceTx<ControlMsg>,
    _alive: Arc<()>,
}

impl ControlSender {
    /// Send `value` for `control`, stamped now. Returns false if the ring
    /// was full and the event was dropped (it's counted either way).
    pub fn send(&mut self, control: Control, value: f32) -> bool {
        self.send_at(clock::now_ns(), control, value)
    }

    /// Send with an explicit arrival time on [`crate::clock`] (tests,
    /// sources that stamp on arrival before decoding).
    pub fn send_at(&mut self, time_ns: u64, control: Control, value: f32) -> bool {
        self.inner.push(time_ns, ControlMsg { control, value })
    }
}

impl fmt::Debug for ControlSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlSender").finish_non_exhaustive()
    }
}

/// The audio-thread end: drains every source into block-offset events.
pub struct ControlReceiver {
    rings: RingDrain<ControlMsg>,
}

impl fmt::Debug for ControlReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlReceiver").finish_non_exhaustive()
    }
}

impl ControlReceiver {
    /// Collect everything received since the last call into `out` (cleared
    /// first, filled up to its capacity; the rest waits), stamped for a
    /// callback of `frames` frames that started at `t_callback_ns`, in
    /// offset order. Real-time safe as long as `out` has capacity.
    pub fn drain(
        &mut self,
        t_callback_ns: u64,
        frames: usize,
        sample_rate: f64,
        out: &mut Vec<ControlEvent>,
    ) {
        self.rings.drain(
            t_callback_ns,
            frames,
            sample_rate,
            out,
            |offset, m| ControlEvent::new(offset, m.control, m.value),
            |e| e.offset,
        );
    }
}

struct Source {
    slot: usize,
    name: String,
    alive: Weak<()>,
}

/// The control-thread end: hands out senders and reports on them.
pub struct ControlInputs {
    sources: Vec<Source>,
    rings: RingSet<ControlMsg>,
}

impl fmt::Debug for ControlInputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlInputs")
            .field("connected", &self.connected())
            .finish_non_exhaustive()
    }
}

impl ControlInputs {
    /// A control input set with no sources, and its audio-thread end.
    pub fn new() -> (ControlInputs, ControlReceiver) {
        clock::init();
        let (rings, drain) = ring_set();
        (
            ControlInputs {
                sources: Vec::new(),
                rings,
            },
            ControlReceiver { rings: drain },
        )
    }

    /// A new source called `name` feeding the receiver. Its ring is reused
    /// once the sender is dropped.
    pub fn sender(&mut self, name: &str) -> Result<ControlSender, ControlError> {
        self.collect_garbage();
        let sources = &self.sources;
        let (slot, tx) = self
            .rings
            .claim(|i| sources.iter().any(|s| s.slot == i))
            .map_err(|e| match e {
                ClaimError::Full => {
                    ControlError(format!("at most {MAX_SOURCES} control sources can be open"))
                }
                ClaimError::Busy => ControlError("control input is busy; try again".into()),
            })?;
        let alive = Arc::new(());
        self.sources.push(Source {
            slot,
            name: name.to_string(),
            alive: Arc::downgrade(&alive),
        });
        Ok(ControlSender {
            inner: tx,
            _alive: alive,
        })
    }

    /// Forget dropped senders and drop rings the audio side has finished
    /// with. Call now and then.
    pub fn collect_garbage(&mut self) {
        self.sources.retain(|s| s.alive.strong_count() > 0);
        self.rings.collect_garbage();
    }

    /// Names of the live sources.
    pub fn connected(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter(|s| s.alive.strong_count() > 0)
            .map(|s| s.name.clone())
            .collect()
    }

    /// Events dropped because a source's ring was full, over every source.
    pub fn dropped(&self) -> u64 {
        self.rings.dropped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rings::SOURCE_RING;
    use ac_core::{GamepadAxis, GamepadButton, Key, osc_address_hash};

    const RATE: f64 = 48_000.0;
    const MS: u64 = 1_000_000;

    fn key(name: &str) -> Control {
        Control::Key(Key::from_name(name).unwrap())
    }

    #[test]
    fn sources_reach_the_receiver_one_block_late_in_order() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let mut keys = inputs.sender("keys").unwrap();
        let mut pad = inputs.sender("gamepads").unwrap();
        let stick = Control::GamepadAxis {
            pad: 0,
            axis: GamepadAxis::LeftX,
        };
        // A 10 ms callback (480 frames at 48 kHz) starting at 100 ms.
        keys.send_at(97 * MS, key("A"), 1.0);
        keys.send_at(99 * MS, key("A"), 0.0);
        pad.send_at(91 * MS, stick, -0.5);
        pad.send_at(95 * MS, stick, 0.25);
        let mut out = Vec::with_capacity(MAX_CONTROLS_PER_CALLBACK);
        assert_no_alloc::assert_no_alloc(|| rx.drain(100 * MS, 480, RATE, &mut out));
        let got: Vec<(u32, Control, f32)> =
            out.iter().map(|e| (e.offset, e.control, e.value)).collect();
        assert_eq!(
            got,
            vec![
                (48, stick, -0.5),
                (240, stick, 0.25),
                (336, key("A"), 1.0),
                (432, key("A"), 0.0),
            ]
        );
        // Nothing twice.
        rx.drain(110 * MS, 480, RATE, &mut out);
        assert!(out.is_empty());
        assert_eq!(inputs.connected(), ["keys", "gamepads"]);
    }

    #[test]
    fn equal_offsets_keep_arrival_order() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let mut s = inputs.sender("osc").unwrap();
        let c = Control::Osc {
            address: osc_address_hash("/x"),
            arg: 0,
        };
        s.send_at(5, c, 1.0);
        s.send_at(5, c, 2.0);
        s.send_at(5, c, 3.0);
        let mut out = Vec::with_capacity(8);
        rx.drain(100 * MS, 64, RATE, &mut out);
        let values: Vec<f32> = out.iter().map(|e| e.value).collect();
        assert_eq!(values, [1.0, 2.0, 3.0]);
        assert!(out.iter().all(|e| e.offset == 0), "old events land on 0");
    }

    #[test]
    fn a_full_ring_drops_and_counts_and_delivery_is_capped() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let mut s = inputs.sender("flood").unwrap();
        let b = Control::GamepadButton {
            pad: 1,
            button: GamepadButton::South,
        };
        let mut accepted = 0;
        for _ in 0..SOURCE_RING + 7 {
            if s.send_at(0, b, 1.0) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, SOURCE_RING);
        assert_eq!(inputs.dropped(), 7);
        let mut out = Vec::with_capacity(100);
        rx.drain(MS, 48, RATE, &mut out);
        assert_eq!(out.len(), 100, "capped by the caller's buffer");
        let mut rest = Vec::with_capacity(SOURCE_RING);
        rx.drain(MS, 48, RATE, &mut rest);
        assert_eq!(rest.len(), SOURCE_RING - 100, "the rest waited");
    }

    #[test]
    fn dropped_senders_swap_rings_without_losing_events() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let mut out = Vec::with_capacity(8);
        for i in 0..MAX_SOURCES * 3 {
            let mut s = inputs.sender("keys").unwrap();
            s.send_at(0, key("Q"), i as f32);
            drop(s);
            // The audio side installs the new ring and drains the old one.
            rx.drain(MS, 48, RATE, &mut out);
            assert_eq!(out.len(), 1, "round {i}");
            assert_eq!(out[0].value, i as f32);
        }
        inputs.collect_garbage();
        assert!(inputs.connected().is_empty());
    }

    #[test]
    fn too_many_sources_is_an_error_not_a_panic() {
        let (mut inputs, _rx) = ControlInputs::new();
        let held: Vec<ControlSender> = (0..MAX_SOURCES)
            .map(|_| inputs.sender("s").unwrap())
            .collect();
        assert!(inputs.sender("one more").is_err());
        drop(held);
        assert!(inputs.sender("now fine").is_ok());
    }

    #[test]
    fn send_stamps_with_the_shared_clock() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let mut s = inputs.sender("keys").unwrap();
        let t0 = clock::now_ns();
        assert!(s.send(key("Space"), 1.0));
        // A callback 10 ms long that starts 5 ms after the send: the event
        // lands about halfway through.
        let mut out = Vec::with_capacity(4);
        rx.drain(t0 + 5 * MS, 480, RATE, &mut out);
        assert_eq!(out.len(), 1);
        assert!((240..=300).contains(&out[0].offset), "{}", out[0].offset);
    }
}
