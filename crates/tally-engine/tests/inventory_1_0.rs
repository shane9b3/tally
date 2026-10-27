//! Locks in behaviour checked for the "1.0 status" block of
//! `docs/calc-features-inventory.md` (2026-10-27) that had no test of its
//! own. Where Tally differs from AOO 4.1 the test names the deviation ID
//! from `docs/parity-deviations.md`.
#![allow(clippy::unwrap_used)]

mod common;

use common::Fixture;
use tally_types::Value;

fn n(x: f64) -> Value {
    Value::Number(x)
}

/// Section 0: two-digit years use the AOO window starting at 1930
/// (fixed; AOO makes it configurable, P17).
#[test]
fn two_digit_year_window_is_1930() {
    let mut f = Fixture::new();
    f.set("A1", "1/2/29");
    f.set("A2", "1/2/30");
    f.set("B1", "=YEAR(A1)");
    f.set("B2", "=YEAR(A2)");
    f.set("B3", "=YEAR(DATEVALUE(\"1/2/29\"))");
    assert_eq!(f.value("B1"), n(2029.0));
    assert_eq!(f.value("B2"), n(1930.0));
    assert_eq!(f.value("B3"), n(2029.0));
}

/// Section 2: a leading apostrophe forces text; a leading `+` or `-`
/// before a number is its sign. (`+B1+B2` staying text is gap P18, not
/// locked here.)
#[test]
fn apostrophe_forces_text_and_signs_stay_numbers() {
    let mut f = Fixture::new();
    f.set("A1", "'123");
    assert_eq!(f.value("A1"), Value::text("123"));
    f.set("A4", "+5");
    assert_eq!(f.value("A4"), n(5.0));
    f.set("A5", "-5");
    assert_eq!(f.value("A5"), n(-5.0));
}

/// Section 27: the intersection operator `!`.
#[test]
fn intersection_operator() {
    let mut f = Fixture::new();
    for (a, v) in [("A1", "1"), ("A2", "2"), ("B1", "3"), ("B2", "4")] {
        f.set(a, v);
    }
    f.set("C1", "=SUM(A1:B2!B1:B3)");
    assert_eq!(f.value("C1"), n(7.0));
    f.set("C2", "=A1:A2!A2:B2");
    assert_eq!(f.value("C2"), n(2.0));
}

/// Section 0: formulas past AOO's 512-token limit and 1,024-character
/// strings are accepted (P16).
#[test]
fn formula_limits_are_larger_than_aoo() {
    let mut f = Fixture::new();
    let long_sum = format!("={}", vec!["1"; 600].join("+"));
    f.set("A1", &long_sum);
    assert_eq!(f.value("A1"), n(600.0));
    let long_text = format!("=LEN(\"{}\")", "a".repeat(2000));
    f.set("A2", &long_text);
    assert_eq!(f.value("A2"), n(2000.0));
    let deep = format!("={}1{}", "ABS(".repeat(60), ")".repeat(60));
    f.set("A3", &deep);
    assert_eq!(f.value("A3"), n(1.0));
}
