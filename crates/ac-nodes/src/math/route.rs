//! Shared plumbing for the routing nodes (`Select`, `Switch`): the `type`
//! config that sets their socket types, a click-free crossfader for audio,
//! and a held-note tracker that releases notes when a MIDI route changes.

use ac_core::{
    AudioIn, AudioOut, Config, ConfigExt, MidiEvent, MidiMsg, MidiOut, SocketSpec, Unit,
};

/// What a routing node carries, from its `type` config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteType {
    Float,
    Audio,
    Int,
    Bool,
    Trigger,
    Midi,
}

impl RouteType {
    pub fn from_config(config: &Config) -> RouteType {
        match config.get_str_or("type", "float") {
            "audio" => RouteType::Audio,
            "int" => RouteType::Int,
            "bool" => RouteType::Bool,
            "trigger" => RouteType::Trigger,
            "midi" => RouteType::Midi,
            // Unknown types fall back to the default rather than failing to load.
            _ => RouteType::Float,
        }
    }

    /// An input socket of this type.
    pub fn input(self, name: String) -> SocketSpec {
        match self {
            RouteType::Float => SocketSpec::float(name, Unit::Generic, 0.0),
            RouteType::Audio => SocketSpec::audio(name),
            RouteType::Int => SocketSpec::int(name, 0),
            RouteType::Bool => SocketSpec::boolean(name, false),
            RouteType::Trigger => SocketSpec::trigger(name),
            RouteType::Midi => SocketSpec::midi(name),
        }
    }

    /// The `out` socket: Float and Bool follow the fastest input, Audio
    /// follows the widest input, Int runs at block rate.
    pub fn output(self) -> SocketSpec {
        match self {
            RouteType::Float => SocketSpec::float("out", Unit::Generic, 0.0).poly(),
            RouteType::Bool => SocketSpec::boolean("out", false).poly(),
            other => other.input("out".to_string()),
        }
    }
}

/// Crossfade length when an audio route changes.
pub(crate) const FADE_SECONDS: f64 = 0.005;

/// Most routes a fader handles (`Select` allows 64 inputs).
pub(crate) const MAX_ROUTES: usize = 64;

/// Equal-gain crossfader over up to [`MAX_ROUTES`] audio inputs. Each
/// input's gain slews toward 1 while it is the target and toward 0
/// otherwise, at a rate that crosses fully in [`FADE_SECONDS`], so any
/// sequence of changes (even mid-fade) stays click-free.
#[derive(Debug)]
pub(crate) struct Fader {
    gains: Vec<f32>,
    scratch: Vec<f32>,
    step: f32,
    primed: bool,
}

impl Fader {
    pub fn new(routes: usize, sample_rate: f64) -> Fader {
        let n = routes.clamp(1, MAX_ROUTES);
        let samples = (FADE_SECONDS * sample_rate).max(1.0);
        Fader {
            gains: vec![0.0; n],
            scratch: vec![0.0; n],
            step: (1.0 / samples) as f32,
            primed: false,
        }
    }

    /// Forget the current route; the next block starts on its target with
    /// no fade.
    pub fn reset(&mut self) {
        self.primed = false;
    }

    fn settled_on(&self, t: usize) -> bool {
        self.gains
            .iter()
            .enumerate()
            .all(|(k, &g)| if k == t { g >= 1.0 } else { g <= 0.0 })
    }

    /// Write one block: `input(k)` is route `k`, `target(i)` the route
    /// wanted at frame `i` (only read at frame 0 unless `varying`).
    pub fn render<'a>(
        &mut self,
        out: &mut AudioOut,
        frames: usize,
        input: impl Fn(usize) -> AudioIn<'a>,
        target: impl Fn(usize) -> usize,
        varying: bool,
    ) {
        let n = self.gains.len();
        let clamp = |t: usize| t.min(n - 1);
        if !self.primed {
            self.gains.fill(0.0);
            self.gains[clamp(target(0))] = 1.0;
            self.primed = true;
        }
        let t0 = clamp(target(0));
        if !varying && self.settled_on(t0) {
            let src = input(t0);
            for c in 0..out.channels() {
                let dst = out.channel(c);
                dst.fill(0.0);
                for (o, s) in dst.iter_mut().zip(src.channel(c)) {
                    *o = if s.is_finite() { *s } else { 0.0 };
                }
            }
            return;
        }
        let step = self.step;
        for c in 0..out.channels() {
            let mut srcs: [&[f32]; MAX_ROUTES] = [&[]; MAX_ROUTES];
            for (k, s) in srcs.iter_mut().enumerate().take(n) {
                *s = input(k).channel(c);
            }
            // Every channel replays the same gain trajectory from the
            // block's starting gains.
            self.scratch.copy_from_slice(&self.gains);
            let dst = out.channel(c);
            for (i, o) in dst.iter_mut().enumerate().take(frames) {
                let t = if varying { clamp(target(i)) } else { t0 };
                let mut acc = 0.0f32;
                for (k, g) in self.scratch.iter_mut().enumerate() {
                    if k == t {
                        *g = (*g + step).min(1.0);
                    } else if *g > 0.0 {
                        *g = (*g - step).max(0.0);
                    }
                    if *g > 0.0 {
                        acc += *g * srcs[k].get(i).copied().unwrap_or(0.0);
                    }
                }
                *o = if acc.is_finite() { acc } else { 0.0 };
            }
        }
        if out.channels() == 0 {
            // Nothing to write, but keep time moving.
            self.scratch.copy_from_slice(&self.gains);
            for i in 0..frames {
                let t = if varying { clamp(target(i)) } else { t0 };
                for (k, g) in self.scratch.iter_mut().enumerate() {
                    *g = if k == t {
                        (*g + step).min(1.0)
                    } else {
                        (*g - step).max(0.0)
                    };
                }
            }
        }
        self.gains.copy_from_slice(&self.scratch);
    }
}

/// Notes (and sustain pedals) passed on by a MIDI route and not yet
/// released, so changing the route can release them instead of leaving
/// them hanging.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HeldNotes {
    notes: [u128; 16],
    pedals: u16,
}

const SUSTAIN_PEDAL: u8 = 64;

impl HeldNotes {
    /// Note what `msg` does to the held set.
    pub fn track(&mut self, msg: &MidiMsg) {
        match *msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } => {
                let bit = 1u128 << (note & 0x7f);
                let ch = usize::from(channel & 0x0f);
                if velocity > 0 {
                    self.notes[ch] |= bit;
                } else {
                    self.notes[ch] &= !bit;
                }
            }
            MidiMsg::NoteOff { channel, note, .. } => {
                self.notes[usize::from(channel & 0x0f)] &= !(1u128 << (note & 0x7f));
            }
            MidiMsg::ControlChange {
                channel,
                controller: SUSTAIN_PEDAL,
                value,
            } => {
                let bit = 1u16 << (channel & 0x0f);
                if value >= 64 {
                    self.pedals |= bit;
                } else {
                    self.pedals &= !bit;
                }
            }
            _ => {}
        }
    }

    /// Pass `ev` on and track it.
    pub fn forward(&mut self, out: &mut MidiOut, ev: MidiEvent) {
        self.track(&ev.msg);
        out.push(ev);
    }

    /// Release everything held, at `offset`: pedals up, then note-offs.
    pub fn release_all(&mut self, out: &mut MidiOut, offset: u32) {
        for ch in 0..16u8 {
            if self.pedals & (1 << ch) != 0 {
                out.push(MidiEvent::new(
                    offset,
                    MidiMsg::ControlChange {
                        channel: ch,
                        controller: SUSTAIN_PEDAL,
                        value: 0,
                    },
                ));
            }
            let mut bits = self.notes[usize::from(ch)];
            while bits != 0 {
                let note = bits.trailing_zeros() as u8;
                bits &= bits - 1;
                out.push(MidiEvent::new(
                    offset,
                    MidiMsg::NoteOff {
                        channel: ch,
                        note,
                        velocity: 0,
                    },
                ));
            }
        }
        *self = HeldNotes::default();
    }

    pub fn clear(&mut self) {
        *self = HeldNotes::default();
    }
}
