//! An ADSR envelope state machine with linear or analog-style exponential
//! stages.
//!
//! Exponential stages are one-pole filters heading for a target beyond the
//! stage's end, like a capacitor charging toward a higher voltage, and stop
//! when they cross the end (N. Redmon, "Envelope generators—ADSR code",
//! EarLevel Engineering, 2013). The overshoot ratio sets the curvature: 0.3
//! for the attack (gently bowed, like an analog charge curve) and 0.0001 for
//! decay and release (strongly exponential).
//!
//! Timing, the same for both curves:
//! - **attack** is the time from 0 to full; retriggering from a higher level
//!   gets there sooner, and never jumps.
//! - **decay** is the time from full down to the sustain level.
//! - **release** is the time from wherever the level is down to 0.
//!
//! Every stage lasts at least one sample, so a zero time is a jump that
//! lands on the very next sample.

use crate::util::snap_samples;

/// Stage curve, in the order of the ADSR's `curve` choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Exponential,
}

impl Curve {
    /// Choice names, in index order.
    pub const NAMES: &'static [&'static str] = &["linear", "exponential"];

    pub fn from_index(i: usize) -> Curve {
        if i == 0 {
            Curve::Linear
        } else {
            Curve::Exponential
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Settings for one sample. Times are in samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub attack: f64,
    pub decay: f64,
    /// 0..1.
    pub sustain: f64,
    pub release: f64,
    pub curve: Curve,
}

impl Params {
    /// Params from times in seconds, sanitized: negative or non-finite times
    /// are 0, times are capped at an hour, sustain is clamped to 0..1
    /// (non-finite → 0). Sample counts off a whole number only by `f32`
    /// rounding are snapped to it.
    pub fn from_seconds(
        attack: f64,
        decay: f64,
        sustain: f64,
        release: f64,
        curve: Curve,
        rate: f64,
    ) -> Params {
        let samples = |t: f64| {
            let t = if t.is_finite() {
                t.clamp(0.0, 3600.0)
            } else {
                0.0
            };
            snap_samples(t * rate)
        };
        Params {
            attack: samples(attack),
            decay: samples(decay),
            sustain: if sustain.is_finite() {
                sustain.clamp(0.0, 1.0)
            } else {
                0.0
            },
            release: samples(release),
            curve,
        }
    }
}

const RATIO_ATTACK: f64 = 0.3;
const RATIO_DECAY: f64 = 0.0001;
/// Slack for float error when checking whether a stage reached its end.
const EPS: f64 = 1e-9;

/// One-pole coefficient so a stage overshooting by `ratio` of its range
/// arrives in exactly `samples` steps: `c^n = ratio / (1 + ratio)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coef {
    samples: f64,
    c: f64,
}

impl Coef {
    /// Callers only ask for stages of at least one sample.
    fn get(&mut self, samples: f64, ratio: f64) -> f64 {
        if samples != self.samples {
            self.samples = samples;
            self.c = if samples >= 1.0 {
                ((ratio / (1.0 + ratio)).ln() / samples).exp()
            } else {
                0.0
            };
        }
        self.c
    }
}

/// The envelope. No allocation; `tick` once per sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    stage: Stage,
    level: f64,
    /// Level when the release began.
    release_from: f64,
    attack: Coef,
    decay: Coef,
    release: Coef,
}

impl Default for Adsr {
    fn default() -> Self {
        Adsr {
            stage: Stage::Idle,
            level: 0.0,
            release_from: 0.0,
            attack: Coef::default(),
            decay: Coef::default(),
            release: Coef::default(),
        }
    }
}

impl Adsr {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    /// Anything but idle.
    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// Start (or restart) the attack from the current level.
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Start the release from the current level (if not idle).
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle && self.stage != Stage::Release {
            self.stage = Stage::Release;
            self.release_from = self.level;
        }
    }

    /// Back to idle at 0.
    pub fn reset(&mut self) {
        *self = Adsr::default();
    }

    /// Advance one sample. Returns the level and whether the release
    /// finished on this sample.
    #[inline]
    pub fn tick(&mut self, p: &Params) -> (f64, bool) {
        let linear = p.curve == Curve::Linear;
        let mut ended = false;
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Sustain => self.level = p.sustain,
            Stage::Attack => {
                self.level = if p.attack < 1.0 {
                    1.0
                } else if linear {
                    self.level + 1.0 / p.attack
                } else {
                    let target = 1.0 + RATIO_ATTACK;
                    let c = self.attack.get(p.attack, RATIO_ATTACK);
                    target + (self.level - target) * c
                };
                if self.level >= 1.0 - EPS {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let s = p.sustain;
                self.level = if p.decay < 1.0 {
                    s
                } else if linear {
                    self.level - (1.0 - s) / p.decay
                } else {
                    let target = s - RATIO_DECAY * (1.0 - s);
                    let c = self.decay.get(p.decay, RATIO_DECAY);
                    target + (self.level - target) * c
                };
                if self.level <= s + EPS {
                    self.level = s;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Release => {
                let from = self.release_from;
                self.level = if p.release < 1.0 {
                    0.0
                } else if linear {
                    self.level - from / p.release
                } else {
                    let target = -RATIO_DECAY * from;
                    let c = self.release.get(p.release, RATIO_DECAY);
                    target + (self.level - target) * c
                };
                if self.level <= EPS {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                    ended = true;
                }
            }
        }
        if !self.level.is_finite() {
            self.reset();
        }
        (self.level, ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(a: f64, d: f64, s: f64, r: f64, curve: Curve) -> Params {
        Params {
            attack: a,
            decay: d,
            sustain: s,
            release: r,
            curve,
        }
    }

    /// Ticks until the stage changes; returns how many ticks that took.
    fn ticks_in_stage(env: &mut Adsr, p: &Params) -> usize {
        let stage = env.stage();
        let mut n = 0;
        while env.stage() == stage && n < 1_000_000 {
            env.tick(p);
            n += 1;
        }
        n
    }

    #[test]
    fn stages_last_their_times() {
        for curve in [Curve::Linear, Curve::Exponential] {
            let p = params(100.0, 200.0, 0.5, 300.0, curve);
            let mut env = Adsr::new();
            env.gate_on();
            assert_eq!(ticks_in_stage(&mut env, &p), 100, "{curve:?} attack");
            assert_eq!(env.level(), 1.0);
            let d = ticks_in_stage(&mut env, &p);
            assert!((199..=201).contains(&d), "{curve:?} decay {d}");
            assert_eq!(env.tick(&p).0, 0.5);
            env.gate_off();
            let r = ticks_in_stage(&mut env, &p);
            assert!((299..=301).contains(&r), "{curve:?} release {r}");
            assert_eq!(env.level(), 0.0);
        }
    }

    #[test]
    fn exponential_is_bowed_and_linear_is_straight() {
        let lin = params(100.0, 100.0, 0.0, 100.0, Curve::Linear);
        let exp = params(100.0, 100.0, 0.0, 100.0, Curve::Exponential);
        let halfway = |p: &Params| {
            let mut env = Adsr::new();
            env.gate_on();
            (0..50).map(|_| env.tick(p).0).last().unwrap_or(0.0)
        };
        assert!((halfway(&lin) - 0.5).abs() < 1e-9);
        // An RC charge toward 1.3 is past halfway at half the time.
        assert!(halfway(&exp) > 0.6);
    }

    #[test]
    fn release_takes_its_time_from_any_level() {
        for curve in [Curve::Linear, Curve::Exponential] {
            let p = params(1000.0, 10.0, 1.0, 400.0, curve);
            let mut env = Adsr::new();
            env.gate_on();
            for _ in 0..300 {
                env.tick(&p);
            }
            env.gate_off();
            let r = ticks_in_stage(&mut env, &p);
            assert!((399..=401).contains(&r), "{curve:?} {r}");
        }
    }

    #[test]
    fn zero_times_jump_on_the_next_sample() {
        let p = params(0.0, 0.0, 0.25, 0.0, Curve::Exponential);
        let mut env = Adsr::new();
        env.gate_on();
        assert_eq!(env.tick(&p), (1.0, false));
        assert_eq!(env.tick(&p), (0.25, false));
        env.gate_off();
        assert_eq!(env.tick(&p), (0.0, true));
        assert!(!env.is_active());
        assert_eq!(env.tick(&p), (0.0, false));
    }

    #[test]
    fn retrigger_continues_from_the_current_level() {
        for curve in [Curve::Linear, Curve::Exponential] {
            let p = params(100.0, 100.0, 0.5, 1000.0, curve);
            let mut env = Adsr::new();
            env.gate_on();
            for _ in 0..400 {
                env.tick(&p);
            }
            env.gate_off();
            for _ in 0..100 {
                env.tick(&p);
            }
            let before = env.level();
            env.gate_on();
            let after = env.tick(&p).0;
            assert!(after > before && after - before < 0.05, "{curve:?}");
        }
    }

    #[test]
    fn release_while_idle_does_nothing() {
        let p = params(10.0, 10.0, 0.5, 10.0, Curve::Linear);
        let mut env = Adsr::new();
        env.gate_off();
        assert_eq!(env.tick(&p), (0.0, false));
        assert_eq!(env.stage(), Stage::Idle);
    }

    #[test]
    fn params_are_sanitized() {
        let p = Params::from_seconds(f64::NAN, -1.0, 7.0, f64::INFINITY, Curve::Linear, 48_000.0);
        assert_eq!(
            (p.attack, p.decay, p.sustain, p.release),
            (0.0, 0.0, 1.0, 0.0)
        );
        let p = Params::from_seconds(0.5, 1e9, f64::NAN, 0.25, Curve::Linear, 48_000.0);
        assert_eq!(
            (p.attack, p.decay, p.sustain),
            (24_000.0, 3600.0 * 48_000.0, 0.0)
        );
    }
}
