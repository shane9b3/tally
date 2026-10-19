//! Mathematical functions.

use crate::registry::Registry;

mod rounding;
mod sum;

/// Registers the Mathematical functions.
pub fn register(r: &mut Registry) {
    sum::register(r);
    rounding::register(r);
}
