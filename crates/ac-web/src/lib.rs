//! Audio Constructor in a browser (ARCHITECTURE §13, DECISIONS D150–D159).
//!
//! Two wasm instances share no memory: the page runs the editor with a
//! mirror engine, and an audio worklet runs the engine that plays. The
//! page relays what the mirror's handle sends ([`link`]) through rings in
//! shared memory ([`ring`], [`wire`]) and posts each project to compile;
//! the worklet ([`audio`]) compiles between render quanta, plays, and
//! publishes telemetry the mirror imports ([`snapshot`]).
//!
//! Natively this crate builds the parts both sides share, so they're
//! tested here: the rings, the messages, the worklet's engine, and the
//! page's link. On wasm32 the `worklet` feature exports the worklet's
//! functions and the `ui` feature builds the page.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod audio;
pub mod gamepad;
pub mod link;
pub mod ring;
pub mod snapshot;
pub mod wire;
