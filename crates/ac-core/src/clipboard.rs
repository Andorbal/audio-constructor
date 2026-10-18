//! Copy and paste: a subgraph serialized as JSON text, so patches can travel
//! through the system clipboard and chat messages.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, NodeInstance, Wire};
use crate::group::GroupDef;
use crate::id::{GroupId, IdGen, NodeId};
use crate::project::to_canonical_json;

/// Marker identifying clipboard text as ours.
pub const SUBGRAPH_FORMAT: &str = "audio-constructor/subgraph";

/// Selected nodes, the wires between them, and the group definitions they use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subgraph {
    pub format: String,
    pub version: u32,
    pub nodes: Vec<NodeInstance>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub group_defs: BTreeMap<GroupId, GroupDef>,
}

impl Subgraph {
    /// Copy `selection` out of `graph`. Only wires with both ends selected
    /// come along. Group definitions used by selected nodes (and by the
    /// definitions themselves) are included.
    pub fn copy(
        graph: &Graph,
        defs: &BTreeMap<GroupId, GroupDef>,
        selection: &BTreeSet<NodeId>,
    ) -> Subgraph {
        let nodes: Vec<NodeInstance> = graph
            .nodes
            .iter()
            .filter(|n| selection.contains(&n.id))
            .cloned()
            .collect();
        let wires = graph
            .wires
            .iter()
            .filter(|w| selection.contains(&w.from.node) && selection.contains(&w.to.node))
            .cloned()
            .collect();
        let mut used = BTreeMap::new();
        let mut stack: Vec<GroupId> = nodes.iter().filter_map(NodeInstance::group_def).collect();
        while let Some(id) = stack.pop() {
            if used.contains_key(&id) {
                continue;
            }
            if let Some(def) = defs.get(&id) {
                stack.extend(def.graph.referenced_groups());
                used.insert(id, def.clone());
            }
        }
        Subgraph {
            format: SUBGRAPH_FORMAT.to_string(),
            version: 1,
            nodes,
            wires,
            group_defs: used,
        }
    }

    pub fn to_json(&self) -> String {
        to_canonical_json(self)
    }

    /// Parse clipboard text; `None` if it isn't one of ours.
    pub fn from_json(text: &str) -> Option<Subgraph> {
        let s: Subgraph = serde_json::from_str(text.trim()).ok()?;
        (s.format == SUBGRAPH_FORMAT).then_some(s)
    }

    /// Paste into `graph`, giving every node a fresh id and moving it by
    /// `offset`. Group definitions are merged into `defs`: identical ones are
    /// reused, conflicting ones get a new id. Returns the new node ids in the
    /// order they appear in the subgraph.
    pub fn paste(
        &self,
        graph: &mut Graph,
        defs: &mut BTreeMap<GroupId, GroupDef>,
        ids: &mut IdGen,
        offset: [f32; 2],
    ) -> Vec<NodeId> {
        // Merge definitions first so pasted nodes can be pointed at the final ids.
        let mut def_map: HashMap<GroupId, GroupId> = HashMap::new();
        for (id, def) in &self.group_defs {
            match defs.get(id) {
                None => {
                    def_map.insert(id.clone(), id.clone());
                }
                Some(existing) if existing == def => {
                    def_map.insert(id.clone(), id.clone());
                }
                Some(_) => {
                    let mut n = 2;
                    let new_id = loop {
                        let candidate = GroupId(format!("{}-{n}", id.0));
                        if !defs.contains_key(&candidate)
                            && !self.group_defs.contains_key(&candidate)
                        {
                            break candidate;
                        }
                        n += 1;
                    };
                    def_map.insert(id.clone(), new_id);
                }
            }
        }
        for (id, def) in &self.group_defs {
            let target = def_map.get(id).cloned().unwrap_or_else(|| id.clone());
            if defs.contains_key(&target) {
                continue;
            }
            let mut def = def.clone();
            retarget_groups(&mut def.graph, &def_map);
            defs.insert(target, def);
        }

        let mut id_map: HashMap<NodeId, NodeId> = HashMap::new();
        let mut new_ids = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let taken: BTreeSet<NodeId> = graph.ids();
            let fresh = ids.fresh(|id| taken.contains(&id) || id_map.values().any(|v| *v == id));
            id_map.insert(node.id, fresh);
            let mut n = node.clone();
            n.id = fresh;
            n.ui.pos = [n.ui.pos[0] + offset[0], n.ui.pos[1] + offset[1]];
            graph.nodes.push(n);
            new_ids.push(fresh);
        }
        retarget_groups(graph, &def_map);
        for w in &self.wires {
            if let (Some(&f), Some(&t)) = (id_map.get(&w.from.node), id_map.get(&w.to.node)) {
                let mut w = w.clone();
                w.from.node = f;
                w.to.node = t;
                graph.connect(w.from, w.to);
            }
        }
        new_ids
    }
}

fn retarget_groups(graph: &mut Graph, map: &HashMap<GroupId, GroupId>) {
    for n in &mut graph.nodes {
        // Groups and Voices nodes both reference definitions.
        if let Some(def) = n.group_def()
            && let Some(new) = map.get(&def)
            && *new != def
            && let Some(obj) = n.config.as_object_mut()
        {
            obj.insert("def".into(), new.0.clone().into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Endpoint;
    use crate::graph::GROUP_KIND;

    fn graph() -> Graph {
        let mut g = Graph::default();
        g.nodes
            .push(NodeInstance::new(NodeId(1), "oscillator").at(0.0, 0.0));
        g.nodes
            .push(NodeInstance::new(NodeId(2), "gain").at(100.0, 0.0));
        g.nodes
            .push(NodeInstance::new(NodeId(3), "audio_out").at(200.0, 0.0));
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        );
        g.connect(
            Endpoint::new(NodeId(2), "out"),
            Endpoint::new(NodeId(3), "in"),
        );
        g
    }

    #[test]
    fn copy_keeps_only_internal_wires() {
        let g = graph();
        let sel: BTreeSet<_> = [NodeId(1), NodeId(2)].into();
        let s = Subgraph::copy(&g, &BTreeMap::new(), &sel);
        assert_eq!(s.nodes.len(), 2);
        assert_eq!(s.wires.len(), 1);
    }

    #[test]
    fn paste_remaps_ids_and_wires() {
        let mut g = graph();
        let sel: BTreeSet<_> = [NodeId(1), NodeId(2)].into();
        let text = Subgraph::copy(&g, &BTreeMap::new(), &sel).to_json();
        let s = Subgraph::from_json(&text).unwrap();
        let mut defs = BTreeMap::new();
        let new = s.paste(&mut g, &mut defs, &mut IdGen::new(1), [10.0, 20.0]);
        assert_eq!(new.len(), 2);
        assert_eq!(g.nodes.len(), 5);
        for id in &new {
            assert!(![NodeId(1), NodeId(2), NodeId(3)].contains(id));
        }
        assert!(
            g.wires
                .iter()
                .any(|w| w.from.node == new[0] && w.to.node == new[1])
        );
        assert_eq!(g.node(new[1]).unwrap().ui.pos, [110.0, 20.0]);
        // The original wiring is untouched.
        assert_eq!(g.wires.len(), 3);
    }

    #[test]
    fn foreign_text_is_rejected() {
        assert!(Subgraph::from_json("hello").is_none());
        assert!(Subgraph::from_json(r#"{"format": "other", "version": 1, "nodes": []}"#).is_none());
    }

    #[test]
    fn conflicting_group_defs_are_renamed() {
        let mut def_a = GroupDef::new("A");
        def_a.description = "first".into();
        let mut def_b = GroupDef::new("A");
        def_b.description = "second".into();
        let mut src = Graph::default();
        src.nodes.push(
            NodeInstance::new(NodeId(1), GROUP_KIND).with_config(serde_json::json!({"def": "g"})),
        );
        let src_defs: BTreeMap<_, _> = [(GroupId::from("g"), def_b.clone())].into();
        let sub = Subgraph::copy(&src, &src_defs, &[NodeId(1)].into());
        let mut dst = Graph::default();
        let mut dst_defs: BTreeMap<_, _> = [(GroupId::from("g"), def_a.clone())].into();
        let new = sub.paste(&mut dst, &mut dst_defs, &mut IdGen::new(3), [0.0, 0.0]);
        assert_eq!(dst_defs.len(), 2);
        assert_eq!(dst_defs[&GroupId::from("g")], def_a);
        assert_eq!(dst_defs[&GroupId::from("g-2")], def_b);
        assert_eq!(
            dst.node(new[0]).unwrap().group_def(),
            Some(GroupId::from("g-2"))
        );
    }

    #[test]
    fn identical_group_defs_are_reused() {
        let def = GroupDef::new("A");
        let mut src = Graph::default();
        src.nodes.push(
            NodeInstance::new(NodeId(1), GROUP_KIND).with_config(serde_json::json!({"def": "g"})),
        );
        let defs: BTreeMap<_, _> = [(GroupId::from("g"), def.clone())].into();
        let sub = Subgraph::copy(&src, &defs, &[NodeId(1)].into());
        let mut dst_defs = defs.clone();
        let mut dst = Graph::default();
        let new = sub.paste(&mut dst, &mut dst_defs, &mut IdGen::new(3), [0.0, 0.0]);
        assert_eq!(dst_defs.len(), 1);
        assert_eq!(
            dst.node(new[0]).unwrap().group_def(),
            Some(GroupId::from("g"))
        );
    }
}
