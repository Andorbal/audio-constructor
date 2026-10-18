use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::time_samples;

node! {
    kind: "threshold_trigger",
    name: "Threshold Trigger",
    category: Triggers,
    keywords: ["level", "detect", "envelope", "gate", "schmitt", "hit", "transient", "sidechain"],
    /// Fires a trigger when a level rises above a threshold.
    ///
    /// The level counts as *above* from the sample `in` rises past
    /// `threshold` until it falls to `threshold − hysteresis` or below, so a
    /// noisy level hovering around the threshold doesn't fire over and over.
    /// With `hysteresis` at 0 it is a plain `in > threshold`.
    ///
    /// `hold` (seconds) is how long the level must stay above before the
    /// trigger fires: at 0 it fires on the sample the level crosses; at
    /// 0.01 it fires 10 ms later, and only if the level is still above by
    /// then, which ignores short spikes. `gate` is high from the trigger
    /// until the level falls back, and `release` fires on that sample. A
    /// level that falls back before `hold` is up fires nothing at all.
    ///
    /// Every input is read sample by sample when wired at audio rate, and
    /// the triggers land on the exact sample. NaN or infinite inputs count
    /// as 0, and a negative `hysteresis` or `hold` counts as 0. Restarting
    /// the transport forgets the level, so a level that is already above
    /// fires again.
    ///
    /// **Try this:** wire an `Envelope Follower` on a drum loop into `in`,
    /// set `threshold` to 0.3, and use `trigger` to start a `Clip` or an
    /// `ADSR`, so every hit fires a sample of your own.
    pub struct ThresholdTrigger {
        /// The level is above (with hysteresis).
        above: bool,
        /// Samples the level has been above.
        above_for: u64,
        /// The trigger fired for the current time above.
        fired: bool,
        /// Release offsets of the current block.
        releases: Vec<u32>,
    }
}

const IN: usize = 0;
const THRESHOLD: usize = 1;
const HYSTERESIS: usize = 2;
const HOLD: usize = 3;

const TRIGGER: usize = 0;
const GATE: usize = 1;
const RELEASE: usize = 2;

fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

impl Node for ThresholdTrigger {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("in", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("The level to watch."),
            )
            .input(
                SocketSpec::float("threshold", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("The level that fires the trigger."),
            )
            .input(
                SocketSpec::float("hysteresis", Unit::Generic, 0.05)
                    .range(0.0, 0.5)
                    .doc("How far below the threshold the level must fall to re-arm."),
            )
            .input(
                SocketSpec::float("hold", Unit::Seconds, 0.0)
                    .range(0.0, 1.0)
                    .doc("How long the level must stay above before firing."),
            )
            .output(SocketSpec::trigger("trigger").doc("Fires when the level has risen above."))
            .output(
                SocketSpec::boolean("gate", false)
                    .doc("High from the trigger until the level falls back."),
            )
            .output(SocketSpec::trigger("release").doc("Fires when the level falls back."))
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        ThresholdTrigger {
            above: false,
            above_for: 0,
            fired: false,
            releases: Vec::with_capacity(ctx.max_block.max(1)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let level = io.inputs.float(IN);
        let threshold = io.inputs.float(THRESHOLD);
        let hysteresis = io.inputs.float(HYSTERESIS);
        let hold = io.inputs.float(HOLD);
        self.releases.clear();
        {
            let (mut gate, mut trigger) = io.outputs.float_trigger(GATE, TRIGGER);
            for i in 0..ctx.frames {
                let x = finite_or_zero(level.at(i));
                let t = finite_or_zero(threshold.at(i));
                let h = finite_or_zero(hysteresis.at(i)).max(0.0);
                if self.above {
                    if x <= t - h {
                        self.above = false;
                        if self.fired && self.releases.len() < self.releases.capacity() {
                            self.releases.push(i as u32);
                        }
                        self.fired = false;
                    } else {
                        self.above_for = self.above_for.saturating_add(1);
                    }
                } else if x > t {
                    self.above = true;
                    self.above_for = 0;
                }
                if self.above
                    && !self.fired
                    && self.above_for >= time_samples(hold.at(i), false, ctx, i)
                {
                    self.fired = true;
                    trigger.push(i as u32);
                }
                gate.set(i, if self.fired { 1.0 } else { 0.0 });
            }
        }
        let mut release = io.outputs.trigger(RELEASE);
        for &r in &self.releases {
            release.push(r);
        }
    }

    fn reset(&mut self) {
        self.above = false;
        self.above_for = 0;
        self.fired = false;
        self.releases.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    /// A level that is `high` over the given frame ranges and 0 elsewhere.
    fn level(len: usize, spans: &[(usize, usize, f32)]) -> Vec<f32> {
        let mut v = vec![0.0; len];
        for &(a, b, x) in spans {
            v[a..b].fill(x);
        }
        v
    }

    fn run(block: usize, lvl: &[f32], set: &[(&str, f32)]) -> ac_core::testing::Output {
        let mut h = Harness::with_resources::<ThresholdTrigger>(
            json!({}),
            48_000.0,
            block,
            &ac_core::NoResources,
        );
        h.set_samples("in", lvl.to_vec());
        for &(k, v) in set {
            h.set(k, v);
        }
        h.run(lvl.len())
    }

    #[test]
    fn fires_on_the_crossing_sample_and_releases_when_it_falls() {
        let lvl = level(400, &[(10, 100, 0.8), (250, 300, 0.6)]);
        let out = run(64, &lvl, &[("hysteresis", 0.0)]);
        assert_eq!(out.triggers("trigger"), &[10, 250]);
        assert_eq!(out.triggers("release"), &[100, 300]);
        let gate = out.float("gate");
        assert!(gate[..10].iter().all(|&g| g == 0.0));
        assert!(gate[10..100].iter().all(|&g| g == 1.0));
        assert!(gate[100..250].iter().all(|&g| g == 0.0));
        assert!(gate[250..300].iter().all(|&g| g == 1.0));
        assert!(gate[300..].iter().all(|&g| g == 0.0));
        // Exactly at the threshold is not above.
        let out = run(64, &level(64, &[(5, 20, 0.5)]), &[("hysteresis", 0.0)]);
        assert!(out.triggers("trigger").is_empty());
    }

    #[test]
    fn hysteresis_ignores_chatter() {
        // Dips to 0.47 don't re-arm with 0.05 hysteresis; the dip to 0.4 does.
        let mut lvl = level(300, &[(10, 200, 0.6)]);
        for i in (20..100).step_by(10) {
            lvl[i] = 0.47;
        }
        lvl[150] = 0.4;
        let out = run(64, &lvl, &[]);
        assert_eq!(out.triggers("trigger"), &[10, 151]);
        assert_eq!(out.triggers("release"), &[150, 200]);
        // Without hysteresis every dip fires again.
        let out = run(64, &lvl, &[("hysteresis", 0.0)]);
        assert_eq!(out.triggers("trigger").len(), 10);
    }

    #[test]
    fn hold_waits_and_skips_short_spikes() {
        // 10 ms hold = 480 samples. The 100-sample spike fires nothing.
        let lvl = level(2_000, &[(50, 150, 0.9), (500, 1_500, 0.9)]);
        let out = run(64, &lvl, &[("hold", 0.01)]);
        assert_eq!(out.triggers("trigger"), &[980]);
        assert_eq!(out.triggers("release"), &[1_500]);
        let gate = out.float("gate");
        assert!(gate[..980].iter().all(|&g| g == 0.0));
        assert!(gate[980..1_500].iter().all(|&g| g == 1.0));
        assert!(gate[1_500..].iter().all(|&g| g == 0.0));
    }

    #[test]
    fn block_size_does_not_matter() {
        let mut lvl = level(
            5_000,
            &[(100, 900, 0.7), (1_000, 1_030, 0.9), (2_000, 4_000, 0.55)],
        );
        for i in (2_100..3_900).step_by(97) {
            lvl[i] = 0.3;
        }
        let a = run(1, &lvl, &[("hold", 0.001)]);
        for block in [64, 512] {
            let b = run(block, &lvl, &[("hold", 0.001)]);
            assert_eq!(a.triggers("trigger"), b.triggers("trigger"));
            assert_eq!(a.triggers("release"), b.triggers("release"));
            assert_eq!(a.float("gate"), b.float("gate"));
        }
        assert!(!a.triggers("trigger").is_empty());
    }

    #[test]
    fn bad_inputs_count_as_zero() {
        let lvl = vec![f32::NAN, f32::INFINITY, 1.0, f32::NEG_INFINITY, 1.0, 0.0];
        let out = run(64, &lvl, &[("hysteresis", -1.0), ("hold", -5.0)]);
        assert_eq!(out.triggers("trigger"), &[2, 4]);
        assert_eq!(out.triggers("release"), &[3, 5]);
        // A NaN threshold counts as 0, so a level of 0.2 is above it.
        let mut h = Harness::new::<ThresholdTrigger>(json!({}));
        h.set_samples("in", vec![0.2; 64]);
        h.set_samples("threshold", vec![f32::NAN; 64]);
        assert_eq!(h.run(64).triggers("trigger"), &[0]);
    }

    #[test]
    fn a_constant_level_fires_once_and_reset_rearms() {
        let mut h = Harness::new::<ThresholdTrigger>(json!({}));
        h.set("in", 1.0);
        let out = h.run(256);
        assert_eq!(out.triggers("trigger"), &[0]);
        assert!(out.float("gate").iter().all(|&g| g == 1.0));
        h.reset();
        assert_eq!(h.run(64).triggers("trigger"), &[256]);
        h.set("in", 0.0);
        assert_eq!(h.run(64).triggers("release"), &[320]);
    }
}
