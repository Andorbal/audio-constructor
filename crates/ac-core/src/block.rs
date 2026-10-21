//! Blocks: group definitions saved in files of their own (`.acblock`), so a
//! patch built once can be reused in any project (ARCHITECTURE §12).
//!
//! A block file is a [`GroupDef`] (name, description, version, author,
//! tags, interface, graph) with the project `schema_version`, plus the
//! definitions its own group nodes use. Projects embed a copy of every
//! block they use, and the copy's `source` names the library block it came
//! from (`shipped:compressor`, `user:my-bass`), so the editor can offer to
//! update instances when the library has a newer version.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clipboard::{SUBGRAPH_FORMAT, Subgraph, merge_defs};
use crate::graph::{GROUP_KIND, Graph, NodeInstance, VOICE_KIND, VOICES_KIND};
use crate::group::GroupDef;
use crate::id::{GroupId, NodeId};
use crate::project::{LoadError, Project, SCHEMA_VERSION, migrate, to_canonical_json};
use crate::registry::{group_signature, voices_signature};

/// File extension of block files.
pub const BLOCK_EXTENSION: &str = "acblock";

/// What a block's preview file is called, next to the block file:
/// `<id>.preview.wav`.
pub const PREVIEW_SUFFIX: &str = ".preview.wav";

/// A block: its definition and the definitions its group nodes use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    #[serde(flatten)]
    pub def: GroupDef,
    /// Definitions referenced by group nodes inside `def` (and inside them).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub uses: BTreeMap<GroupId, GroupDef>,
    /// The short recording of the block that sits next to its file, if it
    /// has one. It describes the file, not the definition: projects don't
    /// copy it, and two blocks that differ only here are the same block
    /// ([`Block::same_definition`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<BlockPreview>,
}

/// A block's preview: a few seconds of it playing, rendered offline
/// (DECISIONS D190), kept as a WAV file beside the block file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockPreview {
    /// The WAV file's name, in the block file's folder.
    pub file: String,
    /// SHA-256 of the file, as 64 lowercase hex digits.
    pub sha256: String,
}

/// The preview file name for block `id`.
pub fn preview_file_name(id: &str) -> String {
    format!("{id}{PREVIEW_SUFFIX}")
}

#[derive(Serialize)]
struct BlockFileOut<'a> {
    schema_version: u32,
    #[serde(flatten)]
    block: &'a Block,
}

impl Block {
    /// Parse an `.acblock` file, upgrading older formats.
    pub fn from_json(text: &str) -> Result<Block, LoadError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        let mut value = migrate(value)?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("schema_version");
        }
        let mut block: Block =
            serde_json::from_value(value).map_err(|e| LoadError::Parse(e.to_string()))?;
        // The file is the library's copy; where it came from is the
        // library's business, not the file's.
        block.def.source = None;
        // A preview is a file beside this one: a name that could point
        // anywhere else isn't one.
        if block
            .preview
            .as_ref()
            .is_some_and(|p| !is_plain_file_name(&p.file))
        {
            block.preview = None;
        }
        Ok(block)
    }

    /// Whether `other` is the same block: the same definition and the same
    /// definitions inside, whatever their files say about previews.
    pub fn same_definition(&self, other: &Block) -> bool {
        self.def == other.def && self.uses == other.uses
    }

    /// Whether the block plays notes as a polyphonic instrument: it reads a
    /// Voice node, so it's used as a Voices node.
    pub fn is_polyphonic(&self) -> bool {
        self.def.graph.nodes.iter().any(|n| n.kind == VOICE_KIND)
    }

    /// The canonical file form.
    pub fn to_json(&self) -> String {
        to_canonical_json(&BlockFileOut {
            schema_version: SCHEMA_VERSION,
            block: self,
        })
    }

    /// The block made from definition `id` of `project`, with every
    /// definition it uses. `None` if the project has no such definition.
    pub fn from_project(project: &Project, id: &GroupId) -> Option<Block> {
        let mut def = project.group_defs.get(id)?.clone();
        def.source = None;
        let mut uses = BTreeMap::new();
        let mut stack: Vec<GroupId> = defs_used(&def.graph).into_iter().collect();
        while let Some(d) = stack.pop() {
            if &d == id || uses.contains_key(&d) {
                continue;
            }
            if let Some(inner) = project.group_defs.get(&d) {
                stack.extend(defs_used(&inner.graph));
                uses.insert(d, inner.clone());
            }
        }
        Some(Block {
            def,
            uses,
            preview: None,
        })
    }

    /// A subgraph holding one instance of this block, its definition stored
    /// under `id` and tagged with `source`, ready to paste into a project
    /// (which merges the definitions: an identical one already there is
    /// reused, a different one with the same id gets a new id).
    pub fn instance(&self, id: &GroupId, source: &BlockSource, polyphonic: bool) -> Subgraph {
        let mut def = self.def.clone();
        def.source = Some(source.to_string());
        let mut group_defs = self.uses.clone();
        group_defs.insert(id.clone(), def);
        let kind = if polyphonic { VOICES_KIND } else { GROUP_KIND };
        Subgraph {
            format: SUBGRAPH_FORMAT.to_string(),
            version: 1,
            nodes: vec![
                NodeInstance::new(NodeId(1), kind)
                    .with_config(serde_json::json!({ "def": id.0 }))
                    .with_label(&self.def.name),
            ],
            wires: Vec::new(),
            group_defs,
        }
    }
}

/// Whether `name` is a file name and nothing more: no folders, no `..` or
/// other leading dot, no drive letter, nothing a file system would read
/// specially. Spaces and other letters are fine.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
}

/// Definitions referenced by group and voices nodes in `graph`.
pub fn defs_used(graph: &Graph) -> BTreeSet<GroupId> {
    graph
        .nodes
        .iter()
        .filter_map(NodeInstance::group_def)
        .collect()
}

/// Which library a block lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// Ships with the app (`blocks/` in the repository).
    Shipped,
    /// Saved by the person from their own groups.
    User,
    /// Came from someone else: a pack, or a block file opened from disk.
    Imported,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Shipped => "shipped",
            Origin::User => "user",
            Origin::Imported => "imported",
        }
    }

    pub const ALL: [Origin; 3] = [Origin::Shipped, Origin::User, Origin::Imported];
}

/// A library block's identity: `<origin>:<id>`, stored in a definition's
/// `source`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockSource {
    pub origin: Origin,
    pub id: String,
}

impl BlockSource {
    pub fn new(origin: Origin, id: impl Into<String>) -> BlockSource {
        BlockSource {
            origin,
            id: id.into(),
        }
    }

    /// Parse `shipped:compressor`; `None` for anything else.
    pub fn parse(s: &str) -> Option<BlockSource> {
        let (origin, id) = s.split_once(':')?;
        let origin = Origin::ALL.into_iter().find(|o| o.name() == origin)?;
        (!id.is_empty()).then(|| BlockSource::new(origin, id))
    }
}

impl fmt::Display for BlockSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.origin.name(), self.id)
    }
}

/// A file-name-safe id for a block name: lowercase ASCII letters, digits,
/// and single dashes (`Sidechain Ducker` → `sidechain-ducker`).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "block".to_string()
    } else {
        out
    }
}

/// What [`update_def`] changed besides the definition itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateReport {
    /// Wires removed because the new version has no socket of that name,
    /// as `(instance, socket)`.
    pub dropped_wires: Vec<(NodeId, String)>,
}

/// Replace definition `id` with `block`'s (keeping the id and setting
/// `source`), merge the definitions it uses, and remove wires into or out
/// of instances whose sockets the new version no longer has. Instance
/// parameters stay, so values set on sockets that still exist carry over.
pub fn update_def(
    project: &mut Project,
    id: &GroupId,
    block: &Block,
    source: &BlockSource,
) -> UpdateReport {
    let incoming = block.instance(id, source, false).group_defs;
    // The definition being replaced mustn't make the merge rename it.
    project.group_defs.remove(id);
    merge_defs(&incoming, &mut project.group_defs);
    let mut report = UpdateReport::default();
    let Some(def) = project.group_defs.get(id).cloned() else {
        return report;
    };
    let sockets = |kind: &str| -> (BTreeSet<String>, BTreeSet<String>) {
        let sig = if kind == VOICES_KIND {
            voices_signature(&def)
        } else {
            group_signature(&def)
        };
        (
            sig.inputs.iter().map(|s| s.name.to_string()).collect(),
            sig.outputs.iter().map(|s| s.name.to_string()).collect(),
        )
    };
    let mut prune = |graph: &mut Graph| {
        let instances: BTreeMap<NodeId, (BTreeSet<String>, BTreeSet<String>)> = graph
            .nodes
            .iter()
            .filter(|n| n.group_def().as_ref() == Some(id))
            .map(|n| (n.id, sockets(&n.kind)))
            .collect();
        graph.wires.retain(|w| {
            if let Some((ins, _)) = instances.get(&w.to.node)
                && !ins.contains(&w.to.socket)
            {
                report.dropped_wires.push((w.to.node, w.to.socket.clone()));
                return false;
            }
            if let Some((_, outs)) = instances.get(&w.from.node)
                && !outs.contains(&w.from.socket)
            {
                report
                    .dropped_wires
                    .push((w.from.node, w.from.socket.clone()));
                return false;
            }
            true
        });
    };
    prune(&mut project.graph);
    for d in project.group_defs.values_mut() {
        prune(&mut d.graph);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Endpoint;
    use crate::group::{GroupInput, GroupOutput};
    use crate::signal::{SignalType, Unit};

    /// gain in → out, with `amount` exposed.
    fn def(name: &str, exposed: &[&str]) -> GroupDef {
        let mut d = GroupDef::new(name);
        d.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
        for e in exposed {
            d.inputs.push(GroupInput {
                name: (*e).into(),
                ty: SignalType::Audio,
                unit: Unit::Generic,
                default: None,
                range: None,
                targets: vec![Endpoint::new(NodeId(1), "in")],
            });
        }
        d.outputs.push(GroupOutput {
            name: "out".into(),
            ty: SignalType::Audio,
            source: Endpoint::new(NodeId(1), "out"),
        });
        d
    }

    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(slug("Sidechain Ducker"), "sidechain-ducker");
        assert_eq!(slug("  Karplus–Strong String! "), "karplus-strong-string");
        assert_eq!(slug("FM Operator Pair (2)"), "fm-operator-pair-2");
        assert_eq!(slug("???"), "block");
    }

    #[test]
    fn sources_round_trip() {
        let s = BlockSource::new(Origin::Shipped, "compressor");
        assert_eq!(s.to_string(), "shipped:compressor");
        assert_eq!(BlockSource::parse("shipped:compressor"), Some(s));
        assert_eq!(
            BlockSource::parse("user:x").map(|s| s.origin),
            Some(Origin::User)
        );
        assert_eq!(BlockSource::parse("elsewhere:x"), None);
        assert_eq!(BlockSource::parse("user:"), None);
        assert_eq!(BlockSource::parse("nocolon"), None);
    }

    #[test]
    fn block_files_round_trip_with_nested_definitions() {
        let mut p = Project::default();
        let mut outer = def("Outer", &["in"]);
        outer.version = 3;
        outer.tags = vec!["dynamics".into()];
        outer.source = Some("user:outer".into());
        outer.graph.nodes.push(
            NodeInstance::new(NodeId(2), GROUP_KIND)
                .with_config(serde_json::json!({ "def": "inner" })),
        );
        p.group_defs.insert(GroupId("outer".into()), outer);
        p.group_defs
            .insert(GroupId("inner".into()), def("Inner", &["in"]));
        p.group_defs
            .insert(GroupId("unrelated".into()), def("Other", &[]));
        let b = Block::from_project(&p, &GroupId("outer".into())).unwrap();
        assert_eq!(
            b.uses.keys().map(|k| k.0.as_str()).collect::<Vec<_>>(),
            ["inner"]
        );
        assert_eq!(
            b.def.source, None,
            "the file doesn't say where it came from"
        );
        let text = b.to_json();
        let current = format!("\"schema_version\": {}", crate::project::SCHEMA_VERSION);
        assert!(text.contains(&current));
        assert!(text.contains("\"version\": 3"));
        assert_eq!(Block::from_json(&text).unwrap(), b);
        assert!(matches!(
            Block::from_json(&text.replace(&current, "\"schema_version\": 99")),
            Err(LoadError::TooNew { .. })
        ));
        assert!(Block::from_json("{").is_err());
    }

    #[test]
    fn a_preview_belongs_to_the_file_not_the_definition() {
        let mut b = Block {
            def: def("Comp", &["in"]),
            uses: BTreeMap::new(),
            preview: Some(BlockPreview {
                file: preview_file_name("comp"),
                sha256: "ab".repeat(32),
            }),
        };
        let text = b.to_json();
        assert!(text.contains("\"file\": \"comp.preview.wav\""), "{text}");
        assert_eq!(Block::from_json(&text).unwrap(), b);
        // A project's copy has no preview, and is still the same block.
        let mut p = Project::default();
        p.group_defs.insert(GroupId("comp".into()), b.def.clone());
        let copy = Block::from_project(&p, &GroupId("comp".into())).unwrap();
        assert_eq!(copy.preview, None);
        assert!(copy.same_definition(&b));
        assert_ne!(copy, b);
        // A name that would reach outside the block's folder isn't kept.
        for bad in [
            "../x.wav",
            "/etc/x.wav",
            "a/b.wav",
            "a\\b.wav",
            ".hidden.wav",
            "c:x.wav",
            "",
        ] {
            b.preview.as_mut().unwrap().file = bad.into();
            let back = Block::from_json(&b.to_json()).unwrap();
            assert_eq!(back.preview, None, "{bad}");
        }
        assert!(is_plain_file_name("My Comp.preview.wav"));
        // Older apps' files (no preview) still read.
        b.preview = None;
        assert!(!b.to_json().contains("preview"));
    }

    #[test]
    fn instruments_are_the_blocks_with_a_voice_inside() {
        let mut b = Block {
            def: def("Synth", &[]),
            uses: BTreeMap::new(),
            preview: None,
        };
        assert!(!b.is_polyphonic());
        b.def
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(9), VOICE_KIND));
        assert!(b.is_polyphonic());
    }

    #[test]
    fn an_instance_pastes_with_its_source() {
        let b = Block {
            def: def("Comp", &["in"]),
            uses: BTreeMap::new(),
            preview: None,
        };
        let src = BlockSource::new(Origin::Shipped, "comp");
        let s = b.instance(&GroupId("comp".into()), &src, false);
        let mut p = Project::default();
        let mut ids = crate::id::IdGen::new(7);
        let new = s.paste(&mut p.graph, &mut p.group_defs, &mut ids, [10.0, 20.0]);
        assert_eq!(new.len(), 1);
        let n = p.graph.node(new[0]).unwrap();
        assert_eq!(n.kind, GROUP_KIND);
        assert_eq!(n.ui.label.as_deref(), Some("Comp"));
        let d = &p.group_defs[&GroupId("comp".into())];
        assert_eq!(d.source.as_deref(), Some("shipped:comp"));
        // A second instance reuses the same definition.
        s.paste(&mut p.graph, &mut p.group_defs, &mut ids, [0.0, 0.0]);
        assert_eq!(p.group_defs.len(), 1);
        let poly = b.instance(&GroupId("comp".into()), &src, true);
        assert_eq!(poly.nodes[0].kind, VOICES_KIND);
    }

    #[test]
    fn updating_keeps_params_and_drops_wires_to_removed_sockets() {
        let mut p = Project::default();
        let id = GroupId("fx".into());
        let mut old = def("Fx", &["in", "side"]);
        old.version = 1;
        p.group_defs.insert(id.clone(), old);
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "oscillator"));
        p.graph.nodes.push(
            NodeInstance::new(NodeId(2), GROUP_KIND)
                .with_config(serde_json::json!({ "def": "fx" }))
                .with_param("in", 0.5),
        );
        p.graph.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        );
        p.graph.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "side"),
        );
        let mut new = def("Fx", &["in"]);
        new.version = 2;
        let block = Block {
            def: new,
            uses: BTreeMap::new(),
            preview: None,
        };
        let src = BlockSource::new(Origin::Shipped, "fx");
        let r = update_def(&mut p, &id, &block, &src);
        assert_eq!(r.dropped_wires, [(NodeId(2), "side".to_string())]);
        assert_eq!(p.graph.wires.len(), 1);
        assert_eq!(p.group_defs[&id].version, 2);
        assert_eq!(p.group_defs[&id].source.as_deref(), Some("shipped:fx"));
        assert_eq!(p.graph.node(NodeId(2)).unwrap().params.len(), 1);
    }
}
