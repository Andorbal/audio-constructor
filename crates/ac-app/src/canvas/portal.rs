//! Portals on the canvas (see `edit::portal`): their tags, what lights up
//! when one is hovered or selected, jumping between the two ends, and the
//! commands that make and remove them.

use std::collections::BTreeSet;

use ac_core::{Endpoint, NodeId, Registry, SignalType, Wire};
use egui::{Painter, Shape, Stroke, pos2};

use super::scene::{Class, NodeView, Scene};
use super::{Canvas, CanvasIo, Hit, PendingWire};
use crate::document::Document;
use crate::edit::portal;
use crate::theme;

/// The tags to light up: the other ends of a hovered or selected tag.
pub fn lit(scene: &Scene, hover: Option<NodeId>, selection: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    for n in &scene.nodes {
        if let Class::Portal(info) = &n.class
            && (hover == Some(n.id) || selection.contains(&n.id))
        {
            out.extend(info.partners.iter().copied());
            if hover == Some(n.id) {
                out.insert(n.id);
            }
        }
    }
    out
}

/// Where the sockets of `scene` are, for placing new tags next to them.
fn places(scene: &Scene) -> impl Fn(&Endpoint, bool) -> Option<[f32; 2]> + '_ {
    |ep, input| {
        let s = scene.node(ep.node)?.socket(&ep.socket, input)?;
        Some([s.pos.x, s.pos.y])
    }
}

/// What a portal command did, for the status line.
pub type Outcome = Result<String, String>;

impl Canvas {
    /// The portal tags lit in the last drawn frame (tests).
    pub fn portal_lit(&self) -> &BTreeSet<NodeId> {
        &self.portal_lit
    }

    /// A portal tag: a flag pointing the way the signal goes, its name, and
    /// its socket dot (a sender's input on the left, a receiver's output at
    /// the tip).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_portal(
        &mut self,
        painter: &Painter,
        n: &NodeView,
        info: &portal::Info,
        hover: &Hit,
        wiring: Option<&PendingWire>,
        alpha: f32,
    ) {
        let t = self.transform();
        let z = self.view.zoom;
        let r = t.rect_to_screen(n.rect);
        if !r.expand(20.0).intersects(self.screen) {
            return;
        }
        let ty = n
            .outputs
            .first()
            .or(n.inputs.first())
            .map_or(SignalType::Any, |s| s.ty);
        let color = theme::type_color(ty);
        let hovered = hover.node() == Some(n.id);
        let lit = self.portal_lit.contains(&n.id);
        let tip = (r.height() * 0.5).min(r.width() * 0.3);
        let points = vec![
            r.left_top(),
            pos2(r.max.x - tip, r.min.y),
            pos2(r.max.x, r.center().y),
            pos2(r.max.x - tip, r.max.y),
            r.left_bottom(),
        ];
        let mut shapes = Vec::new();
        if lit || hovered {
            shapes.push(Shape::convex_polygon(
                points.clone(),
                egui::Color32::TRANSPARENT,
                Stroke::new(theme::WIRE_GLOW, theme::fade(color, 0.22 * alpha)),
            ));
        }
        let outline = if n.selected {
            Stroke::new(1.8, theme::fade(theme::node_selected(), alpha))
        } else if info.alone() {
            Stroke::new(1.2, theme::fade(theme::warning(), alpha))
        } else {
            Stroke::new(1.4, theme::fade(color, alpha))
        };
        shapes.push(Shape::convex_polygon(
            points,
            theme::fade(theme::node_bg(), alpha),
            outline,
        ));
        painter.extend(shapes);
        if z >= 0.3 {
            let pad_left = if info.send { 10.0 } else { 7.0 } * z;
            let max_w = (r.width() - pad_left - tip - 2.0 * z).max(4.0);
            let mut job = egui::text::LayoutJob::single_section(
                info.name.clone(),
                egui::TextFormat::simple(
                    theme::canvas_font(theme::FONT_NODE_LABEL, z, false),
                    theme::fade(theme::text_strong(), alpha),
                ),
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
            let galley = painter.layout_job(job);
            let y = r.center().y - galley.size().y / 2.0;
            painter.galley(pos2(r.min.x + pad_left, y), galley, theme::text_strong());
        }
        let mut dots = Vec::new();
        for s in n.inputs.iter().chain(n.outputs.iter()) {
            if super::portal_side_open(info, s.input) {
                self.paint_socket_dot(&mut dots, t.to_screen(s.pos), n, s, hover, wiring, alpha);
            }
        }
        painter.extend(dots);
        self.tips.push((r, describe(info)));
    }

    /// Double-click on a tag: select the other end and bring it into view.
    /// A sender with several receivers goes to the next one each time.
    pub fn jump_to_partner(&mut self, doc: &mut Document, registry: &Registry, id: NodeId) -> bool {
        let info = portal::index(doc.graph());
        let Some(me) = info.get(&id) else {
            return false;
        };
        if me.partners.is_empty() {
            return false;
        }
        let k = match self.portal_jump.get(&id) {
            Some(k) => (k + 1) % me.partners.len(),
            None => 0,
        };
        self.portal_jump.insert(id, k);
        let target = me.partners[k];
        doc.selection = [target].into();
        self.focus_node(doc, registry, target);
        true
    }

    /// Make or remove portals from the selection (the Portal command): tags
    /// go back to wires; plain reroutes become senders; otherwise the wires
    /// between the selected nodes become portals.
    pub fn toggle_portal(&mut self, doc: &mut Document, registry: &Registry, now: f64) -> Outcome {
        let sel: BTreeSet<NodeId> = doc.selection.clone();
        let graph = doc.graph();
        let tags: BTreeSet<NodeId> = sel
            .iter()
            .copied()
            .filter(|id| graph.node(*id).and_then(portal::tag).is_some())
            .collect();
        if !tags.is_empty() {
            let gone = doc.edit(registry, "Back to wire", |g, _| portal::remove(g, &tags));
            doc.selection.retain(|id| !gone.contains(id));
            return Ok(format!(
                "Back to {}",
                if gone.len() > 2 { "wires" } else { "a wire" }
            ));
        }
        let reroutes: Vec<NodeId> = sel
            .iter()
            .copied()
            .filter(|id| {
                graph
                    .node(*id)
                    .is_some_and(|n| n.kind == ac_core::REROUTE_KIND)
            })
            .collect();
        let scene = self.build(doc, registry);
        let places = places(&scene);
        let made: Vec<NodeId> = if !reroutes.is_empty() {
            doc.edit(registry, "Make portal", |g, ctx| {
                let mut made = Vec::new();
                for r in &reroutes {
                    if let Some(new) = portal::make_from_reroute(g, ctx, *r, &places) {
                        made.push(*r);
                        made.extend(new);
                    }
                }
                made
            })
        } else {
            let wires = portal::wires_between(&doc.display_graph(), &sel);
            if wires.is_empty() {
                return Err(
                    "To make a portal, select a reroute, or the two nodes a wire joins".into(),
                );
            }
            doc.edit(registry, "Make portal", |g, ctx| {
                let mut made = Vec::new();
                for w in &wires {
                    if let Some((s, r)) = portal::make_from_wire(g, ctx, w, &places) {
                        made.extend([s, r]);
                    }
                }
                made
            })
        };
        self.after_making(doc, &made, now)
    }

    /// Make a portal of one wire (its right-click menu).
    pub(super) fn portal_from_wire(&mut self, io: &mut CanvasIo, wire: &Wire, now: f64) -> Outcome {
        let scene = self.build(io.doc, io.registry);
        let places = places(&scene);
        let made = io.doc.edit(io.registry, "Make portal", |g, ctx| {
            portal::make_from_wire(g, ctx, wire, &places)
                .map(|(s, r)| vec![s, r])
                .unwrap_or_default()
        });
        self.after_making(io.doc, &made, now)
    }

    /// Make a portal of one plain reroute (its right-click menu).
    pub(super) fn portal_from_reroute(
        &mut self,
        io: &mut CanvasIo,
        id: NodeId,
        now: f64,
    ) -> Outcome {
        let scene = self.build(io.doc, io.registry);
        let places = places(&scene);
        let made = io.doc.edit(io.registry, "Make portal", |g, ctx| {
            portal::make_from_reroute(g, ctx, id, &places)
                .map(|r| std::iter::once(id).chain(r).collect::<Vec<_>>())
                .unwrap_or_default()
        });
        self.after_making(io.doc, &made, now)
    }

    fn after_making(&mut self, doc: &mut Document, made: &[NodeId], now: f64) -> Outcome {
        let graph = doc.graph();
        let Some(name) = made
            .iter()
            .find_map(|id| graph.node(*id).and_then(portal::tag))
            .map(|t| t.name)
        else {
            return Err("Nothing there could become a portal".into());
        };
        let fresh: Vec<NodeId> = made
            .iter()
            .copied()
            .filter(|id| !self.born.contains_key(id))
            .collect();
        self.mark_born(&fresh, now);
        Ok(format!(
            "Made portal “{name}”. Double-click a tag to jump to the other end; F2 renames it."
        ))
    }
}

/// The hover text of a tag.
fn describe(info: &portal::Info) -> String {
    let n = info.partners.len();
    let what = match (info.send, n) {
        (true, 0) => "No receivers: copy and paste one of this portal's receivers, or \
                      turn another wire from the same output into a portal."
            .to_string(),
        (true, 1) => "Sends to 1 receiver.".to_string(),
        (true, n) => format!("Sends to {n} receivers."),
        (false, 0) => "No sender is linked to it.".to_string(),
        (false, _) => "Receives from its sender.".to_string(),
    };
    format!(
        "Portal “{}”. {what}\nDouble-click to jump to the other end · F2 renames · \
         Shift+P turns it back into a wire",
        info.name
    )
}
