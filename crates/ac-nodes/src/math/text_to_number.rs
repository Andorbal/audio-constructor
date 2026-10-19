use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};

node! {
    kind: "text_to_number",
    name: "Text to Number",
    category: Math,
    keywords: ["parse", "string", "extract", "read", "pattern", "address"],
    /// Reads a number out of text: with the pattern `/fader/{}`, the text
    /// `/fader/3` gives 3.
    ///
    /// `pattern` is the text to expect, with one `{}` where the number is;
    /// the rest must match exactly, and whatever sits in the placeholder's
    /// place (spaces around it ignored) is read as a number. An empty
    /// pattern reads the whole text. `value` holds the number and `ok` is
    /// on while the text matches; when it doesn't (wrong text, nothing
    /// numeric in the placeholder, no `{}` in a non-empty pattern), `value`
    /// is 0 and `ok` is off, and nothing stops.
    ///
    /// The `type` setting picks the output: `float` (the default) or
    /// `int`, which rounds. Both change once per block.
    ///
    /// **Try this:** wire a group's text input through this node to turn
    /// a channel name such as `ch3` into the `index` of a `Select`, with
    /// the pattern `ch{}` and type `int`.
    pub struct TextToNumber {
        int: bool,
    }
}

/// The number in `text` where `pattern` has its `{}`, if the rest of the
/// pattern matches.
pub(crate) fn parse_with(text: &str, pattern: &str) -> Option<f64> {
    let (prefix, suffix) = match pattern.find("{}") {
        None if pattern.is_empty() => ("", ""),
        None => return None,
        Some(i) => (&pattern[..i], &pattern[i + 2..]),
    };
    if suffix.contains("{}") {
        return None;
    }
    let inner = text.strip_prefix(prefix)?.strip_suffix(suffix)?.trim();
    let v: f64 = inner.parse().ok()?;
    v.is_finite().then_some(v)
}

fn is_int(config: &Config) -> bool {
    config.get_str_or("type", "float") == "int"
}

const TEXT: usize = 0;
const PATTERN: usize = 1;
const VALUE: usize = 0;
const OK: usize = 1;

impl Node for TextToNumber {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let value = if is_int(config) {
            SocketSpec::int("value", 0)
        } else {
            SocketSpec::float("value", Unit::Generic, 0.0).rate(Rate::Block)
        };
        Signature::new()
            .input(SocketSpec::text("text", "").doc("The text to read."))
            .input(
                SocketSpec::text("pattern", "{}").doc("The expected text, with {} at the number."),
            )
            .output(value.doc("The number read, or 0."))
            .output(
                SocketSpec::boolean("ok", false)
                    .rate(Rate::Block)
                    .doc("On while the text matches the pattern."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TextToNumber {
            int: is_int(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let text = io.inputs.text(TEXT);
        let pattern = io.inputs.text(PATTERN);
        let parsed = parse_with(text.as_str(), pattern.as_str());
        let v = parsed.unwrap_or(0.0);
        if self.int {
            let i = v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            io.outputs.set_int(VALUE, i);
        } else {
            io.outputs.set_float(VALUE, v as f32);
        }
        io.outputs.set_bool(OK, parsed.is_some());
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn reads_the_number_at_the_placeholder() {
        assert_eq!(parse_with("/fader/3", "/fader/{}"), Some(3.0));
        assert_eq!(parse_with("/fader/ -2.5 ", "/fader/{}"), Some(-2.5));
        assert_eq!(parse_with("ch3/vol", "ch{}/vol"), Some(3.0));
        assert_eq!(parse_with("42", ""), Some(42.0));
        assert_eq!(parse_with("1e3", "{}"), Some(1000.0));
        assert_eq!(parse_with("/fader/3", "/knob/{}"), None);
        assert_eq!(parse_with("/fader/x", "/fader/{}"), None);
        assert_eq!(parse_with("/fader/", "/fader/{}"), None);
        assert_eq!(parse_with("/fader/3", "/fader/3"), None, "no placeholder");
        assert_eq!(parse_with("1/2", "{}/{}"), None, "two placeholders");
        assert_eq!(parse_with("inf", "{}"), None);
        assert_eq!(parse_with("", ""), None);
    }

    #[test]
    fn outputs_follow_the_text_block_by_block() {
        let mut h = Harness::new::<TextToNumber>(json!({}));
        h.set("pattern", "/fader/{}");
        h.texts("text", &["/fader/1", "/fader/2.5", "/knob/1"]);
        let out = h.run(192);
        assert_eq!(out.float("value")[0], 1.0);
        assert_eq!(out.float("value")[64], 2.5);
        assert_eq!(out.float("value")[128], 0.0);
        assert_eq!(out.float("ok")[0], 1.0);
        assert_eq!(out.float("ok")[150], 0.0);
    }

    #[test]
    fn int_type_rounds() {
        let mut h = Harness::new::<TextToNumber>(json!({"type": "int"}));
        h.set("text", "ch 2.6").set("pattern", "ch{}");
        let out = h.run(64);
        assert_eq!(out.int("value")[0], 3);
        assert_eq!(out.float("ok")[0], 1.0);
        let mut h = Harness::new::<TextToNumber>(json!({"type": "int"}));
        h.set("text", "1e30");
        assert_eq!(h.run(64).int("value")[0], i32::MAX);
    }
}
