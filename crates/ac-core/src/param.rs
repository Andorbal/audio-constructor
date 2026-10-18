//! Resolving stored parameter values into the numbers the engine uses.

use crate::signal::{SignalType, SocketSpec};
use crate::text::TextBuf;
use crate::value::Value;

/// The number a parameter slot holds for an unwired input: the stored param
/// if present and sensible, else the socket's default. Enumerated inputs
/// (including generator start/stop modes) resolve choice names to indices.
pub fn param_value(spec: &SocketSpec, stored: Option<&Value>) -> f32 {
    stored
        .and_then(|v| resolve(spec, v))
        .or_else(|| resolve(spec, &spec.default))
        .unwrap_or(0.0)
}

fn resolve(spec: &SocketSpec, v: &Value) -> Option<f32> {
    if !spec.choices.is_empty() {
        let n = spec.choices.len();
        return match v {
            Value::Text(s) => spec.choice_index(s).map(|i| i as f32),
            other => other
                .as_f64()
                .filter(|f| f.is_finite())
                .map(|f| (f.round().max(0.0) as usize).min(n - 1) as f32),
        };
    }
    match spec.ty {
        // Text has no number; its value goes through `param_text`.
        SignalType::Text => None,
        SignalType::Trigger | SignalType::Midi | SignalType::Audio => match v {
            Value::Text(_) => None,
            other => other.as_f64().map(|f| f as f32),
        },
        SignalType::Bool => v.as_bool().map(|b| if b { 1.0 } else { 0.0 }),
        SignalType::Int => v
            .as_f64()
            .filter(|f| f.is_finite())
            .map(|f| f.round() as f32),
        _ => v.as_f64().filter(|f| f.is_finite()).map(|f| f as f32),
    }
}

/// The text an unwired `Text` input holds: the stored param if it's text,
/// else the socket's default (cut to [`crate::TEXT_MAX`] bytes). A number
/// stored by mistake reads as its decimal form, so a hand-edited file
/// still does something visible.
pub fn param_text(spec: &SocketSpec, stored: Option<&Value>) -> TextBuf {
    match stored {
        Some(Value::Text(s)) => TextBuf::new(s),
        Some(other) => TextBuf::new(&other.to_string()),
        None => TextBuf::new(spec.default.as_str().unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::{AutoTrigger, Unit};

    #[test]
    fn text_inputs_read_their_param_or_default() {
        let s = SocketSpec::text("address", "/fader/1");
        assert_eq!(param_text(&s, None), "/fader/1");
        assert_eq!(param_text(&s, Some(&"/xy".into())), "/xy");
        assert_eq!(param_text(&s, Some(&Value::Int(7))), "7");
        assert_eq!(param_value(&s, Some(&"/xy".into())), 0.0);
    }

    #[test]
    fn choices_resolve_by_name_or_index() {
        let s = SocketSpec::choice("shape", &["sine", "saw"], "saw");
        assert_eq!(param_value(&s, None), 1.0);
        assert_eq!(param_value(&s, Some(&"sine".into())), 0.0);
        assert_eq!(param_value(&s, Some(&Value::Int(7))), 1.0);
        assert_eq!(param_value(&s, Some(&"bogus".into())), 1.0);
    }

    #[test]
    fn start_modes_resolve() {
        let s = SocketSpec::trigger("start").auto(AutoTrigger::Start);
        assert_eq!(param_value(&s, None), 0.0);
        assert_eq!(param_value(&s, Some(&"never".into())), 2.0);
    }

    #[test]
    fn numbers_and_bad_values() {
        let s = SocketSpec::float("freq", Unit::Hz, 440.0);
        assert_eq!(param_value(&s, Some(&Value::Int(220))), 220.0);
        assert_eq!(param_value(&s, Some(&"loud".into())), 440.0);
        assert_eq!(param_value(&s, Some(&Value::Float(f64::NAN))), 440.0);
        let b = SocketSpec::boolean("on", true);
        assert_eq!(param_value(&b, None), 1.0);
    }
}
