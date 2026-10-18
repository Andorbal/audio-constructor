use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use super::finite_or_zero;
use super::route::{Fader, HeldNotes, RouteType};

node! {
    kind: "switch",
    name: "Switch",
    category: Math,
    keywords: ["toggle", "choose", "either", "if", "router", "bypass", "ab"],
    /// Passes on `a` while `select` is false and `b` while it is true.
    ///
    /// The `type` config sets what `a`, `b`, and `out` carry: `float` (the
    /// default), `audio`, `int`, `bool`, `trigger`, or `midi`. `select` is
    /// a gate read per sample, so a sample-rate gate switches on its exact
    /// sample.
    ///
    /// - `float` and `bool`: `out` runs at the fastest rate among the
    ///   inputs; NaN or infinite values come out as 0.
    /// - `audio`: flipping `select` crossfades over 5 ms so the switch
    ///   doesn't click. `out` has as many channels as the wider input.
    /// - `int`: reads `select` once per block.
    /// - `trigger`: each trigger passes if it arrives from the input
    ///   `select` picks at that sample.
    /// - `midi`: events pass the same way, and flipping `select` releases
    ///   any notes (and sustain pedals) still held from the other input.
    ///
    /// **Try this:** wire a `Toggle` into `select` with `type` `audio` for a
    /// click-free A/B comparison of a dry and an effected signal, or switch
    /// an `Oscillator` frequency between two `Constant`s with a `Clock` gate
    /// for a two-note trill.
    pub struct Switch {
        ty: RouteType,
        fader: Fader,
        held: HeldNotes,
        /// The side picked at the end of the last block (MIDI tracking).
        last: Option<bool>,
    }
}

const A: usize = 0;
const B: usize = 1;
const SELECT: usize = 2;
const OUT: usize = 0;

impl Node for Switch {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let ty = RouteType::from_config(config);
        Signature::new()
            .input(
                ty.input("a".to_string())
                    .doc("Passed on while `select` is false."),
            )
            .input(
                ty.input("b".to_string())
                    .doc("Passed on while `select` is true."),
            )
            .input(SocketSpec::boolean("select", false).doc("False picks `a`, true picks `b`."))
            .output(ty.output().doc("The picked input."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let ty = RouteType::from_config(config);
        Switch {
            ty,
            fader: Fader::new(2, ctx.sample_rate),
            held: HeldNotes::default(),
            last: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let sel = io.inputs.boolean(SELECT);
        let frames = ctx.frames;
        match self.ty {
            RouteType::Float => {
                let (a, b) = (io.inputs.float(A), io.inputs.float(B));
                io.outputs
                    .float(OUT)
                    .fill_with(|i| finite_or_zero(if sel.at(i) { b.at(i) } else { a.at(i) }));
            }
            RouteType::Bool => {
                let (a, b) = (io.inputs.boolean(A), io.inputs.boolean(B));
                io.outputs.float(OUT).fill_with(|i| {
                    let v = if sel.at(i) { b.at(i) } else { a.at(i) };
                    if v { 1.0 } else { 0.0 }
                });
            }
            RouteType::Int => {
                let v = io.inputs.int(if sel.first() { B } else { A });
                io.outputs.set_int(OUT, v);
            }
            RouteType::Audio => {
                let ins = io.inputs;
                let mut out = io.outputs.audio(OUT);
                self.fader.render(
                    &mut out,
                    frames,
                    |j| ins.audio(j),
                    |i| usize::from(sel.at(i)),
                    sel.is_varying(),
                );
            }
            RouteType::Trigger => {
                let (a, b) = (io.inputs.triggers(A), io.inputs.triggers(B));
                let mut out = io.outputs.trigger(OUT);
                for &at in a.iter().filter(|&&at| !sel.at(at as usize)) {
                    out.push(at);
                }
                for &at in b.iter().filter(|&&at| sel.at(at as usize)) {
                    out.push(at);
                }
            }
            RouteType::Midi => {
                let (a, b) = (io.inputs.midi(A), io.inputs.midi(B));
                let mut out = io.outputs.midi(OUT);
                let mut cur = self.last.unwrap_or_else(|| sel.first());
                let (mut ia, mut ib) = (0, 0);
                for i in 0..frames {
                    // Only a varying gate can flip inside the block.
                    if i == 0 || sel.is_varying() {
                        let s = sel.at(i);
                        if s != cur {
                            self.held.release_all(&mut out, i as u32);
                            cur = s;
                        }
                    }
                    while let Some(&ev) = a.get(ia).filter(|e| e.offset as usize <= i) {
                        ia += 1;
                        if !cur {
                            self.held.forward(&mut out, ev);
                        }
                    }
                    while let Some(&ev) = b.get(ib).filter(|e| e.offset as usize <= i) {
                        ib += 1;
                        if cur {
                            self.held.forward(&mut out, ev);
                        }
                    }
                    if !sel.is_varying() && ia >= a.len() && ib >= b.len() {
                        break;
                    }
                }
                // Events stamped past the block (malformed) still pass.
                for &ev in &a[ia.min(a.len())..] {
                    if !cur {
                        self.held.forward(&mut out, ev);
                    }
                }
                for &ev in &b[ib.min(b.len())..] {
                    if cur {
                        self.held.forward(&mut out, ev);
                    }
                }
                self.last = Some(cur);
            }
        }
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

    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

    fn gate(from: usize, to: usize, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if (from..to).contains(&i) { 1.0 } else { 0.0 })
            .collect()
    }

    #[test]
    fn type_config_sets_the_sockets() {
        for (ty, want) in [
            ("float", SignalType::Float),
            ("audio", SignalType::Audio),
            ("int", SignalType::Int),
            ("bool", SignalType::Bool),
            ("trigger", SignalType::Trigger),
            ("midi", SignalType::Midi),
        ] {
            let sig = Switch::describe(&json!({ "type": ty }));
            assert_eq!(sig.inputs[A].ty, want);
            assert_eq!(sig.inputs[B].ty, want);
            assert_eq!(sig.inputs[SELECT].ty, SignalType::Bool);
            assert_eq!(sig.outputs[OUT].ty, want);
        }
    }

    #[test]
    fn float_switches_on_the_exact_sample() {
        let mut h = Harness::new::<Switch>(json!({}));
        h.set("a", 1.0)
            .set("b", 2.0)
            .set_samples("select", gate(10, 20, 64));
        let o = h.run(64).float("out").to_vec();
        assert!(o[..10].iter().all(|&v| v == 1.0));
        assert!(o[10..20].iter().all(|&v| v == 2.0));
        assert!(o[20..].iter().all(|&v| v == 1.0));
    }

    #[test]
    fn controls_give_block_rate_output() {
        let mut h = Harness::new::<Switch>(json!({}));
        h.set("a", 1.0)
            .set("b", 2.0)
            .set_block("select", gate(64, 128, 128));
        let o = h.run(128).float("out").to_vec();
        assert_eq!(o[63], 1.0);
        assert_eq!(o[64], 2.0);
    }

    #[test]
    fn bool_and_int_types() {
        let mut h = Harness::new::<Switch>(json!({ "type": "bool" }));
        h.set("a", true).set_samples("b", vec![0.0, 1.0, 0.0, 1.0]);
        h.set_samples("select", vec![0.0, 0.0, 1.0, 1.0]);
        assert_eq!(h.run(4).float("out"), vec![1.0, 1.0, 0.0, 1.0]);

        let mut h = Harness::new::<Switch>(json!({ "type": "int" }));
        h.set_int("a", 3).set_int("b", 9).set("select", true);
        assert_eq!(h.run(64).int("out")[0], 9);
    }

    #[test]
    fn triggers_pass_from_the_side_picked_at_their_sample() {
        let mut h = Harness::new::<Switch>(json!({ "type": "trigger" }));
        h.triggers("a", &[5, 15, 25])
            .triggers("b", &[6, 16, 26])
            .set_samples("select", gate(10, 20, 64));
        assert_eq!(h.run(64).triggers("out"), &[5, 16, 25]);
    }

    #[test]
    fn audio_crossfades_both_ways() {
        let n = 1_200;
        let mut h = Harness::new::<Switch>(json!({ "type": "audio" }));
        h.set_audio("a", vec![vec![1.0; n]])
            .set_audio("b", vec![vec![0.0; n]])
            .set_samples("select", gate(100, 600, n));
        let o = h.run(n).audio("out")[0].clone();
        assert_eq!(o[99], 1.0);
        assert!((o[100 + 119] - 0.5).abs() < 0.01);
        assert_eq!(o[100 + 240], 0.0);
        assert!((o[600 + 119] - 0.5).abs() < 0.01);
        assert_eq!(o[600 + 240], 1.0);
        // Click-free: no step bigger than one fade increment.
        let max_step = o
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(max_step <= 1.0 / 240.0 + 1e-6, "{max_step}");
    }

    #[test]
    fn audio_is_the_same_at_any_block_size() {
        let n = 3_000;
        let a: Vec<f32> = (0..n).map(|i| (i as f32 * 0.03).sin()).collect();
        let b: Vec<f32> = (0..n).map(|i| (i as f32 * 0.17).sin() * 0.5).collect();
        let sel: Vec<f32> = (0..n).map(|i| ((i / 333) % 2) as f32).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Switch>(
                json!({ "type": "audio" }),
                48_000.0,
                block,
                &NoResources,
            );
            h.set_audio("a", vec![a.clone(), a.clone()])
                .set_audio("b", vec![b.clone()])
                .set_samples("select", sel.clone());
            h.run(n).audio("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one.len(), 2);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn midi_flips_release_held_notes() {
        let mut h = Harness::new::<Switch>(json!({ "type": "midi" }));
        h.midi("a", &[(2, on(60)), (12, off(60)), (14, on(62))])
            .midi("b", &[(3, on(70)), (11, on(71))])
            .set_samples("select", gate(10, 13, 64));
        let out = h.run(64);
        assert_eq!(
            out.midi("out"),
            &[
                (2, on(60)),
                (10, off(60)),
                (11, on(71)),
                (13, off(71)),
                (14, on(62)),
            ]
        );
    }

    #[test]
    fn midi_is_the_same_at_any_block_size() {
        let a: Vec<(u64, MidiMsg)> = (0..40).map(|k| (k * 25, on(40 + k as u8))).collect();
        let b: Vec<(u64, MidiMsg)> = (0..40).map(|k| (k * 25 + 7, on(80 + k as u8))).collect();
        let sel: Vec<f32> = (0..1_000).map(|i| ((i / 90) % 2) as f32).collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Switch>(
                json!({ "type": "midi" }),
                48_000.0,
                block,
                &NoResources,
            );
            h.midi("a", &a)
                .midi("b", &b)
                .set_samples("select", sel.clone());
            h.run(1_000).midi("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
