//! Unit tests for the pure editing operations.

use std::collections::BTreeSet;

use ac_core::{
    Endpoint, GROUP_KIND, GroupId, IdGen, NodeId, Project, REROUTE_KIND, Registry, SignalType, Wire,
};

use super::group::{instance_paths, make_group, ungroup};
use super::*;

fn reg() -> Registry {
    ac_nodes::registry()
}

fn ep(id: NodeId, s: &str) -> Endpoint {
    Endpoint::new(id, s)
}

fn has_wire(g: &ac_core::Graph, from: Endpoint, to: Endpoint) -> bool {
    g.wires.contains(&Wire::new(from, to))
}

/// constant → gain1 → gain2 → audio_out
struct Chain {
    project: Project,
    c: NodeId,
    g1: NodeId,
    g2: NodeId,
    out: NodeId,
}

fn chain() -> Chain {
    let r = reg();
    let mut project = Project::default();
    let mut ids = IdGen::new(1);
    let (c, g1, g2, out) = edit_scope(&mut project, None, &r, &mut ids, |g, ctx| {
        let c = add_node(g, ctx, "constant", ac_core::empty_config(), [0.0, 0.0]);
        let g1 = add_node(g, ctx, "gain", ac_core::empty_config(), [200.0, 0.0]);
        let g2 = add_node(g, ctx, "gain", ac_core::empty_config(), [400.0, 0.0]);
        let out = add_node(g, ctx, "audio_out", ac_core::empty_config(), [600.0, 0.0]);
        connect(g, ctx, ep(c, "out"), ep(g1, "in")).unwrap();
        connect(g, ctx, ep(g1, "out"), ep(g2, "in")).unwrap();
        connect(g, ctx, ep(g2, "out"), ep(out, "in")).unwrap();
        (c, g1, g2, out)
    });
    Chain {
        project,
        c,
        g1,
        g2,
        out,
    }
}

fn root<R>(project: &mut Project, f: impl FnOnce(&mut ac_core::Graph, &mut Ctx) -> R) -> R {
    let r = reg();
    let mut ids = IdGen::new(99);
    edit_scope(project, None, &r, &mut ids, f)
}

#[test]
fn connect_checks_types_and_replaces_the_input_wire() {
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        let b = add_node(g, ctx, "button", ac_core::empty_config(), [0.0, 200.0]);
        // Trigger → Float is not an implicit conversion.
        assert!(matches!(
            connect(g, ctx, ep(b, "trigger"), ep(ch.g1, "gain")),
            Err(ConnectError::Incompatible(
                SignalType::Trigger,
                SignalType::Float
            ))
        ));
        // Bool → Float is.
        connect(g, ctx, ep(b, "gate"), ep(ch.g1, "gain")).unwrap();
        // Connecting into a wired input replaces its wire.
        connect(g, ctx, ep(ch.c, "out"), ep(ch.g2, "in")).unwrap();
        assert!(!has_wire(g, ep(ch.g1, "out"), ep(ch.g2, "in")));
        assert_eq!(g.wires_to(ch.g2).count(), 1);
        assert_eq!(
            connect(g, ctx, ep(ch.g1, "out"), ep(ch.g1, "in")),
            Err(ConnectError::SameNode)
        );
        assert_eq!(
            connect(g, ctx, ep(ch.g1, "nope"), ep(ch.out, "in")),
            Err(ConnectError::NoSuchOutput)
        );
    });
}

#[test]
fn delete_with_reconnect_keeps_the_chain() {
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        let removed = delete_reconnect(g, ctx, &[ch.g1].into());
        assert_eq!(removed.len(), 1);
        assert!(has_wire(g, ep(ch.c, "out"), ep(ch.g2, "in")));
        assert_eq!(g.wires.len(), 2);
    });
    // Deleting a whole selected chain collapses it into one wire.
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        delete_reconnect(g, ctx, &[ch.g1, ch.g2].into());
        assert_eq!(g.wires, vec![Wire::new(ep(ch.c, "out"), ep(ch.out, "in"))]);
    });
}

#[test]
fn delete_with_reconnect_skips_incompatible_pairs() {
    let mut project = Project::default();
    root(&mut project, |g, ctx| {
        let b = add_node(g, ctx, "button", ac_core::empty_config(), [0.0, 0.0]);
        let t = add_node(g, ctx, "toggle", ac_core::empty_config(), [200.0, 0.0]);
        let gain = add_node(g, ctx, "gain", ac_core::empty_config(), [400.0, 0.0]);
        connect(g, ctx, ep(b, "trigger"), ep(t, "flip")).unwrap();
        connect(g, ctx, ep(t, "out"), ep(gain, "gain")).unwrap();
        delete_reconnect(g, ctx, &[t].into());
        // A trigger can't feed a float input, so nothing is reconnected.
        assert!(g.wires.is_empty());
    });
}

#[test]
fn plain_delete_removes_wires() {
    let mut ch = chain();
    root(&mut ch.project, |g, _| {
        delete_nodes(g, &[ch.g1].into());
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.wires.len(), 1);
        // Virtual ids are ignored.
        delete_nodes(g, &[GROUP_IN].into());
        assert_eq!(g.nodes.len(), 3);
    });
}

#[test]
fn duplicate_with_and_without_incoming_wires() {
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        let sel: BTreeSet<_> = [ch.g1, ch.g2].into();
        let new = duplicate(g, ctx, &sel, false, [20.0, 20.0]);
        assert_eq!(new.len(), 2);
        assert_eq!(g.nodes.len(), 6);
        assert!(has_wire(g, ep(new[0], "out"), ep(new[1], "in")));
        assert!(g.wires_to(new[0]).next().is_none());
        assert_eq!(g.node(new[0]).unwrap().ui.pos, [220.0, 20.0]);
        // The originals keep their wiring.
        assert!(has_wire(g, ep(ch.c, "out"), ep(ch.g1, "in")));
    });
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        let sel: BTreeSet<_> = [ch.g1, ch.g2].into();
        let new = duplicate(g, ctx, &sel, true, [20.0, 20.0]);
        assert!(has_wire(g, ep(ch.c, "out"), ep(new[0], "in")));
        assert!(has_wire(g, ep(new[0], "out"), ep(new[1], "in")));
        // Outgoing wires are not duplicated (an input takes one wire).
        assert!(g.wires_from(new[1]).next().is_none());
        assert!(has_wire(g, ep(ch.c, "out"), ep(ch.g1, "in")));
    });
}

#[test]
fn insert_on_wire_splices_when_types_fit() {
    let mut project = Project::default();
    root(&mut project, |g, ctx| {
        let c = add_node(g, ctx, "constant", ac_core::empty_config(), [0.0, 0.0]);
        let out = add_node(g, ctx, "audio_out", ac_core::empty_config(), [400.0, 0.0]);
        connect(g, ctx, ep(c, "out"), ep(out, "in")).unwrap();
        let wire = g.wires[0].clone();
        let gain = add_node(g, ctx, "gain", ac_core::empty_config(), [200.0, 0.0]);
        assert!(insert_on_wire(g, ctx, gain, &wire));
        assert!(has_wire(g, ep(c, "out"), ep(gain, "in")));
        assert!(has_wire(g, ep(gain, "out"), ep(out, "in")));
        assert_eq!(g.wires.len(), 2);
        // A node with nothing fitting is left alone.
        let wire = g.wires[1].clone();
        let comment = add_node(g, ctx, "comment", ac_core::empty_config(), [0.0, 0.0]);
        assert!(!insert_on_wire(g, ctx, comment, &wire));
        assert_eq!(g.wires.len(), 2);
    });
}

#[test]
fn reroutes_split_wires_and_adopt_their_type() {
    let mut ch = chain();
    root(&mut ch.project, |g, ctx| {
        let wire = Wire::new(ep(ch.g1, "out"), ep(ch.g2, "in"));
        let new = add_reroutes(g, ctx, &[(wire.clone(), [300.0, 10.0])]);
        assert_eq!(new.len(), 1);
        let r = new[0];
        assert_eq!(g.node(r).unwrap().kind, REROUTE_KIND);
        assert!(has_wire(g, ep(ch.g1, "out"), ep(r, "in")));
        assert!(has_wire(g, ep(r, "out"), ep(ch.g2, "in")));
        assert!(!g.wires.contains(&wire));
        assert_eq!(ctx.out_type(g, &ep(r, "out")), Some(SignalType::Audio));
        assert_eq!(resolve_source(g, &ep(r, "out")), ep(ch.g1, "out"));
    });
}

#[test]
fn paste_remaps_ids_and_places_at_the_target() {
    let mut ch = chain();
    let defs = ch.project.group_defs.clone();
    let text = copy(&ch.project.graph, &defs, &[ch.g1, ch.g2].into());
    root(&mut ch.project, |g, ctx| {
        let new = paste(g, ctx, &text, [1000.0, 500.0]).unwrap();
        assert_eq!(new.len(), 2);
        assert_eq!(g.node(new[0]).unwrap().ui.pos, [1000.0, 500.0]);
        assert_eq!(g.node(new[1]).unwrap().ui.pos, [1200.0, 500.0]);
        assert!(has_wire(g, ep(new[0], "out"), ep(new[1], "in")));
        assert!(paste(g, ctx, "not a patch", [0.0, 0.0]).is_err());
    });
}

#[test]
fn group_turns_boundary_wires_into_interface_sockets() {
    let mut ch = chain();
    // The constant also feeds g2's gain, so one outside source feeds two
    // inner sockets of different types.
    root(&mut ch.project, |g, ctx| {
        connect(g, ctx, ep(ch.c, "out"), ep(ch.g2, "gain")).unwrap();
    });
    let (inst, def_id) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1, ch.g2].into()).unwrap()
    });
    let p = &ch.project;
    let def = &p.group_defs[&def_id];
    assert_eq!(def.graph.nodes.len(), 2);
    assert_eq!(
        def.graph.wires,
        vec![Wire::new(ep(ch.g1, "out"), ep(ch.g2, "in"))]
    );
    // One input per outside source.
    assert_eq!(def.inputs.len(), 1);
    assert_eq!(def.inputs[0].targets.len(), 2);
    assert_eq!(def.outputs.len(), 1);
    assert_eq!(def.outputs[0].source, ep(ch.g2, "out"));
    assert_eq!(def.outputs[0].ty, SignalType::Audio);
    // The instance replaces the selection and is wired in its place.
    let g = &p.graph;
    assert_eq!(g.nodes.len(), 3);
    assert_eq!(g.node(inst).unwrap().kind, GROUP_KIND);
    let in_name = &def.inputs[0].name;
    let out_name = &def.outputs[0].name;
    assert!(has_wire(g, ep(ch.c, "out"), ep(inst, in_name)));
    assert!(has_wire(g, ep(inst, out_name), ep(ch.out, "in")));
    assert_eq!(g.wires.len(), 2);
    assert_eq!(
        instance_paths(p, &def_id),
        vec![ac_core::NodePath::root(inst)]
    );
}

#[test]
fn ungroup_restores_the_wiring_and_drops_the_unused_definition() {
    let mut ch = chain();
    let (inst, def_id) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1, ch.g2].into()).unwrap()
    });
    // A value set on the instance's (later unwired) input carries inward.
    root(&mut ch.project, |g, _| {
        let w: Vec<Wire> = g.wires_to(inst).cloned().collect();
        for w in w {
            g.disconnect_input(&w.to);
            g.node_mut(inst)
                .unwrap()
                .params
                .insert(w.to.socket.clone(), 0.25.into());
        }
    });
    let new = root(&mut ch.project, |g, ctx| ungroup(g, ctx, inst).unwrap());
    let g = &ch.project.graph;
    assert_eq!(new.len(), 2);
    assert_eq!(g.nodes.len(), 4);
    assert!(g.node(inst).is_none());
    assert!(!ch.project.group_defs.contains_key(&def_id));
    // Fresh ids, the same chain: new0 → new1 → out.
    let first = g.node(new[0]).unwrap();
    assert_eq!(first.params.get("in"), Some(&0.25.into()));
    assert!(has_wire(g, ep(new[0], "out"), ep(new[1], "in")));
    assert!(has_wire(g, ep(new[1], "out"), ep(ch.out, "in")));
}

#[test]
fn group_and_ungroup_round_trip_through_the_interface() {
    let mut ch = chain();
    let before = ch.project.graph.wires.len();
    let (inst, _) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1].into()).unwrap()
    });
    root(&mut ch.project, |g, ctx| ungroup(g, ctx, inst).unwrap());
    let g = &ch.project.graph;
    assert_eq!(g.nodes.len(), 4);
    assert_eq!(g.wires.len(), before);
    let gain = g
        .nodes
        .iter()
        .find(|n| n.kind == "gain" && n.id != ch.g2)
        .unwrap();
    assert!(has_wire(g, ep(ch.c, "out"), ep(gain.id, "in")));
    assert!(has_wire(g, ep(gain.id, "out"), ep(ch.g2, "in")));
}

#[test]
fn editing_inside_a_group_sees_the_interface_as_wires() {
    let mut ch = chain();
    let (_, def_id) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1, ch.g2].into()).unwrap()
    });
    let r = reg();
    let mut ids = IdGen::new(5);
    // Inside: the interface input feeds g1, g2 feeds the interface output.
    edit_scope(&mut ch.project, Some(&def_id), &r, &mut ids, |g, ctx| {
        assert!(
            g.wires
                .iter()
                .any(|w| w.from.node == GROUP_IN && w.to == ep(ch.g1, "in"))
        );
        assert!(
            g.wires
                .iter()
                .any(|w| w.to.node == GROUP_OUT && w.from == ep(ch.g2, "out"))
        );
        // Deleting g2 with reconnect routes g1 straight to the interface output.
        delete_reconnect(g, ctx, &[ch.g2].into());
        // Expose g1's gain as a new group input.
        let name = expose_input(g, ctx, &ep(ch.g1, "gain")).unwrap();
        assert_eq!(name, "gain");
    });
    let def = &ch.project.group_defs[&def_id];
    assert_eq!(def.outputs[0].source, ep(ch.g1, "out"));
    assert_eq!(def.inputs.len(), 2);
    assert_eq!(def.inputs[1].targets, vec![ep(ch.g1, "gain")]);
    assert_eq!(def.inputs[1].ty, SignalType::Float);
    assert!(def.graph.wires.is_empty());
}

#[test]
fn nested_grouping_rewires_the_outer_interface() {
    let mut ch = chain();
    let (outer_inst, outer) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1, ch.g2].into()).unwrap()
    });
    let r = reg();
    let mut ids = IdGen::new(5);
    let (inner_inst, inner) = edit_scope(&mut ch.project, Some(&outer), &r, &mut ids, |g, ctx| {
        make_group(g, ctx, &[ch.g1].into()).unwrap()
    });
    let def = &ch.project.group_defs[&outer];
    assert_eq!(def.inputs[0].targets.len(), 1);
    assert_eq!(def.inputs[0].targets[0].node, inner_inst);
    let paths = instance_paths(&ch.project, &inner);
    assert_eq!(paths, vec![ac_core::NodePath(vec![outer_inst, inner_inst])]);
    // A group can't be pasted inside itself.
    let text = copy(
        &ch.project.graph,
        &ch.project.group_defs,
        &[outer_inst].into(),
    );
    let err = edit_scope(&mut ch.project, Some(&inner), &r, &mut ids, |g, ctx| {
        paste(g, ctx, &text, [0.0, 0.0])
    });
    assert!(err.is_err());
}

#[test]
fn interface_sockets_can_be_added_by_dropping_on_the_empty_socket() {
    let mut ch = chain();
    let (_, def_id) = root(&mut ch.project, |g, ctx| {
        make_group(g, ctx, &[ch.g1].into()).unwrap()
    });
    let r = reg();
    let mut ids = IdGen::new(5);
    edit_scope(&mut ch.project, Some(&def_id), &r, &mut ids, |g, ctx| {
        connect(g, ctx, ep(GROUP_IN, NEW_SOCKET), ep(ch.g1, "gain")).unwrap();
        connect(g, ctx, ep(ch.g1, "out"), ep(GROUP_OUT, NEW_SOCKET)).unwrap();
    });
    let def = &ch.project.group_defs[&def_id];
    assert_eq!(def.inputs.len(), 2);
    assert_eq!(def.outputs.len(), 2);
    assert_eq!(def.outputs[1].name, "out_2");
}

#[test]
fn missing_nodes_keep_their_wires_and_refuse_new_ones() {
    let mut project = Project::default();
    root(&mut project, |g, ctx| {
        let m = add_node(g, ctx, "quantum_flux", ac_core::empty_config(), [0.0, 0.0]);
        let gain = add_node(g, ctx, "gain", ac_core::empty_config(), [200.0, 0.0]);
        g.connect(ep(m, "warp"), ep(gain, "in"));
        let s = ctx.sockets(g, m);
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "warp");
        assert_eq!(
            connect(g, ctx, ep(gain, "out"), ep(m, "x")),
            Err(ConnectError::MissingNode)
        );
        assert!(matches!(
            classify(g.node(m).unwrap(), ctx.registry, ctx.defs),
            NodeClass::Missing(_)
        ));
    });
    let missing_def = ac_core::NodeInstance::new(NodeId(9), GROUP_KIND)
        .with_config(serde_json::json!({"def": "nope"}));
    assert!(matches!(
        classify(&missing_def, &reg(), &Default::default()),
        NodeClass::Missing(_)
    ));
    let _ = GroupId::from("x");
}

#[test]
fn linked_follows_wires_both_ways() {
    let ch = chain();
    let all = linked(&ch.project.graph, &[ch.g2].into());
    assert_eq!(all.len(), 4);
}

#[test]
fn a_new_voices_node_has_a_playable_body_and_groups_toggle_polyphony() {
    let mut p = Project::default();
    let v = root(&mut p, |g, ctx| {
        let v = super::group::add_voices(g, ctx, [0.0, 0.0]);
        let s = ctx.sockets(g, v);
        assert_eq!(s.inputs[0].name, "midi");
        assert_eq!(s.outputs[0].name, "out");
        assert!(matches!(
            classify(g.node(v).unwrap(), ctx.registry, ctx.defs),
            NodeClass::Group(_)
        ));
        v
    });
    assert_eq!(p.group_defs.len(), 1);
    // The body compiles and makes sound from MIDI.
    p.graph
        .nodes
        .push(ac_core::NodeInstance::new(NodeId(500), "midi_in"));
    p.graph
        .nodes
        .push(ac_core::NodeInstance::new(NodeId(501), "audio_out"));
    p.graph.connect(ep(NodeId(500), "midi"), ep(v, "midi"));
    p.graph.connect(ep(v, "out"), ep(NodeId(501), "in"));
    let r = ac_engine::render(
        &p,
        std::sync::Arc::new(reg()),
        std::sync::Arc::new(ac_core::NoResources),
        &ac_engine::RenderOptions {
            to: Some(0.2),
            midi: vec![(
                0,
                ac_core::MidiMsg::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            )],
            ..Default::default()
        },
    );
    assert!(!r.report.has_errors(), "{}", r.report);
    assert!(r.channels[0].iter().any(|s| s.abs() > 0.05));
    // Back to a plain group: `midi` goes away with its wire.
    root(&mut p, |g, ctx| {
        super::group::set_polyphonic(g, v, false);
        assert_eq!(g.node(v).unwrap().kind, GROUP_KIND);
        assert!(ctx.sockets(g, v).input("midi").is_none());
        assert!(g.wire_into(&ep(v, "midi")).is_none());
        super::group::set_polyphonic(g, v, true);
        assert_eq!(g.node(v).unwrap().kind, ac_core::VOICES_KIND);
    });
}

#[test]
fn hidden_sockets_are_not_shown() {
    let mut p = Project::default();
    root(&mut p, |g, ctx| {
        let v = add_node(
            g,
            ctx,
            ac_core::VOICE_KIND,
            ac_core::empty_config(),
            [0.0, 0.0],
        );
        let s = ctx.sockets(g, v);
        assert!(s.input("voice_midi").is_none());
        assert!(s.input("bend_range").is_some());
    });
}

#[test]
fn midi_wires_into_a_polyphonic_block_stay() {
    // The Karplus-Strong String block as the palette inserts it: a Voices
    // instance. MIDI from a Keyboard wires into its `midi` input, which
    // isn't part of the block's interface, and the wire outlives the
    // clean-up after every edit (#21).
    let text = include_str!("../../../../blocks/karplus-strong-string.acblock");
    let block = ac_core::Block::from_json(text).unwrap();
    let source = ac_core::BlockSource::new(ac_core::Origin::Shipped, "karplus-strong-string");
    let id = GroupId(ac_core::slug(&block.def.name));
    let clip = block.instance(&id, &source, true).to_json();
    let r = reg();
    let mut doc =
        crate::document::Document::new(Project::default(), None, std::path::PathBuf::from("."), 1);
    let (v, k) = doc.edit(&r, "add", |g, ctx| {
        let v = paste(g, ctx, &clip, [0.0, 0.0]).unwrap()[0];
        let k = add_node(g, ctx, "keyboard", ac_core::empty_config(), [-300.0, 0.0]);
        let s = ctx.sockets(g, v);
        assert_eq!(s.inputs[0].name, "midi");
        assert_eq!(s.inputs[0].ty, SignalType::Midi);
        (v, k)
    });
    let made = doc.edit(&r, "wire", |g, ctx| {
        connect(g, ctx, ep(k, "midi"), ep(v, "midi"))
    });
    assert_eq!(made, Ok(()));
    assert!(has_wire(doc.graph(), ep(k, "midi"), ep(v, "midi")));
    // A later edit elsewhere keeps it.
    doc.edit(&r, "add", |g, ctx| {
        add_node(g, ctx, "gain", ac_core::empty_config(), [300.0, 0.0])
    });
    assert!(has_wire(doc.graph(), ep(k, "midi"), ep(v, "midi")));
    // Played once instead, the group has no `midi` input, and the wire goes.
    doc.edit(&r, "mono", |g, _| group::set_polyphonic(g, v, false));
    doc.edit(&r, "wire", |g, ctx| {
        let _ = connect(g, ctx, ep(k, "midi"), ep(v, "midi"));
    });
    assert!(!has_wire(doc.graph(), ep(k, "midi"), ep(v, "midi")));
}
