//! Add-in functions.

use crate::registry::Registry;

pub mod complex;
pub mod dates;
pub mod engineering;
pub mod financial;

mod randbetween;

/// Registers the Add-in functions.
pub fn register(r: &mut Registry) {
    dates::register(r);
    engineering::register(r);
    complex::register(r);
    financial::register(r);
    randbetween::register(r);
}
