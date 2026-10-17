//! Parameter values and node configuration.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The value of an unwired input, as stored in a project file.
///
/// Untagged so files read naturally: `440.0`, `3`, `true`, `"saw"`. Integers
/// and floats stay distinct so a load → save round trip is byte-identical.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    /// Numeric view: floats as-is, ints converted, bools as 0/1. Text is `None`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Text(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Int(i) => Some(*i != 0),
            Value::Float(f) => Some(*f > 0.5),
            Value::Text(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Text(s) => write!(f, "{s:?}"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Text(s) => f.write_str(s),
            other => write!(f, "{other:?}"),
        }
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(f64::from(v))
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(i64::from(v))
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

/// Node configuration: settings that change a node's socket list (a mixer's
/// input count, a channel selection, a file path). A JSON object.
pub type Config = serde_json::Value;

/// Typed reads from a [`Config`] with defaults. Never panics on a malformed
/// or missing key; the default is returned instead.
pub trait ConfigExt {
    fn get_u64_or(&self, key: &str, default: u64) -> u64;
    fn get_f64_or(&self, key: &str, default: f64) -> f64;
    fn get_bool_or(&self, key: &str, default: bool) -> bool;
    fn get_str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str;
}

impl ConfigExt for Config {
    fn get_u64_or(&self, key: &str, default: u64) -> u64 {
        match self.get(key) {
            Some(v) => v
                .as_u64()
                .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
                .unwrap_or(default),
            None => default,
        }
    }

    fn get_f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    fn get_bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn get_str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).and_then(|v| v.as_str()).unwrap_or(default)
    }
}

/// An empty configuration object.
pub fn empty_config() -> Config {
    serde_json::Value::Object(serde_json::Map::new())
}

pub(crate) fn config_is_empty(c: &Config) -> bool {
    match c {
        serde_json::Value::Null => true,
        serde_json::Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_keeps_int_float_distinction() {
        let v: Vec<Value> = serde_json::from_str(r#"[1, 1.0, true, "saw"]"#).unwrap();
        assert_eq!(
            v,
            vec![
                Value::Int(1),
                Value::Float(1.0),
                Value::Bool(true),
                Value::Text("saw".into())
            ]
        );
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"[1,1.0,true,"saw"]"#);
    }

    #[test]
    fn config_reads_with_defaults() {
        let c: Config = serde_json::json!({"inputs": 4, "name": "x", "bad": "str"});
        assert_eq!(c.get_u64_or("inputs", 2), 4);
        assert_eq!(c.get_u64_or("missing", 2), 2);
        assert_eq!(c.get_u64_or("bad", 2), 2);
        assert_eq!(c.get_str_or("name", "y"), "x");
        let null = Config::Null;
        assert_eq!(null.get_u64_or("inputs", 3), 3);
    }
}
