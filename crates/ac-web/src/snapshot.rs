//! The worklet's telemetry, published for the page (DECISIONS D151).
//!
//! Telemetry isn't a stream: the page wants the latest values, so instead
//! of a ring it's one buffer the worklet rewrites now and then under a
//! sequence lock. The writer makes the sequence odd, writes, and makes it
//! even again; the reader copies the words and keeps them only if the
//! sequence was the same even number before and after. Neither side ever
//! waits: a reader that catches a write in progress tries again next
//! frame.
//!
//! Layout (words): sequence, format, the mirror generation of the project
//! the worklet runs (two words), the telemetry export's length, the
//! binding values' length, then the export ([`TelemetryReader::export`])
//! and the binding values.
//!
//! [`TelemetryReader::export`]: ac_engine::TelemetryReader::export

use crate::ring::SharedMem;

const SEQ: usize = 0;
const FORMAT: usize = 1;
const GENERATION: usize = 2;
const TELEMETRY_LEN: usize = 4;
const BINDINGS_LEN: usize = 5;
/// Words before the data.
pub const SNAPSHOT_HEADER: usize = 6;
const FORMAT_V1: u32 = 0x4143_5401;

/// Words a snapshot buffer of `bytes` bytes holds after its header.
pub fn data_words(bytes: usize) -> usize {
    (bytes / 4).saturating_sub(SNAPSHOT_HEADER)
}

/// The worklet's end.
#[derive(Debug)]
pub struct SnapshotWriter<M> {
    mem: M,
    seq: u32,
}

impl<M: SharedMem> SnapshotWriter<M> {
    pub fn new(mem: M) -> SnapshotWriter<M> {
        mem.store(FORMAT, FORMAT_V1);
        let seq = mem.load(SEQ) & !1;
        SnapshotWriter { mem, seq }
    }

    /// Words of data the buffer has room for.
    pub fn capacity(&self) -> usize {
        data_words(self.mem.byte_len())
    }

    /// Publish a telemetry export and binding values for the project the
    /// mirror knows as `generation`. Returns false, publishing nothing,
    /// when they don't fit. Real-time safe.
    pub fn publish(&mut self, generation: u64, telemetry: &[u32], bindings: &[u32]) -> bool {
        if telemetry.len() + bindings.len() > self.capacity() {
            return false;
        }
        self.seq = self.seq.wrapping_add(1);
        self.mem.store(SEQ, self.seq);
        self.mem.store(GENERATION, generation as u32);
        self.mem.store(GENERATION + 1, (generation >> 32) as u32);
        self.mem.store(TELEMETRY_LEN, telemetry.len() as u32);
        self.mem.store(BINDINGS_LEN, bindings.len() as u32);
        let base = SNAPSHOT_HEADER * 4;
        self.mem.write_words(base, telemetry);
        self.mem.write_words(base + telemetry.len() * 4, bindings);
        self.seq = self.seq.wrapping_add(1);
        self.mem.store(SEQ, self.seq);
        true
    }
}

/// A consistent copy of a snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot<'a> {
    /// The mirror generation of the project the worklet ran.
    pub generation: u64,
    pub telemetry: &'a [u32],
    pub bindings: &'a [u32],
}

/// The page's end.
#[derive(Debug)]
pub struct SnapshotReader<M> {
    mem: M,
    words: Vec<u32>,
    last_seq: u32,
}

impl<M: SharedMem> SnapshotReader<M> {
    pub fn new(mem: M) -> SnapshotReader<M> {
        SnapshotReader {
            mem,
            words: Vec::new(),
            last_seq: 0,
        }
    }

    /// The newest snapshot, if one was published since the last call and
    /// it could be read without catching a write halfway.
    pub fn read(&mut self) -> Option<Snapshot<'_>> {
        let before = self.mem.load(SEQ);
        if before & 1 == 1 || before == self.last_seq || self.mem.load(FORMAT) != FORMAT_V1 {
            return None;
        }
        let generation =
            u64::from(self.mem.load(GENERATION)) | (u64::from(self.mem.load(GENERATION + 1)) << 32);
        let t = self.mem.load(TELEMETRY_LEN) as usize;
        let b = self.mem.load(BINDINGS_LEN) as usize;
        if t.saturating_add(b) > data_words(self.mem.byte_len()) {
            return None;
        }
        self.words.resize(t + b, 0);
        self.mem.read_words(SNAPSHOT_HEADER * 4, &mut self.words);
        if self.mem.load(SEQ) != before {
            return None;
        }
        self.last_seq = before;
        let (telemetry, bindings) = self.words.split_at(t);
        Some(Snapshot {
            generation,
            telemetry,
            bindings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::HeapShared;

    #[test]
    fn a_snapshot_reads_back_once() {
        let mem = HeapShared::new(4096);
        let mut w = SnapshotWriter::new(&mem);
        let mut r = SnapshotReader::new(&mem);
        assert_eq!(r.read(), None, "nothing published yet");
        assert!(w.publish(7 << 33, &[1, 2, 3], &[9]));
        let s = r.read().unwrap();
        assert_eq!(s.generation, 7 << 33);
        assert_eq!(s.telemetry, &[1, 2, 3]);
        assert_eq!(s.bindings, &[9]);
        assert_eq!(r.read(), None, "already read");
        assert!(w.publish(8, &[4], &[]));
        assert_eq!(r.read().unwrap().telemetry, &[4]);
    }

    #[test]
    fn too_big_publishes_nothing() {
        let mem = HeapShared::new(64);
        let mut w = SnapshotWriter::new(&mem);
        assert_eq!(w.capacity(), 10);
        assert!(!w.publish(1, &[0; 8], &[0; 3]));
        assert!(w.publish(1, &[0; 8], &[0; 2]));
    }

    #[test]
    fn a_write_in_progress_is_skipped() {
        let mem = HeapShared::new(4096);
        let mut w = SnapshotWriter::new(&mem);
        let mut r = SnapshotReader::new(&mem);
        assert!(w.publish(1, &[5], &[]));
        // Caught halfway: the sequence is odd.
        mem.store(SEQ, mem.load(SEQ) + 1);
        assert_eq!(r.read(), None);
        mem.store(SEQ, mem.load(SEQ) + 1);
        assert!(r.read().is_some());
        // Lengths that don't fit the buffer aren't believed.
        mem.store(TELEMETRY_LEN, 1 << 30);
        mem.store(SEQ, mem.load(SEQ) + 2);
        assert_eq!(r.read(), None);
    }

    #[test]
    fn concurrent_reads_never_see_a_torn_snapshot() {
        let mem = std::sync::Arc::new(HeapShared::new(1 << 14));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let (mem, stop) = (mem.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut w = SnapshotWriter::new(&*mem);
                let mut k = 0u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let words = [k; 500];
                    w.publish(u64::from(k), &words, &[k]);
                    k = k.wrapping_add(1);
                    // Like the worklet, publish now and then rather than
                    // nonstop, so readers get through.
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            })
        };
        let mut r = SnapshotReader::new(&*mem);
        let mut good = 0;
        let t0 = std::time::Instant::now();
        while good < 50 && t0.elapsed().as_secs() < 20 {
            if let Some(s) = r.read() {
                let k = s.generation as u32;
                assert!(s.telemetry.iter().all(|v| *v == k), "torn at {k}");
                assert_eq!(s.bindings, &[k]);
                good += 1;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        writer.join().unwrap();
        assert_eq!(good, 50, "clean reads get through");
    }
}
