//! The waveform editor for Clip, Sampler, and Slicer (DECISIONS D209): the
//! sound with its playhead and, as handles, a Sampler's start, end, and loop
//! points; a Slicer's slices, with its markers to click in, drag, and remove.
//! It draws with the same picture as the Waveform card.

use std::sync::Arc;

use ac_core::AudioData;
use egui::{Pos2, Rect};

use super::{area, load, store};
use crate::canvas::bodies::{self, WaveView};

/// What the person changed.
#[derive(Clone, Debug, PartialEq)]
pub enum SoundEdit {
    /// Handle `k` (start, end, loop start, loop end) moved to `value`.
    Handle { k: usize, value: f32, done: bool },
    /// The markers, after a click, a drag, or a removal.
    Markers { markers: Vec<f32>, done: bool },
}

/// What the editor shows and what can be changed.
pub struct Sound<'a> {
    pub data: Option<&'a Arc<AudioData>>,
    pub playhead: Option<f32>,
    /// Handles, and which of them can be dragged (unwired).
    pub handles: Option<([f32; 4], [bool; 4])>,
    pub slices: Vec<f32>,
    /// Markers, when the slices are the markers.
    pub markers: Option<Vec<f32>>,
}

/// Columns of the overview: about one per two points.
fn columns(width: f32) -> usize {
    ((width / 2.0) as usize).clamp(32, 1_024)
}

/// The overview of `data` in `n` columns, computed once per sound and
/// width and kept in egui's memory.
fn overview(ui: &egui::Ui, data: &Arc<AudioData>, n: usize) -> Vec<(f32, f32)> {
    let key = egui::Id::unique(("sound-overview", Arc::as_ptr(data) as usize, n));
    if let Some(v) = load::<Arc<Vec<(f32, f32)>>>(ui, key) {
        return v.as_ref().clone();
    }
    let mut cols = vec![(0.0, 0.0); n];
    ac_nodes::display::waveform::overview(data, &mut cols);
    store(ui, key, Some(Arc::new(cols.clone())));
    cols
}

/// The marker nearest `x` within reach, by index.
pub fn marker_at(rect: Rect, markers: &[f32], x: f32, reach: f32) -> Option<usize> {
    markers
        .iter()
        .enumerate()
        .map(|(k, m)| (k, (rect.min.x + m * rect.width() - x).abs()))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Markers kept inside the sound, in order, without repeats.
pub fn tidy(markers: &mut Vec<f32>) {
    markers.retain(|m| m.is_finite() && *m > 0.0 && *m < 1.0);
    markers.sort_by(f32::total_cmp);
    markers.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
}

#[derive(Clone, Copy)]
enum Drag {
    Handle(usize),
    Marker(usize),
}

pub fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    sound: &Sound,
    width: f32,
    height: f32,
) -> Option<SoundEdit> {
    let (resp, rect) = area(ui, width, height);
    let view = WaveView {
        columns: sound
            .data
            .map(|d| overview(ui, d, columns(rect.width())))
            .unwrap_or_default(),
        playhead: sound.playhead,
        handles: sound.handles.map(|h| h.0),
        slices: sound.slices.clone(),
    };
    bodies::waveform(&ui.painter_at(rect), rect, 1.0, &view, 1.0);
    let frac = |p: Pos2| ((p.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let mut out = None;
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
    {
        let handle = sound.handles.and_then(|(h, free)| {
            let h = std::array::from_fn(|k| if free[k] { h[k] } else { f32::NAN });
            bodies::waveform_handle_at(rect, h, o.x, 8.0)
        });
        let marker = sound
            .markers
            .as_ref()
            .and_then(|m| marker_at(rect, m, o.x, 6.0));
        let drag = handle.map(Drag::Handle).or(marker.map(Drag::Marker));
        store(ui, id, drag);
    }
    if let (Some(d), Some(p)) = (load::<Drag>(ui, id), pointer)
        && (resp.dragged() || resp.drag_stopped())
    {
        let done = resp.drag_stopped();
        let f = frac(p);
        out = match d {
            Drag::Handle(k) => Some(SoundEdit::Handle { k, value: f, done }),
            Drag::Marker(k) => sound.markers.as_ref().map(|m| {
                let mut m = m.clone();
                if let Some(x) = m.get_mut(k) {
                    *x = f;
                }
                if done {
                    tidy(&mut m);
                }
                SoundEdit::Markers { markers: m, done }
            }),
        };
        if done {
            store::<Drag>(ui, id, None);
        }
    }
    if let Some(m) = sound.markers.as_ref()
        && let Some(p) = pointer
    {
        if resp.clicked() && marker_at(rect, m, p.x, 6.0).is_none() {
            let mut m = m.clone();
            m.push(frac(p));
            tidy(&mut m);
            out = Some(SoundEdit::Markers {
                markers: m,
                done: true,
            });
        }
        if resp.secondary_clicked()
            && let Some(k) = marker_at(rect, m, p.x, 6.0)
        {
            let mut m = m.clone();
            m.remove(k);
            out = Some(SoundEdit::Markers {
                markers: m,
                done: true,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn markers_are_found_and_tidied() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 50.0));
        assert_eq!(marker_at(r, &[0.25, 0.5], 52.0, 6.0), Some(0));
        assert_eq!(marker_at(r, &[0.25, 0.5], 75.0, 6.0), None);
        let mut m = vec![0.5, 0.25, 0.5, 1.2, 0.0, f32::NAN];
        tidy(&mut m);
        assert_eq!(m, vec![0.25, 0.5]);
    }
}
