//! What the node help's "Used in" lists (UX.md: "Every node's help has
//! 'try this' tips and links to blocks that use it"): the library's blocks
//! and the shipped example projects whose graphs hold a node kind, inside
//! nested groups too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ac_core::{Block, Graph, GroupDef, GroupId, Project};
use ac_io::{Library, LibraryBlock};

/// Every node kind in `graph` and in the definitions its group nodes
/// reach through `defs` (each definition once, so a cycle can't hang it).
/// Definitions in `defs` that nothing reaches don't count.
pub fn kinds_in(graph: &Graph, defs: &BTreeMap<GroupId, GroupDef>) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![graph];
    while let Some(g) = stack.pop() {
        for n in &g.nodes {
            kinds.insert(n.kind.clone());
            if let Some(d) = n.group_def()
                && seen.insert(d.clone())
                && let Some(def) = defs.get(&d)
            {
                stack.push(&def.graph);
            }
        }
    }
    kinds
}

/// The node kinds a block is made of, with the definitions it `uses`.
pub fn block_kinds(block: &Block) -> BTreeSet<String> {
    kinds_in(&block.def.graph, &block.uses)
}

/// The node kinds in a project, inside its groups too.
pub fn project_kinds(project: &Project) -> BTreeSet<String> {
    kinds_in(&project.graph, &project.group_defs)
}

/// The library's blocks holding `kind`: shipped first, then the person's
/// own and imported ones, each by name.
pub fn blocks_using<'a>(library: &'a Library, kind: &str) -> Vec<&'a LibraryBlock> {
    library
        .search("")
        .into_iter()
        .filter(|b| block_kinds(&b.block).contains(kind))
        .collect()
}

/// A shipped example project, read for its name and the kinds it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    pub name: String,
    pub path: PathBuf,
    pub kinds: BTreeSet<String>,
}

/// The example projects (`*.acproj`) in `dir`, by name. Files that don't
/// read are left out: help is no place to report them.
pub fn load_examples(dir: &Path) -> Vec<Example> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Example> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "acproj"))
        .filter_map(|path| {
            let project = ac_io::load_project(&path).ok()?;
            let name = if project.name.trim().is_empty() {
                title_from_stem(&path)
            } else {
                project.name.trim().to_string()
            };
            Some(Example {
                name,
                kinds: project_kinds(&project),
                path,
            })
        })
        .collect();
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// The examples holding `kind`.
pub fn examples_using<'a>(examples: &'a [Example], kind: &str) -> Vec<&'a Example> {
    examples.iter().filter(|e| e.kinds.contains(kind)).collect()
}

/// `guitar-pedalboard.acproj` → "Guitar pedalboard".
fn title_from_stem(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().replace(['-', '_'], " "))
        .unwrap_or_default();
    let mut c = stem.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => "Example".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{GROUP_KIND, NodeId, NodeInstance, VOICES_KIND};
    use serde_json::json;

    fn def(name: &str, nodes: Vec<NodeInstance>) -> GroupDef {
        let mut d = GroupDef::new(name);
        d.graph.nodes = nodes;
        d
    }

    fn node(id: u64, kind: &str) -> NodeInstance {
        NodeInstance::new(NodeId(id), kind)
    }

    fn group(id: u64, kind: &str, def: &str) -> NodeInstance {
        NodeInstance::new(NodeId(id), kind).with_config(json!({ "def": def }))
    }

    #[test]
    fn a_block_holds_its_own_kinds() {
        let block = Block {
            def: def("Fx", vec![node(1, "gain"), node(2, "biquad")]),
            uses: BTreeMap::new(),
            preview: None,
        };
        let kinds = block_kinds(&block);
        assert!(kinds.contains("gain") && kinds.contains("biquad"));
        assert!(!kinds.contains("delay"));
    }

    #[test]
    fn nested_definitions_count_through_uses() {
        let mut uses = BTreeMap::new();
        uses.insert(
            GroupId("inner".into()),
            def(
                "Inner",
                vec![node(1, "gain_computer"), group(2, VOICES_KIND, "deep")],
            ),
        );
        uses.insert(GroupId("deep".into()), def("Deep", vec![node(1, "adsr")]));
        // Carried along but not used by anything: doesn't count.
        uses.insert(
            GroupId("spare".into()),
            def("Spare", vec![node(1, "delay")]),
        );
        let block = Block {
            def: def("Outer", vec![group(1, GROUP_KIND, "inner"), node(2, "mix")]),
            uses,
            preview: None,
        };
        let kinds = block_kinds(&block);
        for k in ["gain_computer", "adsr", "mix", GROUP_KIND, VOICES_KIND] {
            assert!(kinds.contains(k), "{k}");
        }
        assert!(
            !kinds.contains("delay"),
            "unreached definitions don't count"
        );
    }

    #[test]
    fn cycles_and_missing_definitions_are_harmless() {
        let mut uses = BTreeMap::new();
        uses.insert(
            GroupId("a".into()),
            def("A", vec![group(1, GROUP_KIND, "a"), node(2, "noise")]),
        );
        let block = Block {
            def: def(
                "Loop",
                vec![group(1, GROUP_KIND, "a"), group(2, GROUP_KIND, "gone")],
            ),
            uses,
            preview: None,
        };
        let kinds = block_kinds(&block);
        assert!(kinds.contains("noise"));
    }

    #[test]
    fn shipped_blocks_using_the_gain_computer() {
        let lib = Library::shipped_only();
        let names: Vec<&str> = blocks_using(&lib, "gain_computer")
            .iter()
            .map(|b| b.name())
            .collect();
        assert!(names.contains(&"Compressor"), "{names:?}");
        // The multiband compressor holds it only inside a nested definition.
        assert!(names.contains(&"Multiband Compressor"), "{names:?}");
        assert!(blocks_using(&lib, "no_such_node").is_empty());
    }

    #[test]
    fn examples_read_from_a_folder() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let examples = load_examples(&dir);
        assert!(examples.len() >= 4, "{examples:?}");
        let start = examples.iter().find(|e| e.path.ends_with("start.acproj"));
        assert!(start.is_some_and(|e| e.kinds.contains("oscillator")));
        let using = examples_using(&examples, "audio_out");
        assert_eq!(using.len(), examples.len(), "every example plays");
        assert!(load_examples(Path::new("/no/such/folder")).is_empty());
        assert_eq!(
            title_from_stem(Path::new("x/guitar-pedalboard.acproj")),
            "Guitar pedalboard"
        );
    }
}
