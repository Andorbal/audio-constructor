//! The plugin window for the framework: an [`Editor`] that opens the
//! shared editor in the host's window through `ac_app::embed`, resizable,
//! and the handle the host resizes, shows, hides, and closes it with.

use std::error::Error;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ac_app::embed::baseview::{self, HandlerError, WindowSize};
use ac_app::embed::{self, EditorWindow, EmbedError, RepaintNotifier, SharedEditor, WindowOptions};
use ac_app::{AppOptions, EditorApp, HostAudio};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::{LogicalSize, NativeSize, Size};
use nice_plug::editor::{
    Editor, EditorHandle, HostMethods, ParentWindowHandle, ResizeHint, SpawnedEditor,
};

use super::EditorContext;
use super::frame::{self, PUSH_EVERY, UNSEEN};
use super::host_macros::PluginMacros;
use crate::session::{Kind, Shared};

/// The window's size before anyone resized it, in points at 100 %.
pub(crate) const DEFAULT_SIZE: [f64; 2] = [1200.0, 760.0];
/// The smallest the window gets, in points at 100 %.
pub(crate) const MIN_SIZE: [f64; 2] = [720.0, 480.0];

/// The window's size as the host knows it: logical pixels and the
/// system's scale factor (the UI scale is part of the logical size).
#[derive(Debug)]
pub(crate) struct SizeState(Mutex<(LogicalSize<f64>, f64)>);

impl SizeState {
    fn new() -> SizeState {
        SizeState(Mutex::new((
            LogicalSize::new(DEFAULT_SIZE[0], DEFAULT_SIZE[1]),
            1.0,
        )))
    }

    fn get(&self) -> (LogicalSize<f64>, f64) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, logical: LogicalSize<f64>, scale: f64) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = (logical, scale);
    }

    fn native(&self) -> NativeSize<u32> {
        let (logical, scale) = self.get();
        NativeSize::from_size(Size::Logical(logical), scale)
    }
}

/// The plugin's editor: one per instance, made when the host asks.
pub struct AcEditor {
    context: EditorContext,
    /// The editor itself, made the first time the window opens.
    editor: OnceLock<SharedEditor>,
    /// The session revision the editor's document came from.
    seen: Arc<AtomicU64>,
    size: Arc<SizeState>,
}

impl AcEditor {
    pub(crate) fn new(context: EditorContext) -> AcEditor {
        AcEditor {
            context,
            editor: OnceLock::new(),
            seen: Arc::new(AtomicU64::new(UNSEEN)),
            size: Arc::new(SizeState::new()),
        }
    }

    /// The shared editor, made with the person's settings, recent files,
    /// and block library the first time.
    fn editor(&self, api: &str) -> SharedEditor {
        self.editor
            .get_or_init(|| {
                let kind = self.context.shared.lock().kind();
                let audio = HostAudio {
                    host: format!("{api} host"),
                    in_channels: if kind == Kind::Effect { 2 } else { 0 },
                    ..HostAudio::default()
                };
                editor_for(AppOptions::hosted(audio))
            })
            .clone()
    }
}

/// An editor whose project the plugin's session keeps.
pub(crate) fn editor_for(opts: AppOptions) -> SharedEditor {
    let mut editor = EditorApp::embedded(opts);
    editor.host_keeps_project();
    embed::share(editor)
}

fn resize_hint() -> ResizeHint {
    ResizeHint::resizable().with_min_logical_size(nice_plug::editor::dpi::LogicalSize::new(
        MIN_SIZE[0] as f32,
        MIN_SIZE[1] as f32,
    ))
}

impl Editor for AcEditor {
    type Handle = AcEditorHandle;

    fn spawn(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        gui_context: GuiContext,
        host: Option<HostMethods>,
    ) -> Result<SpawnedEditor<Self::Handle>, Box<dyn Error>> {
        let editor = self.editor(&gui_context.plugin_api().to_string());
        let zoom = {
            let mut e = embed::lock(&editor);
            e.set_host_controls(Box::new(PluginMacros::new(
                self.context.params.clone(),
                self.context.shared.labels().clone(),
                gui_context,
            )));
            f64::from(e.settings().prefs.ui_scale)
        };
        // The window opens at the size the host knows; egui-baseview
        // scales the size it's given by the UI scale.
        let (logical, _) = self.size.get();
        let opts = WindowOptions {
            title: "Audio Constructor".into(),
            size: [logical.width / zoom, logical.height / zoom],
            min_size: Some(MIN_SIZE),
            resizable: true,
            fallback_scale_factor,
            wait_for_parent,
        };
        let hook = SessionHook {
            shared: self.context.shared.clone(),
            seen: self.seen.clone(),
            last_push: None,
            size: self.size.clone(),
        };
        let window = EditorWindow::create_hooked(
            &editor,
            &opts,
            parent.as_ref(),
            host.map(host_of),
            Box::new(hook),
        )?;
        let (window, repaint) = window.into_parts();
        Ok(SpawnedEditor {
            handle: AcEditorHandle {
                editor,
                shared: self.context.shared.clone(),
                size: self.size.clone(),
                repaint,
            },
            window,
        })
    }

    fn size(&self) -> NativeSize<u32> {
        self.size.native()
    }

    fn resize_hint(&self) -> ResizeHint {
        resize_hint()
    }
}

/// The session's side of every frame, on the window's thread.
struct SessionHook {
    shared: Arc<Shared>,
    seen: Arc<AtomicU64>,
    last_push: Option<Instant>,
    size: Arc<SizeState>,
}

impl embed::WindowHook for SessionHook {
    fn frame(
        &mut self,
        ctx: &egui::Context,
        editor: &mut EditorApp,
        draw: &mut dyn FnMut(&mut EditorApp),
    ) {
        let outcome = {
            let mut session = self.shared.lock();
            frame::frame(&mut session, editor, &self.seen, &mut self.last_push, |e| {
                draw(e)
            })
        };
        if outcome.asked {
            ctx.request_repaint();
        } else if outcome.pending {
            ctx.request_repaint_after(PUSH_EVERY);
        }
    }

    fn resized(&mut self, size: WindowSize) {
        self.size.set(
            size.physical.to_logical(size.scale_factor),
            size.scale_factor,
        );
    }
}

/// The open window, as the host holds it.
pub struct AcEditorHandle {
    editor: SharedEditor,
    shared: Arc<Shared>,
    size: Arc<SizeState>,
    repaint: RepaintNotifier,
}

impl Drop for AcEditorHandle {
    fn drop(&mut self) {
        // Changes still waiting for their quarter second go to the session
        // now, with the view as it was left.
        let mut editor = embed::lock(&self.editor);
        let mut session = self.shared.lock();
        frame::send_out(&mut session, &mut editor, &mut None, true);
    }
}

impl EditorHandle for AcEditorHandle {
    type Window = baseview::Window;
    type Error = EmbedError;

    fn run_until_closed(window: Self::Window) -> Result<(), Self::Error> {
        Ok(window.run_until_closed()?)
    }

    fn set_parent(
        &self,
        parent: ParentWindowHandle,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        Ok(window.set_parent(embed::parent_handle(&parent)?)?)
    }

    fn show(&self, window: &Self::Window) -> Result<(), Self::Error> {
        window.show()?;
        // The first frame doesn't always follow on its own.
        self.repaint.request_repaint();
        Ok(())
    }

    fn hide(&self, window: &Self::Window) -> Result<(), Self::Error> {
        Ok(window.hide()?)
    }

    fn set_size(
        &self,
        new_size: NativeSize<u32>,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        let (_, scale) = self.size.get();
        let logical: LogicalSize<f64> = new_size.cast::<f64>().to_logical(scale);
        window.resize(baseview::dpi::Size::Logical(logical))?;
        self.size.set(logical, scale);
        Ok(())
    }

    fn host_main_thread_callback(&self, window: &Self::Window) {
        window.host_main_thread_callback();
    }

    fn adjust_size(
        &self,
        new_size: NativeSize<u32>,
        window: &Self::Window,
    ) -> Option<NativeSize<u32>> {
        let now = window.size();
        Some(resize_hint().adjust_size(
            new_size,
            NativeSize::from_logical_or_physical(now.logical, now.physical),
            now.scale_factor,
        ))
    }

    fn set_fallback_scale_factor(
        &self,
        scale_factor: f64,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        Ok(window.suggest_fallback_scale_factor(scale_factor)?)
    }

    fn state_changed(&self) {
        self.repaint.request_repaint();
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.repaint.request_repaint();
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.repaint.request_repaint();
    }
}

/// The framework's host callbacks as baseview takes them (the framework
/// mirrors baseview's traits so it needn't depend on it).
fn host_of(host: HostMethods) -> baseview::host::Host {
    struct Callbacks(Box<dyn nice_plug::editor::HostCallbacks>);
    impl baseview::host::HostCallbacks for Callbacks {
        fn request_resize(&mut self, new_size: WindowSize) -> Result<(), HandlerError> {
            self.0
                .request_resize(new_size.physical.into(), new_size.scale_factor)
                .map_err(HandlerError::from_boxed)
        }
        fn destroyed(&mut self) {
            self.0.destroyed();
        }
    }
    struct MainThread(Box<dyn nice_plug::editor::HostMainThreadCaller>);
    impl baseview::host::HostMainThreadCaller for MainThread {
        fn call_main_thread(&mut self) {
            self.0.call_main_thread();
        }
    }
    baseview::host::Host::new()
        .with_callbacks(Callbacks(host.callbacks))
        .with_main_thread(MainThread(host.main_thread_caller))
}
