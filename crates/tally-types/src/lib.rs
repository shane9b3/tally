//! Core types shared by every Tally crate.
//!
//! This crate is a frozen WP0 contract: addresses ([`CellAddr`], [`CellRange`]),
//! values ([`Value`], [`Matrix`]), OpenOffice-compatible error codes
//! ([`ErrorKind`]), calculation settings ([`CalcSettings`]) and a few numeric
//! and A1-notation helpers. Changes go through lead review (see
//! `docs/contributing.md`).
//!
//! The only dependency is `serde`, behind the default-on `serde` feature.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod addr;
mod error;
mod math;
mod settings;
mod value;

pub use addr::{
    col_to_letters, letters_to_col, parse_a1, CellAddr, CellRange, CellRangeIter, SheetId,
    MAX_COLS, MAX_ROWS,
};
pub use error::ErrorKind;
pub use math::{approx_equal, round_to_15_digits};
pub use settings::{
    expand_two_digit_year, CalcSettings, CriteriaMode, DateEpoch, IterSettings, TextToNumber,
    DEFAULT_TWO_DIGIT_YEAR, MAX_STANDARD_DECIMALS, TWO_DIGIT_YEAR_RANGE,
};
pub use value::{Matrix, Value};
