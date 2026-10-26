//! Which Audio Out nodes are sounding, for the status bar (DECISIONS
//! D201): when something won't go quiet, this says where it comes out, and
//! clicking the readout shows the node.

use std::collections::HashMap;

use ac_core::{Graph, NodePath, Project, Registry};
use ac_engine::TelemetryReader;

/// Peak (linear) above which an output counts as sounding: about −66 dBFS.
const AUDIBLE: f32 = 5e-4;
/// How long an output still counts as sounding after its last audible
/// block (seconds), so short notes don't flicker.
const HOLD: f64 = 0.4;

/// One sounding output: what to call it, and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub name: String,
    pub path: NodePath,
}

#[derive(Default)]
pub struct Sounding {
    last: HashMap<NodePath, f64>,
}

impl Sounding {
    /// The outputs heard in the last [`HOLD`] seconds, in project order.
    pub fn update(
        &mut self,
        project: &Project,
        registry: &Registry,
        tele: &TelemetryReader,
        now: f64,
    ) -> Vec<Output> {
        let mut outs = Vec::new();
        collect(project, registry, &project.graph, None, "", &mut outs, 0);
        let mut heard = Vec::new();
        for (path, name) in outs {
            let peak = (0..2)
                .filter_map(|k| tele.node_value(&path, k))
                .fold(0.0f32, f32::max);
            if peak.is_finite() && peak > AUDIBLE {
                self.last.insert(path.clone(), now);
            }
            if self.last.get(&path).is_some_and(|&t| now - t <= HOLD) {
                heard.push(Output { name, path });
            }
        }
        self.last.retain(|_, t| now - *t <= HOLD);
        heard
    }
}

/// Every Audio Out under `graph` (inside groups too), with its engine path
/// and its name (a group's outputs read "Group › Output").
fn collect(
    project: &Project,
    registry: &Registry,
    graph: &Graph,
    parent: Option<&NodePath>,
    prefix: &str,
    out: &mut Vec<(NodePath, String)>,
    depth: usize,
) {
    // Groups can't contain themselves (the compiler rejects it), but a
    // broken file could say so; stop well before that matters.
    if depth > 16 {
        return;
    }
    for n in &graph.nodes {
        let path = match parent {
            Some(p) => p.child(n.id),
            None => NodePath::root(n.id),
        };
        let name = n.ui.label.clone().unwrap_or_else(|| {
            registry
                .get(&n.kind)
                .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string())
        });
        let full = if prefix.is_empty() {
            name
        } else {
            format!("{prefix} › {name}")
        };
        if n.kind == "audio_out" {
            out.push((path, full));
        } else if let Some(def) = n.group_def().and_then(|d| project.group_defs.get(&d)) {
            let name = n.ui.label.clone().unwrap_or_else(|| def.name.clone());
            let prefix = if prefix.is_empty() {
                name
            } else {
                format!("{prefix} › {name}")
            };
            collect(
                project,
                registry,
                &def.graph,
                Some(&path),
                &prefix,
                out,
                depth + 1,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{NodeId, NodeInstance};

    #[test]
    fn outputs_are_found_inside_groups_with_their_names() {
        let registry = ac_nodes::registry();
        let mut project = Project::default();
        let mut main = NodeInstance::new(NodeId(1), "audio_out");
        main.ui.label = Some("Main".into());
        project.graph.nodes.push(main);
        project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "audio_out"));
        let mut out = Vec::new();
        collect(&project, &registry, &project.graph, None, "", &mut out, 0);
        let names: Vec<&str> = out.iter().map(|o| o.1.as_str()).collect();
        assert_eq!(names, ["Main", "Audio Out"]);
        assert_eq!(out[1].0, NodePath::root(NodeId(2)));
    }
}
