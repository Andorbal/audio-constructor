//! Identifiers for nodes and group definitions, and the paths that address
//! nodes inside flattened groups.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A node's stable identity within its graph.
///
/// Serialized as a lowercase hexadecimal string so files stay readable and
/// survive tools that mangle large JSON numbers. Hand-written projects can use
/// short ids like `"1"`; generated ids are random 64-bit values.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u64);

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({self})")
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

impl FromStr for NodeId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        u64::from_str_radix(s, 16)
            .map(NodeId)
            .map_err(|_| format!("invalid node id `{s}` (expected hexadecimal)"))
    }
}

impl Serialize for NodeId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Identity of a group definition embedded in a project.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub String);

impl fmt::Debug for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GroupId({})", self.0)
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for GroupId {
    fn from(s: &str) -> Self {
        GroupId(s.to_string())
    }
}

/// The address of a node after group flattening: the chain of instance ids
/// from the root graph down to the node. A root-level node has a path of
/// length one.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodePath(pub Vec<NodeId>);

impl NodePath {
    pub fn root(id: NodeId) -> Self {
        NodePath(vec![id])
    }

    pub fn child(&self, id: NodeId) -> Self {
        let mut v = self.0.clone();
        v.push(id);
        NodePath(v)
    }

    /// The node's own id (last element).
    pub fn leaf(&self) -> Option<NodeId> {
        self.0.last().copied()
    }

    /// The path of the group instance that contains this node, if any.
    pub fn parent(&self) -> Option<NodePath> {
        if self.0.len() > 1 {
            Some(NodePath(self.0[..self.0.len() - 1].to_vec()))
        } else {
            None
        }
    }

    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// A stable 64-bit hash of the path, used to derive per-node PRNG seeds.
    pub fn stable_hash(&self) -> u64 {
        let mut h = crate::util::Fnv64::new();
        for id in &self.0 {
            h.write(&id.0.to_le_bytes());
        }
        h.finish()
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, id) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{id}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodePath({self})")
    }
}

impl FromStr for NodePath {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ids: Result<Vec<_>, _> = s.split('/').map(NodeId::from_str).collect();
        let ids = ids?;
        if ids.is_empty() {
            return Err("empty node path".into());
        }
        Ok(NodePath(ids))
    }
}

/// Serialized as its display form (`"1/ab"`), like node ids.
impl Serialize for NodePath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodePath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Generates fresh node ids. Deterministic for a given seed, so tests and
/// paste operations are reproducible; the app seeds it from the clock.
#[derive(Debug, Clone)]
pub struct IdGen {
    state: u64,
}

impl IdGen {
    pub fn new(seed: u64) -> Self {
        IdGen { state: seed }
    }

    /// Next id that `taken` reports as unused. Never returns zero.
    pub fn fresh(&mut self, taken: impl Fn(NodeId) -> bool) -> NodeId {
        loop {
            let v = crate::util::splitmix64(&mut self.state);
            let id = NodeId(v);
            if v != 0 && !taken(id) {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_round_trips_as_hex() {
        let id = NodeId(0xdead_beef);
        let s = serde_json::to_string(&id).unwrap();
        assert_eq!(s, "\"deadbeef\"");
        let back: NodeId = serde_json::from_str(&s).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn bad_node_id_is_an_error_not_a_panic() {
        assert!(serde_json::from_str::<NodeId>("\"xyz\"").is_err());
    }

    #[test]
    fn path_display_and_parse() {
        let p = NodePath(vec![NodeId(1), NodeId(0xab)]);
        assert_eq!(p.to_string(), "1/ab");
        assert_eq!("1/ab".parse::<NodePath>().unwrap(), p);
        assert_eq!(p.parent(), Some(NodePath::root(NodeId(1))));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"1/ab\"");
        assert_eq!(serde_json::from_str::<NodePath>(&json).unwrap(), p);
        assert!(serde_json::from_str::<NodePath>("\"1//x\"").is_err());
    }

    #[test]
    fn id_gen_is_deterministic_and_skips_taken() {
        let mut a = IdGen::new(7);
        let mut b = IdGen::new(7);
        let first = a.fresh(|_| false);
        assert_eq!(first, b.fresh(|_| false));
        let mut c = IdGen::new(7);
        let skipped = c.fresh(|id| id == first);
        assert_ne!(skipped, first);
    }
}
