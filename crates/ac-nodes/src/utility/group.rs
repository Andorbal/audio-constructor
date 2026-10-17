use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, node};

node! {
    kind: "group",
    name: "Group",
    category: Utility,
    keywords: ["subpatch", "subgraph", "block", "container"],
    /// A piece of patch packed into one node, with its own sockets.
    /// Double-click it (or select it and press `Tab`) to go inside and edit
    /// it; `Tab` again comes back out.
    ///
    /// Select some nodes and press `Ctrl+G` to make a group: wires that
    /// crossed the edge of the selection become the group's sockets. To add a
    /// socket later, go inside, right-click an input, and choose *expose as a
    /// group input*. `Ctrl+Alt+G` ungroups, putting the nodes back in place.
    ///
    /// Each Group node is an instance of a definition, and every instance of
    /// a definition shares it: change the inside of one and all of them
    /// change. A project carries the definitions it uses, so it opens
    /// anywhere. The blocks in the library are saved groups; open one up to
    /// see how it works.
    ///
    /// Groups cost nothing when playing: the engine flattens them into the
    /// surrounding patch.
    ///
    /// **Try this:** build an `Oscillator` → `Biquad` → `Gain` voice, group
    /// it, and duplicate the group (`Shift+D`) a few times with different
    /// `freq` values wired in. Tweak the filter inside one copy and hear
    /// every copy follow.
    pub struct Group {}
}

impl Node for Group {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    // An instance's sockets come from its definition (`group_signature`); a
    // bare kind has none.
    fn describe(_: &Config) -> Signature {
        Signature::new()
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Group {}
    }

    // The compiler flattens groups, so an instance never runs.
    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{GROUP_KIND, testing::Harness};
    use serde_json::json;

    #[test]
    fn is_the_group_kind_with_no_sockets_of_its_own() {
        assert_eq!(Group::KIND, GROUP_KIND);
        let sig = Group::describe(&json!({}));
        assert!(sig.inputs.is_empty() && sig.outputs.is_empty());
    }

    #[test]
    fn processing_is_harmless() {
        let mut h = Harness::new::<Group>(json!({}));
        h.run(64);
        h.reset();
        h.run(1);
    }
}
