//! MIDI output (ARCHITECTURE §8 "Events out", §11).
//!
//! The audio side, [`MidiOutQueue`], stamps each callback's outgoing
//! messages with the time they should leave and pushes them into a
//! lock-free ring. It never blocks; a full ring drops the message and counts
//! it.
//!
//! Messages leave one block after the callback that produced them, which
//! mirrors MIDI input (see [`crate::midi`]). An event a quarter of the way
//! into a block goes out a quarter of a block after the next callback
//! starts, so the spacing between messages matches the audio's instead of
//! bunching up at callback times.
//!
//! The control side, [`MidiOutputs`], owns the port connections and a sender
//! thread. The thread pops messages in order, waits for each one's time,
//! and writes it to every connected port.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ac_core::{MidiEvent, MidiMsg};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::midi::{MidiError, MidiPortInfo, MidiSelect};

/// Messages that can wait between the audio thread and the sender.
const OUT_RING: usize = 4096;
/// The sender's longest sleep: the worst lateness of a message that arrives
/// while it sleeps.
const IDLE: Duration = Duration::from_millis(1);
const CLIENT_NAME: &str = "Audio Constructor";

/// A message and when it should leave, on [`crate::clock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedMidi {
    pub time_ns: u64,
    pub msg: MidiMsg,
}

/// The audio-thread end: queues each callback's messages.
pub struct MidiOutQueue {
    tx: Producer<TimedMidi>,
    dropped: Arc<AtomicU64>,
}

impl fmt::Debug for MidiOutQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiOutQueue").finish_non_exhaustive()
    }
}

impl MidiOutQueue {
    /// Queue one callback's messages: `events` at frame offsets in a
    /// callback of `frames` frames that started at `t_callback_ns`.
    /// Real-time safe: no allocation, no locks.
    pub fn push_block(
        &mut self,
        t_callback_ns: u64,
        frames: usize,
        sample_rate: f64,
        events: &[MidiEvent],
    ) {
        let ns_per_frame = if sample_rate.is_finite() && sample_rate > 0.0 {
            1e9 / sample_rate
        } else {
            0.0
        };
        let block_ns = frames as f64 * ns_per_frame;
        for e in events {
            let late = block_ns + f64::from(e.offset) * ns_per_frame;
            let item = TimedMidi {
                time_ns: t_callback_ns.saturating_add(late as u64),
                msg: e.msg,
            };
            if self.tx.push(item).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Every MIDI output port on the system.
pub fn list_outputs() -> Result<Vec<MidiPortInfo>, MidiError> {
    let output = open_client()?;
    Ok(output
        .ports()
        .iter()
        .map(|p| MidiPortInfo {
            name: output
                .port_name(p)
                .unwrap_or_else(|_| "(unnamed port)".to_string()),
            id: p.id(),
        })
        .collect())
}

fn open_client() -> Result<midir::MidiOutput, MidiError> {
    midir::MidiOutput::new(CLIENT_NAME)
        .map_err(|e| MidiError(format!("MIDI output unavailable: {e}")))
}

type Sink = Box<dyn FnMut(&[u8]) + Send>;

struct Port {
    /// The system's port id; `None` for virtual outputs.
    id: Option<String>,
    name: String,
    sink: Sink,
}

struct Shared {
    ports: Mutex<Vec<Port>>,
    stop: AtomicBool,
    sent: AtomicU64,
}

/// The control-thread end: opens ports and runs the sender thread.
pub struct MidiOutputs {
    select: MidiSelect,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
    last_error: Option<MidiError>,
    scan_ok: bool,
}

impl fmt::Debug for MidiOutputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MidiOutputs")
            .field("select", &self.select)
            .field("connected", &self.connected())
            .finish_non_exhaustive()
    }
}

impl MidiOutputs {
    /// A MIDI output set with nothing connected, and its audio-thread end.
    pub fn new() -> (MidiOutputs, MidiOutQueue) {
        clock::init();
        let (tx, rx) = RingBuffer::new(OUT_RING);
        let shared = Arc::new(Shared {
            ports: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            sent: AtomicU64::new(0),
        });
        let dropped = Arc::new(AtomicU64::new(0));
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ac-midi-out".into())
            .spawn(move || sender_thread(rx, &thread_shared))
            .ok();
        let outputs = MidiOutputs {
            select: MidiSelect::None,
            shared,
            last_error: thread
                .is_none()
                .then(|| MidiError("couldn't start the MIDI output thread".into())),
            thread,
            dropped: dropped.clone(),
            scan_ok: false,
        };
        (outputs, MidiOutQueue { tx, dropped })
    }

    /// Choose which ports to send to and connect them now. Returns the names
    /// of the connected outputs.
    pub fn connect(&mut self, select: MidiSelect) -> Vec<String> {
        self.select = select;
        self.rescan();
        self.connected()
    }

    /// Re-scan the system's ports: close ones that disappeared or no longer
    /// match, open matching new ones. Returns the newly connected names.
    pub fn rescan(&mut self) -> Vec<String> {
        let ports = match list_outputs() {
            Ok(p) => {
                self.last_error = None;
                self.scan_ok = true;
                p
            }
            Err(e) => {
                self.last_error = Some(e);
                self.scan_ok = false;
                return Vec::new();
            }
        };
        let select = self.select.clone();
        let Ok(mut open) = self.shared.ports.lock() else {
            return Vec::new();
        };
        open.retain(|p| match &p.id {
            Some(id) => ports.iter().any(|q| &q.id == id && select.matches(&q.name)),
            None => true,
        });
        let mut added = Vec::new();
        for info in ports.iter().filter(|p| select.matches(&p.name)) {
            if open.iter().any(|p| p.id.as_ref() == Some(&info.id)) {
                continue;
            }
            match open_port(info) {
                Ok(sink) => {
                    open.push(Port {
                        id: Some(info.id.clone()),
                        name: info.name.clone(),
                        sink,
                    });
                    added.push(info.name.clone());
                }
                Err(e) => self.last_error = Some(e),
            }
        }
        added
    }

    /// An output that hands each message's bytes to `sink` on the sender
    /// thread (tests, activity lights).
    pub fn add_virtual(&mut self, name: &str, sink: impl FnMut(&[u8]) + Send + 'static) {
        if let Ok(mut open) = self.shared.ports.lock() {
            open.push(Port {
                id: None,
                name: name.to_string(),
                sink: Box::new(sink),
            });
        }
    }

    /// Names of the connected outputs.
    pub fn connected(&self) -> Vec<String> {
        self.shared
            .ports
            .lock()
            .map(|p| p.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Messages dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Messages sent so far (to at least the outputs connected then).
    pub fn sent(&self) -> u64 {
        self.shared.sent.load(Ordering::Relaxed)
    }

    /// Whether the last scan reached the system's MIDI service.
    pub fn system_available(&self) -> bool {
        self.scan_ok
    }

    /// Why the last scan or connection failed, if it did.
    pub fn last_error(&self) -> Option<&MidiError> {
        self.last_error.as_ref()
    }
}

impl Drop for MidiOutputs {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn open_port(info: &MidiPortInfo) -> Result<Sink, MidiError> {
    let output = open_client()?;
    let port = output
        .ports()
        .into_iter()
        .find(|p| p.id() == info.id)
        .ok_or_else(|| MidiError(format!("MIDI port {} disappeared", info.name)))?;
    let mut conn = output
        .connect(&port, "audio-constructor-out")
        .map_err(|e| MidiError(format!("couldn't open MIDI port {}: {e}", info.name)))?;
    Ok(Box::new(move |bytes: &[u8]| {
        // A port that went away fails here; the next rescan closes it.
        let _ = conn.send(bytes);
    }))
}

fn sender_thread(mut rx: Consumer<TimedMidi>, shared: &Shared) {
    let mut pending: VecDeque<TimedMidi> = VecDeque::with_capacity(OUT_RING);
    while !shared.stop.load(Ordering::Relaxed) {
        while let Ok(m) = rx.pop() {
            pending.push_back(m);
        }
        let now = clock::now_ns();
        // In order, even if a later callback stamped a slightly earlier
        // time: a note-off must never overtake its note-on.
        while pending.front().is_some_and(|m| m.time_ns <= now) {
            let Some(m) = pending.pop_front() else { break };
            let (bytes, len) = m.msg.to_bytes();
            if let Ok(mut ports) = shared.ports.lock() {
                for p in ports.iter_mut() {
                    (p.sink)(&bytes[..len]);
                }
            }
            shared.sent.fetch_add(1, Ordering::Relaxed);
        }
        let wait = pending.front().map_or(IDLE, |m| {
            Duration::from_nanos(m.time_ns.saturating_sub(now)).min(IDLE)
        });
        std::thread::sleep(wait.max(Duration::from_micros(50)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn note(n: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note: n,
            velocity: 100,
        }
    }

    #[test]
    fn messages_leave_in_order_one_block_late() {
        let (mut outputs, mut queue) = MidiOutputs::new();
        let got = Arc::new(Mutex::new(Vec::new()));
        let log = got.clone();
        outputs.add_virtual("test", move |b| {
            log.lock().unwrap().push((clock::now_ns(), b.to_vec()));
        });
        assert_eq!(outputs.connected(), ["test"]);
        // A 10 ms callback at 48 kHz; events at its start and halfway.
        let t = clock::now_ns();
        let events = [MidiEvent::new(0, note(60)), MidiEvent::new(240, note(62))];
        assert_no_alloc::assert_no_alloc(|| queue.push_block(t, 480, 48_000.0, &events));
        let t0 = Instant::now();
        while got.lock().unwrap().len() < 2 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1, [0x90, 60, 100]);
        assert_eq!(got[1].1, [0x90, 62, 100]);
        // Not before their time: one block (10 ms) late, then 5 ms apart.
        assert!(got[0].0 >= t + 10_000_000, "{} {}", got[0].0, t);
        assert!(got[1].0 >= t + 15_000_000);
        assert_eq!(outputs.sent(), 2);
        assert_eq!(outputs.dropped(), 0);
    }

    #[test]
    fn a_full_queue_drops_and_counts() {
        let (outputs, mut queue) = MidiOutputs::new();
        // Far in the future, so the sender holds them.
        let t = clock::now_ns() + 60_000_000_000;
        let events: Vec<MidiEvent> = (0..OUT_RING + 10)
            .map(|k| MidiEvent::new(k as u32, note(60)))
            .collect();
        queue.push_block(t, 64, 48_000.0, &events);
        // The sender may already have taken some into its own queue.
        std::thread::sleep(Duration::from_millis(20));
        queue.push_block(t, 64, 48_000.0, &events);
        assert!(outputs.dropped() >= 10);
    }
}
