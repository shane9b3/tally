//! insta snapshots of ASTs, one snapshot per category. Origin is C3 so
//! relative offsets are visible (`A1` is `R[-2]C[-2]`).

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::cases::*;
use common::*;
use std::fmt::Write as _;
use tally_formula::{parse, Dialect};

fn dump(cases: &[&str], dialect: Dialect) -> String {
    let r = resolver();
    let mut out = String::new();
    for src in cases {
        match parse(src, a1("C3"), dialect, &r) {
            Ok(f) => writeln!(out, "{src}\n    {}", sx(&f.expr)).unwrap(),
            Err(e) => {
                writeln!(out, "{src}\n    ERROR {} at {:?}", e.kind.display(), e.span).unwrap()
            }
        }
    }
    out
}

#[test]
fn literals() {
    insta::assert_snapshot!("literals", dump(LITERALS, calc()));
}
#[test]
fn references() {
    insta::assert_snapshot!("references", dump(REFERENCES, calc()));
}
#[test]
fn operators() {
    insta::assert_snapshot!("operators", dump(OPERATORS, calc()));
}
#[test]
fn functions() {
    insta::assert_snapshot!("functions", dump(FUNCTIONS, calc()));
}
#[test]
fn arrays() {
    insta::assert_snapshot!("arrays", dump(ARRAYS, calc()));
}
#[test]
fn names() {
    insta::assert_snapshot!("names", dump(NAMES, calc()));
}
#[test]
fn whitespace() {
    insta::assert_snapshot!("whitespace", dump(WHITESPACE, calc()));
}
#[test]
fn malformed() {
    insta::assert_snapshot!("malformed", dump(MALFORMED, calc()));
}
#[test]
fn open_formula() {
    insta::assert_snapshot!("open_formula", dump(OPEN_FORMULA, Dialect::open_formula()));
}

#[test]
fn at_least_150_formulas() {
    let n = LITERALS.len()
        + REFERENCES.len()
        + OPERATORS.len()
        + FUNCTIONS.len()
        + ARRAYS.len()
        + NAMES.len()
        + WHITESPACE.len()
        + MALFORMED.len()
        + OPEN_FORMULA.len();
    assert!(n >= 150, "{n}");
}
