//! Add-in functions.

use crate::registry::Registry;

mod randbetween;

/// Registers the Add-in functions.
pub fn register(r: &mut Registry) {
    randbetween::register(r);
}
