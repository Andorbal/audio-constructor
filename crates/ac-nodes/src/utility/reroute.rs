use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

node! {
    kind: "reroute",
    name: "Reroute",
    category: Utility,
    keywords: ["dot", "wire", "route", "tidy"],
    /// A dot on a wire, for tidy wiring. Whatever goes in comes straight out,
    /// of any type.
    ///
    /// Add one by `Shift`+right-dragging across a wire, then drag the dot to
    /// bend the wire around other nodes. A Reroute takes the type of the wire
    /// connected to it. It costs nothing when playing: the engine removes
    /// every Reroute and connects the wires straight through.
    ///
    /// **Try this:** fan one signal out from a Reroute to several inputs, so
    /// a single wire leaves the source node and the patch reads like a bus.
    /// Or steer a long wire round the edge of a busy patch with two Reroutes
    /// at the corners.
    pub struct Reroute {}
}

impl Node for Reroute {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::any("in").doc("Any signal."))
            .output(SocketSpec::any("out").doc("The same signal."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Reroute {}
    }

    // The compiler splices reroutes out of the graph, so this never runs in a
    // compiled patch.
    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{REROUTE_KIND, SignalType, testing::Harness};
    use serde_json::json;

    #[test]
    fn has_one_any_input_and_output() {
        let sig = Reroute::describe(&json!({}));
        assert_eq!(sig.inputs.len(), 1);
        assert_eq!(sig.outputs.len(), 1);
        assert_eq!(sig.inputs[0].ty, SignalType::Any);
        assert_eq!(sig.outputs[0].ty, SignalType::Any);
        assert_eq!(Reroute::KIND, REROUTE_KIND);
    }

    #[test]
    fn processing_is_harmless() {
        let mut h = Harness::new::<Reroute>(json!({}));
        h.run(128);
        h.reset();
        h.run(1);
    }
}
