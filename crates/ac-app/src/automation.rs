//! Recording automation (DECISIONS D219): while the transport records and
//! the transport's automation mode is Write or Touch, every move of an
//! unwired number input (a knob's value, a fader, a node's own knob) is
//! kept with its song beat. When recording stops, each input's moves
//! become a Graph placed in the song and wired into that input, one undo
//! step for the pass.

use ac_core::{AutomationMode, NodeId, TempoMap, Value};
use ac_engine::EngineHandle;

use crate::document::Document;
use crate::edit;
use crate::i18n::tr;

/// Points closer than this, in beats, are one point.
const STEP: f64 = 1.0 / 64.0;

/// One input's moves this pass.
#[derive(Clone, Debug)]
struct Lane {
    scope: Vec<NodeId>,
    node: NodeId,
    socket: String,
    /// The value before the first move.
    before: f64,
    points: Vec<(f64, f64)>,
    /// Held now (Touch returns to `before` when let go).
    touching: bool,
}

/// The moves being recorded, kept by the document while a pass records.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    lanes: Vec<Lane>,
    /// The transport was recording at the last poll.
    was_recording: bool,
    /// A gesture ended since the last poll.
    released: bool,
}

impl Capture {
    /// A gesture ended (the document's coalescing ended).
    pub fn release(&mut self) {
        if self.lanes.iter().any(|l| l.touching) {
            self.released = true;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

fn recording(engine: &EngineHandle) -> bool {
    engine.is_playing()
        && engine
            .status()
            .recording
            .load(std::sync::atomic::Ordering::Relaxed)
}

/// The song beat the engine is at.
fn beat_now(doc: &Document, engine: &EngineHandle) -> f64 {
    let rate = engine
        .status()
        .sample_rate
        .load(std::sync::atomic::Ordering::Relaxed);
    let rate = if rate > 0 { f64::from(rate) } else { 48_000.0 };
    TempoMap::new(rate, &doc.project.settings.tempo).beat_at(engine.position())
}

/// Keep a move of `socket` on `id` to `value`, when automation records.
/// Call before the document takes the value.
pub fn capture(doc: &mut Document, engine: &EngineHandle, id: NodeId, socket: &str, value: &Value) {
    if doc.project.settings.automation.is_off() || !recording(engine) {
        return;
    }
    let Value::Float(v) = *value else {
        return;
    };
    let beat = beat_now(doc, engine);
    let scope = doc.scope_prefix();
    let before = doc
        .graph()
        .node(id)
        .and_then(|n| n.params.get(socket))
        .and_then(Value::as_f64);
    let lanes = &mut doc.automation.lanes;
    let lane = match lanes
        .iter_mut()
        .position(|l| l.scope == scope && l.node == id && l.socket == socket)
    {
        Some(i) => &mut lanes[i],
        None => {
            lanes.push(Lane {
                scope,
                node: id,
                socket: socket.to_string(),
                before: before.unwrap_or(v),
                points: Vec::new(),
                touching: false,
            });
            let l = lanes.last_mut().expect("just pushed");
            // Before the first move, the value it had.
            l.points.push((beat, l.before));
            l
        }
    };
    if !lane.touching
        && let Some(&(_, last)) = lane.points.last()
    {
        // A new touch starts from where the input is.
        lane.points.push((beat, last));
    }
    lane.touching = true;
    lane.points.push((beat, v));
}

/// Once a frame: let go of released lanes, and write every lane when
/// recording stops. Returns how many inputs were written.
pub fn poll(doc: &mut Document, engine: &EngineHandle, registry: &ac_core::Registry) -> usize {
    let now = recording(engine);
    let beat = beat_now(doc, engine);
    let touch = doc.project.settings.automation == AutomationMode::Touch;
    if std::mem::take(&mut doc.automation.released) {
        for l in doc.automation.lanes.iter_mut().filter(|l| l.touching) {
            l.touching = false;
            if touch && let Some(&(_, last)) = l.points.last() {
                // Let go: back to where it was.
                l.points.push((beat, last));
                l.points.push((beat + STEP, l.before));
            }
        }
    }
    let was = std::mem::replace(&mut doc.automation.was_recording, now);
    if now || !was || doc.automation.lanes.is_empty() {
        return 0;
    }
    let scope = doc.scope_prefix();
    let lanes: Vec<Lane> = std::mem::take(&mut doc.automation.lanes)
        .into_iter()
        .filter(|l| l.scope == scope && l.points.len() > 1)
        .collect();
    if lanes.is_empty() {
        return 0;
    }
    doc.edit(registry, &tr("undo-write-automation"), |g, ctx| {
        let mut written = 0;
        for l in &lanes {
            let Some(pos) = g.node(l.node).map(|n| n.ui.pos) else {
                continue;
            };
            let at = [pos[0] - 280.0, pos[1] + 40.0 * written as f32];
            let target = ac_core::Endpoint::new(l.node, l.socket.clone());
            if g.wire_into(&target).is_some() {
                continue;
            }
            let points = edit::automation::thin(&l.points, STEP);
            if edit::automation::automation_graph(g, ctx, target, &points, at).is_some() {
                written += 1;
            }
        }
        written
    })
}
