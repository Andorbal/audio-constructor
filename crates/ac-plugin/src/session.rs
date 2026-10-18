//! The control side of a plugin instance: the project, its assets, and the
//! engine's [`EngineHandle`] while the host has the plugin active.
//!
//! Everything here runs off the audio thread: on the host's main thread
//! (state save and restore, activation), on the plugin's control thread
//! (dropping retired graphs, baking), and in the editor. They share one
//! [`Session`] behind a mutex in [`Shared`]; the audio thread never takes
//! it. Project changes recompile here and reach the audio thread through
//! the engine's graph swap.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use ac_core::{Project, Registry};
use ac_engine::{
    BakeJob, BakeResult, CompileReport, Engine, EngineHandle, EngineOptions, Processor,
};
use serde::{Deserialize, Serialize};

use crate::assets::AssetBank;
use crate::macros::MacroLabels;
use crate::{defaults, state};

/// Which plugin this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Audio in, audio out (and optional notes in).
    Effect,
    /// Notes in, audio out.
    Instrument,
}

/// The node registry every instance shares.
pub fn registry() -> Arc<Registry> {
    static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(ac_nodes::registry()))
        .clone()
}

/// What a plugin instance shares between the host's threads, the
/// control thread, and the editor.
#[derive(Debug)]
pub struct Shared {
    session: Mutex<Session>,
    labels: Arc<MacroLabels>,
}

impl Shared {
    /// A new instance's shared state, with the default project for `kind`.
    pub fn new(kind: Kind) -> Arc<Shared> {
        let labels = Arc::new(MacroLabels::default());
        Arc::new(Shared {
            session: Mutex::new(Session::new(kind, registry(), labels.clone())),
            labels,
        })
    }

    /// The session. Never call this on the audio thread.
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        // A panic while holding it can't leave the session half-written in a
        // way that matters more than losing the plugin; carry on.
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What each macro drives, for readouts.
    pub fn labels(&self) -> &Arc<MacroLabels> {
        &self.labels
    }
}

/// How often the control thread looks after the engine.
pub(crate) const TICK: std::time::Duration = std::time::Duration::from_millis(40);
/// How long parameter changes upstream of a baked node settle before it
/// bakes again (as in the app).
const BAKE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// A plugin instance's project, assets, and engine control.
pub struct Session {
    kind: Kind,
    registry: Arc<Registry>,
    /// The project while no engine runs; the handle owns it while one does.
    project: Project,
    assets: AssetBank,
    engine: Option<EngineHandle>,
    labels: Arc<MacroLabels>,
    report: Vec<String>,
    origin: Option<String>,
    revision: u64,
    dirty_since: Option<Instant>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("kind", &self.kind)
            .field("project", &self.project().name)
            .field("assets", &self.assets.len())
            .field("running", &self.engine.is_some())
            .finish()
    }
}

impl Session {
    pub fn new(kind: Kind, registry: Arc<Registry>, labels: Arc<MacroLabels>) -> Session {
        let project = defaults::project(kind);
        labels.update(&project, &registry);
        Session {
            kind,
            registry,
            project,
            assets: AssetBank::default(),
            engine: None,
            labels,
            report: Vec::new(),
            origin: None,
            revision: 0,
            dirty_since: None,
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The project as it stands (what a state save writes).
    pub fn project(&self) -> &Project {
        self.engine.as_ref().map_or(&self.project, |h| h.project())
    }

    /// The engine's control half while the host has the plugin active:
    /// telemetry, parameter changes, binding values, input monitoring.
    /// Change the project through [`Session::set_project`], not the
    /// handle's `edit`, so the macro readouts follow.
    pub fn engine(&mut self) -> Option<&mut EngineHandle> {
        self.engine.as_mut()
    }

    pub fn assets(&self) -> &AssetBank {
        &self.assets
    }

    /// What the last load or restore had to say: problems with the state,
    /// the project's compile errors, missing assets. Empty when all went
    /// well.
    pub fn report(&self) -> &[String] {
        &self.report
    }

    /// Where the project was loaded from, if from a file.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// Bumped whenever the project is replaced from outside the editor:
    /// a state restore, a file load, or going back to the default. An open
    /// editor rebuilds its document when it changes; its own
    /// [`Session::set_project`] calls don't bump it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replace the project (an edit, a load). Recompiles and hot-swaps when
    /// an engine runs, keeping the state of unchanged nodes. Returns the
    /// compile report (empty while inactive: the project compiles when the
    /// host activates the plugin).
    pub fn set_project(&mut self, project: Project) -> CompileReport {
        self.labels.update(&project, &self.registry);
        match self.engine.as_mut() {
            Some(h) => h.edit(ac_engine::GraphEdit::Replace(Box::new(project))),
            None => {
                self.project = project;
                CompileReport::default()
            }
        }
    }

    /// Add (or replace) an audio file the project references at config
    /// path `path`, as the editor does when someone picks a file for a
    /// Clip or Sampler. Its bytes go into the saved state.
    pub fn add_asset(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let r = self.assets.insert(path, bytes);
        self.refresh_resources();
        r
    }

    fn refresh_resources(&mut self) {
        let resources = self.assets.resources();
        if let Some(h) = self.engine.as_mut() {
            h.set_resources(resources);
        }
    }

    /// Open a project file (`.acproj`) and the audio files it references
    /// (relative to its folder). On success the report lists any missing
    /// assets and compile problems; on failure nothing changes.
    pub fn load_project_file(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let project = Project::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        let mut assets = AssetBank::default();
        let mut report = assets.load_from_disk(&project, &self.registry, &dir);
        self.assets = assets;
        self.origin = Some(path.display().to_string());
        let compiled = self.install(project);
        report.extend(compile_lines(&compiled));
        self.report = report;
        Ok(())
    }

    /// Go back to the default project.
    pub fn reset_to_default(&mut self) {
        self.assets = AssetBank::default();
        self.origin = None;
        let compiled = self.install(defaults::project(self.kind));
        self.report = compile_lines(&compiled);
    }

    /// New project and assets together: resources first, then one compile.
    fn install(&mut self, project: Project) -> CompileReport {
        self.revision += 1;
        let resources = self.assets.resources();
        if let Some(h) = self.engine.as_mut() {
            // Rebuilding with new resources resets nodes that read assets;
            // the project swap right after compiles once more with them.
            h.set_resources(resources);
        }
        self.set_project(project)
    }

    /// The state the host saves (see [`state`]).
    pub fn save_state(&self) -> String {
        let mut assets = self.assets.clone();
        assets.retain_used(self.project(), &self.registry);
        state::encode(self.kind, self.project(), &assets, self.origin.as_deref())
    }

    /// Restore a saved state. Never fails: a state that can't be used
    /// leaves the default project, and the report says why.
    pub fn restore_state(&mut self, text: &str) {
        match state::decode(text) {
            Ok(d) => {
                let mut report = d.warnings;
                if d.kind.is_some_and(|k| k != self.kind) {
                    report.push(format!(
                        "this state was saved by the {} plugin",
                        kind_name(d.kind.unwrap_or(self.kind))
                    ));
                }
                let mut assets = AssetBank::default();
                for (path, bytes) in d.assets {
                    if let Err(e) = assets.insert(&path, bytes) {
                        report.push(format!("{path} can't be played: {e}"));
                    }
                }
                for (path, _) in ac_core::referenced_assets(&d.project, &self.registry) {
                    if assets.get(&path).is_none() {
                        report.push(format!(
                            "{path} isn't in the saved state; it plays as silence"
                        ));
                    }
                }
                self.assets = assets;
                self.origin = d.origin;
                let compiled = self.install(d.project);
                report.extend(compile_lines(&compiled));
                self.report = report;
            }
            Err(why) => {
                self.reset_to_default();
                self.report
                    .insert(0, format!("Started with the default project: {why}"));
            }
        }
    }

    /// Build the engine at the host's rate and largest block, compile the
    /// project, and hand back the audio half. A running engine is replaced.
    pub(crate) fn start(
        &mut self,
        sample_rate: f64,
        max_block: usize,
    ) -> (Processor, CompileReport) {
        let project = self.project().clone();
        self.engine = None;
        let (engine, report) = Engine::new(
            project,
            self.registry.clone(),
            self.assets.resources(),
            EngineOptions {
                sample_rate,
                max_block: max_block.max(1),
                socket_telemetry: true,
                offline: false,
            },
        );
        let (handle, mut processor) = engine.split();
        crate::clock::init();
        processor.set_clock(Some(crate::clock::now_ns));
        self.engine = Some(handle);
        (processor, report)
    }

    /// Drop the engine (the host deactivated the plugin), keeping the
    /// project it ran.
    pub(crate) fn stop(&mut self) {
        if let Some(h) = self.engine.take() {
            self.project = h.project().clone();
        }
    }

    /// The control thread's regular care: drop retired graphs, rebake
    /// after parameter changes settle, and hand out bake jobs.
    pub(crate) fn tick(&mut self) -> Vec<BakeJob> {
        let Some(h) = self.engine.as_mut() else {
            return Vec::new();
        };
        h.collect_garbage();
        if h.bakes_dirty() {
            match self.dirty_since {
                None => self.dirty_since = Some(Instant::now()),
                Some(t) if t.elapsed() >= BAKE_DEBOUNCE => {
                    h.refresh();
                    self.dirty_since = None;
                }
                Some(_) => {}
            }
        } else {
            self.dirty_since = None;
        }
        h.take_bake_jobs()
    }

    /// Install a finished bake (dropped if the project moved on).
    pub(crate) fn finish_bake(&mut self, result: BakeResult) {
        if let Some(h) = self.engine.as_mut() {
            h.finish_bake(result);
        }
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Effect => "effect",
        Kind::Instrument => "instrument",
    }
}

/// Compile errors and warnings as report lines.
fn compile_lines(report: &CompileReport) -> Vec<String> {
    report.diagnostics.iter().map(|d| d.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Binding, NodeId, NodePath, Source};

    fn session(kind: Kind) -> Session {
        Session::new(kind, registry(), Arc::new(MacroLabels::default()))
    }

    #[test]
    fn state_round_trips_through_a_running_engine() {
        let mut a = session(Kind::Effect);
        let (_proc, report) = a.start(48_000.0, 256);
        assert!(report.diagnostics.is_empty(), "{report}");
        let mut p = a.project().clone();
        p.name = "Changed".into();
        p.bindings.push(Binding::input(
            Source::Macro { number: 9 },
            NodePath::root(NodeId(3)),
            "gain",
        ));
        assert!(a.set_project(p.clone()).diagnostics.is_empty());
        let saved = a.save_state();
        // Into a fresh, inactive instance, then activated.
        let mut b = session(Kind::Effect);
        b.restore_state(&saved);
        assert!(b.report().is_empty(), "{:?}", b.report());
        assert_eq!(b.project(), &p);
        let _ = b.start(44_100.0, 64);
        assert_eq!(b.project(), &p);
        b.stop();
        assert_eq!(b.project(), &p, "kept when the host deactivates");
    }

    #[test]
    fn bad_states_fall_back_to_the_default_with_a_report() {
        let mut s = session(Kind::Instrument);
        let mut p = s.project().clone();
        p.name = "Mine".into();
        s.set_project(p);
        for bad in ["", "{", "{\"format\":\"x\"}", "\u{0}\u{1}garbage"] {
            s.restore_state(bad);
            assert_eq!(s.project(), &defaults::project(Kind::Instrument), "{bad:?}");
            assert!(
                s.report()[0].starts_with("Started with the default project"),
                "{:?}",
                s.report()
            );
        }
        // A newer state says so.
        s.restore_state(r#"{"format":"audio-constructor-plugin-state","version":7,"project":{}}"#);
        assert!(s.report()[0].contains("newer version"), "{:?}", s.report());
    }

    #[test]
    fn project_files_load_with_their_assets_and_missing_ones_are_reported() {
        let dir = std::env::temp_dir().join(format!("ac-plugin-load-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let wav = crate::assets::wav_bytes(48_000, &[vec![0.5; 64]]);
        std::fs::write(dir.join("assets/hit.wav"), &wav).unwrap();
        let mut p = defaults::project(Kind::Effect);
        for (id, file) in [(10, "assets/hit.wav"), (11, "assets/gone.wav")] {
            p.graph.nodes.push(
                ac_core::NodeInstance::new(NodeId(id), "clip")
                    .with_config(serde_json::json!({ "file": file })),
            );
        }
        let file = dir.join("p.acproj");
        std::fs::write(&file, p.to_json()).unwrap();
        let mut s = session(Kind::Effect);
        let _ = s.start(48_000.0, 128);
        s.load_project_file(&file).unwrap();
        assert_eq!(s.project(), &p);
        assert!(s.assets().get("assets/hit.wav").is_some());
        assert!(
            s.report().iter().any(|l| l.contains("assets/gone.wav")),
            "{:?}",
            s.report()
        );
        // The asset travels in the state.
        let mut t = session(Kind::Effect);
        t.restore_state(&s.save_state());
        assert_eq!(*t.assets().get("assets/hit.wav").unwrap().bytes, wav);
        assert!(t.report().iter().any(|l| l.contains("assets/gone.wav")));
        assert!(s.load_project_file(&dir.join("nope.acproj")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
