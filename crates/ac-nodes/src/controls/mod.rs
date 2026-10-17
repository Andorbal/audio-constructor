//! controls nodes.

use ac_core::Registry;

pub mod button;
pub mod slider;
pub mod toggle;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<button::Button>();
    r.register::<slider::Slider>();
    r.register::<toggle::Toggle>();
}
