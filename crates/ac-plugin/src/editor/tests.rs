//! The plugin window without a window: the editor's frames driven by
//! egui_kittest against a real session, and the macros through a stand-in
//! for the framework's GUI context.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp, HostAudio};
use ac_core::NodeId;
use egui::{Event, Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use nice_plug::context::PluginApi;
use nice_plug::context::gui::{GuiContext, GuiContextInner};
use nice_plug::editor::Editor;
use nice_plug::editor::dpi::{LogicalSize, NativeSize, Size};
use nice_plug::params::internals::ParamPtr;
use nice_plug::plugin::PluginState;
use nice_plug::prelude::Param;

use super::frame::{self, UNSEEN};
use super::host_macros::PluginMacros;
use super::{EditorContext, NoEditor, WindowKind, create};
use crate::params::PluginParams;
use crate::session::{Kind, Shared};

fn context(kind: Kind) -> EditorContext {
    let shared = Shared::new(kind);
    EditorContext {
        params: Arc::new(PluginParams::new(shared.clone())),
        shared,
    }
}

#[test]
fn the_clap_plugins_offer_a_resizable_window_and_vst3_none() {
    let ctx = context(Kind::Effect);
    let editor = create(ctx.clone()).expect("the CLAP plugins have an editor");
    assert_eq!(
        editor.size(),
        NativeSize::from_size(Size::Logical(LogicalSize::new(1200.0, 760.0)), 1.0),
        "the default size until someone resizes it"
    );
    let hint = editor.resize_hint();
    assert!(hint.can_resize && hint.can_resize_horizontally && hint.can_resize_vertically);
    assert!(NoEditor::create(ctx).is_none(), "VST3 has no window yet");
}

/// What the framework's GUI context heard.
#[derive(Default)]
struct Heard(Mutex<Vec<(&'static str, ParamPtr, f32)>>);

impl Heard {
    fn push(&self, what: &'static str, param: ParamPtr, value: f32) {
        if let Ok(mut h) = self.0.lock() {
            h.push((what, param, value));
        }
    }
}

// Implementing the framework's trait is the only way to see what its
// `ParamSetter` sends; the trait declares these methods `unsafe`.
#[allow(unsafe_code)]
impl GuiContextInner for Heard {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    // SAFETY: only records the pointer; never dereferences it.
    unsafe fn raw_begin_set_parameter(&self, param: ParamPtr) {
        self.push("begin", param, 0.0);
    }
    // SAFETY: as above.
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
        self.push("set", param, normalized);
    }
    // SAFETY: as above.
    unsafe fn raw_end_set_parameter(&self, param: ParamPtr) {
        self.push("end", param, 0.0);
    }
    fn get_state(&self) -> PluginState {
        PluginState {
            version: String::new(),
            params: BTreeMap::new(),
            fields: BTreeMap::new(),
        }
    }
    fn set_state(&self, _state: PluginState) {}
    fn request_restart(&self) {}
}

/// The window's side of an instance: its editor and the session.
struct Window {
    editor: EditorApp,
    shared: Arc<Shared>,
    seen: AtomicU64,
    last_push: Option<Instant>,
    _processor: ac_engine::Processor,
}

fn window(ctx: &EditorContext, heard: &Arc<Heard>) -> Harness<'static, Window> {
    // The host activated the plugin: the session runs an engine.
    let (processor, _) = ctx.shared.lock().start(48_000.0, 128);
    let mut editor = EditorApp::embedded(AppOptions {
        audio: AudioMode::Hosted(HostAudio::default()),
        ..AppOptions::headless()
    });
    editor.host_keeps_project();
    editor.set_host_controls(Box::new(PluginMacros::new(
        ctx.params.clone(),
        ctx.shared.labels().clone(),
        GuiContext::new(heard.clone()),
    )));
    let state = Window {
        editor,
        shared: ctx.shared.clone(),
        seen: AtomicU64::new(UNSEEN),
        last_push: None,
        _processor: processor,
    };
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(
            |ui, w: &mut Window| {
                let mut session = w.shared.lock();
                frame::frame(
                    &mut session,
                    &mut w.editor,
                    &w.seen,
                    &mut w.last_push,
                    |e| e.ui(ui),
                );
            },
            state,
        );
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'static, Window>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

#[test]
fn edits_in_the_editor_reach_the_session() {
    let ctx = context(Kind::Effect);
    let heard = Arc::new(Heard::default());
    let mut h = window(&ctx, &heard);
    let revision = ctx.shared.lock().revision();
    // The window shows the session's project, on the session's engine.
    assert_eq!(
        h.state().editor.document().project.graph,
        ctx.shared.lock().project().graph
    );
    assert_eq!(h.state().editor.engine.info.sample_rate, 48_000);

    // Add a node from the palette: the session runs it at once.
    let before = ctx.shared.lock().project().graph.nodes.len();
    h.hover_at(Pos2::new(700.0, 600.0));
    h.run_steps(2);
    press(&mut h, Modifiers::SHIFT, Key::A);
    h.event(Event::Text("oscillator".into()));
    h.run_steps(3);
    press(&mut h, Modifiers::NONE, Key::Enter);
    {
        // With an engine running, the session's project is the engine's.
        let session = ctx.shared.lock();
        let graph = &session.project().graph;
        assert_eq!(graph.nodes.len(), before + 1);
        assert!(graph.nodes.iter().any(|n| n.kind == "oscillator"));
        assert_eq!(session.revision(), revision, "the editor's own edit");
    }

    // A move waits its quarter second, then goes too.
    let id = h.state().editor.document().graph().nodes[0].id;
    h.state_mut()
        .editor
        .doc
        .move_nodes(&[(id, [-400.0, 333.0])]);
    h.run_steps(1);
    std::thread::sleep(Duration::from_millis(300));
    h.run_steps(2);
    let pos = |shared: &Shared| shared.lock().project().graph.node(id).map(|n| n.ui.pos);
    assert_eq!(pos(&ctx.shared), Some([-400.0, 333.0]));

    // The host restores other state: the window follows, starting a fresh
    // undo history.
    ctx.shared.lock().reset_to_default();
    h.run_steps(2);
    let doc = h.state().editor.document();
    assert_eq!(doc.project.graph, ctx.shared.lock().project().graph);
    assert!(!doc.can_undo());
}

#[test]
fn new_and_picked_files_go_to_the_session() {
    let ctx = context(Kind::Instrument);
    let heard = Arc::new(Heard::default());
    let mut h = window(&ctx, &heard);
    // A file picked for a Clip: its bytes go into the session's state.
    let dir = std::env::temp_dir().join(format!("ac-plugin-editor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("kick.wav");
    std::fs::write(&wav, crate::assets::wav_bytes(48_000, &[vec![0.5; 64]])).unwrap();
    h.state_mut().editor.doc.edit_project("Add clip", |p| {
        p.graph
            .nodes
            .push(ac_core::NodeInstance::new(NodeId(99), "clip").at(0.0, 400.0));
    });
    h.run_steps(2);
    h.state_mut().editor.assign_clip(NodeId(99), &wav, 0.0);
    h.run_steps(2);
    {
        let session = ctx.shared.lock();
        let file = session.project().graph.node(NodeId(99)).unwrap().config["file"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(file.starts_with("assets/kick-"), "{file}");
        assert!(session.assets().get(&file).is_some(), "kept in the state");
        assert!(session.save_state().contains(&file));
    }
    // File ▸ New: the instrument's default comes back.
    press(&mut h, Modifiers::COMMAND, Key::N);
    // Unsaved changes are asked about first.
    if h.query_by_label("Don't Save").is_some() {
        h.get_by_label("Don't Save").click();
        h.run_steps(3);
    }
    h.run_steps(2);
    assert_eq!(
        ctx.shared.lock().project(),
        &crate::defaults::project(Kind::Instrument)
    );
    assert_eq!(
        h.state().editor.document().project.graph,
        crate::defaults::project(Kind::Instrument).graph
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_macro_moved_in_the_editor_goes_through_the_param_setter() {
    let ctx = context(Kind::Effect);
    let heard = Arc::new(Heard::default());
    let mut h = window(&ctx, &heard);
    h.state_mut().editor.side.open = true;
    h.state_mut().editor.side.tab = Tab::Devices;
    h.run_steps(3);
    h.get_by_label("MACROS");
    let rect = h.get_by_label("Macro 1").rect();
    let y = rect.center().y;
    let from = Pos2::new(rect.max.x + 40.0, y);
    let to = Pos2::new(rect.max.x + 120.0, y);
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=4 {
        h.hover_at(from + (to - from) * (i as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let macro_1 = ctx.params.macros[0].value.as_ptr();
    let heard = heard.0.lock().unwrap().clone();
    assert!(!heard.is_empty(), "the host heard the move");
    assert!(heard.iter().all(|(_, p, _)| *p == macro_1), "only Macro 1");
    assert_eq!(heard.first().map(|e| e.0), Some("begin"));
    assert_eq!(heard.last().map(|e| e.0), Some("end"));
    let sets: Vec<f32> = heard.iter().filter(|e| e.0 == "set").map(|e| e.2).collect();
    assert!(sets.len() >= 2, "{heard:?}");
    assert!(
        sets.windows(2).all(|w| w[1] >= w[0]),
        "dragging right raises it: {sets:?}"
    );
    assert!(sets.iter().all(|v| (0.0..=1.0).contains(v)));
}
