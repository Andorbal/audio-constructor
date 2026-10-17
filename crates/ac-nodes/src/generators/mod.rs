//! generators nodes.

use ac_core::Registry;

pub mod transport;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<transport::Transport>();
}
