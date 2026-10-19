//! Statistical functions.

use crate::registry::Registry;

pub mod descriptive;
pub mod distributions;

mod average;
mod count;
mod median;
mod minmax;

/// Registers the Statistical functions.
pub fn register(r: &mut Registry) {
    descriptive::register(r);
    distributions::register(r);
    average::register(r);
    count::register(r);
    minmax::register(r);
    median::register(r);
}
