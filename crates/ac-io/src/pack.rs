//! Project packs (`.acpack`): a project and the audio files it uses in one
//! ZIP archive, for sharing (ARCHITECTURE §12).
//!
//! A pack holds `manifest.json`, the project at its root, and its assets at
//! their paths relative to the project. Assets the project reaches outside
//! its own folder (absolute paths, `..`) are copied in under `assets/` and
//! the packed project points at the copies, so a pack is always
//! self-contained. Unpacking checks every name, so a pack can never write
//! outside the folder it's unpacked into.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use ac_core::{Project, Registry};
use serde_json::json;

use crate::assets::referenced_assets;
use crate::zip::{self, Entry};

pub const PACK_EXTENSION: &str = "acpack";
/// The manifest's `format`.
pub const PACK_FORMAT: &str = "audio-constructor/pack";
const MANIFEST: &str = "manifest.json";

/// What packing did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackReport {
    /// Files in the pack, the project included.
    pub files: usize,
    /// Assets the project references that couldn't be read (left out).
    pub missing: Vec<String>,
    /// Assets copied in from outside the project folder: `(was, now)`.
    pub moved: Vec<(String, String)>,
}

/// Whether `path` is relative and stays inside its base folder.
fn inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// The pack's name for an asset path that stays inside the project folder:
/// forward slashes, no `./`.
fn pack_name(path: &str) -> String {
    Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Build a pack of the project at `project_path`.
pub fn pack(project_path: &Path, registry: &Registry) -> Result<(Vec<u8>, PackReport), String> {
    let mut project = crate::load_project(project_path)?;
    let dir = project_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stem = project_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let project_name = format!("{}.acproj", ac_core::slug(stem));
    let mut report = PackReport::default();
    let mut entries: Vec<Entry> = Vec::new();
    let mut taken: BTreeSet<String> = [MANIFEST.to_string(), project_name.clone()].into();
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    // Assets inside the folder keep their names; copies from outside are
    // named after them, so they can't take an inside asset's name.
    let mut refs: Vec<String> = referenced_assets(&project, registry)
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    refs.sort_by_key(|p| !inside(p));
    refs.dedup();
    for path in refs {
        let full = if Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            dir.join(&path)
        };
        let Ok(data) = std::fs::read(&full) else {
            report.missing.push(path);
            continue;
        };
        let name = if inside(&path) {
            pack_name(&path)
        } else {
            // From outside the folder: copy it in under assets/.
            let file = Path::new(&path)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("asset.wav")
                .to_string();
            let mut candidate = format!("assets/{file}");
            let mut n = 2;
            while taken.contains(&candidate) {
                let (base, ext) = file.rsplit_once('.').unwrap_or((&file, ""));
                candidate = format!("assets/{base}-{n}.{ext}");
                n += 1;
            }
            renames.insert(path.clone(), candidate.clone());
            report.moved.push((path.clone(), candidate.clone()));
            candidate
        };
        if taken.insert(name.clone()) {
            entries.push(Entry { name, data });
        }
    }
    if !renames.is_empty() {
        rewrite_assets(&mut project, registry, &renames);
    }
    let manifest = json!({
        "format": PACK_FORMAT,
        "version": 1,
        "project": project_name,
    });
    entries.insert(
        0,
        Entry {
            name: MANIFEST.into(),
            data: ac_core::project::to_canonical_json(&manifest).into_bytes(),
        },
    );
    entries.insert(
        1,
        Entry {
            name: project_name,
            data: project.to_json().into_bytes(),
        },
    );
    report.files = entries.len() - 1;
    let bytes = zip::write(&entries).map_err(|e| e.to_string())?;
    Ok((bytes, report))
}

/// Point every asset reference in `renames` at its new path.
fn rewrite_assets(project: &mut Project, registry: &Registry, renames: &BTreeMap<String, String>) {
    let fix = |g: &mut ac_core::Graph| {
        for n in &mut g.nodes {
            let Some(entry) = registry.get(&n.kind) else {
                continue;
            };
            for key in entry.meta.asset_keys {
                let new = n
                    .config
                    .get(*key)
                    .and_then(|v| v.as_str())
                    .and_then(|p| renames.get(p))
                    .cloned();
                if let (Some(new), Some(obj)) = (new, n.config.as_object_mut()) {
                    obj.insert((*key).to_string(), new.into());
                }
            }
        }
    };
    fix(&mut project.graph);
    for d in project.group_defs.values_mut() {
        fix(&mut d.graph);
    }
}

/// Pack the project at `project_path` into `out`.
pub fn write_pack(
    project_path: &Path,
    out: &Path,
    registry: &Registry,
) -> Result<PackReport, String> {
    let (bytes, report) = pack(project_path, registry)?;
    std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(report)
}

/// Unpack a pack into `dir` (created if needed). Refuses to overwrite
/// existing files unless `overwrite`. Returns the project's path.
pub fn unpack(bytes: &[u8], dir: &Path, overwrite: bool) -> Result<PathBuf, String> {
    let entries = zip::read(bytes).map_err(|e| format!("not a readable pack: {e}"))?;
    let project_name = entries
        .iter()
        .find(|e| e.name == MANIFEST)
        .and_then(|m| serde_json::from_slice::<serde_json::Value>(&m.data).ok())
        .filter(|m| m["format"] == PACK_FORMAT)
        .and_then(|m| m["project"].as_str().map(str::to_string))
        .or_else(|| {
            entries
                .iter()
                .find(|e| !e.name.contains('/') && e.name.ends_with(".acproj"))
                .map(|e| e.name.clone())
        })
        .ok_or("the pack holds no project")?;
    if !entries.iter().any(|e| e.name == project_name) {
        return Err(format!("the pack's project {project_name} is missing"));
    }
    // Check the project before writing anything.
    let text = entries
        .iter()
        .find(|e| e.name == project_name)
        .map(|e| String::from_utf8_lossy(&e.data).to_string())
        .unwrap_or_default();
    Project::from_json(&text).map_err(|e| format!("{project_name}: {e}"))?;
    if !overwrite {
        let existing: Vec<&str> = entries
            .iter()
            .filter(|e| e.name != MANIFEST && dir.join(&e.name).exists())
            .map(|e| e.name.as_str())
            .collect();
        if !existing.is_empty() {
            return Err(format!(
                "{} already has {}; unpack somewhere else or overwrite",
                dir.display(),
                existing.join(", ")
            ));
        }
    }
    for e in entries.iter().filter(|e| e.name != MANIFEST) {
        let path = dir.join(&e.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|x| format!("{}: {x}", parent.display()))?;
        }
        std::fs::write(&path, &e.data).map_err(|x| format!("{}: {x}", path.display()))?;
    }
    Ok(dir.join(project_name))
}

/// Unpack the pack at `pack_path` into a new folder next to it, named after
/// it (`song.acpack` → `song/`, or `song-2/` if that exists). Returns the
/// project's path.
pub fn unpack_beside(pack_path: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(pack_path).map_err(|e| format!("{}: {e}", pack_path.display()))?;
    let parent = pack_path.parent().unwrap_or(Path::new("."));
    let stem = pack_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("pack");
    let mut dir = parent.join(stem);
    let mut n = 2;
    while dir.exists() {
        dir = parent.join(format!("{stem}-{n}"));
        n += 1;
    }
    unpack(&bytes, &dir, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{AudioData, NodeId, NodeInstance};

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-pack-{tag}-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn wav(path: &Path, v: f32) {
        crate::wav::write_wav(path, &[vec![v; 64]], 48_000, crate::wav::BitDepth::Float32).unwrap();
        let _: Option<AudioData> = None;
    }

    #[test]
    fn packs_round_trip_with_assets_inside_and_outside_the_folder() {
        let reg = ac_nodes::registry();
        let src = temp("src");
        let outside = temp("outside");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        wav(&src.join("assets/kick.wav"), 0.5);
        wav(&outside.join("kick.wav"), 0.25);
        let mut p = Project::default();
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "clip").with_config(json!({"file": "assets/kick.wav"})),
        );
        p.graph.nodes.push(
            NodeInstance::new(NodeId(2), "clip")
                .with_config(json!({"file": outside.join("kick.wav").to_string_lossy()})),
        );
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "clip").with_config(json!({"file": "gone.wav"})));
        let proj = src.join("My Song.acproj");
        crate::save_project(&proj, &p).unwrap();
        let out = src.join("song.acpack");
        let r = write_pack(&proj, &out, &reg).unwrap();
        assert_eq!(r.files, 3, "project and two assets: {r:?}");
        assert_eq!(r.missing, ["gone.wav"]);
        assert_eq!(r.moved.len(), 1);
        assert_eq!(r.moved[0].1, "assets/kick-2.wav");

        let dest = temp("dest");
        let unpacked = unpack(&std::fs::read(&out).unwrap(), &dest, false).unwrap();
        assert_eq!(unpacked, dest.join("my-song.acproj"));
        let q = crate::load_project(&unpacked).unwrap();
        let file = |id| q.graph.node(NodeId(id)).unwrap().config["file"].clone();
        assert_eq!(file(1), json!("assets/kick.wav"));
        assert_eq!(file(2), json!("assets/kick-2.wav"), "points at the copy");
        let a = crate::read_wav(&dest.join("assets/kick-2.wav")).unwrap();
        assert_eq!(a.channels[0][0], 0.25);
        // Unpacking again over the same files is refused, unless asked.
        assert!(unpack(&std::fs::read(&out).unwrap(), &dest, false).is_err());
        assert!(unpack(&std::fs::read(&out).unwrap(), &dest, true).is_ok());
        // Beside the pack, in a fresh folder each time.
        let first = unpack_beside(&out).unwrap();
        let second = unpack_beside(&out).unwrap();
        assert_ne!(first.parent(), second.parent());
        for d in [src, outside, dest] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn broken_packs_are_reported() {
        let dest = temp("broken");
        assert!(unpack(b"nope", &dest, false).is_err());
        let no_project = zip::write(&[Entry {
            name: "readme.txt".into(),
            data: b"hi".to_vec(),
        }])
        .unwrap();
        assert!(
            unpack(&no_project, &dest, false)
                .unwrap_err()
                .contains("no project")
        );
        let bad_project = zip::write(&[Entry {
            name: "x.acproj".into(),
            data: b"{".to_vec(),
        }])
        .unwrap();
        assert!(unpack(&bad_project, &dest, false).is_err());
        assert!(
            std::fs::read_dir(&dest).unwrap().next().is_none(),
            "nothing written"
        );
        let _ = std::fs::remove_dir_all(dest);
    }

    #[test]
    fn inside_means_relative_without_parent_steps() {
        assert!(inside("assets/a.wav"));
        assert!(inside("./a.wav"));
        assert!(!inside("../a.wav"));
        assert!(!inside("/abs/a.wav"));
        assert!(!inside(""));
        assert_eq!(pack_name("./assets/a.wav"), "assets/a.wav");
    }
}
