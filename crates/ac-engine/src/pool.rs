//! A real-time worker pool for the multi-core scheduler (ARCHITECTURE §8).
//!
//! The audio thread hands a slice of jobs to [`RtPool::run`] and works on it
//! too; workers and the caller claim jobs one at a time from a shared
//! counter. `run` returns once every job has finished, so jobs may borrow
//! data that lives only for the block.
//!
//! Real-time rules: the caller never allocates, locks, sleeps, or makes a
//! system call. Idle workers spin, then yield, then sleep briefly; the caller
//! never waits for a sleeping worker, because it only waits for jobs that
//! some thread has already claimed. A worker that wakes late finds the jobs
//! taken and goes back to waiting. (DECISIONS D57.)
//!
//! This module holds the engine's only `unsafe` code: handing a borrowed,
//! type-erased job slice to persistent threads. Scoped threads would avoid
//! it, but spawning per block allocates and makes system calls on the audio
//! thread; a lock-free hand-off to persistent threads needs the erasure.

#![allow(unsafe_code)]

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;

/// A unit of work run on some thread of the pool.
pub trait Job: Send {
    fn run(&mut self);
}

/// Most worker threads a pool will start.
pub const MAX_WORKERS: usize = 31;

/// Spins before an idle worker starts yielding, and yields before it starts
/// sleeping. A block's waves arrive microseconds apart, so spinning covers
/// them; yielding covers the gap between blocks.
const SPINS: u32 = 1 << 14;
const YIELDS: u32 = 1 << 12;
const SLEEP: std::time::Duration = std::time::Duration::from_micros(250);

/// Type-erased pointer to the current round's job slice.
#[derive(Clone, Copy)]
struct Round {
    base: *mut (),
    call: unsafe fn(*mut (), usize),
}

struct Shared {
    /// The open round: round number in bits 32..64, job count in 16..32,
    /// next unclaimed job in 0..16. One word, so a claim checks all three
    /// at once.
    state: AtomicU64,
    /// Written only by the caller of `run`, between rounds (see `run`).
    round: UnsafeCell<Round>,
    /// Jobs of the current round that have finished.
    done: AtomicUsize,
    shutdown: AtomicBool,
    /// Nanoseconds each thread (0 = caller) spent running jobs, since last read.
    busy: Box<[AtomicU64]>,
    clock: Option<fn() -> u64>,
    debug_guard: bool,
}

// SAFETY: `round` is the only non-Sync field. It is written by the thread
// calling `run` only while no round is open (before publishing the new
// round with a Release store of `state`, after every job of the previous
// round has finished), and read by workers only after they claim a job of
// the open round with an Acquire compare-exchange on `state`. A claimed,
// unfinished job keeps the round open, so the caller can't be writing
// `round` during such a read, and the read happens-after the write.
unsafe impl Sync for Shared {}
// SAFETY: the raw pointer in `round` refers to a job slice that is only
// dereferenced under the protocol above; it is never dropped through.
unsafe impl Send for Shared {}

/// A pool of real-time worker threads. Dropping it tells the workers to stop
/// without waiting for them (never block the thread that drops it).
pub struct RtPool {
    shared: Arc<Shared>,
    workers: usize,
}

impl std::fmt::Debug for RtPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtPool")
            .field("workers", &self.workers)
            .finish()
    }
}

unsafe fn call_job<J: Job>(base: *mut (), i: usize) {
    // SAFETY: `base` points to a live `[J]` of more than `i` elements (see
    // `RtPool::run`), and index `i` is claimed by exactly one thread, so this
    // is the only reference to element `i`.
    unsafe { (*base.cast::<J>().add(i)).run() }
}

unsafe fn no_job(_: *mut (), _: usize) {}

impl RtPool {
    /// Start `workers` threads (at most [`MAX_WORKERS`]). `on_start` runs
    /// first on each worker, e.g. to ask for real-time priority. `clock`
    /// (nanoseconds) enables per-thread load measurement. Threads that fail
    /// to start (wasm, resource limits) are left out.
    pub fn new(workers: usize, on_start: Option<fn()>, clock: Option<fn() -> u64>) -> RtPool {
        let workers = workers.min(MAX_WORKERS);
        let shared = Arc::new(Shared {
            state: AtomicU64::new(0),
            round: UnsafeCell::new(Round {
                base: std::ptr::null_mut(),
                call: no_job,
            }),
            done: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            busy: (0..=workers).map(|_| AtomicU64::new(0)).collect(),
            clock,
            debug_guard: cfg!(debug_assertions),
        });
        let mut started = 0;
        for w in 0..workers {
            let s = shared.clone();
            let spawned = thread::Builder::new()
                .name(format!("ac-worker-{w}"))
                .spawn(move || {
                    if let Some(f) = on_start {
                        f();
                    }
                    worker_loop(&s, w + 1);
                });
            if spawned.is_ok() {
                started += 1;
            }
        }
        RtPool {
            shared,
            workers: started,
        }
    }

    /// Worker threads running (the caller of `run` makes one more).
    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Run every job once, on the caller and the workers, and return when
    /// all have finished. Real-time safe.
    pub fn run<J: Job>(&self, jobs: &mut [J]) {
        let n = jobs.len();
        if n == 0 {
            return;
        }
        if self.workers == 0 || n == 1 || n > MAX_ROUND {
            let t0 = self.shared.clock.map(|c| c());
            for j in jobs.iter_mut() {
                j.run();
            }
            self.add_busy(0, t0);
            return;
        }
        let s = &*self.shared;
        let epoch = (s.state.load(Ordering::Relaxed) >> 32).wrapping_add(1) & 0xffff_ffff;
        let open = (epoch << 32) | ((n as u64) << 16);
        s.done.store(0, Ordering::Relaxed);
        // SAFETY: no round is open (the previous `run` returned only after
        // all its jobs finished, and nothing else opens rounds: `run` takes
        // `&self` but the engine calls it from the audio thread only), so no
        // worker reads `round` now.
        unsafe {
            *s.round.get() = Round {
                base: jobs.as_mut_ptr().cast(),
                call: call_job::<J>,
            };
        }
        // Opens the round; Release publishes `round` and `done`.
        s.state.store(open, Ordering::Release);
        // Wait for every claimed job even if one of ours panics, so no
        // worker is left holding a pointer into `jobs`.
        let wait = WaitAll { shared: s, n };
        let t0 = s.clock.map(|c| c());
        while let Some(i) = claim(s, epoch) {
            // SAFETY: `i < n` was claimed by this thread alone.
            unsafe { call_job::<J>(jobs.as_mut_ptr().cast(), i) };
            s.done.fetch_add(1, Ordering::Release);
        }
        self.add_busy(0, t0);
        drop(wait);
    }

    fn add_busy(&self, thread: usize, t0: Option<u64>) {
        if let (Some(t0), Some(c)) = (t0, self.shared.clock)
            && let Some(b) = self.shared.busy.get(thread)
        {
            b.fetch_add(c().saturating_sub(t0), Ordering::Relaxed);
        }
    }

    /// Nanoseconds each thread (0 = the caller, then each worker) spent on
    /// jobs since the last call, written into `out`. Real-time safe.
    pub fn take_busy(&self, out: &mut [u64]) {
        for (o, b) in out.iter_mut().zip(self.shared.busy.iter()) {
            *o = b.swap(0, Ordering::Relaxed);
        }
    }
}

impl Drop for RtPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
    }
}

/// Waits (spinning) until `n` jobs of the open round have finished.
struct WaitAll<'a> {
    shared: &'a Shared,
    n: usize,
}

impl Drop for WaitAll<'_> {
    fn drop(&mut self) {
        while self.shared.done.load(Ordering::Acquire) < self.n {
            std::hint::spin_loop();
        }
    }
}

/// Jobs per round that fit the claim counter.
const MAX_ROUND: usize = 0xffff;

/// Claim the next job of round `epoch`, if any is left.
fn claim(s: &Shared, epoch: u64) -> Option<usize> {
    let mut cur = s.state.load(Ordering::Acquire);
    loop {
        if cur >> 32 != epoch {
            return None;
        }
        let n = ((cur >> 16) & 0xffff) as usize;
        let i = (cur & 0xffff) as usize;
        if i >= n {
            return None;
        }
        match s
            .state
            .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => return Some(i),
            Err(now) => cur = now,
        }
    }
}

/// Counts a finished job even if it panicked, so `run` never waits forever.
struct Finish<'a>(&'a Shared);

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        self.0.done.fetch_add(1, Ordering::Release);
    }
}

fn worker_loop(s: &Shared, index: usize) {
    let mut seen = 0u64;
    let mut idle = 0u32;
    loop {
        if s.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let state = s.state.load(Ordering::Acquire);
        let epoch = state >> 32;
        if epoch != seen {
            seen = epoch;
            idle = 0;
            while let Some(i) = claim(s, epoch) {
                // SAFETY: the claim succeeded in the open round, so `round`
                // was written before the round opened and won't change until
                // this job finishes (the caller waits for it).
                let round = unsafe { *s.round.get() };
                let finish = Finish(s);
                let t0 = s.clock.map(|c| c());
                if s.debug_guard {
                    assert_no_alloc::assert_no_alloc(|| {
                        // SAFETY: job `i` of the open round was claimed by this thread alone.
                        unsafe { (round.call)(round.base, i) }
                    });
                } else {
                    // SAFETY: as above.
                    unsafe { (round.call)(round.base, i) };
                }
                // Count the time before signalling, so the caller sees it.
                if let (Some(t0), Some(c)) = (t0, s.clock)
                    && let Some(b) = s.busy.get(index)
                {
                    b.fetch_add(c().saturating_sub(t0), Ordering::Relaxed);
                }
                drop(finish);
            }
            continue;
        }
        idle = idle.saturating_add(1);
        if idle < SPINS {
            std::hint::spin_loop();
        } else if idle < SPINS + YIELDS {
            thread::yield_now();
        } else {
            thread::sleep(SLEEP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    struct Add<'a> {
        hits: &'a [AtomicU32],
        i: usize,
        out: &'a mut u64,
    }

    impl Job for Add<'_> {
        fn run(&mut self) {
            self.hits[self.i].fetch_add(1, Ordering::Relaxed);
            *self.out = (self.i as u64 + 1) * 3;
        }
    }

    #[test]
    fn every_job_runs_exactly_once_across_many_rounds() {
        let pool = RtPool::new(3, None, None);
        assert_eq!(pool.workers(), 3);
        for round in 0..2_000 {
            let n = 1 + round % 17;
            let hits: Vec<AtomicU32> = (0..n).map(|_| AtomicU32::new(0)).collect();
            let mut outs = vec![0u64; n];
            let mut jobs: Vec<Add<'_>> = outs
                .iter_mut()
                .enumerate()
                .map(|(i, out)| Add {
                    hits: &hits,
                    i,
                    out,
                })
                .collect();
            pool.run(&mut jobs);
            drop(jobs);
            assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
            for (i, o) in outs.iter().enumerate() {
                assert_eq!(*o, (i as u64 + 1) * 3);
            }
        }
    }

    #[test]
    fn a_pool_without_workers_runs_jobs_inline() {
        let pool = RtPool::new(0, None, None);
        let hits: Vec<AtomicU32> = (0..4).map(|_| AtomicU32::new(0)).collect();
        let mut outs = [0u64; 4];
        let mut jobs: Vec<Add<'_>> = outs
            .iter_mut()
            .enumerate()
            .map(|(i, out)| Add {
                hits: &hits,
                i,
                out,
            })
            .collect();
        pool.run(&mut jobs);
        assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
    }

    #[test]
    fn busy_time_is_measured_per_thread() {
        fn clock() -> u64 {
            use std::time::Instant;
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            START.get_or_init(Instant::now).elapsed().as_nanos() as u64
        }
        struct Spin;
        impl Job for Spin {
            fn run(&mut self) {
                let t = std::time::Instant::now();
                while t.elapsed() < std::time::Duration::from_micros(200) {
                    std::hint::spin_loop();
                }
            }
        }
        let pool = RtPool::new(2, None, Some(clock));
        let mut jobs: Vec<Spin> = (0..12).map(|_| Spin).collect();
        pool.run(&mut jobs);
        let mut busy = [0u64; 3];
        pool.take_busy(&mut busy);
        let total: u64 = busy.iter().sum();
        assert!(total >= 12 * 200_000, "{busy:?}");
        pool.take_busy(&mut busy);
        assert_eq!(busy, [0, 0, 0]);
    }
}
