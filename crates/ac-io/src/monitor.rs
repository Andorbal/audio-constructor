//! The last few messages a device thread received, kept for the editor
//! (ARCHITECTURE §11, DECISIONS D115).
//!
//! The engine only ever sees hashed OSC addresses and raw MIDI, which is
//! what makes receiving allocation-free. The device threads see the whole
//! message before that, so they keep the newest [`RECENT_MESSAGES`] behind
//! a mutex for the Devices tab to show: what a control surface really
//! sends, as the person needs to type it. Nothing here runs on the audio
//! thread.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// How many messages a device keeps for the editor.
pub const RECENT_MESSAGES: usize = 64;

/// A bounded, shared log of the newest messages; cloning shares the log.
#[derive(Debug)]
pub struct Recent<T> {
    inner: Arc<Mutex<VecDeque<T>>>,
    capacity: usize,
}

impl<T> Clone for Recent<T> {
    fn clone(&self) -> Self {
        Recent {
            inner: self.inner.clone(),
            capacity: self.capacity,
        }
    }
}

impl<T> Default for Recent<T> {
    fn default() -> Self {
        Recent::new(RECENT_MESSAGES)
    }
}

impl<T> Recent<T> {
    pub fn new(capacity: usize) -> Recent<T> {
        Recent {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity: capacity.max(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<T>> {
        // A thread that panicked while holding the lock left a plain
        // queue behind; nothing in it can be half-written.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keep `item`, dropping the oldest when full.
    pub fn push(&self, item: T) {
        let mut q = self.lock();
        if q.len() >= self.capacity {
            q.pop_front();
        }
        q.push_back(item);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

impl<T: Clone> Recent<T> {
    /// Everything kept, oldest first.
    pub fn snapshot(&self) -> Vec<T> {
        self.lock().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_in_order() {
        let r = Recent::new(3);
        assert!(r.is_empty());
        for i in 0..5 {
            r.push(i);
        }
        assert_eq!(r.snapshot(), vec![2, 3, 4]);
        assert_eq!(r.len(), 3);
        r.clear();
        assert!(r.snapshot().is_empty());
    }

    #[test]
    fn clones_share_the_log() {
        let a: Recent<u8> = Recent::default();
        let b = a.clone();
        b.push(7);
        assert_eq!(a.snapshot(), vec![7]);
    }
}
