//! triggers nodes.

use ac_core::Registry;

pub mod clock;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clock::Clock>();
}
