//! The transport bar (UX.md): play/stop/record, position as bars:beats and
//! time, tempo and time signature, loop, master meter, load and xruns.

use ac_core::TempoMap;
use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::actions::Action;
use crate::audio::EngineHost;
use crate::canvas::bodies::meter_bar;
use crate::canvas::live::MeterChannel;
use crate::document::Document;
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::theme;

/// Pixels of a sideways drag on the position per beat.
const SCRUB_PX: f32 = 8.0;

/// Master meter ballistics, and the position's drag.
#[derive(Default)]
pub struct TransportState {
    meter: [MeterChannel; 2],
    last: f64,
    /// Dragging the position: the beat it started at.
    scrub: Option<f64>,
    /// A beat the playhead should move to, for the app to take.
    seek: Option<f64>,
    /// The bar typed into the position's popup.
    go_to: u32,
}

/// `bar:beat` (1-based) and `m:ss.mmm` for a sample position.
pub fn position_text(tempo: &TempoMap, sample: u64) -> (String, String) {
    let (bar, beat) = tempo.bar_at(sample);
    let bars = format!("{}:{:.2}", bar + 1, beat + 1.0);
    let secs = tempo.seconds_at(sample);
    let m = (secs / 60.0).floor();
    let time = format!("{}:{:06.3}", m as u64, secs - m * 60.0);
    (bars, time)
}

/// The beat bar `bar` (from 0) starts on, following time signature changes.
fn bar_start(tempo: &TempoMap, bar: f64) -> f64 {
    // Walk to it a bar at a time from where its time signature begins: few
    // songs change meter, and a bar is at least a beat.
    let mut beat = 0.0;
    let mut n = 0.0;
    while n < bar && beat < 1.0e6 {
        let [num, den] = tempo.time_sig_at(tempo.sample_at(beat));
        beat += f64::from(num.max(1)) * 4.0 / f64::from(den.max(1));
        n += 1.0;
    }
    beat
}

/// A transport button's icon.
fn big(icon: &str, on: bool, color: Color32) -> RichText {
    RichText::new(icon)
        .family(theme::icons_fill_family())
        .size(18.0)
        .color(if on { color } else { theme::text() })
}

impl TransportState {
    /// The beat the position's drag or popup asked to move to, once.
    pub fn take_seek(&mut self) -> Option<f64> {
        self.seek.take()
    }

    /// Go to start, play/stop, record, and loop: the editor's own
    /// transport (not shown when a host runs it).
    fn transport_buttons(
        &self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        playing: bool,
        recording: bool,
        actions: &mut Vec<Action>,
    ) {
        if icons::spoken(
            ui.add(
                egui::Button::new(big(icons::SKIP_BACK, false, theme::text()))
                    .min_size(vec2(32.0, 28.0)),
            ),
        )
        .on_hover_text(tr("transport-go-to-start"))
        .accessible_name(tr("transport-go-to-start"))
        .clicked()
        {
            actions.push(Action::Rewind);
        }
        // Play and Stop are separate, so Stop is there with the transport
        // already stopped: it still stops whatever buttons or live inputs
        // started (D200).
        if ui
            .add(
                egui::Button::new(big(icons::PLAY, playing, theme::play()))
                    .selected(playing)
                    .min_size(vec2(40.0, 28.0)),
            )
            .on_hover_text(tr("transport-play-tip"))
            .accessible_name(tr("transport-play"))
            .clicked()
            && !playing
        {
            actions.push(Action::PlayStop);
        }
        if ui
            .add(
                egui::Button::new(big(icons::STOP, false, theme::text()))
                    .min_size(vec2(32.0, 28.0)),
            )
            .on_hover_text(tr("transport-stop-tip"))
            .accessible_name(tr("transport-stop"))
            .clicked()
        {
            actions.push(Action::Stop);
        }
        if icons::spoken(
            ui.add(
                egui::Button::new(big(icons::RECORD, recording, theme::record()))
                    .selected(recording)
                    .min_size(vec2(32.0, 28.0)),
            ),
        )
        .on_hover_text(tr("transport-record-tip"))
        .accessible_name(tr("transport-record"))
        .clicked()
        {
            actions.push(Action::Record);
        }
        let looping = doc.project.settings.loop_region.is_some();
        if icons::spoken(
            ui.add(
                egui::Button::new(RichText::new(icons::REPEAT).size(17.0).color(if looping {
                    theme::accent()
                } else {
                    theme::text()
                }))
                .selected(looping)
                .min_size(vec2(32.0, 28.0)),
            ),
        )
        .on_hover_text(tr("transport-loop"))
        .accessible_name(tr("transport-loop"))
        .clicked()
        {
            actions.push(Action::ToggleLoop);
        }
        // The click, live only; its menu sets the count-in (D207).
        let click = doc.project.settings.metronome;
        let r = icons::spoken(
            ui.add(
                egui::Button::new(RichText::new(icons::METRONOME).size(17.0).color(if click {
                    theme::accent()
                } else {
                    theme::text()
                }))
                .selected(click)
                .min_size(vec2(32.0, 28.0)),
            ),
        )
        .on_hover_text(tr("transport-metronome-tip"))
        .accessible_name(tr("transport-metronome"));
        if r.clicked() {
            actions.push(Action::Metronome);
        }
        r.context_menu(|ui| {
            ui.label(tr("transport-count-in"));
            let now = doc.project.settings.count_in.min(ac_core::MAX_COUNT_IN);
            for bars in 0..=ac_core::MAX_COUNT_IN {
                let label = tr_args("transport-count-in-bars", &[("bars", bars.into())]);
                if ui.radio(now == bars, label).clicked() {
                    doc.edit_project(&tr("undo-count-in"), |p| p.settings.count_in = bars);
                    ui.close();
                }
            }
        });
        // Automation: moving controls while recording writes Graphs (D219).
        use ac_core::AutomationMode;
        let mode = doc.project.settings.automation;
        let on = !mode.is_off();
        let tip = match mode {
            AutomationMode::Off => tr("transport-automation-off-tip"),
            AutomationMode::Write => tr("transport-automation-write-tip"),
            AutomationMode::Touch => tr("transport-automation-touch-tip"),
        };
        let r = icons::spoken(
            ui.add(
                egui::Button::new(RichText::new(icons::SLIDERS).size(17.0).color(if on {
                    theme::record()
                } else {
                    theme::text()
                }))
                .selected(on)
                .min_size(vec2(32.0, 28.0)),
            ),
        )
        .on_hover_text(tip)
        .accessible_name(tr("transport-automation"));
        let mut set = None;
        if r.clicked() {
            set = Some(if on {
                AutomationMode::Off
            } else {
                AutomationMode::Touch
            });
        }
        r.context_menu(|ui| {
            for (m, label) in [
                (AutomationMode::Off, tr("transport-automation-off")),
                (AutomationMode::Write, tr("transport-automation-write")),
                (AutomationMode::Touch, tr("transport-automation-touch")),
            ] {
                if ui.radio(mode == m, label).clicked() {
                    set = Some(m);
                    ui.close();
                }
            }
        });
        if let Some(m) = set {
            doc.edit_project(&tr("undo-automation-mode"), |p| p.settings.automation = m);
        }
        panic_button(ui, actions);
    }

    /// Draw the bar; returns commands to run. `panels`: the panels are
    /// shown instead of the canvas.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        engine: &EngineHost,
        panels: bool,
        play_keys: bool,
        now: f64,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let status = engine.handle.status().clone();
        let playing = engine.handle.is_playing();
        let recording = status.recording.load(std::sync::atomic::Ordering::Relaxed);
        let rate = f64::from(engine.info.sample_rate);
        let tempo = TempoMap::new(rate, &doc.project.settings.tempo);
        let pos = engine.handle.position();

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if engine.in_host() {
                // A plugin follows the host's transport: show it, and offer
                // no buttons that would fight it.
                let (icon, state) = if playing {
                    (icons::PLAY, tr("transport-host-playing"))
                } else {
                    (icons::STOP, tr("transport-host-stopped"))
                };
                ui.add_sized(
                    vec2(40.0, 28.0),
                    egui::Label::new(big(icon, playing, theme::play())),
                )
                .on_hover_text(tr_args("transport-host-tip", &[("state", (&state).into())]))
                .accessible_name(state);
            } else {
                self.transport_buttons(ui, doc, playing, recording, &mut actions);
            }
            if engine.keys_reach_engine()
                && icons::spoken(
                    ui.add(
                        egui::Button::new(RichText::new(icons::KEYBOARD).size(17.0).color(
                            if play_keys {
                                theme::accent()
                            } else {
                                theme::text()
                            },
                        ))
                        .selected(play_keys)
                        .min_size(vec2(32.0, 28.0)),
                    ),
                )
                .on_hover_text({
                    let sc = Action::PlayKeys
                        .shortcut()
                        .map(|s| ui.ctx().format_shortcut(&s))
                        .unwrap_or_default();
                    if play_keys {
                        tr_args("transport-play-keys-on-tip", &[("shortcut", sc.into())])
                    } else {
                        tr_args("transport-play-keys-tip", &[("shortcut", sc.into())])
                    }
                })
                .accessible_name(tr("transport-play-keys"))
                .clicked()
            {
                actions.push(Action::PlayKeys);
            }
            ui.add_space(10.0);
            // Position: bars:beats, large; time underneath. Drag it sideways
            // to move the playhead by beats; click it to go to a bar.
            let (bars, time) = position_text(&tempo, pos);
            let (rect, position) = ui.allocate_exact_size(
                vec2(160.0, 32.0),
                if engine.in_host() {
                    Sense::hover()
                } else {
                    Sense::click_and_drag()
                },
            );
            if position.drag_started() {
                self.scrub = Some(tempo.beat_at(pos).round());
            }
            if let Some(from) = self.scrub
                && position.dragged()
            {
                let dx = position.total_drag_delta().map_or(0.0, |d| d.x);
                let beat = (from + f64::from((dx / SCRUB_PX).round())).max(0.0);
                if (tempo.beat_at(pos) - beat).abs() > 1e-6 {
                    self.seek = Some(beat);
                }
            }
            if position.drag_stopped() {
                self.scrub = None;
            }
            if position.clicked() {
                self.go_to = (tempo.bar_at(pos).0 + 1).max(1) as u32;
            }
            let position = position.on_hover_text(tr("transport-position-tip"));
            egui::Popup::from_toggle_button_response(&position).show(|ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("transport-go-to-bar"));
                    let r = ui.add(egui::DragValue::new(&mut self.go_to).range(1..=9999));
                    let go = ui.button(icons::PLAY).clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if go {
                        let bar = f64::from(self.go_to.saturating_sub(1));
                        self.seek = Some(bar_start(&tempo, bar));
                        ui.close();
                    }
                });
            });
            let counting = engine.handle.is_counting_in();
            // A screen reader reads the position as one readout.
            let spoken = tr_args(
                "transport-position",
                &[("bars", (&bars).into()), ("time", (&time).into())],
            );
            position.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &spoken));
            ui.painter()
                .rect_filled(rect, theme::radius(6.0), theme::well_bg());
            ui.painter().text(
                pos2(rect.min.x + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                if counting {
                    tr("transport-counting-in")
                } else {
                    bars
                },
                theme::mono_font(16.0),
                if playing {
                    theme::text_strong()
                } else {
                    theme::text()
                },
            );
            ui.painter().text(
                pos2(rect.max.x - 10.0, rect.center().y + 1.0),
                Align2::RIGHT_CENTER,
                time,
                theme::mono_font(theme::FONT_SMALL),
                theme::text_muted(),
            );
            ui.add_space(6.0);

            // Tempo and time signature of the first segment, click to edit.
            let seg = doc
                .project
                .settings
                .tempo
                .first()
                .cloned()
                .unwrap_or_default();
            let extra = doc.project.settings.tempo.len().saturating_sub(1);
            let tempo_btn = ui
                .add(
                    egui::Button::new(
                        RichText::new(tr_args(
                            "transport-tempo",
                            &[
                                ("bpm", format!("{:.1}", seg.bpm).into()),
                                ("beats", seg.time_sig[0].to_string().into()),
                                ("unit", seg.time_sig[1].to_string().into()),
                                (
                                    "more",
                                    if extra > 0 {
                                        format!("  +{extra}")
                                    } else {
                                        String::new()
                                    }
                                    .into(),
                                ),
                            ],
                        ))
                        .font(theme::mono_font(theme::FONT_VALUE + 0.5)),
                    )
                    .min_size(vec2(0.0, 28.0)),
                )
                .on_hover_text(tr("transport-tempo-tip"));
            egui::Popup::from_toggle_button_response(&tempo_btn).show(|ui| {
                ui.set_min_width(200.0);
                let mut s = seg.clone();
                let mut changed = false;
                egui::Grid::new("tempo-popup")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(tr("transport-tempo-label"));
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.bpm)
                                    .range(1.0..=999.0)
                                    .speed(0.1)
                                    .fixed_decimals(1)
                                    .suffix(tr("transport-bpm-suffix")),
                            )
                            .changed();
                        ui.end_row();
                        ui.label(tr("transport-beats-per-bar"));
                        changed |= ui
                            .add(egui::DragValue::new(&mut s.time_sig[0]).range(1..=32))
                            .changed();
                        ui.end_row();
                        ui.label(tr("transport-beat-unit"));
                        egui::ComboBox::from_id_salt("tempo-den")
                            .selected_text(s.time_sig[1].to_string())
                            .show_ui(ui, |ui| {
                                for d in [1u8, 2, 4, 8, 16, 32] {
                                    changed |= ui
                                        .selectable_value(&mut s.time_sig[1], d, d.to_string())
                                        .changed();
                                }
                            });
                        ui.end_row();
                    });
                if changed {
                    doc.edit_settings("tempo0", now, |p| {
                        if p.settings.tempo.is_empty() {
                            p.settings.tempo.push(s.clone());
                        } else {
                            p.settings.tempo[0] = ac_core::TempoSegment {
                                beat: 0.0,
                                ..s.clone()
                            };
                        }
                    });
                }
            });

            // Loop range, shown when looping.
            if let Some([a, b]) = doc.project.settings.loop_region {
                let ta = TempoMap::new(rate, &doc.project.settings.tempo);
                let fmt = |beat: f64| {
                    let (bar, bb) = ta.bar_at(ta.sample_at(beat));
                    format!("{}:{:.0}", bar + 1, bb + 1.0)
                };
                let r = ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("{} – {}", fmt(a), fmt(b)))
                                .font(theme::mono_font(theme::FONT_VALUE)),
                        )
                        .min_size(vec2(0.0, 28.0)),
                    )
                    .on_hover_text(tr("transport-loop-range-tip"));
                egui::Popup::from_toggle_button_response(&r).show(|ui| {
                    let (mut a2, mut b2) = (a, b);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label(tr("transport-loop-beats"));
                        changed |= ui
                            .add(egui::DragValue::new(&mut a2).range(0.0..=1.0e6).speed(0.25))
                            .changed();
                        ui.label("–");
                        changed |= ui
                            .add(egui::DragValue::new(&mut b2).range(0.0..=1.0e6).speed(0.25))
                            .changed();
                    });
                    if changed {
                        let region = [a2.min(b2), a2.max(b2).max(a2.min(b2) + 0.25)];
                        doc.edit_settings("loop", now, |p| p.settings.loop_region = Some(region));
                    }
                });
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                view_switch(ui, panels, &mut actions);
                ui.add_space(10.0);
                // Xruns and load.
                let xruns = status.total_xruns();
                let x_color = if xruns > 0 {
                    theme::warning()
                } else {
                    theme::text_muted()
                };
                ui.label(
                    RichText::new(format!("{} {xruns}", icons::LIGHTNING))
                        .color(x_color)
                        .font(theme::mono_font(theme::FONT_SMALL)),
                )
                .on_hover_text(tr("transport-xruns-tip"))
                .accessible_name(tr_args("transport-xruns", &[("count", xruns.into())]));
                let load = status.load_fraction();
                let l_color = if load > 0.8 {
                    theme::error()
                } else if load > 0.5 {
                    theme::warning()
                } else {
                    theme::text_muted()
                };
                ui.label(
                    RichText::new(format!(
                        "{} {:>3.0}%",
                        icons::CPU,
                        (load * 100.0).clamp(0.0, 999.0)
                    ))
                    .color(l_color)
                    .font(theme::mono_font(theme::FONT_SMALL)),
                )
                .on_hover_text(tr("transport-load-tip"))
                .accessible_name(tr_args(
                    "transport-load",
                    &[(
                        "percent",
                        format!("{:.0}", (load * 100.0).clamp(0.0, 999.0)).into(),
                    )],
                ));
                ui.add_space(8.0);
                self.master_meter(ui, status.master_peak(), now);
            });
        });
        actions
    }

    fn master_meter(&mut self, ui: &mut egui::Ui, peak: [f32; 2], now: f64) {
        let dt = (now - self.last).clamp(0.0, 0.25) as f32;
        self.last = now;
        let fall = 10f32.powf(-24.0 * dt / 20.0);
        for (c, p) in self.meter.iter_mut().zip(peak) {
            let p = if p.is_finite() { p.max(0.0) } else { 0.0 };
            c.peak = p.max(c.peak * fall);
            c.rms = c.peak;
            if p >= c.hold || now - c.hold_at > 1.5 {
                c.hold = p.max(c.peak);
                c.hold_at = now;
            }
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(150.0, 24.0), Sense::hover());
        let bars = Rect::from_min_max(rect.min + vec2(0.0, 3.0), rect.max - vec2(0.0, 3.0));
        let h = (bars.height() - 2.0) / 2.0;
        for (i, c) in self.meter.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(bars.min.x, bars.min.y + i as f32 * (h + 2.0)),
                vec2(bars.width(), h),
            );
            meter_bar(ui.painter(), r, 1.0, *c, 1.0);
        }
        ui.painter().rect_stroke(
            bars.expand(1.0),
            theme::radius(3.0),
            Stroke::new(1.0, theme::border()),
            StrokeKind::Outside,
        );
        if self.meter.iter().any(|c| c.peak > 1e-5) {
            ui.ctx().request_repaint();
        }
        let hold = self.meter[0].hold.max(self.meter[1].hold);
        let text = if hold < 1e-6 {
            tr("transport-master-silent")
        } else {
            tr_args(
                "transport-master-peak",
                &[("db", format!("{:.1}", 20.0 * hold.log10()).into())],
            )
        };
        // A meter to screen readers, its peak in its name.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Meter, true, &text));
        resp.on_hover_text(text);
    }
}

/// Panic (D201): stop, reset, and silence everything; `Escape` twice.
pub fn panic_button(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    if icons::spoken(
        ui.add(
            egui::Button::new(
                RichText::new(icons::WARNING_OCTAGON)
                    .size(17.0)
                    .color(theme::warning()),
            )
            .min_size(vec2(32.0, 28.0)),
        ),
    )
    .on_hover_text(tr("transport-panic-tip"))
    .accessible_name(tr("transport-panic"))
    .clicked()
    {
        actions.push(Action::Panic);
    }
}

/// Canvas or panels, and performance mode (right to left).
fn view_switch(ui: &mut egui::Ui, panels: bool, actions: &mut Vec<Action>) {
    let perform = icons::spoken(ui.add(
        egui::Button::new(RichText::new(icons::CORNERS_OUT).size(16.0)).min_size(vec2(30.0, 28.0)),
    ))
    .on_hover_text(tr("transport-performance-tip"))
    .accessible_name(Action::PerformanceMode.label());
    if perform.clicked() {
        actions.push(Action::PerformanceMode);
    }
    for (on, icon, label, tip) in [
        (
            true,
            icons::SQUARES_FOUR,
            tr("transport-panels"),
            tr("transport-panels-tip"),
        ),
        (
            false,
            icons::TREE_STRUCTURE,
            tr("transport-canvas"),
            tr("transport-canvas-tip"),
        ),
    ] {
        let b = egui::Button::new(RichText::new(format!("{icon}  {label}")).size(theme::FONT_UI))
            .selected(panels == on)
            .min_size(vec2(0.0, 28.0));
        let r = ui.add(b);
        // A switch between two views: each is a tab, the one shown selected.
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, panels == on, &label));
        if r.on_hover_text(tip).clicked() && panels != on {
            actions.push(Action::TogglePanels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_read_as_bars_beats_and_time() {
        let t = TempoMap::constant(48_000.0, 120.0);
        assert_eq!(position_text(&t, 0), ("1:1.00".into(), "0:00.000".into()));
        // Two beats in at 120 BPM: bar 1, beat 3; one second.
        assert_eq!(
            position_text(&t, 48_000),
            ("1:3.00".into(), "0:01.000".into())
        );
        // Five bars of 4/4 at 120 BPM is ten seconds.
        assert_eq!(position_text(&t, 480_000).0, "6:1.00");
        assert_eq!(position_text(&t, 48_000 * 75).1, "1:15.000");
    }
}
