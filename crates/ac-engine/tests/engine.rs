//! Compiler and engine tests: conversions, channels, cycles, Feedback,
//! groups, reroutes, carry-over, parameters, transport, determinism, and
//! real-time hygiene (the allocation guard aborts on any allocation in
//! `Processor::process`).

mod common;

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use common::{P, path, registry};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn engine(project: Project, block: usize) -> (Engine, CompileReport) {
    Engine::new(
        project,
        Arc::new(registry()),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: 48_000.0,
            max_block: block,
            socket_telemetry: true,
            offline: false,
        },
    )
}

fn offline(project: &Project, block: usize, seconds: f64) -> Rendered {
    render(
        project,
        Arc::new(registry()),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(block),
            to: Some(seconds),
            ..Default::default()
        },
    )
}

/// src → sum.in (Float → Audio at sample rate) → out (mono → stereo upmix).
fn chain() -> Project {
    P::new()
        .node(1, "t_src")
        .node(2, "t_sum")
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build()
}

#[test]
fn chain_renders_expected_samples_with_upmix() {
    let r = offline(&chain(), 64, 0.01);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels.len(), 2);
    for i in [0usize, 1, 63, 64, 200, 479] {
        assert_eq!(r.channels[0][i], i as f32);
        assert_eq!(r.channels[1][i], i as f32);
    }
}

#[test]
fn block_rate_float_into_audio_is_materialized() {
    let p = P::new()
        .node(1, "t_block")
        .param(1, "value", 0.5)
        .node(2, "t_sum")
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    let r = offline(&p, 32, 0.005);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert!(r.channels[0].iter().all(|&v| v == 0.5));
}

#[test]
fn int_and_bool_feed_float_inputs() {
    let p = P::new()
        .node(1, "t_int")
        .param(1, "value", 3)
        .node(2, "t_add")
        .param(2, "b", 0.25)
        .node(4, "t_sum")
        .node(5, "t_out")
        .wire(1, "out", 2, "a")
        .wire(2, "out", 4, "amount")
        .wire(4, "out", 5, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert!(r.channels[0].iter().all(|&v| v == 3.25));
}

#[test]
fn audio_feeds_float_inputs_from_channel_zero() {
    let p = P::new()
        .node(1, "t_src")
        .node(2, "t_sum")
        .node(3, "t_add")
        .node(4, "t_sum")
        .node(5, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "a")
        .wire(3, "out", 4, "amount")
        .wire(4, "out", 5, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels[0][17], 17.0);
}

#[test]
fn channel_mismatch_is_reported_not_fatal() {
    let p = P::new()
        .node(1, "t_quad")
        .node(2, "t_out")
        .wire(1, "out", 2, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.has_errors());
    assert!(r.report.to_string().contains("channel mismatch"));
    assert!(r.channels[0].iter().all(|&v| v == 0.0));
}

#[test]
fn type_mismatch_is_reported_and_the_rest_runs() {
    let p = P::new()
        .node(1, "t_starts")
        .node(2, "t_sum")
        .node(3, "t_out")
        .node(4, "t_src")
        // Float → Trigger has no implicit conversion.
        .wire(4, "out", 1, "start")
        .wire(4, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.has_errors());
    assert!(r.report.to_string().contains("can't connect Float"));
    assert_eq!(r.channels[0][5], 5.0);
}

#[test]
fn cycles_without_feedback_are_rejected_and_named() {
    let p = P::new()
        .node(1, "t_add")
        .node(2, "t_add")
        .wire(1, "out", 2, "a")
        .wire(2, "out", 1, "a")
        // An independent chain keeps working.
        .node(3, "t_src")
        .node(4, "t_sum")
        .node(5, "t_out")
        .wire(3, "out", 4, "in")
        .wire(4, "out", 5, "in")
        // Downstream of the cycle: still runs, with the input unwired.
        .node(6, "t_add")
        .wire(2, "out", 6, "a")
        .build();
    let r = offline(&p, 32, 0.002);
    let errors: Vec<_> = r.report.errors().collect();
    assert_eq!(errors.len(), 2, "{}", r.report);
    assert!(errors[0].message.contains("cycle without a Feedback node"));
    assert!(errors.iter().all(|e| e.node != Some(path(6))));
    assert_eq!(r.channels[0][9], 9.0);
}

#[test]
fn feedback_closes_a_loop_with_one_block_of_delay() {
    // y = x + fb(y): with x = constant 1, block k outputs k+1.
    let p = P::new()
        .node(1, "t_block")
        .param(1, "value", 1.0)
        .node(2, "t_sum")
        .node(3, "t_fb")
        .node(4, "t_out")
        .wire(1, "out", 2, "in")
        .wire(3, "out", 2, "b")
        .wire(2, "out", 3, "in")
        .wire(2, "out", 4, "in")
        .build();
    let r = offline(&p, 16, 64.0 / 48_000.0);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels[0][0], 1.0);
    assert_eq!(r.channels[0][16], 2.0);
    assert_eq!(r.channels[0][63], 4.0);
}

fn group_def() -> GroupDef {
    let mut def = GroupDef::new("Offset");
    def.graph.nodes.push(NodeInstance::new(NodeId(10), "t_sum"));
    def.inputs.push(GroupInput {
        name: "in".into(),
        ty: SignalType::Audio,
        unit: Unit::Generic,
        default: None,
        range: None,
        targets: vec![Endpoint::new(NodeId(10), "in")],
    });
    def.inputs.push(GroupInput {
        name: "amount".into(),
        ty: SignalType::Float,
        unit: Unit::Generic,
        default: Some(Value::Float(0.125)),
        range: None,
        targets: vec![Endpoint::new(NodeId(10), "amount")],
    });
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(10), "out"),
    });
    def
}

fn group_node(id: u64, def: &str) -> NodeInstance {
    NodeInstance::new(NodeId(id), GROUP_KIND).with_config(json!({ "def": def }))
}

#[test]
fn groups_flatten_with_exposed_sockets_and_defaults() {
    let mut p = P::new()
        .node(1, "t_src")
        .node_with(group_node(2, "off"))
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.group_defs.insert(GroupId::from("off"), group_def());
    let r = offline(&p, 32, 0.002);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels[0][4], 4.125);
    // Instance param overrides the definition default.
    p.graph
        .node_mut(NodeId(2))
        .unwrap()
        .params
        .insert("amount".into(), Value::Float(0.5));
    let r = offline(&p, 32, 0.002);
    assert_eq!(r.channels[0][4], 4.5);
}

#[test]
fn nested_groups_and_inner_paths() {
    // outer def contains an instance of "off" with amount 0.25.
    let mut outer = GroupDef::new("Outer");
    outer
        .graph
        .nodes
        .push(group_node(20, "off").with_param("amount", 0.25));
    outer.inputs.push(GroupInput {
        name: "in".into(),
        ty: SignalType::Audio,
        unit: Unit::Generic,
        default: None,
        range: None,
        targets: vec![Endpoint::new(NodeId(20), "in")],
    });
    outer.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(20), "out"),
    });
    let mut p = P::new()
        .node(1, "t_src")
        .node_with(group_node(2, "outer"))
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.group_defs.insert(GroupId::from("off"), group_def());
    p.group_defs.insert(GroupId::from("outer"), outer);
    let (e, report) = engine(p, 32);
    assert!(report.diagnostics.is_empty(), "{report}");
    // Inner node 0xa inside instance 0x14 inside root instance 2.
    let paths = e.handle.telemetry().layout_paths();
    assert!(paths.contains(&"2/14/a".to_string()), "{paths:?}");
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    let out = host.run(64, 32);
    assert_eq!(out[0][10], 10.25);
}

#[test]
fn self_containing_and_missing_groups_are_errors() {
    let mut def = GroupDef::new("Loop");
    def.graph.nodes.push(group_node(5, "loop"));
    let mut p = P::new()
        .node_with(group_node(1, "loop"))
        .node_with(group_node(2, "nope"))
        .build();
    p.group_defs.insert(GroupId::from("loop"), def);
    let r = offline(&p, 32, 0.001);
    let text = r.report.to_string();
    assert!(text.contains("contains itself"), "{text}");
    assert!(text.contains("missing"), "{text}");
}

#[test]
fn reroutes_are_removed() {
    let p = P::new()
        .node(1, "t_src")
        .node(7, REROUTE_KIND)
        .node(8, REROUTE_KIND)
        .node(2, "t_sum")
        .node(3, "t_out")
        .wire(1, "out", 7, "in")
        .wire(7, "out", 8, "in")
        .wire(8, "out", 2, "in")
        .wire(8, "out", 2, "b")
        .wire(2, "out", 3, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels[0][6], 12.0);
}

#[test]
fn unknown_kinds_warn_and_dont_block() {
    let mut p = chain();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(99), "from_the_future"));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(99), "out"),
        Endpoint::new(NodeId(2), "b"),
    ));
    let r = offline(&p, 32, 0.002);
    assert!(!r.report.has_errors());
    assert!(r.report.to_string().contains("from_the_future"));
    assert_eq!(r.channels[0][3], 3.0);
}

#[test]
fn dynamic_sockets_from_config() {
    let p = P::new()
        .node(1, "t_src")
        .node_with(NodeInstance::new(NodeId(2), "t_mix").with_config(json!({"inputs": 3})))
        .node(3, "t_out")
        .wire(1, "out", 2, "in_0")
        .wire(1, "out", 2, "in_2")
        .wire(2, "out", 3, "in")
        .build();
    let r = offline(&p, 32, 0.002);
    assert!(r.report.diagnostics.is_empty(), "{}", r.report);
    assert_eq!(r.channels[0][5], 10.0);
}

#[test]
fn state_carries_over_unless_config_changes() {
    let p = P::new()
        .node(1, "t_counter")
        .node(2, "t_sum")
        .node(3, "t_out")
        .wire(1, "out", 2, "amount")
        .wire(2, "out", 3, "in")
        .build();
    let (e, _) = engine(p.clone(), 32);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(32 * 5, 32);
    // Unrelated edit: counter keeps counting.
    let mut p2 = p.clone();
    p2.graph.nodes.push(NodeInstance::new(NodeId(50), "t_add"));
    host.handle().edit(GraphEdit::Replace(Box::new(p2.clone())));
    let out = host.run(32, 32);
    assert_eq!(out[0][0], 6.0);
    // Config change: counter restarts.
    let mut p3 = p2.clone();
    p3.graph.node_mut(NodeId(1)).unwrap().config = json!({"changed": true});
    host.handle().edit(GraphEdit::Replace(Box::new(p3)));
    let out = host.run(32, 32);
    assert_eq!(out[0][0], 1.0);
}

#[test]
fn set_param_smooths_float_changes() {
    let p = P::new()
        .node(1, "t_sum")
        .node(2, "t_out")
        .wire(1, "out", 2, "in")
        .build();
    let (e, _) = engine(p, 32);
    let mut host = NullHost::new(e, 0, 2);
    host.run(64, 32);
    assert!(
        host.handle()
            .set_param(&path(1), "amount", Value::Float(1.0))
    );
    let out = host.run(480, 32);
    // Ramps over 5 ms (240 samples at 48 kHz): no jump, then holds.
    assert!(out[0][0] < 0.01, "{}", out[0][0]);
    assert!((out[0][120] - 0.5).abs() < 0.02, "{}", out[0][120]);
    assert_eq!(out[0][300], 1.0);
    // The handle's project copy is updated so a recompile keeps the value.
    assert_eq!(
        host.handle()
            .project()
            .graph
            .node(NodeId(1))
            .unwrap()
            .params["amount"],
        Value::Float(1.0)
    );
    assert!(!host.handle().set_param(&path(1), "in", Value::Float(1.0)));
}

#[test]
fn generator_start_modes() {
    let p = P::new()
        .node(1, "t_starts")
        .node(2, "t_starts")
        .param(2, "start", "load")
        .node(3, "t_starts")
        .param(3, "start", "never")
        .build();
    let (e, _) = engine(p, 32);
    let mut host = NullHost::new(e, 0, 2);
    host.run(64, 32); // stopped
    let t = host.handle().telemetry();
    assert_eq!(t.node_value(&path(1), 0), Some(0.0));
    assert_eq!(t.node_value(&path(2), 0), Some(1.0));
    host.handle().play();
    host.run(64, 32);
    assert_eq!(t.node_value(&path(1), 0), Some(1.0));
    // Play from zero resets nodes, so "load" fires again.
    assert_eq!(t.node_value(&path(2), 0), Some(1.0));
    assert_eq!(t.node_value(&path(3), 0), Some(0.0));
    host.handle().fire(&path(3), "start");
    host.run(32, 32);
    assert_eq!(t.node_value(&path(3), 0), Some(1.0));
}

#[test]
fn offline_matches_null_host_bit_for_bit() {
    for block in [32usize, 1024] {
        let p = {
            let mut p = chain();
            p.graph
                .nodes
                .push(NodeInstance::new(NodeId(9), "t_counter"));
            p.graph.wires.push(Wire::new(
                Endpoint::new(NodeId(9), "out"),
                Endpoint::new(NodeId(2), "amount"),
            ));
            p
        };
        let seconds = 0.25;
        let off = offline(&p, block, seconds);
        let (e, _) = engine(p, block);
        let mut host = NullHost::new(e, 0, 2);
        host.handle().play();
        let live = host.run(off.frames(), block);
        assert_eq!(off.channels, live, "block {block}");
    }
}

#[test]
fn buffers_are_reused_along_a_chain() {
    let mut b = P::new().node(1, "t_src");
    let mut prev = 1u64;
    for id in 2..60u64 {
        b = b.node(id, "t_sum").wire(prev, "out", id, "in");
        prev = id;
    }
    let p = b.node(100, "t_out").wire(prev, "out", 100, "in").build();
    let (_, report) = engine(p, 64);
    assert!(report.stats.sample_buffers <= 4, "{:?}", report.stats);
}

#[test]
fn muted_nodes_pass_through() {
    let mut p = chain();
    p.graph
        .node_mut(NodeId(2))
        .unwrap()
        .params
        .insert("amount".into(), Value::Float(100.0));
    p.graph.node_mut(NodeId(2)).unwrap().muted = true;
    let r = offline(&p, 32, 0.002);
    assert_eq!(r.channels[0][7], 7.0);
}

#[test]
fn transport_loops_and_restarts() {
    let mut p = chain();
    p.settings.tempo = vec![TempoSegment::default()];
    // Loop beats 0..0.01 at 120 BPM = 240 samples.
    p.settings.loop_region = Some([0.0, 0.01]);
    let (e, _) = engine(p, 64);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    let out = host.run(600, 64);
    assert_eq!(out[0][239], 239.0);
    assert_eq!(out[0][240], 0.0);
    assert_eq!(out[0][250], 10.0);
}

#[test]
fn stop_and_seek() {
    let (e, _) = engine(chain(), 32);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(64, 32);
    host.handle().stop();
    let out = host.run(32, 32);
    assert_eq!(out[0][0], 64.0, "position frozen while stopped");
    host.handle().seek(1000);
    host.handle().play();
    let out = host.run(32, 32);
    assert_eq!(out[0][0], 1000.0);
    assert!(host.handle().is_playing());
}

#[test]
fn telemetry_reports_socket_levels() {
    let (e, _) = engine(chain(), 32);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(64, 32);
    let t = host.handle().telemetry();
    let level = t.socket_level(&path(2), "out").unwrap();
    assert!(level >= 63.0, "{level}");
    assert_eq!(t.socket_value(&path(1), "out"), Some(63.0));
    assert!(t.status().blocks.load(std::sync::atomic::Ordering::Relaxed) >= 2);
}

#[test]
fn host_blocks_larger_than_max_block_are_split() {
    let (e, _) = engine(chain(), 32);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    let out = host.run(500, 250);
    assert_eq!(out[0][499], 499.0);
}

#[test]
fn garbage_in_projects_never_panics() {
    let mut p = chain();
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "nope"),
        Endpoint::new(NodeId(2), "b"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "nope"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(77), "out"),
        Endpoint::new(NodeId(2), "b"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    ));
    p.graph
        .node_mut(NodeId(1))
        .unwrap()
        .params
        .insert("scale".into(), Value::Text("loud".into()));
    p.settings.tempo = vec![TempoSegment {
        beat: -4.0,
        bpm: f64::NAN,
        time_sig: [0, 0],
    }];
    let r = offline(&p, 0, 0.001);
    assert!(!r.report.diagnostics.is_empty());
    assert_eq!(r.channels[0][3], 3.0);
}
