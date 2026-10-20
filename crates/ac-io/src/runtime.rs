//! What every live host has in common (ARCHITECTURE §8, §11).
//!
//! - [`AudioHost`]: a configured backend, ready to run an engine. It knows
//!   its sample rate and buffer size before it starts, so the engine can be
//!   built at the device's rate. [`crate::audio::CpalAudio`] (devices) and
//!   [`crate::null::NullAudio`] (no devices, paced by a timer) implement it.
//! - [`AudioStream`]: a running host. It owns the engine's
//!   [`Processor`], reports counters, measures round-trip latency, and gives
//!   the processor back when stopped so the editor can switch devices
//!   without rebuilding the engine.
//! - [`CallbackCore`]: the device callback both hosts run. It pulls
//!   device input out of the duplex ring, drains MIDI and control-surface
//!   events ([`EventPorts`]), runs the processor (or the latency probe),
//!   queues the MIDI and OSC the patch sent, and hands each processed chunk
//!   to the host to write in the device's format. It never allocates,
//!   locks, or blocks.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use ac_core::{ControlEvent, MidiEvent, OSC_OUT_CAPACITY, OscMessage};
use ac_engine::{EngineStatus, HostIo, Processor};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::clock;
use crate::control::{ControlReceiver, MAX_CONTROLS_PER_CALLBACK};
use crate::convert::deinterleave_parts;
use crate::latency::{Latency, LatencyError, Probe};
use crate::midi::{MAX_EVENTS_PER_CALLBACK, MidiReceiver};
use crate::midi_out::MidiOutQueue;
use crate::osc_out::OscOutQueue;

/// Frames processed per pass. Device callbacks larger than this are split;
/// scratch buffers are sized for it once, up front.
pub const CHUNK_FRAMES: usize = 8192;

/// What a host is (or will be) running with.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub host: String,
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub sample_rate: u32,
    /// Frames per callback: what was requested, or what the device reports.
    pub buffer_size: u32,
    pub in_channels: usize,
    pub out_channels: usize,
    /// Things worth telling the person: fallbacks taken, inputs disabled.
    pub notes: Vec<String>,
}

impl StreamInfo {
    /// A good engine `max_block` for this stream: one callback, within the
    /// range the engine is tuned for. Longer callbacks are split.
    pub fn max_block(&self) -> usize {
        (self.buffer_size as usize).clamp(32, 4096)
    }
}

/// Counters a running stream keeps (shared with its callbacks).
#[derive(Debug, Default)]
pub struct StreamCounters {
    pub callbacks: AtomicU64,
    pub max_callback_frames: AtomicU64,
    /// Callbacks where the input ring ran dry (zeros were used).
    pub input_underruns: AtomicU64,
    /// Times the input ring backed up and old input was skipped.
    pub input_overruns: AtomicU64,
    /// Device errors other than xruns.
    pub device_errors: AtomicU64,
    /// The OS refused real-time priority for the callback thread.
    pub realtime_denied: AtomicBool,
    /// The device went away; the stream must be rebuilt.
    pub disconnected: AtomicBool,
    /// Code of the last device error (see [`error_text`]); 0 for none.
    pub last_error: AtomicU8,
}

/// A snapshot of [`StreamCounters`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamStats {
    pub callbacks: u64,
    pub max_callback_frames: u64,
    pub input_underruns: u64,
    pub input_overruns: u64,
    pub device_errors: u64,
    pub realtime_denied: bool,
    pub disconnected: bool,
    pub last_error: Option<&'static str>,
}

impl StreamCounters {
    pub fn snapshot(&self) -> StreamStats {
        StreamStats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            max_callback_frames: self.max_callback_frames.load(Ordering::Relaxed),
            input_underruns: self.input_underruns.load(Ordering::Relaxed),
            input_overruns: self.input_overruns.load(Ordering::Relaxed),
            device_errors: self.device_errors.load(Ordering::Relaxed),
            realtime_denied: self.realtime_denied.load(Ordering::Relaxed),
            disconnected: self.disconnected.load(Ordering::Relaxed),
            last_error: error_text(self.last_error.load(Ordering::Relaxed)),
        }
    }
}

/// Device error codes stored in [`StreamCounters::last_error`]; an atomic
/// code rather than a string, because errors arrive on the audio thread.
pub mod error_code {
    pub const DEVICE_GONE: u8 = 1;
    pub const STREAM_INVALID: u8 = 2;
    pub const BACKEND: u8 = 3;
    pub const OTHER: u8 = 4;
}

pub fn error_text(code: u8) -> Option<&'static str> {
    match code {
        error_code::DEVICE_GONE => Some("the audio device was disconnected"),
        error_code::STREAM_INVALID => Some("the audio stream must be restarted"),
        error_code::BACKEND => Some("the audio driver reported an error"),
        error_code::OTHER => Some("an audio error occurred"),
        _ => None,
    }
}

/// A host couldn't be opened or started.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioError {
    UnknownHost {
        name: String,
        available: Vec<String>,
    },
    NoOutputDevice {
        host: String,
    },
    DeviceNotFound {
        kind: &'static str,
        name: String,
        available: Vec<String>,
    },
    UnsupportedConfig {
        device: String,
        reason: String,
    },
    Backend(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[String]| {
            if v.is_empty() {
                "none".to_string()
            } else {
                v.join(", ")
            }
        };
        match self {
            AudioError::UnknownHost { name, available } => write!(
                f,
                "no audio host named `{name}` (available: {})",
                list(available)
            ),
            AudioError::NoOutputDevice { host } => {
                write!(f, "no audio output device found ({host})")
            }
            AudioError::DeviceNotFound {
                kind,
                name,
                available,
            } => write!(
                f,
                "no {kind} device matching `{name}` (available: {})",
                list(available)
            ),
            AudioError::UnsupportedConfig { device, reason } => {
                write!(f, "{device}: {reason}")
            }
            AudioError::Backend(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for AudioError {}

/// The event sources and sinks a stream connects to its engine: MIDI in and
/// out, control surfaces (keys, gamepads, OSC) in, and OSC out. Each is
/// optional; a stream without one passes the engine nothing for it.
#[derive(Debug, Default)]
pub struct EventPorts {
    /// Messages from MIDI devices, for `MidiIn` nodes.
    pub midi_in: Option<MidiReceiver>,
    /// Where `MidiOut` nodes' messages go.
    pub midi_out: Option<MidiOutQueue>,
    /// Key, gamepad, and OSC events, for control-surface nodes and bindings.
    pub controls: Option<ControlReceiver>,
    /// Where `OscOut` nodes' messages go.
    pub osc_out: Option<OscOutQueue>,
}

impl EventPorts {
    /// MIDI input only.
    pub fn midi(midi_in: Option<MidiReceiver>) -> EventPorts {
        EventPorts {
            midi_in,
            ..EventPorts::default()
        }
    }
}

/// A configured audio backend, ready to run an engine built at
/// [`StreamInfo::sample_rate`].
pub trait AudioHost {
    fn info(&self) -> &StreamInfo;

    /// Start calling `processor` from the device callback, feeding it MIDI
    /// from `midi` if given. On failure the processor comes back with the
    /// error (when the backend released it), so another device can be tried.
    fn start(
        self: Box<Self>,
        processor: Processor,
        midi: Option<MidiReceiver>,
    ) -> Result<Box<dyn AudioStream>, StartError> {
        self.start_with(processor, EventPorts::midi(midi))
    }

    /// [`AudioHost::start`] with every event port: MIDI and OSC output, and
    /// control-surface input.
    fn start_with(
        self: Box<Self>,
        processor: Processor,
        ports: EventPorts,
    ) -> Result<Box<dyn AudioStream>, StartError>;
}

/// A stream that couldn't start, and the engine half it was given.
pub struct StartError {
    pub error: AudioError,
    pub returned: Option<Box<Stopped>>,
}

impl fmt::Debug for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartError")
            .field("error", &self.error)
            .field("returned", &self.returned.is_some())
            .finish()
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for StartError {}

/// A running host. Dropping it stops the stream.
pub trait AudioStream: Send {
    fn info(&self) -> &StreamInfo;

    fn stats(&self) -> StreamStats;

    /// Begin a round-trip latency measurement: about a second of test signal
    /// on every output instead of the engine. Poll with
    /// [`AudioStream::poll_latency`].
    fn start_latency(&mut self) -> Result<(), LatencyError>;

    /// The measurement's result once the probe has come back.
    fn poll_latency(&mut self) -> Option<Result<Latency, LatencyError>>;

    /// Measure and wait (up to `timeout`) for the result.
    fn measure_latency(&mut self, timeout: Duration) -> Result<Latency, LatencyError> {
        self.start_latency()?;
        let t0 = Instant::now();
        loop {
            if let Some(r) = self.poll_latency() {
                return r;
            }
            if t0.elapsed() > timeout {
                return Err(LatencyError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Stop the stream and take the processor (and its event ports) back,
    /// to restart on another device at the same rate. `None` if the backend
    /// didn't give them back in time.
    fn stop(self: Box<Self>) -> Option<Stopped>;
}

/// What a stopped stream hands back: the processor and the event ports, to
/// start the next stream with.
pub struct Stopped {
    pub processor: Processor,
    pub ports: EventPorts,
}

impl fmt::Debug for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stopped")
            .field("ports", &self.ports)
            .finish_non_exhaustive()
    }
}

/// The device input side of a duplex stream: the input callback pushes
/// interleaved `f32` frames, the output callback pulls them.
///
/// Two separate device streams never tick in lockstep, so the ring keeps a
/// one-buffer cushion: after a dry spell it waits until a buffer more than
/// needed has arrived before using input again, and if input piles up
/// (clock drift between devices) it skips back to the cushion. Both events
/// are counted as device xruns.
pub(crate) struct InputRing {
    rx: Consumer<f32>,
    channels: usize,
    cushion: usize,
    primed: bool,
}

/// Capacity of the duplex ring for a stream.
pub(crate) fn input_ring_frames(buffer_size: u32) -> usize {
    (buffer_size as usize * 8).max(CHUNK_FRAMES * 2)
}

impl InputRing {
    pub(crate) fn new(rx: Consumer<f32>, channels: usize, buffer_size: u32) -> InputRing {
        InputRing {
            rx,
            channels: channels.max(1),
            cushion: (buffer_size as usize).clamp(16, CHUNK_FRAMES),
            primed: false,
        }
    }

    /// Fill `frames` frames of channel-major `dst` with device input.
    fn read(
        &mut self,
        dst: &mut [f32],
        frames: usize,
        counters: &StreamCounters,
        status: &EngineStatus,
    ) {
        let ch = self.channels;
        if let Some(d) = dst.get_mut(..ch * frames) {
            d.fill(0.0);
        }
        let avail = self.rx.slots() / ch;
        if !self.primed {
            if avail < frames + self.cushion {
                return;
            }
            self.primed = true;
        }
        if avail > frames + 3 * self.cushion {
            let skip = (avail - frames - self.cushion) * ch;
            if let Ok(c) = self.rx.read_chunk(skip) {
                c.commit_all();
            }
            counters.input_overruns.fetch_add(1, Ordering::Relaxed);
            status.device_xruns.fetch_add(1, Ordering::Relaxed);
        }
        let take = self.rx.slots().min(frames * ch) / ch;
        if let Ok(chunk) = self.rx.read_chunk(take * ch) {
            deinterleave_parts(chunk.as_slices(), ch, dst, frames, 0);
            chunk.commit_all();
        }
        if take < frames {
            self.primed = false;
            counters.input_underruns.fetch_add(1, Ordering::Relaxed);
            status.device_xruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The audio-thread end of latency probes.
pub(crate) struct ProbeAudio {
    req_rx: Consumer<Box<Probe>>,
    res_tx: Producer<Box<Probe>>,
    active: Option<Box<Probe>>,
}

/// The control end of latency probes.
pub(crate) struct ProbeControl {
    req_tx: Producer<Box<Probe>>,
    res_rx: Consumer<Box<Probe>>,
    in_flight: bool,
    sample_rate: u32,
    in_channels: usize,
}

pub(crate) fn probe_pair(sample_rate: u32, in_channels: usize) -> (ProbeControl, ProbeAudio) {
    let (req_tx, req_rx) = RingBuffer::new(1);
    let (res_tx, res_rx) = RingBuffer::new(2);
    (
        ProbeControl {
            req_tx,
            res_rx,
            in_flight: false,
            sample_rate,
            in_channels,
        },
        ProbeAudio {
            req_rx,
            res_tx,
            active: None,
        },
    )
}

impl ProbeControl {
    pub(crate) fn start(&mut self) -> Result<(), LatencyError> {
        if self.in_channels == 0 {
            return Err(LatencyError::NoInput);
        }
        // A result from a measurement that timed out is stale.
        while let Ok(stale) = self.res_rx.pop() {
            drop(stale);
            self.in_flight = false;
        }
        if self.in_flight {
            return Err(LatencyError::Busy);
        }
        let probe = Box::new(Probe::new(self.sample_rate, self.in_channels));
        self.req_tx.push(probe).map_err(|_| LatencyError::Busy)?;
        self.in_flight = true;
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Option<Result<Latency, LatencyError>> {
        if !self.in_flight {
            return None;
        }
        let probe = self.res_rx.pop().ok()?;
        self.in_flight = false;
        Some(probe.analyze())
    }
}

impl ProbeAudio {
    /// Run the probe for this chunk if one is active. Returns false when the
    /// engine should run instead.
    fn step(
        &mut self,
        input: &[f32],
        in_channels: usize,
        output: &mut [f32],
        out_channels: usize,
        frames: usize,
    ) -> bool {
        if self.active.as_ref().is_some_and(|p| p.done()) {
            self.hand_back();
        }
        if self.active.is_none()
            && let Ok(p) = self.req_rx.pop()
        {
            self.active = Some(p);
        }
        let Some(p) = self.active.as_mut() else {
            return false;
        };
        if p.done() {
            // Finished but not yet handed back (result ring full).
            return false;
        }
        p.step(input, in_channels, output, out_channels, frames);
        if p.done() {
            self.hand_back();
        }
        true
    }

    /// Return the finished probe; it's freed on the control thread.
    fn hand_back(&mut self) {
        if let Some(p) = self.active.take()
            && let Err(rtrb::PushError::Full(p)) = self.res_tx.push(p)
        {
            self.active = Some(p);
        }
    }
}

/// The device callback shared by every live host.
pub(crate) struct CallbackCore {
    processor: Option<Processor>,
    /// Where the processor goes when the callback is dropped.
    return_tx: Option<mpsc::Sender<Stopped>>,
    status: Arc<EngineStatus>,
    counters: Arc<StreamCounters>,
    sample_rate: f64,
    in_channels: usize,
    out_channels: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    input_ring: Option<InputRing>,
    ports: EventPorts,
    midi_all: Vec<MidiEvent>,
    midi_chunk: Vec<MidiEvent>,
    midi_out_chunk: Vec<MidiEvent>,
    controls_all: Vec<ControlEvent>,
    controls_chunk: Vec<ControlEvent>,
    osc_out_chunk: Vec<OscMessage>,
    probe: ProbeAudio,
    /// Ask for real-time priority on the first callback (see `priority.rs`).
    promote: bool,
}

pub(crate) struct CoreParts {
    pub processor: Processor,
    pub counters: Arc<StreamCounters>,
    pub sample_rate: u32,
    pub in_channels: usize,
    pub out_channels: usize,
    pub input_ring: Option<InputRing>,
    pub ports: EventPorts,
    pub promote: bool,
}

/// The control-side handles that go with a [`CallbackCore`].
pub(crate) struct CoreControl {
    pub probe: ProbeControl,
    pub returned: mpsc::Receiver<Stopped>,
}

impl CallbackCore {
    /// Build the callback state. Allocates everything it will ever need;
    /// call before the stream starts.
    pub(crate) fn new(parts: CoreParts) -> (CallbackCore, CoreControl) {
        let CoreParts {
            mut processor,
            counters,
            sample_rate,
            in_channels,
            out_channels,
            input_ring,
            ports,
            promote,
        } = parts;
        clock::init();
        processor.set_clock(Some(clock::now_ns));
        let (probe_ctl, probe) = probe_pair(sample_rate, in_channels);
        let (return_tx, returned) = mpsc::channel();
        let core = CallbackCore {
            status: processor.status().clone(),
            processor: Some(processor),
            return_tx: Some(return_tx),
            counters,
            sample_rate: f64::from(sample_rate),
            in_channels,
            out_channels,
            input: vec![0.0; in_channels * CHUNK_FRAMES],
            output: vec![0.0; out_channels.max(1) * CHUNK_FRAMES],
            input_ring,
            ports,
            midi_all: Vec::with_capacity(MAX_EVENTS_PER_CALLBACK),
            midi_chunk: Vec::with_capacity(MAX_EVENTS_PER_CALLBACK),
            midi_out_chunk: Vec::with_capacity(MAX_EVENTS_PER_CALLBACK),
            controls_all: Vec::with_capacity(MAX_CONTROLS_PER_CALLBACK),
            controls_chunk: Vec::with_capacity(MAX_CONTROLS_PER_CALLBACK),
            osc_out_chunk: Vec::with_capacity(OSC_OUT_CAPACITY),
            probe,
            promote,
        };
        (
            core,
            CoreControl {
                probe: probe_ctl,
                returned,
            },
        )
    }

    /// One device callback of `frames` frames. `write(chunk, n, offset)`
    /// receives each processed chunk (channel-major, `n` frames, starting at
    /// frame `offset` of the callback) to convert into the device buffer.
    /// Real-time safe: asserted allocation-free in debug builds.
    pub(crate) fn run(&mut self, frames: usize, write: impl FnMut(&[f32], usize, usize)) {
        self.run_at(clock::now_ns(), frames, write);
    }

    /// [`CallbackCore::run`] for a callback that started at `t` on [`clock`].
    pub(crate) fn run_at(
        &mut self,
        t: u64,
        frames: usize,
        mut write: impl FnMut(&[f32], usize, usize),
    ) {
        if self.promote {
            // Once, before the guarded section: one system call at stream start.
            self.promote = false;
            if !crate::priority::promote_current_thread() {
                self.counters.realtime_denied.store(true, Ordering::Relaxed);
            }
        }
        assert_no_alloc::assert_no_alloc(|| {
            self.counters.callbacks.fetch_add(1, Ordering::Relaxed);
            self.counters
                .max_callback_frames
                .fetch_max(frames as u64, Ordering::Relaxed);
            if frames == 0 {
                // Leave events queued for a callback that can place them.
                return;
            }
            let rate = self.sample_rate;
            match self.ports.midi_in.as_mut() {
                Some(m) => m.drain(t, frames, rate, &mut self.midi_all),
                None => self.midi_all.clear(),
            }
            match self.ports.controls.as_mut() {
                Some(c) => c.drain(t, frames, rate, &mut self.controls_all),
                None => self.controls_all.clear(),
            }
            let (ic, oc) = (self.in_channels, self.out_channels.max(1));
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(CHUNK_FRAMES);
                let input = &mut self.input[..ic * n];
                match self.input_ring.as_mut() {
                    Some(r) => r.read(input, n, &self.counters, &self.status),
                    None => input.fill(0.0),
                }
                self.midi_chunk.clear();
                for e in &self.midi_all {
                    let off = e.offset as usize;
                    if off >= done && off < done + n {
                        self.midi_chunk
                            .push(MidiEvent::new((off - done) as u32, e.msg));
                    }
                }
                self.controls_chunk.clear();
                for e in &self.controls_all {
                    let off = e.offset as usize;
                    if off >= done && off < done + n {
                        self.controls_chunk.push(ControlEvent {
                            offset: (off - done) as u32,
                            ..*e
                        });
                    }
                }
                let output = &mut self.output[..oc * n];
                let probing = self.probe.step(&self.input[..ic * n], ic, output, oc, n);
                if !probing {
                    match self.processor.as_mut() {
                        Some(p) => {
                            self.midi_out_chunk.clear();
                            self.osc_out_chunk.clear();
                            let mut io = HostIo {
                                frames: n,
                                input: &self.input[..ic * n],
                                in_channels: ic,
                                output,
                                out_channels: oc,
                                midi: &self.midi_chunk,
                                midi_out: self
                                    .ports
                                    .midi_out
                                    .is_some()
                                    .then_some(&mut self.midi_out_chunk),
                                controls: &self.controls_chunk,
                                osc_out: self
                                    .ports
                                    .osc_out
                                    .is_some()
                                    .then_some(&mut self.osc_out_chunk),
                            };
                            p.process(&mut io);
                            let t_chunk = t + (done as f64 * 1e9 / rate) as u64;
                            if let Some(q) = self.ports.midi_out.as_mut() {
                                q.push_block(t_chunk, n, rate, &self.midi_out_chunk);
                            }
                            if let Some(q) = self.ports.osc_out.as_mut() {
                                q.push_block(t_chunk, n, rate, &self.osc_out_chunk);
                            }
                        }
                        None => output.fill(0.0),
                    }
                }
                write(&self.output[..oc * n], n, done);
                done += n;
            }
        });
    }
}

impl Drop for CallbackCore {
    fn drop(&mut self) {
        // Runs where the backend drops the callback (never inside it), so
        // sending is fine here.
        if let (Some(processor), Some(tx)) = (self.processor.take(), self.return_tx.take()) {
            let _ = tx.send(Stopped {
                processor,
                ports: std::mem::take(&mut self.ports),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    //! These run under the allocation guard (see `lib.rs`): any allocation
    //! inside `CallbackCore::run` aborts the test binary.

    use super::*;
    use crate::assets::{AssetStore, FileResources};
    use crate::midi::MidiInputs;
    use crate::record::DiskRecorder;
    use ac_core::{
        Config, Endpoint, Io, MidiMsg, Node, NodeId, NodeInstance, NodeMeta, PrepareCtx,
        ProcessCtx, Project, Registry, Resources, Signature, Wire, node,
    };
    use ac_engine::{Engine, EngineHandle, EngineOptions};
    use serde_json::json;
    use std::collections::VecDeque;

    const RATE: u32 = 48_000;

    node! {
        kind: "t_midi_marks",
        name: "MIDI Marks",
        category: Utility,
        keywords: [],
        /// Adds 1 to device output 0 at each MIDI event's offset. Try this: nothing.
        pub struct MidiMarks {}
    }

    impl Node for MidiMarks {
        fn meta() -> NodeMeta {
            Self::base_meta().host_output()
        }
        fn describe(_: &Config) -> Signature {
            // Nodes without sockets are skipped by the compiler (comments).
            Signature::new().input(ac_core::SocketSpec::float(
                "unused",
                ac_core::Unit::Generic,
                0.0,
            ))
        }
        fn new(_: &Config, _: &PrepareCtx) -> Self {
            MidiMarks {}
        }
        fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
            let midi = io.host.midi();
            if let Some(out) = io.host.output_channel(0) {
                for e in midi {
                    if let Some(s) = out.get_mut(e.offset as usize) {
                        *s += 1.0;
                    }
                }
            }
        }
        fn reset(&mut self) {}
    }

    node! {
        kind: "t_control_echo",
        name: "Control Echo",
        category: Utility,
        keywords: [],
        /// Adds each control event's value to device output 0 at its offset,
        /// and sends it as OSC `/echo` to the `to` address in its config.
        /// Try this: nothing.
        pub struct ControlEcho {
            to: Option<std::net::SocketAddr>,
        }
    }

    impl Node for ControlEcho {
        fn meta() -> NodeMeta {
            Self::base_meta().host_output()
        }
        fn describe(_: &Config) -> Signature {
            Signature::new().input(ac_core::SocketSpec::float(
                "unused",
                ac_core::Unit::Generic,
                0.0,
            ))
        }
        fn new(config: &Config, _: &PrepareCtx) -> Self {
            use ac_core::ConfigExt;
            ControlEcho {
                to: config.get_str_or("to", "").parse().ok(),
            }
        }
        fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
            let controls = io.host.controls();
            if let Some(out) = io.host.output_channel(0) {
                for e in controls {
                    if let Some(s) = out.get_mut(e.offset as usize) {
                        *s += e.value;
                    }
                }
            }
            if let Some(to) = self.to {
                for e in controls {
                    if let Some(m) = ac_core::OscMessage::new(e.offset, to, "/echo", &[e.value]) {
                        io.host.send_osc(m);
                    }
                }
            }
        }
        fn reset(&mut self) {}
    }

    fn registry() -> Arc<Registry> {
        let mut r = ac_nodes::registry();
        r.register::<MidiMarks>();
        r.register::<ControlEcho>();
        Arc::new(r)
    }

    fn wire(a: u64, out: &str, b: u64, input: &str) -> Wire {
        Wire::new(
            Endpoint::new(NodeId(a), out),
            Endpoint::new(NodeId(b), input),
        )
    }

    /// Audio In (2 ch) → Audio Out (2 ch).
    fn passthrough() -> Project {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "audio_in").with_config(json!({"channels": 2})));
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "audio_out").with_config(json!({"channels": 2})));
        p.graph.wires.push(wire(1, "out", 2, "in"));
        p
    }

    fn engine(
        project: Project,
        resources: Arc<dyn Resources>,
        block: usize,
    ) -> (EngineHandle, Processor) {
        let (engine, report) = Engine::new(
            project,
            registry(),
            resources,
            EngineOptions {
                sample_rate: f64::from(RATE),
                max_block: block,
                socket_telemetry: false,
                offline: false,
            },
        );
        assert!(!report.has_errors(), "{report}");
        engine.split()
    }

    /// A core with a two-channel duplex ring; returns the ring's producer.
    fn core(
        processor: Processor,
        buffer: u32,
        midi: Option<MidiReceiver>,
    ) -> (
        CallbackCore,
        CoreControl,
        Producer<f32>,
        Arc<StreamCounters>,
    ) {
        let (tx, rx) = RingBuffer::new(input_ring_frames(buffer) * 2);
        let counters = Arc::new(StreamCounters::default());
        let (core, ctl) = CallbackCore::new(CoreParts {
            processor,
            counters: counters.clone(),
            sample_rate: RATE,
            in_channels: 2,
            out_channels: 2,
            input_ring: Some(InputRing::new(rx, 2, buffer)),
            ports: EventPorts::midi(midi),
            promote: false,
        });
        (core, ctl, tx, counters)
    }

    /// Interleaved stereo frames `[start, start + n)` of a test pattern.
    fn push_frames(tx: &mut Producer<f32>, start: usize, n: usize) {
        for i in start..start + n {
            tx.push(i as f32 * 1e-5).unwrap();
            tx.push(-(i as f32) * 1e-5).unwrap();
        }
    }

    /// Run one callback, copying the channel-major output into `out`
    /// (preallocated: the guard forbids allocating inside `run`).
    fn run(core: &mut CallbackCore, frames: usize, out: &mut [f32]) {
        core.run(frames, |chunk, n, off| {
            for c in 0..2 {
                out[c * frames + off..c * frames + off + n]
                    .copy_from_slice(&chunk[c * n..(c + 1) * n]);
            }
        });
    }

    #[test]
    fn device_input_passes_through_the_duplex_ring() {
        let (_h, p) = engine(passthrough(), Arc::new(ac_core::NoResources), 64);
        let (mut core, _ctl, mut tx, counters) = core(p, 64, None);
        let mut out = vec![0.0; 2 * 64];
        // Not primed yet: silence, and no xrun counted.
        push_frames(&mut tx, 0, 64);
        run(&mut core, 64, &mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(counters.input_underruns.load(Ordering::Relaxed), 0);
        // A cushion's worth more arrives: input flows from the first frame.
        push_frames(&mut tx, 64, 64);
        let mut next = 0;
        for k in 0..20 {
            run(&mut core, 64, &mut out);
            for i in 0..64 {
                assert_eq!(out[i], (next + i) as f32 * 1e-5, "callback {k} frame {i}");
                assert_eq!(out[64 + i], -((next + i) as f32) * 1e-5);
            }
            next += 64;
            push_frames(&mut tx, 128 + 64 * k, 64);
        }
        assert_eq!(counters.input_underruns.load(Ordering::Relaxed), 0);
        assert_eq!(counters.callbacks.load(Ordering::Relaxed), 21);
    }

    #[test]
    fn dry_and_flooded_rings_count_xruns() {
        let (h, p) = engine(passthrough(), Arc::new(ac_core::NoResources), 64);
        let (mut core, _ctl, mut tx, counters) = core(p, 64, None);
        let mut out = vec![0.0; 2 * 64];
        push_frames(&mut tx, 0, 128);
        run(&mut core, 64, &mut out);
        run(&mut core, 64, &mut out);
        // The ring is dry now.
        run(&mut core, 64, &mut out);
        assert_eq!(counters.input_underruns.load(Ordering::Relaxed), 1);
        assert_eq!(h.status().device_xruns.load(Ordering::Relaxed), 1);
        // Input piles up far past the cushion: old frames are skipped.
        push_frames(&mut tx, 0, 64 * 10);
        run(&mut core, 64, &mut out);
        assert_eq!(counters.input_overruns.load(Ordering::Relaxed), 1);
        // What's left is the cushion.
        assert_eq!(tx.buffer().capacity() - tx.slots(), 2 * 64);
    }

    #[test]
    fn long_callbacks_are_chunked_and_midi_is_rebased() {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "t_midi_marks"));
        let (_h, processor) = engine(p, Arc::new(ac_core::NoResources), 1024);
        let (mut inputs, rx) = MidiInputs::new();
        let mut kbd = inputs.sender("test").unwrap();
        let (mut core, _ctl, _tx, counters) = core(processor, 1024, Some(rx));
        let frames = CHUNK_FRAMES + 2_000;
        let t: u64 = 10_000_000_000;
        let rate = f64::from(RATE);
        let block_ns = frames as f64 * 1e9 / rate;
        let at = |off: usize| (t as f64 - block_ns + (off as f64 + 0.5) * 1e9 / rate) as u64;
        let note = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 1,
        };
        let marks = [5, CHUNK_FRAMES - 1, CHUNK_FRAMES, frames - 1];
        for &m in &marks {
            kbd.send_at(at(m), note);
        }
        let mut out = vec![0.0; 2 * frames];
        core.run_at(t, frames, |chunk, n, off| {
            out[off..off + n].copy_from_slice(&chunk[..n]);
        });
        let hits: Vec<usize> = (0..frames).filter(|&i| out[i] != 0.0).collect();
        assert_eq!(hits, marks);
        assert_eq!(
            counters.max_callback_frames.load(Ordering::Relaxed),
            frames as u64
        );
    }

    #[test]
    fn latency_probe_pauses_the_engine_and_measures_the_loop() {
        let buffer = 256usize;
        let (h, p) = engine(passthrough(), Arc::new(ac_core::NoResources), buffer);
        let (mut core, mut ctl, mut tx, _) = core(p, buffer as u32, None);
        // A simulated loopback cable with 100 frames of converter delay,
        // from output 1 to both inputs.
        let delay = 100;
        let mut line: VecDeque<f32> = std::iter::repeat_n(0.0, delay).collect();
        push_frames(&mut tx, 0, 2 * buffer);
        let mut out = vec![0.0; 2 * buffer];
        ctl.probe.start().unwrap();
        assert_eq!(ctl.probe.start(), Err(LatencyError::Busy));
        let mut result = None;
        for _ in 0..1_000 {
            run(&mut core, buffer, &mut out);
            line.extend(&out[..buffer]);
            for s in line.drain(..buffer) {
                tx.push(s).unwrap();
                tx.push(s).unwrap();
            }
            if let Some(r) = ctl.probe.poll() {
                result = Some(r);
                break;
            }
        }
        let latency = result.expect("probe finished").unwrap();
        // Converter delay, plus one callback of output, plus the input cushion.
        assert_eq!(latency.samples, delay + 2 * buffer);
        assert_eq!(latency.sample_rate, RATE);
        // The engine runs again afterwards.
        let blocks = h.status().blocks.load(Ordering::Relaxed);
        run(&mut core, buffer, &mut out);
        assert_eq!(h.status().blocks.load(Ordering::Relaxed), blocks + 1);
        // No input, no measurement.
        let (mut none, _) = probe_pair(RATE, 0);
        assert_eq!(none.start(), Err(LatencyError::NoInput));
    }

    #[test]
    fn dropping_the_core_returns_the_processor_and_midi() {
        let (_h, p) = engine(Project::default(), Arc::new(ac_core::NoResources), 64);
        let (_inputs, rx) = MidiInputs::new();
        let (core, ctl, _tx, _) = core(p, 64, Some(rx));
        drop(core);
        let back = ctl.returned.try_recv().unwrap();
        assert!(back.ports.midi_in.is_some());
    }

    #[test]
    fn midi_out_nodes_reach_the_output_ports_on_time() {
        // MIDI In → MIDI Out, with a virtual port on each end.
        let mut p = Project::default();
        p.graph.nodes.push(NodeInstance::new(NodeId(1), "midi_in"));
        p.graph.nodes.push(NodeInstance::new(NodeId(2), "midi_out"));
        p.graph.wires.push(wire(1, "midi", 2, "midi"));
        let (_h, processor) = engine(p, Arc::new(ac_core::NoResources), 480);
        let (mut inputs, rx) = MidiInputs::new();
        let mut kbd = inputs.sender("test").unwrap();
        let (mut outputs, queue) = crate::midi_out::MidiOutputs::new();
        let got = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = got.clone();
        outputs.add_virtual("synth", move |b| {
            log.lock().unwrap().push((clock::now_ns(), b.to_vec()));
        });
        let (mut core, ctl) = CallbackCore::new(CoreParts {
            processor,
            counters: Arc::new(StreamCounters::default()),
            sample_rate: RATE,
            in_channels: 0,
            out_channels: 2,
            input_ring: None,
            ports: EventPorts {
                midi_in: Some(rx),
                midi_out: Some(queue),
                ..EventPorts::default()
            },
            promote: false,
        });
        let note = MidiMsg::NoteOn {
            channel: 2,
            note: 64,
            velocity: 90,
        };
        // Received halfway through the previous 10 ms block.
        let t = clock::now_ns() + 20_000_000;
        kbd.send_at(t - 5_000_000, note);
        core.run_at(t, 480, |_, _, _| {});
        let t0 = Instant::now();
        while got.lock().unwrap().is_empty() && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, [0x92, 64, 90]);
        // Out one block after the callback, halfway into it: input and
        // output each add a block.
        assert!(got[0].0 >= t + 15_000_000, "{} vs {}", got[0].0, t);
        drop(core);
        let back = ctl.returned.try_recv().unwrap();
        assert!(back.ports.midi_out.is_some());
    }

    #[test]
    fn control_events_reach_the_engine_and_osc_reaches_the_network() {
        use crate::control::ControlInputs;
        use crate::osc_out::OscOutput;
        use ac_core::{Control, Key};

        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let to = socket.local_addr().unwrap();
        let mut p = Project::default();
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "t_control_echo")
                .with_config(json!({ "to": to.to_string() })),
        );
        let (_h, processor) = engine(p, Arc::new(ac_core::NoResources), 1024);
        let (mut inputs, controls) = ControlInputs::new();
        let mut keys = inputs.sender("keys").unwrap();
        let (osc, osc_queue) = OscOutput::new();
        let (mut core, ctl) = CallbackCore::new(CoreParts {
            processor,
            counters: Arc::new(StreamCounters::default()),
            sample_rate: RATE,
            in_channels: 0,
            out_channels: 2,
            input_ring: None,
            ports: EventPorts {
                controls: Some(controls),
                osc_out: Some(osc_queue),
                ..EventPorts::default()
            },
            promote: false,
        });
        // Longer than a chunk, so events are rebased into the second one.
        let frames = CHUNK_FRAMES + 1_000;
        let rate = f64::from(RATE);
        let t = clock::now_ns() + 1_000_000_000;
        let block_ns = frames as f64 * 1e9 / rate;
        let at = |off: usize| (t as f64 - block_ns + (off as f64 + 0.5) * 1e9 / rate) as u64;
        let space = Control::Key(Key::from_name("Space").unwrap());
        keys.send_at(at(100), space, 1.0);
        keys.send_at(at(CHUNK_FRAMES + 500), space, 0.5);
        let mut out = vec![0.0; frames];
        core.run_at(t, frames, |chunk, n, off| {
            out[off..off + n].copy_from_slice(&chunk[..n]);
        });
        let hits: Vec<(usize, f32)> = (0..frames)
            .filter(|&i| out[i] != 0.0)
            .map(|i| (i, out[i]))
            .collect();
        assert_eq!(hits, [(100, 1.0), (CHUNK_FRAMES + 500, 0.5)]);
        // Both echoes go out over UDP, in order, after their time.
        let mut buf = [0u8; 256];
        let mut got = Vec::new();
        for _ in 0..2 {
            let (n, _) = socket.recv_from(&mut buf).unwrap();
            let when = clock::now_ns();
            let (_, p) = rosc::decoder::decode_udp(&buf[..n]).unwrap();
            let rosc::OscPacket::Message(m) = p else {
                panic!("not a message");
            };
            assert_eq!(m.addr, "/echo");
            got.push((when, m.args));
        }
        assert_eq!(got[0].1, [rosc::OscType::Float(1.0)]);
        assert_eq!(got[1].1, [rosc::OscType::Float(0.5)]);
        assert!(got[0].0 >= t, "one block after the callback");
        drop(core);
        let back = ctl.returned.try_recv().unwrap();
        assert!(back.ports.controls.is_some() && back.ports.osc_out.is_some());
        assert_eq!(osc.sent(), 2);
        assert_eq!(inputs.dropped(), 0);
    }

    #[test]
    fn the_null_stream_carries_controls_end_to_end() {
        use crate::control::ControlInputs;
        use ac_core::{Control, GamepadButton};

        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let to = socket.local_addr().unwrap();
        let mut p = Project::default();
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "t_control_echo")
                .with_config(json!({ "to": to.to_string() })),
        );
        let (_h, processor) = engine(p, Arc::new(ac_core::NoResources), 256);
        let (mut inputs, controls) = ControlInputs::new();
        let mut pad = inputs.sender("gamepads").unwrap();
        let (osc, osc_queue) = crate::osc_out::OscOutput::new();
        let host = Box::new(crate::null::NullAudio::new(RATE, 256, 0, 2));
        let stream = host
            .start_with(
                processor,
                EventPorts {
                    controls: Some(controls),
                    osc_out: Some(osc_queue),
                    ..EventPorts::default()
                },
            )
            .unwrap();
        let south = Control::GamepadButton {
            pad: 0,
            button: GamepadButton::South,
        };
        assert!(pad.send(south, 0.75));
        // The paced null stream picks it up, the node echoes it, and the
        // OSC sender delivers it.
        let mut buf = [0u8; 256];
        let (n, _) = socket.recv_from(&mut buf).unwrap();
        let (_, p) = rosc::decoder::decode_udp(&buf[..n]).unwrap();
        let rosc::OscPacket::Message(m) = p else {
            panic!("not a message");
        };
        assert_eq!(
            (m.addr.as_str(), m.args),
            ("/echo", vec![rosc::OscType::Float(0.75)])
        );
        let back = stream.stop().expect("stopped");
        assert!(back.ports.controls.is_some() && back.ports.osc_out.is_some());
        assert_eq!(osc.sent(), 1);
    }

    #[test]
    fn records_device_input_to_disk_through_the_runtime() {
        let dir = std::env::temp_dir().join(format!(
            "ac-runtime-rec-{}-{}",
            std::process::id(),
            clock::now_ns()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let recorder = Arc::new(DiskRecorder::new(&dir).unwrap());
        let resources = Arc::new(FileResources {
            assets: AssetStore::new(&dir),
            recorder: Some(recorder.clone()),
            plugins: None,
        });
        let mut project = Project::default();
        project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "audio_in").with_config(json!({"channels": 2})));
        project.graph.nodes.push(
            NodeInstance::new(NodeId(2), "record")
                .with_config(json!({"channels": 2}))
                .with_label("Guitar DI"),
        );
        project.graph.wires.push(wire(1, "out", 2, "in"));
        let (mut h, p) = engine(project, resources, 128);
        let (mut core, _ctl, mut tx, _) = core(p, 128, None);
        let mut out = vec![0.0; 2 * 128];
        push_frames(&mut tx, 0, 256);
        h.set_recording(true);
        h.play();
        for k in 0..40 {
            run(&mut core, 128, &mut out);
            push_frames(&mut tx, 256 + 128 * k, 128);
        }
        h.set_recording(false);
        run(&mut core, 128, &mut out);
        drop(core);
        drop(h);
        recorder.shutdown();
        let takes = recorder.finished_takes();
        assert_eq!(takes.len(), 1, "{takes:?}");
        assert_eq!(takes[0].relative_path, "assets/Guitar-DI-001.wav");
        assert_eq!(takes[0].frames, 40 * 128);
        let wav = crate::wav::read_wav(&takes[0].path).unwrap();
        for i in 0..wav.frames() {
            assert_eq!(wav.channels[0][i], i as f32 * 1e-5, "frame {i}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
