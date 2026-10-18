use ac_core::{
    Config, ConfigExt, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Value, node,
};

use crate::segments::Runs;

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
    /// them on. The `cells` config holds the grid as one string per row,
    /// `x` for an on step and `.` for an off one, such as
    /// `["x...x...x...x...", "....x.......x..."]`. Missing rows and steps
    /// are off, and any other character counts as off.
    ///
    /// Each `clock` trigger moves to the next step. Where that step is on
    /// in row *n*, `trigger_n` fires on the clock's sample and `gate_n`
    /// goes high until the next clock (rows count from 0, the top row). On
    /// steps next to each other keep the gate high from one to the next,
    /// so use the trigger to restart an envelope on every step. `step` is
    /// the last step played, counting from 0. Nothing moves without a
    /// clock: wire a `Clock` into `clock` (`interval` 0.25 for sixteenth
    /// notes).
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
    /// `....x.......x...`.
    pub struct Pattern {
        steps: usize,
        rows: usize,
        /// One bit per step, per row.
        cells: [u64; MAX_ROWS],
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
        /// Steps played this block: (frame, rows whose trigger fires).
        fires: Vec<(u32, u16)>,
        ends: Vec<u32>,
    }
}

const CLOCK: usize = 0;
const START: usize = 1;
const STOP: usize = 2;
const RESET: usize = 3;
const LOOP: usize = 4;
const REVERSE: usize = 5;

pub const MAX_STEPS: u64 = 64;
pub const MAX_ROWS: usize = 16;

/// Steps per row from config, clamped (DECISIONS D29).
pub fn step_count(config: &Config) -> usize {
    config.get_u64_or("steps", 16).clamp(1, MAX_STEPS) as usize
}

/// Rows from config, clamped.
pub fn row_count(config: &Config) -> usize {
    config.get_u64_or("rows", 4).clamp(1, MAX_ROWS as u64) as usize
}

/// The `cells` config as one bit per step per row.
fn parse_cells(config: &Config, steps: usize, rows: usize) -> [u64; MAX_ROWS] {
    let mut cells = [0u64; MAX_ROWS];
    let Some(list) = config.get("cells").and_then(|v| v.as_array()) else {
        return cells;
    };
    for (bits, row) in cells.iter_mut().zip(list).take(rows) {
        let Some(text) = row.as_str() else {
            continue;
        };
        for (k, c) in text.chars().take(steps).enumerate() {
            if c == 'x' || c == 'X' {
                *bits |= 1 << k;
            }
        }
    }
    cells
}

fn trigger_out(row: usize) -> usize {
    2 * row
}

fn gate_out(row: usize) -> usize {
    2 * row + 1
}

/// Sets frames `from..to` of a gate output.
fn fill(out: &mut FloatOut, from: usize, to: usize, high: bool) {
    let v = if high { 1.0 } else { 0.0 };
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
    /// Rows that are on at step `s`.
    fn column(&self, s: usize) -> u16 {
        let mut m = 0u16;
        for (r, bits) in self.cells.iter().enumerate().take(self.rows) {
            if bits >> s & 1 == 1 {
                m |= 1 << r;
            }
        }
        m
    }

    fn first_step(&self) -> usize {
        if self.forward { 0 } else { self.steps - 1 }
    }

    /// One clock: the next step, or the end when not looping.
    fn advance(&mut self, at: u32, looping: bool) {
        let last = self.steps - 1;
        let next = match self.current {
            None => Some(self.first_step()),
            Some(c) if self.forward && c < last => Some(c + 1),
            Some(c) if !self.forward && c > 0 => Some(c - 1),
            Some(_) if looping => Some(self.first_step()),
            Some(_) => None,
        };
        match next {
            Some(s) => {
                self.current = Some(s);
                self.shown = s;
                self.gates = self.column(s);
                if self.gates != 0 && self.fires.len() < self.fires.capacity() {
                    self.fires.push((at, self.gates));
                }
            }
            None => {
                self.playing = false;
                self.gates = 0;
                self.push_end(at);
            }
        }
    }

    fn push_end(&mut self, at: u32) {
        if self.ends.len() < self.ends.capacity() {
            self.ends.push(at);
        }
    }
}

impl Node for Pattern {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::trigger("clock").doc("Moves to the next step."))
            .generator_inputs(true, false)
            .input(SocketSpec::trigger("reverse").doc("Toggles the direction."));
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
                );
        }
        sig.output(SocketSpec::int("step", 0).doc("The last step played, from 0."))
            .generator_outputs()
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let steps = step_count(config);
        let rows = row_count(config);
        let cap = ctx.max_block.max(1) + 1;
        Pattern {
            steps,
            rows,
            cells: parse_cells(config, steps, rows),
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

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        let looping = io.inputs.boolean(LOOP);
        let events = [
            io.inputs.triggers(REVERSE),
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
            io.inputs.triggers(CLOCK),
        ];
        self.changes.clear();
        self.fires.clear();
        self.ends.clear();
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
                self.advance(at, looping.at(run.at));
            }
            if self.gates != before && self.changes.len() < self.changes.capacity() {
                self.changes.push((at, self.gates));
            }
        }

        for r in 0..self.rows {
            let mut trig = io.outputs.trigger(trigger_out(r));
            for &(at, rows) in &self.fires {
                if rows >> r & 1 == 1 {
                    trig.push(at);
                }
            }
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
        let base = 2 * self.rows;
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
    fn sockets_follow_the_config() {
        let sig = Pattern::describe(&json!({ "rows": 3 }));
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(
            outs,
            [
                "trigger_0",
                "gate_0",
                "trigger_1",
                "gate_1",
                "trigger_2",
                "gate_2",
                "step",
                "playing",
                "end"
            ]
        );
        let ins: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(ins, ["clock", "start", "stop", "reset", "loop", "reverse"]);
        assert_eq!(sig.inputs[LOOP].default, Value::Bool(true));
        assert_eq!(Pattern::describe(&json!({})).outputs.len(), 4 * 2 + 3);
        assert_eq!(
            Pattern::describe(&json!({ "rows": 99 })).outputs.len(),
            16 * 2 + 3
        );
        assert_eq!(
            Pattern::describe(&json!({ "rows": 0 })).outputs.len(),
            2 + 3
        );
    }

    #[test]
    fn cells_parse_and_repair() {
        let c = parse_cells(&json!({ "cells": ["x.X.", 5, "..xq", "xxxx"] }), 3, 3);
        assert_eq!(&c[..4], &[0b101, 0, 0b100, 0]);
        assert_eq!(parse_cells(&json!({}), 16, 4), [0; MAX_ROWS]);
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
