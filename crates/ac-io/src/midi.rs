//! MIDI input (ARCHITECTURE §8 "Events in", §11).
//!
//! Each connected port (and each virtual source, such as the computer
//! keyboard) gets its own lock-free ring. Its callback thread stamps every
//! message with [`crate::clock::now_ns`] the moment it arrives and pushes it;
//! a full ring drops the message and counts it, so nothing ever blocks.
//!
//! The audio side, [`MidiReceiver`], drains every ring at the start of a
//! device callback and turns arrival times into sample offsets with
//! [`block_offset`]. Events are placed one block late: an event that arrived
//! a quarter of the way through the previous block lands a quarter of the
//! way into this one. That costs one buffer of latency and buys timing that
//! doesn't jitter with where the callback happened to fall.
//!
//! The control side, [`MidiInputs`], opens and closes ports. Rings are
//! never shared between connections: connecting creates a fresh ring and
//! hands its consumer to the audio side through a swap ring; the replaced
//! consumer comes back through a garbage ring to be dropped here, so the
//! audio thread never frees memory.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use ac_core::{MidiEvent, MidiMsg};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;

/// Ports and virtual sources that can be open at once.
pub const MAX_SOURCES: usize = 16;
/// Messages each source can queue between two callbacks.
const SOURCE_RING: usize = 1024;
const SWAP_RING: usize = 64;
/// Events delivered per callback; the rest wait for the next one.
pub const MAX_EVENTS_PER_CALLBACK: usize = 1024;
const CLIENT_NAME: &str = "Audio Constructor";

/// A message with its arrival time on [`crate::clock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StampedMidi {
    pub time_ns: u64,
    pub msg: MidiMsg,
}

/// The sample offset inside a callback of `frames` frames, starting at
/// `t_callback_ns`, for a message received at `t_recv_ns`.
///
/// The callback plays the span `[t_callback - block, t_callback)` shifted one
/// block later, so `offset = (t_recv - (t_callback - block)) * rate`,
/// clamped to `0..frames`. Messages older than that span land on frame 0;
/// ones stamped after the callback started (a race with the MIDI thread)
/// land on the last frame.
pub fn block_offset(t_recv_ns: u64, t_callback_ns: u64, frames: usize, sample_rate: f64) -> u32 {
    if frames == 0 || sample_rate.is_nan() || sample_rate <= 0.0 {
        return 0;
    }
    let block_ns = frames as f64 * 1e9 / sample_rate;
    // Differences first, in signed integers, so large clock values keep
    // their precision.
    let since = (i128::from(t_recv_ns) - i128::from(t_callback_ns)) as f64 + block_ns;
    let off = (since * sample_rate / 1e9).floor();
    let last = (frames - 1) as f64;
    if off.is_nan() {
        0
    } else {
        off.clamp(0.0, last) as u32
    }
}

/// Which ports to connect.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum MidiSelect {
    None,
    #[default]
    All,
    /// Ports whose name contains this text (case-insensitive).
    Matching(String),
}

impl MidiSelect {
    /// `none`, `all`, or a name fragment.
    pub fn parse(s: &str) -> MidiSelect {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "" => MidiSelect::None,
            "all" | "*" => MidiSelect::All,
            _ => MidiSelect::Matching(s.trim().to_string()),
        }
    }

    pub fn matches(&self, port_name: &str) -> bool {
        match self {
            MidiSelect::None => false,
            MidiSelect::All => true,
            MidiSelect::Matching(f) => port_name.to_lowercase().contains(&f.to_lowercase()),
        }
    }
}

/// A MIDI input port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPortInfo {
    pub name: String,
    /// Stable identifier (opaque).
    pub id: String,
}

/// The MIDI system couldn't be opened (no sequencer, no driver).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiError(pub String);

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MidiError {}

fn open_client() -> Result<midir::MidiInput, MidiError> {
    let mut input = midir::MidiInput::new(CLIENT_NAME)
        .map_err(|e| MidiError(format!("MIDI input unavailable: {e}")))?;
    // Clock, active sensing, and sysex never become `MidiMsg`s; filtering
    // them in the driver saves waking our callback for them.
    input.ignore(midir::Ignore::All);
    Ok(input)
}

/// Every MIDI input port on the system.
pub fn list_inputs() -> Result<Vec<MidiPortInfo>, MidiError> {
    let input = open_client()?;
    Ok(input
        .ports()
        .iter()
        .map(|p| MidiPortInfo {
            name: input
                .port_name(p)
                .unwrap_or_else(|_| "(unnamed port)".to_string()),
            id: p.id(),
        })
        .collect())
}

/// What a source's callback owns.
struct SourceTx {
    tx: Producer<StampedMidi>,
    dropped: Arc<AtomicU64>,
}

impl SourceTx {
    fn push(&mut self, time_ns: u64, msg: MidiMsg) {
        if self.tx.push(StampedMidi { time_ns, msg }).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A virtual MIDI source: the computer keyboard, an on-screen keyboard, a
/// test. Messages go through the same rings and timing as a port.
pub struct MidiSender {
    inner: SourceTx,
    _alive: Arc<()>,
}

impl MidiSender {
    /// Send a message stamped now. Never blocks; a full ring drops it.
    pub fn send(&mut self, msg: MidiMsg) {
        self.inner.push(clock::now_ns(), msg);
    }

    /// Send with an explicit arrival time (tests, replay).
    pub fn send_at(&mut self, time_ns: u64, msg: MidiMsg) {
        self.inner.push(time_ns, msg);
    }
}

impl fmt::Debug for MidiSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiSender").finish_non_exhaustive()
    }
}

enum SourceKind {
    Port {
        id: String,
        _conn: midir::MidiInputConnection<SourceTx>,
    },
    Virtual(Weak<()>),
}

struct Source {
    slot: usize,
    name: String,
    kind: SourceKind,
}

/// The audio-thread end: drains every source into block-offset events.
pub struct MidiReceiver {
    slots: Vec<Option<Consumer<StampedMidi>>>,
    swap_rx: Consumer<(usize, Consumer<StampedMidi>)>,
    garbage_tx: Producer<Consumer<StampedMidi>>,
}

impl fmt::Debug for MidiReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiReceiver").finish_non_exhaustive()
    }
}

impl MidiReceiver {
    /// Collect everything received since the last call into `out` (cleared
    /// first, filled up to its capacity), stamped for a callback of `frames`
    /// frames that started at `t_callback_ns`, in offset order. Real-time
    /// safe as long as `out` has capacity: no allocation, no locks.
    pub fn drain(
        &mut self,
        t_callback_ns: u64,
        frames: usize,
        sample_rate: f64,
        out: &mut Vec<MidiEvent>,
    ) {
        out.clear();
        self.apply_swaps(t_callback_ns, frames, sample_rate, out);
        for ring in self.slots.iter_mut().flatten() {
            drain_ring(ring, t_callback_ns, frames, sample_rate, out);
        }
        sort_by_offset(out);
    }

    /// Install consumers for newly connected sources. The consumer being
    /// replaced is drained one last time and sent back to be dropped; a swap
    /// waits while the garbage ring is full rather than freeing here.
    fn apply_swaps(&mut self, t: u64, frames: usize, rate: f64, out: &mut Vec<MidiEvent>) {
        while self.garbage_tx.slots() > 0 {
            let Ok((slot, new)) = self.swap_rx.pop() else {
                break;
            };
            let Some(cell) = self.slots.get_mut(slot) else {
                // A bad slot index can't come from MidiInputs; return the ring.
                let _ = self.garbage_tx.push(new);
                continue;
            };
            if let Some(mut old) = cell.replace(new) {
                drain_ring(&mut old, t, frames, rate, out);
                let _ = self.garbage_tx.push(old);
            }
        }
    }
}

fn drain_ring(
    ring: &mut Consumer<StampedMidi>,
    t: u64,
    frames: usize,
    rate: f64,
    out: &mut Vec<MidiEvent>,
) {
    while out.len() < out.capacity() {
        let Ok(e) = ring.pop() else {
            return;
        };
        out.push(MidiEvent::new(
            block_offset(e.time_ns, t, frames, rate),
            e.msg,
        ));
    }
}

/// Stable insertion sort: each source is already in time order, so this is
/// close to linear, and unlike `sort_by_key` it never allocates. Stability
/// keeps a note-off before a note-on at the same offset.
fn sort_by_offset(v: &mut [MidiEvent]) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1].offset > v[j].offset {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// The control-thread end: opens ports and virtual sources.
pub struct MidiInputs {
    select: MidiSelect,
    sources: Vec<Source>,
    swap_tx: Producer<(usize, Consumer<StampedMidi>)>,
    garbage_rx: Consumer<Consumer<StampedMidi>>,
    dropped: Arc<AtomicU64>,
    last_error: Option<MidiError>,
}

impl fmt::Debug for MidiInputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiInputs")
            .field("select", &self.select)
            .field("connected", &self.connected())
            .finish_non_exhaustive()
    }
}

impl MidiInputs {
    /// A MIDI input set with nothing connected, and its audio-thread end.
    pub fn new() -> (MidiInputs, MidiReceiver) {
        let (swap_tx, swap_rx) = RingBuffer::new(SWAP_RING);
        let (garbage_tx, garbage_rx) = RingBuffer::new(SWAP_RING);
        let inputs = MidiInputs {
            select: MidiSelect::None,
            sources: Vec::new(),
            swap_tx,
            garbage_rx,
            dropped: Arc::new(AtomicU64::new(0)),
            last_error: None,
        };
        let receiver = MidiReceiver {
            slots: (0..MAX_SOURCES).map(|_| None).collect(),
            swap_rx,
            garbage_tx,
        };
        (inputs, receiver)
    }

    /// Choose which ports to connect and connect them now. Returns the names
    /// of the connected ports.
    pub fn connect(&mut self, select: MidiSelect) -> Vec<String> {
        self.select = select;
        self.rescan();
        self.connected()
    }

    /// Re-scan the system's ports (after a device was plugged in or
    /// removed): close ports that disappeared or no longer match, and open
    /// matching new ones. Returns the names of newly connected ports.
    pub fn rescan(&mut self) -> Vec<String> {
        self.collect_garbage();
        self.reclaim_virtual();
        let ports = match list_inputs() {
            Ok(p) => {
                self.last_error = None;
                p
            }
            Err(e) => {
                self.last_error = Some(e);
                Vec::new()
            }
        };
        let select = self.select.clone();
        self.sources.retain(|s| match &s.kind {
            SourceKind::Port { id, .. } => {
                ports.iter().any(|p| &p.id == id && select.matches(&p.name))
            }
            SourceKind::Virtual(_) => true,
        });
        let mut added = Vec::new();
        for p in ports.iter().filter(|p| select.matches(&p.name)) {
            let open = self
                .sources
                .iter()
                .any(|s| matches!(&s.kind, SourceKind::Port { id, .. } if id == &p.id));
            if open {
                continue;
            }
            match self.open_port(p) {
                Ok(()) => added.push(p.name.clone()),
                Err(e) => self.last_error = Some(e),
            }
        }
        added
    }

    fn open_port(&mut self, port: &MidiPortInfo) -> Result<(), MidiError> {
        let input = open_client()?;
        let handle = input
            .ports()
            .into_iter()
            .find(|p| p.id() == port.id)
            .ok_or_else(|| MidiError(format!("MIDI port {} disappeared", port.name)))?;
        let (slot, tx) = self.claim_slot()?;
        let conn = input
            .connect(
                &handle,
                "audio-constructor-in",
                |_driver_us, bytes, src: &mut SourceTx| {
                    // Stamp before anything else; the driver's timestamp is on
                    // a different clock.
                    let t = clock::now_ns();
                    if let Some(msg) = MidiMsg::from_bytes(bytes) {
                        src.push(t, msg);
                    }
                },
                tx,
            )
            .map_err(|e| MidiError(format!("couldn't open MIDI port {}: {e}", port.name)))?;
        self.sources.push(Source {
            slot,
            name: port.name.clone(),
            kind: SourceKind::Port {
                id: port.id.clone(),
                _conn: conn,
            },
        });
        Ok(())
    }

    /// A virtual source feeding the same receiver (computer keyboard, tests).
    /// Its slot is reused after the sender is dropped and a rescan runs.
    pub fn sender(&mut self, name: &str) -> Result<MidiSender, MidiError> {
        self.collect_garbage();
        self.reclaim_virtual();
        let (slot, tx) = self.claim_slot()?;
        let alive = Arc::new(());
        self.sources.push(Source {
            slot,
            name: name.to_string(),
            kind: SourceKind::Virtual(Arc::downgrade(&alive)),
        });
        Ok(MidiSender {
            inner: tx,
            _alive: alive,
        })
    }

    /// A fresh ring for the first free slot; the consumer goes to the audio
    /// side through the swap ring.
    fn claim_slot(&mut self) -> Result<(usize, SourceTx), MidiError> {
        let slot = (0..MAX_SOURCES)
            .find(|i| !self.sources.iter().any(|s| s.slot == *i))
            .ok_or_else(|| MidiError(format!("at most {MAX_SOURCES} MIDI sources can be open")))?;
        let (tx, rx) = RingBuffer::new(SOURCE_RING);
        self.swap_tx
            .push((slot, rx))
            .map_err(|_| MidiError("MIDI is busy; try again".to_string()))?;
        Ok((
            slot,
            SourceTx {
                tx,
                dropped: self.dropped.clone(),
            },
        ))
    }

    fn reclaim_virtual(&mut self) {
        self.sources.retain(|s| match &s.kind {
            SourceKind::Virtual(w) => w.strong_count() > 0,
            SourceKind::Port { .. } => true,
        });
    }

    /// Drop rings the audio side has finished with. Call now and then.
    pub fn collect_garbage(&mut self) {
        while let Ok(r) = self.garbage_rx.pop() {
            drop(r);
        }
    }

    /// Names of the connected ports and live virtual sources.
    pub fn connected(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter(|s| match &s.kind {
                SourceKind::Virtual(w) => w.strong_count() > 0,
                SourceKind::Port { .. } => true,
            })
            .map(|s| s.name.clone())
            .collect()
    }

    /// Messages dropped because a source's ring was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Why the last scan or connection failed, if it did.
    pub fn last_error(&self) -> Option<&MidiError> {
        self.last_error.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;
    const MS: u64 = 1_000_000;

    fn note(n: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note: n,
            velocity: 100,
        }
    }

    #[test]
    fn offsets_are_one_block_late() {
        // 480 frames at 48 kHz is 10 ms; the callback starts at t = 100 ms.
        let t = 100 * MS;
        assert_eq!(
            block_offset(90 * MS, t, 480, RATE),
            0,
            "start of the last block"
        );
        assert_eq!(block_offset(95 * MS, t, 480, RATE), 240, "halfway");
        assert_eq!(
            block_offset(t - 1, t, 480, RATE),
            479,
            "just before the callback"
        );
    }

    #[test]
    fn old_and_future_events_are_clamped() {
        let t = 100 * MS;
        assert_eq!(block_offset(0, t, 480, RATE), 0);
        assert_eq!(block_offset(50 * MS, t, 480, RATE), 0);
        assert_eq!(block_offset(t + 5 * MS, t, 480, RATE), 479);
        assert_eq!(block_offset(u64::MAX, t, 480, RATE), 479);
    }

    #[test]
    fn degenerate_inputs_dont_panic() {
        assert_eq!(block_offset(5, 10, 0, RATE), 0);
        assert_eq!(block_offset(5, 10, 64, 0.0), 0);
        assert_eq!(block_offset(5, 10, 64, f64::NAN), 0);
        assert_eq!(block_offset(5, 10, 1, RATE), 0);
    }

    #[test]
    fn offsets_scale_with_rate() {
        let t = 100 * MS;
        // 5 ms into a 10 ms block, at 96 kHz (960 frames).
        assert_eq!(block_offset(95 * MS, t, 960, 96_000.0), 480);
    }

    #[test]
    fn select_parsing_and_matching() {
        assert_eq!(MidiSelect::parse("none"), MidiSelect::None);
        assert_eq!(MidiSelect::parse("ALL"), MidiSelect::All);
        let m = MidiSelect::parse("keystep");
        assert!(m.matches("Arturia KeyStep 32 MIDI 1"));
        assert!(!m.matches("Midi Through Port-0"));
        assert!(!MidiSelect::None.matches("anything"));
    }

    #[test]
    fn virtual_sources_reach_the_receiver_in_order() {
        let (mut inputs, mut rx) = MidiInputs::new();
        let mut a = inputs.sender("a").unwrap();
        let mut b = inputs.sender("b").unwrap();
        let t = 100 * MS;
        a.send_at(97 * MS, note(3));
        a.send_at(99 * MS, note(4));
        b.send_at(91 * MS, note(1));
        b.send_at(95 * MS, note(2));
        let mut out = Vec::with_capacity(MAX_EVENTS_PER_CALLBACK);
        rx.drain(t, 480, RATE, &mut out);
        let notes: Vec<(u32, MidiMsg)> = out.iter().map(|e| (e.offset, e.msg)).collect();
        assert_eq!(
            notes,
            vec![
                (48, note(1)),
                (240, note(2)),
                (336, note(3)),
                (432, note(4))
            ]
        );
        rx.drain(t + 10 * MS, 480, RATE, &mut out);
        assert!(out.is_empty());
        assert_eq!(inputs.connected(), vec!["a", "b"]);
    }

    #[test]
    fn same_offset_keeps_arrival_order() {
        let (mut inputs, mut rx) = MidiInputs::new();
        let mut a = inputs.sender("a").unwrap();
        let off = MidiMsg::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        };
        a.send_at(10, off);
        a.send_at(10, note(60));
        let mut out = Vec::with_capacity(8);
        rx.drain(100 * MS, 480, RATE, &mut out);
        assert_eq!(out[0].msg, off);
        assert_eq!(out[1].msg, note(60));
    }

    #[test]
    fn full_ring_drops_and_counts() {
        let (mut inputs, mut rx) = MidiInputs::new();
        let mut a = inputs.sender("a").unwrap();
        for _ in 0..SOURCE_RING + 10 {
            a.send_at(0, note(1));
        }
        assert_eq!(inputs.dropped(), 10);
        // Delivery is capped by the caller's buffer; the rest waits.
        let mut out = Vec::with_capacity(100);
        rx.drain(MS, 48, RATE, &mut out);
        assert_eq!(out.len(), 100);
        let mut all = Vec::with_capacity(SOURCE_RING);
        rx.drain(MS, 48, RATE, &mut all);
        assert_eq!(all.len(), SOURCE_RING - 100);
    }

    #[test]
    fn dropped_senders_free_their_slot() {
        let (mut inputs, mut rx) = MidiInputs::new();
        let mut out = Vec::with_capacity(8);
        for i in 0..MAX_SOURCES * 3 {
            let mut s = inputs.sender("kbd").unwrap();
            s.send_at(0, note((i % 128) as u8));
            drop(s);
            // The audio side picks up the swap (and returns the old ring).
            rx.drain(MS, 48, RATE, &mut out);
            assert_eq!(out.len(), 1, "round {i}");
        }
        inputs.collect_garbage();
        assert!(inputs.connected().is_empty());
    }

    #[test]
    fn too_many_sources_is_an_error_not_a_panic() {
        let (mut inputs, _rx) = MidiInputs::new();
        let held: Vec<MidiSender> = (0..MAX_SOURCES)
            .map(|_| inputs.sender("s").unwrap())
            .collect();
        assert!(inputs.sender("one more").is_err());
        drop(held);
        assert!(inputs.sender("now fine").is_ok());
    }

    #[test]
    fn scanning_without_a_midi_system_is_fine() {
        // The container running CI has no sequencer; either outcome is fine,
        // as long as nothing panics.
        let (mut inputs, _rx) = MidiInputs::new();
        let _ = inputs.connect(MidiSelect::All);
        let _ = list_inputs();
        let _ = inputs.rescan();
    }
}
