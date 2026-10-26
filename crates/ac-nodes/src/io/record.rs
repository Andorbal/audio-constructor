use ac_core::{
    Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, RecordRequest,
    RecordSink, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "record",
    name: "Record",
    category: Io,
    keywords: ["recorder", "capture", "bounce", "take", "tape", "file", "save"],
    /// Writes audio to a file, one take at a time.
    ///
    /// A take runs while `arm` is on and the transport is recording, or from
    /// a `start` trigger until a `stop` trigger, whichever keeps it going
    /// longer. Each take is a new file; the host chooses where it goes and
    /// names it after the node's label. Takes begin and end on block
    /// boundaries (a few milliseconds).
    ///
    /// `recording` is on while a take runs and `length` is the duration of
    /// the current or last take. Where recording isn't available (some quick
    /// renders) the node still reports its state but writes nothing. The
    /// node's readout also shows how many samples were dropped because the
    /// disk couldn't keep up.
    ///
    /// The `channels` config sets how many channels are written (2 by
    /// default); a mono signal into a stereo Record is written to both.
    ///
    /// `buffer` holds the last finished take (up to ten minutes of it) as
    /// soon as it is safely on disk, so a recording can feed a `Sampler`
    /// right away. It is empty until the first take finishes, and where
    /// recording isn't available.
    ///
    /// **Try this:** wire an `Audio In` into `in` and press record on the
    /// transport to capture a take in time with the song, or wire a
    /// `Button`'s `trigger` and `release` into `start` and `stop` for a
    /// hold-to-record pedal.
    pub struct Record {
        sink: Option<Box<dyn RecordSink>>,
        /// The last finished take, from the sink.
        buffer: Option<std::sync::Arc<ac_core::AudioData>>,
        sample_rate: f64,
        /// Started by the `start` trigger and not yet stopped.
        manual: bool,
        recording: bool,
        take_frames: u64,
    }
}

const IN: usize = 0;
const ARM: usize = 1;
const START: usize = 2;
const STOP: usize = 3;

const RECORDING: usize = 0;
const LENGTH: usize = 1;
const BUFFER: usize = 2;

/// Files are written by the host, so there's no device limit; this only
/// keeps a typo from asking for thousands of channels.
const MAX_CHANNELS: u64 = 16;

fn channel_count(config: &Config) -> u32 {
    config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as u32
}

/// The manual on/off state after this block's `start` and `stop` triggers:
/// the later one wins, and `stop` wins a tie.
fn manual_after(on: bool, starts: &[u32], stops: &[u32]) -> bool {
    match (starts.last(), stops.last()) {
        (None, None) => on,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (Some(start), Some(stop)) => start > stop,
    }
}

impl Node for Record {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(3)
    }

    fn describe(config: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(Channels::Fixed(channel_count(config)))
                    .doc("Signal to record."),
            )
            .input(SocketSpec::boolean("arm", true).doc("Record while the transport is recording."))
            .input(SocketSpec::trigger("start").doc("Starts a take, whatever the transport does."))
            .input(SocketSpec::trigger("stop").doc("Ends a take begun by `start`."))
            .output(
                SocketSpec::boolean("recording", false)
                    .rate(Rate::Block)
                    .doc("True while a take runs."),
            )
            .output(
                SocketSpec::float("length", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("Duration of the current or last take."),
            )
            .output(SocketSpec::buffer("buffer").doc("The last finished take."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let sink = ctx.resources.recorder(&RecordRequest {
            node_key: ctx.node_key,
            name: ctx.name,
            channels: channel_count(config) as usize,
            sample_rate: ctx.sample_rate.round() as u32,
        });
        Record {
            sink,
            buffer: None,
            sample_rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            manual: false,
            recording: false,
            take_frames: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let armed = io.inputs.boolean(ARM).first();
        self.manual = manual_after(
            self.manual,
            io.inputs.triggers(START),
            io.inputs.triggers(STOP),
        );
        let want = (armed && ctx.transport.recording) || self.manual;
        match (self.recording, want, self.sink.as_deref_mut()) {
            (false, true, sink) => {
                self.take_frames = 0;
                if let Some(sink) = sink {
                    let t = ctx.transport;
                    sink.start_at(t.playing.then_some(t.position));
                }
            }
            (true, false, Some(sink)) => sink.stop(),
            _ => {}
        }
        self.recording = want;

        if self.recording {
            if let Some(sink) = self.sink.as_deref_mut() {
                sink.write(&io.inputs.audio(IN));
            }
            self.take_frames = self.take_frames.saturating_add(ctx.frames as u64);
        }

        if let Some(b) = self.sink.as_deref_mut().and_then(|s| s.take_buffer()) {
            // The sink keeps its own handle, so the old one isn't freed here.
            self.buffer = Some(b);
        }
        let length = (self.take_frames as f64 / self.sample_rate) as f32;
        io.outputs.set_bool(RECORDING, self.recording);
        io.outputs.set_float(LENGTH, length);
        io.outputs.set_buffer(BUFFER, self.buffer.as_ref());
        let dropped = self.sink.as_deref().map_or(0, |s| s.dropped());
        io.telemetry.set(0, if self.recording { 1.0 } else { 0.0 });
        io.telemetry.set(1, length);
        io.telemetry.set(2, dropped as f32);
    }

    fn reset(&mut self) {
        if self.recording
            && let Some(sink) = self.sink.as_deref_mut()
        {
            sink.stop();
        }
        self.manual = false;
        self.recording = false;
        self.take_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{
        AudioIn, AutoFlags, FloatSrc, HostAccess, InputSlot, Inputs, NoResources, OutputSlot,
        Outputs, ParamSlot, Phase, Resources, TelemetryAccess, TempoMap, TransportView, TriggerSrc,
    };
    use serde_json::json;
    use std::sync::atomic::AtomicU32;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Start,
        Write { frames: usize, channels: usize },
        Stop,
    }

    type Log = Arc<Mutex<Vec<Call>>>;
    /// (node key, name, channels, sample rate) of each sink request.
    type Requests = Arc<Mutex<Vec<(String, String, usize, u32)>>>;

    #[derive(Default)]
    struct FakeRecorder {
        calls: Log,
        requests: Requests,
    }

    struct FakeSink {
        calls: Log,
        frames: usize,
        ready: Option<Arc<ac_core::AudioData>>,
        /// Handles kept off the audio thread, as real sinks must.
        keep: Vec<Arc<ac_core::AudioData>>,
    }

    impl RecordSink for FakeSink {
        fn start(&mut self) {
            self.calls.lock().unwrap().push(Call::Start);
        }
        fn write(&mut self, audio: &AudioIn<'_>) {
            self.frames += audio.frames();
            self.calls.lock().unwrap().push(Call::Write {
                frames: audio.frames(),
                channels: audio.channels(),
            });
        }
        fn stop(&mut self) {
            self.calls.lock().unwrap().push(Call::Stop);
            let take = Arc::new(ac_core::AudioData {
                sample_rate: 48_000,
                channels: vec![vec![0.5; self.frames]],
            });
            self.keep.push(take.clone());
            self.ready = Some(take);
            self.frames = 0;
        }
        fn dropped(&self) -> u64 {
            7
        }
        fn take_buffer(&mut self) -> Option<Arc<ac_core::AudioData>> {
            self.ready.take()
        }
    }

    impl Resources for FakeRecorder {
        fn recorder(&self, req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
            self.requests.lock().unwrap().push((
                req.node_key.to_string(),
                req.name.to_string(),
                req.channels,
                req.sample_rate,
            ));
            Some(Box::new(FakeSink {
                calls: self.calls.clone(),
                frames: 0,
                ready: None,
                keep: Vec::new(),
            }))
        }
    }

    impl FakeRecorder {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
        fn writes(&self) -> usize {
            self.calls()
                .iter()
                .filter(|c| matches!(c, Call::Write { .. }))
                .count()
        }
    }

    fn prepare(config: serde_json::Value, res: &dyn Resources) -> Record {
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "a/b",
            name: "Vocals",
            resources: res,
        };
        Record::new(&config, &ctx)
    }

    /// Runs one 64-frame block with a chosen transport recording state,
    /// which the harness doesn't model. Returns (`recording`, `length`).
    fn step(
        node: &mut Record,
        transport_recording: bool,
        armed: bool,
        start: &[u32],
        stop: &[u32],
    ) -> (bool, f32) {
        let frames = 64;
        let samples = vec![vec![0.5; frames]];
        let slots = [
            InputSlot::Audio { start: 0, count: 2 },
            InputSlot::Float(FloatSrc::Param(0)),
            InputSlot::Trigger(TriggerSrc::Wired(0)),
            InputSlot::Trigger(TriggerSrc::Wired(1)),
        ];
        let params = [ParamSlot {
            value: if armed { 1.0 } else { 0.0 },
            ..ParamSlot::default()
        }];
        let triggers = [start.to_vec(), stop.to_vec()];
        let out_slots = [
            OutputSlot::Float {
                local: 0,
                sample: false,
            },
            OutputSlot::Float {
                local: 1,
                sample: false,
            },
        ];
        let mut scalars = [0.0f32; 2];
        let telemetry: Vec<AtomicU32> = (0..3).map(|_| AtomicU32::new(0)).collect();
        let tempo = TempoMap::default();
        let mut transport = TransportView::stopped_at_zero(&tempo);
        transport.playing = transport_recording;
        transport.recording = transport_recording;
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
                samples: &samples,
                chan_table: &[0, 0],
                triggers: &triggers,
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
                samples: &mut [],
                scalars: &mut scalars,
                ints: &mut [],
                triggers: &mut [],
                midi: &mut [],
                spectra: &mut [],
                buffers: &mut [],
                texts: &mut [],
            },
            host: HostAccess::none(frames),
            telemetry: TelemetryAccess {
                values: &telemetry,
                scope: &[],
                scope_pos: None,
            },
        };
        node.process(&ctx, &mut io);
        (scalars[0] > 0.5, scalars[1])
    }

    #[test]
    fn asks_the_host_for_a_sink_with_node_details() {
        let res = FakeRecorder::default();
        prepare(json!({"channels": 1}), &res);
        assert_eq!(
            res.requests.lock().unwrap().as_slice(),
            &[("a/b".to_string(), "Vocals".to_string(), 1, 48_000)]
        );
    }

    #[test]
    fn arm_and_transport_recording_start_and_stop_takes() {
        let res = FakeRecorder::default();
        let mut r = prepare(json!({}), &res);
        assert!(!step(&mut r, false, true, &[], &[]).0);
        assert!(res.calls().is_empty());

        assert!(step(&mut r, true, true, &[], &[]).0);
        assert!(step(&mut r, true, true, &[], &[]).0);
        assert!(!step(&mut r, false, true, &[], &[]).0);
        let w = Call::Write {
            frames: 64,
            channels: 2,
        };
        assert_eq!(
            res.calls(),
            vec![Call::Start, w.clone(), w.clone(), Call::Stop]
        );

        // Disarming ends a take even while the transport keeps recording.
        step(&mut r, true, true, &[], &[]);
        assert!(!step(&mut r, true, false, &[], &[]).0);
        assert_eq!(res.calls().last(), Some(&Call::Stop));
        // An unarmed node ignores transport recording.
        assert!(!step(&mut r, true, false, &[], &[]).0);
        assert_eq!(res.writes(), 3);
    }

    #[test]
    fn manual_triggers_run_regardless_of_transport() {
        let res = FakeRecorder::default();
        let mut h = Harness::with_resources::<Record>(json!({}), 48_000.0, 64, &res);
        h.triggers("start", &[100]).triggers("stop", &[1000]);
        let out = h.run(2048);
        let rec = out.float("recording");
        // The take covers the blocks from the start trigger's block up to
        // the stop trigger's block.
        assert!(rec[..64].iter().all(|&v| v == 0.0));
        assert!(rec[64..960].iter().all(|&v| v == 1.0));
        assert!(rec[960..].iter().all(|&v| v == 0.0));
        let calls = res.calls();
        assert_eq!(calls.first(), Some(&Call::Start));
        assert_eq!(calls.last(), Some(&Call::Stop));
        assert_eq!(res.writes(), (960 - 64) / 64);
        let len = out.float("length");
        assert!((len[2047] - (960.0 - 64.0) / 48_000.0).abs() < 1e-7);
    }

    #[test]
    fn manual_take_outlasts_transport_recording() {
        let res = FakeRecorder::default();
        let mut r = prepare(json!({}), &res);
        step(&mut r, true, true, &[], &[]);
        assert!(step(&mut r, true, true, &[10], &[]).0);
        // Transport stops recording; the manual start keeps the take going.
        assert!(step(&mut r, false, true, &[], &[]).0);
        assert!(!step(&mut r, false, true, &[], &[5]).0);
        assert_eq!(res.calls().iter().filter(|c| **c == Call::Start).count(), 1);
    }

    #[test]
    fn later_trigger_wins_within_a_block() {
        assert!(manual_after(false, &[3], &[1]));
        assert!(!manual_after(true, &[1], &[3]));
        assert!(!manual_after(false, &[2], &[2]));
        assert!(manual_after(true, &[], &[]));
        assert!(!manual_after(false, &[], &[]));
    }

    #[test]
    fn length_counts_seconds_of_the_current_take() {
        let res = FakeRecorder::default();
        let mut h = Harness::with_resources::<Record>(json!({}), 48_000.0, 64, &res);
        h.triggers("start", &[0]);
        let out = h.run(48_000);
        assert_eq!(*out.float("length").last().unwrap(), 1.0);
        assert_eq!(h.telemetry(0), 1.0);
        assert_eq!(h.telemetry(1), 1.0);
        assert_eq!(h.telemetry(2), 7.0);

        // A second take starts again from zero.
        let res = FakeRecorder::default();
        let mut r = prepare(json!({}), &res);
        step(&mut r, false, true, &[0], &[]);
        step(&mut r, false, true, &[], &[]);
        let (_, held) = step(&mut r, false, true, &[], &[0]);
        assert!((held - 128.0 / 48_000.0).abs() < 1e-7);
        let (_, fresh) = step(&mut r, false, true, &[0], &[]);
        assert!((fresh - 64.0 / 48_000.0).abs() < 1e-7);
    }

    #[test]
    fn without_a_sink_the_state_is_still_tracked() {
        let mut h = Harness::new::<Record>(json!({}));
        h.triggers("start", &[0]).triggers("stop", &[640]);
        let out = h.run(1280);
        assert_eq!(out.float("recording")[100], 1.0);
        assert_eq!(out.float("recording")[700], 0.0);
        assert!((out.float("length")[1279] - 640.0 / 48_000.0).abs() < 1e-7);
        assert_eq!(h.telemetry(2), 0.0);

        let mut r = prepare(json!({}), &NoResources);
        assert!(step(&mut r, true, true, &[], &[]).0);
        r.reset();
    }

    #[test]
    fn reset_ends_a_take_in_progress() {
        let res = FakeRecorder::default();
        let mut r = prepare(json!({}), &res);
        step(&mut r, false, true, &[0], &[]);
        r.reset();
        assert_eq!(res.calls().last(), Some(&Call::Stop));
        // The manual start is forgotten.
        let (on, len) = step(&mut r, false, true, &[], &[]);
        assert!(!on);
        assert_eq!(len, 0.0);
        // Resetting while idle doesn't stop anything twice.
        r.reset();
        assert_eq!(res.calls().iter().filter(|c| **c == Call::Stop).count(), 1);
    }

    #[test]
    fn channels_config_sets_the_input() {
        let ch = |c| Record::describe(&c).inputs[IN].channels;
        assert_eq!(ch(json!({})), Channels::Fixed(2));
        assert_eq!(ch(json!({"channels": 1})), Channels::Fixed(1));
        assert_eq!(ch(json!({"channels": 0})), Channels::Fixed(1));
        assert_eq!(ch(json!({"channels": 500})), Channels::Fixed(16));
    }

    #[test]
    fn finished_takes_come_out_as_buffers() {
        let res = FakeRecorder::default();
        let mut h = Harness::with_resources::<Record>(json!({}), 48_000.0, 64, &res);
        h.set_audio("in", vec![vec![0.1; 1_000]])
            .set("arm", false)
            .triggers("start", &[0])
            .triggers("stop", &[256]);
        let out = h.run(512);
        let buf = out.buffer("buffer").expect("a take");
        assert_eq!(buf.frames(), 256);
        assert_eq!(buf.channels[0][0], 0.5);
        // No take yet: no buffer.
        let res = FakeRecorder::default();
        let mut h = Harness::with_resources::<Record>(json!({}), 48_000.0, 64, &res);
        assert!(h.run(128).buffer("buffer").is_none());
    }
}
