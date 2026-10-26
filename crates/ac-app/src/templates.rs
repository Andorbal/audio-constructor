//! Templates (DECISIONS D222): Add ▸ Track, Drum Pad, Clip Slot, and Send
//! each paste a shipped block's instance, wire it in, and pin it, in one
//! undo step; and a selection saved as a template pastes again from the
//! same menu.

use ac_core::{Endpoint, Graph, NodeId, SignalType};
use serde::{Deserialize, Serialize};

use crate::edit::{self, Ctx, generate};
use crate::i18n::tr;

/// The templates that come with the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Track,
    DrumPad,
    ClipSlot,
    Send,
}

impl Template {
    pub const ALL: [Template; 4] = [
        Template::Track,
        Template::DrumPad,
        Template::ClipSlot,
        Template::Send,
    ];

    /// The shipped block it's an instance of.
    pub fn block(self) -> &'static str {
        match self {
            Template::Track => "track",
            Template::DrumPad => "drum-pad",
            Template::ClipSlot => "clip-slot",
            Template::Send => "send",
        }
    }

    pub fn name(self) -> String {
        match self {
            Template::Track => tr("template-track"),
            Template::DrumPad => tr("template-drum-pad"),
            Template::ClipSlot => tr("template-clip-slot"),
            Template::Send => tr("template-send"),
        }
    }

    pub fn tip(self) -> String {
        match self {
            Template::Track => tr("template-track-tip"),
            Template::DrumPad => tr("template-drum-pad-tip"),
            Template::ClipSlot => tr("template-clip-slot-tip"),
            Template::Send => tr("template-send-tip"),
        }
    }
}

/// A selection saved as a template, kept in the editor's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub name: String,
    /// The nodes, as copied to the clipboard.
    pub text: String,
}

/// The first selected node with an output that can feed `ty` (one of
/// that type first, else one that converts), and that output.
fn source(graph: &Graph, ctx: &mut Ctx, selection: &[NodeId], ty: SignalType) -> Option<Endpoint> {
    selection.iter().find_map(|id| {
        let outs = ctx.sockets(graph, *id).outputs;
        let out = outs
            .iter()
            .find(|s| s.ty == ty)
            .or_else(|| outs.iter().find(|s| edit::compatible(s.ty, ty)))?;
        Some(Endpoint::new(*id, out.name.clone()))
    })
}

/// Wire the pasted `instance` of `t` in: from the selection where it
/// takes something, into the master mix where it makes sound. A Track
/// with no sound selected records: an Audio In feeds it and an armed
/// Record. Returns the nodes it added beside it (a Drum Pad's Sample, a
/// recording Track's Audio In and Record).
pub fn wire(
    t: Template,
    graph: &mut Graph,
    ctx: &mut Ctx,
    instance: NodeId,
    selection: &[NodeId],
) -> Vec<NodeId> {
    let mut added = Vec::new();
    let at = graph.node(instance).map_or([0.0, 0.0], |n| n.ui.pos);
    match t {
        Template::Track => {
            let from = source(graph, ctx, selection, SignalType::Audio).unwrap_or_else(|| {
                // Nothing to play: a track that records the input. Its take
                // becomes a Clip in its place (Create Clip from take).
                let input = edit::add_node(
                    graph,
                    ctx,
                    "audio_in",
                    serde_json::json!({ "channels": 1 }),
                    [at[0] - 520.0, at[1]],
                );
                let record = edit::add_node(
                    graph,
                    ctx,
                    "record",
                    serde_json::json!({ "channels": 1 }),
                    [at[0] - 260.0, at[1] + 140.0],
                );
                if let Some(n) = graph.node_mut(record) {
                    n.ui.label = Some(tr("template-track-take"));
                }
                let _ = edit::connect(
                    graph,
                    ctx,
                    Endpoint::new(input, "out"),
                    Endpoint::new(record, "in"),
                );
                added.extend([input, record]);
                Endpoint::new(input, "out")
            });
            let _ = edit::connect(graph, ctx, from, Endpoint::new(instance, "in"));
            generate::hear(graph, ctx, instance);
        }
        Template::Send => {
            if let Some(from) = source(graph, ctx, selection, SignalType::Audio) {
                let _ = edit::connect(graph, ctx, from, Endpoint::new(instance, "in"));
            }
            generate::hear(graph, ctx, instance);
        }
        Template::DrumPad => {
            let sample = selection
                .iter()
                .copied()
                .find(|id| graph.node(*id).is_some_and(|n| n.kind == "sample"))
                .unwrap_or_else(|| {
                    let id = edit::add_node(
                        graph,
                        ctx,
                        "sample",
                        ac_core::empty_config(),
                        [at[0] - 260.0, at[1]],
                    );
                    added.push(id);
                    id
                });
            let _ = edit::connect(
                graph,
                ctx,
                Endpoint::new(sample, "buffer"),
                Endpoint::new(instance, "sound"),
            );
            if let Some(from) = source(graph, ctx, selection, SignalType::Trigger) {
                let _ = edit::connect(graph, ctx, from, Endpoint::new(instance, "hit"));
            }
            generate::hear(graph, ctx, instance);
        }
        Template::ClipSlot => {
            let clip = selection
                .iter()
                .copied()
                .find(|id| graph.node(*id).is_some_and(|n| n.kind == "clip"));
            if let Some(clip) = clip {
                for (o, i) in [("start", "start"), ("end", "stop")] {
                    let _ = edit::connect(
                        graph,
                        ctx,
                        Endpoint::new(instance, o),
                        Endpoint::new(clip, i),
                    );
                }
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ac_core::{NodeInstance, Project};

    use super::*;
    use crate::document::Document;

    fn instance(d: &mut Document, r: &ac_core::Registry, id: &str) -> NodeId {
        let lib = ac_io::Library::shipped_only();
        let src = ac_core::BlockSource::new(ac_core::Origin::Shipped, id);
        let b = lib.get(&src).expect("shipped");
        let text = b
            .block
            .instance(&ac_core::GroupId(id.into()), &src, false)
            .to_json();
        d.edit(r, "paste", |g, ctx| {
            edit::paste(g, ctx, &text, [400.0, 0.0]).unwrap()[0]
        })
    }

    #[test]
    fn a_track_takes_the_selected_sound_into_the_master_mix_behind_its_limiter() {
        let mut p = Project::default();
        for (id, kind) in [(1, "oscillator"), (2, "mix"), (3, "gain"), (4, "audio_out")] {
            p.graph.nodes.push(NodeInstance::new(NodeId(id), kind));
        }
        p.graph.nodes[1].config = serde_json::json!({ "inputs": 1 });
        p.graph.connect(
            Endpoint::new(NodeId(2), "out"),
            Endpoint::new(NodeId(3), "in"),
        );
        p.graph.connect(
            Endpoint::new(NodeId(3), "out"),
            Endpoint::new(NodeId(4), "in"),
        );
        p.graph.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in_0"),
        );
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let track = instance(&mut d, &r, "track");
        d.edit(&r, "wire", |g, ctx| {
            wire(Template::Track, g, ctx, track, &[NodeId(1)])
        });
        let g = &d.project.graph;
        assert_eq!(
            g.wire_into(&Endpoint::new(track, "in"))
                .map(|w| w.from.node),
            Some(NodeId(1))
        );
        assert_eq!(
            g.wire_into(&Endpoint::new(NodeId(2), "in_1"))
                .map(|w| w.from.node),
            Some(track),
            "the full Mix grew an input"
        );
    }

    #[test]
    fn a_drum_pad_gets_a_sample_and_a_clip_slot_drives_the_clip() {
        let mut p = Project::default();
        p.graph.nodes.push(NodeInstance::new(NodeId(1), "clip"));
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "audio_out"));
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let pad = instance(&mut d, &r, "drum-pad");
        let added = d.edit(&r, "wire", |g, ctx| {
            wire(Template::DrumPad, g, ctx, pad, &[])
        });
        assert_eq!(added.len(), 1);
        let g = &d.project.graph;
        assert_eq!(g.node(added[0]).unwrap().kind, "sample");
        assert!(g.wire_into(&Endpoint::new(pad, "sound")).is_some());
        assert_eq!(
            g.wire_into(&Endpoint::new(NodeId(2), "in"))
                .map(|w| w.from.node),
            Some(pad),
            "heard"
        );
        let slot = instance(&mut d, &r, "clip-slot");
        d.edit(&r, "wire", |g, ctx| {
            wire(Template::ClipSlot, g, ctx, slot, &[NodeId(1)])
        });
        let g = &d.project.graph;
        for socket in ["start", "stop"] {
            assert_eq!(
                g.wire_into(&Endpoint::new(NodeId(1), socket))
                    .map(|w| w.from.node),
                Some(slot),
                "{socket}"
            );
        }
    }
}
