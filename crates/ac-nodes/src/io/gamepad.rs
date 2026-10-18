use ac_core::{
    Config, ConfigExt, Control, Determinism, GamepadAxis, GamepadButton, Io, Node, NodeMeta,
    PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::{fill_range, gate, stamped};

node! {
    kind: "gamepad",
    name: "Gamepad",
    category: Io,
    keywords: ["controller", "joystick", "joypad", "game", "stick", "buttons", "device", "input"],
    /// Brings a game controller into the patch: every button as a gate and
    /// a press trigger, the sticks and analog triggers as numbers.
    ///
    /// The `pad` config picks the controller: 1 is the first one connected,
    /// 2 the second, and so on up to 16. A controller that isn't connected
    /// reads as at rest: buttons up, sticks centered, triggers released.
    ///
    /// Buttons are named by their place on the standard layout: `south`,
    /// `east`, `west`, and `north` are the four face buttons (bottom, right,
    /// left, top), then come the bumpers, the triggers pulled past halfway,
    /// `select`, `start`, `mode` (the center button), the stick presses,
    /// and the D-pad. Each has a gate that is high while it's held and a
    /// `_press` trigger; turn on the `releases` config to add a `_release`
    /// trigger for each as well. An analog button counts as held from
    /// halfway down until it's back under 40 %, so one resting near the
    /// middle doesn't chatter.
    ///
    /// The sticks (`left_x`, `left_y`, `right_x`, `right_y`) run from −1
    /// to 1, right and up positive; the analog triggers
    /// (`left_trigger_axis`, `right_trigger_axis`) from 0 to 1.
    /// `dead_zone` ignores small movements around rest and rescales the
    /// rest of the travel, so a stick pushed all the way still reaches 1.
    /// For a stick it's measured from the center with both axes together,
    /// so a diagonal push behaves like a straight one. `curve` bends the
    /// response: 1 is straight, 2 gives finer control near rest, 0.5 makes
    /// small movements count for more.
    ///
    /// Every change lands on the sample it arrived on, and values hold
    /// until the controller moves again. Restarting the transport doesn't
    /// forget what the controller is doing: a button held through a
    /// restart stays held.
    ///
    /// Gamepad is a live source: nodes that need their whole input before
    /// they can play are disabled downstream of it.
    ///
    /// **Try this:** wire `left_x` through a `Map` into a filter's `cutoff`
    /// and `south` into an `ADSR`'s `gate` for a synth you play from the
    /// couch, or wire `right_trigger_axis` into a `Crossfade`'s `mix` to
    /// pull in an effect.
    pub struct Gamepad {
        /// The controller's index (0-based).
        pad: u8,
        releases: bool,
        /// Each button's held state at the end of the last block.
        held: [bool; BUTTONS],
        /// Each axis's raw position at the end of the last block.
        axes: [f32; AXES],
    }
}

const DEAD_ZONE: usize = 0;
const CURVE: usize = 1;

/// Output index of the first button's gate; the axes come first.
const FIRST_BUTTON: usize = AXES;

const BUTTONS: usize = GamepadButton::ALL.len();
const AXES: usize = GamepadAxis::ALL.len();
const MAX_PAD: u64 = 16;

const DEFAULT_DEAD_ZONE: f64 = 0.1;
const MIN_CURVE: f32 = 0.1;
const MAX_CURVE: f32 = 10.0;

/// An analog button goes down at this value...
const PRESS_AT: f32 = 0.5;
/// ...and comes up below this one (hysteresis against chatter).
const RELEASE_BELOW: f32 = 0.4;

/// The controller (0-based) and whether buttons get release triggers, from
/// config, repaired.
fn setup(config: &Config) -> (u8, bool) {
    let pad = config.get_u64_or("pad", 1).clamp(1, MAX_PAD) as u8 - 1;
    (pad, config.get_bool_or("releases", false))
}

/// Outputs per button: the gate and press trigger, and a release trigger
/// when asked for.
fn per_button(releases: bool) -> usize {
    if releases { 3 } else { 2 }
}

/// The `dead_zone` and `curve` inputs, repaired: NaN takes the neutral
/// value and both are kept in range.
fn shaping(dead_zone: f32, curve: f32) -> (f32, f32) {
    let dz = if dead_zone.is_nan() {
        0.0
    } else {
        dead_zone.clamp(0.0, 1.0)
    };
    let curve = if curve.is_nan() {
        1.0
    } else {
        curve.clamp(MIN_CURVE, MAX_CURVE)
    };
    (dz, curve)
}

/// A deflection `r` (0 at rest) past the dead zone, rescaled so the edge of
/// the dead zone is 0 and full travel is 1, then raised to `curve`.
fn response(r: f32, dz: f32, curve: f32) -> f32 {
    if r.is_nan() || r <= dz || dz >= 1.0 {
        return 0.0;
    }
    let t = (r - dz) / (1.0 - dz);
    if curve == 1.0 { t } else { t.powf(curve) }
}

/// A stick position after the dead zone and curve. The dead zone is radial
/// (a circle around the center), and the shaping scales the vector without
/// turning it, so the stick keeps its direction.
pub(crate) fn stick(x: f32, y: f32, dz: f32, curve: f32) -> (f32, f32) {
    let r = (x * x + y * y).sqrt();
    let m = response(r, dz, curve);
    if m == 0.0 || !m.is_finite() {
        return (0.0, 0.0);
    }
    let s = m / r;
    ((x * s).clamp(-1.0, 1.0), (y * s).clamp(-1.0, 1.0))
}

/// An analog trigger's position after the dead zone and curve.
pub(crate) fn trigger(v: f32, dz: f32, curve: f32) -> f32 {
    response(v.clamp(0.0, 1.0), dz, curve).clamp(0.0, 1.0)
}

/// The other axis of the same stick.
fn partner(axis: GamepadAxis) -> Option<GamepadAxis> {
    match axis {
        GamepadAxis::LeftX => Some(GamepadAxis::LeftY),
        GamepadAxis::LeftY => Some(GamepadAxis::LeftX),
        GamepadAxis::RightX => Some(GamepadAxis::RightY),
        GamepadAxis::RightY => Some(GamepadAxis::RightX),
        GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger => None,
    }
}

/// Stores a raw axis value: sticks within −1 to 1, triggers 0 to 1; NaN is
/// ignored.
fn set_axis(axes: &mut [f32; AXES], axis: GamepadAxis, v: f32) {
    if v.is_nan() {
        return;
    }
    let v = if axis.is_trigger() {
        v.clamp(0.0, 1.0)
    } else {
        v.clamp(-1.0, 1.0)
    };
    if let Some(a) = axes.get_mut(axis as usize) {
        *a = v;
    }
}

/// What output `axis` reads with these raw positions.
fn axis_value(axes: &[f32; AXES], axis: GamepadAxis, (dz, curve): (f32, f32)) -> f32 {
    let raw = |a: GamepadAxis| axes.get(a as usize).copied().unwrap_or(0.0);
    match axis {
        GamepadAxis::LeftX => stick(raw(axis), raw(GamepadAxis::LeftY), dz, curve).0,
        GamepadAxis::LeftY => stick(raw(GamepadAxis::LeftX), raw(axis), dz, curve).1,
        GamepadAxis::RightX => stick(raw(axis), raw(GamepadAxis::RightY), dz, curve).0,
        GamepadAxis::RightY => stick(raw(GamepadAxis::RightX), raw(axis), dz, curve).1,
        GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger => trigger(raw(axis), dz, curve),
    }
}

/// A button's held state after a value arrives (NaN changes nothing).
fn next_held(held: bool, v: f32) -> bool {
    if held {
        v.is_nan() || v >= RELEASE_BELOW
    } else {
        v >= PRESS_AT
    }
}

fn button_doc(button: GamepadButton) -> &'static str {
    match button {
        GamepadButton::South => "Bottom face button (often A), high while held.",
        GamepadButton::East => "Right face button (often B), high while held.",
        GamepadButton::West => "Left face button (often X), high while held.",
        GamepadButton::North => "Top face button (often Y), high while held.",
        GamepadButton::LeftBumper => "Left bumper, high while held.",
        GamepadButton::RightBumper => "Right bumper, high while held.",
        GamepadButton::LeftTrigger => "Left trigger, high while pulled past halfway.",
        GamepadButton::RightTrigger => "Right trigger, high while pulled past halfway.",
        GamepadButton::Select => "Select (back), high while held.",
        GamepadButton::Start => "Start, high while held.",
        GamepadButton::Mode => "The center button, high while held.",
        GamepadButton::LeftStick => "Left stick pressed in, high while held.",
        GamepadButton::RightStick => "Right stick pressed in, high while held.",
        GamepadButton::DpadUp => "D-pad up, high while held.",
        GamepadButton::DpadDown => "D-pad down, high while held.",
        GamepadButton::DpadLeft => "D-pad left, high while held.",
        GamepadButton::DpadRight => "D-pad right, high while held.",
    }
}

fn axis_doc(axis: GamepadAxis) -> &'static str {
    match axis {
        GamepadAxis::LeftX => "Left stick, −1 (left) to 1 (right).",
        GamepadAxis::LeftY => "Left stick, −1 (down) to 1 (up).",
        GamepadAxis::RightX => "Right stick, −1 (left) to 1 (right).",
        GamepadAxis::RightY => "Right stick, −1 (down) to 1 (up).",
        GamepadAxis::LeftTrigger => "Left analog trigger, 0 (released) to 1 (pulled).",
        GamepadAxis::RightTrigger => "Right analog trigger, 0 (released) to 1 (pulled).",
    }
}

impl Node for Gamepad {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(config: &Config) -> Signature {
        let (_, releases) = setup(config);
        let mut sig = Signature::new()
            .input(
                SocketSpec::float("dead_zone", Unit::Generic, DEFAULT_DEAD_ZONE)
                    .range(0.0, 1.0)
                    .doc("Movement around rest to ignore, 0–1; full travel still reaches 1."),
            )
            .input(
                SocketSpec::float("curve", Unit::Generic, 1.0)
                    .range(0.25, 4.0)
                    .log()
                    .doc("Response curve: 1 straight, above 1 finer near rest, below 1 touchier."),
            );
        for axis in GamepadAxis::ALL {
            sig =
                sig.output(SocketSpec::float(axis.name(), Unit::Generic, 0.0).doc(axis_doc(axis)));
        }
        for button in GamepadButton::ALL {
            sig = sig
                .output(SocketSpec::boolean(button.name(), false).doc(button_doc(button)))
                .output(
                    SocketSpec::trigger(format!("{}_press", button.name()))
                        .doc("Fires when the button goes down."),
                );
            if releases {
                sig = sig.output(
                    SocketSpec::trigger(format!("{}_release", button.name()))
                        .doc("Fires when the button comes up."),
                );
            }
        }
        sig
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (pad, releases) = setup(config);
        Gamepad {
            pad,
            releases,
            held: [false; BUTTONS],
            axes: [0.0; AXES],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let pad = self.pad;
        let events = io.host.controls();
        let dead_zone = io.inputs.float(DEAD_ZONE);
        let curve = io.inputs.float(CURVE);
        let varying = dead_zone.is_varying() || curve.is_varying();
        let shape_at = |i: usize| shaping(dead_zone.at(i), curve.at(i));

        // Each axis output walks the events for its stick (both axes, for
        // the radial dead zone) from the positions the block started with.
        for (k, &axis) in GamepadAxis::ALL.iter().enumerate() {
            let mut out = io.outputs.float(k);
            let mut axes = self.axes;
            let mut from = 0;
            for (at, ev) in stamped(events, frames) {
                let Control::GamepadAxis { pad: p, axis: a } = ev.control else {
                    continue;
                };
                if p != pad || (a != axis && Some(a) != partner(axis)) {
                    continue;
                }
                fill_range(&mut out, from, at, varying, |i| {
                    axis_value(&axes, axis, shape_at(i))
                });
                set_axis(&mut axes, a, ev.value);
                from = at;
            }
            fill_range(&mut out, from, frames, varying, |i| {
                axis_value(&axes, axis, shape_at(i))
            });
        }
        for ev in events {
            if let Control::GamepadAxis { pad: p, axis } = ev.control
                && p == pad
            {
                set_axis(&mut self.axes, axis, ev.value);
            }
        }

        let per = per_button(self.releases);
        for (b, &button) in GamepadButton::ALL.iter().enumerate() {
            let first = FIRST_BUTTON + b * per;
            let this = Control::GamepadButton { pad, button };
            let before = self.held.get(b).copied().unwrap_or(false);
            let (mut gate_out, mut press) = io.outputs.float_trigger(first, first + 1);
            let mut held = before;
            let mut from = 0;
            for (at, ev) in stamped(events, frames) {
                if ev.control != this {
                    continue;
                }
                let now = next_held(held, ev.value);
                if now == held {
                    continue;
                }
                fill_range(&mut gate_out, from, at, false, |_| gate(held));
                if now {
                    press.push(at as u32);
                }
                held = now;
                from = at;
            }
            fill_range(&mut gate_out, from, frames, false, |_| gate(held));
            if self.releases {
                let mut release = io.outputs.trigger(first + 2);
                let mut was = before;
                for (at, ev) in stamped(events, frames) {
                    if ev.control != this {
                        continue;
                    }
                    let now = next_held(was, ev.value);
                    if was && !now {
                        release.push(at as u32);
                    }
                    was = now;
                }
            }
            if let Some(h) = self.held.get_mut(b) {
                *h = held;
            }
        }
    }

    fn determinism(&self) -> Determinism {
        Determinism::Live
    }

    fn reset(&mut self) {
        // The state mirrors the controller, which a transport restart
        // doesn't move: a button held through it stays held (DECISIONS D90).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn button(pad: u8, button: GamepadButton) -> Control {
        Control::GamepadButton { pad, button }
    }

    fn axis(pad: u8, axis: GamepadAxis) -> Control {
        Control::GamepadAxis { pad, axis }
    }

    const SOUTH: GamepadButton = GamepadButton::South;

    #[test]
    fn enum_order_matches_the_lists() {
        for (i, b) in GamepadButton::ALL.iter().enumerate() {
            assert_eq!(*b as usize, i);
        }
        for (i, a) in GamepadAxis::ALL.iter().enumerate() {
            assert_eq!(*a as usize, i);
        }
    }

    #[test]
    fn buttons_change_on_the_exact_sample() {
        let mut h = Harness::new::<Gamepad>(json!({"releases": true}));
        h.host_controls(&[(10, button(0, SOUTH), 1.0), (100, button(0, SOUTH), 0.0)]);
        let out = h.run(256);
        let g = out.float("south");
        assert!(g[..10].iter().all(|&v| v == 0.0));
        assert!(g[10..100].iter().all(|&v| v == 1.0));
        assert!(g[100..].iter().all(|&v| v == 0.0));
        assert_eq!(out.triggers("south_press"), &[10]);
        assert_eq!(out.triggers("south_release"), &[100]);
        assert!(out.float("east").iter().all(|&v| v == 0.0));
        assert!(out.triggers("east_press").is_empty());
    }

    #[test]
    fn held_state_spans_blocks_and_repeats_dont_retrigger() {
        let mut h = Harness::new::<Gamepad>(json!({}));
        let dpad = GamepadButton::DpadUp;
        h.host_controls(&[
            (30, button(0, dpad), 1.0),
            (90, button(0, dpad), 1.0),
            (300, button(0, dpad), 0.0),
        ]);
        let out = h.run(512);
        let g = out.float("dpad_up");
        assert!(g[30..300].iter().all(|&v| v == 1.0));
        assert_eq!(g[29], 0.0);
        assert_eq!(g[300], 0.0);
        assert_eq!(out.triggers("dpad_up_press"), &[30]);
        // No release triggers unless asked for.
        assert!(h.signature().find_output("dpad_up_release").is_none());
    }

    #[test]
    fn analog_buttons_have_hysteresis() {
        let mut h = Harness::new::<Gamepad>(json!({"releases": true}));
        let lt = GamepadButton::LeftTrigger;
        h.host_controls(&[
            (1, button(0, lt), 0.3),
            (2, button(0, lt), 0.5),
            (3, button(0, lt), 0.45),
            (4, button(0, lt), 0.55),
            (5, button(0, lt), 0.39),
            (6, button(0, lt), f32::NAN),
            (7, button(0, lt), 0.49),
        ]);
        let out = h.run(16);
        assert_eq!(out.triggers("left_trigger_press"), &[2]);
        assert_eq!(out.triggers("left_trigger_release"), &[5]);
        assert_eq!(
            &out.float("left_trigger")[..8],
            &[0., 0., 1., 1., 1., 0., 0., 0.]
        );
    }

    #[test]
    fn a_press_and_release_on_one_sample_both_fire() {
        let mut h = Harness::new::<Gamepad>(json!({"releases": true}));
        h.host_controls(&[(5, button(0, SOUTH), 1.0), (5, button(0, SOUTH), 0.0)]);
        let out = h.run(16);
        assert_eq!(out.triggers("south_press"), &[5]);
        assert_eq!(out.triggers("south_release"), &[5]);
        assert!(out.float("south").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn only_the_chosen_pad_is_heard() {
        let mut h = Harness::new::<Gamepad>(json!({"pad": 2}));
        h.host_controls(&[
            (0, button(0, SOUTH), 1.0),
            (1, axis(0, GamepadAxis::LeftX), 1.0),
            (2, button(1, GamepadButton::North), 1.0),
            (3, axis(1, GamepadAxis::RightY), -1.0),
            (4, axis(5, GamepadAxis::RightY), 1.0),
            (5, Control::Key(ac_core::Key::from_char('a').unwrap()), 1.0),
        ]);
        let out = h.run(64);
        assert!(out.float("south").iter().all(|&v| v == 0.0));
        assert!(out.float("left_x").iter().all(|&v| v == 0.0));
        assert_eq!(out.float("north")[2], 1.0);
        assert_eq!(out.float("right_y")[3], -1.0);
        assert_eq!(out.float("right_y")[63], -1.0);
        // An unknown pad is simply at rest.
        let mut h = Harness::new::<Gamepad>(json!({"pad": 9}));
        h.host_controls(&[(0, button(0, SOUTH), 1.0)]);
        let out = h.run(64);
        assert!(out.floats.values().flatten().all(|&v| v == 0.0));
        assert!(out.triggers.values().all(Vec::is_empty));
    }

    #[test]
    fn config_is_repaired() {
        assert_eq!(setup(&json!({})), (0, false));
        assert_eq!(setup(&json!({"pad": 0})), (0, false));
        assert_eq!(setup(&json!({"pad": 99, "releases": true})), (15, true));
        assert_eq!(setup(&json!({"pad": "two", "releases": "yes"})), (0, false));
        let sig = Gamepad::describe(&json!({}));
        assert_eq!(sig.outputs.len(), AXES + 2 * BUTTONS);
        assert_eq!(sig.outputs[0].name, "left_x");
        assert_eq!(sig.outputs[FIRST_BUTTON].name, "south");
        assert_eq!(sig.outputs[FIRST_BUTTON + 1].name, "south_press");
        let sig = Gamepad::describe(&json!({"releases": true}));
        assert_eq!(sig.outputs.len(), AXES + 3 * BUTTONS);
        assert_eq!(sig.outputs.last().unwrap().name, "dpad_right_release");
    }

    #[test]
    fn sticks_move_on_the_exact_sample_and_hold() {
        let mut h = Harness::new::<Gamepad>(json!({}));
        h.set("dead_zone", 0.0);
        h.host_controls(&[
            (20, axis(0, GamepadAxis::LeftX), 0.5),
            (70, axis(0, GamepadAxis::LeftY), -0.25),
            (75, axis(0, GamepadAxis::RightTrigger), 0.75),
        ]);
        let out = h.run(192);
        let x = out.float("left_x");
        assert_eq!(x[19], 0.0);
        assert_eq!(x[20], 0.5);
        assert_eq!(x[191], 0.5);
        let y = out.float("left_y");
        assert_eq!(y[69], 0.0);
        assert_eq!(y[70], -0.25);
        let t = out.float("right_trigger_axis");
        assert_eq!(t[74], 0.0);
        assert_eq!(t[75], 0.75);
        // The next block starts where this one left off.
        let out = h.run(64);
        assert!(out.float("left_x").iter().all(|&v| v == 0.5));
    }

    #[test]
    fn radial_dead_zone_rescales_to_full_travel() {
        let dz = 0.2;
        // Inside the circle: nothing, even if one axis alone would pass.
        assert_eq!(stick(0.15, 0.1, dz, 1.0), (0.0, 0.0));
        // Full travel still reaches 1, either way.
        assert_eq!(stick(1.0, 0.0, dz, 1.0), (1.0, 0.0));
        assert_eq!(stick(0.0, -1.0, dz, 1.0), (0.0, -1.0));
        // Halfway past the dead zone reads half, and direction is kept.
        let (x, y) = stick(0.6 * 0.6, 0.6 * 0.8, dz, 1.0);
        assert!((x - 0.5 * 0.6).abs() < 1e-6 && (y - 0.5 * 0.8).abs() < 1e-6);
        // A square gate's corner is kept within range.
        let (x, y) = stick(1.0, 1.0, dz, 1.0);
        assert!(x <= 1.0 && y <= 1.0 && x > 0.9);
        // No dead zone and a straight curve change nothing.
        assert_eq!(stick(0.3, -0.7, 0.0, 1.0), (0.3, -0.7));
        assert_eq!(stick(0.0, 0.0, 0.0, 1.0), (0.0, 0.0));
        // A dead zone of 1 swallows everything.
        assert_eq!(stick(1.0, 0.0, 1.0, 1.0), (0.0, 0.0));
        assert_eq!(trigger(1.0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn curve_bends_the_response() {
        let (x, _) = stick(0.5, 0.0, 0.0, 2.0);
        assert!((x - 0.25).abs() < 1e-6);
        let (x, _) = stick(0.25, 0.0, 0.0, 0.5);
        assert!((x - 0.5).abs() < 1e-6);
        assert!((trigger(0.6, 0.2, 2.0) - 0.25).abs() < 1e-6);
        assert_eq!(trigger(0.1, 0.2, 2.0), 0.0);
        assert_eq!(trigger(1.0, 0.2, 3.0), 1.0);
        // Through the node, with the inputs repaired.
        let mut h = Harness::new::<Gamepad>(json!({}));
        h.set("dead_zone", 0.0).set("curve", 2.0);
        h.host_controls(&[(0, axis(0, GamepadAxis::RightX), -0.5)]);
        assert!((h.run(4).float("right_x")[0] + 0.25).abs() < 1e-6);
        assert_eq!(shaping(f32::NAN, f32::NAN), (0.0, 1.0));
        assert_eq!(shaping(-1.0, 0.0), (0.0, MIN_CURVE));
        assert_eq!(shaping(5.0, 1e9), (1.0, MAX_CURVE));
    }

    #[test]
    fn extreme_values_are_kept_in_range() {
        let mut h = Harness::new::<Gamepad>(json!({}));
        h.set("dead_zone", 0.0);
        h.host_controls(&[
            (1, axis(0, GamepadAxis::LeftX), f32::INFINITY),
            (2, axis(0, GamepadAxis::LeftY), f32::NEG_INFINITY),
            (3, axis(0, GamepadAxis::LeftX), f32::NAN),
            (4, axis(0, GamepadAxis::LeftTrigger), -3.0),
            (5, axis(0, GamepadAxis::RightTrigger), 7.0),
            (6, button(0, SOUTH), f32::INFINITY),
        ]);
        let out = h.run(16);
        for (name, v) in &out.floats {
            assert!(
                v.iter().all(|x| x.is_finite() && (-1.0..=1.0).contains(x)),
                "{name}"
            );
        }
        // NaN left the stick where it was (pushed into the corner).
        assert!(out.float("left_x")[15] > 0.7);
        assert!(out.float("left_y")[15] < -0.7);
        assert_eq!(out.float("left_trigger_axis")[15], 0.0);
        assert_eq!(out.float("right_trigger_axis")[15], 1.0);
        assert_eq!(out.float("south")[15], 1.0);
    }

    #[test]
    fn sample_rate_shaping_follows_its_inputs() {
        let mut h = Harness::new::<Gamepad>(json!({}));
        let dz: Vec<f32> = (0..64).map(|i| if i < 32 { 0.0 } else { 0.5 }).collect();
        h.set_samples("dead_zone", dz);
        h.host_controls(&[(0, axis(0, GamepadAxis::LeftX), 0.5)]);
        let x = h.run(64).float("left_x").to_vec();
        assert_eq!(x[31], 0.5);
        assert_eq!(x[32], 0.0);
    }

    #[test]
    fn block_size_does_not_matter() {
        let mut events = Vec::new();
        let mut s = 7u32;
        for i in 0..400u64 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let v = (s >> 8) as f32 / (1 << 24) as f32;
            let at = i * 10 + u64::from(s % 7);
            let c = match s % 4 {
                0 => button(0, GamepadButton::ALL[(s >> 3) as usize % BUTTONS]),
                _ => axis(0, GamepadAxis::ALL[(s >> 3) as usize % AXES]),
            };
            events.push((at, c, v * 2.0 - 1.0));
        }
        let run = |block| {
            let mut h = Harness::with_resources::<Gamepad>(
                json!({"releases": true}),
                48_000.0,
                block,
                &NoResources,
            );
            let dz: Vec<f32> = (0..4_000).map(|i| (i % 500) as f32 / 1_000.0).collect();
            h.set_samples("dead_zone", dz).set("curve", 1.5);
            h.host_controls(&events);
            let out = h.run(4_000);
            (out.floats, out.triggers)
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.1.values().map(Vec::len).sum::<usize>() > 20);
    }

    #[test]
    fn reset_keeps_what_the_controller_is_doing() {
        let mut h = Harness::new::<Gamepad>(json!({}));
        h.set("dead_zone", 0.0);
        h.host_controls(&[
            (0, button(0, SOUTH), 1.0),
            (0, axis(0, GamepadAxis::LeftX), -0.5),
        ]);
        h.run(64);
        h.reset();
        let out = h.run(64);
        assert!(out.float("south").iter().all(|&v| v == 1.0));
        assert!(out.triggers("south_press").is_empty());
        assert!(out.float("left_x").iter().all(|&v| v == -0.5));
    }

    #[test]
    fn is_a_live_source() {
        assert!(Gamepad::meta().live);
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Gamepad",
            resources: &NoResources,
        };
        assert_eq!(
            Gamepad::new(&json!({}), &ctx).determinism(),
            Determinism::Live
        );
    }
}
