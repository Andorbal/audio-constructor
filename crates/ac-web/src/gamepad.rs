//! Gamepads in a browser: the Gamepad API's standard layout turned into
//! the same control events the desktop's gamepad thread sends (ARCHITECTURE
//! §11), so a project's gamepad bindings and Gamepad nodes work alike.
//!
//! The browser reports state, not events, so the page polls every frame
//! and [`Pads::update`] sends what changed. Buttons are 0–1; the analog
//! triggers are buttons and axes (0–1) at once; sticks are −1 to 1 with
//! right and up positive (the browser's down-positive Y is turned around)
//! and a small dead zone. A gamepad keeps its index while connected, and
//! when it goes away its held buttons are released and its axes return to
//! rest, so nothing hangs.

use ac_core::{Control, GamepadAxis, GamepadButton};

/// Buttons in the standard layout.
pub const STANDARD_BUTTONS: usize = 17;
/// Axes in the standard layout: left X, left Y, right X, right Y.
pub const STANDARD_AXES: usize = 4;
/// Sticks closer to center than this read as centered.
pub const DEAD_ZONE: f32 = 0.08;
/// Smaller moves than this aren't sent (the browser reports noise).
const STEP: f32 = 1.0 / 512.0;
/// Gamepads that get an index (as on the desktop).
pub const MAX_PADS: usize = 16;

/// The standard layout's button `index` (the Gamepad API's order).
pub fn standard_button(index: usize) -> Option<GamepadButton> {
    use GamepadButton::*;
    Some(match index {
        0 => South,
        1 => East,
        2 => West,
        3 => North,
        4 => LeftBumper,
        5 => RightBumper,
        6 => LeftTrigger,
        7 => RightTrigger,
        8 => Select,
        9 => Start,
        10 => LeftStick,
        11 => RightStick,
        12 => DpadUp,
        13 => DpadDown,
        14 => DpadLeft,
        15 => DpadRight,
        16 => Mode,
        _ => return None,
    })
}

/// The standard layout's axis `index`, and whether its sign is turned
/// around (the browser's Y axes point down).
fn standard_axis(index: usize) -> Option<(GamepadAxis, bool)> {
    Some(match index {
        0 => (GamepadAxis::LeftX, false),
        1 => (GamepadAxis::LeftY, true),
        2 => (GamepadAxis::RightX, false),
        3 => (GamepadAxis::RightY, true),
        _ => return None,
    })
}

/// One gamepad as the browser reports it this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadState {
    /// The browser's name for it.
    pub name: String,
    /// Button values 0–1 in the standard layout's order.
    pub buttons: [f32; STANDARD_BUTTONS],
    /// Axis values −1 to 1 in the standard layout's order.
    pub axes: [f32; STANDARD_AXES],
}

fn clean(v: f32, lo: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, 1.0) } else { 0.0 }
}

fn stick(v: f32) -> f32 {
    let v = clean(v, -1.0);
    if v.abs() < DEAD_ZONE { 0.0 } else { v }
}

/// What each connected gamepad last reported, by index.
#[derive(Clone, Debug, Default)]
pub struct Pads {
    last: Vec<Option<PadState>>,
}

impl Pads {
    /// The connected gamepads, by index, with their names.
    pub fn connected(&self) -> impl Iterator<Item = (usize, &str)> {
        self.last
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i, p.name.as_str())))
    }

    /// Compare `now` (the browser's gamepads, by its index; `None` where a
    /// slot is empty) with last frame and emit a control event for every
    /// change. A gamepad that went away releases what it held.
    pub fn update(&mut self, now: &[Option<PadState>], mut emit: impl FnMut(Control, f32)) {
        let n = now.len().max(self.last.len()).min(MAX_PADS);
        self.last.resize(n, None);
        for (index, last) in self.last.iter_mut().enumerate() {
            let pad = index as u8;
            let rest = PadState::default();
            let new = now.get(index).cloned().flatten();
            let before = last.as_ref().unwrap_or(&rest);
            let after = new.as_ref().unwrap_or(&rest);
            for i in 0..STANDARD_BUTTONS {
                let Some(button) = standard_button(i) else {
                    continue;
                };
                let (a, b) = (clean(before.buttons[i], 0.0), clean(after.buttons[i], 0.0));
                let changed = (a - b).abs() >= STEP || (a != b && (b == 0.0 || b == 1.0));
                if !changed {
                    continue;
                }
                emit(Control::GamepadButton { pad, button }, b);
                let axis = match button {
                    GamepadButton::LeftTrigger => Some(GamepadAxis::LeftTrigger),
                    GamepadButton::RightTrigger => Some(GamepadAxis::RightTrigger),
                    _ => None,
                };
                if let Some(axis) = axis {
                    emit(Control::GamepadAxis { pad, axis }, b);
                }
            }
            for i in 0..STANDARD_AXES {
                let Some((axis, flip)) = standard_axis(i) else {
                    continue;
                };
                let sign = if flip { -1.0 } else { 1.0 };
                let a = stick(before.axes[i]) * sign;
                let b = stick(after.axes[i]) * sign;
                if (a - b).abs() >= STEP || (a != b && b == 0.0) {
                    emit(Control::GamepadAxis { pad, axis }, b);
                }
            }
            *last = new;
        }
        while self.last.last().is_some_and(Option::is_none) {
            self.last.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(f: impl FnOnce(&mut PadState)) -> Option<PadState> {
        let mut p = PadState {
            name: "Pad".into(),
            ..PadState::default()
        };
        f(&mut p);
        Some(p)
    }

    fn events(pads: &mut Pads, now: &[Option<PadState>]) -> Vec<(Control, f32)> {
        let mut v = Vec::new();
        pads.update(now, |c, x| v.push((c, x)));
        v
    }

    #[test]
    fn the_standard_layout_maps_every_button() {
        let mut seen: Vec<GamepadButton> =
            (0..STANDARD_BUTTONS).filter_map(standard_button).collect();
        seen.sort();
        let mut all = GamepadButton::ALL.to_vec();
        all.sort();
        assert_eq!(seen, all);
        assert_eq!(standard_button(17), None);
    }

    #[test]
    fn presses_and_releases_are_sent_once() {
        let mut pads = Pads::default();
        assert!(events(&mut pads, &[pad(|_| {})]).is_empty(), "at rest");
        let south = Control::GamepadButton {
            pad: 0,
            button: GamepadButton::South,
        };
        let down = pad(|p| p.buttons[0] = 1.0);
        assert_eq!(
            events(&mut pads, std::slice::from_ref(&down)),
            vec![(south, 1.0)]
        );
        assert!(events(&mut pads, &[down]).is_empty(), "held: no repeat");
        assert_eq!(events(&mut pads, &[pad(|_| {})]), vec![(south, 0.0)]);
        assert_eq!(pads.connected().collect::<Vec<_>>(), vec![(0, "Pad")]);
    }

    #[test]
    fn triggers_are_buttons_and_axes_and_sticks_point_up() {
        let mut pads = Pads::default();
        let got = events(
            &mut pads,
            &[
                None,
                pad(|p| {
                    p.buttons[7] = 0.5;
                    p.axes[1] = -1.0;
                    p.axes[2] = 0.03;
                }),
            ],
        );
        assert_eq!(
            got,
            vec![
                (
                    Control::GamepadButton {
                        pad: 1,
                        button: GamepadButton::RightTrigger
                    },
                    0.5
                ),
                (
                    Control::GamepadAxis {
                        pad: 1,
                        axis: GamepadAxis::RightTrigger
                    },
                    0.5
                ),
                (
                    Control::GamepadAxis {
                        pad: 1,
                        axis: GamepadAxis::LeftY
                    },
                    1.0
                ),
            ],
            "pushed up reads as +1; a stick inside the dead zone is centered"
        );
    }

    #[test]
    fn a_gamepad_that_leaves_lets_go() {
        let mut pads = Pads::default();
        events(
            &mut pads,
            &[pad(|p| {
                p.buttons[12] = 1.0;
                p.axes[0] = 0.7;
            })],
        );
        let got = events(&mut pads, &[None]);
        assert!(got.contains(&(
            Control::GamepadButton {
                pad: 0,
                button: GamepadButton::DpadUp
            },
            0.0
        )));
        assert!(got.contains(&(
            Control::GamepadAxis {
                pad: 0,
                axis: GamepadAxis::LeftX
            },
            0.0
        )));
        assert_eq!(pads.connected().count(), 0);
        assert!(events(&mut pads, &[]).is_empty());
    }

    #[test]
    fn noise_and_nonsense_are_ignored() {
        let mut pads = Pads::default();
        events(&mut pads, &[pad(|p| p.axes[0] = 0.5)]);
        assert!(events(&mut pads, &[pad(|p| p.axes[0] = 0.5001)]).is_empty());
        let got = events(&mut pads, &[pad(|p| p.axes[0] = f32::NAN)]);
        assert_eq!(
            got,
            vec![(
                Control::GamepadAxis {
                    pad: 0,
                    axis: GamepadAxis::LeftX
                },
                0.0
            )]
        );
    }
}
