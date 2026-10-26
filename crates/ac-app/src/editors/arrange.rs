//! The Arrangement view (DECISIONS D221): the song in time, a lane for each
//! generator placed in it (`start` *At beat…*), drawn from the nodes'
//! own `at`, `until`, and lengths. Dragging a block moves its `at` (and
//! `until` with it), dragging its right edge sets `until`, and a
//! double-click clears `until`. It keeps nothing of its own.

use ac_core::{Category, NodeId, NodeInstance, Project, Registry, Settings, TempoMap, Value};
use egui::{Align2, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

use super::{area, load, store};
use crate::theme;

/// The kinds that can be placed in the song.
pub const PLACEABLE: [&str; 4] = ["clip", "midi_pattern", "pattern", "graph"];

/// One block on its lane.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub node: NodeId,
    pub label: String,
    pub category: Category,
    pub at: f64,
    /// `until`, when set.
    pub until: Option<f64>,
    /// Where it stops sounding, in beats.
    pub end: f64,
    /// One pass, in beats.
    pub pass: f64,
    /// It loops with nothing to stop it: it runs to the song's end.
    pub open: bool,
}

/// A rate for beats ↔ seconds; any rate gives the same beats.
const RATE: f64 = 48_000.0;
/// Shortest block, in beats.
const MIN_LEN: f64 = 1.0 / 16.0;

/// The lanes of `doc`'s project, with Clip lengths from `live` telemetry.
pub fn lanes_of(
    doc: &crate::document::Document,
    registry: &Registry,
    tele: &ac_engine::TelemetryReader,
) -> Vec<Lane> {
    use ac_nodes::generators::clip::TELEMETRY_SECONDS;
    lanes(&doc.project, registry, |id| {
        tele.node_value(&ac_core::NodePath(vec![id]), TELEMETRY_SECONDS)
            .map(f64::from)
    })
}

/// Where the engine is, in beats.
pub fn playhead(settings: &Settings, engine: &ac_engine::EngineHandle) -> f64 {
    let rate = engine
        .status()
        .sample_rate
        .load(std::sync::atomic::Ordering::Relaxed);
    let rate = if rate > 0 { f64::from(rate) } else { RATE };
    TempoMap::new(rate, &settings.tempo).beat_at(engine.position())
}

/// Store a move or trim on its node, one undo step per gesture.
pub fn apply(
    doc: &mut crate::document::Document,
    engine: &mut ac_engine::EngineHandle,
    e: &Edit,
    now: f64,
) {
    let mut values = vec![("at", Value::Float(e.at))];
    if let Some(u) = e.until {
        values.push(("until", Value::Float(u)));
    }
    crate::params::set_many(doc, engine, e.node, &values, now);
    if e.done {
        doc.end_coalescing();
    }
}

/// The song's end in beats.
pub fn song_end(settings: &Settings) -> f64 {
    let tempo = TempoMap::new(RATE, &settings.tempo);
    tempo.beat_at((settings.length.max(0.0) * RATE) as u64)
}

/// Beats from `at` that `seconds` take.
fn beats_after(tempo: &TempoMap, at: f64, seconds: f64) -> f64 {
    let start = tempo.sample_at(at);
    tempo.beat_at(start + (seconds.max(0.0) * RATE) as u64) - at
}

/// A node's number input: its stored value, else its default.
fn number(node: &NodeInstance, registry: &Registry, name: &str) -> Option<f64> {
    match node.params.get(name) {
        Some(Value::Float(v)) => Some(*v),
        Some(Value::Int(v)) => Some(*v as f64),
        Some(Value::Bool(b)) => Some(f64::from(u8::from(*b))),
        _ => {
            let sig = registry
                .get(&node.kind)
                .map(|e| (e.describe)(&node.config))?;
            let spec = sig.inputs.iter().find(|s| s.name == name)?;
            match &spec.default {
                Value::Float(v) => Some(*v),
                Value::Int(v) => Some(*v as f64),
                Value::Bool(b) => Some(f64::from(u8::from(*b))),
                _ => None,
            }
        }
    }
}

/// Every generator placed in the song, in the root graph, top to bottom as
/// on the canvas. `seconds(id)` is a Clip's sound length, when known.
pub fn lanes(
    project: &Project,
    registry: &Registry,
    seconds: impl Fn(NodeId) -> Option<f64>,
) -> Vec<Lane> {
    let settings = &project.settings;
    let tempo = TempoMap::new(RATE, &settings.tempo);
    let end = song_end(settings);
    let mut out: Vec<(Lane, [f32; 2])> = Vec::new();
    for n in &project.graph.nodes {
        if !PLACEABLE.contains(&n.kind.as_str()) {
            continue;
        }
        if n.params.get("start") != Some(&Value::Text("at".into())) {
            continue;
        }
        let at = number(n, registry, "at").unwrap_or(0.0).max(0.0);
        let rate = number(n, registry, "rate")
            .map(f64::abs)
            .filter(|r| *r > 1e-6)
            .unwrap_or(1.0);
        let pass = match n.kind.as_str() {
            "clip" => seconds(n.id)
                .filter(|s| *s > 0.0)
                .map_or(4.0, |s| beats_after(&tempo, at, s / rate)),
            "midi_pattern" => ac_nodes::sequencing::midi_pattern::clip_length(&n.config) / rate,
            "pattern" => {
                let steps = ac_nodes::sequencing::pattern::step_count(&n.config) as f64;
                steps
                    * number(n, registry, "interval")
                        .unwrap_or(0.25)
                        .max(1.0 / 64.0)
            }
            _ => {
                let d = number(n, registry, "duration").unwrap_or(1.0).max(0.0) / rate;
                if n.config.get("unit").and_then(|u| u.as_str()) == Some("beats") {
                    d
                } else {
                    beats_after(&tempo, at, d)
                }
            }
        }
        .max(MIN_LEN);
        let looping = number(n, registry, "loop").is_some_and(|v| v > 0.5);
        let until = number(n, registry, "until").filter(|u| *u > at);
        let (stop, open) = match (until, looping) {
            (Some(u), true) => (u, false),
            (Some(u), false) => (u.min(at + pass), false),
            (None, true) => (end.max(at + pass), true),
            (None, false) => (at + pass, false),
        };
        let label = n.ui.label.clone().unwrap_or_else(|| {
            registry
                .get(&n.kind)
                .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string())
        });
        let category = registry
            .get(&n.kind)
            .map_or(Category::Generators, |e| e.meta.category);
        out.push((
            Lane {
                node: n.id,
                label,
                category,
                at,
                until,
                end: stop,
                pass,
                open,
            },
            n.ui.pos,
        ));
    }
    out.sort_by(|a, b| a.1[1].total_cmp(&b.1[1]).then(a.1[0].total_cmp(&b.1[0])));
    out.into_iter().map(|(l, _)| l).collect()
}

/// What to show: the lanes, the song's settings (markers, loop, tempo), and
/// the playhead in beats.
pub struct View<'a> {
    pub lanes: &'a [Lane],
    pub settings: &'a Settings,
    pub playhead: Option<f64>,
}

/// A block moved or trimmed: its node's new `at` and `until` (`Some(0)`
/// clears `until`).
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub node: NodeId,
    pub at: f64,
    pub until: Option<f64>,
    pub done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Body,
    End,
}

#[derive(Clone, Copy)]
struct Drag {
    lane: usize,
    part: Part,
    origin: f64,
    at: f64,
    until: Option<f64>,
    end: f64,
}

/// Height of the ruler.
const RULER: f32 = 16.0;
/// Pixels at a block's right end that trim it.
const EDGE: f32 = 6.0;

/// Where things are drawn.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub lanes: Rect,
    pub ruler: Rect,
    /// Beats shown, from 0.
    pub span: f64,
    pub lane_h: f32,
}

impl Geometry {
    pub fn new(rect: Rect, lanes: usize, span: f64) -> Geometry {
        let label = (rect.width() * 0.18).clamp(0.0, 110.0);
        let ruler = Rect::from_min_max(
            pos2(rect.left() + label, rect.top() + 2.0),
            pos2(rect.right() - 4.0, rect.top() + 2.0 + RULER),
        );
        let lanes_rect = Rect::from_min_max(
            pos2(rect.left() + label, ruler.bottom() + 2.0),
            pos2(rect.right() - 4.0, rect.bottom() - 4.0),
        );
        let lane_h = (lanes_rect.height() / lanes.max(1) as f32).min(30.0);
        Geometry {
            lanes: lanes_rect,
            ruler,
            span: span.max(1.0),
            lane_h,
        }
    }

    pub fn x(&self, beat: f64) -> f32 {
        self.lanes.left() + (beat / self.span) as f32 * self.lanes.width()
    }

    pub fn beat(&self, x: f32) -> f64 {
        f64::from((x - self.lanes.left()) / self.lanes.width().max(1.0)) * self.span
    }

    pub fn block(&self, i: usize, lane: &Lane) -> Rect {
        let y = self.lanes.top() + i as f32 * self.lane_h;
        Rect::from_min_max(
            pos2(self.x(lane.at), y + 2.0),
            pos2(
                self.x(lane.end).max(self.x(lane.at) + 3.0),
                y + self.lane_h - 2.0,
            ),
        )
    }

    /// The block and part under `p`.
    pub fn hit(&self, lanes: &[Lane], p: Pos2) -> Option<(usize, Part)> {
        lanes.iter().enumerate().find_map(|(i, l)| {
            let r = self.block(i, l);
            if !r.expand2(vec2(EDGE, 0.0)).contains(p) {
                return None;
            }
            Some((
                i,
                if (p.x - r.right()).abs() <= EDGE && !l.open {
                    Part::End
                } else {
                    Part::Body
                },
            ))
        })
    }
}

/// Beats shown: the song, or further if a block reaches past it, to the
/// next whole bar.
pub fn span(lanes: &[Lane], settings: &Settings) -> f64 {
    let bar = f64::from(settings.tempo.first().map_or(4, |t| t.time_sig[0].max(1)));
    let last = lanes
        .iter()
        .filter(|l| !l.open)
        .map(|l| l.end)
        .fold(song_end(settings), f64::max);
    ((last / bar).ceil() * bar).max(bar)
}

fn snap(beat: f64, fine: bool) -> f64 {
    let step = if fine { 0.25 } else { 1.0 };
    (beat / step).round() * step
}

/// The edit for dragging `d` to `beat`.
pub fn dragged(
    lanes: &[Lane],
    d: (usize, Part, f64, f64, Option<f64>, f64),
    beat: f64,
    fine: bool,
) -> Option<Edit> {
    let (i, part, origin, at, until, end) = d;
    let lane = lanes.get(i)?;
    Some(match part {
        Part::Body => {
            let delta = snap(beat - origin, fine);
            let new_at = (at + delta).max(0.0);
            let shift = new_at - at;
            Edit {
                node: lane.node,
                at: new_at,
                until: until.map(|u| u + shift),
                done: false,
            }
        }
        Part::End => {
            let u = snap(beat, fine).max(at + MIN_LEN);
            let _ = end;
            Edit {
                node: lane.node,
                at,
                until: Some(u),
                done: false,
            }
        }
    })
}

pub fn ui(ui: &mut egui::Ui, id: egui::Id, view: &View, width: f32, height: f32) -> Option<Edit> {
    let (resp, rect) = area(ui, width, height);
    let lanes = view.lanes;
    let geo = Geometry::new(rect, lanes.len(), span(lanes, view.settings));
    let painter = ui.painter_at(rect);
    let fine = ui.input(|i| i.modifiers.shift);
    let drag_id = id.with("drag");
    let mut out = None;
    let pointer = resp.interact_pointer_pos();
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
        && let Some((lane, part)) = geo.hit(lanes, o)
    {
        let l = &lanes[lane];
        store(
            ui,
            drag_id,
            Some(Drag {
                lane,
                part,
                origin: geo.beat(o.x),
                at: l.at,
                until: l.until,
                end: l.end,
            }),
        );
    }
    if let (Some(d), Some(p)) = (load::<Drag>(ui, drag_id), pointer)
        && (resp.dragged() || resp.drag_stopped())
    {
        out = dragged(
            lanes,
            (d.lane, d.part, d.origin, d.at, d.until, d.end),
            geo.beat(p.x),
            fine,
        )
        .map(|mut e| {
            e.done = resp.drag_stopped();
            e
        });
        if resp.drag_stopped() {
            store::<Drag>(ui, drag_id, None);
        }
    }
    if resp.double_clicked()
        && let Some(p) = pointer
        && let Some((i, _)) = geo.hit(lanes, p)
        && lanes[i].until.is_some()
    {
        out = Some(Edit {
            node: lanes[i].node,
            at: lanes[i].at,
            until: Some(0.0),
            done: true,
        });
    }
    if let Some(p) = resp.hover_pos() {
        let icon = match geo.hit(lanes, p) {
            Some((_, Part::End)) => egui::CursorIcon::ResizeHorizontal,
            Some((_, Part::Body)) => egui::CursorIcon::Grab,
            None => egui::CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(icon);
    }

    // The ruler: bars, the loop, the markers.
    let s = view.settings;
    let bar = f64::from(s.tempo.first().map_or(4, |t| t.time_sig[0].max(1)));
    let small = egui::FontId::proportional(theme::FONT_SMALL);
    if let Some([a, b]) = s.loop_region {
        let r = Rect::from_min_max(
            pos2(geo.x(a), geo.ruler.top()),
            pos2(geo.x(b), geo.ruler.bottom()),
        );
        painter.rect_filled(r, 0.0, theme::fade(theme::accent(), 0.3));
    }
    let bars = (geo.span / bar).ceil() as usize;
    let every = ((bars as f32 * 40.0 / geo.lanes.width().max(1.0)).ceil() as usize).max(1);
    for k in 0..=bars {
        let x = geo.x(k as f64 * bar);
        painter.line_segment(
            [pos2(x, geo.ruler.top()), pos2(x, geo.lanes.bottom())],
            Stroke::new(1.0, super::grid(k % 4 == 0)),
        );
        if k % every == 0 && k < bars {
            painter.text(
                pos2(x + 2.0, geo.ruler.center().y),
                Align2::LEFT_CENTER,
                format!("{}", k + 1),
                small.clone(),
                theme::text_faint(),
            );
        }
    }
    for m in &s.markers {
        let x = geo.x(m.beat);
        painter.line_segment(
            [pos2(x, geo.ruler.top()), pos2(x, geo.lanes.bottom())],
            Stroke::new(1.0, theme::warning()),
        );
        painter.text(
            pos2(x + 2.0, geo.ruler.top()),
            Align2::LEFT_TOP,
            &m.name,
            small.clone(),
            theme::warning(),
        );
    }
    // The lanes.
    let label_x = rect.left() + 6.0;
    for (i, l) in lanes.iter().enumerate() {
        let r = geo.block(i, l);
        if geo.lanes.left() - rect.left() > 30.0 {
            painter.text(
                pos2(label_x, r.center().y),
                Align2::LEFT_CENTER,
                &l.label,
                small.clone(),
                theme::text_muted(),
            );
        }
        let color = theme::category_color(l.category);
        painter.rect_filled(r, theme::radius(3.0), theme::fade(color, 0.55));
        // Each pass after the first, a line.
        let mut b = l.at + l.pass;
        while b < l.end - 1e-9 && l.pass > 0.0 {
            let x = geo.x(b);
            if x > geo.lanes.right() {
                break;
            }
            painter.line_segment(
                [pos2(x, r.top() + 3.0), pos2(x, r.bottom() - 3.0)],
                Stroke::new(1.0, theme::fade(theme::text_strong(), 0.35)),
            );
            b += l.pass;
        }
        painter.rect_stroke(
            r,
            theme::radius(3.0),
            Stroke::new(1.0, color),
            StrokeKind::Inside,
        );
        let name = if l.open {
            format!("{} {}", l.label, crate::i18n::tr("arrangement-loops"))
        } else {
            l.label.clone()
        };
        painter.text(
            r.left_center() + vec2(4.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            small.clone(),
            theme::text_strong(),
        );
    }
    if lanes.is_empty() {
        painter.text(
            geo.lanes.center(),
            Align2::CENTER_CENTER,
            crate::i18n::tr("arrangement-empty"),
            small,
            theme::text_faint(),
        );
    }
    if let Some(b) = view.playhead.filter(|b| *b <= geo.span) {
        super::playhead(&painter, geo.x(b), geo.ruler.top(), geo.lanes.bottom());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NodeInstance;
    use serde_json::json;

    fn placed(id: u64, kind: &str, at: f64, y: f32) -> NodeInstance {
        let mut n = NodeInstance::new(NodeId(id), kind).at(0.0, y);
        n.params.insert("start".into(), Value::Text("at".into()));
        n.params.insert("at".into(), Value::Float(at));
        n
    }

    #[test]
    fn lanes_come_from_placed_generators() {
        let registry = ac_nodes::registry();
        let mut p = Project::default();
        // 120 BPM, a 20-second song: 40 beats.
        p.settings.length = 20.0;
        let mut clip = placed(1, "clip", 8.0, 200.0);
        clip.params.insert("loop".into(), Value::Bool(false));
        p.graph.nodes.push(clip);
        let mut riff = placed(2, "midi_pattern", 0.0, 0.0).with_config(json!({ "length": 8 }));
        riff.params.insert("until".into(), Value::Float(24.0));
        p.graph.nodes.push(riff);
        p.graph.nodes.push(placed(3, "pattern", 4.0, 100.0));
        // Not placed: no lane.
        p.graph.nodes.push(NodeInstance::new(NodeId(4), "clip"));
        let l = lanes(&p, &registry, |id| (id == NodeId(1)).then_some(3.0));
        assert_eq!(
            l.iter().map(|l| l.node.0).collect::<Vec<_>>(),
            [2, 3, 1],
            "top to bottom"
        );
        assert_eq!(
            (l[0].at, l[0].end, l[0].pass),
            (0.0, 24.0, 8.0),
            "looped to until"
        );
        assert!(l[1].open, "a looping Pattern runs to the end");
        assert_eq!(l[1].end, 40.0);
        assert_eq!(l[1].pass, 4.0, "16 sixteenths");
        assert_eq!(
            (l[2].at, l[2].end),
            (8.0, 14.0),
            "three seconds of clip, once"
        );
        assert_eq!(span(&l, &p.settings), 40.0);
    }

    #[test]
    fn drags_move_and_trim_on_the_beat() {
        let lane = Lane {
            node: NodeId(1),
            label: "x".into(),
            category: Category::Generators,
            at: 4.0,
            until: Some(12.0),
            end: 12.0,
            pass: 4.0,
            open: false,
        };
        let lanes = [lane];
        let moved = dragged(
            &lanes,
            (0, Part::Body, 5.0, 4.0, Some(12.0), 12.0),
            7.3,
            false,
        )
        .unwrap();
        assert_eq!((moved.at, moved.until), (6.0, Some(14.0)));
        let left = dragged(
            &lanes,
            (0, Part::Body, 5.0, 4.0, Some(12.0), 12.0),
            -20.0,
            false,
        )
        .unwrap();
        assert_eq!(left.at, 0.0, "not before the song");
        let trimmed = dragged(
            &lanes,
            (0, Part::End, 12.0, 4.0, Some(12.0), 12.0),
            9.6,
            false,
        )
        .unwrap();
        assert_eq!(trimmed.until, Some(10.0));
        let fine = dragged(&lanes, (0, Part::End, 12.0, 4.0, None, 12.0), 9.6, true).unwrap();
        assert_eq!(fine.until, Some(9.5));
        let squashed = dragged(&lanes, (0, Part::End, 12.0, 4.0, None, 12.0), 1.0, false).unwrap();
        assert_eq!(squashed.until, Some(4.0 + MIN_LEN));
    }

    #[test]
    fn blocks_are_hit_by_body_and_end() {
        let lanes = [Lane {
            node: NodeId(1),
            label: "x".into(),
            category: Category::Generators,
            at: 0.0,
            until: None,
            end: 10.0,
            pass: 10.0,
            open: false,
        }];
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 100.0));
        let g = Geometry::new(rect, 1, 20.0);
        let mid = pos2(g.x(5.0), g.lanes.top() + g.lane_h / 2.0);
        assert_eq!(g.hit(&lanes, mid), Some((0, Part::Body)));
        let edge = pos2(g.x(10.0) - 2.0, mid.y);
        assert_eq!(g.hit(&lanes, edge), Some((0, Part::End)));
        assert_eq!(g.hit(&lanes, pos2(g.x(15.0), mid.y)), None);
    }
}
