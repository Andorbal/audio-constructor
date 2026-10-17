//! The canvas scene for one frame: every node's card laid out in canvas
//! units with its sockets, plus the wires between them. Built from the
//! document; drawing and hit testing both read it.

use std::collections::{BTreeSet, HashMap};

use ac_core::{
    Category, ConfigExt, Endpoint, Graph, NodeId, Registry, SignalType, SocketSpec, Wire,
};
use egui::{Color32, Pos2, Rect, pos2, vec2};

use crate::document::Document;
use crate::edit::{self, GROUP_IN, GROUP_OUT, HIDE_UNWIRED, NEW_SOCKET, NodeClass, Sockets};
use crate::theme;

/// Nodes whose body is a widget (NODES.md Controls and Display).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Button,
    Toggle,
    Slider,
    Meter,
    Scope,
    Readout,
    Clip,
    Record,
    AudioOut,
}

impl Body {
    pub fn of(kind: &str) -> Option<Body> {
        Some(match kind {
            "button" => Body::Button,
            "toggle" => Body::Toggle,
            "slider" => Body::Slider,
            "meter" => Body::Meter,
            "scope" => Body::Scope,
            "readout" => Body::Readout,
            "clip" => Body::Clip,
            "record" => Body::Record,
            "audio_out" => Body::AudioOut,
            _ => return None,
        })
    }

    fn height(self) -> f32 {
        match self {
            Body::Button => 40.0,
            Body::Toggle => 30.0,
            Body::Slider => 30.0,
            Body::Meter => 40.0,
            Body::Scope => 92.0,
            Body::Readout => 34.0,
            Body::Clip => 24.0,
            Body::Record => 26.0,
            Body::AudioOut => 16.0,
        }
    }

    /// The input the body controls; its row shows no inline widget.
    pub fn controls(self) -> Option<&'static str> {
        match self {
            Body::Button => Some("pressed"),
            Body::Toggle => Some("value"),
            Body::Slider => Some("value"),
            _ => None,
        }
    }
}

/// What a card is.
#[derive(Clone, Debug, PartialEq)]
pub enum Class {
    Normal,
    Group(ac_core::GroupId),
    Missing(String),
    Reroute,
    Comment,
    /// The group interface inside a definition (`input_side`: Group Input).
    Interface {
        input_side: bool,
    },
}

#[derive(Clone, Debug)]
pub struct SocketView {
    pub name: String,
    pub ty: SignalType,
    pub spec: Option<SocketSpec>,
    /// Canvas position of the socket dot.
    pub pos: Pos2,
    /// The row it sits on (canvas); empty for collapsed nodes and reroutes.
    pub row: Rect,
    pub wired: bool,
    pub input: bool,
}

impl SocketView {
    pub fn endpoint(&self, node: NodeId) -> Endpoint {
        Endpoint::new(node, self.name.clone())
    }

    /// The empty "drop to add" socket of an interface node.
    pub fn is_new_slot(&self) -> bool {
        self.name == NEW_SOCKET
    }
}

#[derive(Clone, Debug)]
pub struct NodeView {
    pub id: NodeId,
    pub kind: String,
    pub class: Class,
    pub title: String,
    pub category: Option<Category>,
    pub accent: Color32,
    /// The whole card (canvas).
    pub rect: Rect,
    pub header: Rect,
    pub collapsed: bool,
    pub muted: bool,
    pub selected: bool,
    pub inputs: Vec<SocketView>,
    pub outputs: Vec<SocketView>,
    pub body: Option<(Body, Rect)>,
}

impl NodeView {
    pub fn socket(&self, name: &str, input: bool) -> Option<&SocketView> {
        let list = if input { &self.inputs } else { &self.outputs };
        list.iter().find(|s| s.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct WireView {
    pub wire: Wire,
    pub from: Pos2,
    pub to: Pos2,
    pub ty: SignalType,
    /// Touches a selected node (drawn highlighted).
    pub highlight: bool,
}

pub struct Scene {
    /// Nodes in drawing order (selected nodes last, on top).
    pub nodes: Vec<NodeView>,
    pub wires: Vec<WireView>,
    pub index: HashMap<NodeId, usize>,
}

impl Scene {
    pub fn node(&self, id: NodeId) -> Option<&NodeView> {
        self.index.get(&id).map(|&i| &self.nodes[i])
    }

    /// Bounding box of all cards (canvas), if any.
    pub fn bounds(&self, only: Option<&BTreeSet<NodeId>>) -> Option<Rect> {
        self.nodes
            .iter()
            .filter(|n| only.is_none_or(|s| s.contains(&n.id)))
            .map(|n| n.rect)
            .reduce(|a, b| a.union(b))
    }
}

/// Where the interface cards sit: left and right of the inner nodes, unless
/// the user moved them this session.
pub fn interface_positions(graph: &Graph, moved: Option<[Pos2; 2]>) -> [Pos2; 2] {
    if let Some(m) = moved {
        return m;
    }
    let mut b: Option<Rect> = None;
    for n in &graph.nodes {
        let r = Rect::from_min_size(
            pos2(n.ui.pos[0], n.ui.pos[1]),
            vec2(theme::NODE_WIDTH, 80.0),
        );
        b = Some(b.map_or(r, |x| x.union(r)));
    }
    let b = b.unwrap_or(Rect::from_center_size(Pos2::ZERO, vec2(200.0, 100.0)));
    [
        pos2(b.min.x - theme::NODE_WIDTH - 90.0, b.center().y - 50.0),
        pos2(b.max.x + 90.0, b.center().y - 50.0),
    ]
}

/// Build the scene for the document's current scope.
pub fn build(doc: &Document, registry: &Registry, iface_pos: Option<[Pos2; 2]>) -> Scene {
    let graph = doc.display_graph();
    let iface = doc.interface();
    let defs = &doc.project.group_defs;
    let wired_in: BTreeSet<(NodeId, &str)> = graph
        .wires
        .iter()
        .map(|w| (w.to.node, w.to.socket.as_str()))
        .collect();
    let wired_out: BTreeSet<(NodeId, &str)> = graph
        .wires
        .iter()
        .map(|w| (w.from.node, w.from.socket.as_str()))
        .collect();

    let mut nodes = Vec::new();
    if let Some(iface) = &iface {
        let pos = interface_positions(doc.graph(), iface_pos);
        for (i, input_side) in [true, false].into_iter().enumerate() {
            let id = if input_side { GROUP_IN } else { GROUP_OUT };
            let sockets = edit::sockets_of(&graph, id, registry, defs, Some(iface));
            let title = if input_side {
                "Group Input"
            } else {
                "Group Output"
            };
            nodes.push(layout(
                LayoutIn {
                    id,
                    kind: String::new(),
                    class: Class::Interface { input_side },
                    title: title.into(),
                    category: None,
                    accent: theme::GROUP_HEADER,
                    pos: pos[i],
                    width: theme::NODE_WIDTH - 30.0,
                    size: None,
                    collapsed: false,
                    hide_unwired: false,
                    muted: false,
                    selected: doc.selection.contains(&id),
                    body: None,
                },
                &sockets,
                &wired_in,
                &wired_out,
            ));
        }
    }

    for n in &graph.nodes {
        let class = match edit::classify(n, registry, defs) {
            NodeClass::Known if n.kind == "comment" => Class::Comment,
            NodeClass::Known => Class::Normal,
            NodeClass::Group(d) => Class::Group(d),
            NodeClass::Missing(m) => Class::Missing(m),
            NodeClass::Reroute => Class::Reroute,
        };
        let entry = registry.get(&n.kind);
        let category = entry.map(|e| e.meta.category);
        let (title, accent) = match &class {
            Class::Group(d) => (
                n.ui.label
                    .clone()
                    .unwrap_or_else(|| defs.get(d).map_or("Group".into(), |d| d.name.clone())),
                theme::GROUP_HEADER,
            ),
            Class::Missing(_) => (
                n.ui.label.clone().unwrap_or_else(|| n.kind.clone()),
                theme::MISSING_HEADER,
            ),
            _ => (
                n.ui.label
                    .clone()
                    .unwrap_or_else(|| entry.map_or(n.kind.clone(), |e| e.meta.name.to_string())),
                category.map_or(theme::MISSING_HEADER, theme::category_color),
            ),
        };
        let sockets = edit::sockets_of(&graph, n.id, registry, defs, iface.as_ref());
        let body = match class {
            Class::Normal => Body::of(&n.kind),
            _ => None,
        };
        let width = if class == Class::Comment {
            n.ui.size.map_or(240.0, |s| s[0])
        } else {
            n.ui.size.map_or(theme::NODE_WIDTH, |s| s[0]).max(80.0)
        };
        nodes.push(layout(
            LayoutIn {
                id: n.id,
                kind: n.kind.clone(),
                class,
                title,
                category,
                accent,
                pos: pos2(n.ui.pos[0], n.ui.pos[1]),
                width,
                size: n.ui.size,
                collapsed: n.ui.collapsed,
                hide_unwired: n.extra.get(HIDE_UNWIRED).and_then(|v| v.as_bool()) == Some(true),
                muted: n.muted,
                selected: doc.selection.contains(&n.id),
                body,
            },
            &sockets,
            &wired_in,
            &wired_out,
        ));
    }
    // Selected cards draw (and hit-test) on top without reordering the file.
    let (sel, unsel): (Vec<NodeView>, Vec<NodeView>) = nodes.into_iter().partition(|n| n.selected);
    let nodes: Vec<NodeView> = unsel.into_iter().chain(sel).collect();
    let index: HashMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();

    let mut wires = Vec::new();
    for w in &graph.wires {
        let (Some(a), Some(b)) = (index.get(&w.from.node), index.get(&w.to.node)) else {
            continue;
        };
        let (a, b) = (&nodes[*a], &nodes[*b]);
        let (Some(from), Some(to)) = (
            a.socket(&w.from.socket, false),
            b.socket(&w.to.socket, true),
        ) else {
            continue;
        };
        let ty = if from.ty == SignalType::Any {
            to.ty
        } else {
            from.ty
        };
        wires.push(WireView {
            wire: w.clone(),
            from: from.pos,
            to: to.pos,
            ty,
            highlight: a.selected || b.selected,
        });
    }
    Scene {
        nodes,
        wires,
        index,
    }
}

struct LayoutIn {
    id: NodeId,
    kind: String,
    class: Class,
    title: String,
    category: Option<Category>,
    accent: Color32,
    pos: Pos2,
    width: f32,
    size: Option<[f32; 2]>,
    collapsed: bool,
    hide_unwired: bool,
    muted: bool,
    selected: bool,
    body: Option<Body>,
}

fn layout(
    l: LayoutIn,
    sockets: &Sockets,
    wired_in: &BTreeSet<(NodeId, &str)>,
    wired_out: &BTreeSet<(NodeId, &str)>,
) -> NodeView {
    let id = l.id;
    let mk = |s: &edit::SocketInfo, input: bool| SocketView {
        name: s.name.clone(),
        ty: s.ty,
        spec: s.spec.clone(),
        pos: Pos2::ZERO,
        row: Rect::NOTHING,
        wired: if input {
            wired_in.contains(&(id, s.name.as_str()))
        } else {
            wired_out.contains(&(id, s.name.as_str()))
        },
        input,
    };
    let mut inputs: Vec<SocketView> = sockets.inputs.iter().map(|s| mk(s, true)).collect();
    let mut outputs: Vec<SocketView> = sockets.outputs.iter().map(|s| mk(s, false)).collect();
    if l.hide_unwired {
        inputs.retain(|s| s.wired);
        outputs.retain(|s| s.wired);
    }

    let mut view = NodeView {
        id,
        kind: l.kind,
        class: l.class,
        title: l.title,
        category: l.category,
        accent: l.accent,
        rect: Rect::NOTHING,
        header: Rect::NOTHING,
        collapsed: l.collapsed,
        muted: l.muted,
        selected: l.selected,
        inputs,
        outputs,
        body: None,
    };
    let x = l.pos.x;

    if view.class == Class::Reroute {
        // A reroute's position is its center.
        let r = theme::REROUTE_RADIUS;
        view.rect = Rect::from_center_size(l.pos, vec2(r * 2.0, r * 2.0));
        view.header = view.rect;
        for s in view.inputs.iter_mut().chain(view.outputs.iter_mut()) {
            s.pos = l.pos;
        }
        return view;
    }

    if view.class == Class::Comment {
        let size = l.size.unwrap_or([240.0, 120.0]);
        view.rect = Rect::from_min_size(l.pos, vec2(size[0].max(80.0), size[1].max(40.0)));
        view.header = Rect::from_min_size(l.pos, vec2(view.rect.width(), 20.0));
        return view;
    }

    if l.collapsed {
        let w = (view.title.chars().count() as f32 * 7.0 + 52.0).clamp(80.0, l.width);
        view.rect = Rect::from_min_size(l.pos, vec2(w, theme::PILL_HEIGHT));
        view.header = view.rect;
        let cy = view.rect.center().y;
        for s in &mut view.inputs {
            s.pos = pos2(x, cy);
        }
        for s in &mut view.outputs {
            s.pos = pos2(x + w, cy);
        }
        return view;
    }

    let w = l.width;
    view.header = Rect::from_min_size(l.pos, vec2(w, theme::HEADER_HEIGHT));
    let mut y = l.pos.y + theme::HEADER_HEIGHT + 3.0;
    for s in &mut view.outputs {
        s.row = Rect::from_min_size(pos2(x, y), vec2(w, theme::ROW_HEIGHT));
        s.pos = pos2(x + w, y + theme::ROW_HEIGHT / 2.0);
        y += theme::ROW_HEIGHT;
    }
    if let Some(b) = l.body {
        y += 3.0;
        let r = Rect::from_min_size(
            pos2(x + theme::BODY_PAD, y),
            vec2(w - 2.0 * theme::BODY_PAD, b.height()),
        );
        view.body = Some((b, r));
        y += b.height() + 5.0;
    }
    for s in &mut view.inputs {
        s.row = Rect::from_min_size(pos2(x, y), vec2(w, theme::ROW_HEIGHT));
        s.pos = pos2(x, y + theme::ROW_HEIGHT / 2.0);
        y += theme::ROW_HEIGHT;
    }
    y += theme::NODE_PAD_BOTTOM;
    let min_h = theme::HEADER_HEIGHT + 10.0;
    view.rect = Rect::from_min_max(l.pos, pos2(x + w, y.max(l.pos.y + min_h)));
    view
}

/// The text of a Comment node.
pub fn comment_text(doc: &Document, id: NodeId) -> String {
    doc.graph()
        .node(id)
        .map(|n| n.config.get_str_or("text", "").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{NodeInstance, Project};
    use std::path::PathBuf;

    #[test]
    fn cards_have_outputs_then_body_then_inputs() {
        let registry = ac_nodes::registry();
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "meter").at(10.0, 20.0));
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "quantum").at(300.0, 20.0));
        p.graph.connect(
            Endpoint::new(NodeId(2), "flux"),
            Endpoint::new(NodeId(1), "in"),
        );
        let doc = Document::new(p, None, PathBuf::from("."), 1);
        let scene = build(&doc, &registry, None);
        let m = scene.node(NodeId(1)).unwrap();
        assert_eq!(m.body.map(|b| b.0), Some(Body::Meter));
        let out = m.socket("out", false).unwrap();
        let inp = m.socket("in", true).unwrap();
        let body = m.body.unwrap().1;
        assert!(out.pos.y < body.min.y && body.max.y < inp.pos.y);
        assert_eq!(out.pos.x, m.rect.max.x);
        assert_eq!(inp.pos.x, m.rect.min.x);
        assert!(inp.wired);
        // The missing node is still drawn, with the socket its wire uses.
        let missing = scene.node(NodeId(2)).unwrap();
        assert!(matches!(missing.class, Class::Missing(_)));
        assert!(missing.socket("flux", false).is_some());
        assert_eq!(scene.wires.len(), 1);
    }
}
