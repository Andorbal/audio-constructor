//! Bindings in the editor (UX.md "Bindings"): learning a control for an
//! input or a transport action, the inputs on the canvas that are bound
//! (with the live values their widgets follow), keeping bindings with
//! their nodes through edits, and the gamepad transport preset.
//!
//! The engine applies bindings; the editor only stores them in the
//! project. A binding targets one engine path: binding a node inside a
//! group binds it in the instance the group was entered through, not in
//! every instance (DECISIONS D94).

use std::collections::HashMap;

use ac_core::{
    Binding, GamepadButton, Graph, InputEvent, Learner, NodeId, NodeInstance, NodePath, Project,
    Registry, SignalType, Source, Target, TransportAction, Value,
};
use ac_engine::BindingValues;

use crate::document::Document;

/// What learning will bind.
#[derive(Clone, Debug, PartialEq)]
pub enum LearnTarget {
    Input { node: NodePath, socket: String },
    Transport(TransportAction),
}

/// Learning a binding: waiting for the first control that moves.
#[derive(Clone, Debug)]
pub struct Learn {
    pub target: LearnTarget,
    /// What is being bound, for the status line ("Gain · gain").
    pub label: String,
    learner: Learner,
    /// OSC addresses the project mentions, to name OSC events by.
    osc: Vec<String>,
}

impl Learn {
    pub fn new(target: LearnTarget, label: String, project: &Project) -> Learn {
        Learn {
            target,
            label,
            learner: Learner::new(),
            osc: osc_addresses(project),
        }
    }

    /// Feed events; returns the source to bind once one qualifies.
    pub fn feed(&mut self, events: &[InputEvent]) -> Option<Source> {
        let osc = std::mem::take(&mut self.osc);
        let name = |hash: u64| {
            osc.iter()
                .find(|a| ac_core::osc_address_hash(a) == hash)
                .cloned()
        };
        let mut found = None;
        for e in events {
            if let Some(s) = self.learner.feed(e, &name) {
                found = Some(s);
                break;
            }
        }
        self.osc = osc;
        found
    }

    /// The binding to create for `source`. A toggle suits a Bool input
    /// bound to something pressed; everything else follows the control.
    pub fn binding(&self, source: Source, ty: Option<SignalType>) -> Binding {
        match &self.target {
            LearnTarget::Input { node, socket } => {
                let mut b = Binding::input(source, node.clone(), socket.clone());
                // A key or pad that's pressed and let go is clearer as an
                // on/off switch, except on inputs that mean "while held".
                if ty == Some(SignalType::Bool)
                    && !b.source.is_continuous()
                    && !momentary_socket(socket)
                {
                    b.mode = ac_core::GateMode::Toggle;
                }
                b
            }
            LearnTarget::Transport(action) => Binding::transport(source, *action),
        }
    }
}

/// Inputs that mean "while held" (a Button's `pressed`, a Keyboard key, an
/// XY Pad's `touched`): bound momentary.
fn momentary_socket(socket: &str) -> bool {
    socket == "pressed" || socket == "touched" || socket == "gate" || socket.starts_with("key_")
}

/// Add a binding as one undo step, unless the same source already drives
/// the same target. Returns whether it was added.
pub fn add(doc: &mut Document, label: &str, binding: Binding) -> bool {
    add_all(doc, label, vec![binding]) > 0
}

/// Add several bindings as one undo step, skipping ones that duplicate a
/// binding's source and target. Returns how many were added.
pub fn add_all(doc: &mut Document, label: &str, bindings: Vec<Binding>) -> usize {
    doc.edit_project(label, |p| {
        let mut n = 0;
        for b in bindings {
            if !p
                .bindings
                .iter()
                .any(|o| o.source == b.source && o.target == b.target)
            {
                p.bindings.push(b);
                n += 1;
            }
        }
        n
    })
}

/// The gamepad transport preset (UX.md): A plays and stops, B stops, X
/// records, on any gamepad.
pub fn transport_preset() -> Vec<Binding> {
    [
        (GamepadButton::South, TransportAction::PlayStop),
        (GamepadButton::East, TransportAction::Stop),
        (GamepadButton::West, TransportAction::Record),
    ]
    .into_iter()
    .map(|(button, action)| Binding::transport(Source::GamepadButton { pad: None, button }, action))
    .collect()
}

/// OSC addresses the project mentions: node settings that look like an
/// address and the addresses of OSC bindings. The engine knows OSC events
/// only by a hash of their address; these are the names learning can give
/// them back.
pub fn osc_addresses(project: &Project) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut visit = |g: &Graph| {
        for n in &g.nodes {
            if let Some(o) = n.config.as_object() {
                for v in o.values() {
                    if let Some(s) = v.as_str()
                        && ac_core::control::valid_osc_address(s)
                    {
                        out.push(s.to_string());
                    }
                }
            }
        }
    };
    visit(&project.graph);
    for d in project.group_defs.values() {
        visit(&d.graph);
    }
    for b in &project.bindings {
        if let Source::Osc { address, .. } = &b.source {
            out.push(address.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The graph a node path's last id lives in.
fn graph_at<'p>(project: &'p Project, parents: &[NodeId]) -> Option<&'p Graph> {
    let mut graph = &project.graph;
    for id in parents {
        let def = graph.node(*id)?.group_def()?;
        graph = &project.group_defs.get(&def)?.graph;
    }
    Some(graph)
}

/// Keep bindings with their nodes after an edit (`before` → `after`): a
/// binding whose node was grouped or ungrouped follows it to its new path;
/// one whose node was deleted goes, as part of the same undo step.
/// Bindings that already pointed nowhere stay (the engine reports them).
pub fn follow_edit(before: &Project, after: &mut Project) {
    if after.bindings.is_empty() {
        return;
    }
    let mut bindings = std::mem::take(&mut after.bindings);
    bindings.retain_mut(|b| {
        let Target::Input { node, .. } = &mut b.target else {
            return true;
        };
        if after.node_at(node).is_some() || before.node_at(node).is_none() {
            return true;
        }
        match relocate(before, after, node) {
            Some(p) => {
                *node = p;
                true
            }
            None => false,
        }
    });
    after.bindings = bindings;
}

/// Where a node that vanished from `path` went: into a group made around
/// it (same id, inside a new instance at its level), or out of a group
/// that was taken apart (a new node at the instance's level with the same
/// kind and settings, when exactly one fits).
fn relocate(before: &Project, after: &Project, path: &NodePath) -> Option<NodePath> {
    let (leaf, parents) = path.0.split_last()?;
    let old = before.node_at(path)?;
    let same = |n: &NodeInstance| n.kind == old.kind && n.config == old.config;
    // Grouped: a new instance at the same level holds it.
    if let (Some(g_after), Some(g_before)) = (graph_at(after, parents), graph_at(before, parents)) {
        let holders: Vec<NodeId> = g_after
            .nodes
            .iter()
            .filter(|n| !g_before.contains(n.id))
            .filter(|n| {
                n.group_def()
                    .and_then(|d| after.group_defs.get(&d))
                    .and_then(|d| d.graph.node(*leaf))
                    .is_some_and(same)
            })
            .map(|n| n.id)
            .collect();
        if let [inst] = holders.as_slice() {
            let mut v = parents.to_vec();
            v.extend([*inst, *leaf]);
            return Some(NodePath(v));
        }
    }
    // Ungrouped: its instance is gone and its copy sits one level up.
    let (inst, grand) = parents.split_last()?;
    let g_after = graph_at(after, grand)?;
    let g_before = graph_at(before, grand)?;
    if g_after.contains(*inst) {
        return None;
    }
    let copies: Vec<NodeId> = g_after
        .nodes
        .iter()
        .filter(|n| !g_before.contains(n.id) && same(n))
        .map(|n| n.id)
        .collect();
    let [copy] = copies.as_slice() else {
        return None;
    };
    let mut v = grand.to_vec();
    v.push(*copy);
    Some(NodePath(v))
}

/// A bound input on the canvas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundInput {
    /// What drives it ("MIDI CC 74 · ch 1"), one per binding.
    pub sources: Vec<String>,
    /// The value the engine's bindings last set, while there is one.
    pub live: Option<Value>,
}

/// The bound inputs of the current scope, by node id and socket.
#[derive(Clone, Debug, Default)]
pub struct BoundInputs {
    pub inputs: HashMap<(NodeId, String), BoundInput>,
}

impl BoundInputs {
    /// Gather the bindings that target nodes in the current scope (through
    /// the instance it was entered by), with live values from the engine.
    pub fn gather(doc: &Document, registry: &Registry, values: &BindingValues) -> BoundInputs {
        let mut inputs: HashMap<(NodeId, String), BoundInput> = HashMap::new();
        let prefix = doc.scope_prefix();
        for (i, b) in doc.project.bindings.iter().enumerate() {
            let Target::Input { node, socket } = &b.target else {
                continue;
            };
            let Some((leaf, parents)) = node.0.split_last() else {
                continue;
            };
            if parents != prefix.as_slice() {
                continue;
            }
            let e = inputs.entry((*leaf, socket.clone())).or_default();
            e.sources.push(b.source.label());
            if let Some(v) = values.get(i)
                && let Some(n) = doc.graph().node(*leaf)
            {
                let ty = registry
                    .signature_of(n, &doc.project.group_defs)
                    .and_then(|s| s.inputs.into_iter().find(|s| s.name == *socket))
                    .map(|s| s.ty);
                e.live = Some(match ty {
                    Some(SignalType::Bool) => Value::Bool(v >= 0.5),
                    Some(SignalType::Int) => Value::Int(v.round() as i64),
                    _ => Value::Float(f64::from(v)),
                });
            }
        }
        BoundInputs { inputs }
    }

    pub fn get(&self, node: NodeId, socket: &str) -> Option<&BoundInput> {
        self.inputs.get(&(node, socket.to_string()))
    }

    pub fn is_bound(&self, node: NodeId, socket: &str) -> bool {
        self.get(node, socket).is_some()
    }

    /// `node` as the canvas should draw it: its stored values, with bound
    /// inputs at the value their bindings last set.
    pub fn shown(&self, node: &NodeInstance) -> NodeInstance {
        let mut n = node.clone();
        for ((id, socket), b) in &self.inputs {
            if *id == node.id
                && let Some(v) = &b.live
            {
                n.params.insert(socket.clone(), v.clone());
            }
        }
        n
    }
}

/// A binding's target for people: "Gain · gain", "Transport: Record".
pub fn target_label(project: &Project, registry: &Registry, target: &Target) -> String {
    match target {
        Target::Transport { action } => format!("Transport: {}", action.label()),
        Target::Input { node, socket } => {
            let name = project.node_at(node).map_or_else(
                || format!("missing node {node}"),
                |n| node_name(project, registry, n),
            );
            format!("{name} · {}", crate::canvas::paint_pretty(socket))
        }
    }
}

/// A node's label, or its kind's (or group's) name.
pub fn node_name(project: &Project, registry: &Registry, n: &NodeInstance) -> String {
    if let Some(l) = n.ui.label.as_ref().filter(|l| !l.trim().is_empty()) {
        return l.clone();
    }
    if let Some(d) = n.group_def().and_then(|d| project.group_defs.get(&d)) {
        return d.name.clone();
    }
    registry
        .get(&n.kind)
        .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string())
}

/// The input a control node's "Bind…" binds: its main value. An XY Pad has
/// two (`x`, `y`); others have none.
pub fn control_inputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "slider" | "knob" | "toggle" | "number" | "dropdown" => &["value"],
        "button" => &["pressed"],
        "xy_pad" => &["x", "y"],
        _ => &[],
    }
}

/// The computer key an editor key press is, for learning key bindings
/// while the canvas has the keyboard.
pub fn key_from_egui(key: egui::Key) -> Option<ac_core::Key> {
    use egui::Key as E;
    let name = match key {
        E::ArrowUp => "Up",
        E::ArrowDown => "Down",
        E::ArrowLeft => "Left",
        E::ArrowRight => "Right",
        E::OpenBracket => "LeftBracket",
        E::CloseBracket => "RightBracket",
        E::Backtick => "Backtick",
        E::Quote => "Quote",
        E::Semicolon => "Semicolon",
        E::Comma => "Comma",
        E::Period => "Period",
        E::Slash => "Slash",
        E::Backslash => "Backslash",
        E::Minus => "Minus",
        E::Equals => "Equals",
        E::Num0 => "0",
        E::Num1 => "1",
        E::Num2 => "2",
        E::Num3 => "3",
        E::Num4 => "4",
        E::Num5 => "5",
        E::Num6 => "6",
        E::Num7 => "7",
        E::Num8 => "8",
        E::Num9 => "9",
        other => other.name(),
    };
    ac_core::Key::from_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{GROUP_KIND, GroupDef, GroupId, NodeId};

    fn cc(cc: u8) -> Source {
        Source::MidiCc { channel: None, cc }
    }

    fn project_with_group() -> Project {
        let mut p = Project::default();
        p.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
        let mut def = GroupDef::new("G");
        def.graph.nodes.push(NodeInstance::new(NodeId(5), "knob"));
        p.group_defs.insert(GroupId::from("g"), def);
        p.graph.nodes.push(
            NodeInstance::new(NodeId(9), GROUP_KIND)
                .with_config(serde_json::json!({"def": "g"})),
        );
        p
    }

    #[test]
    fn deleting_a_node_drops_its_bindings_only() {
        let before = {
            let mut p = project_with_group();
            p.bindings
                .push(Binding::input(cc(1), NodePath::root(NodeId(1)), "gain"));
            p.bindings.push(Binding::input(
                cc(2),
                NodePath(vec![NodeId(9), NodeId(5)]),
                "value",
            ));
            p.bindings
                .push(Binding::input(cc(3), NodePath::root(NodeId(77)), "gain"));
            p.bindings.push(Binding::transport(cc(4), TransportAction::Play));
            p
        };
        let mut after = before.clone();
        after.graph.remove_node(NodeId(1));
        follow_edit(&before, &mut after);
        let ccs: Vec<String> = after.bindings.iter().map(|b| b.source.label()).collect();
        assert_eq!(after.bindings.len(), 3, "{ccs:?}");
        // The dangling one (77) was already dangling: kept for the report.
        assert!(after.bindings.iter().all(|b| b.source != cc(1)));
        // Removing the group instance removes bindings inside it.
        let before = after.clone();
        after.graph.remove_node(NodeId(9));
        follow_edit(&before, &mut after);
        assert!(after.bindings.iter().all(|b| b.source != cc(2)));
    }

    #[test]
    fn bindings_follow_their_node_into_and_out_of_a_group() {
        let mut before = Project::default();
        before
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "slider"));
        before
            .bindings
            .push(Binding::input(cc(1), NodePath::root(NodeId(3)), "value"));
        // Grouped: node 3 moves into a new definition under instance 20.
        let mut grouped = before.clone();
        let n = grouped.graph.remove_node(NodeId(3)).unwrap();
        let mut def = GroupDef::new("Group");
        def.graph.nodes.push(n);
        grouped.group_defs.insert(GroupId::from("group"), def);
        grouped.graph.nodes.push(
            NodeInstance::new(NodeId(20), GROUP_KIND)
                .with_config(serde_json::json!({"def": "group"})),
        );
        follow_edit(&before, &mut grouped);
        assert_eq!(
            grouped.bindings[0].node(),
            Some(&NodePath(vec![NodeId(20), NodeId(3)]))
        );
        // Ungrouped: the copy gets a fresh id at the root.
        let mut ungrouped = grouped.clone();
        ungrouped.graph.remove_node(NodeId(20));
        ungrouped.group_defs.clear();
        ungrouped
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(44), "slider"));
        follow_edit(&grouped, &mut ungrouped);
        assert_eq!(
            ungrouped.bindings[0].node(),
            Some(&NodePath::root(NodeId(44)))
        );
    }

    #[test]
    fn learning_binds_presses_as_toggles_on_bools_except_held_inputs() {
        let p = Project::default();
        let key = ac_core::Key::from_char('t').unwrap();
        let ev = [InputEvent::Control(ac_core::Control::Key(key), 1.0)];
        let mut l = Learn::new(
            LearnTarget::Input {
                node: NodePath::root(NodeId(1)),
                socket: "value".into(),
            },
            "Toggle · value".into(),
            &p,
        );
        let src = l.feed(&ev).unwrap();
        assert_eq!(
            l.binding(src.clone(), Some(SignalType::Bool)).mode,
            ac_core::GateMode::Toggle
        );
        let held = Learn::new(
            LearnTarget::Input {
                node: NodePath::root(NodeId(1)),
                socket: "pressed".into(),
            },
            String::new(),
            &p,
        );
        assert_eq!(
            held.binding(src.clone(), Some(SignalType::Bool)).mode,
            ac_core::GateMode::Momentary
        );
        assert_eq!(
            l.binding(src, Some(SignalType::Float)).mode,
            ac_core::GateMode::Momentary
        );
    }

    #[test]
    fn osc_events_are_named_from_addresses_the_project_mentions() {
        let mut p = Project::default();
        p.graph.nodes.push(
            NodeInstance::new(NodeId(1), "osc_in")
                .with_config(serde_json::json!({"address": "/mix/fader"})),
        );
        let mut l = Learn::new(LearnTarget::Transport(TransportAction::Play), String::new(), &p);
        let h = ac_core::osc_address_hash("/mix/fader");
        let osc = ac_core::Control::Osc { address: h, arg: 0 };
        let got = l.feed(&[
            InputEvent::Control(osc, 0.0),
            InputEvent::Control(osc, 0.5),
        ]);
        assert_eq!(
            got,
            Some(Source::Osc {
                address: "/mix/fader".into(),
                arg: 0
            })
        );
    }

    #[test]
    fn editor_keys_map_to_computer_keys() {
        assert_eq!(key_from_egui(egui::Key::A), ac_core::Key::from_name("A"));
        assert_eq!(key_from_egui(egui::Key::Num7), ac_core::Key::from_name("7"));
        assert_eq!(key_from_egui(egui::Key::Space), ac_core::Key::from_name("Space"));
        assert_eq!(key_from_egui(egui::Key::ArrowUp), ac_core::Key::from_name("Up"));
        assert_eq!(key_from_egui(egui::Key::F5), ac_core::Key::from_name("F5"));
    }
}
