//! The node registry: every node kind the compiler, CLI, palette, and docs
//! generator know about.

use std::collections::BTreeMap;
use std::fmt;

use crate::graph::{GROUP_KIND, NodeInstance, VOICES_KIND};
use crate::group::GroupDef;
use crate::id::GroupId;
use crate::node::{Node, NodeMeta, PrepareCtx};
use crate::signal::{Channels, Signature, SocketSpec};
use crate::value::{Config, Value};
use crate::whole::{WholeCtx, WholeInputs, WholeOutputs};

/// Signature of [`Node::render_whole`].
pub type RenderWholeFn = fn(&Config, &WholeCtx, &WholeInputs) -> WholeOutputs;

/// Function pointers for one node kind.
#[derive(Clone)]
pub struct NodeEntry {
    pub meta: NodeMeta,
    pub describe: fn(&Config) -> Signature,
    pub create: fn(&Config, &PrepareCtx) -> Box<dyn Node>,
    pub render_whole: RenderWholeFn,
}

impl fmt::Debug for NodeEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeEntry")
            .field("kind", &self.meta.kind)
            .finish()
    }
}

fn create_boxed<N: Node>(config: &Config, ctx: &PrepareCtx) -> Box<dyn Node> {
    Box::new(N::new(config, ctx))
}

/// All registered node kinds, keyed by `kind`.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    entries: BTreeMap<&'static str, NodeEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node type. A later registration of the same kind replaces
    /// the earlier one.
    pub fn register<N: Node>(&mut self) -> &mut Self {
        let meta = N::meta();
        self.entries.insert(
            meta.kind,
            NodeEntry {
                meta,
                describe: N::describe,
                create: create_boxed::<N>,
                render_whole: N::render_whole,
            },
        );
        self
    }

    pub fn get(&self, kind: &str) -> Option<&NodeEntry> {
        self.entries.get(kind)
    }

    pub fn contains(&self, kind: &str) -> bool {
        self.entries.contains_key(kind)
    }

    /// Entries in kind order.
    pub fn iter(&self) -> impl Iterator<Item = &NodeEntry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The sockets of a node instance. Group instances take theirs from the
    /// definition's interface. Unknown kinds and missing definitions give `None`.
    pub fn signature_of(
        &self,
        node: &NodeInstance,
        defs: &BTreeMap<GroupId, GroupDef>,
    ) -> Option<Signature> {
        if node.kind == GROUP_KIND {
            let def = defs.get(&node.group_def()?)?;
            return Some(group_signature(def));
        }
        if node.kind == VOICES_KIND {
            let def = defs.get(&node.group_def()?)?;
            return Some(voices_signature(def));
        }
        self.get(&node.kind).map(|e| (e.describe)(&node.config))
    }

    /// Fuzzy search over names, kinds, and keywords, best matches first.
    pub fn search(&self, query: &str) -> Vec<&NodeEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            let mut all: Vec<_> = self.iter().collect();
            all.sort_by_key(|e| (e.meta.category, e.meta.name));
            return all;
        }
        let mut scored: Vec<(i32, &NodeEntry)> = self
            .iter()
            .filter_map(|e| match_score(&q, &e.meta).map(|s| (s, e)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.meta.name.cmp(b.1.meta.name)));
        scored.into_iter().map(|(_, e)| e).collect()
    }
}

/// Signature of a group instance, from its definition's interface.
pub fn group_signature(def: &GroupDef) -> Signature {
    let mut sig = Signature::new();
    for i in &def.inputs {
        let mut s = SocketSpec::any(i.name.clone());
        s.ty = i.ty;
        s.unit = i.unit;
        s.default = i.default.clone().unwrap_or(match i.ty {
            crate::signal::SignalType::Bool => Value::Bool(false),
            crate::signal::SignalType::Int => Value::Int(0),
            _ => Value::Float(0.0),
        });
        s.range = i.range.map(|[a, b]| (a, b));
        s.smooth = i.ty == crate::signal::SignalType::Float;
        sig.inputs.push(s);
    }
    for o in &def.outputs {
        let mut s = SocketSpec::any(o.name.clone())
            .poly()
            .channels(Channels::Follow);
        s.ty = o.ty;
        sig.outputs.push(s);
    }
    sig
}

/// Signature of a `Voices` node: a `midi` input that picks the voices, then
/// the body's interface (inputs reach every voice; outputs are summed over
/// the voices).
pub fn voices_signature(def: &GroupDef) -> Signature {
    let mut sig = group_signature(def);
    sig.inputs.retain(|s| s.name != "midi");
    sig.inputs.insert(
        0,
        SocketSpec::midi("midi").doc("Notes to share out among the voices."),
    );
    sig
}

/// Score how well `q` (lowercase) matches a node; `None` if it doesn't.
fn match_score(q: &str, meta: &NodeMeta) -> Option<i32> {
    let name = meta.name.to_lowercase();
    let kind = meta.kind.to_lowercase();
    if name == q || kind == q {
        return Some(1000);
    }
    let mut best: Option<i32> = None;
    let mut consider = |s: i32| best = Some(best.map_or(s, |b| b.max(s)));
    if name.starts_with(q) || kind.starts_with(q) {
        consider(800 - name.len() as i32);
    }
    for kw in meta.keywords {
        let kw = kw.to_lowercase();
        if kw == q {
            consider(700);
        } else if kw.starts_with(q) {
            consider(600 - kw.len() as i32);
        }
    }
    if name.contains(q) {
        consider(500 - name.len() as i32);
    }
    if let Some(s) = subsequence_score(q, &name) {
        consider(200 + s);
    }
    if q.len() >= 3 && meta.category.name().to_lowercase().contains(q) {
        consider(100);
    }
    best
}

/// Subsequence match ("bqd" in "biquad"); rewards contiguous runs.
fn subsequence_score(q: &str, s: &str) -> Option<i32> {
    let mut chars = s.chars().peekable();
    let mut score = 0;
    let mut run = 0;
    for qc in q.chars() {
        let mut found = false;
        for c in chars.by_ref() {
            if c == qc {
                run += 1;
                score += run;
                found = true;
                break;
            }
            run = 0;
        }
        if !found {
            return None;
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{Category, ProcessCtx};
    use crate::{Io, node};

    node! {
        kind: "test_gain",
        name: "Gain",
        category: Level,
        keywords: ["volume", "amp"],
        /// Scales a signal.
        ///
        /// Try this: wire it.
        pub struct TestGain {}
    }

    impl Node for TestGain {
        fn meta() -> NodeMeta {
            Self::base_meta()
        }
        fn describe(_: &Config) -> Signature {
            Signature::new()
                .input(SocketSpec::audio("in"))
                .output(SocketSpec::audio("out"))
        }
        fn new(_: &Config, _: &PrepareCtx) -> Self {
            TestGain {}
        }
        fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}
        fn reset(&mut self) {}
    }

    node! {
        kind: "test_biquad",
        name: "Biquad",
        category: Filters,
        keywords: ["lpf", "filter", "eq"],
        /// Filters.
        pub struct TestBiquad {}
    }

    impl Node for TestBiquad {
        fn meta() -> NodeMeta {
            Self::base_meta()
        }
        fn describe(_: &Config) -> Signature {
            Signature::new()
        }
        fn new(_: &Config, _: &PrepareCtx) -> Self {
            TestBiquad {}
        }
        fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}
        fn reset(&mut self) {}
    }

    node! {
        kind: "test_mix",
        name: "Mix",
        category: Level,
        keywords: [],
        /// N inputs from config.
        pub struct TestMix {}
    }

    impl Node for TestMix {
        fn meta() -> NodeMeta {
            Self::base_meta()
        }
        fn describe(config: &Config) -> Signature {
            use crate::value::ConfigExt;
            let n = config.get_u64_or("inputs", 2).clamp(1, 64);
            let mut s = Signature::new();
            for i in 0..n {
                s = s.input(SocketSpec::audio(format!("in_{i}")));
            }
            s.output(SocketSpec::audio("out"))
        }
        fn new(_: &Config, _: &PrepareCtx) -> Self {
            TestMix {}
        }
        fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}
        fn reset(&mut self) {}
    }

    #[test]
    fn sockets_follow_config() {
        let mut r = Registry::new();
        r.register::<TestMix>();
        let mut n = NodeInstance::new(crate::NodeId(1), "test_mix");
        let defs = BTreeMap::new();
        assert_eq!(r.signature_of(&n, &defs).unwrap().inputs.len(), 2);
        n.config = serde_json::json!({"inputs": 5});
        let sig = r.signature_of(&n, &defs).unwrap();
        assert_eq!(sig.inputs.len(), 5);
        assert_eq!(sig.input_index("in_4"), Some(4));
    }

    #[test]
    fn group_instances_take_sockets_from_their_definition() {
        use crate::graph::Endpoint;
        use crate::group::{GroupDef, GroupInput, GroupOutput};
        use crate::signal::{SignalType, Unit};
        let mut def = GroupDef::new("Voice");
        def.inputs.push(GroupInput {
            name: "cutoff".into(),
            ty: SignalType::Float,
            unit: Unit::Hz,
            default: Some(Value::Float(800.0)),
            range: Some([20.0, 20000.0]),
            targets: vec![Endpoint::new(crate::NodeId(9), "cutoff")],
        });
        def.outputs.push(GroupOutput {
            name: "out".into(),
            ty: SignalType::Audio,
            source: Endpoint::new(crate::NodeId(9), "out"),
        });
        let defs: BTreeMap<_, _> = [(GroupId::from("v"), def)].into();
        let n = NodeInstance::new(crate::NodeId(1), GROUP_KIND)
            .with_config(serde_json::json!({"def": "v"}));
        let sig = reg().signature_of(&n, &defs).unwrap();
        assert_eq!(sig.inputs[0].name, "cutoff");
        assert_eq!(sig.inputs[0].unit, Unit::Hz);
        assert_eq!(sig.outputs[0].ty, SignalType::Audio);
        let missing = NodeInstance::new(crate::NodeId(2), GROUP_KIND)
            .with_config(serde_json::json!({"def": "nope"}));
        assert!(reg().signature_of(&missing, &defs).is_none());
        // A Voices node adds `midi` ahead of the body's interface.
        let v = NodeInstance::new(crate::NodeId(3), VOICES_KIND)
            .with_config(serde_json::json!({"def": "v"}));
        let sig = reg().signature_of(&v, &defs).unwrap();
        assert_eq!(sig.inputs[0].name, "midi");
        assert_eq!(sig.inputs[1].name, "cutoff");
        assert_eq!(sig.outputs[0].name, "out");
    }

    fn reg() -> Registry {
        let mut r = Registry::new();
        r.register::<TestGain>().register::<TestBiquad>();
        r
    }

    #[test]
    fn macro_captures_docs_and_keywords() {
        let m = TestGain::meta();
        assert_eq!(m.kind, "test_gain");
        assert_eq!(m.category, Category::Level);
        assert_eq!(m.summary(), "Scales a signal.");
        assert!(m.doc_text().contains("Try this: wire it."));
        assert_eq!(m.tip().as_deref(), Some("Wire it."));
        assert_eq!(m.description(), "Scales a signal.");
    }

    #[test]
    fn tips_split_from_the_docs() {
        use crate::node::split_tip;
        let doc = "Does a thing.\n\n**Try this:** wire `out`\ninto a `Meter`.\n\nMore after.";
        let (rest, tip) = split_tip(doc);
        assert_eq!(rest, "Does a thing.\n\nMore after.");
        assert_eq!(tip.as_deref(), Some("Wire `out`\ninto a `Meter`."));
        // Bold label with the colon outside, any case.
        let (_, tip) = split_tip("A.\n\n**TRY THIS**: `MIDI In` → it.");
        assert_eq!(tip.as_deref(), Some("`MIDI In` → it."));
        // Only a paragraph that starts with the label is a tip.
        let (rest, tip) = split_tip("Never try this at home.\n\nTry thistle tea.");
        assert_eq!(tip, None);
        assert_eq!(rest, "Never try this at home.\n\nTry thistle tea.");
        assert_eq!(split_tip("No tip.").1, None);
        assert_eq!(split_tip("A.\n\n**Try this:**").1, None, "an empty tip");
    }

    #[test]
    fn search_uses_synonyms() {
        let r = reg();
        assert_eq!(r.search("volume")[0].meta.kind, "test_gain");
        assert_eq!(r.search("lpf")[0].meta.kind, "test_biquad");
        assert_eq!(r.search("bqd")[0].meta.kind, "test_biquad");
        assert!(r.search("zzz").is_empty());
        assert_eq!(r.search("").len(), 2);
    }

    #[test]
    fn unknown_kinds_have_no_signature() {
        let r = reg();
        let n = NodeInstance::new(crate::NodeId(1), "nope");
        assert!(r.signature_of(&n, &BTreeMap::new()).is_none());
    }
}
