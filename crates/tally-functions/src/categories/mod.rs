//! One module per AOO category. Each exposes `register(r)`; its own
//! `mod.rs` lists the function families of that category, so adding a
//! function touches only the family file and one line there.

use crate::registry::Registry;

mod addin;
mod array;
mod database;
mod datetime;
mod financial;
mod information;
mod logical;
mod mathematical;
mod ooxml;
mod spreadsheet;
mod statistical;
mod text;

/// Registers every category, in AOO wizard order.
pub(crate) fn register_all(r: &mut Registry) {
    database::register(r);
    datetime::register(r);
    financial::register(r);
    information::register(r);
    logical::register(r);
    mathematical::register(r);
    array::register(r);
    statistical::register(r);
    spreadsheet::register(r);
    text::register(r);
    addin::register(r);
    ooxml::apply(r);
}
