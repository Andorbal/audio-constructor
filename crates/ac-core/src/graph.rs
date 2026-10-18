//! The graph model: node instances, wires, frames.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::id::{GroupId, NodeId};
use crate::value::{Config, Value, config_is_empty, empty_config};

/// Registry key of the built-in group-instance node.
pub const GROUP_KIND: &str = "group";
/// Registry key of the reroute dot, which the compiler removes.
pub const REROUTE_KIND: &str = "reroute";
/// Registry key of the polyphony node: a group definition run once per voice.
pub const VOICES_KIND: &str = "voices";
/// Registry key of the node inside a `Voices` body that reports its voice.
pub const VOICE_KIND: &str = "voice";

/// A graph: node instances and the wires between them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<NodeInstance>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    /// Labeled rectangles that move their contents (a canvas feature).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,
}

impl Graph {
    pub fn node(&self, id: NodeId) -> Option<&NodeInstance> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut NodeInstance> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    pub fn ids(&self) -> BTreeSet<NodeId> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    /// The wire feeding an input, if any (an input accepts at most one wire).
    pub fn wire_into(&self, to: &Endpoint) -> Option<&Wire> {
        self.wires.iter().find(|w| &w.to == to)
    }

    pub fn wires_from<'a>(&'a self, node: NodeId) -> impl Iterator<Item = &'a Wire> + 'a {
        self.wires.iter().filter(move |w| w.from.node == node)
    }

    pub fn wires_to<'a>(&'a self, node: NodeId) -> impl Iterator<Item = &'a Wire> + 'a {
        self.wires.iter().filter(move |w| w.to.node == node)
    }

    /// Add a wire, replacing any wire already feeding that input.
    pub fn connect(&mut self, from: Endpoint, to: Endpoint) {
        self.wires.retain(|w| w.to != to);
        self.wires.push(Wire { from, to });
    }

    pub fn disconnect_input(&mut self, to: &Endpoint) {
        self.wires.retain(|w| &w.to != to);
    }

    /// Remove a node and every wire touching it.
    pub fn remove_node(&mut self, id: NodeId) -> Option<NodeInstance> {
        let idx = self.nodes.iter().position(|n| n.id == id)?;
        self.wires.retain(|w| w.from.node != id && w.to.node != id);
        Some(self.nodes.remove(idx))
    }

    /// Group defs referenced directly by nodes in this graph.
    pub fn referenced_groups(&self) -> BTreeSet<GroupId> {
        self.nodes.iter().filter_map(|n| n.group_def()).collect()
    }
}

/// One node in a graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeInstance {
    pub id: NodeId,
    /// Registry key, e.g. `"oscillator"`, or `"group"` for a group instance.
    pub kind: String,
    /// Settings that change the socket list. A JSON object.
    #[serde(default = "empty_config", skip_serializing_if = "config_is_empty")]
    pub config: Config,
    /// Values for unwired inputs, by socket name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub ui: NodeUi,
    /// Bypassed: passes `in` to `out` where the types match, else silence.
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    /// Fields written by a newer version, preserved on save.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl NodeInstance {
    pub fn new(id: NodeId, kind: impl Into<String>) -> Self {
        NodeInstance {
            id,
            kind: kind.into(),
            config: empty_config(),
            params: BTreeMap::new(),
            ui: NodeUi::default(),
            muted: false,
            extra: serde_json::Map::new(),
        }
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.ui.pos = [x, y];
        self
    }

    pub fn with_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    pub fn with_param(mut self, socket: &str, value: impl Into<Value>) -> Self {
        self.params.insert(socket.to_string(), value.into());
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.ui.label = Some(label.to_string());
        self
    }

    /// For a group instance (or a `Voices` node, whose body is a group
    /// definition), the definition it references.
    pub fn group_def(&self) -> Option<GroupId> {
        if self.kind == GROUP_KIND || self.kind == VOICES_KIND {
            self.config
                .get("def")
                .and_then(|v| v.as_str())
                .map(|s| GroupId(s.to_string()))
        } else {
            None
        }
    }
}

/// Editor-only state for a node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeUi {
    /// Canvas position of the top-left corner.
    pub pos: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// User-visible name override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One end of a wire: a node and one of its sockets, by name.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Endpoint {
    pub node: NodeId,
    pub socket: String,
}

impl Endpoint {
    pub fn new(node: NodeId, socket: impl Into<String>) -> Self {
        Endpoint {
            node,
            socket: socket.into(),
        }
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.node, self.socket)
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl FromStr for Endpoint {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (node, socket) = s
            .split_once(':')
            .ok_or_else(|| format!("invalid endpoint `{s}` (expected `node:socket`)"))?;
        if socket.is_empty() {
            return Err(format!("invalid endpoint `{s}` (empty socket name)"));
        }
        Ok(Endpoint {
            node: node.parse()?,
            socket: socket.to_string(),
        })
    }
}

impl Serialize for Endpoint {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Endpoint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A connection from an output socket to an input socket.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Wire {
    pub from: Endpoint,
    pub to: Endpoint,
}

impl Wire {
    pub fn new(from: Endpoint, to: Endpoint) -> Self {
        Wire { from, to }
    }
}

/// A labeled frame around part of the canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub label: String,
    /// `[x, y, width, height]` in canvas units.
    pub rect: [f32; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_parses_and_prints() {
        let e: Endpoint = "a1:out".parse().unwrap();
        assert_eq!(e, Endpoint::new(NodeId(0xa1), "out"));
        assert_eq!(e.to_string(), "a1:out");
        assert!("nope".parse::<Endpoint>().is_err());
        assert!("1:".parse::<Endpoint>().is_err());
    }

    #[test]
    fn connect_replaces_existing_input_wire() {
        let mut g = Graph::default();
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(3), "in"),
        );
        g.connect(
            Endpoint::new(NodeId(2), "out"),
            Endpoint::new(NodeId(3), "in"),
        );
        assert_eq!(g.wires.len(), 1);
        assert_eq!(g.wires[0].from.node, NodeId(2));
    }
}
