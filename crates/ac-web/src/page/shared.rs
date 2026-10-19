//! Shared buffers on the page: a `SharedArrayBuffer` behind
//! [`SharedMem`], with `Atomics` for the header words.

use js_sys::{Atomics, Int32Array, SharedArrayBuffer, Uint8Array};

use crate::ring::SharedMem;

/// A `SharedArrayBuffer` and views on it.
#[derive(Clone, Debug)]
pub struct JsShared {
    buffer: SharedArrayBuffer,
    words: Int32Array,
    bytes: Uint8Array,
}

impl JsShared {
    /// A new zeroed buffer of `len` bytes (a multiple of 4).
    pub fn new(len: usize) -> JsShared {
        let buffer = SharedArrayBuffer::new(len as u32);
        JsShared {
            words: Int32Array::new(&buffer),
            bytes: Uint8Array::new(&buffer),
            buffer,
        }
    }

    pub fn buffer(&self) -> &SharedArrayBuffer {
        &self.buffer
    }
}

impl SharedMem for JsShared {
    fn byte_len(&self) -> usize {
        self.bytes.length() as usize
    }

    fn load(&self, word: usize) -> u32 {
        Atomics::load(&self.words, word as u32).map_or(0, |v| v as u32)
    }

    fn store(&self, word: usize, value: u32) {
        let _ = Atomics::store(&self.words, word as u32, value as i32);
    }

    fn read(&self, offset: usize, out: &mut [u8]) {
        let end = (offset + out.len()).min(self.byte_len());
        if offset >= end {
            return;
        }
        self.bytes
            .subarray(offset as u32, end as u32)
            .copy_to(&mut out[..end - offset]);
    }

    fn write(&self, offset: usize, data: &[u8]) {
        let end = (offset + data.len()).min(self.byte_len());
        if offset >= end {
            return;
        }
        self.bytes
            .subarray(offset as u32, end as u32)
            .copy_from(&data[..end - offset]);
    }

    fn read_words(&self, offset: usize, out: &mut [u32]) {
        let first = offset / 4;
        let end = (first + out.len()).min(self.words.length() as usize);
        if first >= end {
            return;
        }
        let view = js_sys::Uint32Array::new(&self.buffer).subarray(first as u32, end as u32);
        view.copy_to(&mut out[..end - first]);
    }
}
