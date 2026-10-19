//! Library indexes (ARCHITECTURE §12, DECISIONS D191): an `index.json` that
//! describes a folder of blocks, so the folder can be shared from any
//! static web host and browsed and installed from a link.
//!
//! ```json
//! {
//!   "format": "audio-constructor/library",
//!   "version": 1,
//!   "name": "Tape and Space",
//!   "description": "Delays and reverbs.",
//!   "blocks": [
//!     {
//!       "id": "warm-echo",
//!       "name": "Warm Echo",
//!       "version": 3,
//!       "description": "A tape echo that darkens as it repeats.",
//!       "tags": ["delay", "space"],
//!       "author": "Sam",
//!       "schema_version": 2,
//!       "file": "warm-echo.acblock",
//!       "sha256": "…64 hex digits…",
//!       "size": 18211,
//!       "preview": { "file": "warm-echo.preview.wav", "sha256": "…", "size": 480044 }
//!     }
//!   ]
//! }
//! ```
//!
//! `file` links are relative to the index (percent-encoded where a file
//! name needs it), so the folder works wherever it's put. A reader skips
//! entries it can't use and says why; an index from a newer `version` is
//! refused rather than half-read.

use std::path::{Path, PathBuf};

use ac_core::{
    BLOCK_EXTENSION, Block, GROUP_KIND, GroupId, NoResources, NodeId, NodeInstance, Project,
    Registry, VOICES_KIND, preview_file_name,
};
use ac_engine::{BakeStore, CompileOptions, compile};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hash::{is_sha256, sha256_hex};
use crate::wav::read_wav_bytes;

/// The index's `format`.
pub const INDEX_FORMAT: &str = "audio-constructor/library";
/// The newest index `version` this reads (and the one it writes).
pub const INDEX_VERSION: u32 = 1;
/// What an index is called in its folder.
pub const INDEX_FILE: &str = "index.json";

/// A shared library: its name and its blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryIndex {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub blocks: Vec<IndexBlock>,
}

/// One block in an index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexBlock {
    /// Its id: the block file's name without `.acblock`.
    pub id: String,
    pub name: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// The project schema the block file is written in.
    #[serde(default = "one")]
    pub schema_version: u32,
    /// The block file, relative to the index.
    pub file: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<IndexFile>,
}

fn one() -> u32 {
    1
}

/// A file an index links to, with what it must hash to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFile {
    /// Relative to the index.
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

impl LibraryIndex {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> LibraryIndex {
        LibraryIndex {
            format: INDEX_FORMAT.to_string(),
            version: INDEX_VERSION,
            name: name.into(),
            description: description.into(),
            blocks: Vec::new(),
        }
    }

    /// The index file's text: canonical JSON, so a rebuilt index diffs
    /// cleanly.
    pub fn to_json(&self) -> String {
        ac_core::project::to_canonical_json(self)
    }

    /// Read an index. Entries that can't be used are left out, each with
    /// the reason in the second value; anything that isn't an index of a
    /// version this reads is an error.
    pub fn parse(text: &str) -> Result<(LibraryIndex, Vec<String>), String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| format!("not a library index (it isn't JSON: {e})"))?;
        if value.get("format").and_then(Value::as_str) != Some(INDEX_FORMAT) {
            return Err(format!(
                "not a library index (its \"format\" isn't \"{INDEX_FORMAT}\")"
            ));
        }
        let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version == 0 {
            return Err("the library index has no \"version\"".into());
        }
        if version > u64::from(INDEX_VERSION) {
            return Err(format!(
                "the library index is version {version}, from a newer Audio Constructor; \
                 update to read it"
            ));
        }
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("Unnamed library")
            .to_string();
        let description = value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut index = LibraryIndex::new(name, description);
        let mut skipped = Vec::new();
        let entries = match value.get("blocks") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(a)) => a.as_slice(),
            Some(_) => return Err("the library index's \"blocks\" isn't a list".into()),
        };
        for (i, entry) in entries.iter().enumerate() {
            let label = entry
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("entry {}", i + 1), |id| format!("\"{id}\""));
            let parsed: IndexBlock = match serde_json::from_value(entry.clone()) {
                Ok(b) => b,
                Err(e) => {
                    skipped.push(format!("{label}: {e}"));
                    continue;
                }
            };
            if let Err(why) = check_entry(&parsed) {
                skipped.push(format!("{label}: {why}"));
                continue;
            }
            if index.blocks.iter().any(|b| b.id == parsed.id) {
                skipped.push(format!("{label}: listed twice; the first is kept"));
                continue;
            }
            index.blocks.push(parsed);
        }
        Ok((index, skipped))
    }

    pub fn get(&self, id: &str) -> Option<&IndexBlock> {
        self.blocks.iter().find(|b| b.id == id)
    }
}

/// Why an entry can't be used, if it can't.
fn check_entry(b: &IndexBlock) -> Result<(), String> {
    if b.id.trim().is_empty() || b.id.contains(['/', '\\']) {
        return Err("its id isn't a file name".into());
    }
    if b.file.trim().is_empty() {
        return Err("it names no file".into());
    }
    if !is_sha256(&b.sha256) {
        return Err("its sha256 isn't 64 hex digits".into());
    }
    if let Some(p) = &b.preview
        && (p.file.trim().is_empty() || !is_sha256(&p.sha256))
    {
        return Err("its preview needs a file and a sha256 of 64 hex digits".into());
    }
    Ok(())
}

/// What building an index found.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexBuild {
    pub index: LibraryIndex,
    /// Blocks left out, with why: the index never lists a block that
    /// doesn't load.
    pub problems: Vec<String>,
    /// Listed, but worth knowing (a preview missing or out of date).
    pub warnings: Vec<String>,
}

/// Index the `.acblock` files directly in `dir` and their previews. The
/// name and description default to the folder's existing index (or the
/// folder's name). Every block is read and compiled with `registry`;
/// blocks that fail are left out and listed in [`IndexBuild::problems`].
pub fn build_index(
    dir: &Path,
    registry: &Registry,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<IndexBuild, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == BLOCK_EXTENSION))
        .collect();
    files.sort();
    let old = std::fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|t| LibraryIndex::parse(&t).ok())
        .map(|(i, _)| i);
    let folder = dir
        .canonicalize()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "Blocks".to_string());
    let name = name
        .map(str::to_string)
        .or_else(|| old.as_ref().map(|o| o.name.clone()))
        .unwrap_or(folder);
    let description = description
        .map(str::to_string)
        .or_else(|| old.as_ref().map(|o| o.description.clone()))
        .unwrap_or_default();
    let mut build = IndexBuild {
        index: LibraryIndex::new(name, description),
        problems: Vec::new(),
        warnings: Vec::new(),
    };
    for path in files {
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        match index_entry(dir, &path, registry, &mut build.warnings) {
            Ok(entry) => build.index.blocks.push(entry),
            Err(why) => build.problems.push(format!("{file}: {why}")),
        }
    }
    Ok(build)
}

fn index_entry(
    dir: &Path,
    path: &Path,
    registry: &Registry,
    warnings: &mut Vec<String>,
) -> Result<IndexBlock, String> {
    let file = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or("its name isn't valid text")?
        .to_string();
    let id = path
        .file_stem()
        .and_then(|f| f.to_str())
        .ok_or("its name isn't valid text")?
        .to_string();
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "it isn't text".to_string())?;
    let block = Block::from_json(text).map_err(|e| e.to_string())?;
    check_compiles(&block, registry)?;
    let schema_version = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("schema_version").and_then(Value::as_u64))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(1);
    // The preview the block records, or one beside it under the usual name.
    let preview_name = block
        .preview
        .as_ref()
        .map(|p| p.file.clone())
        .unwrap_or_else(|| preview_file_name(&id));
    let preview_path = dir.join(&preview_name);
    let preview = match std::fs::read(&preview_path) {
        Ok(wav) => {
            let sha256 = sha256_hex(&wav);
            if read_wav_bytes(&wav).is_err() {
                warnings.push(format!(
                    "{file}: its preview {preview_name} isn't a readable WAV file; listed without it"
                ));
                None
            } else {
                match &block.preview {
                    Some(rec) if !rec.sha256.eq_ignore_ascii_case(&sha256) => {
                        warnings.push(format!(
                            "{file}: {preview_name} isn't the preview the block was exported with; \
                             listing the file as it is (export the block again to match)"
                        ));
                    }
                    None => warnings.push(format!(
                        "{file}: the block doesn't record {preview_name}; listing it anyway"
                    )),
                    _ => {}
                }
                Some(IndexFile {
                    file: url_escape(&preview_name),
                    sha256,
                    size: wav.len() as u64,
                })
            }
        }
        Err(_) if block.preview.is_some() => {
            warnings.push(format!(
                "{file}: its preview {preview_name} is missing; listed without one"
            ));
            None
        }
        Err(_) => None,
    };
    let d = &block.def;
    Ok(IndexBlock {
        id,
        name: d.name.clone(),
        version: d.version,
        description: d.description.clone(),
        tags: d.tags.clone(),
        author: d.author.clone(),
        schema_version,
        file: url_escape(&file),
        sha256: sha256_hex(&bytes),
        size: bytes.len() as u64,
        preview,
    })
}

/// A block "loads" when an instance of it compiles without errors and
/// every node in it is one this version has.
pub fn check_compiles(block: &Block, registry: &Registry) -> Result<(), String> {
    let mut p = Project {
        group_defs: block.uses.clone(),
        ..Project::default()
    };
    let id = GroupId("block".into());
    let id = if p.group_defs.contains_key(&id) {
        GroupId("block-under-test".into())
    } else {
        id
    };
    p.group_defs.insert(id.clone(), block.def.clone());
    let kind = if block.is_polyphonic() {
        VOICES_KIND
    } else {
        GROUP_KIND
    };
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), kind).with_config(serde_json::json!({ "def": id.0 })));
    let compiled = compile(
        &p,
        registry,
        &NoResources,
        &CompileOptions::default(),
        None,
        &BakeStore::default(),
    );
    let mut errors: Vec<String> = compiled
        .report
        .errors()
        .map(|d| d.message.clone())
        .collect();
    errors.extend(
        compiled
            .report
            .diagnostics
            .iter()
            .filter(|d| d.message.starts_with("unknown node kind"))
            .map(|d| d.message.clone()),
    );
    errors.dedup();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Write `index` into `dir` as `index.json` (through a temporary file, so
/// a reader never sees half of one).
pub fn write_index(dir: &Path, index: &LibraryIndex) -> Result<PathBuf, String> {
    let path = dir.join(INDEX_FILE);
    let tmp = dir.join(format!("{INDEX_FILE}.tmp"));
    std::fs::write(&tmp, index.to_json()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// A file name as a relative link: anything but letters, digits, `-`, `.`,
/// `_`, and `~` percent-encoded (RFC 3986's unreserved characters).
pub fn url_escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Undo percent-encoding (for naming things after a link); bad escapes
/// stay as they are.
pub fn url_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// `link` (from an index at `base`) as an absolute link, the way a browser
/// resolves a relative one (RFC 3986 §5.2, without the corner cases no
/// index needs): a full link stays, `/x` is from the server's root, and
/// anything else is beside the index, with `.` and `..` resolved.
pub fn resolve_url(base: &str, link: &str) -> Result<String, String> {
    let link = link.trim();
    if link.contains("://") {
        return Ok(link.to_string());
    }
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| format!("{base} isn't a full link"))?;
    if let Some(rest) = link.strip_prefix("//") {
        return Ok(format!("{scheme}://{rest}"));
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    // The base's path, without its query or fragment.
    let path = path.split(['?', '#']).next().unwrap_or("");
    let joined = if link.starts_with('/') {
        link.to_string()
    } else {
        let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
        format!("{dir}{link}")
    };
    let (joined_path, tail) = match joined.find(['?', '#']) {
        Some(i) => joined.split_at(i),
        None => (joined.as_str(), ""),
    };
    let mut segments: Vec<&str> = Vec::new();
    let parts: Vec<&str> = joined_path.split('/').skip(1).collect();
    let last = parts.len().saturating_sub(1);
    for (i, seg) in parts.iter().enumerate() {
        match *seg {
            "." => {
                if i == last {
                    segments.push("");
                }
            }
            ".." => {
                segments.pop();
                if i == last {
                    segments.push("");
                }
            }
            s => segments.push(s),
        }
    }
    Ok(format!(
        "{scheme}://{authority}/{}{tail}",
        segments.join("/")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::shipped_blocks;
    use crate::preview::export_block;
    use std::sync::Arc;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-index-{tag}-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn shipped(id: &str) -> Block {
        shipped_blocks()
            .into_iter()
            .find(|b| b.source.id == id)
            .unwrap()
            .block
    }

    #[test]
    fn links_resolve_beside_the_index() {
        let base = "https://example.org/libs/tape/index.json?v=2";
        let r = |l| resolve_url(base, l).unwrap();
        assert_eq!(
            r("echo.acblock"),
            "https://example.org/libs/tape/echo.acblock"
        );
        assert_eq!(
            r("./sub/echo.acblock"),
            "https://example.org/libs/tape/sub/echo.acblock"
        );
        assert_eq!(
            r("../other/x.acblock"),
            "https://example.org/libs/other/x.acblock"
        );
        assert_eq!(r("../../../../x"), "https://example.org/x");
        assert_eq!(r("/root.acblock"), "https://example.org/root.acblock");
        assert_eq!(r("//cdn.example.org/a"), "https://cdn.example.org/a");
        assert_eq!(r("https://elsewhere.org/a"), "https://elsewhere.org/a");
        assert_eq!(
            r("x.acblock?raw=1"),
            "https://example.org/libs/tape/x.acblock?raw=1"
        );
        assert_eq!(
            resolve_url("http://127.0.0.1:8000", "a.acblock").unwrap(),
            "http://127.0.0.1:8000/a.acblock"
        );
        assert!(resolve_url("index.json", "a").is_err());
    }

    #[test]
    fn names_escape_for_links_and_back() {
        assert_eq!(url_escape("warm-echo.acblock"), "warm-echo.acblock");
        assert_eq!(url_escape("My Comp #2.acblock"), "My%20Comp%20%232.acblock");
        assert_eq!(url_unescape("My%20Comp%20%232"), "My Comp #2");
        assert_eq!(url_unescape("100%"), "100%");
        assert_eq!(url_unescape("%zz"), "%zz");
    }

    #[test]
    fn an_index_lists_good_blocks_and_their_previews_only() {
        let dir = temp("build");
        let registry = Arc::new(ac_nodes::registry());
        export_block(
            shipped("tremolo"),
            &dir.join("tremolo.acblock"),
            Some(registry.clone()),
        )
        .unwrap();
        export_block(shipped("compressor"), &dir.join("compressor.acblock"), None).unwrap();
        std::fs::write(dir.join("broken.acblock"), "{ not json").unwrap();
        let mut odd = shipped("tremolo");
        odd.def.name = "Odd".into();
        odd.def.graph.nodes[0].kind = "no_such_node".into();
        std::fs::write(dir.join("odd.acblock"), odd.to_json()).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a block").unwrap();

        let b = build_index(&dir, &registry, Some("Test Library"), None).unwrap();
        let ids: Vec<&str> = b.index.blocks.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["compressor", "tremolo"]);
        assert_eq!(b.problems.len(), 2, "{:?}", b.problems);
        assert!(b.problems[0].starts_with("broken.acblock"));
        assert!(b.problems[1].contains("no_such_node"), "{:?}", b.problems);
        let trem = b.index.get("tremolo").unwrap();
        let bytes = std::fs::read(dir.join("tremolo.acblock")).unwrap();
        assert_eq!(trem.sha256, sha256_hex(&bytes));
        assert_eq!(trem.size, bytes.len() as u64);
        assert_eq!(trem.name, "Tremolo");
        assert_eq!(trem.schema_version, ac_core::SCHEMA_VERSION);
        let p = trem.preview.as_ref().unwrap();
        assert_eq!(p.file, "tremolo.preview.wav");
        let wav = std::fs::read(dir.join("tremolo.preview.wav")).unwrap();
        assert_eq!(p.sha256, sha256_hex(&wav));
        assert!(b.index.get("compressor").unwrap().preview.is_none());
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);

        // Written, it reads back the same; the name is kept next time.
        write_index(&dir, &b.index).unwrap();
        let text = std::fs::read_to_string(dir.join(INDEX_FILE)).unwrap();
        let (back, skipped) = LibraryIndex::parse(&text).unwrap();
        assert_eq!(back, b.index);
        assert!(skipped.is_empty());
        let again = build_index(&dir, &registry, None, None).unwrap();
        assert_eq!(again.index.name, "Test Library");

        // A preview edited after export is listed as it is, with a warning.
        std::fs::write(dir.join("tremolo.preview.wav"), &wav[..wav.len() - 2]).unwrap();
        let changed = build_index(&dir, &registry, None, None).unwrap();
        assert_eq!(changed.warnings.len(), 1, "{:?}", changed.warnings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_indexes_are_reports_not_panics() {
        assert!(LibraryIndex::parse("nope").is_err());
        assert!(LibraryIndex::parse("{\"format\": \"something-else\"}").is_err());
        assert!(
            LibraryIndex::parse(&format!(
                "{{\"format\": \"{INDEX_FORMAT}\", \"version\": 9}}"
            ))
            .unwrap_err()
            .contains("newer")
        );
        let good = |id: &str| {
            serde_json::json!({
                "id": id, "name": "X", "version": 1, "file": format!("{id}.acblock"),
                "sha256": "a".repeat(64), "size": 10
            })
        };
        let text = serde_json::json!({
            "format": INDEX_FORMAT, "version": 1, "name": " ",
            "blocks": [
                good("a"),
                good("a"),
                {"id": "b", "name": "B"},
                {"id": "c", "name": "C", "version": 1, "file": "c.acblock", "sha256": "xyz", "size": 1},
                {"id": "../d", "name": "D", "version": 1, "file": "d", "sha256": "a".repeat(64), "size": 1},
                good("e"),
                7
            ]
        })
        .to_string();
        let (index, skipped) = LibraryIndex::parse(&text).unwrap();
        assert_eq!(index.name, "Unnamed library");
        let ids: Vec<&str> = index.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["a", "e"]);
        assert_eq!(skipped.len(), 5, "{skipped:?}");
    }
}
