//! Plugin libraries are never unloaded (DECISIONS D139): once a plugin has
//! been described, played, and its host dropped, its library is still
//! loaded, so exit handlers, thread-local destructors, and threads it left
//! behind have code to run when the process ends. Its own binary, so that
//! nothing else in the process keeps these copies loaded.

mod common;

use std::path::Path;
use std::sync::Arc;

use ac_core::{
    Endpoint, MidiMsg, NodeId, NodeInstance, PluginFormat, PluginLoader, PluginProcessor,
    PluginRequest, Project, Resources, Wire,
};
use ac_engine::{RenderOptions, render};
use ac_host::{Catalog, PluginHost};
use serde_json::json;

use ac_plugin as _;

struct Plugins(Arc<PluginHost>);

impl Resources for Plugins {
    fn plugin(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        self.0.load(req)
    }
}

/// Describe the bundle, play a note on its instrument, and drop it all.
fn play_and_drop(bundle: &Path, format: PluginFormat) {
    let plugins = ac_host::describe_bundle(bundle).expect("describes");
    let desc = plugins
        .iter()
        .find(|d| d.format == format && d.instrument)
        .cloned()
        .expect("an instrument");
    let host = Arc::new(PluginHost::new(Catalog {
        plugins,
        ..Catalog::default()
    }));
    let mut p = Project::default();
    p.settings.length = 0.5;
    p.graph.nodes = vec![
        NodeInstance::new(NodeId(1), "midi_in"),
        NodeInstance::new(NodeId(2), "plugin").with_config(json!({ "plugin": desc })),
        NodeInstance::new(NodeId(3), "audio_out"),
    ];
    let w = |a, o: &str, b, i: &str| {
        Wire::new(Endpoint::new(NodeId(a), o), Endpoint::new(NodeId(b), i))
    };
    p.graph.wires = vec![w(1, "midi", 2, "midi"), w(2, "out", 3, "in")];
    let opts = RenderOptions {
        sample_rate: Some(48_000),
        block_size: Some(256),
        midi: vec![(
            0,
            MidiMsg::NoteOn {
                channel: 0,
                note: 69,
                velocity: 110,
            },
        )],
        ..RenderOptions::default()
    };
    let out = render(
        &p,
        Arc::new(ac_nodes::registry()),
        Arc::new(Plugins(host.clone())),
        &opts,
    );
    assert!(!out.report.has_errors(), "{}", out.report);
    assert!(
        out.channels.iter().flatten().any(|v| v.abs() > 0.001),
        "{} played",
        format.name()
    );
    drop(out);
    drop(host);
    assert!(ac_host::running().is_empty());
}

#[test]
fn plugin_libraries_stay_loaded_after_their_hosts_go() {
    let (clap, vst3, vst3_binary) = common::copied_bundles("residency");
    for (bundle, binary, format) in [
        (&clap, &clap, PluginFormat::Clap),
        (&vst3, &vst3_binary, PluginFormat::Vst3),
    ] {
        #[cfg(unix)]
        assert!(
            !common::still_loaded(binary),
            "{} isn't loaded before the test",
            format.name()
        );
        play_and_drop(bundle, format);
        // The outcome: still loaded. (Our own plugin can't fail this alone:
        // the system already pins a library that registered thread-local
        // destructors, as Rust code does; many plugins don't.)
        #[cfg(unix)]
        assert!(
            common::still_loaded(binary),
            "{}'s library is still loaded after its host went",
            format.name()
        );
        // The mechanism: every format loads through the resident list.
        assert!(ac_host::is_resident(binary), "{} is kept", format.name());
        // Loading it again uses the copy that's there.
        play_and_drop(bundle, format);
    }
    if let Some(dir) = clap.parent() {
        // Deleting a loaded library's file is fine on Unix; on Windows it
        // fails and the temporary folder stays.
        let _ = std::fs::remove_dir_all(dir);
    }
}
