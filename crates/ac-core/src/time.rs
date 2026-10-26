//! Musical time. The integer sample position is ground truth; beats and bars
//! are derived per tempo segment (ARCHITECTURE §6, DECISIONS D6).

use crate::project::TempoSegment;

#[derive(Clone, Debug, PartialEq)]
struct Seg {
    start_sample: u64,
    start_beat: f64,
    start_bar: f64,
    bpm: f64,
    beats_per_bar: f64,
    time_sig: [u8; 2],
}

/// A tempo map resolved at a concrete sample rate.
///
/// Within a segment, `beat = start_beat + (sample - start_sample) * bpm / (60 * rate)`
/// in `f64`; conversions to samples round to the nearest sample. Segment
/// starts are themselves integers, so error never accumulates across segments.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMap {
    sample_rate: f64,
    segs: Vec<Seg>,
}

impl TempoMap {
    /// Resolve segments (from project settings) at `sample_rate`. Invalid
    /// entries are repaired rather than rejected: a missing segment at beat 0
    /// gets 120 BPM 4/4, non-positive tempos become 120, unsorted segments
    /// are sorted.
    pub fn new(sample_rate: f64, segments: &[TempoSegment]) -> TempoMap {
        let sample_rate = if sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let mut defs: Vec<TempoSegment> = segments
            .iter()
            .filter(|s| s.beat.is_finite() && s.beat >= 0.0)
            .cloned()
            .collect();
        defs.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        defs.dedup_by(|b, a| a.beat == b.beat);
        if defs.first().is_none_or(|s| s.beat > 0.0) {
            defs.insert(0, TempoSegment::default());
        }
        let mut segs: Vec<Seg> = Vec::with_capacity(defs.len());
        for d in &defs {
            let bpm = if d.bpm.is_finite() && d.bpm > 0.0 {
                d.bpm.clamp(1.0, 2000.0)
            } else {
                120.0
            };
            let num = d.time_sig[0].max(1);
            let den = d.time_sig[1].max(1);
            let beats_per_bar = f64::from(num) * 4.0 / f64::from(den);
            let (start_sample, start_bar) = match segs.last() {
                None => (0, 0.0),
                Some(p) => {
                    let beats = d.beat - p.start_beat;
                    let samples = (beats * 60.0 * sample_rate / p.bpm).round() as u64;
                    (
                        p.start_sample + samples,
                        p.start_bar + beats / p.beats_per_bar,
                    )
                }
            };
            segs.push(Seg {
                start_sample,
                start_beat: d.beat,
                start_bar,
                bpm,
                beats_per_bar,
                time_sig: [num, den],
            });
        }
        TempoMap { sample_rate, segs }
    }

    /// A single tempo, 4/4.
    pub fn constant(sample_rate: f64, bpm: f64) -> TempoMap {
        TempoMap::new(
            sample_rate,
            &[TempoSegment {
                beat: 0.0,
                bpm,
                time_sig: [4, 4],
            }],
        )
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn seg_at_sample(&self, sample: u64) -> &Seg {
        let i = self.segs.partition_point(|s| s.start_sample <= sample);
        &self.segs[i.saturating_sub(1).min(self.segs.len() - 1)]
    }

    fn seg_at_beat(&self, beat: f64) -> &Seg {
        let i = self.segs.partition_point(|s| s.start_beat <= beat);
        &self.segs[i.saturating_sub(1).min(self.segs.len() - 1)]
    }

    /// Beat position (quarter notes since transport zero) at a sample.
    pub fn beat_at(&self, sample: u64) -> f64 {
        let s = self.seg_at_sample(sample);
        s.start_beat
            + sample.saturating_sub(s.start_sample) as f64 * s.bpm / (60.0 * self.sample_rate)
    }

    /// The sample nearest to a beat position.
    pub fn sample_at(&self, beat: f64) -> u64 {
        let beat = beat.max(0.0);
        let s = self.seg_at_beat(beat);
        // `as u64` saturates for huge values; the add must too.
        s.start_sample.saturating_add(
            ((beat - s.start_beat) * 60.0 * self.sample_rate / s.bpm).round() as u64,
        )
    }

    /// Exact (unrounded) sample position of a beat, for sub-sample phase.
    pub fn sample_at_exact(&self, beat: f64) -> f64 {
        let beat = beat.max(0.0);
        let s = self.seg_at_beat(beat);
        s.start_sample as f64 + (beat - s.start_beat) * 60.0 * self.sample_rate / s.bpm
    }

    pub fn bpm_at(&self, sample: u64) -> f64 {
        self.seg_at_sample(sample).bpm
    }

    pub fn time_sig_at(&self, sample: u64) -> [u8; 2] {
        self.seg_at_sample(sample).time_sig
    }

    /// Bar number (from 0) and beat within the bar (from 0) at a sample.
    pub fn bar_at(&self, sample: u64) -> (i64, f64) {
        let s = self.seg_at_sample(sample);
        let beats_in = self.beat_at(sample) - s.start_beat;
        let bar_pos = s.start_bar + beats_in / s.beats_per_bar;
        // Guard against values like 3.9999999 at exact bar lines.
        let bar = (bar_pos + 1e-9).floor();
        let beat_in_bar = ((bar_pos - bar) * s.beats_per_bar).max(0.0);
        (bar as i64, beat_in_bar)
    }

    /// Seconds since transport zero.
    pub fn seconds_at(&self, sample: u64) -> f64 {
        sample as f64 / self.sample_rate
    }

    /// Samples per beat at a position (for interval computations).
    pub fn samples_per_beat_at(&self, sample: u64) -> f64 {
        60.0 * self.sample_rate / self.bpm_at(sample)
    }
}

impl Default for TempoMap {
    fn default() -> Self {
        TempoMap::constant(48_000.0, 120.0)
    }
}

/// What a node sees of the transport during one block.
#[derive(Clone, Copy, Debug)]
pub struct TransportView<'a> {
    pub playing: bool,
    pub recording: bool,
    /// Sample position at the first frame of this block. Frozen while stopped.
    pub position: u64,
    pub tempo: &'a TempoMap,
    /// Playback started (or restarted at a loop point) at frame 0 of this block.
    pub started: bool,
    /// Playback stopped at frame 0 of this block.
    pub stopped: bool,
}

/// What a song-locked generator (its `start` set to *At beat…*) does in a
/// block, from [`TransportView::song_lock`] (DECISIONS D206).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SongLock {
    /// Carry on as it is.
    Hold,
    /// The transport (re)started before the generator's beat, or past the
    /// beat it stops at: be stopped.
    Wait,
    /// The transport (re)started past the beat: be playing from frame 0,
    /// this far into itself, in samples and in beats.
    Land { samples: u64, beats: f64 },
    /// The song reaches the beat at this frame of the block: start there.
    Start(usize),
    /// The song reaches the beat it stops at, at this frame: stop there.
    Stop(usize),
}

impl<'a> TransportView<'a> {
    /// Where a generator placed in the song from beat `at` until beat
    /// `until` (no end when `until` isn't past `at`) should be this block
    /// of `frames`. It lands whenever the transport starts (play, a seek, a
    /// loop), so it's in the same place however the song got there; while
    /// stopped it holds.
    pub fn song_lock(&self, frames: usize, at: f64, until: f64) -> SongLock {
        if !self.playing {
            return SongLock::Hold;
        }
        let beat = |b: f64| if b.is_finite() { b.max(0.0) } else { 0.0 };
        let (at, until) = (beat(at), beat(until));
        let target = self.tempo.sample_at(at);
        let end = (until > at).then(|| self.tempo.sample_at(until));
        let within = |s: u64| s >= self.position && s - self.position < frames as u64;
        if self.started && end.is_some_and(|e| self.position >= e) {
            return SongLock::Wait;
        }
        if self.started && self.position > target {
            return SongLock::Land {
                samples: self.position - target,
                beats: (self.tempo.beat_at(self.position) - at).max(0.0),
            };
        }
        if within(target) {
            return SongLock::Start((target - self.position) as usize);
        }
        if let Some(e) = end.filter(|&e| within(e)) {
            return SongLock::Stop((e - self.position) as usize);
        }
        if self.started {
            SongLock::Wait
        } else {
            SongLock::Hold
        }
    }

    /// A stopped transport at zero.
    pub fn stopped_at_zero(tempo: &'a TempoMap) -> Self {
        TransportView {
            playing: false,
            recording: false,
            position: 0,
            tempo,
            started: false,
            stopped: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(beat: f64, bpm: f64, sig: [u8; 2]) -> TempoSegment {
        TempoSegment {
            beat,
            bpm,
            time_sig: sig,
        }
    }

    #[test]
    fn a_song_lock_lands_waits_and_starts_on_its_sample() {
        let tempo = TempoMap::constant(48_000.0, 120.0);
        let at = |position: u64, started: bool| TransportView {
            playing: true,
            recording: false,
            position,
            tempo: &tempo,
            started,
            stopped: false,
        };
        // Beat 8 is sample 192 000 at 120 BPM; beat 10, 240 000.
        assert_eq!(at(0, true).song_lock(64, 8.0, 0.0), SongLock::Wait);
        assert_eq!(at(64, false).song_lock(64, 8.0, 0.0), SongLock::Hold);
        assert_eq!(
            at(191_990, false).song_lock(64, 8.0, 0.0),
            SongLock::Start(10)
        );
        assert_eq!(
            at(192_000, true).song_lock(64, 8.0, 0.0),
            SongLock::Start(0)
        );
        assert_eq!(
            at(216_000, true).song_lock(64, 8.0, 10.0),
            SongLock::Land {
                samples: 24_000,
                beats: 1.0
            }
        );
        let stopped = TransportView {
            playing: false,
            ..at(216_000, true)
        };
        assert_eq!(stopped.song_lock(64, 8.0, 0.0), SongLock::Hold);
        // It stops on the sample of `until`, and past it stays stopped.
        assert_eq!(
            at(239_990, false).song_lock(64, 8.0, 10.0),
            SongLock::Stop(10)
        );
        assert_eq!(at(240_000, true).song_lock(64, 8.0, 10.0), SongLock::Wait);
        assert_eq!(at(250_000, false).song_lock(64, 8.0, 10.0), SongLock::Hold);
        // An `until` not past `at` is no end.
        assert!(matches!(
            at(250_000, true).song_lock(64, 8.0, 4.0),
            SongLock::Land {
                samples: 58_000,
                ..
            }
        ));
    }

    #[test]
    fn beat_lands_on_exact_sample_at_common_rates() {
        for (rate, bpm, per_beat) in [
            (48_000.0, 120.0, 24_000u64),
            (44_100.0, 120.0, 22_050),
            (96_000.0, 90.0, 64_000),
            (48_000.0, 140.0, 20_571), // 20571.43 rounds down
        ] {
            let m = TempoMap::constant(rate, bpm);
            assert_eq!(m.sample_at(1.0), per_beat, "rate {rate} bpm {bpm}");
            assert_eq!(m.sample_at(0.0), 0);
            let far = m.sample_at(10_000.0);
            let expected = (10_000.0 * 60.0 * rate / bpm).round() as u64;
            assert_eq!(far, expected, "no drift over 10k beats");
        }
    }

    #[test]
    fn beat_at_inverts_sample_at() {
        let m = TempoMap::constant(44_100.0, 133.0);
        for b in [0.0, 0.25, 1.0, 7.5, 1234.75] {
            let s = m.sample_at(b);
            let back = m.beat_at(s);
            let tolerance = 133.0 / (60.0 * 44_100.0);
            assert!((back - b).abs() <= tolerance, "{b} -> {s} -> {back}");
        }
    }

    #[test]
    fn tempo_change_starts_on_an_exact_sample() {
        // 4 beats at 120 BPM (2 s), then 60 BPM.
        let m = TempoMap::new(48_000.0, &[seg(0.0, 120.0, [4, 4]), seg(4.0, 60.0, [4, 4])]);
        assert_eq!(m.sample_at(4.0), 96_000);
        assert_eq!(m.sample_at(5.0), 96_000 + 48_000);
        assert_eq!(m.beat_at(96_000 + 24_000), 4.5);
        assert_eq!(m.bpm_at(95_999), 120.0);
        assert_eq!(m.bpm_at(96_000), 60.0);
    }

    #[test]
    fn tempo_changes_at_several_rates() {
        for rate in [22_050.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
            let m = TempoMap::new(
                rate,
                &[
                    seg(0.0, 100.0, [4, 4]),
                    seg(3.0, 150.0, [3, 4]),
                    seg(10.5, 72.5, [7, 8]),
                ],
            );
            let s1 = (3.0 * 60.0 * rate / 100.0_f64).round() as u64;
            assert_eq!(m.sample_at(3.0), s1);
            let s2 = s1 + (7.5 * 60.0 * rate / 150.0_f64).round() as u64;
            assert_eq!(m.sample_at(10.5), s2);
            let s3 = s2 + (2.0 * 60.0 * rate / 72.5_f64).round() as u64;
            assert_eq!(m.sample_at(12.5), s3);
            // Monotonic.
            let mut last = 0;
            for i in 0..200 {
                let s = m.sample_at(i as f64 * 0.1);
                assert!(s >= last);
                last = s;
            }
        }
    }

    #[test]
    fn bars_follow_time_signature_changes() {
        // Two bars of 4/4 (8 beats), then 3/4.
        let m = TempoMap::new(
            48_000.0,
            &[seg(0.0, 120.0, [4, 4]), seg(8.0, 120.0, [3, 4])],
        );
        assert_eq!(m.bar_at(m.sample_at(0.0)), (0, 0.0));
        assert_eq!(m.bar_at(m.sample_at(4.0)), (1, 0.0));
        assert_eq!(m.bar_at(m.sample_at(8.0)), (2, 0.0));
        assert_eq!(m.bar_at(m.sample_at(11.0)), (3, 0.0));
        let (bar, beat) = m.bar_at(m.sample_at(12.5));
        assert_eq!(bar, 3);
        assert!((beat - 1.5).abs() < 1e-9);
        // 6/8 has three quarter-note beats per bar.
        let m = TempoMap::new(48_000.0, &[seg(0.0, 120.0, [6, 8])]);
        assert_eq!(m.bar_at(m.sample_at(3.0)), (1, 0.0));
    }

    #[test]
    fn huge_beats_saturate_instead_of_overflowing() {
        let m = TempoMap::new(48_000.0, &[seg(0.0, 120.0, [4, 4]), seg(1e12, 1.0, [4, 4])]);
        assert_eq!(m.sample_at(1e300), u64::MAX);
        assert!(m.beat_at(u64::MAX).is_finite());
    }

    #[test]
    fn invalid_maps_are_repaired() {
        let m = TempoMap::new(48_000.0, &[seg(2.0, -5.0, [0, 0])]);
        assert_eq!(m.bpm_at(0), 120.0);
        let m = TempoMap::new(0.0, &[]);
        assert_eq!(m.sample_rate(), 48_000.0);
    }
}
