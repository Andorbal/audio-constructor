//! The plugin's control thread: while the host has the plugin active it
//! looks after the engine every [`TICK`] (dropping graphs the audio thread
//! retired, which must not happen there) and bakes whole-input nodes, so
//! a plugin with no editor open behaves like the app.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::session::{Shared, TICK};

#[derive(Debug, Default)]
struct Stop {
    stop: Mutex<bool>,
    wake: Condvar,
}

impl Stop {
    fn requested(&self) -> bool {
        self.stop.lock().map_or(true, |s| *s)
    }
}

/// A running control thread; stopping (or dropping) it waits for it.
#[derive(Debug)]
pub(crate) struct ControlThread {
    stop: Arc<Stop>,
    thread: Option<JoinHandle<()>>,
}

impl ControlThread {
    pub(crate) fn start(shared: Arc<Shared>) -> ControlThread {
        let stop = Arc::new(Stop::default());
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ac-plugin-control".into())
            .spawn(move || run(&shared, &s))
            .ok();
        ControlThread { stop, thread }
    }

    /// Stop after the current step (a bake in progress finishes first:
    /// bakes can't be interrupted, and the host may unload the plugin's
    /// code once this returns).
    pub(crate) fn stop(&mut self) {
        if let Ok(mut s) = self.stop.stop.lock() {
            *s = true;
        }
        self.stop.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ControlThread {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(shared: &Shared, stop: &Stop) {
    loop {
        {
            let Ok(guard) = stop.stop.lock() else { return };
            let Ok((guard, _)) = stop.wake.wait_timeout_while(guard, TICK, |s| !*s) else {
                return;
            };
            if *guard {
                return;
            }
        }
        let jobs = shared.lock().tick();
        for job in jobs {
            if stop.requested() {
                return;
            }
            // Outside the lock: a bake can take a while.
            let result = job.run();
            shared.lock().finish_bake(result);
        }
    }
}
