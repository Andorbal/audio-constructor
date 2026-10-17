use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, node};

node! {
    kind: "comment",
    name: "Comment",
    category: Utility,
    keywords: ["note", "text", "label", "sticky"],
    /// A sticky note on the canvas, for explaining a patch to others or to
    /// yourself later.
    ///
    /// Its `text` setting holds the note. A Comment has no sockets and does
    /// nothing to the sound.
    ///
    /// To label a whole area of a patch, use a frame instead: select the
    /// nodes and press `Ctrl+J`. Frames are a canvas feature rather than a
    /// node, and they move their contents with them.
    ///
    /// **Try this:** leave a Comment beside the part of a patch worth playing
    /// with, saying what to try ("turn `cutoff` down slowly"), the way the
    /// example projects do.
    pub struct Comment {}
}

impl Node for Comment {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
    }

    // The text is for the editor; nothing here needs it.
    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Comment {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn has_no_sockets() {
        let sig = Comment::describe(&json!({"text": "turn it up"}));
        assert!(sig.inputs.is_empty() && sig.outputs.is_empty());
    }

    #[test]
    fn processing_is_harmless() {
        let mut h = Harness::new::<Comment>(json!({"text": "hello"}));
        h.run(64);
        h.reset();
        h.run(1);
    }
}
