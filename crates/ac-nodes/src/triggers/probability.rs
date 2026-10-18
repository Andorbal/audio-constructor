use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::rng::Rng;

use super::frames_of;

node! {
    kind: "probability",
    name: "Probability",
    category: Triggers,
    keywords: ["chance", "random", "maybe", "coin", "bernoulli", "skip", "humanize", "gate"],
    /// Lets each trigger through with a chance.
    ///
    /// Each trigger on `in` passes to `out` with probability `chance`
    /// (0–1): at 0.5 about half get through, at 1 all of them, at 0 none.
    /// The ones that don't pass come out of `else` instead, on the same
    /// sample, so the two outputs split one stream between two sounds.
    /// `chance` is read at each trigger's sample; values below 0, or NaN,
    /// count as 0 and values above 1 count as 1. Several triggers on one
    /// sample count once.
    ///
    /// The coin flips come from the project's seed, so a render sounds the
    /// same every time, and restarting the transport replays the same
    /// sequence. Every trigger uses up one flip whatever `chance` is, so
    /// changing `chance` keeps the same triggers in the lead.
    ///
    /// **Try this:** put a Probability at 0.7 between a sixteenth-note
    /// `Clock` and a hi-hat for a part that never quite repeats, and wire
    /// `else` into a quieter sound to fill the gaps.
    pub struct Probability {
        seed: u64,
        rng: Rng,
    }
}

const IN: usize = 0;
const CHANCE: usize = 1;

const OUT: usize = 0;
const ELSE: usize = 1;

impl Node for Probability {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("in").doc("The triggers to filter."))
            .input(
                SocketSpec::float("chance", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Probability that a trigger passes, 0–1."),
            )
            .output(SocketSpec::trigger("out").doc("The triggers that passed."))
            .output(SocketSpec::trigger("else").doc("The triggers that didn't."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        Probability {
            seed: ctx.seed,
            rng: Rng::new(ctx.seed),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let chance = io.inputs.float(CHANCE);
        let ins = io.inputs.triggers(IN);
        // Flips are made once, in order, and replayed for the second output
        // from a copy of the generator, so nothing is buffered.
        let start = self.rng.clone();
        let passes = |rng: &mut Rng, o: u64| {
            let p = chance.at(o as usize);
            let p = if p.is_nan() { 0.0 } else { f64::from(p) };
            rng.uniform() < p
        };
        {
            let mut out = io.outputs.trigger(OUT);
            for o in frames_of(ins, ctx.frames) {
                if passes(&mut self.rng, o) {
                    out.push(o as u32);
                }
            }
        }
        let mut replay = start;
        let mut other = io.outputs.trigger(ELSE);
        for o in frames_of(ins, ctx.frames) {
            if !passes(&mut replay, o) {
                other.push(o as u32);
            }
        }
    }

    fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn run(block: usize, at: &[u64], chance: f32, frames: usize) -> (Vec<u64>, Vec<u64>) {
        let mut h = Harness::with_resources::<Probability>(
            json!({}),
            48_000.0,
            block,
            &ac_core::NoResources,
        );
        h.triggers("in", at);
        h.set("chance", chance);
        let out = h.run(frames);
        (out.triggers("out").to_vec(), out.triggers("else").to_vec())
    }

    #[test]
    fn splits_every_trigger_between_the_outputs() {
        let at: Vec<u64> = (0..2_000).map(|k| k * 5).collect();
        let (out, other) = run(64, &at, 0.3, 10_000);
        assert_eq!(out.len() + other.len(), at.len());
        let mut all = [out.clone(), other].concat();
        all.sort_unstable();
        assert_eq!(all, at);
        let share = out.len() as f64 / at.len() as f64;
        assert!((share - 0.3).abs() < 0.04, "{share}");
    }

    #[test]
    fn zero_and_one_and_bad_chances() {
        let at = [1, 7, 30];
        assert_eq!(run(64, &at, 1.0, 64), (at.to_vec(), vec![]));
        assert_eq!(run(64, &at, 5.0, 64), (at.to_vec(), vec![]));
        assert_eq!(run(64, &at, 0.0, 64), (vec![], at.to_vec()));
        assert_eq!(run(64, &at, -1.0, 64), (vec![], at.to_vec()));
        let mut h = Harness::new::<Probability>(json!({}));
        h.triggers("in", &at);
        h.set_samples("chance", vec![f32::NAN; 64]);
        assert!(h.run(64).triggers("out").is_empty());
    }

    #[test]
    fn the_same_seed_gives_the_same_triggers_at_any_block_size() {
        let at: Vec<u64> = (0..500).map(|k| k * 13 + k % 3).collect();
        let a = run(1, &at, 0.5, 7_000);
        assert_eq!(a, run(64, &at, 0.5, 7_000));
        assert_eq!(a, run(512, &at, 0.5, 7_000));
        assert!(!a.0.is_empty() && !a.1.is_empty());
    }

    #[test]
    fn changing_chance_keeps_the_same_leaders() {
        // Lowering the chance only removes triggers; it never adds new ones.
        let at: Vec<u64> = (0..300).map(|k| k * 3).collect();
        let (high, _) = run(64, &at, 0.8, 1_000);
        let (low, _) = run(64, &at, 0.4, 1_000);
        assert!(low.iter().all(|t| high.contains(t)));
    }

    #[test]
    fn reset_replays_the_sequence() {
        let at: Vec<u64> = (0..32).map(|k| k * 2).collect();
        let mut h = Harness::new::<Probability>(json!({}));
        h.triggers("in", &at);
        let first = h.run(64).triggers("out").to_vec();
        h.reset();
        let shifted: Vec<u64> = at.iter().map(|t| t + 64).collect();
        h.triggers("in", &shifted);
        let again: Vec<u64> = h.run(64).triggers("out").iter().map(|t| t - 64).collect();
        assert_eq!(first, again);
        assert!(!first.is_empty() && first.len() < at.len());
    }
}
