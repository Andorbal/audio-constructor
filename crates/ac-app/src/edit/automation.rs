//! Written automation (DECISIONS D219): the moves recorded on one input
//! become a Graph placed in the song and wired into that input.

use ac_core::{Endpoint, Graph, NodeId, Value};

use super::{Ctx, add_node, connect};

/// Shortest lane, in beats, so a single move still has a length.
const MIN_SPAN: f64 = 1.0 / 16.0;

/// A Graph playing `points` (song beat, value) once, from the first beat,
/// wired into `target`; labelled after the input. Returns it, or `None`
/// when there's nothing to write.
pub fn automation_graph(
    graph: &mut Graph,
    ctx: &mut Ctx,
    target: Endpoint,
    points: &[(f64, f64)],
    pos: [f32; 2],
) -> Option<NodeId> {
    let first = points.first()?.0;
    let last = points.last()?.0;
    let span = (last - first).max(MIN_SPAN);
    let round = |v: f64| (v * 1e6).round() / 1e6;
    let curve: Vec<serde_json::Value> = points
        .iter()
        .map(|&(b, v)| serde_json::json!([round((b - first) / span), round(v)]))
        .collect();
    let config = serde_json::json!({ "unit": "beats", "points": curve });
    let id = add_node(graph, ctx, "graph", config, pos);
    if let Some(n) = graph.node_mut(id) {
        n.params.insert("start".into(), Value::Text("at".into()));
        n.params
            .insert("at".into(), Value::Float(round(first.max(0.0))));
        n.params
            .insert("duration".into(), Value::Float(round(span)));
        n.params.insert("loop".into(), Value::Bool(false));
        n.ui.label = Some(format!("Automation: {}", target.socket));
    }
    if connect(graph, ctx, Endpoint::new(id, "out"), target).is_err() {
        graph.nodes.retain(|n| n.id != id);
        return None;
    }
    Some(id)
}

/// `points` thinned to at most one every `step` beats, with the final
/// point always kept.
pub fn thin(points: &[(f64, f64)], step: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for (i, &p) in points.iter().enumerate() {
        let last = i + 1 == points.len();
        match out.last() {
            Some(&(b, _)) if p.0 - b < step && !last => {}
            _ => out.push(p),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::Document;
    use ac_core::{NodeInstance, Project};

    #[test]
    fn moves_become_a_placed_graph_into_the_input() {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "knob").at(400.0, 0.0));
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let id = d.edit(&r, "Write automation", |g, ctx| {
            automation_graph(
                g,
                ctx,
                Endpoint::new(NodeId(1), "value"),
                &[(8.0, 0.2), (9.0, 0.6), (10.0, 0.4)],
                [0.0, 0.0],
            )
        });
        let id = id.expect("a graph");
        let g = &d.project.graph;
        let n = g.node(id).unwrap();
        assert_eq!(
            n.config["points"],
            serde_json::json!([[0.0, 0.2], [0.5, 0.6], [1.0, 0.4]])
        );
        assert_eq!(n.params.get("at"), Some(&Value::Float(8.0)));
        assert_eq!(n.params.get("duration"), Some(&Value::Float(2.0)));
        assert_eq!(
            g.wire_into(&Endpoint::new(NodeId(1), "value"))
                .map(|w| w.from.node),
            Some(id)
        );
    }

    #[test]
    fn thinning_keeps_one_move_a_step_and_the_last() {
        let pts: Vec<(f64, f64)> = (0..10).map(|k| (k as f64 * 0.01, k as f64)).collect();
        let t = thin(&pts, 0.05);
        assert_eq!(t.first(), Some(&(0.0, 0.0)));
        assert_eq!(t.last(), Some(&(0.09, 9.0)));
        assert!(t.len() <= 4, "{t:?}");
        assert!(t.windows(2).all(|w| w[1].0 > w[0].0));
    }
}
