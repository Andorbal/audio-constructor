//! Transport Stop reaches every generator, wired or not, and works with the
//! transport already stopped; Panic silences everything (ROADMAP M4.1,
//! DECISIONS D200, D201).

mod common;

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use common::{P, path, registry};

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

fn host_generation(host: &mut NullHost) -> u64 {
    host.handle()
        .status()
        .generation
        .load(std::sync::atomic::Ordering::Relaxed)
}

fn stops(host: &mut NullHost, id: u64) -> f32 {
    host.handle()
        .telemetry()
        .node_value(&path(id), 0)
        .unwrap_or(-1.0)
}

/// Node 1 has its `stop` wired from a pulse; node 2's is unwired; node 3's
/// is wired and set to *Never*.
fn generators() -> Project {
    P::new()
        .node(1, "t_stops")
        .node(2, "t_stops")
        .node(3, "t_stops")
        .param(3, "stop", "never")
        .node(9, "t_pulse")
        .wire(9, "out", 1, "stop")
        .wire(9, "out", 3, "stop")
        .build()
}

#[test]
fn transport_stop_fires_wired_and_unwired_stops() {
    let (e, report) = engine(generators(), 64);
    assert!(!report.has_errors(), "{report}");
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(128, 64);
    host.handle().stop();
    host.run(64, 64);
    assert_eq!(stops(&mut host, 1), 1.0, "a wired stop hears the transport");
    assert_eq!(stops(&mut host, 2), 1.0);
    assert_eq!(stops(&mut host, 3), 0.0, "Never keeps it to the wire");
}

#[test]
fn stop_fires_with_the_transport_already_stopped() {
    let (e, _) = engine(generators(), 64);
    let mut host = NullHost::new(e, 0, 2);
    host.run(64, 64);
    host.handle().stop();
    host.run(64, 64);
    host.handle().stop();
    host.run(64, 64);
    assert_eq!(stops(&mut host, 1), 2.0);
    assert_eq!(stops(&mut host, 2), 2.0);
    assert!(!host.handle().is_playing());
}

#[test]
fn the_wires_own_stops_in_the_stop_block_are_kept() {
    // The pulse fires 5 frames into the block the transport stops in, and
    // on that block's first frame in a second run.
    for (pulse, expected) in [(64 + 5, 2.0), (64, 1.0)] {
        let p = generators();
        let mut p = p;
        p.graph
            .node_mut(NodeId(9))
            .unwrap()
            .params
            .insert("at".into(), Value::Int(pulse));
        let (e, _) = engine(p, 64);
        let mut host = NullHost::new(e, 0, 2);
        host.handle().play();
        host.run(64, 64);
        host.handle().stop();
        host.run(64, 64);
        assert_eq!(stops(&mut host, 1), expected, "pulse at {pulse}");
        // "Never" still hears its wire.
        assert_eq!(stops(&mut host, 3), 1.0, "pulse at {pulse}");
    }
}

#[test]
fn the_dropdown_beside_a_wire_changes_without_a_recompile() {
    let (e, _) = engine(generators(), 64);
    let mut host = NullHost::new(e, 0, 2);
    host.run(64, 64);
    let before = host_generation(&mut host);
    assert!(
        host.handle()
            .set_param(&path(1), "stop", Value::Text("never".into()))
    );
    host.run(64, 64);
    assert_eq!(host_generation(&mut host), before, "no recompile");
    host.handle().stop();
    host.run(64, 64);
    assert_eq!(stops(&mut host, 1), 0.0);
    assert_eq!(stops(&mut host, 2), 1.0);
}

#[test]
fn a_binding_on_a_wired_stop_is_still_reported_as_wired() {
    let mut p = generators();
    p.bindings.push(Binding::new(
        Source::Key {
            key: Key::from_name("A").unwrap(),
        },
        Target::Input {
            node: path(1),
            socket: "stop".into(),
        },
    ));
    let (_, report) = engine(p, 64);
    let text = report.to_string();
    assert!(text.contains("the input is wired"), "{text}");
}

#[test]
fn panic_stops_resets_and_mutes_one_block() {
    let p = P::new()
        .node(1, "t_src")
        .node(2, "t_out")
        .wire(1, "out", 2, "in")
        .node(3, "t_starts")
        .param(3, "start", "load")
        .node(4, "t_stops")
        .build();
    let (e, _) = engine(p, 64);
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(256, 64);
    host.handle().panic();
    let out = host.run(64, 64);
    assert!(out[0].iter().all(|&v| v == 0.0), "muted for the block");
    assert!(!host.handle().is_playing());
    assert_eq!(stops(&mut host, 4), 1.0, "every stop fired");
    // Reset: the start counter went back and "On load" fired once more.
    let t = host.handle().telemetry();
    assert_eq!(t.node_value(&path(3), 0), Some(1.0));
    // The next block plays out again (the source follows the stopped
    // transport's position).
    let out = host.run(64, 64);
    assert!(out[0].iter().any(|&v| v != 0.0));
}

#[test]
fn panic_is_a_bindable_transport_action() {
    let mut p = P::new().node(4, "t_stops").build();
    p.bindings.push(Binding::transport(
        Source::Key {
            key: Key::from_name("Escape").unwrap(),
        },
        TransportAction::Panic,
    ));
    let (e, report) = engine(p, 64);
    assert!(report.diagnostics.is_empty(), "{report}");
    let mut host = NullHost::new(e, 0, 2);
    host.handle().play();
    host.run(64, 64);
    let esc = Key::from_name("Escape").unwrap();
    host.tick_with_controls(
        64,
        &[],
        &[],
        &[ControlEvent::new(0, Control::Key(esc), 1.0)],
    );
    assert!(!host.handle().is_playing());
    assert_eq!(stops(&mut host, 4), 1.0);
}
