//! Text functions.

use crate::registry::Registry;

mod case;
mod concatenate;
mod find_search;
mod len;
mod substitute;
mod substr;
mod text_fn;
mod trim;
mod value_fn;

/// Registers the Text functions.
pub fn register(r: &mut Registry) {
    concatenate::register(r);
    substr::register(r);
    len::register(r);
    case::register(r);
    trim::register(r);
    text_fn::register(r);
    value_fn::register(r);
    find_search::register(r);
    substitute::register(r);
}
