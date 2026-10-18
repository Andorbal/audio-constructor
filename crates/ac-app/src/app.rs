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
use crate::audio::{AudioMode, EngineHost};
use crate::canvas::{Canvas, CanvasEvent, CanvasIo, PendingWire};
use crate::dialogs::{self, Confirm, FilePurpose, PathPrompt};
use crate::document::Document;
use crate::edit::{self, HIDE_UNWIRED};
use crate::export::{ExportSummary, export_wav};
use crate::files::{self, Recent};
use crate::icons;
use crate::menubar::{self, MenuCommand, MenuState};
use crate::palette::{Candidate, Palette};
use crate::panels::{PanelIo, PanelRequest, SidePanel, Tab};
use crate::play_keys::{KeyInput, PlayKeys};
use crate::theme;
use crate::transport::TransportState;

mod sharing;

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
        }
    }
}

/// Something waiting for the user to confirm discarding changes.
#[derive(Clone, Debug, PartialEq)]
enum AfterConfirm {
    New,
    Open(PathBuf),
    Quit,
}

pub struct EditorApp {
    pub registry: Arc<Registry>,
    pub doc: Document,
    pub canvas: Canvas,
    pub palette: Palette,
    pub engine: EngineHost,
    pub side: SidePanel,
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
    /// The computer keys as a control source (see `play_keys.rs`).
    play_keys: PlayKeys,
}

impl EditorApp {
    pub fn new(ctx: &egui::Context, opts: AppOptions) -> EditorApp {
        theme::apply(ctx);
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
            play_keys: PlayKeys::default(),
        };
        app.canvas.reset_for(&app.doc, &app.registry);
        if let Some(p) = later {
            app.open(&p, 0.0);
        }
        app.report_library_issues();
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

    /// The status bar's message, if one is showing (tests read this).
    pub fn status(&self) -> Option<&str> {
        self.status.as_ref().map(|(text, _, _)| text.as_str())
    }

    fn say(&mut self, text: impl Into<String>, now: f64) {
        self.status = Some((text.into(), now, false));
    }

    fn warn(&mut self, text: impl Into<String>, now: f64) {
        self.status = Some((text.into(), now, true));
    }

    // ------------------------------------------------------------ frame

    /// Draw the whole editor into `ui` (the window's root, or a test harness).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        // The app may be drawn into a context other than the one it was
        // created with (a test harness); give that one the look too.
        let themed = egui::Id::unique("audio-constructor-theme");
        if !ctx.data(|d| d.get_temp::<bool>(themed).unwrap_or(false)) {
            theme::apply(&ctx);
            ctx.data_mut(|d| d.insert_temp(themed, true));
        }
        // The canvas is keyboard-driven: with nothing focused, Tab and the
        // arrow keys belong to it, not to egui's focus navigation.
        if ctx.memory(|m| m.focused().is_none()) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        self.handle_keys(&ctx, now);

        egui::Panel::top("menu-bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| {
                let state = MenuState {
                    undo: self.doc.undo_label(),
                    redo: self.doc.redo_label(),
                    has_selection: !self.doc.selection.is_empty(),
                    side_panel: self.side.open,
                    timing: self.show_timing,
                    playing: self.engine.handle.is_playing(),
                    looping: self.doc.project.settings.loop_region.is_some(),
                    play_keys: self.play_keys.is_on(),
                    recent: &self.recent,
                    registry: &self.registry,
                };
                let cmds = menubar::show(ui, &state);
                for c in cmds {
                    self.menu_command(&ctx, c, now);
                }
            });
        egui::Panel::top("transport")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .exact_size(44.0)
            .show(ui, |ui| {
                let play_keys = self.play_keys.is_on();
                let actions = self
                    .transport
                    .show(ui, &mut self.doc, &self.engine, play_keys, now);
                for a in actions {
                    self.run(&ctx, a, now);
                }
            });
        egui::Panel::bottom("status-bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
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
                        .fill(theme::PANEL_BG)
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
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::CANVAS_BG))
            .show(ui, |ui| {
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
                    show_timing: self.show_timing,
                    events: Vec::new(),
                };
                self.canvas.show(ui, &mut io);
                let events = std::mem::take(&mut io.events);
                for e in events {
                    self.canvas_event(&ctx, e, now);
                }
            });
        self.library_drop(&ctx, now);

        if let Some(p) = self
            .palette
            .show(&ctx, &self.registry, &self.library, &self.doc)
        {
            self.add_candidate(&p.candidate, p.at, p.pending, now);
        }
        self.dialogs(&ctx, now);
        self.finish_frame(&ctx, now);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.horizontal_centered(|ui| {
            // A recorded take waits here until it's made into a Clip.
            if let Some(take) = self.takes.first().cloned() {
                ui.label(
                    RichText::new(format!(
                        "{}  Recorded {}",
                        icons::RECORD,
                        take.relative_path
                    ))
                    .small()
                    .color(theme::TEXT),
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
                            .color(theme::TEXT_MUTED),
                    );
                }
            } else if let Some((text, t, warn)) = &self.status {
                if now - t < 8.0 || *t == 0.0 {
                    let color = if *warn { theme::WARNING } else { theme::TEXT };
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
                    .color(theme::ACCENT),
                );
            } else {
                let hint = if self.doc.selection.is_empty() {
                    "Shift+A add · drag from a socket to wire · double-click empty canvas to search"
                } else {
                    "G move · Shift+D duplicate · X delete · Ctrl+G group · M mute · H collapse"
                };
                ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = if self.engine.is_running() {
                    icons::SPEAKER_HIGH
                } else {
                    icons::SPEAKER_SLASH
                };
                let text = format!("{icon} {}", self.engine.describe());
                let r = ui.add(
                    egui::Button::new(RichText::new(text).small().color(theme::TEXT_MUTED))
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
                                .color(if errors { theme::ERROR } else { theme::WARNING }),
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

    fn finish_frame(&mut self, ctx: &egui::Context, now: f64) {
        // The engine always runs what the document says.
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

        let title = format!(
            "{}{} — Audio Constructor",
            if self.doc.is_dirty() { "● " } else { "" },
            self.doc.title()
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.engine.handle.is_playing() {
            ctx.request_repaint();
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
        self.assets = AssetStore::load_for(&self.doc.project, &self.doc.base_dir, &self.registry);
        self.missing = missing_of(&self.assets);
        self.loaded_assets = ac_io::assets::referenced_assets(&self.doc.project, &self.registry);
        let resources = self.resources();
        self.engine.set_resources(&self.doc.project, resources);
        self.engine.synced_revision = self.doc.engine_revision;
    }

    /// Assets and, for a saved project, the recorder writing into its
    /// assets folder (recreated when the project moves).
    fn resources(&mut self) -> Arc<FileResources> {
        let dir = self.doc.path.as_deref().map(files::project_dir);
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
        let typing =
            ctx.egui_wants_keyboard_input() || self.canvas.is_typing() || self.palette.open;
        let modal = self.confirm.is_some() || self.prompt.is_some();
        // Play keys: held keys come up when typing starts, a dialog opens,
        // or the window loses focus, so nothing is left down in the patch.
        let focused = ctx.input(|i| i.focused);
        let play = self.play_keys.is_on() && !typing && !modal && focused;
        if self.play_keys.is_on() && !play {
            self.play_keys.release_all(&mut self.engine);
        }
        let events = ctx.input(|i| i.events.clone());
        let mut played = Vec::new();
        for (n, e) in events.into_iter().enumerate() {
            if play
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
                        if self.palette.open {
                            continue; // the palette handles it
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
                        Key::Plus | Key::Equals if !modifiers.command && !modifiers.alt => {
                            Some(Action::ZoomIn)
                        }
                        _ => actions::for_key(key, modifiers),
                    };
                    let Some(a) = action else { continue };
                    if typing && !a.works_while_typing() {
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
            } => self.palette.open_at(screen, canvas, pending),
            CanvasEvent::ChooseClipFile(node) => {
                self.file_dialog(FilePurpose::ChooseClip(node), now)
            }
            CanvasEvent::ShowHelp(node) => {
                self.doc.selection = [node].into();
                self.side.open = true;
                self.side.tab = Tab::Node;
            }
            CanvasEvent::Status(s) => self.warn(s, now),
            CanvasEvent::Run(a) => self.run(ctx, a, now),
        }
    }

    fn panel_request(&mut self, ctx: &egui::Context, r: PanelRequest, now: f64) {
        let _ = ctx;
        match r {
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
                    self.canvas.focus_node(&self.doc, &self.registry, id);
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
            MenuCommand::Add(c) => {
                let at = self.canvas.pointer_canvas();
                self.add_candidate(&c, at, None, now);
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
        let config = c.config.clone();
        let socket = c.socket.clone();
        let mut error = None;
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
                Some(id)
            });
        if let Some(e) = error {
            self.warn(e, now);
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
                self.palette.open_at(screen, at, None);
            }
            Action::Frame => self.canvas.frame_selection(&mut self.doc, &reg),
            Action::Group => {
                let sel = self.selected_real();
                if sel.is_empty() {
                    return;
                }
                if let Some((inst, _)) = self.doc.edit(&reg, "Make group", |g, ctx| {
                    edit::group::make_group(g, ctx, &sel)
                }) {
                    self.doc.selection = [inst].into();
                    self.canvas.mark_born(&[inst], now);
                    self.say("Grouped. Tab goes inside; Ctrl+Alt+G ungroups.", now);
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
                    if let Some(new) = self
                        .doc
                        .edit(&reg, "Ungroup", |g, ctx| edit::group::ungroup(g, ctx, g_id))
                    {
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
                self.side.open = true;
                self.side.tab = Tab::Node;
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
            Action::ToggleSidePanel => self.side.open = !self.side.open,
            Action::ShowTiming => {
                self.show_timing = !self.show_timing;
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
            Action::Shortcuts => self.show_shortcuts = true,
            Action::About => self.show_about = true,
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

    /// Show `project` as a new, unsaved document (examples, tests).
    pub fn open_unsaved(&mut self, project: Project) {
        let assets = AssetStore::load_for(&project, &cwd(), &self.registry);
        self.replace_document(project, None, cwd(), assets);
    }

    fn replace_document(
        &mut self,
        project: Project,
        path: Option<PathBuf>,
        base: PathBuf,
        assets: AssetStore,
    ) {
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
        self.canvas.reset_for(&self.doc, &self.registry);
    }

    fn save(&mut self, save_as: bool, now: f64) {
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
                    Ok(a) => {
                        let reg = self.registry.clone();
                        let n = self.doc.edit_project("Locate file", |p| {
                            files::relink_asset(p, &reg, &old, &a)
                        });
                        self.reload_assets();
                        self.say(
                            format!("Relinked {n} node{}", if n == 1 { "" } else { "s" }),
                            now,
                        );
                    }
                    Err(e) => self.warn(e, now),
                }
            }
        }
    }

    /// Give a Clip node a file (copied into the project's assets when the
    /// project has a home).
    pub fn assign_clip(&mut self, node: NodeId, path: &Path, now: f64) {
        match files::import_audio(path, self.doc.path.as_deref()) {
            Ok(a) => {
                let a2 = a.clone();
                self.doc.edit(&self.registry, "Choose clip file", |g, _| {
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
                });
                self.reload_assets();
                match a.note {
                    Some(n) => self.say(n, now),
                    None => self.say(format!("Clip plays {}", a.file), now),
                }
            }
            Err(e) => self.warn(format!("Couldn't use that file: {e}"), now),
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

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        EditorApp::ui(self, ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::CANVAS_BG.to_normalized_gamma_f32()
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
