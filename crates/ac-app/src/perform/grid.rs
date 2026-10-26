//! Where a panel's cards, strips, and labels go (DECISIONS D202): a pure
//! function of the panel and each card's natural size, in grid cells
//! across and lines (half cells) down.
//!
//! Things with a place of their own go there; a strip stacks its cards
//! under its header in pin order; everything without a place flows after
//! the placed things, in pin order, as panels did before places existed.
//! Overlapping places (from a hand-edited file) move down until they fit,
//! so nothing is ever drawn over something else.

use std::collections::HashSet;

use ac_core::Panel;
use ac_core::panel::LINES_PER_CELL;

/// What a placed rectangle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element {
    /// `items[i]` (in a strip or not).
    Item(usize),
    /// The header of the strip with this id.
    Strip(u32),
    /// `labels[i]`.
    Label(usize),
}

/// One rectangle on the grid, in cells across and lines down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub element: Element,
    pub col: u32,
    pub line: u32,
    pub cols: u32,
    pub lines: u32,
    /// The strip an item is stacked in.
    pub strip: Option<u32>,
}

impl Placed {
    pub fn right(&self) -> u32 {
        self.col + self.cols
    }

    pub fn bottom(&self) -> u32 {
        self.line + self.lines
    }

    fn cells(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (self.line..self.bottom()).flat_map(move |l| (self.col..self.right()).map(move |c| (c, l)))
    }
}

/// The whole panel laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub placed: Vec<Placed>,
    /// Columns and lines the placed rectangles reach.
    pub columns: u32,
    pub lines: u32,
}

impl Layout {
    pub fn get(&self, e: Element) -> Option<&Placed> {
        self.placed.iter().find(|p| p.element == e)
    }

    fn taken(&self) -> HashSet<(u32, u32)> {
        self.placed.iter().flat_map(Placed::cells).collect()
    }

    /// The first free place, reading line by line, for something `size`
    /// big within `columns` (beyond them when it's wider).
    pub fn free_spot(&self, size: [u32; 2], columns: u32) -> [u32; 2] {
        free_in(&self.taken(), size, columns, [0, 0])
    }
}

/// The first free place at or after `from` (line by line, left to right
/// from column 0 on later lines).
fn free_in(
    taken: &HashSet<(u32, u32)>,
    [cols, lines]: [u32; 2],
    columns: u32,
    from: [u32; 2],
) -> [u32; 2] {
    let columns = columns.max(cols).max(1);
    let fits =
        |c: u32, l: u32| (l..l + lines).all(|y| (c..c + cols).all(|x| !taken.contains(&(x, y))));
    let (mut c, mut l) = (from[0], from[1]);
    // Bounded: the grid is finite, so a place is always found well before.
    for _ in 0..1_000_000 {
        if c + cols > columns {
            c = 0;
            l += 1;
            continue;
        }
        if fits(c, l) {
            return [c, l];
        }
        c += 1;
    }
    [0, l]
}

/// The first free place in column `col` at or below line `line`.
fn free_below(
    taken: &HashSet<(u32, u32)>,
    [cols, lines]: [u32; 2],
    [col, line]: [u32; 2],
) -> [u32; 2] {
    let fits = |l: u32| (l..l + lines).all(|y| (col..col + cols).all(|x| !taken.contains(&(x, y))));
    let mut l = line;
    while !fits(l) && l < line + 100_000 {
        l += 1;
    }
    [col, l]
}

/// The rectangles placed so far and the cells they take.
#[derive(Default)]
struct Builder {
    out: Layout,
    taken: HashSet<(u32, u32)>,
}

impl Builder {
    fn put(&mut self, p: Placed) {
        self.taken.extend(p.cells());
        self.out.columns = self.out.columns.max(p.right());
        self.out.lines = self.out.lines.max(p.bottom());
        self.out.placed.push(p);
    }

    /// A strip's header at `at` and its `members` (item, lines) under it.
    fn stack(&mut self, id: u32, [col, line]: [u32; 2], width: u32, members: &[(usize, u32)]) {
        self.put(Placed {
            element: Element::Strip(id),
            col,
            line,
            cols: width,
            lines: 1,
            strip: None,
        });
        let mut y = line + 1;
        for &(i, h) in members {
            self.put(Placed {
                element: Element::Item(i),
                col,
                line: y,
                cols: width,
                lines: h,
                strip: Some(id),
            });
            y += h;
        }
    }
}

/// Lay `panel` out. `size(i)` is item `i`'s span `[cells across, lines
/// down]`, or `None` for an item whose node is gone (left out); `columns`
/// is how many cells fit the view, which only flowing things wrap at.
pub fn layout(panel: &Panel, size: impl Fn(usize) -> Option<[u32; 2]>, columns: u32) -> Layout {
    let columns = columns.max(1);
    let mut b = Builder::default();
    // A strip's cards, with their heights, and the strip's height.
    let strip_size = |id: u32| -> (u32, Vec<(usize, u32)>) {
        let members: Vec<(usize, u32)> = panel
            .strip_items(id)
            .filter_map(|(i, _)| size(i).map(|s| (i, s[1].max(1))))
            .collect();
        (1 + members.iter().map(|m| m.1).sum::<u32>(), members)
    };

    // Placed things first: strips, labels, then cards, each in its order.
    let mut flowing_strips = Vec::new();
    for s in &panel.strips {
        let (lines, members) = strip_size(s.id);
        match s.place() {
            Some(at) => {
                let at = free_below(&b.taken, [s.width(), lines], at);
                b.stack(s.id, at, s.width(), &members);
            }
            None => flowing_strips.push((s.id, s.width(), lines, members)),
        }
    }
    for (k, l) in panel.labels.iter().enumerate() {
        let [col, line] = free_below(&b.taken, [l.span(), 1], l.place());
        b.put(Placed {
            element: Element::Label(k),
            col,
            line,
            cols: l.span(),
            lines: 1,
            strip: None,
        });
    }
    let mut flowing = Vec::new();
    for (i, it) in panel.items.iter().enumerate() {
        if it.strip.is_some_and(|s| panel.strip(s).is_some()) {
            continue;
        }
        let Some([c, l]) = size(i) else { continue };
        match it.place() {
            Some(at) => {
                let [col, line] = free_below(&b.taken, [c, l], at);
                b.put(Placed {
                    element: Element::Item(i),
                    col,
                    line,
                    cols: c,
                    lines: l,
                    strip: None,
                });
            }
            None => flowing.push((i, [c.min(columns), l])),
        }
    }
    // Then the flow, below whatever is placed, each after the one before.
    let mut cursor = [0, b.out.lines];
    for (i, s) in flowing {
        let [col, line] = free_in(&b.taken, s, columns, cursor);
        b.put(Placed {
            element: Element::Item(i),
            col,
            line,
            cols: s[0],
            lines: s[1],
            strip: None,
        });
        cursor = [col + s[0], line];
    }
    for (id, width, lines, members) in flowing_strips {
        let at = free_in(&b.taken, [width.min(columns), lines], columns, cursor);
        b.stack(id, at, width, &members);
        cursor = [at[0] + width, at[1]];
    }
    b.out
}

/// Give every card and strip of `panel` the place `layout` drew it at, so
/// nothing moves again when the window changes or something is pinned
/// (the first change a person makes to a flowing panel's layout).
pub fn freeze(panel: &mut Panel, layout: &Layout) {
    for p in &layout.placed {
        match p.element {
            Element::Item(i) if p.strip.is_none() => {
                if let Some(it) = panel.items.get_mut(i) {
                    it.at = Some([p.col, p.line]);
                }
            }
            Element::Strip(id) => {
                if let Some(s) = panel.strips.iter_mut().find(|s| s.id == id) {
                    s.at = Some([p.col, p.line]);
                }
            }
            _ => {}
        }
    }
}

/// Lines an item of `rows` cells takes, or one when compact.
pub fn item_lines(rows: u32, compact: bool) -> u32 {
    if compact { 1 } else { rows * LINES_PER_CELL }
}

/// Where dragged things were dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    /// On empty grid (or other cards): they move by the drag.
    Place,
    /// One card onto a strip: it joins it, before the card `before` (an
    /// item index) or at the end.
    Strip { id: u32, before: Option<usize> },
}

fn shifted(v: u32, d: i32) -> u32 {
    (v as i64 + d as i64).max(0) as u32
}

/// Move `elements` (as `layout` drew them) by `delta` places, or drop one
/// card into a strip. A flowing panel gets places first, so the rest stay
/// where they were drawn. A single card dropped where one other card of
/// its size sits swaps places with it. Returns whether anything changed.
pub fn move_elements(
    panel: &mut Panel,
    layout: &Layout,
    elements: &[Element],
    delta: [i32; 2],
    drop: Drop,
) -> bool {
    let before = panel.clone();
    if !panel.is_placed() {
        freeze(panel, layout);
    }
    match drop {
        Drop::Strip { id, before } => {
            let [Element::Item(item)] = elements else {
                return false;
            };
            if panel.strip(id).is_none() || *item >= panel.items.len() {
                return false;
            }
            let mut it = panel.items.remove(*item);
            it.strip = Some(id);
            it.at = None;
            // Indices after the removed one moved down by one.
            let to = match before {
                Some(b) if b > *item => b - 1,
                Some(b) => b,
                None => panel
                    .items
                    .iter()
                    .rposition(|x| x.strip == Some(id))
                    .map_or(panel.items.len(), |k| k + 1),
            };
            panel.items.insert(to.min(panel.items.len()), it);
        }
        Drop::Place => {
            let single = match elements {
                [Element::Item(i)] => Some(*i),
                _ => None,
            };
            for e in elements {
                let Some(p) = layout.get(*e) else { continue };
                let at = [shifted(p.col, delta[0]), shifted(p.line, delta[1])];
                match *e {
                    Element::Item(i) => {
                        if let Some(it) = panel.items.get_mut(i) {
                            it.at = Some(at);
                            // Dragged out of its strip (a strip moves as
                            // one by its header).
                            it.strip = None;
                        }
                    }
                    Element::Strip(id) => {
                        if let Some(s) = panel.strips.iter_mut().find(|s| s.id == id) {
                            s.at = Some(at);
                        }
                    }
                    Element::Label(k) => {
                        if let Some(l) = panel.labels.get_mut(k) {
                            l.at = at;
                        }
                    }
                }
            }
            if let Some(i) = single
                && let Some(p) = layout.get(Element::Item(i))
            {
                let at = [shifted(p.col, delta[0]), shifted(p.line, delta[1])];
                let hits: Vec<&Placed> = layout
                    .placed
                    .iter()
                    .filter(|q| {
                        q.element != Element::Item(i)
                            && q.strip.is_none()
                            && q.col < at[0] + p.cols
                            && at[0] < q.right()
                            && q.line < at[1] + p.lines
                            && at[1] < q.bottom()
                    })
                    .collect();
                if let [other] = hits.as_slice()
                    && let Element::Item(o) = other.element
                    && other.cols == p.cols
                    && other.lines == p.lines
                    && let Some(it) = panel.items.get_mut(o)
                {
                    it.at = Some([p.col, p.line]);
                }
            }
        }
    }
    *panel != before
}

/// Make a strip of `items` (as `layout` drew them) at the top-left of where
/// they were, named `name`. A card already in a strip moves to the new one.
/// Returns the new strip's id.
pub fn make_strip(panel: &mut Panel, layout: &Layout, items: &[usize], name: &str) -> Option<u32> {
    let mut items: Vec<usize> = items
        .iter()
        .copied()
        .filter(|&i| i < panel.items.len())
        .collect();
    items.sort_unstable();
    items.dedup();
    if items.is_empty() {
        return None;
    }
    if !panel.is_placed() {
        freeze(panel, layout);
    }
    let placed: Vec<&Placed> = items
        .iter()
        .filter_map(|&i| layout.get(Element::Item(i)))
        .collect();
    let col = placed.iter().map(|p| p.col).min().unwrap_or(0);
    let line = placed.iter().map(|p| p.line).min().unwrap_or(0);
    let id = panel.fresh_strip_id();
    let mut strip = ac_core::Strip::new(id, name);
    strip.at = Some([col, line]);
    panel.strips.push(strip);
    // In the order they read: top to bottom, then left to right.
    let mut order: Vec<(u32, u32, usize)> = items
        .iter()
        .map(|&i| {
            layout
                .get(Element::Item(i))
                .map_or((u32::MAX, u32::MAX, i), |p| (p.line, p.col, i))
        })
        .collect();
    order.sort_unstable();
    let moved: Vec<ac_core::PanelItem> = order
        .iter()
        .map(|&(_, _, i)| {
            let mut it = panel.items[i].clone();
            it.strip = Some(id);
            it.at = None;
            it
        })
        .collect();
    let mut k = 0;
    panel.items.retain(|_| {
        let keep = !items.contains(&k);
        k += 1;
        keep
    });
    panel.items.extend(moved);
    ac_core::panel::prune_strips(panel);
    Some(id)
}

/// Take a card out of its strip, keeping it where it was drawn.
pub fn take_out_of_strip(panel: &mut Panel, layout: &Layout, item: usize) -> bool {
    let Some(p) = layout.get(Element::Item(item)) else {
        return false;
    };
    if !panel.is_placed() {
        freeze(panel, layout);
    }
    let Some(it) = panel.items.get_mut(item) else {
        return false;
    };
    if it.strip.is_none() {
        return false;
    }
    it.strip = None;
    it.at = Some([p.col, p.line]);
    ac_core::panel::prune_strips(panel);
    true
}

/// Dissolve a strip: its cards stay where they were drawn, loose.
pub fn dissolve_strip(panel: &mut Panel, layout: &Layout, id: u32) -> bool {
    if panel.strip(id).is_none() {
        return false;
    }
    if !panel.is_placed() {
        freeze(panel, layout);
    }
    for (i, it) in panel.items.iter_mut().enumerate() {
        if it.strip == Some(id) {
            it.strip = None;
            it.at = layout.get(Element::Item(i)).map(|p| [p.col, p.line]);
        }
    }
    panel.strips.retain(|s| s.id != id);
    true
}

/// Unpin a strip and its cards.
pub fn remove_strip(panel: &mut Panel, id: u32) -> bool {
    if panel.strip(id).is_none() {
        return false;
    }
    panel.items.retain(|it| it.strip != Some(id));
    panel.strips.retain(|s| s.id != id);
    true
}

/// A strip one column wider (`d` 1) or narrower (−1).
pub fn widen_strip(panel: &mut Panel, id: u32, d: i32) -> bool {
    let Some(s) = panel.strips.iter_mut().find(|s| s.id == id) else {
        return false;
    };
    let next = (s.width() as i32 + d).clamp(1, ac_core::panel::MAX_SPAN as i32) as u32;
    let changed = next != s.width();
    s.width = next;
    changed
}

/// Add a label (a separator when `text` is empty) at `at`, `span` columns
/// wide. A flowing panel gets places first, so nothing moves under it.
pub fn add_label(panel: &mut Panel, layout: &Layout, at: [u32; 2], text: &str, span: u32) {
    if !panel.is_placed() {
        freeze(panel, layout);
    }
    panel.labels.push(ac_core::PanelLabel {
        text: text.to_string(),
        at,
        span: span.max(1),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{NodeId, PanelItem, PanelLabel, Strip};

    fn panel(n: usize) -> Panel {
        let mut p = Panel::new("P");
        for k in 0..n {
            p.items.push(PanelItem::new(NodeId(k as u64 + 1)));
        }
        p
    }

    fn at(l: &Layout, e: Element) -> (u32, u32) {
        let p = l.get(e).expect("placed");
        (p.col, p.line)
    }

    #[test]
    fn a_flowing_panel_wraps_in_pin_order_as_before() {
        // A 2-wide slider, a button, a 3×2 scope that won't fit after them,
        // then a button after the scope (in lines: cells are two lines).
        let p = panel(4);
        let sizes = [[2, 2], [1, 2], [3, 4], [1, 2]];
        let l = layout(&p, |i| sizes.get(i).copied(), 4);
        assert_eq!(at(&l, Element::Item(0)), (0, 0));
        assert_eq!(at(&l, Element::Item(1)), (2, 0));
        assert_eq!(at(&l, Element::Item(2)), (0, 2));
        assert_eq!(at(&l, Element::Item(3)), (3, 2));
        assert_eq!((l.columns, l.lines), (4, 6));
    }

    #[test]
    fn placed_cards_stay_put_and_the_rest_flow_below() {
        let mut p = panel(3);
        p.items[1].at = Some([5, 0]);
        let l = layout(&p, |_| Some([1, 2]), 3);
        assert_eq!(at(&l, Element::Item(1)), (5, 0));
        // The others flow from the bottom of the placed ones.
        assert_eq!(at(&l, Element::Item(0)), (0, 2));
        assert_eq!(at(&l, Element::Item(2)), (1, 2));
        assert_eq!(l.columns, 6, "placed cards can reach past the view");
    }

    #[test]
    fn a_strip_stacks_its_cards_under_one_header() {
        let mut p = panel(4);
        let mut s = Strip::new(7, "Drums");
        s.at = Some([2, 1]);
        p.strips.push(s);
        for i in [0, 2, 3] {
            p.items[i].strip = Some(7);
        }
        p.items[2].compact = true;
        let sizes = [[1, 4], [1, 2], [1, 1], [1, 2]];
        let l = layout(&p, |i| sizes.get(i).copied(), 6);
        assert_eq!(at(&l, Element::Strip(7)), (2, 1));
        assert_eq!(at(&l, Element::Item(0)), (2, 2));
        assert_eq!(at(&l, Element::Item(2)), (2, 6));
        assert_eq!(at(&l, Element::Item(3)), (2, 7));
        assert_eq!(l.get(Element::Item(3)).unwrap().strip, Some(7));
        // The loose card flows below everything placed.
        assert_eq!(at(&l, Element::Item(1)), (0, 9));
    }

    #[test]
    fn overlapping_places_move_down_and_labels_take_a_line() {
        let mut p = panel(2);
        p.items[0].at = Some([0, 1]);
        p.items[1].at = Some([0, 1]);
        p.labels.push(PanelLabel {
            text: "Tracks".into(),
            at: [0, 0],
            span: 3,
        });
        let l = layout(&p, |_| Some([1, 2]), 4);
        assert_eq!(at(&l, Element::Label(0)), (0, 0));
        assert_eq!(at(&l, Element::Item(0)), (0, 1));
        assert_eq!(
            at(&l, Element::Item(1)),
            (0, 3),
            "moved down, not drawn over"
        );
        // Missing nodes are left out.
        let l = layout(&p, |i| (i == 0).then_some([1, 2]), 4);
        assert!(l.get(Element::Item(1)).is_none());
    }

    #[test]
    fn freezing_keeps_cards_where_they_were_drawn() {
        let mut p = panel(3);
        let l = layout(&p, |_| Some([1, 2]), 2);
        freeze(&mut p, &l);
        assert_eq!(p.items[2].at, Some([0, 2]));
        // A wider view no longer moves them.
        let wide = layout(&p, |_| Some([1, 2]), 8);
        assert_eq!(at(&wide, Element::Item(2)), (0, 2));
        // A new card goes in the first free place.
        assert_eq!(wide.free_spot([1, 2], 8), [2, 0]);
        assert_eq!(wide.free_spot([2, 2], 2), [0, 4]);
    }

    #[test]
    fn moving_a_card_onto_one_of_its_size_swaps_them() {
        let mut p = panel(3);
        let l = layout(&p, |_| Some([1, 2]), 3);
        // Card 0 dragged two columns right, onto card 2.
        assert!(move_elements(
            &mut p,
            &l,
            &[Element::Item(0)],
            [2, 0],
            Drop::Place
        ));
        assert_eq!(p.items[0].at, Some([2, 0]));
        assert_eq!(p.items[2].at, Some([0, 0]), "the other takes its place");
        assert_eq!(
            p.items[1].at,
            Some([1, 0]),
            "the rest froze where they were"
        );
        // Several move together and keep their arrangement.
        let l = layout(&p, |_| Some([1, 2]), 3);
        assert!(move_elements(
            &mut p,
            &l,
            &[Element::Item(0), Element::Item(1)],
            [0, 4],
            Drop::Place
        ));
        assert_eq!((p.items[0].at, p.items[1].at), (Some([2, 4]), Some([1, 4])));
        // Off the top-left is clamped.
        let l = layout(&p, |_| Some([1, 2]), 3);
        move_elements(&mut p, &l, &[Element::Item(2)], [-9, -9], Drop::Place);
        assert_eq!(p.items[2].at, Some([0, 0]));
    }

    #[test]
    fn strips_are_made_joined_left_and_dissolved() {
        let mut p = panel(4);
        let l = layout(&p, |_| Some([1, 2]), 4);
        let id = make_strip(&mut p, &l, &[2, 1], "Drums").expect("a strip");
        // The cards go to the end of the list, in reading order.
        let order: Vec<u64> = p.items.iter().map(|i| i.node.0).collect();
        assert_eq!(order, vec![1, 4, 2, 3]);
        assert_eq!(p.strip(id).unwrap().at, Some([1, 0]));
        assert_eq!(p.strip_items(id).count(), 2);
        // Card 0 dropped on the strip joins it before node 3.
        let l = layout(&p, |_| Some([1, 2]), 4);
        assert!(move_elements(
            &mut p,
            &l,
            &[Element::Item(0)],
            [0, 0],
            Drop::Strip {
                id,
                before: Some(3)
            }
        ));
        let in_strip: Vec<u64> = p.strip_items(id).map(|(_, i)| i.node.0).collect();
        assert_eq!(in_strip, vec![2, 1, 3]);
        // Taking one out keeps it where it was drawn.
        let l = layout(&p, |_| Some([1, 2]), 4);
        let k = p.items.iter().position(|i| i.node == NodeId(1)).unwrap();
        let drawn = l.get(Element::Item(k)).map(|q| [q.col, q.line]);
        assert!(take_out_of_strip(&mut p, &l, k));
        assert_eq!(p.items[k].at, drawn);
        // Dissolving leaves the cards placed; removing unpins them.
        let l = layout(&p, |_| Some([1, 2]), 4);
        assert!(dissolve_strip(&mut p, &l, id));
        assert!(p.strips.is_empty() && p.items.iter().all(|i| i.at.is_some()));
        let l = layout(&p, |_| Some([1, 2]), 4);
        let id = make_strip(&mut p, &l, &[0, 1], "Keys").unwrap();
        assert!(widen_strip(&mut p, id, 1));
        assert_eq!(p.strip(id).unwrap().width(), 2);
        assert!(remove_strip(&mut p, id));
        assert_eq!(p.items.len(), 2);
    }

    #[test]
    fn a_label_on_a_flowing_panel_freezes_it() {
        let mut p = panel(2);
        let l = layout(&p, |_| Some([1, 2]), 2);
        add_label(&mut p, &l, [0, 4], "Buses", 2);
        assert!(p.is_placed());
        assert_eq!(p.items[1].at, Some([1, 0]));
        let l = layout(&p, |_| Some([1, 2]), 2);
        assert_eq!(at(&l, Element::Label(0)), (0, 4));
    }
}
