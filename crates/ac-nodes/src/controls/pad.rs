use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

use super::button::for_each_edge;

node! {
    kind: "pad",
    name: "Pad",
    category: Controls,
    keywords: ["drum pad", "button", "velocity", "pressure", "hit", "trigger", "mpc", "finger drum"],
    /// A drum pad: a button that feels how hard it's hit, and lights up.
    ///
    /// Press the pad in the node (or on a panel): where you hit it sets
    /// the velocity, harder towards the top, and `trigger` fires with
    /// `velocity` at that level (0–1) until the next hit. `gate` stays
    /// high while it's held and `release` fires when it comes up. On a
    /// pen or touch screen that reports pressure, `pressure` follows it
    /// while the pad is held (aftertouch); otherwise it holds the velocity.
    ///
    /// `lit` lights the pad without playing it: wire a pattern's gate or a
    /// clip's `playing` into it so the pad shows what plays. Right-click
    /// `pressed` to bind the pad to a key or a MIDI note.
    ///
    /// **Try this:** wire `trigger` into a `Sampler`'s `start` and
    /// `velocity` into its `gain` through a `Map` (0–1 to −24–0 dB), pin
    /// the pad to a panel, and play it soft and hard.
    pub struct Pad {
        pressed: bool,
        velocity: f32,
    }
}

const PRESSED: usize = 0;
const LIT: usize = 1;
const HIT: usize = 2;
const PRESSURE: usize = 3;

const TRIGGER: usize = 0;
const GATE: usize = 1;
const RELEASE: usize = 2;
const VELOCITY: usize = 3;
const PRESSURE_OUT: usize = 4;

/// Telemetry: 1 while `lit` was high in the last block.
pub const TELEMETRY_LIT: usize = 0;
/// Telemetry: the velocity of the last hit.
pub const TELEMETRY_VELOCITY: usize = 1;

/// A hit's velocity, kept within 0–1 (NaN is a full-strength hit).
fn clean(v: f32) -> f32 {
    if v.is_nan() { 1.0 } else { v.clamp(0.0, 1.0) }
}

impl Node for Pad {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("pressed", false).doc("True while the pad is held."))
            .input(
                SocketSpec::boolean("lit", false)
                    .late()
                    .doc("Lights the pad without playing it."),
            )
            .input(
                SocketSpec::float("hit", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .hidden()
                    .doc("How hard the next press is, 0–1 (set where the pad is hit)."),
            )
            .input(
                SocketSpec::float("pressure", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .hidden()
                    .doc("Pressure while held, 0–1, where the screen reports it."),
            )
            .output(SocketSpec::trigger("trigger").doc("Fires when the pad is hit."))
            .output(SocketSpec::boolean("gate", false).doc("High while the pad is held."))
            .output(SocketSpec::trigger("release").doc("Fires when the pad comes up."))
            .output(
                SocketSpec::float("velocity", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("How hard the last hit was, 0–1."),
            )
            .output(
                SocketSpec::float("pressure", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("Pressure while held (aftertouch); the velocity otherwise."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Pad {
            pressed: false,
            velocity: 0.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let pressed = io.inputs.boolean(PRESSED);
        let hit = clean(io.inputs.float(HIT).first());
        let pressure = io.inputs.float(PRESSURE).first();
        let before = self.pressed;
        let (mut gate, mut press) = io.outputs.float_trigger(GATE, TRIGGER);
        gate.fill_with(|i| if pressed.at(i) { 1.0 } else { 0.0 });
        let mut struck = false;
        self.pressed = for_each_edge(pressed, before, ctx.frames, |i, down| {
            if down {
                press.push(i);
                struck = true;
            }
        });
        if struck {
            self.velocity = hit;
        }
        let mut release = io.outputs.trigger(RELEASE);
        for_each_edge(pressed, before, ctx.frames, |i, down| {
            if !down {
                release.push(i);
            }
        });
        io.outputs.set_float(VELOCITY, self.velocity);
        let felt = if self.pressed && pressure > 0.0 && pressure.is_finite() {
            pressure.min(1.0)
        } else if self.pressed {
            self.velocity
        } else {
            0.0
        };
        io.outputs.set_float(PRESSURE_OUT, felt);
        let lit = super::lit_level(io.inputs.boolean(LIT), ctx.frames);
        io.telemetry.set(TELEMETRY_LIT, lit);
        io.telemetry.set(TELEMETRY_VELOCITY, self.velocity);
    }

    fn reset(&mut self) {
        self.pressed = false;
        self.velocity = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn a_hit_carries_its_velocity() {
        let mut h = Harness::new::<Pad>(json!({}));
        h.set("hit", 0.3).set("pressed", true);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger"), &[0]);
        assert_eq!(out.float("velocity")[0], 0.3);
        assert_eq!(out.float("gate")[0], 1.0);
        // Held without pressure, `pressure` holds the velocity.
        assert_eq!(out.float("pressure")[0], 0.3);
        h.set("pressure", 0.9);
        assert_eq!(h.run(64).float("pressure")[0], 0.9);
        // A new hit point doesn't change a pad already down.
        h.set("hit", 1.0);
        assert_eq!(h.run(64).float("velocity")[0], 0.3);
        h.set("pressed", false);
        let out = h.run(64);
        assert_eq!(out.triggers("release"), &[192]);
        assert_eq!(out.float("pressure")[0], 0.0);
        assert_eq!(out.float("velocity")[0], 0.3, "held until the next hit");
        h.set("pressed", true);
        assert_eq!(h.run(64).float("velocity")[0], 1.0);
    }

    #[test]
    fn a_wired_gate_plays_on_its_sample_and_lit_shows() {
        let mut h = Harness::new::<Pad>(json!({}));
        let mut gate = vec![0.0; 64];
        gate[10..20].fill(1.0);
        h.set_samples("pressed", gate);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger"), &[10]);
        assert_eq!(out.triggers("release"), &[20]);
        assert_eq!(h.telemetry(TELEMETRY_LIT), 0.0);
        // Frame 40 of the second block (frames count from the start).
        let mut lit = vec![0.0; 128];
        lit[104] = 1.0;
        h.set_samples("lit", lit);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LIT), 1.0, "a one-sample gate shows");
    }

    #[test]
    fn nan_hits_are_full_strength() {
        let mut h = Harness::new::<Pad>(json!({}));
        h.set("hit", f64::NAN).set("pressed", true);
        assert_eq!(h.run(16).float("velocity")[0], 1.0);
    }
}
