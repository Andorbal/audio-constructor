use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::finite_or_zero;
use super::routing::{FADE_SECONDS, HeldNotes, MAX_ROUTES, RouteType};

node! {
    kind: "route",
    name: "Route",
    category: Math,
    keywords: [
        "demux", "demultiplexer", "send", "distribute", "gate", "round robin", "layer",
        "split", "router", "pass"
    ],
    /// Sends its input to one of several outputs, picked by `index`: the
    /// mirror of Select. Or, in gate mode, passes it while `open` is on.
    ///
    /// The `type` config sets what it carries: `trigger` (the default),
    /// `midi`, `audio`, `buffer`, `float`, `bool`, or `int`. The `outputs`
    /// config sets how many outputs there are (1 to 64, default 4), named
    /// `out_0`, `out_1`, ….
    ///
    /// `index` picks the output, counting from 0, and wraps around like
    /// Select's: with four outputs, 4 is `out_0` again and −1 is `out_3`,
    /// so a `Counter` steps through them for round robin. It's an `Int`, so
    /// the choice changes at block boundaries. The other outputs carry
    /// nothing: no triggers or events, silence, no sound, or 0.
    ///
    /// - `audio`: a new `index` fades the old output out and the new one in
    ///   over 5 ms, so it doesn't click.
    /// - `midi`: a new `index` releases the notes (and sustain pedals) still
    ///   held on the old output, so nothing hangs there.
    ///
    /// **Gate mode** (`mode` config `gate`): one output, `out`, which passes
    /// `in` while `open` is on, on its exact sample: triggers and MIDI
    /// events that arrive while it's open, audio faded in and out over
    /// 5 ms, the sound while open at the start of a block, and values
    /// (else 0). Closing releases held notes.
    ///
    /// **Try this:** wire a pad's trigger into `in` and a `Counter` of the
    /// same trigger into `index`, then `out_0` and `out_1` into two
    /// Samplers of the same snare: round robin, so a roll never sounds
    /// like a machine gun. Or give `index` a `Zones` node's `zone` to send
    /// each hit to the sample for its key or velocity.
    pub struct Route {
        ty: RouteType,
        outputs: usize,
        gate: bool,
        /// Per output: the fade gain (audio) and the notes held (MIDI).
        gains: Vec<f32>,
        held: Vec<HeldNotes>,
        step: f32,
        /// The output picked (or whether open) last block.
        last: Option<usize>,
    }
}

const IN: usize = 0;
const CONTROL: usize = 1;

fn output_count(config: &Config) -> usize {
    config.get_u64_or("outputs", 4).clamp(1, MAX_ROUTES as u64) as usize
}

fn is_gate(config: &Config) -> bool {
    config.get_str_or("mode", "index") == "gate"
}

fn route_type(config: &Config) -> RouteType {
    match config.get_str_or("type", "trigger") {
        "trigger" => RouteType::Trigger,
        _ => RouteType::from_config(config),
    }
}

impl Route {
    /// Outputs that carry anything this block (one, or none when shut).
    fn outs(&self) -> usize {
        if self.gate { 1 } else { self.outputs }
    }
}

impl Node for Route {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let ty = route_type(config);
        let mut sig = Signature::new().input(ty.input("in".to_string()).doc("What to send on."));
        if is_gate(config) {
            sig = sig
                .input(SocketSpec::boolean("open", true).doc("Passes `in` while on."))
                .output(ty.output().doc("`in` while `open` is on."));
        } else {
            let n = output_count(config);
            sig = sig.input(
                SocketSpec::int("index", 0)
                    .range(0.0, (n - 1) as f64)
                    .doc("Which output gets `in` (wraps around)."),
            );
            for k in 0..n {
                let mut out = ty.input(format!("out_{k}"));
                out.rate = ty.output().rate;
                sig = sig.output(out.doc("`in`, while `index` picks it."));
            }
        }
        sig
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let gate = is_gate(config);
        let outputs = if gate { 1 } else { output_count(config) };
        let samples = (FADE_SECONDS * ctx.sample_rate).max(1.0);
        Route {
            ty: route_type(config),
            outputs,
            gate,
            gains: vec![0.0; outputs],
            held: vec![HeldNotes::default(); outputs],
            step: (1.0 / samples) as f32,
            last: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let n = self.outs();
        let open = io.inputs.boolean(CONTROL);
        // The output picked, or whether the gate is open, at frame `i`.
        let picked = if self.gate {
            None
        } else {
            Some(i64::from(io.inputs.int(CONTROL)).rem_euclid(n as i64) as usize)
        };
        let on = |k: usize, i: usize| match picked {
            Some(p) => p == k,
            None => open.at(i),
        };
        match self.ty {
            RouteType::Trigger => {
                let events = io.inputs.triggers(IN);
                for k in 0..n {
                    let mut out = io.outputs.trigger(k);
                    for &at in events.iter().filter(|&&at| on(k, at as usize)) {
                        out.push(at);
                    }
                }
            }
            RouteType::Midi => {
                let events = io.inputs.midi(IN);
                for k in 0..n {
                    let mut out = io.outputs.midi(k);
                    let held = &mut self.held[k];
                    let mut was = match (self.last, picked) {
                        (Some(l), Some(_)) => l == k,
                        (Some(l), None) => l == 1,
                        (None, _) => on(k, 0),
                    };
                    let mut next = 0;
                    for i in 0..frames {
                        let now = on(k, i);
                        if was && !now {
                            held.release_all(&mut out, i as u32);
                        }
                        was = now;
                        while let Some(&ev) = events.get(next).filter(|e| e.offset as usize <= i) {
                            next += 1;
                            if now {
                                held.forward(&mut out, ev);
                            }
                        }
                        if picked.is_some() && next >= events.len() && i > 0 {
                            break;
                        }
                    }
                }
            }
            RouteType::Audio => {
                let input = io.inputs.audio(IN);
                let primed = self.last.is_some();
                for k in 0..n {
                    let mut out = io.outputs.audio(k);
                    let mut g = if primed {
                        self.gains[k]
                    } else if on(k, 0) {
                        1.0
                    } else {
                        0.0
                    };
                    let start = g;
                    for c in 0..out.channels() {
                        g = start;
                        let src = input.channel(c);
                        for (i, o) in out.channel(c).iter_mut().enumerate() {
                            g = if on(k, i) {
                                (g + self.step).min(1.0)
                            } else {
                                (g - self.step).max(0.0)
                            };
                            let s = src.get(i).copied().unwrap_or(0.0);
                            *o = if g > 0.0 { finite_or_zero(s * g) } else { 0.0 };
                        }
                    }
                    if out.channels() == 0 {
                        for i in 0..frames {
                            g = if on(k, i) {
                                (g + self.step).min(1.0)
                            } else {
                                (g - self.step).max(0.0)
                            };
                        }
                    }
                    self.gains[k] = g;
                }
            }
            RouteType::Buffer => {
                let b = io.inputs.buffer(IN);
                for k in 0..n {
                    io.outputs.set_buffer(k, b.filter(|_| on(k, 0)));
                }
            }
            RouteType::Int => {
                let v = io.inputs.int(IN);
                for k in 0..n {
                    io.outputs.set_int(k, if on(k, 0) { v } else { 0 });
                }
            }
            RouteType::Float | RouteType::Bool => {
                let x = io.inputs.float(IN);
                let bool = self.ty == RouteType::Bool;
                for k in 0..n {
                    io.outputs.float(k).fill_with(|i| {
                        if !on(k, i) {
                            0.0
                        } else if bool {
                            if x.at(i) > 0.5 { 1.0 } else { 0.0 }
                        } else {
                            finite_or_zero(x.at(i))
                        }
                    });
                }
            }
        }
        self.last = match picked {
            Some(p) => Some(p),
            None => Some(usize::from(open.at(frames.saturating_sub(1)))),
        };
    }

    fn reset(&mut self) {
        self.gains.fill(0.0);
        for h in &mut self.held {
            h.clear();
        }
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{MidiMsg, SignalType};
    use serde_json::json;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }

    #[test]
    fn config_sets_type_outputs_and_mode() {
        let sig = Route::describe(&json!({}));
        let ins: Vec<_> = sig.inputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(ins, ["in", "index"]);
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(outs, ["out_0", "out_1", "out_2", "out_3"]);
        assert_eq!(sig.inputs[IN].ty, SignalType::Trigger);
        for (ty, want) in [
            ("midi", SignalType::Midi),
            ("audio", SignalType::Audio),
            ("buffer", SignalType::Buffer),
            ("float", SignalType::Float),
            ("bogus", SignalType::Float),
        ] {
            let sig = Route::describe(&json!({ "type": ty, "outputs": 2 }));
            assert_eq!(sig.outputs.len(), 2, "{ty}");
            assert_eq!(sig.outputs[1].ty, want, "{ty}");
        }
        let gate = Route::describe(&json!({ "mode": "gate" }));
        let ins: Vec<_> = gate.inputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(ins, ["in", "open"]);
        assert_eq!(gate.outputs.len(), 1);
        assert_eq!(
            Route::describe(&json!({ "outputs": 500 })).outputs.len(),
            64
        );
    }

    #[test]
    fn triggers_go_to_the_picked_output_and_index_wraps() {
        let mut h = Harness::new::<Route>(json!({ "outputs": 3 }));
        h.triggers("in", &[4, 9]).set("index", 4);
        let out = h.run(64);
        assert_eq!(out.triggers("out_1"), &[4, 9]);
        assert!(out.triggers("out_0").is_empty() && out.triggers("out_2").is_empty());
    }

    #[test]
    fn a_new_index_releases_the_notes_left_on_the_old_output() {
        let mut h = Harness::new::<Route>(json!({ "type": "midi", "outputs": 2 }));
        h.midi("in", &[(3, on(60))]);
        let out = h.run(64);
        assert_eq!(out.midi("out_0"), &[(3, on(60))]);
        h.set("index", 1);
        let out = h.run(64);
        assert!(matches!(
            out.midi("out_0"),
            [(64, MidiMsg::NoteOff { note: 60, .. })]
        ));
        assert!(out.midi("out_1").is_empty());
    }

    #[test]
    fn audio_fades_between_outputs() {
        let n = 1_024;
        let mut h = Harness::new::<Route>(json!({ "type": "audio", "outputs": 2 }));
        h.set_audio("in", vec![vec![1.0; n]]);
        let out = h.run(512);
        assert!(out.audio("out_0")[0].iter().all(|&s| s == 1.0));
        assert!(out.audio("out_1")[0].iter().all(|&s| s == 0.0));
        h.set("index", 1);
        let out = h.run(512);
        let a = &out.audio("out_0")[0];
        let b = &out.audio("out_1")[0];
        // 5 ms at 48 kHz is 240 samples.
        assert!(a[0] < 1.0 && a[0] > 0.9 && b[0] > 0.0 && b[0] < 0.1);
        assert_eq!(a[300], 0.0);
        assert_eq!(b[300], 1.0);
    }

    #[test]
    fn gate_mode_passes_while_open() {
        let mut h = Harness::new::<Route>(json!({ "mode": "gate" }));
        let open: Vec<f32> = (0..64).map(|i| if i < 32 { 1.0 } else { 0.0 }).collect();
        h.triggers("in", &[10, 40]).set_samples("open", open);
        assert_eq!(h.run(64).triggers("out"), &[10]);

        let mut h = Harness::new::<Route>(json!({ "mode": "gate", "type": "midi" }));
        let open: Vec<f32> = (0..64).map(|i| if i < 20 { 1.0 } else { 0.0 }).collect();
        h.midi("in", &[(5, on(62)), (30, on(64))])
            .set_samples("open", open);
        let out = h.run(64);
        let notes: Vec<(u64, bool)> = out
            .midi("out")
            .iter()
            .map(|(t, m)| (*t, matches!(m, MidiMsg::NoteOn { .. })))
            .collect();
        assert_eq!(notes, vec![(5, true), (20, false)]);
    }

    #[test]
    fn buffers_and_values_reach_only_the_picked_output() {
        let sound = std::sync::Arc::new(ac_core::AudioData {
            sample_rate: 48_000,
            channels: vec![vec![0.5; 8]],
        });
        let mut h = Harness::new::<Route>(json!({ "type": "buffer", "outputs": 2 }));
        h.buffer("in", sound).set("index", 1);
        let out = h.run(64);
        assert!(out.buffer("out_0").is_none());
        assert!(out.buffer("out_1").is_some());

        let mut h = Harness::new::<Route>(json!({ "type": "float", "outputs": 2 }));
        h.set("in", 7.0).set("index", 0);
        let out = h.run(8);
        assert_eq!(out.float("out_0")[0], 7.0);
        assert_eq!(out.float("out_1")[0], 0.0);
    }
}
