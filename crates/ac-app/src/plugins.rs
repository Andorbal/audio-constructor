//! Plugins in the editor (DECISIONS D130–D135): the plugin host the
//! engine's resources carry, scans in the background (each bundle
//! described by a child copy of the editor, `audio-constructor
//! --scan-plugin <bundle>`, so a plugin that crashes while being described
//! can't take the editor down), the palette's entries for what was found,
//! and the main-thread work `ac_host::poll` asks for every frame.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use ac_core::{
    Category, PLUGIN_KIND, PluginDesc, PluginLoader, Project, Registry, project_plugins,
};
use ac_host::{Catalog, Failure, PluginHost, Scanner};
use serde_json::json;

use crate::canvas::PendingWire;
use crate::palette::{Candidate, pick_socket};

/// The argument a scanning child copy of the editor gets, before the
/// bundle's path.
pub const SCAN_FLAG: &str = "--scan-plugin";

/// Where the editor's plugins come from.
#[derive(Clone, Debug, Default)]
pub struct PluginsConfig {
    /// The scan cache (shared with `audioc`).
    pub cache: Option<PathBuf>,
    /// How to describe a bundle in a child process; without one the
    /// editor doesn't scan.
    pub scanner: Option<Scanner>,
    /// A catalog to start from instead of the cache (tests).
    pub catalog: Option<Catalog>,
    /// Look for new and changed plugins when the editor starts.
    pub scan_at_start: bool,
}

impl PluginsConfig {
    /// The desktop editor's: the shared cache, this executable as the
    /// scanner, and a scan at start (only new or changed bundles are
    /// opened, so it's quick after the first time).
    pub fn desktop() -> PluginsConfig {
        PluginsConfig {
            cache: ac_host::default_cache_path(),
            scanner: std::env::current_exe().ok().map(|program| Scanner {
                program,
                args: vec![SCAN_FLAG.into()],
            }),
            catalog: None,
            scan_at_start: true,
        }
    }

    /// Plugins from a fixed catalog, never scanned (tests).
    pub fn with_catalog(catalog: Catalog) -> PluginsConfig {
        PluginsConfig {
            catalog: Some(catalog),
            ..PluginsConfig::default()
        }
    }
}

/// The editor's plugin host and its catalog.
pub struct Plugins {
    pub host: Arc<PluginHost>,
    cache: Option<PathBuf>,
    scanner: Option<Scanner>,
    scan: Option<Receiver<Catalog>>,
    /// Every plugin found, sorted for menus.
    pub list: Vec<PluginDesc>,
    pub failed: Vec<Failure>,
}

impl Plugins {
    pub fn new(cfg: PluginsConfig) -> Plugins {
        let catalog = match (&cfg.catalog, &cfg.cache) {
            (Some(c), _) => c.clone(),
            (None, Some(p)) => Catalog::load(p),
            (None, None) => Catalog::default(),
        };
        let mut p = Plugins {
            host: Arc::new(PluginHost::new(Catalog::default())),
            cache: cfg.cache,
            scanner: cfg.scanner,
            scan: None,
            list: Vec::new(),
            failed: Vec::new(),
        };
        p.install(catalog);
        if cfg.scan_at_start {
            p.rescan(false);
        }
        p
    }

    fn install(&mut self, catalog: Catalog) {
        self.list = catalog.sorted().into_iter().cloned().collect();
        self.failed = catalog.failed.clone();
        self.host.set_catalog(catalog);
    }

    /// What the engine's resources load plugins with.
    pub fn loader(&self) -> Arc<dyn PluginLoader> {
        self.host.clone()
    }

    /// Look for plugins again in the background: every bundle when
    /// `fresh`, else only new or changed ones. Needs a scanner.
    pub fn rescan(&mut self, fresh: bool) {
        if self.scan.is_some() {
            return;
        }
        let Some(scanner) = self.scanner.clone() else {
            return;
        };
        let previous = if fresh {
            Catalog::default()
        } else {
            self.host.catalog()
        };
        let cache = self.cache.clone();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("ac-plugin-scan".into())
            .spawn(move || {
                let c = ac_host::scan::scan(Some(&scanner), &previous, |_| {});
                if let Some(p) = &cache {
                    let _ = c.save(p);
                }
                let _ = tx.send(c);
            });
        if spawned.is_ok() {
            self.scan = Some(rx);
        }
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    pub fn can_scan(&self) -> bool {
        self.scanner.is_some()
    }

    /// Install a finished scan. Returns whether one finished.
    pub fn poll_scan(&mut self) -> bool {
        let Some(rx) = &self.scan else {
            return false;
        };
        match rx.try_recv() {
            Ok(c) => {
                self.scan = None;
                self.install(c);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.scan = None;
                true
            }
        }
    }

    /// Keep the state of every plugin running here for the instances
    /// that replace them (before the engine rebuilds every node).
    pub fn keep_states(&self) {
        for key in ac_host::running() {
            self.host.prepare_restart(&key);
        }
    }
}

/// Write the state of each running top-level plugin into its node's
/// config, as the project saves it. A plugin inside a group shares its
/// definition with every instance, so it saves its parameters but not a
/// state of its own.
pub fn store_states(project: &mut Project) {
    for n in project
        .graph
        .nodes
        .iter_mut()
        .filter(|n| n.kind == PLUGIN_KIND)
    {
        let Some(state) = ac_host::save_state(&n.id.to_string()) else {
            continue;
        };
        if !n.config.is_object() {
            n.config = ac_core::empty_config();
        }
        if let Some(o) = n.config.as_object_mut() {
            o.insert("state".into(), json!(state));
        }
    }
}

/// Whether a project has Plugin nodes.
pub fn uses_plugins(project: &Project) -> bool {
    !project_plugins(project).is_empty()
}

/// "CLAP instrument", "VST3 effect".
pub fn kind_label(d: &PluginDesc) -> String {
    let kind = if d.instrument || (d.audio_in.is_empty() && d.midi_in) {
        "instrument"
    } else {
        "effect"
    };
    format!("{} {kind}", d.format.name())
}

/// The palette's (and the Library tab's) entry for a plugin, or `None`
/// when it doesn't match `query` or can't complete `pending`.
pub fn candidate(
    d: &PluginDesc,
    registry: &Registry,
    query: &str,
    pending: Option<&PendingWire>,
) -> Option<Candidate> {
    let q = query.trim().to_lowercase();
    let label = kind_label(d);
    let haystack = format!("{} {} {label} plugin", d.name, d.vendor).to_lowercase();
    if !q.is_empty() && !q.split_whitespace().all(|w| haystack.contains(w)) {
        return None;
    }
    let config = json!({ "plugin": d });
    let socket = match pending {
        Some(p) => {
            let sig = (registry.get(PLUGIN_KIND)?.describe)(&config);
            Some(pick_socket(&sig, p)?)
        }
        None => None,
    };
    let by = if d.vendor.is_empty() {
        String::new()
    } else {
        format!(" by {}", d.vendor)
    };
    Some(Candidate {
        name: d.name.clone(),
        kind: PLUGIN_KIND.to_string(),
        category: Some(Category::Utility),
        summary: format!("A {label}{by}, installed on this computer."),
        config,
        socket,
        block: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{PluginFormat, PluginParam, PluginPort};

    pub(crate) fn synth() -> PluginDesc {
        PluginDesc {
            format: PluginFormat::Clap,
            id: "org.example.synth".into(),
            name: "Example Synth".into(),
            vendor: "Example Audio".into(),
            instrument: true,
            audio_out: vec![PluginPort {
                name: "Out".into(),
                channels: 2,
            }],
            midi_in: true,
            params: vec![PluginParam {
                id: 1,
                name: "Cutoff".into(),
                socket: "cutoff".into(),
                min: 0.0,
                max: 1.0,
                default: 0.5,
                stepped: false,
            }],
            ..PluginDesc::default()
        }
    }

    #[test]
    fn candidates_match_name_vendor_and_kind() {
        let r = ac_nodes::registry();
        let d = synth();
        assert!(candidate(&d, &r, "", None).is_some());
        assert!(candidate(&d, &r, "example syn", None).is_some());
        assert!(candidate(&d, &r, "instrument", None).is_some());
        assert!(candidate(&d, &r, "clap", None).is_some());
        assert!(candidate(&d, &r, "reverb", None).is_none());
        let c = candidate(&d, &r, "", None).unwrap();
        assert_eq!(c.kind, PLUGIN_KIND);
        assert_eq!(
            c.summary,
            "A CLAP instrument by Example Audio, installed on this computer."
        );
        assert_eq!(PluginDesc::from_config(&c.config), Some(d));
    }
}
