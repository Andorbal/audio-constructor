//! Math and logic nodes. All are rate-polymorphic: the output runs at the
//! fastest rate among the `Float`/`Bool` inputs.

use ac_core::Registry;

pub mod compare;
pub mod constant;
pub mod map;
// Node files are named after their kind, and this kind is `math`.
#[allow(clippy::module_inception)]
pub mod math;
pub mod unary;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<constant::Constant>();
    r.register::<math::Math>();
    r.register::<unary::Unary>();
    r.register::<compare::Compare>();
    r.register::<map::Map>();
}

/// `x` if it is finite, else 0. Math nodes promise finite outputs, so NaN and
/// infinity never travel down a wire from here.
#[inline]
pub(crate) fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}
