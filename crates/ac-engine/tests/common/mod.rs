//! Test-only nodes with trivially predictable outputs, so engine tests don't
//! depend on the real node library.

#![allow(dead_code)]

use ac_core::*;

node! {
    kind: "t_src",
    name: "Src",
    category: Generators,
    keywords: [],
    /// Sample-rate ramp: transport position × scale. Try this: nothing.
    pub struct TSrc {}
}
impl Node for TSrc {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("scale", Unit::Generic, 1.0))
            .output(SocketSpec::float("out", Unit::Generic, 0.0).rate(Rate::Sample))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TSrc {}
    }
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let scale = io.inputs.float(0);
        let pos = ctx.transport.position;
        io.outputs
            .float(0)
            .fill_with(|i| (pos + i as u64) as f32 * scale.at(i));
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_block",
    name: "Block",
    category: Math,
    keywords: [],
    /// Block-rate value. Try this: nothing.
    pub struct TBlock {}
}
impl Node for TBlock {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("value", Unit::Generic, 0.0))
            .output(SocketSpec::float("out", Unit::Generic, 0.0).rate(Rate::Block))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TBlock {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let v = io.inputs.float(0).first();
        io.outputs.set_float(0, v);
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_int",
    name: "Int",
    category: Math,
    keywords: [],
    /// Int value. Try this: nothing.
    pub struct TInt {}
}
impl Node for TInt {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::int("value", 0))
            .output(SocketSpec::int("out", 0))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TInt {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let v = io.inputs.int(0);
        io.outputs.set_int(0, v);
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_add",
    name: "Add",
    category: Math,
    keywords: [],
    /// a + b, rate-polymorphic. Try this: nothing.
    pub struct TAdd {}
}
impl Node for TAdd {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("a", Unit::Generic, 0.0))
            .input(SocketSpec::float("b", Unit::Generic, 0.0))
            .output(SocketSpec::float("out", Unit::Generic, 0.0).poly())
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TAdd {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let (a, b) = (io.inputs.float(0), io.inputs.float(1));
        io.outputs.float(0).fill_with(|i| a.at(i) + b.at(i));
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_sum",
    name: "Sum",
    category: Level,
    keywords: [],
    /// Audio a + audio b + amount. Try this: nothing.
    pub struct TSum {}
}
impl Node for TSum {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in"))
            .input(SocketSpec::audio("b"))
            .input(SocketSpec::float("amount", Unit::Generic, 0.0))
            .output(SocketSpec::audio("out"))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TSum {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.audio(0);
        let b = io.inputs.audio(1);
        let amt = io.inputs.float(2);
        let mut out = io.outputs.audio(0);
        for c in 0..out.channels() {
            let (x, y) = (a.channel(c), b.channel(c));
            for (i, o) in out.channel(c).iter_mut().enumerate() {
                *o =
                    x.get(i).copied().unwrap_or(0.0) + y.get(i).copied().unwrap_or(0.0) + amt.at(i);
            }
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_out",
    name: "Out",
    category: Io,
    keywords: [],
    /// Adds a stereo input to device outputs 0 and 1. Try this: nothing.
    pub struct TOut {}
}
impl Node for TOut {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().input(SocketSpec::audio("in").channels(Channels::Fixed(2)))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TOut {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.audio(0);
        for c in 0..2 {
            let src = a.channel(c);
            if let Some(dst) = io.host.output_channel(c) {
                for (d, s) in dst.iter_mut().zip(src) {
                    *d += s;
                }
            }
        }
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_quad",
    name: "Quad",
    category: Generators,
    keywords: [],
    /// Four channels of constant 1. Try this: nothing.
    pub struct TQuad {}
}
impl Node for TQuad {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::audio("out").channels(Channels::Fixed(4)))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TQuad {}
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        io.outputs.audio(0).fill(1.0);
    }
    fn reset(&mut self) {}
}

node! {
    kind: "t_fb",
    name: "Feedback",
    category: Utility,
    keywords: [],
    /// One block of delay. Try this: nothing.
    pub struct TFb { store: Vec<f32>, len: usize }
}
impl Node for TFb {
    fn meta() -> NodeMeta {
        Self::base_meta().feedback()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").channels(Channels::Fixed(1)))
            .output(SocketSpec::audio("out").channels(Channels::Fixed(1)))
    }
    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        TFb {
            store: vec![0.0; ctx.max_block],
            len: 0,
        }
    }
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        if ctx.phase != Phase::FeedbackWrite {
            let mut out = io.outputs.audio(0);
            let o = out.channel(0);
            for (i, v) in o.iter_mut().enumerate() {
                *v = if i < self.len { self.store[i] } else { 0.0 };
            }
        }
        if ctx.phase != Phase::FeedbackRead {
            let src = io.inputs.audio(0).channel(0);
            let n = src.len().min(self.store.len());
            self.store[..n].copy_from_slice(&src[..n]);
            self.len = n;
        }
    }
    fn reset(&mut self) {
        self.store.fill(0.0);
        self.len = 0;
    }
}

node! {
    kind: "t_counter",
    name: "Counter",
    category: Math,
    keywords: [],
    /// Counts processed blocks (state). Try this: nothing.
    pub struct TCounter { n: f32 }
}
impl Node for TCounter {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(_: &Config) -> Signature {
        Signature::new().output(SocketSpec::float("out", Unit::Generic, 0.0).rate(Rate::Block))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TCounter { n: 0.0 }
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        self.n += 1.0;
        io.outputs.set_float(0, self.n);
    }
    fn reset(&mut self) {
        self.n = 0.0;
    }
}

node! {
    kind: "t_starts",
    name: "Starts",
    category: Generators,
    keywords: [],
    /// Counts start triggers into telemetry 0 and outputs the count. Try this: nothing.
    pub struct TStarts { n: f32 }
}
impl Node for TStarts {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1)
    }
    fn describe(_: &Config) -> Signature {
        Signature::new()
            .generator_inputs(false, false)
            .output(SocketSpec::float("out", Unit::Generic, 0.0).rate(Rate::Block))
    }
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        TStarts { n: 0.0 }
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        self.n += io.inputs.triggers(0).len() as f32;
        io.telemetry.set(0, self.n);
        io.outputs.set_float(0, self.n);
    }
    fn reset(&mut self) {
        self.n = 0.0;
    }
}

node! {
    kind: "t_mix",
    name: "Mix",
    category: Level,
    keywords: [],
    /// N audio inputs from config. Try this: nothing.
    pub struct TMix { n: usize }
}
impl Node for TMix {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }
    fn describe(config: &Config) -> Signature {
        let n = config.get_u64_or("inputs", 2) as usize;
        let mut s = Signature::new();
        for i in 0..n {
            s = s.input(SocketSpec::audio(format!("in_{i}")));
        }
        s.output(SocketSpec::audio("out"))
    }
    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TMix {
            n: config.get_u64_or("inputs", 2) as usize,
        }
    }
    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let mut out = io.outputs.audio(0);
        for k in 0..self.n {
            let a = io.inputs.audio(k);
            for c in 0..out.channels() {
                let src = a.channel(c);
                for (o, s) in out.channel(c).iter_mut().zip(src) {
                    *o += s;
                }
            }
        }
    }
    fn reset(&mut self) {}
}

pub fn registry() -> Registry {
    let mut r = Registry::new();
    r.register::<TSrc>()
        .register::<TBlock>()
        .register::<TInt>()
        .register::<TAdd>()
        .register::<TSum>()
        .register::<TOut>()
        .register::<TQuad>()
        .register::<TFb>()
        .register::<TCounter>()
        .register::<TStarts>()
        .register::<TMix>();
    r
}

/// A tiny project builder for tests.
pub struct P {
    pub project: Project,
}

impl P {
    pub fn new() -> P {
        let mut project = Project::default();
        project.settings.length = 0.1;
        P { project }
    }
    pub fn node(mut self, id: u64, kind: &str) -> P {
        self.project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(id), kind));
        self
    }
    pub fn node_with(mut self, n: NodeInstance) -> P {
        self.project.graph.nodes.push(n);
        self
    }
    pub fn param(mut self, id: u64, socket: &str, v: impl Into<Value>) -> P {
        if let Some(n) = self.project.graph.node_mut(NodeId(id)) {
            n.params.insert(socket.into(), v.into());
        }
        self
    }
    pub fn wire(mut self, from: u64, out: &str, to: u64, inp: &str) -> P {
        self.project.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(from), out),
            Endpoint::new(NodeId(to), inp),
        ));
        self
    }
    pub fn build(self) -> Project {
        self.project
    }
}

pub fn path(id: u64) -> NodePath {
    NodePath::root(NodeId(id))
}
