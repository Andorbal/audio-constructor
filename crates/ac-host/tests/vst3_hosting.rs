//! A real VST3 plugin as a node: our own Audio Constructor Instrument and
//! Effect in their VST3 form (the `ac-plugin` build in a `.vst3` bundle
//! layout), described by the scanner and rendered by the engine. The same
//! checks as the CLAP tests: pitch, parameters, state, a missing plugin,
//! and the engine's allocation guard around the plugin (D133).

mod common;

use std::sync::{Arc, OnceLock};

use ac_core::{
    Endpoint, MidiMsg, NodeId, NodeInstance, NodePath, PluginDesc, PluginFormat, PluginLoader,
    PluginProcessor, PluginRequest, Project, Resources, Wire,
};
use ac_engine::{Engine, EngineOptions, HostIo, RenderOptions, Severity, render};
use ac_host::{Catalog, PluginHost};
use serde_json::json;

use ac_plugin as _;

const RATE: f64 = 48_000.0;

fn host() -> &'static Arc<PluginHost> {
    static HOST: OnceLock<Arc<PluginHost>> = OnceLock::new();
    HOST.get_or_init(|| {
        let plugins = ac_host::describe_bundle(&common::vst3_bundle()).expect("describes");
        Arc::new(PluginHost::new(Catalog {
            plugins,
            ..Catalog::default()
        }))
    })
}

fn desc(name: &str) -> PluginDesc {
    host()
        .catalog()
        .plugins
        .into_iter()
        .find(|d| d.name == name && d.format == PluginFormat::Vst3)
        .unwrap_or_else(|| panic!("{name} described"))
}

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

fn opts(note: u8) -> RenderOptions {
    RenderOptions {
        sample_rate: Some(RATE as u32),
        block_size: Some(256),
        midi: vec![
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
        ],
        ..RenderOptions::default()
    }
}

fn rms(x: &[f32]) -> f32 {
    ac_core::testing::rms(x)
}

fn pitch(x: &[f32]) -> f64 {
    (50..2000)
        .map(|f| (f, ac_core::testing::tone_magnitude(x, f64::from(f), RATE)))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a })
        .0
        .into()
}

#[test]
fn the_scanner_describes_both_vst3_plugins() {
    let inst = desc("Audio Constructor Instrument");
    assert!(inst.instrument);
    assert!(inst.midi_in);
    assert_eq!(inst.id.len(), 32, "a class id: {}", inst.id);
    assert_eq!(inst.audio_out.first().map(|p| p.channels), Some(2));
    let sockets: Vec<&str> = inst.params.iter().map(|p| p.socket.as_str()).collect();
    assert!(sockets.contains(&"macro_4"), "{sockets:?}");
    let fx = desc("Audio Constructor Effect");
    assert!(!fx.instrument);
    assert_eq!(fx.audio_in.first().map(|p| p.channels), Some(2));
}

#[test]
fn a_hosted_vst3_synth_plays_notes_at_their_pitch() {
    let project = synth_project(&desc("Audio Constructor Instrument"));
    let registry = Arc::new(ac_nodes::registry());
    for (note, hz) in [(57u8, 220.0), (69, 440.0)] {
        let out = render(&project, registry.clone(), resources(), &opts(note));
        assert!(
            out.report
                .diagnostics
                .iter()
                .all(|d| d.severity < Severity::Warning),
            "{}",
            out.report
        );
        let sustain = &out.channels[0][4_800..43_200];
        assert!(rms(sustain) > 0.01, "silent: {}", rms(sustain));
        let f = pitch(sustain);
        assert!((f - hz).abs() <= 3.0, "note {note}: strongest at {f} Hz");
    }
}

#[test]
fn vst3_parameter_inputs_reach_the_plugin() {
    let registry = Arc::new(ac_nodes::registry());
    let level = |v: f64| {
        let mut p = synth_project(&desc("Audio Constructor Instrument"));
        p.graph.nodes[1] = p.graph.nodes[1].clone().with_param("macro_4", v);
        let out = render(&p, registry.clone(), resources(), &opts(69));
        rms(&out.channels[0][4_800..43_200])
    };
    let (quiet, loud) = (level(0.0), level(1.0));
    let db = 20.0 * (loud / quiet).log10();
    assert!((db - 36.0).abs() < 3.0, "{quiet} vs {loud}: {db:.1} dB");
}

#[test]
fn a_vst3_effect_processes_audio() {
    let fx = desc("Audio Constructor Effect");
    let mut p = Project::default();
    p.settings.length = 0.5;
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 330.0),
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": fx })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    p.graph.wires = vec![wire(1, "out", 2, "in"), wire(2, "out", 3, "in")];
    let out = render(
        &p,
        Arc::new(ac_nodes::registry()),
        resources(),
        &RenderOptions::default(),
    );
    assert!(!out.report.has_errors(), "{}", out.report);
    let l = &out.channels[0][4_800..];
    let unity = 1.0 / 2f32.sqrt();
    assert!((rms(l) - unity).abs() < 0.05, "rms {}", rms(l));
    assert!((pitch(l) - 330.0).abs() <= 2.0);
}

fn block(engine: &mut Engine, midi: &[ac_core::MidiEvent]) -> Vec<f32> {
    let mut out = vec![0.0f32; 512];
    engine.process(&mut HostIo {
        frames: 256,
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
fn vst3_state_saves_and_restores() {
    let project = synth_project(&desc("Audio Constructor Instrument"));
    let registry = Arc::new(ac_nodes::registry());
    let (mut engine, report) = Engine::new(
        project.clone(),
        registry.clone(),
        resources(),
        EngineOptions::default(),
    );
    assert!(!report.has_errors(), "{report}");
    let key = NodePath::root(NodeId(2)).to_string();
    block(&mut engine, &[]);
    let state = ac_host::save_state(&key).expect("a state");
    drop(engine);
    assert!(ac_host::running().is_empty());
    let mut saved = project;
    saved.graph.nodes[1].config["state"] = json!(state);
    let (mut engine, report) = Engine::new(saved, registry, resources(), EngineOptions::default());
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
    let mut sound = block(&mut engine, &on);
    for _ in 0..20 {
        sound.extend(block(&mut engine, &[]));
    }
    assert!(rms(&sound) > 0.01);
}

#[test]
fn a_missing_vst3_plugin_is_a_silent_placeholder() {
    let mut gone = desc("Audio Constructor Instrument");
    gone.id = "0123456789ABCDEF0123456789ABCDEF".into();
    gone.name = "Gone".into();
    let out = render(
        &synth_project(&gone),
        Arc::new(ac_nodes::registry()),
        resources(),
        &opts(69),
    );
    assert!(out.channels.iter().flatten().all(|v| *v == 0.0));
    assert!(
        out.report.to_string().contains("isn't installed"),
        "{}",
        out.report
    );
}
