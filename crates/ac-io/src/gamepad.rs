//! Gamepads (ARCHITECTURE §11), through gilrs.
//!
//! [`Gamepads`] runs a thread that waits on gilrs events and turns them into
//! control events for its [`ControlSender`], stamped as they arrive:
//!
//! - Buttons are 0–1 (`Control::GamepadButton`). The analog triggers are
//!   also axes (`GamepadAxis::LeftTrigger`/`RightTrigger`, 0–1), so a
//!   trigger works both as a button and as a fader.
//! - Sticks are −1 to 1, right and up positive (`Control::GamepadAxis`).
//!   gilrs applies its usual dead zone first.
//! - A gamepad's index is the first one free when it connects (the first
//!   controller is 0, the second 1) and stays put while it's connected.
//!   When a gamepad goes away, its held buttons are released and its axes
//!   return to rest, so nothing hangs.
//!
//! No gamepad subsystem (a container, an unsupported platform) is reported,
//! never fatal.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ac_core::{Control, GamepadAxis, GamepadButton};
use gilrs::{Axis, Button, EventType, Gilrs};

use crate::clock;
use crate::control::ControlSender;

/// Gamepads that get an index; more are ignored until one leaves.
pub const MAX_PADS: usize = 16;
/// How long the thread waits for an event before checking whether to stop.
const WAIT: Duration = Duration::from_millis(20);
/// How long [`Gamepads::start`] waits for the gamepad system to open.
const INIT_TIMEOUT: Duration = Duration::from_secs(3);

/// A connected gamepad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamepadInfo {
    /// The `pad` in its control events.
    pub index: u8,
    pub name: String,
}

/// The gamepad system couldn't be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamepadError(pub String);

impl fmt::Display for GamepadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GamepadError {}

/// The standard-layout button a gilrs button is, if any. gilrs calls the
/// bumpers `LeftTrigger`/`RightTrigger` and the analog triggers
/// `LeftTrigger2`/`RightTrigger2`.
pub fn map_button(b: Button) -> Option<GamepadButton> {
    Some(match b {
        Button::South => GamepadButton::South,
        Button::East => GamepadButton::East,
        Button::West => GamepadButton::West,
        Button::North => GamepadButton::North,
        Button::LeftTrigger => GamepadButton::LeftBumper,
        Button::RightTrigger => GamepadButton::RightBumper,
        Button::LeftTrigger2 => GamepadButton::LeftTrigger,
        Button::RightTrigger2 => GamepadButton::RightTrigger,
        Button::Select => GamepadButton::Select,
        Button::Start => GamepadButton::Start,
        Button::Mode => GamepadButton::Mode,
        Button::LeftThumb => GamepadButton::LeftStick,
        Button::RightThumb => GamepadButton::RightStick,
        Button::DPadUp => GamepadButton::DpadUp,
        Button::DPadDown => GamepadButton::DpadDown,
        Button::DPadLeft => GamepadButton::DpadLeft,
        Button::DPadRight => GamepadButton::DpadRight,
        Button::C | Button::Z | Button::Unknown => return None,
    })
}

/// The axis a gilrs axis is, if any. `LeftZ`/`RightZ` are triggers that a
/// gamepad without a known layout reports as axes; the D-pad axes arrive as
/// buttons (gilrs converts them).
pub fn map_axis(a: Axis) -> Option<GamepadAxis> {
    Some(match a {
        Axis::LeftStickX => GamepadAxis::LeftX,
        Axis::LeftStickY => GamepadAxis::LeftY,
        Axis::RightStickX => GamepadAxis::RightX,
        Axis::RightStickY => GamepadAxis::RightY,
        Axis::LeftZ => GamepadAxis::LeftTrigger,
        Axis::RightZ => GamepadAxis::RightTrigger,
        Axis::DPadX | Axis::DPadY | Axis::Unknown => return None,
    })
}

/// A value change on a gamepad, as gilrs reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadInput {
    Button(Button, f32),
    Axis(Axis, f32),
}

/// The control events one gamepad input becomes, for the gamepad at `pad`.
pub fn translate(pad: u8, input: PadInput, mut emit: impl FnMut(Control, f32)) {
    let trigger_axis = |b| match b {
        GamepadButton::LeftTrigger => Some(GamepadAxis::LeftTrigger),
        GamepadButton::RightTrigger => Some(GamepadAxis::RightTrigger),
        _ => None,
    };
    let trigger_button = |a| match a {
        GamepadAxis::LeftTrigger => Some(GamepadButton::LeftTrigger),
        GamepadAxis::RightTrigger => Some(GamepadButton::RightTrigger),
        _ => None,
    };
    match input {
        PadInput::Button(b, v) => {
            let Some(button) = map_button(b) else { return };
            let v = finite_or_zero(v).clamp(0.0, 1.0);
            emit(Control::GamepadButton { pad, button }, v);
            if let Some(axis) = trigger_axis(button) {
                emit(Control::GamepadAxis { pad, axis }, v);
            }
        }
        PadInput::Axis(a, v) => {
            let Some(axis) = map_axis(a) else { return };
            let v = finite_or_zero(v).clamp(-1.0, 1.0);
            if axis.is_trigger() {
                // gilrs scales these −1 (rest) to 1 (pressed).
                let v = (v + 1.0) * 0.5;
                emit(Control::GamepadAxis { pad, axis }, v);
                if let Some(button) = trigger_button(axis) {
                    emit(Control::GamepadButton { pad, button }, v);
                }
            } else {
                emit(Control::GamepadAxis { pad, axis }, v);
            }
        }
    }
}

fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// One connected gamepad's place and what it's holding.
struct Pad<I> {
    id: I,
    name: String,
    /// Buttons away from 0, by position in [`GamepadButton::ALL`].
    held: u32,
    /// Axes away from rest, by position in [`GamepadAxis::ALL`].
    moved: u8,
}

/// Gamepad indices: the first free slot on connect, kept while connected.
/// Generic over gilrs's id so tests can use plain numbers.
struct PadTable<I> {
    slots: Vec<Option<Pad<I>>>,
}

impl<I: Copy + PartialEq> PadTable<I> {
    fn new() -> Self {
        PadTable {
            slots: (0..MAX_PADS).map(|_| None).collect(),
        }
    }

    fn index_of(&self, id: I) -> Option<u8> {
        self.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|p| p.id == id))
            .map(|i| i as u8)
    }

    /// Give `id` an index (its old one if it already has one).
    fn connect(&mut self, id: I, name: &str) -> Option<u8> {
        if let Some(i) = self.index_of(id) {
            return Some(i);
        }
        let i = self.slots.iter().position(Option::is_none)?;
        self.slots[i] = Some(Pad {
            id,
            name: name.to_string(),
            held: 0,
            moved: 0,
        });
        Some(i as u8)
    }

    /// Free `id`'s index, releasing whatever it held.
    fn disconnect(&mut self, id: I, mut emit: impl FnMut(Control, f32)) -> Option<u8> {
        let i = self.index_of(id)?;
        let pad = self.slots.get_mut(usize::from(i))?.take()?;
        for (k, button) in GamepadButton::ALL.iter().enumerate() {
            if pad.held & (1 << k) != 0 {
                emit(
                    Control::GamepadButton {
                        pad: i,
                        button: *button,
                    },
                    0.0,
                );
            }
        }
        for (k, axis) in GamepadAxis::ALL.iter().enumerate() {
            if pad.moved & (1 << k) != 0 {
                emit(
                    Control::GamepadAxis {
                        pad: i,
                        axis: *axis,
                    },
                    0.0,
                );
            }
        }
        Some(i)
    }

    /// Translate an input from `id`, remembering what's held.
    fn input(&mut self, id: I, input: PadInput, mut emit: impl FnMut(Control, f32)) {
        let Some(i) = self.index_of(id) else { return };
        let Some(pad) = self.slots.get_mut(usize::from(i)).and_then(Option::as_mut) else {
            return;
        };
        translate(i, input, |c, v| {
            match c {
                Control::GamepadButton { button, .. } => {
                    if let Some(k) = GamepadButton::ALL.iter().position(|b| *b == button) {
                        set_bit32(&mut pad.held, k, v != 0.0);
                    }
                }
                Control::GamepadAxis { axis, .. } => {
                    if let Some(k) = GamepadAxis::ALL.iter().position(|a| *a == axis) {
                        set_bit8(&mut pad.moved, k, v != 0.0);
                    }
                }
                _ => {}
            }
            emit(c, v);
        });
    }

    fn list(&self) -> Vec<GamepadInfo> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref().map(|p| GamepadInfo {
                    index: i as u8,
                    name: p.name.clone(),
                })
            })
            .collect()
    }
}

fn set_bit32(bits: &mut u32, k: usize, on: bool) {
    if on {
        *bits |= 1 << k;
    } else {
        *bits &= !(1 << k);
    }
}

fn set_bit8(bits: &mut u8, k: usize, on: bool) {
    if on {
        *bits |= 1 << k;
    } else {
        *bits &= !(1 << k);
    }
}

fn open() -> Result<Gilrs, GamepadError> {
    match Gilrs::new() {
        Ok(g) => Ok(g),
        Err(gilrs::Error::NotImplemented(_)) => Err(GamepadError(
            "gamepads aren't supported on this system".into(),
        )),
        Err(e) => Err(GamepadError(format!("gamepads unavailable: {e}"))),
    }
}

/// The gamepads connected now (a one-off scan for `audioc devices`; the
/// editor asks its running [`Gamepads`] instead).
pub fn list_gamepads() -> Result<Vec<GamepadInfo>, GamepadError> {
    let gilrs = open()?;
    let mut table = PadTable::new();
    for (id, g) in gilrs.gamepads() {
        table.connect(id, g.name());
    }
    Ok(table.list())
}

#[derive(Debug, Default)]
struct Shared {
    pads: Mutex<Vec<GamepadInfo>>,
    error: Mutex<Option<String>>,
    stop: AtomicBool,
    events: AtomicU64,
    /// Bumped on every connect and disconnect.
    generation: AtomicU64,
}

impl Shared {
    fn publish(&self, list: Vec<GamepadInfo>) {
        if let Ok(mut p) = self.pads.lock() {
            *p = list;
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    fn fail(&self, e: String) {
        if let Ok(mut err) = self.error.lock() {
            *err = Some(e);
        }
    }
}

/// The gamepad thread. Dropping it stops the thread.
pub struct Gamepads {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for Gamepads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gamepads")
            .field("connected", &self.connected())
            .field("error", &self.error())
            .finish_non_exhaustive()
    }
}

impl Gamepads {
    /// Start listening to gamepads, feeding `sender`. Waits (briefly) for
    /// the gamepad system to open so [`Gamepads::connected`] and
    /// [`Gamepads::error`] are meaningful right away. Never fails: a missing
    /// gamepad system is reported by [`Gamepads::error`].
    pub fn start(sender: ControlSender) -> Gamepads {
        clock::init();
        let shared = Arc::new(Shared::default());
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ac-gamepads".into())
            .spawn(move || run(sender, &s, &ready_tx));
        let thread = match thread {
            Ok(t) => {
                let _ = ready_rx.recv_timeout(INIT_TIMEOUT);
                Some(t)
            }
            Err(e) => {
                shared.fail(format!("couldn't start the gamepad thread: {e}"));
                None
            }
        };
        Gamepads { shared, thread }
    }

    /// The connected gamepads, by index.
    pub fn connected(&self) -> Vec<GamepadInfo> {
        self.shared
            .pads
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Why gamepads aren't available, if they aren't.
    pub fn error(&self) -> Option<String> {
        let e = self.shared.error.lock().ok().and_then(|e| e.clone());
        if e.is_none() && !self.is_running() {
            return Some("the gamepad thread stopped".into());
        }
        e
    }

    /// Whether the thread is listening.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Control events sent so far.
    pub fn events(&self) -> u64 {
        self.shared.events.load(Ordering::Relaxed)
    }

    /// Changes with every connect and disconnect (to notice hot-plugging).
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Relaxed)
    }
}

impl Drop for Gamepads {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(mut sender: ControlSender, shared: &Shared, ready: &mpsc::SyncSender<()>) {
    let mut gilrs = match open() {
        Ok(g) => g,
        Err(e) => {
            shared.fail(e.0);
            let _ = ready.try_send(());
            return;
        }
    };
    let mut table = PadTable::new();
    for (id, g) in gilrs.gamepads() {
        table.connect(id, g.name());
    }
    shared.publish(table.list());
    let _ = ready.try_send(());
    while !shared.stop.load(Ordering::Relaxed) {
        let asked = Instant::now();
        let Some(ev) = gilrs.next_event_blocking(Some(WAIT)) else {
            // Nothing within the wait. If the wait returned at once, the
            // backend has failed (its event source closed): don't spin.
            if asked.elapsed() < WAIT / 4 {
                std::thread::sleep(WAIT);
            }
            continue;
        };
        let t = clock::now_ns();
        let mut emit = |c: Control, v: f32| {
            sender.send_at(t, c, v);
            shared.events.fetch_add(1, Ordering::Relaxed);
        };
        match ev.event {
            EventType::Connected => {
                let name = gilrs.gamepad(ev.id).name().to_string();
                table.connect(ev.id, &name);
                shared.publish(table.list());
            }
            EventType::Disconnected => {
                table.disconnect(ev.id, &mut emit);
                shared.publish(table.list());
            }
            EventType::ButtonChanged(b, v, _) => {
                table.input(ev.id, PadInput::Button(b, v), &mut emit);
            }
            EventType::AxisChanged(a, v, _) => {
                table.input(ev.id, PadInput::Axis(a, v), &mut emit);
            }
            // Pressed/released/repeated duplicate ButtonChanged.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlInputs;

    fn collect(pad: u8, input: PadInput) -> Vec<(Control, f32)> {
        let mut v = Vec::new();
        translate(pad, input, |c, x| v.push((c, x)));
        v
    }

    #[test]
    fn every_standard_button_maps_once() {
        let gilrs_buttons = [
            Button::South,
            Button::East,
            Button::West,
            Button::North,
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::LeftTrigger2,
            Button::RightTrigger2,
            Button::Select,
            Button::Start,
            Button::Mode,
            Button::LeftThumb,
            Button::RightThumb,
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
        ];
        let mut mapped: Vec<GamepadButton> = gilrs_buttons
            .iter()
            .map(|b| map_button(*b).unwrap())
            .collect();
        mapped.sort();
        let mut all = GamepadButton::ALL.to_vec();
        all.sort();
        assert_eq!(mapped, all, "every ac-core button has a gilrs button");
        assert_eq!(map_button(Button::C), None);
        assert_eq!(map_button(Button::Unknown), None);
        assert_eq!(
            map_button(Button::LeftTrigger),
            Some(GamepadButton::LeftBumper)
        );
        assert_eq!(
            map_button(Button::RightTrigger2),
            Some(GamepadButton::RightTrigger)
        );
    }

    #[test]
    fn axes_map_and_dpad_axes_are_left_to_buttons() {
        assert_eq!(map_axis(Axis::LeftStickX), Some(GamepadAxis::LeftX));
        assert_eq!(map_axis(Axis::RightStickY), Some(GamepadAxis::RightY));
        assert_eq!(map_axis(Axis::RightZ), Some(GamepadAxis::RightTrigger));
        assert_eq!(map_axis(Axis::DPadX), None);
        assert_eq!(map_axis(Axis::Unknown), None);
    }

    #[test]
    fn analog_triggers_are_buttons_and_axes() {
        let lt = collect(2, PadInput::Button(Button::LeftTrigger2, 0.4));
        assert_eq!(
            lt,
            vec![
                (
                    Control::GamepadButton {
                        pad: 2,
                        button: GamepadButton::LeftTrigger
                    },
                    0.4
                ),
                (
                    Control::GamepadAxis {
                        pad: 2,
                        axis: GamepadAxis::LeftTrigger
                    },
                    0.4
                ),
            ]
        );
        // As a raw axis (−1 rest to 1 pressed), halfway is 0.5.
        let rz = collect(0, PadInput::Axis(Axis::RightZ, 0.0));
        assert_eq!(
            rz,
            vec![
                (
                    Control::GamepadAxis {
                        pad: 0,
                        axis: GamepadAxis::RightTrigger
                    },
                    0.5
                ),
                (
                    Control::GamepadButton {
                        pad: 0,
                        button: GamepadButton::RightTrigger
                    },
                    0.5
                ),
            ]
        );
        // A bumper is only a button.
        assert_eq!(
            collect(0, PadInput::Button(Button::LeftTrigger, 1.0)).len(),
            1
        );
    }

    #[test]
    fn values_are_clamped_and_finite() {
        let stick = collect(0, PadInput::Axis(Axis::LeftStickY, 3.0));
        assert_eq!(stick[0].1, 1.0);
        let nan = collect(0, PadInput::Axis(Axis::LeftStickX, f32::NAN));
        assert_eq!(nan[0].1, 0.0);
        let b = collect(0, PadInput::Button(Button::South, -1.0));
        assert_eq!(b[0].1, 0.0);
        assert!(collect(0, PadInput::Button(Button::Unknown, 1.0)).is_empty());
    }

    #[test]
    fn indices_follow_connection_order_and_stay_put() {
        let mut t = PadTable::<usize>::new();
        assert_eq!(t.connect(10, "first"), Some(0));
        assert_eq!(t.connect(11, "second"), Some(1));
        assert_eq!(t.connect(10, "first"), Some(0), "already connected");
        t.disconnect(10, |_, _| {});
        assert_eq!(t.index_of(11), Some(1), "the second keeps its index");
        assert_eq!(t.connect(12, "third"), Some(0), "takes the free index");
        assert_eq!(
            t.list(),
            vec![
                GamepadInfo {
                    index: 0,
                    name: "third".into()
                },
                GamepadInfo {
                    index: 1,
                    name: "second".into()
                },
            ]
        );
        let mut full = PadTable::<usize>::new();
        for i in 0..MAX_PADS {
            assert!(full.connect(i, "p").is_some());
        }
        assert_eq!(full.connect(99, "one too many"), None);
        // Input from a gamepad without an index is ignored.
        let mut n = 0;
        full.input(99, PadInput::Button(Button::South, 1.0), |_, _| n += 1);
        assert_eq!(n, 0);
    }

    #[test]
    fn disconnecting_releases_what_was_held() {
        let mut t = PadTable::<usize>::new();
        t.connect(7, "pad");
        let mut sink = |_: Control, _: f32| {};
        t.input(7, PadInput::Button(Button::South, 1.0), &mut sink);
        t.input(7, PadInput::Button(Button::East, 1.0), &mut sink);
        t.input(7, PadInput::Button(Button::East, 0.0), &mut sink);
        t.input(7, PadInput::Axis(Axis::LeftStickX, -0.7), &mut sink);
        t.input(7, PadInput::Button(Button::RightTrigger2, 0.3), &mut sink);
        let mut released = Vec::new();
        assert_eq!(t.disconnect(7, |c, v| released.push((c, v))), Some(0));
        assert_eq!(
            released,
            vec![
                (
                    Control::GamepadButton {
                        pad: 0,
                        button: GamepadButton::South
                    },
                    0.0
                ),
                (
                    Control::GamepadButton {
                        pad: 0,
                        button: GamepadButton::RightTrigger
                    },
                    0.0
                ),
                (
                    Control::GamepadAxis {
                        pad: 0,
                        axis: GamepadAxis::LeftX
                    },
                    0.0
                ),
                (
                    Control::GamepadAxis {
                        pad: 0,
                        axis: GamepadAxis::RightTrigger
                    },
                    0.0
                ),
            ]
        );
        assert_eq!(t.disconnect(7, |_, _| {}), None);
    }

    #[test]
    fn the_thread_starts_and_stops_without_gamepads() {
        // CI machines have no gamepads and often no gamepad system; either
        // way the thread must start, report, and stop cleanly.
        let (mut inputs, _rx) = ControlInputs::new();
        let pads = Gamepads::start(inputs.sender("gamepads").unwrap());
        let _ = pads.connected();
        if !pads.is_running() {
            assert!(pads.error().is_some(), "a stopped thread says why");
        }
        let t0 = std::time::Instant::now();
        drop(pads);
        assert!(t0.elapsed() < Duration::from_secs(2), "stops promptly");
        let _ = list_gamepads();
    }
}
