//! Editors for node settings that are more than a number: breakpoint
//! curves (`Graph`, `Envelope`, `Shaper`), step grids (`Pattern`), piano
//! rolls (`MidiPattern`), crossover bands (`BandSplit`), and step values
//! (`StepSequencer`).
//!
//! Each editor reads one config key and, when the person changes it, returns
//! the new value; the caller stores it as an undo step (coalesced while a
//! drag lasts). The editing rules live in plain functions next to each
//! editor so they can be tested without a window.

pub mod bands;
pub mod curve;
pub mod pattern;
pub mod piano_roll;
pub mod steps;

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
        }
    }

    /// A good height at `width`.
    pub fn height(self, width: f32) -> f32 {
        match self {
            Editor::Curve(_) | Editor::Steps => (width * 0.55).clamp(120.0, 360.0),
            Editor::Pattern => (width * 0.4).clamp(80.0, 320.0),
            Editor::PianoRoll => (width * 0.7).clamp(160.0, 480.0),
            Editor::Bands => (width * 0.4).clamp(90.0, 240.0),
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
        _ => return None,
    })
}

/// A change an editor made.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub value: serde_json::Value,
    /// The gesture ended: the next change is a new undo step.
    pub done: bool,
}

/// Show `editor` for `config` in a `width` × `height` area. `salt` keeps
/// the editor's drag state apart from other instances.
pub fn show(
    ui: &mut egui::Ui,
    editor: Editor,
    config: &Config,
    salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let id = egui::Id::unique(("node-editor", salt));
    match editor {
        Editor::Curve(space) => curve::ui(ui, id, space, config, width, height),
        Editor::Pattern => pattern::ui(ui, id, config, width, height),
        Editor::PianoRoll => piano_roll::ui(ui, id, config, width, height),
        Editor::Bands => bands::ui(ui, id, config, width, height),
        Editor::Steps => steps::ui(ui, id, config, width, height),
    }
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
        assert_eq!(editor_for("oscillator", &c), None);
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(0.37, 0.25), 0.25);
        assert_eq!(snap(0.38, 0.25), 0.5);
        assert_eq!(snap(0.37, 0.0), 0.37);
    }
}
