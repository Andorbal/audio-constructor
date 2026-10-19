//! The page's frame: the editor (a third caller of `EditorApp::ui`, after
//! the desktop and the plugin window) between the link's upkeep.
//!
//! Each frame imports the worklet's telemetry into the mirror engine,
//! sends the computer keys and gamepads to the worklet, hands finished
//! file pickers and downloads to the editor, draws the editor, starts the
//! file requests it made, and syncs the link: the project when it
//! compiled a new one, then what its handle sent.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ac_app::{EditorApp, FileRequest, SaveKind};
use ac_core::{ControlEvent, Project, Registry};
use ac_engine::Processor;
use ac_io::ControlReceiver;
use egui::RichText;

use super::devices::{self, Devices};
use super::js;
use super::state::{PageState, Pending};

/// The editor on the page.
pub struct WebApp {
    pub editor: EditorApp,
    /// The mirror engine's processor, which relays instead of playing.
    pub mirror: Processor,
    /// The computer keys (Play keys), for the worklet.
    keys: Option<ControlReceiver>,
    key_events: Vec<ControlEvent>,
    pub state: Rc<RefCell<PageState>>,
    /// Node kinds of the project last looked at, and its engine revision.
    kinds: (u64, std::collections::BTreeSet<String>),
    told_osc: bool,
    told_record: bool,
    frames: u64,
    /// The settings as last kept in the browser's storage.
    saved_settings: String,
}

impl WebApp {
    pub fn new(
        editor: EditorApp,
        mirror: Processor,
        keys: Option<ControlReceiver>,
        state: Rc<RefCell<PageState>>,
    ) -> WebApp {
        WebApp {
            editor,
            mirror,
            keys,
            key_events: Vec::with_capacity(ac_core::CONTROL_CAPACITY),
            state,
            kinds: (u64::MAX, Default::default()),
            told_osc: false,
            told_record: false,
            frames: 0,
            saved_settings: String::new(),
        }
    }

    /// The audio files the project plays, in path order.
    fn assets(&self) -> Vec<(String, std::sync::Arc<ac_core::AudioData>)> {
        let store = self.editor.assets();
        let mut v: Vec<_> = store
            .paths()
            .filter_map(|p| store.get(p).map(|a| (p.clone(), a)))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Keys and gamepads to the worklet; MIDI Out messages to the ports.
    fn devices(&mut self) {
        let mut st = self.state.borrow_mut();
        let PageState { link, devices, .. } = &mut *st;
        if let Some(keys) = self.keys.as_mut() {
            let rate = f64::from(self.editor.engine.info.sample_rate);
            keys.drain(ac_io::clock::now_ns(), 1, rate, &mut self.key_events);
            for e in &self.key_events {
                link.send_control(e.control, e.value);
            }
        }
        let pads = Devices::poll_gamepads();
        let mut events = Vec::new();
        devices.pads.update(&pads, |c, v| events.push((c, v)));
        for (c, v) in events {
            link.send_control(c, v);
        }
        link.take_midi_out(|m| devices.send(&m));
    }

    /// Ask for the microphone the first time a patch listens to an input,
    /// and say once that OSC can't work here.
    fn notice_needs(&mut self) {
        // Opening a project starts its revisions over, so look again now
        // and then too, not only when the revision moves.
        let doc = self.editor.document();
        if self.kinds.0 != doc.engine_revision || self.frames.is_multiple_of(30) {
            self.kinds = (
                doc.engine_revision,
                ac_app::help::project_kinds(&doc.project),
            );
        }
        let kinds = &self.kinds.1;
        if kinds.contains("audio_in") {
            let ask = {
                let mut st = self.state.borrow_mut();
                let first = !st.devices.mic_asked && js::mic_status() == "off";
                st.devices.mic_asked = true;
                first
            };
            if ask {
                devices::ask_for_mic(&self.state);
            }
        }
        let osc = kinds.contains("osc_in") || kinds.contains("osc_out");
        if osc && !self.told_osc {
            self.told_osc = true;
            self.editor.notify(
                "This patch uses OSC, which needs UDP: browsers can't send or receive it, so its \
                 OSC nodes stay quiet here",
                true,
            );
        }
        if kinds.contains("record") && !self.told_record {
            self.told_record = true;
            self.editor.notify(
                "Record nodes can't write takes in the browser; Export WAV renders the project",
                true,
            );
        }
    }

    /// Hand finished pickers and downloads to the editor.
    fn deliver(&mut self) {
        let pending = std::mem::take(&mut self.state.borrow_mut().pending);
        for p in pending {
            match p {
                Pending::Opened { name, bytes } => self.editor.open_file(&name, &bytes),
                Pending::Audio {
                    purpose,
                    name,
                    bytes,
                } => self.editor.audio_file(purpose, &name, bytes),
                Pending::Example { path, text, assets } => {
                    self.editor.open_example_files(&path, &text, assets);
                    // A new file's Save asks where.
                    self.state.borrow_mut().file_handle = wasm_bindgen::JsValue::NULL;
                }
                Pending::Examples { list, open } => {
                    self.editor.set_examples(list);
                    if let Some(path) = open {
                        self.editor.request_example(path);
                    }
                }
                Pending::Saved {
                    kind,
                    name,
                    revision,
                } => self.editor.file_saved(kind, &name, revision),
                Pending::NotSaved(why) => self.editor.file_not_saved(why.as_deref()),
                Pending::Say { text, warning } => self.editor.notify(text, warning),
            }
        }
    }

    /// Start what the editor asked of its files this frame.
    fn file_requests(&mut self) {
        for r in self.editor.take_file_requests() {
            let weak = Rc::downgrade(&self.state);
            let registry = self.editor.registry.clone();
            match r {
                FileRequest::Open => spawn(weak, async move {
                    let accept = [".acproj", ".acpack", ".acblock"];
                    pick(
                        "Audio Constructor project, pack, or block",
                        &accept,
                        |name, bytes, handle| (Pending::Opened { name, bytes }, Some(handle)),
                    )
                    .await
                }),
                FileRequest::ImportBlock => spawn(weak, async move {
                    pick(
                        "Audio Constructor block",
                        &[".acblock"],
                        |name, bytes, _| (Pending::Opened { name, bytes }, None),
                    )
                    .await
                }),
                FileRequest::Audio(purpose) => spawn(weak, async move {
                    pick("WAV audio", &[".wav", ".wave"], move |name, bytes, _| {
                        (
                            Pending::Audio {
                                purpose,
                                name,
                                bytes,
                            },
                            None,
                        )
                    })
                    .await
                }),
                FileRequest::Example(path) => spawn(weak, async move {
                    match fetch_example(&path, &registry).await {
                        Ok(p) => Some((p, None)),
                        Err(e) => Some((
                            Pending::Say {
                                text: format!("Couldn't load the example: {e}"),
                                warning: true,
                            },
                            None,
                        )),
                    }
                }),
                FileRequest::Save {
                    name,
                    bytes,
                    kind,
                    same_place,
                    revision,
                } => {
                    let handle = if kind == SaveKind::Project {
                        self.state.borrow().file_handle.clone()
                    } else {
                        wasm_bindgen::JsValue::NULL
                    };
                    spawn(weak, async move {
                        let description = match kind {
                            SaveKind::Project => "Audio Constructor project",
                            SaveKind::Wav => "WAV audio",
                            SaveKind::Pack => "Audio Constructor pack",
                            SaveKind::Block => "Audio Constructor block",
                        };
                        let r = js::save_file(
                            &name,
                            &bytes,
                            kind.mime(),
                            description,
                            &handle,
                            !same_place,
                        )
                        .await;
                        Some(match r {
                            Ok(v) if v.is_null() || v.is_undefined() => {
                                (Pending::NotSaved(None), None)
                            }
                            Ok(v) => {
                                let saved = js::get(&v, "name").as_string().unwrap_or(name);
                                let handle =
                                    (kind == SaveKind::Project).then(|| js::get(&v, "handle"));
                                (
                                    Pending::Saved {
                                        kind,
                                        name: saved,
                                        revision,
                                    },
                                    handle,
                                )
                            }
                            Err(e) => (Pending::NotSaved(Some(js::error_text(&e))), None),
                        })
                    });
                }
            }
        }
    }

    /// The page's title follows the document, as a window's does, and
    /// leaving with unsaved changes asks first.
    fn title(&mut self) {
        let doc = self.editor.document();
        let dirty = doc.is_dirty();
        let title = format!(
            "{}{} — Audio Constructor",
            if dirty { "● " } else { "" },
            doc.title()
        );
        let mut st = self.state.borrow_mut();
        st.dirty = dirty;
        if st.title != title {
            if let Some(d) = web_sys::window().and_then(|w| w.document()) {
                d.set_title(&title);
            }
            st.title = title;
        }
    }

    /// The theme and UI scale outlive the page, in the browser's storage.
    fn keep_settings(&mut self) {
        let text = self.editor.settings().to_text();
        if text != self.saved_settings {
            super::store_settings(&text);
            self.saved_settings = text;
        }
    }

    /// What the page draws over the editor: why there's no sound yet, or
    /// that the audio stopped.
    fn overlay(&self, ctx: &egui::Context) {
        let audio = js::audio_state();
        let worklet = js::worklet_state();
        let (text, warning) = if worklet == "failed" {
            (
                format!(
                    "The audio stopped: {}. Reload the page to start it again.",
                    js::worklet_message()
                ),
                true,
            )
        } else if audio == "suspended" {
            (
                "Click anywhere or press a key to start the sound (browsers wait for you)."
                    .to_string(),
                false,
            )
        } else {
            return;
        };
        egui::Area::new(egui::Id::unique("ac-web-audio-state"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 36.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let color = if warning {
                        ac_app::theme::error()
                    } else {
                        ac_app::theme::text_strong()
                    };
                    ui.label(RichText::new(text).color(color));
                });
            });
    }
}

/// Run a browser call; its result goes to the next frame. A returned
/// handle becomes where the document's Save writes.
fn spawn(
    weak: std::rc::Weak<RefCell<PageState>>,
    task: impl std::future::Future<Output = Option<(Pending, Option<wasm_bindgen::JsValue>)>> + 'static,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = task.await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Ok(mut st) = state.try_borrow_mut() else {
            return;
        };
        if let Some((p, handle)) = result {
            if let Some(h) = handle {
                st.file_handle = h;
            }
            st.finish(p);
        }
    });
}

/// Let the person pick a file and turn it into a result.
async fn pick(
    description: &str,
    accept: &[&str],
    make: impl FnOnce(
        String,
        Vec<u8>,
        wasm_bindgen::JsValue,
    ) -> (Pending, Option<wasm_bindgen::JsValue>),
) -> Option<(Pending, Option<wasm_bindgen::JsValue>)> {
    let accept = accept.iter().map(|s| s.to_string()).collect();
    match js::pick_file(description, accept).await {
        Ok(v) if v.is_null() || v.is_undefined() => None,
        Ok(v) => {
            let name = js::get(&v, "name").as_string().unwrap_or_default();
            let bytes = js_sys::Uint8Array::new(&js::get(&v, "bytes")).to_vec();
            let handle = js::get(&v, "handle");
            let lower = name.to_ascii_lowercase();
            let (p, h) = make(name, bytes, handle);
            // Only a project's own file is somewhere Save can write back to.
            Some((p, h.filter(|h| !h.is_null() && lower.ends_with(".acproj"))))
        }
        Err(e) => Some((
            Pending::Say {
                text: format!("Couldn't open the file: {}", js::error_text(&e)),
                warning: true,
            },
            None,
        )),
    }
}

/// Fetch a file from the site.
pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let v = js::fetch_bytes(url).await.map_err(|e| js::error_text(&e))?;
    Ok(js_sys::Uint8Array::new(&v).to_vec())
}

/// Fetch an example and the audio files it plays (beside it on the site).
async fn fetch_example(path: &Path, registry: &Registry) -> Result<Pending, String> {
    let url = path.to_string_lossy().replace('\\', "/");
    let text = String::from_utf8(fetch(&url).await?).map_err(|e| e.to_string())?;
    let project = Project::from_json(&text).map_err(|e| e.to_string())?;
    let dir = url.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    let mut assets = Vec::new();
    for (file, _) in ac_core::project::referenced_assets(&project, registry) {
        // Only files beside it: an example never reaches outside its folder.
        if file.starts_with('/') || file.contains("..") {
            continue;
        }
        let at = if dir.is_empty() {
            file.clone()
        } else {
            format!("{dir}/{file}")
        };
        if let Ok(bytes) = fetch(&at).await {
            assets.push((file, bytes));
        }
    }
    Ok(Pending::Example {
        path: path.to_path_buf(),
        text,
        assets,
    })
}

/// Fetch the site's examples (`examples/index.json` lists them).
pub async fn fetch_examples() -> Result<Vec<(PathBuf, Project)>, String> {
    let index = fetch("examples/index.json").await?;
    let index: serde_json::Value = serde_json::from_slice(&index).map_err(|e| e.to_string())?;
    let mut list = Vec::new();
    for file in index["examples"].as_array().into_iter().flatten() {
        let Some(file) = file.as_str() else { continue };
        let path = PathBuf::from(format!("examples/{file}"));
        let Ok(bytes) = fetch(&path.to_string_lossy()).await else {
            continue;
        };
        if let Ok(p) = Project::from_json(&String::from_utf8_lossy(&bytes)) {
            list.push((path, p));
        }
    }
    Ok(list)
}

impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        {
            let mut st = self.state.borrow_mut();
            if st.ctx.is_none() {
                st.ctx = Some(ctx.clone());
            }
            st.link.import(&self.editor.engine.handle);
        }
        self.devices();
        self.deliver();
        self.notice_needs();
        self.editor.ui(ui);
        self.file_requests();
        let assets = self.assets();
        let posts = {
            let mut st = self.state.borrow_mut();
            st.link
                .sync(&self.editor.engine.handle, &mut self.mirror, &assets)
        };
        for p in posts {
            js::post_to_audio(&p);
        }
        // Now and then the worklet frees graphs it retired (it never frees
        // inside process()).
        self.frames += 1;
        if self.frames.is_multiple_of(120) {
            js::post_to_audio(&crate::wire::Posted::Collect.encode());
        }
        self.overlay(&ctx);
        self.title();
        self.keep_settings();
        // The worklet keeps playing between frames; meters and scopes
        // follow it at a steady rate (the editor asks for more while the
        // transport runs).
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ac_app::theme::canvas_bg().to_normalized_gamma_f32()
    }
}
