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
use crate::icons;
use crate::menubar::{self, MenuCommand, MenuState};
use crate::palette::{Candidate, Palette};
use crate::panels::{PanelIo, PanelRequest, SidePanel, Tab};
use crate::perform::{PerformView, PinRequest};
use crate::play_keys::{KeyInput, PlayKeys};
use crate::settings::{self, AppSettings};
use crate::theme::{self, ThemeChoice};
use crate::transport::TransportState;

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
    Quit,
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
    loop_sent: Option<Option<(u64, u64)>>,
    space_shift: bool,
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

    /// An editor made before anything shows it: a plugin's, which lives as
    /// long as the plugin while its window opens and closes. Each context
    /// that shows it ([`EditorApp::ui`]) gets the editor's look on its
    /// first frame. With `AudioMode::Hosted` the host runs the processor
    /// ([`EditorApp::take_processor`]) and the editor opens no devices.
    pub fn embedded(opts: AppOptions) -> EditorApp {
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
                    status = Some((format!("Couldn't open {}: {e}", p.display()), 0.0, true));
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
        let resources = Arc::new(FileResources {
            assets: assets.clone(),
            recorder: recorder
                .clone()
                .map(|r| r as Arc<dyn ac_io::RecorderFactory>),
        });
        let engine = EngineHost::start(&project, registry.clone(), resources, &opts.audio);
        if path.is_some() {
            library.import_unknown(&project);
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
            title: String::new(),
            allow_close: false,
            loop_sent: None,
            space_shift: false,
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
            settings: opts.settings,
            clock: Clock::default(),
            epoch: 0,
            host_seen: (0, 0, 0),
            host_files: opts.host_files,
            file_requests: Vec::new(),
            asset_bank: Default::default(),
        };
        // A plugin window can't go full screen: performance mode fills
        // the window instead.
        app.perform.fills_window = app.is_hosted();
        app.canvas.reset_for(&app.doc, &app.registry);
        if let Some(p) = later {
            app.open(&p, 0.0);
        }
        app.report_library_issues();
        app.host_seen = app.change_stamp();
        app
    }

    /// Block files in the library folder that couldn't be read, said once
    /// at startup (after anything else worth saying).
    fn report_library_issues(&mut self) {
        let Some(first) = self.library.issues.first() else {
            return;
        };
        let n = self.library.issues.len();
        let text = format!(
            "{n} block file{} in your library couldn't be read ({first})",
            plural(n)
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

    /// The project, if it changed since the host last asked or last loaded
    /// one: an edit, an undo or redo, New, Open, or Save. A plugin marks its
    /// state dirty on `Some`. Moving the view alone isn't a change, though
    /// [`EditorApp::project`] carries it.
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
        // Questions about the document being replaced no longer apply.
        self.confirm = None;
        self.prompt = None;
        self.replace_document(project, path.map(Path::to_path_buf), base, assets);
        self.host_seen = self.change_stamp();
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
    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().map(|(text, _, _)| text.as_str())
    }

    /// The editor's settings (theme and UI scale).
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Show the settings' theme and scale in `ctx`.
    fn apply_settings(&self, ctx: &egui::Context) {
        theme::set_choice(ctx, self.settings.prefs.theme);
        ctx.set_zoom_factor(self.settings.prefs.ui_scale);
    }

    /// View ▸ Theme: switch now and remember the choice.
    pub fn set_theme(&mut self, ctx: &egui::Context, choice: ThemeChoice) {
        self.settings.set_theme(choice);
        theme::set_choice(ctx, choice);
        ctx.request_repaint();
    }

    /// View ▸ UI Scale: everything (canvas text included) scales by
    /// `scale` from the next frame, and the choice is remembered.
    pub fn set_ui_scale(&mut self, ctx: &egui::Context, scale: f32, now: f64) {
        self.settings.set_ui_scale(scale);
        let scale = self.settings.prefs.ui_scale;
        ctx.set_zoom_factor(scale);
        self.say(format!("Interface at {:.0} %", scale * 100.0), now);
    }

    fn say(&mut self, text: impl Into<String>, now: f64) {
        self.status = Some((text.into(), now, false));
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
        if ctx.memory(|m| m.focused().is_none()) {
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
                let examples = self.side.help.examples();
                let state = MenuState {
                    undo: self.doc.undo_label(),
                    redo: self.doc.redo_label(),
                    has_selection: !self.doc.selection.is_empty(),
                    side_panel: self.side.open,
                    timing: self.show_timing,
                    minimap: self.canvas.minimap,
                    panels: self.perform.shown,
                    playing: self.engine.handle.is_playing(),
                    looping: self.doc.project.settings.loop_region.is_some(),
                    play_keys: self.play_keys.is_on(),
                    theme: self.settings.prefs.theme,
                    ui_scale: self.settings.prefs.ui_scale,
                    recent: &self.recent,
                    examples,
                    registry: &self.registry,
                    can_quit: self.engine.hosted().is_none(),
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
            });
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
                        requests: Vec::new(),
                        now,
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
                        "{}  Binding {label}: move a control (MIDI, key, gamepad, or OSC)… Esc cancels",
                        icons::LINK
                    ))
                    .small()
                    .color(theme::accent()),
                );
                if ui.small_button("Cancel").clicked() {
                    self.cancel_learn(now);
                }
            // A recorded take waits here until it's made into a Clip.
            } else if let Some(take) = self.takes.first().cloned() {
                ui.label(
                    RichText::new(format!(
                        "{}  Recorded {}",
                        icons::RECORD,
                        take.relative_path
                    ))
                    .small()
                    .color(theme::text()),
                );
                if ui.small_button("Create Clip from take").clicked() {
                    self.clip_from_take(&take, now);
                }
                if ui.small_button(icons::X).on_hover_text("Dismiss").clicked()
                    && !self.takes.is_empty()
                {
                    self.takes.remove(0);
                }
                if self.takes.len() > 1 {
                    ui.label(
                        RichText::new(format!("+{} more", self.takes.len() - 1))
                            .small()
                            .color(theme::text_muted()),
                    );
                }
            } else if let Some((text, t, warn)) = &self.status {
                if now - t < 8.0 || *t == 0.0 {
                    let color = if *warn { theme::warning() } else { theme::text() };
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
                ui.label(RichText::new(format!("Exporting {}…", p.display())).small());
            } else if self.play_keys.is_on() {
                let stop = Action::PlayKeys
                    .shortcut()
                    .map(|s| ui.ctx().format_shortcut(&s))
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "{}  Play keys: keys go to the patch · Esc and shortcuts with \
                         modifiers still work · {stop} stops",
                        icons::KEYBOARD
                    ))
                    .small()
                    .color(theme::accent()),
                );
            } else {
                let hint = if self.perform.shown {
                    "Drag or click a widget to play it · drag a card by its edge to move it · \
                     F11 performance mode · Ctrl+Tab canvas"
                } else if self.doc.selection.is_empty() {
                    "Shift+A add · drag from a socket to wire · double-click empty canvas to search \
                     · Ctrl+F find · Ctrl+L lay out"
                } else if self.doc.selection.len() >= 2 {
                    "F connect two · Shift+A add a node wired from these · Ctrl+L lay out · \
                     Shift+Alt+A align · Ctrl+G group"
                } else {
                    "G move · Shift+D duplicate · X delete · Ctrl+G group · M mute · H collapse"
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
                let r = ui.add(
                    egui::Button::new(RichText::new(text).small().color(theme::text_muted()))
                        .frame(false),
                );
                if r.on_hover_text("Audio device settings").clicked() {
                    self.side.open = true;
                    self.side.tab = Tab::Devices;
                }
                let n = self.engine.report.diagnostics.len();
                if n > 0 {
                    let errors = self.engine.report.has_errors();
                    let r = ui.add(
                        egui::Button::new(
                            RichText::new(format!("{} {n}", icons::WARNING))
                                .small()
                                .color(if errors { theme::error() } else { theme::warning() }),
                        )
                        .frame(false),
                    );
                    if r.clicked() {
                        self.side.open = true;
                        self.side.tab = Tab::Problems;
                    }
                }
            });
        });
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
        self.poll_takes(now);
        self.sync_loop();
    }

    fn finish_frame(&mut self, ctx: &egui::Context, now: f64) {
        self.sync_engine(now);

        if let Some((path, rx)) = &self.export_job
            && let Ok(r) = rx.try_recv()
        {
            let path = path.clone();
            self.export_job = None;
            match r {
                Ok(s) => {
                    let secs = s.frames as f64 / f64::from(s.sample_rate.max(1));
                    self.say(
                        format!(
                            "Exported {:.2} s × {} ch at {} Hz to {}",
                            secs,
                            s.channels,
                            s.sample_rate,
                            path.display()
                        ),
                        now,
                    );
                }
                Err(e) => self.warn(format!("Export failed: {e}"), now),
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
        let title = format!(
            "{}{} — Audio Constructor",
            if self.doc.is_dirty() { "● " } else { "" },
            self.doc.title()
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

    /// Keep the engine's loop region in step with the project.
    fn sync_loop(&mut self) {
        let rate = f64::from(self.engine.info.sample_rate);
        let tempo = TempoMap::new(rate, &self.doc.project.settings.tempo);
        let want = self.doc.project.settings.loop_region.and_then(|[a, b]| {
            let (s, e) = (tempo.sample_at(a), tempo.sample_at(b));
            (e > s).then_some((s, e))
        });
        if self.loop_sent != Some(want) {
            self.engine.handle.set_loop(want);
            self.loop_sent = Some(want);
        }
    }

    fn reload_assets(&mut self) {
        self.assets = if self.host_files {
            self.reload_bank_assets()
        } else {
            AssetStore::load_for(&self.doc.project, &self.doc.base_dir, &self.registry)
        };
        self.missing = missing_of(&self.assets);
        self.loaded_assets = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
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
        })
    }

    /// Takes the recorder finished: offer to turn them into Clip nodes.
    fn poll_takes(&mut self, now: f64) {
        let Some(r) = &self.recorder else { return };
        let done = r.finished_takes();
        for t in done {
            match &t.error {
                Some(e) => self.warn(format!("Take {} is incomplete: {e}", t.relative_path), now),
                None => self.say(format!("Recorded {}", t.relative_path), now),
            }
            self.takes.push(t);
        }
    }

    /// Add a Clip node playing a recorded take, next to its Record node when
    /// that's in the current graph.
    pub fn clip_from_take(&mut self, take: &ac_io::Take, now: f64) {
        let hash = std::fs::read(&take.path)
            .map(|b| ac_core::util::content_hash(&b))
            .unwrap_or_default();
        let prefix = self.doc.scope_prefix();
        let near = take
            .node_key
            .parse::<ac_core::NodePath>()
            .ok()
            .filter(|p| p.0.len() == prefix.len() + 1 && p.0[..prefix.len()] == prefix[..])
            .and_then(|p| p.leaf())
            .and_then(|id| self.doc.graph().node(id))
            .map(|n| egui::pos2(n.ui.pos[0] + theme::NODE_WIDTH + 60.0, n.ui.pos[1]));
        let at = near.unwrap_or(self.canvas.view.center);
        let c = Candidate {
            name: "Clip".into(),
            kind: "clip".into(),
            category: None,
            summary: String::new(),
            config: serde_json::json!({
                "file": take.relative_path,
                "channels": take.channels,
                "hash": hash,
            }),
            socket: None,
            block: None,
        };
        self.add_candidate(&c, at + vec2(theme::NODE_WIDTH / 2.0, 14.0), None, now);
        self.takes.retain(|t| t.path != take.path);
    }

    // ------------------------------------------------------------ input

    fn handle_keys(&mut self, ctx: &egui::Context, now: f64) {
        let typing = ctx.egui_wants_keyboard_input()
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
                    if key == Key::Escape && pressed {
                        if self.palette.open || self.find.open {
                            continue; // they handle it
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
                        self.canvas.cancel(&mut self.doc);
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
            self.say("Binding cancelled", now);
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
        if crate::bindings::add(&mut self.doc, "Bind", binding) {
            self.say(format!("Bound {} to {}", source.label(), learn.label), now);
        } else {
            self.say(
                format!("{} already drives {}", source.label(), learn.label),
                now,
            );
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
            PanelRequest::Locate(path) => self.file_dialog(FilePurpose::Locate(path), now),
            PanelRequest::Add(c) => {
                let at = self.canvas.view.center;
                self.add_candidate(&c, at, None, now);
            }
            PanelRequest::Navigate(path) => {
                if let Some(id) = self.doc.navigate_to(&path) {
                    self.canvas.jump_to(&self.doc, &self.registry, id);
                } else {
                    self.warn("That node no longer exists.", now);
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
            PanelRequest::SwitchAudio(config) => {
                let resources = self.resources();
                self.engine
                    .switch(config, &self.doc.project, self.registry.clone(), resources);
                // A rebuilt engine has seen none of the document's revisions.
                self.engine
                    .replace(&self.doc.project, self.doc.engine_revision);
                self.loop_sent = None;
                match &self.engine.device_error {
                    Some(e) => self.warn(format!("Audio: {e}"), now),
                    None => {
                        let d = self.engine.describe();
                        self.say(format!("Audio: {d}"), now);
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
            MenuCommand::SetUiScale(s) => self.set_ui_scale(ctx, s, now),
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
            self.doc.edit(&self.registry, "Insert on wire", |g, ctx| {
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
        self.palette.note = (n >= 2).then(|| {
            format!("The {n} selected nodes will be wired into it, in the order you selected them")
        });
    }

    /// Why the selected nodes weren't wired into a new node, for the
    /// status line.
    fn feed_reason(&self, e: &FeedError) -> String {
        let name = |id: NodeId| {
            self.doc.graph().node(id).map_or_else(
                || "a selected node".to_string(),
                |n| connect::title(n, &self.registry, &self.doc.project.group_defs),
            )
        };
        match e {
            FeedError::NoOutput(id) => format!("{} has no output", name(*id)),
            FeedError::NoInput(id, ty) => format!(
                "it has no free input that takes the {} output of {}",
                ty.name(),
                name(*id)
            ),
        }
    }

    /// Jump to a node the find bar found: into its group, selected and
    /// in view.
    fn jump_to_found(&mut self, f: &Found, now: f64) {
        match self.doc.navigate_to(&f.path) {
            Some(id) => {
                self.canvas.jump_to(&self.doc, &self.registry, id);
                if f.place.is_empty() {
                    self.say(format!("{} ({})", f.title, f.kind), now);
                } else {
                    self.say(format!("{} ({}) in {}", f.title, f.kind, f.place), now);
                }
            }
            None => self.warn("That node no longer exists.", now),
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
                    self.warn(format!("{} is no longer in the library", c.name), now);
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
        let id = self
            .doc
            .edit(&self.registry, &format!("Add {}", c.name), |g, ctx| {
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
            });
        if let Some(e) = error {
            self.warn(e, now);
        }
        match &fed {
            Some(Ok(n)) => self.say(
                format!(
                    "Added {} and wired the {n} selected nodes into it, in the order you selected \
                     them",
                    c.name
                ),
                now,
            ),
            Some(Err(e)) => {
                let why = self.feed_reason(e);
                self.warn(format!("Added {} without wiring: {why}", c.name), now);
            }
            None => {}
        }
        let id = id?;
        self.doc.selection = [id].into();
        self.canvas.mark_born(&[id], now);
        self.last_added = vec![id];
        Some(id)
    }

    fn paste(&mut self, text: &str, now: f64) {
        if ac_core::Subgraph::from_json(text).is_none() {
            return;
        }
        let at = self.canvas.pointer_canvas();
        let text = text.to_string();
        match self.doc.edit(&self.registry, "Paste", |g, ctx| {
            edit::paste(g, ctx, &text, [at.x, at.y])
        }) {
            Ok(ids) => {
                self.doc.selection = ids.iter().copied().collect();
                self.canvas.mark_born(&ids, now);
                self.say(
                    format!(
                        "Pasted {} node{}",
                        ids.len(),
                        if ids.len() == 1 { "" } else { "s" }
                    ),
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
        match a {
            Action::New => self.request(AfterConfirm::New, now),
            Action::Open => self.file_dialog(FilePurpose::Open, now),
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
                    self.say("Nothing to undo", now);
                }
            }
            Action::Redo => {
                if self.doc.redo() {
                    self.canvas.edit = None;
                } else {
                    self.say("Nothing to redo", now);
                }
            }
            Action::Copy => {
                let sel = self.selected_real();
                if !sel.is_empty() {
                    let text = edit::copy(self.doc.graph(), &self.doc.project.group_defs, &sel);
                    ctx.copy_text(text.clone());
                    self.clipboard = Some(text);
                    self.say(
                        format!(
                            "Copied {} node{}",
                            sel.len(),
                            if sel.len() == 1 { "" } else { "s" }
                        ),
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
                let new = self.doc.edit(&reg, "Duplicate", |g, ctx| {
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
                Ok(n) => self.say(format!("Laid out {n} nodes · Ctrl+Z undoes it"), now),
                Err(e) => self.say(e, now),
            },
            Action::ArrangeMenu => {
                if self.selected_real().len() >= 2 {
                    self.canvas.open_arrange_menu();
                } else {
                    self.say("Select two or more nodes to align or distribute", now);
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
                if let Some((inst, _)) = self.doc.edit(&reg, "Make group", |g, ctx| {
                    edit::group::make_group(g, ctx, &sel)
                }) {
                    self.doc.selection = [inst].into();
                    self.canvas.mark_born(&[inst], now);
                    let dropped = pins_before.saturating_sub(pin_count(&self.doc.project));
                    if dropped > 0 {
                        self.say(
                            format!(
                                "Grouped. {dropped} pinned node{} left the panels: nodes inside \
                                 groups can't be pinned (Ctrl+Z to undo).",
                                plural(dropped)
                            ),
                            now,
                        );
                    } else {
                        self.say("Grouped. Tab goes inside; Ctrl+Alt+G ungroups.", now);
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
                    self.say("Select a group to ungroup", now);
                    return;
                }
                let mut all = Vec::new();
                for g_id in groups {
                    let ungrouped = self.doc.edit(&reg, "Ungroup", |g, ctx| {
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
                    self.say("That node isn't a group", now);
                }
            }
            Action::Mute => self.toggle_nodes("Mute", |n| n.muted = !n.muted),
            Action::Collapse => self.toggle_nodes("Collapse", |n| n.ui.collapsed = !n.ui.collapsed),
            Action::HideUnwired => self.toggle_nodes("Hide unwired sockets", |n| {
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
                    .edit_project("Loop", |p| p.settings.loop_region = region);
            }
            Action::Rewind => self.engine.handle.seek(0),
            Action::PlayKeys => {
                let on = !self.play_keys.is_on();
                self.set_play_keys(on);
                if on {
                    let stop = Action::PlayKeys
                        .shortcut()
                        .map(|s| ctx.format_shortcut(&s))
                        .unwrap_or_default();
                    self.say(
                        format!(
                            "Play keys on: keys go to the patch ({stop} or the keyboard button \
                             turns it off)"
                        ),
                        now,
                    );
                } else {
                    self.say("Play keys off", now);
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
            self.say(
                "Only top-level nodes can be pinned: leave the group (Tab) first",
                now,
            );
            return;
        }
        let nodes: Vec<NodeId> = self
            .selected_real()
            .into_iter()
            .filter(|id| self.doc.project.graph.node(*id).is_some_and(pins::pinnable))
            .collect();
        if nodes.is_empty() {
            self.say(
                "Select a control or display to pin: a slider, knob, button, keyboard, meter, scope…",
                now,
            );
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
                    self.say("Only controls and displays can be pinned", now);
                    return;
                }
                let (index, added, name) = self.doc.edit_panels("Pin to panel", |panels, g| {
                    let index = match panel {
                        Some(i) if i < panels.len() => i,
                        _ => pins::add_panel(panels, ""),
                    };
                    let added = pins::pin(panels, g, index, &nodes);
                    let name = panels.get(index).map_or(String::new(), |p| p.name.clone());
                    (index, added, name)
                });
                self.perform.current = index;
                if added > 0 {
                    self.say(
                        format!(
                            "Pinned {added} node{} to {name} · Ctrl+Tab shows the panels",
                            plural(added)
                        ),
                        now,
                    );
                } else {
                    self.say(format!("Already pinned to {name}"), now);
                }
            }
            PinRequest::Unpin { nodes, panel } => {
                let from = panel
                    .and_then(|i| self.doc.project.panels.get(i))
                    .map_or("every panel".to_string(), |p| p.name.clone());
                let removed = self
                    .doc
                    .edit_panels("Unpin", |panels, _| pins::unpin(panels, panel, &nodes));
                self.say(
                    format!("Unpinned {removed} node{} from {from}", plural(removed)),
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
            AfterConfirm::New => {
                let p = files::default_project(&self.registry);
                self.replace_document(p, None, cwd(), AssetStore::default());
                self.say("New project", now);
            }
            AfterConfirm::Open(path) => self.open(&path, now),
            AfterConfirm::OpenExample(path) => self.open_example(&path, now),
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
                if let Err(e) = self.open_project(path, format!("Opened {}", path.display()), now) {
                    self.warn(format!("Couldn't open: {e}"), now);
                }
            }
            FileKind::Pack => match ac_io::unpack_beside(path) {
                Ok(project) => {
                    let dir = files::project_dir(&project);
                    let mut what = format!("Unpacked {} into {}", file_name(path), dir.display());
                    // Someone else's blocks: import them and point the
                    // copies at the imports, before the project opens.
                    if let Ok(mut p) = ac_io::load_project(&project) {
                        let adopted = self.library.adopt(&mut p);
                        if adopted > 0 && ac_io::save_project(&project, &p).is_ok() {
                            what.push_str(&format!(
                                " · added {adopted} block{} from it to your library",
                                plural(adopted)
                            ));
                        }
                    }
                    if let Err(e) = self.open_project(&project, what, now) {
                        self.warn(
                            format!(
                                "Unpacked into {}, but couldn't open the project: {e}",
                                dir.display()
                            ),
                            now,
                        );
                    }
                }
                Err(e) => self.warn(format!("Couldn't unpack {}: {e}", path.display()), now),
            },
            FileKind::Block => self.import_block_file(path, now),
        }
    }

    /// Replace the document with the project at `path`. The status starts
    /// with `what` and adds the blocks it brought and any missing files.
    fn open_project(&mut self, path: &Path, what: String, now: f64) -> Result<(), String> {
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
            text.push_str(&format!(
                " · added {blocks} block{} it uses to your library",
                plural(blocks)
            ));
        }
        if missing > 0 {
            text.push_str(&format!(
                " — {missing} audio file problem(s); see the Project tab"
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
                    format!(
                        "Opened the example “{name}” as a new project: Save As keeps your changes"
                    ),
                    now,
                );
            }
            Err(e) => self.warn(format!("Couldn't open the example: {e}"), now),
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
        self.engine.handle.stop();
        self.engine.handle.set_recording(false);
        self.engine.handle.seek(0);
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
                self.warn(format!("Couldn't copy audio files: {e}"), now);
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
                self.recent.add(&path);
                if assets_changed {
                    self.reload_assets();
                }
                let extra = if copied > 0 {
                    format!(
                        " (copied {copied} audio file{} into assets/)",
                        if copied == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                };
                self.say(format!("Saved {}{extra}", path.display()), now);
            }
            Err(e) => self.warn(format!("Couldn't save: {e}"), now),
        }
    }

    fn export(&mut self, path: PathBuf, now: f64) {
        let path = files::with_extension(path, "wav");
        let project = self.doc.project.clone();
        let registry = self.registry.clone();
        // Offline exports never record.
        let resources: Arc<dyn ac_core::Resources> = Arc::new(FileResources {
            assets: self.assets.clone(),
            recorder: None,
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(export_wav(&project, registry, resources, &p));
        });
        self.export_job = Some((path, rx));
        self.say("Exporting…", now);
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
            FilePurpose::Export => self.export(path, now),
            FilePurpose::ChooseClip(node) => self.assign_clip(node, &path, now),
            FilePurpose::Locate(old) => {
                match files::import_audio(&path, self.doc.path.as_deref()) {
                    Ok(a) => self.relink(&old, &a, now),
                    Err(e) => self.warn(e, now),
                }
            }
        }
    }

    /// Point the nodes using the missing file `old` at `a`.
    fn relink(&mut self, old: &str, a: &files::ImportedAsset, now: f64) {
        let reg = self.registry.clone();
        let n = self
            .doc
            .edit_project("Locate file", |p| files::relink_asset(p, &reg, old, a));
        self.reload_assets();
        self.say(
            format!("Relinked {n} node{}", if n == 1 { "" } else { "s" }),
            now,
        );
    }

    /// Give a Clip node a file (copied into the project's assets when the
    /// project has a home).
    pub fn assign_clip(&mut self, node: NodeId, path: &Path, now: f64) {
        match files::import_audio(path, self.doc.path.as_deref()) {
            Ok(a) => self.assign_clip_asset(node, a, now),
            Err(e) => self.warn(format!("Couldn't use that file: {e}"), now),
        }
    }

    /// Give a Clip node the audio file `a`.
    fn assign_clip_asset(&mut self, node: NodeId, a: files::ImportedAsset, now: f64) {
        let a2 = a.clone();
        let reg = self.registry.clone();
        let assign = |doc: &mut Document| {
            doc.edit(&reg, "Choose clip file", |g, _| {
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
            None => self.say(format!("Clip plays {}", a.file), now),
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
        dialogs::shortcuts(ctx, &mut self.show_shortcuts);
        dialogs::about(ctx, &mut self.show_about);
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

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
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
