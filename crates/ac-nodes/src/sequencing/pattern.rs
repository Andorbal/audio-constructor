use ac_core::{
    Config, ConfigExt, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    SongLock, Unit, Value, node,
};

use crate::segments::Runs;
use crate::song;

node! {
    kind: "pattern",
    name: "Pattern",
    category: Sequencing,
    keywords: [
        "step sequencer", "drum machine", "grid", "beat", "rhythm", "steps", "sequencer",
        "drums", "trigger", "gate"
    ],
    /// A grid of step buttons that plays one step per clock tick, like a drum machine's pattern.
    ///
    /// The grid has `steps` columns (config, 1–64, default 16) and `rows`
    /// rows (config, 1–16, default 4); click cells in the node to switch
    /// them on, and drag up or down on one to set its level. The `cells`
    /// config holds the grid as one string per row, `x` for an on step at
    /// full level, `1` to `9` for an on step at that many tenths, and `.`
    /// for an off one, such as `["x...x...x...x...", "....x.......x..."]`.
    /// Missing rows and steps are off, and any other character counts as
    /// off.
    ///
    /// **Banks:** a Pattern can hold several grids of the same size (up to
    /// 16), added as tabs in the editor; `cells` is then a list of grids,
    /// one list of strings each. The `bank` input picks which plays,
    /// counting from 0. A new bank starts at the beginning of the next
    /// pass, so a scene change lands on the downbeat; while stopped it
    /// applies at once.
    ///
    /// Each `clock` trigger moves to the next step. Where that step is on
    /// in row *n*, `trigger_n` fires on the clock's sample, `gate_n` goes
    /// high until the next clock, and `velocity_n` holds the step's level
    /// (0 to 1) until the row's next on step (rows count from 0, the top
    /// row). On steps next to each other keep the gate high from one to the
    /// next, so use the trigger to restart an envelope on every step. `step` is
    /// the last step played, counting from 0. Nothing moves without a
    /// clock: wire a `Clock` into `clock` (`interval` 0.25 for sixteenth
    /// notes).
    ///
    /// **Placed in the song:** set `start` to *At beat…* and the pattern
    /// starts at song beat `at` (and stops at `until`, unless that's 0), with `interval` beats per step (0.25,
    /// sixteenths, by default: set it to the clock's interval). Each clock
    /// then plays the step the song is on, counted from `at`, so after a
    /// seek, a loop, or play from the middle the pattern is on the right
    /// step at once; clocks before its beat play nothing.
    ///
    /// Generator contract: `start` (on transport play by default) goes
    /// back to the beginning and plays, so the next clock, or one on the
    /// same sample, plays the first step (the gates close until then).
    /// `stop` closes the gates, ignores the clock, and fires `end`; `reset`
    /// goes back to the beginning without stopping. After the last step the
    /// pattern wraps around while `loop` is on (the default). With `loop`
    /// off, the clock after the last step stops it and fires `end` instead,
    /// so the last step gets its full length and a second Pattern started
    /// by `end` plays its first step on that same clock. `reverse` flips
    /// the direction: the pattern steps backwards, and its beginning
    /// becomes the last step. Events on the same sample apply in the order
    /// reverse, stop, start, reset, clock.
    ///
    /// **Try this:** wire a `Clock` (`interval` 0.25) into `clock`, then
    /// `trigger_0` into the `on` of an `ADSR` (sustain 0) on a low
    /// `Oscillator` for a kick, and `trigger_1` into another on a `Noise`
    /// for a snare. Fill row 0 with `x...x...x...x...` and row 1 with
    /// `....x.......x...`. Paint the hats at level 5 with an `x` on every
    /// fourth step, and wire `velocity_2` into their amplifier, for accents.
    pub struct Pattern {
        steps: usize,
        rows: usize,
        banks: usize,
        /// Each step's level, 0 (off) to 10, by bank, then row, then step.
        levels: Vec<u8>,
        /// The bank playing.
        bank: usize,
        /// Each row's `velocity` output: the level of its last on step.
        velocity: [f32; MAX_ROWS],
        playing: bool,
        /// The step playing; `None` at the beginning, before the first clock.
        current: Option<usize>,
        forward: bool,
        /// Rows whose gate is high, one bit each.
        gates: u16,
        /// The `step` output.
        shown: usize,
        /// Gate changes this block: (frame, rows high from there).
        changes: Vec<(u32, u16)>,
        /// Steps played this block: (frame, rows whose trigger fires, bank,
        /// step).
        fires: Vec<(u32, u16, u8, u8)>,
        ends: Vec<u32>,
    }
}

const CLOCK: usize = 0;
const START: usize = 1;
const AT: usize = 2;
const INTERVAL: usize = 4;
const STOP: usize = 5;
const RESET: usize = 6;
const LOOP: usize = 7;
const REVERSE: usize = 8;
const BANK: usize = 9;

/// Shortest step a placed pattern counts, in beats.
const MIN_INTERVAL: f64 = 1.0 / 64.0;

pub const MAX_STEPS: u64 = 64;
pub const MAX_ROWS: usize = 16;
pub const MAX_BANKS: usize = 16;
/// Telemetry, for the grid's playhead: the step playing (−1 before the
/// first clock), the bank playing, and whether it plays (1 or 0).
pub const TELEMETRY_STEP: usize = 0;
pub const TELEMETRY_BANK: usize = 1;
pub const TELEMETRY_PLAYING: usize = 2;
/// A full-level step.
pub const FULL: u8 = 10;

/// Steps per row from config, clamped (DECISIONS D29).
pub fn step_count(config: &Config) -> usize {
    config.get_u64_or("steps", 16).clamp(1, MAX_STEPS) as usize
}

/// Rows from config, clamped.
pub fn row_count(config: &Config) -> usize {
    config.get_u64_or("rows", 4).clamp(1, MAX_ROWS as u64) as usize
}

/// A cell's level from its character: `x` full, `1`–`9` tenths, else off.
pub fn level_of(c: char) -> u8 {
    match c {
        'x' | 'X' => FULL,
        '1'..='9' => c as u8 - b'0',
        _ => 0,
    }
}

/// The character for a level.
pub fn char_of(level: u8) -> char {
    match level {
        0 => '.',
        1..=9 => (b'0' + level) as char,
        _ => 'x',
    }
}

/// The grids in `cells`: banks, each `rows` rows of `steps` levels. A list
/// of strings is one bank; a list of lists is several. At least one bank.
pub fn grids(config: &Config) -> Vec<Vec<Vec<u8>>> {
    let steps = step_count(config);
    let rows = row_count(config);
    let list = config.get("cells").and_then(|v| v.as_array());
    let nested = list
        .and_then(|l| l.first())
        .is_some_and(serde_json::Value::is_array);
    let banks: Vec<Option<&Vec<serde_json::Value>>> = match list {
        Some(l) if nested => l.iter().take(MAX_BANKS).map(|b| b.as_array()).collect(),
        Some(l) => vec![Some(l)],
        None => vec![None],
    };
    banks
        .into_iter()
        .map(|bank| {
            (0..rows)
                .map(|r| {
                    let text = bank
                        .and_then(|b| b.get(r))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let mut chars = text.chars();
                    (0..steps)
                        .map(|_| chars.next().map_or(0, level_of))
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// The `cells` value for `grids`: one list of strings for one bank, a list
/// of them for several.
pub fn cells_value(grids: &[Vec<Vec<u8>>]) -> serde_json::Value {
    let bank = |g: &Vec<Vec<u8>>| {
        serde_json::Value::Array(
            g.iter()
                .map(|row| serde_json::Value::String(row.iter().map(|&l| char_of(l)).collect()))
                .collect(),
        )
    };
    match grids {
        [one] => bank(one),
        _ => serde_json::Value::Array(grids.iter().map(bank).collect()),
    }
}

fn trigger_out(row: usize) -> usize {
    3 * row
}

fn gate_out(row: usize) -> usize {
    3 * row + 1
}

fn velocity_out(row: usize) -> usize {
    3 * row + 2
}

/// Sets frames `from..to` of a gate output.
fn fill(out: &mut FloatOut, from: usize, to: usize, high: bool) {
    fill_with(out, from, to, if high { 1.0 } else { 0.0 });
}

/// Sets frames `from..to` of a float output to `v`.
fn fill_with(out: &mut FloatOut, from: usize, to: usize, v: f32) {
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

impl Pattern {
    fn level(&self, bank: usize, row: usize, step: usize) -> u8 {
        let i = (bank * self.rows + row) * self.steps + step;
        self.levels.get(i).copied().unwrap_or(0)
    }

    /// Rows that are on at step `s` of the bank playing.
    fn column(&self, s: usize) -> u16 {
        let mut m = 0u16;
        for r in 0..self.rows {
            if self.level(self.bank, r, s) > 0 {
                m |= 1 << r;
            }
        }
        m
    }

    /// Plays step `s`: its gates, and its triggers if any row is on.
    fn play(&mut self, at: u32, s: usize) {
        self.current = Some(s);
        self.shown = s;
        self.gates = self.column(s);
        if self.gates != 0 && self.fires.len() < self.fires.capacity() {
            self.fires.push((at, self.gates, self.bank as u8, s as u8));
        }
    }

    fn first_step(&self) -> usize {
        if self.forward { 0 } else { self.steps - 1 }
    }

    /// One clock: the next step, or the end when not looping. A pass
    /// begins with the bank asked for.
    fn advance(&mut self, at: u32, looping: bool, bank: usize) {
        let last = self.steps - 1;
        let (next, begins) = match self.current {
            None => (Some(self.first_step()), true),
            Some(c) if self.forward && c < last => (Some(c + 1), false),
            Some(c) if !self.forward && c > 0 => (Some(c - 1), false),
            Some(_) if looping => (Some(self.first_step()), true),
            Some(_) => (None, false),
        };
        if begins {
            self.bank = bank;
        }
        match next {
            Some(s) => self.play(at, s),
            None => {
                self.playing = false;
                self.gates = 0;
                self.push_end(at);
            }
        }
    }

    /// One clock while placed in the song: the step the song is on, `k`
    /// steps past the pattern's beat; before it, nothing; past the last
    /// step unlooped, the end.
    fn song_step(&mut self, at: u32, k: i64, looping: bool, bank: usize) {
        if k < 0 {
            self.gates = 0;
            return;
        }
        let k = k as u64;
        if k >= self.steps as u64 && !looping {
            self.playing = false;
            self.gates = 0;
            self.push_end(at);
            return;
        }
        let i = (k % self.steps as u64) as usize;
        if i == 0 || self.current.is_none() {
            self.bank = bank;
        }
        let s = if self.forward { i } else { self.steps - 1 - i };
        self.play(at, s);
    }

    fn push_end(&mut self, at: u32) {
        if self.ends.len() < self.ends.capacity() {
            self.ends.push(at);
        }
    }
}

impl Node for Pattern {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(3)
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::trigger("clock").doc("Moves to the next step."))
            .song_generator_inputs(true, false)
            .input(SocketSpec::trigger("reverse").doc("Toggles the direction."))
            .input(
                SocketSpec::int("bank", 0)
                    .range(0.0, (MAX_BANKS - 1) as f64)
                    .doc("Which grid plays, from 0; a new one starts with the next pass."),
            );
        sig.inputs.insert(
            INTERVAL,
            SocketSpec::float("interval", Unit::Beats, 0.25)
                .range(0.0, 4.0)
                .song()
                .doc(
                    "Beats per step, placed in the song: each clock plays the step the song is on.",
                ),
        );
        if let Some(l) = sig.inputs.iter_mut().find(|s| s.name == "loop") {
            l.default = Value::Bool(true);
        }
        for r in 0..row_count(config) {
            sig = sig
                .output(
                    SocketSpec::trigger(format!("trigger_{r}"))
                        .doc("Fires when a clock lands on an on step in this row."),
                )
                .output(
                    SocketSpec::boolean(format!("gate_{r}"), false)
                        .doc("High from an on step until the next clock."),
                )
                .output(
                    SocketSpec::float(format!("velocity_{r}"), Unit::Generic, 0.0)
                        .range(0.0, 1.0)
                        .doc("The level of this row's last on step, 0 to 1."),
                );
        }
        sig.output(SocketSpec::int("step", 0).doc("The last step played, from 0."))
            .generator_outputs()
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let steps = step_count(config);
        let rows = row_count(config);
        let grids = grids(config);
        let cap = ctx.max_block.max(1) + 1;
        Pattern {
            steps,
            rows,
            banks: grids.len(),
            levels: grids.into_iter().flatten().flatten().collect(),
            bank: 0,
            velocity: [0.0; MAX_ROWS],
            playing: false,
            current: None,
            forward: true,
            gates: 0,
            shown: 0,
            changes: Vec::with_capacity(cap),
            fires: Vec::with_capacity(cap),
            ends: Vec::with_capacity(cap),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        let looping = io.inputs.boolean(LOOP);
        let bank = (io.inputs.int(BANK).max(0) as usize).min(self.banks - 1);
        if !self.playing || self.current.is_none() {
            self.bank = bank;
        }
        let transport = ctx.transport;
        // Placed in the song: the beat it starts on and the beats per step.
        let song = io.inputs.song_locked(START).then(|| {
            let at = f64::from(io.inputs.float(AT).first());
            let interval = f64::from(io.inputs.float(INTERVAL).first());
            (
                if at.is_finite() { at.max(0.0) } else { 0.0 },
                if interval.is_finite() {
                    interval.max(MIN_INTERVAL)
                } else {
                    0.25
                },
            )
        });
        let lock = song::lock(ctx, io, START);
        let mut at = [0; 2];
        let (start, stop) = song::triggers(
            lock,
            io.inputs.triggers(START),
            io.inputs.triggers(STOP),
            &mut at,
        );
        let events = [
            io.inputs.triggers(REVERSE),
            stop,
            start,
            io.inputs.triggers(RESET),
            io.inputs.triggers(CLOCK),
        ];
        self.changes.clear();
        self.fires.clear();
        self.ends.clear();
        match lock {
            SongLock::Wait => {
                self.playing = false;
                self.gates = 0;
            }
            SongLock::Land { .. } => {
                // The next clock plays the step the song is on.
                self.playing = true;
                self.current = None;
                self.gates = 0;
            }
            _ => {}
        }
        let initial = self.gates;

        for run in Runs::new(events, frames) {
            let [reverse, stop, start, reset, clock] = run.fired;
            let at = run.at as u32;
            let before = self.gates;
            if reverse {
                self.forward = !self.forward;
            }
            if stop && self.playing {
                self.playing = false;
                self.gates = 0;
                self.push_end(at);
            }
            if start {
                self.playing = true;
                self.current = None;
                self.gates = 0;
            }
            if reset {
                self.current = None;
            }
            if clock && self.playing {
                match song {
                    Some((beat, interval)) => {
                        let now = transport.tempo.beat_at(transport.position + run.at as u64);
                        // A hair of slack so a tick on the grid counts as on it.
                        let k = ((now - beat) / interval + 1e-6).floor();
                        self.song_step(at, k as i64, looping.at(run.at), bank);
                    }
                    None => self.advance(at, looping.at(run.at), bank),
                }
            }
            if self.gates != before && self.changes.len() < self.changes.capacity() {
                self.changes.push((at, self.gates));
            }
        }

        for r in 0..self.rows {
            let mut trig = io.outputs.trigger(trigger_out(r));
            for &(at, rows, ..) in &self.fires {
                if rows >> r & 1 == 1 {
                    trig.push(at);
                }
            }
            let mut vel = io.outputs.float(velocity_out(r));
            let mut held = self.velocity[r];
            let mut from = 0;
            for &(at, rows, bank, step) in &self.fires {
                if rows >> r & 1 == 1 {
                    fill_with(&mut vel, from, at as usize, held);
                    held = f32::from(self.level(bank as usize, r, step as usize)) / f32::from(FULL);
                    from = at as usize;
                }
            }
            fill_with(&mut vel, from, frames, held);
            self.velocity[r] = held;
            let mut gate = io.outputs.float(gate_out(r));
            let mut high = initial >> r & 1 == 1;
            let mut from = 0;
            for &(at, rows) in &self.changes {
                fill(&mut gate, from, at as usize, high);
                high = rows >> r & 1 == 1;
                from = at as usize;
            }
            fill(&mut gate, from, frames, high);
        }
        let base = 3 * self.rows;
        let step = match (self.playing, self.current) {
            (true, Some(s)) => s as f32,
            _ => -1.0,
        };
        io.telemetry.set(TELEMETRY_STEP, step);
        io.telemetry.set(TELEMETRY_BANK, self.bank as f32);
        io.telemetry
            .set(TELEMETRY_PLAYING, if self.playing { 1.0 } else { 0.0 });
        io.outputs.set_int(base, self.shown as i32);
        io.outputs.set_bool(base + 1, self.playing);
        let mut end = io.outputs.trigger(base + 2);
        for &at in &self.ends {
            end.push(at);
        }
    }

    fn reset(&mut self) {
        self.playing = false;
        self.current = None;
        self.forward = true;
        self.gates = 0;
        self.shown = 0;
        self.velocity = [0.0; MAX_ROWS];
    }

    /// An edited grid keeps playing from the step it was on; the next
    /// clock plays the next step of the new grid. Gates of rows that are
    /// gone close.
    fn take_over(&mut self, previous: &dyn Node) -> bool {
        let Some(old) = (previous as &dyn std::any::Any).downcast_ref::<Pattern>() else {
            return false;
        };
        let last = self.steps - 1;
        self.playing = old.playing;
        self.current = old.current.map(|c| c.min(last));
        self.forward = old.forward;
        self.gates = old.gates & ((1u32 << self.rows) - 1) as u16;
        self.shown = old.shown.min(last);
        self.bank = old.bank.min(self.banks - 1);
        self.velocity = old.velocity;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<Pattern>(config, 48_000.0, block, &NoResources)
    }

    fn two_rows() -> serde_json::Value {
        json!({ "steps": 4, "rows": 2, "cells": ["x.x.", ".xx"] })
    }

    #[test]
    fn a_placed_pattern_plays_the_step_the_song_is_on() {
        // 120 BPM: a sixteenth is 6000 samples; placed at beat 1.
        const STEP: u64 = 6_000;
        let mut h = harness(two_rows(), 64);
        h.set("start", "at").set("at", 1.0);
        let ticks: Vec<u64> = (0..9).map(|k| k * STEP).collect();
        h.triggers("clock", &ticks);
        let out = h.run((9 * STEP) as usize);
        assert_eq!(out.triggers("trigger_0"), &[4 * STEP, 6 * STEP, 8 * STEP]);
        assert_eq!(out.triggers("trigger_1"), &[5 * STEP, 6 * STEP]);
        // From the middle of the song: six steps in is step 2 of the next
        // pass, which both rows play.
        let t = 9 * STEP;
        h.seek(4 * STEP + 6 * STEP);
        h.triggers("clock", &[t, t + STEP]);
        let out = h.run(STEP as usize + 64);
        assert_eq!(out.triggers("trigger_0"), &[t]);
        assert_eq!(out.triggers("trigger_1"), &[t]);
        assert_eq!(out.int("step").last(), Some(&3));
        // Unlooped, the clock after the last step ends it.
        let mut h = harness(two_rows(), 64);
        h.set("start", "at").set("at", 1.0).set("loop", false);
        h.triggers("clock", &ticks);
        let out = h.run((9 * STEP) as usize);
        assert_eq!(out.triggers("end"), &[8 * STEP]);
    }

    #[test]
    fn sockets_follow_the_config() {
        let sig = Pattern::describe(&json!({ "rows": 3 }));
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            outs,
            [
                "trigger_0",
                "gate_0",
                "velocity_0",
                "trigger_1",
                "gate_1",
                "velocity_1",
                "trigger_2",
                "gate_2",
                "velocity_2",
                "step",
                "playing",
                "end"
            ]
        );
        let ins: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            ins,
            [
                "clock", "start", "at", "until", "interval", "stop", "reset", "loop", "reverse",
                "bank"
            ]
        );
        assert!(sig.inputs[AT].song && sig.inputs[INTERVAL].song);
        assert_eq!(sig.inputs[LOOP].default, Value::Bool(true));
        assert_eq!(Pattern::describe(&json!({})).outputs.len(), 4 * 3 + 3);
        assert_eq!(
            Pattern::describe(&json!({ "rows": 99 })).outputs.len(),
            16 * 3 + 3
        );
        assert_eq!(
            Pattern::describe(&json!({ "rows": 0 })).outputs.len(),
            3 + 3
        );
    }

    #[test]
    fn cells_parse_and_repair() {
        let g = grids(&json!({ "steps": 3, "rows": 3, "cells": ["x.X.", 5, "..7q", "xxxx"] }));
        assert_eq!(g, [vec![vec![10, 0, 10], vec![0, 0, 0], vec![0, 0, 7]]]);
        let g = grids(&json!({}));
        assert_eq!((g.len(), g[0].len(), g[0][0].len()), (1, 4, 16));
        assert!(g.iter().flatten().flatten().all(|&l| l == 0));
        // Several banks; a bank that isn't a list is empty.
        let c = json!({ "steps": 2, "rows": 1, "cells": [["x5"], 3, [".x"]] });
        let g = grids(&c);
        assert_eq!(g, [vec![vec![10, 5]], vec![vec![0, 0]], vec![vec![0, 10]]]);
        assert_eq!(
            cells_value(&g),
            json!([["x5"], [".."], [".x"]]),
            "written back as banks"
        );
        assert_eq!(cells_value(&g[..1]), json!(["x5"]), "one bank, one list");
        let many: Vec<_> = (0..40).map(|_| json!(["x"])).collect();
        assert_eq!(grids(&json!({ "cells": many })).len(), MAX_BANKS);
        assert_eq!(step_count(&json!({ "steps": 0 })), 1);
        assert_eq!(step_count(&json!({ "steps": 1000 })), 64);
    }

    #[test]
    fn clocks_play_steps_with_triggers_and_gates() {
        let mut h = harness(two_rows(), 64);
        h.triggers("clock", &[0, 10, 20, 30, 40]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_0"), &[0, 20, 40]);
        assert_eq!(out.triggers("trigger_1"), &[10, 20]);
        let g0 = out.float("gate_0");
        assert!(g0[..10].iter().all(|&g| g == 1.0));
        assert!(g0[10..20].iter().all(|&g| g == 0.0));
        assert!(g0[20..30].iter().all(|&g| g == 1.0));
        assert!(g0[30..40].iter().all(|&g| g == 0.0));
        assert!(g0[40..].iter().all(|&g| g == 1.0));
        let g1 = out.float("gate_1");
        assert!(g1[..10].iter().all(|&g| g == 0.0));
        assert!(g1[10..30].iter().all(|&g| g == 1.0), "tied across on steps");
        assert!(g1[30..].iter().all(|&g| g == 0.0));
        // The fifth clock wrapped around to step 0.
        assert_eq!(out.int("step")[63], 0);
        assert!(out.float("playing").iter().all(|&p| p == 1.0));
        assert!(out.triggers("end").is_empty());
    }

    #[test]
    fn nothing_moves_without_a_clock() {
        let mut h = harness(two_rows(), 64);
        let out = h.run(256);
        assert!(out.triggers("trigger_0").is_empty());
        assert!(out.float("gate_0").iter().all(|&g| g == 0.0));
        assert_eq!(out.float("playing")[0], 1.0);
    }

    #[test]
    fn without_loop_it_stops_on_the_clock_after_the_last_step() {
        let mut h = harness(two_rows(), 64);
        h.set("loop", false);
        h.triggers("clock", &[0, 10, 20, 30, 40, 50]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_0"), &[0, 20]);
        assert_eq!(out.triggers("end"), &[40]);
        assert!(out.float("gate_0")[40..].iter().all(|&g| g == 0.0));
        assert_eq!(out.float("playing")[63], 0.0);
        assert_eq!(out.int("step")[63], 3);
    }

    #[test]
    fn start_stop_reset_and_reverse() {
        let mut h = harness(two_rows(), 64);
        h.triggers("clock", &[0, 10, 20, 30, 40, 50, 60]);
        h.triggers("stop", &[15]);
        // A wired `start` replaces the transport-play default.
        h.triggers("start", &[0, 30]);
        h.triggers("reverse", &[50]);
        let out = h.run(64);
        // Steps: 0 at 0, 1 at 10, stop at 15 (end), clock 20 ignored, start
        // and step 0 at 30, step 1 at 40, reversed: step 0 at 50, step 3 at 60.
        assert_eq!(out.triggers("end"), &[15]);
        assert_eq!(out.triggers("trigger_0"), &[0, 30, 50]);
        assert_eq!(out.triggers("trigger_1"), &[10, 40]);
        assert!(out.float("gate_1")[15..30].iter().all(|&g| g == 0.0));
        assert_eq!(out.int("step")[63], 3);

        let mut h = harness(two_rows(), 64);
        h.triggers("clock", &[0, 10, 20, 30]);
        h.triggers("reset", &[20]);
        let out = h.run(64);
        // Reset and clock on the same sample: the clock plays step 0.
        assert_eq!(out.triggers("trigger_0"), &[0, 20]);
        assert_eq!(out.triggers("trigger_1"), &[10, 30]);
    }

    #[test]
    fn reversed_before_starting_it_begins_at_the_last_step() {
        let mut h = harness(json!({ "steps": 3, "rows": 1, "cells": ["..x"] }), 64);
        h.triggers("reverse", &[0]).triggers("clock", &[0, 5, 9]);
        let out = h.run(16);
        assert_eq!(out.triggers("trigger_0"), &[0]);
        assert_eq!(out.int("step")[15], 0);
    }

    #[test]
    fn transport_stop_and_restart() {
        let mut h = harness(two_rows(), 64);
        h.triggers("clock", &[0, 64, 128, 192, 256]);
        h.run(128);
        h.stop();
        let out = h.run(128);
        // Stopped on the transport stop: the clocks at 128 and 192 do nothing.
        assert_eq!(out.triggers("end"), &[128]);
        assert!(out.triggers("trigger_0").is_empty());
        assert!(out.float("gate_0").iter().all(|&g| g == 0.0));
        h.play();
        assert_eq!(h.run(64).triggers("trigger_0"), &[256]);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let config = json!({
            "steps": 16,
            "rows": 3,
            "cells": ["x...x...x...x...", "....x.......x..x", "xx.xx.xxx.x.xx.."]
        });
        let clocks: Vec<u64> = (0..100).map(|k| k * 47 + (k % 3)).collect();
        let render = |block| {
            let mut h = harness(config.clone(), block);
            h.triggers("clock", &clocks)
                .triggers("reverse", &[1_000, 3_001])
                .triggers("reset", &[2_000])
                .triggers("stop", &[4_000])
                .triggers("start", &[0, 4_200]);
            let out = h.run(4_800);
            (0..3)
                .map(|r| {
                    (
                        out.triggers(&format!("trigger_{r}")).to_vec(),
                        out.float(&format!("gate_{r}")).to_vec(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let one = render(1);
        assert!(one.iter().all(|(t, _)| !t.is_empty()));
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn an_edited_grid_plays_on_from_the_same_step() {
        // Clocks every 16 samples; the grid is edited after the third step.
        let mut h = harness(json!({ "steps": 4, "rows": 1, "cells": ["x..."] }), 48);
        h.triggers("clock", &(0..12).map(|k| k * 16).collect::<Vec<_>>());
        let out = h.run(48);
        assert_eq!(out.triggers("trigger_0"), &[0]);
        assert_eq!(out.int("step")[47], 2);
        // Without the edit the next hit is step 0 at 64; the new grid has
        // step 3 on, so it fires at 48 and the pattern keeps its place.
        h.reconfigure::<Pattern>(json!({ "steps": 4, "rows": 1, "cells": ["x..x"] }));
        let out = h.run(48);
        assert_eq!(out.triggers("trigger_0"), &[48, 64]);
        assert_eq!(out.float("playing")[0], 1.0);
        // A grid now no longer than the step it was on wraps at the next
        // clock, still playing.
        h.reconfigure::<Pattern>(json!({ "steps": 1, "rows": 1, "cells": ["x"] }));
        let out = h.run(48);
        assert_eq!(out.triggers("trigger_0"), &[96, 112, 128]);
    }

    #[test]
    fn velocity_holds_each_rows_last_level() {
        let mut h = harness(
            json!({ "steps": 4, "rows": 2, "cells": ["x.5.", "..2x"] }),
            64,
        );
        h.triggers("clock", &[0, 10, 20, 30]);
        let out = h.run(64);
        let v0 = out.float("velocity_0");
        assert!(v0[..20].iter().all(|&v| v == 1.0));
        assert!(v0[20..].iter().all(|&v| v == 0.5), "held over the off step");
        let v1 = out.float("velocity_1");
        assert!(v1[..20].iter().all(|&v| v == 0.0), "nothing played yet");
        assert!(v1[20..30].iter().all(|&v| (v - 0.2).abs() < 1e-6));
        assert!(v1[30..].iter().all(|&v| v == 1.0));
        // Still held in the next block.
        assert_eq!(h.run(8).float("velocity_1")[0], 1.0);
    }

    #[test]
    fn a_new_bank_starts_with_the_next_pass() {
        let c = json!({ "steps": 4, "rows": 1, "cells": [["x..."], ["xxxx"]] });
        let mut h = harness(c.clone(), 16);
        h.triggers("clock", &(0..12).map(|k| k * 10).collect::<Vec<_>>());
        // Bank 1 asked for during the first pass.
        let out = h.run(16);
        assert_eq!(out.triggers("trigger_0"), &[0]);
        h.set("bank", 1);
        let out = h.run(64);
        // Steps 2 and 3 still bank 0; the pass from 40 on is bank 1.
        assert_eq!(out.triggers("trigger_0"), &[40, 50, 60, 70]);
        // Stopped, a bank applies at once; out of range, the last.
        let mut h = harness(c, 16);
        h.set("bank", 7);
        h.triggers("clock", &[0, 10]);
        assert_eq!(h.run(16).triggers("trigger_0"), &[0, 10]);
    }

    #[test]
    fn telemetry_shows_the_step_and_bank_playing() {
        let mut h = harness(
            json!({ "steps": 4, "rows": 1, "cells": [["x..."], ["x.x."]] }),
            16,
        );
        h.run(16);
        assert_eq!(h.telemetry(TELEMETRY_STEP), -1.0, "before the first clock");
        h.set("bank", 1).triggers("clock", &[16, 26]);
        h.run(16);
        assert_eq!(h.telemetry(TELEMETRY_STEP), 1.0);
        assert_eq!(h.telemetry(TELEMETRY_BANK), 1.0);
        assert_eq!(h.telemetry(TELEMETRY_PLAYING), 1.0);
    }

    #[test]
    fn reset_stops_and_rewinds() {
        let mut h = harness(two_rows(), 64);
        h.triggers("clock", &[0, 10]);
        h.run(64);
        // A transport restart resets every node and plays from zero again,
        // which fires the default start.
        h.reset().seek(0);
        h.triggers("clock", &[64 + 5]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_0"), &[69]);
        assert_eq!(out.int("step")[0], 0);
    }
}
