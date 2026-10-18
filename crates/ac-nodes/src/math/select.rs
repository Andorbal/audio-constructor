use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::finite_or_zero;
use super::route::{Fader, HeldNotes, RouteType};

node! {
    kind: "select",
    name: "Select",
    category: Math,
    keywords: ["switch", "choose", "pick", "router", "multiplexer", "mux", "index"],
    /// Passes on one of several inputs, picked by `index`.
    ///
    /// The `inputs` config sets how many inputs there are (1 to 64, default
    /// 4), named `in_0`, `in_1`, …, and the `type` config sets what they
    /// carry: `float` (the default), `audio`, `int`, `bool`, `trigger`, or
    /// `midi`. `out` has the same type.
    ///
    /// `index` picks the input, counting from 0. It wraps around: with four
    /// inputs, 4 picks `in_0` again and −1 picks `in_3`, so a `Counter`
    /// without a `max` steps through the inputs over and over. `index` is
    /// an `Int`, so the choice changes at block boundaries.
    ///
    /// - `float` and `bool`: `out` runs at the fastest rate among the
    ///   inputs; NaN or infinite values come out as 0.
    /// - `audio`: changing `index` crossfades from the old input to the new
    ///   one over 5 ms so the switch doesn't click. `out` has as many
    ///   channels as the widest input.
    /// - `trigger`: passes on the picked input's triggers.
    /// - `midi`: passes on the picked input's events. Changing `index`
    ///   releases any notes (and sustain pedals) still held from the old
    ///   input, so nothing hangs.
    ///
    /// **Try this:** set four `Constant`s to note numbers, wire them into
    /// `in_0`..`in_3`, and drive `index` from a `Counter` counting `Clock`
    /// ticks: a four-step arpeggio for `Convert` (`note_to_hz`) and an
    /// `Oscillator`. With `type` `audio`, Select switches between sound
    /// sources.
    pub struct Select {
        ty: RouteType,
        inputs: usize,
        fader: Fader,
        held: HeldNotes,
        /// The input picked last block (MIDI release tracking).
        last: Option<usize>,
    }
}

const MAX_INPUTS: u64 = 64;
const OUT: usize = 0;

fn input_count(config: &Config) -> usize {
    config.get_u64_or("inputs", 4).clamp(1, MAX_INPUTS) as usize
}

impl Node for Select {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let ty = RouteType::from_config(config);
        let n = input_count(config);
        let mut sig = Signature::new();
        for i in 0..n {
            sig = sig.input(ty.input(format!("in_{i}")).doc("An input to pick from."));
        }
        sig.input(
            SocketSpec::int("index", 0)
                .range(0.0, (n - 1) as f64)
                .doc("Which input to pass on (wraps around)."),
        )
        .output(ty.output().doc("The picked input."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let ty = RouteType::from_config(config);
        let inputs = input_count(config);
        Select {
            ty,
            inputs,
            fader: Fader::new(
                if ty == RouteType::Audio { inputs } else { 1 },
                ctx.sample_rate,
            ),
            held: HeldNotes::default(),
            last: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let n = self.inputs;
        let k = i64::from(io.inputs.int(n)).rem_euclid(n as i64) as usize;
        let frames = ctx.frames;
        match self.ty {
            RouteType::Float => {
                let x = io.inputs.float(k);
                io.outputs.float(OUT).fill_with(|i| finite_or_zero(x.at(i)));
            }
            RouteType::Bool => {
                let x = io.inputs.boolean(k);
                io.outputs
                    .float(OUT)
                    .fill_with(|i| if x.at(i) { 1.0 } else { 0.0 });
            }
            RouteType::Int => {
                let v = io.inputs.int(k);
                io.outputs.set_int(OUT, v);
            }
            RouteType::Audio => {
                let ins = io.inputs;
                let mut out = io.outputs.audio(OUT);
                self.fader
                    .render(&mut out, frames, |j| ins.audio(j), |_| k, false);
            }
            RouteType::Trigger => {
                let events = io.inputs.triggers(k);
                let mut out = io.outputs.trigger(OUT);
                for &at in events {
                    out.push(at);
                }
            }
            RouteType::Midi => {
                let events = io.inputs.midi(k);
                let mut out = io.outputs.midi(OUT);
                if self.last.is_some_and(|last| last != k) {
                    self.held.release_all(&mut out, 0);
                }
                for &ev in events {
                    self.held.forward(&mut out, ev);
                }
            }
        }
        self.last = Some(k);
    }

    fn reset(&mut self) {
        self.fader.reset();
        self.held.clear();
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{MidiMsg, NoResources, SignalType};
    use serde_json::json;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }

    #[test]
    fn config_sets_count_and_type() {
        let sig = Select::describe(&json!({}));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(names, ["in_0", "in_1", "in_2", "in_3", "index"]);
        assert_eq!(sig.outputs[OUT].ty, SignalType::Float);

        for (ty, want) in [
            ("audio", SignalType::Audio),
            ("int", SignalType::Int),
            ("bool", SignalType::Bool),
            ("trigger", SignalType::Trigger),
            ("midi", SignalType::Midi),
            ("bogus", SignalType::Float),
        ] {
            let sig = Select::describe(&json!({ "type": ty, "inputs": 2 }));
            assert_eq!(sig.inputs.len(), 3, "{ty}");
            assert_eq!(sig.inputs[1].ty, want, "{ty}");
            assert_eq!(sig.outputs[OUT].ty, want, "{ty}");
        }
        assert_eq!(Select::describe(&json!({ "inputs": 0 })).inputs.len(), 2);
        assert_eq!(Select::describe(&json!({ "inputs": 500 })).inputs.len(), 65);
    }

    #[test]
    fn index_picks_and_wraps() {
        for (index, want) in [
            (0, 10.0),
            (2, 12.0),
            (3, 13.0),
            (4, 10.0),
            (-1, 13.0),
            (9, 11.0),
        ] {
            let mut h = Harness::new::<Select>(json!({}));
            for i in 0..4 {
                h.set(&format!("in_{i}"), 10.0 + i as f32);
            }
            h.set("index", index);
            assert_eq!(h.run(1).float("out")[0], want, "index {index}");
        }
    }

    #[test]
    fn float_output_follows_the_inputs_rate() {
        let ramp: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let mut h = Harness::new::<Select>(json!({}));
        h.set_samples("in_1", ramp.clone()).set_int("index", 1);
        let out = h.run(128);
        assert_eq!(out.float("out")[77], 77.0);

        let mut h = Harness::new::<Select>(json!({}));
        h.set_block("in_1", ramp).set_int("index", 1);
        let out = h.run(128);
        assert_eq!(out.float("out")[77], 64.0);
    }

    #[test]
    fn bool_int_and_trigger_types() {
        let mut h = Harness::new::<Select>(json!({ "type": "bool", "inputs": 2 }));
        h.set_samples("in_1", vec![0.0, 0.7, f32::NAN, 1.0])
            .set("index", 1);
        assert_eq!(h.run(4).float("out"), vec![0.0, 1.0, 0.0, 1.0]);

        let mut h = Harness::new::<Select>(json!({ "type": "int", "inputs": 3 }));
        h.set_int("in_2", 42).set("in_0", 7).set("index", 2);
        assert_eq!(h.run(64).int("out")[0], 42);

        let mut h = Harness::new::<Select>(json!({ "type": "trigger", "inputs": 2 }));
        h.triggers("in_0", &[1, 2])
            .triggers("in_1", &[5, 70])
            .set("index", 1);
        assert_eq!(h.run(128).triggers("out"), &[5, 70]);
    }

    #[test]
    fn audio_crossfades_over_five_ms() {
        let n = 1_024;
        let mut h = Harness::new::<Select>(json!({ "type": "audio", "inputs": 2 }));
        let index: Vec<f32> = (0..n).map(|i| if i < 256 { 0.0 } else { 1.0 }).collect();
        h.set_audio("in_0", vec![vec![1.0; n]])
            .set_audio("in_1", vec![vec![-1.0; n]])
            .set_block("index", index);
        let o = h.run(n).audio("out")[0].clone();
        assert!(o[..256].iter().all(|&v| v == 1.0));
        // 5 ms is 240 samples: halfway through, the two cancel.
        assert!(o[256 + 119].abs() < 0.01, "{}", o[256 + 119]);
        assert!(o[256..256 + 240].windows(2).all(|w| w[1] < w[0]));
        assert!(o[256 + 240..].iter().all(|&v| v == -1.0));
    }

    #[test]
    fn audio_follows_the_widest_input_and_starts_without_a_fade() {
        let mut h = Harness::new::<Select>(json!({ "type": "audio", "inputs": 2 }));
        h.set_audio("in_0", vec![vec![0.5; 64], vec![-0.5; 64]])
            .set_audio("in_1", vec![vec![0.25; 64]])
            .set("index", 1);
        let out = h.run(64);
        let o = out.audio("out");
        assert_eq!(o.len(), 2);
        assert_eq!((o[0][0], o[1][0]), (0.25, 0.25), "mono upmixed, no fade-in");
    }

    #[test]
    fn audio_crossfade_is_the_same_at_any_block_size() {
        let n = 2_000;
        let a: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin()).collect();
        let b: Vec<f32> = (0..n).map(|i| (i as f32 * 0.21).cos()).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Select>(
                json!({ "type": "audio", "inputs": 3 }),
                48_000.0,
                block,
                &NoResources,
            );
            // Index changes on a 64-frame boundary so every block size sees it there.
            let index: Vec<f32> = (0..n).map(|i| if i < 512 { 0.0 } else { 2.0 }).collect();
            h.set_audio("in_0", vec![a.clone()])
                .set_audio("in_2", vec![b.clone()])
                .set_block("index", index);
            h.run(n).audio("out")[0].clone()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn midi_switch_releases_held_notes() {
        let mut h = Harness::new::<Select>(json!({ "type": "midi", "inputs": 2 }));
        let index: Vec<f32> = (0..128).map(|i| if i < 64 { 0.0 } else { 1.0 }).collect();
        h.midi(
            "in_0",
            &[
                (10, on(60)),
                (20, on(64)),
                (
                    30,
                    MidiMsg::NoteOff {
                        channel: 0,
                        note: 64,
                        velocity: 0,
                    },
                ),
            ],
        )
        .midi("in_1", &[(5, on(70)), (80, on(72))])
        .set_block("index", index);
        let out = h.run(128);
        let ev = out.midi("out");
        assert_eq!(ev[0], (10, on(60)));
        assert_eq!(ev[1], (20, on(64)));
        assert_eq!(
            ev[3],
            (
                64,
                MidiMsg::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 0
                }
            )
        );
        assert_eq!(ev[4], (80, on(72)));
        assert_eq!(ev.len(), 5);
    }

    #[test]
    fn reset_starts_on_the_current_input_without_a_fade() {
        let mut h = Harness::new::<Select>(json!({ "type": "audio", "inputs": 2 }));
        h.set_audio("in_0", vec![vec![1.0; 256]])
            .set_audio("in_1", vec![vec![-1.0; 256]])
            .set("index", 0);
        h.run(64);
        h.reset();
        h.set("index", 1);
        assert_eq!(h.run(64).audio("out")[0][0], -1.0);
    }
}
