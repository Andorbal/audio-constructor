//! Phase 2 engine features: latency compensation, whole-input baking and
//! disabling, the multi-core scheduler, and the Spectrum and Buffer signal
//! types. Every `Processor::process` runs under the allocation guard.

mod common;

use std::sync::Arc;

use ac_core::*;
use ac_engine::*;
use assert_no_alloc::AllocDisabler;
use common::{P, path};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

// --- test nodes ------------------------------------------------------------------

node! {
    kind: "t_lat",
    name: "Lat",
    category: Utility,
    keywords: [],
    /// Delays mono audio by config `samples` and reports it. Try this: nothing.
    pub struct TLat { line: Vec<f32>, w: usize }
}
impl Node for TLat {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").channels(Channels::Fixed(1)))
            .output(SocketSpec::audio("out").channels(Channels::Fixed(1)))
    }
    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let n = config.get_u64_or("samples", 0) as usize;
        TLat {
            line: vec![0.0; n],
            w: 0,
        }
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(0);
        let src = input.channel(0);
        let mut out = io.outputs.audio(0);
        let dst = out.channel(0);
        for (i, o) in dst.iter_mut().enumerate() {
            let x = src.get(i).copied().unwrap_or(0.0);
            if self.line.is_empty() {
                *o = x;
            } else {
                *o = self.line[self.w];
                self.line[self.w] = x;
                self.w = (self.w + 1) % self.line.len();
            }
        }
    }
    fn latency(&self) -> u32 {
        self.line.len() as u32
    }
    fn reset(&mut self) {
        self.line.fill(0.0);
        self.w = 0;
    }
}

node! {
    kind: "t_live",
    name: "Live",
    category: Io,
    keywords: [],
    /// A live source: device input channel 0. Try this: nothing.
    pub struct TLive {}
}
impl Node for TLive {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::audio("out").channels(Channels::Fixed(1)))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TLive {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let src = io.host.input_channel(0).to_owned_array();
        let mut out = io.outputs.audio(0);
        for (o, s) in out.channel(0).iter_mut().zip(src.iter()) {
            *o = *s;
        }
    }
    fn reset(&mut self) {}
}

/// `to_owned` without allocating: copy into a fixed array (test blocks are
/// at most 1024 frames).
trait ToArray {
    fn to_owned_array(&self) -> [f32; 1024];
}
impl ToArray for &[f32] {
    fn to_owned_array(&self) -> [f32; 1024] {
        let mut a = [0.0; 1024];
        let n = self.len().min(1024);
        a[..n].copy_from_slice(&self[..n]);
        a
    }
}

node! {
    kind: "t_double",
    name: "Double",
    category: Time,
    keywords: [],
    /// Whole-input: out = 2 × in, plus 1000 × the frame count at frame 0.
    /// Try this: nothing.
    pub struct TDouble {}
}
impl Node for TDouble {
    fn meta() -> NodeMeta {
        Self::base_meta().whole_input()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in"))
            .input(SocketSpec::float("gain", Unit::Generic, 2.0))
            .output(SocketSpec::audio("out"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TDouble {}
    }
    fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}
    fn render_whole(_: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs {
        let gain = inputs.float_at(1, 0, 2.0);
        let chans = inputs
            .audio(0)
            .iter()
            .map(|c| c.iter().map(|v| v * gain).collect())
            .collect();
        let _ = ctx;
        let mut out = WholeOutputs::with_outputs(1);
        out.set(0, WholeValue::Audio(chans));
        out
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_spec",
    name: "Spec",
    category: Spectral,
    keywords: [],
    /// Emits a 16-point frame every 8 samples whose bin 0 is the frame's
    /// sample position; reports config `latency`. Try this: nothing.
    pub struct TSpec {
        latency: u32,
    }
}
impl Node for TSpec {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::spectrum("out").shape(16, 8))
    }
    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TSpec {
            latency: config.get("latency").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        }
    }
    fn latency(&self) -> u32 {
        self.latency
    }
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let pos = ctx.transport.position;
        if let Some(out) = io.outputs.spectrum(0) {
            for i in 0..ctx.frames {
                let at = pos + i as u64;
                if at % 8 == 7
                    && let Some(bins) = out.push(i as u32)
                {
                    bins[0] = at as f32;
                }
            }
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_spec_pass",
    name: "SpecPass",
    category: Spectral,
    keywords: [],
    /// Copies frames through, adding 0.5 to bin 0. Try this: nothing.
    pub struct TSpecPass {}
}
impl Node for TSpecPass {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("in"))
            .output(SocketSpec::spectrum("out"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TSpecPass {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(0);
        if let Some(out) = io.outputs.spectrum(0) {
            for (offset, bins) in input.iter() {
                if let Some(dst) = out.push(offset) {
                    dst.copy_from_slice(bins);
                    dst[0] += 0.5;
                }
            }
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_spec_mix",
    name: "SpecMix",
    category: Spectral,
    keywords: [],
    /// Passes audio `in` through; reads a spectrum it ignores. Try this:
    /// nothing.
    pub struct TSpecMix {}
}
impl Node for TSpecMix {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("spec"))
            .input(SocketSpec::audio("in"))
            .output(SocketSpec::audio("out"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TSpecMix {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(1);
        let mut out = io.outputs.audio(0);
        for c in 0..out.channels() {
            out.channel(c)
                .copy_from_slice(input.channel(c.min(input.channels() - 1)));
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_spec_out",
    name: "SpecOut",
    category: Spectral,
    keywords: [],
    /// Writes each frame's bin 0 to device output 0 at its offset.
    /// Try this: nothing.
    pub struct TSpecOut {}
}
impl Node for TSpecOut {
    fn meta() -> NodeMeta {
        Self::base_meta().host_output()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().input(SocketSpec::spectrum("in"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TSpecOut {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(0);
        if let Some(out) = io.host.output_channel(0) {
            for (offset, bins) in input.iter() {
                if let Some(o) = out.get_mut(offset as usize) {
                    *o = bins[0];
                }
            }
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_buf",
    name: "Buf",
    category: Io,
    keywords: [],
    /// Outputs a 3-frame buffer once the transport passes sample 100.
    /// Try this: nothing.
    pub struct TBuf { data: Arc<AudioData> }
}
impl Node for TBuf {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::buffer("out"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TBuf {
            data: Arc::new(AudioData {
                sample_rate: 48_000,
                channels: vec![vec![0.25, 0.5, 0.75]],
            }),
        }
    }
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let ready = ctx.transport.position >= 100;
        io.outputs.set_buffer(0, ready.then_some(&self.data));
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_buf_out",
    name: "BufOut",
    category: Io,
    keywords: [],
    /// Writes the input buffer's frame count plus its last sample to device
    /// output 0 (0 when no buffer). Try this: nothing.
    pub struct TBufOut {}
}
impl Node for TBufOut {
    fn meta() -> NodeMeta {
        Self::base_meta().host_output()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().input(SocketSpec::buffer("in"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TBufOut {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let v = io.inputs.buffer(0).map_or(0.0, |b| {
            b.frames() as f32 + b.channels[0].last().copied().unwrap_or(0.0)
        });
        if let Some(out) = io.host.output_channel(0) {
            out.fill(v);
        }
    }
    fn reset(&mut self) {}
}

fn registry() -> Arc<Registry> {
    let mut r = common::registry();
    ac_nodes::register_all(&mut r);
    r.register::<TLat>()
        .register::<TLive>()
        .register::<TDouble>()
        .register::<TSpec>()
        .register::<TSpecPass>()
        .register::<TSpecMix>()
        .register::<TSpecOut>()
        .register::<TBuf>()
        .register::<TBufOut>();
    Arc::new(r)
}

fn offline(project: &Project, block: usize, seconds: f64) -> Rendered {
    render(
        project,
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(block),
            to: Some(seconds),
            ..Default::default()
        },
    )
}

fn live(project: Project, block: usize) -> Engine {
    Engine::new(
        project,
        registry(),
        Arc::new(NoResources),
        EngineOptions {
            sample_rate: 48_000.0,
            max_block: block,
            socket_telemetry: true,
            offline: false,
        },
    )
    .0
}

/// Run `blocks` blocks of `block` frames and return device output 0.
fn run(engine: &mut Engine, block: usize, blocks: usize, input: Option<&[f32]>) -> Vec<f32> {
    let mut out = vec![0.0f32; 2 * block];
    let mut all = Vec::new();
    for b in 0..blocks {
        let inp: Vec<f32> = match input {
            Some(x) => (0..block)
                .map(|i| x.get(b * block + i).copied().unwrap_or(0.0))
                .collect(),
            None => Vec::new(),
        };
        let mut io = HostIo {
            frames: block,
            input: &inp,
            in_channels: usize::from(input.is_some()),
            output: &mut out,
            out_channels: 2,
            midi: &[],
            midi_out: None,
            controls: &[],
            osc_out: None,
        };
        engine.process(&mut io);
        all.extend_from_slice(&out[..block]);
    }
    all
}

// --- latency compensation ---------------------------------------------------------

/// src → t_lat(100) → sum.a ; src → sum.b ; sum → out.
fn merge(samples: u64) -> Project {
    P::new()
        .node(1, "t_src")
        .node_with(NodeInstance::new(NodeId(2), "t_lat").with_config(json!({ "samples": samples })))
        .node(3, "t_sum")
        .node(4, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(1, "out", 3, "b")
        .wire(3, "out", 4, "in")
        .build()
}

#[test]
fn merging_paths_are_lined_up() {
    for block in [1, 64, 1000] {
        let r = offline(&merge(100), block, 0.02);
        let out = &r.channels[0];
        // Both inputs are the ramp delayed by 100: out = 2 × (t − 100).
        for (t, &v) in out.iter().enumerate() {
            let want = if t < 100 { 0.0 } else { 2.0 * (t - 100) as f32 };
            assert_eq!(v, want, "block {block} frame {t}");
        }
        assert_eq!(r.report.stats.latency, 100);
        assert_eq!(
            r.report.stats.compensation,
            vec![(path(3), "b".to_string(), 100)]
        );
        assert_eq!(r.report.stats.node_latency, vec![(path(2), 100)]);
    }
}

#[test]
fn sample_rate_float_inputs_are_compensated_too() {
    // amount (a sample-rate float from the ramp) is delayed to match `a`.
    let p = P::new()
        .node(1, "t_src")
        .node_with(NodeInstance::new(NodeId(2), "t_lat").with_config(json!({ "samples": 10 })))
        .node(3, "t_sum")
        .node(4, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(1, "out", 3, "amount")
        .wire(3, "out", 4, "in")
        .build();
    let r = offline(&p, 16, 0.002);
    for (t, &v) in r.channels[0].iter().enumerate().skip(10) {
        assert_eq!(v, 2.0 * (t - 10) as f32, "frame {t}");
    }
}

#[test]
fn compensation_survives_recompiles_without_a_dropout() {
    let block = 64;
    let mut a = live(merge(100), block);
    let mut b = live(merge(100), block);
    a.handle.play();
    b.handle.play();
    let first_a = run(&mut a, block, 5, None);
    let first_b = run(&mut b, block, 5, None);
    assert_eq!(first_a, first_b);
    // Recompile `b` mid-stream with an unrelated node added.
    let mut p = merge(100);
    p.graph.nodes.push(NodeInstance::new(NodeId(9), "t_block"));
    b.handle.edit(GraphEdit::Replace(Box::new(p)));
    assert_eq!(run(&mut a, block, 5, None), run(&mut b, block, 5, None));
}

#[test]
fn latency_probes_report_path_latency() {
    // src → t_lat(300) → Latency(probe, 0) → Readout-free sink.
    let p = P::new()
        .node(1, "t_src")
        .node_with(NodeInstance::new(NodeId(2), "t_lat").with_config(json!({ "samples": 300 })))
        .node_with(
            NodeInstance::new(NodeId(3), "latency")
                .with_config(json!({ "samples": 20, "channels": 1 })),
        )
        .node(4, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .build();
    let mut e = live(p.clone(), 64);
    e.handle.play();
    run(&mut e, 64, 2, None);
    let tele = e.handle.telemetry();
    assert_eq!(tele.socket_value(&path(3), "path_latency"), Some(320.0));
    assert_eq!(e.handle.report().stats.latency, 320);
    // A recompile that changes the path rebuilds the probe with the new value.
    let mut q = p;
    q.graph.nodes[1].config = json!({ "samples": 100 });
    e.handle.edit(GraphEdit::Replace(Box::new(q)));
    run(&mut e, 64, 2, None);
    let tele = e.handle.telemetry();
    assert_eq!(tele.socket_value(&path(3), "path_latency"), Some(120.0));
}

// --- whole-input nodes -------------------------------------------------------------

/// src → reverse → out, 480 frames long.
fn reversed() -> Project {
    let mut p = P::new()
        .node(1, "t_src")
        .node(2, "reverse")
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.settings.length = 0.01;
    p
}

#[test]
fn offline_renders_bake_whole_input_nodes_first() {
    for block in [1, 64, 256] {
        let r = offline(&reversed(), block, 0.01);
        let out = &r.channels[0];
        assert_eq!(out.len(), 480);
        for (t, &v) in out.iter().enumerate() {
            assert_eq!(v, (479 - t) as f32, "block {block} frame {t}");
        }
        assert_eq!(
            r.report.stats.whole_input,
            vec![(path(2), WholeState::Baked)]
        );
    }
}

#[test]
fn live_whole_input_nodes_are_silent_until_baked_then_stream_the_bake() {
    let block = 64;
    let mut e = live(reversed(), block);
    assert_eq!(e.handle.bakes()[0].state, WholeState::Pending);
    e.handle.play();
    let before = run(&mut e, block, 2, None);
    assert!(before.iter().all(|&v| v == 0.0));
    // A host runs the jobs on a worker thread; here, on this one.
    let jobs = e.handle.take_bake_jobs();
    assert_eq!(jobs.len(), 1);
    assert!(e.handle.take_bake_jobs().is_empty(), "already handed out");
    assert!(e.handle.is_baking(&path(2)));
    let result = std::thread::spawn({
        let job = jobs[0].clone();
        move || job.run()
    })
    .join()
    .unwrap();
    assert!(e.handle.finish_bake(result));
    assert_eq!(e.handle.bakes()[0].state, WholeState::Baked);
    e.handle.seek(0);
    let after = run(&mut e, block, 8, None);
    for (t, &v) in after.iter().enumerate().take(480) {
        assert_eq!(v, (479 - t) as f32, "frame {t}");
    }
    assert!(after[480..].iter().all(|&v| v == 0.0), "past the bake");
}

#[test]
fn live_sources_disable_whole_input_nodes_but_offline_renders_bake_them() {
    let mut p = P::new()
        .node(1, "audio_in")
        .node(2, "reverse")
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.settings.length = 0.01;
    let e = live(p.clone(), 64);
    let spec = &e.handle.bakes()[0];
    assert!(
        matches!(&spec.state, WholeState::Disabled { sources } if sources == &["Audio In".to_string()])
    );
    assert!(
        e.handle
            .report()
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Info && d.message.contains("disabled while live"))
    );
    let mut e = e;
    assert!(e.handle.take_bake_jobs().is_empty());
    // Offline, the input is known: it bakes and plays backwards.
    let input: Vec<f32> = (0..480).map(|i| i as f32 * 0.001).collect();
    let r = render(
        &p,
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            input: Some(AudioData {
                sample_rate: 48_000,
                channels: vec![input.clone()],
            }),
            ..Default::default()
        },
    );
    for (t, &v) in r.channels[0].iter().enumerate() {
        assert_eq!(v, input[479 - t], "frame {t}");
    }
}

#[test]
fn upstream_edits_invalidate_bakes_and_the_old_bake_plays_meanwhile() {
    let block = 64;
    let mut p = P::new()
        .node(1, "t_src")
        .node(2, "t_double")
        .node(3, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .build();
    p.settings.length = 0.005;
    let mut e = live(p, block);
    e.handle.bake_all();
    assert_eq!(e.handle.bakes()[0].state, WholeState::Baked);
    let key = e.handle.bakes()[0].key;

    // An unrelated node doesn't touch the bake.
    let mut q = e.handle.project().clone();
    q.graph.nodes.push(NodeInstance::new(NodeId(8), "t_block"));
    e.handle.edit(GraphEdit::Replace(Box::new(q)));
    assert_eq!(e.handle.bakes()[0].key, key);
    assert_eq!(e.handle.bakes()[0].state, WholeState::Baked);

    // A parameter upstream marks bakes dirty; refreshing makes it stale.
    assert!(!e.handle.bakes_dirty());
    e.handle.set_param(&path(1), "scale", Value::Float(3.0));
    assert!(e.handle.bakes_dirty());
    e.handle.refresh();
    assert_eq!(e.handle.bakes()[0].state, WholeState::Stale);
    e.handle.play();
    let stale = run(&mut e, block, 1, None);
    assert_eq!(stale[10], 20.0, "old bake: 2 × 10");
    e.handle.bake_all();
    assert_eq!(e.handle.bakes()[0].state, WholeState::Baked);
    e.handle.seek(0);
    let fresh = run(&mut e, block, 1, None);
    assert_eq!(fresh[10], 60.0, "new bake: 2 × 3 × 10");

    // The node's own unwired params are part of the key too.
    let mut q = e.handle.project().clone();
    q.graph.nodes[1]
        .params
        .insert("gain".into(), Value::Float(5.0));
    e.handle.edit(GraphEdit::Replace(Box::new(q)));
    assert_eq!(e.handle.bakes()[0].state, WholeState::Stale);
}

#[test]
fn nested_whole_input_nodes_bake_in_order() {
    // src → reverse → reverse → out is the identity.
    let mut p = P::new()
        .node(1, "t_src")
        .node(2, "reverse")
        .node(3, "reverse")
        .node(4, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .build();
    p.settings.length = 0.01;
    let r = offline(&p, 64, 0.01);
    for (t, &v) in r.channels[0].iter().enumerate() {
        assert_eq!(v, t as f32, "frame {t}");
    }
    // Live: the outer node waits for the inner one.
    let mut e = live(p, 64);
    let jobs = e.handle.take_bake_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].path, path(2));
    let r = jobs[0].run();
    e.handle.finish_bake(r);
    let jobs = e.handle.take_bake_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].path, path(3));
}

#[test]
fn stale_bake_results_are_dropped() {
    let mut e = live(reversed(), 64);
    let job = e.handle.take_bake_jobs().remove(0);
    let mut q = e.handle.project().clone();
    q.graph.nodes[0]
        .params
        .insert("scale".into(), Value::Float(2.0));
    e.handle.edit(GraphEdit::Replace(Box::new(q)));
    assert!(!e.handle.finish_bake(job.run()), "key changed meanwhile");
    assert_eq!(e.handle.bakes()[0].state, WholeState::Pending);
    assert_eq!(e.handle.take_bake_jobs().len(), 1);
}

// --- multi-core scheduler -----------------------------------------------------------

/// `chains` independent chains of real DSP mixed to the output.
fn wide(chains: usize) -> Project {
    let mut p = P::new();
    let mix = 1000;
    p = p.node_with(NodeInstance::new(NodeId(mix), "mix").with_config(json!({ "inputs": chains })));
    p = p.node(mix + 1, "audio_out");
    p = p.wire(mix, "out", mix + 1, "in");
    for c in 0..chains as u64 {
        let base = 10 * (c + 1);
        p = p
            .node(base, "oscillator")
            .param(base, "freq", 110.0 * (c + 1) as f64)
            .param(base, "shape", "saw")
            .node(base + 1, "biquad")
            .param(base + 1, "cutoff", 400.0 + 300.0 * c as f64)
            .node(base + 2, "delay")
            .param(base + 2, "time", 0.003 * (c + 1) as f64)
            .node(base + 3, "gain")
            .param(base + 3, "gain", -12.0)
            .wire(base, "out", base + 1, "in")
            .wire(base + 1, "out", base + 2, "in")
            .wire(base + 2, "out", base + 3, "in")
            .wire(base + 3, "out", mix, &format!("in_{c}"));
    }
    p.build()
}

#[test]
fn the_worker_pool_gives_bit_identical_results() {
    let block = 128;
    let blocks = 40;
    let reference = {
        let mut e = live(wide(8), block);
        e.handle.play();
        run(&mut e, block, blocks, None)
    };
    assert!(reference.iter().any(|&v| v != 0.0));
    for workers in [1, 3] {
        let mut e = live(wide(8), block);
        e.processor
            .set_workers(Some(Arc::new(RtPool::new(workers, None, None))));
        e.handle.play();
        assert_eq!(
            run(&mut e, block, blocks, None),
            reference,
            "{workers} workers"
        );
        assert_eq!(
            e.handle
                .status()
                .cores
                .load(std::sync::atomic::Ordering::Relaxed),
            workers as u32 + 1
        );
    }
    // Waves: each chain is one job and the eight run side by side, then the
    // mix and the output.
    let e = live(wide(8), block);
    assert_eq!(e.handle.report().stats.waves, 3);
    assert_eq!(e.handle.report().stats.chains, 9);
}

#[test]
fn per_core_load_is_published() {
    fn clock() -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos() as u64
    }
    let block = 256;
    let mut e = live(wide(16), block);
    e.processor.set_clock(Some(clock));
    e.processor
        .set_workers(Some(Arc::new(RtPool::new(2, None, Some(clock)))));
    e.handle.play();
    run(&mut e, block, 20, None);
    let loads = e.handle.status().core_loads();
    assert_eq!(loads.len(), 3);
    assert!(loads.iter().sum::<f32>() > 0.0, "{loads:?}");
}

// --- Spectrum and Buffer ------------------------------------------------------------

#[test]
fn spectrum_frames_flow_with_their_offsets_and_shapes_follow() {
    for block in [1, 5, 64] {
        let p = P::new()
            .node(1, "t_spec")
            .node(2, "t_spec_pass")
            .node(3, "t_spec_out")
            .wire(1, "out", 2, "in")
            .wire(2, "out", 3, "in")
            .build();
        let r = offline(&p, block, 0.001);
        let out = &r.channels[0];
        for (t, &v) in out.iter().enumerate() {
            let want = if t % 8 == 7 { t as f32 + 0.5 } else { 0.0 };
            assert_eq!(v, want, "block {block} frame {t}");
        }
    }
}

#[test]
fn spectrum_paths_carry_latency_and_audio_lines_up_with_them() {
    // t_spec (latency 7) → mix.spec ; src → mix.in ; mix → out.
    let p = P::new()
        .node_with(NodeInstance::new(NodeId(1), "t_spec").with_config(json!({ "latency": 7 })))
        .node(2, "t_src")
        .node(3, "t_spec_mix")
        .node(4, "t_out")
        .wire(1, "out", 3, "spec")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .build();
    let r = offline(&p, 16, 0.001);
    assert_eq!(
        r.report.stats.compensation,
        vec![(path(3), "in".to_string(), 7)]
    );
    assert_eq!(r.report.stats.latency, 7);
    for (t, &v) in r.channels[0].iter().enumerate().skip(7) {
        assert_eq!(v, (t - 7) as f32, "frame {t}");
    }
}

#[test]
fn a_dry_path_cancels_against_fft_then_ifft() {
    // osc → fft → ifft → math.a ; osc → math.b ; a − b → out. The engine
    // delays the dry path by the FFT's window, so the difference is silence.
    let p = P::new()
        .node_with(NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 220.0))
        .node(2, "fft")
        .node(3, "ifft")
        .node_with(
            NodeInstance::new(NodeId(4), "math")
                .with_param("op", ac_core::Value::Text("subtract".into())),
        )
        .node(5, "t_out")
        .wire(1, "out", 2, "in")
        .wire(2, "spectrum", 3, "spectrum")
        .wire(3, "out", 4, "a")
        .wire(1, "out", 4, "b")
        .wire(4, "out", 5, "in")
        .build();
    let r = offline(&p, 64, 0.2);
    assert_eq!(
        r.report.stats.compensation,
        vec![(path(4), "b".to_string(), 2047)]
    );
    let out = &r.channels[0];
    let peak = out[4096..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 1e-4, "residual {peak}");
}

#[test]
fn spectrum_wires_are_type_checked() {
    let p = P::new()
        .node(1, "t_spec")
        .node(2, "t_out")
        .node(3, "t_src")
        .node(4, "t_spec_out")
        .wire(1, "out", 2, "in")
        .wire(3, "out", 4, "in")
        .build();
    let r = offline(&p, 64, 0.001);
    let errors: Vec<_> = r.report.errors().collect();
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn buffers_pass_handles_and_persist_across_blocks() {
    let p = P::new()
        .node(1, "t_buf")
        .node(2, "t_buf_out")
        .wire(1, "out", 2, "in")
        .build();
    let r = offline(&p, 64, 0.005);
    let out = &r.channels[0];
    // Frames 0..128 see nothing (the buffer appears in the block at 128).
    assert!(out[..128].iter().all(|&v| v == 0.0));
    assert!(out[128..].iter().all(|&v| v == 3.75));
}

// --- Voices ---------------------------------------------------------------------------

fn on(note: u8) -> MidiMsg {
    MidiMsg::NoteOn {
        channel: 0,
        note,
        velocity: 100,
    }
}

fn off(note: u8) -> MidiMsg {
    MidiMsg::NoteOff {
        channel: 0,
        note,
        velocity: 0,
    }
}

/// A body whose output is `gate + level`: summed over voices it counts the
/// sounding notes, plus `voices × level`.
fn counting_voices(voices: u64, steal: &str) -> Project {
    let mut def = GroupDef::new("Counter");
    def.graph.nodes.push(NodeInstance::new(NodeId(1), "voice"));
    def.graph.nodes.push(NodeInstance::new(NodeId(2), "t_add"));
    def.graph.wires.push(Wire::new(
        Endpoint::new(NodeId(1), "gate"),
        Endpoint::new(NodeId(2), "a"),
    ));
    def.inputs.push(GroupInput {
        name: "level".into(),
        ty: SignalType::Float,
        unit: Unit::Generic,
        default: Some(Value::Float(0.0)),
        range: None,
        targets: vec![Endpoint::new(NodeId(2), "b")],
    });
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Float,
        source: Endpoint::new(NodeId(2), "out"),
    });
    let mut p = P::new()
        .node(1, "midi_in")
        .node_with(
            NodeInstance::new(NodeId(2), "voices")
                .with_config(json!({ "def": "counter", "voices": voices, "steal": steal })),
        )
        .node(3, "t_sum")
        .node(4, "t_out")
        .wire(1, "midi", 2, "midi")
        .wire(2, "out", 3, "amount")
        .wire(3, "out", 4, "in")
        .build();
    p.group_defs.insert(GroupId::from("counter"), def);
    p.settings.length = 0.01;
    p
}

fn render_midi(p: &Project, midi: Vec<(u64, MidiMsg)>, block: usize) -> Rendered {
    render(
        p,
        registry(),
        Arc::new(NoResources),
        &RenderOptions {
            block_size: Some(block),
            midi,
            ..Default::default()
        },
    )
}

#[test]
fn voices_play_one_note_each_and_outputs_sum() {
    let midi = vec![(0, on(60)), (100, on(64)), (200, on(67)), (300, off(64))];
    for block in [1, 64, 256] {
        let r = render_midi(&counting_voices(4, "oldest"), midi.clone(), block);
        assert!(!r.report.has_errors(), "{}", r.report);
        let out = &r.channels[0];
        assert_eq!(out[50], 1.0, "block {block}");
        assert_eq!(out[150], 2.0);
        assert_eq!(out[250], 3.0);
        assert_eq!(out[350], 2.0);
    }
}

#[test]
fn voices_inputs_reach_every_voice() {
    let mut p = counting_voices(4, "oldest");
    p.graph.nodes[1]
        .params
        .insert("level".into(), Value::Float(0.5));
    let r = render_midi(&p, vec![(0, on(60))], 64);
    // One note plus 4 × 0.5.
    assert_eq!(r.channels[0][100], 3.0);
    // And set_param on the Voices node reaches them live.
    let mut e = live(counting_voices(4, "oldest"), 64);
    assert!(e.handle.set_param(&path(2), "level", Value::Float(0.25)));
    e.handle.play();
    let out = run(&mut e, 64, 4, None);
    assert_eq!(out[255], 1.0, "4 × 0.25 after smoothing");
}

#[test]
fn busy_voices_are_stolen_or_notes_ignored() {
    let midi = vec![(0, on(60)), (10, on(62)), (20, on(64))];
    let r = render_midi(&counting_voices(2, "oldest"), midi.clone(), 64);
    assert_eq!(r.channels[0][100], 2.0, "still two voices sounding");
    let r = render_midi(&counting_voices(2, "none"), midi, 64);
    assert_eq!(r.channels[0][100], 2.0);
}

#[test]
fn voice_paths_are_namespaced_and_stray_voice_nodes_warn() {
    let e = live(counting_voices(3, "oldest"), 64);
    let report = e.handle.report();
    // allocator + 3 × (voice, add) + sum + midi_in + t_sum + t_out.
    assert_eq!(report.stats.nodes, 1 + 6 + 1 + 3);
    assert!(!report.has_errors(), "{report}");
    let p = P::new().node(1, "voice").build();
    let e = live(p, 64);
    assert!(
        e.handle
            .report()
            .diagnostics
            .iter()
            .any(|d| d.message.contains("inside the body of a Voices node"))
    );
}

/// A 16-voice subtractive synth: Voice → Oscillator → Biquad → Gain (ADSR).
fn synth(voices: u64) -> Project {
    let mut def = GroupDef::new("Synth voice");
    let n = |id: u64, kind: &str| NodeInstance::new(NodeId(id), kind);
    def.graph.nodes.extend([
        n(1, "voice"),
        n(2, "oscillator").with_param("shape", "saw"),
        n(3, "biquad").with_param("cutoff", 1200.0),
        n(4, "adsr").with_param("release", 0.05),
        n(5, "math").with_param("op", "multiply"),
    ]);
    for (f, o, t, i) in [
        (1, "freq", 2, "freq"),
        (2, "out", 3, "in"),
        (1, "gate", 4, "gate"),
        (3, "out", 5, "a"),
        (4, "out", 5, "b"),
    ] {
        def.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(f), o),
            Endpoint::new(NodeId(t), i),
        ));
    }
    def.outputs.push(GroupOutput {
        name: "out".into(),
        ty: SignalType::Audio,
        source: Endpoint::new(NodeId(5), "out"),
    });
    let mut p = P::new()
        .node(1, "midi_in")
        .node_with(
            NodeInstance::new(NodeId(2), "voices")
                .with_config(json!({ "def": "synth", "voices": voices })),
        )
        .node(3, "gain")
        .param(3, "gain", -18.0)
        .node(4, "audio_out")
        .wire(1, "midi", 2, "midi")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .build();
    p.group_defs.insert(GroupId::from("synth"), def);
    p.settings.length = 0.5;
    p
}

#[test]
fn a_polyphonic_synth_sounds_and_is_identical_on_many_cores() {
    let block = 128;
    let chord: Vec<(u64, MidiMsg)> = [48u8, 55, 60, 64, 67, 71, 74, 79]
        .iter()
        .enumerate()
        .map(|(k, &n)| (k as u64 * 300, on(n)))
        .collect();
    let mut midi = chord.clone();
    midi.push((6_000, off(60)));
    let render_with = |workers: Option<usize>| {
        let mut e = live(synth(16), block);
        if let Some(w) = workers {
            e.processor
                .set_workers(Some(Arc::new(RtPool::new(w, None, None))));
        }
        e.handle.play();
        let mut out = vec![0.0f32; 2 * block];
        let mut all = Vec::new();
        for b in 0..80u64 {
            let events: Vec<MidiEvent> = midi
                .iter()
                .filter(|(at, _)| *at >= b * block as u64 && *at < (b + 1) * block as u64)
                .map(|(at, m)| MidiEvent::new((*at - b * block as u64) as u32, *m))
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
    };
    let one = render_with(None);
    let peak = one.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.05, "peak {peak}");
    assert_eq!(render_with(Some(3)), one);
    let e = live(synth(16), block);
    // The 16 voice copies run side by side: waves stay as deep as one voice.
    assert!(
        e.handle.report().stats.waves <= 9,
        "{}",
        e.handle.report().stats.waves
    );
}
