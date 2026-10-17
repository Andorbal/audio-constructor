use ac_core::{
    BoolIn, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

node! {
    kind: "button",
    name: "Button",
    category: Controls,
    keywords: ["momentary", "press", "push", "pad", "bang"],
    /// A momentary push button: a trigger on press, a gate while held, and
    /// a trigger on release.
    ///
    /// Holding the button down in the node (or on a panel) sets `pressed`.
    /// Right-click `pressed` to bind it to a key or a MIDI note. `pressed`
    /// can also be wired, which turns any gate into press and release
    /// triggers. Both triggers land on the exact sample `pressed` changes.
    ///
    /// **Try this:** wire `gate` into an `ADSR` to sound a note for as long
    /// as the button is held, or wire `trigger` into a `Clip`'s `start` to
    /// fire a sample.
    pub struct Button {
        /// `pressed` at the end of the last block.
        pressed: bool,
    }
}

const PRESSED: usize = 0;

const TRIGGER: usize = 0;
const GATE: usize = 1;
const RELEASE: usize = 2;

/// Calls `f(offset, pressed)` wherever `pressed` changes, starting from its
/// state before the block. Returns the state at the end of the block.
fn for_each_edge(
    pressed: BoolIn,
    before: bool,
    frames: usize,
    mut f: impl FnMut(u32, bool),
) -> bool {
    let mut prev = before;
    let changes_at = if pressed.is_varying() {
        frames
    } else {
        frames.min(1)
    };
    for i in 0..changes_at {
        let p = pressed.at(i);
        if p != prev {
            f(i as u32, p);
            prev = p;
        }
    }
    prev
}

impl Node for Button {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("pressed", false).doc("True while the button is held."))
            .output(SocketSpec::trigger("trigger").doc("Fires when the button goes down."))
            .output(SocketSpec::boolean("gate", false).doc("High while the button is held."))
            .output(SocketSpec::trigger("release").doc("Fires when the button comes up."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Button { pressed: false }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let pressed = io.inputs.boolean(PRESSED);
        let before = self.pressed;
        let (mut gate, mut press) = io.outputs.float_trigger(GATE, TRIGGER);
        gate.fill_with(|i| if pressed.at(i) { 1.0 } else { 0.0 });
        self.pressed = for_each_edge(pressed, before, ctx.frames, |i, down| {
            if down {
                press.push(i);
            }
        });
        let mut release = io.outputs.trigger(RELEASE);
        for_each_edge(pressed, before, ctx.frames, |i, down| {
            if !down {
                release.push(i);
            }
        });
    }

    fn reset(&mut self) {
        self.pressed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn held(from: usize, to: usize, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| if (from..to).contains(&i) { 1.0 } else { 0.0 })
            .collect()
    }

    #[test]
    fn press_and_release_are_sample_accurate() {
        let mut h = Harness::new::<Button>(json!({}));
        let pressed = held(10, 100, 256);
        h.set_samples("pressed", pressed.clone());
        let out = h.run(256);
        assert_eq!(out.triggers("trigger"), &[10]);
        assert_eq!(out.triggers("release"), &[100]);
        assert_eq!(out.float("gate"), pressed.as_slice());
    }

    #[test]
    fn a_held_press_spans_blocks_without_retriggering() {
        let mut h = Harness::new::<Button>(json!({}));
        h.set_samples(
            "pressed",
            [held(30, 200, 256), held(210, 220, 256)].concat(),
        );
        let out = h.run(512);
        assert_eq!(out.triggers("trigger"), &[30, 466]);
        assert_eq!(out.triggers("release"), &[200, 476]);
    }

    #[test]
    fn widget_presses_land_on_block_starts() {
        let mut h = Harness::new::<Button>(json!({}));
        let out = h.run(64);
        assert!(out.triggers("trigger").is_empty());
        assert!(out.float("gate").iter().all(|&g| g == 0.0));

        h.set("pressed", true);
        let out = h.run(128);
        assert_eq!(out.triggers("trigger"), &[64]);
        assert!(out.float("gate").iter().all(|&g| g == 1.0));

        h.set("pressed", false);
        let out = h.run(64);
        assert_eq!(out.triggers("release"), &[192]);
        assert!(out.triggers("trigger").is_empty());
    }

    #[test]
    fn reset_forgets_the_press() {
        let mut h = Harness::new::<Button>(json!({}));
        h.set("pressed", true);
        h.run(64);
        h.reset();
        assert_eq!(h.run(64).triggers("trigger"), &[64]);
    }
}
