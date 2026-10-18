//! Open Sound Control input (ARCHITECTURE §8 "Events in", §11).
//!
//! [`OscInput`] listens on a UDP port on its own thread. Each packet is
//! stamped with [`crate::clock::now_ns`] when it arrives (bundle time tags
//! are ignored), decoded, and every argument of every message becomes a
//! control event, `Control::Osc { address: osc_address_hash(addr), arg: i }`:
//! integers, floats, doubles, and booleans as `f32`; other argument types
//! are skipped (their position still counts). A message without arguments
//! becomes argument 0 with value 1, so it works as a trigger. Bundles are
//! flattened in order.
//!
//! Bad packets are counted and ignored; nothing a sender does can panic the
//! listener or block the audio thread.

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use ac_core::{Control, osc_address_hash};
use rosc::{OscPacket, OscType};

use crate::clock;
use crate::control::ControlSender;

/// The port the editor suggests for OSC input.
pub const DEFAULT_OSC_PORT: u16 = 9000;
/// How often the listener wakes to check whether it should stop.
const POLL: Duration = Duration::from_millis(50);
/// The largest UDP payload.
const MAX_PACKET: usize = 65_536;
/// Bundles nested deeper than this are refused (the decoder recurses).
const MAX_BUNDLE_DEPTH: usize = 16;
/// Arguments past this index are ignored (`arg` is a `u8`).
const MAX_ARGS: usize = 256;

#[derive(Debug, Default)]
struct Counters {
    packets: AtomicU64,
    messages: AtomicU64,
    dropped: AtomicU64,
    malformed: AtomicU64,
}

/// What an [`OscInput`] has seen so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OscStats {
    /// UDP packets received.
    pub packets: u64,
    /// Messages decoded (bundles count each message inside).
    pub messages: u64,
    /// Events the control ring had no room for.
    pub dropped: u64,
    /// Packets that weren't valid OSC.
    pub malformed: u64,
}

/// A running OSC listener. Dropping it stops the thread and closes the port.
pub struct OscInput {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for OscInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscInput")
            .field("addr", &self.addr)
            .field("stats", &self.stats())
            .finish()
    }
}

impl OscInput {
    /// Listen on UDP `port` on every network interface, feeding `sender`.
    pub fn listen(port: u16, sender: ControlSender) -> io::Result<OscInput> {
        OscInput::bind(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
            sender,
        )
    }

    /// Listen on `addr` (tests bind `127.0.0.1:0`).
    pub fn bind(addr: SocketAddr, mut sender: ControlSender) -> io::Result<OscInput> {
        clock::init();
        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(POLL))?;
        let local = socket.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(Counters::default());
        let (flag, count) = (stop.clone(), counters.clone());
        let thread = std::thread::Builder::new()
            .name("ac-osc-in".into())
            .spawn(move || {
                let mut buf = vec![0u8; MAX_PACKET];
                while !flag.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf) {
                        Ok((n, _from)) => {
                            let t = clock::now_ns();
                            let packet = buf.get(..n).unwrap_or(&[]);
                            receive(packet, t, &mut sender, &count);
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                            ) => {}
                        // Windows reports an earlier ICMP error as a reset on
                        // the next read; anything else, don't spin on it.
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            })?;
        Ok(OscInput {
            addr: local,
            stop,
            counters,
            thread: Some(thread),
        })
    }

    /// The address the listener is bound to (with the real port when bound
    /// to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn stats(&self) -> OscStats {
        let c = &self.counters;
        OscStats {
            packets: c.packets.load(Ordering::Relaxed),
            messages: c.messages.load(Ordering::Relaxed),
            dropped: c.dropped.load(Ordering::Relaxed),
            malformed: c.malformed.load(Ordering::Relaxed),
        }
    }

    /// Whether the listening thread is still running.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for OscInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Handle one packet received at `t`.
fn receive(packet: &[u8], t: u64, sender: &mut ControlSender, counters: &Counters) {
    counters.packets.fetch_add(1, Ordering::Relaxed);
    let result = decode(packet, |control, value| {
        if !sender.send_at(t, control, value) {
            counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    });
    match result {
        Ok(messages) => {
            counters
                .messages
                .fetch_add(messages as u64, Ordering::Relaxed);
        }
        Err(_) => {
            counters.malformed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A packet that isn't valid OSC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OscDecodeError(pub String);

impl fmt::Display for OscDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OscDecodeError {}

/// Decode one UDP packet and hand each control event to `emit`, in order.
/// Returns how many messages it held.
pub fn decode(packet: &[u8], mut emit: impl FnMut(Control, f32)) -> Result<usize, OscDecodeError> {
    if !nesting_ok(packet, 0) {
        return Err(OscDecodeError("bundles nested too deeply".into()));
    }
    let (_, decoded) =
        rosc::decoder::decode_udp(packet).map_err(|e| OscDecodeError(format!("{e:?}")))?;
    // Flatten bundles without recursion, keeping the elements' order.
    let mut stack = vec![decoded];
    let mut messages = 0;
    while let Some(p) = stack.pop() {
        match p {
            OscPacket::Message(m) => {
                messages += 1;
                let address = osc_address_hash(&m.addr);
                if m.args.is_empty() {
                    emit(Control::Osc { address, arg: 0 }, 1.0);
                }
                for (i, a) in m.args.iter().enumerate().take(MAX_ARGS) {
                    if let Some(v) = arg_value(a) {
                        emit(
                            Control::Osc {
                                address,
                                arg: i as u8,
                            },
                            v,
                        );
                    }
                }
            }
            OscPacket::Bundle(b) => stack.extend(b.content.into_iter().rev()),
        }
    }
    Ok(messages)
}

/// An argument as a control value: numbers and booleans; finite only.
fn arg_value(a: &OscType) -> Option<f32> {
    let v = match a {
        OscType::Int(v) => *v as f32,
        OscType::Float(v) => *v,
        OscType::Double(v) => *v as f32,
        OscType::Long(v) => *v as f32,
        OscType::Bool(b) => f32::from(u8::from(*b)),
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// Whether bundles in `packet` nest no deeper than [`MAX_BUNDLE_DEPTH`].
/// A cheap walk over the bundle framing before the (recursive) decoder sees
/// it; anything that doesn't parse is left for the decoder to reject.
fn nesting_ok(packet: &[u8], depth: usize) -> bool {
    if !packet.starts_with(b"#bundle\0") {
        return true;
    }
    if depth >= MAX_BUNDLE_DEPTH {
        return false;
    }
    // Tag (8 bytes) and time tag (8 bytes), then size-prefixed elements.
    let mut rest = packet.get(16..).unwrap_or(&[]);
    while let Some(size) = rest.get(..4) {
        let n = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
        let Some(element) = rest.get(4..4usize.saturating_add(n)) else {
            return true;
        };
        if !nesting_ok(element, depth + 1) {
            return false;
        }
        rest = rest.get(4 + n..).unwrap_or(&[]);
    }
    true
}

/// This computer's address on the local network, for telling a phone or
/// tablet where to send OSC. No packets are sent.
pub fn local_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // Connecting a UDP socket only picks a route; a documentation address
    // (TEST-NET-1) is never contacted.
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlInputs;
    use ac_core::ControlEvent;
    use rosc::{OscBundle, OscMessage, OscTime};
    use std::time::Instant;

    fn msg(addr: &str, args: Vec<OscType>) -> OscPacket {
        OscPacket::Message(OscMessage {
            addr: addr.into(),
            args,
        })
    }

    fn bundle(content: Vec<OscPacket>) -> OscPacket {
        OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 1,
                fractional: 0,
            },
            content,
        })
    }

    fn encode(p: &OscPacket) -> Vec<u8> {
        rosc::encoder::encode(p).unwrap()
    }

    fn osc(addr: &str, arg: u8) -> Control {
        Control::Osc {
            address: osc_address_hash(addr),
            arg,
        }
    }

    fn decoded(p: &[u8]) -> Result<Vec<(Control, f32)>, OscDecodeError> {
        let mut v = Vec::new();
        decode(p, |c, x| v.push((c, x)))?;
        Ok(v)
    }

    #[test]
    fn arguments_become_controls_by_position() {
        let p = encode(&msg(
            "/mix/fader",
            vec![
                OscType::Float(0.5),
                OscType::Int(3),
                OscType::String("label".into()),
                OscType::Double(0.25),
                OscType::Bool(true),
                OscType::Long(-2),
                OscType::Float(f32::NAN),
            ],
        ));
        assert_eq!(
            decoded(&p).unwrap(),
            vec![
                (osc("/mix/fader", 0), 0.5),
                (osc("/mix/fader", 1), 3.0),
                // The string at 2 is skipped; positions still count.
                (osc("/mix/fader", 3), 0.25),
                (osc("/mix/fader", 4), 1.0),
                (osc("/mix/fader", 5), -2.0),
                // NaN never reaches the engine.
            ]
        );
    }

    #[test]
    fn a_message_without_arguments_is_a_trigger() {
        let p = encode(&msg("/go", vec![]));
        assert_eq!(decoded(&p).unwrap(), vec![(osc("/go", 0), 1.0)]);
    }

    #[test]
    fn bundles_flatten_in_order() {
        let p = encode(&bundle(vec![
            msg("/a", vec![OscType::Float(1.0)]),
            bundle(vec![
                msg("/b", vec![OscType::Float(2.0)]),
                msg("/c", vec![]),
            ]),
            msg("/d", vec![OscType::Int(4)]),
        ]));
        let mut got = Vec::new();
        let n = decode(&p, |c, v| got.push((c, v))).unwrap();
        assert_eq!(n, 4);
        assert_eq!(
            got,
            vec![
                (osc("/a", 0), 1.0),
                (osc("/b", 0), 2.0),
                (osc("/c", 0), 1.0),
                (osc("/d", 0), 4.0),
            ]
        );
    }

    #[test]
    fn garbage_is_rejected_without_panicking() {
        let mut cases: Vec<Vec<u8>> = vec![
            vec![],
            b"hello".to_vec(),
            b"/addr".to_vec(),
            b"/addr\0\0\0,f\0\0".to_vec(),
            b"#bundle\0".to_vec(),
            b"#bundle\0\0\0\0\0\0\0\0\0\xff\xff\xff\xff".to_vec(),
            vec![0xff; 64],
        ];
        // Truncations of a valid message and bundle.
        let good = encode(&bundle(vec![msg(
            "/x",
            vec![OscType::Float(1.0), OscType::String("s".into())],
        )]));
        for n in 0..good.len() {
            cases.push(good[..n].to_vec());
        }
        // A pseudo-random fuzz of bytes after a valid prefix.
        let mut x: u32 = 0x1234_5678;
        for len in 1..200 {
            let mut v = b"/f\0\0,ifsb\0\0\0".to_vec();
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                v.push(x as u8);
            }
            cases.push(v);
        }
        for c in &cases {
            let _ = decoded(c);
        }
        assert!(decoded(b"hello").is_err());
        assert!(decoded(&[]).is_err());
    }

    #[test]
    fn deeply_nested_bundles_are_refused_before_decoding() {
        // Each level: "#bundle\0", a time tag, then one size-prefixed
        // element holding the next level.
        let mut p = encode(&msg("/deep", vec![]));
        for _ in 0..3000 {
            let mut outer = b"#bundle\0".to_vec();
            outer.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
            outer.extend_from_slice(&(p.len() as u32).to_be_bytes());
            outer.extend_from_slice(&p);
            p = outer;
        }
        assert!(decoded(&p).is_err());
        // Shallow nesting is fine.
        let mut q = encode(&msg("/shallow", vec![]));
        for _ in 0..4 {
            q = encode(&bundle(vec![rosc::decoder::decode_udp(&q).unwrap().1]));
        }
        assert_eq!(decoded(&q).unwrap(), vec![(osc("/shallow", 0), 1.0)]);
    }

    /// Wait until `rx` has delivered `n` events (or time out).
    fn collect(rx: &mut crate::control::ControlReceiver, n: usize) -> Vec<ControlEvent> {
        let mut all = Vec::new();
        let mut out = Vec::with_capacity(64);
        let t0 = Instant::now();
        while all.len() < n && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
            rx.drain(clock::now_ns(), 480, 48_000.0, &mut out);
            all.extend_from_slice(&out);
        }
        all
    }

    #[test]
    fn loopback_messages_bundles_and_garbage() {
        let (mut inputs, mut rx) = ControlInputs::new();
        let sender = inputs.sender("osc").unwrap();
        let listener = OscInput::bind("127.0.0.1:0".parse().unwrap(), sender).unwrap();
        let to = listener.local_addr();
        assert_ne!(to.port(), 0);
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(&encode(&msg("/fader/1", vec![OscType::Float(0.75)])), to)
            .unwrap();
        tx.send_to(b"not osc at all", to).unwrap();
        tx.send_to(
            &encode(&bundle(vec![
                msg("/xy", vec![OscType::Float(0.1), OscType::Float(0.9)]),
                msg("/button", vec![]),
            ])),
            to,
        )
        .unwrap();
        tx.send_to(&[0xff; 3], to).unwrap();
        let got = collect(&mut rx, 4);
        let got: Vec<(Control, f32)> = got.iter().map(|e| (e.control, e.value)).collect();
        assert_eq!(
            got,
            vec![
                (osc("/fader/1", 0), 0.75),
                (osc("/xy", 0), 0.1),
                (osc("/xy", 1), 0.9),
                (osc("/button", 0), 1.0),
            ]
        );
        let t0 = Instant::now();
        while listener.stats().packets < 4 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            listener.stats(),
            OscStats {
                packets: 4,
                messages: 3,
                dropped: 0,
                malformed: 2,
            }
        );
        assert!(listener.is_running());
        let t0 = Instant::now();
        drop(listener);
        assert!(t0.elapsed() < Duration::from_secs(1), "stops promptly");
        // The port is free again.
        assert!(UdpSocket::bind(to).is_ok());
    }

    #[test]
    fn a_taken_port_is_an_error() {
        let (mut inputs, _rx) = ControlInputs::new();
        let held = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = held.local_addr().unwrap();
        let sender = inputs.sender("osc").unwrap();
        assert!(OscInput::bind(addr, sender).is_err());
    }

    #[test]
    fn local_ip_never_panics() {
        // No network in some test machines: either answer is fine.
        let _ = local_ip();
    }
}
