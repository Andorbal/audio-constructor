use ac_core::{
    Config, ConfigExt, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    TempoMap, Unit, node,
};

node! {
    kind: "clock",
    name: "Clock",
    category: Triggers,
    keywords: ["metronome", "tick", "pulse", "lfo"],
    /// Fires a trigger every `interval`, locked to the transport.
    ///
    /// Ticks sit on a grid counted from the start of the song: tick *k*
    /// falls on `offset + k × interval`, measured in beats (following tempo
    /// changes) or in seconds, as chosen by the `unit` config. Every tick
    /// lands on its exact sample. Because the grid is worked out from the
    /// song position rather than counted up, the clock never drifts and is
    /// already in step after a seek, a loop, or a restart. A new `interval`
    /// or `offset` takes effect from the next tick.
    ///
    /// `gate` goes high at each tick for `duty` × `interval` (at 1 it stays
    /// high). `count` is the number of ticks since the start of the song on
    /// the current grid, so it jumps when `interval` changes. The clock is
    /// quiet while the transport is stopped.
    ///
    /// **Try this:** set `interval` to 0.25 for sixteenth notes and wire
    /// `trigger` into an `ADSR` for a hi-hat pulse, or wire `gate` into an
    /// envelope and use `duty` as the note length.
    pub struct Clock {
        seconds: bool,
    }
}

const INTERVAL: usize = 0;
const DUTY: usize = 1;
const OFFSET: usize = 2;

const TRIGGER: usize = 0;
const GATE: usize = 1;
const COUNT: usize = 2;

/// Upper bound on `interval` and `offset`. Far beyond any musical use; it
/// keeps grid positions small enough that sample arithmetic can't overflow.
const MAX_SPAN: f64 = 1.0e6;

fn uses_seconds(config: &Config) -> bool {
    config.get_str_or("unit", "beats") == "seconds"
}

/// The clock's inputs for one tick, in its unit (beats or seconds).
#[derive(Clone, Copy, Debug)]
struct Grid {
    interval: f64,
    offset: f64,
    duty: f64,
}

/// Maps between sample positions and the clock's unit.
#[derive(Clone, Copy)]
struct Timeline<'a> {
    tempo: &'a TempoMap,
    rate: f64,
    seconds: bool,
}

impl Timeline<'_> {
    fn unit_at(&self, sample: u64) -> f64 {
        if self.seconds {
            sample as f64 / self.rate
        } else {
            self.tempo.beat_at(sample)
        }
    }

    fn sample_at(&self, t: f64) -> u64 {
        if self.seconds {
            (t.max(0.0) * self.rate).round() as u64
        } else {
            self.tempo.sample_at(t)
        }
    }

    /// The length of one sample in the clock's unit, at `sample`.
    fn one_sample(&self, sample: u64) -> f64 {
        if self.seconds {
            1.0 / self.rate
        } else {
            1.0 / self.tempo.samples_per_beat_at(sample)
        }
    }

    /// Sanitized inputs read at `sample`. The interval is at least one
    /// sample so every tick gets its own sample.
    fn grid(&self, sample: u64, interval: f32, duty: f32, offset: f32) -> Grid {
        let finite = |v: f32, d: f64| if v.is_finite() { f64::from(v) } else { d };
        Grid {
            interval: finite(interval, 1.0).clamp(self.one_sample(sample), MAX_SPAN),
            offset: finite(offset, 0.0).clamp(0.0, MAX_SPAN),
            duty: finite(duty, 0.5).clamp(0.0, 1.0),
        }
    }

    fn tick(&self, g: &Grid, k: u64) -> u64 {
        self.sample_at(g.offset + k as f64 * g.interval)
    }

    /// Where tick `k`'s gate falls (exclusive).
    fn gate_end(&self, g: &Grid, k: u64) -> u64 {
        self.sample_at(g.offset + (k as f64 + g.duty) * g.interval)
    }

    /// The grid index at `sample`, rounded down (0 before the first tick).
    /// Rounding of tick samples means the exact tick may be one index on
    /// either side, which the callers check.
    fn index_at(&self, g: &Grid, sample: u64) -> Option<u64> {
        let x = ((self.unit_at(sample) - g.offset) / g.interval).floor();
        x.is_finite().then(|| x.max(0.0) as u64)
    }

    /// The first tick at or after `sample`, as (index, sample).
    fn next_tick(&self, g: &Grid, sample: u64) -> Option<(u64, u64)> {
        let k0 = self.index_at(g, sample)?;
        (k0..k0.saturating_add(3))
            .map(|k| (k, self.tick(g, k)))
            .find(|&(_, s)| s >= sample)
    }

    /// The last tick before `sample`, as (index, sample).
    fn prev_tick(&self, g: &Grid, sample: u64) -> Option<(u64, u64)> {
        let k1 = self.index_at(g, sample)?.saturating_add(1);
        (k1.saturating_sub(2)..=k1)
            .rev()
            .map(|k| (k, self.tick(g, k)))
            .find(|&(_, s)| s < sample)
    }
}

/// Sets the gate high over absolute samples `from..to`, clipped to the block.
fn gate_high(gate: &mut FloatOut, block: (u64, u64), from: u64, to: u64) {
    let (start, end) = block;
    let a = from.clamp(start, end) - start;
    let b = to.clamp(start, end) - start;
    for i in a..b {
        gate.set(i as usize, 1.0);
    }
}

impl Node for Clock {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let unit = if uses_seconds(config) {
            Unit::Seconds
        } else {
            Unit::Beats
        };
        Signature::new()
            .input(
                SocketSpec::float("interval", unit, 1.0)
                    .range(0.0625, 16.0)
                    .no_smooth()
                    .doc("Time between ticks."),
            )
            .input(
                SocketSpec::float("duty", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .doc("Gate length as a fraction of the interval."),
            )
            .input(
                SocketSpec::float("offset", unit, 0.0)
                    .range(0.0, 16.0)
                    .no_smooth()
                    .doc("Shifts the grid later."),
            )
            .output(SocketSpec::trigger("trigger").doc("Fires on each tick."))
            .output(SocketSpec::boolean("gate", false).doc("High for `duty` of each interval."))
            .output(SocketSpec::int("count", 0).doc("Ticks since the start of the song."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Clock {
            seconds: uses_seconds(config),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let t = ctx.transport;
        let line = Timeline {
            tempo: t.tempo,
            rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                t.tempo.sample_rate()
            },
            seconds: self.seconds,
        };
        let interval = io.inputs.float(INTERVAL);
        let duty = io.inputs.float(DUTY);
        let offset = io.inputs.float(OFFSET);
        let start = t.position;
        let end = start.saturating_add(ctx.frames as u64);
        let grid_at = |sample: u64| {
            let i = sample.saturating_sub(start) as usize;
            line.grid(sample, interval.at(i), duty.at(i), offset.at(i))
        };

        let g0 = grid_at(start);
        let prev = line.prev_tick(&g0, start);
        let mut count = prev.map_or(0, |(k, _)| k.saturating_add(1));

        let (mut gate, mut trig) = io.outputs.float_trigger(GATE, TRIGGER);
        gate.fill(0.0);
        if t.playing {
            if let Some((k, s)) = prev {
                gate_high(&mut gate, (start, end), s, line.gate_end(&g0, k));
            }
            // Each step reads the inputs just after the last tick, which is
            // what makes a change take effect from the next tick. `cur`
            // strictly increases, so this runs at most `frames` times.
            let mut cur = start;
            while cur < end {
                let g = grid_at(cur);
                let Some((k, s)) = line.next_tick(&g, cur) else {
                    break;
                };
                if s >= end {
                    break;
                }
                trig.push((s - start) as u32);
                gate_high(&mut gate, (start, end), s, line.gate_end(&g, k));
                count = k.saturating_add(1);
                cur = s + 1;
            }
        }
        io.outputs.set_int(COUNT, count.min(i32::MAX as u64) as i32);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{
        AutoFlags, FloatSrc, HostAccess, InputSlot, Inputs, NoResources, OutputSlot, Outputs,
        ParamSlot, Phase, TelemetryAccess, TempoSegment, TransportView,
    };
    use serde_json::json;

    fn harness(config: serde_json::Value, rate: f64, block: usize) -> Harness {
        Harness::with_resources::<Clock>(config, rate, block, &NoResources)
    }

    #[test]
    fn one_beat_ticks_land_on_exact_samples_with_odd_blocks() {
        let mut h = harness(json!({}), 48_000.0, 37);
        let out = h.run(24_000 * 40 + 1);
        let expected: Vec<u64> = (0..=40).map(|k| k * 24_000).collect();
        assert_eq!(out.triggers("trigger"), expected.as_slice());
        assert_eq!(*out.int("count").last().unwrap(), 41);
    }

    #[test]
    fn seconds_mode() {
        let mut h = harness(json!({"unit": "seconds"}), 44_100.0, 37);
        h.set("interval", 0.1);
        let out = h.run(44_101);
        let expected: Vec<u64> = (0..=10).map(|k| k * 4_410).collect();
        assert_eq!(out.triggers("trigger"), expected.as_slice());
        assert_eq!(
            Clock::describe(&json!({"unit": "seconds"})).inputs[INTERVAL].unit,
            Unit::Seconds
        );
        assert_eq!(Clock::describe(&json!({})).inputs[OFFSET].unit, Unit::Beats);
    }

    #[test]
    fn offset_shifts_the_grid() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.set("offset", 0.5);
        let out = h.run(60_000);
        assert_eq!(out.triggers("trigger"), &[12_000, 36_000]);
        assert_eq!(out.int("count")[0], 0);
    }

    #[test]
    fn gate_is_high_for_duty_of_the_interval() {
        let mut h = harness(json!({}), 48_000.0, 37);
        h.set("duty", 0.25);
        let out = h.run(48_000);
        let gate = out.float("gate");
        assert!(gate[..6_000].iter().all(|&g| g == 1.0));
        assert!(gate[6_000..24_000].iter().all(|&g| g == 0.0));
        assert!(gate[24_000..30_000].iter().all(|&g| g == 1.0));
        assert!(gate[30_000..].iter().all(|&g| g == 0.0));

        let mut h = harness(json!({}), 48_000.0, 64);
        h.set("duty", 1.0);
        assert!(h.run(50_000).float("gate").iter().all(|&g| g == 1.0));
    }

    #[test]
    fn stopped_transport_emits_nothing() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.stopped();
        let out = h.run(48_000);
        assert!(out.triggers("trigger").is_empty());
        assert!(out.float("gate").iter().all(|&g| g == 0.0));
        assert!(out.int("count").iter().all(|&c| c == 0));
    }

    #[test]
    fn restart_resumes_on_the_grid() {
        let mut h = harness(json!({}), 48_000.0, 64);
        assert_eq!(h.run(30_016).triggers("trigger"), &[0, 24_000]);
        h.stop();
        let out = h.run(1_024);
        assert!(out.triggers("trigger").is_empty());
        assert!(out.int("count").iter().all(|&c| c == 2));
        h.play();
        // Transport position 48 000 is harness frame 49 024.
        assert_eq!(h.run(30_000).triggers("trigger"), &[49_024]);
    }

    #[test]
    fn follows_a_tempo_change() {
        let tempo = TempoMap::new(
            48_000.0,
            &[
                TempoSegment {
                    beat: 0.0,
                    bpm: 120.0,
                    time_sig: [4, 4],
                },
                TempoSegment {
                    beat: 4.0,
                    bpm: 60.0,
                    time_sig: [4, 4],
                },
            ],
        );
        let mut h = harness(json!({}), 48_000.0, 37);
        h.tempo(tempo);
        let out = h.run(200_000);
        assert_eq!(
            out.triggers("trigger"),
            &[0, 24_000, 48_000, 72_000, 96_000, 144_000, 192_000]
        );
    }

    #[test]
    fn interval_change_takes_effect_at_the_next_tick() {
        let mut h = harness(json!({}), 48_000.0, 64);
        let interval: Vec<f32> = (0..70_000)
            .map(|i| if i < 30_000 { 1.0 } else { 0.5 })
            .collect();
        h.set_block("interval", interval);
        let out = h.run(70_000);
        assert_eq!(
            out.triggers("trigger"),
            &[0, 24_000, 36_000, 48_000, 60_000]
        );
    }

    #[test]
    fn tiny_or_bad_intervals_tick_at_most_once_per_sample() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.set("interval", 0.0);
        let out = h.run(128);
        let expected: Vec<u64> = (0..128).collect();
        assert_eq!(out.triggers("trigger"), expected.as_slice());

        let mut h = harness(json!({}), 48_000.0, 64);
        h.set_samples("interval", vec![f32::NAN; 64]);
        h.set_samples("offset", vec![f32::INFINITY; 64]);
        assert_eq!(h.run(64).triggers("trigger"), &[0]);

        let mut h = harness(json!({}), 48_000.0, 64);
        h.set("interval", 1.0e30);
        assert_eq!(h.run(64).triggers("trigger"), &[0]);
    }

    /// Runs one block at an arbitrary transport position (a seek), which
    /// the harness can't do. Returns (trigger offsets, gate, count).
    fn block_at(position: u64, frames: usize, duty: f32) -> (Vec<u32>, Vec<f32>, i32) {
        let mut clock = Clock { seconds: false };
        let slots = [
            InputSlot::Float(FloatSrc::Param(0)),
            InputSlot::Float(FloatSrc::Param(1)),
            InputSlot::Float(FloatSrc::Param(2)),
        ];
        let param = |value| ParamSlot {
            value,
            ..ParamSlot::default()
        };
        let params = [param(1.0), param(duty), param(0.0)];
        let out_slots = [
            OutputSlot::Trigger(0),
            OutputSlot::Float {
                local: 0,
                sample: true,
            },
            OutputSlot::Int(0),
        ];
        let mut samples = vec![vec![0.0; frames]];
        let mut ints = [0];
        let mut triggers = vec![Vec::with_capacity(frames)];
        let tempo = TempoMap::constant(48_000.0, 120.0);
        let mut transport = TransportView::stopped_at_zero(&tempo);
        transport.playing = true;
        transport.position = position;
        let ctx = ProcessCtx {
            frames,
            sample_rate: 48_000.0,
            transport,
            phase: Phase::Main,
            engine: Default::default(),
        };
        let mut io = Io {
            inputs: Inputs {
                slots: &slots,
                frames,
                params: &params,
                scalars: &[],
                ints: &[],
                samples: &[],
                chan_table: &[],
                triggers: &[],
                midi: &[],
                spectra: &[],
                buffers: &[],
                texts: &[],
                auto: AutoFlags::default(),
                local: Default::default(),
            },
            outputs: Outputs {
                slots: &out_slots,
                frames,
                samples: &mut samples,
                scalars: &mut [],
                ints: &mut ints,
                triggers: &mut triggers,
                midi: &mut [],
                spectra: &mut [],
                buffers: &mut [],
                texts: &mut [],
            },
            host: HostAccess::none(frames),
            telemetry: TelemetryAccess::none(),
        };
        clock.process(&ctx, &mut io);
        (triggers[0].clone(), samples[0].clone(), ints[0])
    }

    #[test]
    fn a_seek_lands_on_the_grid() {
        let (trig, gate, count) = block_at(23_990, 64, 0.5);
        assert_eq!(trig, vec![10]);
        assert_eq!(count, 2);
        assert!(gate[..10].iter().all(|&g| g == 0.0));
        assert!(gate[10..].iter().all(|&g| g == 1.0));

        // Mid-gate: the gate from a tick before the block is still high.
        let (trig, gate, count) = block_at(984_000 + 100, 64, 0.5);
        assert!(trig.is_empty());
        assert_eq!(count, 42);
        assert!(gate.iter().all(|&g| g == 1.0));
    }
}
