//! Performance panels (docs/UX.md "Panels"): named grids of pinned nodes.
//!
//! A panel is a view. It holds nothing but references to nodes in the
//! project's root graph and how they're laid out, so deleting a node is all
//! it takes to take it off every panel, and a panel never changes what the
//! patch does.
//!
//! The grid is cells wide and *lines* tall: a line is half a cell, so a
//! compact card (a toggle or a button) is one line and two of them share a
//! cell (DECISIONS D202). Cards can sit at a place of their own (`at`), or
//! flow in pin order where they have none, as panels from before places
//! existed do. A **strip** is a named column of cards with one header,
//! moved, resized, and unpinned as one; a **label** is a line of text (a
//! section heading, a row or column name) or, with no text, a separator.

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, NodeInstance};
use crate::id::NodeId;

/// The largest span, in grid cells, an item can take either way.
pub const MAX_SPAN: u32 = 8;
/// Lines (half cells) in one cell.
pub const LINES_PER_CELL: u32 = 2;
/// Furthest a place can be from the grid's corner, either way, in cells or
/// lines; places beyond it are pulled in (DECISIONS D29).
pub const MAX_PLACE: u32 = 256;

fn is_false(b: &bool) -> bool {
    !*b
}

/// One panel: a name, the nodes pinned to it, its strips and labels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PanelItem>,
    /// Named columns of cards; items say which they're in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strips: Vec<Strip>,
    /// Section headings, row and column names, and separators.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<PanelLabel>,
    /// Fields written by a newer version, preserved on save.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Panel {
    pub fn new(name: impl Into<String>) -> Panel {
        Panel {
            name: name.into(),
            items: Vec::new(),
            strips: Vec::new(),
            labels: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    /// Whether `node` is pinned here.
    pub fn contains(&self, node: NodeId) -> bool {
        self.items.iter().any(|i| i.node == node)
    }

    /// The pinned nodes that exist in `graph`, in order, with their items.
    /// Pins to nodes that are gone are skipped, never an error.
    pub fn pinned<'a>(
        &'a self,
        graph: &'a Graph,
    ) -> impl Iterator<Item = (&'a PanelItem, &'a NodeInstance)> + 'a {
        self.items
            .iter()
            .filter_map(move |i| graph.node(i.node).map(|n| (i, n)))
    }

    /// Whether anything here has a place of its own; a panel without places
    /// flows its cards in pin order, as panels did before places.
    pub fn is_placed(&self) -> bool {
        self.items.iter().any(|i| i.at.is_some())
            || self.strips.iter().any(|s| s.at.is_some())
            || !self.labels.is_empty()
    }

    /// The strip with id `id`.
    pub fn strip(&self, id: u32) -> Option<&Strip> {
        self.strips.iter().find(|s| s.id == id)
    }

    /// An id no strip here has.
    pub fn fresh_strip_id(&self) -> u32 {
        self.strips.iter().map(|s| s.id + 1).max().unwrap_or(1)
    }

    /// The items in strip `id`, in order, with their indices.
    pub fn strip_items(&self, id: u32) -> impl Iterator<Item = (usize, &PanelItem)> + '_ {
        self.items
            .iter()
            .enumerate()
            .filter(move |(_, it)| it.strip == Some(id))
    }
}

/// A node pinned to a panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelItem {
    /// A node in the project's root graph.
    pub node: NodeId,
    /// Cells the item spans, `[columns, rows]`; `None` gives the widget its
    /// natural size. Read through [`PanelItem::span`], which keeps it in
    /// range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    /// Where the card sits, `[column, line]` from the grid's top-left;
    /// `None` flows it after the placed cards, in pin order. Ignored in a
    /// strip, where cards stack in pin order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
    /// The strip it's in (a [`Strip::id`] of this panel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strip: Option<u32>,
    /// Half height: a toggle or a button that takes one line, so two share
    /// a cell.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact: bool,
    /// The card's color: a name (`amber`, `green`, `red`, `blue`, `purple`,
    /// `grey`) or `#RRGGBB`; `None` is the node's category color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl PanelItem {
    pub fn new(node: NodeId) -> PanelItem {
        PanelItem {
            node,
            size: None,
            at: None,
            strip: None,
            compact: false,
            color: None,
        }
    }

    /// The item's span, or `natural` without a size of its own, each way
    /// between 1 and [`MAX_SPAN`] cells.
    pub fn span(&self, natural: [u32; 2]) -> [u32; 2] {
        let [c, r] = self.size.unwrap_or(natural);
        [c.clamp(1, MAX_SPAN), r.clamp(1, MAX_SPAN)]
    }

    /// The item's place, pulled within [`MAX_PLACE`].
    pub fn place(&self) -> Option<[u32; 2]> {
        self.at.map(clamp_place)
    }
}

/// A named column of cards with one header (DECISIONS D202).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Strip {
    /// Unique within its panel; items refer to it.
    pub id: u32,
    pub name: String,
    /// Where its header sits, `[column, line]`; `None` flows it after the
    /// placed cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[u32; 2]>,
    /// Columns wide (1 to [`MAX_SPAN`]); read through [`Strip::width`].
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub width: u32,
    /// The header's color, as for a card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

fn one() -> u32 {
    1
}

fn is_one(v: &u32) -> bool {
    *v == 1
}

impl Strip {
    pub fn new(id: u32, name: impl Into<String>) -> Strip {
        Strip {
            id,
            name: name.into(),
            at: None,
            width: 1,
            color: None,
        }
    }

    pub fn width(&self) -> u32 {
        self.width.clamp(1, MAX_SPAN)
    }

    /// The strip's place, pulled within [`MAX_PLACE`].
    pub fn place(&self) -> Option<[u32; 2]> {
        self.at.map(clamp_place)
    }
}

/// A line of text on the grid, or a separator when the text is empty.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelLabel {
    #[serde(default)]
    pub text: String,
    /// `[column, line]`.
    pub at: [u32; 2],
    /// Columns wide (1 to [`MAX_PLACE`]); read through [`PanelLabel::span`].
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub span: u32,
}

impl PanelLabel {
    pub fn span(&self) -> u32 {
        self.span.clamp(1, MAX_PLACE)
    }

    pub fn place(&self) -> [u32; 2] {
        clamp_place(self.at)
    }

    /// A label without text is drawn as a line between the rows.
    pub fn is_separator(&self) -> bool {
        self.text.trim().is_empty()
    }
}

fn clamp_place([c, l]: [u32; 2]) -> [u32; 2] {
    [c.min(MAX_PLACE), l.min(MAX_PLACE * LINES_PER_CELL)]
}

/// Drop pins to nodes that aren't in `graph` (deleted, moved into a group,
/// or never there), strips left with no cards, and items naming strips
/// that are gone. Returns how many pins were dropped.
pub fn prune(panels: &mut [Panel], graph: &Graph) -> usize {
    let mut dropped = 0;
    for p in panels {
        let before = p.items.len();
        p.items.retain(|i| graph.contains(i.node));
        dropped += before - p.items.len();
        prune_strips(p);
    }
    dropped
}

/// Forget strips with no cards left, and cards' references to strips that
/// are gone.
pub fn prune_strips(p: &mut Panel) {
    let ids: Vec<u32> = p.strips.iter().map(|s| s.id).collect();
    for it in &mut p.items {
        if it.strip.is_some_and(|s| !ids.contains(&s)) {
            it.strip = None;
        }
    }
    let used: Vec<u32> = p.items.iter().filter_map(|i| i.strip).collect();
    p.strips.retain(|s| used.contains(&s.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_panels_read_and_write_as_before() {
        let text = r#"{"name":"Live","items":[{"node":"1"},{"node":"2","size":[2,1]}]}"#;
        let p: Panel = serde_json::from_str(text).unwrap();
        assert!(!p.is_placed());
        assert_eq!(serde_json::to_string(&p).unwrap(), text);
    }

    #[test]
    fn places_strips_and_labels_round_trip() {
        let mut p = Panel::new("Mixer");
        let mut a = PanelItem::new(NodeId(1));
        a.strip = Some(3);
        a.compact = true;
        a.color = Some("green".into());
        p.items.push(a);
        let mut b = PanelItem::new(NodeId(2));
        b.at = Some([4, 2]);
        p.items.push(b);
        let mut s = Strip::new(3, "Drums");
        s.at = Some([0, 1]);
        p.strips.push(s);
        p.labels.push(PanelLabel {
            text: "Tracks".into(),
            at: [0, 0],
            span: 4,
        });
        let text = serde_json::to_string(&p).unwrap();
        let back: Panel = serde_json::from_str(&text).unwrap();
        assert_eq!(back, p);
        assert!(back.is_placed());
        assert_eq!(back.strip_items(3).count(), 1);
        assert_eq!(back.fresh_strip_id(), 4);
    }

    #[test]
    fn out_of_range_places_and_widths_are_pulled_in() {
        let mut it = PanelItem::new(NodeId(1));
        it.at = Some([u32::MAX, u32::MAX]);
        assert_eq!(it.place(), Some([MAX_PLACE, MAX_PLACE * LINES_PER_CELL]));
        let mut s = Strip::new(1, "x");
        s.width = 0;
        assert_eq!(s.width(), 1);
        s.width = 99;
        assert_eq!(s.width(), MAX_SPAN);
        let l = PanelLabel {
            text: "  ".into(),
            at: [0, 0],
            span: 0,
        };
        assert!(l.is_separator());
        assert_eq!(l.span(), 1);
    }

    #[test]
    fn pruning_drops_empty_strips_and_dangling_strip_refs() {
        let mut g = Graph::default();
        g.nodes.push(NodeInstance::new(NodeId(1), "toggle"));
        let mut p = Panel::new("Mixer");
        let mut a = PanelItem::new(NodeId(1));
        a.strip = Some(9);
        p.items.push(a);
        let mut b = PanelItem::new(NodeId(2));
        b.strip = Some(1);
        p.items.push(b);
        p.strips.push(Strip::new(1, "Gone"));
        let mut panels = vec![p];
        assert_eq!(prune(&mut panels, &g), 1);
        assert!(panels[0].strips.is_empty());
        assert_eq!(panels[0].items[0].strip, None);
    }
}
