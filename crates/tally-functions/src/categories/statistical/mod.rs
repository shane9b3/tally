//! Statistical functions.

use crate::registry::Registry;

mod average;
mod count;
mod median;
mod minmax;

/// Registers the Statistical functions.
pub fn register(r: &mut Registry) {
    average::register(r);
    count::register(r);
    minmax::register(r);
    median::register(r);
}
