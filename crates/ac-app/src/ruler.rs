//! The ruler under the transport bar (UX.md "Transport bar"): the song's
//! bars with the playhead, the loop, the punch range, and the markers.
//! Click to move the playhead there (snapped to the beat), drag to set the
//! loop, `Shift`-drag to set the punch range, click a marker to jump to it
//! or drag it to move it; right-click for markers and to clear the loop or
//! punch range (DECISIONS D207).

use ac_core::{Marker, Project, TempoMap};
use egui::{Align2, Pos2, Rect, Sense, Stroke, pos2};

use crate::document::Document;
use crate::i18n::{tr, tr_args};
use crate::theme;

/// Left and right margin inside the ruler.
const MARGIN: f32 = 10.0;
/// Narrowest room for a bar number.
const LABEL_ROOM: f32 = 30.0;
/// How near the pointer must be to grab a marker.
const MARKER_GRAB: f32 = 6.0;

/// A drag under way.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// The loop (or with `punch`, the punch range) from `anchor` to the
    /// pointer, in beats.
    Range { anchor: f64, punch: bool },
    /// A marker moving.
    Marker { index: usize },
}

/// The ruler's own state between frames.
#[derive(Default)]
pub struct Ruler {
    drag: Option<Drag>,
    /// The marker a right-click opened the menu on, and the beat it was at.
    menu: Option<(Option<usize>, f64)>,
}

/// Beats to the end of the bar holding `beat`.
fn bar_beats(tempo: &TempoMap, beat: f64) -> f64 {
    let s = tempo.sample_at(beat);
    let [num, den] = tempo.time_sig_at(s);
    f64::from(num.max(1)) * 4.0 / f64::from(den.max(1))
}

/// How the ruler maps beats to x and back.
#[derive(Clone, Copy)]
struct Scale {
    left: f32,
    width: f32,
    beats: f64,
}

impl Scale {
    fn x(&self, beat: f64) -> f32 {
        self.left + (beat / self.beats) as f32 * self.width
    }

    fn beat(&self, x: f32) -> f64 {
        (f64::from((x - self.left) / self.width) * self.beats).clamp(0.0, self.beats)
    }

    /// Beats a snapped position moves by: a beat when beats are wide
    /// enough to aim at, else a bar of `bar` beats.
    fn snap_unit(&self, bar: f64) -> f64 {
        if self.width / self.beats as f32 >= 6.0 {
            1.0
        } else {
            bar
        }
    }
}

/// The span the ruler shows, in beats: the song, the loop, the punch
/// range, the markers, and the playhead, in whole bars plus one.
fn span(project: &Project, tempo: &TempoMap, position: f64) -> f64 {
    let s = &project.settings;
    let length = if s.length.is_finite() {
        tempo.beat_at((s.length.max(0.0) * tempo.sample_rate()) as u64)
    } else {
        0.0
    };
    let mut end = length.max(position);
    for r in [s.loop_region, s.punch].into_iter().flatten() {
        end = end.max(r[1]);
    }
    for m in s.markers.iter().take(ac_core::MAX_MARKERS) {
        if m.beat.is_finite() {
            end = end.max(m.beat);
        }
    }
    let bar = bar_beats(tempo, 0.0);
    let bars = (end / bar).ceil() + 1.0;
    if bars.is_finite() {
        bars.clamp(8.0, 4096.0) * bar
    } else {
        8.0 * bar
    }
}

impl Ruler {
    /// Draw the ruler and handle its pointer gestures. `rate` is the
    /// engine's sample rate and `position` the playhead. Returns the beat
    /// to move the playhead to, when a click asked for one.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        rate: f64,
        position: u64,
        now: f64,
    ) -> Option<f64> {
        let tempo = TempoMap::new(rate, &doc.project.settings.tempo);
        let head = tempo.beat_at(position);
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, tr("ruler-name")));
        let scale = Scale {
            left: rect.left() + MARGIN,
            width: (rect.width() - 2.0 * MARGIN).max(1.0),
            beats: span(&doc.project, &tempo, head),
        };
        let bar = bar_beats(&tempo, 0.0);
        let unit = scale.snap_unit(bar);
        let snap = |b: f64| (b / unit).round() * unit;
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let marker_at = |p: Pos2, markers: &[Marker]| {
            markers
                .iter()
                .take(ac_core::MAX_MARKERS)
                .enumerate()
                .filter(|(_, m)| m.beat.is_finite())
                .map(|(i, m)| (i, (scale.x(m.beat) - p.x).abs()))
                .filter(|(_, d)| *d <= MARKER_GRAB)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let mut seek = None;
        let shift = ui.input(|i| i.modifiers.shift);

        // A drag starts where the button went down, not where it had moved
        // to by the time it counted as a drag.
        let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        if resp.drag_started()
            && let Some(p) = origin
        {
            self.drag = Some(match marker_at(p, &doc.project.settings.markers) {
                Some(index) => Drag::Marker { index },
                None => Drag::Range {
                    anchor: snap(scale.beat(p.x)),
                    punch: shift,
                },
            });
        }
        if let (Some(Drag::Marker { index }), Some(p)) = (self.drag, pointer)
            && resp.dragged()
        {
            let beat = snap(scale.beat(p.x));
            doc.edit_settings("marker-move", now, |pr| {
                if let Some(m) = pr.settings.markers.get_mut(index) {
                    m.beat = beat;
                }
            });
        }
        let preview = match (self.drag, pointer) {
            (Some(Drag::Range { anchor, punch }), Some(p)) => {
                let b = snap(scale.beat(p.x));
                (b != anchor).then(|| (anchor.min(b), anchor.max(b), punch))
            }
            _ => None,
        };
        if resp.drag_stopped() {
            if let Some((a, b, punch)) = preview {
                if punch {
                    doc.edit_project(&tr("undo-punch"), |p| p.settings.punch = Some([a, b]));
                } else {
                    doc.edit_project(&tr("undo-loop"), |p| p.settings.loop_region = Some([a, b]));
                }
            }
            self.drag = None;
        }
        if resp.clicked()
            && let Some(p) = pointer
        {
            seek = Some(match marker_at(p, &doc.project.settings.markers) {
                Some(i) => doc.project.settings.markers[i].beat.max(0.0),
                None => snap(scale.beat(p.x)),
            });
        }
        if resp.secondary_clicked()
            && let Some(p) = pointer
        {
            self.menu = Some((
                marker_at(p, &doc.project.settings.markers),
                snap(scale.beat(p.x)),
            ));
        }
        self.context_menu(&resp, doc, now);

        // --- drawing ---
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::well_bg());
        let s = &doc.project.settings;
        let shade = |a: f64, b: f64, color: egui::Color32, band: Rect| {
            let r = Rect::from_x_y_ranges(scale.x(a)..=scale.x(b), band.y_range());
            painter.rect_filled(r, theme::radius(2.0), color);
        };
        let looped = preview
            .filter(|p| !p.2)
            .map(|p| [p.0, p.1])
            .or(s.loop_region);
        if let Some([a, b]) = looped {
            shade(a, b, theme::accent().gamma_multiply(0.25), rect);
        }
        let punched = preview.filter(|p| p.2).map(|p| [p.0, p.1]).or(s.punch);
        if let Some([a, b]) = punched {
            let band = Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 4.0..=rect.bottom());
            shade(a, b, theme::record(), band);
        }
        let px_bar = scale.width * (bar / scale.beats) as f32;
        // Number every bar, or every second, fourth... where they're narrow.
        let mut every = 1u32;
        while px_bar * (every as f32) < LABEL_ROOM && every < 4096 {
            every *= 2;
        }
        let bars = (scale.beats / bar).round() as u32;
        for b in 0..=bars {
            let x = scale.x(f64::from(b) * bar);
            let labelled = b % every == 0;
            let top = if labelled {
                rect.top() + 2.0
            } else {
                rect.center().y
            };
            painter.line_segment(
                [pos2(x, top), pos2(x, rect.bottom())],
                Stroke::new(1.0, theme::text_faint()),
            );
            if labelled && b < bars {
                painter.text(
                    pos2(x + 3.0, rect.top() + 1.0),
                    Align2::LEFT_TOP,
                    (b + 1).to_string(),
                    theme::mono_font(theme::FONT_SMALL - 1.0),
                    theme::text_muted(),
                );
            }
        }
        if px_bar / bar as f32 >= 6.0 {
            let beats = scale.beats.round() as u32;
            for k in 0..beats {
                if f64::from(k) % bar != 0.0 {
                    let x = scale.x(f64::from(k));
                    painter.line_segment(
                        [pos2(x, rect.bottom() - 4.0), pos2(x, rect.bottom())],
                        Stroke::new(1.0, theme::text_faint()),
                    );
                }
            }
        }
        for m in s.markers.iter().take(ac_core::MAX_MARKERS) {
            if !m.beat.is_finite() {
                continue;
            }
            let x = scale.x(m.beat);
            let flag = [
                pos2(x, rect.top()),
                pos2(x + 7.0, rect.top() + 4.0),
                pos2(x, rect.top() + 8.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                flag.to_vec(),
                theme::warning(),
                Stroke::NONE,
            ));
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, theme::warning()),
            );
            if !m.name.is_empty() {
                painter.text(
                    pos2(x + 9.0, rect.center().y + 1.0),
                    Align2::LEFT_CENTER,
                    &m.name,
                    theme::mono_font(theme::FONT_SMALL - 1.0),
                    theme::text(),
                );
            }
        }
        let x = scale.x(head.min(scale.beats));
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(2.0, theme::play()),
        );
        if resp.hovered() && self.drag.is_none() {
            resp.clone().on_hover_text(tr("ruler-tip"));
        }
        seek
    }

    /// The right-click menu: add a marker, rename or remove the one under
    /// the pointer, clear the loop or the punch range.
    fn context_menu(&mut self, resp: &egui::Response, doc: &mut Document, now: f64) {
        let Some((marker, beat)) = self.menu else {
            return;
        };
        let mut close = false;
        resp.context_menu(|ui| {
            match marker {
                Some(i) => {
                    let mut name = doc
                        .project
                        .settings
                        .markers
                        .get(i)
                        .map(|m| m.name.clone())
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(tr("ruler-marker-name"));
                        if ui.text_edit_singleline(&mut name).changed() {
                            doc.edit_settings("marker-name", now, |p| {
                                if let Some(m) = p.settings.markers.get_mut(i) {
                                    m.name = name.clone();
                                }
                            });
                        }
                    });
                    if ui.button(tr("ruler-remove-marker")).clicked() {
                        doc.edit_project(&tr("undo-marker"), |p| {
                            if i < p.settings.markers.len() {
                                p.settings.markers.remove(i);
                            }
                        });
                        close = true;
                    }
                }
                None => {
                    if ui.button(tr("ruler-add-marker")).clicked() {
                        add_marker(doc, beat);
                        close = true;
                    }
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    doc.project.settings.loop_region.is_some(),
                    egui::Button::new(tr("ruler-clear-loop")),
                )
                .clicked()
            {
                doc.edit_project(&tr("undo-loop"), |p| p.settings.loop_region = None);
                close = true;
            }
            if ui
                .add_enabled(
                    doc.project.settings.punch.is_some(),
                    egui::Button::new(tr("ruler-clear-punch")),
                )
                .clicked()
            {
                doc.edit_project(&tr("undo-punch"), |p| p.settings.punch = None);
                close = true;
            }
            if close {
                ui.close();
            }
        });
        if close {
            self.menu = None;
        }
    }
}

/// Add a marker at `beat`, named after how many there are, in beat order.
pub fn add_marker(doc: &mut Document, beat: f64) {
    if doc.project.settings.markers.len() >= ac_core::MAX_MARKERS {
        return;
    }
    let n = doc.project.settings.markers.len() + 1;
    let name = tr_args("ruler-marker-n", &[("n", n.into())]);
    doc.edit_project(&tr("undo-marker"), |p| {
        let m = &mut p.settings.markers;
        let at = m.partition_point(|x| x.beat <= beat);
        m.insert(
            at,
            Marker {
                beat: beat.max(0.0),
                name,
            },
        );
    });
}

/// The marker before (`-1`) or after (`1`) `beat`, in beats.
pub fn next_marker(project: &Project, beat: f64, direction: i32) -> Option<f64> {
    let beats = project
        .settings
        .markers
        .iter()
        .map(|m| m.beat)
        .filter(|b| b.is_finite());
    if direction < 0 {
        beats.filter(|&b| b < beat - 1e-6).max_by(f64::total_cmp)
    } else {
        beats.filter(|&b| b > beat + 1e-6).min_by(f64::total_cmp)
    }
}

/// The song's end, in beats: its length.
pub fn song_end(project: &Project, tempo: &TempoMap) -> f64 {
    let s = project.settings.length;
    if s.is_finite() {
        tempo.beat_at((s.max(0.0) * tempo.sample_rate()) as u64)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_span_covers_the_song_and_everything_on_the_ruler_in_bars() {
        let mut p = Project::default();
        p.settings.length = 10.0; // 20 beats at 120 BPM
        let tempo = TempoMap::new(48_000.0, &p.settings.tempo);
        assert_eq!(span(&p, &tempo, 0.0), 32.0, "at least 8 bars");
        p.settings.loop_region = Some([40.0, 50.0]);
        assert_eq!(span(&p, &tempo, 0.0), 56.0);
        p.settings.markers.push(Marker {
            beat: 70.0,
            name: String::new(),
        });
        assert_eq!(span(&p, &tempo, 0.0), 76.0);
    }

    #[test]
    fn markers_step_back_and_forth() {
        let mut p = Project::default();
        for b in [8.0, 16.0, 4.0] {
            p.settings.markers.push(Marker {
                beat: b,
                name: String::new(),
            });
        }
        assert_eq!(next_marker(&p, 10.0, 1), Some(16.0));
        assert_eq!(next_marker(&p, 10.0, -1), Some(8.0));
        assert_eq!(next_marker(&p, 8.0, -1), Some(4.0));
        assert_eq!(next_marker(&p, 16.0, 1), None);
    }
}
