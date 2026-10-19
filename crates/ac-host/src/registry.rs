//! The main-thread halves of running plugin instances, kept per thread
//! (DECISIONS D132).
//!
//! A plugin's main-thread half must stay on the thread that created it,
//! so each thread keeps its own list. The audio half (inside a `Plugin`
//! node) carries only an id and shared atomics. When the audio half is
//! dropped on its owner thread (the engine drops retired graphs on the
//! thread that edits them), the main half goes at once; dropped anywhere
//! else, it's marked orphaned and goes at the owner's next [`poll`].

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::clap::ClapInstance;
use crate::clap::host::Requests;

/// Something the host of the editor (or `audioc`) should act on, from
/// [`poll`].
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// A parameter moved in the plugin's window (or a preset loaded): set
    /// the node's input to match, as if it were dragged.
    ParamChanged {
        node_key: String,
        socket: String,
        value: f64,
    },
    /// The plugin needs to start over (its latency, ports, or parameter
    /// list changed): rebuild the node. [`crate::PluginHost::prepare_restart`]
    /// keeps its state for the new instance.
    Restart { node_key: String },
    /// The plugin's state changed in a way the project should save.
    StateChanged { node_key: String },
    /// The plugin's window closed.
    WindowClosed { node_key: String },
}

pub(crate) enum MainHalf {
    Clap(Box<ClapInstance>),
}

pub(crate) struct Entry {
    pub id: u64,
    pub node_key: String,
    /// Window title.
    pub title: String,
    /// Socket names of the description's parameters, by index.
    pub sockets: Vec<String>,
    pub requests: Arc<Requests>,
    /// Parameter moves the audio half saw the plugin make.
    pub from_audio: rtrb::Consumer<(u32, f64)>,
    pub half: MainHalf,
}

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn insert(entry: Entry) {
    ENTRIES.with(|e| e.borrow_mut().push(entry));
}

/// Drop instance `id`'s main half (its audio half is gone).
pub(crate) fn release(id: u64) {
    let gone = ENTRIES.with(|e| {
        let mut list = e.borrow_mut();
        list.iter().position(|x| x.id == id).map(|i| list.remove(i))
    });
    // Dropped outside the borrow: a plugin may call back while it's
    // destroyed.
    drop(gone);
}

/// Run `f` on the newest instance for `node_key` on this thread.
pub(crate) fn with<R>(node_key: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    ENTRIES.with(|e| {
        let mut list = e.borrow_mut();
        list.iter_mut()
            .filter(|x| x.node_key == node_key && !x.requests.orphaned.load(Ordering::Acquire))
            .max_by_key(|x| x.id)
            .map(f)
    })
}

/// Node keys of the plugins running on this thread, oldest first.
pub fn running() -> Vec<String> {
    ENTRIES.with(|e| {
        let mut keys: Vec<String> = Vec::new();
        for x in e.borrow().iter() {
            if !x.requests.orphaned.load(Ordering::Acquire) && !keys.contains(&x.node_key) {
                keys.push(x.node_key.clone());
            }
        }
        keys
    })
}

/// Answer what this thread's plugins asked for: their main-thread
/// callbacks, timers, and file descriptors; parameter moves and restarts
/// to report; windows to resize or close; orphans to destroy. Call it
/// regularly on the thread that compiles the graph (every editor frame,
/// or every few milliseconds in `audioc run`).
pub fn poll() -> Vec<HostEvent> {
    let mut events = Vec::new();
    let orphans: Vec<Entry> = ENTRIES.with(|e| {
        let mut list = e.borrow_mut();
        let mut orphans = Vec::new();
        let mut i = 0;
        while i < list.len() {
            if list[i].requests.orphaned.load(Ordering::Acquire) {
                orphans.push(list.remove(i));
            } else {
                i += 1;
            }
        }
        orphans
    });
    drop(orphans);
    // Plugin callbacks can re-enter the host (and this module), so each
    // entry is taken out of the list while it's serviced.
    let ids: Vec<u64> = ENTRIES.with(|e| e.borrow().iter().map(|x| x.id).collect());
    for id in ids {
        let taken = ENTRIES.with(|e| {
            let mut list = e.borrow_mut();
            list.iter().position(|x| x.id == id).map(|i| list.remove(i))
        });
        let Some(mut entry) = taken else { continue };
        service(&mut entry, &mut events);
        ENTRIES.with(|e| e.borrow_mut().push(entry));
    }
    ENTRIES.with(|e| e.borrow_mut().sort_by_key(|x| x.id));
    events
}

fn service(entry: &mut Entry, events: &mut Vec<HostEvent>) {
    let r = entry.requests.clone();
    let key = entry.node_key.clone();
    match &mut entry.half {
        MainHalf::Clap(c) => {
            if r.callback.swap(false, Ordering::AcqRel) {
                c.instance.call_on_main_thread_callback();
            }
            c.service();
            if r.rescan_values.swap(false, Ordering::AcqRel) {
                for (i, value) in c.values() {
                    if let Some(socket) = entry.sockets.get(i) {
                        events.push(HostEvent::ParamChanged {
                            node_key: key.clone(),
                            socket: socket.clone(),
                            value,
                        });
                    }
                }
            }
            if crate::gui::service_clap(c, &r) {
                events.push(HostEvent::WindowClosed {
                    node_key: key.clone(),
                });
            }
        }
    }
    while let Ok((index, value)) = entry.from_audio.pop() {
        if let Some(socket) = entry.sockets.get(index as usize) {
            // Only the last move of a parameter matters.
            events.retain(|e| {
                !matches!(e, HostEvent::ParamChanged { node_key, socket: s, .. }
                    if *node_key == key && s == socket)
            });
            events.push(HostEvent::ParamChanged {
                node_key: key.clone(),
                socket: socket.clone(),
                value,
            });
        }
    }
    if r.restart.swap(false, Ordering::AcqRel) | r.latency.swap(false, Ordering::AcqRel) {
        events.push(HostEvent::Restart {
            node_key: key.clone(),
        });
    }
    if r.dirty.swap(false, Ordering::AcqRel) {
        events.push(HostEvent::StateChanged { node_key: key });
    }
}

/// The saved state of the newest instance for `node_key` on this thread.
pub(crate) fn save_state(node_key: &str) -> Option<Vec<u8>> {
    with(node_key, |e| match &mut e.half {
        MainHalf::Clap(c) => c.save_state(),
    })
    .flatten()
}

/// Destroy every instance on this thread whose audio half is gone, and
/// close every window. Instances still in a running graph stay (their
/// graph owns them); call this after dropping the engine to tear down.
pub fn shutdown() {
    crate::gui::close_all();
    let _ = poll();
}
