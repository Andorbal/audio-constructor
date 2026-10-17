use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "transport",
    name: "Transport",
    category: Generators,
    keywords: ["song time", "songtime", "position", "tempo", "bpm"],
    /// Song time as signals: seconds, beats, bars, tempo, and play state.
    ///
    /// `time` (seconds) and `beat` (quarter notes) count from the start of
    /// the song and advance every sample while the transport plays; they
    /// hold still while it is stopped. `bar` and `beat_in_bar` count from 0,
    /// update once per block, and follow time-signature changes. `bpm`
    /// follows tempo changes. `play` fires when the transport starts (or
    /// jumps back at a loop point) and `stop` when it stops.
    ///
    /// **Try this:** multiply `beat` by 6.283 with a `Math` node and feed it
    /// to a `Unary` (sin) for a wobble locked to the beat, or wire `play`
    /// into an envelope's trigger to restart it whenever the song starts.
    pub struct Transport {}
}

const TIME: usize = 0;
const BEAT: usize = 1;
const BAR: usize = 2;
const BEAT_IN_BAR: usize = 3;
const BPM: usize = 4;
const PLAYING: usize = 5;
const PLAY: usize = 6;
const STOP: usize = 7;

/// Integer part of a beat-in-bar position. The small bias keeps a value a
/// rounding error below a whole beat (2.9999999) from reading as the beat
/// before.
fn whole_beat(beat_in_bar: f64) -> i32 {
    (beat_in_bar + 1e-9).floor() as i32
}

impl Node for Transport {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .output(
                SocketSpec::float("time", Unit::Seconds, 0.0)
                    .rate(Rate::Sample)
                    .doc("Song position in seconds."),
            )
            .output(
                SocketSpec::float("beat", Unit::Beats, 0.0)
                    .rate(Rate::Sample)
                    .doc("Song position in beats (quarter notes)."),
            )
            .output(SocketSpec::int("bar", 0).doc("Bar number, from 0."))
            .output(SocketSpec::int("beat_in_bar", 0).doc("Beat within the bar, from 0."))
            .output(
                SocketSpec::float("bpm", Unit::Bpm, 120.0)
                    .rate(Rate::Block)
                    .doc("Current tempo."),
            )
            .output(
                SocketSpec::boolean("playing", false)
                    .rate(Rate::Block)
                    .doc("True while the transport plays."),
            )
            .output(SocketSpec::trigger("play").doc("Fires when the transport starts."))
            .output(SocketSpec::trigger("stop").doc("Fires when the transport stops."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Transport {}
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let t = ctx.transport;
        let tempo = t.tempo;
        let rate = if ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            tempo.sample_rate()
        };
        let at = |i: usize| {
            if t.playing {
                t.position.saturating_add(i as u64)
            } else {
                t.position
            }
        };
        io.outputs
            .float(TIME)
            .fill_with(|i| (at(i) as f64 / rate) as f32);
        io.outputs
            .float(BEAT)
            .fill_with(|i| tempo.beat_at(at(i)) as f32);

        let (bar, beat_in_bar) = tempo.bar_at(t.position);
        io.outputs
            .set_int(BAR, bar.clamp(0, i64::from(i32::MAX)) as i32);
        io.outputs.set_int(BEAT_IN_BAR, whole_beat(beat_in_bar));
        io.outputs.set_float(BPM, tempo.bpm_at(t.position) as f32);
        io.outputs.set_bool(PLAYING, t.playing);
        if t.started {
            io.outputs.trigger(PLAY).push(0);
        }
        if t.stopped {
            io.outputs.trigger(STOP).push(0);
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, TempoMap, TempoSegment};
    use serde_json::json;

    fn close(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() < 1e-7
    }

    #[test]
    fn beat_and_time_at_120_bpm() {
        let mut h = Harness::new::<Transport>(json!({}));
        let out = h.run(48_064);
        let beat = out.float("beat");
        let time = out.float("time");
        assert_eq!(beat[0], 0.0);
        assert_eq!(beat[24_000], 1.0);
        assert_eq!(beat[48_000], 2.0);
        assert_eq!(time[48_000], 1.0);
        assert!(close(time[1], 1.0 / 48_000.0));
        assert!(time.windows(2).all(|w| w[1] > w[0]));
        assert!(out.float("bpm").iter().all(|&b| b == 120.0));
        assert!(out.float("playing").iter().all(|&p| p == 1.0));
    }

    #[test]
    fn bars_and_beats_in_bar() {
        let mut h = Harness::new::<Transport>(json!({}));
        let out = h.run(96_000 + 64);
        let bar = out.int("bar");
        let bib = out.int("beat_in_bar");
        assert_eq!((bar[0], bib[0]), (0, 0));
        assert_eq!((bar[24_000], bib[24_000]), (0, 1));
        assert_eq!((bar[72_000], bib[72_000]), (0, 3));
        assert_eq!((bar[96_000], bib[96_000]), (1, 0));
    }

    #[test]
    fn follows_tempo_and_time_signature_changes() {
        // Two bars of 4/4 at 120, then 3/4 at 60.
        let tempo = TempoMap::new(
            48_000.0,
            &[
                TempoSegment {
                    beat: 0.0,
                    bpm: 120.0,
                    time_sig: [4, 4],
                },
                TempoSegment {
                    beat: 8.0,
                    bpm: 60.0,
                    time_sig: [3, 4],
                },
            ],
        );
        let mut h = Harness::new::<Transport>(json!({}));
        h.tempo(tempo);
        let out = h.run(192_000 + 3 * 48_000 + 64);
        assert_eq!(out.float("bpm")[191_999], 120.0);
        assert_eq!(out.float("bpm")[192_000], 60.0);
        assert_eq!(out.float("beat")[192_000 + 48_000], 9.0);
        assert_eq!(out.int("bar")[192_000], 2);
        assert_eq!(out.int("bar")[192_000 + 3 * 48_000], 3);
    }

    #[test]
    fn stopped_transport_freezes_position() {
        let mut h = Harness::new::<Transport>(json!({}));
        h.run(1_024);
        h.stop();
        let out = h.run(256);
        assert!(
            out.float("time")
                .iter()
                .all(|&t| close(t, 1_024.0 / 48_000.0))
        );
        assert!(
            out.float("beat")
                .iter()
                .all(|&b| close(b, 2_048.0 / 48_000.0))
        );
        assert!(out.float("playing").iter().all(|&p| p == 0.0));

        let mut h = Harness::new::<Transport>(json!({}));
        h.stopped();
        let out = h.run(128);
        assert!(out.float("time").iter().all(|&t| t == 0.0));
        assert!(out.triggers("play").is_empty());
    }

    #[test]
    fn play_and_stop_triggers() {
        let mut h = Harness::new::<Transport>(json!({}));
        let out = h.run(256);
        assert_eq!(out.triggers("play"), &[0]);
        assert!(out.triggers("stop").is_empty());
        h.stop();
        let out = h.run(256);
        assert_eq!(out.triggers("stop"), &[256]);
        assert!(out.triggers("play").is_empty());
        h.play();
        let out = h.run(128);
        assert_eq!(out.triggers("play"), &[512]);
        // Playback resumes from where it stopped.
        assert!(close(out.float("time")[0], 256.0 / 48_000.0));
    }

    #[test]
    fn sample_rate_independent() {
        let mut h = Harness::with_resources::<Transport>(json!({}), 44_100.0, 37, &NoResources);
        let out = h.run(44_200);
        assert_eq!(out.float("time")[44_100], 1.0);
        assert_eq!(out.float("beat")[22_050], 1.0);
    }

    #[test]
    fn whole_beats_survive_rounding() {
        assert_eq!(whole_beat(2.999_999_999_99), 3);
        assert_eq!(whole_beat(2.5), 2);
        assert_eq!(whole_beat(0.0), 0);
    }
}
