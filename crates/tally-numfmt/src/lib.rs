//! Number formatting for Tally (WP4).
//!
//! - [`parse_format`] / [`print_format`]: OOo format codes ⇄ [`FormatAst`]
//!   ([`parse_format_with_locale`] for codes in a locale's spelling);
//! - [`format_value`] / [`format_value_with`]: value + format → [`Formatted`];
//! - [`recognize_input`] / [`recognize_input_with`]: typed text → value and
//!   implied format; [`parse_number_unambiguous`] for ConvertUnambiguous;
//! - [`date`]: proleptic Gregorian serials for the three epochs;
//! - [`Locale`] (en-US data), [`builtin_format`] and Excel translation.
//!
//! See `docs/architecture-plan.md` §3.8 and §7 WP4.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod builtin;
pub mod date;
mod decimal;
pub mod excel;
mod format;
mod format_ast;
mod locale;
mod parse_format;
mod precision;
mod recognize;

pub use builtin::{builtin_format, QuickFormat};
pub use format::{
    format_language, format_value, format_value_in_locale, format_value_with, general_string,
    AlignHint, Formatted, OVERFLOW,
};
pub use format_ast::{
    is_general_code, Color, CompareOp, Condition, DateLayout, DateTimeToken, Digit, ElapsedUnit,
    ExponentLayout, FormatAst, FractionLayout, GeneralKeyword, NamedColor, NumberLayout, Section,
    SectionKind, Token, COLOR_PALETTE,
};
pub use locale::{
    lcid_for_language, lcid_language, locale_by_lcid, locale_by_tag, DateOrder, Locale, DE_DE,
    EN_GB, EN_US, ES_ES, FR_FR, JA_JP, LCID_LANGUAGES, LOCALES,
};
pub use parse_format::{parse_format, parse_format_with_locale, print_format, FormatError};
pub use precision::{display_decimals, display_decimals_for, display_decimals_of};
pub use recognize::{
    parse_number_unambiguous, parse_number_unambiguous_with, recognize_input, recognize_input_with,
    RecognizeContext, Recognized,
};
