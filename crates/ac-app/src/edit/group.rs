//! Groups: make a group from a selection, ungroup, and the bookkeeping that
//! keeps definitions and instances consistent.

use std::collections::{BTreeMap, BTreeSet};

use ac_core::{
    Endpoint, GROUP_KIND, Graph, GroupDef, GroupId, GroupInput, GroupOutput, NodeId, NodeInstance,
    NodePath, Project, SignalType, Value, Wire,
};

use super::{
    Ctx, GROUP_IN, GROUP_OUT, Interface, add_virtual_wires, approx_size, is_virtual, unique_name,
};

/// Does `def` (directly or through nested groups) contain an instance of `target`?
pub fn def_contains(defs: &BTreeMap<GroupId, GroupDef>, def: &GroupId, target: &GroupId) -> bool {
    let mut stack = vec![def.clone()];
    let mut seen = BTreeSet::new();
    while let Some(d) = stack.pop() {
        if !seen.insert(d.clone()) {
            continue;
        }
        let Some(g) = defs.get(&d) else { continue };
        for r in g.graph.referenced_groups() {
            if r == *target {
                return true;
            }
            stack.push(r);
        }
    }
    false
}

/// Every flattened path of an instance of `def` in the project (the paths
/// the engine uses for nodes inside it).
pub fn instance_paths(project: &Project, def: &GroupId) -> Vec<NodePath> {
    let mut out = Vec::new();
    fn walk(
        project: &Project,
        graph: &Graph,
        prefix: &[NodeId],
        def: &GroupId,
        out: &mut Vec<NodePath>,
        depth: usize,
    ) {
        if depth > 32 {
            return;
        }
        for n in &graph.nodes {
            let Some(d) = n.group_def() else { continue };
            let mut path = prefix.to_vec();
            path.push(n.id);
            if d == *def {
                out.push(NodePath(path.clone()));
            }
            if let Some(inner) = project.group_defs.get(&d) {
                walk(project, &inner.graph, &path, def, out, depth + 1);
            }
        }
    }
    walk(project, &project.graph, &[], def, &mut out, 0);
    out
}

/// Is `def` used by any instance in the project (or in `extra`, a graph
/// taken out of the project while it's being edited)?
pub fn def_in_use(
    project_graph: &Graph,
    defs: &BTreeMap<GroupId, GroupDef>,
    extra: &Graph,
    def: &GroupId,
) -> bool {
    std::iter::once(project_graph)
        .chain(std::iter::once(extra))
        .chain(defs.values().map(|d| &d.graph))
        .any(|g| g.nodes.iter().any(|n| n.group_def().as_ref() == Some(def)))
}

/// Remove wires to or from sockets that a group instance no longer has
/// (after its definition's interface lost a socket), in every graph. A
/// `Voices` instance also has the `midi` input that picks its voices,
/// which isn't part of the interface.
pub fn prune_instance_wires(project: &mut Project) {
    let iface: BTreeMap<GroupId, (BTreeSet<String>, BTreeSet<String>)> = project
        .group_defs
        .iter()
        .map(|(id, d)| {
            (
                id.clone(),
                (
                    d.inputs.iter().map(|i| i.name.clone()).collect(),
                    d.outputs.iter().map(|o| o.name.clone()).collect(),
                ),
            )
        })
        .collect();
    let prune = |g: &mut Graph| {
        // Each instance's definition, and whether it's a Voices node.
        let defs: BTreeMap<NodeId, (GroupId, bool)> = g
            .nodes
            .iter()
            .filter_map(|n| {
                n.group_def()
                    .map(|d| (n.id, (d, n.kind == ac_core::VOICES_KIND)))
            })
            .collect();
        g.wires.retain(|w| {
            let bad_to = defs.get(&w.to.node).is_some_and(|(d, voices)| {
                let picks_voices = *voices && w.to.socket == "midi";
                iface
                    .get(d)
                    .is_some_and(|(ins, _)| !ins.contains(&w.to.socket) && !picks_voices)
            });
            let bad_from = defs
                .get(&w.from.node)
                .and_then(|(d, _)| iface.get(d))
                .is_some_and(|(_, outs)| !outs.contains(&w.from.socket));
            !bad_to && !bad_from
        });
    };
    prune(&mut project.graph);
    for d in project.group_defs.values_mut() {
        prune(&mut d.graph);
    }
}

fn fresh_def_id(defs: &BTreeMap<GroupId, GroupDef>) -> (GroupId, String) {
    for n in 1.. {
        let id = GroupId(format!("group-{n}"));
        if !defs.contains_key(&id) {
            let name = if n == 1 {
                "Group".to_string()
            } else {
                format!("Group {n}")
            };
            return (id, name);
        }
    }
    (GroupId("group".into()), "Group".into())
}

/// Bounding box `[min_x, min_y, max_x, max_y]` of nodes, using approximate sizes.
fn bbox(graph: &Graph, ctx: &Ctx, ids: &BTreeSet<NodeId>) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for n in graph.nodes.iter().filter(|n| ids.contains(&n.id)) {
        let s = approx_size(n, &ctx.sockets(graph, n.id));
        b[0] = b[0].min(n.ui.pos[0]);
        b[1] = b[1].min(n.ui.pos[1]);
        b[2] = b[2].max(n.ui.pos[0] + s[0]);
        b[3] = b[3].max(n.ui.pos[1] + s[1]);
    }
    if b[0] > b[2] { [0.0; 4] } else { b }
}

/// Make a group of `selection`: the nodes and the wires between them move
/// into a new definition; wires crossing the boundary become interface
/// sockets (one input per outside source, one output per inside source);
/// an instance replaces the selection at its center and is rewired.
/// Returns the instance id and the new definition's id.
pub fn make_group(
    graph: &mut Graph,
    ctx: &mut Ctx,
    selection: &BTreeSet<NodeId>,
) -> Option<(NodeId, GroupId)> {
    let sel: BTreeSet<NodeId> = selection
        .iter()
        .copied()
        .filter(|id| !is_virtual(*id) && graph.contains(*id))
        .collect();
    if sel.is_empty() {
        return None;
    }
    let b = bbox(graph, ctx, &sel);
    let center = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];

    let inside = |e: &Endpoint| sel.contains(&e.node);
    let mut incoming: Vec<Wire> = graph
        .wires
        .iter()
        .filter(|w| inside(&w.to) && !inside(&w.from))
        .cloned()
        .collect();
    // Interface order follows the inner nodes top to bottom.
    let ypos = |id: NodeId| graph.node(id).map_or(0.0, |n| n.ui.pos[1]);
    incoming.sort_by(|a, b| ypos(a.to.node).total_cmp(&ypos(b.to.node)));
    let mut outgoing: Vec<Wire> = graph
        .wires
        .iter()
        .filter(|w| inside(&w.from) && !inside(&w.to))
        .cloned()
        .collect();
    outgoing.sort_by(|a, b| ypos(a.from.node).total_cmp(&ypos(b.from.node)));

    let mut iface = Interface::default();
    // Outside source → interface input name.
    let mut input_for: Vec<(Endpoint, String)> = Vec::new();
    for w in &incoming {
        if let Some((_, name)) = input_for.iter().find(|(s, _)| *s == w.from) {
            let name = name.clone();
            if let Some(i) = iface.inputs.iter_mut().find(|i| i.name == name) {
                i.targets.push(w.to.clone());
            }
            continue;
        }
        let info = ctx.sockets(graph, w.to.node).input(&w.to.socket).cloned();
        let taken: Vec<&str> = iface.inputs.iter().map(|i| i.name.as_str()).collect();
        let name = unique_name(&w.to.socket, &taken);
        let mut ty = info.as_ref().map_or(SignalType::Any, |i| i.ty);
        if ty == SignalType::Any {
            ty = ctx.out_type(graph, &w.from).unwrap_or(SignalType::Float);
        }
        if ty == SignalType::Any {
            ty = SignalType::Float;
        }
        let spec = info.and_then(|i| i.spec);
        iface.inputs.push(GroupInput {
            name: name.clone(),
            ty,
            unit: spec.as_ref().map_or(ac_core::Unit::Generic, |s| s.unit),
            default: spec
                .as_ref()
                .map(|s| s.default.clone())
                .filter(|v| !matches!(v, Value::Text(t) if t.is_empty())),
            range: spec.as_ref().and_then(|s| s.range.map(|(a, b)| [a, b])),
            targets: vec![w.to.clone()],
        });
        input_for.push((w.from.clone(), name));
    }
    // Inside source → interface output name.
    let mut output_for: Vec<(Endpoint, String)> = Vec::new();
    for w in &outgoing {
        if output_for.iter().any(|(s, _)| *s == w.from) {
            continue;
        }
        let taken: Vec<&str> = iface.outputs.iter().map(|o| o.name.as_str()).collect();
        let name = unique_name(&w.from.socket, &taken);
        let mut ty = ctx.out_type(graph, &w.from).unwrap_or(SignalType::Audio);
        if ty == SignalType::Any {
            ty = ctx.in_type(graph, &w.to).unwrap_or(SignalType::Audio);
        }
        if ty == SignalType::Any {
            ty = SignalType::Audio;
        }
        iface.outputs.push(GroupOutput {
            name: name.clone(),
            ty,
            source: w.from.clone(),
        });
        output_for.push((w.from.clone(), name));
    }

    // The definition's graph: selected nodes centered on the origin.
    let mut inner = Graph::default();
    let mut rest = Vec::new();
    for n in std::mem::take(&mut graph.nodes) {
        if sel.contains(&n.id) {
            let mut n = n;
            n.ui.pos = [n.ui.pos[0] - center[0], n.ui.pos[1] - center[1]];
            inner.nodes.push(n);
        } else {
            rest.push(n);
        }
    }
    graph.nodes = rest;
    let (internal, external): (Vec<Wire>, Vec<Wire>) = std::mem::take(&mut graph.wires)
        .into_iter()
        .partition(|w| inside(&w.from) && inside(&w.to));
    inner.wires = internal;
    graph.wires = external
        .into_iter()
        .filter(|w| !inside(&w.from) && !inside(&w.to))
        .collect();

    let (def_id, name) = fresh_def_id(ctx.defs);
    let mut def = GroupDef::new(name);
    def.inputs = iface.inputs;
    def.outputs = iface.outputs;
    def.graph = inner;
    ctx.defs.insert(def_id.clone(), def);

    let instance = ctx.fresh_id(graph);
    let mut node =
        NodeInstance::new(instance, GROUP_KIND).with_config(serde_json::json!({ "def": def_id.0 }));
    let size = approx_size(&node, &ctx.sockets_for_def(&def_id));
    node.ui.pos = [center[0] - size[0] / 2.0, center[1] - size[1] / 2.0];
    graph.nodes.push(node);
    for (src, name) in &input_for {
        graph.connect(src.clone(), Endpoint::new(instance, name.clone()));
    }
    for w in &outgoing {
        if let Some((_, name)) = output_for.iter().find(|(s, _)| *s == w.from) {
            graph.connect(Endpoint::new(instance, name.clone()), w.to.clone());
        }
    }
    Some((instance, def_id))
}

impl Ctx<'_> {
    fn sockets_for_def(&self, def: &GroupId) -> super::Sockets {
        let mut g = Graph::default();
        let id = NodeId(1);
        g.nodes.push(
            NodeInstance::new(id, GROUP_KIND).with_config(serde_json::json!({ "def": def.0 })),
        );
        self.sockets(&g, id)
    }
}

/// Ungroup an instance: the definition's nodes come back into `graph` with
/// fresh ids around the instance's position, rewired through the interface.
/// Values set on the instance's unwired inputs carry over to the inner
/// inputs. An unused definition is removed. Returns the new node ids.
pub fn ungroup(graph: &mut Graph, ctx: &mut Ctx, instance: NodeId) -> Option<Vec<NodeId>> {
    let node = graph.node(instance)?.clone();
    let def_id = node.group_def()?;
    let def = ctx.defs.get(&def_id)?.clone();
    let inst_size = approx_size(&node, &ctx.sockets(graph, instance));
    let center = [
        node.ui.pos[0] + inst_size[0] / 2.0,
        node.ui.pos[1] + inst_size[1] / 2.0,
    ];

    // The definition's own interface as virtual wires, so its inner routing
    // (including input → output pass-throughs) maps like any other wire.
    let mut inner = def.graph.clone();
    add_virtual_wires(
        &mut inner,
        &Interface {
            inputs: def.inputs.clone(),
            outputs: def.outputs.clone(),
        },
    );
    // Center the inner nodes on the instance.
    let b = bbox(&inner, ctx, &inner.ids());
    let inner_center = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut new_ids = Vec::new();
    for n in &inner.nodes {
        let taken = graph.ids();
        let fresh = ctx
            .ids
            .fresh(|id| taken.contains(&id) || is_virtual(id) || map.values().any(|v| *v == id));
        map.insert(n.id, fresh);
        let mut copy = n.clone();
        copy.id = fresh;
        copy.ui.pos = [
            n.ui.pos[0] - inner_center[0] + center[0],
            n.ui.pos[1] - inner_center[1] + center[1],
        ];
        graph.nodes.push(copy);
        new_ids.push(fresh);
    }
    let remap = |e: &Endpoint| {
        map.get(&e.node)
            .map(|n| Endpoint::new(*n, e.socket.clone()))
    };

    // Instance input name → its outside source (if wired).
    let sources: BTreeMap<String, Endpoint> = graph
        .wires_to(instance)
        .map(|w| (w.to.socket.clone(), w.from.clone()))
        .collect();
    // Instance output name → outside targets.
    let mut targets: BTreeMap<String, Vec<Endpoint>> = BTreeMap::new();
    for w in graph.wires_from(instance) {
        targets
            .entry(w.from.socket.clone())
            .or_default()
            .push(w.to.clone());
    }
    let mut new_wires: Vec<Wire> = Vec::new();
    for w in &inner.wires {
        // Resolve the source: an inner node, or the outside source of an interface input.
        let from = if w.from.node == GROUP_IN {
            sources.get(&w.from.socket).cloned()
        } else {
            remap(&w.from)
        };
        let tos: Vec<Endpoint> = if w.to.node == GROUP_OUT {
            targets.get(&w.to.socket).cloned().unwrap_or_default()
        } else {
            remap(&w.to).into_iter().collect()
        };
        let Some(from) = from else {
            // Unwired interface input: carry the instance's value inward.
            if w.from.node == GROUP_IN {
                let value = node.params.get(&w.from.socket).cloned().or_else(|| {
                    def.inputs
                        .iter()
                        .find(|i| i.name == w.from.socket)
                        .and_then(|i| i.default.clone())
                });
                if let (Some(v), Some(t)) = (value, remap(&w.to))
                    && let Some(n) = graph.node_mut(t.node)
                {
                    n.params.insert(t.socket.clone(), v);
                }
            }
            continue;
        };
        for to in tos {
            new_wires.push(Wire::new(from.clone(), to));
        }
    }
    graph.remove_node(instance);
    for w in new_wires {
        graph.connect(w.from, w.to);
    }
    let empty = Graph::default();
    if !def_in_use(&empty, ctx.defs, graph, &def_id) {
        // The root graph isn't in `defs`; callers that edit inside a group
        // pass the root separately (see `Document`). Only drop the definition
        // when this is the root graph, where `graph` is the whole picture.
        if ctx.scope_def.is_none() {
            ctx.defs.remove(&def_id);
        }
    }
    Some(new_ids)
}

/// Add a `Voices` node with a starter body: a Voice node playing a saw
/// Oscillator through an ADSR-driven gain, so it sounds as soon as MIDI
/// reaches it. Returns the new node.
pub fn add_voices(graph: &mut Graph, ctx: &mut Ctx, pos: [f32; 2]) -> NodeId {
    let (def_id, _) = fresh_def_id(ctx.defs);
    let mut def = GroupDef::new("Voice");
    def.description = "One voice of a polyphonic synth: edit it to change every voice.".into();
    let n = |id: u64, kind: &str, x: f32, y: f32| NodeInstance::new(NodeId(id), kind).at(x, y);
    def.graph.nodes.extend([
        n(1, ac_core::VOICE_KIND, 0.0, 0.0),
        n(2, "oscillator", 240.0, -60.0).with_param("shape", "saw"),
        n(3, "adsr", 240.0, 140.0).with_param("release", 0.3),
        n(4, "math", 480.0, 40.0).with_param("op", "multiply"),
    ]);
    for (f, o, t, i) in [
        (1, "freq", 2, "freq"),
        (1, "gate", 3, "gate"),
        (2, "out", 4, "a"),
        (3, "out", 4, "b"),
    ] {
        def.graph
            .connect(Endpoint::new(NodeId(f), o), Endpoint::new(NodeId(t), i));
    }
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: ac_core::SignalType::Audio,
        source: Endpoint::new(NodeId(4), "out"),
    });
    ctx.defs.insert(def_id.clone(), def);
    let id = ctx.fresh_id(graph);
    graph.nodes.push(
        NodeInstance::new(id, ac_core::VOICES_KIND)
            .with_config(serde_json::json!({ "def": def_id.0, "voices": 8 }))
            .at(pos[0], pos[1]),
    );
    id
}

/// Switch a group instance between playing once (`group`) and once per
/// voice (`voices`), keeping its definition and wires.
pub fn set_polyphonic(graph: &mut Graph, id: NodeId, on: bool) {
    let Some(node) = graph.node_mut(id) else {
        return;
    };
    let Some(def) = node.group_def() else {
        return;
    };
    if on {
        node.kind = ac_core::VOICES_KIND.to_string();
        node.config = serde_json::json!({ "def": def.0, "voices": 8 });
    } else {
        node.kind = GROUP_KIND.to_string();
        node.config = serde_json::json!({ "def": def.0 });
        graph.disconnect_input(&Endpoint::new(id, "midi"));
    }
}
