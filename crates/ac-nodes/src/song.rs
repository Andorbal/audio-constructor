//! Generators placed in the song (DECISIONS D206): a generator whose `start`
//! is *At beat…* plays from its `at` input's song beat until its `until`
//! (inputs that follow `start`; see `Signature::song_generator_inputs`).

use ac_core::{Io, ProcessCtx, SongLock};

/// Where the generator with `start` at input `start` (and `at`, `until`
/// right after it) should be this block; [`SongLock::Hold`] when it isn't
/// placed in the song.
pub(crate) fn lock(ctx: &ProcessCtx, io: &Io, start: usize) -> SongLock {
    if !io.inputs.song_locked(start) {
        return SongLock::Hold;
    }
    let beat = |i: usize| f64::from(io.inputs.float(i).first());
    ctx.transport
        .song_lock(ctx.frames, beat(start + 1), beat(start + 2))
}

/// The start and stop triggers to play: the inputs' own, or the song's
/// start or stop where `lock` has one (written into `frames`).
pub(crate) fn triggers<'a>(
    lock: SongLock,
    start: &'a [u32],
    stop: &'a [u32],
    frames: &'a mut [u32; 2],
) -> (&'a [u32], &'a [u32]) {
    let (a, b) = frames.split_at_mut(1);
    match lock {
        SongLock::Start(at) => {
            a[0] = at as u32;
            (a, stop)
        }
        SongLock::Stop(at) => {
            b[0] = at as u32;
            (start, b)
        }
        _ => (start, stop),
    }
}
