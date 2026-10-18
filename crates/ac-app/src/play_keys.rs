//! Play keys: the computer keyboard as a control source (ARCHITECTURE §11).
//!
//! While it's on, key presses and releases go to the engine as
//! `Control::Key` events (1 down, 0 up) instead of running canvas
//! shortcuts. Escape keeps its usual job, keys held with Ctrl, Cmd, or Alt
//! still run their shortcuts, and text fields still get typing. Auto-repeat
//! is ignored, and every held key is released when Play keys turns off,
//! when the window loses focus, or when typing starts, so no key is left
//! down in the patch.

use ac_core::Key;

use crate::audio::EngineHost;

/// The control key an egui key plays, if any. Shifted symbols play the key
/// they're on (`+` is `Equals`, `?` is `Slash`).
pub fn map_key(key: egui::Key) -> Option<Key> {
    use egui::Key as E;
    let name = match key {
        E::A => "A",
        E::B => "B",
        E::C => "C",
        E::D => "D",
        E::E => "E",
        E::F => "F",
        E::G => "G",
        E::H => "H",
        E::I => "I",
        E::J => "J",
        E::K => "K",
        E::L => "L",
        E::M => "M",
        E::N => "N",
        E::O => "O",
        E::P => "P",
        E::Q => "Q",
        E::R => "R",
        E::S => "S",
        E::T => "T",
        E::U => "U",
        E::V => "V",
        E::W => "W",
        E::X => "X",
        E::Y => "Y",
        E::Z => "Z",
        E::Num0 => "0",
        E::Num1 => "1",
        E::Num2 => "2",
        E::Num3 => "3",
        E::Num4 => "4",
        E::Num5 => "5",
        E::Num6 => "6",
        E::Num7 => "7",
        E::Num8 => "8",
        E::Num9 => "9",
        E::F1 => "F1",
        E::F2 => "F2",
        E::F3 => "F3",
        E::F4 => "F4",
        E::F5 => "F5",
        E::F6 => "F6",
        E::F7 => "F7",
        E::F8 => "F8",
        E::F9 => "F9",
        E::F10 => "F10",
        E::F11 => "F11",
        E::F12 => "F12",
        E::Space => "Space",
        E::Enter => "Enter",
        E::Tab => "Tab",
        E::Backspace => "Backspace",
        E::ArrowUp => "Up",
        E::ArrowDown => "Down",
        E::ArrowLeft => "Left",
        E::ArrowRight => "Right",
        E::Insert => "Insert",
        E::Delete => "Delete",
        E::Home => "Home",
        E::End => "End",
        E::PageUp => "PageUp",
        E::PageDown => "PageDown",
        E::Minus => "Minus",
        E::Equals | E::Plus => "Equals",
        E::OpenBracket | E::OpenCurlyBracket => "LeftBracket",
        E::CloseBracket | E::CloseCurlyBracket => "RightBracket",
        E::Backslash | E::Pipe => "Backslash",
        E::Semicolon | E::Colon => "Semicolon",
        E::Quote => "Quote",
        E::Backtick => "Backtick",
        E::Comma => "Comma",
        E::Period => "Period",
        E::Slash | E::Questionmark => "Slash",
        _ => return None,
    };
    Key::from_name(name)
}

/// One key event as egui reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyInput {
    pub key: egui::Key,
    pub physical: Option<egui::Key>,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: egui::Modifiers,
}

/// Whether Play keys is on, and which keys it's holding down.
#[derive(Debug, Default)]
pub struct PlayKeys {
    on: bool,
    /// Keys sent down and not yet up: the control key and the egui key
    /// (logical, physical) that pressed it.
    held: Vec<(Key, egui::Key, Option<egui::Key>)>,
}

impl PlayKeys {
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Turn Play keys on or off; off releases every held key.
    pub fn set(&mut self, on: bool, engine: &mut EngineHost) {
        if !on {
            self.release_all(engine);
        }
        self.on = on;
    }

    /// Keys currently held down in the patch.
    pub fn held(&self) -> impl Iterator<Item = Key> + '_ {
        self.held.iter().map(|h| h.0)
    }

    /// Send a release for every held key.
    pub fn release_all(&mut self, engine: &mut EngineHost) {
        for (k, _, _) in self.held.drain(..) {
            engine.send_key(k, false);
        }
    }

    /// Route one key event. Returns true when it went to the patch (and so
    /// must not run a shortcut).
    pub fn key(&mut self, input: KeyInput, engine: &mut EngineHost) -> bool {
        if !self.on || input.key == egui::Key::Escape {
            return false;
        }
        if !input.pressed {
            // Release whatever this key pressed, whatever the modifiers are
            // now (a shifted symbol can come up as its other self).
            let mapped = map_key(input.key);
            let found = self.held.iter().position(|(k, logical, physical)| {
                *logical == input.key
                    || Some(*k) == mapped
                    || (physical.is_some() && *physical == input.physical)
            });
            return match found {
                Some(i) => {
                    let (k, _, _) = self.held.remove(i);
                    engine.send_key(k, false);
                    true
                }
                None => false,
            };
        }
        let m = input.modifiers;
        if m.command || m.ctrl || m.alt || m.mac_cmd {
            return false;
        }
        let Some(k) = map_key(input.key).or_else(|| input.physical.and_then(map_key)) else {
            return false;
        };
        if input.repeat || self.held.iter().any(|h| h.0 == k) {
            // Auto-repeat, or a second press without a release: already down.
            return true;
        }
        self.held.push((k, input.key, input.physical));
        engine.send_key(k, true);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioMode;
    use std::sync::Arc;

    fn engine() -> EngineHost {
        EngineHost::start(
            &ac_core::Project::default(),
            Arc::new(ac_nodes::registry()),
            Arc::new(ac_core::NoResources),
            &AudioMode::Off,
        )
    }

    fn input(key: egui::Key, pressed: bool) -> KeyInput {
        KeyInput {
            key,
            physical: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn name(k: Option<Key>) -> Option<&'static str> {
        k.map(Key::name)
    }

    #[test]
    fn keys_map_by_what_they_say() {
        assert_eq!(name(map_key(egui::Key::Q)), Some("Q"));
        assert_eq!(name(map_key(egui::Key::Num7)), Some("7"));
        assert_eq!(name(map_key(egui::Key::ArrowLeft)), Some("Left"));
        assert_eq!(name(map_key(egui::Key::Plus)), Some("Equals"));
        assert_eq!(name(map_key(egui::Key::Questionmark)), Some("Slash"));
        assert_eq!(name(map_key(egui::Key::F12)), Some("F12"));
        assert_eq!(map_key(egui::Key::F13), None);
        assert_eq!(map_key(egui::Key::Copy), None);
        // Every name used here is a real key.
        for k in [
            egui::Key::A,
            egui::Key::Z,
            egui::Key::Space,
            egui::Key::Backtick,
            egui::Key::PageDown,
            egui::Key::CloseCurlyBracket,
        ] {
            assert!(map_key(k).is_some(), "{k:?}");
        }
    }

    #[test]
    fn presses_and_releases_reach_the_engine_once() {
        let mut e = engine();
        let mut keys = PlayKeys::default();
        assert!(
            !keys.key(input(egui::Key::A, true), &mut e),
            "off: not routed"
        );
        keys.set(true, &mut e);
        assert!(keys.key(input(egui::Key::A, true), &mut e));
        let repeat = KeyInput {
            repeat: true,
            ..input(egui::Key::A, true)
        };
        assert!(keys.key(repeat, &mut e), "repeats are swallowed");
        assert_eq!(e.keys_sent(), 1);
        assert!(keys.key(input(egui::Key::A, false), &mut e));
        assert_eq!(e.keys_sent(), 2);
        assert!(!keys.key(input(egui::Key::A, false), &mut e), "not held");
        // Escape and Ctrl+key stay with the editor.
        assert!(!keys.key(input(egui::Key::Escape, true), &mut e));
        let save = KeyInput {
            modifiers: egui::Modifiers::COMMAND,
            ..input(egui::Key::S, true)
        };
        assert!(!keys.key(save, &mut e));
        // Shift plays the key it's on.
        let shifted = KeyInput {
            key: egui::Key::Exclamationmark,
            physical: Some(egui::Key::Num1),
            modifiers: egui::Modifiers::SHIFT,
            ..input(egui::Key::Num1, true)
        };
        assert!(keys.key(shifted, &mut e));
        assert_eq!(keys.held().map(Key::name).collect::<Vec<_>>(), ["1"]);
        assert_eq!(e.keys_sent(), 3);
    }

    #[test]
    fn turning_off_releases_held_keys() {
        let mut e = engine();
        let mut keys = PlayKeys::default();
        keys.set(true, &mut e);
        keys.key(input(egui::Key::Z, true), &mut e);
        keys.key(input(egui::Key::X, true), &mut e);
        assert_eq!(e.keys_sent(), 2);
        keys.set(false, &mut e);
        assert_eq!(e.keys_sent(), 4, "two releases");
        assert_eq!(keys.held().count(), 0);
        assert!(!keys.is_on());
    }
}
