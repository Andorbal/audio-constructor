use ac_core::plugin::{PLUGIN_EVENT_CAPACITY, PLUGIN_PARAM_STRIDE};
use ac_core::{
    Channels, Config, ConfigExt, Io, MidiEvent, MidiSink, Node, NodeMeta, PluginBlock, PluginDesc,
    PluginEvent, PluginProcessor, PluginRequest, PluginTransport, PrepareCtx, ProcessCtx,
    Signature, SocketSpec, Unit, node,
};

use super::copy_into;

node! {
    kind: "plugin",
    name: "Plugin",
    category: Utility,
    keywords: ["clap", "vst", "vst3", "instrument", "effect", "synth", "third party", "host"],
    /// Runs a CLAP or VST3 plugin installed on this computer as a node: a
    /// synth played from MIDI, or an effect on audio.
    ///
    /// Insert one from the add-node palette, where every plugin the editor
    /// found is listed by name (rescan in the Devices tab after installing
    /// more). The node's sockets come from the plugin: its audio inputs and
    /// outputs (`in`, `out`, then `in_2`, `out_2`, … for extra ports, such
    /// as a sidechain), `midi` in and `midi_out` when it takes or sends
    /// notes, and a `Float` input for each of its parameters (up to 128),
    /// named after the parameter (a CLAP plugin's in its own range and
    /// units, a VST3 plugin's from 0 to 1). A parameter input works like
    /// any other:
    /// set it inline, or wire an LFO or an envelope into it and the plugin
    /// follows sample-accurately (a moving input sends a change every 32
    /// samples). The node reports the plugin's latency, so the engine lines
    /// up the dry paths around it.
    ///
    /// **Show Plugin Window** in the node's menu opens the plugin's own
    /// interface in a window of its own. Changes made there move the
    /// node's inputs, and saving the project saves everything the plugin
    /// keeps (its state goes in the project).
    ///
    /// A project keeps working on a computer without the plugin: the node
    /// keeps its sockets and wires, plays silence, and says what's
    /// missing (in the Problems tab and `audioc validate`).
    ///
    /// **Try this:** add a synth plugin, wire a `Keyboard` (or `MIDI In`)
    /// into its `midi` and its `out` to `Audio Out`; then wire an `LFO`
    /// into its filter cutoff, or send its `out` through a second plugin
    /// for effects.
    pub struct Plugin {
        processor: Option<Box<dyn PluginProcessor>>,
        /// Why there's no processor (shown as a compile warning).
        problem: Option<String>,
        audio_in: Vec<Vec<Vec<f32>>>,
        audio_out: Vec<Vec<Vec<f32>>>,
        midi_in: Option<usize>,
        midi_out: Option<usize>,
        /// Index of the first parameter input.
        param_base: usize,
        /// The value last sent to each parameter (NaN: never).
        sent: Vec<f32>,
        /// Parameter inputs that change within this block.
        moving: Vec<usize>,
        params: Vec<PluginEvent>,
        events: Vec<PluginEvent>,
        out_events: Vec<MidiEvent>,
        /// Frames processed since the node started or was reset.
        position: u64,
        failed: bool,
    }
}

/// Telemetry: 0 running, 1 no plugin (missing or failed to start), 2 the
/// plugin failed the last block.
pub const TELEMETRY_STATUS: usize = 0;

fn port_channels(desc: &PluginDesc, input: bool) -> Vec<usize> {
    let ports = if input {
        &desc.audio_in
    } else {
        &desc.audio_out
    };
    ports.iter().map(|p| p.channels as usize).collect()
}

impl Node for Plugin {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1).state_keys(&["state"])
    }

    fn describe(config: &Config) -> Signature {
        let Some(desc) = PluginDesc::from_config(config) else {
            return Signature::new();
        };
        let mut sig = Signature::new();
        for (i, p) in desc.audio_in.iter().enumerate() {
            sig = sig.input(
                SocketSpec::audio(PluginDesc::port_socket("in", i))
                    .channels(Channels::Fixed(p.channels))
                    .doc("Audio into the plugin."),
            );
        }
        if desc.midi_in {
            sig = sig.input(SocketSpec::midi("midi").doc("Notes and MIDI for the plugin."));
        }
        for p in &desc.params {
            let mut s = SocketSpec::float(p.socket.clone(), Unit::Generic, p.default)
                .range(p.min, p.max)
                .no_smooth()
                .doc("A parameter of the plugin.");
            if p.stepped {
                s = s.doc("A parameter of the plugin (whole numbers).");
            }
            sig = sig.input(s);
        }
        for (i, p) in desc.audio_out.iter().enumerate() {
            sig = sig.output(
                SocketSpec::audio(PluginDesc::port_socket("out", i))
                    .channels(Channels::Fixed(p.channels))
                    .doc("Audio from the plugin."),
            );
        }
        if desc.midi_out {
            sig = sig.output(SocketSpec::midi("midi_out").doc("Notes and MIDI the plugin sends."));
        }
        sig
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let desc = PluginDesc::from_config(config);
        let block = ctx.max_block.max(1);
        let buffers = |chans: Vec<usize>| -> Vec<Vec<Vec<f32>>> {
            chans
                .into_iter()
                .map(|c| (0..c).map(|_| vec![0.0; block]).collect())
                .collect()
        };
        let (audio_in, audio_out, midi_in, midi_out, params) = match &desc {
            Some(d) => (
                buffers(port_channels(d, true)),
                buffers(port_channels(d, false)),
                d.midi_in,
                d.midi_out,
                d.params.len(),
            ),
            None => (Vec::new(), Vec::new(), false, false, 0),
        };
        let midi_in = midi_in.then_some(audio_in.len());
        let param_base = audio_in.len() + usize::from(midi_in.is_some());
        let midi_out = midi_out.then_some(audio_out.len());
        let (processor, problem) = match &desc {
            None => (None, Some("no plugin chosen".to_string())),
            Some(d) => {
                let req = PluginRequest {
                    node_key: ctx.node_key,
                    name: ctx.name,
                    desc: d,
                    state: config.get_str_or("state", ""),
                    sample_rate: ctx.sample_rate,
                    max_block: block,
                };
                match ctx.resources.plugin(&req) {
                    Ok(p) => (Some(p), None),
                    Err(why) => (
                        None,
                        Some(format!("{}: {why}; the node plays silence", d.label())),
                    ),
                }
            }
        };
        Plugin {
            processor,
            problem,
            audio_in,
            audio_out,
            midi_in,
            midi_out,
            param_base,
            sent: vec![f32::NAN; params],
            moving: Vec::with_capacity(params),
            params: Vec::with_capacity(PLUGIN_EVENT_CAPACITY),
            events: Vec::with_capacity(PLUGIN_EVENT_CAPACITY),
            out_events: Vec::with_capacity(ac_core::MIDI_CAPACITY),
            position: 0,
            failed: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let status = if self.processor.is_none() {
            1.0
        } else if self.failed {
            2.0
        } else {
            0.0
        };
        io.telemetry.set(TELEMETRY_STATUS, status);
        let Some(processor) = self.processor.as_mut() else {
            for i in 0..self.audio_out.len() {
                io.outputs.audio(i).fill(0.0);
            }
            return;
        };
        for (p, port) in self.audio_in.iter_mut().enumerate() {
            let input = io.inputs.audio(p);
            for (c, buf) in port.iter_mut().enumerate() {
                copy_into(&mut buf[..frames], input.channel(c));
            }
        }
        for port in &mut self.audio_out {
            for buf in port {
                buf[..frames].fill(0.0);
            }
        }
        param_events(
            io,
            self.param_base,
            &mut self.sent,
            &mut self.moving,
            &mut self.params,
            self.position,
            frames,
        );
        let midi: &[MidiEvent] = match self.midi_in {
            Some(i) => io.inputs.midi(i),
            None => &[],
        };
        merge(&self.params, midi, &mut self.events, frames);
        let transport = transport_at(ctx);
        let ok = {
            let mut block = PluginBlock {
                frames,
                audio_in: &mut self.audio_in,
                audio_out: &mut self.audio_out,
                events: &self.events,
                midi_out: MidiSink::new(&mut self.out_events),
                transport,
            };
            processor.process(&mut block)
        };
        self.failed = !ok;
        for (p, port) in self.audio_out.iter().enumerate() {
            let mut out = io.outputs.audio(p);
            for (c, buf) in port.iter().enumerate() {
                if c >= out.channels() {
                    break;
                }
                let dst = out.channel(c);
                if ok {
                    copy_into(dst, &buf[..frames]);
                } else {
                    dst.fill(0.0);
                }
            }
        }
        if let Some(i) = self.midi_out {
            let last = frames.saturating_sub(1) as u32;
            let mut out = io.outputs.midi(i);
            for ev in &self.out_events {
                out.push(MidiEvent::new(ev.offset.min(last), ev.msg));
            }
        }
        self.position += frames as u64;
    }

    fn latency(&self) -> u32 {
        self.processor.as_ref().map_or(0, |p| p.latency())
    }

    fn warning(&self) -> Option<String> {
        self.problem.clone()
    }

    fn reset(&mut self) {
        if let Some(p) = self.processor.as_mut() {
            p.reset();
        }
        self.position = 0;
        self.failed = false;
    }
}

/// Parameter changes for this block, in time order: an input that holds
/// still sends its value at the start of the block when it differs from
/// what the plugin last got; one that moves sends at every multiple of
/// [`PLUGIN_PARAM_STRIDE`] samples (counted from the node's start) where
/// it changed. Past the event capacity the rest wait for the next block.
fn param_events(
    io: &Io,
    base: usize,
    sent: &mut [f32],
    moving: &mut Vec<usize>,
    out: &mut Vec<PluginEvent>,
    position: u64,
    frames: usize,
) {
    out.clear();
    moving.clear();
    let room = out.capacity();
    for (k, last) in sent.iter_mut().enumerate() {
        let input = io.inputs.float(base + k);
        if input.is_varying() {
            moving.push(k);
            continue;
        }
        let v = input.first();
        if v.to_bits() != last.to_bits() && out.len() < room {
            out.push(PluginEvent::Param {
                offset: 0,
                index: k as u32,
                value: f64::from(v),
            });
            *last = v;
        }
    }
    if moving.is_empty() {
        return;
    }
    let stride = PLUGIN_PARAM_STRIDE;
    // The first multiple of the stride in this block.
    let mut at = ((stride - position % stride) % stride) as usize;
    while at < frames {
        for &k in moving.iter() {
            let v = io.inputs.float(base + k).at(at);
            if v.to_bits() != sent[k].to_bits() && out.len() < room {
                out.push(PluginEvent::Param {
                    offset: at as u32,
                    index: k as u32,
                    value: f64::from(v),
                });
                sent[k] = v;
            }
        }
        at += stride as usize;
    }
}

/// Merge parameter changes and MIDI (both in time order) into `out`,
/// parameters first on a shared sample, so a change and a note on the
/// same frame reach the plugin in that order.
fn merge(params: &[PluginEvent], midi: &[MidiEvent], out: &mut Vec<PluginEvent>, frames: usize) {
    out.clear();
    let room = out.capacity();
    let last = frames.saturating_sub(1) as u32;
    let (mut i, mut j) = (0, 0);
    while (i < params.len() || j < midi.len()) && out.len() < room {
        let take_param = match (params.get(i), midi.get(j)) {
            (Some(p), Some(m)) => p.offset() <= m.offset.min(last),
            (Some(_), None) => true,
            _ => false,
        };
        if take_param {
            out.push(params[i]);
            i += 1;
        } else {
            let m = midi[j];
            out.push(PluginEvent::Midi(MidiEvent::new(m.offset.min(last), m.msg)));
            j += 1;
        }
    }
}

fn transport_at(ctx: &ProcessCtx) -> PluginTransport {
    let t = &ctx.transport;
    let pos = t.position;
    let beats = t.tempo.beat_at(pos);
    let (bar, in_bar) = t.tempo.bar_at(pos);
    PluginTransport {
        playing: t.playing,
        recording: t.recording,
        sample: pos,
        seconds: t.tempo.seconds_at(pos),
        beats,
        bpm: t.tempo.bpm_at(pos),
        bar_start: beats - in_bar,
        bar,
        time_sig: t.tempo.time_sig_at(pos),
    }
}

/// A processor for tests: records what it's sent and plays a sine at the
/// frequency of the last note, scaled by parameter 0.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct Log {
        pub events: Vec<(u64, PluginEvent)>,
        pub resets: usize,
        pub transports: Vec<PluginTransport>,
    }

    pub struct Fake {
        pub log: Arc<Mutex<Log>>,
        pub position: u64,
        pub latency: u32,
        pub fail: bool,
        phase: f64,
        freq: f64,
        level: f64,
        rate: f64,
    }

    impl Fake {
        pub fn new(log: Arc<Mutex<Log>>, rate: f64) -> Fake {
            Fake {
                log,
                position: 0,
                latency: 64,
                fail: false,
                phase: 0.0,
                freq: 0.0,
                level: 1.0,
                rate,
            }
        }
    }

    impl PluginProcessor for Fake {
        fn process(&mut self, block: &mut PluginBlock<'_>) -> bool {
            let Ok(mut log) = self.log.lock() else {
                return false;
            };
            log.transports.push(block.transport);
            let mut next = 0;
            for i in 0..block.frames {
                while let Some(e) = block.events.get(next).filter(|e| e.offset() as usize == i) {
                    log.events.push((self.position + i as u64, *e));
                    match e {
                        PluginEvent::Midi(m) => match m.msg {
                            ac_core::MidiMsg::NoteOn { note, .. } => {
                                self.freq = f64::from(ac_core::note_to_hz(f32::from(note)));
                            }
                            ac_core::MidiMsg::NoteOff { .. } => self.freq = 0.0,
                            _ => {}
                        },
                        PluginEvent::Param {
                            index: 0, value, ..
                        } => self.level = *value,
                        PluginEvent::Param { .. } => {}
                    }
                    next += 1;
                }
                let v = if self.freq > 0.0 {
                    (self.phase * std::f64::consts::TAU).sin() * self.level
                } else {
                    0.0
                };
                self.phase = (self.phase + self.freq / self.rate).fract();
                for port in block.audio_out.iter_mut() {
                    for ch in port.iter_mut() {
                        ch[i] = v as f32;
                    }
                }
            }
            for port in block.audio_in.iter() {
                for (c, ch) in port.iter().enumerate() {
                    if let Some(out) = block.audio_out.first_mut().and_then(|p| p.get_mut(c)) {
                        for i in 0..block.frames {
                            out[i] += ch[i];
                        }
                    }
                }
            }
            if self.freq > 0.0 {
                block.midi_out.push(MidiEvent::new(
                    0,
                    ac_core::MidiMsg::ControlChange {
                        channel: 0,
                        controller: 1,
                        value: 64,
                    },
                ));
            }
            self.position += block.frames as u64;
            !self.fail
        }

        fn latency(&self) -> u32 {
            self.latency
        }

        fn reset(&mut self) {
            if let Ok(mut l) = self.log.lock() {
                l.resets += 1;
            }
            self.freq = 0.0;
        }
    }

    /// Resources whose plugins are [`Fake`]s sharing one log.
    pub struct FakeHost {
        pub log: Arc<Mutex<Log>>,
        pub fail: bool,
        pub installed: bool,
    }

    impl ac_core::Resources for FakeHost {
        fn plugin(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
            if !self.installed {
                return Err("isn't installed on this computer".into());
            }
            let mut f = Fake::new(self.log.clone(), req.sample_rate);
            f.fail = self.fail;
            Ok(Box::new(f))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeHost, Log};
    use super::*;
    use ac_core::testing::{Harness, rms, zero_crossing_freq};
    use ac_core::{MidiMsg, NoResources, SignalType};
    use serde_json::{Value as Json, json};
    use std::sync::{Arc, Mutex};

    fn desc() -> Json {
        json!({
            "format": "clap",
            "id": "org.example.synth",
            "name": "Synth",
            "vendor": "Example",
            "instrument": true,
            "audio_in": [{ "name": "Sidechain", "channels": 1 }],
            "audio_out": [{ "name": "Main", "channels": 2 }, { "name": "Aux", "channels": 1 }],
            "midi_in": true,
            "midi_out": true,
            "params": [
                { "id": 10, "name": "Level", "socket": "level", "min": 0.0, "max": 1.0, "default": 0.5 },
                { "id": 11, "name": "Cutoff", "socket": "cutoff", "min": 20.0, "max": 20000.0, "default": 1000.0 },
            ],
        })
    }

    fn host(installed: bool) -> (FakeHost, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        (
            FakeHost {
                log: log.clone(),
                fail: false,
                installed,
            },
            log,
        )
    }

    fn note_on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }

    #[test]
    fn sockets_come_from_the_description() {
        let sig = Plugin::describe(&json!({ "plugin": desc() }));
        let ins: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(ins, ["in", "midi", "level", "cutoff"]);
        assert_eq!(outs, ["out", "out_2", "midi_out"]);
        assert_eq!(sig.inputs[0].channels, Channels::Fixed(1));
        assert_eq!(sig.outputs[0].channels, Channels::Fixed(2));
        assert_eq!(sig.inputs[3].ty, SignalType::Float);
        assert_eq!(sig.inputs[3].range, Some((20.0, 20_000.0)));
        assert!(!sig.inputs[3].smooth);
        assert!(Plugin::describe(&json!({})).inputs.is_empty());
        assert!(
            Plugin::describe(&json!({ "plugin": "junk" }))
                .outputs
                .is_empty()
        );
    }

    #[test]
    fn notes_play_at_their_pitch_and_report_latency() {
        let (fake, _) = host(true);
        for block in [1usize, 64, 1000] {
            let mut h = Harness::with_resources::<Plugin>(
                json!({ "plugin": desc() }),
                48_000.0,
                block,
                &fake,
            );
            h.midi("midi", &[(100, note_on(69))]);
            let out = h.run(9600);
            let l = &out.audio("out")[0];
            assert_eq!(l[99], 0.0);
            let f = zero_crossing_freq(&l[200..], 48_000.0);
            assert!((f - 440.0).abs() < 2.0, "block {block}: {f} Hz");
            assert!(rms(&l[200..]) > 0.3);
            // The fake answers each block it sounds in with a CC at its start.
            let first = out.midi("midi_out").first().map(|e| e.0);
            assert_eq!(first, Some((100 / block * block) as u64));
        }
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 64, &fake);
        h.run(64);
        let node = Plugin::new(
            &json!({ "plugin": desc() }),
            &PrepareCtx {
                sample_rate: 48_000.0,
                max_block: 64,
                seed: 0,
                node_key: "1",
                name: "Synth",
                resources: &fake,
            },
        );
        assert_eq!(node.latency(), 64);
        assert_eq!(node.warning(), None);
    }

    #[test]
    fn parameters_are_sent_when_they_change_and_on_a_stride_when_they_move() {
        let (fake, log) = host(true);
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 100, &fake);
        h.set("level", 0.25);
        h.run(300);
        {
            let l = log.lock().unwrap();
            let params: Vec<_> = l
                .events
                .iter()
                .filter_map(|(at, e)| match e {
                    PluginEvent::Param { index, value, .. } => Some((*at, *index, *value)),
                    _ => None,
                })
                .collect();
            // Both parameters once, at the start; nothing again while still.
            assert_eq!(params, [(0, 0, 0.25), (0, 1, 1000.0)]);
        }
        // A sample-rate ramp: one change every 32 samples, at multiples of
        // 32 from the start, whatever the block size.
        let mut per_block = Vec::new();
        for block in [7usize, 64, 1000] {
            let (fake, log) = host(true);
            let mut h = Harness::with_resources::<Plugin>(
                json!({ "plugin": desc() }),
                48_000.0,
                block,
                &fake,
            );
            h.set_samples("cutoff", (0..256).map(|i| 100.0 + i as f32).collect());
            h.run(256);
            let l = log.lock().unwrap();
            let at: Vec<u64> = l
                .events
                .iter()
                .filter_map(|(at, e)| match e {
                    PluginEvent::Param { index: 1, .. } => Some(*at),
                    _ => None,
                })
                .collect();
            per_block.push(at);
        }
        assert_eq!(per_block[0], (0..8).map(|k| k * 32).collect::<Vec<u64>>());
        assert!(per_block.iter().all(|a| a == &per_block[0]));
    }

    #[test]
    fn a_change_and_a_note_on_one_frame_arrive_change_first() {
        let (fake, log) = host(true);
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 64, &fake);
        h.midi("midi", &[(0, note_on(60))]);
        h.run(64);
        let l = log.lock().unwrap();
        let kinds: Vec<bool> = l
            .events
            .iter()
            .map(|(_, e)| matches!(e, PluginEvent::Param { .. }))
            .collect();
        assert_eq!(kinds, [true, true, false]);
    }

    #[test]
    fn the_transport_reaches_the_plugin() {
        let (fake, log) = host(true);
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 24_000, &fake);
        h.tempo(ac_core::TempoMap::constant(48_000.0, 120.0));
        h.play();
        h.run(72_000);
        let l = log.lock().unwrap();
        let last = l.transports.last().unwrap();
        assert!(last.playing);
        assert_eq!(last.sample, 48_000);
        assert!((last.beats - 2.0).abs() < 1e-9);
        assert!((last.bpm - 120.0).abs() < 1e-9);
        assert_eq!(last.time_sig, [4, 4]);
    }

    #[test]
    fn audio_passes_into_the_plugin() {
        let (fake, _) = host(true);
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 64, &fake);
        h.set_audio("in", vec![vec![0.5; 256]]);
        let out = h.run(256);
        assert!(out.audio("out")[0].iter().all(|v| (*v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_missing_plugin_keeps_its_sockets_plays_silence_and_says_why() {
        for (res, why) in [
            (
                Box::new(host(false).0) as Box<dyn ac_core::Resources>,
                "isn't installed",
            ),
            (Box::new(NoResources), "don't run here"),
        ] {
            let mut h = Harness::with_resources::<Plugin>(
                json!({ "plugin": desc() }),
                48_000.0,
                64,
                res.as_ref(),
            );
            assert_eq!(h.signature().outputs.len(), 3);
            h.midi("midi", &[(0, note_on(60))]);
            let out = h.run(256);
            assert!(out.audio("out").iter().flatten().all(|v| *v == 0.0));
            assert_eq!(h.telemetry(TELEMETRY_STATUS), 1.0);
            let node = Plugin::new(
                &json!({ "plugin": desc() }),
                &PrepareCtx {
                    sample_rate: 48_000.0,
                    max_block: 64,
                    seed: 0,
                    node_key: "1",
                    name: "Synth",
                    resources: res.as_ref(),
                },
            );
            let w = node.warning().unwrap();
            assert!(w.starts_with("Synth (Example, CLAP): "), "{w}");
            assert!(w.contains(why), "{w}");
            assert_eq!(node.latency(), 0);
        }
    }

    #[test]
    fn a_failing_plugin_is_silenced_and_flagged() {
        let (mut fake, _) = host(true);
        fake.fail = true;
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 64, &fake);
        h.midi("midi", &[(0, note_on(60))]);
        let out = h.run(256);
        assert!(out.audio("out").iter().flatten().all(|v| *v == 0.0));
        assert_eq!(h.telemetry(TELEMETRY_STATUS), 2.0);
    }

    #[test]
    fn reset_reaches_the_plugin_and_restarts_the_stride() {
        let (fake, log) = host(true);
        let mut h =
            Harness::with_resources::<Plugin>(json!({ "plugin": desc() }), 48_000.0, 50, &fake);
        h.run(100);
        h.reset();
        h.run(100);
        assert_eq!(log.lock().unwrap().resets, 1);
    }

    #[test]
    fn the_saved_state_doesnt_rebuild_a_running_plugin() {
        assert_eq!(Plugin::meta().state_keys, &["state"]);
    }
}
