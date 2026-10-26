use ac_core::{
    Config, ConfigExt, FloatOut, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, node,
};

node! {
    kind: "note_split",
    name: "Note Split",
    category: Sequencing,
    keywords: [
        "drum map", "drum", "pad", "note", "split", "trigger", "gate", "velocity", "choke",
        "midi to trigger"
    ],
    /// Splits MIDI into a trigger, a gate, and a velocity for each of a
    /// list of notes: one node turns a drum pad's or a pattern's notes into
    /// what each sound needs.
    ///
    /// The `notes` config lists the notes, such as `[36, 38, 42, 46]`
    /// (kick, snare, closed and open hat); without it, `count` notes from
    /// `low` (default one note, 36). Up to 64. For the note at place *k* in
    /// the list:
    ///
    /// - `trigger_k` fires on its note-on, on that sample;
    /// - `gate_k` is high from the note-on until its note-off;
    /// - `velocity_k` is how hard it was played, 0 to 1, from the note-on
    ///   (wire it into a `Sampler`'s `velocity`);
    /// - `off_k` fires when the gate ends.
    ///
    /// **Choke groups:** the `chokes` config gives each note a group, such
    /// as `[0, 0, 1, 1]`. A note in a group ends every other note of its
    /// group (their gates close and their `off` fires), so wiring `off_k`
    /// into a Sampler's `stop` lets the closed hat cut the open one. Group 0
    /// chokes nothing. `channel` (config, 1–16; 0, the default, for all)
    /// picks the MIDI channel to listen to.
    ///
    /// **Try this:** wire a `Pads` grid's `midi` into Note Split with
    /// `notes` `[36, 37, 38, 39]`, and each `trigger_k` and `velocity_k`
    /// into the `start` and `velocity` of a Sampler: four pads that play
    /// soft and hard, in two nodes per pad.
    pub struct NoteSplit {
        notes: Vec<u8>,
        chokes: Vec<u32>,
        channel: Option<u8>,
        gates: Vec<bool>,
        velocities: Vec<f32>,
        /// Where the gates and velocities were at the block's start.
        start_gates: Vec<bool>,
        start_velocities: Vec<f32>,
        /// Gate and velocity changes this block: (frame, place, gate, velocity).
        changes: Vec<(u32, u16, bool, f32)>,
    }
}

/// Most notes one node splits.
pub const MAX_NOTES: usize = 64;
/// Most gate and velocity changes kept in a block; more are dropped.
const MAX_CHANGES: usize = 1_024;

/// The notes from config: the `notes` list, else `count` from `low`.
pub fn note_list(config: &Config) -> Vec<u8> {
    let listed: Vec<u8> = config
        .get("notes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_f64)
                .filter(|n| n.is_finite())
                .map(|n| n.round().clamp(0.0, 127.0) as u8)
                .take(MAX_NOTES)
                .collect()
        })
        .unwrap_or_default();
    if !listed.is_empty() {
        return listed;
    }
    let low = config.get_u64_or("low", 36).min(127) as u8;
    let count = config.get_u64_or("count", 1).clamp(1, MAX_NOTES as u64) as usize;
    (0..count)
        .map(|k| low.saturating_add(k as u8).min(127))
        .collect()
}

fn choke_list(config: &Config, n: usize) -> Vec<u32> {
    let mut groups: Vec<u32> = config
        .get("chokes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|g| g.as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32)
                .collect()
        })
        .unwrap_or_default();
    groups.resize(n, 0);
    groups
}

fn trigger_out(k: usize) -> usize {
    4 * k
}
fn gate_out(k: usize) -> usize {
    4 * k + 1
}
fn velocity_out(k: usize) -> usize {
    4 * k + 2
}
fn off_out(k: usize) -> usize {
    4 * k + 3
}

/// Sets frames `from..to` of a float output to `v`.
fn fill(out: &mut FloatOut, from: usize, to: usize, v: f32) {
    match out.as_slice_mut() {
        Some(s) => {
            if let Some(seg) = s.get_mut(from..to.min(s.len())) {
                seg.fill(v);
            }
        }
        None => {
            if from == 0 && to > 0 {
                out.set(0, v);
            }
        }
    }
}

impl NoteSplit {
    fn change(&mut self, at: u32, k: usize) {
        if self.changes.len() < self.changes.capacity() {
            self.changes
                .push((at, k as u16, self.gates[k], self.velocities[k]));
        }
    }
}

impl Node for NoteSplit {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new().input(SocketSpec::midi("midi").doc("Notes to split."));
        for k in 0..note_list(config).len() {
            sig = sig
                .output(
                    SocketSpec::trigger(format!("trigger_{k}"))
                        .doc("Fires on this note's note-on."),
                )
                .output(
                    SocketSpec::boolean(format!("gate_{k}"), false)
                        .doc("High while this note is held."),
                )
                .output(
                    SocketSpec::float(format!("velocity_{k}"), Unit::Generic, 0.0)
                        .doc("How hard this note was played, 0 to 1."),
                )
                .output(
                    SocketSpec::trigger(format!("off_{k}"))
                        .doc("Fires when this note ends or is choked."),
                );
        }
        sig
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let notes = note_list(config);
        let n = notes.len();
        let channel = match config.get_u64_or("channel", 0) {
            0 => None,
            c => Some((c.min(16) - 1) as u8),
        };
        NoteSplit {
            chokes: choke_list(config, n),
            notes,
            channel,
            gates: vec![false; n],
            velocities: vec![0.0; n],
            start_gates: vec![false; n],
            start_velocities: vec![0.0; n],
            changes: Vec::with_capacity(MAX_CHANGES.min(4 * ctx.max_block.max(1) + 64)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let n = self.notes.len();
        self.start_gates.copy_from_slice(&self.gates);
        self.start_velocities.copy_from_slice(&self.velocities);
        self.changes.clear();
        for e in io.inputs.midi(0) {
            let at = e.offset.min(frames.saturating_sub(1) as u32);
            let (channel, note, on, velocity) = match e.msg {
                MidiMsg::NoteOn {
                    channel,
                    note,
                    velocity,
                } => (channel, note, velocity > 0, velocity),
                MidiMsg::NoteOff { channel, note, .. } => (channel, note, false, 0),
                _ => continue,
            };
            if self.channel.is_some_and(|c| c != channel) {
                continue;
            }
            for k in 0..n {
                if self.notes[k] != note {
                    continue;
                }
                if on {
                    let group = self.chokes[k];
                    if group != 0 {
                        for j in 0..n {
                            if j != k && self.chokes[j] == group && self.gates[j] {
                                self.gates[j] = false;
                                self.change(at, j);
                                io.outputs.trigger(off_out(j)).push(at);
                            }
                        }
                    }
                    self.gates[k] = true;
                    self.velocities[k] = f32::from(velocity.min(127)) / 127.0;
                    self.change(at, k);
                    io.outputs.trigger(trigger_out(k)).push(at);
                } else if self.gates[k] {
                    self.gates[k] = false;
                    self.change(at, k);
                    io.outputs.trigger(off_out(k)).push(at);
                }
            }
        }
        // Each gate and velocity from where it was at the block's start,
        // changing where the events changed it.
        for k in 0..n {
            let mut gate = io.outputs.float(gate_out(k));
            let (mut from, mut high) = (0usize, self.start_gates[k]);
            for &(at, _, now, _) in self.changes.iter().filter(|c| usize::from(c.1) == k) {
                fill(&mut gate, from, at as usize, if high { 1.0 } else { 0.0 });
                (from, high) = (at as usize, now);
            }
            fill(&mut gate, from, frames, if high { 1.0 } else { 0.0 });
            let mut vel = io.outputs.float(velocity_out(k));
            let (mut from, mut level) = (0usize, self.start_velocities[k]);
            for &(at, _, _, v) in self.changes.iter().filter(|c| usize::from(c.1) == k) {
                fill(&mut vel, from, at as usize, level);
                (from, level) = (at as usize, v);
            }
            fill(&mut vel, from, frames, level);
        }
    }

    fn reset(&mut self) {
        self.gates.fill(false);
        self.velocities.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(note: u8, velocity: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity,
        }
    }

    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

    #[test]
    fn sockets_follow_the_note_list() {
        let sig = NoteSplit::describe(&json!({ "notes": [36, 38] }));
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(
            outs,
            [
                "trigger_0",
                "gate_0",
                "velocity_0",
                "off_0",
                "trigger_1",
                "gate_1",
                "velocity_1",
                "off_1"
            ]
        );
        assert_eq!(
            note_list(&json!({ "low": 40, "count": 3 })),
            vec![40, 41, 42]
        );
        assert_eq!(note_list(&json!({})), vec![36]);
        assert_eq!(note_list(&json!({ "notes": [300, -5, "x"] })), vec![127, 0]);
        assert_eq!(note_list(&json!({ "count": 999 })).len(), MAX_NOTES);
    }

    #[test]
    fn each_note_gets_a_trigger_a_gate_and_a_velocity_on_its_sample() {
        let mut h = Harness::new::<NoteSplit>(json!({ "notes": [36, 38] }));
        h.midi(
            "midi",
            &[
                (10, on(38, 127)),
                (20, on(36, 64)),
                (40, off(38)),
                (41, on(50, 100)),
            ],
        );
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_1"), &[10]);
        assert_eq!(out.triggers("trigger_0"), &[20]);
        assert_eq!(out.triggers("off_1"), &[40]);
        let g1 = out.float("gate_1");
        assert_eq!((g1[9], g1[10], g1[39], g1[40]), (0.0, 1.0, 1.0, 0.0));
        let v0 = out.float("velocity_0");
        assert_eq!(v0[19], 0.0);
        assert!((v0[20] - 64.0 / 127.0).abs() < 1e-6);
        assert_eq!(
            out.float("velocity_1")[63],
            1.0,
            "velocity holds after the note"
        );
        // The gate stays open into the next block.
        h.midi("midi", &[]);
        assert_eq!(h.run(64).float("gate_0")[0], 1.0);
    }

    #[test]
    fn a_choke_group_ends_the_others_in_it() {
        let mut h =
            Harness::new::<NoteSplit>(json!({ "notes": [42, 46, 36], "chokes": [1, 1, 0] }));
        h.midi(
            "midi",
            &[(0, on(46, 100)), (5, on(36, 100)), (30, on(42, 100))],
        );
        let out = h.run(64);
        assert_eq!(
            out.triggers("off_1"),
            &[30],
            "the closed hat cuts the open one"
        );
        assert!(out.triggers("off_2").is_empty(), "group 0 is never choked");
        assert_eq!(out.float("gate_1")[29], 1.0);
        assert_eq!(out.float("gate_1")[30], 0.0);
        assert_eq!(out.float("gate_2")[63], 1.0);
    }

    #[test]
    fn channel_filters_and_reset_closes_gates() {
        let mut h = Harness::new::<NoteSplit>(json!({ "notes": [36], "channel": 2 }));
        let ch2 = MidiMsg::NoteOn {
            channel: 1,
            note: 36,
            velocity: 90,
        };
        h.midi("midi", &[(3, on(36, 100)), (7, ch2)]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_0"), &[7]);
        h.reset();
        h.midi("midi", &[]);
        assert_eq!(h.run(64).float("gate_0")[0], 0.0);
    }
}
