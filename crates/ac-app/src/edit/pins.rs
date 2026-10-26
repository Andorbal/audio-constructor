//! Editing performance panels (docs/UX.md "Panels"): pinning nodes, and
//! adding, renaming, ordering, and removing panels and their items.
//!
//! Panels only ever refer to nodes in the root graph that have a widget
//! body (controls, displays, Clip, Record, Audio Out), a drawn editor
//! (Pattern, MIDI Pattern, Step Sequencer, Graph, D217), or the
//! Arrangement view (D221), or that are group instances (shown as a card
//! of their sockets, D203). Nodes inside groups
//! can't be pinned: a definition is shared by every instance, so a pin
//! there wouldn't say which instance it shows (DECISIONS D96).

use ac_core::{Graph, NodeId, NodeInstance, Panel, PanelItem, Strip};

use crate::canvas::scene::Body;
use crate::perform::grid;

/// Whether a node can be pinned: it has a widget body or a drawn editor,
/// it's the Arrangement, or it's a group or block instance.
pub fn pinnable(node: &NodeInstance) -> bool {
    Body::of(&node.kind).is_some()
        || crate::editors::editor_for(&node.kind, &node.config).is_some()
        || node.kind == "arrangement"
        || crate::perform::group_card::is_group(node)
}

/// Cards that pin compact by default (half a cell): a toggle, a button, a
/// light.
pub fn compact_by_default(node: &NodeInstance) -> bool {
    matches!(node.kind.as_str(), "toggle" | "button" | "light")
}

/// Pin as [`pin`] does; on a panel whose cards have places, each new card
/// gets the first free place for its `size` (cells across, lines down)
/// within `columns`, so nothing already there moves (D202).
pub fn pin_placed(
    panels: &mut [Panel],
    graph: &Graph,
    panel: usize,
    ids: &[NodeId],
    size: &dyn Fn(&PanelItem) -> Option<[u32; 2]>,
    columns: u32,
) -> usize {
    let added = pin(panels, graph, panel, ids);
    let Some(p) = panels.get_mut(panel) else {
        return added;
    };
    if !p.is_placed() {
        return added;
    }
    let n = p.items.len();
    for k in n - added..n {
        let Some(span) = size(&p.items[k]) else {
            continue;
        };
        let layout = grid::layout(
            p,
            |i| if i >= k { None } else { size(&p.items[i]) },
            columns,
        );
        let at = layout.free_spot(span, columns);
        p.items[k].at = Some(at);
    }
    added
}

/// Pin the pinnable `ids` (in that order) to `panel` as one new strip named
/// `name`, at the first free place; nodes already on the panel move into
/// it. Toggles, buttons, and lights go in compact. A flowing panel gets
/// places first, so its cards stay where they were. Returns the strip's id.
pub fn pin_strip(
    panels: &mut [Panel],
    graph: &Graph,
    panel: usize,
    ids: &[NodeId],
    name: &str,
    size: &dyn Fn(&PanelItem) -> Option<[u32; 2]>,
    columns: u32,
) -> Option<u32> {
    let p = panels.get_mut(panel)?;
    let nodes: Vec<&NodeInstance> = ids
        .iter()
        .filter_map(|id| graph.node(*id))
        .filter(|n| pinnable(n))
        .collect();
    if nodes.is_empty() {
        return None;
    }
    let before = grid::layout(p, |i| size(&p.items[i]), columns);
    if !p.is_placed() && !p.items.is_empty() {
        grid::freeze(p, &before);
    }
    let id = p.fresh_strip_id();
    p.items.retain(|it| !nodes.iter().any(|n| n.id == it.node));
    for n in &nodes {
        let mut it = PanelItem::new(n.id);
        it.strip = Some(id);
        it.compact = compact_by_default(n);
        p.items.push(it);
    }
    let mut strip = Strip::new(id, name);
    let lines = 1 + p
        .items
        .iter()
        .filter(|it| it.strip == Some(id))
        .filter_map(|it| size(it).map(|s| s[1]))
        .sum::<u32>();
    let others = grid::layout(
        p,
        |i| {
            let it = &p.items[i];
            if it.strip == Some(id) { None } else { size(it) }
        },
        columns,
    );
    strip.at = Some(others.free_spot([1, lines], columns));
    p.strips.push(strip);
    Some(id)
}

/// A name for a new panel that no panel has yet, counting from its place:
/// "Panel 1" for the first, "Panel 3" for a third, …
pub fn fresh_name(panels: &[Panel]) -> String {
    (panels.len() + 1..)
        .map(|n| format!("Panel {n}"))
        .find(|name| panels.iter().all(|p| p.name != *name))
        .unwrap_or_else(|| "Panel".into())
}

/// Add a panel at the end (named `name`, or a fresh name when empty).
/// Returns its index.
pub fn add_panel(panels: &mut Vec<Panel>, name: &str) -> usize {
    let name = name.trim();
    let name = if name.is_empty() {
        fresh_name(panels)
    } else {
        name.to_string()
    };
    panels.push(Panel::new(name));
    panels.len() - 1
}

/// Pin the pinnable nodes of `ids` that `graph` holds to panel `panel`, in
/// the order given, after what's there. Nodes already on it stay where
/// they are. Returns how many were added.
pub fn pin(panels: &mut [Panel], graph: &Graph, panel: usize, ids: &[NodeId]) -> usize {
    let Some(p) = panels.get_mut(panel) else {
        return 0;
    };
    let mut added = 0;
    for id in ids {
        let ok = graph.node(*id).is_some_and(pinnable);
        if ok && !p.contains(*id) {
            p.items.push(PanelItem::new(*id));
            added += 1;
        }
    }
    added
}

/// Take `ids` off panel `panel`, or off every panel with `None`. Returns
/// how many pins were removed.
pub fn unpin(panels: &mut [Panel], panel: Option<usize>, ids: &[NodeId]) -> usize {
    let mut removed = 0;
    for (i, p) in panels.iter_mut().enumerate() {
        if panel.is_some_and(|k| k != i) {
            continue;
        }
        let before = p.items.len();
        p.items.retain(|it| !ids.contains(&it.node));
        removed += before - p.items.len();
    }
    removed
}

/// The names of the panels `node` is pinned to.
pub fn panels_of(panels: &[Panel], node: NodeId) -> Vec<&str> {
    panels
        .iter()
        .filter(|p| p.contains(node))
        .map(|p| p.name.as_str())
        .collect()
}

/// Rename a panel. Blank names are ignored. Returns whether it changed.
pub fn rename_panel(panels: &mut [Panel], panel: usize, name: &str) -> bool {
    let name = name.trim();
    match panels.get_mut(panel) {
        Some(p) if !name.is_empty() && p.name != name => {
            p.name = name.to_string();
            true
        }
        _ => false,
    }
}

/// Remove a panel (its nodes stay in the graph).
pub fn remove_panel(panels: &mut Vec<Panel>, panel: usize) -> bool {
    if panel < panels.len() {
        panels.remove(panel);
        true
    } else {
        false
    }
}

/// Move the element at `from` to index `to` (both clamped to the list).
fn move_in<T>(list: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from >= list.len() {
        return false;
    }
    let to = to.min(list.len() - 1);
    if from == to {
        return false;
    }
    let x = list.remove(from);
    list.insert(to, x);
    true
}

/// Move a panel to another place in the tab order.
pub fn move_panel(panels: &mut Vec<Panel>, from: usize, to: usize) -> bool {
    move_in(panels, from, to)
}

/// Move an item within a panel.
pub fn move_item(panels: &mut [Panel], panel: usize, from: usize, to: usize) -> bool {
    panels
        .get_mut(panel)
        .is_some_and(|p| move_in(&mut p.items, from, to))
}

/// Set an item's span in cells (`None`: its natural size).
pub fn resize_item(
    panels: &mut [Panel],
    panel: usize,
    item: usize,
    size: Option<[u32; 2]>,
) -> bool {
    let Some(it) = panels.get_mut(panel).and_then(|p| p.items.get_mut(item)) else {
        return false;
    };
    let size = size.map(|[c, r]| {
        [
            c.clamp(1, ac_core::panel::MAX_SPAN),
            r.clamp(1, ac_core::panel::MAX_SPAN),
        ]
    });
    if it.size == size {
        return false;
    }
    it.size = size;
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ac_core::{Endpoint, Project, Registry};

    use super::*;
    use crate::document::Document;
    use crate::edit;

    #[test]
    fn editors_and_the_arrangement_pin() {
        use ac_core::NodeInstance;
        for kind in [
            "pattern",
            "midi_pattern",
            "step_sequencer",
            "graph",
            "arrangement",
            "knob",
        ] {
            assert!(pinnable(&NodeInstance::new(NodeId(1), kind)), "{kind}");
        }
        assert!(!pinnable(&NodeInstance::new(NodeId(1), "gain")));
    }

    /// A slider and a knob feeding a gain, and a meter after it.
    fn doc() -> (Document, Registry, [NodeId; 4]) {
        let r = ac_nodes::registry();
        let mut d = Document::new(Project::default(), None, PathBuf::from("."), 1);
        let ids = d.edit(&r, "Add", |g, ctx| {
            let s = edit::add_node(g, ctx, "slider", ac_core::empty_config(), [0.0, 0.0]);
            let k = edit::add_node(g, ctx, "knob", ac_core::empty_config(), [0.0, 200.0]);
            let gain = edit::add_node(g, ctx, "gain", ac_core::empty_config(), [300.0, 0.0]);
            let m = edit::add_node(g, ctx, "meter", ac_core::empty_config(), [600.0, 0.0]);
            edit::connect(g, ctx, Endpoint::new(s, "out"), Endpoint::new(gain, "gain")).ok();
            edit::connect(g, ctx, Endpoint::new(gain, "out"), Endpoint::new(m, "in")).ok();
            [s, k, gain, m]
        });
        (d, r, ids)
    }

    fn items(d: &Document, panel: usize) -> Vec<NodeId> {
        d.project.panels[panel]
            .items
            .iter()
            .map(|i| i.node)
            .collect()
    }

    #[test]
    fn pinning_takes_controls_and_displays_only() {
        let (mut d, _, [s, k, gain, m]) = doc();
        let added = d.edit_panels("Pin", |panels, g| {
            let i = add_panel(panels, "");
            pin(panels, g, i, &[s, gain, m, k, s, NodeId(999)])
        });
        assert_eq!(added, 3, "the gain and a missing node aren't pinned");
        assert_eq!(d.project.panels[0].name, "Panel 1");
        assert_eq!(items(&d, 0), vec![s, m, k]);
        assert_eq!(panels_of(&d.project.panels, m), vec!["Panel 1"]);
        assert_eq!(fresh_name(&d.project.panels), "Panel 2");
    }

    #[test]
    fn pin_and_unpin_are_undo_steps_without_a_recompile() {
        let (mut d, _, [s, k, ..]) = doc();
        let engine = d.engine_revision;
        d.edit_panels("Pin", |panels, g| {
            let i = add_panel(panels, "Live");
            pin(panels, g, i, &[s, k])
        });
        assert_eq!(d.undo_label(), Some("Pin"));
        assert_eq!(d.engine_revision, engine, "panels don't reach the engine");
        d.edit_panels("Unpin", |panels, _| unpin(panels, Some(0), &[s]));
        assert_eq!(items(&d, 0), vec![k]);
        // Nothing changed: no step.
        d.edit_panels("Unpin", |panels, _| unpin(panels, Some(0), &[s]));
        assert_eq!(d.undo_label(), Some("Unpin"));
        assert!(d.undo());
        assert_eq!(items(&d, 0), vec![s, k]);
        assert!(d.undo());
        assert!(d.project.panels.is_empty());
        assert!(d.redo());
        assert_eq!(items(&d, 0), vec![s, k]);
    }

    #[test]
    fn deleting_a_node_takes_it_off_every_panel_in_the_same_step() {
        let (mut d, r, [s, k, _, m]) = doc();
        d.edit_panels("Pin", |panels, g| {
            let a = add_panel(panels, "Live");
            let b = add_panel(panels, "Mixer");
            pin(panels, g, a, &[s, k, m]);
            pin(panels, g, b, &[m, s])
        });
        d.edit(&r, "Delete", |g, _| {
            edit::delete_nodes(g, &[s].into());
        });
        assert_eq!(items(&d, 0), vec![k, m]);
        assert_eq!(items(&d, 1), vec![m]);
        // One undo brings back the node and its pins.
        assert_eq!(d.undo_label(), Some("Delete"));
        assert!(d.undo());
        assert_eq!(items(&d, 0), vec![s, k, m]);
        assert_eq!(items(&d, 1), vec![m, s]);
        // Delete with reconnect too.
        d.edit(&r, "Delete and reconnect", |g, ctx| {
            edit::delete_reconnect(g, ctx, &[m].into());
        });
        assert_eq!(items(&d, 0), vec![s, k]);
        assert_eq!(items(&d, 1), vec![s]);
    }

    #[test]
    fn grouping_drops_the_grouped_pins_and_ungrouping_keeps_the_rest() {
        let (mut d, r, [s, k, gain, m]) = doc();
        d.edit_panels("Pin", |panels, g| {
            let a = add_panel(panels, "Live");
            pin(panels, g, a, &[s, k, m])
        });
        // The knob and the gain go into a group: the knob's pin goes (nodes
        // inside groups can't be pinned), the others stay.
        let (inst, _) = d
            .edit(&r, "Make group", |g, ctx| {
                edit::group::make_group(g, ctx, &[k, gain].into())
            })
            .unwrap();
        assert_eq!(items(&d, 0), vec![s, m]);
        // Ungrouping brings the nodes back with new ids, unpinned; the
        // other pins are untouched.
        let new = d
            .edit(&r, "Ungroup", |g, ctx| edit::group::ungroup(g, ctx, inst))
            .unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(items(&d, 0), vec![s, m]);
        // Undoing both steps brings the knob's pin back.
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(items(&d, 0), vec![s, k, m]);
    }

    #[test]
    fn panels_rename_reorder_resize_and_go() {
        let (mut d, _, [s, k, _, m]) = doc();
        d.edit_panels("Add panels", |panels, g| {
            let a = add_panel(panels, "Live");
            add_panel(panels, "  ");
            add_panel(panels, "Mixer");
            pin(panels, g, a, &[s, k, m])
        });
        let names = |d: &Document| -> Vec<String> {
            d.project.panels.iter().map(|p| p.name.clone()).collect()
        };
        assert_eq!(names(&d), ["Live", "Panel 2", "Mixer"]);
        let mut panels = d.project.panels.clone();
        assert!(rename_panel(&mut panels, 1, " Drums "));
        assert!(!rename_panel(&mut panels, 1, "   "));
        assert!(!rename_panel(&mut panels, 9, "x"));
        assert!(move_panel(&mut panels, 2, 0));
        assert!(!move_panel(&mut panels, 7, 0));
        assert!(move_item(&mut panels, 1, 0, 5), "clamped to the end");
        assert!(resize_item(&mut panels, 1, 0, Some([0, 99])));
        assert_eq!(panels[1].items[0].size, Some([1, ac_core::panel::MAX_SPAN]));
        assert!(!resize_item(&mut panels, 1, 9, None));
        let order: Vec<NodeId> = panels[1].items.iter().map(|i| i.node).collect();
        assert_eq!(order, vec![k, m, s]);
        assert!(remove_panel(&mut panels, 2));
        assert!(!remove_panel(&mut panels, 2));
        let names: Vec<&str> = panels.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Mixer", "Live"]);
    }

    #[test]
    fn documents_drop_dangling_pins_when_opened() {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(ac_core::NodeInstance::new(NodeId(1), "toggle"));
        let mut panel = Panel::new("Live");
        panel.items.push(PanelItem::new(NodeId(7)));
        panel.items.push(PanelItem::new(NodeId(1)));
        p.panels.push(panel);
        let d = Document::new(p, None, PathBuf::from("."), 1);
        assert_eq!(items(&d, 0), vec![NodeId(1)]);
        assert!(!d.is_dirty());
    }
}
