//! Standard MIDI files (format 0), for the messages `MidiOut` nodes send
//! during an offline render (`audioc render --midi-out`).
//!
//! Times go through the project's tempo map, so notes land on the same beats
//! in a sequencer, and the file carries the project's tempo and time
//! signature changes. Format: the Standard MIDI File 1.0 specification.

use std::io;
use std::path::Path;

use ac_core::{MidiMsg, TempoMap, TempoSegment};

/// Resolution of the file's time stamps.
pub const TICKS_PER_BEAT: u16 = 960;

/// A format-0 file holding `events` (at frames counted from `start`, the
/// render's first frame) at `sample_rate`.
pub fn midi_file_bytes(
    events: &[(u64, MidiMsg)],
    start: u64,
    sample_rate: f64,
    tempo: &[TempoSegment],
) -> Vec<u8> {
    let map = TempoMap::new(sample_rate, tempo);
    let start_beat = map.beat_at(start);
    let tick =
        |beat: f64| ((beat - start_beat).max(0.0) * f64::from(TICKS_PER_BEAT)).round() as u64;

    // (tick, meta first, bytes); a stable sort keeps the messages' order.
    let mut items: Vec<(u64, u8, Vec<u8>)> = Vec::new();
    let mut segs: Vec<&TempoSegment> = tempo.iter().collect();
    segs.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    for (k, seg) in segs.iter().enumerate() {
        let next = segs.get(k + 1).map_or(f64::INFINITY, |n| n.beat);
        if next <= start_beat {
            continue;
        }
        let at = tick(seg.beat);
        let bpm = if seg.bpm.is_finite() && seg.bpm > 0.0 {
            seg.bpm
        } else {
            120.0
        };
        let us = (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32;
        items.push((
            at,
            0,
            vec![0xff, 0x51, 3, (us >> 16) as u8, (us >> 8) as u8, us as u8],
        ));
        let [num, den] = seg.time_sig;
        let den_pow = den.max(1).ilog2() as u8;
        items.push((at, 0, vec![0xff, 0x58, 4, num.max(1), den_pow, 24, 8]));
    }
    for &(at, msg) in events {
        let (bytes, len) = msg.to_bytes();
        items.push((tick(map.beat_at(start + at)), 1, bytes[..len].to_vec()));
    }
    items.sort_by_key(|(t, meta, _)| (*t, *meta));

    let mut track = Vec::new();
    let mut last = 0;
    for (t, _, bytes) in &items {
        push_vlq(&mut track, t - last);
        track.extend_from_slice(bytes);
        last = *t;
    }
    track.extend_from_slice(&[0, 0xff, 0x2f, 0]);

    let mut out = Vec::with_capacity(track.len() + 22);
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&TICKS_PER_BEAT.to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(&track);
    out
}

/// Write [`midi_file_bytes`] to `path`.
pub fn write_midi_file(
    path: &Path,
    events: &[(u64, MidiMsg)],
    start: u64,
    sample_rate: f64,
    tempo: &[TempoSegment],
) -> io::Result<()> {
    std::fs::write(path, midi_file_bytes(events, start, sample_rate, tempo))
}

/// A variable-length quantity: seven bits per byte, most significant first.
/// Deltas past the format's 28-bit limit are clamped.
fn push_vlq(out: &mut Vec<u8>, v: u64) {
    let v = v.min(0x0fff_ffff) as u32;
    let mut groups = [0u8; 4];
    let mut n = 0;
    let mut x = v;
    loop {
        groups[n] = (x & 0x7f) as u8;
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    for k in (0..n).rev() {
        out.push(groups[k] | if k > 0 { 0x80 } else { 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }

    fn vlq(v: u64) -> Vec<u8> {
        let mut o = Vec::new();
        push_vlq(&mut o, v);
        o
    }

    #[test]
    fn variable_length_quantities_match_the_spec() {
        assert_eq!(vlq(0), [0x00]);
        assert_eq!(vlq(0x40), [0x40]);
        assert_eq!(vlq(0x7f), [0x7f]);
        assert_eq!(vlq(0x80), [0x81, 0x00]);
        assert_eq!(vlq(0x2000), [0xc0, 0x00]);
        assert_eq!(vlq(0x3fff), [0xff, 0x7f]);
        assert_eq!(vlq(0x0fff_ffff), [0xff, 0xff, 0xff, 0x7f]);
    }

    #[test]
    fn a_beat_is_960_ticks_at_any_tempo() {
        let tempo = [TempoSegment {
            beat: 0.0,
            bpm: 120.0,
            time_sig: [4, 4],
        }];
        // 120 BPM at 48 kHz: a beat is 24 000 frames.
        let bytes = midi_file_bytes(&[(0, on(60)), (24_000, on(62))], 0, 48_000.0, &tempo);
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(&bytes[8..14], &[0, 0, 0, 1, 0x03, 0xc0]);
        assert_eq!(&bytes[14..18], b"MTrk");
        let track = &bytes[22..];
        // Tempo: 500 000 µs per beat.
        assert_eq!(&track[..7], &[0, 0xff, 0x51, 3, 0x07, 0xa1, 0x20]);
        // Time signature 4/4.
        assert_eq!(&track[7..15], &[0, 0xff, 0x58, 4, 4, 2, 24, 8]);
        assert_eq!(&track[15..19], &[0, 0x90, 60, 100]);
        // 960 = 0x3c0 → 0x87 0x40.
        assert_eq!(&track[19..24], &[0x87, 0x40, 0x90, 62, 100]);
        assert_eq!(&track[24..], &[0, 0xff, 0x2f, 0]);
        let len = u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize;
        assert_eq!(len, track.len());
    }

    #[test]
    fn a_render_from_later_starts_the_file_there() {
        let tempo = [
            TempoSegment {
                beat: 0.0,
                bpm: 120.0,
                time_sig: [4, 4],
            },
            TempoSegment {
                beat: 4.0,
                bpm: 60.0,
                time_sig: [3, 4],
            },
        ];
        // Start at beat 2; the tempo change comes 2 beats in.
        let bytes = midi_file_bytes(&[(0, on(60))], 48_000, 48_000.0, &tempo);
        let track = &bytes[22..];
        assert_eq!(&track[..7], &[0, 0xff, 0x51, 3, 0x07, 0xa1, 0x20]);
        assert_eq!(&track[15..19], &[0, 0x90, 60, 100]);
        // 1920 ticks later (0x780 → 0x8f 0x00): one second per beat.
        assert_eq!(&track[19..26], &[0x8f, 0x00, 0xff, 0x51, 3, 0x0f, 0x42]);
    }
}
