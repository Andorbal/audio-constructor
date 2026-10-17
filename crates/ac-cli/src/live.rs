//! `audioc run`: play a project live and headless (ARCHITECTURE §13).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ac_engine::{Engine, EngineHandle, EngineOptions, Severity, required_output_channels};
use ac_io::{
    AudioConfig, AudioHost, AudioStream, CpalAudio, DiskRecorder, FileResources, InputChoice,
    MidiInputs, MidiSelect, NullAudio, StreamInfo, Take,
};
use anyhow::{Context, Result, anyhow};
use clap::Args;

use crate::util::{load_with_assets, parse_time, project_dir};

/// How often the control loop wakes to collect garbage and check for stop.
const TICK: Duration = Duration::from_millis(20);
const STATUS_EVERY: Duration = Duration::from_secs(1);
const MIDI_RESCAN_EVERY: Duration = Duration::from_secs(2);
/// Buffer size for `--null` when none is given.
const NULL_BUFFER: u32 = 256;

/// Which devices to open, shared by `run` and `latency`.
#[derive(Debug, Args)]
pub struct DeviceArgs {
    /// Audio host (see `audioc devices`).
    #[arg(long, value_name = "NAME")]
    host: Option<String>,
    /// Output device (exact name, or part of it).
    #[arg(long, value_name = "NAME")]
    output_device: Option<String>,
    /// Input device, or `none`.
    #[arg(long, value_name = "NAME")]
    input_device: Option<String>,
    /// Sample rate (default: the device's).
    #[arg(long)]
    sample_rate: Option<u32>,
    /// Frames per callback.
    #[arg(long)]
    buffer_size: Option<u32>,
}

impl DeviceArgs {
    fn config(&self) -> AudioConfig {
        AudioConfig {
            host: self.host.clone(),
            output_device: self.output_device.clone(),
            input: match self.input_device.as_deref() {
                None => InputChoice::Default,
                Some(n) if n.eq_ignore_ascii_case("none") => InputChoice::None,
                Some(n) => InputChoice::Named(n.to_string()),
            },
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
        }
    }

    fn open(&self) -> Result<CpalAudio> {
        CpalAudio::open(&self.config()).map_err(|e| {
            anyhow!(
                "{e}\nhint: `audioc devices` lists what's available; `--null` runs without a \
                 device"
            )
        })
    }
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The project to run.
    project: PathBuf,
    #[command(flatten)]
    device: DeviceArgs,
    /// MIDI inputs to open: `all`, `none`, or part of a port name.
    #[arg(long, value_name = "NAME", default_value = "all")]
    midi: String,
    /// Stop after this long (seconds or m:ss); otherwise run until Ctrl+C.
    #[arg(long, value_name = "TIME")]
    duration: Option<String>,
    /// Don't start the transport.
    #[arg(long)]
    no_play: bool,
    /// Turn on transport recording (armed Record nodes write takes).
    #[arg(long)]
    record: bool,
    /// Run without audio devices (silent input, output discarded), in real time.
    #[arg(long)]
    null: bool,
}

/// Run until `stop` is set or the duration elapses.
pub fn run(args: RunArgs, stop: &AtomicBool) -> Result<ExitCode> {
    let registry = Arc::new(ac_nodes::registry());
    let (project, assets) = load_with_assets(&args.project, &registry)?;
    let dir = project_dir(&args.project).to_path_buf();
    let duration = args
        .duration
        .as_deref()
        .map(parse_time)
        .transpose()?
        .map(Duration::from_secs_f64);

    let host: Box<dyn AudioHost> = if args.null {
        Box::new(NullAudio::new(
            args.device
                .sample_rate
                .unwrap_or(project.settings.sample_rate),
            args.device.buffer_size.unwrap_or(NULL_BUFFER),
            2,
            required_output_channels(&project),
        ))
    } else {
        Box::new(args.device.open()?)
    };
    let info = host.info().clone();
    println!("{}", describe_stream(&info));
    for n in &info.notes {
        println!("note: {n}");
    }

    let recorder =
        Arc::new(DiskRecorder::new(&dir).context("couldn't start the recorder's disk thread")?);
    let resources = Arc::new(FileResources {
        assets,
        recorder: Some(recorder.clone()),
    });
    let (engine, report) = Engine::new(
        project,
        registry,
        resources,
        EngineOptions {
            sample_rate: f64::from(info.sample_rate),
            max_block: info.max_block(),
            socket_telemetry: false,
        },
    );
    for d in &report.diagnostics {
        if d.severity >= Severity::Warning {
            eprintln!("{d}");
        }
    }
    let (mut handle, processor) = engine.split();

    let select = MidiSelect::parse(&args.midi);
    let select_any = select != MidiSelect::None;
    let (mut midi, midi_rx) = MidiInputs::new();
    if select_any {
        let ports = midi.connect(select);
        match (ports.is_empty(), midi.last_error()) {
            (false, _) => println!("midi: {}", ports.join(", ")),
            (true, Some(e)) => println!("midi: {e}"),
            (true, None) => println!("midi: no matching inputs"),
        }
    }

    let mut stream = host
        .start(processor, Some(midi_rx))
        .map_err(|e| anyhow!("{e}\nhint: try another `--buffer-size`, or `--null`"))?;
    if stream.info().notes.len() > info.notes.len() {
        for n in &stream.info().notes[info.notes.len()..] {
            println!("note: {n}");
        }
    }
    if args.record {
        handle.set_recording(true);
    }
    if !args.no_play {
        handle.play();
    }

    let rescan_midi = select_any && midi.system_available();
    let outcome = control_loop(
        &mut handle,
        stream.as_mut(),
        &mut midi,
        &recorder,
        stop,
        duration,
        rescan_midi,
    );
    shut_down(&mut handle, stream, &recorder);
    drop(handle);
    recorder.shutdown();
    for t in recorder.finished_takes() {
        print_take(&t);
    }
    outcome?;
    Ok(if report.has_errors() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

fn control_loop(
    handle: &mut EngineHandle,
    stream: &mut dyn AudioStream,
    midi: &mut MidiInputs,
    recorder: &DiskRecorder,
    stop: &AtomicBool,
    duration: Option<Duration>,
    rescan_midi: bool,
) -> Result<()> {
    let t0 = Instant::now();
    let mut next_status = t0 + STATUS_EVERY;
    let mut next_rescan = t0 + MIDI_RESCAN_EVERY;
    let mut warned_rt = false;
    loop {
        std::thread::sleep(TICK);
        handle.collect_garbage();
        midi.collect_garbage();
        for t in recorder.finished_takes() {
            print_take(&t);
        }
        let stats = stream.stats();
        if stats.realtime_denied && !warned_rt {
            warned_rt = true;
            eprintln!(
                "warning: the system refused real-time priority for the audio thread; \
                 expect dropouts under load"
            );
        }
        if stats.disconnected {
            return Err(anyhow!(
                "{}",
                stats.last_error.unwrap_or("the audio device stopped")
            ));
        }
        let now = Instant::now();
        if now >= next_status {
            next_status += STATUS_EVERY;
            println!("{}", status_line(handle, now - t0, midi.dropped()));
        }
        if rescan_midi && now >= next_rescan {
            // Hot-plug: pick up controllers plugged in after start.
            next_rescan += MIDI_RESCAN_EVERY;
            let added = midi.rescan();
            if !added.is_empty() {
                println!("midi: connected {}", added.join(", "));
            }
        }
        if stop.load(Ordering::Relaxed) || duration.is_some_and(|d| now - t0 >= d) {
            return Ok(());
        }
    }
}

/// Stop the transport and let the engine apply it (so Record nodes end
/// their takes cleanly), then stop the stream.
fn shut_down(handle: &mut EngineHandle, stream: Box<dyn AudioStream>, recorder: &DiskRecorder) {
    handle.set_recording(false);
    handle.stop();
    let status = handle.status().clone();
    let deadline = Instant::now() + Duration::from_millis(500);
    while (status.playing.load(Ordering::Relaxed) || status.recording.load(Ordering::Relaxed))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    // One more block so a take's stop reaches the disk thread.
    let blocks = status.blocks.load(Ordering::Relaxed);
    while status.blocks.load(Ordering::Relaxed) == blocks && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    // The processor (and every Record sink in its graph) is dropped here.
    drop(stream.stop());
    handle.collect_garbage();
    for t in recorder.finished_takes() {
        print_take(&t);
    }
}

#[derive(Debug, Args)]
pub struct LatencyArgs {
    #[command(flatten)]
    device: DeviceArgs,
    /// Measure this many times.
    #[arg(long, default_value_t = 1)]
    count: u32,
}

/// `audioc latency`: measure the round trip through a loopback cable.
pub fn latency(args: LatencyArgs) -> Result<ExitCode> {
    let host = Box::new(args.device.open()?);
    let info = host.info().clone();
    println!("{}", describe_stream(&info));
    for n in &info.notes {
        println!("note: {n}");
    }
    let (engine, _) = Engine::new(
        ac_core::Project::default(),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        EngineOptions {
            sample_rate: f64::from(info.sample_rate),
            max_block: info.max_block(),
            socket_telemetry: false,
        },
    );
    let (mut handle, processor) = engine.split();
    let mut stream = host.start(processor, None).map_err(|e| anyhow!("{e}"))?;
    // Let both streams settle so the input ring is primed.
    std::thread::sleep(Duration::from_millis(300));
    println!(
        "playing a noise burst (-12 dBFS) on every output and listening on every input; \
         connect an output to an input with a cable"
    );
    let mut failed = false;
    for i in 0..args.count.max(1) {
        match stream.measure_latency(Duration::from_secs(5)) {
            Ok(l) => println!("round trip {}: {l}", i + 1),
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
        handle.collect_garbage();
    }
    let stats = stream.stats();
    println!(
        "{} callbacks, {} device xruns",
        stats.callbacks,
        handle.status().device_xruns.load(Ordering::Relaxed)
    );
    drop(stream.stop());
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

pub fn describe_stream(info: &StreamInfo) -> String {
    let ms = f64::from(info.buffer_size) * 1000.0 / f64::from(info.sample_rate.max(1));
    let devices = match (&info.output_device, &info.input_device) {
        (Some(o), Some(i)) if o == i => o.clone(),
        (Some(o), Some(i)) => format!("{o} (in: {i})"),
        (Some(o), None) => o.clone(),
        (None, _) => "no device".into(),
    };
    format!(
        "audio: {} · {} · {} Hz · {} frames ({:.1} ms) · {} in / {} out",
        info.host,
        devices,
        info.sample_rate,
        info.buffer_size,
        ms,
        info.in_channels,
        info.out_channels
    )
}

/// `m:ss.mmm` for a sample position.
pub fn format_position(samples: u64, rate: u32) -> String {
    let ms = samples.saturating_mul(1000) / u64::from(rate.max(1));
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

fn db(peak: f32) -> String {
    if peak <= 1e-6 {
        "-inf".into()
    } else {
        format!("{:.1}", 20.0 * peak.log10())
    }
}

fn status_line(handle: &EngineHandle, elapsed: Duration, midi_dropped: u64) -> String {
    let s = handle.status();
    let rate = s.sample_rate.load(Ordering::Relaxed);
    let [l, r] = s.master_peak();
    let state = match (
        s.playing.load(Ordering::Relaxed),
        s.recording.load(Ordering::Relaxed),
    ) {
        (true, true) => "recording",
        (true, false) => "playing",
        (false, true) => "armed",
        (false, false) => "stopped",
    };
    format!(
        "{:>6.1}s  {:<9}  pos {}  load {:>5.1}%  xruns {} (device {})  dropped {}  peak {}/{} dB",
        elapsed.as_secs_f64(),
        state,
        format_position(s.position.load(Ordering::Relaxed), rate),
        s.load_fraction() * 100.0,
        s.total_xruns(),
        s.device_xruns.load(Ordering::Relaxed),
        s.dropped_messages.load(Ordering::Relaxed) + midi_dropped,
        db(l),
        db(r),
    )
}

fn print_take(t: &Take) {
    match &t.error {
        None => println!(
            "recorded {} ({} ch, {:.2} s)",
            t.path.display(),
            t.channels,
            t.frames as f64 / f64::from(t.sample_rate.max(1))
        ),
        Some(e) => eprintln!("warning: take from {} failed: {e}", t.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        assert_eq!(format_position(0, 48_000), "0:00.000");
        assert_eq!(format_position(48_000 * 75 + 24_000, 48_000), "1:15.500");
        // A zero rate (not yet published) doesn't divide by zero.
        assert_eq!(format_position(5, 0), "0:05.000");
    }

    #[test]
    fn levels() {
        assert_eq!(db(0.0), "-inf");
        assert_eq!(db(1.0), "0.0");
        assert_eq!(db(0.5), "-6.0");
    }

    #[test]
    fn stream_description() {
        let info = StreamInfo {
            host: "ALSA".into(),
            output_device: Some("default".into()),
            input_device: Some("default".into()),
            sample_rate: 48_000,
            buffer_size: 240,
            in_channels: 2,
            out_channels: 2,
            notes: vec![],
        };
        assert_eq!(
            describe_stream(&info),
            "audio: ALSA · default · 48000 Hz · 240 frames (5.0 ms) · 2 in / 2 out"
        );
    }
}
