//! Array functions.

use crate::registry::Registry;

mod sumproduct;

/// Registers the Array functions.
pub fn register(r: &mut Registry) {
    sumproduct::register(r);
}
