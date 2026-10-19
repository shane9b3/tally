//! Information functions.

use crate::registry::Registry;

mod is;
mod na;

/// Registers the Information functions.
pub fn register(r: &mut Registry) {
    is::register(r);
    na::register(r);
}
