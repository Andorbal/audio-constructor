//! Create Clip from take (DECISIONS D208): a Clip of the take, placed in the
//! song at the beat the take started, playing where the recorded sound went.

use ac_core::{Endpoint, Graph, NodeId, SignalType, Value};

use super::{Ctx, add_node, connect};

/// What a take knows about itself, for the clip made from it.
pub struct TakeClip {
    /// The Clip's config (its file, channels, hash).
    pub config: ac_core::Config,
    /// The song beat the take started at, when the transport was playing.
    pub beat: Option<f64>,
    /// The Record node that made it, in this graph.
    pub record: Option<NodeId>,
    /// Where to put the Clip.
    pub pos: [f32; 2],
}

/// Add the Clip: started *At beat…* where the take began, and wired into
/// every audio input the Record's source also feeds (the take replaces
/// the live sound there, which stays wired to the Record for the next
/// take). Returns the Clip's id.
pub fn clip_from_take(graph: &mut Graph, ctx: &mut Ctx, take: TakeClip) -> NodeId {
    let clip = add_node(graph, ctx, "clip", take.config, take.pos);
    if let (Some(beat), Some(n)) = (take.beat, graph.node_mut(clip)) {
        n.params.insert("start".into(), Value::Text("at".into()));
        n.params.insert("at".into(), Value::Float(beat.max(0.0)));
    }
    let Some(record) = take.record else {
        return clip;
    };
    let source = graph
        .wires_to(record)
        .find(|w| w.to.socket == "in")
        .map(|w| w.from.clone());
    let Some(source) = source else {
        return clip;
    };
    let targets: Vec<Endpoint> = graph
        .wires_from(source.node)
        .filter(|w| w.from.socket == source.socket && w.to.node != record)
        .map(|w| w.to.clone())
        .filter(|to| ctx.in_type(graph, to) == Some(SignalType::Audio))
        .collect();
    for to in targets {
        let _ = connect(graph, ctx, Endpoint::new(clip, "out"), to);
    }
    clip
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::Document;
    use ac_core::{NodeInstance, Project};

    #[test]
    fn a_take_becomes_a_placed_clip_playing_where_the_mic_went() {
        let mut p = Project::default();
        for (id, kind) in [(1, "audio_in"), (2, "record"), (3, "mix"), (4, "meter")] {
            p.graph.nodes.push(NodeInstance::new(NodeId(id), kind));
        }
        let g = &mut p.graph;
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        );
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(3), "in_0"),
        );
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(4), "in"),
        );
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let clip = d.edit(&r, "Create Clip from take", |g, ctx| {
            clip_from_take(
                g,
                ctx,
                TakeClip {
                    config: serde_json::json!({ "file": "assets/vox-001.wav", "channels": 1 }),
                    beat: Some(16.0),
                    record: Some(NodeId(2)),
                    pos: [0.0, 0.0],
                },
            )
        });
        let g = &d.project.graph;
        let n = g.node(clip).expect("the clip");
        assert_eq!(n.params.get("start"), Some(&Value::Text("at".into())));
        assert_eq!(n.params.get("at"), Some(&Value::Float(16.0)));
        let from = |id: u64, socket: &str| {
            g.wire_into(&Endpoint::new(NodeId(id), socket))
                .map(|w| w.from.node)
        };
        assert_eq!(from(3, "in_0"), Some(clip), "the take plays in the mix");
        assert_eq!(from(4, "in"), Some(clip));
        assert_eq!(from(2, "in"), Some(NodeId(1)), "the mic still records");
    }
}
