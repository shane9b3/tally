//! Date&Time functions.

use crate::registry::Registry;

mod date_fn;
mod datevalue;
mod now;
mod parts;
mod weekday;

/// Registers the Date&Time functions.
pub fn register(r: &mut Registry) {
    now::register(r);
    date_fn::register(r);
    parts::register(r);
    weekday::register(r);
    datevalue::register(r);
}
