//! Editors for node settings that are more than a number: breakpoint
//! curves (`Graph`, `Envelope`, `Shaper`), step grids (`Pattern`), piano
//! rolls (`MidiPattern`), crossover bands (`BandSplit`), step values
//! (`StepSequencer`), and key and velocity zones (`Zones`). The waveform
//! editor for Clip, Sampler, and Slicer (`sound`) edits inputs and markers
//! instead, so the Node tab drives it directly.
//!
//! Each editor reads one config key and, when the person changes it, returns
//! the new value; the caller stores it as an undo step (coalesced while a
//! drag lasts). The editing rules live in plain functions next to each
//! editor so they can be tested without a window.

pub mod arrange;
pub mod bands;
pub mod banks;
pub mod curve;
pub mod pattern;
pub mod piano_roll;
pub mod sound;
pub mod steps;
pub mod zones;

use ac_core::Config;
use egui::{Color32, Rect, Sense, Stroke, StrokeKind};

use crate::i18n::tr;
use crate::theme;

/// Which editor a node kind gets, and the config key it edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Editor {
    Curve(curve::Space),
    Pattern,
    PianoRoll,
    Bands,
    Steps,
    Zones,
}

impl Editor {
    /// The config key this editor writes.
    pub fn key(self) -> &'static str {
        match self {
            Editor::Curve(_) => "points",
            Editor::Pattern => "cells",
            Editor::PianoRoll => "notes",
            Editor::Bands => "edges",
            Editor::Steps => "values",
            Editor::Zones => "zones",
        }
    }

    /// How to use it, in a line.
    pub fn hint(self) -> String {
        match self {
            Editor::Curve(_) => tr("editor-curve-hint"),
            Editor::Pattern => tr("editor-pattern-hint"),
            Editor::PianoRoll => tr("editor-piano-roll-hint"),
            Editor::Bands => tr("editor-bands-hint"),
            Editor::Steps => tr("editor-steps-hint"),
            Editor::Zones => tr("editor-zones-hint"),
        }
    }

    /// A good height at `width`.
    pub fn height(self, width: f32) -> f32 {
        match self {
            Editor::Curve(_) | Editor::Steps => (width * 0.55).clamp(120.0, 360.0),
            Editor::Pattern => (width * 0.4).clamp(80.0, 320.0),
            Editor::PianoRoll => (width * 0.7).clamp(160.0, 480.0),
            Editor::Bands => (width * 0.4).clamp(90.0, 240.0),
            Editor::Zones => (width * 0.45).clamp(100.0, 280.0),
        }
    }
}

/// The editor for a node of `kind` with `config`, if it has one.
pub fn editor_for(kind: &str, config: &Config) -> Option<Editor> {
    Some(match kind {
        "graph" => Editor::Curve(curve::Space::graph()),
        "envelope" => Editor::Curve(curve::Space::envelope(config)),
        // Only the custom transfer curve has points to edit.
        "shaper" => Editor::Curve(curve::Space::shaper()),
        "pattern" => Editor::Pattern,
        "midi_pattern" => Editor::PianoRoll,
        "band_split" => Editor::Bands,
        "step_sequencer" => Editor::Steps,
        "zones" => Editor::Zones,
        _ => return None,
    })
}

/// What an editor shows of its node playing, from the node's telemetry:
/// a playhead on the grid, roll, steps, or curve.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LiveView {
    /// The step playing (Pattern, Step Sequencer).
    pub step: Option<usize>,
    /// Where it is: beats into a MIDI Pattern, 0 to 1 along a Graph.
    pub position: Option<f64>,
    /// The bank playing (Pattern, MIDI Pattern).
    pub bank: Option<usize>,
}

/// The [`LiveView`] of a node of `kind` whose telemetry slot `k` reads
/// `value(k)` (nothing while the engine hasn't sent any).
pub fn live_view(kind: &str, value: impl Fn(usize) -> Option<f32>) -> LiveView {
    use ac_nodes::generators::graph as g;
    use ac_nodes::modulation::step_sequencer as ss;
    use ac_nodes::sequencing::{midi_pattern as mp, pattern as pt};
    let index = |k: usize| value(k).filter(|v| *v >= 0.0).map(|v| v.round() as usize);
    let playing = |k: usize| value(k).is_some_and(|v| v > 0.5);
    match kind {
        "pattern" => LiveView {
            step: index(pt::TELEMETRY_STEP),
            bank: index(pt::TELEMETRY_BANK),
            position: None,
        },
        "midi_pattern" => LiveView {
            position: playing(mp::TELEMETRY_PLAYING)
                .then(|| value(mp::TELEMETRY_POSITION).map(f64::from))
                .flatten(),
            bank: index(mp::TELEMETRY_BANK),
            step: None,
        },
        "step_sequencer" => LiveView {
            step: index(ss::TELEMETRY_STEP),
            ..LiveView::default()
        },
        "graph" => LiveView {
            position: playing(g::TELEMETRY_PLAYING)
                .then(|| value(g::TELEMETRY_POSITION).map(f64::from))
                .flatten(),
            ..LiveView::default()
        },
        _ => LiveView::default(),
    }
}

/// A change an editor made.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub value: serde_json::Value,
    /// The gesture ended: the next change is a new undo step.
    pub done: bool,
}

/// Show `editor` for `config` in a `width` × `height` area, with the
/// node's playhead from `live`. `salt` keeps the editor's drag state apart
/// from other instances.
pub fn show(
    ui: &mut egui::Ui,
    editor: Editor,
    config: &Config,
    live: &LiveView,
    salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let id = egui::Id::unique(("node-editor", salt));
    match editor {
        Editor::Curve(space) => curve::ui(ui, id, space, config, live, width, height),
        Editor::Pattern => pattern::ui(ui, id, config, live, width, height),
        Editor::PianoRoll => piano_roll::ui(ui, id, config, live, width, height),
        Editor::Bands => bands::ui(ui, id, config, width, height),
        Editor::Steps => steps::ui(ui, id, config, live, width, height),
        Editor::Zones => zones::ui(ui, id, config, width, height),
    }
}

/// The playhead's line, drawn over an editor.
fn playhead(painter: &egui::Painter, x: f32, top: f32, bottom: f32) {
    painter.line_segment(
        [egui::pos2(x, top), egui::pos2(x, bottom)],
        Stroke::new(1.5, theme::play()),
    );
}

/// Allocate the editor's area and paint its well.
fn area(ui: &mut egui::Ui, width: f32, height: f32) -> (egui::Response, Rect) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, theme::radius(4.0), theme::well_bg());
    p.rect_stroke(
        rect,
        theme::radius(4.0),
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
    (resp, rect)
}

/// Thin grid line color; `major` lines are brighter.
fn grid(major: bool) -> Color32 {
    if major {
        theme::grid_dot_major()
    } else {
        theme::scope_grid()
    }
}

/// What a drag in progress holds on to, kept in egui memory between frames.
fn load<T: Clone + Send + Sync + 'static>(ui: &egui::Ui, id: egui::Id) -> Option<T> {
    ui.data(|d| d.get_temp::<T>(id))
}

fn store<T: Clone + Send + Sync + 'static>(ui: &egui::Ui, id: egui::Id, v: Option<T>) {
    ui.data_mut(|d| match v {
        Some(v) => {
            d.insert_temp(id, v);
        }
        None => d.remove::<T>(id),
    });
}

/// `v` rounded to a multiple of `step` (unchanged for a zero step).
fn snap(v: f64, step: f64) -> f64 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_editable_kind_gets_its_editor() {
        let c = json!({});
        assert_eq!(editor_for("graph", &c).map(Editor::key), Some("points"));
        assert_eq!(editor_for("envelope", &c).map(Editor::key), Some("points"));
        assert_eq!(editor_for("shaper", &c).map(Editor::key), Some("points"));
        assert_eq!(editor_for("pattern", &c), Some(Editor::Pattern));
        assert_eq!(editor_for("midi_pattern", &c), Some(Editor::PianoRoll));
        assert_eq!(editor_for("band_split", &c), Some(Editor::Bands));
        assert_eq!(editor_for("step_sequencer", &c), Some(Editor::Steps));
        assert_eq!(editor_for("zones", &c), Some(Editor::Zones));
        assert_eq!(editor_for("oscillator", &c), None);
    }

    #[test]
    fn live_views_read_each_kinds_telemetry() {
        let t = |v: &'static [f32]| move |k: usize| v.get(k).copied();
        let p = live_view("pattern", t(&[3.0, 1.0, 1.0]));
        assert_eq!((p.step, p.bank), (Some(3), Some(1)));
        assert_eq!(live_view("pattern", t(&[-1.0, 0.0, 0.0])).step, None);
        let m = live_view("midi_pattern", t(&[2.5, 0.0, 1.0]));
        assert_eq!(m.position, Some(2.5));
        assert_eq!(
            live_view("midi_pattern", t(&[2.5, 0.0, 0.0])).position,
            None,
            "stopped"
        );
        assert_eq!(live_view("step_sequencer", t(&[5.0])).step, Some(5));
        assert_eq!(live_view("graph", t(&[0.25, 1.0])).position, Some(0.25));
        assert_eq!(live_view("oscillator", t(&[1.0])), LiveView::default());
        assert_eq!(
            live_view("pattern", |_| None),
            LiveView::default(),
            "no engine"
        );
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(0.37, 0.25), 0.25);
        assert_eq!(snap(0.38, 0.25), 0.5);
        assert_eq!(snap(0.37, 0.0), 0.37);
    }
}
