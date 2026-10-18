//! Group flattening and reroute removal (ARCHITECTURE §7 step 1).
//!
//! Every group instance is expanded in place; inner nodes get paths prefixed
//! by the instance path so their state stays addressable. Reroute dots are
//! removed by connecting their source straight to their targets. Wires are
//! resolved to socket indices; anything that doesn't resolve is reported and
//! dropped.

use std::collections::{BTreeMap, HashMap};

use ac_core::{
    Config, Endpoint, GROUP_KIND, Graph, GroupDef, GroupId, Node, NodeId, NodeMeta, NodePath,
    PrepareCtx, Project, REROUTE_KIND, Registry, RenderWholeFn, Signature, Value,
};

use crate::report::CompileReport;

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
    Group { scope: usize, def: &'p GroupDef },
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
}

pub(crate) fn flatten(project: &Project, registry: &Registry, report: &mut CompileReport) -> Flat {
    let mut b = Builder {
        project,
        registry,
        scopes: Vec::new(),
        nodes: Vec::new(),
    };
    let mut stack = Vec::new();
    b.build_scope(&project.graph, None, &mut stack, report);

    let mut wires = Vec::new();
    for s in 0..b.scopes.len() {
        b.resolve_scope_wires(s, &mut wires, report);
    }

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

impl<'p> Builder<'p> {
    fn build_scope(
        &mut self,
        graph: &'p Graph,
        prefix: Option<NodePath>,
        stack: &mut Vec<GroupId>,
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
            let item = if node.kind == GROUP_KIND {
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
                            let child =
                                self.build_scope(&def.graph, Some(path.clone()), stack, report);
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
                self.nodes.push(FlatNode {
                    path,
                    kind: node.kind.clone(),
                    config: node.config.clone(),
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
            let Some(Item::Group { scope: child, def }) = self.scopes[s].items.get(&node.id) else {
                continue;
            };
            let (child, def) = (*child, *def);
            let path = self.scopes[s].path(node.id);
            for input in &def.inputs {
                let mut targets = Vec::new();
                for t in &input.targets {
                    self.resolve_in(child, t, 0, &mut targets);
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
                if let Some(src) = self.resolve_out(child, &output.source, 0) {
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
