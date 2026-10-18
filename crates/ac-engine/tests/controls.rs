//! Control-surface events (keys, gamepads, OSC) offline and live: a render
//! feeds them at absolute frames, the null host feeds them per callback, and
//! both give the same samples and the same OSC output, whatever the block
//! size. Bakes replay them too. Every `Processor::process` runs under the
//! allocation guard.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use common::{P, path};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

node! {
    kind: "t_ctl_hold",
    name: "Control Hold",
    category: Io,
    keywords: [],
    /// A live source: outputs the last value received for key `A`, sample
    /// accurately, and sends every event as OSC `/held` to config `to`.
    /// Try this: nothing.
    pub struct TCtlHold { value: f32, to: Option<SocketAddr> }
}
impl Node for TCtlHold {
    fn meta() -> NodeMeta {
        Self::base_meta().live().host_output()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::float("out", Unit::Generic, 0.0).rate(Rate::Sample))
    }
    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TCtlHold {
            value: 0.0,
            to: config.get_str_or("to", "").parse().ok(),
        }
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let a = key('a');
        let controls = io.host.controls();
        let mut next = 0;
        let mut v = self.value;
        let mut out = io.outputs.float(0);
        out.fill_with(|i| {
            while let Some(e) = controls.get(next) {
                if e.offset as usize > i {
                    break;
                }
                if e.control == a {
                    v = e.value;
                }
                next += 1;
            }
            v
        });
        self.value = v;
        if let Some(to) = self.to {
            for e in controls {
                if let Some(m) = OscMessage::new(e.offset, to, "/held", &[e.value]) {
                    io.host.send_osc(m);
                }
            }
        }
    }
    fn reset(&mut self) {
        self.value = 0.0;
    }
}

node! {
    kind: "t_float_out",
    name: "Float Out",
    category: Io,
    keywords: [],
    /// Writes a float input to device output 0. Try this: nothing.
    pub struct TFloatOut {}
}
impl Node for TFloatOut {
    fn meta() -> NodeMeta {
        Self::base_meta().host_output()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().input(SocketSpec::audio("in").channels(Channels::Fixed(1)))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TFloatOut {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.audio(0);
        let src = a.channel(0);
        if let Some(dst) = io.host.output_channel(0) {
            for (d, s) in dst.iter_mut().zip(src) {
                *d += s;
            }
        }
    }
    fn reset(&mut self) {}
}

fn registry() -> Arc<Registry> {
    let mut r = common::registry();
    ac_nodes::register_all(&mut r);
    r.register::<TCtlHold>().register::<TFloatOut>();
    Arc::new(r)
}

fn key(c: char) -> Control {
    Control::Key(Key::from_char(c).expect("a letter"))
}

const TO: &str = "127.0.0.1:9000";

/// Control Hold → Float Out, 0.05 s at 48 kHz (2400 frames).
fn project() -> Project {
    let mut p = P::new()
        .node_with(NodeInstance::new(NodeId(1), "t_ctl_hold").with_config(json!({ "to": TO })))
        .node(2, "t_float_out")
        .wire(1, "out", 2, "in")
        .build();
    p.settings.length = 0.05;
    p
}

/// Key A presses and releases, a key that isn't A, and two events on one
/// frame (the later one wins), some on block edges.
fn events() -> Vec<(u64, Control, f32)> {
    vec![
        (0, key('a'), 0.25),
        (31, key('b'), 9.0),
        (32, key('a'), 0.5),
        (700, key('a'), 1.0),
        (700, key('a'), 0.75),
        (1023, key('a'), 0.0),
        (2399, key('a'), 2.0),
    ]
}

fn offline(p: &Project, block: usize, controls: Vec<(u64, Control, f32)>) -> Rendered {
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

/// What the hold should output for `events()`.
fn expected() -> Vec<f32> {
    (0..2400)
        .map(|t| match t {
            0..=31 => 0.25,
            32..=699 => 0.5,
            700..=1022 => 0.75,
            1023..=2398 => 0.0,
            _ => 2.0,
        })
        .collect()
}

#[test]
fn offline_controls_land_on_their_frames_at_any_block_size() {
    let want = expected();
    let mut osc_by_block = Vec::new();
    for block in [1, 32, 100, 1024] {
        let r = offline(&project(), block, events());
        assert!(!r.report.has_errors(), "{}", r.report);
        assert_eq!(r.channels[0], want, "block {block}");
        let osc: Vec<(u64, String, Vec<f32>)> = r
            .osc
            .iter()
            .map(|(at, m)| (*at, m.address().to_string(), m.args().to_vec()))
            .collect();
        osc_by_block.push(osc);
    }
    let to: SocketAddr = TO.parse().unwrap();
    assert!(
        offline(&project(), 64, events())
            .osc
            .iter()
            .all(|(_, m)| m.target == to)
    );
    // One message per event, at the event's frame, whatever the block.
    assert_eq!(osc_by_block[0].len(), events().len());
    for (i, (at, _, v)) in events().iter().enumerate() {
        assert_eq!(osc_by_block[0][i].0, *at);
        assert_eq!(osc_by_block[0][i].2, [*v]);
    }
    for o in &osc_by_block[1..] {
        assert_eq!(o, &osc_by_block[0]);
    }
}

#[test]
fn offline_renders_are_deterministic_and_unsorted_input_is_fine() {
    let a = offline(&project(), 64, events());
    let mut shuffled = events();
    shuffled.reverse();
    // Reversing swaps the two events at frame 700; put them back so only
    // the order across frames is scrambled.
    let i = shuffled.iter().position(|e| e.0 == 700).unwrap();
    shuffled.swap(i, i + 1);
    let b = offline(&project(), 64, shuffled);
    assert_eq!(a.channels, b.channels);
    assert_eq!(
        a.osc
            .iter()
            .map(|(t, m)| (*t, m.args().to_vec()))
            .collect::<Vec<_>>(),
        b.osc
            .iter()
            .map(|(t, m)| (*t, m.args().to_vec()))
            .collect::<Vec<_>>()
    );
    let c = offline(&project(), 64, events());
    assert_eq!(a.channels, c.channels);
}

#[test]
fn renders_from_an_offset_see_events_from_their_own_start() {
    let r = render(
        &project(),
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            from: 0.01,
            block_size: Some(64),
            controls: vec![(10, key('a'), 0.5)],
            out_channels: Some(1),
            ..Default::default()
        },
    );
    let out = &r.channels[0];
    assert_eq!(out.len(), 1920);
    assert!(out[..10].iter().all(|&v| v == 0.0));
    assert!(out[10..].iter().all(|&v| v == 0.5));
}

#[test]
fn live_and_offline_agree() {
    for block in [32usize, 1024] {
        let off = offline(&project(), block, events());
        let (engine, report) = Engine::new(
            project(),
            registry(),
            Arc::new(NoResources),
            EngineOptions {
                sample_rate: 48_000.0,
                max_block: block,
                socket_telemetry: true,
                offline: false,
            },
        );
        assert!(!report.has_errors(), "{report}");
        let mut host = NullHost::new(engine, 0, 1);
        host.handle().play();
        let total = off.frames();
        let evs = events();
        let mut live = Vec::with_capacity(total);
        let mut done = 0;
        while done < total {
            let n = block.min(total - done);
            let controls: Vec<ControlEvent> = evs
                .iter()
                .filter(|e| (e.0 as usize) >= done && (e.0 as usize) < done + n)
                .map(|e| ControlEvent::new((e.0 as usize - done) as u32, e.1, e.2))
                .collect();
            live.extend_from_slice(host.tick_with_controls(n, &[], &[], &controls));
            done += n;
        }
        assert_eq!(off.channels[0], live, "block {block}");
        assert_eq!(off.osc, host.take_sent_osc(), "block {block}");
    }
}

#[test]
fn bakes_replay_control_events() {
    // Control Hold → Reverse → Float Out: offline, the reverse bakes from
    // the same events the render plays.
    let mut p = P::new()
        .node(1, "t_ctl_hold")
        .node(2, "reverse")
        .node(3, "t_float_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.settings.length = 0.05;
    let r = offline(&p, 64, events());
    assert_eq!(
        r.report.stats.whole_input,
        vec![(path(2), WholeState::Baked)]
    );
    let mut want = expected();
    want.reverse();
    assert_eq!(r.channels[0], want);
    // Different events, a different bake (the input is part of its key).
    let r2 = offline(&p, 64, vec![(0, key('a'), 1.0)]);
    assert!(r2.channels[0].iter().all(|&v| v == 1.0));
}
