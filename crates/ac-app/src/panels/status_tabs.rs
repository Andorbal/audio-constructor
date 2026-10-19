//! The Devices tab (audio device, latency, MIDI inputs and outputs,
//! gamepads, OSC, computer keys) and the Problems tab (compile diagnostics,
//! click to jump).

use ac_engine::Severity;
use egui::RichText;

use ac_io::{InputChoice, MidiSelect};

use super::{DevicesState, PanelIo, PanelRequest, section};
use crate::audio::LatencyState;
use crate::icons;
use crate::theme;

/// The rows every host shows: the rate, the buffer, the channels, and how
/// hard the engine works.
fn engine_rows(ui: &mut egui::Ui, info: &ac_io::StreamInfo, status: &ac_engine::EngineStatus) {
    ui.label("Sample rate");
    ui.label(format!("{} Hz", info.sample_rate));
    ui.end_row();
    ui.label("Buffer");
    ui.label(format!(
        "{} frames ({:.1} ms)",
        info.buffer_size,
        f64::from(info.buffer_size) * 1000.0 / f64::from(info.sample_rate.max(1))
    ));
    ui.end_row();
    ui.label("Channels");
    ui.label(format!(
        "{} in · {} out",
        info.in_channels, info.out_channels
    ));
    ui.end_row();
    ui.label("Load");
    let load = status.load_fraction();
    ui.add(
        egui::ProgressBar::new(load.clamp(0.0, 1.0))
            .text(format!("{:.0} %", load * 100.0))
            .desired_width(160.0),
    );
    ui.end_row();
    let cores = status.core_loads();
    if cores.len() > 1 {
        ui.label("Per core");
        ui.vertical(|ui| {
            for (k, c) in cores.iter().enumerate() {
                let name = if k == 0 {
                    "audio".to_string()
                } else {
                    format!("worker {k}")
                };
                ui.add(
                    egui::ProgressBar::new(c.clamp(0.0, 1.0))
                        .text(format!("{name} {:.0} %", c * 100.0))
                        .desired_width(160.0),
                );
            }
        });
        ui.end_row();
    }
    ui.label("Xruns");
    let x = status.total_xruns();
    ui.colored_label(
        if x > 0 {
            theme::warning()
        } else {
            theme::text()
        },
        x.to_string(),
    )
    .on_hover_text("Engine blocks over budget plus device underruns/overruns");
    ui.end_row();
}

/// The Devices tab in a plugin: the host owns the audio and MIDI, so there
/// is nothing to pick, only what the engine runs at.
fn hosted_devices(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    state: &mut DevicesState,
    host: &crate::audio::HostAudio,
) {
    let status = io.engine.handle.status().clone();
    let info = io.engine.info.clone();
    section(ui, "Running");
    egui::Grid::new("devices-now")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            ui.label("Audio");
            ui.label(RichText::new(&host.host).color(theme::text_strong()));
            ui.end_row();
            engine_rows(ui, &info, &status);
        });
    ui.add_space(4.0);
    if let Some(panel) = io.engine.devices_panel.as_mut() {
        panel.show(ui);
    } else {
        ui.label(
            RichText::new(format!(
                "{}  The host provides audio and MIDI. Set the sample rate, the buffer size, \
                 and MIDI routing in the host.",
                icons::INFO
            ))
            .color(theme::text_muted()),
        );
    }
    section(ui, "Computer keys");
    ui.label(
        RichText::new(
            "Turn on Play keys (the keyboard button in the transport bar) to send key \
             presses to the patch instead of running shortcuts.",
        )
        .color(theme::text_muted()),
    );
    let choice = state
        .transport_choice
        .get_or_insert(ac_core::TransportAction::PlayStop);
    super::bindings_ui::transport_section(ui, io, choice);
}

pub(super) fn devices(ui: &mut egui::Ui, io: &mut PanelIo, state: &mut DevicesState) {
    if let Some(host) = io.engine.hosted().cloned() {
        hosted_devices(ui, io, state, &host);
        return;
    }
    let status = io.engine.handle.status().clone();
    let info = io.engine.info.clone();
    section(ui, "Running");
    egui::Grid::new("devices-now")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            ui.label("Audio");
            if !io.engine.is_running() {
                ui.label(RichText::new("off").color(theme::text_muted()));
            } else if io.engine.is_null() {
                ui.label(RichText::new("no device (null)").color(theme::text_strong()));
            } else {
                ui.label(RichText::new(&info.host).color(theme::text_strong()));
            }
            ui.end_row();
            if io.engine.is_running() && !io.engine.is_null() {
                ui.label("Output");
                ui.label(info.output_device.as_deref().unwrap_or("default"));
                ui.end_row();
                ui.label("Input");
                ui.label(info.input_device.as_deref().unwrap_or("none"));
                ui.end_row();
            }
            engine_rows(ui, &info, &status);
            if let Some(st) = io.engine.stats() {
                if st.input_underruns + st.input_overruns + st.device_errors > 0 {
                    ui.label("Device");
                    ui.colored_label(
                        theme::warning(),
                        format!(
                            "{} under · {} over · {} errors",
                            st.input_underruns, st.input_overruns, st.device_errors
                        ),
                    );
                    ui.end_row();
                }
                if st.realtime_denied {
                    ui.label("Priority");
                    ui.colored_label(theme::warning(), "real-time scheduling was denied");
                    ui.end_row();
                }
                if st.disconnected {
                    ui.label("Status");
                    ui.colored_label(theme::error(), "device disconnected");
                    ui.end_row();
                }
            }
            ui.label("Latency");
            match &io.engine.latency {
                LatencyState::Idle => {
                    ui.label(RichText::new("not measured").color(theme::text_muted()));
                }
                LatencyState::Running => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("measuring…");
                    });
                }
                LatencyState::Done(Ok(l)) => {
                    ui.label(
                        RichText::new(format!("{:.2} ms round trip", l.ms))
                            .color(theme::text_strong()),
                    )
                    .on_hover_text(l.to_string());
                }
                LatencyState::Done(Err(e)) => {
                    ui.colored_label(theme::warning(), e.to_string());
                }
            }
            ui.end_row();
        });
    if let Some(e) = &io.engine.device_error {
        ui.colored_label(theme::warning(), format!("{} {e}", icons::WARNING));
        ui.label(
            RichText::new("Running on the silent null stream so the editor keeps working.")
                .small()
                .color(theme::text_faint()),
        );
    }
    for n in &info.notes {
        ui.label(
            RichText::new(format!("{} {n}", icons::INFO))
                .small()
                .color(theme::text_muted()),
        );
    }
    ui.add_space(4.0);
    let can_measure = io.engine.is_running() && !io.engine.is_null() && info.in_channels > 0;
    let r = ui.add_enabled(
        can_measure && io.engine.latency != LatencyState::Running,
        egui::Button::new(format!("{}  Measure latency", icons::TIMER)),
    );
    if r.clicked() {
        io.engine.measure_latency();
    }
    r.on_hover_text(
        "Plays a short burst on every output and listens on the inputs. Connect an output to an \
         input with a cable first (turn speakers down).",
    )
    .on_disabled_hover_text("Needs an audio device with an input.");

    section(ui, "Device");
    if state.hosts.is_none() {
        state.hosts = Some(ac_io::enumerate());
    }
    let hosts = state.hosts.clone().unwrap_or_default();
    let draft = state.draft.get_or_insert_with(|| io.engine.config.clone());
    if hosts.is_empty() {
        ui.label(RichText::new("No audio hosts on this machine.").color(theme::text_muted()));
    } else {
        let host_name = draft
            .host
            .clone()
            .or_else(|| hosts.iter().find(|h| h.default).map(|h| h.name.clone()))
            .unwrap_or_default();
        let host = hosts.iter().find(|h| h.name == host_name).or(hosts.first());
        egui::Grid::new("devices-pick")
            .num_columns(2)
            .spacing(egui::vec2(10.0, 6.0))
            .show(ui, |ui| {
                ui.label("Host");
                egui::ComboBox::from_id_salt("audio-host")
                    .selected_text(&host_name)
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for h in &hosts {
                            if ui.selectable_label(h.name == host_name, &h.name).clicked() {
                                draft.host = Some(h.name.clone());
                                draft.output_device = None;
                                draft.input = InputChoice::Default;
                            }
                        }
                    });
                ui.end_row();
                if let Some(h) = host {
                    ui.label("Output");
                    let out_name = draft.output_device.clone().unwrap_or_else(|| {
                        h.outputs
                            .iter()
                            .find(|d| d.default)
                            .map_or("default".into(), |d| d.name.clone())
                    });
                    egui::ComboBox::from_id_salt("audio-out")
                        .selected_text(&out_name)
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            for d in &h.outputs {
                                if ui.selectable_label(d.name == out_name, &d.name).clicked() {
                                    draft.output_device = Some(d.name.clone());
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Input");
                    let in_name = match &draft.input {
                        InputChoice::Default => "default".to_string(),
                        InputChoice::None => "none".to_string(),
                        InputChoice::Named(n) => n.clone(),
                    };
                    egui::ComboBox::from_id_salt("audio-in")
                        .selected_text(&in_name)
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.input == InputChoice::None, "none")
                                .clicked()
                            {
                                draft.input = InputChoice::None;
                            }
                            if ui
                                .selectable_label(draft.input == InputChoice::Default, "default")
                                .clicked()
                            {
                                draft.input = InputChoice::Default;
                            }
                            for d in &h.inputs {
                                if ui.selectable_label(in_name == d.name, &d.name).clicked() {
                                    draft.input = InputChoice::Named(d.name.clone());
                                }
                            }
                        });
                    ui.end_row();
                    let dev = h
                        .outputs
                        .iter()
                        .find(|d| Some(&d.name) == draft.output_device.as_ref())
                        .or_else(|| h.outputs.iter().find(|d| d.default));
                    ui.label("Sample rate");
                    let rate_text = draft
                        .sample_rate
                        .map_or("device default".into(), |r| format!("{r} Hz"));
                    egui::ComboBox::from_id_salt("audio-rate")
                        .selected_text(rate_text)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.sample_rate.is_none(), "device default")
                                .clicked()
                            {
                                draft.sample_rate = None;
                            }
                            for r in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
                                let ok = dev
                                    .and_then(|d| d.sample_rates)
                                    .is_none_or(|(lo, hi)| (lo..=hi).contains(&r));
                                if ok
                                    && ui
                                        .selectable_label(
                                            draft.sample_rate == Some(r),
                                            format!("{r} Hz"),
                                        )
                                        .clicked()
                                {
                                    draft.sample_rate = Some(r);
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Buffer");
                    let buf_text = draft
                        .buffer_size
                        .map_or("default".into(), |b| format!("{b} frames"));
                    egui::ComboBox::from_id_salt("audio-buffer")
                        .selected_text(buf_text)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(draft.buffer_size.is_none(), "default")
                                .clicked()
                            {
                                draft.buffer_size = None;
                            }
                            for b in [32u32, 64, 128, 256, 512, 1024, 2048] {
                                let ok = dev
                                    .and_then(|d| d.buffer_sizes)
                                    .is_none_or(|(lo, hi)| (lo..=hi).contains(&b));
                                if ok
                                    && ui
                                        .selectable_label(
                                            draft.buffer_size == Some(b),
                                            format!("{b} frames"),
                                        )
                                        .clicked()
                                {
                                    draft.buffer_size = Some(b);
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Engine threads");
                    let threads = draft.threads.max(1);
                    egui::ComboBox::from_id_salt("audio-threads")
                        .selected_text(if threads == 1 {
                            "1 (audio thread only)".to_string()
                        } else {
                            format!("{threads}")
                        })
                        .show_ui(ui, |ui| {
                            for t in 1..=ac_io::available_threads() {
                                let label = if t == 1 {
                                    "1 (audio thread only)".to_string()
                                } else {
                                    format!("{t}")
                                };
                                if ui.selectable_label(threads == t, label).clicked() {
                                    draft.threads = t;
                                }
                            }
                        })
                        .response
                        .on_hover_text(
                            "Run independent parts of the patch (voices, parallel chains) \
                             side by side. Extra threads keep a core busy while playing.",
                        );
                    ui.end_row();
                    if let Some(e) = &h.error {
                        ui.label("");
                        ui.colored_label(theme::warning(), e);
                        ui.end_row();
                    }
                }
            });
    }
    ui.horizontal(|ui| {
        let changed = state.draft.as_ref() != Some(&io.engine.config) || io.engine.is_null();
        if ui
            .add_enabled(
                !hosts.is_empty() && changed,
                egui::Button::new(format!("{}  Apply", icons::CHECK)),
            )
            .clicked()
            && let Some(d) = state.draft.clone()
        {
            io.requests.push(PanelRequest::SwitchAudio(d));
        }
        if ui.button(format!("{}  Refresh", icons::REDO)).clicked() {
            state.hosts = Some(ac_io::enumerate());
        }
    });

    section(ui, "MIDI inputs");
    match io.engine.midi.as_mut() {
        None => {
            ui.label(RichText::new("MIDI is off (no device runtime).").color(theme::text_muted()));
        }
        Some(m) => {
            let connected = m.connected();
            if let Some(e) = m.last_error() {
                ui.label(RichText::new(e.to_string()).color(theme::text_muted()));
            } else if connected.is_empty() {
                ui.label(RichText::new("No MIDI inputs found.").color(theme::text_muted()));
            }
            for p in &connected {
                ui.label(format!("{}  {p}", icons::MUSIC_NOTES));
            }
            if m.dropped() > 0 {
                ui.colored_label(theme::warning(), format!("{} events dropped", m.dropped()));
            }
            if !connected.is_empty() {
                // What each port last sent, to find a knob's controller
                // number or a pad's note without guessing.
                let seen = m.recent();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Last messages")
                            .small()
                            .color(theme::text_muted()),
                    );
                    if !seen.is_empty() && ui.small_button("Clear").clicked() {
                        m.clear_recent();
                    }
                });
                if seen.is_empty() {
                    ui.label(
                        RichText::new("None yet. Play or turn something.")
                            .color(theme::text_muted()),
                    );
                } else {
                    let now = ac_io::clock::now_ns();
                    let one_port = connected.len() == 1;
                    egui::ScrollArea::vertical()
                        .id_salt("midi-recent")
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for e in seen.iter().rev() {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(midi_text(&e.msg))
                                            .font(theme::mono_font(theme::FONT_SMALL)),
                                    );
                                    if !one_port {
                                        ui.label(
                                            RichText::new(&e.port)
                                                .small()
                                                .color(theme::text_muted()),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(age_text(now, e.time_ns))
                                            .small()
                                            .color(theme::text_faint()),
                                    );
                                });
                            }
                        });
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(250));
                }
            }
            if ui.button(format!("{}  Rescan", icons::REDO)).clicked() {
                io.engine.rescan_midi();
            }
        }
    }

    section(ui, "MIDI outputs");
    midi_outputs(ui, io, state);
    control_surfaces(ui, io);

    let choice = state
        .transport_choice
        .get_or_insert(ac_core::TransportAction::PlayStop);
    super::bindings_ui::transport_section(ui, io, choice);
}

fn midi_outputs(ui: &mut egui::Ui, io: &mut PanelIo, state: &mut DevicesState) {
    if io.engine.midi_out.is_none() {
        ui.label(RichText::new("MIDI is off (no device runtime).").color(theme::text_muted()));
        return;
    }
    if state.midi_outputs.is_none() {
        state.midi_outputs = Some(
            ac_io::list_outputs()
                .map(|ports| ports.into_iter().map(|p| p.name).collect())
                .unwrap_or_default(),
        );
    }
    let ports = state.midi_outputs.clone().unwrap_or_default();
    let choice = io.engine.midi_out_choice.clone();
    let mut pick = None;
    ui.label(RichText::new("MIDI Out nodes send to:").color(theme::text_muted()));
    if ui.radio(choice == MidiSelect::None, "nowhere").clicked() {
        pick = Some(MidiSelect::None);
    }
    for p in &ports {
        if ui
            .radio(choice == MidiSelect::Matching(p.clone()), p)
            .clicked()
        {
            pick = Some(MidiSelect::Matching(p.clone()));
        }
    }
    if ports.len() > 1 && ui.radio(choice == MidiSelect::All, "all ports").clicked() {
        pick = Some(MidiSelect::All);
    }
    if let Some(p) = pick {
        io.engine.set_midi_out(p);
    }
    if let Some(m) = io.engine.midi_out.as_ref() {
        if let Some(e) = m.last_error() {
            ui.label(RichText::new(e.to_string()).color(theme::text_muted()));
        } else if ports.is_empty() {
            ui.label(RichText::new("No MIDI outputs found.").color(theme::text_muted()));
        }
        if m.dropped() > 0 {
            ui.colored_label(
                theme::warning(),
                format!("{} messages dropped", m.dropped()),
            );
        }
    }
    if ui.button(format!("{}  Refresh", icons::REDO)).clicked() {
        state.midi_outputs = None;
        io.engine.rescan_midi();
    }
}

/// The one selected node, if it's an OSC In: the Use buttons' target.
fn selected_osc_in(io: &PanelIo) -> Option<ac_core::NodeId> {
    let mut ids = io.doc.selection.iter();
    let id = *ids.next()?;
    if ids.next().is_some() {
        return None;
    }
    (io.doc.graph().node(id)?.kind == "osc_in").then_some(id)
}

/// How long ago `t_ns` was, for the message lists.
fn age_text(now_ns: u64, t_ns: u64) -> String {
    let s = now_ns.saturating_sub(t_ns) as f64 / 1e9;
    if s < 1.0 {
        "now".to_string()
    } else if s < 60.0 {
        format!("{s:.0} s ago")
    } else {
        format!("{:.0} min ago", s / 60.0)
    }
}

/// A MIDI message as people read it ("Note on C4 · vel 100 · ch 1").
fn midi_text(msg: &ac_core::MidiMsg) -> String {
    use ac_core::MidiMsg;
    use ac_core::binding::note_name;
    let ch = |c: u8| format!("ch {}", u16::from(c) + 1);
    match *msg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity,
        } => format!(
            "Note on {} · vel {velocity} · {}",
            note_name(note),
            ch(channel)
        ),
        MidiMsg::NoteOff { channel, note, .. } => {
            format!("Note off {} · {}", note_name(note), ch(channel))
        }
        MidiMsg::ControlChange {
            channel,
            controller,
            value,
        } => format!("CC {controller} = {value} · {}", ch(channel)),
        MidiMsg::PitchBend { channel, value } => format!("Pitch bend {value} · {}", ch(channel)),
        MidiMsg::ChannelPressure { channel, value } => {
            format!("Pressure {value} · {}", ch(channel))
        }
        MidiMsg::PolyPressure {
            channel,
            note,
            value,
        } => format!("Pressure {} {value} · {}", note_name(note), ch(channel)),
        MidiMsg::ProgramChange { channel, program } => {
            format!("Program {program} · {}", ch(channel))
        }
    }
}

/// Gamepads, OSC in and out, and the computer keys.
fn control_surfaces(ui: &mut egui::Ui, io: &mut PanelIo) {
    let muted = |ui: &mut egui::Ui, t: &str| {
        ui.label(RichText::new(t).color(theme::text_muted()));
    };

    section(ui, "Gamepads");
    match io.engine.gamepads.as_ref() {
        None => muted(ui, "Gamepads are off (no device runtime)."),
        Some(g) => {
            let pads = g.connected();
            if let Some(e) = g.error() {
                muted(ui, &e);
            } else if pads.is_empty() {
                muted(ui, "No gamepads connected. Plug one in any time.");
            }
            for p in &pads {
                ui.label(format!(
                    "{}  {}  {}",
                    icons::GAME_CONTROLLER,
                    p.number(),
                    p.name
                ))
                .on_hover_text(format!("Pad {} in Gamepad nodes and bindings", p.number()));
            }
            // Hot-plugging shows up without a click.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    section(ui, "OSC");
    let listening = io.engine.osc_in.as_ref().map(|l| l.port());
    ui.horizontal(|ui| {
        let mut on = listening.is_some();
        let r = ui.checkbox(&mut on, "Listen on UDP port");
        let mut port = io.engine.osc_port;
        let field = ui.add(
            egui::DragValue::new(&mut port)
                .range(1024..=65535)
                .speed(1.0),
        );
        if field.changed() {
            io.engine.osc_port = port;
        }
        let retarget = listening.is_some_and(|p| p != io.engine.osc_port)
            && (field.drag_stopped() || field.lost_focus());
        if r.changed() || retarget {
            let want = on.then_some(io.engine.osc_port);
            io.engine.set_osc_in(want);
        }
        r.on_hover_text(
            "Receive OSC from phone and tablet apps or other programs on the network. Each \
             message argument is a control for OSC In nodes and bindings.",
        );
    });
    if let Some(e) = &io.engine.osc_error {
        ui.colored_label(theme::warning(), format!("{} {e}", icons::WARNING));
    }
    if let Some(l) = io.engine.osc_in.as_ref() {
        let port = l.port();
        let st = l.stats();
        let seen = l.recent();
        let to = match ac_io::osc::local_ip() {
            Some(ip) => format!("Send to {ip}:{port}"),
            None => format!("Send to this computer, port {port}"),
        };
        ui.label(RichText::new(to).color(theme::text_strong()));
        let mut line = format!("{} messages", st.messages);
        if st.malformed > 0 {
            line.push_str(&format!(" · {} not OSC", st.malformed));
        }
        if st.dropped > 0 {
            line.push_str(&format!(" · {} dropped", st.dropped));
        }
        ui.label(RichText::new(line).small().color(theme::text_muted()));
        if st.packets == 0 {
            // The usual silent failure: a phone or tablet refused the app's
            // local-network permission, so nothing ever leaves the device.
            ui.label(
                RichText::new(
                    "Nothing yet? Check the surface's host and port, and that the \
                     device lets its app use the local network (a phone or tablet \
                     asks once; if refused, the app sends nothing and shows no error).",
                )
                .small()
                .color(theme::text_muted()),
            );
        }
        // The newest messages as the surface wrote them, so an address can
        // be read off (or handed to a selected OSC In) instead of guessed.
        let target = selected_osc_in(io);
        let mut use_address: Option<String> = None;
        let mut clear = false;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Last messages")
                    .small()
                    .color(theme::text_muted()),
            );
            if !seen.is_empty() && ui.small_button("Clear").clicked() {
                clear = true;
            }
        });
        if seen.is_empty() {
            muted(ui, "None yet. Move a control on the surface.");
        } else {
            if target.is_some() {
                muted(ui, "Use sets the selected OSC In's address.");
            }
            let now = ac_io::clock::now_ns();
            egui::ScrollArea::vertical()
                .id_salt("osc-recent")
                .max_height(180.0)
                .show(ui, |ui| {
                    for m in seen.iter().rev() {
                        ui.horizontal(|ui| {
                            if target.is_some() && ui.small_button("Use").clicked() {
                                use_address = Some(m.address.clone());
                            }
                            let addr = ui.add(
                                egui::Label::new(
                                    RichText::new(&m.address)
                                        .font(theme::mono_font(theme::FONT_SMALL)),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if addr.on_hover_text("Click to copy").clicked() {
                                ui.ctx().copy_text(m.address.clone());
                            }
                            if !m.args.is_empty() {
                                ui.label(RichText::new(m.args.join("  ")).small());
                            }
                            ui.label(
                                RichText::new(age_text(now, m.time_ns))
                                    .small()
                                    .color(theme::text_faint()),
                            );
                        });
                    }
                });
        }
        if clear && let Some(l) = io.engine.osc_in.as_ref() {
            l.clear_recent();
        }
        if let (Some(id), Some(address)) = (target, use_address) {
            crate::params::set_text(io.doc, id, "address", address, io.now);
            io.doc.end_coalescing();
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }

    match io.engine.osc_out.as_ref() {
        None => muted(ui, "OSC Out is off (no device runtime)."),
        Some(o) => {
            let mut line = format!(
                "OSC Out nodes send to the address in their settings · {} sent",
                o.sent()
            );
            if o.errors() > 0 {
                line.push_str(&format!(" · {} failed", o.errors()));
            }
            if o.dropped() > 0 {
                line.push_str(&format!(" · {} dropped", o.dropped()));
            }
            ui.label(RichText::new(line).small().color(theme::text_muted()));
        }
    }

    section(ui, "Computer keys");
    ui.label(
        RichText::new(
            "Turn on Play keys (the keyboard button in the transport bar) to send key \
             presses to the patch instead of running shortcuts.",
        )
        .color(theme::text_muted()),
    );
    if let Some(c) = io.engine.controls.as_ref()
        && c.dropped() > 0
    {
        ui.colored_label(
            theme::warning(),
            format!("{} control events dropped", c.dropped()),
        );
    }
}

pub(super) fn problems(ui: &mut egui::Ui, io: &mut PanelIo) {
    let report = io.engine.report.clone();
    if report.diagnostics.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(icons::CHECK_CIRCLE)
                    .size(30.0)
                    .color(theme::ok()),
            );
            ui.label(RichText::new("No problems").color(theme::text_muted()));
        });
        return;
    }
    for d in &report.diagnostics {
        let (icon, color) = match d.severity {
            Severity::Error => (icons::WARNING_CIRCLE, theme::error()),
            Severity::Warning => (icons::WARNING, theme::warning()),
            Severity::Info => (icons::INFO, theme::accent()),
        };
        // The node id tells apart nodes that share a name (two "Math" nodes).
        let where_ = d
            .node
            .as_ref()
            .map(|p| format!("{} #{}", node_name(io, p), p.leaf().unwrap_or_default()));
        let text = match (&where_, &d.socket) {
            (Some(n), Some(s)) => format!("{n} · {s}"),
            (Some(n), None) => n.clone(),
            _ => "project".into(),
        };
        let resp = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 4))
            .corner_radius(theme::radius(5.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.colored_label(color, icon);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(text).small().color(theme::text_muted()));
                        ui.label(&d.message);
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_stroke(
                resp.rect,
                theme::radius(5.0),
                egui::Stroke::new(1.0, theme::border()),
                egui::StrokeKind::Inside,
            );
        }
        if resp.clicked()
            && let Some(p) = &d.node
        {
            io.requests.push(PanelRequest::Navigate(p.clone()));
        }
        resp.on_hover_text("Click to show the node");
    }
}

/// A readable name for a node path: labels or kind names, joined by `›`.
fn node_name(io: &PanelIo, path: &ac_core::NodePath) -> String {
    let mut graph = &io.doc.project.graph;
    let mut parts = Vec::new();
    for id in &path.0 {
        let Some(n) = graph.node(*id) else {
            parts.push(id.to_string());
            break;
        };
        let name = n.ui.label.clone().unwrap_or_else(|| match n.group_def() {
            Some(d) => io
                .doc
                .project
                .group_defs
                .get(&d)
                .map_or("Group".into(), |d| d.name.clone()),
            None => io
                .registry
                .get(&n.kind)
                .map_or(n.kind.clone(), |e| e.meta.name.to_string()),
        });
        parts.push(name);
        match n
            .group_def()
            .and_then(|d| io.doc.project.group_defs.get(&d))
        {
            Some(def) => graph = &def.graph,
            None => break,
        }
    }
    parts.join(" › ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::MidiMsg;

    #[test]
    fn midi_messages_read_as_people_do() {
        let on = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        };
        assert_eq!(midi_text(&on), "Note on C4 · vel 100 · ch 1");
        let cc = MidiMsg::ControlChange {
            channel: 9,
            controller: 74,
            value: 127,
        };
        assert_eq!(midi_text(&cc), "CC 74 = 127 · ch 10");
        let bend = MidiMsg::PitchBend {
            channel: 15,
            value: -8192,
        };
        assert_eq!(midi_text(&bend), "Pitch bend -8192 · ch 16");
    }

    #[test]
    fn ages_round_to_what_matters() {
        assert_eq!(age_text(1_000, 500), "now");
        assert_eq!(age_text(3_000_000_000, 500_000_000), "2 s ago"); // 2.5 s rounds half to even
        assert_eq!(age_text(130_000_000_000, 0), "2 min ago");
        assert_eq!(
            age_text(0, 10),
            "now",
            "a stamp from the future never underflows"
        );
    }
}
