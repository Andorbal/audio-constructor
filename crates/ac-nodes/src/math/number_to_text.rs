use std::fmt::Write;

use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, TextBuf,
    Unit, node,
};

use crate::io::osc_in::value_name;

node! {
    kind: "number_to_text",
    name: "Number to Text",
    category: Math,
    keywords: ["format", "string", "template", "label", "address", "print", "join"],
    /// Writes numbers into a text template: `/fader/{}` with `value` 3
    /// gives `/fader/3`.
    ///
    /// `template` is the text to produce, with `{}` where each number goes.
    /// Placeholders take the values in order (`value`, then `value_2` to
    /// `value_4`, as many as the `args` setting gives, 1–4). A placeholder
    /// can name a value, counting from 0 (`{1}`), and ask for a fixed
    /// number of decimals (`{:.2}`, `{0:.1}`); plain `{}` writes whole
    /// numbers without a decimal point and others with as many digits as
    /// they need. `{{` and `}}` give literal braces. A template with a
    /// stray brace, an unknown format, or a placeholder past the last
    /// value writes nothing for that part; a value that isn't a number
    /// writes 0. The text is cut at 128 bytes.
    ///
    /// `out` changes once per block, from the values at the block's start,
    /// so a moving signal doesn't produce every intermediate number.
    ///
    /// **Try this:** wire a `Counter` into `value` with the template
    /// `/fader/{}` and feed `out` into an `OSC In`'s `address` to step
    /// through a surface's faders, or expose `value` as a group input so
    /// each instance of a block listens to its own control.
    pub struct NumberToText {
        args: usize,
    }
}

/// Most values a template can place.
const MAX_ARGS: u64 = 4;

fn args_of(config: &Config) -> usize {
    config.get_u64_or("args", 1).clamp(1, MAX_ARGS) as usize
}

/// How one placeholder writes its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spec {
    /// `{}`: whole numbers without decimals, others as short as possible.
    Auto,
    /// `{:.N}`: exactly `N` decimals.
    Fixed(usize),
}

impl Spec {
    /// The part between `{` and `}`: an optional value index, then an
    /// optional `:.N`. `None` for anything else.
    fn parse(inner: &str) -> Option<(Option<usize>, Spec)> {
        let digits = inner.len() - inner.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let index = if digits > 0 {
            Some(inner[..digits].parse::<usize>().ok()?)
        } else {
            None
        };
        let rest = &inner[digits..];
        let spec = if rest.is_empty() {
            Spec::Auto
        } else {
            let n = rest.strip_prefix(":.")?;
            if n.is_empty() || n.len() > 2 || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Spec::Fixed(n.parse::<usize>().ok()?.min(9))
        };
        Some((index, spec))
    }

    fn write(self, out: &mut TextBuf, v: f32) {
        let v = if v.is_finite() { v } else { 0.0 };
        // Writing into a `TextBuf` never fails; what doesn't fit is cut.
        let _ = match self {
            Spec::Auto if v.fract() == 0.0 && v.abs() < 1e9 => write!(out, "{}", v as i64),
            Spec::Auto => write!(out, "{v}"),
            Spec::Fixed(n) => write!(out, "{v:.n$}"),
        };
    }
}

/// Write `template` into `out` with `values` in its placeholders. Returns
/// false when the template is malformed; `out` is then empty.
pub(crate) fn format_into(out: &mut TextBuf, template: &str, values: &[f32]) -> bool {
    out.clear();
    let mut next = 0;
    let mut rest = template;
    loop {
        let Some(i) = rest.find(['{', '}']) else {
            out.push_str(rest);
            return true;
        };
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{{") {
            out.push('{');
            rest = after;
            continue;
        }
        if let Some(after) = tail.strip_prefix("}}") {
            out.push('}');
            rest = after;
            continue;
        }
        let (Some(inner), Some(end)) = (tail.strip_prefix('{'), tail.find('}')) else {
            out.clear();
            return false;
        };
        let Some((index, spec)) = Spec::parse(&inner[..end - 1]) else {
            out.clear();
            return false;
        };
        let k = index.unwrap_or(next);
        if let Some(&v) = values.get(k) {
            spec.write(out, v);
        }
        next = k + 1;
        rest = &tail[end + 1..];
    }
}

const TEMPLATE: usize = 0;
const FIRST_VALUE: usize = 1;
const OUT: usize = 0;

impl Node for NumberToText {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new().input(
            SocketSpec::text("template", "{}").doc("The text to write, with {} where numbers go."),
        );
        for k in 0..args_of(config) {
            let doc = if k == 0 {
                "The number for the first placeholder."
            } else {
                "The number for the next placeholder."
            };
            sig = sig.input(
                SocketSpec::float(value_name(k), Unit::Generic, 0.0)
                    .no_smooth()
                    .doc(doc),
            );
        }
        sig.output(SocketSpec::text("out", "").doc("The filled-in text, once per block."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        NumberToText {
            args: args_of(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let mut values = [0.0f32; MAX_ARGS as usize];
        for (k, v) in values.iter_mut().enumerate().take(self.args) {
            *v = io.inputs.float(FIRST_VALUE + k).first();
        }
        let template = *io.inputs.text(TEMPLATE);
        if let Some(out) = io.outputs.text(OUT) {
            format_into(out, template.as_str(), &values[..self.args]);
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn fmt(template: &str, values: &[f32]) -> (bool, String) {
        let mut out = TextBuf::EMPTY;
        let ok = format_into(&mut out, template, values);
        (ok, out.as_str().to_string())
    }

    #[test]
    fn placeholders_take_the_values_in_order() {
        assert_eq!(fmt("/fader/{}", &[3.0]), (true, "/fader/3".into()));
        assert_eq!(fmt("{}/{}", &[1.0, 2.5]), (true, "1/2.5".into()));
        assert_eq!(fmt("{1}-{0}", &[1.0, 2.0]), (true, "2-1".into()));
        assert_eq!(fmt("{1}{}", &[1.0, 2.0, 3.0]), (true, "23".into()));
        assert_eq!(fmt("plain", &[1.0]), (true, "plain".into()));
        assert_eq!(fmt("", &[1.0]), (true, String::new()));
        assert_eq!(fmt("{{{}}}", &[7.0]), (true, "{7}".into()));
    }

    #[test]
    fn decimals_are_fixed_on_request() {
        assert_eq!(fmt("{:.2}", &[0.5]), (true, "0.50".into()));
        assert_eq!(fmt("{0:.1}", &[2.0]), (true, "2.0".into()));
        assert_eq!(fmt("{:.0}", &[2.6]), (true, "3".into()));
        assert_eq!(fmt("{:.12}", &[1.0]).1.len(), 11, "at most 9 decimals");
        assert_eq!(fmt("{}", &[0.1]), (true, "0.1".into()));
        assert_eq!(fmt("{}", &[-4.0]), (true, "-4".into()));
        assert_eq!(fmt("{}", &[1e12]), (true, "1000000000000".into()));
        assert_eq!(fmt("{}", &[f32::NAN]), (true, "0".into()));
        assert_eq!(fmt("{:.1}", &[f32::INFINITY]), (true, "0.0".into()));
    }

    #[test]
    fn bad_templates_and_missing_values_write_nothing() {
        assert_eq!(fmt("{", &[1.0]), (false, String::new()));
        assert_eq!(fmt("a}", &[1.0]), (false, String::new()));
        assert_eq!(fmt("{x}", &[1.0]), (false, String::new()));
        assert_eq!(fmt("{:.}", &[1.0]), (false, String::new()));
        assert_eq!(fmt("{:3}", &[1.0]), (false, String::new()));
        assert_eq!(fmt("{}/{}", &[1.0]), (true, "1/".into()));
        assert_eq!(fmt("{5}", &[1.0]), (true, String::new()));
        let long = "x".repeat(300);
        assert_eq!(fmt(&long, &[]).1.len(), ac_core::TEXT_MAX);
    }

    #[test]
    fn formats_once_per_block_from_the_inputs() {
        let mut h = Harness::new::<NumberToText>(json!({"args": 2}));
        h.set("template", "/{}/fader{:.1}");
        h.set_samples("value", (0..256).map(|i| (i / 64) as f32).collect());
        h.set("value_2", 2.0);
        let out = h.run(256);
        assert_eq!(
            out.text("out"),
            ["/0/fader2.0", "/1/fader2.0", "/2/fader2.0", "/3/fader2.0"]
        );
        // A wired template changes the output too.
        let mut h = Harness::new::<NumberToText>(json!({}));
        h.texts("template", &["a{}", "b{}"]).set("value", 1.0);
        assert_eq!(h.run(128).text("out"), ["a1", "b1"]);
        let names: Vec<_> = NumberToText::describe(&json!({"args": 9}))
            .inputs
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(
            names,
            ["template", "value", "value_2", "value_3", "value_4"]
        );
    }
}
