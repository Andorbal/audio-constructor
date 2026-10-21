//! The page (DECISIONS D150–D155): the editor on eframe's web target, the
//! link to the audio worklet, the browser's devices, and its files.
//!
//! `boot.js` calls [`start`] with the canvas once the page is cross-origin
//! isolated (shared memory needs it). [`start`] makes the shared buffers,
//! starts the audio context and the worklet (`audio.js`), builds the
//! editor with a mirror engine at the context's rate, asks for MIDI, fetches
//! the site's examples, and opens the Start example (or the one named in
//! the page's `?example=` parameter). The [`WebHandle`] it returns is what
//! scripts and the end-to-end tests use.

mod app;
mod devices;
mod js;
mod shared;
mod state;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use ac_app::audio::HostAudio;
use ac_app::settings::AppSettings;
use ac_app::{AppOptions, EditorApp};
use ac_core::{NodePath, Value};
use wasm_bindgen::prelude::*;

use self::app::WebApp;
use self::shared::JsShared;
use self::state::{PageState, Pending};
use crate::link::PageLink;
use crate::ring::{init_ring, ring_bytes};

/// The render quantum: the most frames the worklet hands the engine at once.
const QUANTUM: usize = 128;
/// Channels the worklet gives the engine (the microphone comes in on both).
const IN_CHANNELS: usize = 2;
const OUT_CHANNELS: usize = 2;
/// Ring and snapshot sizes.
const TO_AUDIO_BYTES: usize = ring_bytes(1 << 18);
const FROM_AUDIO_BYTES: usize = ring_bytes(1 << 16);
const SNAPSHOT_BYTES: usize = 8 << 20;

/// Where the editor's settings live in the browser's storage.
const SETTINGS_KEY: &str = "audio-constructor-settings";

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The editor's settings as the browser kept them (the defaults if it
/// kept none, or keeps nothing).
fn stored_settings() -> AppSettings {
    local_storage()
        .and_then(|s| s.get_item(SETTINGS_KEY).ok().flatten())
        .map_or_else(AppSettings::in_memory, |t| AppSettings::from_text(&t))
}

/// Keep the editor's settings in the browser's storage (quietly not, where
/// the browser refuses).
fn store_settings(text: &str) {
    if let Some(s) = local_storage() {
        let _ = s.set_item(SETTINGS_KEY, text);
    }
}

fn js_error(text: impl Into<String>) -> JsValue {
    js_sys::Error::new(&text.into()).into()
}

/// The running page, for scripts and tests: what the engine does, and a
/// few ways to drive it as a person would.
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
    state: Rc<RefCell<PageState>>,
}

#[wasm_bindgen]
impl WebHandle {
    /// What the engine is doing, as JSON: `audio` (the context's state),
    /// `worklet`, `playing`, `position` (samples), `seconds`, `peak`
    /// (left and right output levels), `sampleRate`, `inStep` (the worklet
    /// runs the project the editor shows), `project`, `errors` (compile
    /// errors), `missingFiles` (audio files the project can't find),
    /// `baked` (whole-input nodes baked), `dirty` (unsaved changes),
    /// `midiInputs`, `midiSent` (messages MIDI
    /// Out nodes sent), `mic`, and `dropped` (ring records that didn't
    /// fit).
    pub fn status(&self) -> String {
        let Some(app) = self.runner.app_mut::<WebApp>() else {
            return "{}".into();
        };
        let handle = &app.editor.engine.handle;
        let status = handle.status();
        let st = self.state.borrow();
        let rate = st.sample_rate.max(1);
        serde_json::json!({
            "audio": js::audio_state(),
            "worklet": js::worklet_state(),
            "playing": handle.is_playing(),
            "position": handle.position(),
            "seconds": handle.position() as f64 / f64::from(rate),
            "peak": status.master_peak(),
            "sampleRate": rate,
            "inStep": st.link.in_step,
            "project": app.editor.document().title(),
            "errors": app.editor.engine.report.errors().count(),
            "warnings": app
                .editor
                .engine
                .report
                .diagnostics
                .iter()
                .filter(|d| d.severity == ac_engine::Severity::Warning)
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>(),
            "missingFiles": app.editor.assets().issues.len(),
            "baked": handle
                .bakes()
                .iter()
                .filter(|b| b.state == ac_engine::WholeState::Baked)
                .count(),
            "dirty": app.editor.document().is_dirty(),
            "midiInputs": st.devices.inputs,
            "midiSent": st.devices.sent,
            "mic": js::mic_status(),
            "dropped": st.link.dropped,
        })
        .to_string()
    }

    /// Start the transport, as the play button does.
    pub fn play(&self) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.editor.engine.handle.play();
        }
    }

    /// Stop the transport.
    pub fn stop(&self) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.editor.engine.handle.stop();
        }
    }

    /// A MIDI message (`[0x90, 60, 100]`), taken as if a MIDI input sent
    /// it.
    pub fn midi(&self, bytes: &[u8]) {
        if let Ok(mut st) = self.state.try_borrow_mut() {
            devices::Devices::heard(&mut st, "script", bytes);
        }
    }

    /// Set an unwired input of the node at `node` (an engine path, `2` or
    /// `5/3`) to `value` (JSON: `440`, `true`, `"sine"`), as dragging its
    /// control does. Returns whether the input took it.
    pub fn set_param(&self, node: &str, socket: &str, value: &str) -> bool {
        let Ok(path) = node.parse::<NodePath>() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(value) else {
            return false;
        };
        let Some(mut app) = self.runner.app_mut::<WebApp>() else {
            return false;
        };
        app.editor.engine.handle.set_param(&path, socket, value)
    }

    /// Open the site's example `file` (`guitar-pedalboard.acproj`) as a new
    /// project, asking first if there are unsaved changes.
    pub fn open_example(&self, file: &str) {
        if let Some(mut app) = self.runner.app_mut::<WebApp>() {
            app.editor
                .request_example(PathBuf::from(format!("examples/{file}")));
        }
    }

    /// Ask for the microphone (the browser asks the person).
    pub fn use_microphone(&self) {
        devices::ask_for_mic(&self.state);
    }
}

/// Start the page on `canvas`. Fails with a message when the browser
/// lacks what it needs (shared memory, audio worklets) or the engine's
/// files can't be fetched.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<WebHandle, JsValue> {
    let isolated = web_sys::window()
        .map(|w| {
            js_sys::Reflect::get(&w, &JsValue::from_str("crossOriginIsolated"))
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !isolated {
        return Err(js_error(
            "this page isn't cross-origin isolated, so the audio engine can't share memory \
             with it (the site needs the COOP and COEP headers, or its service worker)",
        ));
    }
    let to_audio = JsShared::new(TO_AUDIO_BYTES);
    let from_audio = JsShared::new(FROM_AUDIO_BYTES);
    let snapshot = JsShared::new(SNAPSHOT_BYTES);
    if init_ring(&to_audio).is_none() || init_ring(&from_audio).is_none() {
        return Err(js_error("couldn't set up the rings"));
    }
    let shared = js_sys::Object::new();
    for (k, v) in [
        ("toAudio", to_audio.buffer()),
        ("fromAudio", from_audio.buffer()),
        ("snapshot", snapshot.buffer()),
    ] {
        js_sys::Reflect::set(&shared, &JsValue::from_str(k), v)?;
    }
    let options = js_sys::Object::new();
    for (k, v) in [
        ("wasmUrl", JsValue::from_str("ac_audio.wasm")),
        ("workletUrl", JsValue::from_str("worklet.js")),
        ("inChannels", JsValue::from(IN_CHANNELS as u32)),
        ("outChannels", JsValue::from(OUT_CHANNELS as u32)),
        ("maxBlock", JsValue::from(QUANTUM as u32)),
    ] {
        js_sys::Reflect::set(&options, &JsValue::from_str(k), &v)?;
    }
    let rate = js::create_audio(&shared, &options)
        .await?
        .as_f64()
        .unwrap_or(48_000.0) as u32;

    let audio = HostAudio {
        host: "Web Audio".into(),
        sample_rate: rate,
        max_block: QUANTUM,
        in_channels: IN_CHANNELS,
        out_channels: OUT_CHANNELS,
    };
    let seed = (js_sys::Math::random() * 9.0e15) as u64 | 1;
    let mut editor = EditorApp::embedded(AppOptions::web(audio, seed, stored_settings()));
    let mirror = editor
        .take_processor()
        .ok_or_else(|| js_error("the editor kept its engine"))?;
    let keys = editor.take_control_receiver();
    let link = PageLink::new(to_audio, from_audio, snapshot)
        .ok_or_else(|| js_error("couldn't open the rings"))?;
    let state = Rc::new(RefCell::new(PageState::new(link, rate)));
    editor.engine.devices_panel = Some(Box::new(devices::DevicesUi(state.clone())));
    devices::start_midi(&state);

    // The examples come from the site; the first one opens when they're in.
    let weak = Rc::downgrade(&state);
    wasm_bindgen_futures::spawn_local(async move {
        let result = app::fetch_examples().await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        let wanted = js::query_param("example");
        let pending = match result {
            Ok(list) => {
                let file = if wanted.is_empty() {
                    "start".to_string()
                } else {
                    wanted.trim_end_matches(".acproj").to_string()
                };
                let open = list
                    .iter()
                    .map(|(p, _)| p.clone())
                    .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == file));
                Pending::Examples { list, open }
            }
            Err(e) => Pending::Say {
                text: ac_app::i18n::tr_args("web-couldnt-load-examples", &[("error", e.into())]),
                warning: true,
            },
        };
        if let Ok(mut st) = state.try_borrow_mut() {
            st.finish(pending);
        }
    });

    // Leaving the page with unsaved changes asks first, as closing the
    // desktop editor's window does.
    if let Some(window) = web_sys::window() {
        let weak = Rc::downgrade(&state);
        let on_leave = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
            move |e: web_sys::BeforeUnloadEvent| {
                let dirty = weak
                    .upgrade()
                    .and_then(|s| s.try_borrow().ok().map(|s| s.dirty))
                    .unwrap_or(false);
                if dirty {
                    e.prevent_default();
                    e.set_return_value("unsaved");
                }
            },
        );
        window.set_onbeforeunload(Some(on_leave.as_ref().unchecked_ref()));
        on_leave.forget();
    }

    let runner = eframe::WebRunner::new();
    let app = WebApp::new(editor, mirror, keys, state.clone());
    runner
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                ac_app::theme::apply(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        )
        .await?;
    Ok(WebHandle { runner, state })
}
