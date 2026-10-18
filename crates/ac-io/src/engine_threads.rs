//! Threads a live host runs next to the engine: background baking of
//! whole-input nodes (ARCHITECTURE §9) and the real-time worker pool of the
//! multi-core scheduler (§8). The engine crate builds for wasm and spawns
//! nothing itself; hosts use these.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ac_engine::{BakeJob, BakeResult, EngineHandle, RtPool};

/// How long parameter changes upstream of a baked node must settle before
/// it bakes again.
pub const BAKE_DEBOUNCE: Duration = Duration::from_millis(300);

struct Queue {
    jobs: Mutex<(VecDeque<BakeJob>, bool)>,
    ready: Condvar,
}

/// Runs bake jobs on a background thread, one at a time, newest first per
/// node. Call [`Baker::poll`] regularly (every UI frame, or every 100 ms).
pub struct Baker {
    queue: Arc<Queue>,
    results: mpsc::Receiver<BakeResult>,
    results_tx: mpsc::Sender<BakeResult>,
    started: bool,
    dirty_since: Option<Instant>,
    outstanding: usize,
}

impl std::fmt::Debug for Baker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Baker")
            .field("outstanding", &self.outstanding)
            .finish()
    }
}

impl Default for Baker {
    fn default() -> Self {
        Self::new()
    }
}

impl Baker {
    pub fn new() -> Baker {
        let (results_tx, results) = mpsc::channel();
        Baker {
            queue: Arc::new(Queue {
                jobs: Mutex::new((VecDeque::new(), false)),
                ready: Condvar::new(),
            }),
            results,
            results_tx,
            started: false,
            dirty_since: None,
            outstanding: 0,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let queue = self.queue.clone();
        let tx = self.results_tx.clone();
        let spawned = std::thread::Builder::new()
            .name("ac-bake".into())
            .spawn(move || bake_thread(&queue, &tx));
        if spawned.is_err() {
            self.started = false;
        }
    }

    /// Install finished bakes, rebake after parameter changes settle, and
    /// queue new jobs. Returns whether the engine recompiled (so the host's
    /// copy of the compile report is stale).
    pub fn poll(&mut self, handle: &mut EngineHandle) -> bool {
        let mut changed = false;
        while let Ok(r) = self.results.try_recv() {
            self.outstanding = self.outstanding.saturating_sub(1);
            changed |= handle.finish_bake(r);
        }
        if handle.bakes_dirty() {
            match self.dirty_since {
                None => self.dirty_since = Some(Instant::now()),
                Some(t) if t.elapsed() >= BAKE_DEBOUNCE => {
                    handle.refresh();
                    self.dirty_since = None;
                    changed = true;
                }
                Some(_) => {}
            }
        } else {
            self.dirty_since = None;
        }
        let jobs = handle.take_bake_jobs();
        if !jobs.is_empty() {
            self.start();
            if let Ok(mut q) = self.queue.jobs.lock() {
                for job in jobs {
                    // A newer job for the same node replaces a queued one.
                    let before = q.0.len();
                    q.0.retain(|j| j.path != job.path);
                    self.outstanding -= before - q.0.len();
                    q.0.push_back(job);
                    self.outstanding += 1;
                }
            }
            self.queue.ready.notify_one();
        }
        changed
    }

    /// Bakes queued or running.
    pub fn outstanding(&self) -> usize {
        self.outstanding
    }
}

impl Drop for Baker {
    fn drop(&mut self) {
        // Stop after the current job; don't wait for it (bakes can be long).
        if let Ok(mut q) = self.queue.jobs.lock() {
            q.0.clear();
            q.1 = true;
        }
        self.queue.ready.notify_all();
    }
}

fn bake_thread(queue: &Queue, tx: &mpsc::Sender<BakeResult>) {
    loop {
        let job = {
            let Ok(mut q) = queue.jobs.lock() else {
                return;
            };
            loop {
                if q.1 {
                    return;
                }
                if let Some(j) = q.0.pop_front() {
                    break j;
                }
                q = match queue.ready.wait(q) {
                    Ok(q) => q,
                    Err(_) => return,
                };
            }
        };
        if tx.send(job.run()).is_err() {
            return;
        }
    }
}

/// A real-time worker pool with `threads - 1` workers (the audio thread is
/// the other one), or `None` for one thread. Workers ask for real-time
/// priority where this crate can grant it (Linux; elsewhere they run at
/// normal priority).
pub fn worker_pool(threads: usize) -> Option<Arc<RtPool>> {
    if threads <= 1 {
        return None;
    }
    let pool = RtPool::new(
        threads - 1,
        Some(promote_worker),
        Some(crate::clock::now_ns),
    );
    (pool.workers() > 0).then(|| Arc::new(pool))
}

fn promote_worker() {
    let _ = crate::priority::promote_current_thread();
}

/// Threads to suggest for this machine: one per core, up to 8.
pub fn available_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{NodeId, NodeInstance, Project};
    use ac_engine::{Engine, EngineOptions, WholeState};

    fn reversed() -> Project {
        let mut p = Project::default();
        p.settings.length = 0.05;
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "oscillator").with_param("freq", 100.0));
        p.graph.nodes.push(NodeInstance::new(NodeId(2), "reverse"));
        p.graph.wires.push(ac_core::Wire::new(
            ac_core::Endpoint::new(NodeId(1), "out"),
            ac_core::Endpoint::new(NodeId(2), "in"),
        ));
        p
    }

    #[test]
    fn the_baker_bakes_in_the_background_and_debounces_param_changes() {
        let (engine, _) = Engine::new(
            reversed(),
            Arc::new(ac_nodes::registry()),
            Arc::new(ac_core::NoResources),
            EngineOptions::default(),
        );
        let (mut handle, _processor) = engine.split();
        let mut baker = Baker::new();
        let t0 = Instant::now();
        while handle.bakes()[0].state != WholeState::Baked && t0.elapsed().as_secs() < 10 {
            baker.poll(&mut handle);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(handle.bakes()[0].state, WholeState::Baked);
        assert_eq!(baker.outstanding(), 0);
        // A parameter change upstream rebakes once it has settled.
        handle.set_param(
            &ac_core::NodePath::root(NodeId(1)),
            "freq",
            ac_core::Value::Float(200.0),
        );
        baker.poll(&mut handle);
        assert_eq!(handle.bakes()[0].state, WholeState::Baked, "not yet");
        let t0 = Instant::now();
        let mut went_stale = false;
        while t0.elapsed().as_secs() < 10 {
            baker.poll(&mut handle);
            match handle.bakes()[0].state {
                WholeState::Stale => went_stale = true,
                WholeState::Baked if went_stale => break,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(went_stale);
        assert_eq!(handle.bakes()[0].state, WholeState::Baked);
    }

    #[test]
    fn one_thread_means_no_pool() {
        assert!(worker_pool(1).is_none());
        assert!(worker_pool(0).is_none());
        let p = worker_pool(3).unwrap();
        assert_eq!(p.workers(), 2);
        assert!(available_threads() >= 1);
    }
}
