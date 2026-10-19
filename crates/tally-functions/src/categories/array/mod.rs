//! Array functions.

use crate::registry::Registry;

mod frequency;
mod mat;
mod matrix;
mod regression;
mod sumproduct;
mod sumx;

/// Registers the Array functions.
pub fn register(r: &mut Registry) {
    sumproduct::register(r);
    matrix::register(r);
    frequency::register(r);
    sumx::register(r);
    regression::register(r);
}
