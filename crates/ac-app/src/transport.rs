//! The transport bar (UX.md): play/stop/record, position as bars:beats and
//! time, tempo and time signature, loop, master meter, load and xruns.

use ac_core::TempoMap;
use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::actions::Action;
use crate::audio::EngineHost;
use crate::canvas::bodies::meter_bar;
use crate::canvas::live::MeterChannel;
use crate::document::Document;
use crate::icons;
use crate::theme;

/// Master meter ballistics.
#[derive(Default)]
pub struct TransportState {
    meter: [MeterChannel; 2],
    last: f64,
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

impl TransportState {
    /// Draw the bar; returns commands to run.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        engine: &EngineHost,
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
            let big = |icon: &str, on: bool, color: Color32| {
                RichText::new(icon)
                    .family(theme::icons_fill_family())
                    .size(18.0)
                    .color(if on { color } else { theme::TEXT })
            };
            if ui
                .add(
                    egui::Button::new(big(icons::SKIP_BACK, false, theme::TEXT))
                        .min_size(vec2(32.0, 28.0)),
                )
                .on_hover_text("Go to start")
                .clicked()
            {
                actions.push(Action::Rewind);
            }
            let play = if playing { icons::STOP } else { icons::PLAY };
            if ui
                .add(
                    egui::Button::new(big(play, playing, theme::PLAY))
                        .selected(playing)
                        .min_size(vec2(40.0, 28.0)),
                )
                .on_hover_text(if playing {
                    "Stop (Space)"
                } else {
                    "Play (Space) · from start: Shift+Space"
                })
                .clicked()
            {
                actions.push(Action::PlayStop);
            }
            if ui
                .add(
                    egui::Button::new(big(icons::RECORD, recording, theme::RECORD))
                        .selected(recording)
                        .min_size(vec2(32.0, 28.0)),
                )
                .on_hover_text("Record (R)")
                .clicked()
            {
                actions.push(Action::Record);
            }
            let looping = doc.project.settings.loop_region.is_some();
            if ui
                .add(
                    egui::Button::new(RichText::new(icons::REPEAT).size(17.0).color(if looping {
                        theme::ACCENT
                    } else {
                        theme::TEXT
                    }))
                    .selected(looping)
                    .min_size(vec2(32.0, 28.0)),
                )
                .on_hover_text("Loop")
                .clicked()
            {
                actions.push(Action::ToggleLoop);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(icons::KEYBOARD).size(17.0).color(
                        if play_keys {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        },
                    ))
                    .selected(play_keys)
                    .min_size(vec2(32.0, 28.0)),
                )
                .on_hover_text({
                    let sc = Action::PlayKeys
                        .shortcut()
                        .map(|s| ui.ctx().format_shortcut(&s))
                        .unwrap_or_default();
                    if play_keys {
                        format!("Play keys is on: computer keys go to the patch ({sc})")
                    } else {
                        format!(
                            "Play keys: send computer keys to the patch instead of shortcuts \
                             ({sc})"
                        )
                    }
                })
                .clicked()
            {
                actions.push(Action::PlayKeys);
            }
            ui.add_space(10.0);

            // Position: bars:beats, large; time underneath.
            let (bars, time) = position_text(&tempo, pos);
            let (rect, _) = ui.allocate_exact_size(vec2(160.0, 32.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, theme::radius(6.0), theme::WELL_BG);
            ui.painter().text(
                pos2(rect.min.x + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                bars,
                theme::mono_font(16.0),
                if playing {
                    theme::TEXT_STRONG
                } else {
                    theme::TEXT
                },
            );
            ui.painter().text(
                pos2(rect.max.x - 10.0, rect.center().y + 1.0),
                Align2::RIGHT_CENTER,
                time,
                theme::mono_font(theme::FONT_SMALL),
                theme::TEXT_MUTED,
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
                        RichText::new(format!(
                            "{:.1} BPM  {}/{}{}",
                            seg.bpm,
                            seg.time_sig[0],
                            seg.time_sig[1],
                            if extra > 0 {
                                format!("  +{extra}")
                            } else {
                                String::new()
                            }
                        ))
                        .font(theme::mono_font(theme::FONT_VALUE + 0.5)),
                    )
                    .min_size(vec2(0.0, 28.0)),
                )
                .on_hover_text(
                    "Tempo and time signature (click to edit; more changes in the Project tab)",
                );
            egui::Popup::from_toggle_button_response(&tempo_btn).show(|ui| {
                ui.set_min_width(200.0);
                let mut s = seg.clone();
                let mut changed = false;
                egui::Grid::new("tempo-popup")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Tempo");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut s.bpm)
                                    .range(1.0..=999.0)
                                    .speed(0.1)
                                    .fixed_decimals(1)
                                    .suffix(" BPM"),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Beats per bar");
                        changed |= ui
                            .add(egui::DragValue::new(&mut s.time_sig[0]).range(1..=32))
                            .changed();
                        ui.end_row();
                        ui.label("Beat unit");
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
                    .on_hover_text("Loop range (click to edit)");
                egui::Popup::from_toggle_button_response(&r).show(|ui| {
                    let (mut a2, mut b2) = (a, b);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("Beats");
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
                // Xruns and load.
                let xruns = status.total_xruns();
                let x_color = if xruns > 0 {
                    theme::WARNING
                } else {
                    theme::TEXT_MUTED
                };
                ui.label(
                    RichText::new(format!("{} {xruns}", icons::LIGHTNING))
                        .color(x_color)
                        .font(theme::mono_font(theme::FONT_SMALL)),
                )
                .on_hover_text("Xruns: blocks that missed their deadline");
                let load = status.load_fraction();
                let l_color = if load > 0.8 {
                    theme::ERROR
                } else if load > 0.5 {
                    theme::WARNING
                } else {
                    theme::TEXT_MUTED
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
                .on_hover_text("Audio load: the last block's processing time against its budget");
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
            Stroke::new(1.0, theme::BORDER),
            StrokeKind::Outside,
        );
        if self.meter.iter().any(|c| c.peak > 1e-5) {
            ui.ctx().request_repaint();
        }
        let hold = self.meter[0].hold.max(self.meter[1].hold);
        resp.on_hover_text(if hold < 1e-6 {
            "Master: silent".to_string()
        } else {
            format!("Master peak {:.1} dBFS", 20.0 * hold.log10())
        });
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
