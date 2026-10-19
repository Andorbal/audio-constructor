//! Audio Constructor as a plugin: the engine inside a host, as a CLAP and
//! VST3 **effect** (audio in and out, notes in) and **instrument** (notes
//! in, audio out), both from this one library (ARCHITECTURE §13).
//!
//! - The host's sample rate and largest block size build the engine when
//!   it activates the plugin; its buffers, notes, transport, and the macro
//!   parameters feed the engine each block ([`bridge`]), without
//!   allocating.
//! - Sixteen **macro** parameters are what the host automates; the project
//!   maps them onto inputs with ordinary bindings from `Source::Macro`
//!   ([`params`], [`macros`]).
//! - The **state** the host saves is the project, its audio files, and the
//!   macro values ([`state`]); restoring recompiles on the host's main
//!   thread and swaps the graph in at a block boundary ([`session`]).
//! - A new instance starts with a working patch ([`defaults`]).
//! - The editor plugs in through [`editor`].
//!
//! Build the bundles with `python3 scripts/bundle_plugin.py`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod assets;
pub mod base64;
pub mod bridge;
mod clock;
mod control_thread;
pub mod defaults;
pub mod editor;
pub mod macros;
pub mod params;
mod plugin;
pub mod session;
pub mod state;

pub use plugin::{
    AcPlugin, Effect, EffectVariant, Instrument, InstrumentVariant, Variant, Vst3Effect,
    Vst3Instrument,
};
pub use session::{Kind, Session, Shared};

// Debug builds (what the tests and the CLAP test host load) abort on any
// allocation inside the plugin's process call, as the engine's own tests
// do (ARCHITECTURE §8). Release builds use the system allocator untouched.
// A binary linking this library must not declare its own allocator.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

nice_plug::nice_export_clap!(Effect, Instrument);
// The VST3 plugins have no window yet (see `editor.rs`); same class ids.
nice_plug::nice_export_vst3!(Vst3Effect, Vst3Instrument);
