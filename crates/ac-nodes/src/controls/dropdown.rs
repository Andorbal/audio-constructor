use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

node! {
    kind: "dropdown",
    name: "Dropdown",
    category: Controls,
    keywords: ["menu", "choice", "select", "list", "options", "mode", "picker", "combo"],
    /// A menu of named choices; outputs the index of the one picked.
    ///
    /// The `choices` config is the list of labels the menu shows (up to 64;
    /// "A", "B", "C" by default). Picking one sets `value`, and `out` is
    /// its index, from 0 for the first label. Labels that are numbers or
    /// true/false are shown as text, other entries are skipped, extra
    /// labels past 64 are dropped, and an empty or missing list falls back
    /// to the default.
    ///
    /// `changed` fires at the start of the block in which the choice
    /// changes (not when the project loads). `value` can also be wired from
    /// any `Int`; it is kept within the list, so `out` is always a valid
    /// index.
    ///
    /// **Try this:** list "sine", "triangle", "saw", "square" and wire `out`
    /// into an `Oscillator`'s `shape` for a labeled waveform menu on a
    /// panel, or wire `changed` into an `ADSR` to hear each switch.
    pub struct Dropdown {
        len: i32,
        /// `out` in the last block; `None` before the first.
        last: Option<i32>,
    }
}

const VALUE: usize = 0;

const OUT: usize = 0;
const CHANGED: usize = 1;

/// Most labels a dropdown holds.
pub const MAX_CHOICES: usize = 64;
const DEFAULT_CHOICES: [&str; 3] = ["A", "B", "C"];

fn label(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The labels a dropdown shows, from its `choices` config, repaired as its
/// docs say: numbers and booleans become text, other entries are skipped,
/// at most [`MAX_CHOICES`] are kept, and an empty list becomes the default.
/// `out` indexes this list.
pub fn choices(config: &Config) -> Vec<String> {
    let list: Vec<String> = config
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(label).take(MAX_CHOICES).collect())
        .unwrap_or_default();
    if list.is_empty() {
        DEFAULT_CHOICES.iter().map(|s| s.to_string()).collect()
    } else {
        list
    }
}

fn count(config: &Config) -> i32 {
    choices(config).len().clamp(1, MAX_CHOICES) as i32
}

impl Node for Dropdown {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let last = f64::from(count(config) - 1);
        Signature::new()
            .input(
                SocketSpec::int("value", 0)
                    .range(0.0, last)
                    .doc("Index of the chosen label."),
            )
            .output(SocketSpec::int("out", 0).doc("Index of the chosen label, from 0."))
            .output(SocketSpec::trigger("changed").doc("Fires when the choice changes."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Dropdown {
            len: count(config),
            last: None,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let v = io.inputs.int(VALUE).clamp(0, self.len - 1);
        io.outputs.set_int(OUT, v);
        if self.last.is_some_and(|l| l != v) {
            io.outputs.trigger(CHANGED).push(0);
        }
        self.last = Some(v);
    }

    /// The choice is the editor's, not the song's: a transport restart
    /// keeps it, and keeps what `changed` compares against.
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn outputs_the_index_and_fires_changed() {
        let mut h =
            Harness::new::<Dropdown>(json!({"choices": ["sine", "triangle", "saw", "square"]}));
        let out = h.run(128);
        assert!(out.int("out").iter().all(|&v| v == 0));
        assert!(out.triggers("changed").is_empty());
        h.set("value", 2);
        let out = h.run(128);
        assert!(out.int("out").iter().all(|&v| v == 2));
        assert_eq!(out.triggers("changed"), &[128]);
        h.set("value", 2);
        assert!(h.run(64).triggers("changed").is_empty());
    }

    #[test]
    fn value_is_kept_within_the_list() {
        let mut h = Harness::new::<Dropdown>(json!({}));
        h.set("value", 7);
        assert_eq!(h.run(64).int("out")[0], 2);
        h.set_int("value", i32::MIN);
        let out = h.run(64);
        assert_eq!(out.int("out")[0], 0);
        assert_eq!(out.triggers("changed"), &[64]);
    }

    #[test]
    fn choices_are_repaired() {
        assert_eq!(choices(&json!({})), ["A", "B", "C"]);
        assert_eq!(choices(&json!({"choices": []})), ["A", "B", "C"]);
        assert_eq!(choices(&json!({"choices": "x"})), ["A", "B", "C"]);
        assert_eq!(
            choices(&json!({"choices": ["low", 2, true, null, {"a": 1}, "high"]})),
            ["low", "2", "true", "high"]
        );
        let many: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
        let c = choices(&json!({ "choices": many }));
        assert_eq!(c.len(), MAX_CHOICES);
        assert_eq!(c[63], "c63");
        let sig = Dropdown::describe(&json!({ "choices": many }));
        assert_eq!(sig.inputs[VALUE].range, Some((0.0, 63.0)));
        let sig = Dropdown::describe(&json!({"choices": ["only"]}));
        assert_eq!(sig.inputs[VALUE].range, Some((0.0, 0.0)));
    }

    #[test]
    fn reset_keeps_the_choice() {
        let mut h = Harness::new::<Dropdown>(json!({}));
        h.set("value", 1);
        h.run(64);
        h.reset();
        let out = h.run(64);
        assert_eq!(out.int("out")[0], 1);
        assert!(out.triggers("changed").is_empty());
    }
}
