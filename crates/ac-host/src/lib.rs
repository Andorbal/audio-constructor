//! Hosting CLAP and VST3 plugins as Audio Constructor nodes (ARCHITECTURE
//! §13, DECISIONS D130–D136).
//!
//! - [`scan`] finds plugins in the standard folders (in a child process if
//!   asked, so one that crashes can't take the caller down) and describes
//!   them: ports, note ports, parameters. A description is what a `Plugin`
//!   node stores and builds its sockets from.
//! - [`PluginHost`] is the [`ac_core::PluginLoader`] the editor and
//!   `audioc` put in their resources: when the engine prepares a `Plugin`
//!   node, it finds the plugin, creates and activates it at the engine's
//!   rate and largest block, and hands back the audio half.
//! - The main-thread half stays on the thread that created it, which hosts
//!   keep as their main thread; [`poll`] there runs what plugins ask of it
//!   and reports parameter moves and restarts. Plugin windows open with
//!   [`show_window`].
//!
//! This crate is native only: loading plugin libraries needs `unsafe`
//! (each use says why), and none of it builds for the web.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod base64;
mod clap;
mod gui;
mod library;
mod registry;
pub mod scan;
mod vst3;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use ac_core::{PluginDesc, PluginFormat, PluginLoader, PluginProcessor, PluginRequest};

pub use gui::{close_window, has_window, show_window, window_open};
pub use library::is_resident;
pub use registry::{HostEvent, poll, running, shutdown};
pub use scan::{Catalog, Failure, Scanner, child_main, default_cache_path, describe_bundle};

/// Loads plugins for `Plugin` nodes. Shared by every engine of a host
/// (`Arc<PluginHost>`); everything that touches a plugin's main thread
/// happens on the thread calling [`PluginLoader::load`].
#[derive(Default)]
pub struct PluginHost {
    catalog: RwLock<Catalog>,
    clap_entries: Mutex<HashMap<PathBuf, clack_host::prelude::PluginEntry>>,
    vst3_modules: Mutex<HashMap<PathBuf, Arc<vst3::Module>>>,
    /// State to start a node's next instance with (a restart in progress).
    restarts: Mutex<HashMap<String, Vec<u8>>>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost").finish_non_exhaustive()
    }
}

impl PluginHost {
    pub fn new(catalog: Catalog) -> PluginHost {
        PluginHost {
            catalog: RwLock::new(catalog),
            ..PluginHost::default()
        }
    }

    pub fn catalog(&self) -> Catalog {
        self.catalog.read().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn set_catalog(&self, catalog: Catalog) {
        if let Ok(mut c) = self.catalog.write() {
            *c = catalog;
        }
    }

    /// Where the plugin a description names is installed: the catalog's
    /// entry for its id (the catalog holds only what a scan of the plugin
    /// folders found), or the path it was described at if that's in a
    /// plugin folder too (installed since the last scan). A path anywhere
    /// else is never loaded, whatever the project says (DECISIONS D197).
    pub fn locate(&self, desc: &PluginDesc) -> Option<PathBuf> {
        let from_catalog = self.catalog.read().ok().and_then(|c| {
            c.find(desc.format, &desc.id)
                .map(|d| PathBuf::from(&d.path))
        });
        from_catalog
            .filter(|p| p.exists())
            .or_else(|| scan::in_plugin_folders(Path::new(&desc.path), desc.format))
    }

    /// Keep the state of the plugin running for `node_key` (on this
    /// thread) for the instance that replaces it: call before rebuilding a
    /// node after [`HostEvent::Restart`].
    pub fn prepare_restart(&self, node_key: &str) {
        if let Some(state) = registry::save_state(node_key)
            && let Ok(mut r) = self.restarts.lock()
        {
            r.insert(node_key.to_string(), state);
        }
    }

    /// Forget states kept for restarts (a different project opened).
    pub fn clear_restarts(&self) {
        if let Ok(mut r) = self.restarts.lock() {
            r.clear();
        }
    }

    fn vst3_module(&self, path: &Path) -> Result<Arc<vst3::Module>, String> {
        let mut modules = self
            .vst3_modules
            .lock()
            .map_err(|_| "plugin host poisoned")?;
        if let Some(m) = modules.get(path) {
            return Ok(m.clone());
        }
        let m = vst3::Module::load(path)?;
        modules.insert(path.to_path_buf(), m.clone());
        Ok(m)
    }

    fn clap_entry(&self, path: &Path) -> Result<clack_host::prelude::PluginEntry, String> {
        let mut entries = self
            .clap_entries
            .lock()
            .map_err(|_| "plugin host poisoned")?;
        if let Some(e) = entries.get(path) {
            return Ok(e.clone());
        }
        let e = clap::load_entry(path)?;
        entries.insert(path.to_path_buf(), e.clone());
        Ok(e)
    }
}

/// Why a plugin that [`PluginHost::locate`] can't find doesn't run, with
/// where the project last saw it, if anywhere: only a hint for the person,
/// never a place to load from.
fn not_installed(desc: &PluginDesc) -> String {
    if desc.path.is_empty() {
        "isn't installed on this computer".into()
    } else {
        format!(
            "isn't installed on this computer (in its plugin folders; last seen at {})",
            desc.path
        )
    }
}

/// The saved state of the plugin running for `node_key` on this thread,
/// as the project stores it (base64).
pub fn save_state(node_key: &str) -> Option<String> {
    registry::save_state(node_key).map(|b| base64::encode(&b))
}

impl PluginLoader for PluginHost {
    fn load(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        let Some(path) = self.locate(req.desc) else {
            return Err(not_installed(req.desc));
        };
        let restart = self
            .restarts
            .lock()
            .ok()
            .and_then(|mut r| r.remove(req.node_key));
        let saved;
        let state: Option<&[u8]> = match &restart {
            Some(s) => Some(s),
            None if req.state.is_empty() => None,
            None => {
                saved = base64::decode(req.state);
                match &saved {
                    Some(s) => Some(s),
                    None => {
                        eprintln!(
                            "warning: {}'s saved state is damaged; it starts fresh",
                            req.desc.label()
                        );
                        None
                    }
                }
            }
        };
        match req.desc.format {
            PluginFormat::Clap => {
                let entry = self.clap_entry(&path)?;
                clap::instantiate(&entry, req, state)
            }
            PluginFormat::Vst3 => {
                let module = self.vst3_module(&path)?;
                vst3::instantiate(&module, req, state)
            }
        }
    }
}
