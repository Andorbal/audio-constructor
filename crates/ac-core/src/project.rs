//! Projects: settings, the root graph, embedded group definitions, and the
//! JSON file format.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::binding::Binding;
use crate::graph::{Graph, NodeInstance};
use crate::group::GroupDef;
use crate::id::{GroupId, NodePath};

/// The current project file format version. Bumping it requires a migration
/// in [`migrate`] and a fixture of the old format in the tests.
pub const SCHEMA_VERSION: u32 = 1;

/// A whole project: what `.acproj` files contain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub graph: Graph,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub group_defs: BTreeMap<GroupId, GroupDef>,
    /// Controls mapped onto inputs and transport actions (UX.md "Bindings").
    /// Left out of the file when empty, so projects without bindings read
    /// and diff as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<Binding>,
    /// Editor view state (pan/zoom), kept so a project reopens where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Fields written by a newer version, preserved on save.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            schema_version: SCHEMA_VERSION,
            name: String::new(),
            settings: Settings::default(),
            graph: Graph::default(),
            group_defs: BTreeMap::new(),
            bindings: Vec::new(),
            view: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// Project-wide settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Nominal sample rate; offline renders use it unless overridden. The
    /// live engine runs at the device's rate.
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Project length in seconds; bounds offline renders.
    #[serde(default = "default_length")]
    pub length: f64,
    /// Seed for every random source in the project.
    #[serde(default)]
    pub seed: u64,
    /// Block size for offline renders. The same project, rate, and block size
    /// always produce the same samples.
    #[serde(default = "default_block")]
    pub block_size: u32,
    /// Tempo changes, in order; the first starts at beat 0.
    #[serde(default = "default_tempo")]
    pub tempo: Vec<TempoSegment>,
    /// Loop region in beats, `[start, end)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_region: Option<[f64; 2]>,
}

fn default_rate() -> u32 {
    48_000
}
fn default_length() -> f64 {
    10.0
}
fn default_block() -> u32 {
    256
}
fn default_tempo() -> Vec<TempoSegment> {
    vec![TempoSegment::default()]
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sample_rate: default_rate(),
            length: default_length(),
            seed: 0,
            block_size: default_block(),
            tempo: default_tempo(),
            loop_region: None,
        }
    }
}

/// A tempo segment starting at a musical position. Beats are quarter notes;
/// a bar holds `time_sig[0] * 4 / time_sig[1]` beats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TempoSegment {
    /// Start position in beats.
    pub beat: f64,
    pub bpm: f64,
    pub time_sig: [u8; 2],
}

impl Default for TempoSegment {
    fn default() -> Self {
        TempoSegment {
            beat: 0.0,
            bpm: 120.0,
            time_sig: [4, 4],
        }
    }
}

/// Canvas pan and zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub center: [f32; 2],
    pub zoom: f32,
}

/// A project file that couldn't be read.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    /// Not valid JSON, or not shaped like a project.
    Parse(String),
    /// Written by a newer version than this build understands.
    TooNew { found: u32, supported: u32 },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Parse(m) => write!(f, "not a valid project file: {m}"),
            LoadError::TooNew { found, supported } => write!(
                f,
                "project uses format version {found}, but this build reads up to {supported}"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

impl Project {
    /// Parse a project from JSON text, migrating older formats.
    pub fn from_json(text: &str) -> Result<Project, LoadError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        let value = migrate(value)?;
        serde_json::from_value(value).map_err(|e| LoadError::Parse(e.to_string()))
    }

    /// Serialize to the canonical file form: two-space indent, stable key
    /// order, trailing newline.
    pub fn to_json(&self) -> String {
        to_canonical_json(self)
    }

    /// The node at an engine path: through each group instance on the way
    /// (by its definition) down to the last id. `None` if any step is
    /// missing or isn't a group instance.
    pub fn node_at(&self, path: &NodePath) -> Option<&NodeInstance> {
        let (last, parents) = path.0.split_last()?;
        let mut graph = &self.graph;
        for id in parents {
            let def = graph.node(*id)?.group_def()?;
            graph = &self.group_defs.get(&def)?.graph;
        }
        graph.node(*last)
    }
}

/// Canonical pretty JSON with a trailing newline.
pub fn to_canonical_json<T: Serialize>(value: &T) -> String {
    // Serializing our own types to a String can only fail on non-string map
    // keys, which none of them have; fall back to an empty object regardless.
    let mut s = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    s.push('\n');
    s
}

/// Upgrade a parsed project document to [`SCHEMA_VERSION`].
pub fn migrate(mut value: serde_json::Value) -> Result<serde_json::Value, LoadError> {
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| LoadError::Parse("missing `schema_version`".into()))?;
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > SCHEMA_VERSION {
        return Err(LoadError::TooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    // Version 1 is the first format; future migrations chain here:
    // if version < 2 { value = v1_to_v2(value); }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("schema_version".into(), SCHEMA_VERSION.into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Endpoint, NodeInstance, Wire};
    use crate::id::NodeId;

    fn sample() -> Project {
        let mut p = Project {
            name: "Test".into(),
            ..Project::default()
        };
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "oscillator")
                .at(10.0, 20.5)
                .with_param("freq", 440.0)
                .with_param("shape", "saw"),
        );
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "audio_out").at(200.0, 20.0));
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        ));
        p
    }

    #[test]
    fn save_load_save_is_identical() {
        let text = sample().to_json();
        let loaded = Project::from_json(&text).unwrap();
        assert_eq!(loaded, sample());
        assert_eq!(loaded.to_json(), text);
    }

    #[test]
    fn unknown_kinds_and_fields_are_preserved() {
        let text = r#"{
  "schema_version": 1,
  "name": "future",
  "graph": {
    "nodes": [
      {
        "id": "1",
        "kind": "quantum_flux",
        "config": {
          "zeta": [1, 2, 3]
        },
        "params": {
          "warp": 9.5
        },
        "ui": {
          "pos": [0.0, 0.0]
        },
        "hologram": true
      }
    ],
    "wires": []
  },
  "plugins": {
    "x": 1
  }
}
"#;
        let p = Project::from_json(text).unwrap();
        assert_eq!(p.graph.nodes[0].kind, "quantum_flux");
        assert_eq!(p.graph.nodes[0].extra.get("hologram"), Some(&true.into()));
        let again = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(again, p);
        assert!(p.to_json().contains("\"hologram\": true"));
        assert!(p.to_json().contains("\"plugins\""));
    }

    #[test]
    fn newer_versions_are_rejected_with_a_message() {
        let err = Project::from_json(r#"{"schema_version": 99}"#).unwrap_err();
        assert!(matches!(err, LoadError::TooNew { found: 99, .. }));
        assert!(err.to_string().contains("99"));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(Project::from_json("not json").is_err());
        assert!(Project::from_json("{}").is_err());
        assert!(Project::from_json(r#"{"schema_version": 1, "graph": 5}"#).is_err());
        assert!(
            Project::from_json(
                r#"{"schema_version": 1, "graph": {"nodes": [], "wires": [{"from": "x", "to": "1:in"}]}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn minimal_file_gets_defaults() {
        let p = Project::from_json(r#"{"schema_version": 1}"#).unwrap();
        assert_eq!(p.settings.sample_rate, 48_000);
        assert_eq!(p.settings.tempo[0].bpm, 120.0);
        assert!(p.bindings.is_empty());
    }

    #[test]
    fn bindings_round_trip_and_stay_out_of_files_without_them() {
        use crate::binding::{Binding, GateMode, Source, TransportAction};
        use crate::control::GamepadButton;
        // A project without bindings writes exactly what it did before.
        let text = sample().to_json();
        assert!(!text.contains("bindings"));
        assert_eq!(Project::from_json(&text).unwrap().to_json(), text);

        let mut p = sample();
        p.bindings.push(Binding {
            min: Some(110.0),
            max: Some(880.0),
            ..Binding::input(
                Source::MidiCc {
                    channel: Some(2),
                    cc: 74,
                },
                NodePath::root(NodeId(1)),
                "freq",
            )
        });
        p.bindings.push(Binding {
            mode: GateMode::Toggle,
            ..Binding::transport(
                Source::GamepadButton {
                    pad: None,
                    button: GamepadButton::West,
                },
                TransportAction::Record,
            )
        });
        let text = p.to_json();
        let back = Project::from_json(&text).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.to_json(), text);
        assert!(text.contains("\"bindings\""));
        // A binding that doesn't parse is a load error, like a bad wire.
        let bad = text.replace("\"midi_cc\"", "\"midi_banjo\"");
        assert!(Project::from_json(&bad).is_err());
    }

    #[test]
    fn node_paths_resolve_through_group_instances() {
        use crate::graph::GROUP_KIND;
        let mut p = sample();
        let mut def = GroupDef::new("Inner");
        def.graph.nodes.push(NodeInstance::new(NodeId(7), "gain"));
        p.group_defs.insert(GroupId::from("g"), def);
        p.graph.nodes.push(
            NodeInstance::new(NodeId(9), GROUP_KIND).with_config(serde_json::json!({"def": "g"})),
        );
        let at = |ids: &[u64]| {
            p.node_at(&NodePath(ids.iter().map(|&i| NodeId(i)).collect()))
                .map(|n| n.kind.clone())
        };
        assert_eq!(at(&[1]).as_deref(), Some("oscillator"));
        assert_eq!(at(&[9, 7]).as_deref(), Some("gain"));
        assert_eq!(at(&[9, 8]), None);
        assert_eq!(at(&[1, 7]), None, "not a group instance");
        assert_eq!(at(&[]), None);
    }
}
