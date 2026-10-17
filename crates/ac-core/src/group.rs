//! Group definitions: reusable subgraphs with their own sockets.

use serde::{Deserialize, Serialize};

use crate::graph::{Endpoint, Graph};
use crate::signal::{SignalType, Unit};
use crate::value::Value;

/// A reusable subgraph. Every `Group` node instance references one by id;
/// editing the definition updates every instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The library block this definition came from, so the app can offer updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub inputs: Vec<GroupInput>,
    #[serde(default)]
    pub outputs: Vec<GroupOutput>,
    pub graph: Graph,
}

fn one() -> u32 {
    1
}

impl GroupDef {
    pub fn new(name: impl Into<String>) -> Self {
        GroupDef {
            name: name.into(),
            description: String::new(),
            version: 1,
            author: String::new(),
            tags: Vec::new(),
            source: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            graph: Graph::default(),
        }
    }
}

/// An exposed input: a socket on every instance, feeding inner sockets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupInput {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: SignalType,
    #[serde(default, skip_serializing_if = "is_generic")]
    pub unit: Unit,
    /// Value used when the instance's socket is unwired and has no param.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Inner input sockets this input feeds.
    pub targets: Vec<Endpoint>,
}

/// An exposed output: a socket on every instance, fed by one inner output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupOutput {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: SignalType,
    pub source: Endpoint,
}

fn is_generic(u: &Unit) -> bool {
    *u == Unit::Generic
}
