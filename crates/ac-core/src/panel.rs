//! Performance panels (docs/UX.md "Panels"): named grids of pinned nodes.
//!
//! A panel is a view. It holds nothing but references to nodes in the
//! project's root graph, in the order they are laid out, so deleting a node
//! is all it takes to take it off every panel, and a panel never changes
//! what the patch does.

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, NodeInstance};
use crate::id::NodeId;

/// The largest span, in grid cells, an item can take either way.
pub const MAX_SPAN: u32 = 8;

/// One panel: a name and the nodes pinned to it, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PanelItem>,
    /// Fields written by a newer version, preserved on save.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Panel {
    pub fn new(name: impl Into<String>) -> Panel {
        Panel {
            name: name.into(),
            items: Vec::new(),
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
}

impl PanelItem {
    pub fn new(node: NodeId) -> PanelItem {
        PanelItem { node, size: None }
    }

    /// The item's span, or `natural` without a size of its own, each way
    /// between 1 and [`MAX_SPAN`] cells.
    pub fn span(&self, natural: [u32; 2]) -> [u32; 2] {
        let [c, r] = self.size.unwrap_or(natural);
        [c.clamp(1, MAX_SPAN), r.clamp(1, MAX_SPAN)]
    }
}

/// Drop pins to nodes that aren't in `graph` (deleted, moved into a group,
/// or never there). Returns how many were dropped.
pub fn prune(panels: &mut [Panel], graph: &Graph) -> usize {
    let mut dropped = 0;
    for p in panels {
        let before = p.items.len();
        p.items.retain(|i| graph.contains(i.node));
        dropped += before - p.items.len();
    }
    dropped
}
