//! Portals (UX.md "Wire routing"): a long wire drawn as named tags, a
//! sender by its source and a receiver by each destination, with no line
//! between them.
//!
//! In the graph a portal is plain Reroutes and wires: the source feeds the
//! sender, the sender's `out` feeds every receiver's `in`, and each receiver
//! feeds its destinations. The engine splices reroutes out, so a portal costs
//! nothing and plays exactly like the wire it replaced; an editor that
//! doesn't know portals shows two reroutes and a wire. Each tag stores its
//! name and role in the reroute's preserved extra fields (DECISIONS D104):
//! `"portal": {"name": "Bass", "role": "send"}` (or `"receive"`). A wire
//! from a sender to a receiver is the portal's link and isn't drawn.

use std::collections::{BTreeMap, BTreeSet};

use ac_core::{Endpoint, Graph, NodeId, NodeInstance, REROUTE_KIND, Wire};

use super::{Ctx, GROUP_IN, add_node, is_virtual};

/// Key in a reroute's preserved extra fields that makes it a portal tag.
pub const PORTAL: &str = "portal";

/// How far a new tag sits from the socket it's wired to (canvas units).
pub const GAP: f32 = 36.0;

/// Longest portal name kept.
const MAX_NAME: usize = 40;

/// A reroute's portal tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The sender (by the source) rather than a receiver.
    pub send: bool,
}

/// The tag of a node, if it's a portal.
pub fn tag(node: &NodeInstance) -> Option<Tag> {
    if node.kind != REROUTE_KIND {
        return None;
    }
    let v = node.extra.get(PORTAL)?.as_object()?;
    let send = match v.get("role")?.as_str()? {
        "send" => true,
        "receive" => false,
        _ => return None,
    };
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .and_then(clean_name)
        .unwrap_or_else(|| "portal".into());
    Some(Tag { name, send })
}

fn tag_of(graph: &Graph, id: NodeId) -> Option<Tag> {
    graph.node(id).and_then(tag)
}

fn set_tag(graph: &mut Graph, id: NodeId, t: Option<&Tag>) {
    let Some(n) = graph.node_mut(id) else { return };
    match t {
        Some(t) => {
            n.extra.insert(
                PORTAL.into(),
                serde_json::json!({
                    "name": t.name,
                    "role": if t.send { "send" } else { "receive" },
                }),
            );
        }
        None => {
            n.extra.remove(PORTAL);
        }
    }
}

fn is_sender(graph: &Graph, id: NodeId) -> bool {
    tag_of(graph, id).is_some_and(|t| t.send)
}

fn is_receiver(graph: &Graph, id: NodeId) -> bool {
    tag_of(graph, id).is_some_and(|t| !t.send)
}

/// Is `w` the hidden link from a sender to one of its receivers?
pub fn is_link(graph: &Graph, w: &Wire) -> bool {
    w.from.socket == "out"
        && w.to.socket == "in"
        && is_sender(graph, w.from.node)
        && is_receiver(graph, w.to.node)
}

/// The sender feeding a receiver.
pub fn sender_of(graph: &Graph, receiver: NodeId) -> Option<NodeId> {
    let w = graph.wire_into(&Endpoint::new(receiver, "in"))?;
    is_link(graph, w).then_some(w.from.node)
}

/// The receivers a sender feeds, in graph order.
pub fn receivers_of(graph: &Graph, sender: NodeId) -> Vec<NodeId> {
    graph
        .wires_from(sender)
        .filter(|w| is_link(graph, w))
        .map(|w| w.to.node)
        .collect()
}

/// A portal as the canvas draws one of its tags.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// The portal's name (a linked receiver shows its sender's).
    pub name: String,
    pub send: bool,
    /// The other end: a sender's receivers, a receiver's sender.
    pub partners: Vec<NodeId>,
}

impl Info {
    /// A receiver whose sender is gone, or a sender with no receivers.
    pub fn alone(&self) -> bool {
        self.partners.is_empty()
    }
}

/// Every portal tag in a graph, in one pass over its nodes and wires (the
/// canvas builds this every frame).
pub fn index(graph: &Graph) -> BTreeMap<NodeId, Info> {
    let mut out: BTreeMap<NodeId, Info> = graph
        .nodes
        .iter()
        .filter_map(|n| {
            let t = tag(n)?;
            Some((
                n.id,
                Info {
                    name: t.name,
                    send: t.send,
                    partners: Vec::new(),
                },
            ))
        })
        .collect();
    if out.is_empty() {
        return out;
    }
    let links: Vec<(NodeId, NodeId)> = graph
        .wires
        .iter()
        .filter(|w| is_link_in(&out, w))
        .map(|w| (w.from.node, w.to.node))
        .collect();
    for (s, r) in links {
        let name = out.get(&s).map(|i| i.name.clone());
        if let Some(i) = out.get_mut(&s) {
            i.partners.push(r);
        }
        if let Some(i) = out.get_mut(&r) {
            i.partners = vec![s];
            if let Some(name) = name {
                i.name = name;
            }
        }
    }
    out
}

/// [`is_link`] against an [`index`] of the graph.
pub fn is_link_in(index: &BTreeMap<NodeId, Info>, w: &Wire) -> bool {
    w.from.socket == "out"
        && w.to.socket == "in"
        && index.get(&w.from.node).is_some_and(|i| i.send)
        && index.get(&w.to.node).is_some_and(|i| !i.send)
}

/// The tags of the portal `id` belongs to: its sender (if any) and every
/// receiver. Empty if `id` isn't a portal tag.
pub fn members(graph: &Graph, id: NodeId) -> Vec<NodeId> {
    let Some(t) = tag_of(graph, id) else {
        return Vec::new();
    };
    let sender = if t.send {
        Some(id)
    } else {
        sender_of(graph, id)
    };
    match sender {
        Some(s) => std::iter::once(s).chain(receivers_of(graph, s)).collect(),
        None => vec![id],
    }
}

/// A name as typed, tidied: trimmed, inner whitespace collapsed, at most
/// 40 characters. `None` if nothing is left.
pub fn clean_name(s: &str) -> Option<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(MAX_NAME).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `base`, or `base 2`, `base 3`, … whichever no other portal uses.
pub fn unique_name(base: &str, taken: &[String]) -> String {
    let free = |c: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(c));
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| free(c))
        .unwrap_or_else(|| base.to_string())
}

/// Names of the portals in a graph other than those with tags in `except`.
fn names_in_use(graph: &Graph, except: &BTreeSet<NodeId>) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|n| !except.contains(&n.id))
        .filter_map(tag)
        .map(|t| t.name)
        .collect()
}

/// A name for the signal leaving `source`: the node's label or kind name,
/// with the socket when the node has more than one output.
pub fn default_name(graph: &Graph, ctx: &Ctx, source: &Endpoint) -> String {
    if source.node == GROUP_IN {
        return clean_name(&pretty(&source.socket)).unwrap_or_else(|| "portal".into());
    }
    let Some(n) = graph.node(source.node) else {
        return "portal".into();
    };
    if let Some(t) = tag(n) {
        return t.name;
    }
    let title = n.ui.label.clone().unwrap_or_else(|| {
        ctx.registry
            .get(&n.kind)
            .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string())
    });
    let outputs = ctx.sockets(graph, source.node).outputs.len();
    let name = if outputs > 1 {
        format!("{title} {}", pretty(&source.socket))
    } else {
        title
    };
    clean_name(&name).unwrap_or_else(|| "portal".into())
}

/// `cutoff_hz` → `cutoff hz`, as sockets are labeled on cards.
fn pretty(socket: &str) -> String {
    socket.replace('_', " ")
}

/// Where sockets sit on the canvas, for placing new tags next to them:
/// `(endpoint, is_input)` → canvas point. The editor reads them from the
/// drawn scene; `None` places a tag by its node's corner.
pub type Places<'a> = &'a dyn Fn(&Endpoint, bool) -> Option<[f32; 2]>;

fn place(graph: &Graph, places: Places, ep: &Endpoint, input: bool) -> [f32; 2] {
    places(ep, input)
        .or_else(|| graph.node(ep.node).map(|n| n.ui.pos))
        .unwrap_or([0.0, 0.0])
}

fn new_tag(graph: &mut Graph, ctx: &mut Ctx, at: [f32; 2], t: &Tag) -> NodeId {
    let id = add_node(graph, ctx, REROUTE_KIND, ac_core::empty_config(), at);
    set_tag(graph, id, Some(t));
    id
}

/// The sender already on output `source`, if there is one.
fn sender_on(graph: &Graph, source: &Endpoint) -> Option<NodeId> {
    if is_sender(graph, source.node) {
        return Some(source.node);
    }
    graph
        .wires
        .iter()
        .find(|w| w.from == *source && w.to.socket == "in" && is_sender(graph, w.to.node))
        .map(|w| w.to.node)
}

/// Turn `wire` into a portal: a sender by its source (the one already on
/// that output, if any, so one source has one sender and several
/// receivers) and a receiver by its destination. A wire into a receiver
/// whose sender is gone links that receiver instead. Returns the sender and
/// the receiver; `None` if the wire isn't in the graph or is a portal link.
pub fn make_from_wire(
    graph: &mut Graph,
    ctx: &mut Ctx,
    wire: &Wire,
    places: Places,
) -> Option<(NodeId, NodeId)> {
    if !graph.wires.contains(wire) || is_link(graph, wire) {
        return None;
    }
    let source = wire.from.clone();
    let sender = match sender_on(graph, &source) {
        Some(s) => s,
        None => {
            let name = unique_name(
                &default_name(graph, ctx, &source),
                &names_in_use(graph, &BTreeSet::new()),
            );
            let p = place(graph, places, &source, false);
            let s = new_tag(
                graph,
                ctx,
                [p[0] + GAP, p[1]],
                &Tag {
                    name: name.clone(),
                    send: true,
                },
            );
            graph.connect(source.clone(), Endpoint::new(s, "in"));
            s
        }
    };
    let name = tag_of(graph, sender).map_or_else(|| "portal".into(), |t| t.name);
    graph.wires.retain(|w| w != wire);
    let receiver = if wire.to.socket == "in" && is_receiver(graph, wire.to.node) {
        set_tag(graph, wire.to.node, Some(&Tag { name, send: false }));
        wire.to.node
    } else {
        let p = place(graph, places, &wire.to, true);
        let r = new_tag(graph, ctx, [p[0] - GAP, p[1]], &Tag { name, send: false });
        graph.connect(Endpoint::new(r, "out"), wire.to.clone());
        r
    };
    graph.connect(Endpoint::new(sender, "out"), Endpoint::new(receiver, "in"));
    Some((sender, receiver))
}

/// Turn a plain reroute into a portal: it becomes the sender, and each node
/// it fed gets a receiver by its inputs (wires into one node share one).
/// Returns the receivers; `None` if `id` isn't a plain reroute.
pub fn make_from_reroute(
    graph: &mut Graph,
    ctx: &mut Ctx,
    id: NodeId,
    places: Places,
) -> Option<Vec<NodeId>> {
    let node = graph.node(id)?;
    if node.kind != REROUTE_KIND || tag(node).is_some() {
        return None;
    }
    let source = graph
        .wire_into(&Endpoint::new(id, "in"))
        .map(|w| w.from.clone());
    let base = match &source {
        Some(s) => default_name(graph, ctx, s),
        None => "portal".into(),
    };
    let name = unique_name(&base, &names_in_use(graph, &BTreeSet::new()));
    set_tag(
        graph,
        id,
        Some(&Tag {
            name: name.clone(),
            send: true,
        }),
    );
    let outgoing: Vec<Wire> = graph.wires_from(id).cloned().collect();
    let mut by_node: Vec<(NodeId, Vec<Wire>)> = Vec::new();
    for w in outgoing {
        match by_node.iter_mut().find(|(n, _)| *n == w.to.node) {
            Some((_, v)) => v.push(w),
            None => by_node.push((w.to.node, vec![w])),
        }
    }
    let mut receivers = Vec::new();
    for (_, wires) in by_node {
        // Placed by the topmost of the node's inputs it feeds.
        let p = wires
            .iter()
            .map(|w| place(graph, places, &w.to, true))
            .min_by(|a, b| a[1].total_cmp(&b[1]))
            .unwrap_or([0.0, 0.0]);
        let r = new_tag(
            graph,
            ctx,
            [p[0] - GAP, p[1]],
            &Tag {
                name: name.clone(),
                send: false,
            },
        );
        for w in &wires {
            graph.wires.retain(|x| x != w);
            graph.connect(Endpoint::new(r, "out"), w.to.clone());
        }
        graph.connect(Endpoint::new(id, "out"), Endpoint::new(r, "in"));
        receivers.push(r);
    }
    Some(receivers)
}

/// Turn portals back into plain wires. A sender takes all its receivers
/// with it; a receiver goes alone, and its sender with it once no receiver
/// is left. Each destination is wired straight from the source again.
/// Returns the tags removed.
pub fn remove(graph: &mut Graph, ids: &BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    let mut gone = BTreeSet::new();
    for &id in ids {
        let Some(t) = tag_of(graph, id) else { continue };
        if t.send {
            gone.insert(id);
            gone.extend(receivers_of(graph, id));
        } else {
            gone.insert(id);
        }
    }
    // Senders left with no receivers go too.
    let senders: Vec<NodeId> = gone.iter().filter_map(|r| sender_of(graph, *r)).collect();
    for s in senders {
        if receivers_of(graph, s).iter().all(|r| gone.contains(r)) {
            gone.insert(s);
        }
    }
    if gone.is_empty() {
        return gone;
    }
    let mut rewire = Vec::new();
    for w in &graph.wires {
        if gone.contains(&w.from.node)
            && !gone.contains(&w.to.node)
            && let Some(src) = upstream(graph, w.from.node, &gone, 0)
        {
            rewire.push((src, w.to.clone()));
        }
    }
    graph.nodes.retain(|n| !gone.contains(&n.id));
    graph
        .wires
        .retain(|w| !gone.contains(&w.from.node) && !gone.contains(&w.to.node));
    for (from, to) in rewire {
        graph.connect(from, to);
    }
    gone
}

/// What really feeds tag `id` once the tags in `gone` are taken out: the
/// output feeding it, looking through removed tags and through the sender
/// of a receiver.
fn upstream(graph: &Graph, id: NodeId, gone: &BTreeSet<NodeId>, depth: usize) -> Option<Endpoint> {
    if depth > 64 {
        return None;
    }
    let src = graph.wire_into(&Endpoint::new(id, "in"))?.from.clone();
    let through_sender = is_receiver(graph, id) && is_sender(graph, src.node);
    if gone.contains(&src.node) || through_sender {
        upstream(graph, src.node, gone, depth + 1)
    } else {
        Some(src)
    }
}

/// Rename the portal `id` belongs to (its sender and every receiver).
/// Another portal's name gets a number added. Returns the name it got;
/// `None` if `id` isn't a portal or the name is empty.
pub fn rename(graph: &mut Graph, id: NodeId, name: &str) -> Option<String> {
    let name = clean_name(name)?;
    let members: BTreeSet<NodeId> = members(graph, id).into_iter().collect();
    if members.is_empty() {
        return None;
    }
    let name = unique_name(&name, &names_in_use(graph, &members));
    for m in &members {
        if let Some(t) = tag_of(graph, *m) {
            set_tag(
                graph,
                *m,
                Some(&Tag {
                    name: name.clone(),
                    send: t.send,
                }),
            );
        }
    }
    Some(name)
}

/// Keep portals sane after `new` nodes were pasted or duplicated into
/// `graph`: a pasted sender whose name another portal already uses gets a
/// fresh one (with the receivers pasted along with it), and a pasted
/// receiver that came without its sender joins the sender of its name, if
/// the graph has one.
pub fn after_paste(graph: &mut Graph, new: &[NodeId]) {
    for &id in new {
        let Some(t) = tag_of(graph, id).filter(|t| t.send) else {
            continue;
        };
        let group: BTreeSet<NodeId> = members(graph, id).into_iter().collect();
        let others = names_in_use(graph, &group);
        if others.iter().any(|n| n.eq_ignore_ascii_case(&t.name)) {
            let fresh = unique_name(&t.name, &others);
            rename(graph, id, &fresh);
        }
    }
    for &id in new {
        let Some(t) = tag_of(graph, id).filter(|t| !t.send) else {
            continue;
        };
        if graph.wire_into(&Endpoint::new(id, "in")).is_some() {
            continue;
        }
        let sender = graph
            .nodes
            .iter()
            .filter(|n| n.id != id)
            .find(|n| tag(n).is_some_and(|s| s.send && s.name.eq_ignore_ascii_case(&t.name)))
            .map(|n| n.id);
        if let Some(s) = sender {
            graph.connect(Endpoint::new(s, "out"), Endpoint::new(id, "in"));
        }
    }
}

/// The wires between `ids` that could become portals (not portal links).
pub fn wires_between(graph: &Graph, ids: &BTreeSet<NodeId>) -> Vec<Wire> {
    graph
        .wires
        .iter()
        .filter(|w| ids.contains(&w.from.node) && ids.contains(&w.to.node))
        .filter(|w| !is_link(graph, w))
        .filter(|w| !(is_virtual(w.from.node) && is_virtual(w.to.node)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{self, connect, edit_scope};
    use ac_core::{IdGen, Project, Registry};

    fn reg() -> Registry {
        ac_nodes::registry()
    }

    fn ep(id: NodeId, s: &str) -> Endpoint {
        Endpoint::new(id, s)
    }

    fn no_places(_: &Endpoint, _: bool) -> Option<[f32; 2]> {
        None
    }

    /// constant → gain, and the constant also → a second gain.
    fn fan() -> (Project, NodeId, NodeId, NodeId) {
        let r = reg();
        let mut p = Project::default();
        let mut ids = IdGen::new(1);
        let (c, a, b) = edit_scope(&mut p, None, &r, &mut ids, |g, ctx| {
            let c = add_node(g, ctx, "constant", ac_core::empty_config(), [0.0, 0.0]);
            let a = add_node(g, ctx, "gain", ac_core::empty_config(), [900.0, 0.0]);
            let b = add_node(g, ctx, "gain", ac_core::empty_config(), [900.0, 400.0]);
            connect(g, ctx, ep(c, "out"), ep(a, "in")).unwrap();
            connect(g, ctx, ep(c, "out"), ep(b, "in")).unwrap();
            (c, a, b)
        });
        (p, c, a, b)
    }

    fn root<R>(p: &mut Project, f: impl FnOnce(&mut Graph, &mut Ctx) -> R) -> R {
        let r = reg();
        let mut ids = IdGen::new(77);
        edit_scope(p, None, &r, &mut ids, f)
    }

    /// What a destination input is fed by, looking through every reroute
    /// (what the engine sees).
    fn source(g: &Graph, to: Endpoint) -> Option<Endpoint> {
        let w = g.wire_into(&to)?;
        Some(edit::resolve_source(g, &w.from))
    }

    #[test]
    fn a_wire_becomes_a_portal_and_back() {
        let (mut p, c, a, b) = fan();
        let wire = Wire::new(ep(c, "out"), ep(a, "in"));
        let (s, r) = root(&mut p, |g, ctx| {
            make_from_wire(g, ctx, &wire, &|ep, _| {
                (ep.node == a).then_some([900.0, 50.0])
            })
        })
        .unwrap();
        let g = &p.graph;
        // Plain reroutes and wires underneath: the signal is unchanged.
        assert_eq!(source(g, ep(a, "in")), Some(ep(c, "out")));
        assert_eq!(source(g, ep(b, "in")), Some(ep(c, "out")));
        assert!(g.wires.contains(&Wire::new(ep(c, "out"), ep(s, "in"))));
        assert!(g.wires.contains(&Wire::new(ep(r, "out"), ep(a, "in"))));
        let link = Wire::new(ep(s, "out"), ep(r, "in"));
        assert!(is_link(g, &link), "the link is the hidden wire");
        assert_eq!(
            tag_of(g, s),
            Some(Tag {
                name: "Constant".into(),
                send: true
            })
        );
        assert_eq!(tag_of(g, r).map(|t| t.send), Some(false));
        // The receiver sits left of the input it feeds.
        assert_eq!(g.node(r).unwrap().ui.pos, [900.0 - GAP, 50.0]);
        let info = index(g);
        assert_eq!(info[&s].partners, vec![r]);
        assert_eq!(info[&r].partners, vec![s]);
        assert_eq!(members(g, r), vec![s, r]);

        // Back to a wire: the reroutes go and the source feeds `a` again.
        let gone = root(&mut p, |g, _| remove(g, &[s].into()));
        assert_eq!(gone, [s, r].into());
        let g = &p.graph;
        assert!(g.wires.contains(&Wire::new(ep(c, "out"), ep(a, "in"))));
        assert!(g.wires.contains(&Wire::new(ep(c, "out"), ep(b, "in"))));
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.wires.len(), 2);
    }

    #[test]
    fn one_source_has_one_sender_and_several_receivers() {
        let (mut p, c, a, b) = fan();
        let (s1, r1) = root(&mut p, |g, ctx| {
            make_from_wire(g, ctx, &Wire::new(ep(c, "out"), ep(a, "in")), &no_places)
        })
        .unwrap();
        let (s2, r2) = root(&mut p, |g, ctx| {
            make_from_wire(g, ctx, &Wire::new(ep(c, "out"), ep(b, "in")), &no_places)
        })
        .unwrap();
        assert_eq!(s1, s2, "the second wire reuses the sender");
        let g = &p.graph;
        assert_eq!(receivers_of(g, s1), vec![r1, r2]);
        assert_eq!(g.wires_from(c).count(), 1, "one wire leaves the source");
        assert_eq!(source(g, ep(b, "in")), Some(ep(c, "out")));

        // One receiver back to a wire: the other stays, fed by the sender,
        // and the destination is wired from the source, not the tag.
        root(&mut p, |g, _| remove(g, &[r1].into()));
        let g = &p.graph;
        assert!(g.wires.contains(&Wire::new(ep(c, "out"), ep(a, "in"))));
        assert_eq!(receivers_of(g, s1), vec![r2]);
        // The last receiver takes its sender with it.
        root(&mut p, |g, _| remove(g, &[r2].into()));
        let g = &p.graph;
        assert_eq!(g.nodes.len(), 3);
        assert!(g.wires.contains(&Wire::new(ep(c, "out"), ep(b, "in"))));
    }

    #[test]
    fn a_reroute_becomes_a_sender_with_a_receiver_per_node() {
        let (mut p, c, a, b) = fan();
        let rr = root(&mut p, |g, ctx| {
            edit::add_reroutes(
                g,
                ctx,
                &[
                    (Wire::new(ep(c, "out"), ep(a, "in")), [300.0, 0.0]),
                    (Wire::new(ep(c, "out"), ep(b, "in")), [300.0, 40.0]),
                ],
            )
        });
        let [rr] = rr.as_slice() else {
            panic!("{rr:?}")
        };
        let receivers = root(&mut p, |g, ctx| make_from_reroute(g, ctx, *rr, &no_places)).unwrap();
        assert_eq!(receivers.len(), 2);
        let g = &p.graph;
        assert!(is_sender(g, *rr));
        assert_eq!(receivers_of(g, *rr), receivers);
        assert_eq!(source(g, ep(a, "in")), Some(ep(c, "out")));
        assert_eq!(source(g, ep(b, "in")), Some(ep(c, "out")));
        // Already a portal: nothing to do.
        assert!(root(&mut p, |g, ctx| make_from_reroute(g, ctx, *rr, &no_places)).is_none());
    }

    #[test]
    fn names_stay_unique_and_renaming_covers_the_whole_portal() {
        let (mut p, c, a, b) = fan();
        let (s1, r1) = root(&mut p, |g, ctx| {
            make_from_wire(g, ctx, &Wire::new(ep(c, "out"), ep(a, "in")), &no_places)
        })
        .unwrap();
        let name = root(&mut p, |g, _| rename(g, r1, "  Bass   line ")).unwrap();
        assert_eq!(name, "Bass line");
        assert_eq!(tag_of(&p.graph, s1).unwrap().name, "Bass line");
        // A second source's portal can't take the same name.
        let (s2, _) = root(&mut p, |g, ctx| {
            let k = add_node(g, ctx, "constant", ac_core::empty_config(), [0.0, 300.0]);
            connect(g, ctx, ep(k, "out"), ep(b, "gain")).unwrap();
            make_from_wire(g, ctx, &Wire::new(ep(k, "out"), ep(b, "gain")), &no_places)
        })
        .unwrap();
        assert_ne!(s1, s2);
        let n2 = root(&mut p, |g, _| rename(g, s2, "bass LINE")).unwrap();
        assert_eq!(n2, "bass LINE 2");
        // Renaming to its own name keeps it.
        assert_eq!(
            root(&mut p, |g, _| rename(g, s1, "Bass line")).unwrap(),
            "Bass line"
        );
        assert!(root(&mut p, |g, _| rename(g, s1, "   ")).is_none());
        assert!(
            root(&mut p, |g, _| rename(g, c, "x")).is_none(),
            "not a portal"
        );
        assert_eq!(unique_name("a", &["a".into(), "a 2".into()]), "a 3");
    }

    #[test]
    fn copy_and_paste_keep_portals_sane() {
        let (mut p, c, a, _) = fan();
        let (s, r) = root(&mut p, |g, ctx| {
            make_from_wire(g, ctx, &Wire::new(ep(c, "out"), ep(a, "in")), &no_places)
        })
        .unwrap();
        // A receiver pasted alone joins the sender of its name: one more
        // receiver of the same portal.
        let text = edit::copy(&p.graph, &p.group_defs, &[r].into());
        let new = root(&mut p, |g, ctx| edit::paste(g, ctx, &text, [0.0, 600.0])).unwrap();
        let [r2] = new.as_slice() else { panic!() };
        assert_eq!(sender_of(&p.graph, *r2), Some(s));
        assert_eq!(receivers_of(&p.graph, s).len(), 2);

        // The whole portal pasted: a new pair with its own name, linked to
        // itself, not to the original.
        let text = edit::copy(&p.graph, &p.group_defs, &[s, r].into());
        let new = root(&mut p, |g, ctx| edit::paste(g, ctx, &text, [0.0, 900.0])).unwrap();
        let g = &p.graph;
        let s3 = *new.iter().find(|id| is_sender(g, **id)).unwrap();
        let r3 = *new.iter().find(|id| is_receiver(g, **id)).unwrap();
        assert_eq!(sender_of(g, r3), Some(s3));
        assert_eq!(tag_of(g, s3).unwrap().name, "Constant 2");
        assert_eq!(index(g)[&r3].name, "Constant 2");
        assert_eq!(receivers_of(g, s).len(), 2, "the original is untouched");

        // Duplicating a sender alone gives a fresh name, no receivers.
        let new = root(&mut p, |g, ctx| {
            edit::duplicate(g, ctx, &[s].into(), true, [30.0, 30.0])
        });
        let g = &p.graph;
        let [s4] = new.as_slice() else { panic!() };
        assert_eq!(tag_of(g, *s4).unwrap().name, "Constant 3");
        assert!(receivers_of(g, *s4).is_empty());
        assert_eq!(
            source(g, ep(*s4, "in")),
            Some(ep(c, "out")),
            "kept its input"
        );
    }

    #[test]
    fn portals_work_inside_groups() {
        let (mut p, c, a, _) = fan();
        let r = reg();
        let mut ids = IdGen::new(5);
        let (inst, def) = edit_scope(&mut p, None, &r, &mut ids, |g, ctx| {
            edit::group::make_group(g, ctx, &[c, a].into())
        })
        .unwrap();
        // Inside the definition: the constant feeds the gain and the group
        // output; make a portal of the wire to the gain.
        let (s, rcv) = edit_scope(&mut p, Some(&def), &r, &mut ids, |g, ctx| {
            make_from_wire(g, ctx, &Wire::new(ep(c, "out"), ep(a, "in")), &no_places)
        })
        .unwrap();
        let inner = &p.group_defs[&def].graph;
        assert_eq!(sender_of(inner, rcv), Some(s));
        assert_eq!(source(inner, ep(a, "in")), Some(ep(c, "out")));
        let _ = inst;
        // A portal into the group's output (a virtual wire inside).
        let outputs = p.group_defs[&def].outputs.len();
        edit_scope(&mut p, Some(&def), &r, &mut ids, |g, ctx| {
            let w = g
                .wires
                .iter()
                .find(|w| w.to.node == crate::edit::GROUP_OUT)
                .cloned();
            if let Some(w) = w {
                make_from_wire(g, ctx, &w, &no_places).unwrap();
            }
        });
        let d = &p.group_defs[&def];
        assert_eq!(d.outputs.len(), outputs);
        // The group output is now fed by a receiver, whose sender is fed by
        // the constant.
        let out_src = &d.outputs[0].source;
        assert!(is_receiver(&d.graph, out_src.node), "{out_src:?}");
        assert_eq!(edit::resolve_source(&d.graph, out_src), ep(c, "out"));
    }

    #[test]
    fn a_portal_renders_exactly_like_its_wire() {
        use std::sync::Arc;
        let render = |p: &Project| {
            let r = ac_engine::render(
                p,
                Arc::new(reg()),
                Arc::new(ac_core::NoResources),
                &ac_engine::RenderOptions {
                    to: Some(0.01),
                    ..Default::default()
                },
            );
            assert!(!r.report.has_errors(), "{}", r.report);
            r.channels
        };
        let mut p = Project::default();
        let (osc, out) = root(&mut p, |g, ctx| {
            let osc = add_node(g, ctx, "oscillator", ac_core::empty_config(), [0.0, 0.0]);
            let out = add_node(g, ctx, "audio_out", ac_core::empty_config(), [900.0, 0.0]);
            connect(g, ctx, ep(osc, "out"), ep(out, "in")).unwrap();
            (osc, out)
        });
        let before = render(&p);
        assert!(before[0].iter().any(|s| s.abs() > 0.1), "it plays");
        root(&mut p, |g, ctx| {
            make_from_wire(
                g,
                ctx,
                &Wire::new(ep(osc, "out"), ep(out, "in")),
                &no_places,
            )
        })
        .unwrap();
        // The project file says what a portal is, next to plain reroutes.
        let text = serde_json::to_string(&p).unwrap();
        assert!(
            text.contains(r#""portal":{"name":"Oscillator out","role":"send"}"#),
            "{text}"
        );
        assert_eq!(render(&p), before);
    }

    #[test]
    fn bad_tags_and_wires_are_ignored() {
        let (mut p, c, a, _) = fan();
        // A tag on something that isn't a reroute, or with a role nobody
        // knows, is no portal.
        p.graph.node_mut(c).unwrap().extra.insert(
            PORTAL.into(),
            serde_json::json!({"name": "x", "role": "send"}),
        );
        assert!(tag_of(&p.graph, c).is_none());
        let none = root(&mut p, |g, ctx| {
            let x = add_node(g, ctx, REROUTE_KIND, ac_core::empty_config(), [0.0, 0.0]);
            g.node_mut(x)
                .unwrap()
                .extra
                .insert(PORTAL.into(), serde_json::json!({"role": "sideways"}));
            tag_of(g, x)
        });
        assert!(none.is_none());
        // A wire that isn't there.
        let missing = Wire::new(ep(a, "out"), ep(c, "in"));
        assert!(
            root(&mut p, |g, ctx| make_from_wire(
                g, ctx, &missing, &no_places
            ))
            .is_none()
        );
        // Removing things that aren't portals does nothing.
        assert!(root(&mut p, |g, _| remove(g, &[c, a].into())).is_empty());
    }
}
