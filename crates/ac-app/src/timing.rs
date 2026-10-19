//! Per-node processing time in the editor (UX.md "Live feedback", `Ctrl+T`).
//!
//! The engine measures each node's time per host block only while timing is
//! on (two clock reads a step) and publishes it through relaxed atomics. The
//! editor reads every node's time once a frame, smooths it per engine path,
//! and sums the paths under each node of the scope on screen, so a group
//! shows the cost of its insides and a Voices node the cost of every voice.
//! The heat color follows the node's share of the block's real-time budget
//! on a log scale. Off, none of this runs and the engine reads no clock.

use std::collections::HashMap;

use ac_core::{NodeId, NodePath, VOICES_KIND};
use ac_engine::TelemetryReader;

use crate::document::Document;
use crate::i18n::{tr, tr_args};

/// Smoothing time constant (seconds): steady enough to read, quick enough to
/// follow a change in the patch.
pub const SMOOTH_SECONDS: f64 = 0.4;

/// Shares of the block's budget at the cool and the hot end of the heat
/// scale (0.2 % and 25 %).
pub const COOL_SHARE: f32 = 0.002;
pub const HOT_SHARE: f32 = 0.25;

/// What the editor shows for one node: smoothed time per host block and its
/// share of the block's budget (when the engine has run a block).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub ns: f32,
    pub share: Option<f32>,
    /// Engine nodes summed into it (more than one for groups and Voices).
    pub parts: usize,
}

impl Reading {
    /// Where the reading sits on the heat scale, 0 (cool) to 1 (hot).
    pub fn heat(&self) -> f32 {
        self.share.map_or(0.0, heat)
    }
}

/// Smoothed per-node timing, read from the engine at frame rate.
#[derive(Default)]
pub struct Timings {
    /// Smoothed time per engine path (ns), with the frame it was last seen.
    paths: HashMap<NodePath, (f32, u64)>,
    /// Sums for the nodes of the scope last updated, with their part counts.
    by_node: HashMap<NodeId, (f32, usize)>,
    budget_ns: Option<f64>,
    last: Option<f64>,
    frame: u64,
}

impl Timings {
    /// Read the engine's times (call once a frame while timing is shown).
    pub fn update(&mut self, tele: &TelemetryReader, doc: &Document, now: f64) {
        let dt = self.last.map_or(f64::INFINITY, |t| (now - t).max(0.0));
        self.last = Some(now);
        self.frame += 1;
        self.budget_ns = tele.status().block_budget_ns();
        let k = smoothing(dt);
        let frame = self.frame;
        for (path, ns) in tele.node_times() {
            let x = ns as f32;
            match self.paths.get_mut(path) {
                Some((v, seen)) => {
                    *v += (x - *v) * k;
                    *seen = frame;
                }
                None => {
                    self.paths.insert(path.clone(), (x, frame));
                }
            }
        }
        // Nodes that left the graph (a recompile) stop counting.
        self.paths.retain(|_, (_, seen)| *seen == frame);
        let pattern = scope_pattern(doc);
        self.by_node.clear();
        for (path, (ns, _)) in &self.paths {
            if let Some(id) = node_in_scope(&pattern, path) {
                let e = self.by_node.entry(id).or_insert((0.0, 0));
                e.0 += *ns;
                e.1 += 1;
            }
        }
    }

    /// Forget everything (timing was turned off).
    pub fn clear(&mut self) {
        *self = Timings::default();
    }

    /// The reading for a node of the scope on screen.
    pub fn node(&self, id: NodeId) -> Option<Reading> {
        let &(ns, parts) = self.by_node.get(&id)?;
        Some(Reading {
            ns,
            share: self.share(ns),
            parts,
        })
    }

    /// `ns` as a share of the last block's budget.
    pub fn share(&self, ns: f32) -> Option<f32> {
        self.budget_ns
            .filter(|b| *b > 0.0)
            .map(|b| (f64::from(ns) / b) as f32)
    }

    /// The last host block's budget, in nanoseconds.
    pub fn budget_ns(&self) -> Option<f64> {
        self.budget_ns
    }
}

/// How much of the way a smoothed value moves toward a new sample after
/// `dt` seconds (exponential smoothing, frame-rate independent). The first
/// sample (`dt` infinite) is taken as it is.
pub fn smoothing(dt: f64) -> f32 {
    if !dt.is_finite() {
        return 1.0;
    }
    (1.0 - (-dt / SMOOTH_SECONDS).exp()) as f32
}

/// Heat 0–1 for a share of the block's budget: logarithmic, so 0.2 %,
/// 2.2 %, and 25 % are evenly spaced, and a node at a quarter of the budget
/// or more is as hot as it gets.
pub fn heat(share: f32) -> f32 {
    if !share.is_finite() || share <= COOL_SHARE {
        return 0.0;
    }
    let lo = COOL_SHARE.ln();
    let hi = HOT_SHARE.ln();
    ((share.ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// A time per block for a header: `8.4 µs`, `37 µs`, `1.24 ms`.
pub fn format_ns(ns: f32) -> String {
    let us = f64::from(ns.max(0.0)) / 1e3;
    if us < 10.0 {
        format!("{us:.1} µs")
    } else if us < 1000.0 {
        format!("{us:.0} µs")
    } else {
        format!("{:.2} ms", us / 1e3)
    }
}

/// A share of the budget: `0.7 %`, `12 %`.
pub fn format_share(share: f32) -> String {
    let pct = f64::from(share.max(0.0)) * 100.0;
    if pct < 10.0 {
        format!("{pct:.1} %")
    } else {
        format!("{pct:.0} %")
    }
}

/// The hover text for a reading: time, share, and what was summed.
pub fn describe(r: &Reading, budget_ns: Option<f64>, voices: bool) -> String {
    let time = format_ns(r.ns);
    let mut s = match (r.share, budget_ns) {
        (Some(share), Some(b)) => tr_args(
            "timing-per-block-share",
            &[
                ("time", time.into()),
                ("share", format_share(share).into()),
                ("budget", format_ns(b as f32).into()),
            ],
        ),
        _ => tr_args("timing-per-block", &[("time", time.into())]),
    };
    if r.parts > 1 {
        s.push('\n');
        s.push_str(&if voices {
            tr("timing-sum-of-voices")
        } else {
            tr("timing-sum-of-nodes")
        });
    }
    s
}

/// The engine path prefix of the current scope: each group instance the
/// editor entered through, and after a Voices instance any voice (the
/// engine runs one copy of the body per voice, under its index).
fn scope_pattern(doc: &Document) -> Vec<Option<NodeId>> {
    let mut out = Vec::new();
    let mut graph = &doc.project.graph;
    for e in &doc.scope {
        out.push(Some(e.instance));
        if graph
            .node(e.instance)
            .is_some_and(|n| n.kind == VOICES_KIND)
        {
            out.push(None);
        }
        match doc.project.group_defs.get(&e.def) {
            Some(d) => graph = &d.graph,
            None => break,
        }
    }
    out
}

/// The node of the scope that contains the engine node at `path`, if it is
/// inside the scope at all.
fn node_in_scope(pattern: &[Option<NodeId>], path: &NodePath) -> Option<NodeId> {
    let ids = &path.0;
    if ids.len() <= pattern.len() {
        return None;
    }
    let inside = pattern
        .iter()
        .zip(ids)
        .all(|(p, id)| p.is_none_or(|p| p == *id));
    inside.then(|| ids[pattern.len()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Endpoint;

    #[test]
    fn smoothing_is_frame_rate_independent() {
        assert_eq!(smoothing(f64::INFINITY), 1.0);
        assert_eq!(smoothing(0.0), 0.0);
        // Two half frames move as far as one whole frame.
        let one = smoothing(1.0 / 30.0);
        let half = smoothing(1.0 / 60.0);
        let two_halves = 1.0 - (1.0 - half) * (1.0 - half);
        assert!((one - two_halves).abs() < 1e-6);
        // After one time constant, about 63 % of a step.
        assert!((smoothing(SMOOTH_SECONDS) - 0.632).abs() < 1e-3);
    }

    #[test]
    fn heat_is_logarithmic_in_the_share_of_the_budget() {
        assert_eq!(heat(0.0), 0.0);
        assert_eq!(heat(COOL_SHARE), 0.0);
        assert_eq!(heat(f32::NAN), 0.0);
        assert_eq!(heat(HOT_SHARE), 1.0);
        assert_eq!(heat(3.0), 1.0, "over budget is as hot as it gets");
        let mid = (COOL_SHARE * HOT_SHARE).sqrt();
        assert!((heat(mid) - 0.5).abs() < 1e-4);
        assert!(heat(0.01) < heat(0.05) && heat(0.05) < heat(0.2));
    }

    #[test]
    fn times_and_shares_read_well() {
        assert_eq!(format_ns(0.0), "0.0 µs");
        assert_eq!(format_ns(8_400.0), "8.4 µs");
        assert_eq!(format_ns(37_400.0), "37 µs");
        assert_eq!(format_ns(1_240_000.0), "1.24 ms");
        assert_eq!(format_share(0.007), "0.7 %");
        assert_eq!(format_share(0.123), "12 %");
        let r = Reading {
            ns: 53_333.0,
            share: Some(0.01),
            parts: 3,
        };
        let text = describe(&r, Some(5_333_333.0), false);
        assert!(text.starts_with("53 µs a block, 1.0 % of the 5.33 ms budget"));
        assert!(text.contains("inside"));
    }

    #[test]
    fn engine_paths_sum_into_the_nodes_of_the_scope() {
        let p = |ids: &[u64]| NodePath(ids.iter().map(|i| NodeId(*i)).collect());
        // At the root, a group's insides count toward the group.
        assert_eq!(node_in_scope(&[], &p(&[4])), Some(NodeId(4)));
        assert_eq!(node_in_scope(&[], &p(&[4, 9, 2])), Some(NodeId(4)));
        // Inside group 4.
        let g = [Some(NodeId(4))];
        assert_eq!(node_in_scope(&g, &p(&[4, 9, 2])), Some(NodeId(9)));
        assert_eq!(node_in_scope(&g, &p(&[4])), None);
        assert_eq!(node_in_scope(&g, &p(&[5, 9])), None);
        // Inside Voices node 7: any voice index, and the allocator and
        // summing nodes stay outside the body.
        let v = [Some(NodeId(7)), None];
        assert_eq!(node_in_scope(&v, &p(&[7, 0, 3])), Some(NodeId(3)));
        assert_eq!(node_in_scope(&v, &p(&[7, 5, 3, 1])), Some(NodeId(3)));
        assert_eq!(node_in_scope(&v, &p(&[7, 0])), None);
        assert_eq!(node_in_scope(&v, &p(&[7])), None);
    }

    /// A clock that moves on 1 µs every time it's read: each step of a node
    /// measures exactly 1 µs.
    fn ticking_clock() -> u64 {
        static NOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        NOW.fetch_add(1_000, std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn groups_and_voices_show_the_sum_of_their_insides() {
        use crate::edit;
        use std::sync::Arc;
        let registry = Arc::new(ac_nodes::registry());
        let mut doc = Document::new(
            ac_core::Project::default(),
            None,
            std::path::PathBuf::from("."),
            1,
        );
        let (c, group, voices) = doc.edit(&registry, "Build", |g, ctx| {
            let c = edit::add_node(g, ctx, "constant", ac_core::empty_config(), [0.0, 0.0]);
            let a = edit::add_node(g, ctx, "gain", ac_core::empty_config(), [200.0, 0.0]);
            let b = edit::add_node(g, ctx, "gain", ac_core::empty_config(), [400.0, 0.0]);
            edit::connect(g, ctx, Endpoint::new(c, "out"), Endpoint::new(a, "in")).unwrap();
            edit::connect(g, ctx, Endpoint::new(a, "out"), Endpoint::new(b, "in")).unwrap();
            let (group, _) = edit::group::make_group(g, ctx, &[a, b].into()).unwrap();
            let voices = edit::group::add_voices(g, ctx, [0.0, 300.0]);
            (c, group, voices)
        });
        let (engine, _) = ac_engine::Engine::new(
            doc.project.clone(),
            registry.clone(),
            Arc::new(ac_core::NoResources),
            ac_engine::EngineOptions {
                sample_rate: 48_000.0,
                max_block: 256,
                socket_telemetry: false,
                offline: false,
            },
        );
        let mut host = ac_engine::NullHost::new(engine, 0, 2);
        host.processor().set_clock(Some(ticking_clock));
        let tele = host.handle().telemetry();
        tele.status()
            .timing_enabled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        host.run(256, 256);

        let mut t = Timings::default();
        t.update(&tele, &doc, 0.0);
        let one = t.node(c).unwrap();
        assert_eq!((one.ns, one.parts), (1_000.0, 1));
        let budget = 256.0 / 48_000.0 * 1e9;
        assert!((one.share.unwrap() - (1_000.0 / budget) as f32).abs() < 1e-7);
        let g = t.node(group).unwrap();
        assert!(g.parts >= 2, "both gains: {g:?}");
        assert_eq!(g.ns, g.parts as f32 * 1_000.0);
        // A Voices node counts its allocator, every voice's copy of the
        // body (four nodes, eight voices), and its summing node.
        let v = t.node(voices).unwrap();
        assert!(v.parts > 8 * 4, "{v:?}");
        assert_eq!(v.ns, v.parts as f32 * 1_000.0);
        // Inside the body, each node sums its eight copies.
        assert!(doc.enter_group(voices));
        t.update(&tele, &doc, 1.0 / 30.0);
        let def = doc.current_group().unwrap();
        let osc = def
            .graph
            .nodes
            .iter()
            .find(|n| n.kind == "oscillator")
            .unwrap();
        let r = t.node(osc.id).unwrap();
        assert_eq!(r.parts, 8);
        assert!((r.ns - 8_000.0).abs() < 1e-3, "{r:?}");
        // Turned off, nothing is kept.
        t.clear();
        assert!(t.node(c).is_none());
    }

    #[test]
    fn the_scope_pattern_skips_voice_indices() {
        use crate::edit;
        let registry = ac_nodes::registry();
        let mut doc = Document::new(
            ac_core::Project::default(),
            None,
            std::path::PathBuf::from("."),
            1,
        );
        let voices = doc.edit(&registry, "Add", |g, ctx| {
            edit::group::add_voices(g, ctx, [0.0, 0.0])
        });
        assert!(doc.enter_group(voices));
        assert_eq!(scope_pattern(&doc), vec![Some(voices), None]);
        doc.leave_group();
        assert!(scope_pattern(&doc).is_empty());
    }
}
