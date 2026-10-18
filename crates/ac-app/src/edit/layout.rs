//! Auto-layout (`Ctrl+L`): a layered drawing, left to right, after
//! K. Sugiyama, S. Tagawa, and M. Toda, "Methods for Visual Understanding
//! of Hierarchical System Structures", IEEE Trans. Systems, Man, and
//! Cybernetics 11(2), 1981:
//!
//! 1. **Break cycles.** Loops only legally pass through `Feedback` nodes
//!    (D4), so a wire leaving a Feedback node on a loop is turned round
//!    first; any loop still left (an error the compiler reports) loses the
//!    wires a depth-first search finds going back.
//! 2. **Layers** by longest path from the sources; then, from the sinks
//!    back, each node with no more wires in than out moves right to just
//!    before the first node it feeds, so a source or a modulator sits next
//!    to what it drives (its wires get no longer overall).
//! 3. **Order** each layer to reduce crossings: wires spanning several
//!    layers get a dummy point in each, and layers are sorted by the
//!    barycenter of their neighbours' sockets, sweeping down and up from
//!    the current top-to-bottom order and keeping the order with the
//!    fewest crossings.
//! 4. **Place**: a column per layer, as wide as its widest card. Within a
//!    column the cards keep their order and move as close as they can to
//!    the sockets they're wired to: a least-squares fit under the spacing
//!    constraints, which pool-adjacent-violators solves exactly. A node
//!    wired to many pulls each of them less, so a Mix's inputs line up
//!    with what feeds them rather than bunching at the Mix.
//!
//! A frame whose nodes are all laid out is laid out as one block: its
//! insides first, then the block among its neighbours, so the frame stays
//! around its contents (D102). Separate pieces of the graph stack top to
//! bottom in the order they were in; unwired cards (notes, mostly) line up
//! in a row above them or below, on the side they were on. The result
//! keeps the top-left corner where it was, in whole canvas units.

use std::collections::{BTreeMap, HashMap};

use ac_core::{NodeId, Wire};
use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::theme;

/// Room between columns, for the wires.
pub const COLUMN_GAP: f32 = 80.0;
/// Room between cards in a column.
pub const ROW_GAP: f32 = 30.0;
/// Room between separate pieces of the graph, and above the row of
/// unwired cards.
pub const PIECE_GAP: f32 = 60.0;
/// Room between unwired cards in their row.
const ISOLATED_GAP: f32 = 40.0;
/// Height kept for a wire passing through a column, and the room around it.
const DUMMY_HEIGHT: f32 = 12.0;
const DUMMY_GAP: f32 = 8.0;
/// Crossing-reduction sweeps (alternately down and up).
const ORDER_SWEEPS: usize = 12;
/// Placement sweeps (down, up, …, then both ways once).
const PLACE_SWEEPS: usize = 8;
/// A frame never gets narrower than this.
const MIN_FRAME_WIDTH: f32 = 120.0;

/// A card to lay out, as the canvas draws it.
#[derive(Clone, Debug)]
pub struct Card {
    pub id: NodeId,
    /// Where it is now (canvas units).
    pub rect: Rect,
    /// Each input socket and its height below the top of the card.
    pub inputs: Vec<(String, f32)>,
    /// Each output socket and its height below the top of the card.
    pub outputs: Vec<(String, f32)>,
    /// A `Feedback` node: the wires out of it close loops.
    pub feedback: bool,
}

/// A frame that moves with its contents.
#[derive(Clone, Debug)]
pub struct FrameBox {
    /// Its index in the graph's frames.
    pub index: usize,
    pub rect: Rect,
}

/// Where everything goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrangement {
    /// Each card's new top-left corner.
    pub cards: Vec<(NodeId, Pos2)>,
    /// Each frame's new rectangle.
    pub frames: Vec<(usize, Rect)>,
}

/// Lay out `cards`, wired by those `wires` that join two of them, with
/// `frames` (each holding the cards whose centers it contains) moving and
/// resizing to stay around their contents.
pub fn arrange(cards: &[Card], wires: &[Wire], frames: &[FrameBox]) -> Arrangement {
    if cards.is_empty() {
        return Arrangement::default();
    }
    let cards: Vec<Card> = cards.iter().map(sanitized).collect();
    let frames: Vec<FrameBox> = frames
        .iter()
        .filter(|f| f.rect.is_finite())
        .cloned()
        .collect();
    let index: HashMap<NodeId, usize> = cards.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    let links: Vec<Link> = wires
        .iter()
        .filter_map(|w| {
            let a = *index.get(&w.from.node)?;
            let b = *index.get(&w.to.node)?;
            if a == b {
                return None;
            }
            Some(Link {
                from: a,
                to: b,
                out_y: port(&cards[a].outputs, &w.from.socket, cards[a].rect.height()),
                in_y: port(&cards[b].inputs, &w.to.socket, cards[b].rect.height()),
                weak: cards[a].feedback,
            })
        })
        .collect();
    let tree = Tree::new(&cards, &frames);
    let placed = tree.place(None, &cards, &frames, &links, 0);
    let anchor = cards
        .iter()
        .map(|c| c.rect)
        .chain(
            placed
                .frames
                .iter()
                .filter_map(|(f, _)| frames.get(*f).map(|f| f.rect)),
        )
        .reduce(|a, b| a.union(b))
        .map_or(Pos2::ZERO, |r| r.min)
        .to_vec2();
    // Whole canvas units keep project files tidy; every gap is wider than
    // the rounding.
    let mut out = Arrangement::default();
    for (i, c) in cards.iter().enumerate() {
        let at = placed.cards.get(&i).copied().unwrap_or(Vec2::ZERO);
        out.cards.push((c.id, (at + anchor).to_pos2().round()));
    }
    for (f, r) in &placed.frames {
        if let Some(fb) = frames.get(*f) {
            let r = r.translate(anchor);
            out.frames
                .push((fb.index, Rect::from_min_max(r.min.round(), r.max.round())));
        }
    }
    out
}

/// A card with a usable rectangle (a damaged file can hold anything).
fn sanitized(c: &Card) -> Card {
    let mut c = c.clone();
    if !c.rect.is_finite() || c.rect.width() < 0.0 || c.rect.height() < 0.0 {
        c.rect = Rect::from_min_size(Pos2::ZERO, vec2(theme::NODE_WIDTH, 80.0));
    }
    c
}

/// A socket's height below the top of its card (the middle if unknown).
fn port(list: &[(String, f32)], name: &str, height: f32) -> f32 {
    list.iter()
        .find(|(n, _)| n == name)
        .map(|(_, y)| *y)
        .filter(|y| y.is_finite())
        .unwrap_or(height / 2.0)
}

/// A wire between two cards (indices into the cards).
#[derive(Clone, Copy, Debug)]
struct Link {
    from: usize,
    to: usize,
    out_y: f32,
    in_y: f32,
    weak: bool,
}

/// One thing a block lays out: a card, or a frame laid out as a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Member {
    Card(usize),
    Frame(usize),
}

/// Which frame holds each card and each frame.
struct Tree {
    card_parent: Vec<Option<usize>>,
    frame_parent: Vec<Option<usize>>,
    /// Frames holding at least one card (directly or through others).
    used: Vec<bool>,
}

impl Tree {
    fn new(cards: &[Card], frames: &[FrameBox]) -> Tree {
        let area = |r: Rect| r.width() * r.height();
        // The smallest frame holding a point; frames compare by area, then
        // by index, so nesting can't loop.
        let smallest = |p: Pos2, above: Option<usize>| -> Option<usize> {
            frames
                .iter()
                .enumerate()
                .filter(|(i, f)| {
                    f.rect.contains(p)
                        && above.is_none_or(|a| {
                            let (fa, aa) = (area(f.rect), area(frames[a].rect));
                            fa > aa || (fa == aa && *i < a)
                        })
                        && Some(*i) != above
                })
                .min_by(|(i, a), (j, b)| area(a.rect).total_cmp(&area(b.rect)).then(j.cmp(i)))
                .map(|(i, _)| i)
        };
        let card_parent: Vec<Option<usize>> = cards
            .iter()
            .map(|c| smallest(c.rect.center(), None))
            .collect();
        let frame_parent: Vec<Option<usize>> = (0..frames.len())
            .map(|f| smallest(frames[f].rect.center(), Some(f)))
            .collect();
        let mut used = vec![false; frames.len()];
        for p in &card_parent {
            let mut at = *p;
            // At most one step per frame: the parent order has no loops.
            for _ in 0..=frames.len() {
                let Some(f) = at else { break };
                if used[f] {
                    break;
                }
                used[f] = true;
                at = frame_parent[f];
            }
        }
        Tree {
            card_parent,
            frame_parent,
            used,
        }
    }

    /// What `block` (a frame, or the root) lays out directly.
    fn members(&self, block: Option<usize>) -> Vec<Member> {
        let cards = (0..self.card_parent.len())
            .filter(|i| self.card_parent[*i] == block)
            .map(Member::Card);
        let frames = (0..self.frame_parent.len())
            .filter(|f| self.used[*f] && self.frame_parent[*f] == block && Some(*f) != block)
            .map(Member::Frame);
        cards.chain(frames).collect()
    }

    /// The member of `block` that holds `card`, if `block` holds it.
    fn member_in(&self, block: Option<usize>, card: usize) -> Option<Member> {
        let mut m = Member::Card(card);
        let mut parent = self.card_parent.get(card).copied().flatten();
        for _ in 0..=self.frame_parent.len() {
            if parent == block {
                return Some(m);
            }
            let f = parent?;
            m = Member::Frame(f);
            parent = self.frame_parent.get(f).copied().flatten();
        }
        None
    }

    /// Lay out a block: its frames first, then its members.
    fn place(
        &self,
        block: Option<usize>,
        cards: &[Card],
        frames: &[FrameBox],
        links: &[Link],
        depth: usize,
    ) -> Placed {
        let members = self.members(block);
        let mut inner: HashMap<usize, Placed> = HashMap::new();
        if depth <= frames.len() {
            for m in &members {
                if let Member::Frame(f) = m {
                    inner.insert(*f, self.place(Some(*f), cards, frames, links, depth + 1));
                }
            }
        }
        let local: HashMap<Member, usize> =
            members.iter().enumerate().map(|(i, m)| (*m, i)).collect();
        let nodes: Vec<Node> = members
            .iter()
            .map(|m| match m {
                Member::Card(i) => Node {
                    size: cards[*i].rect.size(),
                    seed: cards[*i].rect.center(),
                },
                Member::Frame(f) => Node {
                    size: inner.get(f).map_or(Vec2::ZERO, |p| p.size),
                    seed: frames[*f].rect.center(),
                },
            })
            .collect();
        // Where a card's socket sits inside the member holding it.
        let offset = |m: Member, card: usize| -> f32 {
            match m {
                Member::Card(_) => 0.0,
                Member::Frame(f) => inner
                    .get(&f)
                    .and_then(|p| p.cards.get(&card))
                    .map_or(0.0, |v| v.y),
            }
        };
        let edges: Vec<Edge> = links
            .iter()
            .filter_map(|l| {
                let a = self.member_in(block, l.from)?;
                let b = self.member_in(block, l.to)?;
                if a == b {
                    return None;
                }
                Some(Edge {
                    from: *local.get(&a)?,
                    to: *local.get(&b)?,
                    out_y: l.out_y + offset(a, l.from),
                    in_y: l.in_y + offset(b, l.to),
                    weak: l.weak,
                })
            })
            .collect();
        let (positions, size) = layered(&nodes, &edges);
        let pad = if block.is_some() {
            vec2(theme::FRAME_PAD, theme::FRAME_PAD + theme::FRAME_HEADER)
        } else {
            Vec2::ZERO
        };
        let mut placed = Placed::default();
        for (m, p) in members.iter().zip(&positions) {
            let at = p.to_vec2() + pad;
            match m {
                Member::Card(i) => {
                    placed.cards.insert(*i, at);
                }
                Member::Frame(f) => {
                    if let Some(sub) = inner.remove(f) {
                        placed
                            .frames
                            .push((*f, Rect::from_min_size(at.to_pos2(), sub.size)));
                        for (c, v) in sub.cards {
                            placed.cards.insert(c, v + at);
                        }
                        for (g, r) in sub.frames {
                            placed.frames.push((g, r.translate(at)));
                        }
                    }
                }
            }
        }
        placed.size = if block.is_some() {
            let s = size + vec2(2.0 * pad.x, pad.y + theme::FRAME_PAD);
            vec2(s.x.max(MIN_FRAME_WIDTH), s.y)
        } else {
            size
        };
        placed
    }
}

/// A block laid out: its size and where its cards and frames go, relative
/// to its top-left corner.
#[derive(Default)]
struct Placed {
    size: Vec2,
    cards: BTreeMap<usize, Vec2>,
    frames: Vec<(usize, Rect)>,
}

/// Something to lay out in a layer: its size and where it was (for the
/// starting order).
#[derive(Clone, Copy, Debug)]
struct Node {
    size: Vec2,
    seed: Pos2,
}

/// A wire between two nodes, with its sockets' heights on each.
#[derive(Clone, Copy, Debug)]
struct Edge {
    from: usize,
    to: usize,
    out_y: f32,
    in_y: f32,
    weak: bool,
}

/// Lay out `nodes`: each connected piece on its own, stacked top to bottom
/// in the order they were, and the unwired ones in a row above them (those
/// that were above the rest) and a row below (the others). Returns each
/// node's top-left corner and the size of the whole.
fn layered(nodes: &[Node], edges: &[Edge]) -> (Vec<Pos2>, Vec2) {
    let mut out = vec![Pos2::ZERO; nodes.len()];
    let mut pieces = components(nodes.len(), edges);
    let isolated: Vec<usize> = pieces
        .iter()
        .filter(|p| p.len() == 1)
        .flatten()
        .copied()
        .collect();
    pieces.retain(|p| p.len() > 1);
    let top_left = |p: &[usize]| {
        p.iter().fold(pos2(f32::INFINITY, f32::INFINITY), |a, &i| {
            let r = Rect::from_center_size(nodes[i].seed, nodes[i].size);
            pos2(a.x.min(r.min.x), a.y.min(r.min.y))
        })
    };
    pieces.sort_by(|a, b| {
        let (pa, pb) = (top_left(a), top_left(b));
        pa.y.total_cmp(&pb.y).then(pa.x.total_cmp(&pb.x))
    });
    // Notes and loose nodes keep their side: above the wired pieces' middle
    // goes in the top row.
    let wired: Vec<f32> = pieces.iter().flatten().map(|i| nodes[*i].seed.y).collect();
    let middle = if wired.is_empty() {
        f32::INFINITY
    } else {
        wired.iter().sum::<f32>() / wired.len() as f32
    };
    let (mut above, mut below): (Vec<usize>, Vec<usize>) = isolated
        .into_iter()
        .partition(|i| nodes[*i].seed.y < middle);
    let by_x = |a: &usize, b: &usize| {
        let (pa, pb) = (nodes[*a].seed, nodes[*b].seed);
        pa.x.total_cmp(&pb.x)
            .then(pa.y.total_cmp(&pb.y))
            .then(a.cmp(b))
    };
    above.sort_by(by_x);
    below.sort_by(by_x);
    let mut y = 0.0f32;
    let mut size = Vec2::ZERO;
    let mut stack = |block: Vec2, y: &mut f32| {
        size = vec2(size.x.max(block.x), *y + block.y);
        *y += block.y + PIECE_GAP;
    };
    if !above.is_empty() {
        stack(row(nodes, &above, y, &mut out), &mut y);
    }
    for piece in &pieces {
        let (pos, piece_size) = layer_piece(nodes, edges, piece);
        for (i, p) in piece.iter().zip(pos) {
            out[*i] = p + vec2(0.0, y);
        }
        stack(piece_size, &mut y);
    }
    if !below.is_empty() {
        stack(row(nodes, &below, y, &mut out), &mut y);
    }
    (out, size)
}

/// Put `list` side by side, left to right, at height `y`; returns the
/// row's size.
fn row(nodes: &[Node], list: &[usize], y: f32, out: &mut [Pos2]) -> Vec2 {
    let mut x = 0.0f32;
    let mut tallest = 0.0f32;
    for i in list {
        out[*i] = pos2(x, y);
        x += nodes[*i].size.x + ISOLATED_GAP;
        tallest = tallest.max(nodes[*i].size.y);
    }
    vec2((x - ISOLATED_GAP).max(0.0), tallest)
}

/// Connected pieces (wires taken either way), each sorted, in order of
/// their first node.
fn components(n: usize, edges: &[Edge]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for e in edges {
        let (a, b) = (root(&mut parent, e.from), root(&mut parent, e.to));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    groups.into_values().collect()
}

/// A point in a layer: a node, or a dummy where a long wire passes.
#[derive(Clone, Copy, Debug)]
struct Point {
    size: Vec2,
    layer: usize,
    /// Starting order within the layer.
    seed: f32,
    real: bool,
}

/// A wire piece between points in neighbouring layers, with its heights.
#[derive(Clone, Copy, Debug)]
struct Seg {
    upper: usize,
    lower: usize,
    upper_y: f32,
    lower_y: f32,
}

/// Lay out one connected piece; returns each of its nodes' top-left
/// corners (in `piece` order) and the piece's size.
fn layer_piece(nodes: &[Node], edges: &[Edge], piece: &[usize]) -> (Vec<Pos2>, Vec2) {
    let local: HashMap<usize, usize> = piece.iter().enumerate().map(|(k, i)| (*i, k)).collect();
    let n = piece.len();
    let es: Vec<Edge> = edges
        .iter()
        .filter_map(|e| {
            Some(Edge {
                from: *local.get(&e.from)?,
                to: *local.get(&e.to)?,
                ..*e
            })
        })
        .collect();
    let seed = |k: usize| nodes[piece[k]].seed;
    // Starting order: top to bottom, then left to right, as they were.
    let mut by_seed: Vec<usize> = (0..n).collect();
    by_seed.sort_by(|a, b| {
        let (pa, pb) = (seed(*a), seed(*b));
        pa.y.total_cmp(&pb.y)
            .then(pa.x.total_cmp(&pb.x))
            .then(a.cmp(b))
    });

    // 1. Break cycles.
    let reversed = back_edges(n, &es, &by_seed);
    let dag: Vec<Edge> = es
        .iter()
        .zip(&reversed)
        .map(|(e, r)| {
            if *r {
                Edge {
                    from: e.to,
                    to: e.from,
                    out_y: e.in_y,
                    in_y: e.out_y,
                    weak: e.weak,
                }
            } else {
                *e
            }
        })
        .collect();

    // 2. Layers.
    let layer = layers(n, &dag, &by_seed);
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;

    // 3. Dummies for wires spanning several layers.
    let mut points: Vec<Point> = (0..n)
        .map(|k| Point {
            size: nodes[piece[k]].size,
            layer: layer[k],
            seed: seed(k).y,
            real: true,
        })
        .collect();
    let mut segs: Vec<Seg> = Vec::new();
    for e in &dag {
        let (la, lb) = (layer[e.from], layer[e.to]);
        if lb <= la {
            continue; // can't happen after layering; nothing to draw
        }
        let mut prev = e.from;
        let mut prev_y = e.out_y;
        for l in la + 1..lb {
            let t = (l - la) as f32 / (lb - la) as f32;
            let d = points.len();
            points.push(Point {
                size: vec2(0.0, DUMMY_HEIGHT),
                layer: l,
                seed: seed(e.from).y + (seed(e.to).y - seed(e.from).y) * t,
                real: false,
            });
            segs.push(Seg {
                upper: prev,
                lower: d,
                upper_y: prev_y,
                lower_y: DUMMY_HEIGHT / 2.0,
            });
            prev = d;
            prev_y = DUMMY_HEIGHT / 2.0;
        }
        segs.push(Seg {
            upper: prev,
            lower: e.to,
            upper_y: prev_y,
            lower_y: e.in_y,
        });
    }

    // 4. Order each layer.
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for (p, pt) in points.iter().enumerate() {
        layers[pt.layer].push(p);
    }
    for l in &mut layers {
        l.sort_by(|a, b| {
            points[*a]
                .seed
                .total_cmp(&points[*b].seed)
                .then(points[*b].real.cmp(&points[*a].real))
                .then(a.cmp(b))
        });
    }
    let mut ups: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    let mut downs: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (s, seg) in segs.iter().enumerate() {
        downs[seg.upper].push(s);
        ups[seg.lower].push(s);
    }
    let layers = order(&points, &segs, &ups, &downs, layers);

    // 5. Place.
    let ys = place(&points, &segs, &ups, &downs, &layers);
    let mut column_x = Vec::with_capacity(depth);
    let mut x = 0.0f32;
    for l in &layers {
        column_x.push(x);
        let w = l
            .iter()
            .filter(|p| points[**p].real)
            .map(|p| points[*p].size.x)
            .fold(0.0f32, f32::max);
        x += w + COLUMN_GAP;
    }
    let width = (x - COLUMN_GAP).max(0.0);
    let top = (0..n).map(|k| ys[k]).fold(f32::INFINITY, f32::min);
    let top = if top.is_finite() { top } else { 0.0 };
    let bottom = (0..n)
        .map(|k| ys[k] + points[k].size.y)
        .fold(0.0f32, f32::max);
    let pos = (0..n)
        .map(|k| pos2(column_x[layer[k]], ys[k] - top))
        .collect();
    (pos, vec2(width, (bottom - top).max(0.0)))
}

/// Which edges to turn round so no cycle is left: feedback wires on a
/// cycle first, then whatever a depth-first search (from the nodes in
/// `order`) finds going back.
fn back_edges(n: usize, edges: &[Edge], order: &[usize]) -> Vec<bool> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        adj[e.from].push(e.to);
    }
    let scc = strongly_connected(&adj);
    let mut reversed: Vec<bool> = edges
        .iter()
        .map(|e| e.weak && scc[e.from] == scc[e.to])
        .collect();
    // Out-edges still forward, by index, in wire order.
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, e) in edges.iter().enumerate() {
        if !reversed[k] {
            out[e.from].push(k);
        }
    }
    // 0 unseen, 1 on the search path, 2 finished.
    let mut state = vec![0u8; n];
    for &start in order {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&(v, i)) = stack.last() {
            if let Some(&k) = out[v].get(i) {
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                let w = edges[k].to;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[k] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    reversed
}

/// Tarjan's strongly connected components (iteratively, so a long chain
/// can't overflow the stack): a component number per node.
fn strongly_connected(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp = vec![UNSEEN; n];
    let mut next = 0usize;
    let mut count = 0usize;
    for s in 0..n {
        if index[s] != UNSEEN {
            continue;
        }
        let mut calls: Vec<(usize, usize)> = vec![(s, 0)];
        index[s] = next;
        low[s] = next;
        next += 1;
        stack.push(s);
        on_stack[s] = true;
        while let Some(&(v, i)) = calls.last() {
            if let Some(&w) = adj[v].get(i) {
                if let Some(top) = calls.last_mut() {
                    top.1 += 1;
                }
                if index[w] == UNSEEN {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                calls.pop();
                if let Some(&(u, _)) = calls.last() {
                    low[u] = low[u].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        comp[w] = count;
                        if w == v {
                            break;
                        }
                    }
                    count += 1;
                }
            }
        }
    }
    comp
}

/// Longest-path layers. Then, from the sinks back, a node with no more
/// wires in than out moves right to just before the first node it feeds:
/// that shortens its wires overall (or keeps them as long), and keeps a
/// source or a modulator next to what it drives.
fn layers(n: usize, dag: &[Edge], order: &[usize]) -> Vec<usize> {
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in dag {
        preds[e.to].push(e.from);
        succs[e.from].push(e.to);
    }
    let rank: Vec<usize> = {
        let mut r = vec![0; n];
        for (k, &i) in order.iter().enumerate() {
            r[i] = k;
        }
        r
    };
    // Kahn's algorithm, taking ready nodes in their starting order.
    let mut indeg: Vec<usize> = preds.iter().map(Vec::len).collect();
    let mut ready: std::collections::BTreeSet<(usize, usize)> = (0..n)
        .filter(|i| indeg[*i] == 0)
        .map(|i| (rank[i], i))
        .collect();
    let mut layer = vec![0usize; n];
    let mut topo = Vec::with_capacity(n);
    while let Some((_, v)) = ready.pop_first() {
        topo.push(v);
        for &w in &succs[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indeg[w] = indeg[w].saturating_sub(1);
            if indeg[w] == 0 {
                ready.insert((rank[w], w));
            }
        }
    }
    for &v in topo.iter().rev() {
        if preds[v].len() <= succs[v].len()
            && let Some(first) = succs[v].iter().map(|s| layer[*s]).min()
        {
            layer[v] = layer[v].max(first.saturating_sub(1));
        }
    }
    let min = layer.iter().copied().min().unwrap_or(0);
    layer.iter().map(|l| l - min).collect()
}

/// Where a socket sits on a point, as a fraction of its height (dummies:
/// the middle). Below 1, so a node's sockets sort before the next node's.
fn frac(points: &[Point], p: usize, y: f32) -> f32 {
    let h = points[p].size.y;
    if h > 0.0 {
        (y / h).clamp(0.0, 1.0) * 0.98
    } else {
        0.5
    }
}

/// Crossing reduction by barycenters, sweeping down and up; keeps the
/// best ordering seen.
fn order(
    points: &[Point],
    segs: &[Seg],
    ups: &[Vec<usize>],
    downs: &[Vec<usize>],
    mut layers: Vec<Vec<usize>>,
) -> Vec<Vec<usize>> {
    let mut slot = vec![0usize; points.len()];
    let renumber = |layers: &[Vec<usize>], slot: &mut [usize]| {
        for l in layers {
            for (i, p) in l.iter().enumerate() {
                slot[*p] = i;
            }
        }
    };
    renumber(&layers, &mut slot);
    let mut best = layers.clone();
    let mut best_crossings = crossings(points, segs, &layers, &slot);
    for sweep in 0..ORDER_SWEEPS {
        if best_crossings == 0 {
            break;
        }
        let down = sweep % 2 == 0;
        let range: Vec<usize> = if down {
            (1..layers.len()).collect()
        } else {
            (0..layers.len().saturating_sub(1)).rev().collect()
        };
        for l in range {
            let mut keyed: Vec<(f32, usize)> = layers[l]
                .iter()
                .map(|&p| {
                    let (list, other) = if down {
                        (&ups[p], true)
                    } else {
                        (&downs[p], false)
                    };
                    let vals: Vec<f32> = list
                        .iter()
                        .map(|&s| {
                            let seg = segs[s];
                            let (q, y) = if other {
                                (seg.upper, seg.upper_y)
                            } else {
                                (seg.lower, seg.lower_y)
                            };
                            slot[q] as f32 + frac(points, q, y)
                        })
                        .collect();
                    let bc = if vals.is_empty() {
                        slot[p] as f32
                    } else {
                        vals.iter().sum::<f32>() / vals.len() as f32
                    };
                    (bc, p)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[l] = keyed.into_iter().map(|(_, p)| p).collect();
            for (i, p) in layers[l].iter().enumerate() {
                slot[*p] = i;
            }
        }
        let c = crossings(points, segs, &layers, &slot);
        if c < best_crossings {
            best_crossings = c;
            best = layers.clone();
        }
    }
    best
}

/// Wire crossings between neighbouring layers, sockets included.
fn crossings(points: &[Point], segs: &[Seg], layers: &[Vec<usize>], slot: &[usize]) -> usize {
    let mut by_layer: Vec<Vec<(f32, f32)>> = vec![Vec::new(); layers.len()];
    for s in segs {
        let l = points[s.upper].layer;
        if let Some(list) = by_layer.get_mut(l) {
            list.push((
                slot[s.upper] as f32 + frac(points, s.upper, s.upper_y),
                slot[s.lower] as f32 + frac(points, s.lower, s.lower_y),
            ));
        }
    }
    let mut count = 0;
    for list in &by_layer {
        for (i, a) in list.iter().enumerate() {
            for b in &list[i + 1..] {
                if (a.0 - b.0) * (a.1 - b.1) < 0.0 {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Vertical positions: each column keeps its order and its spacing, and
/// moves its points as close as it can to their wired neighbours.
fn place(
    points: &[Point],
    segs: &[Seg],
    ups: &[Vec<usize>],
    downs: &[Vec<usize>],
    layers: &[Vec<usize>],
) -> Vec<f32> {
    let mut y = vec![0.0f32; points.len()];
    let gap = |a: usize, b: usize| {
        if points[a].real && points[b].real {
            ROW_GAP
        } else {
            DUMMY_GAP
        }
    };
    for l in layers {
        let mut at = 0.0;
        for (i, p) in l.iter().enumerate() {
            y[*p] = at;
            at += points[*p].size.y;
            if let Some(next) = l.get(i + 1) {
                at += gap(*p, *next);
            }
        }
    }
    for sweep in 0..PLACE_SWEEPS {
        let last = sweep + 1 == PLACE_SWEEPS;
        let down = sweep % 2 == 0;
        let range: Vec<usize> = if last || down {
            (0..layers.len()).collect()
        } else {
            (0..layers.len()).rev().collect()
        };
        for l in range {
            let col = &layers[l];
            if col.is_empty() {
                continue;
            }
            let mut want = Vec::with_capacity(col.len());
            let mut weight = Vec::with_capacity(col.len());
            for &p in col {
                // Each wired neighbour pulls; one wired to many pulls each
                // of them less (a Mix doesn't drag its inputs together,
                // they line up with what feeds them).
                let mut pull = 0.0f32;
                let mut sum = 0.0f32;
                if last || down {
                    for &s in &ups[p] {
                        let seg = segs[s];
                        let w = 1.0 / downs[seg.upper].len().max(1) as f32;
                        pull += w;
                        sum += w * (y[seg.upper] + seg.upper_y - seg.lower_y);
                    }
                }
                if last || !down {
                    for &s in &downs[p] {
                        let seg = segs[s];
                        let w = 1.0 / ups[seg.lower].len().max(1) as f32;
                        pull += w;
                        sum += w * (y[seg.lower] + seg.lower_y - seg.upper_y);
                    }
                }
                if pull > 0.0 {
                    want.push(sum / pull);
                    weight.push(pull);
                } else {
                    want.push(y[p]);
                    weight.push(0.25);
                }
            }
            let space: Vec<f32> = col
                .windows(2)
                .map(|w| points[w[0]].size.y + gap(w[0], w[1]))
                .collect();
            for (p, v) in col.iter().zip(fit_column(&want, &weight, &space)) {
                y[*p] = v;
            }
        }
    }
    y
}

/// Least-squares positions for points kept in order: point `i` wants
/// `want[i]` (with `weight[i]`), and point `i + 1` starts at least
/// `space[i]` below point `i`. Subtracting the spacing turns this into
/// isotonic regression, solved by pool-adjacent-violators.
fn fit_column(want: &[f32], weight: &[f32], space: &[f32]) -> Vec<f32> {
    let n = want.len();
    let mut offset = vec![0.0f32; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + space.get(i - 1).copied().unwrap_or(0.0);
    }
    // Blocks of (total weight, weighted sum, count).
    let mut blocks: Vec<(f32, f32, usize)> = Vec::new();
    for i in 0..n {
        let w = weight.get(i).copied().unwrap_or(1.0).max(1e-3);
        let z = want[i] - offset[i];
        blocks.push((w, w * z, 1));
        while blocks.len() >= 2 {
            let (w2, s2, c2) = blocks[blocks.len() - 1];
            let (w1, s1, c1) = blocks[blocks.len() - 2];
            if s1 / w1 <= s2 / w2 {
                break;
            }
            blocks.pop();
            if let Some(b) = blocks.last_mut() {
                *b = (w1 + w2, s1 + s2, c1 + c2);
            }
        }
    }
    let mut out = Vec::with_capacity(n);
    for (w, s, c) in blocks {
        let mean = s / w;
        for _ in 0..c {
            let i = out.len();
            out.push(mean + offset[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Endpoint;

    fn card(id: u64, x: f32, y: f32, h: f32, ins: &[&str], outs: &[&str]) -> Card {
        let row = |i: usize| theme::HEADER_HEIGHT + 3.0 + (i as f32 + 0.5) * theme::ROW_HEIGHT;
        Card {
            id: NodeId(id),
            rect: Rect::from_min_size(pos2(x, y), vec2(theme::NODE_WIDTH, h)),
            inputs: ins
                .iter()
                .enumerate()
                .map(|(i, s)| (s.to_string(), row(i)))
                .collect(),
            outputs: outs
                .iter()
                .enumerate()
                .map(|(i, s)| (s.to_string(), row(i)))
                .collect(),
            feedback: false,
        }
    }

    fn wire(a: u64, out: &str, b: u64, inp: &str) -> Wire {
        Wire::new(Endpoint::new(NodeId(a), out), Endpoint::new(NodeId(b), inp))
    }

    fn rects(cards: &[Card], a: &Arrangement) -> HashMap<NodeId, Rect> {
        a.cards
            .iter()
            .map(|(id, p)| {
                let c = cards.iter().find(|c| c.id == *id).unwrap();
                (*id, Rect::from_min_size(*p, c.rect.size()))
            })
            .collect()
    }

    fn assert_no_overlaps(r: &HashMap<NodeId, Rect>) {
        let all: Vec<(&NodeId, &Rect)> = r.iter().collect();
        for (i, (a, ra)) in all.iter().enumerate() {
            for (b, rb) in &all[i + 1..] {
                assert!(!ra.intersects(**rb), "{a:?} {ra:?} overlaps {b:?} {rb:?}");
            }
        }
    }

    /// A chain a → b → c → d, scrambled on the canvas.
    fn scrambled_chain() -> (Vec<Card>, Vec<Wire>) {
        let cards = vec![
            card(3, 0.0, 400.0, 80.0, &["in"], &["out"]),
            card(1, 700.0, 0.0, 80.0, &["in"], &["out"]),
            card(4, 300.0, 900.0, 80.0, &["in"], &[]),
            card(2, 50.0, 50.0, 80.0, &["in"], &["out"]),
        ];
        let wires = vec![
            wire(1, "out", 2, "in"),
            wire(2, "out", 3, "in"),
            wire(3, "out", 4, "in"),
        ];
        (cards, wires)
    }

    #[test]
    fn a_chain_runs_left_to_right_from_the_old_top_left() {
        let (cards, wires) = scrambled_chain();
        let a = arrange(&cards, &wires, &[]);
        let r = rects(&cards, &a);
        let x = |id| r[&NodeId(id)].min.x;
        for (u, v) in [(1, 2), (2, 3), (3, 4)] {
            assert!(
                r[&NodeId(u)].max.x + COLUMN_GAP <= x(v) + 0.01,
                "{u} before {v}"
            );
        }
        // A straight chain lines up.
        let y = |id| r[&NodeId(id)].min.y;
        assert!((y(1) - y(4)).abs() < 0.5, "{} {}", y(1), y(4));
        // The layout's top-left is where the cards' top-left was.
        let b = r.values().copied().reduce(|a, b| a.union(b)).unwrap();
        assert_eq!(b.min, pos2(0.0, 0.0));
    }

    #[test]
    fn layouts_are_deterministic() {
        let (cards, wires) = scrambled_chain();
        let a = arrange(&cards, &wires, &[]);
        for _ in 0..3 {
            assert_eq!(arrange(&cards, &wires, &[]), a);
        }
        // Laying out a laid-out graph changes nothing.
        let moved: Vec<Card> = cards
            .iter()
            .map(|c| {
                let p = a.cards.iter().find(|(id, _)| *id == c.id).unwrap().1;
                Card {
                    rect: Rect::from_min_size(p, c.rect.size()),
                    ..c.clone()
                }
            })
            .collect();
        assert_eq!(arrange(&moved, &wires, &[]), a);
    }

    #[test]
    fn branches_fan_out_and_join_without_overlaps() {
        // A source feeding four effects of different sizes, mixed, then out.
        let mut cards = vec![card(1, 0.0, 0.0, 80.0, &[], &["out"])];
        for i in 0..4u64 {
            cards.push(card(
                10 + i,
                (i * 37) as f32,
                (i * 13) as f32,
                60.0 + i as f32 * 40.0,
                &["in"],
                &["out"],
            ));
        }
        cards.push(card(
            20,
            10.0,
            10.0,
            200.0,
            &["in_0", "in_1", "in_2", "in_3"],
            &["out"],
        ));
        cards.push(card(30, 5.0, 5.0, 60.0, &["in"], &[]));
        let mut wires = Vec::new();
        for i in 0..4u64 {
            wires.push(wire(1, "out", 10 + i, "in"));
            wires.push(wire(10 + i, "out", 20, &format!("in_{i}")));
        }
        wires.push(wire(20, "out", 30, "in"));
        let a = arrange(&cards, &wires, &[]);
        let r = rects(&cards, &a);
        assert_no_overlaps(&r);
        // Columns along the wires.
        for w in &wires {
            assert!(r[&w.from.node].max.x < r[&w.to.node].min.x, "{w:?}");
        }
        // The effects stack in the order of the mix inputs they feed.
        let ys: Vec<f32> = (0..4).map(|i| r[&NodeId(10 + i)].min.y).collect();
        assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
    }

    #[test]
    fn socket_order_beats_the_old_order() {
        // Two oscillators drawn the other way round from the inputs they feed.
        let cards = vec![
            card(1, 0.0, 0.0, 60.0, &[], &["out"]),
            card(2, 0.0, 300.0, 60.0, &[], &["out"]),
            card(3, 400.0, 0.0, 120.0, &["in_0", "in_1"], &["out"]),
        ];
        let wires = vec![wire(1, "out", 3, "in_1"), wire(2, "out", 3, "in_0")];
        let r = rects(&cards, &arrange(&cards, &wires, &[]));
        assert!(r[&NodeId(2)].min.y < r[&NodeId(1)].min.y);
    }

    #[test]
    fn sources_sit_next_to_what_they_feed() {
        // osc → filter → gain → out, with an LFO on the gain.
        let cards = vec![
            card(1, 0.0, 0.0, 60.0, &[], &["out"]),
            card(2, 0.0, 0.0, 60.0, &["in"], &["out"]),
            card(3, 0.0, 0.0, 80.0, &["in", "gain"], &["out"]),
            card(4, 0.0, 0.0, 60.0, &["in"], &[]),
            card(5, 0.0, 200.0, 60.0, &[], &["out"]),
        ];
        let wires = vec![
            wire(1, "out", 2, "in"),
            wire(2, "out", 3, "in"),
            wire(3, "out", 4, "in"),
            wire(5, "out", 3, "gain"),
        ];
        let r = rects(&cards, &arrange(&cards, &wires, &[]));
        assert_eq!(
            r[&NodeId(5)].min.x,
            r[&NodeId(2)].min.x,
            "the LFO is in the column before the gain"
        );
        assert_no_overlaps(&r);
    }

    #[test]
    fn loops_break_at_the_feedback_node() {
        // mix → delay → feedback → back into the mix, and mix → out.
        let mut fb = card(3, 0.0, 0.0, 60.0, &["in"], &["out"]);
        fb.feedback = true;
        let cards = vec![
            card(1, 500.0, 0.0, 100.0, &["in_0", "in_1"], &["out"]),
            card(2, 0.0, 300.0, 60.0, &["in"], &["out"]),
            fb,
            card(4, 0.0, 600.0, 60.0, &["in"], &[]),
        ];
        let wires = vec![
            wire(1, "out", 2, "in"),
            wire(2, "out", 3, "in"),
            wire(3, "out", 1, "in_1"),
            wire(1, "out", 4, "in"),
        ];
        let r = rects(&cards, &arrange(&cards, &wires, &[]));
        let x = |id| r[&NodeId(id)].min.x;
        assert!(
            x(1) < x(2) && x(2) < x(3),
            "the loop reads mix → delay → feedback"
        );
        assert!(x(1) < x(4));
        assert_no_overlaps(&r);
        // A loop with no Feedback node (a compile error) still lays out.
        let cards = vec![
            card(1, 0.0, 0.0, 60.0, &["in"], &["out"]),
            card(2, 0.0, 100.0, 60.0, &["in"], &["out"]),
            card(3, 0.0, 200.0, 60.0, &["in"], &["out"]),
        ];
        let wires = vec![
            wire(1, "out", 2, "in"),
            wire(2, "out", 3, "in"),
            wire(3, "out", 1, "in"),
        ];
        let r = rects(&cards, &arrange(&cards, &wires, &[]));
        assert!(r[&NodeId(1)].min.x < r[&NodeId(2)].min.x);
        assert!(r[&NodeId(2)].min.x < r[&NodeId(3)].min.x);
    }

    #[test]
    fn frames_stay_around_their_contents() {
        // a → [b → c] → d, with the frame drawn around b and c.
        let cards = vec![
            card(1, 0.0, 0.0, 60.0, &[], &["out"]),
            card(2, 600.0, 400.0, 60.0, &["in"], &["out"]),
            card(3, 300.0, 420.0, 60.0, &["in"], &["out"]),
            card(4, 0.0, 200.0, 60.0, &["in"], &[]),
        ];
        let frame = FrameBox {
            index: 7,
            rect: Rect::from_min_max(pos2(280.0, 380.0), pos2(820.0, 500.0)),
        };
        let wires = vec![
            wire(1, "out", 2, "in"),
            wire(2, "out", 3, "in"),
            wire(3, "out", 4, "in"),
        ];
        let a = arrange(&cards, &wires, std::slice::from_ref(&frame));
        let r = rects(&cards, &a);
        assert_no_overlaps(&r);
        assert_eq!(a.frames.len(), 1);
        let (index, f) = a.frames[0];
        assert_eq!(index, 7);
        for id in [2, 3] {
            assert!(f.contains_rect(r[&NodeId(id)]), "{id} stays inside");
        }
        for id in [1, 4] {
            assert!(!f.intersects(r[&NodeId(id)]), "{id} stays outside");
        }
        assert!(r[&NodeId(1)].max.x < f.min.x && f.max.x < r[&NodeId(4)].min.x);
        assert!(r[&NodeId(2)].max.x < r[&NodeId(3)].min.x, "inside, b → c");
        // Room for the frame's label above its contents.
        assert!(r[&NodeId(2)].min.y - f.min.y >= theme::FRAME_HEADER);
    }

    #[test]
    fn nested_frames_nest_and_loose_cards_keep_their_side() {
        let cards = vec![
            card(1, 10.0, 10.0, 60.0, &[], &["out"]),
            card(2, 400.0, 10.0, 60.0, &["in"], &["out"]),
            // Two unwired cards (a note, a lone node).
            card(3, 900.0, 0.0, 120.0, &[], &[]),
            card(4, 0.0, 700.0, 60.0, &["in"], &[]),
            // Fed from inside the frames.
            card(5, 800.0, 300.0, 60.0, &["in"], &[]),
        ];
        let outer = FrameBox {
            index: 0,
            rect: Rect::from_min_max(pos2(0.0, -40.0), pos2(700.0, 200.0)),
        };
        let inner = FrameBox {
            index: 1,
            rect: Rect::from_min_max(pos2(380.0, 0.0), pos2(650.0, 100.0)),
        };
        let wires = vec![wire(1, "out", 2, "in"), wire(2, "out", 5, "in")];
        let a = arrange(&cards, &wires, &[outer, inner]);
        let r = rects(&cards, &a);
        assert_no_overlaps(&r);
        let f: HashMap<usize, Rect> = a.frames.iter().copied().collect();
        assert!(f[&0].contains_rect(f[&1]));
        assert!(f[&1].contains_rect(r[&NodeId(2)]));
        assert!(f[&0].contains_rect(r[&NodeId(1)]));
        assert!(!f[&1].intersects(r[&NodeId(1)]));
        assert!(r[&NodeId(1)].max.x < f[&1].min.x);
        assert!(f[&0].max.x < r[&NodeId(5)].min.x);
        // The unwired cards keep their side: the one that was up top goes
        // in a row above the rest, the one that was low in a row below.
        let (a3, a4) = (r[&NodeId(3)], r[&NodeId(4)]);
        assert!(a3.max.y < f[&0].min.y, "{a3:?} {:?}", f[&0]);
        assert!(a4.min.y > f[&0].max.y, "{a4:?} {:?}", f[&0]);
        assert_eq!(a3.min.x, a4.min.x, "each row starts at the left");
    }

    #[test]
    fn odd_input_doesnt_panic() {
        assert_eq!(arrange(&[], &[], &[]), Arrangement::default());
        let mut bad = card(1, f32::NAN, 0.0, 60.0, &[], &["out"]);
        bad.rect = Rect::from_min_size(pos2(f32::NAN, f32::INFINITY), vec2(-5.0, 10.0));
        let cards = vec![bad, card(2, 0.0, 0.0, 60.0, &["in"], &[])];
        // Wires to cards that aren't laid out, to unknown sockets, and to itself.
        let wires = vec![
            wire(1, "nope", 2, "in"),
            wire(9, "out", 2, "in"),
            wire(2, "out", 2, "in"),
        ];
        let a = arrange(&cards, &wires, &[]);
        assert!(
            a.cards
                .iter()
                .all(|(_, p)| p.x.is_finite() && p.y.is_finite())
        );
    }

    #[test]
    fn column_fit_keeps_order_and_spacing() {
        // Three points all wanting 0: they spread around it, 10 apart.
        let y = fit_column(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &[10.0, 10.0]);
        assert_eq!(y, vec![-10.0, 0.0, 10.0]);
        // Already far enough apart: everyone gets what they want.
        let y = fit_column(&[0.0, 50.0], &[1.0, 1.0], &[10.0]);
        assert_eq!(y, vec![0.0, 50.0]);
    }

    #[test]
    fn a_big_tangled_graph_lays_out_quickly_without_overlaps() {
        // 300 cards, each fed by up to three earlier ones (a fixed
        // pseudo-random pattern), drawn in a scramble.
        let n = 300u64;
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let cards: Vec<Card> = (1..=n)
            .map(|i| {
                let (x, y) = ((next() % 3000) as f32, (next() % 3000) as f32);
                card(
                    i,
                    x,
                    y,
                    60.0 + (next() % 4) as f32 * 30.0,
                    &["a", "b", "c"],
                    &["out"],
                )
            })
            .collect();
        let mut wires = Vec::new();
        for i in 2..=n {
            for (k, s) in ["a", "b", "c"].iter().enumerate() {
                if k == 0 || next() % 3 == 0 {
                    let from = 1 + next() % (i - 1);
                    wires.push(wire(from, "out", i, s));
                }
            }
        }
        let t = std::time::Instant::now();
        let a = arrange(&cards, &wires, &[]);
        assert!(t.elapsed().as_secs_f32() < 10.0, "{:?}", t.elapsed());
        let r = rects(&cards, &a);
        assert_no_overlaps(&r);
        for w in &wires {
            assert!(r[&w.from.node].max.x < r[&w.to.node].min.x, "{w:?}");
        }
    }
}
