//! Level, dynamics, and shaping nodes.

use ac_core::Registry;

pub mod gain;
pub mod mix;
pub mod pan;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<gain::Gain>();
    r.register::<mix::Mix>();
    r.register::<pan::Pan>();
}
