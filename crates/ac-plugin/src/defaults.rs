//! The projects a new plugin instance starts with: something that works
//! the moment a host loads it, and shows how macros reach the patch.
//!
//! - **Effect**: the host's audio through a Gain whose level is Macro 1
//!   (±24 dB, 0 dB at the macro's default middle position).
//! - **Instrument**: host notes into an eight-voice Subtractive Synth
//!   Voice (the shipped block), with Macros 1–4 on its cutoff, resonance,
//!   envelope amount, and level.

use ac_core::{
    Binding, Block, BlockSource, Endpoint, GroupId, NodeId, NodeInstance, NodePath, Origin,
    Project, Source, VOICES_KIND, Wire,
};
use serde_json::json;

use crate::session::Kind;

/// The shipped block the instrument plays, embedded as the library embeds
/// it (a test checks it's the file in `blocks/`).
pub(crate) const SYNTH_VOICE: &str =
    include_str!("../../../blocks/subtractive-synth-voice.acblock");
const SYNTH_VOICE_ID: &str = "subtractive-synth-voice";

/// The default project for a plugin of `kind`.
pub fn project(kind: Kind) -> Project {
    match kind {
        Kind::Effect => effect(),
        Kind::Instrument => instrument(),
    }
}

fn comment(id: u64, x: f32, y: f32, w: f32, h: f32, text: &str) -> NodeInstance {
    let mut n = NodeInstance::new(NodeId(id), "comment")
        .with_config(json!({ "text": text }))
        .at(x, y);
    n.ui.size = Some([w, h]);
    n
}

fn wire(from: u64, out: &str, to: u64, input: &str) -> Wire {
    Wire::new(
        Endpoint::new(NodeId(from), out),
        Endpoint::new(NodeId(to), input),
    )
}

fn macro_binding(number: u8, node: u64, socket: &str, min: f64, max: f64) -> Binding {
    Binding {
        min: Some(min),
        max: Some(max),
        ..Binding::input(
            Source::Macro { number },
            NodePath::root(NodeId(node)),
            socket,
        )
    }
}

fn effect() -> Project {
    let mut p = Project {
        name: "Effect".into(),
        ..Project::default()
    };
    p.graph.nodes = vec![
        comment(
            1,
            0.0,
            -160.0,
            560.0,
            120.0,
            "The host's audio comes in at Audio In and goes back out of Audio Out. \
             Build anything in between.\n\nMacro 1 sets the level: automate it in the host. \
             Bind more macros in the Bindings panel, or load a project of your own.",
        ),
        NodeInstance::new(NodeId(2), "audio_in")
            .with_config(json!({ "channels": 2 }))
            .at(0.0, 0.0),
        NodeInstance::new(NodeId(3), "gain")
            .with_label("Level")
            .at(270.0, 0.0),
        NodeInstance::new(NodeId(4), "audio_out").at(540.0, 0.0),
    ];
    p.graph.wires = vec![wire(2, "out", 3, "in"), wire(3, "out", 4, "in")];
    p.bindings = vec![macro_binding(1, 3, "gain", -24.0, 24.0)];
    p
}

fn instrument() -> Project {
    let mut p = Project {
        name: "Instrument".into(),
        ..Project::default()
    };
    p.graph.nodes = vec![
        comment(
            1,
            0.0,
            -160.0,
            560.0,
            120.0,
            "Notes from the host come in at MIDI In and play an eight-voice synth. \
             Macros 1 to 4 are its cutoff, resonance, envelope amount, and level: \
             automate them in the host.\n\nOpen the Synth to change its sound, or load a \
             project of your own.",
        ),
        NodeInstance::new(NodeId(2), "midi_in").at(0.0, 0.0),
        NodeInstance::new(NodeId(3), VOICES_KIND)
            .with_config(json!({ "def": SYNTH_VOICE_ID, "voices": 8, "steal": "oldest" }))
            .with_label("Synth")
            .at(270.0, 0.0),
        NodeInstance::new(NodeId(4), "audio_out").at(540.0, 0.0),
    ];
    p.graph.wires = vec![wire(2, "midi", 3, "midi"), wire(3, "out", 4, "in")];
    // The block always parses (a test checks the file); were it broken, the
    // project would report the missing definition instead of failing.
    if let Ok(block) = Block::from_json(SYNTH_VOICE) {
        let mut def = block.def;
        def.source = Some(BlockSource::new(Origin::Shipped, SYNTH_VOICE_ID).to_string());
        p.group_defs.extend(block.uses);
        p.group_defs.insert(GroupId(SYNTH_VOICE_ID.into()), def);
    }
    p.bindings = vec![
        macro_binding(1, 3, "cutoff", 100.0, 8000.0),
        macro_binding(2, 3, "resonance", 0.0, 1.0),
        macro_binding(3, 3, "env_amount", -2.0, 6.0),
        macro_binding(4, 3, "level", -30.0, 6.0),
    ];
    p
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ac_core::NoResources;
    use ac_engine::{Engine, EngineOptions, HostIo};

    #[test]
    fn the_synth_voice_is_the_shipped_block() {
        let on_disk = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../blocks/subtractive-synth-voice.acblock"
        ))
        .unwrap();
        assert_eq!(on_disk, SYNTH_VOICE);
        assert!(Block::from_json(SYNTH_VOICE).is_ok());
    }

    #[test]
    fn defaults_compile_cleanly_and_bind_their_macros() {
        let registry = Arc::new(ac_nodes::registry());
        for kind in [Kind::Effect, Kind::Instrument] {
            let p = project(kind);
            let json = p.to_json();
            assert_eq!(Project::from_json(&json).unwrap(), p, "round-trips");
            let (mut e, report) = Engine::new(
                p.clone(),
                registry.clone(),
                Arc::new(NoResources),
                EngineOptions::default(),
            );
            assert!(report.diagnostics.is_empty(), "{kind:?}: {report}");
            // One block runs; every macro binding found its input.
            let mut out = vec![0.0; 512];
            let input = vec![0.0; 512];
            e.process(&mut HostIo {
                frames: 256,
                input: &input,
                in_channels: 2,
                output: &mut out,
                out_channels: 2,
                midi: &[],
                midi_out: None,
                controls: &[],
                osc_out: None,
            });
            assert_eq!(e.handle.binding_values().len(), p.bindings.len());
        }
    }
}
