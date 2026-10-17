//! modulation nodes.

use ac_core::Registry;

pub mod adsr;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<adsr::Adsr>();
}
