//! Splitting a block at trigger offsets, for sample-accurate event handling.
//!
//! Nodes that react to several trigger inputs (start, stop, reset, sync...)
//! walk the block as a series of runs: apply the events at the run's first
//! frame, render up to the next event, repeat. This keeps the per-sample loop
//! free of trigger bookkeeping and allocates nothing.

/// One run of a block: the events at `at`, then frames `at..until` to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run<const N: usize> {
    /// First frame of the run; the events in `fired` happen here.
    pub at: usize,
    /// End of the run (exclusive).
    pub until: usize,
    /// Which of the merged trigger lists fired at `at`.
    pub fired: [bool; N],
}

/// Merges `N` trigger lists for one block and yields runs between events.
///
/// Offsets past the end of the block are treated as the last frame, and an
/// out-of-order offset is applied at the current frame, so malformed input
/// never loses an event or panics.
#[derive(Clone, Debug)]
pub(crate) struct Runs<'a, const N: usize> {
    lists: [&'a [u32]; N],
    cursors: [usize; N],
    pos: usize,
    frames: usize,
    done: bool,
}

impl<'a, const N: usize> Runs<'a, N> {
    pub fn new(lists: [&'a [u32]; N], frames: usize) -> Self {
        Runs {
            lists,
            cursors: [0; N],
            pos: 0,
            frames,
            done: frames == 0,
        }
    }
}

impl<const N: usize> Iterator for Runs<'_, N> {
    type Item = Run<N>;

    fn next(&mut self) -> Option<Run<N>> {
        if self.done {
            return None;
        }
        let at = self.pos;
        let mut fired = [false; N];
        let mut until = self.frames;
        for ((list, cursor), f) in self.lists.iter().zip(&mut self.cursors).zip(&mut fired) {
            while let Some(&o) = list.get(*cursor) {
                let o = (o as usize).min(self.frames.saturating_sub(1));
                if o <= at {
                    *f = true;
                    *cursor += 1;
                } else {
                    until = until.min(o);
                    break;
                }
            }
        }
        self.pos = until;
        self.done = until >= self.frames;
        Some(Run { at, until, fired })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_events_is_one_run() {
        let runs: Vec<_> = Runs::new([&[][..], &[]], 64).collect();
        assert_eq!(
            runs,
            vec![Run {
                at: 0,
                until: 64,
                fired: [false, false]
            }]
        );
    }

    #[test]
    fn events_split_the_block_in_order() {
        let a: &[u32] = &[0, 10];
        let b: &[u32] = &[10, 30];
        let runs: Vec<_> = Runs::new([a, b], 64).collect();
        assert_eq!(runs.len(), 3);
        assert_eq!(
            (runs[0].at, runs[0].until, runs[0].fired),
            (0, 10, [true, false])
        );
        assert_eq!(
            (runs[1].at, runs[1].until, runs[1].fired),
            (10, 30, [true, true])
        );
        assert_eq!(
            (runs[2].at, runs[2].until, runs[2].fired),
            (30, 64, [false, true])
        );
    }

    #[test]
    fn bad_offsets_are_clamped_not_lost() {
        let a: &[u32] = &[5, 3, 900];
        let runs: Vec<_> = Runs::new([a], 8).collect();
        // 5 fires at 5; 3 is out of order and fires at once; 900 lands on 7.
        let fired_at: Vec<usize> = runs.iter().filter(|r| r.fired[0]).map(|r| r.at).collect();
        assert_eq!(fired_at, vec![5, 7]);
        assert_eq!(runs.last().map(|r| r.until), Some(8));
    }

    #[test]
    fn empty_block_yields_nothing() {
        assert_eq!(Runs::new([&[1u32][..]], 0).count(), 0);
    }
}
