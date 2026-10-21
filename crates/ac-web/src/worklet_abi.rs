//! The audio worklet's module (DECISIONS D151): the engine behind a few
//! exported functions that take and return plain numbers, so the worklet's
//! script (`web/worklet.js`) instantiates it with no bindings generator
//! and no glue that assumes a window. Shared buffers are reached through
//! five functions the script provides (`env.ac_shared_*`), which is how
//! the same ring code runs here and on the page.
//!
//! The script calls `ac_init` once, writes each render quantum's input
//! where `ac_input` points, calls `ac_process`, and reads the output
//! where `ac_output` points. Posted messages go through `ac_message`,
//! from the worklet's message handler, never from `process()`.

// Exporting functions by name and declaring the script's imports is what
// `unsafe` is used for here: there is no other way to give a wasm module
// an interface without a bindings generator, and a bindings generator's
// glue doesn't run in an audio worklet. Each use says why it's sound.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::sync::Arc;

use crate::audio::AudioSide;
use crate::ring::{Consumer, Producer, SharedMem};
use crate::snapshot::SnapshotWriter;

/// The shared buffers, by the number the script knows them by.
const TO_AUDIO: u32 = 0;
const FROM_AUDIO: u32 = 1;
const SNAPSHOT: u32 = 2;

// SAFETY: these are the functions `web/worklet.js` passes as `env`. The
// length, load, and store functions take and return integers and touch
// only the shared buffer they're given, so calling them is safe. The read
// and write functions copy between a shared buffer and this module's
// memory at the address passed, so they're unsafe to call: the caller
// guarantees the range is memory it owns for the duration of the call.
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    safe fn ac_shared_len(buffer: u32) -> u32;
    safe fn ac_shared_load(buffer: u32, word: u32) -> u32;
    safe fn ac_shared_store(buffer: u32, word: u32, value: u32);
    fn ac_shared_read(buffer: u32, offset: u32, ptr: u32, len: u32);
    fn ac_shared_write(buffer: u32, offset: u32, ptr: u32, len: u32);
}

/// A shared buffer the worklet's script holds.
#[derive(Clone, Copy, Debug)]
struct Shared(u32);

impl SharedMem for Shared {
    fn byte_len(&self) -> usize {
        ac_shared_len(self.0) as usize
    }

    fn load(&self, word: usize) -> u32 {
        ac_shared_load(self.0, word as u32)
    }

    fn store(&self, word: usize, value: u32) {
        ac_shared_store(self.0, word as u32, value);
    }

    fn read(&self, offset: usize, out: &mut [u8]) {
        // SAFETY: `out` is a live, exclusively borrowed buffer of exactly
        // `out.len()` bytes for the whole call.
        unsafe {
            ac_shared_read(
                self.0,
                offset as u32,
                out.as_mut_ptr() as u32,
                out.len() as u32,
            );
        }
    }

    fn write(&self, offset: usize, data: &[u8]) {
        // SAFETY: `data` is a live buffer of exactly `data.len()` bytes,
        // only read during the call.
        unsafe {
            ac_shared_write(
                self.0,
                offset as u32,
                data.as_ptr() as u32,
                data.len() as u32,
            );
        }
    }

    fn read_words(&self, offset: usize, out: &mut [u32]) {
        // SAFETY: as `read`; `out` is `out.len() * 4` bytes, and wasm
        // words are little-endian like the buffer's.
        unsafe {
            ac_shared_read(
                self.0,
                offset as u32,
                out.as_mut_ptr() as u32,
                (out.len() * 4) as u32,
            );
        }
    }

    fn write_words(&self, offset: usize, data: &[u32]) {
        // SAFETY: as `write`; `data` is `data.len() * 4` bytes.
        unsafe {
            ac_shared_write(
                self.0,
                offset as u32,
                data.as_ptr() as u32,
                (data.len() * 4) as u32,
            );
        }
    }
}

struct Worklet {
    side: AudioSide,
    inbox: Consumer<Shared>,
    outbox: Producer<Shared>,
    snapshot: SnapshotWriter<Shared>,
    /// Where the script copies a posted message before `ac_message`.
    message: Vec<u8>,
}

thread_local! {
    // A worklet has one thread; this is just a place to keep the engine
    // between calls.
    static WORKLET: RefCell<Option<Worklet>> = const { RefCell::new(None) };
}

/// Build the engine at the context's rate, taking at most `max_block`
/// frames per step, with `in_channels` and `out_channels`. Returns 1, or
/// 0 if the shared buffers don't hold rings.
// SAFETY (all exports): `no_mangle` exports the function under its own
// name; nothing else in the module defines these names.
#[unsafe(no_mangle)]
pub extern "C" fn ac_init(
    sample_rate: f64,
    max_block: u32,
    in_channels: u32,
    out_channels: u32,
) -> u32 {
    let (Some(inbox), Some(outbox)) = (
        Consumer::new(Shared(TO_AUDIO)),
        Producer::new(Shared(FROM_AUDIO)),
    ) else {
        return 0;
    };
    let side = AudioSide::new(
        Arc::new(ac_nodes::registry()),
        sample_rate,
        max_block as usize,
        in_channels as usize,
        out_channels as usize,
    );
    let w = Worklet {
        side,
        inbox,
        outbox,
        snapshot: SnapshotWriter::new(Shared(SNAPSHOT)),
        message: Vec::new(),
    };
    WORKLET.with_borrow_mut(|slot| *slot = Some(w));
    1
}

fn with<R>(default: R, f: impl FnOnce(&mut Worklet) -> R) -> R {
    WORKLET.with_borrow_mut(|w| w.as_mut().map_or(default, f))
}

/// Where the script writes a quantum's input: channel `c` at `c * frames`
/// floats.
#[unsafe(no_mangle)]
pub extern "C" fn ac_input() -> u32 {
    with(0, |w| w.side.input_mut().as_mut_ptr() as u32)
}

/// Where the script reads a quantum's output, laid out like the input.
#[unsafe(no_mangle)]
pub extern "C" fn ac_output() -> u32 {
    with(0, |w| w.side.output().as_ptr() as u32)
}

/// Run one render quantum of `frames` frames. Returns 1, or 0 before
/// `ac_init`. Real-time safe.
#[unsafe(no_mangle)]
pub extern "C" fn ac_process(frames: u32) -> u32 {
    with(0, |w| {
        w.side.process(
            frames as usize,
            &mut w.inbox,
            &mut w.outbox,
            &mut w.snapshot,
        );
        1
    })
}

/// Room for a posted message of `len` bytes: the address the script
/// copies it to before calling [`ac_message`].
#[unsafe(no_mangle)]
pub extern "C" fn ac_message_buffer(len: u32) -> u32 {
    with(0, |w| {
        w.message.clear();
        w.message.resize(len as usize, 0);
        w.message.as_mut_ptr() as u32
    })
}

/// Handle the posted message the script copied in: 1 handled, 2 a project
/// compiled with errors in it, 0 not understood (or before `ac_init`).
#[unsafe(no_mangle)]
pub extern "C" fn ac_message(len: u32) -> u32 {
    with(0, |w| {
        let len = (len as usize).min(w.message.len());
        let bytes = std::mem::take(&mut w.message);
        let r = w.side.message(&bytes[..len]);
        w.message = bytes;
        match r {
            Ok(crate::audio::Handled::Compiled { errors, .. }) if errors > 0 => 2,
            Ok(_) => 1,
            Err(_) => 0,
        }
    })
}
