use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use crate::segments::Runs;

node! {
    kind: "euclidean",
    name: "Euclidean",
    category: Triggers,
    keywords: ["rhythm", "pattern", "bjorklund", "sequencer", "polyrhythm", "drum", "steps"],
    /// Steps through a Euclidean rhythm: `pulses` hits spread as evenly as
    /// possible over `steps`.
    ///
    /// Each trigger on `clock` moves one step (the first clock plays step
    /// 0) and fires `trigger` if that step is a hit. `gate` is high from a
    /// hit's clock until the next clock, so hits on neighbouring steps make
    /// one long gate; use `trigger` to sound each hit. `step` is the step
    /// played last (0 before the first clock), as of the end of the block.
    ///
    /// The patterns come from Bjorklund's algorithm, as in Toussaint's "The
    /// Euclidean Algorithm Generates Traditional Musical Rhythms": 3 pulses
    /// in 8 steps give `x..x..x.`, 5 in 8 give `x.xx.xx.`, 7 in 16 give
    /// `x..x.x.x..x.x.x.`. `rotation` moves the pattern later by that many
    /// steps (rotation 1 turns `x..x..x.` into `.x..x..x`); negative values
    /// move it earlier. `steps` (1–64), `pulses` (0 to `steps`), and
    /// `rotation` are read once per block, and out-of-range values are
    /// clamped or wrapped.
    ///
    /// `reset` makes the next clock play step 0 again, without touching
    /// `gate` or `step`. A `reset` and a `clock` on the same sample apply in
    /// that order, so that clock plays step 0. Restarting the transport
    /// resets the pattern and drops the gate.
    ///
    /// **Try this:** drive three Euclideans from one sixteenth-note `Clock`
    /// with 4, 3, and 7 pulses over 16 steps for a kick, a snare, and a
    /// hi-hat, and turn `rotation` until the groove sits right.
    pub struct Euclidean {
        /// (steps, pulses, rotation) of the cached pattern.
        key: (u32, u32, u32),
        /// Bit `i` set when step `i` is a hit.
        pattern: u64,
        /// The step played last; `None` before the first clock.
        pos: Option<u32>,
        gate: bool,
    }
}

const CLOCK: usize = 0;
const STEPS: usize = 1;
const PULSES: usize = 2;
const ROTATION: usize = 3;
const RESET: usize = 4;

const TRIGGER: usize = 0;
const GATE: usize = 1;
const STEP: usize = 2;

const MAX_STEPS: u32 = 64;

/// Bjorklund's algorithm (Toussaint, "The Euclidean Algorithm Generates
/// Traditional Musical Rhythms", 2005): start with `pulses` groups `[1]`
/// and `steps − pulses` groups `[0]`, and keep appending one remainder
/// group to each leading group until at most one remainder group is left.
/// Groups are bit strings (the first step in the highest bit). Returns the
/// pattern with bit `i` set when step `i` is a hit.
fn bjorklund(steps: u32, pulses: u32) -> u64 {
    let n = steps.clamp(1, MAX_STEPS) as usize;
    let k = (pulses as usize).min(n);
    let mut a = [(1u128, 1u32); MAX_STEPS as usize];
    let mut b = [(0u128, 1u32); MAX_STEPS as usize];
    let (mut na, mut nb) = (k, n - k);
    while nb > 1 && na > 0 {
        let m = na.min(nb);
        let mut rest = [(0u128, 0u32); MAX_STEPS as usize];
        let n_rest = if na > nb {
            rest[..na - m].copy_from_slice(&a[m..na]);
            na - m
        } else {
            rest[..nb - m].copy_from_slice(&b[m..nb]);
            nb - m
        };
        for (x, &(bits, len)) in a[..m].iter_mut().zip(&b[..m]) {
            *x = ((x.0 << len) | bits, x.1 + len);
        }
        na = m;
        b = rest;
        nb = n_rest;
    }
    let mut bits = 0u128;
    for &(g, len) in a[..na].iter().chain(&b[..nb]) {
        bits = (bits << len) | g;
    }
    (0..n)
        .filter(|&i| (bits >> (n - 1 - i)) & 1 == 1)
        .fold(0u64, |mask, i| mask | (1 << i))
}

/// The hits of a Euclidean rhythm, with bit `i` set when step `i` is a hit:
/// `pulses` over `steps` (clamped to 1–64 and 0–`steps`), moved later by
/// `rotation` steps. The editor uses this to draw the pattern.
pub fn pattern(steps: i32, pulses: i32, rotation: i32) -> u64 {
    let n = steps.clamp(1, MAX_STEPS as i32);
    let base = bjorklund(n as u32, pulses.clamp(0, n) as u32);
    let r = rotation.rem_euclid(n) as u32;
    let n = n as u32;
    (0..n)
        .filter(|&i| (base >> ((i + n - r) % n)) & 1 == 1)
        .fold(0u64, |mask, i| mask | (1 << i))
}

impl Node for Euclidean {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("clock").doc("Moves one step."))
            .input(
                SocketSpec::int("steps", 8)
                    .range(1.0, 32.0)
                    .doc("Steps in the pattern (1–64)."),
            )
            .input(
                SocketSpec::int("pulses", 3)
                    .range(0.0, 32.0)
                    .doc("Hits spread over the steps."),
            )
            .input(
                SocketSpec::int("rotation", 0)
                    .range(0.0, 31.0)
                    .doc("Moves the pattern later by this many steps."),
            )
            .input(SocketSpec::trigger("reset").doc("The next clock plays step 0."))
            .output(SocketSpec::trigger("trigger").doc("Fires on each hit."))
            .output(SocketSpec::boolean("gate", false).doc("High from a hit until the next clock."))
            .output(SocketSpec::int("step", 0).doc("The step played last."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Euclidean {
            key: (8, 3, 0),
            pattern: pattern(8, 3, 0),
            pos: None,
            gate: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let steps = io.inputs.int(STEPS).clamp(1, MAX_STEPS as i32);
        let pulses = io.inputs.int(PULSES).clamp(0, steps);
        let rotation = io.inputs.int(ROTATION).rem_euclid(steps);
        let key = (steps as u32, pulses as u32, rotation as u32);
        if key != self.key {
            self.key = key;
            self.pattern = pattern(steps, pulses, rotation);
        }
        let n = steps as u32;
        let lists = [io.inputs.triggers(RESET), io.inputs.triggers(CLOCK)];
        {
            let (mut gate, mut trigger) = io.outputs.float_trigger(GATE, TRIGGER);
            for run in Runs::new(lists, ctx.frames) {
                let [reset, clock] = run.fired;
                if reset {
                    self.pos = None;
                }
                if clock {
                    let p = self.pos.map_or(0, |p| (p + 1) % n);
                    self.pos = Some(p);
                    self.gate = (self.pattern >> p) & 1 == 1;
                    if self.gate {
                        trigger.push(run.at as u32);
                    }
                }
                let g = if self.gate { 1.0 } else { 0.0 };
                for i in run.at..run.until {
                    gate.set(i, g);
                }
            }
        }
        io.outputs.set_int(STEP, self.pos.unwrap_or(0) as i32);
    }

    fn reset(&mut self) {
        self.pos = None;
        self.gate = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn text(steps: i32, pulses: i32, rotation: i32) -> String {
        let p = pattern(steps, pulses, rotation);
        (0..steps.clamp(1, 64))
            .map(|i| if (p >> i) & 1 == 1 { 'x' } else { '.' })
            .collect()
    }

    #[test]
    fn known_patterns_from_toussaint() {
        assert_eq!(text(8, 3, 0), "x..x..x.");
        assert_eq!(text(8, 5, 0), "x.xx.xx.");
        assert_eq!(text(5, 2, 0), "x.x..");
        assert_eq!(text(9, 4, 0), "x.x.x.x..");
        assert_eq!(text(9, 5, 0), "x.x.x.x.x");
        assert_eq!(text(12, 5, 0), "x..x.x..x.x.");
        assert_eq!(text(12, 7, 0), "x.xx.x.xx.x.");
        assert_eq!(text(13, 5, 0), "x..x.x..x.x..");
        assert_eq!(text(16, 7, 0), "x..x.x.x..x.x.x.");
        assert_eq!(text(16, 5, 0), "x..x..x..x..x...");
        assert_eq!(text(7, 3, 0), "x.x.x..");
    }

    #[test]
    fn every_pattern_has_the_right_number_of_hits() {
        for n in 1..=64 {
            for k in 0..=n {
                let p = pattern(n, k, 0);
                assert_eq!(p.count_ones(), k as u32, "E({k},{n})");
                if n < 64 {
                    assert_eq!(p >> n, 0, "E({k},{n}) has bits past the end");
                }
                for r in [1, 5, -3] {
                    assert_eq!(pattern(n, k, r).count_ones(), k as u32);
                }
            }
        }
        assert_eq!(pattern(64, 64, 0), u64::MAX);
        assert_eq!(text(4, 0, 0), "....");
        assert_eq!(text(4, 1, 0), "x...");
    }

    #[test]
    fn rotation_moves_the_pattern_later() {
        assert_eq!(text(8, 3, 1), ".x..x..x");
        assert_eq!(text(8, 3, 2), "x.x..x..");
        assert_eq!(text(8, 3, -1), "..x..x.x");
        assert_eq!(text(8, 3, 9), text(8, 3, 1));
    }

    #[test]
    fn out_of_range_settings_are_clamped() {
        assert_eq!(text(0, 3, 0), "x");
        assert_eq!(text(4, 9, 0), "xxxx");
        assert_eq!(text(4, -2, 0), "....");
        assert_eq!(pattern(1_000, 1, 0), 1);
        assert_eq!(pattern(i32::MIN, i32::MAX, i32::MIN), 1);
    }

    fn run(block: usize, clocks: &[u64], resets: &[u64]) -> ac_core::testing::Output {
        let mut h =
            Harness::with_resources::<Euclidean>(json!({}), 48_000.0, block, &ac_core::NoResources);
        h.triggers("clock", clocks);
        h.triggers("reset", resets);
        h.run(1_000)
    }

    #[test]
    fn clocks_step_through_the_pattern() {
        // E(3,8): hits on steps 0, 3, 6; clocks every 100 samples.
        let clocks: Vec<u64> = (0..10).map(|k| k * 100).collect();
        let out = run(64, &clocks, &[]);
        assert_eq!(out.triggers("trigger"), &[0, 300, 600, 800]);
        let gate = out.float("gate");
        assert!(gate[..100].iter().all(|&g| g == 1.0));
        assert!(gate[100..300].iter().all(|&g| g == 0.0));
        assert!(gate[300..400].iter().all(|&g| g == 1.0));
        assert!(gate[700..800].iter().all(|&g| g == 0.0));
        assert!(gate[800..900].iter().all(|&g| g == 1.0));
        assert!(gate[900..].iter().all(|&g| g == 0.0));
        // Step at the end of each block: 0 before the first clock.
        let step = out.int("step");
        assert_eq!(step[0], 0);
        assert_eq!(step[350], 3);
        assert_eq!(step[999], 1);
    }

    #[test]
    fn reset_comes_before_a_clock_on_the_same_sample() {
        let clocks: Vec<u64> = (0..10).map(|k| k * 100).collect();
        let out = run(64, &clocks, &[200]);
        // Steps: 0, 1, (reset) 0, 1, 2, 3, 4, 5, 6, 7.
        assert_eq!(out.triggers("trigger"), &[0, 200, 500, 800]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let clocks: Vec<u64> = (0..90).map(|k| k * 11 + k % 4).collect();
        let a = run(1, &clocks, &[150, 433]);
        for block in [64, 512] {
            let b = run(block, &clocks, &[150, 433]);
            assert_eq!(a.triggers("trigger"), b.triggers("trigger"));
            assert_eq!(a.float("gate"), b.float("gate"));
        }
        assert!(a.triggers("trigger").len() > 20);
    }

    #[test]
    fn settings_change_the_pattern() {
        let mut h = Harness::new::<Euclidean>(json!({}));
        h.set("steps", 4).set("pulses", 4);
        h.triggers("clock", &[0, 10, 20, 30]);
        assert_eq!(h.run(64).triggers("trigger"), &[0, 10, 20, 30]);
        h.set("pulses", 0);
        h.triggers("clock", &[70]);
        assert!(h.run(64).triggers("trigger").is_empty());
    }

    #[test]
    fn transport_reset_starts_over_and_drops_the_gate() {
        let mut h = Harness::new::<Euclidean>(json!({}));
        h.triggers("clock", &[0, 10, 100]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger"), &[0]);
        assert_eq!(out.int("step")[0], 1);
        h.reset();
        let out = h.run(64);
        assert!(out.float("gate")[..36].iter().all(|&g| g == 0.0));
        assert_eq!(out.triggers("trigger"), &[100]);
    }
}
