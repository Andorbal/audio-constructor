//! The editor: the document, the canvas, the panels, the engine, and the
//! commands that tie them together.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use ac_core::{Endpoint, GROUP_KIND, NodeId, Project, Registry, TempoMap};
use ac_io::assets::AssetIssue;
use ac_io::{AssetStore, FileResources, Library};
use egui::{Event, Key, RichText, vec2};

use crate::actions::{self, Action};
use ac_engine::{EngineHandle, Processor};

use crate::audio::{AudioMode, EngineHost, HostAudio};
use crate::canvas::{Canvas, CanvasEvent, CanvasIo, PendingWire};
use crate::dialogs::{self, Confirm, FilePurpose, PathPrompt};
use crate::document::Document;
use crate::edit::connect::{self, FeedError, SelectionOrder, Source};
use crate::edit::{self, HIDE_UNWIRED, pins};
use crate::export::{ExportSummary, export_wav};
use crate::files::{self, Recent};
use crate::find::{FindBar, Found};
use crate::host::{HostControls, HostRequest};
use crate::i18n::{self, Language, LanguageChoice, tr, tr_args};
use crate::icons;
use crate::menubar::{self, MenuCommand, MenuState};
use crate::online::Online;
use crate::palette::{Candidate, Palette};
use crate::panels::{PanelIo, PanelRequest, SidePanel, Tab};
use crate::perform::{PerformView, PinRequest};
use crate::play_keys::{KeyInput, PlayKeys};
use crate::settings::{self, AppSettings};
use crate::theme::{self, ThemeChoice};
use crate::transport::TransportState;
use crate::user_themes::UserThemes;

mod host_files;
mod sharing;

pub use host_files::{AudioPurpose, FileRequest, SaveKind};

/// How to start the editor.
pub struct AppOptions {
    pub audio: AudioMode,
    /// A project to open.
    pub project: Option<PathBuf>,
    /// With no project, open `examples/start.acproj` if it exists.
    pub start_example: bool,
    pub recent: Recent,
    /// Use the system's file dialogs (tests type paths instead).
    pub native_dialogs: bool,
    /// Seed for node ids (fixed in tests, from the clock otherwise).
    pub seed: u64,
    /// The folder of the person's own and imported blocks. `None` keeps
    /// them in memory, with only the shipped blocks to start from.
    pub library: Option<PathBuf>,
    /// The editor's theme and UI scale, and where they're saved.
    pub settings: AppSettings,
    /// Host CLAP and VST3 plugins (the desktop editor; not tests unless
    /// they ask, and not the editor inside a plugin).
    pub plugins: Option<crate::plugins::PluginsConfig>,
    /// The host keeps the files (the web build, which has no file system
    /// the editor can reach): file commands become
    /// [`EditorApp::take_file_requests`], and audio files live in memory.
    pub host_files: bool,
}

impl AppOptions {
    /// For tests: no audio thread, no persistence, no system dialogs.
    pub fn headless() -> AppOptions {
        AppOptions {
            audio: AudioMode::Off,
            project: None,
            start_example: false,
            recent: Recent::in_memory(),
            native_dialogs: false,
            seed: 1,
            library: None,
            settings: AppSettings::in_memory(),
            plugins: None,
            host_files: false,
        }
    }

    /// For a plugin: the host runs the audio (`audio` until it says
    /// otherwise), and the editor keeps the person's recent list, settings,
    /// and block library where the desktop editor keeps them. It starts on
    /// the default patch; the host restores its own project with
    /// [`EditorApp::load_project`].
    pub fn hosted(audio: HostAudio) -> AppOptions {
        AppOptions {
            audio: AudioMode::Hosted(audio),
            project: None,
            start_example: false,
            recent: Recent::load_default(),
            native_dialogs: cfg!(feature = "native-dialogs"),
            seed: clock_seed(),
            library: files::default_library_dir(),
            settings: AppSettings::load_default(),
            plugins: None,
            host_files: false,
        }
    }

    /// For a page in a browser: the page runs the audio (`audio`), keeps
    /// the files ([`EditorApp::take_file_requests`]), and gives the
    /// shipped examples ([`EditorApp::set_examples`]). Nothing is read
    /// from or written to disk; the settings live as long as the page
    /// (or wherever [`AppSettings`] is given).
    pub fn web(audio: HostAudio, seed: u64, settings: AppSettings) -> AppOptions {
        AppOptions {
            audio: AudioMode::Hosted(audio),
            project: None,
            start_example: false,
            recent: Recent::in_memory(),
            native_dialogs: false,
            seed,
            library: None,
            settings,
            host_files: true,
            plugins: None,
        }
    }
}

/// A seed for node ids that differs from run to run.
pub fn clock_seed() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

/// Something waiting for the user to confirm discarding changes.
#[derive(Clone, Debug, PartialEq)]
enum AfterConfirm {
    New,
    Open(PathBuf),
    /// A shipped example, opened as an untitled copy.
    OpenExample(PathBuf),
    /// The project or pack the host handed over (`EditorApp::open_file`),
    /// waiting in `picked`.
    OpenPicked,
    Quit,
}

/// What the editor last told the engine about the song's transport.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SongSync {
    looped: Option<(u64, u64)>,
    punch: Option<(u64, u64)>,
    metronome: bool,
    count_in: u32,
}

pub struct EditorApp {
    pub registry: Arc<Registry>,
    pub doc: Document,
    pub canvas: Canvas,
    pub palette: Palette,
    pub engine: EngineHost,
    pub side: SidePanel,
    /// The performance panels' view (UX.md "Panels").
    pub perform: PerformView,
    pub recent: Recent,
    /// Shipped, saved, and imported blocks.
    pub library: Library,
    assets: AssetStore,
    missing: BTreeSet<String>,
    loaded_assets: Vec<(String, Option<String>)>,
    transport: TransportState,
    ruler: crate::ruler::Ruler,
    status: Option<(String, f64, bool)>,
    confirm: Option<AfterConfirm>,
    prompt: Option<PathPrompt>,
    native_dialogs: bool,
    /// Writes Record nodes' takes into the saved project's assets folder.
    recorder: Option<Arc<ac_io::DiskRecorder>>,
    /// Finished takes waiting to be turned into Clip nodes.
    pub takes: Vec<ac_io::Take>,
    export_job: Option<(PathBuf, Receiver<Result<ExportSummary, String>>)>,
    show_timing: bool,
    /// Per-node timing read from the engine while it's shown.
    timing: crate::timing::Timings,
    show_shortcuts: bool,
    show_about: bool,
    title: String,
    allow_close: bool,
    /// The transport settings last sent to the engine (loop, punch, click).
    loop_sent: Option<SongSync>,
    space_shift: bool,
    /// When Escape was last pressed: a second press soon after is Panic.
    escape_at: Option<f64>,
    /// Which Audio Out nodes are sounding (the status bar).
    sounding: crate::sounding::Sounding,
    heard: Vec<crate::sounding::Output>,
    close_now: bool,
    /// The last subgraph copied (for pastes that don't carry text).
    clipboard: Option<String>,
    /// Export Pack is waiting for an untitled project to be saved.
    pack_after_save: bool,
    /// Nodes created by the last command (tests read this).
    pub last_added: Vec<NodeId>,
    /// Learning a binding: waiting for a control to move.
    pub learn: Option<crate::bindings::Learn>,
    /// Bound inputs of the current scope and their live values.
    bound: crate::bindings::BoundInputs,
    /// The computer keys as a control source (see `play_keys.rs`).
    play_keys: PlayKeys,
    /// Play keys' state before performance mode turned it on, while
    /// performance mode lasts.
    play_keys_before_performance: Option<bool>,
    /// The find bar (`Ctrl+F`).
    pub find: FindBar,
    /// The order the selected nodes were selected in.
    selection_order: SelectionOrder,
    /// The selected nodes the palette's pick will be wired from, in
    /// selection order (connect selected to new node).
    palette_feed: Vec<NodeId>,
    /// Theme and UI scale (View menu), saved as they change.
    settings: AppSettings,
    /// The person's own themes (the themes folder beside the settings).
    themes: UserThemes,
    /// The language the editor shows (View ▸ Language, resolved).
    language: Language,
    /// The update check and crash reports (desktop only, D173, D174).
    #[cfg(not(target_arch = "wasm32"))]
    product: Option<crate::product::Product>,
    /// The editor's time, kept from going backwards when it moves to
    /// another context (a reopened plugin window's clock starts at zero).
    clock: Clock,
    /// Bumped when the document is replaced or saved (changes the
    /// revisions can't tell apart).
    epoch: u64,
    /// The document's state when the embedding host last heard of it
    /// ([`EditorApp::take_changes`]).
    host_seen: ChangeStamp,
    /// The host keeps the files (see [`AppOptions::host_files`]).
    host_files: bool,
    /// File commands waiting for the host.
    file_requests: Vec<FileRequest>,
    /// Audio files held in memory when the host keeps the files, by the
    /// path the project's nodes use.
    asset_bank: std::collections::BTreeMap<String, Arc<Vec<u8>>>,
    /// A file the host handed over, waiting for the unsaved-changes
    /// question.
    picked: Option<(String, Vec<u8>)>,
    /// What to do once the host has saved the document (the
    /// unsaved-changes question's Save, when the host keeps the files).
    after_host_save: Option<AfterConfirm>,
    /// The host's automatable parameters, shown in the Devices tab.
    host_controls: Option<Box<dyn HostControls>>,
    /// What the editor asked of a host that keeps the project.
    host_requests: Vec<HostRequest>,
    /// Shared libraries, imports from links, block exports, and the
    /// preview playing (`online.rs`).
    pub online: Online,
    /// CLAP and VST3 plugins found here, and the host that runs them.
    pub plugins: Option<crate::plugins::Plugins>,
}

/// Where the document is in its history: equal stamps, same project.
type ChangeStamp = (u64, u64, u64);

/// A monotonic clock over egui's input time.
#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    offset: f64,
    last: f64,
}

impl Clock {
    fn now(&mut self, input_time: f64) -> f64 {
        let t = input_time + self.offset;
        if t < self.last {
            // A new context started its clock over: carry on from here.
            self.offset += self.last - t;
        } else {
            self.last = t;
        }
        self.last
    }
}

/// Two Escapes at most this far apart (seconds) are Panic (D201).
const DOUBLE_ESCAPE: f64 = 0.5;

// The embedding host shares the editor between its own thread and the
// window's (egui-baseview runs a window on its own thread on X11), so the
// editor must stay `Send`; this fails to compile if a field stops being.
// A browser page has one thread, and its devices aren't `Send`.
#[cfg(not(target_arch = "wasm32"))]
const _: () = {
    const fn sendable<T: Send>() {}
    sendable::<EditorApp>();
};

impl EditorApp {
    /// The desktop editor, styling `ctx` (the window's context) now.
    pub fn new(ctx: &egui::Context, opts: AppOptions) -> EditorApp {
        theme::apply(ctx);
        let app = EditorApp::embedded(opts);
        app.apply_settings(ctx);
        app
    }

    /// Turns on the update check and crash reports (the desktop editor;
    /// a plugin's host and the browser have their own): offers unseen
    /// crash reports, and checks for a newer release if it's time.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_product(&mut self, ctx: &egui::Context, opts: crate::product::ProductOptions) {
        self.product = Some(crate::product::Product::start(
            opts,
            &mut self.settings,
            ctx,
        ));
    }

    /// The update check and crash reports, when turned on.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn product(&self) -> Option<&crate::product::Product> {
        self.product.as_ref()
    }

    /// An editor made before anything shows it: a plugin's, which lives as
    /// long as the plugin while its window opens and closes. Each context
    /// that shows it ([`EditorApp::ui`]) gets the editor's look on its
    /// first frame. With `AudioMode::Hosted` the host runs the processor
    /// ([`EditorApp::take_processor`]) and the editor opens no devices.
    pub fn embedded(opts: AppOptions) -> EditorApp {
        // Messages made while starting up are in the person's language too.
        let language = opts.settings.prefs.language.resolve();
        i18n::set_language(language);
        let registry = Arc::new(ac_nodes::registry());
        let mut library = match &opts.library {
            Some(dir) => Library::open(dir),
            None => Library::shipped_only(),
        };
        let mut status = None;
        // A pack or a block file given at startup opens once the editor is
        // up, the way File ▸ Open does.
        let (first, later) = match opts.project.clone() {
            Some(p) if !is_project_file(&p) => (None, Some(p)),
            p => (p, None),
        };
        let (project, path, base, assets) = match &first {
            Some(p) => match files::open_project(p, &registry) {
                Ok((proj, assets)) => (proj, Some(p.clone()), files::project_dir(p), assets),
                Err(e) => {
                    status = Some((
                        tr_args(
                            "status-couldnt-open-path",
                            &[
                                ("path", p.display().to_string().into()),
                                ("error", e.into()),
                            ],
                        ),
                        0.0,
                        true,
                    ));
                    let proj = files::default_project(&registry);
                    (proj, None, cwd(), AssetStore::default())
                }
            },
            None => match files::find_start_example()
                .filter(|_| opts.start_example && later.is_none())
            {
                Some(p) => match files::open_project(&p, &registry) {
                    // The example opens as an untitled copy, so saving
                    // doesn't overwrite the shipped file. Its stored view
                    // was set on someone else's window: frame it to this one.
                    Ok((mut proj, assets)) => {
                        proj.view = None;
                        (proj, None, files::project_dir(&p), assets)
                    }
                    Err(_) => (
                        files::default_project(&registry),
                        None,
                        cwd(),
                        AssetStore::default(),
                    ),
                },
                None => (
                    files::default_project(&registry),
                    None,
                    cwd(),
                    AssetStore::default(),
                ),
            },
        };
        let recorder = path
            .as_ref()
            .filter(|_| !opts.host_files)
            .and_then(|p| ac_io::DiskRecorder::new(&files::project_dir(p)).ok())
            .map(Arc::new);
        let plugins = opts.plugins.clone().map(crate::plugins::Plugins::new);
        let resources = Arc::new(FileResources {
            assets: assets.clone(),
            recorder: recorder
                .clone()
                .map(|r| r as Arc<dyn ac_io::RecorderFactory>),
            plugins: plugins.as_ref().map(crate::plugins::Plugins::loader),
        });
        let engine = EngineHost::start(&project, registry.clone(), resources, &opts.audio);
        if path.is_some() {
            library.import_unknown(&project);
        }
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut online = Online::with_libraries(&opts.settings.prefs.libraries);
        // A library kept on disk means a real session: downloads go where
        // the person keeps them and previews are cached. In memory (tests),
        // nothing outside the session is written; a browser page has neither
        // folder.
        #[cfg(not(target_arch = "wasm32"))]
        if opts.library.is_some() {
            online.cache_dir = ac_io::library::preview_cache_dir();
            online.download_dir = crate::online::default_download_dir();
        }
        let doc = Document::new(project, path.clone(), base, opts.seed);
        let mut recent = opts.recent;
        if let Some(p) = &path {
            recent.add(p);
        }
        let mut app = EditorApp {
            loaded_assets: ac_io::assets::referenced_assets(&doc.project, &registry),
            registry,
            doc,
            canvas: Canvas::default(),
            palette: Palette::default(),
            engine,
            side: SidePanel::default(),
            perform: PerformView::default(),
            recent,
            library,
            missing: missing_of(&assets),
            assets,
            transport: TransportState::default(),
            ruler: crate::ruler::Ruler::default(),
            status,
            confirm: None,
            prompt: None,
            native_dialogs: opts.native_dialogs,
            recorder,
            takes: Vec::new(),
            export_job: None,
            show_timing: false,
            timing: crate::timing::Timings::default(),
            show_shortcuts: false,
            show_about: false,
            #[cfg(not(target_arch = "wasm32"))]
            product: None,
            title: String::new(),
            allow_close: false,
            loop_sent: None,
            space_shift: false,
            escape_at: None,
            sounding: Default::default(),
            heard: Vec::new(),
            close_now: false,
            clipboard: None,
            pack_after_save: false,
            last_added: Vec::new(),
            learn: None,
            bound: crate::bindings::BoundInputs::default(),
            play_keys: PlayKeys::default(),
            play_keys_before_performance: None,
            find: FindBar::default(),
            selection_order: SelectionOrder::default(),
            palette_feed: Vec::new(),
            themes: UserThemes::load(opts.settings.themes_dir()),
            language,
            settings: opts.settings,
            clock: Clock::default(),
            epoch: 0,
            host_seen: (0, 0, 0),
            host_files: opts.host_files,
            file_requests: Vec::new(),
            asset_bank: Default::default(),
            picked: None,
            after_host_save: None,
            host_controls: None,
            host_requests: Vec::new(),
            online,
            plugins,
        };
        if let Some(p) = &app.plugins {
            app.palette.plugins = p.list().to_vec();
        }
        // A plugin window can't go full screen: performance mode fills
        // the window instead.
        app.perform.fills_window = app.is_hosted();
        app.canvas.reset_for(&app.doc, &app.registry);
        if let Some(p) = later {
            app.open(&p, 0.0);
        }
        app.report_library_issues();
        app.report_theme_issues();
        app.host_seen = app.change_stamp();
        app
    }

    /// Theme files that couldn't be used, and a saved theme that's gone,
    /// said once at startup (after anything else worth saying).
    fn report_theme_issues(&mut self) {
        let mut text = Vec::new();
        if let (_, Some(missing)) = self.themes.look(&self.settings.prefs.theme) {
            text.push(tr_args("status-theme-missing", &[("name", missing.into())]));
        }
        if let Some(first) = self.themes.problems.first() {
            let n = self.themes.problems.len();
            text.push(tr_args(
                "status-theme-files-unusable",
                &[("count", n.into()), ("first", first.into())],
            ));
        }
        if text.is_empty() {
            return;
        }
        let text = text.join(" · ");
        match &mut self.status {
            Some((s, _, warn)) => {
                s.push_str(" · ");
                s.push_str(&text);
                *warn = true;
            }
            None => self.status = Some((text, 0.0, true)),
        }
    }

    /// Block files in the library folder that couldn't be read, said once
    /// at startup (after anything else worth saying).
    fn report_library_issues(&mut self) {
        let Some(first) = self.library.issues.first() else {
            return;
        };
        let n = self.library.issues.len();
        let text = tr_args(
            "status-block-files-unreadable",
            &[("count", n.into()), ("first", first.to_string().into())],
        );
        match &mut self.status {
            Some((s, _, warn)) => {
                s.push_str(" · ");
                s.push_str(&text);
                *warn = true;
            }
            None => self.status = Some((text, 0.0, true)),
        }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The audio files the project plays, as loaded (a host running the
    /// engine elsewhere sends them along with the project).
    pub fn assets(&self) -> &AssetStore {
        &self.assets
    }

    // ------------------------------------------------------------ hosting

    /// Whether a host runs the audio (a plugin): the editor opens no
    /// devices, has no Quit, and leaves the window's title and size to the
    /// host.
    pub fn is_hosted(&self) -> bool {
        self.engine.hosted().is_some()
    }

    /// The engine's audio-thread half, for the host to run
    /// (`AudioMode::Hosted`). `None` once taken; after the host's rate or
    /// block size changes, [`EditorApp::restart_engine`] hands out the next.
    pub fn take_processor(&mut self) -> Option<Processor> {
        self.engine.take_processor()
    }

    /// Play keys' control events, for the host's audio thread to drain
    /// into each block's `HostIo::controls`. `None` once taken.
    pub fn take_control_receiver(&mut self) -> Option<ac_io::ControlReceiver> {
        self.engine.take_control_receiver()
    }

    /// The host now runs at `audio` (a plugin activated, or reactivated at
    /// another rate or block size): build the engine for the document there
    /// and return the processor for the host's audio thread. The transport
    /// and node state start over. `None` unless hosted.
    pub fn restart_engine(&mut self, audio: HostAudio) -> Option<Processor> {
        let resources = self.resources();
        let processor = self.engine.restart_hosted(
            audio,
            &self.doc.project,
            self.registry.clone(),
            resources,
        )?;
        self.engine.synced_revision = self.doc.engine_revision;
        self.loop_sent = None;
        Some(processor)
    }

    /// Run an engine the host built, whose processor the host already runs
    /// at `audio`'s rate. The editor sends it the document and its audio
    /// files at once, so the host can build it from any project (usually
    /// [`EditorApp::project`]).
    pub fn attach_engine(&mut self, handle: EngineHandle, audio: HostAudio) {
        self.engine.attach_hosted(handle, audio);
        self.perform.fills_window = true;
        let resources = self.resources();
        self.engine.set_resources(&self.doc.project, resources);
        self.engine.synced_revision = self.doc.engine_revision;
        self.loop_sent = None;
    }

    /// The project as it stands, with the canvas's view, for the host to
    /// keep (a plugin's saved state).
    pub fn project(&self) -> Project {
        let mut project = self.doc.project.clone();
        project.view = Some(ac_core::View {
            center: [self.canvas.view.center.x, self.canvas.view.center.y],
            zoom: self.canvas.view.zoom,
        });
        project
    }

    /// Where the project's file is, if it has one. A host keeping the
    /// project in its own state keeps this too, so audio files the project
    /// refers to by relative path resolve when it's restored.
    pub fn project_path(&self) -> Option<&Path> {
        self.doc.path.as_deref()
    }

    /// Whether the project changed since the host last asked or last loaded
    /// one: an edit, an undo or redo, New, Open, or Save. Moving the view
    /// alone isn't a change, though [`EditorApp::project`] carries it.
    pub fn has_changes(&self) -> bool {
        self.change_stamp() != self.host_seen
    }

    /// The project, once per change (see [`EditorApp::has_changes`]). A
    /// plugin marks its state dirty on `Some`.
    pub fn take_changes(&mut self) -> Option<Project> {
        let stamp = self.change_stamp();
        if stamp == self.host_seen {
            return None;
        }
        self.host_seen = stamp;
        Some(self.project())
    }

    /// Replace the document with `project` (a host restoring its state):
    /// no unsaved-changes question, a fresh undo history, the transport at
    /// zero, the engine rebuilt. `path` is the project's file, if it has
    /// one, so relative audio file paths resolve beside it; without one the
    /// document is untitled. Unknown nodes and missing files load as
    /// placeholders and problems, as opening a file does. The host isn't
    /// told about this change by [`EditorApp::take_changes`].
    pub fn load_project(&mut self, project: Project, path: Option<&Path>) {
        let base = path.map_or_else(cwd, files::project_dir);
        let assets = if self.host_files {
            self.bank_assets_for(&project)
        } else {
            AssetStore::load_for(&project, &base, &self.registry)
        };
        self.load_project_with(project, path, assets);
    }

    /// [`EditorApp::load_project`] with the audio files the host holds
    /// (a plugin keeps them in its saved state, not on disk). Files the
    /// project refers to and `assets` lacks show as missing.
    pub fn load_project_with(&mut self, project: Project, path: Option<&Path>, assets: AssetStore) {
        let base = path.map_or_else(cwd, files::project_dir);
        let assets = with_missing(assets, &project, &self.registry);
        // Questions about the document being replaced no longer apply.
        self.confirm = None;
        self.prompt = None;
        self.replace_document(project, path.map(Path::to_path_buf), base, assets);
        // The host has this project already.
        let _ = self.engine.take_outgoing();
        self.host_seen = self.change_stamp();
    }

    /// The host keeps the project and the engine from now on (a plugin's
    /// session; `AudioMode::Hosted` only). Each frame it lends the editor
    /// its engine handle ([`EditorApp::with_engine`]); edits come back
    /// through [`EditorApp::take_engine_edit`] and
    /// [`EditorApp::take_changes`]; New, Open, and picked audio files
    /// become [`HostRequest`]s; the host runs the transport and the
    /// engine's upkeep.
    pub fn host_keeps_project(&mut self) {
        self.engine.keep_in_host();
    }

    /// Show the host's automatable parameters (a plugin's macros) in the
    /// Devices tab, moved through `controls` so the host records them.
    pub fn set_host_controls(&mut self, controls: Box<dyn HostControls>) {
        self.host_controls = Some(controls);
    }

    /// Run `f` with the host's engine `handle` as the editor's engine (a
    /// frame, or upkeep), then give it back, even if `f` panics.
    pub fn with_engine<R>(
        &mut self,
        handle: &mut EngineHandle,
        f: impl FnOnce(&mut EditorApp) -> R,
    ) -> R {
        self.engine.swap_handle(handle, true);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.engine.swap_handle(handle, false);
        match r {
            Ok(r) => r,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// The project the engine should run, if the document changed in a way
    /// the engine hears since the last call (while the host keeps the
    /// project). The host compiles it and says how that went with
    /// [`EditorApp::host_compiled`].
    pub fn take_engine_edit(&mut self) -> Option<Project> {
        self.engine.take_outgoing()
    }

    /// What compiling the last engine edit found (the Problems tab shows
    /// it). `None` when the host had no engine to compile with; the editor
    /// compiles on its own stand-in then.
    pub fn host_compiled(&mut self, report: Option<ac_engine::CompileReport>) {
        self.engine.host_compiled(report, &self.doc.project);
    }

    /// What the editor asked of the host since the last call.
    pub fn take_host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host_requests)
    }

    /// The audio files the host holds changed (it kept a file the editor
    /// asked it to): show them.
    pub fn set_host_assets(&mut self, assets: AssetStore) {
        self.assets = with_missing(assets, &self.doc.project, &self.registry);
        self.missing = missing_of(&self.assets);
        self.loaded_assets = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
    }

    /// Say something in the status bar (a host reporting what a load found).
    pub fn tell(&mut self, text: impl Into<String>, warning: bool) {
        let now = self.clock.last;
        if warning {
            self.warn(text, now);
        } else {
            self.say(text, now);
        }
    }

    fn in_host(&self) -> bool {
        self.engine.in_host()
    }

    /// Engine upkeep while nothing shows the editor (a plugin whose window
    /// is closed): edits reach the engine, retired graphs are freed,
    /// whole-input nodes bake, and recorded takes are picked up. Every
    /// frame shown does this too; a host calls it now and then otherwise.
    pub fn upkeep(&mut self) {
        let now = self.clock.last;
        self.sync_engine(now);
    }

    fn change_stamp(&self) -> ChangeStamp {
        (self.epoch, self.doc.revision, self.doc.engine_revision)
    }

    /// Whether the editor is in full-screen performance mode (a panel and a
    /// minimal transport, nothing else). Control surfaces that take over
    /// the computer keyboard can follow this.
    pub fn performance_mode(&self) -> bool {
        self.perform.fullscreen
    }

    /// Whether the panels are shown instead of the canvas.
    pub fn panels_shown(&self) -> bool {
        self.perform.shown
    }

    /// Whether Play keys is on: computer keys go to the patch as control
    /// events (`Control::Key`) instead of running canvas shortcuts.
    pub fn play_keys(&self) -> bool {
        self.play_keys.is_on()
    }

    /// Turn Play keys on or off (the transport bar's keyboard button and
    /// Transport ▸ Play Keys do; a performance view can too). Turning it off
    /// releases every key held in the patch.
    pub fn set_play_keys(&mut self, on: bool) {
        // Keys a host doesn't pass on would only stop the shortcuts working.
        let on = on && self.engine.keys_reach_engine();
        self.play_keys.set(on, &mut self.engine);
    }

    /// Play keys follows performance mode: on when it starts, and back to
    /// what it was when it ends (it can still be turned off meanwhile).
    fn follow_performance_mode(&mut self) {
        match (self.perform.fullscreen, self.play_keys_before_performance) {
            (true, None) => {
                self.play_keys_before_performance = Some(self.play_keys.is_on());
                self.set_play_keys(true);
            }
            (false, Some(was)) => {
                self.play_keys_before_performance = None;
                self.set_play_keys(was);
            }
            _ => {}
        }
    }

    /// The status bar's message, if one is showing (tests read this).
    /// The Audio Out nodes the status bar says are sounding.
    pub fn sounding(&self) -> &[crate::sounding::Output] {
        &self.heard
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().map(|(text, _, _)| text.as_str())
    }

    /// The editor's settings (theme and UI scale).
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Show the settings' theme and scale in `ctx`.
    fn apply_settings(&self, ctx: &egui::Context) {
        let (look, _) = self.themes.look(&self.settings.prefs.theme);
        theme::set_look(ctx, look);
        self.apply_motion(ctx);
        ctx.set_zoom_factor(self.settings.prefs.ui_scale);
    }

    /// egui's own animations (collapsing sections, scrolling, menus) off
    /// with Reduce Motion, at egui's usual speed otherwise.
    fn apply_motion(&self, ctx: &egui::Context) {
        let time = if self.settings.prefs.reduce_motion {
            0.0
        } else {
            egui::Style::default().animation_time
        };
        ctx.all_styles_mut(|s| s.animation_time = time);
    }

    /// View ▸ Reduce Motion: no fades, eases, flowing wires, or flashes,
    /// from now on, and remembered.
    pub fn set_reduce_motion(&mut self, ctx: &egui::Context, on: bool, now: f64) {
        self.settings.set_reduce_motion(on);
        self.apply_motion(ctx);
        self.say(
            if on {
                tr("status-reduce-motion-on")
            } else {
                tr("status-reduce-motion-off")
            },
            now,
        );
    }

    /// F6: the keyboard moves between the canvas and the side panel (from
    /// the panel, Tab goes on through its widgets as usual), or into the
    /// panels' cards and out again.
    fn switch_region(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|m| m.focused());
        if self.perform.shown {
            // The panels: into the first card, or back out of them.
            match focused {
                Some(id) => ctx.memory_mut(|m| m.surrender_focus(id)),
                None => self.perform.take_keyboard(ctx),
            }
            return;
        }
        let on_canvas = self.canvas.owns_focus(ctx) || focused.is_none();
        if on_canvas && self.side.open && !self.perform.shown {
            if let Some(id) = self.side.focus_target {
                self.canvas.drop_keyboard(ctx);
                ctx.memory_mut(|m| m.request_focus(id));
            }
            return;
        }
        if let Some(id) = focused {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        if self.canvas.focus.is_none() {
            self.canvas.key(crate::canvas::KeyCmd::Step(1));
        }
        self.canvas.take_keyboard();
    }

    /// A key the canvas handles itself (keys.rs), if this is one.
    fn canvas_key(&self, key: Key, m: egui::Modifiers) -> Option<crate::canvas::KeyCmd> {
        use crate::canvas::{KeyCmd, NUDGE};
        let plain = !m.any();
        let step = if m.shift {
            NUDGE * 5.0
        } else if m.command || m.ctrl {
            1.0
        } else {
            NUDGE
        };
        let arrow = |d: egui::Vec2| (!m.alt).then_some(KeyCmd::Nudge(d * step));
        match key {
            Key::Tab if plain => Some(KeyCmd::Step(1)),
            Key::Tab if m.shift_only() => Some(KeyCmd::Step(-1)),
            Key::Enter if plain => Some(KeyCmd::Activate),
            Key::Enter if m.shift_only() => Some(KeyCmd::Extend),
            Key::ArrowDown if m.alt && !m.shift && !m.command => Some(KeyCmd::SocketStep(1)),
            Key::ArrowUp if m.alt && !m.shift && !m.command => Some(KeyCmd::SocketStep(-1)),
            Key::ArrowLeft => arrow(vec2(-1.0, 0.0)),
            Key::ArrowRight => arrow(vec2(1.0, 0.0)),
            Key::ArrowUp => arrow(vec2(0.0, -1.0)),
            Key::ArrowDown => arrow(vec2(0.0, 1.0)),
            Key::F10 if m.shift_only() => Some(KeyCmd::Menu),
            Key::F2 if plain && self.canvas.socket_focused() => Some(KeyCmd::EditValue),
            _ => None,
        }
    }

    /// The person's own themes, as View ▸ Theme lists them.
    pub fn user_themes(&self) -> &UserThemes {
        &self.themes
    }

    /// View ▸ Theme: switch now and remember the choice. A theme file
    /// whose text is hard to read says so.
    pub fn set_theme(&mut self, ctx: &egui::Context, choice: ThemeChoice) {
        let now = self.clock.last;
        self.settings.set_theme(choice.clone());
        let (look, missing) = self.themes.look(&choice);
        theme::set_look(ctx, look);
        ctx.request_repaint();
        if let Some(name) = missing {
            self.warn(
                tr_args("status-theme-missing", &[("name", name.into())]),
                now,
            );
        } else if let ThemeChoice::User(name) = &choice
            && let Some(t) = self.themes.get(name)
        {
            if t.low_contrast() {
                self.warn(
                    tr_args(
                        "status-theme-low-contrast",
                        &[
                            ("name", name.into()),
                            ("ratio", format!("{:.1}", t.text_contrast.0).into()),
                            ("where", (&t.text_contrast.1).into()),
                        ],
                    ),
                    now,
                );
            } else if let Some(note) = t.notes.first() {
                self.warn(format!("{}: {note}", t.file), now);
            }
        }
    }

    /// View ▸ Language: the menus and messages switch from the next
    /// frame (this one is half drawn), and the choice is remembered.
    pub fn set_language(&mut self, choice: LanguageChoice, now: f64) {
        self.settings.set_language(choice);
        self.language = choice.resolve();
        i18n::set_language(self.language);
        self.say(
            tr_args("status-language", &[("language", choice.label().into())]),
            now,
        );
    }

    /// The language the editor shows.
    pub fn language(&self) -> Language {
        self.language
    }

    /// View ▸ Theme ▸ Reload Themes: read the themes folder again, and
    /// show the chosen theme as its file now says.
    pub fn reload_themes(&mut self, ctx: &egui::Context, now: f64) {
        self.themes = UserThemes::load(self.settings.themes_dir());
        self.apply_settings(ctx);
        let n = self.themes.themes.len();
        let mut text = tr_args("status-themes-loaded", &[("count", n.into())]);
        if let Some(first) = self.themes.problems.first() {
            let bad = self.themes.problems.len();
            text.push_str(" · ");
            text.push_str(&tr_args(
                "status-theme-files-unusable",
                &[("count", bad.into()), ("first", first.into())],
            ));
            self.warn(text, now);
        } else {
            self.say(text, now);
        }
    }

    /// View ▸ Theme ▸ Open Themes Folder.
    fn open_themes_folder(&mut self, now: f64) {
        let Some(dir) = self.settings.themes_dir() else {
            self.warn(tr("status-no-themes-folder"), now);
            return;
        };
        match files::open_folder(&dir) {
            Ok(()) => self.say(
                tr_args(
                    "status-themes-folder",
                    &[("path", dir.display().to_string().into())],
                ),
                now,
            ),
            Err(e) => self.warn(
                tr_args("status-couldnt-open-themes-folder", &[("error", e.into())]),
                now,
            ),
        }
    }

    /// View ▸ UI Scale: everything (canvas text included) scales by
    /// `scale` from the next frame, and the choice is remembered.
    pub fn set_ui_scale(&mut self, ctx: &egui::Context, scale: f32, now: f64) {
        self.settings.set_ui_scale(scale);
        let scale = self.settings.prefs.ui_scale;
        ctx.set_zoom_factor(scale);
        self.say(
            tr_args(
                "status-ui-scale",
                &[("percent", format!("{:.0}", scale * 100.0).into())],
            ),
            now,
        );
    }

    fn say(&mut self, text: impl Into<String>, now: f64) {
        self.status = Some((text.into(), now, false));
    }

    /// Panic (D201): let go of everything the editor holds down in the
    /// patch, then stop, reset every node, and mute the outputs for a block.
    fn panic(&mut self, now: f64) {
        self.play_keys.release_all(&mut self.engine);
        self.perform.let_go(&mut self.doc, &mut self.engine.handle);
        self.engine.handle.panic();
        self.say(tr("status-panic"), now);
    }

    fn warn(&mut self, text: impl Into<String>, now: f64) {
        self.status = Some((text.into(), now, true));
    }

    // ------------------------------------------------------------ frame

    /// Draw the whole editor into `ui`, the root of a frame: the desktop
    /// window's (eframe), a plugin window's (`embed`), or a test harness's.
    /// Every host calls this once a frame.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = self.clock.now(ctx.input(|i| i.time));
        // The thread may be new (a plugin window's): speak the editor's
        // language on it.
        i18n::set_language(self.language);
        // The app may be drawn into a context other than the one it was
        // created with (a test harness); give that one the look too.
        let themed = egui::Id::unique("audio-constructor-theme");
        if !ctx.data(|d| d.get_temp::<bool>(themed).unwrap_or(false)) {
            theme::apply(&ctx);
            self.apply_settings(&ctx);
            ctx.data_mut(|d| d.insert_temp(themed, true));
        }
        // Draw with the theme egui shows (Follow System can change it).
        theme::follow(&ctx);
        // The canvas is keyboard-driven: with nothing focused, Tab and the
        // arrow keys belong to it, not to egui's focus navigation.
        // While a card or socket has the keyboard (D183), the same goes.
        if ctx.memory(|m| m.focused().is_none()) || self.canvas.owns_focus(&ctx) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        self.track_selection();
        self.handle_keys(&ctx, now);
        self.update_bindings(now);
        if self.show_timing {
            let tele = self.engine.handle.telemetry();
            self.timing.update(&tele, &self.doc, now);
            // The numbers move with the audio, not with the pointer.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.learn.is_some() {
            // Keep polling for the control that will bind.
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.perform.follow_window(&ctx);
        self.follow_performance_mode();
        if self.perform.fullscreen {
            self.performance_frame(ui, &ctx, now);
        } else {
            self.chrome(ui, &ctx, now);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::canvas_bg()))
            .show(ui, |ui| {
                if self.perform.shown {
                    self.show_panels(ui, &ctx, now);
                    return;
                }
                let calm = self.engine.handle.status().load_fraction() > 0.8;
                let can_record = self.recorder.is_some();
                let mut io = CanvasIo {
                    can_record,
                    doc: &mut self.doc,
                    registry: &self.registry,
                    engine: &mut self.engine.handle,
                    report: &self.engine.report,
                    missing_assets: &self.missing,
                    calm,
                    timing: self.show_timing.then_some(&self.timing),
                    bound: &self.bound,
                    reduce_motion: self.settings.prefs.reduce_motion,
                    events: Vec::new(),
                };
                self.canvas.show(ui, &mut io);
                let events = std::mem::take(&mut io.events);
                for e in events {
                    self.canvas_event(&ctx, e, now);
                }
            });
        if !self.perform.shown {
            self.library_drop(&ctx, now);
        }

        if let Some(p) = self
            .palette
            .show(&ctx, &self.registry, &self.library, &self.doc)
        {
            let feed = std::mem::take(&mut self.palette_feed);
            self.add_node_from(&p.candidate, p.at, p.pending, &feed, now);
        }
        if !self.palette.open {
            self.palette_feed.clear();
        }
        if self.perform.shown {
            self.find.close();
        } else if let Some(f) =
            self.find
                .show(&ctx, self.canvas.screen, &self.doc.project, &self.registry)
        {
            self.jump_to_found(&f, now);
        }
        self.dialogs(&ctx, now);
        // Buttons this frame may have started or ended performance mode.
        self.follow_performance_mode();
        self.finish_frame(&ctx, now);
    }

    /// The menu bar, transport bar, status bar, and side panel.
    fn chrome(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, now: f64) {
        let ctx = ctx.clone();
        egui::Panel::top("menu-bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| {
                let updates = self.update_menu();
                let examples = self.side.help.examples();
                let state = MenuState {
                    undo: self.doc.undo_label(),
                    redo: self.doc.redo_label(),
                    has_selection: !self.doc.selection.is_empty(),
                    side_panel: self.side.open,
                    timing: self.show_timing,
                    minimap: self.canvas.minimap,
                    reduce_motion: self.settings.prefs.reduce_motion,
                    panels: self.perform.shown,
                    playing: self.engine.handle.is_playing(),
                    looping: self.doc.project.settings.loop_region.is_some(),
                    metronome: self.doc.project.settings.metronome,
                    play_keys: self.play_keys.is_on(),
                    theme: &self.settings.prefs.theme,
                    themes: &self.themes,
                    language: self.settings.prefs.language,
                    ui_scale: self.settings.prefs.ui_scale,
                    recent: &self.recent,
                    examples,
                    registry: &self.registry,
                    can_quit: self.engine.hosted().is_none(),
                    updates,
                    show_recent: !self.host_files,
                };
                let cmds = menubar::show(ui, &state);
                for c in cmds {
                    self.menu_command(&ctx, c, now);
                }
            });
        egui::Panel::top("transport")
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .stroke(egui::Stroke::new(1.0, theme::border())),
            )
            .exact_size(44.0)
            .show(ui, |ui| {
                let panels = self.perform.shown;
                let play_keys = self.play_keys.is_on();
                let actions =
                    self.transport
                        .show(ui, &mut self.doc, &self.engine, panels, play_keys, now);
                for a in actions {
                    self.run(&ctx, a, now);
                }
                if let Some(beat) = self.transport.take_seek() {
                    self.seek_beat(beat);
                }
            });
        // A host that keeps the project runs the transport: no ruler to
        // fight it.
        if !self.in_host() {
            egui::Panel::top("ruler")
                .frame(egui::Frame::new().fill(theme::panel_bg()))
                .exact_size(22.0)
                .show(ui, |ui| {
                    let rate = f64::from(self.engine.info.sample_rate);
                    let position = self.engine.handle.position();
                    if let Some(beat) = self.ruler.show(ui, &mut self.doc, rate, position, now) {
                        self.seek_beat(beat);
                    }
                });
        }
        egui::Panel::bottom("status-bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .exact_size(24.0)
            .show(ui, |ui| self.status_bar(ui, now));
        if self.side.open {
            egui::Panel::right("side-panel")
                .resizable(true)
                .default_size(330.0)
                .size_range(240.0..=640.0)
                .frame(
                    egui::Frame::new()
                        .fill(theme::panel_bg())
                        .inner_margin(egui::Margin::symmetric(10, 4)),
                )
                .show(ui, |ui| {
                    let mut io = PanelIo {
                        doc: &mut self.doc,
                        registry: &self.registry,
                        library: &self.library,
                        engine: &mut self.engine,
                        assets: &self.assets,
                        missing: &self.missing,
                        timing: self.show_timing.then_some(&self.timing),
                        plugins: self.plugins.as_ref(),
                        requests: Vec::new(),
                        now,
                        host: self.host_controls.as_mut(),
                        online: &self.online,
                    };
                    self.side.show(ui, &mut io);
                    let reqs = std::mem::take(&mut io.requests);
                    for r in reqs {
                        self.panel_request(&ctx, r, now);
                    }
                });
        }
    }

    /// Performance mode's only chrome: a minimal transport with the way out.
    fn performance_frame(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, now: f64) {
        egui::Panel::top("performance-bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(egui::Stroke::new(1.0, theme::border())),
            )
            .exact_size(52.0)
            .show(ui, |ui| {
                let play_keys = self.play_keys.is_on();
                let actions = self
                    .perform
                    .performance_bar(ui, &self.doc, &self.engine, play_keys);
                for a in actions {
                    self.run(ctx, a, now);
                }
            });
    }

    /// The panel view in place of the canvas. Panels pin top-level nodes,
    /// so it sees the document at the root.
    fn show_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, now: f64) {
        let calm = self.engine.handle.status().load_fraction() > 0.8;
        let can_record = self.recorder.is_some();
        let perform = &mut self.perform;
        let registry = &self.registry;
        let engine = &mut self.engine.handle;
        let report = &self.engine.report;
        let missing = &self.missing;
        let bound = &self.bound;
        let reduce_motion = self.settings.prefs.reduce_motion;
        let events = self.doc.at_root(|doc| {
            let mut io = CanvasIo {
                can_record,
                doc,
                registry,
                engine,
                report,
                missing_assets: missing,
                calm,
                timing: None,
                bound,
                reduce_motion,
                events: Vec::new(),
            };
            perform.show(ui, &mut io, now);
            io.events
        });
        for e in events {
            self.canvas_event(ctx, e, now);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.horizontal_centered(|ui| {
            if let Some(label) = self.learn.as_ref().map(|l| l.label.clone()) {
                // Learning a binding: say what's listening until it binds.
                ui.label(
                    RichText::new(format!(
                        "{}  {}",
                        icons::LINK,
                        tr_args("status-learning", &[("target", label.into())])
                    ))
                    .small()
                    .color(theme::accent()),
                );
                if ui.small_button(tr("status-cancel")).clicked() {
                    self.cancel_learn(now);
                }
            // A recorded take waits here until it's made into a Clip.
            } else if let Some(take) = self.takes.first().cloned() {
                ui.label(
                    RichText::new(format!(
                        "{}  {}",
                        icons::RECORD,
                        tr_args("status-recorded", &[("file", (&take.relative_path).into())])
                    ))
                    .small()
                    .color(theme::text()),
                );
                if ui.small_button(tr("status-clip-from-take")).clicked() {
                    self.clip_from_take(&take);
                }
                if icons::spoken(ui.small_button(icons::X))
                    .on_hover_text(tr("status-dismiss"))
                    .accessible_name(tr("status-dismiss"))
                    .clicked()
                    && !self.takes.is_empty()
                {
                    self.takes.remove(0);
                }
                if self.takes.len() > 1 {
                    ui.label(
                        RichText::new(tr_args(
                            "status-more-takes",
                            &[("count", (self.takes.len() - 1).into())],
                        ))
                        .small()
                        .color(theme::text_muted()),
                    );
                }
            } else if let Some((text, t, warn)) = &self.status {
                if now - t < 8.0 || *t == 0.0 {
                    let color = if *warn {
                        theme::warning()
                    } else {
                        theme::text()
                    };
                    let icon = if *warn { icons::WARNING } else { icons::INFO };
                    ui.label(
                        RichText::new(format!("{icon}  {text}"))
                            .small()
                            .color(color),
                    );
                } else {
                    self.status = None;
                }
            } else if let Some((p, _)) = &self.export_job {
                ui.spinner();
                ui.label(
                    RichText::new(tr_args(
                        "status-exporting-to",
                        &[("path", p.display().to_string().into())],
                    ))
                    .small(),
                );
            } else if let Some(link) = self.online.importing.first() {
                ui.spinner();
                ui.label(
                    RichText::new(tr_args("status-downloading", &[("link", link.into())])).small(),
                );
            } else if self.online.exporting > 0 {
                ui.spinner();
                ui.label(RichText::new(tr("status-exporting-block")).small());
            } else if self.play_keys.is_on() {
                let stop = Action::PlayKeys
                    .shortcut()
                    .map(|s| ui.ctx().format_shortcut(&s))
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "{}  {}",
                        icons::KEYBOARD,
                        tr_args("status-play-keys-on", &[("stop", stop.into())])
                    ))
                    .small()
                    .color(theme::accent()),
                );
            } else {
                let hint = if self.perform.shown {
                    tr("hint-panels")
                } else if self.doc.selection.is_empty() {
                    tr("hint-nothing-selected")
                } else if self.doc.selection.len() >= 2 {
                    tr("hint-several-selected")
                } else {
                    tr("hint-one-selected")
                };
                ui.label(RichText::new(hint).small().color(theme::text_faint()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = if self.engine.is_running() {
                    icons::SPEAKER_HIGH
                } else {
                    icons::SPEAKER_SLASH
                };
                let text = format!("{icon} {}", self.engine.describe());
                let r = icons::spoken(
                    ui.add(
                        egui::Button::new(RichText::new(text).small().color(theme::text_muted()))
                            .frame(false),
                    ),
                );
                if r.on_hover_text(tr("status-audio-settings")).clicked() {
                    self.side.open = true;
                    self.side.tab = Tab::Devices;
                }
                let n = self.engine.report.diagnostics.len();
                if n > 0 {
                    let errors = self.engine.report.has_errors();
                    let r = icons::spoken(
                        ui.add(
                            egui::Button::new(
                                RichText::new(format!("{} {n}", icons::WARNING))
                                    .small()
                                    .color(if errors {
                                        theme::error()
                                    } else {
                                        theme::warning()
                                    }),
                            )
                            .frame(false),
                        ),
                    )
                    .on_hover_text(tr_args("status-problems", &[("count", n.into())]))
                    // The button shows a count; it says what's counted.
                    .accessible_name(tr_args("status-problems", &[("count", n.into())]));
                    if r.clicked() {
                        self.side.open = true;
                        self.side.tab = Tab::Problems;
                    }
                }
                self.sounding_readout(ui, now);
            });
        });
    }

    /// The outputs that are sounding (D201), named, in the status bar;
    /// clicking shows the first on the canvas.
    fn sounding_readout(&mut self, ui: &mut egui::Ui, now: f64) {
        let tele = self.engine.handle.telemetry();
        let heard = self
            .sounding
            .update(&self.doc.project, &self.registry, &tele, now);
        self.heard = heard.clone();
        let Some(first) = heard.first().cloned() else {
            return;
        };
        // Keep looking while something sounds, so it goes when it stops.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        let names: Vec<&str> = heard.iter().map(|o| o.name.as_str()).collect();
        let list = names.join(" · ");
        let text = tr_args("status-sounding", &[("outputs", (&list).into())]);
        let r = icons::spoken(
            ui.add(
                egui::Button::new(
                    RichText::new(format!("{}  {text}", icons::SPEAKER_HIGH))
                        .small()
                        .color(theme::text_muted()),
                )
                .frame(false)
                .truncate(),
            ),
        )
        .on_hover_text(tr("status-sounding-tip"));
        if r.clicked() {
            self.set_view(false);
            if let Some(id) = self.doc.navigate_to(&first.path) {
                self.canvas.jump_to(&self.doc, &self.registry, id);
            }
        }
    }

    /// The engine runs what the document says, and its upkeep is done.
    fn sync_engine(&mut self, now: f64) {
        if self.doc.engine_revision != self.engine.synced_revision {
            self.engine
                .replace(&self.doc.project, self.doc.engine_revision);
            let refs = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
            if refs != self.loaded_assets {
                self.reload_assets();
            }
        }
        self.engine.tick();
        self.serve_plugins(now);
        self.poll_takes(now);
        self.sync_loop();
    }

    fn finish_frame(&mut self, ctx: &egui::Context, now: f64) {
        self.sync_engine(now);
        self.poll_online(ctx, now);
        // Plugins need their main-thread callbacks and timers answered even
        // while nothing on screen changes; an open plugin window, often.
        if let Some(p) = &self.plugins {
            let running = crate::plugins::running();
            if p.scanning() || !running.is_empty() {
                let open = running.iter().any(|k| crate::plugins::window_open(k));
                let ms = if open { 16 } else { 100 };
                ctx.request_repaint_after(std::time::Duration::from_millis(ms));
            }
        }

        if let Some((path, rx)) = &self.export_job
            && let Ok(r) = rx.try_recv()
        {
            let path = path.clone();
            self.export_job = None;
            match r {
                Ok(s) => {
                    let secs = s.frames as f64 / f64::from(s.sample_rate.max(1));
                    self.say(
                        tr_args(
                            "status-exported",
                            &[
                                ("seconds", format!("{secs:.2}").into()),
                                ("channels", s.channels.to_string().into()),
                                ("rate", s.sample_rate.to_string().into()),
                                ("path", path.display().to_string().into()),
                            ],
                        ),
                        now,
                    );
                }
                Err(e) => self.warn(tr_args("status-export-failed", &[("error", e.into())]), now),
            }
        }
        if self.export_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        if self.engine.handle.is_playing() {
            ctx.request_repaint();
        }
        // The plugin host owns its window: its title, and when it closes.
        // The document lives on with the plugin, so closing asks nothing.
        if self.is_hosted() {
            return;
        }
        let title = tr_args(
            "app-window-title",
            &[
                ("dirty", if self.doc.is_dirty() { "● " } else { "" }.into()),
                ("project", self.doc.title().into()),
            ],
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.close_now {
            self.close_now = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.doc.is_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(AfterConfirm::Quit);
        }
    }

    /// Keep the engine's loop region, punch range, click, and count-in in
    /// step with the project.
    fn sync_loop(&mut self) {
        // A host that keeps the project runs the transport (and its loop).
        if self.in_host() {
            return;
        }
        let rate = f64::from(self.engine.info.sample_rate);
        let s = &self.doc.project.settings;
        let tempo = TempoMap::new(rate, &s.tempo);
        let span = |r: Option<[f64; 2]>| {
            r.and_then(|[a, b]| {
                let (s, e) = (tempo.sample_at(a), tempo.sample_at(b));
                (e > s).then_some((s, e))
            })
        };
        let want = SongSync {
            looped: span(s.loop_region),
            punch: span(s.punch),
            metronome: s.metronome,
            count_in: s.count_in.min(ac_core::MAX_COUNT_IN),
        };
        let sent = self.loop_sent;
        let h = &mut self.engine.handle;
        if sent.is_none_or(|x| x.looped != want.looped) {
            h.set_loop(want.looped);
        }
        if sent.is_none_or(|x| x.punch != want.punch) {
            h.set_punch(want.punch);
        }
        if sent.is_none_or(|x| x.metronome != want.metronome) {
            h.set_metronome(want.metronome);
        }
        if sent.is_none_or(|x| x.count_in != want.count_in) {
            h.set_count_in(want.count_in);
        }
        self.loop_sent = Some(want);
    }

    /// Move the playhead to song beat `beat`.
    fn seek_beat(&mut self, beat: f64) {
        let rate = f64::from(self.engine.info.sample_rate);
        let tempo = TempoMap::new(rate, &self.doc.project.settings.tempo);
        self.engine.handle.seek(tempo.sample_at(beat.max(0.0)));
    }

    /// The playhead's song beat.
    fn playhead_beat(&self) -> f64 {
        let rate = f64::from(self.engine.info.sample_rate);
        TempoMap::new(rate, &self.doc.project.settings.tempo).beat_at(self.engine.handle.position())
    }

    fn reload_assets(&mut self) {
        self.assets = if self.host_files {
            self.reload_bank_assets()
        } else if self.in_host() {
            // The host holds the files; the editor's copies are what it has.
            with_missing(
                std::mem::take(&mut self.assets),
                &self.doc.project,
                &self.registry,
            )
        } else {
            AssetStore::load_for(&self.doc.project, &self.doc.base_dir, &self.registry)
        };
        self.missing = missing_of(&self.assets);
        self.loaded_assets = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
        // Every node is rebuilt: running plugins carry their state over.
        if let Some(p) = &self.plugins {
            p.keep_states();
        }
        let resources = self.resources();
        self.engine.set_resources(&self.doc.project, resources);
        self.engine.synced_revision = self.doc.engine_revision;
    }

    /// Assets and, for a saved project, the recorder writing into its
    /// assets folder (recreated when the project moves).
    fn resources(&mut self) -> Arc<FileResources> {
        // A host keeping the files has no folder to record into.
        let dir = self
            .doc
            .path
            .as_deref()
            .filter(|_| !self.host_files)
            .map(files::project_dir);
        let current = self.recorder.as_ref().map(|r| r.assets_dir());
        let wanted = dir.as_ref().map(|d| d.join("assets"));
        if current != wanted {
            if let Some(old) = self.recorder.take() {
                old.shutdown();
            }
            self.recorder = dir
                .and_then(|d| ac_io::DiskRecorder::new(&d).ok())
                .map(Arc::new);
        }
        Arc::new(FileResources {
            assets: self.assets.clone(),
            recorder: self
                .recorder
                .clone()
                .map(|r| r as Arc<dyn ac_io::RecorderFactory>),
            plugins: self.plugins.as_ref().map(crate::plugins::Plugins::loader),
        })
    }

    /// Takes the recorder finished: offer to turn them into Clip nodes.
    /// What the plugins need from the main thread each frame: a finished
    /// scan to install, their own callbacks (`ac_host::poll`), parameters
    /// they moved themselves to follow, and restarts to rebuild.
    fn serve_plugins(&mut self, now: f64) {
        let Some(p) = self.plugins.as_mut() else {
            return;
        };
        if p.poll_scan() {
            let n = p.list().len();
            self.palette.plugins = p.list().to_vec();
            let failed = p.failed().len();
            let msg = if failed > 0 {
                tr_args(
                    "status-plugins-found-some-failed",
                    &[("count", n.into()), ("failed", failed.into())],
                )
            } else {
                tr_args("status-plugins-found", &[("count", n.into())])
            };
            self.say(msg, now);
            // Plugins this project uses may have just been found.
            if crate::plugins::uses_plugins(&self.doc.project) {
                self.reload_assets();
            }
        }
        for event in crate::plugins::poll() {
            match event {
                crate::plugins::HostEvent::ParamChanged {
                    node_key,
                    socket,
                    value,
                } => {
                    let value = ac_core::Value::Float(value);
                    // A top-level node's value is the document's; inside a
                    // group the definition is shared, so only that
                    // instance follows.
                    if let Ok(id) = node_key.parse::<NodeId>() {
                        let v = value.clone();
                        self.doc.at_root(|d| d.set_param(id, &socket, v, now));
                    }
                    if let Ok(path) = node_key.parse::<ac_core::NodePath>() {
                        self.engine.handle.set_param(&path, &socket, value);
                    }
                }
                crate::plugins::HostEvent::Restart { node_key } => {
                    if let Some(p) = self.plugins.as_ref() {
                        p.prepare_restart(&node_key);
                    }
                    self.engine.report = self.engine.handle.rebuild(&node_key);
                }
                crate::plugins::HostEvent::StateChanged { .. } => self.doc.changed(false),
                crate::plugins::HostEvent::WindowClosed { .. } => {}
            }
        }
    }

    /// Open a Plugin node's own window (its node path as the engine knows
    /// it).
    fn show_plugin_window(&mut self, node: NodeId, now: f64) {
        let key = self.doc.node_path(node).to_string();
        if let Err(e) = crate::plugins::show_window(&key) {
            self.warn(
                tr_args("status-plugin-window-failed", &[("error", e.into())]),
                now,
            );
        }
    }

    fn poll_takes(&mut self, now: f64) {
        let Some(r) = &self.recorder else { return };
        let done = r.finished_takes();
        for t in done {
            match &t.error {
                Some(e) => self.warn(
                    tr_args(
                        "status-take-incomplete",
                        &[("file", (&t.relative_path).into()), ("error", e.into())],
                    ),
                    now,
                ),
                None => self.say(
                    tr_args("status-recorded", &[("file", (&t.relative_path).into())]),
                    now,
                ),
            }
            self.takes.push(t);
        }
    }

    /// Add a Clip node playing a recorded take, next to its Record node when
    /// that's in the current graph: placed in the song at the beat the take
    /// started, and wired where the Record's source went (D208).
    pub fn clip_from_take(&mut self, take: &ac_io::Take) {
        let hash = std::fs::read(&take.path)
            .map(|b| ac_core::util::content_hash(&b))
            .unwrap_or_default();
        let prefix = self.doc.scope_prefix();
        let record = take
            .node_key
            .parse::<ac_core::NodePath>()
            .ok()
            .filter(|p| p.0.len() == prefix.len() + 1 && p.0[..prefix.len()] == prefix[..])
            .and_then(|p| p.leaf())
            .filter(|id| self.doc.graph().node(*id).is_some());
        let pos = record
            .and_then(|id| self.doc.graph().node(id))
            .map(|n| [n.ui.pos[0] + theme::NODE_WIDTH + 60.0, n.ui.pos[1]])
            .unwrap_or([self.canvas.view.center.x, self.canvas.view.center.y]);
        let tempo = TempoMap::new(
            f64::from(take.sample_rate),
            &self.doc.project.settings.tempo,
        );
        let beat = take.song_start.map(|s| tempo.beat_at(s));
        let registry = self.registry.clone();
        let clip = self
            .doc
            .edit(&registry, &tr("status-clip-from-take"), |g, ctx| {
                edit::take::clip_from_take(
                    g,
                    ctx,
                    edit::take::TakeClip {
                        config: serde_json::json!({
                            "file": take.relative_path,
                            "channels": take.channels,
                            "hash": hash,
                        }),
                        beat,
                        record,
                        pos,
                    },
                )
            });
        self.doc.selection = [clip].into();
        self.takes.retain(|t| t.path != take.path);
    }

    // ------------------------------------------------------------ input

    fn handle_keys(&mut self, ctx: &egui::Context, now: f64) {
        // A card or socket with the keyboard isn't typing: its keys are the
        // canvas's.
        let typing = (ctx.egui_wants_keyboard_input() && !self.canvas.owns_focus(ctx))
            || self.canvas.is_typing()
            || self.palette.open
            || self.find.open
            || self.perform.is_typing();
        let modal = self.confirm.is_some() || self.prompt.is_some();
        // Play keys: held keys come up when typing starts, a dialog opens,
        // or the window loses focus, so nothing is left down in the patch.
        let focused = ctx.input(|i| i.focused);
        let play = self.play_keys.is_on() && !typing && !modal && focused;
        if self.play_keys.is_on() && !play {
            self.play_keys.release_all(&mut self.engine);
        }
        let events = ctx.input(|i| i.events.clone());
        if self.learn.is_some() && !modal && !ctx.egui_wants_keyboard_input() {
            // Learning takes the keyboard: Esc cancels, any other key is
            // a candidate (and runs no shortcut or Play key, so nothing
            // Play keys held is left down).
            if self.play_keys.is_on() {
                self.play_keys.release_all(&mut self.engine);
            }
            for e in events {
                let Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } = e
                else {
                    continue;
                };
                if key == Key::Escape {
                    self.cancel_learn(now);
                    return;
                }
                let Some(k) = crate::bindings::key_from_egui(key) else {
                    continue;
                };
                let ev = ac_core::InputEvent::Control(ac_core::Control::Key(k), 1.0);
                if let Some(source) = self.learn.as_mut().and_then(|l| l.feed(&[ev])) {
                    self.finish_learn(source, now);
                    return;
                }
            }
            return;
        }
        let mut played = Vec::new();
        for (n, e) in events.into_iter().enumerate() {
            // The way out of performance mode stays with the editor, like
            // Escape.
            let leaves_performance = matches!(
                e,
                Event::Key { key, modifiers, .. }
                    if actions::for_key(key, modifiers) == Some(Action::PerformanceMode)
            );
            if play
                && !leaves_performance
                && let Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } = e
            {
                let input = KeyInput {
                    key,
                    physical: physical_key,
                    pressed,
                    repeat,
                    modifiers,
                };
                if self.play_keys.key(input, &mut self.engine) {
                    played.push((n, key));
                    continue;
                }
            }
            match e {
                Event::Copy if !typing && !modal => self.run(ctx, Action::Copy, now),
                Event::Cut if !typing && !modal => self.run(ctx, Action::DeleteReconnect, now),
                Event::Paste(text) if !typing && !modal => self.paste(&text, now),
                Event::Key {
                    key,
                    pressed,
                    modifiers,
                    repeat,
                    ..
                } => {
                    if modal {
                        continue;
                    }
                    if key == Key::F6 && pressed {
                        self.switch_region(ctx);
                        continue;
                    }
                    if key == Key::Escape && pressed {
                        if self.palette.open || self.find.open {
                            continue; // they handle it
                        }
                        // Escape twice in quick succession is Panic; the
                        // first one still cancels what it would.
                        if !repeat {
                            if self.escape_at.is_some_and(|t| now - t <= DOUBLE_ESCAPE) {
                                self.escape_at = None;
                                self.run(ctx, Action::Panic, now);
                                continue;
                            }
                            self.escape_at = Some(now);
                        }
                        if self.perform.shown {
                            // A field takes its own Escape; then popups and
                            // held controls; then performance mode ends.
                            if !self.perform.is_typing()
                                && !self.perform.cancel(&mut self.doc)
                                && self.perform.fullscreen
                            {
                                self.perform.set_fullscreen(ctx, false);
                            }
                            continue;
                        }
                        if !self.canvas.cancel(&mut self.doc) {
                            // Nothing to cancel: the focus ring goes.
                            self.canvas.drop_keyboard(ctx);
                        }
                        continue;
                    }
                    // Space plays on release, so Space+drag can pan instead.
                    if key == Key::Space && !typing {
                        if pressed && !repeat {
                            self.space_shift = modifiers.shift;
                            if modifiers.shift {
                                self.run(ctx, Action::PlayFromStart, now);
                            }
                        } else if !pressed {
                            let panned = self.canvas.take_space_panned();
                            if !panned && !self.space_shift {
                                self.run(ctx, Action::PlayStop, now);
                            }
                            self.space_shift = false;
                        }
                        continue;
                    }
                    if !pressed {
                        continue;
                    }
                    if !typing
                        && !self.perform.shown
                        && let Some(cmd) = self.canvas_key(key, modifiers)
                    {
                        self.canvas.key(cmd);
                        continue;
                    }
                    let action = match key {
                        Key::Delete | Key::Backspace if !modifiers.any() => Some(Action::Delete),
                        // `+` needs Shift on most layouts; `=` is the same key.
                        // With Ctrl (Cmd), they scale the whole interface.
                        Key::Plus | Key::Equals if modifiers.alt => None,
                        Key::Plus | Key::Equals if modifiers.command || modifiers.ctrl => {
                            Some(Action::UiScaleUp)
                        }
                        Key::Plus | Key::Equals => Some(Action::ZoomIn),
                        _ => actions::for_key(key, modifiers),
                    };
                    let Some(a) = action else { continue };
                    if typing && !a.works_while_typing() {
                        continue;
                    }
                    if self.perform.shown && !a.works_in_panels() {
                        continue;
                    }
                    if repeat
                        && !matches!(
                            a,
                            Action::Undo | Action::Redo | Action::ZoomIn | Action::ZoomOut
                        )
                    {
                        continue;
                    }
                    self.run(ctx, a, now);
                }
                _ => {}
            }
        }
        if !played.is_empty() {
            // Nothing else this frame sees the played keys: no canvas
            // gesture (Space+drag pans), no widget shortcut.
            ctx.input_mut(|i| {
                let mut n = 0;
                i.events.retain(|_| {
                    n += 1;
                    !played.iter().any(|(k, _)| *k == n - 1)
                });
                for (_, key) in &played {
                    i.keys_down.remove(key);
                }
            });
        }
    }

    fn canvas_event(&mut self, ctx: &egui::Context, e: CanvasEvent, now: f64) {
        match e {
            CanvasEvent::OpenPalette {
                screen,
                canvas,
                pending,
            } => {
                let feed = pending.is_none();
                self.find.close();
                self.palette.open_at(screen, canvas, pending);
                if feed {
                    self.prepare_feed();
                }
            }
            CanvasEvent::ChooseClipFile(node) => {
                self.file_dialog(FilePurpose::ChooseClip(node), now)
            }
            CanvasEvent::ShowPluginWindow(node) => self.show_plugin_window(node, now),
            CanvasEvent::ShowHelp(node) => {
                self.doc.selection = [node].into();
                let kind = self.doc.graph().node(node).map(|n| n.kind.clone());
                self.side.show_help(kind.as_deref());
            }
            CanvasEvent::Status(s) => self.warn(s, now),
            CanvasEvent::Run(a) => self.run(ctx, a, now),
            CanvasEvent::Pin(r) => self.pin_request(r, now),
            CanvasEvent::Bind { node, socket } => {
                let target = ac_core::Target::Input {
                    node: self.doc.node_path(node),
                    socket,
                };
                self.start_learn(target);
            }
        }
    }

    // ------------------------------------------------------------ bindings

    /// Learn a binding for `target`: the next control that moves (seen by
    /// the engine, or a key pressed on the canvas) is bound to it.
    pub fn start_learn(&mut self, target: ac_core::Target) {
        use crate::bindings::{Learn, LearnTarget};
        let label = crate::bindings::target_label(&self.doc.project, &self.registry, &target);
        let target = match target {
            ac_core::Target::Input { node, socket } => LearnTarget::Input { node, socket },
            ac_core::Target::Transport { action } => LearnTarget::Transport(action),
        };
        self.engine.handle.set_input_monitor(true);
        self.learn = Some(Learn::new(target, label, &self.doc.project));
        self.status = None;
    }

    /// Stop learning without binding anything.
    pub fn cancel_learn(&mut self, now: f64) {
        if self.learn.take().is_some() {
            self.engine.handle.set_input_monitor(false);
            self.say(tr("status-binding-cancelled"), now);
        }
    }

    /// Bind the learned `source` (one undo step) and stop learning.
    fn finish_learn(&mut self, source: ac_core::Source, now: f64) {
        let Some(learn) = self.learn.take() else {
            return;
        };
        self.engine.handle.set_input_monitor(false);
        let ty = match &learn.target {
            crate::bindings::LearnTarget::Input { node, socket } => self
                .doc
                .project
                .node_at(node)
                .and_then(|n| self.registry.signature_of(n, &self.doc.project.group_defs))
                .and_then(|sig| sig.inputs.into_iter().find(|s| s.name == *socket))
                .map(|s| s.ty),
            crate::bindings::LearnTarget::Transport(_) => None,
        };
        let binding = learn.binding(source.clone(), ty);
        let args = [
            ("source", source.label().into()),
            ("target", (&learn.label).into()),
        ];
        if crate::bindings::add(&mut self.doc, &tr("undo-bind"), binding) {
            self.say(tr_args("status-bound", &args), now);
        } else {
            self.say(tr_args("status-already-bound", &args), now);
        }
    }

    /// Learn bindings from what the engine received, and gather the bound
    /// inputs the canvas draws (with their live values).
    fn update_bindings(&mut self, now: f64) {
        if self.learn.is_some() {
            let events = self.engine.handle.take_input_events();
            if let Some(source) = self.learn.as_mut().and_then(|l| l.feed(&events)) {
                self.finish_learn(source, now);
            }
        }
        self.bound = crate::bindings::BoundInputs::gather(
            &self.doc.project,
            &self.registry,
            &self.engine.handle.binding_values(),
        );
    }

    fn panel_request(&mut self, ctx: &egui::Context, r: PanelRequest, now: f64) {
        match r {
            PanelRequest::Run(a) => self.run(ctx, a, now),
            PanelRequest::ChooseClipFile(node) => {
                self.file_dialog(FilePurpose::ChooseClip(node), now)
            }
            PanelRequest::ShowPluginWindow(node) => self.show_plugin_window(node, now),
            PanelRequest::RescanPlugins(fresh) => {
                if let Some(p) = self.plugins.as_mut() {
                    p.rescan(fresh);
                    self.say(tr("plugins-looking"), now);
                }
            }
            PanelRequest::Locate(path) => self.file_dialog(FilePurpose::Locate(path), now),
            PanelRequest::Add(c) => {
                let at = self.canvas.view.center;
                self.add_candidate(&c, at, None, now);
            }
            PanelRequest::Navigate(path) => {
                if let Some(id) = self.doc.navigate_to(&path) {
                    self.canvas.jump_to(&self.doc, &self.registry, id);
                } else {
                    self.warn(tr("status-node-gone"), now);
                }
            }
            PanelRequest::Status(s) => self.say(s, now),
            PanelRequest::ImportBlockFile => self.file_dialog(FilePurpose::ImportBlock, now),
            PanelRequest::RemoveBlock(source) => self.remove_block(&source, now),
            PanelRequest::SaveBlock(def) => self.save_block(&def, now),
            PanelRequest::UpdateBlock(def) => {
                self.update_block(&def, now);
            }
            PanelRequest::UpdateAllBlocks => {
                self.update_all_blocks(now);
            }
            PanelRequest::Learn(target) => self.start_learn(target),
            PanelRequest::Help(kind) => self.side.show_help(Some(&kind)),
            PanelRequest::OpenExample(path) => self.request(AfterConfirm::OpenExample(path), now),
            PanelRequest::Preview(key) => self.toggle_preview(key, now),
            PanelRequest::ExportBlock(what) => {
                self.file_dialog(FilePurpose::ExportBlock(what), now)
            }
            PanelRequest::RefreshLibrary(link) => self.online.refresh(&link),
            PanelRequest::RemoveLibrary(link) => self.remove_shared_library(&link, now),
            PanelRequest::Install { link, id } => self.online.install(&link, &id),
            PanelRequest::SwitchAudio(config) => {
                if let Some(p) = &self.plugins {
                    p.keep_states();
                }
                let resources = self.resources();
                self.engine
                    .switch(config, &self.doc.project, self.registry.clone(), resources);
                // A rebuilt engine has seen none of the document's revisions.
                self.engine
                    .replace(&self.doc.project, self.doc.engine_revision);
                self.loop_sent = None;
                match &self.engine.device_error {
                    Some(e) => {
                        let e = e.to_string();
                        self.warn(tr_args("status-audio", &[("what", e.into())]), now)
                    }
                    None => {
                        let d = self.engine.describe();
                        self.say(tr_args("status-audio", &[("what", d.into())]), now);
                    }
                }
            }
        }
    }

    fn menu_command(&mut self, ctx: &egui::Context, c: MenuCommand, now: f64) {
        match c {
            MenuCommand::Run(a) => self.run(ctx, a, now),
            MenuCommand::OpenRecent(p) => self.request_open(p, now),
            MenuCommand::ForgetRecent(p) => self.recent.remove(&p),
            MenuCommand::ClearRecent => self.recent.clear(),
            MenuCommand::OpenExample(p) => self.request(AfterConfirm::OpenExample(p), now),
            MenuCommand::SetTheme(t) => self.set_theme(ctx, t),
            MenuCommand::ReloadThemes => self.reload_themes(ctx, now),
            MenuCommand::OpenThemesFolder => self.open_themes_folder(now),
            MenuCommand::SetLanguage(l) => self.set_language(l, now),
            MenuCommand::SetUiScale(s) => self.set_ui_scale(ctx, s, now),
            #[cfg(not(target_arch = "wasm32"))]
            MenuCommand::CheckForUpdates => {
                if let Some(p) = &mut self.product {
                    p.check(ctx, true);
                }
            }
            MenuCommand::SetUpdateChecks(on) => {
                use crate::settings::UpdateChecks;
                let to = if on {
                    UpdateChecks::Daily
                } else {
                    UpdateChecks::Off
                };
                self.settings.set_update_checks(to);
            }
            #[cfg(not(target_arch = "wasm32"))]
            MenuCommand::ReportProblem => {
                if let Some(p) = &mut self.product {
                    p.report_problem(ctx);
                }
            }
            #[cfg(target_arch = "wasm32")]
            MenuCommand::CheckForUpdates | MenuCommand::ReportProblem => {}
            MenuCommand::Add(c) => {
                let at = self.canvas.pointer_canvas();
                let feed = self.feed_order();
                self.add_node_from(&c, at, None, &feed, now);
            }
        }
    }

    /// A node dragged from the Library and released over the canvas.
    fn library_drop(&mut self, ctx: &egui::Context, now: f64) {
        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if !self.canvas.screen.contains(p) {
            return;
        }
        let Some(c) = egui::DragAndDrop::take_payload::<Candidate>(ctx) else {
            return;
        };
        let at = self.canvas.transform().to_canvas(p);
        let wire = self.canvas.wire_under(&self.doc, p);
        let id = self.add_candidate(&c, at, None, now);
        if let (Some(id), Some(w)) = (id, wire) {
            self.doc
                .edit(&self.registry, &tr("undo-insert-on-wire"), |g, ctx| {
                    edit::insert_on_wire(g, ctx, id, &w);
                });
        }
    }

    // ------------------------------------------------------------ commands

    /// Add a node or a library block from the palette or library at canvas
    /// position `at`, completing a pending wire if there is one.
    pub fn add_candidate(
        &mut self,
        c: &Candidate,
        at: egui::Pos2,
        pending: Option<PendingWire>,
        now: f64,
    ) -> Option<NodeId> {
        self.add_node_from(c, at, pending, &[], now)
    }

    /// Follow the selection, so its order is known (selection is a set).
    fn track_selection(&mut self) {
        let prefix = self.doc.scope_prefix();
        self.selection_order
            .sync(&self.doc.selection, self.doc.graph(), &prefix);
    }

    /// The selected nodes in selection order when there are two or more of
    /// them (what connect-selected-to-new-node wires), else nothing.
    fn feed_order(&mut self) -> Vec<NodeId> {
        self.track_selection();
        let order: Vec<NodeId> = self
            .selection_order
            .get()
            .iter()
            .copied()
            .filter(|id| !edit::is_virtual(*id))
            .collect();
        if order.len() >= 2 { order } else { Vec::new() }
    }

    /// The palette just opened without a wire: with several nodes
    /// selected, what it adds is wired from them.
    fn prepare_feed(&mut self) {
        self.palette_feed = self.feed_order();
        let n = self.palette_feed.len();
        self.palette.note = (n >= 2).then(|| tr_args("palette-feed-note", &[("count", n.into())]));
    }

    /// Why the selected nodes weren't wired into a new node, for the
    /// status line.
    fn feed_reason(&self, e: &FeedError) -> String {
        let name = |id: NodeId| {
            self.doc.graph().node(id).map_or_else(
                || tr("status-a-selected-node"),
                |n| connect::title(n, &self.registry, &self.doc.project.group_defs),
            )
        };
        match e {
            FeedError::NoOutput(id) => tr_args("status-no-output", &[("node", name(*id).into())]),
            FeedError::NoInput(id, ty) => tr_args(
                "status-no-free-input",
                &[("type", ty.name().into()), ("node", name(*id).into())],
            ),
        }
    }

    /// Jump to a node the find bar found: into its group, selected and
    /// in view.
    fn jump_to_found(&mut self, f: &Found, now: f64) {
        match self.doc.navigate_to(&f.path) {
            Some(id) => {
                self.canvas.jump_to(&self.doc, &self.registry, id);
                let args = [
                    ("title", (&f.title).into()),
                    ("kind", (&f.kind).into()),
                    ("place", (&f.place).into()),
                ];
                if f.place.is_empty() {
                    self.say(tr_args("status-found", &args), now);
                } else {
                    self.say(tr_args("status-found-in", &args), now);
                }
            }
            None => self.warn(tr("status-node-gone"), now),
        }
    }

    /// [`EditorApp::add_candidate`], wired from `feed` (two or more nodes
    /// in selection order) when there's no pending wire: each feeds the
    /// new node's inputs in turn, and the node goes to their right (D103).
    fn add_node_from(
        &mut self,
        c: &Candidate,
        at: egui::Pos2,
        pending: Option<PendingWire>,
        feed: &[NodeId],
        now: f64,
    ) -> Option<NodeId> {
        let feed: Vec<NodeId> = if pending.is_none() {
            feed.iter()
                .copied()
                .filter(|id| self.doc.graph().node(*id).is_some())
                .collect()
        } else {
            Vec::new()
        };
        let feeding = feed.len() >= 2;
        let bounds = if feeding {
            self.canvas
                .node_rects(&self.doc, &self.registry, &feed)
                .into_iter()
                .map(|(_, r)| r)
                .reduce(|a, b| a.union(b))
        } else {
            None
        };
        let pos = match &pending {
            Some(p) if p.from.is_some() => at - vec2(14.0, 40.0),
            Some(_) => at - vec2(theme::NODE_WIDTH - 14.0, 40.0),
            None => at - vec2(theme::NODE_WIDTH / 2.0, 14.0),
        };
        // A block pastes as an instance of its definition.
        let block = match &c.block {
            Some(source) => match self.block_instance(source) {
                Some(text) => Some(text),
                None => {
                    self.warn(
                        tr_args("status-block-gone", &[("name", (&c.name).into())]),
                        now,
                    );
                    return None;
                }
            },
            None => None,
        };
        let kind = c.kind.clone();
        let config = if feeding && block.is_none() {
            connect::widen_for(&self.registry, &kind, &c.config, feed.len())
        } else {
            c.config.clone()
        };
        let socket = c.socket.clone();
        let mut error = None;
        let mut fed: Option<Result<usize, FeedError>> = None;
        let id = self.doc.edit(
            &self.registry,
            &tr_args("undo-add-named", &[("name", (&c.name).into())]),
            |g, ctx| {
                let id = if let Some(text) = &block {
                    match edit::paste(g, ctx, text, [pos.x, pos.y]) {
                        Ok(ids) => *ids.first()?,
                        Err(e) => {
                            error = Some(e);
                            return None;
                        }
                    }
                } else if kind == ac_core::VOICES_KIND && config.get("def").is_none() {
                    edit::group::add_voices(g, ctx, [pos.x, pos.y])
                } else {
                    edit::add_node(g, ctx, &kind, config, [pos.x, pos.y])
                };
                if let (Some(p), Some(s)) = (&pending, &socket) {
                    let r = match (&p.from, &p.to) {
                        (Some(from), _) => {
                            edit::connect(g, ctx, from.clone(), Endpoint::new(id, s.clone()))
                        }
                        (None, Some(to)) => {
                            edit::connect(g, ctx, Endpoint::new(id, s.clone()), to.clone())
                        }
                        _ => Ok(()),
                    };
                    if let Err(e) = r {
                        error = Some(e.to_string());
                    }
                }
                if feeding {
                    let sources: Vec<Source> = feed
                        .iter()
                        .map(|n| {
                            let s = connect::wirable(g.node(*n), ctx.sockets(g, *n));
                            Source::of(*n, &s)
                        })
                        .collect();
                    let inputs = ctx.sockets(g, id).inputs;
                    fed = Some(connect::plan_feeds(&sources, &inputs).map(|plan| {
                        let wired = plan
                            .into_iter()
                            .filter(|(from, input)| {
                                let to = Endpoint::new(id, input.clone());
                                edit::connect(g, ctx, from.clone(), to).is_ok()
                            })
                            .count();
                        // To the right of the selection, level with its middle.
                        let size = g
                            .node(id)
                            .map(|n| edit::approx_size(n, &ctx.sockets(g, id)));
                        if let (Some(b), Some(size), Some(n)) = (bounds, size, g.node_mut(id)) {
                            n.ui.pos = [
                                b.max.x + edit::layout::COLUMN_GAP,
                                b.center().y - size[1] / 2.0,
                            ];
                        }
                        wired
                    }));
                }
                Some(id)
            },
        );
        if let Some(e) = error {
            self.warn(e, now);
        }
        match &fed {
            Some(Ok(n)) => self.say(
                tr_args(
                    "status-added-and-wired",
                    &[("name", (&c.name).into()), ("count", (*n).into())],
                ),
                now,
            ),
            Some(Err(e)) => {
                let why = self.feed_reason(e);
                self.warn(
                    tr_args(
                        "status-added-without-wiring",
                        &[("name", (&c.name).into()), ("why", why.into())],
                    ),
                    now,
                );
            }
            None => {}
        }
        let id = id?;
        self.doc.selection = [id].into();
        self.canvas.mark_born(&[id], now);
        self.canvas.focus_added(id);
        self.last_added = vec![id];
        Some(id)
    }

    fn paste(&mut self, text: &str, now: f64) {
        if ac_core::Subgraph::from_json(text).is_none() {
            return;
        }
        let at = self.canvas.pointer_canvas();
        let text = text.to_string();
        match self.doc.edit(&self.registry, &tr("undo-paste"), |g, ctx| {
            edit::paste(g, ctx, &text, [at.x, at.y])
        }) {
            Ok(ids) => {
                self.doc.selection = ids.iter().copied().collect();
                self.canvas.mark_born(&ids, now);
                self.say(
                    tr_args("status-pasted", &[("count", ids.len().into())]),
                    now,
                );
                self.last_added = ids;
            }
            Err(e) => self.warn(e, now),
        }
    }

    fn selected_real(&self) -> BTreeSet<NodeId> {
        self.doc
            .selection
            .iter()
            .copied()
            .filter(|i| !edit::is_virtual(*i))
            .collect()
    }

    /// Run a command (from a shortcut, a menu, or a button).
    pub fn run(&mut self, ctx: &egui::Context, a: Action, now: f64) {
        let reg = self.registry.clone();
        // The host's transport plays the patch (Space usually reaches the
        // host as well); the editor's would fight it.
        let host_transport = matches!(
            a,
            Action::PlayStop
                | Action::Stop
                | Action::PlayFromStart
                | Action::Record
                | Action::Rewind
        );
        if host_transport && self.in_host() {
            return;
        }
        if matches!(a, Action::PlayKeys) && !self.engine.keys_reach_engine() {
            self.say(tr("status-play-keys-unavailable"), now);
            return;
        }
        match a {
            Action::New => self.request(AfterConfirm::New, now),
            Action::Open => self.file_dialog(FilePurpose::Open, now),
            Action::ImportFromLink => self.open_link_dialog(),
            Action::Save => self.save(false, now),
            Action::SaveAs => self.save(true, now),
            Action::ExportWav => self.file_dialog(FilePurpose::Export, now),
            Action::ExportPack => self.export_pack(now),
            // A plugin's editor closes with the host's window.
            Action::Quit if self.is_hosted() => {}
            Action::Quit => self.request(AfterConfirm::Quit, now),
            Action::Undo => {
                if self.doc.undo() {
                    self.canvas.edit = None;
                } else {
                    self.say(tr("status-nothing-to-undo"), now);
                }
            }
            Action::Redo => {
                if self.doc.redo() {
                    self.canvas.edit = None;
                } else {
                    self.say(tr("status-nothing-to-redo"), now);
                }
            }
            Action::Copy => {
                let sel = self.selected_real();
                if !sel.is_empty() {
                    let text = edit::copy(self.doc.graph(), &self.doc.project.group_defs, &sel);
                    ctx.copy_text(text.clone());
                    self.clipboard = Some(text);
                    self.say(
                        tr_args("status-copied", &[("count", sel.len().into())]),
                        now,
                    );
                }
            }
            Action::Paste => {
                // The platform's paste arrives as an event with the text; a
                // bare key press (other integrations, tests) pastes what we
                // copied last.
                if let Some(text) = self.clipboard.clone() {
                    self.paste(&text, now);
                }
            }
            Action::Delete => self
                .canvas
                .delete_selection(&mut self.doc, &reg, false, now),
            Action::DeleteReconnect => self.canvas.delete_selection(&mut self.doc, &reg, true, now),
            Action::Duplicate | Action::DuplicateWithInputs => {
                let sel = self.selected_real();
                if sel.is_empty() {
                    return;
                }
                let keep = a == Action::DuplicateWithInputs;
                let new = self.doc.edit(&reg, &tr("undo-duplicate"), |g, ctx| {
                    edit::duplicate(g, ctx, &sel, keep, [30.0, 30.0])
                });
                self.doc.selection = new.iter().copied().collect();
                self.canvas.mark_born(&new, now);
                self.last_added = new;
                self.canvas.start_grab(&mut self.doc, false);
            }
            Action::SelectAll => {
                self.doc.selection = self.doc.graph().ids();
            }
            Action::SelectNone => self.doc.selection.clear(),
            Action::SelectLinked => {
                let sel = self.selected_real();
                self.doc.selection = edit::linked(self.doc.graph(), &sel);
            }
            Action::Grab => self.canvas.start_grab(&mut self.doc, true),
            Action::Rename => {
                if let Some(id) = self.selected_real().into_iter().next() {
                    self.canvas.rename_node(&self.doc, &reg, id);
                }
            }
            Action::AddNode => {
                let at = self.canvas.pointer_canvas();
                let screen = self.canvas.transform().to_screen(at);
                self.find.close();
                self.palette.open_at(screen, at, None);
                self.prepare_feed();
            }
            Action::AutoLayout => match self.canvas.auto_layout(&mut self.doc, &reg) {
                Ok(n) => self.say(tr_args("status-laid-out", &[("count", n.into())]), now),
                Err(e) => self.say(e, now),
            },
            Action::ArrangeMenu => {
                if self.selected_real().len() >= 2 {
                    self.canvas.open_arrange_menu();
                } else {
                    self.say(tr("status-select-two-to-align"), now);
                }
            }
            Action::AlignLeft
            | Action::AlignCenter
            | Action::AlignRight
            | Action::AlignTop
            | Action::AlignMiddle
            | Action::AlignBottom
            | Action::DistributeHorizontally
            | Action::DistributeVertically => {
                if let Some(how) = a.align()
                    && let Err(e) = self.canvas.align_selection(&mut self.doc, &reg, how)
                {
                    self.say(e, now);
                }
            }
            Action::LazyConnect => match self.canvas.lazy_connect(&mut self.doc, &reg) {
                Ok(what) => self.say(what, now),
                Err(e) => self.warn(e, now),
            },
            Action::Find => {
                self.palette.close();
                self.find.show_bar();
            }
            Action::Frame => self.canvas.frame_selection(&mut self.doc, &reg),
            Action::Group => {
                let sel = self.selected_real();
                if sel.is_empty() {
                    return;
                }
                let pins_before = pin_count(&self.doc.project);
                if let Some((inst, _)) = self.doc.edit(&reg, &tr("undo-make-group"), |g, ctx| {
                    edit::group::make_group(g, ctx, &sel)
                }) {
                    self.doc.selection = [inst].into();
                    self.canvas.mark_born(&[inst], now);
                    let dropped = pins_before.saturating_sub(pin_count(&self.doc.project));
                    if dropped > 0 {
                        self.say(
                            tr_args("status-grouped-unpinned", &[("count", dropped.into())]),
                            now,
                        );
                    } else {
                        self.say(tr("status-grouped"), now);
                    }
                }
            }
            Action::Ungroup => {
                let groups: Vec<NodeId> = self
                    .selected_real()
                    .into_iter()
                    .filter(|id| {
                        self.doc
                            .graph()
                            .node(*id)
                            .is_some_and(|n| n.kind == GROUP_KIND)
                    })
                    .collect();
                if groups.is_empty() {
                    self.say(tr("status-select-group-to-ungroup"), now);
                    return;
                }
                let mut all = Vec::new();
                for g_id in groups {
                    let ungrouped = self.doc.edit(&reg, &tr("undo-ungroup"), |g, ctx| {
                        let new = edit::group::ungroup(g, ctx, g_id)?;
                        // Portals that come out keep names of their own.
                        edit::portal::after_paste(g, &new);
                        Some(new)
                    });
                    if let Some(new) = ungrouped {
                        all.extend(new);
                    }
                }
                self.doc.selection = all.iter().copied().collect();
                self.canvas.mark_born(&all, now);
            }
            Action::EnterLeaveGroup => {
                let sel: Vec<NodeId> = self.selected_real().into_iter().collect();
                let entered = match sel.as_slice() {
                    [one] => self.doc.enter_group(*one),
                    _ => false,
                };
                if !entered && !self.doc.leave_group() && sel.len() == 1 {
                    self.say(tr("status-not-a-group"), now);
                }
            }
            Action::Mute => self.toggle_nodes(&tr("undo-mute"), |n| n.muted = !n.muted),
            Action::Collapse => self.toggle_nodes(&tr("undo-collapse"), |n| {
                n.ui.collapsed = !n.ui.collapsed;
            }),
            Action::HideUnwired => self.toggle_nodes(&tr("undo-hide-unwired"), |n| {
                let on = n.extra.get(HIDE_UNWIRED).and_then(|v| v.as_bool()) == Some(true);
                if on {
                    n.extra.remove(HIDE_UNWIRED);
                } else {
                    n.extra.insert(HIDE_UNWIRED.into(), true.into());
                }
            }),
            Action::NodeHelp => {
                // One node: its reference. Otherwise the list of every node.
                let sel = self.selected_real();
                let kind = sel
                    .first()
                    .filter(|_| sel.len() == 1)
                    .and_then(|id| self.doc.graph().node(*id))
                    .map(|n| n.kind.clone());
                if kind.is_none() {
                    self.side.help.kind = None;
                }
                self.side.show_help(kind.as_deref());
            }
            Action::FitAll => self.canvas.fit_all(&self.doc, &reg),
            Action::FitSelection => self.canvas.fit_selection(&self.doc, &reg),
            Action::ZoomIn | Action::ZoomOut => {
                let f = if a == Action::ZoomIn { 1.25 } else { 0.8 };
                let screen = self.canvas.screen;
                let anchor = self.canvas.pointer.unwrap_or(screen.center());
                let mut v = self.canvas.view;
                v.zoom_at(screen, anchor, f);
                self.canvas.fit(None, 1.0);
                self.canvas.view = v;
            }
            Action::UiScaleUp => {
                let s = settings::scale_up(self.settings.prefs.ui_scale);
                self.set_ui_scale(ctx, s, now);
            }
            Action::UiScaleDown => {
                let s = settings::scale_down(self.settings.prefs.ui_scale);
                self.set_ui_scale(ctx, s, now);
            }
            Action::UiScaleReset => self.set_ui_scale(ctx, 1.0, now),
            Action::ToggleSidePanel => self.side.open = !self.side.open,
            Action::Minimap => self.canvas.minimap = !self.canvas.minimap,
            Action::ReduceMotion => {
                let on = !self.settings.prefs.reduce_motion;
                self.set_reduce_motion(ctx, on, now);
            }
            Action::Portal => match self.canvas.toggle_portal(&mut self.doc, &reg, now) {
                Ok(text) => self.say(text, now),
                Err(text) => self.warn(text, now),
            },
            Action::ShowTiming => {
                self.show_timing = !self.show_timing;
                self.timing.clear();
                self.engine
                    .handle
                    .status()
                    .timing_enabled
                    .store(self.show_timing, std::sync::atomic::Ordering::Relaxed);
            }
            Action::PlayStop => {
                if self.engine.handle.is_playing() {
                    self.engine.handle.stop();
                } else {
                    self.engine.handle.play();
                }
                ctx.request_repaint();
            }
            Action::Stop => {
                self.engine.handle.stop();
                ctx.request_repaint();
            }
            Action::Panic => {
                self.panic(now);
                ctx.request_repaint();
            }
            Action::PlayFromStart => {
                self.engine.handle.play_from_start();
                ctx.request_repaint();
            }
            Action::Record => {
                let on = !self
                    .engine
                    .handle
                    .status()
                    .recording
                    .load(std::sync::atomic::Ordering::Relaxed);
                self.engine.handle.set_recording(on);
                if on && !self.engine.handle.is_playing() {
                    self.engine.handle.play();
                }
                ctx.request_repaint();
            }
            Action::ToggleLoop => {
                let region = match self.doc.project.settings.loop_region {
                    Some(_) => None,
                    None => {
                        let bar = self.doc.project.settings.tempo.first().map_or(4.0, |t| {
                            f64::from(t.time_sig[0].max(1)) * 4.0 / f64::from(t.time_sig[1].max(1))
                        });
                        Some([0.0, bar * 4.0])
                    }
                };
                self.doc
                    .edit_project(&tr("undo-loop"), |p| p.settings.loop_region = region);
            }
            Action::Rewind => self.engine.handle.seek(0),
            Action::GoToEnd => {
                let rate = f64::from(self.engine.info.sample_rate);
                let tempo = TempoMap::new(rate, &self.doc.project.settings.tempo);
                let end = crate::ruler::song_end(&self.doc.project, &tempo);
                self.seek_beat(end);
            }
            Action::AddMarker => {
                let beat = self.playhead_beat();
                crate::ruler::add_marker(&mut self.doc, beat);
            }
            Action::PreviousMarker | Action::NextMarker => {
                let d = if a == Action::NextMarker { 1 } else { -1 };
                let from = self.playhead_beat();
                match crate::ruler::next_marker(&self.doc.project, from, d) {
                    Some(beat) => self.seek_beat(beat),
                    None if d < 0 => self.engine.handle.seek(0),
                    None => {}
                }
            }
            Action::Metronome => {
                let on = !self.doc.project.settings.metronome;
                self.doc
                    .edit_project(&tr("undo-metronome"), |p| p.settings.metronome = on);
            }
            Action::PlayKeys => {
                let on = !self.play_keys.is_on();
                self.set_play_keys(on);
                if on {
                    let stop = Action::PlayKeys
                        .shortcut()
                        .map(|s| ctx.format_shortcut(&s))
                        .unwrap_or_default();
                    self.say(
                        tr_args("status-play-keys-turned-on", &[("stop", stop.into())]),
                        now,
                    );
                } else {
                    self.say(tr("status-play-keys-off"), now);
                }
            }
            Action::Pin => self.pin_selection(now),
            Action::TogglePanels => {
                // Performance mode shows a panel until it's left.
                if !self.perform.fullscreen {
                    let on = !self.perform.shown;
                    self.set_view(on);
                }
            }
            Action::PerformanceMode => {
                let on = !self.perform.fullscreen;
                if on {
                    self.set_view(true);
                }
                self.perform.set_fullscreen(ctx, on);
            }
            Action::Shortcuts => self.show_shortcuts = true,
            Action::About => self.show_about = true,
        }
    }

    /// Show the panels (`true`) or the canvas. Whatever the view going
    /// away holds down lets go.
    fn set_view(&mut self, panels: bool) {
        if self.perform.shown == panels {
            return;
        }
        if panels {
            for _ in 0..4 {
                if !self.canvas.cancel(&mut self.doc) {
                    break;
                }
            }
            self.canvas
                .controls
                .flush(&mut self.doc, &mut self.engine.handle);
            self.palette.close();
        } else {
            self.perform.let_go(&mut self.doc, &mut self.engine.handle);
        }
        self.perform.shown = panels;
    }

    /// `P`: pin the selected controls and displays to the current panel
    /// (making "Panel 1" if there's none), or unpin them when they're all
    /// on it already.
    fn pin_selection(&mut self, now: f64) {
        if !self.doc.scope.is_empty() {
            self.say(tr("status-pin-top-level-only"), now);
            return;
        }
        let nodes: Vec<NodeId> = self
            .selected_real()
            .into_iter()
            .filter(|id| self.doc.project.graph.node(*id).is_some_and(pins::pinnable))
            .collect();
        if nodes.is_empty() {
            self.say(tr("status-select-something-to-pin"), now);
            return;
        }
        let current = self.perform.current;
        let all_on = self
            .doc
            .project
            .panels
            .get(current)
            .is_some_and(|p| nodes.iter().all(|n| p.contains(*n)));
        let panel = Some(current);
        let r = if all_on {
            PinRequest::Unpin { nodes, panel }
        } else {
            PinRequest::Pin { nodes, panel }
        };
        self.pin_request(r, now);
    }

    /// Pin or unpin nodes (one undo step each).
    fn pin_request(&mut self, r: PinRequest, now: f64) {
        match r {
            PinRequest::Pin { nodes, panel } => {
                let graph = &self.doc.project.graph;
                if !nodes
                    .iter()
                    .any(|id| graph.node(*id).is_some_and(pins::pinnable))
                {
                    self.say(tr("status-only-controls-pin"), now);
                    return;
                }
                let registry = self.registry.clone();
                let defs = self.doc.project.group_defs.clone();
                let columns = self.perform.place_columns();
                let (index, added, name) =
                    self.doc.edit_panels(&tr("undo-pin-to-panel"), |panels, g| {
                        let index = match panel {
                            Some(i) if i < panels.len() => i,
                            _ => pins::add_panel(panels, ""),
                        };
                        let size = |it: &ac_core::PanelItem| {
                            crate::perform::item_size(&registry, g, &defs, it)
                        };
                        let added = pins::pin_placed(panels, g, index, &nodes, &size, columns);
                        let name = panels.get(index).map_or(String::new(), |p| p.name.clone());
                        (index, added, name)
                    });
                self.perform.current = index;
                if added > 0 {
                    self.say(
                        tr_args(
                            "status-pinned",
                            &[("count", added.into()), ("panel", (&name).into())],
                        ),
                        now,
                    );
                } else {
                    self.say(
                        tr_args("status-already-pinned", &[("panel", name.into())]),
                        now,
                    );
                }
            }
            PinRequest::Strip { nodes, panel } => {
                let registry = self.registry.clone();
                let defs = self.doc.project.group_defs.clone();
                let columns = self.perform.place_columns();
                let name = nodes
                    .first()
                    .and_then(|id| self.doc.project.graph.node(*id))
                    .and_then(|n| n.ui.label.clone())
                    .unwrap_or_else(|| tr("perform-new-strip"));
                // No panel named: the one the panel view shows, or a new one.
                let current = self.perform.current;
                let made = self.doc.edit_panels(&tr("undo-make-strip"), |panels, g| {
                    let index = match panel.or(Some(current)) {
                        Some(i) if i < panels.len() => i,
                        _ => pins::add_panel(panels, ""),
                    };
                    let size = |it: &ac_core::PanelItem| {
                        crate::perform::item_size(&registry, g, &defs, it)
                    };
                    pins::pin_strip(panels, g, index, &nodes, &name, &size, columns).map(|_| index)
                });
                match made {
                    Some(index) => {
                        self.perform.current = index;
                        self.say(
                            tr_args("status-pinned-strip", &[("name", name.into())]),
                            now,
                        );
                    }
                    None => self.say(tr("status-only-controls-pin"), now),
                }
            }
            PinRequest::FromSelection { nodes, kind } => {
                let words = crate::edit::generate::Words {
                    mixer: tr("perform-mixer"),
                    pads: tr("perform-pads"),
                    grid: tr("perform-grid"),
                    tracks: tr("perform-tracks"),
                    track: |n| tr_args("perform-track-n", &[("n", n.into())]),
                    scene: |n| tr_args("perform-scene-n", &[("n", n.into())]),
                };
                use crate::edit::generate;
                use crate::perform::Generated;
                let label = match kind {
                    Generated::Mixer => tr("undo-mixer-from-selection"),
                    Generated::Pads => tr("undo-pads-from-selection"),
                    Generated::Grid => tr("undo-grid-from-selection"),
                };
                let registry = self.registry.clone();
                let made =
                    self.doc
                        .edit_with_panels(&registry, &label, |g, ctx, panels| match kind {
                            Generated::Mixer => generate::mixer(g, panels, &nodes, &words),
                            Generated::Pads => generate::pads(g, ctx, panels, &nodes, &words),
                            Generated::Grid => {
                                generate::launch_grid(g, ctx, panels, &nodes, &words)
                            }
                        });
                match made {
                    Some(m) => {
                        self.perform.current = m.panel;
                        self.set_view(true);
                        let name = self
                            .doc
                            .project
                            .panels
                            .get(m.panel)
                            .map_or(String::new(), |p| p.name.clone());
                        self.say(tr_args("status-panel-made", &[("panel", name.into())]), now);
                    }
                    None => self.say(tr("status-nothing-for-panel"), now),
                }
            }
            PinRequest::Unpin { nodes, panel } => {
                let from = panel
                    .and_then(|i| self.doc.project.panels.get(i))
                    .map_or_else(|| tr("status-every-panel"), |p| p.name.clone());
                let removed = self.doc.edit_panels(&tr("undo-unpin"), |panels, _| {
                    pins::unpin(panels, panel, &nodes)
                });
                self.say(
                    tr_args(
                        "status-unpinned",
                        &[("count", removed.into()), ("panel", from.into())],
                    ),
                    now,
                );
            }
        }
    }

    fn toggle_nodes(&mut self, label: &str, f: impl Fn(&mut ac_core::NodeInstance)) {
        let sel = self.selected_real();
        if sel.is_empty() {
            return;
        }
        self.doc.edit(&self.registry, label, |g, _| {
            for id in &sel {
                if let Some(n) = g.node_mut(*id) {
                    f(n);
                }
            }
        });
    }

    // ------------------------------------------------------------ files

    fn request(&mut self, after: AfterConfirm, now: f64) {
        if self.doc.is_dirty() {
            self.confirm = Some(after);
        } else {
            self.proceed(after, now);
        }
    }

    /// Open a file the person chose. A block file only joins the library,
    /// so it never asks about unsaved changes; projects and packs do.
    fn request_open(&mut self, path: PathBuf, now: f64) {
        if file_kind(&path) == FileKind::Block {
            self.import_block_file(&path, now);
        } else {
            self.request(AfterConfirm::Open(path), now);
        }
    }

    fn proceed(&mut self, after: AfterConfirm, now: f64) {
        match after {
            // The host starts over with its own default (a plugin's
            // depends on whether it's the effect or the instrument).
            AfterConfirm::New if self.in_host() => {
                self.host_requests.push(HostRequest::New);
                self.say(tr("status-new-project"), now);
            }
            AfterConfirm::New => {
                let p = files::default_project(&self.registry);
                self.replace_document(p, None, cwd(), AssetStore::default());
                self.say(tr("status-new-project"), now);
            }
            AfterConfirm::Open(path) => self.open(&path, now),
            AfterConfirm::OpenExample(path) => self.open_example(&path, now),
            AfterConfirm::OpenPicked => {
                if let Some((name, bytes)) = self.picked.take() {
                    self.open_picked(&name, &bytes);
                }
            }
            AfterConfirm::Quit => {
                self.allow_close = true;
                self.close_now = true;
            }
        }
    }

    /// Open a file: a project (missing files and unknown nodes still
    /// open), a pack (unpacked into a new folder beside it, then opened),
    /// or a block file (imported into the library; the document stays).
    pub fn open(&mut self, path: &Path, now: f64) {
        match file_kind(path) {
            FileKind::Project => {
                let what = tr_args(
                    "status-opened",
                    &[("path", path.display().to_string().into())],
                );
                if let Err(e) = self.open_project(path, what, now) {
                    self.warn(tr_args("status-couldnt-open", &[("error", e.into())]), now);
                }
            }
            FileKind::Pack => match ac_io::unpack_beside(path) {
                Ok(project) => {
                    let dir = files::project_dir(&project);
                    let mut what = tr_args(
                        "status-unpacked",
                        &[
                            ("pack", file_name(path).into()),
                            ("folder", dir.display().to_string().into()),
                        ],
                    );
                    // Someone else's blocks: import them and point the
                    // copies at the imports, before the project opens.
                    if let Ok(mut p) = ac_io::load_project(&project) {
                        let adopted = self.library.adopt(&mut p);
                        if adopted > 0 && ac_io::save_project(&project, &p).is_ok() {
                            what.push_str(" · ");
                            what.push_str(&tr_args(
                                "status-pack-blocks-added",
                                &[("count", adopted.into())],
                            ));
                        }
                    }
                    if let Err(e) = self.open_project(&project, what, now) {
                        self.warn(
                            tr_args(
                                "status-unpacked-but-not-opened",
                                &[
                                    ("folder", dir.display().to_string().into()),
                                    ("error", e.into()),
                                ],
                            ),
                            now,
                        );
                    }
                }
                Err(e) => self.warn(
                    tr_args(
                        "status-couldnt-unpack",
                        &[
                            ("pack", path.display().to_string().into()),
                            ("error", e.to_string().into()),
                        ],
                    ),
                    now,
                ),
            },
            FileKind::Block => self.import_block_file(path, now),
        }
    }

    /// Replace the document with the project at `path`. The status starts
    /// with `what` and adds the blocks it brought and any missing files.
    fn open_project(&mut self, path: &Path, what: String, now: f64) -> Result<(), String> {
        if self.in_host() {
            // The host loads it with its audio files into its own state;
            // the document follows on the next frame.
            self.host_requests
                .push(HostRequest::Open(path.to_path_buf()));
            self.recent.add(path);
            self.say(what, now);
            return Ok(());
        }
        let (project, assets) = files::open_project(path, &self.registry)?;
        let missing = assets.issues.len();
        self.replace_document(
            project,
            Some(path.to_path_buf()),
            files::project_dir(path),
            assets,
        );
        self.recent.add(path);
        // Other people's blocks the project carries join the library.
        let blocks = self.library.import_unknown(&self.doc.project);
        let mut text = what;
        if blocks > 0 {
            text.push_str(" · ");
            text.push_str(&tr_args(
                "status-project-blocks-added",
                &[("count", blocks.into())],
            ));
        }
        if missing > 0 {
            text.push_str(" — ");
            text.push_str(&tr_args(
                "status-audio-file-problems",
                &[("count", missing.into())],
            ));
            self.warn(text, now);
        } else {
            self.say(text, now);
        }
        Ok(())
    }

    /// Open a shipped example the way the Start project opens: as an
    /// untitled copy, so saving never overwrites the shipped file. Its
    /// audio files still load from beside it.
    pub fn open_example(&mut self, path: &Path, now: f64) {
        if self.host_files {
            self.file_requests
                .push(FileRequest::Example(path.to_path_buf()));
            return;
        }
        if self.in_host() {
            // A plugin's copy lives in the host's state, so the shipped
            // file is never written either way.
            self.host_requests
                .push(HostRequest::Open(path.to_path_buf()));
            self.say(
                tr_args(
                    "status-opened-example-file",
                    &[("file", file_name(path).into())],
                ),
                now,
            );
            return;
        }
        match files::open_project(path, &self.registry) {
            Ok((mut project, assets)) => {
                // Its stored view was set on someone else's window.
                project.view = None;
                let name = if project.name.trim().is_empty() {
                    file_name(path)
                } else {
                    project.name.clone()
                };
                self.replace_document(project, None, files::project_dir(path), assets);
                self.say(
                    tr_args("status-opened-example", &[("name", name.into())]),
                    now,
                );
            }
            Err(e) => self.warn(
                tr_args("status-couldnt-open-example", &[("error", e.into())]),
                now,
            ),
        }
    }

    /// Show `project` as a new, unsaved document (examples, tests).
    pub fn open_unsaved(&mut self, project: Project) {
        let assets = if self.host_files {
            self.bank_assets_for(&project)
        } else {
            AssetStore::load_for(&project, &cwd(), &self.registry)
        };
        self.replace_document(project, None, cwd(), assets);
    }

    fn replace_document(
        &mut self,
        project: Project,
        path: Option<PathBuf>,
        base: PathBuf,
        assets: AssetStore,
    ) {
        self.epoch += 1;
        let seed = self.doc.ids.clone().fresh(|_| false).0;
        self.doc = Document::new(project, path, base, seed);
        self.missing = missing_of(&assets);
        self.assets = assets;
        self.loaded_assets = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
        // A new project starts from scratch: transport at zero, every node
        // rebuilt (new resources carry no state over). The device stays open.
        // A plugin host's transport is the host's; a page's is the editor's.
        if !self.is_hosted() || self.host_files {
            self.engine.handle.stop();
            self.engine.handle.set_recording(false);
            self.engine.handle.seek(0);
        }
        // The old project's plugins mustn't hand their state to this one's.
        if let Some(p) = &self.plugins {
            p.clear_restarts();
        }
        let resources = self.resources();
        self.engine.set_resources(&self.doc.project, resources);
        self.engine.synced_revision = self.doc.engine_revision;
        self.loop_sent = None;
        self.palette.close();
        if self.learn.take().is_some() {
            self.engine.handle.set_input_monitor(false);
        }
        self.canvas.reset_for(&self.doc, &self.registry);
        self.perform.reset();
    }

    fn save(&mut self, save_as: bool, now: f64) {
        if self.host_files {
            return self.host_save(save_as, now);
        }
        match (&self.doc.path, save_as) {
            (Some(p), false) => {
                let p = p.clone();
                self.save_to(&p, now);
            }
            _ => self.file_dialog(FilePurpose::SaveAs, now),
        }
    }

    pub fn save_to(&mut self, path: &Path, now: f64) {
        let path = files::with_extension(path.to_path_buf(), "acproj");
        // Plugins' own state goes in their nodes, in the document itself
        // (no undo step; the running plugins already hold it, D138).
        if self.plugins.is_some() {
            crate::plugins::store_states(&mut self.doc.project);
        }
        let mut project = self.doc.project.clone();
        // Keep the view so the project reopens where it was.
        project.view = Some(ac_core::View {
            center: [self.canvas.view.center.x, self.canvas.view.center.y],
            zoom: self.canvas.view.zoom,
        });
        let copied = match files::consolidate_assets(
            &mut project,
            &self.registry,
            &self.doc.base_dir,
            &path,
        ) {
            Ok(n) => n,
            Err(e) => {
                self.warn(
                    tr_args("status-couldnt-copy-audio", &[("error", e.into())]),
                    now,
                );
                0
            }
        };
        match ac_io::save_project(&path, &project) {
            Ok(()) => {
                // Relinked assets, or a new home for the recorder's takes.
                let assets_changed = project.graph != self.doc.project.graph
                    || project.group_defs != self.doc.project.group_defs
                    || self.doc.path.as_ref() != Some(&path);
                self.doc.project = project;
                self.doc.path = Some(path.clone());
                self.doc.base_dir = files::project_dir(&path);
                self.doc.mark_saved();
                // Saving moved the project, relinked its files, or kept its view.
                self.epoch += 1;
                if self.in_host() {
                    // The host holds the audio; it writes it beside the file.
                    self.host_requests.push(HostRequest::WriteAssets {
                        dir: files::project_dir(&path),
                    });
                }
                self.recent.add(&path);
                if assets_changed {
                    self.reload_assets();
                }
                let path = path.display().to_string();
                let text = if copied > 0 {
                    tr_args(
                        "status-saved-with-audio",
                        &[("path", path.into()), ("count", copied.into())],
                    )
                } else {
                    tr_args("status-saved", &[("path", path.into())])
                };
                self.say(text, now);
            }
            Err(e) => self.warn(
                tr_args("status-couldnt-save", &[("error", e.to_string().into())]),
                now,
            ),
        }
    }

    fn export(&mut self, path: PathBuf, now: f64) {
        let path = files::with_extension(path, "wav");
        let mut project = self.doc.project.clone();
        // The plugins as they sound now, not as they were last saved.
        if self.plugins.is_some() {
            crate::plugins::store_states(&mut project);
        }
        let registry = self.registry.clone();
        // Offline exports never record.
        let resources: Arc<dyn ac_core::Resources> = Arc::new(FileResources {
            assets: self.assets.clone(),
            recorder: None,
            // The export renders its own instances on its own thread.
            plugins: self.plugins.as_ref().map(crate::plugins::Plugins::loader),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(export_wav(&project, registry, resources, &p));
        });
        self.export_job = Some((path, rx));
        self.say(tr("status-exporting"), now);
    }

    fn file_dialog(&mut self, purpose: FilePurpose, now: f64) {
        if self.host_files {
            return self.host_file_dialog(purpose, now);
        }
        let suggested = match &purpose {
            FilePurpose::SaveAs => Some(
                self.doc
                    .path
                    .clone()
                    .unwrap_or_else(|| home().join(format!("{}.acproj", slug(&self.doc.title())))),
            ),
            FilePurpose::Export => Some(
                self.doc
                    .path
                    .as_ref()
                    .map(|p| p.with_extension("wav"))
                    .unwrap_or_else(|| home().join(format!("{}.wav", slug(&self.doc.title())))),
            ),
            FilePurpose::Locate(old) => Some(self.doc.base_dir.join(old)),
            FilePurpose::ExportPack => self
                .doc
                .path
                .as_ref()
                .map(|p| p.with_extension(ac_io::PACK_EXTENSION)),
            FilePurpose::ExportBlock(what) => {
                let dir = self
                    .doc
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map_or_else(home, Path::to_path_buf);
                Some(dir.join(format!(
                    "{}.{}",
                    ac_core::slug(&self.export_name(what)),
                    ac_core::BLOCK_EXTENSION
                )))
            }
            _ => self
                .doc
                .path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|d| d.join("")),
        };
        if self.native_dialogs {
            match dialogs::native(&purpose, suggested.as_deref()) {
                Ok(Some(p)) => return self.file_chosen(purpose, p, now),
                Ok(None) => return self.dialog_cancelled(&purpose),
                Err(dialogs::NoDialog) => {} // no system dialog: type the path instead
            }
        }
        self.prompt = Some(PathPrompt::new(purpose, suggested));
    }

    /// The path prompt shown instead of a system file dialog, if one is
    /// open (tests type into it).
    pub fn prompt_mut(&mut self) -> Option<&mut PathPrompt> {
        self.prompt.as_mut()
    }

    fn dialog_cancelled(&mut self, purpose: &FilePurpose) {
        if *purpose == FilePurpose::SaveAs {
            self.pack_after_save = false;
        }
    }

    fn file_chosen(&mut self, purpose: FilePurpose, path: PathBuf, now: f64) {
        match purpose {
            FilePurpose::Open => self.request_open(path, now),
            FilePurpose::SaveAs => {
                self.save_to(&path, now);
                if std::mem::take(&mut self.pack_after_save) && !self.doc.is_dirty() {
                    self.file_dialog(FilePurpose::ExportPack, now);
                }
            }
            FilePurpose::ExportPack => self.export_pack_to(&path, now),
            FilePurpose::ImportBlock => self.import_block_file(&path, now),
            FilePurpose::ExportBlock(what) => self.export_block_to(&what, path, now),
            FilePurpose::Export => self.export(path, now),
            FilePurpose::ChooseClip(node) => self.assign_clip(node, &path, now),
            FilePurpose::Locate(old) => match self.import_audio(&path) {
                Ok(a) => self.relink(&old, &a, now),
                Err(e) => self.warn(e, now),
            },
        }
    }

    /// Point the nodes using the missing file `old` at `a`.
    fn relink(&mut self, old: &str, a: &files::ImportedAsset, now: f64) {
        let reg = self.registry.clone();
        let n = self.doc.edit_project(&tr("undo-locate-file"), |p| {
            files::relink_asset(p, &reg, old, a)
        });
        self.reload_assets();
        self.say(tr_args("status-relinked", &[("count", n.into())]), now);
    }

    /// An audio file someone picked, ready for a config: copied into the
    /// project's assets folder when it has one, or, when the host keeps
    /// the project, handed to the host under `assets/` (a plugin saves the
    /// bytes in its state) and shown from the editor's own copy.
    fn import_audio(&mut self, src: &Path) -> Result<files::ImportedAsset, String> {
        if !self.in_host() {
            return files::import_audio(src, self.doc.path.as_deref());
        }
        let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let data =
            ac_io::wav::read_wav_bytes(&bytes).map_err(|e| format!("{}: {e}", src.display()))?;
        let hash = ac_core::util::content_hash(&bytes);
        let channels = data.channels.len().max(1) as u64;
        // The content in the name: the same file picked twice is kept once,
        // and two files that share a name don't replace each other.
        let stem = src
            .file_stem()
            .map_or_else(|| "clip".into(), |s| s.to_string_lossy().to_string());
        let short: String = hash.chars().take(8).collect();
        let file = format!("assets/{stem}-{short}.wav");
        self.assets.insert(&file, data);
        self.host_requests.push(HostRequest::AddAsset {
            path: file.clone(),
            bytes,
        });
        Ok(files::ImportedAsset {
            file,
            channels,
            hash,
            note: None,
        })
    }

    /// Give a Clip node a file (copied into the project's assets when the
    /// project has a home).
    pub fn assign_clip(&mut self, node: NodeId, path: &Path, now: f64) {
        match self.import_audio(path) {
            Ok(a) => self.assign_clip_asset(node, a, now),
            Err(e) => self.warn(
                tr_args("status-couldnt-use-file", &[("error", e.into())]),
                now,
            ),
        }
    }

    /// Give a Clip node the audio file `a`.
    fn assign_clip_asset(&mut self, node: NodeId, a: files::ImportedAsset, now: f64) {
        let a2 = a.clone();
        let reg = self.registry.clone();
        let assign = |doc: &mut Document| {
            doc.edit(&reg, &tr("undo-choose-clip-file"), |g, _| {
                if let Some(n) = g.node_mut(node) {
                    if !n.config.is_object() {
                        n.config = ac_core::empty_config();
                    }
                    if let Some(obj) = n.config.as_object_mut() {
                        obj.insert("file".into(), a2.file.clone().into());
                        obj.insert("channels".into(), a2.channels.into());
                        obj.insert("hash".into(), a2.hash.clone().into());
                    }
                }
            })
        };
        // A Clip pinned to a panel is a top-level node, whatever
        // group the canvas is in.
        if self.doc.graph().node(node).is_some() {
            assign(&mut self.doc);
        } else {
            self.doc.at_root(assign);
        }
        self.reload_assets();
        match a.note {
            Some(n) => self.say(n, now),
            None => self.say(
                tr_args("status-clip-plays", &[("file", a.file.into())]),
                now,
            ),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(after) = self.confirm.clone() {
            match dialogs::confirm_discard(ctx, &self.doc.title()) {
                Some(Confirm::Save) => {
                    self.confirm = None;
                    self.save(false, now);
                    if !self.doc.is_dirty() {
                        self.proceed(after, now);
                    } else if self.host_files {
                        // The host saves later; go on once it has.
                        self.after_host_save = Some(after);
                    }
                }
                Some(Confirm::Discard) => {
                    self.confirm = None;
                    self.proceed(after, now);
                }
                Some(Confirm::Cancel) => self.confirm = None,
                None => {}
            }
        }
        if let Some(p) = &mut self.prompt
            && let Some(result) = p.show(ctx)
        {
            let purpose = p.purpose.clone();
            self.prompt = None;
            match result {
                Some(path) => self.file_chosen(purpose, path, now),
                None => self.dialog_cancelled(&purpose),
            }
        }
        self.link_dialog(ctx, now);
        dialogs::shortcuts(ctx, &mut self.show_shortcuts);
        dialogs::about(ctx, &mut self.show_about);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = &mut self.product {
            p.ui(ctx, &mut self.settings);
        }
    }

    /// Help's update entries: whether daily checks are on, or `None` where
    /// the editor doesn't check (a plugin, the browser).
    fn update_menu(&self) -> Option<bool> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.product.is_some() {
            return Some(self.settings.prefs.update_checks == crate::settings::UpdateChecks::Daily);
        }
        None
    }
}

#[cfg(feature = "desktop")]
impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        EditorApp::ui(self, ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::canvas_bg().to_normalized_gamma_f32()
    }
}

/// `assets` (a host's, built in memory) with a Missing issue for every file
/// `project` refers to that it lacks, as loading from disk would report.
fn with_missing(mut assets: AssetStore, project: &Project, registry: &Registry) -> AssetStore {
    let issue_path = |i: &AssetIssue| match i {
        AssetIssue::Missing { path, .. } | AssetIssue::Changed { path } => path.clone(),
    };
    // Files found since aren't missing any more.
    let present: BTreeSet<String> = assets.paths().cloned().collect();
    assets
        .issues
        .retain(|i| !matches!(i, AssetIssue::Missing { path, .. } if present.contains(path)));
    for (path, _) in ac_io::assets::referenced_assets(project, registry) {
        let known = present.contains(&path) || assets.issues.iter().any(|i| issue_path(i) == path);
        if !known {
            assets.issues.push(AssetIssue::Missing {
                path,
                reason: "not among the plugin's audio files".into(),
            });
        }
    }
    assets
}

fn missing_of(assets: &AssetStore) -> BTreeSet<String> {
    assets
        .issues
        .iter()
        .filter_map(|i| match i {
            AssetIssue::Missing { path, .. } => Some(path.clone()),
            AssetIssue::Changed { .. } => None,
        })
        .collect()
}

/// Whether `path` is a project (anything but a pack or a block file).
fn is_project_file(path: &Path) -> bool {
    !matches!(file_kind(path), FileKind::Pack | FileKind::Block)
}

/// What File ▸ Open does with a file, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileKind {
    Project,
    Pack,
    Block,
}

fn file_kind(path: &Path) -> FileKind {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some(ac_io::PACK_EXTENSION) => FileKind::Pack,
        Some(ac_core::BLOCK_EXTENSION) => FileKind::Block,
        _ => FileKind::Project,
    }
}

/// Pins on every panel.
fn pin_count(project: &Project) -> usize {
    project.panels.iter().map(|p| p.items.len()).sum()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn home() -> PathBuf {
    directories::UserDirs::new().map_or_else(cwd, |d| d.home_dir().to_path_buf())
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "untitled".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::Clock;

    #[test]
    fn the_clock_carries_on_when_a_new_context_starts_over() {
        let mut c = Clock::default();
        assert_eq!(c.now(1.0), 1.0);
        assert_eq!(c.now(5.0), 5.0);
        // A reopened window's input time starts at zero again.
        assert_eq!(c.now(0.0), 5.0);
        assert!((c.now(0.5) - 5.5).abs() < 1e-12);
        assert!((c.now(2.0) - 7.0).abs() < 1e-12);
    }
}
