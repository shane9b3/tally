//! Logical functions.

use crate::registry::Registry;

mod andor;
mod constants;
mod r#if;
mod iferror;
mod not;

/// Registers the Logical functions.
pub fn register(r: &mut Registry) {
    r#if::register(r);
    andor::register(r);
    not::register(r);
    constants::register(r);
    iferror::register(r);
}
