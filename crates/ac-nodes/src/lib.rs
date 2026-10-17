//! Every built-in Audio Constructor node, one file each, grouped by category.
//!
//! Each node is a primitive (DECISIONS D14), documents itself in its doc
//! comment (with a "try this" tip), is sample-rate independent, and follows
//! the generator contract where it applies (docs/NODES.md).

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod controls;
pub mod display;
pub mod filters;
pub mod generators;
pub mod io;
pub mod level;
pub mod math;
pub mod modulation;
mod segments;
pub mod sequencing;
pub mod time;
pub mod triggers;
pub mod utility;

use ac_core::Registry;

/// Add every built-in node to `r`.
pub fn register_all(r: &mut Registry) {
    io::register(r);
    generators::register(r);
    level::register(r);
    filters::register(r);
    time::register(r);
    math::register(r);
    modulation::register(r);
    triggers::register(r);
    sequencing::register(r);
    controls::register(r);
    display::register(r);
    utility::register(r);
}

/// A registry with every built-in node.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    register_all(&mut r);
    r
}
