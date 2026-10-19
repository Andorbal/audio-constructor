//! A real CLAP plugin as a node: our own Audio Constructor Instrument and
//! Effect (the `ac-plugin` build), described by the scanner, inserted into
//! projects, and rendered by the engine. Notes come out at their pitch,
//! parameter inputs reach the plugin, state survives a save, a missing
//! plugin plays silence and says so, and the engine's audio-thread guard
//! (this binary's allocator aborts on an allocation inside `process`)
//! holds around the plugin.

mod common;

use std::sync::{Arc, OnceLock};

use ac_core::{
    Endpoint, MidiMsg, NodeId, NodeInstance, NodePath, PluginDesc, PluginFormat, PluginLoader,
    PluginProcessor, PluginRequest, Project, Resources, Wire,
};
use ac_engine::{Engine, EngineOptions, HostIo, RenderOptions, Severity, render};
use ac_host::{Catalog, PluginHost};
use serde_json::json;

// The plugin crate's library, linked for its types, brings the allocator
// that aborts on allocations inside `assert_no_alloc` in debug builds
// (it declares it for the plugin binary); the engine wraps `process` in
// that guard, so any allocation of ours around the plugin's call aborts.
use ac_plugin as _;

const INSTRUMENT: &str = "io.github.andorbal.audio-constructor.instrument";
const EFFECT: &str = "io.github.andorbal.audio-constructor.effect";
const RATE: f64 = 48_000.0;

/// The bundle described once, and a host that knows it, kept for the
/// whole run (unloading a library while a failing test prints its
/// backtrace can crash the symbolizer).
fn host() -> &'static Arc<PluginHost> {
    static HOST: OnceLock<Arc<PluginHost>> = OnceLock::new();
    HOST.get_or_init(|| {
        let plugins = ac_host::describe_bundle(&common::clap_bundle()).expect("describes");
        Arc::new(PluginHost::new(Catalog {
            plugins,
            ..Catalog::default()
        }))
    })
}

fn desc(id: &str) -> PluginDesc {
    host()
        .catalog()
        .find(PluginFormat::Clap, id)
        .cloned()
        .expect("described")
}

/// Resources that host plugins, as the editor's and `audioc`'s do.
struct Plugins(Arc<PluginHost>);

impl Resources for Plugins {
    fn plugin(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        self.0.load(req)
    }
}

fn resources() -> Arc<dyn Resources> {
    Arc::new(Plugins(host().clone()))
}

fn wire(a: u64, o: &str, b: u64, i: &str) -> Wire {
    Wire::new(Endpoint::new(NodeId(a), o), Endpoint::new(NodeId(b), i))
}

/// MIDI In → the instrument → Audio Out.
fn synth_project(desc: &PluginDesc) -> Project {
    let mut p = Project::default();
    p.settings.length = 1.5;
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "midi_in"),
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": desc })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    p.graph.wires = vec![wire(1, "midi", 2, "midi"), wire(2, "out", 3, "in")];
    p
}

fn notes(note: u8) -> Vec<(u64, MidiMsg)> {
    vec![
        (
            0,
            MidiMsg::NoteOn {
                channel: 0,
                note,
                velocity: 110,
            },
        ),
        (
            48_000,
            MidiMsg::NoteOff {
                channel: 0,
                note,
                velocity: 0,
            },
        ),
    ]
}

fn opts(note: u8, block: usize) -> RenderOptions {
    RenderOptions {
        sample_rate: Some(RATE as u32),
        block_size: Some(block),
        midi: notes(note),
        ..RenderOptions::default()
    }
}

fn rms(x: &[f32]) -> f32 {
    ac_core::testing::rms(x)
}

/// The strongest frequency between 50 Hz and 2 kHz, to the nearest Hz.
fn pitch(x: &[f32]) -> f64 {
    (50..2000)
        .map(|f| (f, ac_core::testing::tone_magnitude(x, f64::from(f), RATE)))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a })
        .0
        .into()
}

#[test]
fn the_scanner_describes_both_plugins() {
    let inst = desc(INSTRUMENT);
    assert_eq!(inst.name, "Audio Constructor Instrument");
    assert!(inst.instrument);
    assert!(inst.midi_in);
    assert!(inst.audio_in.is_empty());
    assert_eq!(inst.audio_out.len(), 1);
    assert_eq!(inst.audio_out[0].channels, 2);
    let sockets: Vec<&str> = inst.params.iter().map(|p| p.socket.as_str()).collect();
    assert!(sockets.contains(&"macro_1"), "{sockets:?}");
    assert!(sockets.contains(&"macro_16"), "{sockets:?}");
    let fx = desc(EFFECT);
    assert!(!fx.instrument);
    assert_eq!(fx.audio_in.first().map(|p| p.channels), Some(2));
    assert_eq!(fx.audio_out.first().map(|p| p.channels), Some(2));
    // The node's sockets come from that description alone.
    let sig = ac_nodes::registry()
        .get("plugin")
        .map(|e| (e.describe)(&json!({ "plugin": inst })))
        .expect("the plugin node");
    assert!(sig.find_input("midi").is_some());
    assert!(sig.find_input("macro_4").is_some());
    assert!(sig.find_output("out").is_some());
}

#[test]
fn a_hosted_synth_plays_notes_at_their_pitch() {
    let project = synth_project(&desc(INSTRUMENT));
    let registry = Arc::new(ac_nodes::registry());
    for (note, hz) in [(57u8, 220.0), (69, 440.0)] {
        let out = render(&project, registry.clone(), resources(), &opts(note, 256));
        assert!(
            out.report
                .diagnostics
                .iter()
                .all(|d| d.severity < Severity::Warning),
            "{}",
            out.report
        );
        let l = &out.channels[0];
        let sustain = &l[4_800..43_200];
        assert!(rms(sustain) > 0.01, "silent: rms {}", rms(sustain));
        let f = pitch(sustain);
        assert!((f - hz).abs() <= 3.0, "note {note}: strongest at {f} Hz");
        // Released after a second: the tail dies away.
        let tail = &l[l.len() - 4_800..];
        assert!(rms(tail) < rms(sustain) / 10.0);
    }
}

#[test]
fn renders_are_deterministic() {
    let project = synth_project(&desc(INSTRUMENT));
    let registry = Arc::new(ac_nodes::registry());
    let a = render(&project, registry.clone(), resources(), &opts(64, 128));
    let b = render(&project, registry, resources(), &opts(64, 128));
    assert_eq!(a.channels, b.channels);
}

#[test]
fn parameter_inputs_reach_the_plugin() {
    // Macro 4 is the instrument's level, −30 to +6 dB.
    let registry = Arc::new(ac_nodes::registry());
    let level = |v: f64| {
        let mut p = synth_project(&desc(INSTRUMENT));
        p.graph.nodes[1] = p.graph.nodes[1].clone().with_param("macro_4", v);
        let out = render(&p, registry.clone(), resources(), &opts(69, 256));
        rms(&out.channels[0][4_800..43_200])
    };
    let (quiet, loud) = (level(0.0), level(1.0));
    let db = 20.0 * (loud / quiet).log10();
    assert!((db - 36.0).abs() < 3.0, "{quiet} vs {loud}: {db:.1} dB");
}

#[test]
fn an_effect_processes_audio() {
    let fx = desc(EFFECT);
    let mut p = Project::default();
    p.settings.length = 0.5;
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 330.0),
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": fx })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    p.graph.wires = vec![wire(1, "out", 2, "in"), wire(2, "out", 3, "in")];
    let registry = Arc::new(ac_nodes::registry());
    let dry_level = |macro_1: f64| {
        let mut p = p.clone();
        p.graph.nodes[1] = p.graph.nodes[1].clone().with_param("macro_1", macro_1);
        let out = render(&p, registry.clone(), resources(), &RenderOptions::default());
        assert!(!out.report.has_errors(), "{}", out.report);
        rms(&out.channels[0][4_800..])
    };
    // Macro 1 is the effect's level: ±24 dB, 0 dB in the middle.
    let unity = dry_level(0.5);
    let osc = 1.0 / 2f32.sqrt();
    assert!((unity - osc).abs() < 0.05, "unity {unity}");
    assert!(dry_level(0.0) < unity / 10.0);
    let f = pitch(&{
        let out = render(&p, registry.clone(), resources(), &RenderOptions::default());
        out.channels[0][4_800..].to_vec()
    });
    assert!((f - 330.0).abs() <= 2.0, "{f}");
}

#[test]
fn a_missing_plugin_is_a_silent_placeholder_with_a_warning() {
    let mut gone = desc(INSTRUMENT);
    gone.id = "org.example.not-installed".into();
    gone.name = "Not Installed".into();
    gone.path = "/nowhere/Not Installed.clap".into();
    let project = synth_project(&gone);
    let out = render(
        &project,
        Arc::new(ac_nodes::registry()),
        resources(),
        &opts(69, 256),
    );
    assert!(out.channels.iter().flatten().all(|v| *v == 0.0));
    let w: Vec<String> = out
        .report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| d.to_string())
        .collect();
    assert!(
        w.iter()
            .any(|m| m.contains("Not Installed") && m.contains("isn't installed")),
        "{w:?}"
    );
    // Without a plugin host at all (the web build, our own plugin build):
    // the same placeholder.
    let out = render(
        &synth_project(&desc(INSTRUMENT)),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        &opts(69, 256),
    );
    assert!(out.channels.iter().flatten().all(|v| *v == 0.0));
    assert!(out.report.to_string().contains("don't run here"));
}

fn block(engine: &mut Engine, frames: usize, midi: &[ac_core::MidiEvent]) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    engine.process(&mut HostIo {
        frames,
        input: &[],
        in_channels: 0,
        output: &mut out,
        out_channels: 2,
        midi,
        midi_out: None,
        controls: &[],
        osc_out: None,
    });
    out
}

#[test]
fn state_saves_and_restores_without_rebuilding_the_running_plugin() {
    let project = synth_project(&desc(INSTRUMENT));
    let registry = Arc::new(ac_nodes::registry());
    let options = EngineOptions {
        sample_rate: RATE,
        max_block: 256,
        ..EngineOptions::default()
    };
    let (mut engine, report) = Engine::new(
        project.clone(),
        registry.clone(),
        resources(),
        options.clone(),
    );
    assert!(!report.has_errors(), "{report}");
    let key = NodePath::root(NodeId(2)).to_string();
    assert_eq!(ac_host::running(), vec![key.clone()]);
    block(&mut engine, 256, &[]);
    let state = ac_host::save_state(&key).expect("the instrument saves its state");
    assert!(!state.is_empty());
    // Storing the state in the project recompiles, but the running
    // instance carries over: it already has that state.
    let mut saved = project.clone();
    saved.graph.nodes[1].config["state"] = json!(state);
    let report = engine
        .handle
        .edit(ac_engine::GraphEdit::Replace(Box::new(saved.clone())));
    assert!(!report.has_errors());
    block(&mut engine, 256, &[]);
    engine.handle.collect_garbage();
    assert_eq!(ac_host::running().len(), 1);
    drop(engine);
    // Every instance went with the engine, on this thread.
    assert!(ac_host::running().is_empty());
    let _ = ac_host::poll();
    // A fresh engine starts the plugin from the saved state and plays.
    let (mut engine, report) = Engine::new(saved, registry, resources(), options);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.severity < Severity::Warning),
        "{report}"
    );
    let on = [ac_core::MidiEvent::new(
        0,
        MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    )];
    block(&mut engine, 256, &on);
    let mut sound = Vec::new();
    for _ in 0..20 {
        sound.extend(block(&mut engine, 256, &[]));
    }
    assert!(rms(&sound) > 0.01);
}

#[test]
fn a_rebuilt_node_gets_a_fresh_instance_and_the_old_one_goes() {
    let project = synth_project(&desc(INSTRUMENT));
    let (mut engine, _) = Engine::new(
        project.clone(),
        Arc::new(ac_nodes::registry()),
        resources(),
        EngineOptions::default(),
    );
    block(&mut engine, 64, &[]);
    // A different description (here: one parameter fewer) rebuilds it.
    let mut changed = project;
    let mut d = desc(INSTRUMENT);
    d.params.pop();
    changed.graph.nodes[1].config = json!({ "plugin": d });
    engine
        .handle
        .edit(ac_engine::GraphEdit::Replace(Box::new(changed)));
    block(&mut engine, 64, &[]);
    engine.handle.collect_garbage();
    let _ = ac_host::poll();
    assert_eq!(ac_host::running().len(), 1);
    drop(engine);
    assert!(ac_host::running().is_empty());
}
