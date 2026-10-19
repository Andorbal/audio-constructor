//! A third-party plugin's own window in the host's window (CI's
//! third-party-plugin job runs it on a virtual display, with the same
//! pinned download as `ac-cli`'s acceptance test, DECISIONS D136):
//! `AC_THIRD_PARTY_PLUGINS` names the folder, `AC_THIRD_PARTY_NAME` the
//! plugin. It plays a note while the window is open, so the plugin's
//! timers and file descriptors are serviced through `ac_host::poll` the
//! way the editor services them.

use std::sync::Arc;
use std::time::Duration;

use ac_core::{
    Endpoint, MidiEvent, MidiMsg, NodeId, NodeInstance, NodePath, PluginFormat, PluginLoader,
    PluginProcessor, PluginRequest, Project, Resources, Wire,
};
use ac_engine::{Engine, EngineOptions, HostIo};
use ac_host::{Catalog, PluginHost};
use serde_json::json;

struct Plugins(Arc<PluginHost>);

impl Resources for Plugins {
    fn plugin(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        self.0.load(req)
    }
}

fn block(engine: &mut Engine, midi: &[MidiEvent]) -> f32 {
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
    out.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
#[ignore = "needs a downloaded third-party plugin and an X display"]
fn a_third_party_plugin_window_opens_while_it_plays() {
    let (Ok(dir), Ok(name)) = (
        std::env::var("AC_THIRD_PARTY_PLUGINS"),
        std::env::var("AC_THIRD_PARTY_NAME"),
    ) else {
        eprintln!("skipped: set AC_THIRD_PARTY_PLUGINS and AC_THIRD_PARTY_NAME");
        return;
    };
    let formats = std::env::var("AC_THIRD_PARTY_FORMATS").unwrap_or_else(|_| "clap".into());
    let mut plugins = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        if ac_host::scan::format_of(&e.path()).is_some()
            && let Ok(list) = ac_host::describe_bundle(&e.path())
        {
            plugins.extend(list);
        }
    }
    let host = Arc::new(PluginHost::new(Catalog {
        plugins: plugins.clone(),
        ..Catalog::default()
    }));
    for format in formats.split(',').map(str::trim) {
        let format = match format {
            "vst3" => PluginFormat::Vst3,
            _ => PluginFormat::Clap,
        };
        let desc = plugins
            .iter()
            .find(|d| d.name == name && d.format == format)
            .cloned()
            .unwrap_or_else(|| panic!("no {} plugin named {name} in {dir}", format.name()));
        let label = format!("{name} ({})", format.name());
        play_with_its_window(&host, &desc, &label);
    }
}

fn play_with_its_window(host: &Arc<PluginHost>, desc: &ac_core::PluginDesc, label: &str) {
    let mut p = Project::default();
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "midi_in"),
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": desc })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    let w = |a, o: &str, b, i: &str| {
        Wire::new(Endpoint::new(NodeId(a), o), Endpoint::new(NodeId(b), i))
    };
    p.graph.wires = vec![w(1, "midi", 2, "midi"), w(2, "out", 3, "in")];
    let (mut engine, report) = Engine::new(
        p,
        Arc::new(ac_nodes::registry()),
        Arc::new(Plugins(host.clone())),
        EngineOptions::default(),
    );
    assert!(!report.has_errors(), "{report}");
    let key = NodePath::root(NodeId(2)).to_string();
    assert!(ac_host::has_window(&key), "{label} has a window");
    ac_host::show_window(&key).expect("the window opens");
    eprintln!("{label}: window open");
    let on = [MidiEvent::new(
        0,
        MidiMsg::NoteOn {
            channel: 0,
            note: 69,
            velocity: 100,
        },
    )];
    let mut peak = block(&mut engine, &on);
    let hold = std::env::var("AC_WINDOW_HOLD_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500u64);
    for _ in 0..hold / 10 {
        let _ = ac_host::poll();
        peak = peak.max(block(&mut engine, &[]));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ac_host::window_open(&key), "{label}'s window stayed open");
    assert!(peak > 0.001, "{label} played while its window was open");
    eprintln!("{label}: closing the window");
    ac_host::close_window(&key);
    assert!(!ac_host::window_open(&key));
    for _ in 0..20 {
        let _ = ac_host::poll();
        block(&mut engine, &[]);
    }
    eprintln!("{label}: closed; stopping");
    drop(engine);
    assert!(ac_host::running().is_empty());
}
