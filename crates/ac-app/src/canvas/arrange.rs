//! Arranging what's on the canvas: auto-layout, align and distribute, lazy
//! connect, and jumping to a node. The rules live in `edit::{layout,
//! align, connect}`; this reads the cards as they're drawn and applies the
//! result to the document as one undo step. Moving cards doesn't change
//! what the patch does, so the engine isn't told.

use std::collections::BTreeSet;

use ac_core::{Endpoint, NodeId, REROUTE_KIND, Registry};
use egui::Rect;

use super::Canvas;
use super::menu::Target;
use super::scene::Class;
use super::view::View;
use crate::document::Document;
use crate::edit::align::Align;
use crate::edit::{self, connect, layout};
use crate::theme;

impl Canvas {
    /// The cards of `ids` as drawn (canvas units), in the order given;
    /// nodes that aren't drawn are left out.
    pub fn node_rects(
        &self,
        doc: &Document,
        registry: &Registry,
        ids: &[NodeId],
    ) -> Vec<(NodeId, Rect)> {
        let scene = self.build(doc, registry);
        ids.iter()
            .filter_map(|id| scene.node(*id).map(|n| (*id, n.rect)))
            .collect()
    }

    /// `Ctrl+L`: lay out the selection, or the whole graph when nothing is
    /// selected, in layers from left to right. Returns how many nodes it
    /// laid out, or why it didn't.
    pub fn auto_layout(
        &mut self,
        doc: &mut Document,
        registry: &Registry,
    ) -> Result<usize, String> {
        let scene = self.build(doc, registry);
        let selected: BTreeSet<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        let whole = selected.is_empty();
        let graph = doc.graph();
        let set = if whole { graph.ids() } else { selected };
        if set.len() < 2 {
            return Err(if whole {
                "Nothing to lay out yet: add a few nodes first".into()
            } else {
                "Select two or more nodes to lay out (or none, for the whole graph)".into()
            });
        }
        let cards: Vec<layout::Card> = graph
            .nodes
            .iter()
            .filter(|n| set.contains(&n.id))
            .filter_map(|n| {
                let v = scene.node(n.id)?;
                let ports = |list: &[super::scene::SocketView]| {
                    list.iter()
                        .map(|s| (s.name.clone(), s.pos.y - v.rect.min.y))
                        .collect()
                };
                Some(layout::Card {
                    id: n.id,
                    rect: v.rect,
                    inputs: ports(&v.inputs),
                    outputs: ports(&v.outputs),
                    feedback: registry.get(&n.kind).is_some_and(|e| e.meta.feedback),
                })
            })
            .collect();
        // A frame goes with its contents when they're all being laid out.
        let frames: Vec<layout::FrameBox> = graph
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, f)| {
                let rect = super::frame_rect(f);
                let mut inside = scene
                    .nodes
                    .iter()
                    .filter(|n| !edit::is_virtual(n.id) && rect.contains(n.rect.center()))
                    .peekable();
                inside.peek()?;
                inside
                    .all(|n| set.contains(&n.id))
                    .then_some(layout::FrameBox { index, rect })
            })
            .collect();
        // A portal's link isn't drawn, so it doesn't pull its tags
        // together: the sender stays by its source, each receiver by its
        // destination.
        let wires: Vec<ac_core::Wire> = graph
            .wires
            .iter()
            .filter(|w| !edit::portal::is_link(graph, w))
            .cloned()
            .collect();
        let arranged = layout::arrange(&cards, &wires, &frames);
        // Where each node's position sits on its card: the top-left, a
        // reroute's center, a portal tag's socket.
        let offsets: Vec<(NodeId, egui::Vec2)> = graph
            .nodes
            .iter()
            .filter_map(|n| {
                let v = scene.node(n.id)?;
                let d = egui::pos2(n.ui.pos[0], n.ui.pos[1]) - v.rect.min;
                let fallback = if n.kind == REROUTE_KIND {
                    egui::Vec2::splat(theme::REROUTE_RADIUS)
                } else {
                    egui::Vec2::ZERO
                };
                Some((n.id, if d.is_finite() { d } else { fallback }))
            })
            .collect();
        doc.checkpoint("Auto-layout");
        let g = doc.graph_mut_raw();
        for (id, min) in &arranged.cards {
            let d = offsets
                .iter()
                .find(|(i, _)| i == id)
                .map_or(egui::Vec2::ZERO, |(_, d)| *d);
            if let Some(n) = g.node_mut(*id) {
                n.ui.pos = [min.x + d.x, min.y + d.y];
            }
        }
        for (i, r) in &arranged.frames {
            if let Some(f) = g.frames.get_mut(*i) {
                f.rect = [r.min.x, r.min.y, r.width(), r.height()];
            }
        }
        doc.changed(false);
        doc.drop_noop_step();
        // The group interface cards sit beside the inner nodes again.
        if whole && let Some(def) = doc.current_def() {
            self.iface_pos.remove(def);
        }
        Ok(cards.len())
    }

    /// Align or distribute the selection (one undo step). Returns how many
    /// nodes it moved, or why it didn't.
    pub fn align_selection(
        &mut self,
        doc: &mut Document,
        registry: &Registry,
        how: Align,
    ) -> Result<usize, String> {
        let ids: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        if ids.len() < how.needs() {
            return Err(if how.distributes() {
                "Select three or more nodes to distribute".into()
            } else {
                "Select two or more nodes to align".into()
            });
        }
        let rects = self.node_rects(doc, registry, &ids);
        let moves = edit::align::moves(&rects, how);
        doc.checkpoint(how.undo_label());
        let g = doc.graph_mut_raw();
        let mut moved = 0;
        for (id, d) in &moves {
            if let Some(n) = g.node_mut(*id)
                && (d.x != 0.0 || d.y != 0.0)
            {
                n.ui.pos = [n.ui.pos[0] + d.x, n.ui.pos[1] + d.y];
                moved += 1;
            }
        }
        doc.changed(false);
        doc.drop_noop_step();
        Ok(moved)
    }

    /// `F`: wire the two selected nodes, left into right (or right into
    /// left when nothing fits that way). Returns what it wired, or why not.
    pub fn lazy_connect(
        &mut self,
        doc: &mut Document,
        registry: &Registry,
    ) -> Result<String, String> {
        let ids: Vec<NodeId> = doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect();
        let [a, b] = ids[..] else {
            return Err("Select two nodes to connect them (F)".into());
        };
        let scene = self.build(doc, registry);
        let (Some(na), Some(nb)) = (scene.node(a), scene.node(b)) else {
            return Err("Select two nodes to connect them (F)".into());
        };
        if [na, nb]
            .iter()
            .any(|n| matches!(n.class, Class::Missing(_)))
        {
            return Err("Can't wire a missing node".into());
        }
        let a_first = na
            .rect
            .min
            .x
            .total_cmp(&nb.rect.min.x)
            .then(na.rect.min.y.total_cmp(&nb.rect.min.y))
            .then(a.cmp(&b))
            .is_le();
        let (left, right) = if a_first { (na, nb) } else { (nb, na) };
        let graph = doc.display_graph();
        let iface = doc.interface();
        let defs = &doc.project.group_defs;
        let sockets = |id| edit::sockets_of(&graph, id, registry, defs, iface.as_ref());
        let wired = |id: NodeId| -> BTreeSet<String> {
            graph
                .wires
                .iter()
                .filter(|w| w.to.node == id)
                .map(|w| w.to.socket.clone())
                .collect()
        };
        let (sl, sr) = (
            connect::wirable(graph.node(left.id), sockets(left.id)),
            connect::wirable(graph.node(right.id), sockets(right.id)),
        );
        // A wire that would close a loop without a Feedback node is never
        // made (the compiler would leave the loop out, D25).
        let feedback = |id: NodeId| {
            graph
                .node(id)
                .and_then(|n| registry.get(&n.kind))
                .is_some_and(|e| e.meta.feedback)
        };
        let open = |from: NodeId, to: NodeId| !connect::closes_loop(&graph, from, to, feedback);
        let pick = connect::lazy_pick(&sl, &sr, &wired(right.id))
            .filter(|_| open(left.id, right.id))
            .map(|p| (left, right, p))
            .or_else(|| {
                connect::lazy_pick(&sr, &sl, &wired(left.id))
                    .filter(|_| open(right.id, left.id))
                    .map(|p| (right, left, p))
            });
        let Some((from, to, (output, input))) = pick else {
            return Err(format!(
                "Nothing to connect: no free input of {} takes an output of {}, or the other way \
                 round without making a loop",
                right.title, left.title
            ));
        };
        let said = format!(
            "Connected {} {} → {} {}",
            from.title,
            super::paint::pretty(&output),
            to.title,
            super::paint::pretty(&input)
        );
        let (from, to) = (Endpoint::new(from.id, output), Endpoint::new(to.id, input));
        let mut error = None;
        doc.edit(registry, "Connect", |g, ctx| {
            if let Err(e) = edit::connect(g, ctx, from, to) {
                error = Some(e);
            }
        });
        match error {
            Some(e) => Err(format!("Couldn't connect: {e}")),
            None => Ok(said),
        }
    }

    /// Show a node of the current graph: center the view on it (zooming in
    /// enough to read it) and select it. Call after `Document::navigate_to`:
    /// the view of the scope left behind is remembered as when leaving it
    /// with `Tab`.
    pub fn jump_to(&mut self, doc: &Document, registry: &Registry, id: NodeId) {
        let prefix = doc.scope_prefix();
        if self.view_scope.as_ref() != Some(&prefix) {
            if let Some(old) = self.view_scope.take() {
                self.scope_views
                    .insert(old, self.target.unwrap_or(self.view));
            }
            self.view_scope = Some(prefix);
        }
        let scene = self.build(doc, registry);
        if let Some(n) = scene.node(id) {
            self.target = Some(View {
                center: n.rect.center(),
                zoom: self.view.zoom.clamp(0.8, 1.5),
            });
        }
    }

    /// `Shift+Alt+A`: the align and distribute menu, at the pointer.
    pub fn open_arrange_menu(&mut self) {
        let at = self
            .pointer
            .filter(|p| self.screen.contains(*p))
            .unwrap_or(self.screen.center());
        self.menu = Some((Target::Arrange, at));
        self.menu_age = 0;
    }

    /// Whether a canvas popup (right-click menu, dropdown) is open.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
}
