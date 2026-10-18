//! Live readings for the canvas: telemetry looked up through the current
//! group scope, meter ballistics with peak hold, and event pulses.

use std::collections::HashMap;

use ac_core::{Endpoint, Graph, NodeId, NodePath};
use ac_engine::TelemetryReader;

use crate::document::Document;
use crate::edit::GROUP_IN;

/// Meter needles fall at this rate (dB per second) and peaks hold this long.
const FALL_DB_PER_S: f32 = 24.0;
const HOLD_SECONDS: f64 = 1.5;

#[derive(Clone, Copy, Debug, Default)]
pub struct MeterChannel {
    /// Displayed peak and RMS (linear), with ballistics.
    pub peak: f32,
    pub rms: f32,
    /// Held peak and when it was set.
    pub hold: f32,
    pub hold_at: f64,
}

/// Editor-side state that smooths and remembers live values between frames.
#[derive(Default)]
pub struct LiveState {
    meters: HashMap<NodePath, [MeterChannel; 2]>,
    /// Last event count seen on an output and when it last changed.
    events: HashMap<(NodePath, String), (u32, f64)>,
    last_time: f64,
    /// Something moved this frame (keep repainting).
    pub active: bool,
}

impl LiveState {
    pub fn begin_frame(&mut self, now: f64) -> f32 {
        let dt = (now - self.last_time).clamp(0.0, 0.25) as f32;
        self.last_time = now;
        self.active = false;
        dt
    }

    /// Feed one block's meter reading; returns the displayed channels.
    pub fn meter(
        &mut self,
        path: &NodePath,
        reading: [(f32, f32); 2],
        now: f64,
        dt: f32,
    ) -> [MeterChannel; 2] {
        let m = self.meters.entry(path.clone()).or_default();
        let fall = 10f32.powf(-FALL_DB_PER_S * dt / 20.0);
        for (c, (peak, rms)) in m.iter_mut().zip(reading) {
            let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
            let rms = if rms.is_finite() { rms.max(0.0) } else { 0.0 };
            c.peak = peak.max(c.peak * fall);
            c.rms = rms.max(c.rms * fall);
            if c.peak < 1e-5 {
                c.peak = 0.0;
            }
            if c.rms < 1e-5 {
                c.rms = 0.0;
            }
            if peak >= c.hold || now - c.hold_at > HOLD_SECONDS {
                c.hold = peak.max(c.peak);
                c.hold_at = now;
            }
            if c.peak > 0.0 || c.hold > 1e-5 {
                self.active = true;
            }
        }
        *m
    }

    /// The displayed channels of the meter at `path`, if it has been fed.
    pub fn meter_reading(&self, path: &NodePath) -> Option<[MeterChannel; 2]> {
        self.meters.get(path).copied()
    }

    /// Seconds since the event count on an output last changed.
    pub fn pulse_age(&mut self, key: (NodePath, String), count: u32, now: f64) -> Option<f32> {
        let e = self.events.entry(key).or_insert((count, f64::NEG_INFINITY));
        if e.0 != count {
            *e = (count, now);
        }
        let age = (now - e.1) as f32;
        if age < crate::theme::ANIM_SECONDS {
            self.active = true;
        }
        age.is_finite().then_some(age)
    }
}

/// Telemetry seen from the current scope.
pub struct Live<'a> {
    pub tele: &'a TelemetryReader,
    prefix: Vec<NodeId>,
}

impl<'a> Live<'a> {
    pub fn new(tele: &'a TelemetryReader, doc: &Document) -> Live<'a> {
        Live {
            tele,
            prefix: doc.scope_prefix(),
        }
    }

    pub fn path(&self, id: NodeId) -> NodePath {
        let mut v = self.prefix.clone();
        v.push(id);
        NodePath(v)
    }

    pub fn node_value(&self, id: NodeId, k: usize) -> Option<f32> {
        self.tele.node_value(&self.path(id), k)
    }

    pub fn scope(&self, id: NodeId) -> Option<Vec<f32>> {
        self.tele.scope(&self.path(id))
    }

    pub fn node_time_ns(&self, id: NodeId) -> Option<u32> {
        self.tele.node_time_ns(&self.path(id))
    }
}

/// The engine path and socket that produce the signal on `ep` (an output
/// in the current scope), following reroutes and, for the group interface,
/// the wire into the instance one level up. `None` when nothing upstream
/// produces it (an unwired group input).
pub fn source_path(doc: &Document, ep: &Endpoint) -> Option<(NodePath, String)> {
    let mut depth = doc.scope.len();
    let mut ep = ep.clone();
    for _ in 0..256 {
        if ep.node == GROUP_IN {
            if depth == 0 {
                return None;
            }
            let instance = doc.scope[depth - 1].instance;
            depth -= 1;
            ep = feeding(doc, depth, &Endpoint::new(instance, ep.socket.clone()))?;
            continue;
        }
        let is_reroute = graph_at(doc, depth)?
            .node(ep.node)
            .is_some_and(|n| n.kind == ac_core::REROUTE_KIND);
        if !is_reroute {
            break;
        }
        ep = feeding(doc, depth, &Endpoint::new(ep.node, "in"))?;
    }
    let mut path: Vec<NodeId> = doc.scope[..depth].iter().map(|s| s.instance).collect();
    path.push(ep.node);
    Some((NodePath(path), ep.socket))
}

/// What feeds an input at scope depth `depth`: a wire, or (inside a
/// definition) the interface input that targets it.
fn feeding(doc: &Document, depth: usize, to: &Endpoint) -> Option<Endpoint> {
    let g = graph_at(doc, depth)?;
    if let Some(w) = g.wire_into(to) {
        return Some(w.from.clone());
    }
    let def = doc
        .project
        .group_defs
        .get(&doc.scope.get(depth.checked_sub(1)?)?.def)?;
    def.inputs
        .iter()
        .find(|i| i.targets.contains(to))
        .map(|i| Endpoint::new(GROUP_IN, i.name.clone()))
}

fn graph_at(doc: &Document, depth: usize) -> Option<&Graph> {
    if depth == 0 {
        return Some(&doc.project.graph);
    }
    let e = doc.scope.get(depth - 1)?;
    doc.project.group_defs.get(&e.def).map(|d| &d.graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_fall_and_hold() {
        let mut s = LiveState::default();
        let p = NodePath::root(NodeId(1));
        let m = s.meter(&p, [(0.8, 0.5), (0.4, 0.2)], 0.0, 0.016);
        assert_eq!(m[0].peak, 0.8);
        let m = s.meter(&p, [(0.0, 0.0), (0.0, 0.0)], 0.5, 0.5);
        assert!(m[0].peak < 0.8 && m[0].peak > 0.0);
        assert_eq!(m[0].hold, 0.8);
        let m = s.meter(&p, [(0.0, 0.0), (0.0, 0.0)], 3.0, 0.25);
        assert!(m[0].hold < 0.8);
        // NaN readings count as silence.
        let m = s.meter(&p, [(f32::NAN, f32::NAN), (0.0, 0.0)], 3.1, 0.016);
        assert!(m[0].peak.is_finite());
    }

    #[test]
    fn pulses_start_when_the_count_changes() {
        let mut s = LiveState::default();
        let k = (NodePath::root(NodeId(1)), "trigger".to_string());
        assert_eq!(s.pulse_age(k.clone(), 0, 1.0), None);
        let age = s.pulse_age(k.clone(), 1, 2.0).unwrap();
        assert_eq!(age, 0.0);
        let age = s.pulse_age(k, 1, 2.1).unwrap();
        assert!((age - 0.1).abs() < 1e-6);
    }
}
