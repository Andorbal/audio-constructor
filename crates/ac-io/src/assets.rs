//! Asset loading: audio files referenced by node configs (ARCHITECTURE §12).
//! Paths are relative to the project directory; a content hash detects files
//! that changed since they were assigned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{AudioData, Project, RecordRequest, RecordSink, Registry, Resources, util::content_hash};

/// An asset problem found while loading.
#[derive(Clone, Debug, PartialEq)]
pub enum AssetIssue {
    Missing { path: String, reason: String },
    Changed { path: String },
}

/// Decoded audio assets keyed by the path string used in node configs.
#[derive(Clone, Debug, Default)]
pub struct AssetStore {
    pub project_dir: PathBuf,
    assets: HashMap<String, Arc<AudioData>>,
    pub issues: Vec<AssetIssue>,
}

impl AssetStore {
    pub fn new(project_dir: &Path) -> AssetStore {
        AssetStore {
            project_dir: project_dir.to_path_buf(),
            ..AssetStore::default()
        }
    }

    /// Load every asset the project's nodes reference (inside groups too).
    pub fn load_for(project: &Project, project_dir: &Path, registry: &Registry) -> AssetStore {
        let mut store = AssetStore::new(project_dir);
        for (path, hash) in referenced_assets(project, registry) {
            store.load(&path, hash.as_deref());
        }
        store
    }

    /// Resolve a config path against the project directory.
    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.project_dir.join(p)
        }
    }

    /// Load one asset (no-op if already loaded). Returns whether it's available.
    pub fn load(&mut self, path: &str, expected_hash: Option<&str>) -> bool {
        if self.assets.contains_key(path) {
            return true;
        }
        let full = self.resolve(path);
        let bytes = match std::fs::read(&full) {
            Ok(b) => b,
            Err(e) => {
                self.issues.push(AssetIssue::Missing {
                    path: path.to_string(),
                    reason: e.to_string(),
                });
                return false;
            }
        };
        if let Some(h) = expected_hash
            && !h.is_empty()
            && content_hash(&bytes) != h
        {
            self.issues.push(AssetIssue::Changed {
                path: path.to_string(),
            });
        }
        match crate::wav::read_wav_bytes(&bytes) {
            Ok(data) => {
                self.assets.insert(path.to_string(), Arc::new(data));
                true
            }
            Err(e) => {
                self.issues.push(AssetIssue::Missing {
                    path: path.to_string(),
                    reason: e.to_string(),
                });
                false
            }
        }
    }

    pub fn insert(&mut self, path: &str, data: AudioData) {
        self.assets.insert(path.to_string(), Arc::new(data));
    }

    pub fn get(&self, path: &str) -> Option<Arc<AudioData>> {
        self.assets.get(path).cloned()
    }

    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.assets.keys()
    }
}

/// `(path, stored hash)` for every asset reference in a project.
pub fn referenced_assets(project: &Project, registry: &Registry) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let graphs = std::iter::once(&project.graph).chain(project.group_defs.values().map(|d| &d.graph));
    for g in graphs {
        for n in &g.nodes {
            let Some(entry) = registry.get(&n.kind) else {
                continue;
            };
            for key in entry.meta.asset_keys {
                if let Some(p) = n.config.get(*key).and_then(|v| v.as_str())
                    && !p.is_empty()
                {
                    let hash = n.config.get("hash").and_then(|v| v.as_str()).map(str::to_string);
                    out.push((p.to_string(), hash));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Opens recording sinks for `Record` nodes (implemented by the disk recorder).
pub trait RecorderFactory: Send + Sync {
    fn open(&self, req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>>;
}

/// Resources backed by files: loaded assets and an optional recorder.
#[derive(Clone, Default)]
pub struct FileResources {
    pub assets: AssetStore,
    pub recorder: Option<Arc<dyn RecorderFactory>>,
}

impl Resources for FileResources {
    fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
        self.assets.get(path)
    }

    fn recorder(&self, req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
        self.recorder.as_ref().and_then(|r| r.open(req))
    }
}
