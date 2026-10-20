//! Mirroring telemetry into another address space (DECISIONS D150).
//!
//! When the processor runs in another wasm instance (the web build's audio
//! worklet), the editor reads a mirror engine compiled from the same
//! project. The worklet exports what its processor published as plain
//! words, the page copies them over, and the mirror imports them, so every
//! meter, scope, socket value, and transport readout the editor draws works
//! unchanged. Both engines compiled the same project the same way, so their
//! stores have the same shape; an import into a different shape (one side
//! has recompiled and the other hasn't yet) is refused rather than
//! scrambled.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::telemetry::{EngineStatus, TelemetryReader};

/// Words of [`EngineStatus`] a processor publishes, as exported.
pub const STATUS_WORDS: usize = 18;

/// Arrays in a telemetry store, each led by its length in an export.
const STORE_ARRAYS: usize = 7;

fn put64(out: &mut [u32], at: usize, v: u64) {
    out[at] = v as u32;
    out[at + 1] = (v >> 32) as u32;
}

fn get64(data: &[u32], at: usize) -> u64 {
    u64::from(data[at]) | (u64::from(data[at + 1]) << 32)
}

fn load64(a: &AtomicU64) -> u64 {
    a.load(Ordering::Relaxed)
}

impl EngineStatus {
    /// What the processor publishes, as words: playing, recording,
    /// position, blocks, xruns, device xruns, load, master peaks, block
    /// frames, latency, threads, and the audio thread's load. The
    /// editor's own settings (timing, input monitor) and the graph
    /// generation are left out: each side keeps its own.
    pub fn export_published(&self, out: &mut [u32; STATUS_WORDS]) {
        let l = |a: &AtomicU32| a.load(Ordering::Relaxed);
        out[0] = u32::from(self.playing.load(Ordering::Relaxed));
        out[1] = u32::from(self.recording.load(Ordering::Relaxed));
        put64(out, 2, load64(&self.position));
        put64(out, 4, load64(&self.blocks));
        put64(out, 6, load64(&self.xruns));
        put64(out, 8, load64(&self.device_xruns));
        out[10] = l(&self.load);
        out[11] = l(&self.master_peak[0]);
        out[12] = l(&self.master_peak[1]);
        out[13] = l(&self.block_frames);
        out[14] = l(&self.latency);
        out[15] = l(&self.cores);
        out[16] = l(&self.core_load[0]);
        out[17] = l(&self.sample_rate);
    }

    /// Publish words [`EngineStatus::export_published`] wrote, as if this
    /// engine's processor had.
    pub fn import_published(&self, data: &[u32; STATUS_WORDS]) {
        let s = |a: &AtomicU32, v: u32| a.store(v, Ordering::Relaxed);
        self.playing.store(data[0] != 0, Ordering::Relaxed);
        self.recording.store(data[1] != 0, Ordering::Relaxed);
        self.position.store(get64(data, 2), Ordering::Relaxed);
        self.blocks.store(get64(data, 4), Ordering::Relaxed);
        self.xruns.store(get64(data, 6), Ordering::Relaxed);
        self.device_xruns.store(get64(data, 8), Ordering::Relaxed);
        s(&self.load, data[10]);
        s(&self.master_peak[0], data[11]);
        s(&self.master_peak[1], data[12]);
        s(&self.block_frames, data[13]);
        s(&self.latency, data[14]);
        s(&self.cores, data[15]);
        s(&self.core_load[0], data[16]);
        s(&self.sample_rate, data[17]);
    }
}

impl TelemetryReader {
    fn arrays(&self) -> [&[AtomicU32]; STORE_ARRAYS] {
        let s = &self.store;
        [
            &s.node_values,
            &s.scope,
            &s.scope_pos,
            &s.socket_level,
            &s.socket_value,
            &s.socket_events,
            &s.node_time,
        ]
    }

    /// Words [`TelemetryReader::export`] writes for this graph.
    pub fn export_len(&self) -> usize {
        STATUS_WORDS + STORE_ARRAYS + self.arrays().iter().map(|a| a.len()).sum::<usize>()
    }

    /// Copy what the processor published (its status, and every meter,
    /// scope, socket, and timing value of the graph) into `out`. Writes at
    /// most `out.len()` words and returns how many a whole export takes
    /// ([`TelemetryReader::export_len`]); a shorter `out` holds a part
    /// that [`TelemetryReader::import`] refuses. Real-time safe.
    pub fn export(&self, out: &mut [u32]) -> usize {
        let total = self.export_len();
        let mut status = [0u32; STATUS_WORDS];
        self.status.export_published(&mut status);
        let mut k = 0usize;
        let mut put = |v: u32| {
            if let Some(w) = out.get_mut(k) {
                *w = v;
            }
            k += 1;
        };
        for v in status {
            put(v);
        }
        let arrays = self.arrays();
        for a in arrays {
            put(a.len() as u32);
        }
        for a in arrays {
            for v in a {
                put(v.load(Ordering::Relaxed));
            }
        }
        total
    }

    /// Publish what another engine's [`TelemetryReader::export`] wrote, as
    /// if this engine's processor had. The status always goes in; the
    /// graph's values go in only when the store has the same shape as the
    /// exported one (both engines run the same graph), and the return says
    /// whether they did.
    pub fn import(&self, data: &[u32]) -> bool {
        let Some(status) = data.first_chunk::<STATUS_WORDS>() else {
            return false;
        };
        self.status.import_published(status);
        let arrays = self.arrays();
        let lens = &data[STATUS_WORDS..];
        let shape_matches = lens.len() >= STORE_ARRAYS
            && arrays.iter().zip(lens).all(|(a, &n)| a.len() == n as usize)
            && data.len() == self.export_len();
        if !shape_matches {
            return false;
        }
        let mut values = data[STATUS_WORDS + STORE_ARRAYS..].iter();
        for a in arrays {
            for (slot, v) in a.iter().zip(values.by_ref()) {
                slot.store(*v, Ordering::Relaxed);
            }
        }
        true
    }
}
