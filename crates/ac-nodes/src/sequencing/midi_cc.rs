use ac_core::{
    Config, ConfigExt, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature,
    SocketSpec, Unit, node,
};

use crate::io::midi_in::channel_matches;

node! {
    kind: "midi_cc",
    name: "MIDI CC",
    category: Sequencing,
    keywords: [
        "controller", "cc", "control change", "knob", "fader", "mod wheel", "pitch bend",
        "aftertouch", "pressure", "expression"
    ],
    /// Turns one MIDI controller into a `Float` in a range you choose.
    ///
    /// `controller` picks the controller number, 0–127 (1 is the mod wheel,
    /// 7 volume, 11 expression, 64 sustain, 74 brightness). Two numbers past
    /// the controllers follow other continuous messages: 128 is pitch bend
    /// (centered halfway between `min` and `max`) and 129 is channel
    /// pressure (aftertouch). `channel` 0 listens to every MIDI channel,
    /// 1–16 to one.
    ///
    /// `out` maps the controller's value from 0–127 onto `min`–`max` (set
    /// `min` above `max` to turn it upside down) and holds it until the
    /// next message. A new value takes effect on the exact sample its
    /// message arrives. With `smooth` above 0 the output glides there in a
    /// straight line instead, arriving `smooth` seconds later, which hides
    /// the steps of a 7-bit controller; each new message starts a new
    /// glide from wherever the output is. `changed` fires on each message
    /// that changes the value.
    ///
    /// Before its first message a controller sits at the `initial` config,
    /// a fraction from 0 (`min`, the default) to 1 (`max`); pitch bend
    /// always starts centered. The node remembers the last value of every
    /// controller, so switching `controller` moves to that controller's
    /// value (or `initial` if it hasn't moved yet), gliding if `smooth` is
    /// set. Controller values are 7-bit and pitch bend 14-bit; 14-bit
    /// controller pairs aren't combined.
    ///
    /// **Try this:** `MIDI In` → MIDI CC with `controller` 1, `min` 200 and
    /// `max` 8000, into a `Biquad`'s `cutoff`: the mod wheel sweeps the
    /// filter. Add `smooth` 0.02 to take the zipper out of slow sweeps, or
    /// wire `changed` into a `Clip`'s `start` to fire a sample from a pad.
    pub struct MidiCc {
        /// Last value of each controller (0..=127), pitch bend (128), and
        /// channel pressure (129), as 0..1.
        values: [f32; CONTROLS],
        initial: f32,
        /// The controller followed last block; `None` before the first.
        controller: Option<usize>,
        /// Output position, 0..1, and the glide toward `target`.
        current: f64,
        target: f64,
        step: f64,
        left: u64,
        sample_rate: f64,
        /// `changed` offsets this block, preallocated to `max_block`.
        changes: Vec<u32>,
    }
}

const MIDI: usize = 0;
const CONTROLLER: usize = 1;
const CHANNEL: usize = 2;
const MIN: usize = 3;
const MAX: usize = 4;
const SMOOTH: usize = 5;

const OUT: usize = 0;
const CHANGED: usize = 1;

/// Controller numbers 0..=127, then pitch bend and channel pressure.
const CONTROLS: usize = 130;
const PITCH_BEND: usize = 128;
const PRESSURE: usize = 129;

/// Longest glide, beyond the input's soft range, so sample counts stay sane.
const MAX_SMOOTH_SECONDS: f32 = 60.0;

/// Pitch bend as 0..1 with the center at exactly 0.5 and both ends reached.
fn bend_position(value: i16) -> f32 {
    let v = f32::from(value);
    let half = if v >= 0.0 { v / 8191.0 } else { v / 8192.0 };
    (0.5 + 0.5 * half).clamp(0.0, 1.0)
}

/// The control a message moves and its new position, 0..1.
fn control_of(msg: &MidiMsg) -> Option<(usize, f32)> {
    match *msg {
        MidiMsg::ControlChange {
            controller, value, ..
        } => Some((
            usize::from(controller & 127),
            f32::from(value.min(127)) / 127.0,
        )),
        MidiMsg::PitchBend { value, .. } => Some((PITCH_BEND, bend_position(value))),
        MidiMsg::ChannelPressure { value, .. } => {
            Some((PRESSURE, f32::from(value.min(127)) / 127.0))
        }
        _ => None,
    }
}

fn initial_values(initial: f32) -> [f32; CONTROLS] {
    let mut v = [initial; CONTROLS];
    v[PITCH_BEND] = 0.5;
    v
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl MidiCc {
    /// Heads for `target`, gliding over `seconds` (0 jumps).
    fn retarget(&mut self, target: f32, seconds: f32) {
        self.target = f64::from(target);
        let secs = finite_or(seconds, 0.0).clamp(0.0, MAX_SMOOTH_SECONDS);
        let samples = (f64::from(secs) * self.sample_rate).round() as u64;
        if samples == 0 {
            self.current = self.target;
            self.left = 0;
        } else {
            self.step = (self.target - self.current) / samples as f64;
            self.left = samples;
        }
    }

    fn advance(&mut self) {
        if self.left > 0 {
            self.left -= 1;
            self.current = if self.left == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
    }
}

impl Node for MidiCc {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Messages to read the controller from."))
            .input(
                SocketSpec::int("controller", 1)
                    .range(0.0, 129.0)
                    .doc("Controller number 0–127; 128 pitch bend, 129 channel pressure."),
            )
            .input(
                SocketSpec::int("channel", 0)
                    .range(0.0, 16.0)
                    .doc("MIDI channel, 1–16; 0 listens to all."),
            )
            .input(
                SocketSpec::float("min", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Output at controller value 0."),
            )
            .input(
                SocketSpec::float("max", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("Output at controller value 127."),
            )
            .input(
                SocketSpec::float("smooth", Unit::Seconds, 0.0)
                    .range(0.0, 1.0)
                    .doc("Glide time to each new value."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The controller's value mapped onto min–max."),
            )
            .output(SocketSpec::trigger("changed").doc("Fires when the value changes."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let initial = config.get_f64_or("initial", 0.0);
        let initial = if initial.is_finite() {
            initial.clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        MidiCc {
            values: initial_values(initial),
            initial,
            controller: None,
            current: f64::from(initial),
            target: f64::from(initial),
            step: 0.0,
            left: 0,
            sample_rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            changes: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        let controller = io.inputs.int(CONTROLLER).clamp(0, CONTROLS as i32 - 1) as usize;
        let filter = io.inputs.int(CHANNEL);
        let min = io.inputs.float(MIN);
        let max = io.inputs.float(MAX);
        let smooth = io.inputs.float(SMOOTH);
        self.changes.clear();

        match self.controller {
            None => {
                self.current = f64::from(self.values[controller]);
                self.target = self.current;
                self.left = 0;
            }
            Some(c) if c != controller => self.retarget(self.values[controller], smooth.first()),
            Some(_) => {}
        }
        self.controller = Some(controller);

        let last = ctx.frames.saturating_sub(1);
        let mut events = events
            .iter()
            .filter(|e| channel_matches(filter, e))
            .peekable();
        let mut out = io.outputs.float(OUT);
        for i in 0..ctx.frames {
            // Late events are applied on the last frame rather than dropped.
            while let Some(ev) = events.next_if(|e| (e.offset as usize).min(last) <= i) {
                let Some((c, v)) = control_of(&ev.msg) else {
                    continue;
                };
                let old = self.values[c];
                self.values[c] = v;
                if c == controller && v != old {
                    self.retarget(v, smooth.at(i));
                    if self.changes.last() != Some(&(i as u32))
                        && self.changes.len() < self.changes.capacity()
                    {
                        self.changes.push(i as u32);
                    }
                }
            }
            let lo = finite_or(min.at(i), 0.0);
            let hi = finite_or(max.at(i), 1.0);
            let x = self.current as f32;
            // Weighted rather than `lo + (hi - lo) * x`, which can overflow.
            out.set(i, lo * (1.0 - x) + hi * x);
            self.advance();
        }

        let mut changed = io.outputs.trigger(CHANGED);
        for &t in &self.changes {
            changed.push(t);
        }
    }

    fn reset(&mut self) {
        self.values = initial_values(self.initial);
        self.controller = None;
        self.current = f64::from(self.initial);
        self.target = self.current;
        self.step = 0.0;
        self.left = 0;
        self.changes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn cc(channel: u8, controller: u8, value: u8) -> MidiMsg {
        MidiMsg::ControlChange {
            channel,
            controller,
            value,
        }
    }

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<MidiCc>(config, 48_000.0, block, &NoResources)
    }

    #[test]
    fn starts_at_min_and_jumps_on_the_event_sample() {
        let mut h = harness(json!({}), 64);
        h.set("min", 100.0).set("max", 200.0);
        h.midi("midi", &[(10, cc(0, 1, 127)), (70, cc(0, 1, 0))]);
        let out = h.run(128);
        let o = out.float("out");
        assert!(o[..10].iter().all(|&v| v == 100.0));
        assert!(o[10..70].iter().all(|&v| v == 200.0));
        assert!(o[70..].iter().all(|&v| v == 100.0));
        assert_eq!(out.triggers("changed"), &[10, 70]);
    }

    #[test]
    fn only_the_chosen_controller_and_channel_count() {
        let mut h = harness(json!({}), 64);
        h.set("controller", 74).set("channel", 3);
        h.midi(
            "midi",
            &[
                (5, cc(2, 1, 127)),  // wrong controller
                (10, cc(0, 74, 64)), // wrong channel
                (20, cc(2, 74, 127)),
            ],
        );
        let out = h.run(64);
        let o = out.float("out");
        assert_eq!(o[15], 0.0);
        assert_eq!(o[20], 1.0);
        assert_eq!(out.triggers("changed"), &[20]);
    }

    #[test]
    fn repeated_values_do_not_fire_changed() {
        let mut h = harness(json!({}), 64);
        h.midi(
            "midi",
            &[(1, cc(0, 1, 50)), (2, cc(0, 1, 50)), (3, cc(0, 1, 51))],
        );
        assert_eq!(h.run(64).triggers("changed"), &[1, 3]);
    }

    #[test]
    fn smoothing_glides_in_a_straight_line_and_arrives_on_time() {
        let mut h = harness(json!({}), 37);
        h.set("smooth", 0.01); // 480 samples
        h.midi("midi", &[(100, cc(0, 1, 127))]);
        let o = h.run(1_000).float("out").to_vec();
        assert_eq!(o[100], 0.0);
        assert!((o[340] - 0.5).abs() < 1e-4);
        assert!(o[100..580].windows(2).all(|w| w[1] > w[0]));
        assert_eq!(o[580], 1.0);
        assert_eq!(o[999], 1.0);
    }

    #[test]
    fn pitch_bend_and_pressure() {
        let mut h = harness(json!({}), 64);
        h.set("controller", 128).set("min", -2.0).set("max", 2.0);
        h.midi(
            "midi",
            &[
                (
                    10,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: 8191,
                    },
                ),
                (
                    20,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: -8192,
                    },
                ),
            ],
        );
        let o = h.run(64).float("out").to_vec();
        assert_eq!(o[0], 0.0, "bend starts centered");
        assert_eq!(o[10], 2.0);
        assert_eq!(o[20], -2.0);

        let mut h = harness(json!({}), 64);
        h.set("controller", 129);
        h.midi(
            "midi",
            &[(
                5,
                MidiMsg::ChannelPressure {
                    channel: 9,
                    value: 127,
                },
            )],
        );
        assert_eq!(h.run(64).float("out")[5], 1.0);
    }

    #[test]
    fn initial_config_and_controller_memory() {
        let mut h = harness(json!({ "initial": 0.5 }), 64);
        h.set("max", 2.0);
        assert_eq!(h.run(64).float("out")[0], 1.0);

        // Controller 7 moves while controller 1 is followed; switching to 7
        // picks up its value.
        let mut h = harness(json!({}), 64);
        h.midi("midi", &[(3, cc(0, 7, 127))]);
        assert_eq!(h.run(64).float("out")[63], 0.0);
        h.set("controller", 7);
        assert_eq!(h.run(64).float("out")[0], 1.0);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let events = [
            (3, cc(0, 1, 20)),
            (300, cc(0, 1, 120)),
            (301, cc(0, 1, 60)),
            (900, cc(0, 1, 127)),
        ];
        let render = |block| {
            let mut h = harness(json!({}), block);
            h.set("smooth", 0.004).set("min", -1.0);
            h.midi("midi", &events);
            let out = h.run(2_048);
            (out.float("out").to_vec(), out.triggers("changed").to_vec())
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut h = harness(json!({}), 64);
        h.midi("midi", &[(0, cc(0, 1, 127))]);
        assert_eq!(h.run(64).float("out")[63], 1.0);
        h.reset();
        h.midi("midi", &[]);
        assert!(h.run(64).float("out").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn bad_inputs_stay_finite() {
        let mut h = harness(json!({ "initial": "loud" }), 64);
        h.set("min", f64::NAN)
            .set("max", 3.0e38)
            .set("smooth", 1.0e12)
            .set("controller", 999);
        h.midi(
            "midi",
            &[(
                0,
                MidiMsg::ChannelPressure {
                    channel: 0,
                    value: 99,
                },
            )],
        );
        assert!(h.run(256).float("out").iter().all(|v| v.is_finite()));
    }
}
