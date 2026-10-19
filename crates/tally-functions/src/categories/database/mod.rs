//! Database functions (DSUM, DCOUNT, DGET, ...).

mod aggregate;
mod common;
mod dget;
mod dispersion;

use crate::registry::Registry;

/// Registers the Database functions.
pub fn register(r: &mut Registry) {
    aggregate::register(r);
    dispersion::register(r);
    dget::register(r);
}
