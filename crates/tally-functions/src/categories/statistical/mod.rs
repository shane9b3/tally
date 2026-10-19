//! Statistical functions.

use crate::registry::Registry;

mod average;
mod count;

/// Registers the Statistical functions.
pub fn register(r: &mut Registry) {
    average::register(r);
    count::register(r);
}
