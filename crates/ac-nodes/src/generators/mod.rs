//! generators nodes.

use ac_core::Registry;

pub mod oscillator;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<oscillator::Oscillator>();
}
