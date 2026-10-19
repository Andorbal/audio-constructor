//! Files: projects, the recent list, the start project, and audio assets.

use std::path::{Path, PathBuf};

use ac_core::{ConfigExt, Endpoint, NodeId, NodeInstance, Project, Registry, Wire};
use ac_io::AssetStore;

/// Load a project and its assets. Asset problems are in `AssetStore::issues`.
pub fn open_project(path: &Path, registry: &Registry) -> Result<(Project, AssetStore), String> {
    let project = ac_io::load_project(path)?;
    let dir = project_dir(path);
    let assets = AssetStore::load_for(&project, &dir, registry);
    Ok((project, assets))
}

/// The person's block library: `blocks/` in the platform data folder
/// (`None` where the platform has no such folder).
pub fn default_library_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Audio Constructor").map(|d| d.data_dir().join("blocks"))
}

/// The directory a project file's relative paths resolve against.
/// Show `dir` in the system's file manager, making it first if it doesn't
/// exist yet (View ▸ Theme ▸ Open Themes Folder).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_folder(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(opener)
        .arg(dir)
        .spawn()
        .map_err(|e| format!("couldn't run {opener}: {e}"))?;
    // The opener exits as soon as the file manager is up; waiting on it
    // keeps it from lingering as a zombie process.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The browser has no folders to show.
#[cfg(target_arch = "wasm32")]
pub fn open_folder(dir: &Path) -> Result<(), String> {
    Err(format!("{} can't be shown here", dir.display()))
}

pub fn project_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Add the extension if the user left it off.
pub fn with_extension(path: PathBuf, ext: &str) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(".");
        s.push(ext);
        PathBuf::from(s)
    }
}

/// `examples/start.acproj`, looked for in the working directory, next to the
/// executable and its ancestors (a `target/` build inside the workspace),
/// and in the workspace this crate was built from.
pub fn find_start_example() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("examples/start.acproj")];
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            candidates.push(dir.join("examples/start.acproj"));
        }
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/start.acproj"));
    candidates.into_iter().find(|p| p.is_file())
}

/// The shipped examples folder: the one holding the Start example.
pub fn find_examples_dir() -> Option<PathBuf> {
    find_start_example().and_then(|p| p.parent().map(Path::to_path_buf))
}

/// The project shown when there is nothing else: an Oscillator into an
/// Audio Out (or just the output, if this build has no oscillator).
pub fn default_project(registry: &Registry) -> Project {
    let mut p = Project::default();
    let out = NodeId(2);
    p.graph
        .nodes
        .push(NodeInstance::new(out, "audio_out").at(320.0, 0.0));
    if registry.contains("oscillator") {
        let osc = NodeId(1);
        p.graph
            .nodes
            .push(NodeInstance::new(osc, "oscillator").at(0.0, 0.0));
        p.graph.wires.push(Wire::new(
            Endpoint::new(osc, "out"),
            Endpoint::new(out, "in"),
        ));
    }
    p
}

// ---------------------------------------------------------------- recent

const RECENT_MAX: usize = 10;

/// Recently opened projects, most recent first, persisted in the platform
/// config directory. Entries that no longer exist are kept (and shown as
/// missing) until the user removes them or opens something newer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recent {
    pub entries: Vec<PathBuf>,
    /// Where the list is stored; `None` keeps it in memory (tests).
    file: Option<PathBuf>,
}

impl Recent {
    pub fn in_memory() -> Recent {
        Recent::default()
    }

    /// Load from the platform config directory.
    pub fn load_default() -> Recent {
        let file = directories::ProjectDirs::from("", "", "Audio Constructor")
            .map(|d| d.config_dir().join("recent.json"));
        Recent::load_from(file)
    }

    pub fn load_from(file: Option<PathBuf>) -> Recent {
        let entries = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str::<Vec<PathBuf>>(&t).ok())
            .unwrap_or_default();
        Recent { entries, file }
    }

    pub fn add(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.entries.retain(|p| *p != path);
        self.entries.insert(0, path);
        self.entries.truncate(RECENT_MAX);
        self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|p| p != path);
        self.save();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.entries) {
            let _ = std::fs::write(file, text);
        }
    }
}

// ---------------------------------------------------------------- assets

/// The result of assigning an audio file to a Clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedAsset {
    /// Config `file`: relative to the project when copied into its assets.
    pub file: String,
    pub channels: u64,
    pub hash: String,
    /// Something to tell the user (e.g. the file wasn't copied).
    pub note: Option<String>,
}

/// Bring an audio file into the project. When the project has a location,
/// the file is copied to `<project>/assets/` (an identical file already there
/// is reused, a different one gets a new name) and referenced relatively;
/// otherwise the absolute path is kept and the note says so.
pub fn import_audio(src: &Path, project_path: Option<&Path>) -> Result<ImportedAsset, String> {
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let data = ac_io::wav::read_wav_bytes(&bytes).map_err(|e| format!("{}: {e}", src.display()))?;
    let hash = ac_core::util::content_hash(&bytes);
    let channels = data.channels.len().max(1) as u64;
    let Some(project_path) = project_path else {
        let abs = std::fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
        return Ok(ImportedAsset {
            file: abs.to_string_lossy().to_string(),
            channels,
            hash,
            note: Some(
                "The project isn't saved yet, so the clip refers to the file where it is. \
                 Saving the project copies it into the project's assets folder."
                    .into(),
            ),
        });
    };
    let rel = copy_into_assets(src, &bytes, &hash, &project_dir(project_path))?;
    Ok(ImportedAsset {
        file: rel,
        channels,
        hash,
        note: None,
    })
}

/// Copy `bytes` (the content of `src`) into `<dir>/assets/`, returning the
/// relative path to use in configs.
fn copy_into_assets(src: &Path, bytes: &[u8], hash: &str, dir: &Path) -> Result<String, String> {
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets).map_err(|e| format!("{}: {e}", assets.display()))?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "clip".into());
    let ext = src
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "wav".into());
    for n in 1.. {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        let dst = assets.join(&name);
        match std::fs::read(&dst) {
            Ok(existing) if ac_core::util::content_hash(&existing) == hash => {
                return Ok(format!("assets/{name}"));
            }
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&dst, bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
                return Ok(format!("assets/{name}"));
            }
        }
    }
    Err("no free file name in assets/".into())
}

/// Before saving to `target`: copy every asset that isn't already inside
/// the target's folder into its `assets/` and point the configs there, so
/// a saved project carries its files. `base` is where relative paths
/// currently resolve. Returns how many files were copied.
pub fn consolidate_assets(
    project: &mut Project,
    registry: &Registry,
    base: &Path,
    target: &Path,
) -> Result<usize, String> {
    let dir = project_dir(target);
    let dir_abs = std::fs::canonicalize(&dir).unwrap_or(dir.clone());
    let mut copied = 0;
    let mut fix = |n: &mut NodeInstance| -> Result<(), String> {
        let Some(entry) = registry.get(&n.kind) else {
            return Ok(());
        };
        for key in entry.meta.asset_keys {
            let path = n.config.get_str_or(key, "").to_string();
            if path.is_empty() {
                continue;
            }
            let p = Path::new(&path);
            let full = if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            };
            let full_abs = std::fs::canonicalize(&full).unwrap_or(full.clone());
            if full_abs.starts_with(&dir_abs) {
                // Already inside; just make sure it's relative.
                if let Ok(rel) = full_abs.strip_prefix(&dir_abs)
                    && p.is_absolute()
                    && let Some(obj) = n.config.as_object_mut()
                {
                    obj.insert(
                        (*key).to_string(),
                        rel.to_string_lossy().replace('\\', "/").into(),
                    );
                }
                continue;
            }
            let Ok(bytes) = std::fs::read(&full) else {
                continue; // missing: leave the reference for Locate…
            };
            let hash = ac_core::util::content_hash(&bytes);
            let rel = copy_into_assets(&full, &bytes, &hash, &dir)?;
            if let Some(obj) = n.config.as_object_mut() {
                obj.insert((*key).to_string(), rel.into());
            }
            copied += 1;
        }
        Ok(())
    };
    for n in &mut project.graph.nodes {
        fix(n)?;
    }
    for d in project.group_defs.values_mut() {
        for n in &mut d.graph.nodes {
            fix(n)?;
        }
    }
    Ok(copied)
}

/// Point every node that references `old` at the imported file instead.
pub fn relink_asset(
    project: &mut Project,
    registry: &Registry,
    old: &str,
    new: &ImportedAsset,
) -> usize {
    let mut count = 0;
    let mut fix = |n: &mut NodeInstance| {
        let Some(entry) = registry.get(&n.kind) else {
            return;
        };
        for key in entry.meta.asset_keys {
            if n.config.get_str_or(key, "") == old
                && let Some(obj) = n.config.as_object_mut()
            {
                obj.insert((*key).to_string(), new.file.clone().into());
                obj.insert("channels".into(), new.channels.into());
                obj.insert("hash".into(), new.hash.clone().into());
                count += 1;
            }
        }
    };
    for n in &mut project.graph.nodes {
        fix(n);
    }
    for d in project.group_defs.values_mut() {
        for n in &mut d.graph.nodes {
            fix(n);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ac-app-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn wav(path: &Path, value: f32) {
        ac_io::write_wav(
            path,
            &[vec![value; 100], vec![value; 100]],
            48_000,
            ac_io::BitDepth::Pcm16,
        )
        .unwrap();
    }

    #[test]
    fn recent_list_dedups_and_persists() {
        let d = temp("recent");
        let file = d.join("recent.json");
        let mut r = Recent::load_from(Some(file.clone()));
        r.add(&d.join("a.acproj"));
        r.add(&d.join("b.acproj"));
        r.add(&d.join("a.acproj"));
        assert_eq!(r.entries.len(), 2);
        assert!(r.entries[0].ends_with("a.acproj"));
        let again = Recent::load_from(Some(file));
        assert_eq!(again.entries, r.entries);
        // A corrupt file is an empty list, not a crash.
        std::fs::write(d.join("bad.json"), "{{{").unwrap();
        assert!(
            Recent::load_from(Some(d.join("bad.json")))
                .entries
                .is_empty()
        );
    }

    #[test]
    fn importing_copies_into_assets_once() {
        let d = temp("import");
        let src = d.join("kick.wav");
        wav(&src, 0.5);
        let proj = d.join("song/song.acproj");
        std::fs::create_dir_all(proj.parent().unwrap()).unwrap();
        let a = import_audio(&src, Some(&proj)).unwrap();
        assert_eq!(a.file, "assets/kick.wav");
        assert_eq!(a.channels, 2);
        assert!(d.join("song/assets/kick.wav").is_file());
        // Same content: reused. Different content, same name: renamed.
        assert_eq!(
            import_audio(&src, Some(&proj)).unwrap().file,
            "assets/kick.wav"
        );
        wav(&src, 0.25);
        assert_eq!(
            import_audio(&src, Some(&proj)).unwrap().file,
            "assets/kick-2.wav"
        );
        // Unsaved project: absolute path and a note.
        let b = import_audio(&src, None).unwrap();
        assert!(Path::new(&b.file).is_absolute());
        assert!(b.note.is_some());
        assert!(import_audio(&d.join("nope.wav"), None).is_err());
    }

    #[test]
    fn extension_is_added_when_missing() {
        assert_eq!(
            with_extension("a".into(), "acproj"),
            PathBuf::from("a.acproj")
        );
        assert_eq!(
            with_extension("a.ACPROJ".into(), "acproj"),
            PathBuf::from("a.ACPROJ")
        );
    }

    #[test]
    fn default_project_has_an_output() {
        let p = default_project(&ac_nodes::registry());
        assert!(p.graph.nodes.iter().any(|n| n.kind == "audio_out"));
    }
}
