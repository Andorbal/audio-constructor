//! Auditioning: a short recording (a block's preview) played over whatever
//! the engine is playing, on the same output (DECISIONS D193).
//!
//! The control side ([`Audition`]) hands the audio thread a clip already
//! at the stream's rate in a box, over a lock-free ring; the device
//! callback ([`AuditionTap::mix`], called by the stream after the engine)
//! adds it into the output and hands the box back when it's done or
//! replaced, so the clip is freed off the audio thread. Nothing on the
//! audio side allocates, frees, locks, or waits.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// Clips that can be on their way back at once (the control thread
/// collects them every frame).
const RETURN_CAPACITY: usize = 8;

/// Audio to audition: non-interleaved channels at the stream's rate.
struct Clip {
    id: u64,
    channels: Vec<Vec<f32>>,
    gain: f32,
}

enum Command {
    Play(Box<Clip>),
    Stop,
}

/// What the audio side reports.
#[derive(Default)]
struct Shared {
    /// The id of the clip playing, 0 for none.
    playing: AtomicU64,
    /// Frames of it played.
    position: AtomicU64,
    /// The newest clip that finished, was replaced, or was stopped.
    done: AtomicU64,
}

/// The control end: start and stop clips, see what's playing.
pub struct Audition {
    tx: Producer<Command>,
    back: Consumer<Box<Clip>>,
    shared: Arc<Shared>,
    next: u64,
    /// Frames in the clip last started, by id.
    length: (u64, u64),
}

/// The audio end, which a stream's callback mixes into its output.
pub struct AuditionTap {
    rx: Consumer<Command>,
    back: Producer<Box<Clip>>,
    current: Option<Box<Clip>>,
    /// A finished clip the return ring had no room for yet.
    returning: Option<Box<Clip>>,
    pos: usize,
    shared: Arc<Shared>,
}

impl fmt::Debug for Audition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Audition")
            .field("playing", &self.playing())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AuditionTap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuditionTap")
            .field("playing", &self.current.as_ref().map(|c| c.id))
            .finish_non_exhaustive()
    }
}

/// A connected pair.
pub fn audition() -> (Audition, AuditionTap) {
    let (tx, rx) = RingBuffer::new(4);
    let (back_tx, back) = RingBuffer::new(RETURN_CAPACITY);
    let shared = Arc::new(Shared::default());
    (
        Audition {
            tx,
            back,
            shared: shared.clone(),
            next: 1,
            length: (0, 0),
        },
        AuditionTap {
            rx,
            back: back_tx,
            current: None,
            returning: None,
            pos: 0,
            shared,
        },
    )
}

impl Audition {
    /// Play `channels` (at the stream's rate) at `gain`, replacing whatever
    /// was auditioning. Returns the clip's id, or `None` if the audio side
    /// is behind on its queue (try again next frame).
    pub fn play(&mut self, channels: Vec<Vec<f32>>, gain: f32) -> Option<u64> {
        self.collect_garbage();
        let id = self.next;
        let frames = channels.first().map_or(0, Vec::len) as u64;
        let clip = Box::new(Clip {
            id,
            channels,
            gain: if gain.is_finite() { gain } else { 0.0 },
        });
        self.tx.push(Command::Play(clip)).ok()?;
        self.next += 1;
        self.length = (id, frames);
        Some(id)
    }

    /// Stop auditioning.
    pub fn stop(&mut self) {
        let _ = self.tx.push(Command::Stop);
    }

    /// The id of the clip playing, if one is.
    pub fn playing(&self) -> Option<u64> {
        match self.shared.playing.load(Ordering::Acquire) {
            0 => None,
            id => Some(id),
        }
    }

    /// Whether clip `id` has played to its end, or been replaced or
    /// stopped (false until the audio side has taken it).
    pub fn is_done(&self, id: u64) -> bool {
        self.shared.done.load(Ordering::Acquire) >= id
    }

    /// How far into clip `id` playback is, 0 to 1 (0 if it isn't playing).
    pub fn progress(&self, id: u64) -> f32 {
        if self.playing() != Some(id) || self.length.0 != id || self.length.1 == 0 {
            return 0.0;
        }
        let pos = self.shared.position.load(Ordering::Relaxed);
        (pos as f64 / self.length.1 as f64).clamp(0.0, 1.0) as f32
    }

    /// Free clips the audio side is done with.
    pub fn collect_garbage(&mut self) {
        while let Ok(clip) = self.back.pop() {
            drop(clip);
        }
    }
}

impl AuditionTap {
    /// Add the auditioned clip's next `frames` frames to channel-major
    /// `out` (`channels` channels of `frames` each): a mono clip into the
    /// first two channels, a stereo one channel by channel. Real-time safe.
    pub fn mix(&mut self, out: &mut [f32], channels: usize, frames: usize) {
        self.flush_returning();
        if self.returning.is_none() {
            while let Ok(cmd) = self.rx.pop() {
                let next = match cmd {
                    Command::Play(clip) => Some(clip),
                    Command::Stop => None,
                };
                if let Some(old) = self.current.take() {
                    self.shared.done.fetch_max(old.id, Ordering::AcqRel);
                    self.give_back(old);
                }
                self.current = next;
                self.pos = 0;
                if self.returning.is_some() {
                    // No room to hand back another: leave the rest queued.
                    break;
                }
            }
        }
        let Some(clip) = self.current.as_ref() else {
            self.shared.playing.store(0, Ordering::Release);
            return;
        };
        let len = clip.channels.first().map_or(0, Vec::len);
        let n = frames.min(len.saturating_sub(self.pos));
        let src_channels = clip.channels.len();
        if src_channels > 0 {
            for c in 0..channels.min(2) {
                let src = &clip.channels[c % src_channels];
                let (Some(dst), Some(src)) = (
                    out.get_mut(c * frames..c * frames + n),
                    src.get(self.pos..self.pos + n),
                ) else {
                    continue;
                };
                for (d, s) in dst.iter_mut().zip(src) {
                    *d += s * clip.gain;
                }
            }
        }
        self.pos += n;
        if self.pos >= len {
            self.shared.playing.store(0, Ordering::Release);
            if let Some(done) = self.current.take() {
                self.shared.done.fetch_max(done.id, Ordering::AcqRel);
                self.give_back(done);
            }
        } else {
            self.shared.playing.store(clip.id, Ordering::Release);
            self.shared
                .position
                .store(self.pos as u64, Ordering::Relaxed);
        }
    }

    fn give_back(&mut self, clip: Box<Clip>) {
        if let Err(rtrb::PushError::Full(clip)) = self.back.push(clip) {
            // Keep it (never free on the audio thread) until there's room.
            self.returning = Some(clip);
        }
    }

    fn flush_returning(&mut self) {
        if let Some(clip) = self.returning.take()
            && let Err(rtrb::PushError::Full(clip)) = self.back.push(clip)
        {
            self.returning = Some(clip);
        }
    }
}

#[cfg(test)]
mod tests {
    //! These run under the allocation guard (see `lib.rs`).

    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    fn mix(tap: &mut AuditionTap, frames: usize) -> Vec<f32> {
        let mut out = vec![0.5f32; 2 * frames];
        assert_no_alloc::assert_no_alloc(|| tap.mix(&mut out, 2, frames));
        out
    }

    #[test]
    fn a_clip_plays_over_the_output_and_ends() {
        let (mut a, mut tap) = audition();
        assert_eq!(mix(&mut tap, 4), vec![0.5; 8], "nothing playing");
        let id = a
            .play(vec![vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]], 1.0)
            .unwrap();
        let out = mix(&mut tap, 4);
        // Mono into both channels, added to what was there.
        assert!(close(&out[..4], &[0.6, 0.7, 0.8, 0.9]), "{out:?}");
        assert!(close(&out[4..], &[0.6, 0.7, 0.8, 0.9]), "{out:?}");
        assert_eq!(a.playing(), Some(id));
        assert!((a.progress(id) - 4.0 / 6.0).abs() < 1e-6);
        let out = mix(&mut tap, 4);
        assert!(close(&out[..4], &[1.0, 1.1, 0.5, 0.5]), "{out:?}");
        assert_eq!(a.playing(), None);
        assert!(a.is_done(id));
        // The box came back to be freed here.
        a.collect_garbage();
        assert!(a.back.is_empty());
    }

    #[test]
    fn a_new_clip_replaces_the_old_and_stop_stops() {
        let (mut a, mut tap) = audition();
        a.play(vec![vec![1.0; 100], vec![-1.0; 100]], 0.5).unwrap();
        let out = mix(&mut tap, 2);
        assert!(close(&out, &[1.0, 1.0, 0.0, 0.0]), "{out:?}");
        let b = a.play(vec![vec![0.25; 100]], 1.0).unwrap();
        assert!(!a.is_done(b), "not taken yet");
        mix(&mut tap, 2);
        assert_eq!(a.playing(), Some(b));
        assert!(a.is_done(b - 1), "the first was replaced");
        assert!(!a.is_done(b));
        a.stop();
        assert_eq!(mix(&mut tap, 2), vec![0.5; 4]);
        assert_eq!(a.playing(), None);
        assert!(a.is_done(b));
        a.collect_garbage();
    }

    #[test]
    fn a_full_return_ring_holds_clips_instead_of_freeing_them() {
        let (mut a, mut tap) = audition();
        // Many short clips the control side never collects.
        for _ in 0..(RETURN_CAPACITY + 3) {
            let _ = a.tx.push(Command::Play(Box::new(Clip {
                id: 9,
                channels: vec![vec![0.0; 1]],
                gain: 1.0,
            })));
            mix(&mut tap, 4);
        }
        assert!(tap.returning.is_some() || tap.back.is_full());
        // Collecting frees the way again.
        a.collect_garbage();
        mix(&mut tap, 4);
        a.collect_garbage();
    }
}
