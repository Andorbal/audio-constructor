use ac_core::{
    Config, ConfigExt, Io, MidiMsg, MidiRecordSink, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate,
    RecordRequest, Signature, SocketSpec, Unit, node,
};

use super::record::manual_after;

node! {
    kind: "midi_record",
    name: "MIDI Record",
    category: Io,
    keywords: ["record midi", "capture", "take", "performance", "notes", "sequence", "overdub"],
    /// Records the notes played into it, one take at a time; each take
    /// becomes a MIDI Pattern beside it.
    ///
    /// A take runs while `arm` is on and the transport is recording, or from
    /// a `start` trigger until a `stop` trigger, as for `Record`. When it
    /// ends, the editor adds a MIDI Pattern of the notes next to the node:
    /// placed in the song at the beat the take began (when the transport
    /// was playing), as long as the take rounded up to whole bars, and
    /// playing into whatever the recorded MIDI went to, through a `Merge`
    /// so the keys still play there too. The `quantize` config moves each
    /// note's start to the nearest sixteenth, eighth, or quarter note
    /// (`off` by default, keeping the timing as played). Notes still held
    /// at the end of a take end with it.
    ///
    /// `recording` is on while a take runs, `notes` counts the take's
    /// notes, and `length` is how long it has run. Where recording isn't
    /// available (a render), nothing is kept.
    ///
    /// **Try this:** wire a `MIDI In` (or `Keyboard In`) into a MIDI Record
    /// and into a `Voices` synth, press record on the transport, play a
    /// phrase, and stop: the phrase is a MIDI Pattern, ready to loop.
    pub struct MidiRecord {
        sink: Option<Box<dyn MidiRecordSink>>,
        sample_rate: f64,
        manual: bool,
        recording: bool,
        take_frames: u64,
        notes: u32,
    }
}

const MIDI: usize = 0;
const ARM: usize = 1;
const START: usize = 2;
const STOP: usize = 3;

const RECORDING: usize = 0;
const NOTES: usize = 1;
const LENGTH: usize = 2;

/// Telemetry: recording (1 or 0), the take's notes.
pub const TELEMETRY_RECORDING: usize = 0;
pub const TELEMETRY_NOTES: usize = 1;

/// The `quantize` choices, and each one's grid in beats (0 for off).
pub const QUANTIZE: [(&str, f64); 4] = [
    ("off", 0.0),
    ("sixteenths", 0.25),
    ("eighths", 0.5),
    ("quarters", 1.0),
];

/// The grid in beats a take's notes snap to (0 for none).
pub fn quantize_grid(config: &Config) -> f64 {
    let q = config.get_str_or("quantize", "off");
    QUANTIZE
        .iter()
        .find(|(name, _)| *name == q)
        .map_or(0.0, |(_, g)| *g)
}

impl Node for MidiRecord {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("The notes to record."))
            .input(SocketSpec::boolean("arm", true).doc("Record while the transport is recording."))
            .input(SocketSpec::trigger("start").doc("Starts a take, whatever the transport does."))
            .input(SocketSpec::trigger("stop").doc("Ends a take begun by `start`."))
            .output(
                SocketSpec::boolean("recording", false)
                    .rate(Rate::Block)
                    .doc("True while a take runs."),
            )
            .output(SocketSpec::int("notes", 0).doc("Notes in the current or last take."))
            .output(
                SocketSpec::float("length", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("Duration of the current or last take."),
            )
    }

    fn new(_: &Config, ctx: &PrepareCtx) -> Self {
        let sink = ctx.resources.midi_recorder(&RecordRequest {
            node_key: ctx.node_key,
            name: ctx.name,
            channels: 0,
            sample_rate: ctx.sample_rate.round() as u32,
        });
        MidiRecord {
            sink,
            sample_rate: if ctx.sample_rate > 0.0 {
                ctx.sample_rate
            } else {
                48_000.0
            },
            manual: false,
            recording: false,
            take_frames: 0,
            notes: 0,
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
        match (self.recording, want) {
            (false, true) => {
                self.take_frames = 0;
                self.notes = 0;
                if let Some(sink) = self.sink.as_deref_mut() {
                    let t = ctx.transport;
                    sink.start_at(t.playing.then_some(t.position));
                }
            }
            (true, false) => {
                if let Some(sink) = self.sink.as_deref_mut() {
                    sink.stop(self.take_frames);
                }
            }
            _ => {}
        }
        self.recording = want;
        if self.recording {
            for e in io.inputs.midi(MIDI) {
                if matches!(e.msg, MidiMsg::NoteOn { velocity, .. } if velocity > 0) {
                    self.notes = self.notes.saturating_add(1);
                }
                if let Some(sink) = self.sink.as_deref_mut() {
                    sink.event(self.take_frames + u64::from(e.offset), e.msg);
                }
            }
            self.take_frames = self.take_frames.saturating_add(ctx.frames as u64);
        }
        io.outputs.set_bool(RECORDING, self.recording);
        io.outputs
            .set_int(NOTES, self.notes.min(i32::MAX as u32) as i32);
        io.outputs
            .set_float(LENGTH, (self.take_frames as f64 / self.sample_rate) as f32);
        io.telemetry
            .set(TELEMETRY_RECORDING, if self.recording { 1.0 } else { 0.0 });
        io.telemetry.set(TELEMETRY_NOTES, self.notes as f32);
    }

    fn reset(&mut self) {
        if self.recording
            && let Some(sink) = self.sink.as_deref_mut()
        {
            sink.stop(self.take_frames);
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
    use ac_core::{RecordSink, Resources};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<String>>>;

    struct LogSink(Log);

    impl MidiRecordSink for LogSink {
        fn start_at(&mut self, song: Option<u64>) {
            self.0.lock().unwrap().push(format!("start {song:?}"));
        }
        fn event(&mut self, frame: u64, msg: MidiMsg) {
            let note = match msg {
                MidiMsg::NoteOn { note, .. } => format!("on {note}"),
                MidiMsg::NoteOff { note, .. } => format!("off {note}"),
                _ => "other".into(),
            };
            self.0.lock().unwrap().push(format!("{frame} {note}"));
        }
        fn stop(&mut self, frames: u64) {
            self.0.lock().unwrap().push(format!("stop {frames}"));
        }
    }

    struct Res(Log);

    impl Resources for Res {
        fn recorder(&self, _: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
            None
        }
        fn midi_recorder(&self, _: &RecordRequest<'_>) -> Option<Box<dyn MidiRecordSink>> {
            Some(Box::new(LogSink(self.0.clone())))
        }
    }

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 90,
        }
    }

    #[test]
    fn a_take_runs_from_start_to_stop_with_its_notes_timed() {
        let log: Log = Arc::default();
        let res = Res(log.clone());
        let mut h = Harness::with_resources::<MidiRecord>(json!({}), 48_000.0, 64, &res);
        h.triggers("start", &[0]).triggers("stop", &[128]);
        h.midi("midi", &[(10, on(60)), (70, on(64)), (200, on(67))]);
        let out = h.run(256);
        let log = log.lock().unwrap().clone();
        assert_eq!(log[0], "start Some(0)");
        assert_eq!(log[1..3], ["10 on 60".to_string(), "70 on 64".to_string()]);
        assert_eq!(log[3], "stop 128", "the take ends with its block");
        assert_eq!(out.int("notes")[255], 2);
        assert_eq!(out.float("recording")[0], 1.0);
        assert_eq!(out.float("recording")[255], 0.0);
    }

    #[test]
    fn the_transport_records_an_armed_take_and_quantize_reads() {
        let log: Log = Arc::default();
        let res = Res(log.clone());
        let mut h = Harness::with_resources::<MidiRecord>(json!({}), 48_000.0, 64, &res);
        h.recording(true);
        h.run(64);
        h.recording(false);
        h.run(64);
        let log = log.lock().unwrap().clone();
        assert_eq!(log, ["start Some(0)", "stop 64"]);
        assert_eq!(quantize_grid(&json!({ "quantize": "eighths" })), 0.5);
        assert_eq!(quantize_grid(&json!({})), 0.0);
    }
}
