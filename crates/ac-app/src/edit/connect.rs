//! Wiring without dragging (D103): **lazy connect** (`F` with two nodes
//! selected) and **connect selected to new node** (add a node with two or
//! more selected, and they're wired into it in the order they were
//! selected). Both follow the canvas's type rules ([`super::compatible`]).

use std::collections::{BTreeMap, BTreeSet};

use ac_core::{
    Config, ConfigExt, Endpoint, Graph, GroupDef, GroupId, NodeId, NodeInstance, Registry,
    SignalType,
};

use super::{NEW_SOCKET, SocketInfo, Sockets, compatible};

/// Same type (anything goes with `Any`), as opposed to one that converts.
fn same_type(a: SignalType, b: SignalType) -> bool {
    a == b || a == SignalType::Any || b == SignalType::Any
}

/// Whether an output's name answers an input's: the same name, or `out…`
/// for `in…` (`out` → `in`, `out_l` → `in_l`).
fn names_match(output: &str, input: &str) -> bool {
    if output == input {
        return true;
    }
    match (output.strip_prefix("out"), input.strip_prefix("in")) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// The wire `F` makes from a node with sockets `from` into one with
/// sockets `to` whose inputs in `wired` are taken: the first output and
/// the first free input it fits (main signal sockets come first by
/// convention), unless a pair's names match (`out` → `in`, `midi` →
/// `midi`); among one output's inputs, the same type beats a conversion.
/// Returns the output's and the input's names.
pub fn lazy_pick(
    from: &Sockets,
    to: &Sockets,
    wired: &BTreeSet<String>,
) -> Option<(String, String)> {
    let outputs = from
        .outputs
        .iter()
        .enumerate()
        .filter(|(_, o)| o.name != NEW_SOCKET);
    outputs
        .flat_map(|(oi, o)| {
            to.inputs
                .iter()
                .enumerate()
                .filter(move |(_, i)| {
                    i.name != NEW_SOCKET && !wired.contains(&i.name) && compatible(o.ty, i.ty)
                })
                .map(move |(ii, i)| {
                    let key = (
                        u8::from(!names_match(&o.name, &i.name)),
                        oi,
                        u8::from(!same_type(o.ty, i.ty)),
                        ii,
                    );
                    (key, &o.name, &i.name)
                })
        })
        .min_by_key(|(key, _, _)| *key)
        .map(|(_, o, i)| (o.clone(), i.clone()))
}

/// Why the selected nodes weren't wired into a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedError {
    /// This selected node has no output.
    NoOutput(NodeId),
    /// The new node has no free input left for this node's output.
    NoInput(NodeId, SignalType),
}

/// The sockets a node offers for wiring. A portal tag shows one: a
/// sender its input, a receiver its output (the other is the portal's
/// link, D104).
pub fn wirable(node: Option<&NodeInstance>, mut sockets: Sockets) -> Sockets {
    if let Some(t) = node.and_then(super::portal::tag) {
        if t.send {
            sockets.outputs.clear();
        } else {
            sockets.inputs.clear();
        }
    }
    sockets
}

/// A selected node and its main output (the first one: main signal
/// sockets come first by convention).
#[derive(Clone, Debug)]
pub struct Source {
    pub node: NodeId,
    pub output: Option<SocketInfo>,
}

impl Source {
    pub fn of(node: NodeId, sockets: &Sockets) -> Source {
        Source {
            node,
            output: sockets
                .outputs
                .iter()
                .find(|s| s.name != NEW_SOCKET)
                .cloned(),
        }
    }
}

/// Which input of a new node each selected node feeds, in selection order.
/// The inputs fed are all of one type, the type of the first input the
/// first selected node fits (a Mix's `in_N`, not its gains; a Math node's
/// `a` and `b`), and each selected node takes the next of them in order.
/// All or nothing: if one doesn't fit, nothing is wired.
pub fn plan_feeds(
    sources: &[Source],
    inputs: &[SocketInfo],
) -> Result<Vec<(Endpoint, String)>, FeedError> {
    let inputs: Vec<&SocketInfo> = inputs.iter().filter(|s| s.name != NEW_SOCKET).collect();
    let mut row = None;
    let mut next = 0;
    let mut plan = Vec::new();
    for s in sources {
        let Some(out) = &s.output else {
            return Err(FeedError::NoOutput(s.node));
        };
        let ty = *row.get_or_insert_with(|| {
            inputs
                .iter()
                .find(|i| compatible(out.ty, i.ty))
                .map(|i| i.ty)
        });
        let pick = ty.and_then(|ty| {
            inputs
                .iter()
                .enumerate()
                .skip(next)
                .find(|(_, i)| i.ty == ty && compatible(out.ty, i.ty))
        });
        let Some((k, input)) = pick else {
            return Err(FeedError::NoInput(s.node, out.ty));
        };
        next = k + 1;
        plan.push((Endpoint::new(s.node, out.name.clone()), input.name.clone()));
    }
    Ok(plan)
}

/// A node with repeated inputs set by an `inputs` config (Mix, Select)
/// gets enough of them for `needed` wires; anything else keeps `config`.
pub fn widen_for(registry: &Registry, kind: &str, config: &Config, needed: usize) -> Config {
    let Some(entry) = registry.get(kind) else {
        return config.clone();
    };
    if config.get("inputs").is_some() && config.get_u64_or("inputs", 0) as usize >= needed {
        return config.clone();
    }
    let base = (entry.describe)(config).inputs.len();
    let mut wider = if config.is_object() {
        config.clone()
    } else {
        ac_core::empty_config()
    };
    if let Some(o) = wider.as_object_mut() {
        o.insert("inputs".into(), (needed as u64).into());
    }
    if (entry.describe)(&wider).inputs.len() > base {
        wider
    } else {
        config.clone()
    }
}

/// Would a wire from `from` into `to` close a loop that no Feedback node
/// breaks? (Loops are only legal through Feedback, D4.)
pub fn closes_loop(
    graph: &Graph,
    from: NodeId,
    to: NodeId,
    is_feedback: impl Fn(NodeId) -> bool,
) -> bool {
    if is_feedback(from) {
        return false;
    }
    // Downstream of `to`, stopping at Feedback nodes: does it reach `from`?
    let mut seen: BTreeSet<NodeId> = [to].into();
    let mut todo = vec![to];
    while let Some(n) = todo.pop() {
        if n == from {
            return true;
        }
        if is_feedback(n) {
            continue;
        }
        for w in graph.wires_from(n) {
            if seen.insert(w.to.node) {
                todo.push(w.to.node);
            }
        }
    }
    false
}

/// The name a node goes by on the canvas: its label, its kind's name, or
/// its group's name.
pub fn title(
    node: &NodeInstance,
    registry: &Registry,
    defs: &BTreeMap<GroupId, GroupDef>,
) -> String {
    if let Some(l) = node.ui.label.as_ref().filter(|l| !l.trim().is_empty()) {
        return l.clone();
    }
    if let Some(def) = node.group_def().and_then(|d| defs.get(&d)) {
        return def.name.clone();
    }
    registry
        .get(&node.kind)
        .map_or_else(|| node.kind.clone(), |e| e.meta.name.to_string())
}

/// The order nodes were selected in: the selection is a set, so the editor
/// follows it from frame to frame. Nodes selected together (a box, Select
/// All) count top to bottom, then left to right.
#[derive(Clone, Debug, Default)]
pub struct SelectionOrder {
    order: Vec<NodeId>,
    scope: Vec<NodeId>,
}

impl SelectionOrder {
    /// Catch up with `selection` in `graph` (the graph of group scope
    /// `scope`).
    pub fn sync(&mut self, selection: &BTreeSet<NodeId>, graph: &Graph, scope: &[NodeId]) {
        if self.scope != scope {
            self.order.clear();
            self.scope = scope.to_vec();
        }
        self.order.retain(|id| selection.contains(id));
        let mut new: Vec<(f32, f32, NodeId)> = selection
            .iter()
            .filter(|id| !self.order.contains(id))
            .map(|id| {
                let p = graph.node(*id).map_or([0.0, 0.0], |n| n.ui.pos);
                (p[1], p[0], *id)
            })
            .collect();
        new.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        self.order.extend(new.into_iter().map(|(_, _, id)| id));
    }

    /// The selection, first selected first.
    pub fn get(&self) -> &[NodeId] {
        &self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::sockets_of;
    use ac_core::Project;

    fn info(name: &str, ty: SignalType) -> SocketInfo {
        SocketInfo {
            name: name.into(),
            ty,
            spec: None,
        }
    }

    fn sockets(inputs: &[(&str, SignalType)], outputs: &[(&str, SignalType)]) -> Sockets {
        Sockets {
            inputs: inputs.iter().map(|(n, t)| info(n, *t)).collect(),
            outputs: outputs.iter().map(|(n, t)| info(n, *t)).collect(),
        }
    }

    use SignalType::{Audio, Float, Midi, Trigger};

    #[test]
    fn lazy_connect_follows_socket_order_and_names() {
        let gain = sockets(&[("in", Audio), ("gain", Float)], &[("out", Audio)]);
        // Oscillators put out sample-rate floats.
        let osc = sockets(&[("freq", Float)], &[("out", Float)]);
        let env = sockets(&[("gate", Trigger)], &[("env", Float), ("end", Trigger)]);
        let none = BTreeSet::new();
        // `out` → `in`, converting.
        assert_eq!(
            lazy_pick(&osc, &gain, &none),
            Some(("out".into(), "in".into()))
        );
        // No names to go by: the same type before a conversion.
        assert_eq!(
            lazy_pick(&env, &gain, &none),
            Some(("env".into(), "gain".into()))
        );
        // A taken input is skipped; a conversion is fine when it's all that's left.
        let taken: BTreeSet<String> = ["gain".to_string()].into();
        assert_eq!(
            lazy_pick(&env, &gain, &taken),
            Some(("env".into(), "in".into()))
        );
        let taken: BTreeSet<String> = ["in".to_string()].into();
        assert_eq!(
            lazy_pick(&osc, &gain, &taken),
            Some(("out".into(), "gain".into()))
        );
        // Names match: a stereo pair goes left to left.
        let split = sockets(&[], &[("out_l", Audio), ("out_r", Audio)]);
        let pan = sockets(&[("in_r", Audio), ("in_l", Audio)], &[]);
        assert_eq!(
            lazy_pick(&split, &pan, &none),
            Some(("out_l".into(), "in_l".into()))
        );
        // Nothing fits.
        let midi = sockets(&[], &[("midi", Midi)]);
        assert_eq!(lazy_pick(&midi, &gain, &none), None);
        assert_eq!(lazy_pick(&gain, &osc, &["freq".to_string()].into()), None);
        // The interface's "add a socket" slot is never picked.
        let iface = sockets(&[(NEW_SOCKET, SignalType::Any)], &[]);
        assert_eq!(lazy_pick(&osc, &iface, &none), None);
    }

    #[test]
    fn feeds_go_in_selection_order_or_not_at_all() {
        let src = |id: u64, ty: SignalType| Source {
            node: NodeId(id),
            output: Some(info("out", ty)),
        };
        let mix = [
            info("in_0", Audio),
            info("in_1", Audio),
            info("gain_0", Float),
            info("gain_1", Float),
        ];
        let plan = plan_feeds(&[src(2, Audio), src(1, Audio)], &mix).unwrap();
        assert_eq!(
            plan,
            vec![
                (Endpoint::new(NodeId(2), "out"), "in_0".to_string()),
                (Endpoint::new(NodeId(1), "out"), "in_1".to_string()),
            ]
        );
        // A third audio source doesn't spill into a gain input.
        assert_eq!(
            plan_feeds(&[src(1, Audio), src(2, Audio), src(3, Audio)], &mix),
            Err(FeedError::NoInput(NodeId(3), Audio))
        );
        // Sample-rate floats (oscillators) go into a Mix's audio inputs,
        // not its gains.
        let plan = plan_feeds(&[src(1, Float), src(2, Float)], &mix).unwrap();
        assert_eq!(plan[0].1, "in_0");
        assert_eq!(plan[1].1, "in_1");
        // Two into a filter don't spill over into its cutoff.
        let biquad = [info("in", Audio), info("cutoff", Float), info("q", Float)];
        assert_eq!(
            plan_feeds(&[src(1, Float), src(2, Float)], &biquad),
            Err(FeedError::NoInput(NodeId(2), Float))
        );
        // Floats into float inputs, first to last.
        let math = [
            info("a", Float),
            info("b", Float),
            info("op", SignalType::Int),
        ];
        let plan = plan_feeds(&[src(5, Float), src(4, Float)], &math).unwrap();
        assert_eq!((plan[0].1.as_str(), plan[1].1.as_str()), ("a", "b"));
        // A node with no output, or one whose type nothing takes, stops it.
        let mute = Source {
            node: NodeId(9),
            output: None,
        };
        assert_eq!(
            plan_feeds(&[src(1, Audio), mute], &mix),
            Err(FeedError::NoOutput(NodeId(9)))
        );
        assert_eq!(
            plan_feeds(&[src(1, Trigger), src(2, Audio)], &mix),
            Err(FeedError::NoInput(NodeId(1), Trigger))
        );
    }

    #[test]
    fn mix_grows_to_take_every_selected_node() {
        let r = ac_nodes::registry();
        let empty = ac_core::empty_config();
        // Four inputs by default: enough for three.
        assert_eq!(widen_for(&r, "mix", &empty, 3), empty);
        let six = widen_for(&r, "mix", &empty, 6);
        assert_eq!(six.get_u64_or("inputs", 0), 6);
        // Nodes without repeated inputs keep their settings.
        assert_eq!(widen_for(&r, "gain", &empty, 6), empty);
        assert_eq!(widen_for(&r, "nope", &empty, 6), empty);
        // A Mix that already has enough is left alone.
        let eight = serde_json::json!({"inputs": 8});
        assert_eq!(widen_for(&r, "mix", &eight, 6), eight);
        // Its signature now has the inputs.
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "mix").with_config(six));
        let s = sockets_of(&p.graph, NodeId(1), &r, &p.group_defs, None);
        assert!(s.input("in_5").is_some());
    }

    #[test]
    fn selection_order_follows_clicks_and_reads_boxes_top_down() {
        let mut g = Graph::default();
        for (i, y) in [(1u64, 300.0), (2, 0.0), (3, 100.0), (4, 200.0)] {
            g.nodes
                .push(NodeInstance::new(NodeId(i), "gain").at(0.0, y));
        }
        let mut o = SelectionOrder::default();
        let mut sel: BTreeSet<NodeId> = [NodeId(3)].into();
        o.sync(&sel, &g, &[]);
        sel.insert(NodeId(1));
        o.sync(&sel, &g, &[]);
        assert_eq!(o.get(), &[NodeId(3), NodeId(1)]);
        // Two at once: top to bottom.
        sel.insert(NodeId(4));
        sel.insert(NodeId(2));
        o.sync(&sel, &g, &[]);
        assert_eq!(o.get(), &[NodeId(3), NodeId(1), NodeId(2), NodeId(4)]);
        sel.remove(&NodeId(3));
        o.sync(&sel, &g, &[]);
        assert_eq!(o.get(), &[NodeId(1), NodeId(2), NodeId(4)]);
        // A new scope starts over.
        o.sync(&sel, &g, &[NodeId(7)]);
        assert_eq!(o.get(), &[NodeId(2), NodeId(4), NodeId(1)]);
    }

    #[test]
    fn loops_need_a_feedback_node() {
        // 1 → 2 → 3, and 4 is a Feedback node fed by 3.
        let mut g = Graph::default();
        for i in 1..=4u64 {
            g.nodes.push(NodeInstance::new(NodeId(i), "gain"));
        }
        let ep = |n: u64, s: &str| Endpoint::new(NodeId(n), s);
        g.connect(ep(1, "out"), ep(2, "in"));
        g.connect(ep(2, "out"), ep(3, "in"));
        g.connect(ep(3, "out"), ep(4, "in"));
        let fb = |id: NodeId| id == NodeId(4);
        assert!(closes_loop(&g, NodeId(3), NodeId(1), fb));
        assert!(closes_loop(&g, NodeId(2), NodeId(1), fb));
        assert!(
            !closes_loop(&g, NodeId(1), NodeId(3), fb),
            "just a shortcut"
        );
        // Back through the Feedback node is how a loop is made.
        assert!(!closes_loop(&g, NodeId(4), NodeId(1), fb));
        // Into a Feedback node that reaches back: the loop goes through it.
        g.connect(ep(4, "out"), ep(1, "gain"));
        assert!(!closes_loop(&g, NodeId(2), NodeId(4), fb));
    }

    #[test]
    fn portal_tags_wire_by_the_socket_they_show() {
        let both = sockets(&[("in", SignalType::Any)], &[("out", SignalType::Any)]);
        let mut tag = NodeInstance::new(NodeId(1), ac_core::REROUTE_KIND);
        assert_eq!(wirable(Some(&tag), both.clone()), both, "a plain reroute");
        tag.extra.insert(
            "portal".into(),
            serde_json::json!({"name": "Bass", "role": "send"}),
        );
        let s = wirable(Some(&tag), both.clone());
        assert!(s.outputs.is_empty() && s.inputs.len() == 1);
        tag.extra.insert(
            "portal".into(),
            serde_json::json!({"name": "Bass", "role": "receive"}),
        );
        let s = wirable(Some(&tag), both.clone());
        assert!(s.inputs.is_empty() && s.outputs.len() == 1);
        assert_eq!(wirable(None, both.clone()), both);
    }

    #[test]
    fn titles_are_what_the_canvas_shows() {
        let r = ac_nodes::registry();
        let defs = BTreeMap::new();
        let mut n = NodeInstance::new(NodeId(1), "oscillator");
        assert_eq!(title(&n, &r, &defs), "Oscillator");
        n.ui.label = Some("Bass".into());
        assert_eq!(title(&n, &r, &defs), "Bass");
        assert_eq!(
            title(&NodeInstance::new(NodeId(2), "zzz"), &r, &defs),
            "zzz"
        );
    }
}
