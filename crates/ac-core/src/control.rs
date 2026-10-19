//! Control-surface events: computer keys, gamepad controls, Open Sound
//! Control, and a plugin host's macro parameters (ARCHITECTURE §8 "Events
//! in", §11, §13).
//!
//! Like MIDI, they arrive on their own threads, wait in per-source rings,
//! and reach the engine one block late with a sample offset
//! ([`ControlEvent`]). Everything here is `Copy` and fixed-size so the audio
//! thread can pass it around without allocating; an OSC address travels as
//! its [`osc_address_hash`], and an outgoing [`OscMessage`] keeps its address
//! inline.

use std::fmt;
use std::net::SocketAddr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::util::Fnv64;

/// Control events the engine accepts per host block, in the engine and the
/// test harness. Past it events wait or are dropped (and counted) by the host.
pub const CONTROL_CAPACITY: usize = 1024;
/// OSC messages `OscOut` nodes can send per host block.
pub const OSC_OUT_CAPACITY: usize = 256;
/// The longest OSC address an [`OscMessage`] holds, in bytes.
pub const OSC_ADDRESS_MAX: usize = 64;
/// The most float arguments an [`OscMessage`] holds.
pub const OSC_ARGS_MAX: usize = 4;
/// Macro parameters the plugin build shows its host, `Macro 1` to
/// `Macro 16` ([`Control::Macro`]). Raising it adds parameters without
/// breaking saved states or automation (DECISIONS D119).
pub const MACRO_COUNT: usize = 16;

/// A control event stamped with its sample offset inside the current block.
///
/// Values by source:
/// - [`Control::Key`]: 1 when the key goes down, 0 when it comes up.
/// - [`Control::GamepadButton`]: 0–1 (most buttons are 0 or 1; analog
///   buttons report how far they're pressed).
/// - [`Control::GamepadAxis`]: sticks −1 to 1 (right and up positive),
///   triggers 0–1.
/// - [`Control::Osc`]: the argument as a float (integers converted, true and
///   false as 1 and 0). A message without arguments arrives as argument 0
///   with value 1, so it works as a trigger.
/// - [`Control::Macro`]: the host parameter's position, 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlEvent {
    pub offset: u32,
    pub control: Control,
    pub value: f32,
}

impl ControlEvent {
    pub fn new(offset: u32, control: Control, value: f32) -> Self {
        ControlEvent {
            offset,
            control,
            value,
        }
    }
}

/// Which control moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Key(Key),
    /// `pad` is the controller's index, in the order they were connected.
    GamepadButton {
        pad: u8,
        button: GamepadButton,
    },
    GamepadAxis {
        pad: u8,
        axis: GamepadAxis,
    },
    /// Argument `arg` of a message to the address whose
    /// [`osc_address_hash`] is `address`.
    Osc {
        address: u64,
        arg: u8,
    },
    /// A plugin host's macro parameter: `index` 0 is `Macro 1`, up to
    /// [`MACRO_COUNT`]` - 1`.
    Macro {
        index: u8,
    },
}

/// The hash an OSC address travels as. Exact match: no wildcards.
pub fn osc_address_hash(address: &str) -> u64 {
    let mut h = Fnv64::new();
    h.write(address.as_bytes());
    h.finish()
}

/// A computer key, independent of keyboard layout naming in any one
/// toolkit. Stored and shown by name ([`Key::name`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u8);

/// Every key's name, in code order.
const KEY_NAMES: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "Space",
    "Enter",
    "Tab",
    "Backspace",
    "Escape",
    "Up",
    "Down",
    "Left",
    "Right",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Minus",
    "Equals",
    "LeftBracket",
    "RightBracket",
    "Backslash",
    "Semicolon",
    "Quote",
    "Backtick",
    "Comma",
    "Period",
    "Slash",
];

impl Key {
    /// Every key, in code order.
    pub fn all() -> impl Iterator<Item = Key> {
        (0..KEY_NAMES.len() as u8).map(Key)
    }

    /// The key called `name` (as [`Key::name`] spells it, any case).
    pub fn from_name(name: &str) -> Option<Key> {
        KEY_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| Key(i as u8))
    }

    pub fn name(self) -> &'static str {
        KEY_NAMES.get(usize::from(self.0)).copied().unwrap_or("?")
    }

    /// The letter key for `c` (`a`–`z`, any case) or digit key (`0`–`9`).
    pub fn from_char(c: char) -> Option<Key> {
        match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => Some(Key(c as u8 - b'A')),
            c @ '0'..='9' => Some(Key(26 + (c as u8 - b'0'))),
            _ => None,
        }
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({})", self.name())
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Key::from_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown key {name:?}")))
    }
}

/// A gamepad button by its position on the standard layout (four face
/// buttons in a diamond, two bumpers, two triggers, a D-pad, two clickable
/// sticks, and three center buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadButton {
    /// The bottom face button (often labeled A or ✕).
    South,
    /// The right face button (B or ○).
    East,
    /// The left face button (X or □).
    West,
    /// The top face button (Y or △).
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    /// The center logo button.
    Mode,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl GamepadButton {
    pub const ALL: [GamepadButton; 17] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::West,
        GamepadButton::North,
        GamepadButton::LeftBumper,
        GamepadButton::RightBumper,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::Select,
        GamepadButton::Start,
        GamepadButton::Mode,
        GamepadButton::LeftStick,
        GamepadButton::RightStick,
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
    ];

    /// The name sockets and files use (`south`, `left_bumper`, ...).
    pub fn name(self) -> &'static str {
        match self {
            GamepadButton::South => "south",
            GamepadButton::East => "east",
            GamepadButton::West => "west",
            GamepadButton::North => "north",
            GamepadButton::LeftBumper => "left_bumper",
            GamepadButton::RightBumper => "right_bumper",
            GamepadButton::LeftTrigger => "left_trigger",
            GamepadButton::RightTrigger => "right_trigger",
            GamepadButton::Select => "select",
            GamepadButton::Start => "start",
            GamepadButton::Mode => "mode",
            GamepadButton::LeftStick => "left_stick",
            GamepadButton::RightStick => "right_stick",
            GamepadButton::DpadUp => "dpad_up",
            GamepadButton::DpadDown => "dpad_down",
            GamepadButton::DpadLeft => "dpad_left",
            GamepadButton::DpadRight => "dpad_right",
        }
    }

    /// A short label for people: the face buttons by their usual letters.
    pub fn label(self) -> &'static str {
        match self {
            GamepadButton::South => "A (south)",
            GamepadButton::East => "B (east)",
            GamepadButton::West => "X (west)",
            GamepadButton::North => "Y (north)",
            GamepadButton::LeftBumper => "Left bumper",
            GamepadButton::RightBumper => "Right bumper",
            GamepadButton::LeftTrigger => "Left trigger",
            GamepadButton::RightTrigger => "Right trigger",
            GamepadButton::Select => "Select",
            GamepadButton::Start => "Start",
            GamepadButton::Mode => "Mode",
            GamepadButton::LeftStick => "Left stick press",
            GamepadButton::RightStick => "Right stick press",
            GamepadButton::DpadUp => "D-pad up",
            GamepadButton::DpadDown => "D-pad down",
            GamepadButton::DpadLeft => "D-pad left",
            GamepadButton::DpadRight => "D-pad right",
        }
    }
}

/// A gamepad axis: the two sticks (−1 to 1) and the analog triggers (0–1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

impl GamepadAxis {
    pub const ALL: [GamepadAxis; 6] = [
        GamepadAxis::LeftX,
        GamepadAxis::LeftY,
        GamepadAxis::RightX,
        GamepadAxis::RightY,
        GamepadAxis::LeftTrigger,
        GamepadAxis::RightTrigger,
    ];

    /// The name sockets and files use (`left_x`, `right_trigger`, ...).
    pub fn name(self) -> &'static str {
        match self {
            GamepadAxis::LeftX => "left_x",
            GamepadAxis::LeftY => "left_y",
            GamepadAxis::RightX => "right_x",
            GamepadAxis::RightY => "right_y",
            GamepadAxis::LeftTrigger => "left_trigger_axis",
            GamepadAxis::RightTrigger => "right_trigger_axis",
        }
    }

    /// Whether the axis rests at 0 and goes one way (a trigger), rather than
    /// resting in the middle of −1 to 1 (a stick).
    pub fn is_trigger(self) -> bool {
        matches!(self, GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger)
    }
}

/// An OSC message for a device, built on the audio thread without
/// allocating: the address and up to [`OSC_ARGS_MAX`] float arguments are
/// kept inline.
#[derive(Clone, Copy, PartialEq)]
pub struct OscMessage {
    /// Sample offset inside the host block it was sent in.
    pub offset: u32,
    pub target: SocketAddr,
    address: [u8; OSC_ADDRESS_MAX],
    address_len: u8,
    args: [f32; OSC_ARGS_MAX],
    args_len: u8,
}

impl OscMessage {
    /// A message to `address` (which must start with `/` and fit in
    /// [`OSC_ADDRESS_MAX`] bytes) with the first [`OSC_ARGS_MAX`] of `args`.
    pub fn new(offset: u32, target: SocketAddr, address: &str, args: &[f32]) -> Option<Self> {
        if !valid_osc_address(address) {
            return None;
        }
        let mut a = [0u8; OSC_ADDRESS_MAX];
        a.get_mut(..address.len())?
            .copy_from_slice(address.as_bytes());
        let n = args.len().min(OSC_ARGS_MAX);
        let mut v = [0.0f32; OSC_ARGS_MAX];
        v.get_mut(..n)?.copy_from_slice(args.get(..n)?);
        Some(OscMessage {
            offset,
            target,
            address: a,
            address_len: address.len() as u8,
            args: v,
            args_len: n as u8,
        })
    }

    pub fn address(&self) -> &str {
        self.address
            .get(..usize::from(self.address_len))
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }

    pub fn args(&self) -> &[f32] {
        self.args.get(..usize::from(self.args_len)).unwrap_or(&[])
    }
}

impl fmt::Debug for OscMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscMessage")
            .field("offset", &self.offset)
            .field("target", &self.target)
            .field("address", &self.address())
            .field("args", &self.args())
            .finish()
    }
}

/// Whether `address` is a plain OSC address an [`OscMessage`] can carry:
/// starts with `/`, fits in [`OSC_ADDRESS_MAX`] bytes, printable ASCII
/// without spaces or the pattern characters `*?[]{}#,`.
pub fn valid_osc_address(address: &str) -> bool {
    address.starts_with('/')
        && address.len() <= OSC_ADDRESS_MAX
        && address
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"*?[]{}#,".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_by_name() {
        for k in Key::all() {
            assert_eq!(Key::from_name(k.name()), Some(k));
            let json = serde_json::to_string(&k).unwrap();
            assert_eq!(serde_json::from_str::<Key>(&json).unwrap(), k);
        }
        assert_eq!(Key::from_name("space"), Key::from_name("Space"));
        assert_eq!(Key::from_char('q'), Key::from_name("Q"));
        assert_eq!(Key::from_char('7'), Key::from_name("7"));
        assert_eq!(Key::from_char('!'), None);
        assert!(serde_json::from_str::<Key>("\"Hyper\"").is_err());
    }

    #[test]
    fn gamepad_names_are_unique() {
        let mut names: Vec<&str> = GamepadButton::ALL.iter().map(|b| b.name()).collect();
        names.extend(GamepadAxis::ALL.iter().map(|a| a.name()));
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            n,
            "sockets named after controls must not clash"
        );
        let json = serde_json::to_string(&GamepadButton::LeftBumper).unwrap();
        assert_eq!(json, "\"left_bumper\"");
    }

    #[test]
    fn osc_messages_hold_address_and_args_inline() {
        let to: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        let m = OscMessage::new(3, to, "/fader/1", &[0.5, 1.0]).unwrap();
        assert_eq!(m.address(), "/fader/1");
        assert_eq!(m.args(), [0.5, 1.0]);
        let many = OscMessage::new(0, to, "/x", &[1.0; 9]).unwrap();
        assert_eq!(many.args().len(), OSC_ARGS_MAX);
        assert!(OscMessage::new(0, to, "no-slash", &[]).is_none());
        assert!(OscMessage::new(0, to, "/with space", &[]).is_none());
        assert!(OscMessage::new(0, to, "/wild/*", &[]).is_none());
        let long = format!("/{}", "a".repeat(OSC_ADDRESS_MAX));
        assert!(OscMessage::new(0, to, &long, &[]).is_none());
        assert_ne!(osc_address_hash("/a"), osc_address_hash("/b"));
    }
}
