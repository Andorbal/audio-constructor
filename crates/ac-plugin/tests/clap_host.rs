//! The plugin in a real CLAP host: this test is the host. It loads the
//! library cargo built (the same file the `.clap` bundle holds), finds
//! both plugins in its factory, and drives them the way a host does:
//! activate at 48 kHz with up to 256 frames, process on an audio thread,
//! automate a macro, save and restore state, follow the transport, and
//! play notes into the instrument. The library is a debug build, so any
//! allocation inside its process call aborts the test (lib.rs).
//!
//! Phase 3 acceptance #1: "A project runs as a CLAP effect in a host, with
//! its exposed parameters automatable."

// Loading a plugin library and calling into it is unsafe by nature: the
// library's code can do anything. It is our own build, which is what the
// test is for; there's no safe way to load a dynamic library.
#![allow(unsafe_code)]

mod common;

use std::ffi::CStr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use ac_core::{Binding, Endpoint, NodeId, NodeInstance, NodePath, Project, Source, Wire};
use ac_plugin::Kind;
use ac_plugin::assets::AssetBank;
use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui,
};
use clack_extensions::params::{ParamInfoBuffer, PluginParams};
use clack_extensions::state::PluginState;
use clack_host::events::event_types::{
    NoteOffEvent, NoteOnEvent, ParamValueEvent, TransportEvent, TransportFlags,
};
use clack_host::events::{EventFlags, EventHeader};
use clack_host::prelude::*;
use clack_host::utils::{BeatTime, SecondsTime};

const EFFECT: &CStr = c"io.github.andorbal.audio-constructor.effect";
const INSTRUMENT: &CStr = c"io.github.andorbal.audio-constructor.instrument";
const RATE: f64 = 48_000.0;
const MAX: u32 = 256;

// --- the host ---

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = HostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        // A host with windows: the plugin's editor needs this to open.
        builder.register::<HostGui>();
    }
}

impl HostGuiImpl for HostShared {
    fn resize_hints_changed(&self) {}
    fn request_resize(&self, _new_size: GuiSize) -> Result<(), HostError> {
        Ok(())
    }
    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn closed(&self, _was_destroyed: bool) {}
}

#[derive(Default)]
struct HostShared {
    state: OnceLock<Option<PluginState>>,
    params: OnceLock<Option<PluginParams>>,
    gui: OnceLock<Option<PluginGui>>,
    callback: AtomicBool,
}

impl<'a> SharedHandler<'a> for HostShared {
    fn initializing(&self, instance: InitializingPluginHandle<'a>) {
        let _ = self.state.set(instance.get_extension());
        let _ = self.params.set(instance.get_extension());
        let _ = self.gui.set(instance.get_extension());
    }
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback.store(true, Ordering::Relaxed);
    }
}

/// One block a host sends: input channels (none for the instrument),
/// events, and the transport.
#[derive(Default, Clone)]
struct BlockIn {
    frames: usize,
    input: Vec<Vec<f32>>,
    params: Vec<(u32, ClapId, f64)>,
    notes_on: Vec<(u32, u16, f64)>,
    notes_off: Vec<(u32, u16)>,
    transport: Option<TransportEvent>,
}

struct Instance {
    plugin: PluginInstance<TestHost>,
    processor: Option<StartedPluginAudioProcessor<TestHost>>,
    ins: usize,
    outs: usize,
}

fn host_info() -> HostInfo {
    HostInfo::new(
        "Test Host",
        "Audio Constructor tests",
        "https://example.com",
        "1.0",
    )
    .unwrap()
}

impl Instance {
    fn new(entry: &PluginEntry, id: &CStr, ins: usize, outs: usize) -> Instance {
        let plugin = PluginInstance::<TestHost>::new(
            |_| HostShared::default(),
            |_| (),
            entry,
            id,
            &host_info(),
        )
        .expect("the plugin instantiates");
        Instance {
            plugin,
            processor: None,
            ins,
            outs,
        }
    }

    fn state_ext(&mut self) -> PluginState {
        self.plugin
            .access_shared_handler(|h| *h.state.get().expect("initialized"))
            .expect("the plugin has the state extension")
    }

    fn params_ext(&mut self) -> PluginParams {
        self.plugin
            .access_shared_handler(|h| *h.params.get().expect("initialized"))
            .expect("the plugin has the params extension")
    }

    /// The window extension, if the plugin has one.
    fn gui_ext(&mut self) -> Option<PluginGui> {
        self.plugin
            .access_shared_handler(|h| *h.gui.get().expect("initialized"))
    }

    fn save_state(&mut self) -> Vec<u8> {
        let ext = self.state_ext();
        let mut out = Vec::new();
        ext.save(&self.plugin.plugin_handle(), &mut out)
            .expect("state saves");
        out
    }

    fn load_state(&mut self, bytes: &[u8]) -> bool {
        let ext = self.state_ext();
        let mut r = std::io::Cursor::new(bytes);
        let ok = ext.load(&self.plugin.plugin_handle(), &mut r).is_ok();
        self.main_thread();
        ok
    }

    /// The host's main-thread callback, when the plugin asked for one.
    fn main_thread(&mut self) {
        let asked = self
            .plugin
            .access_shared_handler(|h| h.callback.swap(false, Ordering::Relaxed));
        if asked {
            self.plugin.call_on_main_thread_callback();
        }
    }

    /// Parameter ids and names, in the plugin's order.
    fn params(&mut self) -> Vec<(ClapId, String, f64, f64, f64)> {
        let ext = self.params_ext();
        let handle = &mut self.plugin.plugin_handle();
        let mut buf = ParamInfoBuffer::new();
        (0..ext.count(handle))
            .filter_map(|i| {
                ext.get_info(handle, i, &mut buf).map(|info| {
                    (
                        info.id,
                        String::from_utf8_lossy(info.name).into_owned(),
                        info.min_value,
                        info.max_value,
                        info.default_value,
                    )
                })
            })
            .collect()
    }

    fn param_id(&mut self, name: &str) -> ClapId {
        self.params()
            .into_iter()
            .find(|p| p.1 == name)
            .unwrap_or_else(|| panic!("no parameter {name}"))
            .0
    }

    fn value_text(&mut self, id: ClapId, value: f64) -> String {
        let ext = self.params_ext();
        let mut buf = [0u8; 64];
        let text = ext
            .value_to_text(&self.plugin.plugin_handle(), id, value, &mut buf)
            .expect("value to text");
        String::from_utf8_lossy(text)
            .trim_end_matches('\0')
            .to_string()
    }

    fn activate(&mut self) {
        let config = PluginAudioConfiguration {
            sample_rate: RATE,
            min_frames_count: 1,
            max_frames_count: MAX,
        };
        let stopped = self
            .plugin
            .activate(|_, _| (), config)
            .expect("the plugin activates");
        self.processor = Some(stopped.start_processing().expect("processing starts"));
        self.main_thread();
    }

    fn deactivate(&mut self) {
        if let Some(p) = self.processor.take() {
            self.plugin.deactivate(p.stop_processing());
        }
        self.main_thread();
    }

    /// Run blocks on an audio thread of their own; returns each block's
    /// output channels and how many note events the plugin sent.
    fn process(&mut self, blocks: &[BlockIn]) -> (Vec<Vec<f32>>, usize) {
        let mut processor = self.processor.take().expect("active");
        let (ins, outs) = (self.ins, self.outs);
        let blocks = blocks.to_vec();
        let (processor, out, sent) = std::thread::scope(|s| {
            s.spawn(move || {
                let mut joined = vec![Vec::new(); outs];
                let mut sent = 0;
                let mut in_ports = AudioPorts::with_capacity(ins, 1);
                let mut out_ports = AudioPorts::with_capacity(outs, 1);
                let mut out_events = EventBuffer::new();
                for b in blocks {
                    let mut input = b.input.clone();
                    input.resize(ins, vec![0.0; b.frames]);
                    let mut output = vec![vec![0.0f32; b.frames]; outs];
                    let mut events = EventBuffer::new();
                    // Events in time order, as hosts send them.
                    let mut all: Vec<(u32, u8, usize)> = Vec::new();
                    all.extend(b.params.iter().enumerate().map(|(i, p)| (p.0, 0, i)));
                    all.extend(b.notes_on.iter().enumerate().map(|(i, n)| (n.0, 1, i)));
                    all.extend(b.notes_off.iter().enumerate().map(|(i, n)| (n.0, 2, i)));
                    all.sort();
                    for (t, kind, i) in all {
                        match kind {
                            0 => {
                                let (_, id, v) = b.params[i];
                                events.push(&ParamValueEvent::new(t, id, Pckn::match_all(), v));
                            }
                            1 => {
                                let (_, key, vel) = b.notes_on[i];
                                events.push(&NoteOnEvent::new(
                                    t,
                                    Pckn::new(0u16, 0u16, key, 0u32),
                                    vel,
                                ));
                            }
                            _ => {
                                let (_, key) = b.notes_off[i];
                                events.push(&NoteOffEvent::new(
                                    t,
                                    Pckn::new(0u16, 0u16, key, 0u32),
                                    0.0,
                                ));
                            }
                        }
                    }
                    let input_audio = if ins == 0 {
                        InputAudioBuffers::empty()
                    } else {
                        in_ports.with_input_buffers([AudioPortBuffer {
                            latency: 0,
                            channels: AudioPortBufferType::f32_input_only(
                                input
                                    .iter_mut()
                                    .map(|c| InputChannel::variable(c.as_mut_slice())),
                            ),
                        }])
                    };
                    let mut output_audio = out_ports.with_output_buffers([AudioPortBuffer {
                        latency: 0,
                        channels: AudioPortBufferType::f32_output_only(
                            output.iter_mut().map(|c| c.as_mut_slice()),
                        ),
                    }]);
                    out_events.clear();
                    processor
                        .process(
                            &input_audio,
                            &mut output_audio,
                            &InputEvents::from_buffer(&events),
                            &mut OutputEvents::from_buffer(&mut out_events),
                            None,
                            b.transport.as_ref(),
                        )
                        .expect("the block processes");
                    sent += out_events.len() as usize;
                    for (j, o) in joined.iter_mut().zip(output) {
                        j.extend(o);
                    }
                }
                (processor, joined, sent)
            })
            .join()
            .expect("the audio thread")
        });
        self.processor = Some(processor);
        self.main_thread();
        (out, sent)
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.deactivate();
    }
}

/// The plugin library, loaded once and kept for the whole run: unloading
/// it while another test's failure prints a backtrace crashes the
/// symbolizer, which would hide the failure behind a segfault.
fn load() -> &'static PluginEntry {
    static ENTRY: OnceLock<PluginEntry> = OnceLock::new();
    ENTRY.get_or_init(|| {
        let path = common::plugin_library();
        // SAFETY: the library is this crate's own build (see the file's
        // top); loading a dynamic library can't be done without `unsafe`.
        unsafe { PluginEntry::load(&path) }.expect("the plugin library loads")
    })
}

// --- signals ---

fn sine(frames: usize, start: usize, amp: f32) -> Vec<f32> {
    (start..start + frames)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin())
        .collect()
}

fn blocks_of_sine(count: usize, frames: usize, start: usize) -> Vec<BlockIn> {
    (0..count)
        .map(|k| {
            let s = sine(frames, start + k * frames, 0.5);
            BlockIn {
                frames,
                input: vec![s.clone(), s],
                ..BlockIn::default()
            }
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// A JSON state as the plugin's CLAP state stream holds it: its length
/// (8 bytes, little-endian), then the JSON.
fn clap_stream(json: Vec<u8>) -> Vec<u8> {
    let mut out = (json.len() as u64).to_le_bytes().to_vec();
    out.extend(json);
    out
}

/// The JSON of a state stream the plugin saved.
fn stream_json(stream: &[u8]) -> serde_json::Value {
    let len = u64::from_le_bytes(stream[..8].try_into().unwrap()) as usize;
    assert_eq!(len, stream.len() - 8, "the length prefix covers the rest");
    serde_json::from_slice(&stream[8..]).unwrap()
}

/// The plugin's state for `project`, in the framework's envelope: macro
/// values by parameter id, and our document in its persisted field.
fn state_with(project: &Project, macros: &[(usize, f32)]) -> Vec<u8> {
    let inner = ac_plugin::state::encode(Kind::Effect, project, &AssetBank::default(), None);
    let params: serde_json::Map<String, serde_json::Value> = macros
        .iter()
        .map(|(n, v)| (format!("macro_{n}"), serde_json::json!({ "f32": v })))
        .collect();
    clap_stream(
        serde_json::to_vec(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "params": params,
            "fields": { "audio-constructor": serde_json::to_string(&inner).unwrap() },
        }))
        .unwrap(),
    )
}

/// Audio In → Biquad (lowpass 2 kHz) → Gain → Audio Out, with Macro 2 on
/// the gain over −60..0 dB and Macro 3 on the cutoff.
fn filter_project() -> Project {
    let mut p = Project {
        name: "Filter and level".into(),
        ..Project::default()
    };
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "audio_in").with_config(serde_json::json!({ "channels": 2 })),
        NodeInstance::new(NodeId(2), "biquad")
            .with_param("mode", "lowpass")
            .with_param("cutoff", 2000.0),
        NodeInstance::new(NodeId(3), "gain"),
        NodeInstance::new(NodeId(4), "audio_out"),
    ];
    let w = |a, o: &str, b, i: &str| {
        Wire::new(Endpoint::new(NodeId(a), o), Endpoint::new(NodeId(b), i))
    };
    p.graph.wires = vec![
        w(1, "out", 2, "in"),
        w(2, "out", 3, "in"),
        w(3, "out", 4, "in"),
    ];
    p.bindings = vec![
        Binding {
            min: Some(-60.0),
            max: Some(0.0),
            ..Binding::input(
                Source::Macro { number: 2 },
                NodePath::root(NodeId(3)),
                "gain",
            )
        },
        Binding {
            min: Some(200.0),
            max: Some(8000.0),
            ..Binding::input(
                Source::Macro { number: 3 },
                NodePath::root(NodeId(2)),
                "cutoff",
            )
        },
    ];
    p
}

// --- tests ---

#[test]
fn the_factory_lists_both_plugins() {
    let entry = load();
    let factory = entry.get_plugin_factory().expect("a plugin factory");
    let mut found: Vec<(String, String)> = factory
        .plugin_descriptors()
        .map(|d| {
            (
                d.id().unwrap().to_string_lossy().into_owned(),
                d.name().unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            (
                EFFECT.to_string_lossy().into_owned(),
                "Audio Constructor Effect".into()
            ),
            (
                INSTRUMENT.to_string_lossy().into_owned(),
                "Audio Constructor Instrument".into()
            ),
        ]
    );
}

#[test]
fn both_plugins_offer_an_embedded_resizable_editor() {
    let entry = load();
    // Opening the window needs a display, which CI doesn't have; what the
    // host asks before it opens one doesn't.
    let api = GuiApiType::default_for_current_platform().expect("a windowing API here");
    for (id, ins) in [(EFFECT, 2), (INSTRUMENT, 0)] {
        let mut p = Instance::new(entry, id, ins, 2);
        let gui = p.gui_ext();
        if !cfg!(feature = "editor") {
            assert!(gui.is_none(), "no window without the editor feature");
            continue;
        }
        let gui = gui.expect("the plugin has a window");
        let handle = &mut p.plugin.plugin_handle();
        let embedded = GuiConfiguration {
            api_type: api,
            is_floating: false,
        };
        assert!(gui.is_api_supported(handle, embedded), "{id:?}");
        assert!(gui.can_resize(handle));
        assert_eq!(
            gui.get_size(handle),
            Some(GuiSize {
                width: 1200,
                height: 760
            }),
            "the size the window opens at"
        );
    }
}

#[test]
fn a_project_runs_as_a_clap_effect_with_automatable_macros() {
    let entry = load();
    let mut fx = Instance::new(entry, EFFECT, 2, 2);
    // Sixteen macros, 0–1, in the middle by default.
    let params = fx.params();
    assert_eq!(
        params.len(),
        16,
        "{:?}",
        params.iter().map(|p| &p.1).collect::<Vec<_>>()
    );
    for (i, p) in params.iter().enumerate() {
        assert_eq!(p.1, format!("Macro {}", i + 1));
        assert_eq!((p.2, p.3, p.4), (0.0, 1.0, 0.5));
    }
    fx.activate();
    // The default project: Audio In → Gain (Macro 1, ±24 dB) → Audio Out.
    // At the default 0.5 the sine comes back at 0 dB.
    let (out, _) = fx.process(&blocks_of_sine(20, 256, 0));
    let want = sine(20 * 256, 0, 0.5);
    let tail = 10 * 256..20 * 256;
    for ch in &out {
        let err = ch[tail.clone()]
            .iter()
            .zip(&want[tail.clone()])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 1e-5, "unity gain, error {err}");
    }
    // The host automates Macro 1 to 0.25 at frame 100 of a block: −12 dB
    // from that sample on (after the engine's 5 ms smoothing).
    let m1 = fx.param_id("Macro 1");
    assert_eq!(
        fx.value_text(m1, 0.25),
        "-12 dB",
        "the readout is the value it sets"
    );
    assert_eq!(fx.value_text(m1, 0.5), "0 dB");
    let mut blocks = blocks_of_sine(20, 256, 20 * 256);
    blocks[0].params.push((100, m1, 0.25));
    let (out, _) = fx.process(&blocks);
    let want = sine(20 * 256, 20 * 256, 0.5);
    assert!(
        (out[0][99] - want[99]).abs() < 1e-5,
        "unchanged before the event's sample"
    );
    assert!(
        (out[0][100] - want[100]).abs() < 1e-3,
        "the ramp starts at the event"
    );
    let level = db(rms(&out[0][2048..])) - db(rms(&want[2048..]));
    assert!(
        (level + 12.0).abs() < 0.05,
        "−12 dB after automation, got {level:.3}"
    );
    // Variable block sizes up to the maximum still run cleanly.
    let mut uneven = Vec::new();
    let mut at = 40 * 256;
    for frames in [1, 255, 256, 7, 64, 129] {
        let s = sine(frames, at, 0.5);
        uneven.push(BlockIn {
            frames,
            input: vec![s.clone(), s],
            ..BlockIn::default()
        });
        at += frames;
    }
    let (out, _) = fx.process(&uneven);
    let want = sine(at - 40 * 256, 40 * 256, 0.5);
    let ratio = rms(&out[1]) / rms(&want);
    assert!((db(ratio) + 12.0).abs() < 0.05, "{}", db(ratio));
}

#[test]
fn state_restores_the_project_and_macros_in_a_second_instance() {
    let entry = load();
    let mut a = Instance::new(entry, EFFECT, 2, 2);
    // A project of our own, loaded the way a host restores a session:
    // Macro 2 at 0.5 (−30 dB on the gain), Macro 3 at 0.25 on the cutoff.
    assert!(a.load_state(&state_with(&filter_project(), &[(2, 0.5), (3, 0.25)])));
    a.activate();
    let (first, _) = a.process(&blocks_of_sine(30, 256, 0));
    let saved = a.save_state();
    // The saved state is our document inside the framework's envelope.
    let outer = stream_json(&saved);
    let inner: serde_json::Value =
        serde_json::from_str(outer["fields"]["audio-constructor"].as_str().unwrap()).unwrap();
    let inner: serde_json::Value = serde_json::from_str(inner.as_str().unwrap()).unwrap();
    assert_eq!(inner["format"], "audio-constructor-plugin-state");
    assert_eq!(inner["project"]["name"], "Filter and level");
    assert_eq!(outer["params"]["macro_2"]["f32"], 0.5);
    // Restart A from scratch and load the state into a second instance B:
    // both play the same input to the same samples.
    a.deactivate();
    a.activate();
    let mut b = Instance::new(entry, EFFECT, 2, 2);
    assert!(b.load_state(&saved));
    b.activate();
    let input = blocks_of_sine(30, 256, 0);
    let (from_a, _) = a.process(&input);
    let (from_b, _) = b.process(&input);
    assert_eq!(from_a, from_b, "identical output after a restore");
    assert_eq!(from_a, first, "and the same as before the save");
    // It's the restored patch at the restored macro values: −30 dB on the
    // gain, and Macro 3 at 0.25 puts the lowpass at 200 × 40^0.25 ≈ 503 Hz,
    // which takes 2.0 dB off a 440 Hz tone (second-order Butterworth:
    // 1 / √(1 + (440/503)⁴)).
    let want = sine(30 * 256, 0, 0.5);
    let level = db(rms(&from_b[0][4096..])) - db(rms(&want[4096..]));
    assert!((level + 32.0).abs() < 0.2, "{level:.2} dB");
}

#[test]
fn a_state_that_cant_be_used_leaves_a_working_default() {
    let entry = load();
    let mut fx = Instance::new(entry, EFFECT, 2, 2);
    // Not even the framework's envelope: the host is told it failed.
    assert!(!fx.load_state(b"\x00\x01 not a state"));
    // The envelope with our document broken: loads, falls back to the
    // default project (passthrough at 0 dB), never panics.
    let doc = r#"{"format":"audio-constructor-plugin-state","version":99}"#;
    let broken = clap_stream(
        serde_json::to_vec(&serde_json::json!({
            "version": "0.0.1",
            "params": {},
            "fields": { "audio-constructor": serde_json::to_string(doc).unwrap() },
        }))
        .unwrap(),
    );
    assert!(fx.load_state(&broken));
    fx.activate();
    let (out, _) = fx.process(&blocks_of_sine(10, 256, 0));
    let want = sine(10 * 256, 0, 0.5);
    assert!((db(rms(&out[0][1024..])) - db(rms(&want[1024..]))).abs() < 0.01);
    // Garbage while active doesn't stop the audio either.
    assert!(fx.load_state(&broken));
    let (out, _) = fx.process(&blocks_of_sine(10, 256, 0));
    assert!(out[0].iter().all(|v| v.is_finite()));
}

fn transport(playing: bool, seconds: f64) -> TransportEvent {
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_BEATS_TIMELINE;
    if playing {
        flags |= TransportFlags::IS_PLAYING;
    }
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(seconds * 2.0),
        song_pos_seconds: SecondsTime::from_float(seconds),
        tempo: 120.0,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_int(0),
        loop_end_beats: BeatTime::from_int(0),
        loop_start_seconds: SecondsTime::from_int(0),
        loop_end_seconds: SecondsTime::from_int(0),
        bar_start: BeatTime::from_int(0),
        bar_number: 0,
        time_signature_numerator: 4,
        time_signature_denominator: 4,
    }
}

#[test]
fn the_patch_follows_the_hosts_transport() {
    let entry = load();
    let mut fx = Instance::new(entry, EFFECT, 2, 2);
    // An oscillator that starts when the transport plays.
    let mut p = Project::default();
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "oscillator")
            .with_param("start", "play")
            .with_param("stop", "stop"),
        NodeInstance::new(NodeId(2), "audio_out"),
    ];
    p.graph.wires = vec![Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    )];
    assert!(fx.load_state(&state_with(&p, &[])));
    fx.activate();
    let block = |playing, at: usize| BlockIn {
        frames: 256,
        transport: Some(transport(playing, at as f64 / RATE)),
        ..BlockIn::default()
    };
    let (stopped, _) = fx.process(&[block(false, 0), block(false, 0)]);
    assert!(
        stopped[0].iter().all(|v| *v == 0.0),
        "silent while the host is stopped"
    );
    let (playing, _) = fx.process(&[block(true, 0), block(true, 256), block(true, 512)]);
    assert!(rms(&playing[0]) > 0.1, "sounds when the host plays");
    let (stopped, _) = fx.process(&[block(false, 768), block(false, 768)]);
    assert!(rms(&stopped[0][256..]) < 1e-6, "stops with the host");
}

#[test]
fn the_instrument_plays_host_notes_on_their_samples() {
    let entry = load();
    let mut synth = Instance::new(entry, INSTRUMENT, 0, 2);
    synth.activate();
    let silent = BlockIn {
        frames: 256,
        ..BlockIn::default()
    };
    let (out, _) = synth.process(&[silent.clone(), silent.clone()]);
    assert!(
        out.iter().flatten().all(|v| *v == 0.0),
        "silent with no notes"
    );
    let mut on = silent.clone();
    on.notes_on.push((64, 60, 0.8));
    let mut off = silent.clone();
    off.notes_off.push((10, 60));
    let mut blocks = vec![on];
    blocks.extend(std::iter::repeat_n(silent.clone(), 20));
    blocks.push(off);
    blocks.extend(std::iter::repeat_n(silent.clone(), 200));
    let (out, sent) = synth.process(&blocks);
    assert_eq!(sent, 0, "the default patch sends no notes back");
    let l = &out[0];
    assert!(
        l[..64].iter().all(|v| *v == 0.0),
        "nothing before the note's sample"
    );
    assert!(
        l[64..80].iter().any(|v| *v != 0.0),
        "the note starts on its sample"
    );
    assert!(
        rms(&l[1000..5000]) > 1e-3,
        "and holds: {}",
        rms(&l[1000..5000])
    );
    assert_eq!(out[0], out[1], "the mono voice plays on both channels");
    // Released: the tail dies away to silence.
    assert!(
        rms(&l[l.len() - 4096..]) < 1e-4,
        "{}",
        rms(&l[l.len() - 4096..])
    );
    // Macro 1 is the cutoff: its readout says so in Hz.
    let m1 = synth.param_id("Macro 1");
    let text = synth.value_text(m1, 1.0);
    assert!(text.ends_with("kHz"), "{text}");
}

/// The editor in a real window, opened and closed three times: the host
/// makes an X11 window, the plugin opens its editor inside it and draws
/// for a while (the host answering the window thread's calls on the main
/// thread, the patch playing), then closes it. Needs a display:
/// `xvfb-run cargo test -p ac-plugin --test clap_host -- --ignored`.
#[test]
#[cfg(all(target_os = "linux", feature = "editor"))]
#[ignore = "needs an X display (xvfb-run)"]
fn the_editor_opens_in_the_hosts_window_and_closes_again() {
    use clack_extensions::gui::Window;
    use std::time::{Duration, Instant};
    use x11_dl::xlib::Xlib;

    let xlib = Xlib::open().expect("libX11");
    // SAFETY: Xlib's calls, as documented: a display opened here and a
    // plain window on it, used only from this thread and closed below.
    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
    assert!(!display.is_null(), "no X display: set DISPLAY");
    // SAFETY: as above, on the display just checked.
    let parent = unsafe {
        let screen = (xlib.XDefaultScreen)(display);
        let root = (xlib.XRootWindow)(display, screen);
        let w = (xlib.XCreateSimpleWindow)(display, root, 0, 0, 1200, 760, 0, 0, 0);
        (xlib.XMapWindow)(display, w);
        (xlib.XFlush)(display);
        w
    };
    let entry = load();
    let mut fx = Instance::new(entry, EFFECT, 2, 2);
    fx.activate();
    let gui = fx.gui_ext().expect("the plugin has a window");
    let x11 = GuiConfiguration {
        api_type: GuiApiType::X11,
        is_floating: false,
    };
    for _ in 0..3 {
        {
            let handle = &mut fx.plugin.plugin_handle();
            gui.create(handle, x11).expect("the editor opens");
            // SAFETY: the parent window outlives the editor (destroyed below).
            unsafe { gui.set_parent(handle, Window::from_x11_handle(parent)) }
                .expect("it embeds in the host's window");
            gui.show(handle).expect("it shows");
        }
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(800) {
            fx.main_thread();
            fx.process(&blocks_of_sine(1, 256, 0));
            std::thread::sleep(Duration::from_millis(10));
        }
        let handle = &mut fx.plugin.plugin_handle();
        assert!(gui.get_size(handle).is_some());
        gui.hide(handle).expect("it hides");
        gui.destroy(handle);
    }
    fx.deactivate();
    // SAFETY: the editor is gone; nothing uses the window or display now.
    unsafe {
        (xlib.XDestroyWindow)(display, parent);
        (xlib.XCloseDisplay)(display);
    }
}
