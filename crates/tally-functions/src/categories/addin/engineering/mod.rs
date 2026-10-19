//! Addin functions: engineering family.

use crate::registry::Registry;

mod base_conv;
mod convert;
mod math_add;

/// Registers the engineering functions.
pub fn register(r: &mut Registry) {
    base_conv::register(r);
    convert::register(r);
    math_add::register(r);
}
