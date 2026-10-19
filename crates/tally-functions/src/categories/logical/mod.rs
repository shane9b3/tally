//! Logical functions.

use crate::registry::Registry;

mod r#if;

/// Registers the Logical functions.
pub fn register(r: &mut Registry) {
    r#if::register(r);
}
