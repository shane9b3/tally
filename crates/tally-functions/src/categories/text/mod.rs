//! Text functions.

use crate::registry::Registry;

mod concatenate;

/// Registers the Text functions.
pub fn register(r: &mut Registry) {
    concatenate::register(r);
}
