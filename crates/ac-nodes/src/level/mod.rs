//! Level, dynamics, and shaping nodes.

use ac_core::Registry;

pub mod gain;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<gain::Gain>();
}
