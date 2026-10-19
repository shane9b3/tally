//! Mathematical functions.

use crate::registry::Registry;

mod basic;
mod conditional;
mod product;
mod random;
mod rounding;
mod sum;

/// Registers the Mathematical functions.
pub fn register(r: &mut Registry) {
    sum::register(r);
    rounding::register(r);
    product::register(r);
    basic::register(r);
    random::register(r);
    conditional::register(r);
}
