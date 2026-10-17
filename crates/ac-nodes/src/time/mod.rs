//! time nodes.

use ac_core::Registry;

pub mod delay;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<delay::Delay>();
}
