//! Chain fusion: grouping the schedule's steps into chains and waves
//! (ARCHITECTURE §7 step 8, DECISIONS D114).
//!
//! A chain is a run of steps one thread works through in order, reading
//! what earlier steps of the chain wrote straight from the chain's work
//! space. A wave is a set of chains that don't read each other, so they can
//! run on different cores; every value a chain reads from outside itself
//! was written in an earlier wave. Handing a wave to the workers and
//! waiting for them costs microseconds, so a schedule of long chains in few
//! waves scales where one wave per step would spend its time handing off.
//!
//! Which steps share a chain changes when they run, never what they
//! compute: every step still sees exactly the inputs it would see alone.

use std::collections::HashSet;

/// What the planner needs to know about one step, in schedule order.
#[derive(Clone, Debug, Default)]
pub(crate) struct StepInfo {
    /// Steps whose outputs this step reads (all earlier in the schedule).
    pub reads: Vec<usize>,
    /// Steps that must run before this one without passing it values (a
    /// Feedback node's write runs after its read).
    pub after: Vec<usize>,
    /// Writes device outputs: runs alone, on the audio thread, after the
    /// wave's chains, in the order such steps had before chains existed.
    pub serial: bool,
    /// The node this step runs (`None` for a conversion step).
    pub node: Option<u32>,
    /// The voice copy the step belongs to, if any.
    pub voice: Option<u32>,
}

/// One chain: its wave and its steps in the order they run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chain {
    pub wave: usize,
    pub serial: bool,
    pub steps: Vec<usize>,
}

/// Chains in run order: by wave, the parallel chains before the serial
/// steps, each group in the order the planner made them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan {
    pub chains: Vec<Chain>,
    /// The chain (index into `chains`) of every step.
    pub chain_of: Vec<usize>,
}

/// The wave every step would have on its own: one more than the latest
/// wave it reads from or must follow.
pub(crate) fn step_waves(steps: &[StepInfo]) -> Vec<usize> {
    let mut wave = vec![0usize; steps.len()];
    for (s, info) in steps.iter().enumerate() {
        wave[s] = info
            .reads
            .iter()
            .chain(&info.after)
            .filter(|&&d| d < s)
            .map(|&d| wave[d] + 1)
            .max()
            .unwrap_or(0);
    }
    wave
}

/// Group steps into chains and give each chain a wave.
///
/// Steps are visited in their step-wave order (a topological order). A
/// step joins the chain of what it reads when exactly one chain holds the
/// latest of its inputs (everything else it reads or follows is in earlier
/// waves), neither is serial, that chain doesn't already run the step's
/// node (the two steps of a Feedback node share its state), and either
///
/// - each step of that chain it reads feeds nothing but this step (a
///   linear run: fusing it costs no parallelism), or
/// - both belong to the same voice copy (copies run side by side, so each
///   copy's body is one chain).
///
/// Otherwise it starts a chain one wave after the latest it reads. Serial
/// steps never fuse, and each runs no earlier than the serial step before
/// it, so device outputs sum in the same order as they always have.
pub(crate) fn plan(steps: &[StepInfo]) -> Plan {
    let n = steps.len();
    let mut consumers = vec![0u32; n];
    for (s, info) in steps.iter().enumerate() {
        let mut reads: Vec<usize> = info.reads.iter().copied().filter(|&d| d < s).collect();
        reads.sort_unstable();
        reads.dedup();
        for d in reads {
            consumers[d] += 1;
        }
    }
    let step_wave = step_waves(steps);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&s| (step_wave[s], s));

    struct Building {
        wave: usize,
        serial: bool,
        voice: Option<u32>,
        steps: Vec<usize>,
    }
    let mut chains: Vec<Building> = Vec::new();
    let mut chain_of = vec![usize::MAX; n];
    // (node, chain) for every chain that runs a step of the node.
    let mut node_chain: HashSet<(u32, usize)> = HashSet::new();
    let mut last_serial_wave = 0usize;
    for &t in &order {
        let info = &steps[t];
        // The chain holding the latest input, and whether it's the only one.
        let mut top: Option<(usize, usize)> = None; // (wave, chain)
        let mut unique = true;
        for &d in info.reads.iter().chain(&info.after) {
            let Some(&c) = chain_of.get(d) else { continue };
            let Some(ch) = chains.get(c) else { continue };
            match top {
                None => top = Some((ch.wave, c)),
                Some((w, _)) if ch.wave > w => {
                    top = Some((ch.wave, c));
                    unique = true;
                }
                Some((w, tc)) if ch.wave == w && c != tc => unique = false,
                _ => {}
            }
        }
        let join = top.filter(|&(_, c)| {
            let ch = &chains[c];
            if !unique || info.serial || ch.serial {
                return false;
            }
            if info
                .node
                .is_some_and(|node| node_chain.contains(&(node, c)))
            {
                return false;
            }
            let same_voice = info.voice.is_some() && info.voice == ch.voice;
            let linear = info
                .reads
                .iter()
                .filter(|&&d| chain_of.get(d) == Some(&c))
                .all(|&d| consumers[d] == 1);
            same_voice || linear
        });
        let c = match join {
            Some((_, c)) => {
                let ch = &mut chains[c];
                ch.steps.push(t);
                if info.voice.is_some() {
                    ch.voice = info.voice;
                }
                c
            }
            None => {
                let mut wave = top.map_or(0, |(w, _)| w + 1);
                if info.serial {
                    wave = wave.max(last_serial_wave);
                    last_serial_wave = wave;
                }
                chains.push(Building {
                    wave,
                    serial: info.serial,
                    voice: info.voice,
                    steps: vec![t],
                });
                chains.len() - 1
            }
        };
        chain_of[t] = c;
        if let Some(node) = info.node {
            node_chain.insert((node, c));
        }
    }

    let mut run: Vec<(usize, Building)> = chains.into_iter().enumerate().collect();
    run.sort_by_key(|(c, b)| (b.wave, b.serial, *c));
    let mut index = vec![0usize; run.len()];
    for (k, (c, _)) in run.iter().enumerate() {
        index[*c] = k;
    }
    Plan {
        chains: run
            .into_iter()
            .map(|(_, b)| Chain {
                wave: b.wave,
                serial: b.serial,
                steps: b.steps,
            })
            .collect(),
        chain_of: chain_of
            .iter()
            .map(|&c| index.get(c).copied().unwrap_or(0))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(reads: &[usize], node: u32) -> StepInfo {
        StepInfo {
            reads: reads.to_vec(),
            node: Some(node),
            ..StepInfo::default()
        }
    }

    fn waves(plan: &Plan) -> usize {
        plan.chains.iter().map(|c| c.wave + 1).max().unwrap_or(0)
    }

    #[test]
    fn linear_runs_fuse_and_fans_dont() {
        // 0 → 1 → 2 → {3, 4}; 3 → 5, 4 → 6; {5, 6} → 7 → 8 (serial).
        let mut steps = vec![
            step(&[], 0),
            step(&[0], 1),
            step(&[1], 2),
            step(&[2], 3),
            step(&[2], 4),
            step(&[3], 5),
            step(&[4], 6),
            step(&[5, 6], 7),
            step(&[7], 8),
        ];
        steps[8].serial = true;
        let plan = plan(&steps);
        let chains: Vec<(usize, Vec<usize>)> = plan
            .chains
            .iter()
            .map(|c| (c.wave, c.steps.clone()))
            .collect();
        assert_eq!(
            chains,
            vec![
                (0, vec![0, 1, 2]),
                (1, vec![3, 5]),
                (1, vec![4, 6]),
                (2, vec![7]),
                (3, vec![8]),
            ]
        );
        assert_eq!(plan.chain_of[5], 1);
    }

    #[test]
    fn independent_chains_share_their_waves() {
        // 8 chains of 25 steps into a mix and a device output.
        let mut steps = Vec::new();
        let mut ends = Vec::new();
        for _ in 0..8 {
            for i in 0..25 {
                let s = steps.len();
                let reads: &[usize] = if i == 0 { &[] } else { &[s - 1] };
                steps.push(step(reads, s as u32));
                if i == 24 {
                    ends.push(s);
                }
            }
        }
        let mix = steps.len();
        steps.push(step(&ends, mix as u32));
        steps.push(step(&[mix], mix as u32 + 1));
        steps[mix + 1].serial = true;
        let plan = plan(&steps);
        assert_eq!(waves(&plan), 3);
        assert_eq!(plan.chains.len(), 10);
        assert!(plan.chains[..8].iter().all(|c| c.steps.len() == 25));
    }

    #[test]
    fn voice_copies_become_one_chain_each() {
        // An allocator (0) feeding two copies of a diamond body; sums read both.
        let mut steps = vec![step(&[], 0)];
        let mut outs = Vec::new();
        for v in 0..2u32 {
            let b = steps.len();
            for (reads, k) in [
                (vec![0], 0),
                (vec![b], 1),
                (vec![b], 2),
                (vec![b + 1, b + 2], 3),
            ] {
                steps.push(StepInfo {
                    reads,
                    node: Some(10 + v * 10 + k),
                    voice: Some(v),
                    ..StepInfo::default()
                });
            }
            outs.push(b + 3);
        }
        steps.push(step(&outs, 99));
        let plan = plan(&steps);
        assert_eq!(waves(&plan), 3);
        assert_eq!(plan.chains[1].steps, vec![1, 2, 3, 4]);
        assert_eq!(plan.chains[2].steps, vec![5, 6, 7, 8]);
        // Without voices the diamond stays apart.
        let plain: Vec<StepInfo> = steps
            .iter()
            .cloned()
            .map(|s| StepInfo { voice: None, ..s })
            .collect();
        assert_eq!(waves(&super::plan(&plain)), 5);
    }

    #[test]
    fn a_feedback_write_never_shares_its_reads_chain() {
        // Feedback read (0) → filter (1) → feedback write (2, same node as 0).
        let steps = vec![
            step(&[], 7),
            step(&[0], 1),
            StepInfo {
                reads: vec![1],
                after: vec![0],
                node: Some(7),
                ..StepInfo::default()
            },
        ];
        let plan = plan(&steps);
        assert_eq!(plan.chains[0].steps, vec![0, 1]);
        assert_eq!(plan.chains[1].steps, vec![2]);
        assert_eq!(plan.chains[1].wave, 1);
    }

    #[test]
    fn serial_steps_keep_their_order() {
        // Device outputs at step waves 3, 1, 2 run in that order, though
        // fused chains put all three right after wave 0.
        let mut steps = vec![
            step(&[], 0),
            step(&[0], 1),
            step(&[1], 2),
            step(&[2], 3), // serial, step wave 3
            step(&[], 4),
            step(&[4], 5), // serial, step wave 1
            step(&[], 6),
            step(&[6], 7),
            step(&[7], 8), // serial, step wave 2
        ];
        for s in [3, 5, 8] {
            steps[s].serial = true;
        }
        let plan = plan(&steps);
        let serial: Vec<(usize, usize)> = plan
            .chains
            .iter()
            .filter(|c| c.serial)
            .map(|c| (c.wave, c.steps[0]))
            .collect();
        assert_eq!(serial, vec![(1, 5), (1, 8), (1, 3)]);
    }
}
