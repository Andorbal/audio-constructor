use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "info",
    name: "Info",
    category: Utility,
    keywords: ["sample rate", "block size", "cpu", "load", "xruns", "dropouts", "latency", "stats"],
    /// The engine's vital signs as signals: `sample_rate` (Hz), `block_size`
    /// (the largest block, in frames), `cpu_load` (how much of each block's
    /// time the patch used last block, 0 to 1 and beyond), `xruns` (dropouts
    /// so far, from overruns and the audio device), and `latency` (the
    /// patch's output latency in samples).
    ///
    /// `cpu_load` and `xruns` depend on the machine, not the patch, so Info
    /// counts as a live source: a whole-input node fed by them is disabled.
    /// In offline renders they read 0.
    ///
    /// **Try this:** wire `cpu_load` into a `Readout` and a `Meter` to watch
    /// how hard a patch works while you build it, or feed `sample_rate`
    /// through a `Math` node to make a filter cutoff that is always a
    /// fixed fraction of Nyquist.
    pub struct Info {
        block: i32,
    }
}

const SAMPLE_RATE: usize = 0;
const BLOCK_SIZE: usize = 1;
const CPU_LOAD: usize = 2;
const XRUNS: usize = 3;
const LATENCY: usize = 4;

impl Node for Info {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .output(
                SocketSpec::float("sample_rate", Unit::Hz, 48_000.0)
                    .rate(Rate::Block)
                    .doc("The engine's sample rate."),
            )
            .output(SocketSpec::int("block_size", 0).doc("The largest block, in frames."))
            .output(
                SocketSpec::float("cpu_load", Unit::Ratio, 0.0)
                    .rate(Rate::Block)
                    .doc("Last block's processing time over its budget."),
            )
            .output(SocketSpec::int("xruns", 0).doc("Dropouts so far."))
            .output(SocketSpec::int("latency", 0).doc("Output latency in samples."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Info {
            block: i32::try_from(ctx.max_block).unwrap_or(i32::MAX),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let e = ctx.engine;
        io.outputs.set_float(SAMPLE_RATE, ctx.sample_rate as f32);
        io.outputs.set_int(BLOCK_SIZE, self.block);
        io.outputs
            .set_float(CPU_LOAD, if e.load.is_finite() { e.load } else { 0.0 });
        io.outputs
            .set_int(XRUNS, i32::try_from(e.xruns).unwrap_or(i32::MAX));
        io.outputs
            .set_int(LATENCY, i32::try_from(e.latency).unwrap_or(i32::MAX));
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::EngineInfo;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn reports_rate_block_and_engine_figures() {
        let mut h =
            Harness::with_resources::<Info>(json!({}), 44_100.0, 128, &ac_core::NoResources);
        h.engine_info(EngineInfo {
            load: 0.25,
            xruns: 3,
            latency: 512,
        });
        let out = h.run(128);
        assert_eq!(out.float("sample_rate")[0], 44_100.0);
        assert_eq!(out.int("block_size")[0], 128);
        assert_eq!(out.float("cpu_load")[0], 0.25);
        assert_eq!(out.int("xruns")[0], 3);
        assert_eq!(out.int("latency")[0], 512);
        assert!(Info::meta().live);
    }

    #[test]
    fn bad_load_reads_zero() {
        let mut h = Harness::new::<Info>(json!({}));
        h.engine_info(EngineInfo {
            load: f32::NAN,
            xruns: u64::MAX,
            latency: 0,
        });
        let out = h.run(64);
        assert_eq!(out.float("cpu_load")[0], 0.0);
        assert_eq!(out.int("xruns")[0], i32::MAX);
    }
}
