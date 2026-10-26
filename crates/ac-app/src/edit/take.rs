//! Create Clip from take (DECISIONS D208): a Clip of the take, placed in the
//! song at the beat the take started, playing where the recorded sound went.
//! A MIDI take becomes a MIDI Pattern the same way (D218), merged into where
//! the played notes went.

use ac_core::{Endpoint, Graph, MidiMsg, NodeId, SignalType, TempoMap, Value};

use super::{Ctx, add_node, connect};

/// What a take knows about itself, for the clip made from it.
pub struct TakeClip {
    /// The Clip's config (its file, channels, hash).
    pub config: ac_core::Config,
    /// The song beat the take started at, when the transport was playing.
    pub beat: Option<f64>,
    /// The Record node that made it, in this graph.
    pub record: Option<NodeId>,
    /// Where to put the Clip.
    pub pos: [f32; 2],
}

/// Add the Clip: started *At beat…* where the take began, and wired into
/// every audio input the Record's source also feeds (the take replaces
/// the live sound there, which stays wired to the Record for the next
/// take). Returns the Clip's id.
pub fn clip_from_take(graph: &mut Graph, ctx: &mut Ctx, take: TakeClip) -> NodeId {
    let clip = add_node(graph, ctx, "clip", take.config, take.pos);
    if let (Some(beat), Some(n)) = (take.beat, graph.node_mut(clip)) {
        n.params.insert("start".into(), Value::Text("at".into()));
        n.params.insert("at".into(), Value::Float(beat.max(0.0)));
    }
    let Some(record) = take.record else {
        return clip;
    };
    let source = graph
        .wires_to(record)
        .find(|w| w.to.socket == "in")
        .map(|w| w.from.clone());
    let Some(source) = source else {
        return clip;
    };
    let targets: Vec<Endpoint> = graph
        .wires_from(source.node)
        .filter(|w| w.from.socket == source.socket && w.to.node != record)
        .map(|w| w.to.clone())
        .filter(|to| ctx.in_type(graph, to) == Some(SignalType::Audio))
        .collect();
    for to in targets {
        let _ = connect(graph, ctx, Endpoint::new(clip, "out"), to);
    }
    clip
}

/// A MIDI take's notes as a MIDI Pattern would hold them, from the beat
/// the take started: `[start, length, note, velocity]` in beats, sorted.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeNotes {
    pub notes: Vec<[f64; 4]>,
    /// The take rounded up to whole bars, in beats (at least one bar).
    pub length: f64,
    /// The song beat the take started at, when the transport played.
    pub beat: Option<f64>,
}

/// The notes of `events` (samples from the take's start, which was song
/// sample `song_start`), `frames` long, timed by `tempo`, with starts
/// moved to the nearest multiple of `grid` beats (0 for as played).
/// Note-ons pair with the next note-off of their channel and note; notes
/// still held at the end end there.
pub fn take_notes(
    events: &[(u64, MidiMsg)],
    frames: u64,
    song_start: Option<u64>,
    tempo: &TempoMap,
    grid: f64,
) -> TakeNotes {
    let origin = song_start.unwrap_or(0);
    let start_beat = tempo.beat_at(origin);
    let beat = |frame: u64| tempo.beat_at(origin + frame) - start_beat;
    let mut held: Vec<(u8, u8, f64, u8)> = Vec::new();
    let mut notes = Vec::new();
    let close = |notes: &mut Vec<[f64; 4]>, (_, note, on, vel): (u8, u8, f64, u8), off: f64| {
        let start = if grid > 0.0 {
            (on / grid).round() * grid
        } else {
            on
        };
        let len = (off - on).max(if grid > 0.0 { grid / 4.0 } else { 1.0 / 64.0 });
        notes.push([
            round6(start.max(0.0)),
            round6(len),
            f64::from(note),
            f64::from(vel),
        ]);
    };
    for &(frame, msg) in events {
        match msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => {
                // The same note again ends the one held.
                if let Some(i) = held.iter().position(|h| h.0 == channel && h.1 == note) {
                    let h = held.remove(i);
                    close(&mut notes, h, beat(frame));
                }
                held.push((channel, note, beat(frame), velocity));
            }
            MidiMsg::NoteOn { channel, note, .. } | MidiMsg::NoteOff { channel, note, .. } => {
                if let Some(i) = held.iter().position(|h| h.0 == channel && h.1 == note) {
                    let h = held.remove(i);
                    close(&mut notes, h, beat(frame));
                }
            }
            _ => {}
        }
    }
    let end = beat(frames);
    for h in held {
        close(&mut notes, h, end);
    }
    notes.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[2].total_cmp(&b[2])));
    let bar = f64::from(tempo.time_sig_at(origin)[0].max(1));
    let last = notes.iter().map(|n| n[0] + n[1]).fold(end, f64::max);
    TakeNotes {
        notes,
        length: ((last / bar).ceil() * bar).clamp(bar, 1024.0),
        beat: song_start.map(|s| tempo.beat_at(s)),
    }
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Add a MIDI Pattern of a take, next to its MIDI Record node: placed *At
/// beat…* where the take began, played once, and merged into every MIDI
/// input the Record's source also feeds. Returns the pattern.
pub fn pattern_from_take(
    graph: &mut Graph,
    ctx: &mut Ctx,
    take: TakeNotes,
    record: Option<NodeId>,
    pos: [f32; 2],
) -> NodeId {
    let notes: Vec<serde_json::Value> = take
        .notes
        .iter()
        .map(|n| serde_json::json!([n[0], n[1], n[2] as u8, n[3] as u8]))
        .collect();
    let config = serde_json::json!({ "length": take.length, "notes": notes });
    let pattern = add_node(graph, ctx, "midi_pattern", config, pos);
    if let (Some(beat), Some(n)) = (take.beat, graph.node_mut(pattern)) {
        n.params.insert("start".into(), Value::Text("at".into()));
        n.params.insert("at".into(), Value::Float(beat.max(0.0)));
        n.params.insert("loop".into(), Value::Bool(false));
    }
    let Some(record) = record else {
        return pattern;
    };
    let Some(source) = graph
        .wires_to(record)
        .find(|w| w.to.socket == "midi")
        .map(|w| w.from.clone())
    else {
        return pattern;
    };
    let targets: Vec<Endpoint> = graph
        .wires_from(source.node)
        .filter(|w| w.from.socket == source.socket && w.to.node != record)
        .map(|w| w.to.clone())
        .filter(|to| ctx.in_type(graph, to) == Some(SignalType::Midi))
        .collect();
    for (k, to) in targets.into_iter().enumerate() {
        let at = [pos[0], pos[1] + 220.0 + 160.0 * k as f32];
        let _ = super::merge_into(
            graph,
            ctx,
            source.clone(),
            Endpoint::new(pattern, "midi"),
            to,
            SignalType::Midi,
            at,
        );
    }
    pattern
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::Document;
    use ac_core::{NodeInstance, Project};

    #[test]
    fn a_take_becomes_a_placed_clip_playing_where_the_mic_went() {
        let mut p = Project::default();
        for (id, kind) in [(1, "audio_in"), (2, "record"), (3, "mix"), (4, "meter")] {
            p.graph.nodes.push(NodeInstance::new(NodeId(id), kind));
        }
        let g = &mut p.graph;
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        );
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(3), "in_0"),
        );
        g.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(4), "in"),
        );
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let clip = d.edit(&r, "Create Clip from take", |g, ctx| {
            clip_from_take(
                g,
                ctx,
                TakeClip {
                    config: serde_json::json!({ "file": "assets/vox-001.wav", "channels": 1 }),
                    beat: Some(16.0),
                    record: Some(NodeId(2)),
                    pos: [0.0, 0.0],
                },
            )
        });
        let g = &d.project.graph;
        let n = g.node(clip).expect("the clip");
        assert_eq!(n.params.get("start"), Some(&Value::Text("at".into())));
        assert_eq!(n.params.get("at"), Some(&Value::Float(16.0)));
        let from = |id: u64, socket: &str| {
            g.wire_into(&Endpoint::new(NodeId(id), socket))
                .map(|w| w.from.node)
        };
        assert_eq!(from(3, "in_0"), Some(clip), "the take plays in the mix");
        assert_eq!(from(4, "in"), Some(clip));
        assert_eq!(from(2, "in"), Some(NodeId(1)), "the mic still records");
    }

    fn on(note: u8, velocity: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity,
        }
    }

    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 64,
        }
    }

    #[test]
    fn a_takes_notes_come_out_in_beats_quantized_if_asked() {
        // 120 BPM at 48 kHz: a beat is 24 000 samples. The take starts at
        // beat 8.
        let tempo = TempoMap::constant(48_000.0, 120.0);
        let events = [
            (1_000, on(60, 100)),
            (12_500, off(60)),
            (24_000, on(64, 80)),
            (30_000, on(67, 90)),
            (36_000, off(64)),
        ];
        let t = take_notes(&events, 96_000, Some(8 * 24_000), &tempo, 0.0);
        assert_eq!(t.beat, Some(8.0));
        assert_eq!(t.length, 4.0, "one bar");
        assert_eq!(t.notes.len(), 3);
        assert!((t.notes[0][0] - 1.0 / 24.0).abs() < 1e-6, "as played");
        assert_eq!(t.notes[1], [1.0, 0.5, 64.0, 80.0]);
        assert_eq!(t.notes[2], [1.25, 2.75, 67.0, 90.0], "held to the end");
        let q = take_notes(&events, 96_000, None, &tempo, 0.25);
        assert_eq!(q.notes[0][0], 0.0, "on the sixteenth");
        assert_eq!(q.beat, None);
    }

    #[test]
    fn a_midi_take_becomes_a_placed_pattern_merged_where_the_keys_went() {
        let mut p = Project::default();
        for (id, kind) in [(1, "midi_in"), (2, "midi_record"), (3, "midi_note")] {
            p.graph.nodes.push(NodeInstance::new(NodeId(id), kind));
        }
        p.graph.connect(
            Endpoint::new(NodeId(1), "midi"),
            Endpoint::new(NodeId(2), "midi"),
        );
        p.graph.connect(
            Endpoint::new(NodeId(1), "midi"),
            Endpoint::new(NodeId(3), "midi"),
        );
        let mut d = Document::new(p, None, PathBuf::from("."), 1);
        let r = ac_nodes::registry();
        let take = TakeNotes {
            notes: vec![[0.0, 1.0, 60.0, 100.0]],
            length: 4.0,
            beat: Some(4.0),
        };
        let pat = d.edit(&r, "MIDI take", |g, ctx| {
            pattern_from_take(g, ctx, take, Some(NodeId(2)), [0.0, 300.0])
        });
        let g = &d.project.graph;
        let n = g.node(pat).expect("the pattern");
        assert_eq!(n.config["notes"], serde_json::json!([[0.0, 1.0, 60, 100]]));
        assert_eq!(n.params.get("at"), Some(&Value::Float(4.0)));
        assert_eq!(n.params.get("loop"), Some(&Value::Bool(false)));
        let into_synth = g
            .wire_into(&Endpoint::new(NodeId(3), "midi"))
            .map(|w| w.from.node)
            .unwrap();
        let merge = g.node(into_synth).unwrap();
        assert_eq!(merge.kind, "merge", "the keys and the take both play");
        assert!(
            g.wire_into(&Endpoint::new(into_synth, "in_1"))
                .is_some_and(|w| w.from.node == pat)
        );
        assert_eq!(
            g.wire_into(&Endpoint::new(NodeId(2), "midi"))
                .map(|w| w.from.node),
            Some(NodeId(1)),
            "the keys still record"
        );
    }
}
