//! Statistical functions: descriptive family.

use crate::registry::Registry;

mod dispersion;
mod minmax_a;
mod moments;
mod order;
mod probability;
mod regression;
mod simple_dist;
mod util;

/// Registers the descriptive functions.
pub fn register(r: &mut Registry) {
    moments::register(r);
    dispersion::register(r);
    order::register(r);
    minmax_a::register(r);
    regression::register(r);
    probability::register(r);
    simple_dist::register(r);
}
