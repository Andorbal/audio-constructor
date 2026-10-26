//! Panels made from a selection (DECISIONS D204): a mixer from Track
//! blocks, a pad bank from Samplers, a launch grid from Clips. Each is one
//! undo step that adds what plays the panel (a Pads node, a Button per
//! clip) and the panel, laid out with places, labels, and strips.

use ac_core::{Endpoint, Graph, NodeId, NodeInstance, Panel, PanelItem, PanelLabel};

use super::{Ctx, add_node, connect, pins};
use crate::perform::group_card;

/// Words the generated panels use, from the editor's catalog.
pub struct Words {
    pub mixer: String,
    pub pads: String,
    pub grid: String,
    pub tracks: String,
    /// "Track 1", "Scene 2": a column's and a row's names, counting from 1.
    pub track: fn(usize) -> String,
    pub scene: fn(usize) -> String,
}

/// What a generator made.
#[derive(Debug, PartialEq)]
pub struct Made {
    /// The new panel's index.
    pub panel: usize,
    /// Nodes it added to the graph.
    pub added: Vec<NodeId>,
}

fn label(text: &str, at: [u32; 2], span: u32) -> PanelLabel {
    PanelLabel {
        text: text.to_string(),
        at,
        span,
    }
}

fn placed(node: NodeId, at: [u32; 2]) -> PanelItem {
    PanelItem {
        at: Some(at),
        ..PanelItem::new(node)
    }
}

/// The selection, top to bottom then left to right, as people read it.
fn reading_order<'a>(graph: &'a Graph, ids: &[NodeId]) -> Vec<&'a NodeInstance> {
    let mut nodes: Vec<&NodeInstance> = ids.iter().filter_map(|id| graph.node(*id)).collect();
    nodes.sort_by(|a, b| {
        (a.ui.pos[1], a.ui.pos[0])
            .partial_cmp(&(b.ui.pos[1], b.ui.pos[0]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    nodes
}

/// A mixer: each selected group or block instance (a Track's channel
/// strip) side by side under a "Tracks" label, then the selected controls
/// and displays after them. `None` when nothing selected can be pinned.
pub fn mixer(
    graph: &Graph,
    panels: &mut Vec<Panel>,
    ids: &[NodeId],
    words: &Words,
) -> Option<Made> {
    let mut nodes = reading_order(graph, ids);
    nodes.sort_by(|a, b| {
        a.ui.pos[0]
            .partial_cmp(&b.ui.pos[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let groups: Vec<&NodeInstance> = nodes
        .iter()
        .copied()
        .filter(|n| group_card::is_group(n))
        .collect();
    let loose: Vec<&NodeInstance> = nodes
        .iter()
        .copied()
        .filter(|n| !group_card::is_group(n) && pins::pinnable(n))
        .collect();
    if groups.is_empty() && loose.is_empty() {
        return None;
    }
    let index = pins::add_panel(panels, &words.mixer);
    let p = &mut panels[index];
    if !groups.is_empty() {
        p.labels
            .push(label(&words.tracks, [0, 0], groups.len() as u32));
    }
    for (k, g) in groups.iter().enumerate() {
        p.items.push(placed(g.id, [k as u32, 1]));
    }
    let start = groups.len() as u32;
    for (k, n) in loose.iter().enumerate() {
        let mut it = placed(n.id, [start + k as u32, 1]);
        it.compact = pins::compact_by_default(n);
        p.items.push(it);
    }
    Some(Made {
        panel: index,
        added: Vec::new(),
    })
}

/// Generators a pad or a launch button can start: their `start` is free.
fn startable(graph: &Graph, n: &NodeInstance) -> bool {
    matches!(
        n.kind.as_str(),
        "sampler" | "clip" | "granular" | "graph" | "oscillator" | "noise"
    ) && graph.wire_into(&Endpoint::new(n.id, "start")).is_none()
}

/// A pad bank: one Pads node whose pads start the selected Samplers (and
/// Clips), in reading order from the bottom-left pad, pinned to a new
/// panel. Up to 64.
pub fn pads(
    graph: &mut Graph,
    ctx: &mut Ctx,
    panels: &mut Vec<Panel>,
    ids: &[NodeId],
    words: &Words,
) -> Option<Made> {
    let targets: Vec<(NodeId, [f32; 2])> = reading_order(graph, ids)
        .into_iter()
        .filter(|n| startable(graph, n))
        .take(ac_nodes::controls::pads::MAX_PADS)
        .map(|n| (n.id, n.ui.pos))
        .collect();
    if targets.is_empty() {
        return None;
    }
    let count = targets.len();
    let columns = (count as f64).sqrt().ceil().clamp(1.0, 8.0) as usize;
    let rows = count.div_ceil(columns).clamp(1, 8);
    let left = targets.iter().map(|t| t.1[0]).fold(f32::INFINITY, f32::min);
    let top = targets.iter().map(|t| t.1[1]).fold(f32::INFINITY, f32::min);
    let pads = add_node(
        graph,
        ctx,
        "pads",
        serde_json::json!({"rows": rows, "columns": columns}),
        [left - 340.0, top],
    );
    if let Some(n) = graph.node_mut(pads) {
        n.ui.label = Some(words.pads.clone());
    }
    for (k, (id, _)) in targets.iter().enumerate() {
        let _ = connect(
            graph,
            ctx,
            Endpoint::new(pads, format!("trigger_{k}")),
            Endpoint::new(*id, "start"),
        );
    }
    let index = pins::add_panel(panels, &words.pads);
    panels[index].items.push(placed(pads, [0, 0]));
    Some(Made {
        panel: index,
        added: vec![pads],
    })
}

/// Clips (and other generators) by track: a column wherever they sit
/// apart across the canvas, each top to bottom.
fn columns_of<'a>(graph: &'a Graph, ids: &[NodeId]) -> Vec<Vec<&'a NodeInstance>> {
    let mut nodes: Vec<&NodeInstance> = ids
        .iter()
        .filter_map(|id| graph.node(*id))
        .filter(|n| startable(graph, n))
        .collect();
    nodes.sort_by(|a, b| {
        a.ui.pos[0]
            .partial_cmp(&b.ui.pos[0])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut cols: Vec<Vec<&NodeInstance>> = Vec::new();
    for n in nodes {
        match cols.last_mut() {
            Some(c) if (n.ui.pos[0] - c[0].ui.pos[0]).abs() < crate::theme::NODE_WIDTH / 2.0 => {
                c.push(n)
            }
            _ => cols.push(vec![n]),
        }
    }
    for c in &mut cols {
        c.sort_by(|a, b| {
            a.ui.pos[1]
                .partial_cmp(&b.ui.pos[1])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    cols
}

/// A clip in a launch grid: its id, where it sits, and its label.
type Slot = (NodeId, [f32; 2], Option<String>);

/// A launch grid: a Button for each selected Clip, wired to start it and
/// lit while it plays, laid out a column per track and a row per scene
/// with their names along the top and the left.
pub fn launch_grid(
    graph: &mut Graph,
    ctx: &mut Ctx,
    panels: &mut Vec<Panel>,
    ids: &[NodeId],
    words: &Words,
) -> Option<Made> {
    let cols: Vec<Vec<Slot>> = columns_of(graph, ids)
        .into_iter()
        .map(|c| {
            c.into_iter()
                .map(|n| (n.id, n.ui.pos, n.ui.label.clone()))
                .collect()
        })
        .collect();
    if cols.is_empty() {
        return None;
    }
    let index = pins::add_panel(panels, &words.grid);
    let mut added = Vec::new();
    let rows = cols.iter().map(Vec::len).max().unwrap_or(0);
    for r in 0..rows {
        let name = (words.scene)(r + 1);
        panels[index]
            .labels
            .push(label(&name, [0, 2 + 2 * r as u32], 1));
    }
    for (c, col) in cols.iter().enumerate() {
        let name = (words.track)(c + 1);
        panels[index]
            .labels
            .push(label(&name, [1 + c as u32, 0], 1));
        for (r, (id, pos, title)) in col.iter().enumerate() {
            let button = add_node(
                graph,
                ctx,
                "button",
                ac_core::empty_config(),
                [pos[0] - 260.0, pos[1]],
            );
            if let Some(b) = graph.node_mut(button) {
                b.ui.label = title
                    .clone()
                    .or_else(|| Some(format!("{} {}", (words.track)(c + 1), (words.scene)(r + 1))));
            }
            let _ = connect(
                graph,
                ctx,
                Endpoint::new(button, "trigger"),
                Endpoint::new(*id, "start"),
            );
            let _ = connect(
                graph,
                ctx,
                Endpoint::new(*id, "playing"),
                Endpoint::new(button, "lit"),
            );
            panels[index]
                .items
                .push(placed(button, [1 + c as u32, 1 + 2 * r as u32 + 1]));
            added.push(button);
        }
    }
    Some(Made {
        panel: index,
        added,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ac_core::Project;

    use super::*;
    use crate::document::Document;

    fn words() -> Words {
        Words {
            mixer: "Mixer".into(),
            pads: "Pads".into(),
            grid: "Grid".into(),
            tracks: "Tracks".into(),
            track: |n| format!("Track {n}"),
            scene: |n| format!("Scene {n}"),
        }
    }

    fn doc(nodes: &[(u64, &str, [f32; 2])]) -> (Document, ac_core::Registry) {
        let mut p = Project::default();
        for &(id, kind, pos) in nodes {
            p.graph
                .nodes
                .push(NodeInstance::new(NodeId(id), kind).at(pos[0], pos[1]));
        }
        (
            Document::new(p, None, PathBuf::from("."), 1),
            ac_nodes::registry(),
        )
    }

    #[test]
    fn pads_from_samplers_wire_a_pads_node_in_reading_order() {
        let (mut d, r) = doc(&[
            (1, "sampler", [0.0, 200.0]),
            (2, "sampler", [0.0, 0.0]),
            (3, "sampler", [300.0, 0.0]),
            (4, "gain", [0.0, 400.0]),
        ]);
        let made = d
            .edit_with_panels(&r, "Pads", |g, ctx, panels| {
                pads(
                    g,
                    ctx,
                    panels,
                    &[NodeId(1), NodeId(2), NodeId(3), NodeId(4)],
                    &words(),
                )
            })
            .expect("made");
        let pads_id = made.added[0];
        let g = &d.project.graph;
        assert_eq!(g.node(pads_id).unwrap().config["columns"], 2);
        let into = |n: u64| {
            g.wire_into(&Endpoint::new(NodeId(n), "start"))
                .map(|w| w.from.socket.clone())
        };
        assert_eq!(into(2).as_deref(), Some("trigger_0"));
        assert_eq!(into(3).as_deref(), Some("trigger_1"));
        assert_eq!(into(1).as_deref(), Some("trigger_2"));
        assert_eq!(d.project.panels[made.panel].items[0].node, pads_id);
        assert_eq!(d.undo_label(), Some("Pads"));
        assert!(d.undo());
        assert!(
            d.project.panels.is_empty() && d.project.graph.nodes.len() == 4,
            "one step"
        );
    }

    #[test]
    fn a_launch_grid_is_a_column_per_track_and_a_row_per_scene() {
        let (mut d, r) = doc(&[
            (1, "clip", [0.0, 0.0]),
            (2, "clip", [0.0, 200.0]),
            (3, "clip", [400.0, 0.0]),
        ]);
        let made = d
            .edit_with_panels(&r, "Grid", |g, ctx, panels| {
                launch_grid(g, ctx, panels, &[NodeId(3), NodeId(1), NodeId(2)], &words())
            })
            .unwrap();
        assert_eq!(made.added.len(), 3);
        let p = &d.project.panels[made.panel];
        let texts: Vec<&str> = p.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Scene 1", "Scene 2", "Track 1", "Track 2"]);
        let at: Vec<[u32; 2]> = p.items.iter().filter_map(|i| i.at).collect();
        assert_eq!(at, vec![[1, 2], [1, 4], [2, 2]]);
        let g = &d.project.graph;
        let b = made.added[2];
        assert_eq!(
            g.wire_into(&Endpoint::new(NodeId(3), "start"))
                .unwrap()
                .from
                .node,
            b
        );
        assert_eq!(
            g.wire_into(&Endpoint::new(b, "lit")).unwrap().from,
            Endpoint::new(NodeId(3), "playing")
        );
    }

    #[test]
    fn a_mixer_puts_groups_side_by_side_under_a_label() {
        let mut p = Project::default();
        for (id, x) in [(1u64, 300.0f32), (2, 0.0)] {
            let mut n = NodeInstance::new(NodeId(id), ac_core::GROUP_KIND).at(x, 0.0);
            n.config = serde_json::json!({"def": "track"});
            p.graph.nodes.push(n);
        }
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "toggle").at(600.0, 0.0));
        let mut panels = Vec::new();
        let made = mixer(
            &p.graph,
            &mut panels,
            &[NodeId(1), NodeId(2), NodeId(3)],
            &words(),
        )
        .unwrap();
        let pl = &panels[made.panel];
        assert_eq!(pl.labels[0].text, "Tracks");
        assert_eq!(pl.labels[0].span, 2);
        let order: Vec<(u64, Option<[u32; 2]>, bool)> = pl
            .items
            .iter()
            .map(|i| (i.node.0, i.at, i.compact))
            .collect();
        assert_eq!(
            order,
            vec![
                (2, Some([0, 1]), false),
                (1, Some([1, 1]), false),
                (3, Some([2, 1]), true)
            ]
        );
    }
}
