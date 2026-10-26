//! The transport's song tools (ROADMAP M4.3, DECISIONS D206, D207): a
//! punch range that armed recorders follow to the sample, the click and the
//! count-in on the monitor path, and device outputs in the monitor role,
//! which a render leaves out.

mod common;

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use common::{P, registry};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

node! {
    kind: "t_rec_flag",
    name: "Recording Flag",
    category: Io,
    keywords: [],
    /// Writes 1 to device output 0 on every frame the transport says it
    /// records. Try this: nothing.
    pub struct TRecFlag {}
}
impl Node for TRecFlag {
    fn meta() -> NodeMeta {
        Self::base_meta().host_output()
    }
    fn describe(_: &Config) -> Signature {
        // An input of its own, so the compiler schedules it.
        Signature::new().input(SocketSpec::audio("in"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TRecFlag {}
    }
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        if ctx.transport.recording
            && let Some(out) = io.host.output_channel(0)
        {
            out.fill(1.0);
        }
    }
    fn reset(&mut self) {}
}

fn engine(project: Project, block: usize) -> Engine {
    let mut r = registry();
    r.register::<TRecFlag>();
    let (e, report) = Engine::new(
        project,
        Arc::new(r),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: 48_000.0,
            max_block: block,
            socket_telemetry: false,
            offline: false,
        },
    );
    assert!(!report.has_errors(), "{report}");
    e
}

/// 120 BPM at 48 kHz.
const BEAT: usize = 24_000;

#[test]
fn armed_recorders_follow_the_punch_range_to_the_sample() {
    for block in [64, 100] {
        let mut host = NullHost::new(engine(P::new().node(1, "t_rec_flag").build(), block), 0, 2);
        host.handle().set_punch(Some((100, 300)));
        host.handle().set_recording(true);
        host.handle().play();
        let out = host.run(512, block);
        let on: Vec<usize> = (0..512).filter(|&i| out[0][i] == 1.0).collect();
        assert_eq!(on.first(), Some(&100), "block {block}");
        assert_eq!(on.last(), Some(&299), "block {block}");
        assert_eq!(on.len(), 200, "block {block}");
    }
    // Without a punch range, recording is recording.
    let mut host = NullHost::new(engine(P::new().node(1, "t_rec_flag").build(), 64), 0, 2);
    host.handle().set_recording(true);
    host.handle().play();
    assert!(host.run(256, 64)[0].iter().all(|&s| s == 1.0));
}

fn loud(samples: &[f32]) -> bool {
    samples.iter().any(|s| s.abs() > 0.01)
}

#[test]
fn the_click_plays_on_every_beat_live_and_never_in_a_render() {
    let project = P::new().node(1, "t_out").build();
    let mut host = NullHost::new(engine(project.clone(), 256), 0, 2);
    host.handle().set_metronome(true);
    host.handle().play();
    let out = host.run(2 * BEAT + 256, 256);
    for beat in [0, BEAT, 2 * BEAT] {
        assert!(loud(&out[0][beat..beat + 200]), "a click at {beat}");
        assert!(loud(&out[1][beat..beat + 200]));
    }
    assert!(!loud(&out[0][2_000..BEAT]), "silence between beats");
    // Stopped, it's quiet once the last click has rung.
    host.handle().stop();
    let out = host.run(BEAT * 2, 256);
    assert!(!loud(&out[0][2_000..]));

    // A render leaves it out even when the project has it on.
    let mut p = project;
    p.settings.metronome = true;
    let rendered = render(
        &p,
        Arc::new(registry()),
        Arc::new(NoResources),
        &RenderOptions {
            to: Some(1.0),
            ..Default::default()
        },
    );
    assert!(!loud(&rendered.channels[0]));
}

#[test]
fn a_count_in_clicks_the_bars_then_starts_recording() {
    let mut host = NullHost::new(engine(P::new().node(1, "t_rec_flag").build(), 256), 0, 2);
    host.handle().set_count_in(1);
    host.handle().set_recording(true);
    host.handle().play();
    // One bar of 4/4 at 120 BPM: four clicks, then the transport runs.
    let mut out = host.run(4 * BEAT - 256, 256);
    assert!(host.handle().is_counting_in());
    assert!(!host.handle().is_playing());
    assert_eq!(host.handle().position(), 0);
    for beat in 0..3 {
        assert!(
            loud(&out[1][beat * BEAT..beat * BEAT + 200]),
            "count-in beat {beat}"
        );
    }
    assert!(loud(&out[1][3 * BEAT..3 * BEAT + 200]), "count-in beat 3");
    assert!(out[0].iter().all(|&s| s != 1.0), "nothing records yet");
    out = host.run(256, 256);
    assert!(out[0].iter().all(|&s| s != 1.0));
    let out = host.run(512, 256);
    assert!(!host.handle().is_counting_in());
    assert!(host.handle().is_playing());
    assert_eq!(host.handle().position(), 512);
    assert!(
        out[0].iter().all(|&s| s == 1.0),
        "recording from the first frame"
    );
    // Playing (not recording) starts at once.
    let mut host = NullHost::new(engine(P::new().build(), 256), 0, 2);
    host.handle().set_count_in(2);
    host.handle().play();
    host.run(256, 256);
    assert!(host.handle().is_playing());
}

#[test]
fn a_monitor_output_plays_live_and_a_render_leaves_it_out() {
    let mut out = NodeInstance::new(NodeId(2), "t_out");
    out.config = json!({ "role": "monitor" });
    let project = P::new()
        .node(1, "t_src")
        .node_with(out)
        .wire(1, "out", 2, "in")
        .build();
    let mut host = NullHost::new(engine(project.clone(), 64), 0, 2);
    host.handle().play();
    let live = host.run(256, 64);
    assert!(loud(&live[0]));
    let rendered = render(
        &project,
        Arc::new(registry()),
        Arc::new(NoResources),
        &RenderOptions {
            to: Some(0.01),
            ..Default::default()
        },
    );
    assert!(!loud(&rendered.channels[0]));
}
