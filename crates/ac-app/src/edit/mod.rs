//! Pure editing operations on a project's graphs: no UI, no engine, so they
//! can be unit-tested directly.
//!
//! Inside a group definition, the definition's interface is presented as two
//! virtual nodes, [`GROUP_IN`] and [`GROUP_OUT`], and its targets/sources as
//! ordinary wires to and from them. Every operation then works the same at
//! the root and inside a group (deleting with reconnect, inserting on a
//! wire, grouping a selection that touches the interface); [`edit_scope`]
//! converts the virtual wires back into the definition's interface after.

pub mod align;
pub mod connect;
pub mod generate;
pub mod group;
pub mod layout;
pub mod pins;
pub mod portal;

use std::collections::{BTreeMap, BTreeSet};

use ac_core::{
    Category, Endpoint, GROUP_KIND, Graph, GroupDef, GroupId, GroupInput, GroupOutput, IdGen,
    NodeId, NodeInstance, Project, REROUTE_KIND, Registry, SignalType, Signature, SocketSpec,
    Subgraph, Value, Wire, conversion,
};

use crate::i18n::{tr, tr_args};
use crate::theme;

/// The group interface's input side, drawn as a node inside a definition.
pub const GROUP_IN: NodeId = NodeId(u64::MAX - 1);
/// The group interface's output side.
pub const GROUP_OUT: NodeId = NodeId(u64::MAX - 2);
/// The node id a disconnected group output's source points at. Generated ids
/// are never zero, so this never names a real node.
pub const UNCONNECTED: NodeId = NodeId(0);
/// Socket name of the empty "drop here to add" socket on the interface nodes.
pub const NEW_SOCKET: &str = "";

/// Key in a node's preserved extra fields that hides its unwired sockets.
pub const HIDE_UNWIRED: &str = "hide_unwired";

/// Key in a Plugin node's preserved extra fields that shows every knob on
/// its card, rather than the first [`PLUGIN_KNOBS_SHOWN`] and the wired
/// ones (DECISIONS D198).
pub const SHOW_ALL_KNOBS: &str = "show_all_knobs";

/// How many of a Plugin's knobs its card shows (with any wired ones)
/// before it counts the rest in a last row.
pub const PLUGIN_KNOBS_SHOWN: usize = 8;

/// How many knobs (parameter inputs) a Plugin node has.
pub fn knob_count(node: &NodeInstance) -> usize {
    ac_core::PluginDesc::from_config(&node.config).map_or(0, |d| d.params.len())
}

/// Whether a Plugin node's card shows every knob.
pub fn shows_all_knobs(node: &NodeInstance) -> bool {
    node.extra.get(SHOW_ALL_KNOBS).and_then(|v| v.as_bool()) == Some(true)
}

/// Show every knob on a Plugin node's card, or the first few and the
/// wired ones.
pub fn set_show_all_knobs(node: &mut NodeInstance, all: bool) {
    if all {
        node.extra.insert(SHOW_ALL_KNOBS.into(), true.into());
    } else {
        node.extra.remove(SHOW_ALL_KNOBS);
    }
}

pub fn is_virtual(id: NodeId) -> bool {
    id == GROUP_IN || id == GROUP_OUT
}

/// The interface of the definition being edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interface {
    pub inputs: Vec<GroupInput>,
    pub outputs: Vec<GroupOutput>,
}

/// What an operation needs besides the graph it edits.
pub struct Ctx<'a> {
    pub registry: &'a Registry,
    pub defs: &'a mut BTreeMap<GroupId, GroupDef>,
    pub ids: &'a mut IdGen,
    /// Set when editing inside a group definition.
    pub iface: Option<Interface>,
    /// The definition being edited, if any.
    pub scope_def: Option<GroupId>,
}

/// Run `f` on the graph at `scope` (the root graph for `None`). Inside a
/// definition, the interface appears as virtual wires to [`GROUP_IN`] /
/// [`GROUP_OUT`] while `f` runs and is written back afterwards. A missing
/// definition falls back to the root graph.
pub fn edit_scope<R>(
    project: &mut Project,
    scope: Option<&GroupId>,
    registry: &Registry,
    ids: &mut IdGen,
    f: impl FnOnce(&mut Graph, &mut Ctx) -> R,
) -> R {
    let Some(def_id) = scope.filter(|d| project.group_defs.contains_key(*d)) else {
        let mut ctx = Ctx {
            registry,
            defs: &mut project.group_defs,
            ids,
            iface: None,
            scope_def: None,
        };
        return f(&mut project.graph, &mut ctx);
    };
    let (mut graph, iface) = match project.group_defs.get_mut(def_id) {
        Some(def) => (
            std::mem::take(&mut def.graph),
            Interface {
                inputs: std::mem::take(&mut def.inputs),
                outputs: std::mem::take(&mut def.outputs),
            },
        ),
        None => (Graph::default(), Interface::default()),
    };
    add_virtual_wires(&mut graph, &iface);
    let mut ctx = Ctx {
        registry,
        defs: &mut project.group_defs,
        ids,
        iface: Some(iface),
        scope_def: Some(def_id.clone()),
    };
    let r = f(&mut graph, &mut ctx);
    let mut iface = ctx.iface.take().unwrap_or_default();
    take_virtual_wires(&mut graph, &mut iface);
    let def = project
        .group_defs
        .entry(def_id.clone())
        .or_insert_with(|| GroupDef::new("Group"));
    def.graph = graph;
    def.inputs = iface.inputs;
    def.outputs = iface.outputs;
    r
}

/// Present a definition's interface as wires (see the module docs).
pub fn add_virtual_wires(graph: &mut Graph, iface: &Interface) {
    for i in &iface.inputs {
        for t in &i.targets {
            graph.wires.push(Wire::new(
                Endpoint::new(GROUP_IN, i.name.clone()),
                t.clone(),
            ));
        }
    }
    for o in &iface.outputs {
        if o.source.node != UNCONNECTED {
            graph.wires.push(Wire::new(
                o.source.clone(),
                Endpoint::new(GROUP_OUT, o.name.clone()),
            ));
        }
    }
}

/// Move virtual wires back into the interface, removing them from the graph.
pub fn take_virtual_wires(graph: &mut Graph, iface: &mut Interface) {
    for i in &mut iface.inputs {
        i.targets.clear();
    }
    for o in &mut iface.outputs {
        o.source = Endpoint::new(UNCONNECTED, o.name.clone());
    }
    graph.wires.retain(|w| {
        if w.from.node == GROUP_IN {
            if let Some(i) = iface.inputs.iter_mut().find(|i| i.name == w.from.socket)
                && !i.targets.contains(&w.to)
            {
                i.targets.push(w.to.clone());
            }
            false
        } else if w.to.node == GROUP_OUT {
            if let Some(o) = iface.outputs.iter_mut().find(|o| o.name == w.to.socket) {
                o.source = w.from.clone();
            }
            false
        } else {
            true
        }
    });
}

// ---------------------------------------------------------------- sockets

/// One socket as the editor sees it: a spec when the node is known, and the
/// effective type (reroutes adopt their wire's type).
#[derive(Clone, Debug, PartialEq)]
pub struct SocketInfo {
    pub name: String,
    pub ty: SignalType,
    pub spec: Option<SocketSpec>,
}

/// What kind of thing a node is, for drawing and editing.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeClass {
    Known,
    Group(GroupId),
    /// Unknown kind or a group whose definition is missing.
    Missing(String),
    Reroute,
}

/// The socket lists of a node (or a virtual interface node).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sockets {
    pub inputs: Vec<SocketInfo>,
    pub outputs: Vec<SocketInfo>,
}

impl Sockets {
    pub fn input(&self, name: &str) -> Option<&SocketInfo> {
        self.inputs.iter().find(|s| s.name == name)
    }
    pub fn output(&self, name: &str) -> Option<&SocketInfo> {
        self.outputs.iter().find(|s| s.name == name)
    }
}

fn from_signature(sig: Signature) -> Sockets {
    let conv = |s: SocketSpec| SocketInfo {
        name: s.name.to_string(),
        ty: s.ty,
        spec: Some(s),
    };
    // Hidden sockets are wired by the compiler only.
    Sockets {
        inputs: sig
            .inputs
            .into_iter()
            .filter(|s| !s.hidden)
            .map(conv)
            .collect(),
        outputs: sig
            .outputs
            .into_iter()
            .filter(|s| !s.hidden)
            .map(conv)
            .collect(),
    }
}

/// Whether a node is known, a group, a reroute, or missing.
pub fn classify(
    node: &NodeInstance,
    registry: &Registry,
    defs: &BTreeMap<GroupId, GroupDef>,
) -> NodeClass {
    if node.kind == REROUTE_KIND {
        return NodeClass::Reroute;
    }
    if node.kind == GROUP_KIND || node.kind == ac_core::VOICES_KIND {
        return match node.group_def() {
            Some(d) if defs.contains_key(&d) => NodeClass::Group(d),
            Some(d) => NodeClass::Missing(tr_args(
                "error-group-definition-missing",
                &[("def", d.to_string().into())],
            )),
            None => NodeClass::Missing(tr("error-group-no-definition")),
        };
    }
    if registry.contains(&node.kind) {
        NodeClass::Known
    } else {
        NodeClass::Missing(tr_args(
            "error-unknown-kind",
            &[("kind", node.kind.as_str().into())],
        ))
    }
}

impl Ctx<'_> {
    /// Socket lists of a node in `graph`, including the virtual interface
    /// nodes. Missing nodes get sockets made up from their stored wires and
    /// params so they can still be drawn and their wires kept.
    pub fn sockets(&self, graph: &Graph, id: NodeId) -> Sockets {
        sockets_of(graph, id, self.registry, self.defs, self.iface.as_ref())
    }

    pub fn out_type(&self, graph: &Graph, ep: &Endpoint) -> Option<SignalType> {
        out_type(graph, ep, self.registry, self.defs, self.iface.as_ref(), 0)
    }

    pub fn in_type(&self, graph: &Graph, ep: &Endpoint) -> Option<SignalType> {
        self.sockets(graph, ep.node).input(&ep.socket).map(|s| s.ty)
    }

    fn fresh_id(&mut self, graph: &Graph) -> NodeId {
        let taken = graph.ids();
        self.ids.fresh(|id| taken.contains(&id) || is_virtual(id))
    }
}

/// See [`Ctx::sockets`].
pub fn sockets_of(
    graph: &Graph,
    id: NodeId,
    registry: &Registry,
    defs: &BTreeMap<GroupId, GroupDef>,
    iface: Option<&Interface>,
) -> Sockets {
    if id == GROUP_IN || id == GROUP_OUT {
        return interface_sockets(id, iface);
    }
    let Some(node) = graph.node(id) else {
        return Sockets::default();
    };
    match classify(node, registry, defs) {
        NodeClass::Reroute => {
            let ty = out_type(graph, &Endpoint::new(id, "out"), registry, defs, iface, 0)
                .unwrap_or(SignalType::Any);
            let mut s = from_signature(
                registry
                    .get(REROUTE_KIND)
                    .map(|e| (e.describe)(&node.config))
                    .unwrap_or_else(|| {
                        Signature::new()
                            .input(SocketSpec::any("in"))
                            .output(SocketSpec::any("out"))
                    }),
            );
            for x in s.inputs.iter_mut().chain(s.outputs.iter_mut()) {
                x.ty = ty;
            }
            s
        }
        NodeClass::Known | NodeClass::Group(_) => registry
            .signature_of(node, defs)
            .map(from_signature)
            .unwrap_or_default(),
        NodeClass::Missing(_) => {
            let mut s = Sockets::default();
            let mut ins: Vec<String> = Vec::new();
            for w in graph.wires_to(id) {
                if !ins.contains(&w.to.socket) {
                    ins.push(w.to.socket.clone());
                }
            }
            for k in node.params.keys() {
                if !ins.contains(k) {
                    ins.push(k.clone());
                }
            }
            let mut outs: Vec<String> = Vec::new();
            for w in graph.wires_from(id) {
                if !outs.contains(&w.from.socket) {
                    outs.push(w.from.socket.clone());
                }
            }
            s.inputs = ins
                .into_iter()
                .map(|name| SocketInfo {
                    name,
                    ty: SignalType::Any,
                    spec: None,
                })
                .collect();
            s.outputs = outs
                .into_iter()
                .map(|name| SocketInfo {
                    name,
                    ty: SignalType::Any,
                    spec: None,
                })
                .collect();
            s
        }
    }
}

fn interface_sockets(id: NodeId, iface: Option<&Interface>) -> Sockets {
    let mut s = Sockets::default();
    let Some(iface) = iface else {
        return s;
    };
    let new_socket = SocketInfo {
        name: NEW_SOCKET.to_string(),
        ty: SignalType::Any,
        spec: None,
    };
    if id == GROUP_IN {
        s.outputs = iface
            .inputs
            .iter()
            .map(|i| SocketInfo {
                name: i.name.clone(),
                ty: i.ty,
                spec: None,
            })
            .collect();
        s.outputs.push(new_socket);
    } else {
        s.inputs = iface
            .outputs
            .iter()
            .map(|o| SocketInfo {
                name: o.name.clone(),
                ty: o.ty,
                spec: None,
            })
            .collect();
        s.inputs.push(new_socket);
    }
    s
}

/// Type produced by an output endpoint. Reroutes report their upstream type.
pub fn out_type(
    graph: &Graph,
    ep: &Endpoint,
    registry: &Registry,
    defs: &BTreeMap<GroupId, GroupDef>,
    iface: Option<&Interface>,
    depth: usize,
) -> Option<SignalType> {
    if depth > 64 {
        return Some(SignalType::Any);
    }
    if ep.node == GROUP_IN {
        return iface?
            .inputs
            .iter()
            .find(|i| i.name == ep.socket)
            .map(|i| i.ty);
    }
    let node = graph.node(ep.node)?;
    if node.kind == REROUTE_KIND {
        return match graph.wire_into(&Endpoint::new(ep.node, "in")) {
            Some(w) => out_type(graph, &w.from, registry, defs, iface, depth + 1),
            None => Some(SignalType::Any),
        };
    }
    sockets_of(graph, ep.node, registry, defs, iface)
        .output(&ep.socket)
        .map(|s| s.ty)
}

/// Follow reroutes upstream from an endpoint to the output that really
/// produces the signal (for live values on wires).
pub fn resolve_source(graph: &Graph, ep: &Endpoint) -> Endpoint {
    let mut cur = ep.clone();
    for _ in 0..64 {
        let Some(node) = graph.node(cur.node) else {
            break;
        };
        if node.kind != REROUTE_KIND {
            break;
        }
        match graph.wire_into(&Endpoint::new(cur.node, "in")) {
            Some(w) => cur = w.from.clone(),
            None => break,
        }
    }
    cur
}

/// Can an output of type `from` feed an input of type `to`?
pub fn compatible(from: SignalType, to: SignalType) -> bool {
    conversion(from, to).is_some()
}

// ---------------------------------------------------------------- wiring

/// Why a wire couldn't be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    NoSuchOutput,
    NoSuchInput,
    SameNode,
    Incompatible(SignalType, SignalType),
    MissingNode,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::NoSuchOutput => f.write_str(&tr("error-no-such-output")),
            ConnectError::NoSuchInput => f.write_str(&tr("error-no-such-input")),
            ConnectError::SameNode => f.write_str(&tr("error-same-node")),
            ConnectError::Incompatible(a, b) => f.write_str(&tr_args(
                "error-incompatible",
                &[("from", a.to_string().into()), ("to", b.to_string().into())],
            )),
            ConnectError::MissingNode => f.write_str(&tr("error-missing-node")),
        }
    }
}

/// Wire `from` (an output) to `to` (an input), replacing any wire already
/// feeding `to`. Checks types with the implicit conversion rules. Dropping
/// onto an interface node's empty socket adds a new interface socket.
pub fn connect(
    graph: &mut Graph,
    ctx: &mut Ctx,
    from: Endpoint,
    to: Endpoint,
) -> Result<(), ConnectError> {
    if from.node == to.node {
        return Err(ConnectError::SameNode);
    }
    let missing = |id: NodeId, graph: &Graph, ctx: &Ctx| {
        graph
            .node(id)
            .is_some_and(|n| matches!(classify(n, ctx.registry, ctx.defs), NodeClass::Missing(_)))
    };
    if missing(from.node, graph, ctx) || missing(to.node, graph, ctx) {
        return Err(ConnectError::MissingNode);
    }
    // New interface sockets: name and type come from the other end.
    let (from, to) = if from.node == GROUP_IN && from.socket == NEW_SOCKET {
        let name = new_interface_input(graph, ctx, &to).ok_or(ConnectError::NoSuchInput)?;
        (Endpoint::new(GROUP_IN, name), to)
    } else if to.node == GROUP_OUT && to.socket == NEW_SOCKET {
        let name = new_interface_output(graph, ctx, &from).ok_or(ConnectError::NoSuchOutput)?;
        (from, Endpoint::new(GROUP_OUT, name))
    } else {
        (from, to)
    };
    let out_ty = ctx
        .out_type(graph, &from)
        .ok_or(ConnectError::NoSuchOutput)?;
    let in_ty = ctx.in_type(graph, &to).ok_or(ConnectError::NoSuchInput)?;
    if !compatible(out_ty, in_ty) {
        return Err(ConnectError::Incompatible(out_ty, in_ty));
    }
    graph.connect(from, to);
    Ok(())
}

/// Remove the wire feeding an input.
pub fn disconnect(graph: &mut Graph, to: &Endpoint) -> bool {
    let before = graph.wires.len();
    graph.disconnect_input(to);
    graph.wires.len() != before
}

/// Unique snake_case name among `taken`, from `base`.
pub fn unique_name(base: &str, taken: &[&str]) -> String {
    let base = if base.is_empty() { "in" } else { base };
    if !taken.contains(&base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken.contains(&c.as_str()))
        .unwrap_or_else(|| base.to_string())
}

/// Add an interface input shaped like the inner input `target`. Returns its name.
pub fn new_interface_input(graph: &Graph, ctx: &mut Ctx, target: &Endpoint) -> Option<String> {
    let info = ctx
        .sockets(graph, target.node)
        .input(&target.socket)?
        .clone();
    let iface = ctx.iface.as_mut()?;
    let taken: Vec<&str> = iface.inputs.iter().map(|i| i.name.as_str()).collect();
    let name = unique_name(&info.name, &taken);
    let stored = graph
        .node(target.node)
        .and_then(|n| n.params.get(&target.socket))
        .cloned();
    let (unit, default, range) = match &info.spec {
        Some(s) => (
            s.unit,
            stored.or_else(|| Some(s.default.clone())),
            s.range.map(|(a, b)| [a, b]),
        ),
        None => (ac_core::Unit::Generic, stored, None),
    };
    iface.inputs.push(GroupInput {
        name: name.clone(),
        ty: if info.ty == SignalType::Any {
            SignalType::Float
        } else {
            info.ty
        },
        unit,
        default: default.filter(|v| !matches!(v, Value::Text(t) if t.is_empty())),
        range,
        targets: Vec::new(),
    });
    Some(name)
}

/// Add an interface output shaped like the inner output `source`.
pub fn new_interface_output(graph: &Graph, ctx: &mut Ctx, source: &Endpoint) -> Option<String> {
    let ty = ctx.out_type(graph, source)?;
    let iface = ctx.iface.as_mut()?;
    let taken: Vec<&str> = iface.outputs.iter().map(|o| o.name.as_str()).collect();
    let name = unique_name(&source.socket, &taken);
    iface.outputs.push(GroupOutput {
        name: name.clone(),
        ty: if ty == SignalType::Any {
            SignalType::Audio
        } else {
            ty
        },
        source: Endpoint::new(UNCONNECTED, name.clone()),
    });
    Some(name)
}

/// "Expose as group input": a new interface input feeding this inner input.
pub fn expose_input(graph: &mut Graph, ctx: &mut Ctx, target: &Endpoint) -> Option<String> {
    ctx.iface.as_ref()?;
    let name = new_interface_input(graph, ctx, target)?;
    graph.connect(Endpoint::new(GROUP_IN, name.clone()), target.clone());
    Some(name)
}

/// "Expose as group output": a new interface output fed by this inner output.
pub fn expose_output(graph: &mut Graph, ctx: &mut Ctx, source: &Endpoint) -> Option<String> {
    ctx.iface.as_ref()?;
    let name = new_interface_output(graph, ctx, source)?;
    graph.connect(source.clone(), Endpoint::new(GROUP_OUT, name.clone()));
    Some(name)
}

/// Remove an interface socket (and its virtual wires).
pub fn remove_interface_socket(graph: &mut Graph, ctx: &mut Ctx, input_side: bool, name: &str) {
    let Some(iface) = ctx.iface.as_mut() else {
        return;
    };
    if input_side {
        iface.inputs.retain(|i| i.name != name);
        graph
            .wires
            .retain(|w| !(w.from.node == GROUP_IN && w.from.socket == name));
    } else {
        iface.outputs.retain(|o| o.name != name);
        graph
            .wires
            .retain(|w| !(w.to.node == GROUP_OUT && w.to.socket == name));
    }
}

/// Rename an interface socket, keeping its wires.
pub fn rename_interface_socket(
    graph: &mut Graph,
    ctx: &mut Ctx,
    input_side: bool,
    old: &str,
    new: &str,
) -> Option<String> {
    let iface = ctx.iface.as_mut()?;
    let clean: String = new
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if clean.is_empty() {
        return None;
    }
    let name = if input_side {
        let taken: Vec<&str> = iface
            .inputs
            .iter()
            .filter(|i| i.name != old)
            .map(|i| i.name.as_str())
            .collect();
        let name = unique_name(&clean, &taken);
        iface.inputs.iter_mut().find(|i| i.name == old)?.name = name.clone();
        name
    } else {
        let taken: Vec<&str> = iface
            .outputs
            .iter()
            .filter(|o| o.name != old)
            .map(|o| o.name.as_str())
            .collect();
        let name = unique_name(&clean, &taken);
        iface.outputs.iter_mut().find(|o| o.name == old)?.name = name.clone();
        name
    };
    for w in &mut graph.wires {
        if input_side && w.from.node == GROUP_IN && w.from.socket == old {
            w.from.socket = name.clone();
        }
        if !input_side && w.to.node == GROUP_OUT && w.to.socket == old {
            w.to.socket = name.clone();
        }
    }
    Some(name)
}

// ---------------------------------------------------------------- nodes

/// Add a node of `kind` at canvas position `pos` (its top-left corner).
pub fn add_node(
    graph: &mut Graph,
    ctx: &mut Ctx,
    kind: &str,
    config: ac_core::Config,
    pos: [f32; 2],
) -> NodeId {
    let id = ctx.fresh_id(graph);
    graph.nodes.push(
        NodeInstance::new(id, kind)
            .with_config(config)
            .at(pos[0], pos[1]),
    );
    id
}

/// Delete nodes and every wire touching them. Virtual nodes are ignored.
pub fn delete_nodes(graph: &mut Graph, ids: &BTreeSet<NodeId>) -> Vec<NodeInstance> {
    let mut removed = Vec::new();
    for id in ids {
        if let Some(n) = graph.remove_node(*id) {
            removed.push(n);
        }
    }
    graph
        .wires
        .retain(|w| !ids.contains(&w.from.node) && !ids.contains(&w.to.node));
    removed
}

/// Delete nodes but keep the chain: each wire that left a deleted node is
/// fed instead by whatever fed that node, choosing for each outgoing wire
/// the first incoming wire whose type fits (an exact type match first).
pub fn delete_reconnect(
    graph: &mut Graph,
    ctx: &mut Ctx,
    ids: &BTreeSet<NodeId>,
) -> Vec<NodeInstance> {
    let mut removed = Vec::new();
    // One node at a time, so a selected chain collapses into one wire.
    let order: Vec<NodeId> = graph
        .nodes
        .iter()
        .map(|n| n.id)
        .filter(|id| ids.contains(id))
        .collect();
    for id in order {
        let sockets = ctx.sockets(graph, id);
        let mut incoming: Vec<Wire> = graph.wires_to(id).cloned().collect();
        incoming.sort_by_key(|w| {
            sockets
                .inputs
                .iter()
                .position(|s| s.name == w.to.socket)
                .unwrap_or(usize::MAX)
        });
        let outgoing: Vec<Wire> = graph.wires_from(id).cloned().collect();
        let mut new_wires = Vec::new();
        for out in &outgoing {
            let Some(target_ty) = ctx.in_type(graph, &out.to) else {
                continue;
            };
            let out_ty = sockets
                .output(&out.from.socket)
                .map(|s| s.ty)
                .unwrap_or(SignalType::Any);
            let typed: Vec<(Wire, SignalType)> = incoming
                .iter()
                .filter_map(|w| ctx.out_type(graph, &w.from).map(|t| (w.clone(), t)))
                .collect();
            let pick = typed
                .iter()
                .find(|(_, t)| *t == out_ty && compatible(*t, target_ty))
                .or_else(|| typed.iter().find(|(_, t)| compatible(*t, target_ty)));
            if let Some((w, _)) = pick
                && w.from.node != out.to.node
            {
                new_wires.push(Wire::new(w.from.clone(), out.to.clone()));
            }
        }
        if let Some(n) = graph.remove_node(id) {
            removed.push(n);
        }
        for w in new_wires {
            graph.connect(w.from, w.to);
        }
    }
    removed
}

/// Duplicate nodes with the wires between them. With `keep_incoming`, wires
/// from outside the selection into it are duplicated too. Returns the new
/// ids, in graph order.
pub fn duplicate(
    graph: &mut Graph,
    ctx: &mut Ctx,
    ids: &BTreeSet<NodeId>,
    keep_incoming: bool,
    offset: [f32; 2],
) -> Vec<NodeId> {
    let ids: BTreeSet<NodeId> = ids.iter().copied().filter(|i| !is_virtual(*i)).collect();
    let sub = Subgraph::copy(graph, ctx.defs, &ids);
    let old: Vec<NodeId> = sub.nodes.iter().map(|n| n.id).collect();
    let new = sub.paste(graph, ctx.defs, ctx.ids, offset);
    portal::after_paste(graph, &new);
    if keep_incoming {
        let map: BTreeMap<NodeId, NodeId> = old.iter().copied().zip(new.iter().copied()).collect();
        let incoming: Vec<Wire> = graph
            .wires
            .iter()
            .filter(|w| ids.contains(&w.to.node) && !ids.contains(&w.from.node))
            .filter(|w| !map.values().any(|n| *n == w.from.node))
            .cloned()
            .collect();
        for w in incoming {
            if let Some(&to) = map.get(&w.to.node) {
                graph.connect(w.from, Endpoint::new(to, w.to.socket));
            }
        }
    }
    new
}

/// Splice `node` into `wire` (from → node → to) if a socket pair fits.
/// Returns whether it was inserted.
pub fn insert_on_wire(graph: &mut Graph, ctx: &mut Ctx, node: NodeId, wire: &Wire) -> bool {
    if wire.from.node == node || wire.to.node == node || !graph.wires.contains(wire) {
        return false;
    }
    let Some(src_ty) = ctx.out_type(graph, &wire.from) else {
        return false;
    };
    let Some(dst_ty) = ctx.in_type(graph, &wire.to) else {
        return false;
    };
    // Main signal sockets come first by convention, so the first one that
    // fits is the one to splice through.
    let s = ctx.sockets(graph, node);
    let pick_in = s.inputs.iter().find(|i| compatible(src_ty, i.ty));
    let pick_out = s.outputs.iter().find(|o| compatible(o.ty, dst_ty));
    let (Some(i), Some(o)) = (pick_in, pick_out) else {
        return false;
    };
    let (i, o) = (i.name.clone(), o.name.clone());
    graph.wires.retain(|w| w != wire);
    graph.connect(wire.from.clone(), Endpoint::new(node, i));
    graph.connect(Endpoint::new(node, o), wire.to.clone());
    true
}

/// A wire and the canvas point where a gesture crossed it.
pub type Crossing = (Wire, [f32; 2]);

/// Insert a Reroute on each crossed wire, one per distinct source: wires
/// from the same output share a reroute placed at their mean crossing point
/// (`pos` is the reroute's center). Returns the new reroutes.
pub fn add_reroutes(graph: &mut Graph, ctx: &mut Ctx, crossings: &[Crossing]) -> Vec<NodeId> {
    let mut by_source: Vec<(Endpoint, Vec<Crossing>)> = Vec::new();
    for (w, p) in crossings {
        if !graph.wires.contains(w) {
            continue;
        }
        match by_source.iter_mut().find(|(s, _)| *s == w.from) {
            Some((_, v)) => v.push((w.clone(), *p)),
            None => by_source.push((w.from.clone(), vec![(w.clone(), *p)])),
        }
    }
    let mut out = Vec::new();
    for (source, wires) in by_source {
        let n = wires.len() as f32;
        let cx = wires.iter().map(|(_, p)| p[0]).sum::<f32>() / n;
        let cy = wires.iter().map(|(_, p)| p[1]).sum::<f32>() / n;
        let id = add_node(graph, ctx, REROUTE_KIND, ac_core::empty_config(), [cx, cy]);
        for (w, _) in &wires {
            graph.wires.retain(|x| x != w);
            graph.connect(Endpoint::new(id, "out"), w.to.clone());
        }
        graph.connect(source, Endpoint::new(id, "in"));
        out.push(id);
    }
    out
}

/// Nodes wired to any of `ids`, transitively, in both directions.
pub fn linked(graph: &Graph, ids: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    let mut set = ids.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for w in &graph.wires {
            let (a, b) = (w.from.node, w.to.node);
            if is_virtual(a) || is_virtual(b) {
                continue;
            }
            if set.contains(&a) && set.insert(b) {
                changed = true;
            }
            if set.contains(&b) && set.insert(a) {
                changed = true;
            }
        }
    }
    set
}

/// Paste clipboard text at `at` (the pasted nodes' top-left). Refuses text
/// that isn't a subgraph, or one that would put a group inside itself.
pub fn paste(
    graph: &mut Graph,
    ctx: &mut Ctx,
    text: &str,
    at: [f32; 2],
) -> Result<Vec<NodeId>, String> {
    let sub = Subgraph::from_json(text).ok_or_else(|| tr("error-clipboard-not-nodes"))?;
    if let Some(scope) = &ctx.scope_def {
        let mut all = ctx.defs.clone();
        for (k, v) in &sub.group_defs {
            all.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for n in &sub.nodes {
            if let Some(d) = n.group_def()
                && (d == *scope || group::def_contains(&all, &d, scope))
            {
                return Err(tr("error-group-inside-itself"));
            }
        }
    }
    let (minx, miny) = sub.nodes.iter().fold((f32::MAX, f32::MAX), |(x, y), n| {
        (x.min(n.ui.pos[0]), y.min(n.ui.pos[1]))
    });
    let offset = if sub.nodes.is_empty() {
        [0.0, 0.0]
    } else {
        [at[0] - minx, at[1] - miny]
    };
    let new = sub.paste(graph, ctx.defs, ctx.ids, offset);
    portal::after_paste(graph, &new);
    Ok(new)
}

/// Copy the selection as clipboard text.
pub fn copy(graph: &Graph, defs: &BTreeMap<GroupId, GroupDef>, ids: &BTreeSet<NodeId>) -> String {
    Subgraph::copy(graph, defs, ids).to_json()
}

/// Approximate size of a node card (canvas units), for operations that need
/// a node's extent without the canvas (group placement).
pub fn approx_size(node: &NodeInstance, sockets: &Sockets) -> [f32; 2] {
    if let Some(s) = node.ui.size {
        return s;
    }
    if node.kind == REROUTE_KIND {
        return [theme::REROUTE_RADIUS * 2.0, theme::REROUTE_RADIUS * 2.0];
    }
    if node.ui.collapsed {
        return [theme::NODE_WIDTH * 0.7, theme::PILL_HEIGHT];
    }
    let rows = (sockets.inputs.len() + sockets.outputs.len()) as f32;
    [
        theme::NODE_WIDTH,
        theme::HEADER_HEIGHT + rows * theme::ROW_HEIGHT + theme::NODE_PAD_BOTTOM,
    ]
}

/// Category of a node kind, if known.
pub fn category_of(registry: &Registry, kind: &str) -> Option<Category> {
    registry.get(kind).map(|e| e.meta.category)
}

#[cfg(test)]
pub(crate) mod tests;
