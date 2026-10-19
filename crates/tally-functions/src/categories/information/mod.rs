//! Information functions.

use crate::registry::Registry;

mod cell;
mod is;
mod is_more;
mod na;

/// Registers the Information functions.
pub fn register(r: &mut Registry) {
    is::register(r);
    na::register(r);
    is_more::register(r);
    cell::register(r);
}
