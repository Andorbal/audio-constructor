//! A plugin's own window, embedded in the window the host makes: our
//! instrument (whose CLAP build has the node editor as its window) opens,
//! stays open while the host polls, closes, opens again, and goes with its
//! instance. Needs a display, so it's ignored by default:
//! `xvfb-run -a cargo test -p ac-host --test plugin_window -- --ignored`
//! (CI runs it that way on Linux).

mod common;

use std::sync::Arc;
use std::time::Duration;

use ac_core::{
    Endpoint, NodeId, NodeInstance, NodePath, PluginFormat, PluginLoader, PluginProcessor,
    PluginRequest, Project, Resources, Wire,
};
use ac_engine::{Engine, EngineOptions, HostIo};
use ac_host::{Catalog, PluginHost};
use serde_json::json;

use ac_plugin as _;

struct Plugins(Arc<PluginHost>);

impl Resources for Plugins {
    fn plugin(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        self.0.load(req)
    }
}

fn block(engine: &mut Engine) {
    let mut out = vec![0.0f32; 512];
    engine.process(&mut HostIo {
        frames: 256,
        input: &[],
        in_channels: 0,
        output: &mut out,
        out_channels: 2,
        midi: &[],
        midi_out: None,
        controls: &[],
        osc_out: None,
    });
}

/// Poll and play for about `ms` milliseconds, as an editor's frames would.
fn run_for(engine: &mut Engine, ms: u64) -> Vec<ac_host::HostEvent> {
    let mut events = Vec::new();
    for _ in 0..ms / 10 {
        events.extend(ac_host::poll());
        block(engine);
        std::thread::sleep(Duration::from_millis(10));
    }
    events
}

#[test]
#[ignore = "needs an X display (xvfb-run)"]
fn the_plugin_window_opens_in_a_host_window_and_closes() {
    let plugins = ac_host::describe_bundle(&common::clap_bundle()).expect("describes");
    let host = Arc::new(PluginHost::new(Catalog {
        plugins: plugins.clone(),
        ..Catalog::default()
    }));
    let desc = Catalog {
        plugins,
        ..Catalog::default()
    }
    .find(
        PluginFormat::Clap,
        "io.github.andorbal.audio-constructor.instrument",
    )
    .cloned()
    .expect("the instrument");
    let mut p = Project::default();
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": desc })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    p.graph.wires = vec![Wire::new(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(3), "in"),
    )];
    let (mut engine, report) = Engine::new(
        p,
        Arc::new(ac_nodes::registry()),
        Arc::new(Plugins(host)),
        EngineOptions::default(),
    );
    assert!(!report.has_errors(), "{report}");
    let key = NodePath::root(NodeId(2)).to_string();
    assert!(ac_host::has_window(&key), "the instrument has a window");
    assert!(!ac_host::window_open(&key));
    ac_host::show_window(&key).expect("the window opens");
    // `AC_WINDOW_HOLD_MS` keeps it open longer, to look at it.
    let hold = std::env::var("AC_WINDOW_HOLD_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    let events = run_for(&mut engine, hold);
    assert!(ac_host::window_open(&key), "still open: {events:?}");
    // Showing it again just brings it forward.
    ac_host::show_window(&key).expect("shows again");
    ac_host::close_window(&key);
    assert!(!ac_host::window_open(&key));
    run_for(&mut engine, 100);
    ac_host::show_window(&key).expect("reopens");
    run_for(&mut engine, 300);
    assert!(ac_host::window_open(&key));
    // The window goes with its instance.
    drop(engine);
    assert!(ac_host::running().is_empty());
    assert!(!ac_host::window_open(&key));
}
