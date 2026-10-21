//! Audio devices (ARCHITECTURE §11), through cpal.
//!
//! - [`enumerate`] lists every host (ALSA, CoreAudio, WASAPI; JACK with the
//!   `jack` feature; ASIO with the `asio` feature) with its devices, rates,
//!   and buffer sizes. No devices is a normal answer, not an error.
//! - [`CpalAudio::open`] resolves an [`AudioConfig`] (defaults for anything
//!   left out) into a concrete device, rate, and buffer size, so the engine
//!   can be built at the device's rate before anything starts.
//! - [`AudioHost::start`] then builds the streams. The output stream's
//!   callback owns the engine's processor. When there is an input device, a
//!   separate input stream pushes its frames into a lock-free ring that the
//!   output callback drains: one code path whether input and output are the
//!   same interface or not (see [`crate::runtime`] for how the ring absorbs
//!   their clock differences).
//!
//! Device errors are counted, never fatal on the audio thread: xruns go to
//! `EngineStatus::device_xruns`, a vanished device sets
//! [`crate::StreamStats::disconnected`] so the host can rebuild the stream.

// Dropping a stream stops it; cpal's streams in a browser build have
// nothing to drop, which clippy points out there.
#![cfg_attr(target_arch = "wasm32", allow(clippy::drop_non_drop))]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ac_engine::{EngineStatus, Processor};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Data, ErrorKind, FromSample, Sample, SampleFormat, SizedSample, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig, SupportedStreamConfigRange,
};
use rtrb::{Producer, RingBuffer};

use crate::convert::interleave;
use crate::latency::{Latency, LatencyError};
use crate::runtime::EventPorts;
use crate::runtime::{
    AudioError, AudioHost, AudioStream, CallbackCore, CoreControl, CoreParts, InputRing,
    StartError, Stopped, StreamCounters, StreamInfo, StreamStats, error_code, input_ring_frames,
};

/// Frames per callback when nothing was asked for and the device allows it:
/// about 5 ms at 48 kHz.
pub const DEFAULT_BUFFER: u32 = 256;
/// How long a backend may take to open a stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `stop` waits for the backend to release the callback.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// One audio host and what it offers.
#[derive(Clone, Debug, PartialEq)]
pub struct HostInfo {
    pub name: String,
    /// The platform's default host.
    pub default: bool,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    /// Why the host or its device list couldn't be read.
    pub error: Option<String>,
}

/// One device in one direction.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub default: bool,
    /// From the default configuration.
    pub channels: u16,
    pub default_sample_rate: Option<u32>,
    /// Lowest and highest supported rate over every configuration.
    pub sample_rates: Option<(u32, u32)>,
    /// Smallest and largest callback size, when the backend says.
    pub buffer_sizes: Option<(u32, u32)>,
    pub sample_format: Option<String>,
}

/// Names of the hosts compiled in and available on this machine.
pub fn host_names() -> Vec<String> {
    cpal::available_hosts()
        .into_iter()
        .map(|h| h.name().to_string())
        .collect()
}

/// The host used when none is named.
pub fn default_host_name() -> String {
    cpal::default_host().id().name().to_string()
}

/// Every host with its devices. Never fails: problems are reported in
/// [`HostInfo::error`].
pub fn enumerate() -> Vec<HostInfo> {
    let default = default_host_name();
    cpal::available_hosts()
        .into_iter()
        .map(|id| {
            let name = id.name().to_string();
            let mut info = HostInfo {
                default: name == default,
                name,
                outputs: Vec::new(),
                inputs: Vec::new(),
                error: None,
            };
            match cpal::host_from_id(id) {
                Ok(host) => {
                    match host.output_devices() {
                        Ok(devs) => {
                            let def = host.default_output_device();
                            info.outputs = devs
                                .map(|d| describe(&d, def.as_ref() == Some(&d), Direction::Output))
                                .collect();
                        }
                        Err(e) => info.error = Some(e.to_string()),
                    }
                    match host.input_devices() {
                        Ok(devs) => {
                            let def = host.default_input_device();
                            info.inputs = devs
                                .map(|d| describe(&d, def.as_ref() == Some(&d), Direction::Input))
                                .collect();
                        }
                        Err(e) => info.error = Some(e.to_string()),
                    }
                }
                Err(e) => info.error = Some(e.to_string()),
            }
            info
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Input,
    Output,
}

fn describe(d: &cpal::Device, default: bool, dir: Direction) -> DeviceInfo {
    let (def, ranges) = match dir {
        Direction::Output => (
            d.default_output_config().ok(),
            d.supported_output_configs()
                .map(|r| r.collect::<Vec<_>>())
                .unwrap_or_default(),
        ),
        Direction::Input => (
            d.default_input_config().ok(),
            d.supported_input_configs()
                .map(|r| r.collect::<Vec<_>>())
                .unwrap_or_default(),
        ),
    };
    let rates = ranges.iter().fold(None, |acc: Option<(u32, u32)>, r| {
        Some(match acc {
            None => (r.min_sample_rate(), r.max_sample_rate()),
            Some((lo, hi)) => (lo.min(r.min_sample_rate()), hi.max(r.max_sample_rate())),
        })
    });
    DeviceInfo {
        name: d.to_string(),
        default,
        channels: def.as_ref().map_or(0, SupportedStreamConfig::channels),
        default_sample_rate: def.as_ref().map(SupportedStreamConfig::sample_rate),
        sample_rates: rates,
        buffer_sizes: def.as_ref().and_then(|c| match c.buffer_size() {
            SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            SupportedBufferSize::Unknown => None,
        }),
        sample_format: def.as_ref().map(|c| c.sample_format().to_string()),
    }
}

/// Which input device to use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InputChoice {
    /// The host's default input, if there is one.
    #[default]
    Default,
    /// No input: Audio In nodes hear silence.
    None,
    /// The first input whose name matches (exactly, else containing this).
    Named(String),
}

/// What to open. Anything left out uses the host's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioConfig {
    /// Host name (`ALSA`, `JACK`, `CoreAudio`, `WASAPI`, `ASIO`); the
    /// platform default when `None`.
    pub host: Option<String>,
    /// Output device name (exact, else containing this); the default when `None`.
    pub output_device: Option<String>,
    pub input: InputChoice,
    /// The output device's default rate when `None`.
    pub sample_rate: Option<u32>,
    /// [`DEFAULT_BUFFER`] when `None` (or the device default if that isn't supported).
    pub buffer_size: Option<u32>,
    /// Threads running the engine: the audio thread plus `threads - 1`
    /// real-time workers ([`crate::worker_pool`]). 0 and 1 both mean the
    /// audio thread alone.
    pub threads: usize,
}

/// Index of the name that matches `wanted`: exact (ignoring case) first,
/// then the first containing it.
pub fn match_name(names: &[String], wanted: &str) -> Option<usize> {
    let w = wanted.to_lowercase();
    names
        .iter()
        .position(|n| n.to_lowercase() == w)
        .or_else(|| names.iter().position(|n| n.to_lowercase().contains(&w)))
}

fn supported_format(f: SampleFormat) -> bool {
    matches!(
        f,
        SampleFormat::F32
            | SampleFormat::F64
            | SampleFormat::I16
            | SampleFormat::I32
            | SampleFormat::U16
            | SampleFormat::I8
            | SampleFormat::U8
    )
}

/// The configuration to open at `rate`: the default one if it runs at that
/// rate, else the supported range closest to it (same channels and format
/// first). Only sample formats this crate converts are considered.
pub fn pick_config(
    default: &SupportedStreamConfig,
    ranges: &[SupportedStreamConfigRange],
    rate: u32,
) -> Option<SupportedStreamConfig> {
    if default.sample_rate() == rate && supported_format(default.sample_format()) {
        return Some(*default);
    }
    let usable = || {
        ranges
            .iter()
            .filter(|r| r.contains_rate(rate) && supported_format(r.sample_format()))
    };
    usable()
        .find(|r| {
            r.channels() == default.channels() && r.sample_format() == default.sample_format()
        })
        .or_else(|| usable().find(|r| r.channels() == default.channels()))
        .or_else(|| usable().max_by(|a, b| a.cmp_default_heuristics(b)))
        .and_then(|r| r.try_with_sample_rate(rate))
}

/// The buffer size to ask for, plus a note when a request couldn't be met.
pub fn choose_buffer(
    requested: Option<u32>,
    supported: &SupportedBufferSize,
) -> (BufferSize, Option<String>) {
    let want = requested.unwrap_or(DEFAULT_BUFFER).max(1);
    match *supported {
        SupportedBufferSize::Range { min, max } if (min..=max).contains(&want) => {
            (BufferSize::Fixed(want), None)
        }
        SupportedBufferSize::Range { min, max } => match requested {
            Some(_) if min <= max => {
                let got = want.clamp(min, max);
                (
                    BufferSize::Fixed(got),
                    Some(format!(
                        "buffer size {want} isn't supported ({min}–{max}); using {got}"
                    )),
                )
            }
            _ => (BufferSize::Default, None),
        },
        // The backend can't say; ask, and fall back to its default if refused.
        SupportedBufferSize::Unknown => (BufferSize::Fixed(want), None),
    }
}

/// A resolved device setup, ready to start.
pub struct CpalAudio {
    info: StreamInfo,
    output: cpal::Device,
    out_config: StreamConfig,
    out_format: SampleFormat,
    /// Whether the buffer size is a guess the backend may refuse.
    buffer_unknown: bool,
    input: Option<(cpal::Device, StreamConfig, SampleFormat)>,
}

impl std::fmt::Debug for CpalAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalAudio")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

fn find_host(name: Option<&str>) -> Result<cpal::Host, AudioError> {
    let Some(name) = name else {
        return Ok(cpal::default_host());
    };
    let ids = cpal::available_hosts();
    let names: Vec<String> = ids.iter().map(|h| h.name().to_string()).collect();
    let i = match_name(&names, name).ok_or_else(|| AudioError::UnknownHost {
        name: name.to_string(),
        available: names.clone(),
    })?;
    cpal::host_from_id(ids[i]).map_err(|e| AudioError::Backend(format!("{}: {e}", names[i])))
}

fn find_device(
    devices: Result<impl Iterator<Item = cpal::Device>, cpal::Error>,
    kind: &'static str,
    name: &str,
) -> Result<cpal::Device, AudioError> {
    let devices: Vec<cpal::Device> = devices
        .map_err(|e| AudioError::Backend(e.to_string()))?
        .collect();
    let names: Vec<String> = devices.iter().map(ToString::to_string).collect();
    let i = match_name(&names, name).ok_or_else(|| AudioError::DeviceNotFound {
        kind,
        name: name.to_string(),
        available: names,
    })?;
    devices
        .into_iter()
        .nth(i)
        .ok_or_else(|| AudioError::Backend("device list changed".into()))
}

impl CpalAudio {
    /// Resolve devices, rate, and buffer size. Fails with a clear message
    /// when there's no output device; a missing or incompatible input only
    /// adds a note.
    pub fn open(config: &AudioConfig) -> Result<CpalAudio, AudioError> {
        let host = find_host(config.host.as_deref())?;
        let host_name = host.id().name().to_string();
        let output = match &config.output_device {
            Some(n) => find_device(host.output_devices(), "output", n)?,
            None => host
                .default_output_device()
                .ok_or_else(|| AudioError::NoOutputDevice {
                    host: host_name.clone(),
                })?,
        };
        let out_name = output.to_string();
        let default =
            output
                .default_output_config()
                .map_err(|e| AudioError::UnsupportedConfig {
                    device: out_name.clone(),
                    reason: e.to_string(),
                })?;
        let rate = config.sample_rate.unwrap_or(default.sample_rate());
        let ranges: Vec<_> = output
            .supported_output_configs()
            .map(|r| r.collect())
            .unwrap_or_default();
        let chosen =
            pick_config(&default, &ranges, rate).ok_or_else(|| AudioError::UnsupportedConfig {
                device: out_name.clone(),
                reason: format!("can't run at {rate} Hz in a supported sample format"),
            })?;
        let mut notes = Vec::new();
        let (buffer, note) = choose_buffer(config.buffer_size, chosen.buffer_size());
        notes.extend(note);
        let buffer_unknown = matches!(chosen.buffer_size(), SupportedBufferSize::Unknown);
        let out_config = StreamConfig {
            channels: chosen.channels(),
            sample_rate: rate,
            buffer_size: buffer,
        };

        let input_device = match &config.input {
            InputChoice::None => None,
            InputChoice::Default => {
                let d = host.default_input_device();
                if d.is_none() {
                    notes.push("no input device; Audio In nodes hear silence".into());
                }
                d
            }
            InputChoice::Named(n) => Some(find_device(host.input_devices(), "input", n)?),
        };
        let input = input_device.and_then(|d| {
            let name = d.to_string();
            let picked = d.default_input_config().ok().and_then(|def| {
                let ranges: Vec<_> = d
                    .supported_input_configs()
                    .map(|r| r.collect())
                    .unwrap_or_default();
                pick_config(&def, &ranges, rate)
            });
            // The input may offer different callback sizes; match the
            // output's where it can.
            let in_buffer = |c: &SupportedStreamConfig| match buffer {
                BufferSize::Fixed(n) => choose_buffer(Some(n), c.buffer_size()).0,
                BufferSize::Default => BufferSize::Default,
            };
            match picked {
                Some(c) => Some((
                    d,
                    StreamConfig {
                        channels: c.channels(),
                        sample_rate: rate,
                        buffer_size: in_buffer(&c),
                    },
                    c.sample_format(),
                )),
                None => {
                    notes.push(format!(
                        "input {name} can't run at {rate} Hz; input disabled"
                    ));
                    None
                }
            }
        });

        let buffer_size = match buffer {
            BufferSize::Fixed(n) => n,
            BufferSize::Default => match chosen.buffer_size() {
                SupportedBufferSize::Range { max, .. } => (*max).min(1024),
                SupportedBufferSize::Unknown => 1024,
            },
        };
        Ok(CpalAudio {
            info: StreamInfo {
                host: host_name,
                output_device: Some(out_name),
                input_device: input.as_ref().map(|(d, _, _)| d.to_string()),
                sample_rate: rate,
                buffer_size,
                in_channels: input
                    .as_ref()
                    .map_or(0, |(_, c, _)| usize::from(c.channels)),
                out_channels: usize::from(chosen.channels()),
                notes,
            },
            output,
            out_config,
            out_format: chosen.sample_format(),
            buffer_unknown,
            input,
        })
    }
}

/// Forward device errors into counters. Runs on a backend thread, possibly
/// the audio thread: atomics only.
fn on_error(
    counters: Arc<StreamCounters>,
    status: Arc<EngineStatus>,
) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e| {
        let code = match e.kind() {
            ErrorKind::Xrun => {
                status.device_xruns.fetch_add(1, Ordering::Relaxed);
                return;
            }
            ErrorKind::RealtimeDenied => {
                counters.realtime_denied.store(true, Ordering::Relaxed);
                return;
            }
            // Rerouted to the new default device; playback continues.
            ErrorKind::DeviceChanged => return,
            ErrorKind::DeviceNotAvailable => {
                counters.disconnected.store(true, Ordering::Relaxed);
                error_code::DEVICE_GONE
            }
            ErrorKind::StreamInvalidated => {
                counters.disconnected.store(true, Ordering::Relaxed);
                error_code::STREAM_INVALID
            }
            ErrorKind::BackendError => error_code::BACKEND,
            _ => error_code::OTHER,
        };
        counters.device_errors.fetch_add(1, Ordering::Relaxed);
        counters.last_error.store(code, Ordering::Relaxed);
    }
}

/// Run the core for one output callback in device format `T`.
fn render_into<T>(core: &mut CallbackCore, data: &mut Data, channels: usize)
where
    T: SizedSample + FromSample<f32>,
{
    let Some(buf) = data.as_slice_mut::<T>() else {
        return;
    };
    let dc = channels.max(1);
    let frames = buf.len() / dc;
    core.run(frames, |chunk, n, off| {
        if let Some(dst) = buf.get_mut(off * dc..(off + n) * dc) {
            interleave(chunk, dc, n, dst, dc);
        }
    });
}

/// Push one input callback in device format `T` into the duplex ring.
fn capture_from<T>(
    tx: &mut Producer<f32>,
    data: &Data,
    channels: usize,
    counters: &StreamCounters,
    status: &EngineStatus,
) where
    T: SizedSample,
    f32: FromSample<T>,
{
    assert_no_alloc::assert_no_alloc(|| {
        let Some(buf) = data.as_slice::<T>() else {
            return;
        };
        let ch = channels.max(1);
        let frames = buf.len() / ch;
        let n = frames.min(tx.slots() / ch);
        if n < frames {
            counters.input_overruns.fetch_add(1, Ordering::Relaxed);
            status.device_xruns.fetch_add(1, Ordering::Relaxed);
        }
        if let (Ok(chunk), Some(src)) = (tx.write_chunk_uninit(n * ch), buf.get(..n * ch)) {
            chunk.fill_from_iter(src.iter().map(|s| f32::from_sample(*s)));
        }
    });
}

/// [`render_into`] for the stream's format (one of [`supported_format`]).
fn render_any(format: SampleFormat, core: &mut CallbackCore, data: &mut Data, channels: usize) {
    match format {
        SampleFormat::F32 => render_into::<f32>(core, data, channels),
        SampleFormat::F64 => render_into::<f64>(core, data, channels),
        SampleFormat::I16 => render_into::<i16>(core, data, channels),
        SampleFormat::I32 => render_into::<i32>(core, data, channels),
        SampleFormat::U16 => render_into::<u16>(core, data, channels),
        SampleFormat::I8 => render_into::<i8>(core, data, channels),
        SampleFormat::U8 => render_into::<u8>(core, data, channels),
        _ => {}
    }
}

/// [`capture_from`] for the stream's format (one of [`supported_format`]).
fn capture_any(
    format: SampleFormat,
    tx: &mut Producer<f32>,
    data: &Data,
    channels: usize,
    counters: &StreamCounters,
    status: &EngineStatus,
) {
    match format {
        SampleFormat::F32 => capture_from::<f32>(tx, data, channels, counters, status),
        SampleFormat::F64 => capture_from::<f64>(tx, data, channels, counters, status),
        SampleFormat::I16 => capture_from::<i16>(tx, data, channels, counters, status),
        SampleFormat::I32 => capture_from::<i32>(tx, data, channels, counters, status),
        SampleFormat::U16 => capture_from::<u16>(tx, data, channels, counters, status),
        SampleFormat::I8 => capture_from::<i8>(tx, data, channels, counters, status),
        SampleFormat::U8 => capture_from::<u8>(tx, data, channels, counters, status),
        _ => {}
    }
}

fn build_input(
    device: &cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    counters: &Arc<StreamCounters>,
    status: &Arc<EngineStatus>,
) -> Result<(cpal::Stream, InputRing), cpal::Error> {
    let ch = usize::from(config.channels.max(1));
    let nominal = match config.buffer_size {
        BufferSize::Fixed(n) => n,
        BufferSize::Default => 1024,
    };
    let (mut tx, rx) = RingBuffer::new(input_ring_frames(nominal) * ch);
    let (c, s) = (counters.clone(), status.clone());
    let stream = device.build_input_stream_raw(
        config,
        format,
        move |data: &Data, _: &cpal::InputCallbackInfo| {
            capture_any(format, &mut tx, data, ch, &c, &s);
        },
        on_error(counters.clone(), status.clone()),
        Some(OPEN_TIMEOUT),
    )?;
    let nominal = stream.buffer_size().unwrap_or(nominal);
    Ok((stream, InputRing::new(rx, ch, nominal)))
}

impl AudioHost for CpalAudio {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn start_with(
        self: Box<Self>,
        processor: Processor,
        ports: EventPorts,
    ) -> Result<Box<dyn AudioStream>, StartError> {
        let me = *self;
        let mut info = me.info;
        let counters = Arc::new(StreamCounters::default());
        let status = processor.status().clone();
        let mut attempts = vec![me.out_config.buffer_size];
        if me.buffer_unknown && me.out_config.buffer_size != BufferSize::Default {
            attempts.push(BufferSize::Default);
        }
        let mut back = Stopped { processor, ports };
        let mut last_err = None;
        for buffer in attempts {
            let mut input_stream = None;
            let mut input_ring = None;
            if let Some((dev, cfg, fmt)) = &me.input {
                let mut cfg = *cfg;
                if buffer == BufferSize::Default {
                    cfg.buffer_size = BufferSize::Default;
                }
                // An input that refuses the callback size gets its default.
                let built = build_input(dev, cfg, *fmt, &counters, &status).or_else(|e| {
                    if cfg.buffer_size == BufferSize::Default {
                        return Err(e);
                    }
                    let cfg = StreamConfig {
                        buffer_size: BufferSize::Default,
                        ..cfg
                    };
                    build_input(dev, cfg, *fmt, &counters, &status)
                });
                match built {
                    Ok((s, r)) => {
                        input_stream = Some(s);
                        input_ring = Some(r);
                    }
                    Err(e) => info.notes.push(format!("input disabled: {e}")),
                }
            }
            let in_channels = if input_ring.is_some() {
                info.in_channels
            } else {
                0
            };
            let (mut core, ctl) = CallbackCore::new(CoreParts {
                processor: back.processor,
                counters: counters.clone(),
                sample_rate: info.sample_rate,
                in_channels,
                out_channels: info.out_channels,
                input_ring,
                ports: back.ports,
                promote: crate::priority::host_needs_promotion(&info.host),
            });
            let channels = info.out_channels;
            let format = me.out_format;
            let config = StreamConfig {
                buffer_size: buffer,
                ..me.out_config
            };
            let built = me.output.build_output_stream_raw(
                config,
                format,
                move |data: &mut Data, _: &cpal::OutputCallbackInfo| {
                    render_any(format, &mut core, data, channels);
                },
                on_error(counters.clone(), status.clone()),
                Some(OPEN_TIMEOUT),
            );
            match built {
                Ok(output) => {
                    let played = input_stream
                        .as_ref()
                        .map_or(Ok(()), StreamTrait::play)
                        .and_then(|()| output.play());
                    if let Err(e) = played {
                        drop(output);
                        drop(input_stream);
                        return Err(StartError {
                            error: AudioError::Backend(e.to_string()),
                            returned: ctl.returned.recv_timeout(STOP_TIMEOUT).ok().map(Box::new),
                        });
                    }
                    if let Ok(n) = output.buffer_size() {
                        info.buffer_size = n;
                    }
                    info.in_channels = in_channels;
                    if in_channels == 0 {
                        info.input_device = None;
                    }
                    return Ok(Box::new(RunningAudio {
                        info,
                        counters,
                        output: Some(output),
                        input: input_stream,
                        ctl,
                    }));
                }
                Err(e) => {
                    last_err = Some(e);
                    drop(input_stream);
                    // The failed callback was dropped and sent the processor back.
                    match ctl.returned.recv_timeout(STOP_TIMEOUT) {
                        Ok(b) => back = b,
                        Err(_) => {
                            return Err(StartError {
                                error: AudioError::Backend(
                                    "the audio backend kept the engine".into(),
                                ),
                                returned: None,
                            });
                        }
                    }
                }
            }
        }
        Err(StartError {
            error: AudioError::UnsupportedConfig {
                device: info.output_device.unwrap_or_default(),
                reason: last_err.map_or_else(|| "couldn't open".into(), |e| e.to_string()),
            },
            returned: Some(Box::new(back)),
        })
    }
}

/// A running device stream.
pub struct RunningAudio {
    info: StreamInfo,
    counters: Arc<StreamCounters>,
    output: Option<cpal::Stream>,
    input: Option<cpal::Stream>,
    ctl: CoreControl,
}

impl std::fmt::Debug for RunningAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningAudio")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl AudioStream for RunningAudio {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn stats(&self) -> StreamStats {
        self.counters.snapshot()
    }

    fn start_latency(&mut self) -> Result<(), LatencyError> {
        self.ctl.probe.start()
    }

    fn poll_latency(&mut self) -> Option<Result<Latency, LatencyError>> {
        self.ctl.probe.poll()
    }

    fn stop(mut self: Box<Self>) -> Option<Stopped> {
        // Output first: it owns the processor; dropping it ends the callback.
        drop(self.output.take());
        drop(self.input.take());
        self.ctl.returned.recv_timeout(STOP_TIMEOUT).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(ch: u16, lo: u32, hi: u32, fmt: SampleFormat) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            ch,
            lo,
            hi,
            SupportedBufferSize::Range { min: 64, max: 4096 },
            fmt,
        )
    }

    #[test]
    fn enumeration_never_panics() {
        // CI has no audio hardware; an empty list (or errors in it) is fine.
        let hosts = enumerate();
        assert_eq!(hosts.len(), host_names().len());
        assert!(hosts.iter().filter(|h| h.default).count() <= 1);
        let _ = default_host_name();
    }

    #[test]
    fn opening_without_devices_is_an_error_not_a_panic() {
        match CpalAudio::open(&AudioConfig::default()) {
            Ok(a) => assert!(a.info().sample_rate > 0),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
        let bad = AudioConfig {
            host: Some("no-such-host".into()),
            ..AudioConfig::default()
        };
        let err = CpalAudio::open(&bad).unwrap_err();
        assert!(err.to_string().contains("no-such-host"), "{err}");
    }

    #[test]
    fn names_match_exactly_then_by_substring() {
        let names = vec![
            "Speakers (USB)".to_string(),
            "USB".to_string(),
            "HDMI 2".to_string(),
        ];
        assert_eq!(match_name(&names, "usb"), Some(1));
        assert_eq!(match_name(&names, "speak"), Some(0));
        assert_eq!(match_name(&names, "hdmi"), Some(2));
        assert_eq!(match_name(&names, "line"), None);
    }

    #[test]
    fn default_config_wins_at_its_own_rate() {
        let def =
            SupportedStreamConfig::new(2, 48_000, SupportedBufferSize::Unknown, SampleFormat::F32);
        let got = pick_config(&def, &[], 48_000).unwrap();
        assert_eq!(got, def);
    }

    #[test]
    fn other_rates_prefer_the_default_shape() {
        let def =
            SupportedStreamConfig::new(2, 48_000, SupportedBufferSize::Unknown, SampleFormat::I16);
        let ranges = [
            range(8, 8_000, 192_000, SampleFormat::F32),
            range(2, 8_000, 192_000, SampleFormat::F32),
            range(2, 8_000, 192_000, SampleFormat::I16),
        ];
        let got = pick_config(&def, &ranges, 96_000).unwrap();
        assert_eq!(got.sample_rate(), 96_000);
        assert_eq!(got.channels(), 2);
        assert_eq!(got.sample_format(), SampleFormat::I16);
        assert!(pick_config(&def, &ranges, 384_000).is_none());
    }

    #[test]
    fn unsupported_formats_are_skipped() {
        let def =
            SupportedStreamConfig::new(2, 44_100, SupportedBufferSize::Unknown, SampleFormat::I24);
        let ranges = [
            range(2, 44_100, 44_100, SampleFormat::I24),
            range(2, 44_100, 44_100, SampleFormat::I32),
        ];
        let got = pick_config(&def, &ranges, 44_100).unwrap();
        assert_eq!(got.sample_format(), SampleFormat::I32);
    }

    #[test]
    fn buffer_sizes() {
        let r = SupportedBufferSize::Range { min: 64, max: 2048 };
        assert_eq!(
            choose_buffer(None, &r),
            (BufferSize::Fixed(DEFAULT_BUFFER), None)
        );
        assert_eq!(choose_buffer(Some(128), &r), (BufferSize::Fixed(128), None));
        let (b, note) = choose_buffer(Some(16), &r);
        assert_eq!(b, BufferSize::Fixed(64));
        assert!(note.unwrap().contains("64"));
        // The default size isn't offered: let the device choose.
        let big = SupportedBufferSize::Range {
            min: 1024,
            max: 4096,
        };
        assert_eq!(choose_buffer(None, &big), (BufferSize::Default, None));
        assert_eq!(
            choose_buffer(Some(512), &SupportedBufferSize::Unknown),
            (BufferSize::Fixed(512), None)
        );
    }
}
