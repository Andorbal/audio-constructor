//! The browser's devices (DECISIONS D154): MIDI ports through Web MIDI,
//! gamepads through the Gamepad API, and the microphone for Audio In
//! nodes, with their own section in the editor's Devices tab. What a
//! browser can't do (OSC over UDP, recording to disk, choosing the audio
//! device) is said there plainly.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use ac_app::audio::DevicesPanel;
use ac_app::theme;
use ac_core::MidiMsg;
use egui::RichText;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MidiAccess, MidiInput, MidiMessageEvent, MidiOptions, MidiOutput};

use super::js;
use super::state::PageState;
use crate::gamepad::{PadState, STANDARD_AXES, STANDARD_BUTTONS};

/// Messages kept for the Devices tab's "Last messages".
const LAST_MESSAGES: usize = 12;

/// Where Web MIDI stands.
#[derive(Clone, Debug, PartialEq)]
pub enum MidiStatus {
    Asking,
    /// The browser has no Web MIDI.
    Unsupported,
    /// The person (or the browser) refused.
    Refused(String),
    Ready,
}

/// One MIDI output the page can send to.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputPort {
    pub id: String,
    pub name: String,
    /// MIDI Out nodes send to it.
    pub chosen: bool,
}

/// The browser's devices as the page holds them.
pub struct Devices {
    pub midi: MidiStatus,
    access: Option<MidiAccess>,
    pub inputs: Vec<String>,
    pub outputs: Vec<OutputPort>,
    /// Newest first: (port, message).
    pub last: VecDeque<(String, String)>,
    pub pads: crate::gamepad::Pads,
    /// Kept alive while the page runs: the MIDI callbacks.
    on_message: Option<Closure<dyn FnMut(MidiMessageEvent)>>,
    on_state: Option<Closure<dyn FnMut(JsValue)>>,
    /// Whether the microphone was asked for on its own (for a patch with
    /// an Audio In node).
    pub mic_asked: bool,
}

impl Default for Devices {
    fn default() -> Self {
        Devices {
            midi: MidiStatus::Asking,
            access: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            last: VecDeque::new(),
            pads: crate::gamepad::Pads::default(),
            on_message: None,
            on_state: None,
            mic_asked: false,
        }
    }
}

fn describe(msg: &MidiMsg) -> String {
    match *msg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity,
        } => format!("Note on {note} vel {velocity} · ch {}", channel + 1),
        MidiMsg::NoteOff { channel, note, .. } => format!("Note off {note} · ch {}", channel + 1),
        MidiMsg::ControlChange {
            channel,
            controller,
            value,
        } => format!("CC {controller} = {value} · ch {}", channel + 1),
        MidiMsg::PitchBend { channel, value } => format!("Pitch bend {value} · ch {}", channel + 1),
        MidiMsg::ChannelPressure { channel, value } => {
            format!("Pressure {value} · ch {}", channel + 1)
        }
        MidiMsg::PolyPressure {
            channel,
            note,
            value,
        } => format!("Pressure {note} = {value} · ch {}", channel + 1),
        MidiMsg::ProgramChange { channel, program } => {
            format!("Program {program} · ch {}", channel + 1)
        }
    }
}

impl Devices {
    /// MIDI arrived from `port` (or the page's own tests): straight into
    /// the ring, so it plays at the next render quantum.
    pub fn heard(state: &mut PageState, port: &str, bytes: &[u8]) {
        let Some(msg) = MidiMsg::from_bytes(bytes) else {
            return;
        };
        state.link.send_midi(msg);
        let d = &mut state.devices;
        d.last.push_front((port.to_string(), describe(&msg)));
        d.last.truncate(LAST_MESSAGES);
    }

    /// Send MIDI Out nodes' messages to the chosen outputs.
    pub fn send(&self, msg: &MidiMsg) {
        let Some(access) = &self.access else {
            return;
        };
        let (bytes, n) = msg.to_bytes();
        let data = js_sys::Uint8Array::from(&bytes[..n]);
        let outputs = access.outputs();
        for port in self.outputs.iter().filter(|p| p.chosen) {
            if let Some(out) = outputs
                .get(&port.id)
                .and_then(|o| o.dyn_into::<MidiOutput>().ok())
            {
                let _ = out.send(&data);
            }
        }
    }

    /// The gamepads the browser sees now, by its index.
    pub fn poll_gamepads() -> Vec<Option<PadState>> {
        let Some(window) = web_sys::window() else {
            return Vec::new();
        };
        let Ok(list) = window.navigator().get_gamepads() else {
            return Vec::new();
        };
        list.iter()
            .map(|g| {
                let g = g.dyn_into::<web_sys::Gamepad>().ok()?;
                if !g.connected() {
                    return None;
                }
                let mut s = PadState {
                    name: g.id(),
                    ..PadState::default()
                };
                for (i, b) in g.buttons().iter().take(STANDARD_BUTTONS).enumerate() {
                    if let Ok(b) = b.dyn_into::<web_sys::GamepadButton>() {
                        s.buttons[i] = b.value() as f32;
                    }
                }
                for (i, a) in g.axes().iter().take(STANDARD_AXES).enumerate() {
                    s.axes[i] = a.as_f64().unwrap_or(0.0) as f32;
                }
                Some(s)
            })
            .collect()
    }

    /// Read the ports again and listen to every input.
    fn refresh(&mut self) {
        let Some(access) = &self.access else {
            return;
        };
        self.inputs.clear();
        let callback = self.on_message.as_ref().map(|c| c.as_ref().clone());
        for v in access.inputs().values().into_iter().flatten() {
            if let Ok(input) = v.dyn_into::<MidiInput>() {
                self.inputs.push(input.name().unwrap_or_else(|| input.id()));
                if let Some(cb) = &callback {
                    input.set_onmidimessage(Some(cb.unchecked_ref()));
                }
            }
        }
        let mut outputs = Vec::new();
        for v in access.outputs().values().into_iter().flatten() {
            if let Ok(out) = v.dyn_into::<MidiOutput>() {
                let id = out.id();
                let chosen = self.outputs.iter().any(|p| p.id == id && p.chosen);
                outputs.push(OutputPort {
                    name: out.name().unwrap_or_else(|| id.clone()),
                    id,
                    chosen,
                });
            }
        }
        self.outputs = outputs;
    }
}

/// Ask for Web MIDI and listen to every input once it's granted.
pub fn start_midi(state: &Rc<RefCell<PageState>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let navigator = window.navigator();
    let has_midi =
        js_sys::Reflect::has(&navigator, &JsValue::from_str("requestMIDIAccess")).unwrap_or(false);
    if !has_midi {
        state.borrow_mut().devices.midi = MidiStatus::Unsupported;
        return;
    }
    let options = MidiOptions::new();
    options.set_sysex(false);
    let promise = match navigator.request_midi_access_with_options(&options) {
        Ok(p) => p,
        Err(e) => {
            state.borrow_mut().devices.midi = MidiStatus::Refused(js::error_text(&e));
            return;
        }
    };
    let weak = Rc::downgrade(state);
    wasm_bindgen_futures::spawn_local(async move {
        let result = wasm_bindgen_futures::JsFuture::from(promise).await;
        let Some(state) = weak.upgrade() else {
            return;
        };
        match result.and_then(|a| a.dyn_into::<MidiAccess>()) {
            Ok(access) => listen(&state, access),
            Err(e) => {
                state.borrow_mut().devices.midi = MidiStatus::Refused(js::error_text(&e));
            }
        }
    });
}

fn listen(state: &Rc<RefCell<PageState>>, access: MidiAccess) {
    let weak: Weak<RefCell<PageState>> = Rc::downgrade(state);
    let on_message = Closure::<dyn FnMut(MidiMessageEvent)>::new(move |e: MidiMessageEvent| {
        let Some(state) = weak.upgrade() else {
            return;
        };
        let Ok(data) = e.data() else {
            return;
        };
        let port = e
            .target()
            .and_then(|t| t.dyn_into::<MidiInput>().ok())
            .and_then(|i| i.name())
            .unwrap_or_default();
        if let Ok(mut s) = state.try_borrow_mut() {
            Devices::heard(&mut s, &port, &data);
        }
    });
    let weak: Weak<RefCell<PageState>> = Rc::downgrade(state);
    let on_state = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        if let Some(state) = weak.upgrade()
            && let Ok(mut s) = state.try_borrow_mut()
        {
            s.devices.refresh();
            s.repaint();
        }
    });
    access.set_onstatechange(Some(on_state.as_ref().unchecked_ref()));
    let mut s = state.borrow_mut();
    s.devices.access = Some(access);
    s.devices.on_message = Some(on_message);
    s.devices.on_state = Some(on_state);
    s.devices.midi = MidiStatus::Ready;
    s.devices.refresh();
    s.repaint();
}

/// Ask for the microphone (the browser asks the person).
pub fn ask_for_mic(state: &Rc<RefCell<PageState>>) {
    let weak = Rc::downgrade(state);
    wasm_bindgen_futures::spawn_local(async move {
        let _ = js::enable_mic().await;
        if let Some(s) = weak.upgrade()
            && let Ok(s) = s.try_borrow()
        {
            s.repaint();
        }
    });
}

/// The browser's section of the Devices tab.
pub struct DevicesUi(pub Rc<RefCell<PageState>>);

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::text_muted()));
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).strong().color(theme::text_strong()));
}

impl DevicesPanel for DevicesUi {
    fn show(&mut self, ui: &mut egui::Ui) {
        let state = self.0.clone();
        let Ok(mut s) = state.try_borrow_mut() else {
            return;
        };
        muted(
            ui,
            "The browser plays through the system's output; pick another output in the \
             system's sound settings.",
        );

        heading(ui, "Microphone");
        let mic = js::mic_status();
        let mut ask = false;
        match mic.as_str() {
            "on" => {
                let label = js::mic_label();
                ui.label(format!(
                    "On{}: Audio In nodes hear it",
                    if label.is_empty() {
                        String::new()
                    } else {
                        format!(" ({label})")
                    }
                ));
                if ui.button("Stop using the microphone").clicked() {
                    js::disable_mic();
                }
            }
            "asking" => muted(ui, "Waiting for the browser to allow the microphone…"),
            "denied" => {
                ui.colored_label(
                    theme::warning(),
                    "The microphone is blocked. Allow it in the browser's site settings, \
                     then try again.",
                );
                ask = ui.button("Try again").clicked();
            }
            "unavailable" => {
                ui.colored_label(
                    theme::warning(),
                    format!("No microphone: {}", js::mic_message()),
                );
                ask = ui.button("Try again").clicked();
            }
            _ => {
                muted(
                    ui,
                    "Audio In nodes hear the microphone or an audio interface's input once \
                     you allow it. Echo cancellation and noise suppression stay off.",
                );
                ask = ui.button("Use the microphone").clicked();
            }
        }

        heading(ui, "MIDI");
        match &s.devices.midi {
            MidiStatus::Asking => muted(ui, "Asking the browser for MIDI…"),
            MidiStatus::Unsupported => muted(
                ui,
                "This browser has no MIDI (Web MIDI). The on-screen keyboard and the \
                 computer keys still play.",
            ),
            MidiStatus::Refused(why) => {
                ui.colored_label(theme::warning(), format!("MIDI was refused: {why}"));
            }
            MidiStatus::Ready => {
                if s.devices.inputs.is_empty() {
                    muted(ui, "No MIDI inputs. Plug one in; it's picked up at once.");
                } else {
                    ui.label("Inputs (all are heard):");
                    for name in &s.devices.inputs {
                        ui.label(format!("  {name}"));
                    }
                }
                if s.devices.outputs.is_empty() {
                    muted(ui, "No MIDI outputs.");
                } else {
                    ui.label("MIDI Out nodes send to:");
                    for port in &mut s.devices.outputs {
                        ui.checkbox(&mut port.chosen, &port.name);
                    }
                }
            }
        }
        if !s.devices.last.is_empty() {
            ui.label("Last messages:");
            for (port, text) in &s.devices.last {
                ui.label(
                    RichText::new(if port.is_empty() {
                        text.clone()
                    } else {
                        format!("{text} · {port}")
                    })
                    .monospace()
                    .small(),
                );
            }
        }

        heading(ui, "Gamepads");
        let pads: Vec<(usize, String)> = s
            .devices
            .pads
            .connected()
            .map(|(i, n)| (i, n.to_string()))
            .collect();
        if pads.is_empty() {
            muted(
                ui,
                "Press a button on a gamepad to connect it (browsers show gamepads only \
                 after a press).",
            );
        }
        for (i, name) in pads {
            ui.label(format!("  {}: {name}", i + 1));
        }

        heading(ui, "Not in the browser");
        muted(
            ui,
            "OSC needs UDP, which pages can't use: OSC In and OSC Out nodes stay quiet here \
             (use the desktop editor or audioc run). Record nodes can't write takes to disk, \
             and plugins can't load. Export WAV and Export Pack download their files.",
        );
        drop(s);
        if ask {
            ask_for_mic(&state);
        }
    }
}
