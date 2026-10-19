//! Mathematical functions.

use crate::registry::Registry;

mod basic;
mod bits;
mod ceiling_floor;
mod combinatorics;
mod conditional;
mod euro;
mod exp_log;
mod product;
mod random;
mod rounding;
mod subtotal;
mod sum;
mod trig;

/// Registers the Mathematical functions.
pub fn register(r: &mut Registry) {
    sum::register(r);
    rounding::register(r);
    product::register(r);
    basic::register(r);
    random::register(r);
    conditional::register(r);
    trig::register(r);
    exp_log::register(r);
    ceiling_floor::register(r);
    combinatorics::register(r);
    bits::register(r);
    euro::register(r);
    subtotal::register(r);
}
