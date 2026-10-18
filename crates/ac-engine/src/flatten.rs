//! Group flattening and reroute removal (ARCHITECTURE §7 step 1).
//!
//! Every group instance is expanded in place; inner nodes get paths prefixed
//! by the instance path so their state stays addressable. A `Voices` node
//! expands into its voice allocator, one copy of its body per voice (paths
//! `voices/<voice>/...`), and a summing node per output (DECISIONS D12).
//! Reroute dots are removed by connecting their source straight to their
//! targets. Wires are resolved to socket indices; anything that doesn't
//! resolve is reported and dropped.

use std::collections::{BTreeMap, HashMap, HashSet};

use ac_core::{
    Config, ConfigExt, Endpoint, GROUP_KIND, Graph, GroupDef, GroupId, Node, NodeId, NodeMeta,
    NodePath, PrepareCtx, Project, REROUTE_KIND, Registry, RenderWholeFn, SignalType, Signature,
    VOICE_KIND, VOICES_KIND, Value,
};
use serde_json::json;

use crate::internal::{self, VOICES_SUM_KIND, VoicesSum};
use crate::report::CompileReport;

/// Node ids of the summing nodes under a Voices node, one per body output
/// (the output index is added). Voice copies use ids `0..voices`.
const SUM_ID_BASE: u64 = 0x5_0000_0000_0000;
/// Most voices a Voices node runs (matches the node's own clamp).
const MAX_VOICES: u64 = 64;

/// Groups nested deeper than this are reported as an error (it's almost
/// certainly a mistake, and it bounds the work).
const MAX_DEPTH: usize = 32;

pub(crate) struct FlatNode {
    pub path: NodePath,
    pub kind: String,
    pub config: Config,
    pub params: BTreeMap<String, Value>,
    pub muted: bool,
    pub name: String,
    pub sig: Signature,
    pub meta: NodeMeta,
    pub create: fn(&Config, &PrepareCtx) -> Box<dyn Node>,
    pub render_whole: RenderWholeFn,
}

/// A wire between flat nodes, by node index and socket index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlatWire {
    pub from: (usize, usize),
    pub to: (usize, usize),
}

pub(crate) struct Flat {
    pub nodes: Vec<FlatNode>,
    pub wires: Vec<FlatWire>,
    /// Unwired inputs of group instances → the inner inputs they set.
    pub group_inputs: HashMap<(NodePath, String), Vec<(usize, usize)>>,
    /// Outputs of group instances → the inner outputs that feed them.
    pub group_outputs: HashMap<(NodePath, String), (usize, usize)>,
}

enum Item<'p> {
    Flat(usize),
    Group {
        scope: usize,
        def: &'p GroupDef,
    },
    /// A Voices node: its allocator, one scope per voice, and a summing node
    /// per body output (`None` for outputs that aren't summed).
    Voices {
        alloc: usize,
        scopes: Vec<usize>,
        def: &'p GroupDef,
        sums: Vec<Option<usize>>,
    },
    Reroute,
    Missing,
}

struct Scope<'p> {
    prefix: Option<NodePath>,
    graph: &'p Graph,
    items: HashMap<NodeId, Item<'p>>,
}

impl Scope<'_> {
    fn path(&self, id: NodeId) -> NodePath {
        match &self.prefix {
            Some(p) => p.child(id),
            None => NodePath::root(id),
        }
    }
}

struct Builder<'p> {
    project: &'p Project,
    registry: &'p Registry,
    scopes: Vec<Scope<'p>>,
    nodes: Vec<FlatNode>,
    /// Voice nodes and the allocator output that feeds each: (voice node,
    /// allocator, voice index).
    voice_links: Vec<(usize, usize, usize)>,
}

pub(crate) fn flatten(project: &Project, registry: &Registry, report: &mut CompileReport) -> Flat {
    let mut b = Builder {
        project,
        registry,
        scopes: Vec::new(),
        nodes: Vec::new(),
        voice_links: Vec::new(),
    };
    let mut stack = Vec::new();
    b.build_scope(&project.graph, None, &mut stack, None, report);

    let mut wires = Vec::new();
    for s in 0..b.scopes.len() {
        b.resolve_scope_wires(s, &mut wires, report);
    }
    b.voice_wires(&mut wires);

    // Group input params: deepest scopes first so outer instances win.
    let mut group_inputs = HashMap::new();
    let mut group_outputs = HashMap::new();
    for s in (0..b.scopes.len()).rev() {
        b.group_params(s, &wires, &mut group_inputs, &mut group_outputs, report);
    }

    Flat {
        nodes: b.nodes,
        wires,
        group_inputs,
        group_outputs,
    }
}

/// The voice copy each node sits in, numbered from 0: a copy of the
/// outermost Voices node around it (paths `<voices>/<voice>/…`, including
/// nodes the compiler adds under them later). Allocators, sums, and nodes
/// outside every Voices node have none. The scheduler runs each copy's body
/// as one chain (DECISIONS D113).
pub(crate) fn voice_copies(nodes: &[FlatNode]) -> Vec<Option<u32>> {
    let allocs: HashSet<&[NodeId]> = nodes
        .iter()
        .filter(|f| f.kind == VOICES_KIND)
        .map(|f| f.path.0.as_slice())
        .collect();
    let mut ids: HashMap<&[NodeId], u32> = HashMap::new();
    nodes
        .iter()
        .map(|f| {
            let p = f.path.0.as_slice();
            // `p[..k]` is the Voices node, `p[k]` the voice, then the body.
            let k = (1..p.len().saturating_sub(1))
                .find(|&k| p[k].0 < MAX_VOICES && allocs.contains(&p[..k]))?;
            let next = ids.len() as u32;
            Some(*ids.entry(&p[..=k]).or_insert(next))
        })
        .collect()
}

impl<'p> Builder<'p> {
    /// Build a scope. `voice` is the allocator and voice index of the
    /// Voices body this scope sits in, if any.
    fn build_scope(
        &mut self,
        graph: &'p Graph,
        prefix: Option<NodePath>,
        stack: &mut Vec<GroupId>,
        voice: Option<(usize, usize)>,
        report: &mut CompileReport,
    ) -> usize {
        let idx = self.scopes.len();
        self.scopes.push(Scope {
            prefix: prefix.clone(),
            graph,
            items: HashMap::new(),
        });
        for node in &graph.nodes {
            let path = match &prefix {
                Some(p) => p.child(node.id),
                None => NodePath::root(node.id),
            };
            let item = if node.kind == VOICES_KIND {
                self.build_voices(node, &path, stack, report)
            } else if node.kind == GROUP_KIND {
                match node.group_def() {
                    None => {
                        report.error(Some(&path), None, "group node has no `def` in its config");
                        Item::Missing
                    }
                    Some(def_id) => match self.project.group_defs.get(&def_id) {
                        None => {
                            report.error(
                                Some(&path),
                                None,
                                format!("group definition `{def_id}` is missing from the project"),
                            );
                            Item::Missing
                        }
                        Some(_) if stack.contains(&def_id) => {
                            report.error(
                                Some(&path),
                                None,
                                format!("group `{def_id}` contains itself"),
                            );
                            Item::Missing
                        }
                        Some(_) if stack.len() >= MAX_DEPTH => {
                            report.error(Some(&path), None, "groups are nested too deeply");
                            Item::Missing
                        }
                        Some(def) => {
                            stack.push(def_id);
                            let child = self.build_scope(
                                &def.graph,
                                Some(path.clone()),
                                stack,
                                voice,
                                report,
                            );
                            stack.pop();
                            Item::Group { scope: child, def }
                        }
                    },
                }
            } else if node.kind == REROUTE_KIND {
                Item::Reroute
            } else if let Some(entry) = self.registry.get(&node.kind) {
                let sig = (entry.describe)(&node.config);
                let name = node
                    .ui
                    .label
                    .clone()
                    .unwrap_or_else(|| entry.meta.name.to_string());
                let mut config = node.config.clone();
                if node.kind == VOICE_KIND {
                    match voice {
                        Some((alloc, v)) => {
                            if let Some(obj) = config.as_object_mut() {
                                obj.insert("voice_index".into(), v.into());
                            } else {
                                config = json!({ "voice_index": v });
                            }
                            self.voice_links.push((self.nodes.len(), alloc, v));
                        }
                        None => report.warning(
                            Some(&path),
                            None,
                            "a Voice node only works inside the body of a Voices node",
                        ),
                    }
                }
                self.nodes.push(FlatNode {
                    path,
                    kind: node.kind.clone(),
                    config,
                    params: node.params.clone(),
                    muted: node.muted,
                    name,
                    sig,
                    meta: entry.meta.clone(),
                    create: entry.create,
                    render_whole: entry.render_whole,
                });
                Item::Flat(self.nodes.len() - 1)
            } else {
                report.warning(
                    Some(&path),
                    None,
                    format!(
                        "unknown node kind `{}`; it is kept in the project but does nothing",
                        node.kind
                    ),
                );
                Item::Missing
            };
            self.scopes[idx].items.insert(node.id, item);
        }
        idx
    }

    /// Resolve an output endpoint to the flat output that produces it.
    fn resolve_out(&self, scope: usize, ep: &Endpoint, depth: usize) -> Option<(usize, usize)> {
        if depth > 256 {
            return None;
        }
        let sc = &self.scopes[scope];
        match sc.items.get(&ep.node)? {
            Item::Flat(i) => {
                let out = self.nodes[*i].sig.output_index(&ep.socket)?;
                Some((*i, out))
            }
            Item::Group { scope: child, def } => {
                let o = def.outputs.iter().find(|o| o.name == ep.socket)?;
                self.resolve_out(*child, &o.source, depth + 1)
            }
            Item::Voices {
                scopes, def, sums, ..
            } => {
                let k = def.outputs.iter().position(|o| o.name == ep.socket)?;
                match sums.get(k).copied().flatten() {
                    Some(sum) => Some((sum, 0)),
                    None => self.resolve_out(*scopes.first()?, &def.outputs[k].source, depth + 1),
                }
            }
            Item::Reroute => {
                let into = Endpoint::new(ep.node, "in");
                let w = sc.graph.wire_into(&into)?;
                self.resolve_out(scope, &w.from, depth + 1)
            }
            Item::Missing => None,
        }
    }

    /// Resolve an input endpoint to every flat input it feeds.
    fn resolve_in(&self, scope: usize, ep: &Endpoint, depth: usize, out: &mut Vec<(usize, usize)>) {
        if depth > 256 {
            return;
        }
        let sc = &self.scopes[scope];
        match sc.items.get(&ep.node) {
            Some(Item::Flat(i)) => {
                if let Some(inp) = self.nodes[*i].sig.input_index(&ep.socket) {
                    out.push((*i, inp));
                }
            }
            Some(Item::Group { scope: child, def }) => {
                if let Some(input) = def.inputs.iter().find(|x| x.name == ep.socket) {
                    for t in &input.targets {
                        self.resolve_in(*child, t, depth + 1, out);
                    }
                }
            }
            Some(Item::Voices {
                alloc, scopes, def, ..
            }) => {
                if ep.socket == "midi" {
                    out.push((*alloc, 0));
                } else if let Some(input) = def.inputs.iter().find(|x| x.name == ep.socket) {
                    for &child in scopes {
                        for t in &input.targets {
                            self.resolve_in(child, t, depth + 1, out);
                        }
                    }
                }
            }
            Some(Item::Reroute) => {
                for w in sc.graph.wires_from(ep.node) {
                    if w.from.socket == "out" {
                        self.resolve_in(scope, &w.to, depth + 1, out);
                    }
                }
            }
            Some(Item::Missing) | None => {}
        }
    }

    fn resolve_scope_wires(&self, s: usize, wires: &mut Vec<FlatWire>, report: &mut CompileReport) {
        let sc = &self.scopes[s];
        for w in &sc.graph.wires {
            // Wires into a reroute are represented by the reroute's outgoing wires.
            if matches!(sc.items.get(&w.to.node), Some(Item::Reroute)) {
                continue;
            }
            let to_path = sc.path(w.to.node);
            if !sc.items.contains_key(&w.from.node) || !sc.items.contains_key(&w.to.node) {
                report.warning(
                    Some(&to_path),
                    Some(&w.to.socket),
                    format!("wire from {} refers to a node that doesn't exist", w.from),
                );
                continue;
            }
            if matches!(sc.items.get(&w.from.node), Some(Item::Missing))
                || matches!(sc.items.get(&w.to.node), Some(Item::Missing))
            {
                continue; // already reported on the node
            }
            let Some(from) = self.resolve_out(s, &w.from, 0) else {
                if !matches!(sc.items.get(&w.from.node), Some(Item::Reroute)) {
                    report.warning(
                        Some(&sc.path(w.from.node)),
                        Some(&w.from.socket),
                        format!("output `{}` doesn't exist; wire ignored", w.from.socket),
                    );
                }
                continue;
            };
            let mut targets = Vec::new();
            self.resolve_in(s, &w.to, 0, &mut targets);
            if targets.is_empty() && matches!(sc.items.get(&w.to.node), Some(Item::Flat(_))) {
                report.warning(
                    Some(&to_path),
                    Some(&w.to.socket),
                    format!("input `{}` doesn't exist; wire ignored", w.to.socket),
                );
            }
            for to in targets {
                wires.push(FlatWire { from, to });
            }
        }
    }

    /// Expand a Voices node: allocator, voice scopes, summing nodes.
    fn build_voices(
        &mut self,
        node: &'p ac_core::NodeInstance,
        path: &NodePath,
        stack: &mut Vec<GroupId>,
        report: &mut CompileReport,
    ) -> Item<'p> {
        let Some(def_id) = node.group_def() else {
            report.error(
                Some(path),
                None,
                "Voices node has no `def` (body) in its config",
            );
            return Item::Missing;
        };
        let Some(def) = self.project.group_defs.get(&def_id) else {
            report.error(
                Some(path),
                None,
                format!("group definition `{def_id}` is missing from the project"),
            );
            return Item::Missing;
        };
        if stack.contains(&def_id) {
            report.error(
                Some(path),
                None,
                format!("group `{def_id}` contains itself"),
            );
            return Item::Missing;
        }
        if stack.len() >= MAX_DEPTH {
            report.error(Some(path), None, "groups are nested too deeply");
            return Item::Missing;
        }
        let Some(entry) = self.registry.get(VOICES_KIND) else {
            report.error(Some(path), None, "Voices isn't available in this build");
            return Item::Missing;
        };
        let voices = node.config.get_u64_or("voices", 8).clamp(1, MAX_VOICES) as usize;
        let name = node
            .ui
            .label
            .clone()
            .unwrap_or_else(|| entry.meta.name.to_string());
        let alloc = self.nodes.len();
        self.nodes.push(FlatNode {
            path: path.clone(),
            kind: VOICES_KIND.to_string(),
            config: node.config.clone(),
            params: node.params.clone(),
            muted: node.muted,
            name: name.clone(),
            sig: (entry.describe)(&node.config),
            meta: entry.meta.clone(),
            create: entry.create,
            render_whole: entry.render_whole,
        });
        stack.push(def_id);
        let scopes: Vec<usize> = (0..voices)
            .map(|v| {
                let prefix = path.child(NodeId(v as u64));
                self.build_scope(&def.graph, Some(prefix), stack, Some((alloc, v)), report)
            })
            .collect();
        stack.pop();
        let sums = def
            .outputs
            .iter()
            .enumerate()
            .map(|(k, o)| {
                let ty = match o.ty {
                    SignalType::Audio => "audio",
                    SignalType::Float => "float",
                    _ => return None,
                };
                let config = json!({ "voices": voices, "type": ty });
                self.nodes.push(FlatNode {
                    path: path.child(NodeId(SUM_ID_BASE + k as u64)),
                    kind: VOICES_SUM_KIND.to_string(),
                    sig: VoicesSum::describe(&config),
                    config,
                    params: BTreeMap::new(),
                    muted: false,
                    name: format!("{name} {}", o.name),
                    meta: VoicesSum::meta(),
                    create: internal::create::<VoicesSum>,
                    render_whole: VoicesSum::render_whole,
                });
                Some(self.nodes.len() - 1)
            })
            .collect();
        Item::Voices {
            alloc,
            scopes,
            def,
            sums,
        }
    }

    /// Wires inside Voices nodes: allocator → each copy's Voice nodes, and
    /// each copy's outputs → the summing nodes.
    fn voice_wires(&self, wires: &mut Vec<FlatWire>) {
        for &(voice, alloc, v) in &self.voice_links {
            if let Some(i) = self.nodes[voice].sig.input_index("voice_midi") {
                wires.push(FlatWire {
                    from: (alloc, v),
                    to: (voice, i),
                });
            }
        }
        for sc in &self.scopes {
            for item in sc.items.values() {
                let Item::Voices {
                    scopes, def, sums, ..
                } = item
                else {
                    continue;
                };
                for (k, sum) in sums.iter().enumerate() {
                    let Some(sum) = *sum else { continue };
                    for (v, &child) in scopes.iter().enumerate() {
                        if let Some(from) = self.resolve_out(child, &def.outputs[k].source, 0) {
                            wires.push(FlatWire { from, to: (sum, v) });
                        }
                    }
                }
            }
        }
    }

    fn group_params(
        &mut self,
        s: usize,
        wires: &[FlatWire],
        group_inputs: &mut HashMap<(NodePath, String), Vec<(usize, usize)>>,
        group_outputs: &mut HashMap<(NodePath, String), (usize, usize)>,
        _report: &mut CompileReport,
    ) {
        let graph = self.scopes[s].graph;
        for node in &graph.nodes {
            let (children, def): (Vec<usize>, &GroupDef) = match self.scopes[s].items.get(&node.id)
            {
                Some(Item::Group { scope, def }) => (vec![*scope], *def),
                Some(Item::Voices { scopes, def, .. }) => (scopes.clone(), *def),
                _ => continue,
            };
            let path = self.scopes[s].path(node.id);
            for input in &def.inputs {
                let mut targets = Vec::new();
                for &child in &children {
                    for t in &input.targets {
                        self.resolve_in(child, t, 0, &mut targets);
                    }
                }
                let wired = graph
                    .wire_into(&Endpoint::new(node.id, &input.name))
                    .is_some();
                if !wired {
                    let value = node
                        .params
                        .get(&input.name)
                        .cloned()
                        .or_else(|| input.default.clone());
                    if let Some(v) = value {
                        for &(n, i) in &targets {
                            // An inner input that is wired inside the group keeps its wire.
                            if wires.iter().any(|w| w.to == (n, i)) {
                                continue;
                            }
                            let name = self.nodes[n].sig.inputs[i].name.to_string();
                            self.nodes[n].params.insert(name, v.clone());
                        }
                    }
                }
                group_inputs.insert((path.clone(), input.name.clone()), targets);
            }
            for output in &def.outputs {
                let src = self.resolve_out(s, &Endpoint::new(node.id, &output.name), 0);
                if let Some(src) = src {
                    group_outputs.insert((path.clone(), output.name.clone()), src);
                }
            }
            if node.muted {
                let prefix = path.clone();
                for n in &mut self.nodes {
                    if n.path.0.starts_with(&prefix.0) {
                        n.muted = true;
                    }
                }
            }
        }
    }
}
