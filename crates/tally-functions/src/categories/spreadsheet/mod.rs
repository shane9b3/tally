//! Spreadsheet functions.

use crate::registry::Registry;

mod index;
mod lookup;
mod match_;
mod misc;
mod pivot;
mod reference;
mod rowcol;
mod tableop;

/// Registers the Spreadsheet functions.
pub fn register(r: &mut Registry) {
    lookup::register(r);
    index::register(r);
    match_::register(r);
    rowcol::register(r);
    reference::register(r);
    misc::register(r);
    pivot::register(r);
    tableop::register(r);
}
