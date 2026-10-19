//! One frame of the plugin window: the session and the editor kept in step
//! (see the module above for what goes which way).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ac_app::EditorApp;
use ac_app::host::HostRequest;
use ac_io::AssetStore;

use crate::assets::AssetBank;
use crate::session::Session;

/// How long moves and other changes the engine doesn't hear wait before
/// they go to the session (each one recompiles there).
pub(crate) const PUSH_EVERY: Duration = Duration::from_millis(250);

/// No session revision seen yet.
pub(crate) const UNSEEN: u64 = u64::MAX;

/// What a frame left to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// Changes wait to go to the session: draw again after
    /// [`PUSH_EVERY`].
    pub(crate) pending: bool,
    /// The editor asked the session for something that changes the
    /// project: draw again now so the editor shows it.
    pub(crate) asked: bool,
}

/// Run `draw` (the editor's frame) with the session's engine, after
/// bringing the session's project in if it was replaced, and send the
/// editor's changes and requests back. `seen` is the session revision the
/// editor's document came from (shared by every window of an instance);
/// `last_push` when changes last went to the session.
pub(crate) fn frame(
    session: &mut Session,
    editor: &mut EditorApp,
    seen: &AtomicU64,
    last_push: &mut Option<Instant>,
    draw: impl FnOnce(&mut EditorApp),
) -> Outcome {
    // Read before the engine is lent: a running engine holds the project.
    let incoming = incoming(session, seen);
    let body = |e: &mut EditorApp| {
        // Loaded with the session's engine in place, so leaving a learn
        // or a held control lets go of it there.
        if let Some(i) = incoming {
            i.load(e);
        }
        draw(e);
    };
    match session.engine() {
        Some(handle) => editor.with_engine(handle, body),
        None => body(editor),
    }
    send_out(session, editor, last_push, false);
    let asked = run_requests(session, editor);
    Outcome {
        pending: editor.has_changes(),
        asked,
    }
}

/// A project the session holds that the editor hasn't shown yet.
pub(crate) struct Incoming {
    project: ac_core::Project,
    assets: AssetStore,
    report: Vec<String>,
}

impl Incoming {
    pub(crate) fn load(self, editor: &mut EditorApp) {
        editor.load_project_with(self.project, None, self.assets);
        if !self.report.is_empty() {
            editor.tell(self.report.join(" · "), true);
        }
    }
}

/// The session's project, if it was replaced since the editor last showed
/// it (a state restore, a file load, going back to the default).
pub(crate) fn incoming(session: &Session, seen: &AtomicU64) -> Option<Incoming> {
    let revision = session.revision();
    if seen.swap(revision, Ordering::AcqRel) == revision {
        return None;
    }
    Some(Incoming {
        project: session.project().clone(),
        assets: store_of(session.assets()),
        report: session.report().to_vec(),
    })
}

/// The editor's changes to the session: what the engine hears at once,
/// the rest every [`PUSH_EVERY`] (or now, with `now`: the window closes).
pub(crate) fn send_out(
    session: &mut Session,
    editor: &mut EditorApp,
    last_push: &mut Option<Instant>,
    now: bool,
) {
    let engine_edit = editor.take_engine_edit();
    let due = now || engine_edit.is_some() || last_push.is_none_or(|t| t.elapsed() >= PUSH_EVERY);
    if !due {
        return;
    }
    let Some(project) = editor.take_changes().or(engine_edit) else {
        return;
    };
    let running = session.engine().is_some();
    let report = session.set_project(project);
    editor.host_compiled(running.then_some(report));
    *last_push = Some(Instant::now());
}

/// Do what the editor asked of the host. Returns whether it asked
/// anything.
fn run_requests(session: &mut Session, editor: &mut EditorApp) -> bool {
    let requests = editor.take_host_requests();
    let asked = !requests.is_empty();
    for r in requests {
        match r {
            HostRequest::New => session.reset_to_default(),
            HostRequest::Open(path) => {
                if let Err(e) = session.load_project_file(&path) {
                    editor.tell(format!("Couldn't open {}: {e}", path.display()), true);
                }
            }
            HostRequest::AddAsset { path, bytes } => {
                if let Err(e) = session.add_asset(&path, bytes) {
                    editor.tell(format!("{path} can't be played: {e}"), true);
                }
                editor.set_host_assets(store_of(session.assets()));
            }
            HostRequest::WriteAssets { dir } => {
                if let Err(e) = write_assets(session.assets(), &dir) {
                    editor.tell(format!("Saved, but its audio files: {e}"), true);
                }
            }
        }
    }
    asked
}

/// The audio files the session holds, written into `dir` at the paths the
/// project refers to them by (a Save As from the plugin), as they were
/// read. Files already there stay; paths that would leave `dir` (`..`, a
/// root) and absolute ones are skipped.
pub(crate) fn write_assets(bank: &AssetBank, dir: &Path) -> Result<(), String> {
    use std::path::Component;
    for (key, asset) in bank.iter() {
        let rel = Path::new(key);
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            continue;
        }
        let dst = dir.join(rel);
        if dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&dst, asset.bytes.as_slice())
            .map_err(|e| format!("{}: {e}", dst.display()))?;
    }
    Ok(())
}

/// The session's audio files as the editor shows them (shared, not
/// copied).
pub(crate) fn store_of(bank: &AssetBank) -> AssetStore {
    let mut store = AssetStore::new(Path::new(""));
    for (path, asset) in bank.iter() {
        if let Some(audio) = asset.audio() {
            store.insert_shared(path, audio.clone());
        }
    }
    store
}
