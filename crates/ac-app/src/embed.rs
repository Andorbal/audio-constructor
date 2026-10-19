//! The editor in a window its host provides: a plugin's editor window
//! (ARCHITECTURE §13). It draws with egui-baseview's OpenGL renderer
//! (DECISIONS D125) into a child of the host's parent window.
//!
//! A plugin keeps one [`EditorApp`] for its whole life (made with
//! [`EditorApp::embedded`] and `AudioMode::Hosted`), wrapped by [`share`],
//! and opens an [`EditorWindow`] on it each time the host shows the editor.
//! The window locks the editor once a frame; the plugin locks it to save
//! and restore its state ([`EditorApp::project`],
//! [`EditorApp::take_changes`], [`EditorApp::load_project`]). Closing the
//! window (dropping it) leaves the editor, its document, and its undo
//! history as they were, ready for the next window.
//!
//! **Never close or drop a window while holding the editor's lock.** On X11
//! each window draws on its own thread and closing waits for that thread,
//! which may be waiting for the lock.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};

use egui_baseview::baseview::dpi::LogicalSize;
use egui_baseview::baseview::{self as bv, HandlerError};
use egui_baseview::{EguiWindow, EguiWindowSettings, Frame};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::app::EditorApp;
use crate::theme;

pub use egui_baseview::RepaintNotifier;
/// The windowing library the adapter uses, for a plugin to name the same
/// version of it (its `Window`, its host callbacks, `WindowSize`).
pub use egui_baseview::baseview;

/// The editor a plugin keeps and shares with each window it opens.
pub type SharedEditor = Arc<Mutex<EditorApp>>;

/// Share `editor` with the windows that show it.
pub fn share(editor: EditorApp) -> SharedEditor {
    Arc::new(Mutex::new(editor))
}

/// Lock a shared editor. A window whose frame panicked doesn't keep the
/// plugin from saving or showing the editor again.
pub fn lock(editor: &SharedEditor) -> MutexGuard<'_, EditorApp> {
    editor.lock().unwrap_or_else(|e| e.into_inner())
}

/// How to open a window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowOptions {
    /// The window's title, where the platform shows one (standalone).
    pub title: String,
    /// Width and height in points at 100 % UI scale; the editor's UI scale
    /// (View ▸ UI Scale) multiplies them.
    pub size: [f64; 2],
    /// The smallest size, in the same points.
    pub min_size: Option<[f64; 2]>,
    pub resizable: bool,
    /// The host's scale factor, for platforms that can't report one (X11
    /// without `Xft.dpi`, early Windows 10). The platform's wins otherwise.
    pub fallback_scale_factor: Option<f64>,
    /// Create the window before the host gives it a parent; it opens when
    /// [`EditorWindow::set_parent`] attaches it (or when it's shown).
    pub wait_for_parent: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: "Audio Constructor".into(),
            size: [1200.0, 760.0],
            min_size: Some([720.0, 480.0]),
            resizable: true,
            fallback_scale_factor: None,
            wait_for_parent: false,
        }
    }
}

/// Why a window couldn't open.
#[derive(Debug)]
pub enum EmbedError {
    /// The parent isn't a window this platform can hold a child in (X11
    /// Xlib or XCB, a Win32 HWND, or an AppKit NSView).
    UnsupportedParent(String),
    /// The parent's handle couldn't be read.
    Handle(raw_window_handle::HandleError),
    /// The windowing system refused.
    Window(bv::Error),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::UnsupportedParent(kind) => {
                write!(f, "can't open the editor inside a {kind} window here")
            }
            EmbedError::Handle(e) => write!(f, "the parent window's handle: {e}"),
            EmbedError::Window(e) => write!(f, "the editor window: {e}"),
        }
    }
}

impl std::error::Error for EmbedError {}

impl From<bv::Error> for EmbedError {
    fn from(e: bv::Error) -> Self {
        EmbedError::Window(e)
    }
}

/// An open editor window. Dropping it (or [`EditorWindow::close`]) closes
/// the window and frees what it drew with; the editor stays.
pub struct EditorWindow {
    window: bv::Window,
    repaint: RepaintNotifier,
}

impl EditorWindow {
    /// Open the editor inside `parent`, the host's window. Returns at once:
    /// the window draws from the platform's event loop (on X11, its own
    /// thread).
    pub fn open_parented<P: HasWindowHandle>(
        parent: &P,
        editor: &SharedEditor,
        opts: &WindowOptions,
    ) -> Result<EditorWindow, EmbedError> {
        let w = EditorWindow::create(editor, opts, Some(parent), None)?;
        w.show()?;
        Ok(w)
    }

    /// Open the editor in a top-level window of its own, with no host (to
    /// try the adapter without a plugin host; see `examples/embedded.rs`).
    pub fn open_standalone(
        editor: &SharedEditor,
        opts: &WindowOptions,
    ) -> Result<EditorWindow, EmbedError> {
        let w = EditorWindow::create::<bv::WindowContext>(editor, opts, None, None)?;
        w.show()?;
        Ok(w)
    }

    /// Create the window without showing it, for host APIs that show,
    /// hide, and attach it separately. `parent` is the host's window, or
    /// `None` with [`WindowOptions::wait_for_parent`] (attach it later with
    /// [`EditorWindow::set_parent`]) or for a top-level window. `host`
    /// carries the host's callbacks (resize requests, the main-thread hop
    /// X11 needs).
    pub fn create<P: HasWindowHandle>(
        editor: &SharedEditor,
        opts: &WindowOptions,
        parent: Option<&P>,
        host: Option<bv::host::Host>,
    ) -> Result<EditorWindow, EmbedError> {
        EditorWindow::create_hooked(editor, opts, parent, host, Box::new(NoHook))
    }

    /// [`EditorWindow::create`] with `hook` around every frame: a plugin
    /// lends the editor its engine there and takes its edits (see
    /// [`WindowHook`]).
    pub fn create_hooked<P: HasWindowHandle>(
        editor: &SharedEditor,
        opts: &WindowOptions,
        parent: Option<&P>,
        host: Option<bv::host::Host>,
        hook: Box<dyn WindowHook>,
    ) -> Result<EditorWindow, EmbedError> {
        let mut settings = settings_for(opts, lock(editor).settings().prefs.ui_scale);
        if let Some(p) = parent {
            settings.parent = Some(parent_handle(p)?);
        }
        let repaint = RepaintNotifier::new();
        settings.repaint_notifier = Some(repaint.clone());
        let app = WindowApp::new(editor.clone(), hook, settings.zoom_factor);
        let window = EguiWindow::create_with_host(settings, app, host)?;
        Ok(EditorWindow { window, repaint })
    }

    pub fn show(&self) -> Result<(), EmbedError> {
        self.window.show()?;
        // The first frame may not follow on its own (see nice-plug-egui).
        self.repaint.request_repaint();
        Ok(())
    }

    pub fn hide(&self) -> Result<(), EmbedError> {
        Ok(self.window.hide()?)
    }

    /// Attach to the host's window (after [`WindowOptions::wait_for_parent`]).
    pub fn set_parent<P: HasWindowHandle>(&self, parent: &P) -> Result<(), EmbedError> {
        let handle = parent_handle(parent)?;
        Ok(self.window.set_parent(handle)?)
    }

    /// Resize to `size` points at 100 % UI scale (the host resized its
    /// window).
    pub fn resize(&self, editor: &SharedEditor, size: [f64; 2]) -> Result<(), EmbedError> {
        let scale = f64::from(lock(editor).settings().prefs.ui_scale);
        Ok(self
            .window
            .resize(LogicalSize::new(size[0] * scale, size[1] * scale))?)
    }

    /// The window's size now, in physical and logical pixels.
    pub fn size(&self) -> bv::WindowSize {
        self.window.size()
    }

    /// Whether the window is still open (the platform or the host can close
    /// it first).
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// Ask for a redraw from any thread, real-time safe (the plugin
    /// restored state, or the host moved a parameter).
    pub fn repaint_notifier(&self) -> RepaintNotifier {
        self.repaint.clone()
    }

    /// Run work the window's thread scheduled for the main thread (X11),
    /// when the host's main-thread caller says to.
    pub fn host_main_thread_callback(&self) {
        self.window.host_main_thread_callback();
    }

    /// Block until the window closes (a standalone window, as in the
    /// example).
    pub fn run_until_closed(self) -> Result<(), EmbedError> {
        Ok(self.window.run_until_closed()?)
    }

    /// Close the window now. Not while holding the editor's lock.
    pub fn close(self) {
        drop(self);
    }

    /// The window itself, for a host API that holds it (and closes it by
    /// dropping it), with the notifier that wakes it.
    pub fn into_parts(self) -> (bv::Window, RepaintNotifier) {
        (self.window, self.repaint)
    }
}

/// The settings egui-baseview opens with: `opts`, at the editor's UI scale
/// already, so the editor's first frame doesn't resize the window.
fn settings_for(opts: &WindowOptions, ui_scale: f32) -> EguiWindowSettings {
    let size = |s: [f64; 2]| LogicalSize::new(s[0].max(1.0), s[1].max(1.0));
    EguiWindowSettings::new()
        .with_title(opts.title.clone())
        .with_size(size(opts.size))
        .with_min_size(opts.min_size.map(size))
        .with_resizable(opts.resizable)
        .with_zoom_factor(crate::settings::clamp_scale(ui_scale))
        .with_fallback_scale_factor(opts.fallback_scale_factor)
        .with_wait_for_parent(opts.wait_for_parent)
}

/// The parent as baseview takes it, refusing kinds this platform can't
/// hold a child in (baseview would panic on those).
pub fn parent_handle<P: HasWindowHandle>(parent: &P) -> Result<bv::ParentWindowHandle, EmbedError> {
    let raw = parent.window_handle().map_err(EmbedError::Handle)?.as_raw();
    check_parent(raw)?;
    Ok(bv::ParentWindowHandle::from_window(parent))
}

fn check_parent(raw: RawWindowHandle) -> Result<(), EmbedError> {
    let ok = match raw {
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xlib(h) => h.window != 0 && u32::try_from(h.window).is_ok(),
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xcb(_) => true,
        #[cfg(target_os = "windows")]
        RawWindowHandle::Win32(_) => true,
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(_) => true,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(EmbedError::UnsupportedParent(kind_of(raw).into()))
    }
}

fn kind_of(raw: RawWindowHandle) -> &'static str {
    match raw {
        RawWindowHandle::Xlib(_) => "Xlib",
        RawWindowHandle::Xcb(_) => "XCB",
        RawWindowHandle::Wayland(_) => "Wayland",
        RawWindowHandle::Win32(_) => "Win32",
        RawWindowHandle::WinRt(_) => "WinRT",
        RawWindowHandle::AppKit(_) => "AppKit",
        RawWindowHandle::UiKit(_) => "UIKit",
        RawWindowHandle::Web(_) | RawWindowHandle::WebCanvas(_) => "web",
        RawWindowHandle::AndroidNdk(_) => "Android",
        _ => "unknown",
    }
}

/// What a host does around each frame of its editor window, on the
/// window's thread with the editor locked. A plugin whose session keeps
/// the project lends the editor its engine handle for `draw` and takes the
/// edits afterwards ([`EditorApp::host_keeps_project`]).
pub trait WindowHook: Send + 'static {
    /// Run `draw`, the editor's frame, once. `ctx` is the window's (to ask
    /// for another frame later, say).
    fn frame(
        &mut self,
        ctx: &egui::Context,
        editor: &mut EditorApp,
        draw: &mut dyn FnMut(&mut EditorApp),
    ) {
        let _ = ctx;
        draw(editor);
    }

    /// The window's size changed (the host resized it, or the editor's UI
    /// scale did), in the platform's terms: physical pixels and the
    /// system's scale factor, not counting the UI scale.
    fn resized(&mut self, size: bv::WindowSize) {
        let _ = size;
    }
}

/// No host work around frames.
struct NoHook;

impl WindowHook for NoHook {}

/// One window's view of the shared editor.
struct WindowApp {
    editor: SharedEditor,
    hook: Box<dyn WindowHook>,
    /// Set when the editor panicked in this window: the window says so
    /// instead of drawing it again (the host keeps running).
    failed: Option<String>,
    /// The modifier keys held, followed through each frame's events.
    modifiers: egui::Modifiers,
    /// The UI scale egui-baseview draws at (its zoom factor).
    zoom: f32,
}

impl WindowApp {
    fn new(editor: SharedEditor, hook: Box<dyn WindowHook>, zoom: f32) -> WindowApp {
        WindowApp {
            editor,
            hook,
            failed: None,
            modifiers: egui::Modifiers::NONE,
            zoom,
        }
    }

    /// Draw one frame; the color to clear the window with.
    fn frame(&mut self, ui: &mut egui::Ui) -> egui::Rgba {
        let mods = &mut self.modifiers;
        ui.ctx().input_mut(|i| recover_keys(&mut i.events, mods));
        self.frame_with(ui, EditorApp::ui)
    }

    /// A frame drawn by `draw` (the editor's, or a test's). A panic in it
    /// is caught: unwinding into the host would take the host down.
    fn frame_with(
        &mut self,
        ui: &mut egui::Ui,
        draw: impl FnOnce(&mut EditorApp, &mut egui::Ui),
    ) -> egui::Rgba {
        if self.failed.is_none() {
            let editor = &self.editor;
            let hook = &mut self.hook;
            let ctx = ui.ctx().clone();
            let drawn = catch_unwind(AssertUnwindSafe(|| {
                let mut draw = Some(draw);
                hook.frame(&ctx, &mut lock(editor), &mut |e| {
                    if let Some(d) = draw.take() {
                        d(e, ui);
                    }
                });
            }));
            if let Err(panic) = drawn {
                let why = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                self.failed = Some(why);
            }
        }
        if let Some(why) = &self.failed {
            failed(ui, why);
        }
        egui::Rgba::from(theme::canvas_bg())
    }
}

/// What a window shows after the editor failed in it.
fn failed(ui: &mut egui::Ui, why: &str) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("The editor stopped");
            ui.label("Your project is safe with the plugin. Close and reopen this window.");
            if !why.is_empty() {
                ui.label(egui::RichText::new(why).small().color(theme::text_muted()));
            }
        });
    });
}

/// Key presses egui-baseview 0.9 drops, recovered from the text they typed.
///
/// It turns only unshifted letters, digits, and a few named keys into key
/// events, so Shift+A (add node), Shift+D (duplicate), `.` (frame the
/// selection), `+` and `-` (zoom) would reach the editor as text alone.
/// Each such character typed without Ctrl or Cmd becomes the key press it
/// came from, with the modifiers held at the time. What leaves no text
/// can't be recovered: function keys, and Ctrl or Cmd with Shift or a
/// symbol (their menu entries still work).
fn recover_keys(events: &mut Vec<egui::Event>, modifiers: &mut egui::Modifiers) {
    let mut out = Vec::with_capacity(events.len());
    for e in events.drain(..) {
        match &e {
            egui::Event::ModifiersChanged(m) => *modifiers = *m,
            egui::Event::WindowFocused(false) => *modifiers = egui::Modifiers::NONE,
            egui::Event::Text(t) => {
                let mut chars = t.chars();
                let key = match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_uppercase() || c.is_ascii_punctuation() => {
                        egui::Key::from_name(t)
                    }
                    _ => None,
                };
                let sent = |k: egui::Key| {
                    out.iter().any(
                        |o| matches!(o, egui::Event::Key { key, pressed: true, .. } if *key == k),
                    )
                };
                if let Some(k) = key.filter(|k| !sent(*k)) {
                    out.push(egui::Event::Key {
                        key: k,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: *modifiers,
                    });
                }
            }
            _ => {}
        }
        out.push(e);
    }
    *events = out;
}

impl egui_baseview::App for WindowApp {
    fn build(&mut self, _ctx: egui::Context, _frame: &mut Frame) -> Result<(), HandlerError> {
        // The editor styles a new context on its first frame.
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        let clear = self.frame(ui);
        frame.set_clear_color(clear);
    }

    fn resized(&mut self, size: bv::WindowSize) {
        // egui-baseview reports the scale with its zoom (the UI scale) in.
        let system = size.scale_factor / f64::from(self.zoom.max(0.01));
        self.hook
            .resized(bv::WindowSize::from_physical(size.physical, system));
    }

    fn zoom_factor_changed(&mut self, zoom_factor: f32) {
        self.zoom = zoom_factor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppOptions;
    use crate::audio::{AudioMode, HostAudio};
    use raw_window_handle::{WaylandWindowHandle, WebWindowHandle};

    #[test]
    fn parents_this_platform_cant_hold_are_refused_before_baseview_panics() {
        let surface = std::ptr::NonNull::<std::ffi::c_void>::dangling();
        let wayland = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface));
        let web = RawWindowHandle::Web(WebWindowHandle::new(1));
        for raw in [wayland, web] {
            match check_parent(raw) {
                Err(e @ EmbedError::UnsupportedParent(_)) => {
                    assert!(
                        e.to_string().contains("can't open the editor inside"),
                        "{e}"
                    );
                }
                other => panic!("{raw:?} accepted: {other:?}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::XlibWindowHandle;
            let zero = RawWindowHandle::Xlib(XlibWindowHandle::new(0));
            assert!(check_parent(zero).is_err(), "no window has XID 0");
            let wide = RawWindowHandle::Xlib(XlibWindowHandle::new(1 << 40));
            assert!(check_parent(wide).is_err(), "XIDs are 32 bits");
            assert!(check_parent(RawWindowHandle::Xlib(XlibWindowHandle::new(42))).is_ok());
        }
    }

    #[test]
    fn shifted_letters_and_symbols_become_key_presses_again() {
        use egui::{Event, Key, Modifiers};
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let text = |t: &str| Event::Text(t.into());
        let mut mods = Modifiers::NONE;
        let mut events = vec![
            Event::ModifiersChanged(Modifiers::SHIFT),
            text("A"),
            Event::ModifiersChanged(Modifiers::NONE),
            text("+"),
            // Already a key event: left alone.
            key(Key::G, Modifiers::NONE),
            text("g"),
            // Not keys.
            text("é"),
            text("ab"),
        ];
        recover_keys(&mut events, &mut mods);
        assert_eq!(
            events,
            vec![
                Event::ModifiersChanged(Modifiers::SHIFT),
                key(Key::A, Modifiers::SHIFT),
                text("A"),
                Event::ModifiersChanged(Modifiers::NONE),
                key(Key::Plus, Modifiers::NONE),
                text("+"),
                key(Key::G, Modifiers::NONE),
                text("g"),
                text("é"),
                text("ab"),
            ]
        );
        // Held modifiers carry over to the next frame, and losing focus
        // lets go of them.
        let mut events = vec![Event::ModifiersChanged(Modifiers::SHIFT)];
        recover_keys(&mut events, &mut mods);
        let mut events = vec![text("D")];
        recover_keys(&mut events, &mut mods);
        assert_eq!(events[0], key(Key::D, Modifiers::SHIFT));
        let mut events = vec![Event::WindowFocused(false), text(".")];
        recover_keys(&mut events, &mut mods);
        assert_eq!(events[1], key(Key::Period, Modifiers::NONE));
    }

    #[test]
    fn windows_open_at_the_editors_ui_scale() {
        let opts = WindowOptions {
            size: [1000.0, 0.0],
            ..WindowOptions::default()
        };
        let s = settings_for(&opts, 1.25);
        assert_eq!(s.zoom_factor, 1.25);
        assert!(s.parent.is_none() && !s.wait_for_parent && s.resizable);
        // Sizes stay positive; egui-baseview scales them by the zoom.
        assert_eq!(
            s.size,
            bv::dpi::Size::Logical(LogicalSize::new(1000.0, 1.0))
        );
        assert_eq!(settings_for(&opts, f32::NAN).zoom_factor, 1.0);
        assert_eq!(settings_for(&opts, 9.0).zoom_factor, 2.0);
    }

    fn hosted() -> SharedEditor {
        share(EditorApp::embedded(AppOptions {
            audio: AudioMode::Hosted(HostAudio::default()),
            ..AppOptions::headless()
        }))
    }

    /// One frame of `ctx`, its output handled as a renderer would.
    fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
        let mut out = ctx.run_ui(egui::RawInput::default(), draw);
        out.textures_delta.clear();
    }

    #[test]
    fn a_window_draws_the_shared_editor_frame_by_frame() {
        let editor = hosted();
        let mut app = WindowApp::new(editor.clone(), Box::new(NoHook), 1.0);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            frame(&ctx, |ui| {
                let clear = app.frame(ui);
                assert_eq!(clear, egui::Rgba::from(theme::canvas_bg()));
            });
        }
        assert!(app.failed.is_none());
        // Nothing holds the editor between frames: the plugin can save.
        assert!(editor.try_lock().is_ok());
        assert!(lock(&editor).take_changes().is_none());
    }

    #[test]
    fn a_panic_in_a_frame_stays_in_its_window() {
        let editor = hosted();
        let mut app = WindowApp::new(editor.clone(), Box::new(NoHook), 1.0);
        let ctx = egui::Context::default();
        let mut drawn = 0;
        for _ in 0..3 {
            frame(&ctx, |ui| {
                app.frame_with(ui, |_, _| {
                    drawn += 1;
                    panic!("a frame went wrong");
                });
            });
        }
        assert_eq!(drawn, 1, "a failed window stops drawing the editor");
        assert_eq!(app.failed.as_deref(), Some("a frame went wrong"));
        // The lock is poisoned, but the plugin still reaches its project
        // and a new window draws the editor again.
        assert!(editor.is_poisoned());
        assert!(lock(&editor).is_hosted());
        let mut again = WindowApp::new(editor.clone(), Box::new(NoHook), 1.0);
        frame(&ctx, |ui| {
            again.frame(ui);
        });
        assert!(again.failed.is_none());
    }
}
