//! Telemetry: values the audio thread publishes for the UI (meters, scopes,
//! socket levels, timings) through relaxed atomics (DECISIONS D22). The UI
//! reads at frame rate; nothing on the audio thread ever waits.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use ac_core::NodePath;

pub(crate) fn atomics(n: usize) -> Box<[AtomicU32]> {
    (0..n).map(|_| AtomicU32::new(0)).collect()
}

fn load_f32(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

/// Per-graph telemetry storage. Replaced (with a new layout) on every compile.
#[derive(Debug)]
pub struct TelemetryStore {
    pub(crate) node_values: Box<[AtomicU32]>,
    pub(crate) scope: Box<[AtomicU32]>,
    pub(crate) scope_pos: Box<[AtomicU32]>,
    /// Peak level of each output socket, with ~300 ms decay.
    pub(crate) socket_level: Box<[AtomicU32]>,
    /// Last value of each output socket (last frame for sample-rate outputs).
    pub(crate) socket_value: Box<[AtomicU32]>,
    /// Running count of events emitted by each event output.
    pub(crate) socket_events: Box<[AtomicU32]>,
    /// Processing time of each node in the last host block, in
    /// nanoseconds: every step of the node (a Feedback has two) in every
    /// sub-block the block was split into. Written only while timing is on.
    pub(crate) node_time: Box<[AtomicU32]>,
    /// The running sum for the block in progress, moved into `node_time`
    /// when the block ends so a reader never sees a half-counted block.
    pub(crate) node_time_acc: Box<[AtomicU32]>,
}

impl TelemetryStore {
    /// End a host block's timing: publish each node's sum and start over.
    /// Called by the processor once every step of the block has run.
    pub(crate) fn publish_node_times(&self) {
        for (acc, out) in self.node_time_acc.iter().zip(self.node_time.iter()) {
            out.store(acc.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        }
    }
}

/// Where a node's telemetry lives in the store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeTelemetry {
    pub values: (usize, usize),
    pub scope: (usize, usize),
    pub scope_pos: Option<usize>,
    pub time: Option<usize>,
}

/// Maps node paths and sockets to telemetry slots.
#[derive(Clone, Debug, Default)]
pub struct TelemetryLayout {
    pub nodes: HashMap<NodePath, NodeTelemetry>,
    /// Output sockets by (node path, socket name); group instance outputs are
    /// aliases of the inner outputs that feed them.
    pub sockets: HashMap<(NodePath, String), usize>,
}

/// Engine-wide status shared across graph swaps.
#[derive(Debug, Default)]
pub struct EngineStatus {
    pub playing: AtomicBool,
    pub recording: AtomicBool,
    pub position: AtomicU64,
    pub sample_rate: AtomicU32,
    /// Blocks that took longer than their real-time budget.
    pub xruns: AtomicU64,
    /// Device-reported underruns/overruns (incremented by hosts).
    pub device_xruns: AtomicU64,
    /// Last block's processing time as a fraction of its budget (f32 bits).
    pub load: AtomicU32,
    pub master_peak: [AtomicU32; 2],
    pub blocks: AtomicU64,
    /// Parameter or event messages dropped because a ring was full.
    pub dropped_messages: AtomicU64,
    /// Measure per-node processing time (costs two clock reads per node).
    pub timing_enabled: AtomicBool,
    /// Frames in the last host block, which sets its real-time budget.
    pub block_frames: AtomicU32,
    /// Generation of the graph currently running.
    pub generation: AtomicU64,
    /// Threads running the graph (1 without a worker pool).
    pub cores: AtomicU32,
    /// Last block's busy time of each thread (0 = the audio thread) as a
    /// fraction of the block's budget (f32 bits).
    pub core_load: [AtomicU32; 32],
    /// Output latency of the running graph, in samples.
    pub latency: AtomicU32,
    /// Copy incoming MIDI and control events for the editor to learn from
    /// (`EngineHandle::set_input_monitor`).
    pub input_monitor: AtomicBool,
    /// A count-in is under way: the transport starts when it ends.
    pub counting_in: AtomicBool,
}

impl EngineStatus {
    pub fn load_fraction(&self) -> f32 {
        load_f32(&self.load)
    }

    pub fn master_peak(&self) -> [f32; 2] {
        [
            load_f32(&self.master_peak[0]),
            load_f32(&self.master_peak[1]),
        ]
    }

    /// The real-time budget of the last host block (its length), in
    /// nanoseconds; `None` before the first block.
    pub fn block_budget_ns(&self) -> Option<f64> {
        let frames = self.block_frames.load(Ordering::Relaxed);
        let rate = self.sample_rate.load(Ordering::Relaxed);
        (frames > 0 && rate > 0).then(|| f64::from(frames) / f64::from(rate) * 1e9)
    }

    pub fn total_xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed) + self.device_xruns.load(Ordering::Relaxed)
    }

    /// Load of each thread running the graph (audio thread first).
    pub fn core_loads(&self) -> Vec<f32> {
        let n = (self.cores.load(Ordering::Relaxed).max(1) as usize).min(self.core_load.len());
        self.core_load[..n].iter().map(load_f32).collect()
    }
}

/// Read-only view for the UI.
#[derive(Clone, Debug)]
pub struct TelemetryReader {
    pub(crate) store: Arc<TelemetryStore>,
    pub(crate) layout: Arc<TelemetryLayout>,
    pub(crate) status: Arc<EngineStatus>,
}

impl TelemetryReader {
    pub fn status(&self) -> &EngineStatus {
        &self.status
    }

    /// Every node path with telemetry, as strings (sorted).
    pub fn layout_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.layout.nodes.keys().map(|p| p.to_string()).collect();
        v.sort();
        v
    }

    /// Telemetry value `k` of a node (meter levels, readout values, ...).
    pub fn node_value(&self, path: &NodePath, k: usize) -> Option<f32> {
        let t = self.layout.nodes.get(path)?;
        let (s, e) = t.values;
        (s + k < e).then(|| load_f32(&self.store.node_values[s + k]))
    }

    /// A node's scope ring, oldest sample first.
    pub fn scope(&self, path: &NodePath) -> Option<Vec<f32>> {
        let t = self.layout.nodes.get(path)?;
        let (s, e) = t.scope;
        let n = e - s;
        if n == 0 {
            return None;
        }
        let pos = t.scope_pos.map_or(0, |p| {
            self.store.scope_pos[p].load(Ordering::Relaxed) as usize % n
        });
        Some(
            (0..n)
                .map(|i| load_f32(&self.store.scope[s + (pos + i) % n]))
                .collect(),
        )
    }

    /// The ring index the scope writes next. [`Self::scope`] returns the
    /// ring starting here (oldest first); a node that publishes a ring index
    /// (e.g. Scope's trigger position) is located in that order at
    /// `(index - scope_position) mod len`.
    pub fn scope_position(&self, path: &NodePath) -> Option<usize> {
        let t = self.layout.nodes.get(path)?;
        let n = t.scope.1 - t.scope.0;
        let p = t.scope_pos?;
        (n > 0).then(|| self.store.scope_pos[p].load(Ordering::Relaxed) as usize % n)
    }

    pub fn socket_level(&self, path: &NodePath, socket: &str) -> Option<f32> {
        let i = *self
            .layout
            .sockets
            .get(&(path.clone(), socket.to_string()))?;
        Some(load_f32(&self.store.socket_level[i]))
    }

    pub fn socket_value(&self, path: &NodePath, socket: &str) -> Option<f32> {
        let i = *self
            .layout
            .sockets
            .get(&(path.clone(), socket.to_string()))?;
        Some(load_f32(&self.store.socket_value[i]))
    }

    /// Running count of events from an event output; a change means a pulse.
    pub fn socket_events(&self, path: &NodePath, socket: &str) -> Option<u32> {
        let i = *self
            .layout
            .sockets
            .get(&(path.clone(), socket.to_string()))?;
        Some(self.store.socket_events[i].load(Ordering::Relaxed))
    }

    /// Last host block's processing time for a node, in nanoseconds.
    pub fn node_time_ns(&self, path: &NodePath) -> Option<u32> {
        let t = self.layout.nodes.get(path)?.time?;
        Some(self.store.node_time[t].load(Ordering::Relaxed))
    }

    /// Every timed node with its last host block's processing time, in
    /// nanoseconds (in no particular order). Nodes inside groups and the
    /// copies inside Voices have their own paths; the editor sums them.
    pub fn node_times(&self) -> impl Iterator<Item = (&NodePath, u32)> + '_ {
        self.layout.nodes.iter().filter_map(|(path, t)| {
            let a = self.store.node_time.get(t.time?)?;
            Some((path, a.load(Ordering::Relaxed)))
        })
    }
}
