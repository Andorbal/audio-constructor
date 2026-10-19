//! The plugin state a host saves with its session (DECISIONS D120).
//!
//! It travels inside the framework's own state (which also holds the
//! macro parameters' values, by id) as one persisted field, a JSON
//! document of its own:
//!
//! ```json
//! {
//!   "format": "audio-constructor-plugin-state",
//!   "version": 1,
//!   "kind": "effect",
//!   "project": { "schema_version": 1, "...": "the project, as in an .acproj" },
//!   "assets": [ { "path": "assets/kick.wav", "hash": "fnv1a64:…", "data": "<base64>" } ],
//!   "origin": "/where/the/project/was/loaded/from.acproj"
//! }
//! ```
//!
//! The project is canonical: macro mappings are its bindings. Assets are
//! the referenced audio files' bytes, so the session plays the same
//! anywhere. `origin` is only shown to people. Reading never panics: a
//! document that can't be used is an error the session reports, and a
//! broken asset is a warning.

use serde::{Deserialize, Serialize};

use ac_core::Project;

use crate::assets::AssetBank;
use crate::base64;
use crate::session::Kind;

/// The `format` tag of the document.
pub const STATE_FORMAT: &str = "audio-constructor-plugin-state";
/// The newest state version this build writes and reads. A bump comes with
/// a migration in [`decode`] and a test fixture of the old form.
pub const STATE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct StateDoc {
    format: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<Kind>,
    project: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    assets: Vec<StateAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct StateAsset {
    path: String,
    #[serde(default)]
    hash: String,
    data: String,
}

/// A state read back.
#[derive(Debug)]
pub struct Decoded {
    pub kind: Option<Kind>,
    pub project: Project,
    /// `(config path, file bytes)` of each embedded asset that decoded.
    pub assets: Vec<(String, Vec<u8>)>,
    pub origin: Option<String>,
    /// Problems that didn't stop the project loading (a broken asset).
    pub warnings: Vec<String>,
}

/// Write the state for `project` and its assets.
pub fn encode(kind: Kind, project: &Project, assets: &AssetBank, origin: Option<&str>) -> String {
    let doc = StateDoc {
        format: STATE_FORMAT.into(),
        version: STATE_VERSION,
        kind: Some(kind),
        project: serde_json::to_value(project).unwrap_or(serde_json::Value::Null),
        assets: assets
            .iter()
            .map(|(path, a)| StateAsset {
                path: path.clone(),
                hash: a.hash.clone(),
                data: base64::encode(&a.bytes),
            })
            .collect(),
        origin: origin.map(str::to_string),
    };
    serde_json::to_string(&doc).unwrap_or_default()
}

/// Read a state. Errors say, for people, why it can't be used.
pub fn decode(text: &str) -> Result<Decoded, String> {
    let doc: StateDoc =
        serde_json::from_str(text).map_err(|e| format!("the saved state isn't readable ({e})"))?;
    if doc.format != STATE_FORMAT {
        return Err(format!(
            "the saved state is a {:?} document, not this plugin's",
            doc.format
        ));
    }
    if doc.version > STATE_VERSION {
        return Err(format!(
            "the saved state is version {}, but this build reads up to {STATE_VERSION}: \
             it was saved by a newer version",
            doc.version
        ));
    }
    // Version 1 is the first; migrations chain here:
    // if doc.version < 2 { doc = v1_to_v2(doc); }
    let project = Project::from_json(&doc.project.to_string())
        .map_err(|e| format!("the saved project can't be opened: {e}"))?;
    let mut assets = Vec::new();
    let mut warnings = Vec::new();
    for a in doc.assets {
        match base64::decode(&a.data) {
            Some(bytes) => {
                if !a.hash.is_empty() && ac_core::util::content_hash(&bytes) != a.hash {
                    warnings.push(format!("{} was damaged in the saved state", a.path));
                }
                assets.push((a.path, bytes));
            }
            None => warnings.push(format!(
                "{} couldn't be read from the saved state; it plays as silence",
                a.path
            )),
        }
    }
    Ok(Decoded {
        kind: doc.kind,
        project,
        assets,
        origin: doc.origin,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::wav_bytes;
    use crate::defaults;

    #[test]
    fn round_trips_project_assets_and_origin() {
        let p = defaults::project(Kind::Instrument);
        let mut bank = AssetBank::default();
        let wav = wav_bytes(44_100, &[vec![0.1, 0.2, 0.3]]);
        bank.insert("assets/x.wav", wav.clone()).unwrap();
        let text = encode(Kind::Instrument, &p, &bank, Some("/tmp/x.acproj"));
        let back = decode(&text).unwrap();
        assert_eq!(back.project, p);
        assert_eq!(back.kind, Some(Kind::Instrument));
        assert_eq!(back.assets, vec![("assets/x.wav".to_string(), wav)]);
        assert_eq!(back.origin.as_deref(), Some("/tmp/x.acproj"));
        assert!(back.warnings.is_empty());
    }

    #[test]
    fn version_1_fixture_still_reads() {
        // The first state format, as written by 0.8.0. Keep this fixture
        // when the format changes; it must keep loading.
        let v1 = r#"{"format":"audio-constructor-plugin-state","version":1,"kind":"effect",
            "project":{"schema_version":1,"name":"Old","graph":{"nodes":[
              {"id":"1","kind":"audio_in","config":{"channels":2}},
              {"id":"2","kind":"audio_out"}],
              "wires":[{"from":"1:out","to":"2:in"}]},
              "bindings":[{"source":{"type":"macro","number":2},
                "target":{"type":"input","node":"1","socket":"x"}}]},
            "assets":[{"path":"a.wav","hash":"","data":"UklGRg=="}]}"#;
        let d = decode(v1).unwrap();
        assert_eq!(d.project.name, "Old");
        assert_eq!(d.project.graph.nodes.len(), 2);
        assert_eq!(d.assets, vec![("a.wav".to_string(), b"RIFF".to_vec())]);
    }

    #[test]
    fn bad_states_are_errors_and_bad_assets_warnings() {
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"format":"something-else","version":1,"project":{}}"#,
            r#"{"format":"audio-constructor-plugin-state","version":99,"project":{}}"#,
            r#"{"format":"audio-constructor-plugin-state","version":1,"project":{"schema_version":999}}"#,
            r#"{"format":"audio-constructor-plugin-state","version":1,"project":"nope"}"#,
        ] {
            assert!(decode(bad).is_err(), "{bad}");
        }
        let newer =
            decode(r#"{"format":"audio-constructor-plugin-state","version":2,"project":{}}"#)
                .unwrap_err();
        assert!(newer.contains("newer version"), "{newer}");
        let p = defaults::project(Kind::Effect);
        let mut text = encode(Kind::Effect, &p, &AssetBank::default(), None);
        text = text.replacen(
            "\"project\"",
            "\"assets\":[{\"path\":\"x.wav\",\"hash\":\"fnv1a64:0\",\"data\":\"!!\"},\
             {\"path\":\"y.wav\",\"hash\":\"fnv1a64:0\",\"data\":\"UklGRg==\"}],\"project\"",
            1,
        );
        let d = decode(&text).unwrap();
        assert_eq!(d.project, p);
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
        assert_eq!(d.assets.len(), 1, "the damaged one is still kept");
    }
}
