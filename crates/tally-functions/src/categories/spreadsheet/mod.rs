//! Spreadsheet functions.

use crate::registry::Registry;

mod index;
mod lookup;
mod match_;
mod rowcol;

/// Registers the Spreadsheet functions.
pub fn register(r: &mut Registry) {
    lookup::register(r);
    index::register(r);
    match_::register(r);
    rowcol::register(r);
}
