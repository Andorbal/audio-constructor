//! filters nodes.

use ac_core::Registry;

pub mod biquad;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<biquad::Biquad>();
}
