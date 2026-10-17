//! Unlimited undo by snapshots: every step stores a copy of the project as
//! it was before the change. At Phase 1 project sizes a clone is cheap, and
//! snapshots can't drift out of sync with the operations that made them.

use std::collections::BTreeSet;

use ac_core::{NodeId, Project};

use crate::document::ScopeEntry;

/// The document as it was before a change.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub label: String,
    pub project: Project,
    pub scope: Vec<ScopeEntry>,
    pub selection: BTreeSet<NodeId>,
    pub revision: u64,
}

/// Changes with the same key less than this far apart merge into one step.
const COALESCE_SECONDS: f64 = 1.0;

#[derive(Default)]
pub struct History {
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    /// Key and time of the last coalescable change.
    open: Option<(String, f64)>,
}

impl History {
    /// Record `before` as an undo step. With a `key`, a change following
    /// another with the same key within a second extends that step instead.
    pub fn push(&mut self, before: Snapshot, key: Option<&str>, now: f64) {
        if let (Some(k), Some((open, t))) = (key, &mut self.open)
            && open == k
            && now - *t < COALESCE_SECONDS
        {
            *t = now;
            return;
        }
        self.open = key.map(|k| (k.to_string(), now));
        self.past.push(before);
        self.future.clear();
    }

    pub fn end_coalescing(&mut self) {
        self.open = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|s| s.label.as_str())
    }

    /// Step back: returns the state to restore; `current` becomes redoable.
    pub fn undo(&mut self, mut current: Snapshot) -> Option<Snapshot> {
        let prev = self.past.pop()?;
        current.label = prev.label.clone();
        self.future.push(current);
        self.open = None;
        Some(prev)
    }

    /// Step forward again.
    pub fn redo(&mut self, mut current: Snapshot) -> Option<Snapshot> {
        let next = self.future.pop()?;
        current.label = next.label.clone();
        self.past.push(current);
        self.open = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, rev: u64) -> Snapshot {
        Snapshot {
            label: name.into(),
            project: Project {
                name: name.into(),
                ..Project::default()
            },
            scope: Vec::new(),
            selection: BTreeSet::new(),
            revision: rev,
        }
    }

    #[test]
    fn undo_redo_walks_the_steps() {
        let mut h = History::default();
        h.push(snap("a", 0), None, 0.0);
        h.push(snap("b", 1), None, 0.0);
        let s = h.undo(snap("c", 2)).unwrap();
        assert_eq!(s.project.name, "b");
        let s = h.undo(snap("b", 1)).unwrap();
        assert_eq!(s.project.name, "a");
        assert!(h.undo(snap("a", 0)).is_none());
        let s = h.redo(snap("a", 0)).unwrap();
        assert_eq!(s.project.name, "b");
        // A new change discards the redo branch.
        h.push(snap("x", 5), None, 0.0);
        assert!(!h.can_redo());
    }

    #[test]
    fn same_key_changes_coalesce_until_a_pause() {
        let mut h = History::default();
        h.push(snap("a", 0), Some("k"), 0.0);
        h.push(snap("a2", 1), Some("k"), 0.5);
        h.push(snap("a3", 2), Some("k"), 1.2);
        assert_eq!(h.past.len(), 1);
        h.push(snap("b", 3), Some("k"), 3.0);
        assert_eq!(h.past.len(), 2);
        h.push(snap("c", 4), Some("other"), 3.1);
        assert_eq!(h.past.len(), 3);
        h.end_coalescing();
        h.push(snap("d", 5), Some("other"), 3.2);
        assert_eq!(h.past.len(), 4);
    }
}
