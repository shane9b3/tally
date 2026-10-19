//! Logical functions.

use crate::registry::Registry;

mod andor;
mod constants;
mod r#if;
mod iferror;
mod not;
mod xor;

/// Registers the Logical functions.
pub fn register(r: &mut Registry) {
    r#if::register(r);
    andor::register(r);
    not::register(r);
    constants::register(r);
    iferror::register(r);
    xor::register(r);
}
