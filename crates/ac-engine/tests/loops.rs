//! Single-sample feedback (DECISIONS D140): a Feedback node with `delay`
//! set to `sample` closes its loop with one sample of delay, and the
//! compiler runs the loop's strongly connected component one sample at a
//! time while the rest of the graph runs in blocks. Every
//! `Processor::process` here runs under the allocation guard.

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use serde_json::{Value as Json, json};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

const RATE: f64 = 48_000.0;

// --- test nodes ------------------------------------------------------------------

node! {
    kind: "t_kick",
    name: "Kick",
    category: Generators,
    keywords: [],
    /// `out` = `in` plus 1 on every `trigger` frame, plus `note / 100` on
    /// every MIDI note-on frame. Try this: nothing.
    pub struct TKick {}
}
impl Node for TKick {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").channels(Channels::Fixed(1)))
            .input(SocketSpec::trigger("trigger"))
            .input(SocketSpec::midi("midi"))
            .output(SocketSpec::audio("out").channels(Channels::Fixed(1)))
            .output(SocketSpec::trigger("echo"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TKick {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(0);
        let src = input.channel(0);
        let trig = io.inputs.triggers(1);
        let midi = io.inputs.midi(2);
        let frames = io.frames();
        {
            let mut out = io.outputs.audio(0);
            let dst = out.channel(0);
            for (i, o) in dst.iter_mut().enumerate() {
                *o = src.get(i).copied().unwrap_or(0.0);
            }
            for &t in trig {
                if let Some(o) = dst.get_mut(t as usize) {
                    *o += 1.0;
                }
            }
            for e in midi {
                if let MidiMsg::NoteOn { note, .. } = e.msg
                    && let Some(o) = dst.get_mut((e.offset as usize).min(frames - 1))
                {
                    *o += f32::from(note) / 100.0;
                }
            }
        }
        let mut echo = io.outputs.trigger(1);
        for &t in trig {
            echo.push(t);
        }
    }
    fn reset(&mut self) {}
}

fn registry() -> Arc<Registry> {
    let mut r = ac_nodes::registry();
    r.register::<TKick>();
    Arc::new(r)
}

// --- projects --------------------------------------------------------------------

struct P(Project);

impl P {
    fn new(seconds: f64) -> P {
        let mut p = Project::default();
        p.settings.length = seconds;
        P(p)
    }
    fn node(mut self, id: u64, kind: &str, config: Json) -> P {
        self.0
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(id), kind).with_config(config));
        self
    }
    fn param(mut self, id: u64, socket: &str, v: impl Into<Value>) -> P {
        if let Some(n) = self.0.graph.node_mut(NodeId(id)) {
            n.params.insert(socket.into(), v.into());
        }
        self
    }
    fn wire(mut self, from: u64, out: &str, to: u64, inp: &str) -> P {
        self.0.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(from), out),
            Endpoint::new(NodeId(to), inp),
        ));
        self
    }
    fn build(self) -> Project {
        self.0
    }
}

fn path(id: u64) -> NodePath {
    NodePath::root(NodeId(id))
}

/// Impulse (1) → Mix (2) → Delay (3, `d` samples) → Gain (4, −6.02 dB) →
/// Feedback (5) → back into the Mix; the Mix also feeds Audio Out (6).
fn echo_loop(d: usize, delay: &str) -> Project {
    P::new(0.25)
        .node(1, "impulse", json!({}))
        .node(2, "mix", json!({ "inputs": 2 }))
        .node(3, "delay", json!({}))
        .param(3, "time", d as f64 / RATE)
        .param(3, "feedback", 0.0)
        .param(3, "mix", 1.0)
        .node(4, "gain", json!({}))
        .param(4, "gain", 20.0 * 0.5f64.log10())
        .node(5, "feedback", json!({ "delay": delay }))
        .node(6, "audio_out", json!({}))
        .wire(1, "out", 2, "in_0")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .wire(4, "out", 5, "in")
        .wire(5, "out", 2, "in_1")
        .wire(2, "out", 6, "in")
        .build()
}

fn render_at(p: &Project, block: usize) -> Rendered {
    render(
        p,
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(block),
            ..Default::default()
        },
    )
}

/// Frames where channel 0 is louder than `floor`.
fn hits(x: &[f32], floor: f32) -> Vec<usize> {
    x.iter()
        .enumerate()
        .filter(|(_, v)| v.abs() > floor)
        .map(|(i, _)| i)
        .collect()
}

fn live(p: Project, block: usize) -> Engine {
    Engine::new(
        p,
        registry(),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: RATE,
            max_block: block,
            socket_telemetry: true,
            offline: false,
        },
    )
    .0
}

/// Play `blocks` blocks live on `threads` threads; device output channel 0.
fn play(e: &mut Engine, block: usize, blocks: usize, midi: &[(u64, MidiMsg)]) -> Vec<f32> {
    let mut out = vec![0.0f32; 2 * block];
    let mut all = Vec::new();
    let start = e
        .processor
        .status()
        .position
        .load(std::sync::atomic::Ordering::Relaxed);
    for b in 0..blocks {
        let at = start + (b * block) as u64;
        let events: Vec<MidiEvent> = midi
            .iter()
            .filter(|(t, _)| (at..at + block as u64).contains(t))
            .map(|(t, m)| MidiEvent::new((t - at) as u32, *m))
            .collect();
        let mut io = HostIo {
            frames: block,
            input: &[],
            in_channels: 0,
            output: &mut out,
            out_channels: 2,
            midi: &events,
            midi_out: None,
            controls: &[],
            osc_out: None,
        };
        e.process(&mut io);
        all.extend_from_slice(&out[..block]);
    }
    all
}

fn on_threads(p: &Project, threads: usize, block: usize, blocks: usize) -> Vec<f32> {
    let mut e = live(p.clone(), block);
    if threads > 1 {
        e.processor
            .set_workers(Some(Arc::new(RtPool::new(threads - 1, None, None))));
    }
    e.handle.play();
    play(&mut e, block, blocks, &[])
}

// --- timing ----------------------------------------------------------------------

#[test]
fn a_sample_loop_comes_round_after_its_delay_plus_one_sample() {
    let r = render_at(&echo_loop(100, "sample"), 64);
    let x = &r.channels[0];
    assert_eq!(&hits(x, 1e-6)[..4], &[0, 101, 202, 303]);
    assert_eq!(x[0], 1.0);
    assert!((x[101] - 0.5).abs() < 1e-6, "{}", x[101]);
    assert!((x[202] - 0.25).abs() < 1e-6, "{}", x[202]);
    assert!(
        r.report
            .diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error && d.severity != Severity::Warning),
        "{}",
        r.report
    );
}

#[test]
fn a_block_loop_still_comes_round_after_a_block() {
    let r = render_at(&echo_loop(100, "block"), 64);
    assert_eq!(&hits(&r.channels[0], 1e-6)[..3], &[0, 164, 328]);
    assert!(r.report.stats.sample_loops.is_empty());
}

#[test]
fn a_sample_loop_sounds_the_same_at_any_block_size() {
    let p = echo_loop(37, "sample");
    let one = render_at(&p, 1).channels;
    for block in [2, 7, 64, 333, 1024] {
        assert!(render_at(&p, block).channels == one, "block {block}");
    }
}

#[test]
fn a_sample_loop_is_bit_identical_on_any_number_of_threads() {
    // Two loops and an independent chain beside them, so waves hold
    // several chains for the workers to share.
    let mut p = echo_loop(50, "sample");
    let more = echo_loop(73, "sample");
    for mut n in more.graph.nodes {
        n.id = NodeId(n.id.0 + 10);
        p.graph.nodes.push(n);
    }
    for w in more.graph.wires {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(w.from.node.0 + 10), w.from.socket.as_str()),
            Endpoint::new(NodeId(w.to.node.0 + 10), w.to.socket.as_str()),
        ));
    }
    let one = on_threads(&p, 1, 128, 30);
    assert!(one.iter().any(|v| v.abs() > 0.1), "silent");
    for threads in [2, 4] {
        assert!(on_threads(&p, threads, 128, 30) == one, "{threads} threads");
    }
    // And the live render matches the offline one.
    let offline = render_at(&p, 128).channels[0].clone();
    assert_eq!(&one[..], &offline[..one.len()]);
}

#[test]
fn a_sample_feedback_outside_a_loop_delays_by_one_sample() {
    let p = P::new(0.01)
        .node(1, "impulse", json!({}))
        .node(2, "feedback", json!({ "delay": "sample" }))
        .node(3, "audio_out", json!({}))
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    let r = render_at(&p, 64);
    assert_eq!(hits(&r.channels[0], 1e-6), vec![1]);
    assert_eq!(r.report.stats.sample_loops, vec![vec![path(2)]]);
}

#[test]
fn two_sample_feedbacks_in_one_loop_add_a_sample_each() {
    let p = P::new(0.05)
        .node(1, "impulse", json!({}))
        .node(2, "mix", json!({ "inputs": 2 }))
        .node(3, "gain", json!({}))
        .param(3, "gain", 20.0 * 0.5f64.log10())
        .node(4, "feedback", json!({ "delay": "sample" }))
        .node(5, "feedback", json!({ "delay": "sample" }))
        .node(6, "audio_out", json!({}))
        .wire(1, "out", 2, "in_0")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .wire(4, "out", 5, "in")
        .wire(5, "out", 2, "in_1")
        .wire(2, "out", 6, "in")
        .build();
    let r = render_at(&p, 64);
    assert_eq!(&hits(&r.channels[0], 1e-6)[..4], &[0, 2, 4, 6]);
    assert_eq!(r.report.stats.sample_loops.len(), 1);
    assert_eq!(r.report.stats.sample_loops[0].len(), 4);
}

#[test]
fn a_sample_loop_inside_a_block_loop_keeps_both_delays() {
    // The inner loop (2 → 3 → 4 → 2) is single-sample; the outer one
    // (2 → 7 → 8 → 2) goes through a block-mode Feedback.
    let p = P::new(0.05)
        .node(1, "impulse", json!({}))
        .node(2, "mix", json!({ "inputs": 3 }))
        .node(3, "gain", json!({}))
        .param(3, "gain", -40.0)
        .node(4, "feedback", json!({ "delay": "sample" }))
        .node(7, "gain", json!({}))
        .param(7, "gain", 20.0 * 0.5f64.log10())
        .node(8, "feedback", json!({}))
        .node(6, "audio_out", json!({}))
        .wire(1, "out", 2, "in_0")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .wire(4, "out", 2, "in_1")
        .wire(2, "out", 7, "in")
        .wire(7, "out", 8, "in")
        .wire(8, "out", 2, "in_2")
        .wire(2, "out", 6, "in")
        .build();
    let r = render_at(&p, 64);
    let loops = &r.report.stats.sample_loops;
    assert_eq!(loops, &vec![vec![path(2), path(3), path(4)]]);
    let x = &r.channels[0];
    // The quiet inner loop echoes every sample; the outer one returns a
    // block later at half the level.
    assert!((x[1] - 1e-2).abs() < 1e-6, "{}", x[1]);
    assert!((x[64] - 0.5).abs() < 1e-3, "{}", x[64]);
    // Neither loop's members run twice or get lost.
    assert_eq!(render_at(&p, 64).channels, r.channels);
}

// --- events and parameters ---------------------------------------------------------

/// Kick (1) → Delay (2, `d` samples) → Gain (3, −6 dB) → Feedback (4,
/// sample) → back into the Kick; the Kick feeds Audio Out (5) and its
/// trigger echo goes through Trigger To Gate (6) to a second output.
fn kick_loop(d: usize) -> Project {
    P::new(0.1)
        .node(1, "t_kick", json!({}))
        .node(2, "delay", json!({}))
        .param(2, "time", d as f64 / RATE)
        .param(2, "feedback", 0.0)
        .param(2, "mix", 1.0)
        .node(3, "gain", json!({}))
        .param(3, "gain", 20.0 * 0.5f64.log10())
        .node(4, "feedback", json!({ "delay": "sample" }))
        .node(5, "audio_out", json!({}))
        .node(7, "midi_in", json!({}))
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .wire(4, "out", 1, "in")
        .wire(1, "out", 5, "in")
        .wire(7, "midi", 1, "midi")
        .build()
}

#[test]
fn midi_reaches_a_loop_member_on_its_own_sample() {
    let note = |n: u8| MidiMsg::NoteOn {
        channel: 0,
        note: n,
        velocity: 100,
    };
    let midi = vec![(5u64, note(50)), (130, note(100)), (131, note(20))];
    let run = |block: usize| {
        let mut e = live(kick_loop(200), block);
        e.handle.play();
        play(&mut e, block, 1024 / block, &midi)
    };
    let x = run(64);
    assert_eq!(&hits(&x, 1e-6)[..5], &[5, 130, 131, 206, 331]);
    assert!((x[5] - 0.5).abs() < 1e-6);
    assert!((x[206] - 0.25).abs() < 1e-6, "{}", x[206]);
    for block in [1, 16, 256] {
        assert!(run(block) == x, "block {block}");
    }
}

#[test]
fn triggers_reach_a_loop_member_and_leave_it_on_their_sample() {
    // A Clock (8) every 0.002 s strikes the Kick; its echo output strikes
    // an Impulse (9) outside the loop.
    let mut p = kick_loop(30);
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(8), "clock").with_config(json!({ "unit": "seconds" })));
    if let Some(n) = p.graph.node_mut(NodeId(8)) {
        n.params.insert("interval".into(), Value::Float(0.002));
    }
    p.graph.nodes.push(NodeInstance::new(NodeId(9), "impulse"));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(10), "audio_out").with_config(json!({ "channel": 1 })));
    for (a, o, b, i) in [
        (8, "trigger", 1, "trigger"),
        (1, "echo", 9, "trigger"),
        (9, "out", 10, "in"),
    ] {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), o),
            Endpoint::new(NodeId(b), i),
        ));
    }
    let render = |block| render_at(&p, block);
    let r = render(64);
    assert!(!r.report.has_errors(), "{}", r.report);
    let kicks = hits(&r.channels[0], 0.9);
    assert_eq!(&kicks[..3], &[0, 96, 192], "{kicks:?}");
    // The echo leaves the loop on the trigger's own sample.
    assert_eq!(&hits(&r.channels[1], 0.5)[..3], &[0, 96, 192]);
    for block in [1, 7, 1024] {
        assert!(render(block).channels == r.channels, "block {block}");
    }
}

#[test]
fn a_parameter_moved_while_playing_ramps_the_same_at_any_block_size() {
    // The loop's Gain moves from −6 dB to −20 dB at sample 512; the ramp
    // (5 ms) runs sample by sample inside the loop.
    let run = |block: usize| {
        let mut e = live(echo_loop(40, "sample"), block);
        e.handle.play();
        let mut x = play(&mut e, block, 512 / block, &[]);
        assert!(e.handle.set_param(&path(4), "gain", Value::Float(-20.0)));
        x.extend(play(&mut e, block, 2048 / block, &[]));
        x
    };
    let x = run(64);
    // Echoes after the change come back quieter than before it.
    let before = x[41 * 12].abs();
    let after = x[41 * 30].abs();
    assert!(after < before * 0.2f32.powi(18), "{before} {after}");
    for block in [1, 8, 256] {
        assert!(run(block) == x, "block {block}");
    }
}

#[test]
fn the_loop_keeps_ringing_through_a_recompile() {
    let p = echo_loop(400, "sample");
    let mut e = live(p.clone(), 128);
    e.handle.play();
    let mut a = play(&mut e, 128, 8, &[]);
    // An unrelated edit recompiles the graph; the loop's state carries over.
    let mut edited = p.clone();
    edited
        .graph
        .nodes
        .push(NodeInstance::new(NodeId(99), "constant"));
    let report = e.handle.edit(GraphEdit::Replace(Box::new(edited)));
    assert_eq!(report.stats.sample_loops.len(), 1);
    a.extend(play(&mut e, 128, 8, &[]));
    let mut f = live(p, 128);
    f.handle.play();
    let b = play(&mut f, 128, 16, &[]);
    assert!(a == b, "the recompile changed the sound");
    assert!(a[401 * 2].abs() > 0.2, "{}", a[401 * 2]);
}

#[test]
fn a_muted_member_passes_its_input_through() {
    let mut p = echo_loop(20, "sample");
    if let Some(n) = p.graph.node_mut(NodeId(4)) {
        n.muted = true;
    }
    let x = render_at(&p, 64).channels[0].clone();
    // Unity round the loop: every echo is as loud as the first.
    assert_eq!(&hits(&x, 0.99)[..3], &[0, 21, 42]);
}

#[test]
fn loop_members_publish_their_telemetry() {
    // A Scope inside the loop, between the Gain and the Feedback: it sees
    // every sample, one at a time.
    let mut p = echo_loop(30, "sample");
    p.graph.nodes.push(NodeInstance::new(NodeId(20), "scope"));
    p.graph
        .wires
        .retain(|w| !(w.from.node == NodeId(4) && w.to.node == NodeId(5)));
    for (a, o, b, i) in [(4, "out", 20, "in"), (20, "out", 5, "in")] {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), o),
            Endpoint::new(NodeId(b), i),
        ));
    }
    let mut e = live(p, 128);
    assert_eq!(e.handle.report().stats.sample_loops[0].len(), 5);
    e.handle.play();
    play(&mut e, 128, 4, &[]);
    let t = e.handle.telemetry();
    let ring = t.scope(&path(20)).expect("the scope has a ring");
    assert_eq!(t.scope_position(&path(20)), Some(512));
    // The scope sits after the Gain: the first echo arrives 30 samples in
    // at half level, then every 31 samples at half the level before.
    // The ring comes oldest first from the write position: the 512 samples
    // played are its second half.
    let played = &ring[512..];
    assert_eq!(&hits(played, 1e-6)[..3], &[30, 61, 92]);
    assert!((played[30] - 0.5).abs() < 1e-6, "{}", played[30]);
    assert!((played[61] - 0.25).abs() < 1e-6, "{}", played[61]);
    let level = t.socket_level(&path(3), "out").unwrap_or(0.0);
    assert!(level > 0.01, "the delay's output level reads {level}");
}

// --- what can't run sample by sample ---------------------------------------------------

#[test]
fn a_spectral_node_in_the_loop_is_reported_and_the_loop_delays_a_block() {
    let mut p = echo_loop(100, "sample");
    // FFT (30) → IFFT (31) between the Gain and the Feedback.
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(30), "fft").with_config(json!({ "size": 256 })));
    p.graph.nodes.push(NodeInstance::new(NodeId(31), "ifft"));
    p.graph
        .wires
        .retain(|w| !(w.from.node == NodeId(4) && w.to.node == NodeId(5)));
    for (a, o, b, i) in [
        (4, "out", 30, "in"),
        (30, "spectrum", 31, "spectrum"),
        (31, "out", 5, "in"),
    ] {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), o),
            Endpoint::new(NodeId(b), i),
        ));
    }
    let r = render_at(&p, 64);
    assert!(r.report.stats.sample_loops.is_empty());
    let warned = |id: u64, text: &str| {
        r.report.diagnostics.iter().any(|d| {
            d.severity == Severity::Warning
                && d.node.as_ref() == Some(&path(id))
                && d.message.contains(text)
        })
    };
    assert!(warned(30, "FFT frames"), "{}", r.report);
    assert!(warned(31, "FFT frames"), "{}", r.report);
    assert!(warned(5, "one block"), "{}", r.report);
    assert!(r.channels[0].iter().all(|v| v.is_finite()));
}

#[test]
fn a_whole_input_node_in_the_loop_is_reported() {
    let mut p = echo_loop(100, "sample");
    p.graph.nodes.push(NodeInstance::new(NodeId(30), "reverse"));
    p.graph
        .wires
        .retain(|w| !(w.from.node == NodeId(4) && w.to.node == NodeId(5)));
    for (a, o, b, i) in [(4, "out", 30, "in"), (30, "out", 5, "in")] {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), o),
            Endpoint::new(NodeId(b), i),
        ));
    }
    let e = live(p, 64);
    let report = e.handle.report();
    assert!(report.stats.sample_loops.is_empty());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.node.as_ref() == Some(&path(30)) && d.message.contains("whole input")),
        "{report}"
    );
}

#[test]
fn a_cycle_without_feedback_beside_a_sample_loop_is_still_an_error() {
    let mut p = echo_loop(10, "sample");
    // Gain 40 ↔ Gain 41: a cycle with no Feedback node at all.
    for id in [40, 41] {
        p.graph.nodes.push(NodeInstance::new(NodeId(id), "gain"));
    }
    for (a, b) in [(40, 41), (41, 40)] {
        p.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), "out"),
            Endpoint::new(NodeId(b), "in"),
        ));
    }
    let r = render_at(&p, 64);
    assert!(r.report.has_errors());
    assert_eq!(r.report.stats.sample_loops.len(), 1);
    assert_eq!(&hits(&r.channels[0], 1e-6)[..2], &[0, 11]);
}

#[test]
fn a_feedback_node_set_to_nonsense_delays_a_block() {
    for delay in [json!("samples"), json!(1), json!(null)] {
        let mut p = echo_loop(100, "block");
        if let Some(n) = p.graph.node_mut(NodeId(5)) {
            n.config = json!({ "delay": delay });
        }
        let r = render_at(&p, 64);
        assert_eq!(&hits(&r.channels[0], 1e-6)[..2], &[0, 164], "{delay}");
    }
}

// --- inside voices ---------------------------------------------------------------

#[test]
fn each_voice_runs_its_own_loop() {
    // A Voices body: the voice's trigger strikes an Impulse into a loop.
    let mut def = GroupDef::new("Body");
    let n = |id: u64, kind: &str| NodeInstance::new(NodeId(id), kind);
    def.graph.nodes.extend([
        n(1, "voice"),
        n(2, "impulse"),
        n(3, "mix").with_config(json!({ "inputs": 2 })),
        n(4, "gain").with_param("gain", -6.0),
        n(5, "feedback").with_config(json!({ "delay": "sample" })),
    ]);
    for (a, o, b, i) in [
        (1, "trigger", 2, "trigger"),
        (2, "out", 3, "in_0"),
        (3, "out", 4, "in"),
        (4, "out", 5, "in"),
        (5, "out", 3, "in_1"),
    ] {
        def.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(a), o),
            Endpoint::new(NodeId(b), i),
        ));
    }
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(3), "out"),
    });
    let mut p = P::new(0.05)
        .node(10, "midi_in", json!({}))
        .node(11, "voices", json!({ "def": "body", "voices": 4 }))
        .node(12, "audio_out", json!({}))
        .wire(10, "midi", 11, "midi")
        .wire(11, "out", 12, "in")
        .build();
    p.group_defs.insert(GroupId::from("body"), def);
    let note = |n: u8| MidiMsg::NoteOn {
        channel: 0,
        note: n,
        velocity: 100,
    };
    let midi = [(3u64, note(60)), (70, note(64))];
    let run = |block: usize| {
        let mut e = live(p.clone(), block);
        let loops = e.handle.report().stats.sample_loops.clone();
        assert_eq!(loops.len(), 4, "{}", e.handle.report());
        assert!(loops.iter().all(|l| l.len() == 3), "{loops:?}");
        e.handle.play();
        play(&mut e, block, 512 / block, &midi)
    };
    let x = run(64);
    // Each note's own voice rings with a one-sample loop.
    assert!((x[3] - 1.0).abs() < 1e-6, "{}", x[3]);
    assert!((x[4] - 0.501).abs() < 1e-2, "{}", x[4]);
    assert!(x[71].abs() > 0.4, "{}", x[71]);
    for block in [1, 16] {
        assert!(run(block) == x, "block {block}");
    }
}

// --- the golden string -------------------------------------------------------------

/// Fundamental of `x` by autocorrelation (the lag with the most
/// self-similarity, refined by a parabola), searched from 50 to 1000 Hz.
fn pitch(x: &[f32]) -> f64 {
    let lags = (RATE / 1000.0) as usize..(RATE / 50.0) as usize;
    let ac = |lag: usize| -> f64 {
        x.iter()
            .zip(&x[lag..])
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum()
    };
    let scores: Vec<f64> = lags.clone().map(ac).collect();
    let k = (1..scores.len() - 1)
        .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
        .unwrap_or(1);
    let (y0, y1, y2) = (scores[k - 1], scores[k], scores[k + 1]);
    let d = 0.5 * (y0 - y2) / (y0 - 2.0 * y1 + y2);
    RATE / (lags.start as f64 + k as f64 + d)
}

#[test]
fn the_golden_string_plays_in_tune_only_with_single_sample_feedback() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/utility-feedback-sample.acproj"
    ))
    .expect("the golden project");
    let p = Project::from_json(&text).expect("it loads");
    let f0 = |p: &Project, block: usize| {
        let r = render_at(p, block);
        pitch(&r.channels[0][2_400..21_600])
    };
    for block in [32, 256, 1024] {
        let f = f0(&p, block);
        assert!((f - 220.0).abs() < 0.5, "block {block}: {f} Hz");
    }
    // The same loop with a block of delay is far flat, and depends on the
    // block size.
    let mut blocky = p.clone();
    if let Some(n) = blocky.graph.node_mut(NodeId(7)) {
        n.config = json!({ "delay": "block" });
    }
    let (b256, b32) = (f0(&blocky, 256), f0(&blocky, 32));
    assert!(b256 < 110.0, "{b256} Hz");
    assert!(b32 < 200.0 && b32 > b256 + 50.0, "{b32} Hz");
}
