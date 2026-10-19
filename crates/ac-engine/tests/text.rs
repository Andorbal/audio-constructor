//! `Text` signals end to end: typed into a node, wired between nodes, and
//! fed through a group input, with the OSC nodes as the consumers.

mod common;

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use common::P;
use serde_json::json;

fn registry() -> Arc<Registry> {
    let mut r = common::registry();
    ac_nodes::register_all(&mut r);
    Arc::new(r)
}

fn osc(address: &str, arg: u8) -> Control {
    Control::Osc {
        address: osc_address_hash(address),
        arg,
    }
}

/// `(frame, address, args)` of every OSC message sent.
fn sent(r: &Rendered) -> Vec<(u64, String, Vec<f32>)> {
    r.osc
        .iter()
        .map(|(at, m)| (*at, m.address().to_string(), m.args().to_vec()))
        .collect()
}

fn render_with(p: &Project, block: usize, controls: Vec<(u64, Control, f32)>) -> Rendered {
    render(
        p,
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(block),
            controls,
            out_channels: Some(1),
            ..Default::default()
        },
    )
}

/// An OSC In listening on `address` feeding an OSC Out that sends to
/// `out_address`, only on triggers from the In's `received`.
fn relay(id: u64, address: &str, out_address: &str) -> P {
    P::new()
        .node(id, "osc_in")
        .param(id, "address", address)
        .node(id + 1, "osc_out")
        .param(id + 1, "address", out_address)
        .param(id + 1, "on_change", false)
        .wire(id, "value", id + 1, "value")
        .wire(id, "received", id + 1, "send")
}

#[test]
fn typed_addresses_select_what_is_received_and_sent() {
    let mut p = relay(1, "/xy", "/echo").build();
    p.settings.length = 0.01;
    let events = vec![
        (10, osc("/xy", 0), 0.25),
        (40, osc("/other", 0), 0.9),
        (100, osc("/xy", 0), 0.75),
    ];
    for block in [1, 64, 480] {
        let r = render_with(&p, block, events.clone());
        assert_eq!(
            sent(&r),
            [
                (10, "/echo".to_string(), vec![0.25]),
                (100, "/echo".to_string(), vec![0.75]),
            ],
            "block {block}"
        );
    }
    // A bad typed address receives nothing; a bad out address sends nothing.
    let mut bad = relay(1, "no slash", "/echo").build();
    bad.settings.length = 0.01;
    assert!(sent(&render_with(&bad, 64, events.clone())).is_empty());
    let mut bad = relay(1, "/xy", "/no space allowed").build();
    bad.settings.length = 0.01;
    assert!(sent(&render_with(&bad, 64, events)).is_empty());
}

#[test]
fn text_wires_carry_formatted_addresses_and_parsed_numbers() {
    // Constant 3 → "/fader/{}" → OSC In's address; its value → OSC Out,
    // whose address comes from "/out/{}" of the same constant. The parsed
    // number of "/fader/3" goes out as a second argument.
    let mut p = P::new()
        .node_with(NodeInstance::new(NodeId(1), "constant").with_config(json!({"type": "int"})))
        .param(1, "value", 3)
        .node(2, "number_to_text")
        .param(2, "template", "/fader/{}")
        .node(3, "osc_in")
        .node(4, "number_to_text")
        .param(4, "template", "/out/{}")
        .node_with(NodeInstance::new(NodeId(5), "osc_out").with_config(json!({"args": 2})))
        .param(5, "on_change", false)
        .node(6, "text_to_number")
        .param(6, "pattern", "/fader/{}")
        .wire(1, "out", 2, "value")
        .wire(2, "out", 3, "address")
        .wire(1, "out", 4, "value")
        .wire(4, "out", 5, "address")
        .wire(2, "out", 6, "text")
        .wire(6, "value", 5, "value_2")
        .wire(3, "value", 5, "value")
        .wire(3, "received", 5, "send")
        .build();
    p.settings.length = 0.01;
    let events = vec![(5, osc("/fader/3", 0), 0.5), (50, osc("/fader/2", 0), 0.9)];
    for block in [1, 64, 480] {
        let r = render_with(&p, block, events.clone());
        assert_eq!(r.report.errors().count(), 0, "{:?}", r.report.diagnostics);
        assert_eq!(
            sent(&r),
            [(5, "/out/3".to_string(), vec![0.5, 3.0])],
            "block {block}"
        );
    }
}

#[test]
fn a_text_group_input_gives_each_instance_its_own_address() {
    let mut def = GroupDef::new("Listener");
    def.graph.nodes.push(NodeInstance::new(NodeId(1), "osc_in"));
    def.inputs.push(GroupInput {
        name: "address".into(),
        ty: SignalType::Text,
        unit: Unit::Generic,
        default: Some(Value::Text("/default".into())),
        range: None,
        targets: vec![Endpoint::new(NodeId(1), "address")],
    });
    def.outputs.push(GroupOutput {
        name: "value".into(),
        ty: SignalType::Float,
        source: Endpoint::new(NodeId(1), "value"),
    });
    def.outputs.push(GroupOutput {
        name: "received".into(),
        ty: SignalType::Trigger,
        source: Endpoint::new(NodeId(1), "received"),
    });
    let instance = |id: u64| {
        NodeInstance::new(NodeId(id), GROUP_KIND).with_config(json!({ "def": "listener" }))
    };
    let mut p = P::new()
        .node_with(instance(1))
        .param(1, "address", "/a")
        .node_with(instance(2))
        .param(2, "address", "/b")
        .node_with(instance(3))
        .node(4, "osc_out")
        .param(4, "address", "/from_a")
        .param(4, "on_change", false)
        .node(5, "osc_out")
        .param(5, "address", "/from_b")
        .param(5, "on_change", false)
        .node(6, "osc_out")
        .param(6, "address", "/from_default")
        .param(6, "on_change", false)
        .wire(1, "value", 4, "value")
        .wire(1, "received", 4, "send")
        .wire(2, "value", 5, "value")
        .wire(2, "received", 5, "send")
        .wire(3, "value", 6, "value")
        .wire(3, "received", 6, "send")
        .build();
    p.group_defs.insert(GroupId::from("listener"), def);
    p.settings.length = 0.01;
    let events = vec![
        (10, osc("/a", 0), 0.1),
        (20, osc("/b", 0), 0.2),
        (30, osc("/default", 0), 0.3),
    ];
    let r = render_with(&p, 64, events);
    assert_eq!(r.report.errors().count(), 0, "{:?}", r.report.diagnostics);
    assert_eq!(
        sent(&r),
        [
            (10, "/from_a".to_string(), vec![0.1]),
            (20, "/from_b".to_string(), vec![0.2]),
            (30, "/from_default".to_string(), vec![0.3]),
        ]
    );
}

#[test]
fn a_text_input_has_no_param_slot_and_recompiles_cleanly() {
    let mut p = relay(1, "/xy", "/echo").build();
    p.settings.length = 0.01;
    let compiled = compile(
        &p,
        &registry(),
        &NoResources,
        &CompileOptions::default(),
        None,
        &BakeStore::default(),
    );
    assert_eq!(
        compiled.report.errors().count(),
        0,
        "{:?}",
        compiled.report.diagnostics
    );
    assert!(
        !compiled
            .params
            .contains_key(&(common::path(1), "address".to_string())),
        "typed text isn't a parameter slot"
    );
    assert!(
        compiled
            .params
            .contains_key(&(common::path(2), "on_change".to_string()))
    );
}
