//! The open document: the project, where it lives, which group the editor is
//! inside, the selection, and the undo history.

use std::collections::BTreeSet;
use std::path::PathBuf;

use ac_core::{Graph, GroupDef, GroupId, IdGen, NodeId, NodePath, Project, Registry, Value};

use crate::edit::{self, Ctx, Interface};
use crate::undo::{History, Snapshot};

/// One level of group nesting: the definition being edited and the instance
/// it was entered through (telemetry inside a group is read through that
/// instance's path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeEntry {
    pub def: GroupId,
    pub instance: NodeId,
}

pub struct Document {
    pub project: Project,
    /// Where the project was loaded from or last saved to.
    pub path: Option<PathBuf>,
    /// Directory relative asset paths resolve against: the project's folder,
    /// or the examples folder for an unsaved copy of an example.
    pub base_dir: PathBuf,
    /// Group scopes from the root down; empty at the root.
    pub scope: Vec<ScopeEntry>,
    pub selection: BTreeSet<NodeId>,
    pub ids: IdGen,
    /// Bumped on every change.
    pub revision: u64,
    saved_revision: u64,
    /// Bumped by changes the engine must recompile for (everything except
    /// moving nodes, view changes, and parameter values sent directly).
    pub engine_revision: u64,
    history: History,
}

impl Document {
    pub fn new(project: Project, path: Option<PathBuf>, base_dir: PathBuf, seed: u64) -> Document {
        Document {
            project,
            path,
            base_dir,
            scope: Vec::new(),
            selection: BTreeSet::new(),
            ids: IdGen::new(seed),
            revision: 0,
            saved_revision: 0,
            engine_revision: 0,
            history: History::default(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// Display name: the project name, else the file name, else "Untitled".
    pub fn title(&self) -> String {
        if !self.project.name.trim().is_empty() {
            return self.project.name.clone();
        }
        self.path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    // ------------------------------------------------------------ scope

    /// The definition being edited, if inside a group.
    pub fn current_def(&self) -> Option<&GroupId> {
        self.scope.last().map(|s| &s.def)
    }

    /// The graph the canvas shows.
    pub fn graph(&self) -> &Graph {
        match self
            .current_def()
            .and_then(|d| self.project.group_defs.get(d))
        {
            Some(def) => &def.graph,
            None => &self.project.graph,
        }
    }

    pub fn current_group(&self) -> Option<&GroupDef> {
        self.current_def()
            .and_then(|d| self.project.group_defs.get(d))
    }

    /// The interface of the definition being edited.
    pub fn interface(&self) -> Option<Interface> {
        self.current_group().map(|d| Interface {
            inputs: d.inputs.clone(),
            outputs: d.outputs.clone(),
        })
    }

    /// The current graph with the interface's virtual wires (for drawing).
    pub fn display_graph(&self) -> Graph {
        let mut g = self.graph().clone();
        if let Some(iface) = self.interface() {
            edit::add_virtual_wires(&mut g, &iface);
        }
        g
    }

    /// Instance ids from the root to the current scope.
    pub fn scope_prefix(&self) -> Vec<NodeId> {
        self.scope.iter().map(|s| s.instance).collect()
    }

    /// The engine path of a node in the current scope, through the instance
    /// the scope was entered by.
    pub fn node_path(&self, id: NodeId) -> NodePath {
        let mut v = self.scope_prefix();
        v.push(id);
        NodePath(v)
    }

    /// Every engine path of a node in the current scope: one per instance of
    /// the definition being edited (editing a definition affects them all).
    pub fn all_node_paths(&self, id: NodeId) -> Vec<NodePath> {
        match self.current_def() {
            None => vec![NodePath::root(id)],
            Some(def) => edit::group::instance_paths(&self.project, def)
                .into_iter()
                .map(|p| p.child(id))
                .collect(),
        }
    }

    /// Enter a group instance in the current graph. False if it isn't one or
    /// its definition is missing.
    pub fn enter_group(&mut self, instance: NodeId) -> bool {
        let Some(def) = self.graph().node(instance).and_then(|n| n.group_def()) else {
            return false;
        };
        if !self.project.group_defs.contains_key(&def) {
            return false;
        }
        self.scope.push(ScopeEntry { def, instance });
        self.selection.clear();
        true
    }

    /// Go up one level, selecting the instance we came out of.
    pub fn leave_group(&mut self) -> bool {
        let Some(e) = self.scope.pop() else {
            return false;
        };
        self.selection = [e.instance].into();
        true
    }

    /// Drop scope levels whose definition or instance no longer exists
    /// (after an undo, an ungroup, or loading).
    pub fn fix_scope(&mut self) {
        let mut valid = 0;
        let mut graph = &self.project.graph;
        for e in &self.scope {
            let ok = graph
                .node(e.instance)
                .and_then(|n| n.group_def())
                .is_some_and(|d| d == e.def);
            let Some(def) = self.project.group_defs.get(&e.def).filter(|_| ok) else {
                break;
            };
            graph = &def.graph;
            valid += 1;
        }
        self.scope.truncate(valid);
        let ids = self.graph().ids();
        self.selection
            .retain(|id| ids.contains(id) || edit::is_virtual(*id));
    }

    /// Set the scope so that `path` (an engine path) is visible, and select
    /// it. Returns the node's id in that scope.
    pub fn navigate_to(&mut self, path: &NodePath) -> Option<NodeId> {
        let (last, parents) = path.0.split_last()?;
        let mut scope = Vec::new();
        let mut graph = &self.project.graph;
        for id in parents {
            let def = graph.node(*id)?.group_def()?;
            let d = self.project.group_defs.get(&def)?;
            scope.push(ScopeEntry { def, instance: *id });
            graph = &d.graph;
        }
        graph.node(*last)?;
        self.scope = scope;
        self.selection = [*last].into();
        Some(*last)
    }

    /// Breadcrumb labels from the root to the current scope.
    pub fn breadcrumb(&self) -> Vec<String> {
        let mut out = vec![self.title()];
        for e in &self.scope {
            let name = self
                .project
                .group_defs
                .get(&e.def)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| e.def.0.clone());
            out.push(name);
        }
        out
    }

    // ------------------------------------------------------------ editing

    fn snapshot(&self, label: &str) -> Snapshot {
        Snapshot {
            label: label.to_string(),
            project: self.project.clone(),
            scope: self.scope.clone(),
            selection: self.selection.clone(),
            revision: self.revision,
        }
    }

    /// Record an undo step before a change.
    pub fn checkpoint(&mut self, label: &str) {
        let s = self.snapshot(label);
        self.history.push(s, None, 0.0);
    }

    /// Record an undo step unless the previous one had the same `key` and
    /// was recent: a parameter drag becomes one step.
    pub fn checkpoint_coalesced(&mut self, label: &str, key: &str, now: f64) {
        let s = self.snapshot(label);
        self.history.push(s, Some(key), now);
    }

    /// End coalescing (a drag ended), so the next change is a new step.
    pub fn end_coalescing(&mut self) {
        self.history.end_coalescing();
    }

    /// Note that the document changed. `engine`: the engine must recompile.
    pub fn changed(&mut self, engine: bool) {
        self.revision += 1;
        if engine {
            self.engine_revision += 1;
        }
    }

    /// Apply an edit to the current graph as one undo step. If nothing
    /// changed, no step is recorded.
    pub fn edit<R>(
        &mut self,
        registry: &Registry,
        label: &str,
        f: impl FnOnce(&mut Graph, &mut Ctx) -> R,
    ) -> R {
        let before = self.snapshot(label);
        let scope = self.current_def().cloned();
        let r = edit::edit_scope(
            &mut self.project,
            scope.as_ref(),
            registry,
            &mut self.ids,
            f,
        );
        edit::group::prune_instance_wires(&mut self.project);
        if self.project != before.project {
            self.history.push(before, None, 0.0);
            self.changed(true);
        }
        self.fix_scope();
        r
    }

    /// Apply a change to the whole project (settings, defs) as one undo step.
    pub fn edit_project<R>(&mut self, label: &str, f: impl FnOnce(&mut Project) -> R) -> R {
        let before = self.snapshot(label);
        let r = f(&mut self.project);
        if self.project != before.project {
            self.history.push(before, None, 0.0);
            self.changed(true);
        }
        self.fix_scope();
        r
    }

    /// Move nodes (no recompile). Coalesced per gesture by the caller.
    pub fn move_nodes(&mut self, moves: &[(NodeId, [f32; 2])]) {
        let graph = self.graph_mut_raw();
        for (id, pos) in moves {
            if let Some(n) = graph.node_mut(*id) {
                n.ui.pos = *pos;
            }
        }
        self.changed(false);
    }

    /// The current graph, mutably, without the interface's virtual wires.
    /// For changes that don't touch wiring (positions, labels, params).
    pub fn graph_mut_raw(&mut self) -> &mut Graph {
        let def = self.current_def().cloned();
        match def.and_then(|d| self.project.group_defs.get_mut(&d)) {
            Some(def) => &mut def.graph,
            None => &mut self.project.graph,
        }
    }

    /// Store an unwired input's value, as an undo step coalesced with the
    /// previous change of the same socket. Returns whether it changed.
    pub fn set_param(&mut self, id: NodeId, socket: &str, value: Value, now: f64) -> bool {
        let same = self
            .graph()
            .node(id)
            .and_then(|n| n.params.get(socket))
            .is_some_and(|v| *v == value);
        if same {
            return false;
        }
        let key = format!("param:{:?}:{id}:{socket}", self.current_def());
        self.checkpoint_coalesced("Change value", &key, now);
        if let Some(n) = self.graph_mut_raw().node_mut(id) {
            n.params.insert(socket.to_string(), value);
        }
        self.changed(false);
        true
    }

    // ------------------------------------------------------------ history

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    pub fn undo(&mut self) -> bool {
        let current = self.snapshot("");
        let Some(s) = self.history.undo(current) else {
            return false;
        };
        self.restore(s);
        true
    }

    pub fn redo(&mut self) -> bool {
        let current = self.snapshot("");
        let Some(s) = self.history.redo(current) else {
            return false;
        };
        self.restore(s);
        true
    }

    fn restore(&mut self, s: Snapshot) {
        self.project = s.project;
        self.scope = s.scope;
        self.selection = s.selection;
        // Undoing back to the saved state clears the dirty marker.
        self.revision = s.revision;
        self.engine_revision += 1;
        self.fix_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Endpoint;

    fn doc() -> (Document, Registry) {
        (
            Document::new(Project::default(), None, PathBuf::from("."), 1),
            ac_nodes::registry(),
        )
    }

    #[test]
    fn edits_are_undoable_and_no_ops_record_nothing() {
        let (mut d, r) = doc();
        let id = d.edit(&r, "Add", |g, ctx| {
            edit::add_node(g, ctx, "gain", ac_core::empty_config(), [0.0, 0.0])
        });
        assert!(d.is_dirty());
        assert!(d.can_undo());
        // A failed connection changes nothing and leaves no step.
        d.edit(&r, "Connect", |g, ctx| {
            let _ = edit::connect(g, ctx, Endpoint::new(id, "nope"), Endpoint::new(id, "in"));
        });
        assert_eq!(d.undo_label(), Some("Add"));
        assert!(d.undo());
        assert!(d.project.graph.nodes.is_empty());
        assert!(!d.is_dirty());
        assert!(d.redo());
        assert_eq!(d.project.graph.nodes.len(), 1);
    }

    #[test]
    fn parameter_changes_coalesce_into_one_step() {
        let (mut d, r) = doc();
        let id = d.edit(&r, "Add", |g, ctx| {
            edit::add_node(g, ctx, "gain", ac_core::empty_config(), [0.0, 0.0])
        });
        for i in 0..10 {
            d.set_param(
                id,
                "gain",
                Value::Float(f64::from(i)),
                1.0 + f64::from(i) * 0.05,
            );
        }
        d.end_coalescing();
        d.set_param(id, "gain", Value::Float(-3.0), 5.0);
        assert!(d.undo());
        assert_eq!(
            d.project.graph.node(id).unwrap().params.get("gain"),
            Some(&Value::Float(9.0))
        );
        assert!(d.undo());
        assert!(
            !d.project
                .graph
                .node(id)
                .unwrap()
                .params
                .contains_key("gain")
        );
        assert_eq!(d.undo_label(), Some("Add"));
    }

    #[test]
    fn scope_follows_groups_and_survives_undo() {
        let (mut d, r) = doc();
        let a = d.edit(&r, "Add", |g, ctx| {
            edit::add_node(g, ctx, "gain", ac_core::empty_config(), [0.0, 0.0])
        });
        let (inst, def) = d
            .edit(&r, "Group", |g, ctx| {
                edit::group::make_group(g, ctx, &[a].into())
            })
            .unwrap();
        assert!(d.enter_group(inst));
        assert_eq!(d.current_def(), Some(&def));
        assert_eq!(d.graph().nodes.len(), 1);
        assert_eq!(d.node_path(a), NodePath(vec![inst, a]));
        assert_eq!(d.all_node_paths(a), vec![NodePath(vec![inst, a])]);
        assert_eq!(d.breadcrumb().len(), 2);
        // Undoing the grouping leaves the (now missing) scope.
        d.undo();
        assert!(d.scope.is_empty());
        assert_eq!(d.navigate_to(&NodePath::root(a)), Some(a));
        assert!(!d.leave_group());
    }
}
