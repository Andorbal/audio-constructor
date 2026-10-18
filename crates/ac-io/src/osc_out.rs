//! Open Sound Control output (ARCHITECTURE §8 "Events out", §11).
//!
//! Mirrors MIDI output ([`crate::midi_out`]): the audio side,
//! [`OscOutQueue`], stamps each callback's messages with the time they
//! should leave, one block after the callback that produced them, and
//! pushes them into a lock-free ring; a full ring drops and counts.
//! [`OscOutput`] owns a sender thread that encodes each message when its
//! time comes and sends it over UDP to the message's own target. Sending is
//! a system call, so it never happens on the audio thread.

use std::collections::VecDeque;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use ac_core::OscMessage;
use rosc::{OscPacket, OscType};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;

/// Messages that can wait between the audio thread and the sender.
const OUT_RING: usize = 4096;
/// The sender's longest sleep while messages have been flowing: the worst
/// lateness of a message that arrives while it sleeps.
const ACTIVE_WAIT: Duration = Duration::from_millis(1);
/// Its sleep after a quiet spell, so an unused output costs next to nothing.
const IDLE_WAIT: Duration = Duration::from_millis(5);
/// How long without messages counts as a quiet spell.
const IDLE_AFTER_NS: u64 = 2_000_000_000;

/// A message and when it should leave, on [`crate::clock`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct TimedOsc {
    time_ns: u64,
    msg: OscMessage,
}

/// The audio-thread end: queues each callback's messages.
pub struct OscOutQueue {
    tx: Producer<TimedOsc>,
    dropped: Arc<AtomicU64>,
}

impl fmt::Debug for OscOutQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscOutQueue").finish_non_exhaustive()
    }
}

impl OscOutQueue {
    /// Queue one callback's messages: `msgs` at frame offsets in a callback
    /// of `frames` frames that started at `t_callback_ns`. Real-time safe:
    /// no allocation, no locks.
    pub fn push_block(
        &mut self,
        t_callback_ns: u64,
        frames: usize,
        sample_rate: f64,
        msgs: &[OscMessage],
    ) {
        let ns_per_frame = if sample_rate.is_finite() && sample_rate > 0.0 {
            1e9 / sample_rate
        } else {
            0.0
        };
        let block_ns = frames as f64 * ns_per_frame;
        for m in msgs {
            let late = block_ns + f64::from(m.offset) * ns_per_frame;
            let item = TimedOsc {
                time_ns: t_callback_ns.saturating_add(late as u64),
                msg: *m,
            };
            if self.tx.push(item).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Debug, Default)]
struct Shared {
    stop: AtomicBool,
    sent: AtomicU64,
    errors: AtomicU64,
}

/// The control-thread end: runs the sender thread and reports on it.
pub struct OscOutput {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl fmt::Debug for OscOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscOutput")
            .field("sent", &self.sent())
            .field("dropped", &self.dropped())
            .field("errors", &self.errors())
            .finish_non_exhaustive()
    }
}

impl OscOutput {
    /// An OSC output and its audio-thread end. Messages go wherever their
    /// `target` says.
    pub fn new() -> (OscOutput, OscOutQueue) {
        clock::init();
        let (tx, rx) = RingBuffer::new(OUT_RING);
        let shared = Arc::new(Shared::default());
        let dropped = Arc::new(AtomicU64::new(0));
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ac-osc-out".into())
            .spawn(move || sender_thread(rx, &thread_shared))
            .ok();
        (
            OscOutput {
                shared,
                thread,
                dropped: dropped.clone(),
            },
            OscOutQueue { tx, dropped },
        )
    }

    /// Messages sent so far.
    pub fn sent(&self) -> u64 {
        self.shared.sent.load(Ordering::Relaxed)
    }

    /// Messages dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Messages the network refused (no route, unreachable host).
    pub fn errors(&self) -> u64 {
        self.shared.errors.load(Ordering::Relaxed)
    }

    /// Whether the sender thread is running.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for OscOutput {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// UDP sockets for each address family, opened on first use.
#[derive(Default)]
struct Sockets {
    v4: Option<UdpSocket>,
    v6: Option<UdpSocket>,
}

impl Sockets {
    fn for_target(&mut self, target: &SocketAddr) -> Option<&UdpSocket> {
        let (cell, bind): (_, SocketAddr) = match target {
            SocketAddr::V4(_) => (&mut self.v4, (Ipv4Addr::UNSPECIFIED, 0).into()),
            SocketAddr::V6(_) => (&mut self.v6, (Ipv6Addr::UNSPECIFIED, 0).into()),
        };
        if cell.is_none() {
            *cell = UdpSocket::bind(bind).ok();
        }
        cell.as_ref()
    }
}

/// Encode `m` into `buf` (cleared first).
fn encode(m: &OscMessage, buf: &mut Vec<u8>) -> bool {
    buf.clear();
    let packet = OscPacket::Message(rosc::OscMessage {
        addr: m.address().to_string(),
        args: m.args().iter().map(|&v| OscType::Float(v)).collect(),
    });
    rosc::encoder::encode_into(&packet, buf).is_ok()
}

fn sender_thread(mut rx: Consumer<TimedOsc>, shared: &Shared) {
    let mut pending: VecDeque<TimedOsc> = VecDeque::with_capacity(OUT_RING);
    let mut sockets = Sockets::default();
    let mut buf = Vec::with_capacity(256);
    let mut last_activity = 0u64;
    while !shared.stop.load(Ordering::Relaxed) {
        while let Ok(m) = rx.pop() {
            pending.push_back(m);
        }
        let now = clock::now_ns();
        if !pending.is_empty() {
            last_activity = now;
        }
        // In order, even if a later callback stamped a slightly earlier time.
        while pending.front().is_some_and(|m| m.time_ns <= now) {
            let Some(m) = pending.pop_front() else { break };
            let ok = encode(&m.msg, &mut buf)
                && sockets
                    .for_target(&m.msg.target)
                    .is_some_and(|s| s.send_to(&buf, m.msg.target).is_ok());
            if ok {
                shared.sent.fetch_add(1, Ordering::Relaxed);
            } else {
                shared.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
        let idle = now.saturating_sub(last_activity) > IDLE_AFTER_NS;
        let wait = match pending.front() {
            Some(m) => Duration::from_nanos(m.time_ns.saturating_sub(now)).min(ACTIVE_WAIT),
            None if idle => IDLE_WAIT,
            None => ACTIVE_WAIT,
        };
        std::thread::sleep(wait.max(Duration::from_micros(50)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn messages_reach_their_target_in_order_one_block_late() {
        let rx_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx_sock
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let target = rx_sock.local_addr().unwrap();
        let (out, mut queue) = OscOutput::new();
        let t = clock::now_ns();
        let msgs = [
            OscMessage::new(0, target, "/level", &[0.5]).unwrap(),
            OscMessage::new(240, target, "/xy", &[0.25, -1.0]).unwrap(),
            OscMessage::new(479, target, "/bang", &[]).unwrap(),
        ];
        assert_no_alloc::assert_no_alloc(|| queue.push_block(t, 480, 48_000.0, &msgs));
        let mut buf = [0u8; 1024];
        let mut got = Vec::new();
        for _ in 0..3 {
            let (n, _) = rx_sock.recv_from(&mut buf).unwrap();
            let when = clock::now_ns();
            let (_, p) = rosc::decoder::decode_udp(&buf[..n]).unwrap();
            got.push((when, p));
        }
        let addr_args = |p: &OscPacket| match p {
            OscPacket::Message(m) => (m.addr.clone(), m.args.clone()),
            OscPacket::Bundle(_) => panic!("a bundle"),
        };
        assert_eq!(
            addr_args(&got[0].1),
            ("/level".to_string(), vec![OscType::Float(0.5)])
        );
        assert_eq!(
            addr_args(&got[1].1),
            (
                "/xy".to_string(),
                vec![OscType::Float(0.25), OscType::Float(-1.0)]
            )
        );
        assert_eq!(addr_args(&got[2].1), ("/bang".to_string(), vec![]));
        // Not before their time: one 10 ms block late, then 5 ms apart.
        assert!(got[0].0 >= t + 10_000_000, "{} vs {t}", got[0].0);
        assert!(got[1].0 >= t + 15_000_000);
        let t0 = Instant::now();
        while out.sent() < 3 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(out.sent(), 3);
        assert_eq!(out.dropped(), 0);
        assert_eq!(out.errors(), 0);
    }

    #[test]
    fn a_full_queue_drops_and_counts() {
        let (tx, mut rx) = RingBuffer::new(OUT_RING);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut queue = OscOutQueue {
            tx,
            dropped: dropped.clone(),
        };
        let to: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let m = OscMessage::new(0, to, "/x", &[1.0]).unwrap();
        let msgs = vec![m; OUT_RING + 5];
        assert_no_alloc::assert_no_alloc(|| queue.push_block(0, 64, 48_000.0, &msgs));
        assert_eq!(dropped.load(Ordering::Relaxed), 5);
        assert_eq!(rx.pop().unwrap().msg, m);
    }

    #[test]
    fn an_idle_output_stops_promptly() {
        let (out, _queue) = OscOutput::new();
        assert!(out.is_running());
        std::thread::sleep(Duration::from_millis(20));
        let t0 = Instant::now();
        drop(out);
        assert!(t0.elapsed() < Duration::from_secs(1));
    }
}
