//! Bindings in the engine: MIDI and control events moving bound inputs,
//! firing triggers, and pressing transport actions, through the same
//! callback path a device uses (the null host) and offline renders. Every
//! `Processor::process` runs under the allocation guard.

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f64 = 48_000.0;

fn path(id: u64) -> NodePath {
    NodePath::root(NodeId(id))
}

fn cc(controller: u8, value: u8) -> MidiMsg {
    MidiMsg::ControlChange {
        channel: 0,
        controller,
        value,
    }
}

fn host(project: Project, block: usize) -> (NullHost, CompileReport) {
    let (engine, report) = Engine::new(
        project,
        Arc::new(ac_nodes::registry()),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: RATE,
            max_block: block,
            socket_telemetry: true,
            offline: false,
        },
    );
    (NullHost::new(engine, 0, 2), report)
}

/// A constant 1.0 through a Gain (node 2) to the outputs, with CC 7 bound
/// to the gain over −60..0 dB.
fn gain_patch() -> Project {
    let mut p = Project::default();
    p.settings.length = 0.25;
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "constant").with_param("value", 1.0));
    p.graph.nodes.push(NodeInstance::new(NodeId(2), "gain"));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(3), "audio_out"));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(3), "in"),
    ));
    p.bindings.push(Binding {
        max: Some(0.0),
        ..Binding::input(
            Source::MidiCc {
                channel: Some(1),
                cc: 7,
            },
            path(2),
            "gain",
        )
    });
    p
}

fn last(out: &[f32], frames: usize) -> f32 {
    out[frames - 1]
}

#[test]
fn a_midi_controller_moves_a_bound_gain() {
    let (mut h, report) = host(gain_patch(), 256);
    assert!(report.diagnostics.is_empty(), "{report}");
    h.handle().play();
    // Unbound so far: the stored 0 dB.
    assert!((last(h.tick(256), 256) - 1.0).abs() < 1e-6);
    // CC 7 at 0 → −60 dB (silent once the 5 ms smoothing has run).
    h.tick_with(256, &[], &[MidiEvent::new(10, cc(7, 0))]);
    let out = h.tick(256).to_vec();
    assert!(last(&out, 256).abs() < 1e-6, "got {}", last(&out, 256));
    // Halfway: −30 dB.
    h.tick_with(256, &[], &[MidiEvent::new(0, cc(7, 64))]);
    let out = h.tick(256).to_vec();
    let db = 20.0 * last(&out, 256).log10();
    let want = -60.0 + 60.0 * 64.0 / 127.0;
    assert!((db - want).abs() < 0.01, "{db} dB, wanted {want}");
    // Another channel or controller does nothing.
    h.tick_with(
        256,
        &[],
        &[
            MidiEvent::new(0, cc(8, 127)),
            MidiEvent::new(
                1,
                MidiMsg::ControlChange {
                    channel: 4,
                    controller: 7,
                    value: 127,
                },
            ),
        ],
    );
    let again = last(h.tick(256), 256);
    assert!((20.0 * again.log10() - want).abs() < 0.01);
    // The editor can read what the binding set.
    let v = h.handle().binding_values().get(0).expect("a value");
    assert!((v - want).abs() < 1e-4);
}

#[test]
fn bound_values_survive_a_recompile_until_moved_by_hand() {
    let (mut h, _) = host(gain_patch(), 128);
    h.handle().play();
    h.tick_with(128, &[], &[MidiEvent::new(0, cc(7, 0))]);
    h.run(1024, 128);
    assert!(last(h.tick(128), 128).abs() < 1e-6);
    // An unrelated edit recompiles; the gain stays where the knob left it.
    let mut p = h.handle().project().clone();
    p.graph.nodes.push(NodeInstance::new(NodeId(9), "constant"));
    h.handle().edit(GraphEdit::Replace(Box::new(p)));
    h.tick(128);
    assert!(last(h.tick(128), 128).abs() < 1e-6, "snapped back");
    assert!(h.handle().binding_values().get(0).is_some());
    // Moving the input by hand wins, and the binding forgets its value.
    h.handle().set_param(&path(2), "gain", Value::Float(-6.0));
    h.run(1024, 128);
    let out = last(h.tick(128), 128);
    assert!((20.0 * out.log10() + 6.0).abs() < 0.01);
    assert_eq!(h.handle().binding_values().get(0), None);
}

#[test]
fn a_key_toggles_a_bound_bool_and_a_button_fires_a_trigger() {
    let space = Key::from_name("Space").unwrap();
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "constant").with_config(json!({"type": "bool"})));
    p.graph.nodes.push(NodeInstance::new(NodeId(2), "counter"));
    p.bindings.push(Binding {
        mode: GateMode::Toggle,
        ..Binding::input(Source::Key { key: space }, path(1), "value")
    });
    p.bindings.push(Binding::input(
        Source::GamepadButton {
            pad: None,
            button: GamepadButton::South,
        },
        path(2),
        "trigger",
    ));
    let (mut h, report) = host(p, 64);
    assert!(report.diagnostics.is_empty(), "{report}");
    let tele = h.handle().telemetry();
    let key = |down: bool| ControlEvent::new(5, Control::Key(space), if down { 1.0 } else { 0.0 });
    let button = |down: bool| {
        ControlEvent::new(
            3,
            Control::GamepadButton {
                pad: 1,
                button: GamepadButton::South,
            },
            if down { 1.0 } else { 0.0 },
        )
    };
    h.tick(64);
    assert_eq!(tele.socket_value(&path(1), "out"), Some(0.0));
    // Press: on. Holding (a repeat) and releasing change nothing.
    h.tick_with_controls(64, &[], &[], &[key(true)]);
    assert_eq!(tele.socket_value(&path(1), "out"), Some(1.0));
    h.tick_with_controls(64, &[], &[], &[key(true)]);
    h.tick_with_controls(64, &[], &[], &[key(false)]);
    assert_eq!(tele.socket_value(&path(1), "out"), Some(1.0));
    // The next press turns it off.
    h.tick_with_controls(64, &[], &[], &[key(true), key(false)]);
    assert_eq!(tele.socket_value(&path(1), "out"), Some(0.0));
    assert_eq!(h.handle().binding_values().get(0), Some(0.0));

    // Each press of the button counts once; releases don't.
    for _ in 0..3 {
        h.tick_with_controls(64, &[], &[], &[button(true)]);
        h.tick_with_controls(64, &[], &[], &[button(false)]);
    }
    h.tick(64);
    assert_eq!(tele.socket_value(&path(2), "count"), Some(3.0));
    // Triggers publish no value.
    assert_eq!(h.handle().binding_values().get(1), None);
}

#[test]
fn transport_bindings_start_stop_and_record() {
    let a = Control::GamepadButton {
        pad: 0,
        button: GamepadButton::South,
    };
    let b = Control::GamepadButton {
        pad: 0,
        button: GamepadButton::East,
    };
    let x = Control::GamepadButton {
        pad: 0,
        button: GamepadButton::West,
    };
    let mut p = Project::default();
    for (button, action) in [
        (GamepadButton::South, TransportAction::PlayStop),
        (GamepadButton::East, TransportAction::Stop),
        (GamepadButton::West, TransportAction::Record),
    ] {
        p.bindings.push(Binding::transport(
            Source::GamepadButton { pad: None, button },
            action,
        ));
    }
    let (mut h, report) = host(p, 128);
    assert!(report.diagnostics.is_empty(), "{report}");
    let press = |h: &mut NullHost, c: Control, at: u32| {
        h.tick_with_controls(128, &[], &[], &[ControlEvent::new(at, c, 1.0)]);
        h.tick_with_controls(128, &[], &[], &[ControlEvent::new(0, c, 0.0)]);
    };
    h.tick(128);
    assert!(!h.handle().is_playing());
    // A press lands on its sample: playing from frame 100 of 128.
    press(&mut h, a, 100);
    assert!(h.handle().is_playing());
    assert_eq!(h.handle().position(), 28 + 128);
    press(&mut h, a, 0);
    assert!(!h.handle().is_playing());
    // Record starts playback too; Stop ends both.
    press(&mut h, x, 0);
    assert!(h.handle().is_playing());
    assert!(
        h.handle()
            .status()
            .recording
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    press(&mut h, b, 0);
    assert!(!h.handle().is_playing());
    assert!(
        !h.handle()
            .status()
            .recording
            .load(std::sync::atomic::Ordering::Relaxed)
    );
}

#[test]
fn bindings_to_wired_missing_or_unbindable_inputs_are_reported_not_applied() {
    let mut p = gain_patch();
    let src = |cc| Source::MidiCc { channel: None, cc };
    p.bindings.push(Binding::input(src(1), path(2), "in"));
    p.bindings.push(Binding::input(src(2), path(42), "gain"));
    p.bindings.push(Binding::input(src(3), path(2), "nope"));
    p.bindings.push(Binding::input(src(4), path(3), "in"));
    // A constant driving the gain: binding the wired input does nothing.
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(5), "constant").with_param("value", -6.0));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(5), "out"),
        Endpoint::new(NodeId(2), "gain"),
    ));
    let (mut h, report) = host(p, 256);
    let messages: Vec<String> = report.diagnostics.iter().map(|d| d.to_string()).collect();
    let has = |needle: &str| messages.iter().any(|m| m.contains(needle));
    assert!(has("the input is wired"), "{report}");
    assert!(has("isn't in the project"), "{report}");
    assert!(has("no input `nope`"), "{report}");
    assert!(has("Audio inputs can't be bound"), "{report}");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
    h.handle().play();
    h.tick_with(
        256,
        &[],
        &[
            MidiEvent::new(0, cc(7, 127)),
            MidiEvent::new(1, cc(1, 0)),
            MidiEvent::new(2, cc(2, 0)),
        ],
    );
    let out = last(h.tick(256), 256);
    // Still the wired −6 dB.
    assert!((20.0 * out.log10() + 6.0).abs() < 0.01, "{out}");
}

#[test]
fn a_group_instance_input_and_a_node_inside_a_group_can_be_bound() {
    // Group "g": a Gain whose `gain` is exposed as the instance input `level`.
    let mut p = gain_patch();
    p.bindings.clear();
    let mut def = GroupDef::new("Level");
    def.graph.nodes.push(NodeInstance::new(NodeId(7), "gain"));
    def.inputs.push(GroupInput {
        name: "audio".into(),
        ty: SignalType::Audio,
        unit: Unit::Generic,
        default: None,
        range: None,
        targets: vec![Endpoint::new(NodeId(7), "in")],
    });
    def.inputs.push(GroupInput {
        name: "level".into(),
        ty: SignalType::Float,
        unit: Unit::Db,
        default: Some(Value::Float(0.0)),
        range: Some([-60.0, 12.0]),
        targets: vec![Endpoint::new(NodeId(7), "gain")],
    });
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(7), "out"),
    });
    p.group_defs.insert(GroupId::from("g"), def);
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(8), GROUP_KIND).with_config(json!({"def": "g"})));
    p.graph.wires.retain(|w| w.to.node != NodeId(3));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(8), "audio"),
    ));
    p.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(8), "out"),
        Endpoint::new(NodeId(3), "in"),
    ));
    p.bindings.push(Binding {
        min: Some(-12.0),
        max: Some(-12.0),
        ..Binding::input(
            Source::MidiCc {
                channel: None,
                cc: 1,
            },
            path(8),
            "level",
        )
    });
    p.bindings.push(Binding {
        min: Some(-6.0),
        max: Some(-6.0),
        ..Binding::input(
            Source::MidiCc {
                channel: None,
                cc: 2,
            },
            NodePath(vec![NodeId(8), NodeId(7)]),
            "gain",
        )
    });
    let (mut h, report) = host(p, 256);
    assert!(report.diagnostics.is_empty(), "{report}");
    h.handle().play();
    h.tick_with(256, &[], &[MidiEvent::new(0, cc(1, 99))]);
    let out = last(h.tick(256), 256);
    assert!((20.0 * out.log10() + 12.0).abs() < 0.01, "{out}");
    h.tick_with(256, &[], &[MidiEvent::new(0, cc(2, 99))]);
    let out = last(h.tick(256), 256);
    assert!((20.0 * out.log10() + 6.0).abs() < 0.01, "{out}");
}

/// CC sweeps at awkward offsets, plus a note bound to the gain's top.
fn sweep() -> Vec<(u64, MidiMsg)> {
    let mut v: Vec<(u64, MidiMsg)> = (0..40u64)
        .map(|k| (k * 997 + 13, cc(7, (k * 3 % 128) as u8)))
        .collect();
    v.push((
        20_001,
        MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    ));
    v
}

#[test]
fn offline_renders_with_bindings_are_deterministic_and_block_size_independent() {
    let mut p = gain_patch();
    p.bindings.push(Binding::input(
        Source::MidiNote {
            channel: None,
            note: 60,
        },
        path(2),
        "gain",
    ));
    let render_at = |block: usize| {
        render(
            &p,
            Arc::new(ac_nodes::registry()),
            Arc::new(NoResources),
            &RenderOptions {
                block_size: Some(block),
                midi: sweep(),
                ..RenderOptions::default()
            },
        )
    };
    let a = render_at(64);
    assert!(a.report.diagnostics.is_empty(), "{}", a.report);
    // The sweep did something.
    let ch = &a.channels[0];
    assert!(ch.iter().any(|v| *v < 0.01) && ch.iter().any(|v| *v > 0.5));
    assert_eq!(
        render_at(64).channels,
        a.channels,
        "same block, same samples"
    );
    for block in [1usize, 32, 333, 1024] {
        assert_eq!(
            render_at(block).channels,
            a.channels,
            "block {block} differs"
        );
    }
    // Live through the null host at the same block equals offline.
    for block in [32usize, 1024] {
        let (mut h, _) = host(p.clone(), block);
        h.handle().play();
        let total = a.frames();
        let events = sweep();
        let mut live = Vec::with_capacity(total);
        let mut done = 0;
        while done < total {
            let n = block.min(total - done);
            let midi: Vec<MidiEvent> = events
                .iter()
                .filter(|(at, _)| (*at as usize) >= done && (*at as usize) < done + n)
                .map(|(at, m)| MidiEvent::new((*at as usize - done) as u32, *m))
                .collect();
            let out = h.tick_with(n, &[], &midi);
            live.extend_from_slice(&out[..n]);
            done += n;
        }
        assert_eq!(live, a.channels[0], "live at block {block}");
    }
}

#[test]
fn offline_renders_take_control_events() {
    let space = Key::from_name("Space").unwrap();
    let mut p = gain_patch();
    p.bindings.clear();
    p.bindings.push(Binding {
        min: Some(-60.0),
        max: Some(0.0),
        ..Binding::input(Source::Key { key: space }, path(2), "gain")
    });
    let out = render(
        &p,
        Arc::new(ac_nodes::registry()),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(256),
            to: Some(0.1),
            controls: vec![
                (1_000, Control::Key(space), 0.0),
                (3_000, Control::Key(space), 1.0),
            ],
            ..RenderOptions::default()
        },
    );
    let ch = &out.channels[0];
    assert!((ch[900] - 1.0).abs() < 1e-6, "stored 0 dB before the key");
    assert!(ch[2_900].abs() < 1e-6, "key up: −60 dB");
    assert!((ch[4_000] - 1.0).abs() < 1e-6, "key down: 0 dB");
}

#[test]
fn the_input_monitor_hands_raw_events_to_the_editor() {
    let (mut h, _) = host(Project::default(), 64);
    // Off: nothing is kept.
    h.tick_with(64, &[], &[MidiEvent::new(0, cc(1, 1))]);
    assert!(h.handle().take_input_events().is_empty());
    h.handle().set_input_monitor(true);
    let key = Key::from_char('q').unwrap();
    h.tick_with_controls(
        64,
        &[],
        &[MidiEvent::new(3, cc(1, 2))],
        &[ControlEvent::new(9, Control::Key(key), 1.0)],
    );
    assert_eq!(
        h.handle().take_input_events(),
        vec![
            InputEvent::Midi(cc(1, 2)),
            InputEvent::Control(Control::Key(key), 1.0)
        ]
    );
    // A flood fills the ring and the rest is dropped, without allocating.
    let flood: Vec<MidiEvent> = (0..1000)
        .map(|i| MidiEvent::new(i % 64, cc(1, 3)))
        .collect();
    h.tick_with(64, &[], &flood);
    let got = h.handle().take_input_events();
    assert!(!got.is_empty() && got.len() < 1000);
    h.handle().set_input_monitor(false);
    h.tick_with(64, &[], &[MidiEvent::new(0, cc(1, 4))]);
    assert!(h.handle().take_input_events().is_empty());
}

#[test]
fn a_flood_of_bound_events_stays_allocation_free() {
    let (mut h, _) = host(gain_patch(), 512);
    h.handle().play();
    let flood: Vec<MidiEvent> = (0..1024u32)
        .map(|i| MidiEvent::new(i / 2, cc(7, (i % 128) as u8)))
        .collect();
    for _ in 0..4 {
        h.tick_with(512, &[], &flood);
    }
    let out = last(h.tick(512), 512);
    assert!(out.is_finite());
}
