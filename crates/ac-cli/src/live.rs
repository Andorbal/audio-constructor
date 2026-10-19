//! `audioc run`: play a project live and headless (ARCHITECTURE §13).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ac_engine::{Engine, EngineHandle, EngineOptions, Severity, required_output_channels};
use ac_io::{
    AudioConfig, AudioHost, AudioStream, Baker, ControlInputs, CpalAudio, DiskRecorder, EventPorts,
    FileResources, GamepadInfo, Gamepads, InputChoice, MidiInputs, MidiOutputs, MidiSelect,
    NullAudio, OscInput, OscOutput, StreamInfo, Take, worker_pool,
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
            threads: 1,
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
    /// MIDI outputs that MIDI Out nodes send to: `all`, `none`, or part of
    /// a port name.
    #[arg(long, value_name = "NAME", default_value = "none")]
    midi_out: String,
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
    /// Threads to run the engine on (the audio thread plus real-time
    /// workers); independent parts of the patch run side by side.
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Listen for OSC on this UDP port, on every network interface (off
    /// unless given).
    #[arg(long, value_name = "PORT")]
    osc_in: Option<u16>,
    /// Don't listen to gamepads (they're on by default, plugged in any time).
    #[arg(long)]
    no_gamepad: bool,
}

/// Run until `stop` is set or the duration elapses.
pub fn run(args: RunArgs, stop: &AtomicBool) -> Result<ExitCode> {
    let registry = Arc::new(ac_nodes::registry());
    let (project, assets) = load_with_assets(&args.project, &registry)?;
    let sends_midi = uses_kind(&project, "midi_out");
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
    let plugin_host = crate::plugins::host_for(&project);
    let resources = Arc::new(FileResources {
        assets,
        recorder: Some(recorder.clone()),
        plugins: plugin_host
            .clone()
            .map(|h| h as Arc<dyn ac_core::PluginLoader>),
    });
    let (engine, report) = Engine::new(
        project,
        registry,
        resources,
        EngineOptions {
            sample_rate: f64::from(info.sample_rate),
            max_block: info.max_block(),
            socket_telemetry: false,
            offline: false,
        },
    );
    for d in &report.diagnostics {
        if d.severity >= Severity::Warning {
            eprintln!("{d}");
        }
    }
    let (mut handle, mut processor) = engine.split();
    // The engine applies bindings itself, from whatever MIDI and controls
    // reach it.
    let bindings = handle.project().bindings.len();
    if bindings > 0 {
        println!("bindings: {bindings}");
    }
    let workers = worker_pool(args.threads);
    if let Some(w) = &workers {
        println!("engine threads: {}", w.workers() + 1);
    }
    processor.set_workers(workers);

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

    let out_select = MidiSelect::parse(&args.midi_out);
    let (mut midi_out, midi_out_queue) = MidiOutputs::new();
    if out_select != MidiSelect::None {
        let ports = midi_out.connect(out_select.clone());
        match (ports.is_empty(), midi_out.last_error()) {
            (false, _) => println!("midi out: {}", ports.join(", ")),
            (true, Some(e)) => println!("midi out: {e}"),
            (true, None) => println!("midi out: no matching outputs"),
        }
    } else if sends_midi {
        println!("note: MIDI Out nodes send nowhere; choose ports with --midi-out");
    }

    // Control surfaces: gamepads (unless turned off) and OSC (if asked).
    let (mut controls, controls_rx) = ControlInputs::new();
    let gamepads = if args.no_gamepad {
        None
    } else {
        match controls.sender("gamepads") {
            Ok(s) => {
                let g = Gamepads::start(s);
                println!("{}", gamepad_line(&g.connected(), g.error().as_deref()));
                Some(g)
            }
            Err(e) => {
                println!("gamepads: {e}");
                None
            }
        }
    };
    let osc_in = match args.osc_in {
        None | Some(0) => None,
        Some(port) => {
            let sender = controls.sender("osc").map_err(|e| anyhow!("{e}"))?;
            let l = OscInput::listen(port, sender).map_err(|e| {
                anyhow!("couldn't listen for OSC on UDP port {port}: {e}\nhint: try another port")
            })?;
            println!("{}", osc_in_line(l.port(), ac_io::osc::local_ip()));
            Some(l)
        }
    };
    // OSC Out nodes send to the address in their own settings.
    let (osc_out, osc_out_queue) = OscOutput::new();

    let mut stream = host
        .start_with(
            processor,
            EventPorts {
                midi_in: Some(midi_rx),
                midi_out: Some(midi_out_queue),
                controls: Some(controls_rx),
                osc_out: Some(osc_out_queue),
            },
        )
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
    let rescan_out = out_select != MidiSelect::None && midi_out.system_available();
    let outcome = control_loop(
        &mut handle,
        stream.as_mut(),
        Devices {
            inputs: &mut midi,
            outputs: &mut midi_out,
            rescan_inputs: rescan_midi,
            rescan_outputs: rescan_out,
            controls: &mut controls,
            gamepads: gamepads.as_ref(),
            osc_in: osc_in.as_ref(),
            osc_out: &osc_out,
            plugins: plugin_host.as_deref(),
        },
        &recorder,
        stop,
        duration,
    );
    shut_down(&mut handle, stream, &recorder);
    // After the stream, so the notes ended at stop still go out.
    std::thread::sleep(Duration::from_millis(20));
    drop(midi_out);
    drop(osc_out);
    drop(osc_in);
    drop(gamepads);
    drop(handle);
    ac_host::shutdown();
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

/// Whether a node of `kind` is anywhere in the project.
fn uses_kind(project: &ac_core::Project, kind: &str) -> bool {
    std::iter::once(&project.graph)
        .chain(project.group_defs.values().map(|d| &d.graph))
        .any(|g| g.nodes.iter().any(|n| n.kind == kind))
}

/// The MIDI ports and control surfaces the control loop looks after.
struct Devices<'a> {
    inputs: &'a mut MidiInputs,
    outputs: &'a mut MidiOutputs,
    rescan_inputs: bool,
    rescan_outputs: bool,
    controls: &'a mut ControlInputs,
    gamepads: Option<&'a Gamepads>,
    osc_in: Option<&'a OscInput>,
    osc_out: &'a OscOutput,
    /// The plugin host, when the project has Plugin nodes.
    plugins: Option<&'a ac_host::PluginHost>,
}

/// `gamepads: 1 Pad A, 2 Pad B` (numbered as Gamepad nodes count them),
/// or why there are none.
fn gamepad_line(pads: &[GamepadInfo], error: Option<&str>) -> String {
    match (error, pads.is_empty()) {
        (Some(e), _) => format!("gamepads: {e}"),
        (None, true) => "gamepads: none connected (plug one in any time)".into(),
        (None, false) => {
            let list: Vec<String> = pads
                .iter()
                .map(|p| format!("{} {}", p.number(), p.name))
                .collect();
            format!("gamepads: {}", list.join(", "))
        }
    }
}

/// Where to send OSC: the port, and this computer's address when known.
fn osc_in_line(port: u16, ip: Option<std::net::IpAddr>) -> String {
    match ip {
        Some(ip) => format!("osc in: UDP port {port} (send to {ip}:{port})"),
        None => format!("osc in: UDP port {port}"),
    }
}

fn control_loop(
    handle: &mut EngineHandle,
    stream: &mut dyn AudioStream,
    devices: Devices<'_>,
    recorder: &DiskRecorder,
    stop: &AtomicBool,
    duration: Option<Duration>,
) -> Result<()> {
    let Devices {
        inputs: midi,
        outputs: midi_out,
        rescan_inputs: rescan_midi,
        rescan_outputs,
        controls,
        gamepads,
        osc_in,
        osc_out,
        plugins,
    } = devices;
    let mut pads_seen = gamepads.map(Gamepads::generation);
    let mut osc_bad_seen = 0;
    let t0 = Instant::now();
    let mut next_status = t0 + STATUS_EVERY;
    let mut next_rescan = t0 + MIDI_RESCAN_EVERY;
    let mut warned_rt = false;
    let mut baker = Baker::new();
    let mut reported: Vec<(ac_core::NodePath, ac_engine::WholeState)> = Vec::new();
    loop {
        std::thread::sleep(TICK);
        handle.collect_garbage();
        midi.collect_garbage();
        controls.collect_garbage();
        baker.poll(handle);
        if let Some(host) = plugins {
            serve_plugins(host, handle);
        }
        if let Some(g) = gamepads
            && pads_seen != Some(g.generation())
        {
            // Hot-plug: say which gamepads there are now.
            pads_seen = Some(g.generation());
            println!("{}", gamepad_line(&g.connected(), g.error().as_deref()));
        }
        if let Some(l) = osc_in {
            let bad = l.stats().malformed;
            if bad > osc_bad_seen && osc_bad_seen == 0 {
                eprintln!("warning: received UDP packets on the OSC port that aren't OSC");
            }
            osc_bad_seen = bad;
        }
        let states: Vec<_> = handle
            .bakes()
            .iter()
            .map(|b| (b.path.clone(), b.state.clone()))
            .collect();
        if states != reported {
            for (path, state) in &states {
                if !reported.contains(&(path.clone(), state.clone())) {
                    println!("{path}: {}", state.label());
                }
            }
            reported = states;
        }
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
            let dropped = midi.dropped() + controls.dropped() + osc_out.dropped();
            println!("{}", status_line(handle, now - t0, dropped));
        }
        if rescan_midi && now >= next_rescan {
            // Hot-plug: pick up controllers plugged in after start.
            let added = midi.rescan();
            if !added.is_empty() {
                println!("midi: connected {}", added.join(", "));
            }
        }
        if rescan_outputs && now >= next_rescan {
            let added = midi_out.rescan();
            if !added.is_empty() {
                println!("midi out: connected {}", added.join(", "));
            }
        }
        if (rescan_midi || rescan_outputs) && now >= next_rescan {
            next_rescan += MIDI_RESCAN_EVERY;
        }
        if stop.load(Ordering::Relaxed) || duration.is_some_and(|d| now - t0 >= d) {
            return Ok(());
        }
    }
}

/// Answer what hosted plugins asked of the main thread: a plugin that
/// needs to restart gets a fresh instance with its state, and parameters a
/// plugin moved itself (a preset) move the node's inputs.
fn serve_plugins(host: &ac_host::PluginHost, handle: &mut EngineHandle) {
    for event in ac_host::poll() {
        match event {
            ac_host::HostEvent::Restart { node_key } => {
                host.prepare_restart(&node_key);
                let report = handle.rebuild(&node_key);
                println!("{node_key}: plugin restarted");
                for d in report
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity >= Severity::Warning)
                {
                    eprintln!("{d}");
                }
            }
            ac_host::HostEvent::ParamChanged {
                node_key,
                socket,
                value,
            } => {
                if let Ok(path) = node_key.parse::<ac_core::NodePath>() {
                    handle.set_param(&path, &socket, ac_core::Value::Float(value));
                }
            }
            ac_host::HostEvent::StateChanged { .. } | ac_host::HostEvent::WindowClosed { .. } => {}
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
            offline: false,
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
    let cores = s.core_loads();
    let per_core = if cores.len() > 1 {
        let list: Vec<String> = cores.iter().map(|c| format!("{:.0}", c * 100.0)).collect();
        format!("  cores {}%", list.join("/"))
    } else {
        String::new()
    };
    format!(
        "{:>6.1}s  {:<9}  pos {}  load {:>5.1}%{per_core}  xruns {} (device {})  dropped {}  peak {}/{} dB",
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
    fn gamepad_and_osc_lines() {
        assert_eq!(
            gamepad_line(&[], None),
            "gamepads: none connected (plug one in any time)"
        );
        assert_eq!(
            gamepad_line(&[], Some("gamepads aren't supported on this system")),
            "gamepads: gamepads aren't supported on this system"
        );
        let pads = [
            GamepadInfo {
                index: 0,
                name: "Pad A".into(),
            },
            GamepadInfo {
                index: 2,
                name: "Pad B".into(),
            },
        ];
        assert_eq!(gamepad_line(&pads, None), "gamepads: 1 Pad A, 3 Pad B");
        assert_eq!(osc_in_line(9000, None), "osc in: UDP port 9000");
        assert_eq!(
            osc_in_line(9000, Some([192, 168, 1, 20].into())),
            "osc in: UDP port 9000 (send to 192.168.1.20:9000)"
        );
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
