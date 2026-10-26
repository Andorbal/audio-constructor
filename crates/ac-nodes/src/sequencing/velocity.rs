use ac_core::{
    Config, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node,
};

node! {
    kind: "velocity",
    name: "Velocity",
    category: Sequencing,
    keywords: ["dynamics", "velocity curve", "sensitivity", "accent", "midi", "touch", "fixed"],
    /// Reshapes how hard MIDI notes are played: scale, curve, range, or one
    /// fixed velocity for every note.
    ///
    /// Each note-on's velocity, as 0 to 1, is bent by `curve` (positive
    /// makes soft playing louder, negative makes it quieter; 0 leaves it),
    /// multiplied by `scale`, and placed between `min` and `max`. A `fixed`
    /// above 0 replaces it all with that one velocity. Notes stay at least
    /// velocity 1, so a soft hit is never taken for a note-off. Note-offs
    /// and every other message pass unchanged.
    ///
    /// **Try this:** put Velocity between a drum pad controller and a
    /// `Note Split`, with `curve` at 0.5 for pads that feel too stiff, or
    /// `min` at 0.4 so the softest hit still speaks; set `fixed` to 1 to
    /// hear every hit at full strength.
    pub struct Velocity {}
}

const MIDI: usize = 0;
const SCALE: usize = 1;
const CURVE: usize = 2;
const MIN: usize = 3;
const MAX: usize = 4;
const FIXED: usize = 5;

/// A velocity (1–127) reshaped by the settings.
pub fn reshape(velocity: u8, scale: f32, curve: f32, min: f32, max: f32, fixed: f32) -> u8 {
    let finite = |x: f32, d: f32| if x.is_finite() { x } else { d };
    let fixed = finite(fixed, 0.0);
    let v = if fixed > 0.0 {
        fixed.min(1.0)
    } else {
        let v = f32::from(velocity.min(127)) / 127.0;
        let bend = (-2.0 * finite(curve, 0.0).clamp(-1.0, 1.0)).exp2();
        let v = (v.powf(bend) * finite(scale, 1.0).max(0.0)).min(1.0);
        let (lo, hi) = (
            finite(min, 0.0).clamp(0.0, 1.0),
            finite(max, 1.0).clamp(0.0, 1.0),
        );
        lo + v * (hi - lo)
    };
    (v * 127.0).round().clamp(1.0, 127.0) as u8
}

impl Node for Velocity {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        let unit = |name: &'static str, default: f64, doc: &'static str| {
            SocketSpec::float(name, Unit::Generic, default)
                .range(0.0, 1.0)
                .doc(doc)
        };
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Notes to reshape."))
            .input(
                SocketSpec::float("scale", Unit::Ratio, 1.0)
                    .range(0.0, 2.0)
                    .doc("Multiplies every velocity."),
            )
            .input(
                SocketSpec::float("curve", Unit::Generic, 0.0)
                    .range(-1.0, 1.0)
                    .doc("Positive: soft playing louder; negative: quieter."),
            )
            .input(unit("min", 0.0, "The softest velocity out, 0 to 1."))
            .input(unit("max", 1.0, "The hardest velocity out, 0 to 1."))
            .input(unit("fixed", 0.0, "Above 0: every note at this velocity."))
            .output(SocketSpec::midi("midi").doc("The notes, reshaped."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Velocity {}
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let (scale, curve) = (io.inputs.float(SCALE), io.inputs.float(CURVE));
        let (min, max, fixed) = (
            io.inputs.float(MIN),
            io.inputs.float(MAX),
            io.inputs.float(FIXED),
        );
        let events = io.inputs.midi(MIDI);
        let mut out = io.outputs.midi(0);
        for e in events {
            let i = e.offset as usize;
            let msg = match e.msg {
                MidiMsg::NoteOn {
                    channel,
                    note,
                    velocity,
                } if velocity > 0 => MidiMsg::NoteOn {
                    channel,
                    note,
                    velocity: reshape(
                        velocity,
                        scale.at(i),
                        curve.at(i),
                        min.at(i),
                        max.at(i),
                        fixed.at(i),
                    ),
                },
                other => other,
            };
            out.push(MidiEvent::new(e.offset, msg));
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(velocity: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity,
        }
    }

    #[test]
    fn the_defaults_leave_velocities_alone() {
        for v in [1, 40, 64, 127] {
            assert_eq!(reshape(v, 1.0, 0.0, 0.0, 1.0, 0.0), v);
        }
    }

    #[test]
    fn curve_scale_range_and_fixed_reshape() {
        // A positive curve lifts a soft hit; a negative one lowers it.
        assert!(reshape(32, 1.0, 0.5, 0.0, 1.0, 0.0) > 32);
        assert!(reshape(32, 1.0, -0.5, 0.0, 1.0, 0.0) < 32);
        assert_eq!(reshape(127, 0.5, 0.0, 0.0, 1.0, 0.0), 64);
        // The range puts the softest hit at `min`.
        assert_eq!(reshape(1, 1.0, 0.0, 0.5, 1.0, 0.0), 64);
        assert_eq!(reshape(10, 1.0, 0.0, 0.0, 1.0, 1.0), 127);
        // Never a note-off, and nothing strange from NaN.
        assert_eq!(reshape(5, 0.0, 0.0, 0.0, 1.0, 0.0), 1);
        assert_eq!(
            reshape(64, f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN),
            64
        );
    }

    #[test]
    fn note_ons_change_and_everything_else_passes() {
        let mut h = Harness::new::<Velocity>(json!({}));
        let off = MidiMsg::NoteOff {
            channel: 0,
            note: 60,
            velocity: 30,
        };
        h.set("fixed", 0.5).midi("midi", &[(3, on(100)), (9, off)]);
        let out = h.run(64);
        assert_eq!(out.midi("midi"), &[(3, on(64)), (9, off)]);
    }
}
