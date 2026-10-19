//! Text functions.

use crate::registry::Registry;

mod bahttext;
mod base;
mod bytes;
mod case;
mod charcode;
mod concatenate;
mod find_search;
mod fixed;
mod len;
mod misc;
mod numbervalue;
mod replace;
mod roman;
mod substitute;
mod substr;
mod text_fn;
mod trim;
mod value_fn;
mod width;

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
    roman::register(r);
    width::register(r);
    bahttext::register(r);
    base::register(r);
    charcode::register(r);
    misc::register(r);
    replace::register(r);
    fixed::register(r);
    bytes::register(r);
    numbervalue::register(r);
}
