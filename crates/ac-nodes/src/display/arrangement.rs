use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, node};

node! {
    kind: "arrangement",
    name: "Arrangement",
    category: Display,
    keywords: ["timeline", "song", "lanes", "playlist", "arranger", "tracks", "clips", "sequence"],
    /// The song laid out in time: a lane for every Clip, MIDI Pattern,
    /// Pattern, and Graph placed in the song.
    ///
    /// The Arrangement holds nothing of its own. Each generator whose
    /// `start` is *At beat…* is a block on its lane, from its `at` beat for
    /// as long as it plays (to its `until` beat, or its own length; one
    /// that loops with no `until` runs to the end of the song). Drag a
    /// block to move it (its `at`, and its `until` with it), drag its right
    /// edge to trim it (its `until`), and double-click it to play its whole
    /// length again; moves snap to the beat (hold `Shift` for sixteenths).
    /// The markers, the loop, and the playhead are drawn across the top.
    /// Pin it to a panel to see the song; its Node tab shows it too.
    ///
    /// **Try this:** set two Clips' `start` to *At beat…*, pin an
    /// Arrangement, and drag the second clip's block to bar 5: press play
    /// from bar 1 and hear it come in there.
    pub struct Arrangement {}
}

impl Node for Arrangement {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Arrangement {}
    }

    fn process(&mut self, _: &ProcessCtx, _: &mut Io) {}

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_has_no_sockets_of_its_own() {
        let sig = Arrangement::describe(&json!({}));
        assert!(sig.inputs.is_empty() && sig.outputs.is_empty());
    }
}
