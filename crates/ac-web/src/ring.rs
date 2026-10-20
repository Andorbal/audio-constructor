//! A single-producer, single-consumer ring of byte records in shared memory
//! (ARCHITECTURE §13, DECISIONS D151).
//!
//! The page and the audio worklet are separate wasm instances, so their
//! rings live outside both: in a `SharedArrayBuffer` each side reaches
//! through [`SharedMem`]. The algorithm is the usual one (a monotonic
//! write index the producer publishes with release ordering after the
//! bytes are in, a read index the consumer publishes the same way), on
//! records of `[length: u32 LE][bytes]` that may wrap at the end. The
//! producer never waits: a record that doesn't fit is dropped and counted.
//! Nothing here allocates, so the worklet can use it inside `process()`.
//!
//! Layout: four header words (write index, read index, capacity, dropped
//! records), then `capacity` bytes of data. The capacity is a power of two.

/// Header words before a buffer's data.
pub const HEADER_WORDS: usize = 4;
/// Bytes of header before a buffer's data.
pub const HEADER_BYTES: usize = HEADER_WORDS * 4;
/// The longest record a ring carries.
pub const MAX_RECORD: usize = 1024;

const WRITE: usize = 0;
const READ: usize = 1;
const CAPACITY: usize = 2;
const DROPPED: usize = 3;

/// Memory both sides of a ring (or a snapshot) can reach. On the web it is
/// a `SharedArrayBuffer`; in tests, atomics on the heap. Offsets are bytes
/// from the start of the buffer, header included.
pub trait SharedMem {
    /// The buffer's length in bytes.
    fn byte_len(&self) -> usize;
    /// Load 32-bit word `word` with acquire ordering.
    fn load(&self, word: usize) -> u32;
    /// Store 32-bit word `word` with release ordering.
    fn store(&self, word: usize, value: u32);
    /// Copy `out.len()` bytes starting at `offset` into `out`.
    fn read(&self, offset: usize, out: &mut [u8]);
    /// Copy `data` into the buffer starting at `offset`.
    fn write(&self, offset: usize, data: &[u8]);

    /// Copy words starting at byte `offset` (a multiple of 4) into `out`.
    fn read_words(&self, offset: usize, out: &mut [u32]) {
        let mut bytes = [0u8; 256];
        for (k, chunk) in out.chunks_mut(64).enumerate() {
            let b = &mut bytes[..chunk.len() * 4];
            self.read(offset + k * 256, b);
            for (w, q) in chunk.iter_mut().zip(b.chunks_exact(4)) {
                *w = u32::from_le_bytes([q[0], q[1], q[2], q[3]]);
            }
        }
    }

    /// Copy `data` into the buffer starting at byte `offset` (a multiple
    /// of 4).
    fn write_words(&self, offset: usize, data: &[u32]) {
        let mut bytes = [0u8; 256];
        for (k, chunk) in data.chunks(64).enumerate() {
            for (w, q) in chunk.iter().zip(bytes.chunks_exact_mut(4)) {
                q.copy_from_slice(&w.to_le_bytes());
            }
            self.write(offset + k * 256, &bytes[..chunk.len() * 4]);
        }
    }
}

impl<M: SharedMem + ?Sized> SharedMem for &M {
    fn byte_len(&self) -> usize {
        (**self).byte_len()
    }
    fn load(&self, word: usize) -> u32 {
        (**self).load(word)
    }
    fn store(&self, word: usize, value: u32) {
        (**self).store(word, value);
    }
    fn read(&self, offset: usize, out: &mut [u8]) {
        (**self).read(offset, out);
    }
    fn write(&self, offset: usize, data: &[u8]) {
        (**self).write(offset, data);
    }
    fn read_words(&self, offset: usize, out: &mut [u32]) {
        (**self).read_words(offset, out);
    }
    fn write_words(&self, offset: usize, data: &[u32]) {
        (**self).write_words(offset, data);
    }
}

/// The buffer length a ring of `capacity` data bytes needs.
pub const fn ring_bytes(capacity: usize) -> usize {
    HEADER_BYTES + capacity
}

/// Set up a fresh ring in `mem`, using all of it past the header (rounded
/// down to a power of two). Returns the capacity, or `None` if the buffer
/// is too small for a record.
pub fn init_ring(mem: &impl SharedMem) -> Option<u32> {
    let room = mem.byte_len().checked_sub(HEADER_BYTES)?;
    if room < MAX_RECORD + 4 {
        return None;
    }
    let cap = 1u32 << (usize::BITS - 1 - room.leading_zeros()).min(31);
    mem.store(WRITE, 0);
    mem.store(READ, 0);
    mem.store(DROPPED, 0);
    mem.store(CAPACITY, cap);
    Some(cap)
}

/// The capacity a ring set up with [`init_ring`] says it has, if it looks
/// right for this buffer.
fn capacity(mem: &impl SharedMem) -> Option<u32> {
    let cap = mem.load(CAPACITY);
    let fits = (cap as usize)
        .checked_add(HEADER_BYTES)
        .is_some_and(|n| n <= mem.byte_len());
    (cap.is_power_of_two() && cap as usize >= MAX_RECORD + 4 && fits).then_some(cap)
}

/// Copy `data` into the ring's data at `pos` (wrapping).
fn put(mem: &impl SharedMem, cap: u32, pos: u32, data: &[u8]) {
    let at = (pos & (cap - 1)) as usize;
    let first = data.len().min(cap as usize - at);
    mem.write(HEADER_BYTES + at, &data[..first]);
    if first < data.len() {
        mem.write(HEADER_BYTES, &data[first..]);
    }
}

/// Copy bytes out of the ring's data at `pos` (wrapping).
fn get(mem: &impl SharedMem, cap: u32, pos: u32, out: &mut [u8]) {
    let at = (pos & (cap - 1)) as usize;
    let first = out.len().min(cap as usize - at);
    mem.read(HEADER_BYTES + at, &mut out[..first]);
    if first < out.len() {
        mem.read(HEADER_BYTES, &mut out[first..]);
    }
}

/// The writing end.
#[derive(Debug)]
pub struct Producer<M> {
    mem: M,
    cap: u32,
    write: u32,
}

impl<M: SharedMem> Producer<M> {
    /// The producer of a ring [`init_ring`] set up, or `None` if `mem`
    /// doesn't hold one. It carries on from the published write index.
    pub fn new(mem: M) -> Option<Producer<M>> {
        let cap = capacity(&mem)?;
        let write = mem.load(WRITE);
        Some(Producer { mem, cap, write })
    }

    /// Bytes free for records (each takes four more for its length).
    pub fn space(&self) -> usize {
        let used = self.write.wrapping_sub(self.mem.load(READ));
        (self.cap as usize).saturating_sub(used as usize)
    }

    /// Queue one record. Returns false, and counts the drop, if it's longer
    /// than [`MAX_RECORD`] or there's no room. Never blocks or allocates.
    pub fn push(&mut self, record: &[u8]) -> bool {
        let need = record.len() + 4;
        if record.len() > MAX_RECORD || self.space() < need {
            let d = self.mem.load(DROPPED);
            self.mem.store(DROPPED, d.wrapping_add(1));
            return false;
        }
        put(
            &self.mem,
            self.cap,
            self.write,
            &(record.len() as u32).to_le_bytes(),
        );
        put(&self.mem, self.cap, self.write.wrapping_add(4), record);
        self.write = self.write.wrapping_add(need as u32);
        self.mem.store(WRITE, self.write);
        true
    }

    /// Records dropped because they didn't fit.
    pub fn dropped(&self) -> u32 {
        self.mem.load(DROPPED)
    }
}

/// The reading end.
#[derive(Debug)]
pub struct Consumer<M> {
    mem: M,
    cap: u32,
    read: u32,
}

impl<M: SharedMem> Consumer<M> {
    /// The consumer of a ring [`init_ring`] set up, or `None` if `mem`
    /// doesn't hold one.
    pub fn new(mem: M) -> Option<Consumer<M>> {
        let cap = capacity(&mem)?;
        let read = mem.load(READ);
        Some(Consumer { mem, cap, read })
    }

    /// Copy the next record into `out` without taking it, and return its
    /// length; `None` when the ring is empty. A record longer than `out`
    /// (which a producer never writes when `out` holds [`MAX_RECORD`]) is
    /// skipped.
    pub fn peek(&mut self, out: &mut [u8]) -> Option<usize> {
        loop {
            let avail = self.mem.load(WRITE).wrapping_sub(self.read) as usize;
            if avail < 4 {
                return None;
            }
            let mut len = [0u8; 4];
            get(&self.mem, self.cap, self.read, &mut len);
            let len = u32::from_le_bytes(len) as usize;
            if len > avail - 4 || len > MAX_RECORD {
                // Not a record a producer wrote: start over from the
                // producer's position rather than read garbage.
                self.read = self.mem.load(WRITE);
                self.mem.store(READ, self.read);
                return None;
            }
            if len > out.len() {
                self.skip(len);
                continue;
            }
            get(
                &self.mem,
                self.cap,
                self.read.wrapping_add(4),
                &mut out[..len],
            );
            return Some(len);
        }
    }

    /// Take the record [`Consumer::peek`] returned (of length `len`).
    pub fn skip(&mut self, len: usize) {
        self.read = self.read.wrapping_add(len as u32 + 4);
        self.mem.store(READ, self.read);
    }

    /// Take the next record into `out`; its length, or `None` when empty.
    pub fn pop(&mut self, out: &mut [u8]) -> Option<usize> {
        let n = self.peek(out)?;
        self.skip(n);
        Some(n)
    }
}

/// Shared memory on the heap, for tests and for the native side of
/// anything that runs both ends in one process: words of atomics.
#[derive(Debug)]
pub struct HeapShared {
    words: Box<[std::sync::atomic::AtomicU32]>,
}

impl HeapShared {
    pub fn new(bytes: usize) -> HeapShared {
        HeapShared {
            words: (0..bytes.div_ceil(4))
                .map(|_| std::sync::atomic::AtomicU32::new(0))
                .collect(),
        }
    }
}

impl SharedMem for HeapShared {
    fn byte_len(&self) -> usize {
        self.words.len() * 4
    }

    fn load(&self, word: usize) -> u32 {
        self.words
            .get(word)
            .map_or(0, |w| w.load(std::sync::atomic::Ordering::Acquire))
    }

    fn store(&self, word: usize, value: u32) {
        if let Some(w) = self.words.get(word) {
            w.store(value, std::sync::atomic::Ordering::Release);
        }
    }

    // Data goes in and out with acquire and release too, so a reader of a
    // snapshot (a sequence lock) can't have its reads drift past the
    // sequence check, nor a writer its writes before the odd sequence.
    fn read(&self, offset: usize, out: &mut [u8]) {
        use std::sync::atomic::Ordering::Acquire;
        for (k, b) in out.iter_mut().enumerate() {
            let i = offset + k;
            *b = self
                .words
                .get(i / 4)
                .map_or(0, |w| (w.load(Acquire) >> ((i % 4) * 8)) as u8);
        }
    }

    fn write(&self, offset: usize, data: &[u8]) {
        use std::sync::atomic::Ordering::AcqRel;
        for (k, b) in data.iter().enumerate() {
            let i = offset + k;
            if let Some(w) = self.words.get(i / 4) {
                let shift = (i % 4) * 8;
                w.fetch_and(!(0xff << shift), AcqRel);
                w.fetch_or(u32::from(*b) << shift, AcqRel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ring(bytes: usize) -> HeapShared {
        let mem = HeapShared::new(bytes);
        init_ring(&mem).expect("room for a ring");
        mem
    }

    #[test]
    fn records_come_out_in_order_and_whole() {
        let mem = ring(ring_bytes(4096));
        let mut tx = Producer::new(&mem).unwrap();
        let mut rx = Consumer::new(&mem).unwrap();
        let mut out = [0u8; MAX_RECORD];
        assert_eq!(rx.pop(&mut out), None);
        assert!(tx.push(b"one"));
        assert!(tx.push(b""));
        assert!(tx.push(&[7u8; 300]));
        assert_eq!(rx.pop(&mut out), Some(3));
        assert_eq!(&out[..3], b"one");
        assert_eq!(rx.pop(&mut out), Some(0));
        assert_eq!(rx.peek(&mut out), Some(300));
        assert_eq!(rx.peek(&mut out), Some(300), "peeking takes nothing");
        rx.skip(300);
        assert_eq!(rx.pop(&mut out), None);
    }

    #[test]
    fn records_wrap_around_the_end() {
        let mem = ring(ring_bytes(2048));
        let mut tx = Producer::new(&mem).unwrap();
        let mut rx = Consumer::new(&mem).unwrap();
        let mut out = [0u8; MAX_RECORD];
        // Lengths that don't divide the capacity put every split point
        // (in the length word and in the bytes) at the end sooner or later.
        for k in 0..2_000u32 {
            let len = (k * 37 % 700) as usize;
            let rec: Vec<u8> = (0..len).map(|i| (i as u32 + k) as u8).collect();
            assert!(tx.push(&rec), "record {k}");
            assert_eq!(rx.pop(&mut out), Some(len), "record {k}");
            assert_eq!(&out[..len], &rec[..], "record {k}");
        }
        // The indexes wrap past u32::MAX too.
        mem.store(WRITE, u32::MAX - 5);
        mem.store(READ, u32::MAX - 5);
        let mut tx = Producer::new(&mem).unwrap();
        let mut rx = Consumer::new(&mem).unwrap();
        assert!(tx.push(b"across zero"));
        assert_eq!(rx.pop(&mut out), Some(11));
        assert_eq!(&out[..11], b"across zero");
    }

    #[test]
    fn a_full_ring_drops_and_counts() {
        let mem = ring(ring_bytes(2048));
        let mut tx = Producer::new(&mem).unwrap();
        let mut rx = Consumer::new(&mem).unwrap();
        let rec = [1u8; 1000];
        assert!(tx.push(&rec));
        assert!(tx.push(&rec));
        assert!(!tx.push(&rec), "no room left");
        assert!(!tx.push(&[0u8; MAX_RECORD + 1]), "too long");
        assert_eq!(tx.dropped(), 2);
        let mut out = [0u8; MAX_RECORD];
        assert_eq!(rx.pop(&mut out), Some(1000));
        assert!(tx.push(&rec), "room again once read");
    }

    #[test]
    fn a_buffer_without_a_ring_is_refused() {
        let mem = HeapShared::new(64);
        assert!(init_ring(&mem).is_none());
        let mem = HeapShared::new(ring_bytes(4096));
        assert!(Producer::new(&mem).is_none(), "not set up");
        assert!(Consumer::new(&mem).is_none());
        // A capacity that isn't a power of two, or is bigger than the
        // buffer, isn't trusted.
        mem.store(CAPACITY, 3000);
        assert!(Producer::new(&mem).is_none());
        mem.store(CAPACITY, 8192);
        assert!(Producer::new(&mem).is_none());
        // A ring set up in an odd-sized buffer rounds down.
        let mem = HeapShared::new(ring_bytes(3000));
        assert_eq!(init_ring(&mem), Some(2048));
    }

    #[test]
    fn garbage_lengths_are_skipped_not_read() {
        let mem = ring(ring_bytes(2048));
        let mut rx = Consumer::new(&mem).unwrap();
        // A length bigger than what was written.
        mem.write(HEADER_BYTES, &5000u32.to_le_bytes());
        mem.store(WRITE, 8);
        let mut out = [0u8; MAX_RECORD];
        assert_eq!(rx.pop(&mut out), None);
        let mut tx = Producer::new(&mem).unwrap();
        assert!(tx.push(b"ok"));
        assert_eq!(rx.pop(&mut out), Some(2));
    }

    #[test]
    fn two_threads_pass_every_record() {
        let mem = Arc::new(ring(ring_bytes(4096)));
        let n = 20_000u32;
        let writer = {
            let mem = mem.clone();
            std::thread::spawn(move || {
                let mut tx = Producer::new(&*mem).unwrap();
                let mut k = 0u32;
                while k < n {
                    let len = (k % 61) as usize + 4;
                    let mut rec = vec![(k % 251) as u8; len];
                    rec[..4].copy_from_slice(&k.to_le_bytes());
                    if tx.push(&rec) {
                        k += 1;
                    } else {
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut rx = Consumer::new(&*mem).unwrap();
        let mut out = [0u8; MAX_RECORD];
        let mut want = 0u32;
        while want < n {
            match rx.pop(&mut out) {
                Some(len) => {
                    assert_eq!(len, (want % 61) as usize + 4);
                    assert_eq!(u32::from_le_bytes([out[0], out[1], out[2], out[3]]), want);
                    assert!(out[4..len].iter().all(|b| *b == (want % 251) as u8));
                    want += 1;
                }
                None => std::thread::yield_now(),
            }
        }
        writer.join().unwrap();
    }

    #[test]
    fn words_round_trip_through_bytes() {
        let mem = HeapShared::new(4096);
        let words: Vec<u32> = (0..300u32)
            .map(|i| i.wrapping_mul(0x0101_0101).wrapping_add(7))
            .collect();
        mem.write_words(16, &words);
        let mut back = vec![0u32; 300];
        mem.read_words(16, &mut back);
        assert_eq!(back, words);
    }
}
