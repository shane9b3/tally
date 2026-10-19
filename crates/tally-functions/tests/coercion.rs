//! Every coercion rule in `coerce.rs`.

#![allow(clippy::unwrap_used)]

mod common;
use common::*;

use tally_functions::coerce::{
    self, convert_string_to_value, intersect, number_seq, number_seq_with_text_as_zero,
    number_to_text, parse_unambiguous, scalar_value, text_to_number, to_bool, to_integer,
    to_number, to_text, values_seq, SeqSource,
};
use tally_types::{CalcSettings, CellAddr, CellRange, DateEpoch, TextToNumber};

fn ctx() -> MockEvalContext {
    MockEvalContext::new()
}

#[test]
fn to_number_scalars() {
    let mut c = ctx();
    assert_eq!(to_number(&missing(), &mut c), Ok(0.0));
    assert_eq!(to_number(&empty(), &mut c), Ok(0.0));
    assert_eq!(to_number(&n(2.5), &mut c), Ok(2.5));
    assert_eq!(to_number(&b(true), &mut c), Ok(1.0));
    assert_eq!(to_number(&b(false), &mut c), Ok(0.0));
    // ConvertUnambiguous: integers only (AOO ConvertStringToValue).
    assert_eq!(to_number(&t("12.5"), &mut c), Err(ErrorKind::Value));
    assert_eq!(to_number(&t("12"), &mut c), Ok(12.0));
    assert_eq!(to_number(&t(" 3 "), &mut c), Ok(3.0));
    assert_eq!(to_number(&t("1e3"), &mut c), Ok(1000.0));
    assert_eq!(to_number(&t("abc"), &mut c), Err(ErrorKind::Value));
    assert_eq!(to_number(&t(""), &mut c), Err(ErrorKind::Value));
    assert_eq!(to_number(&e(ErrorKind::NA), &mut c), Err(ErrorKind::NA));
}

#[test]
fn text_to_number_modes() {
    let mut s = CalcSettings::default();
    assert_eq!(text_to_number("7", &s), Ok(7.0));
    assert_eq!(text_to_number("2024-01-15", &s), Ok(45_306.0));
    assert_eq!(text_to_number("1,000", &s), Err(ErrorKind::Value));
    s.text_to_number = TextToNumber::GenerateError;
    assert_eq!(text_to_number("7", &s), Err(ErrorKind::Value));
    s.text_to_number = TextToNumber::TreatAsZero;
    assert_eq!(text_to_number("7", &s), Ok(0.0));
    assert_eq!(text_to_number("abc", &s), Ok(0.0));
    s.text_to_number = TextToNumber::ConvertLocale;
    assert_eq!(text_to_number("7", &s), Ok(7.0));
    assert_eq!(text_to_number("abc", &s), Err(ErrorKind::Value));
    // en-US recognizer: thousands separators, currency, percent, US dates.
    assert_eq!(text_to_number("1,000", &s), Ok(1000.0));
    assert_eq!(text_to_number("$1,234.50", &s), Ok(1234.5));
    assert_eq!(text_to_number("50%", &s), Ok(0.5));
    assert_eq!(text_to_number("(12)", &s), Ok(-12.0));
    assert_eq!(text_to_number("1/15/2024", &s), Ok(45_306.0));
    assert_eq!(text_to_number("TRUE", &s), Ok(1.0));
    assert_eq!(text_to_number("", &s), Err(ErrorKind::Value));
    assert_eq!(text_to_number("1,00", &s), Err(ErrorKind::Value));
}

/// AOO interpr4.cxx ConvertStringToValue (review 2 Low: `"1.5"+1` is #VALUE!).
#[test]
fn convert_string_to_value_is_aoo_strict() {
    let p = |s| convert_string_to_value(s, DateEpoch::D1899);
    assert_eq!(p("3"), Some(3.0));
    assert_eq!(p(" 3  "), Some(3.0));
    assert_eq!(p("-2E3"), Some(-2000.0));
    assert_eq!(p("+7"), Some(7.0));
    assert_eq!(p("1e+2"), Some(100.0));
    for bad in [
        "",
        " ",
        "1.5",
        ".5",
        "5.",
        "-1.5",
        "1e",
        "1,000",
        "$5",
        "5%",
        "TRUE",
        "1 2",
        "abc",
        "-2024-01-15",
        "2024-01",
        "2024-01-15T",
        "12:",
        "12:30:",
        "2024-01-15T24:00",
        "12:60",
        "2023-02-29",
        "2024-001-15",
        "1e3-01-01",
    ] {
        assert_eq!(p(bad), None, "{bad:?}");
    }
    assert_eq!(p("2024-01-15"), Some(45_306.0));
    assert_eq!(p("2024-01-15T12:00"), Some(45_306.5));
    assert_eq!(p("2024-01-15 06:00:00"), Some(45_306.25));
    assert_eq!(p("2024-01-15 06:00:00,5"), Some(45_306.25 + 0.5 / 86_400.0));
    assert_eq!(p("2024-01-00"), Some(45_292.0), "day 00 reads as 1 (AOO)");
    assert_eq!(
        p("2024-1-15"),
        Some(45_306.0),
        "one-digit units are allowed"
    );
    assert_eq!(p("12:00"), Some(0.5));
    assert_eq!(p("36:00"), Some(1.5));
    assert_eq!(p("12:00 "), Some(0.5));
    assert_eq!(p("00:00:30.5"), Some(30.5 / 86_400.0));
    assert_eq!(p("00:00:00.05"), Some(0.05 / 86_400.0));
    assert_eq!(
        convert_string_to_value("1904-01-02", DateEpoch::D1904),
        Some(1.0)
    );
    let mut s = CalcSettings::default();
    assert_eq!(text_to_number("1.5", &s), Err(ErrorKind::Value));
    s.text_to_number = TextToNumber::ConvertLocale;
    assert_eq!(text_to_number("1.5", &s), Ok(1.5));
}

#[test]
fn unambiguous_parser() {
    let p = |s| parse_unambiguous(s, DateEpoch::D1899);
    assert_eq!(p("-1.5"), Some(-1.5));
    assert_eq!(p("+.5"), Some(0.5));
    assert_eq!(p("5."), Some(5.0));
    assert_eq!(p("2.5E-2"), Some(0.025));
    for bad in [
        "", " ", ".", "-", "1e", "1e+", "inf", "NaN", "0x10", "1 2", "1,5", "$5", "5%",
    ] {
        assert_eq!(p(bad), None, "{bad:?}");
    }
    assert_eq!(p("1899-12-30"), Some(0.0));
    assert_eq!(p("2024-02-29"), Some(45_351.0));
    assert_eq!(p("2023-02-29"), None);
    // tally-numfmt (the single implementation) accepts unpadded ISO parts.
    assert_eq!(p("2024-1-15"), Some(45_306.0));
    assert_eq!(p("2024-01-15T12:00"), Some(45_306.5));
    assert_eq!(p("2024-01-15 06:00:00"), Some(45_306.25));
    assert_eq!(p("2024-01-15T24:00"), None);
    assert_eq!(p("12:00"), Some(0.5));
    assert_eq!(p("36:00"), Some(1.5));
    assert_eq!(p("00:00:30.5"), Some(30.5 / 86_400.0));
    assert_eq!(p("12:60"), None);
    assert_eq!(parse_unambiguous("1904-01-02", DateEpoch::D1904), Some(1.0));
    assert_eq!(
        // AOO "01/01/1900" epoch: serial 0 = 1900-01-01, no fake 1900-02-29.
        parse_unambiguous("1900-03-01", DateEpoch::D1900),
        Some(59.0)
    );
    assert_eq!(parse_unambiguous("1900-01-01", DateEpoch::D1900), Some(0.0));
}

#[test]
fn implicit_intersection() {
    let origin = CellAddr::new(0, 4, 3); // D5
    let col = CellRange::from_a1(0, "A1:A10").unwrap();
    assert_eq!(intersect(&col, origin), Ok(CellAddr::new(0, 4, 0)));
    let row = CellRange::from_a1(0, "A2:Z2").unwrap();
    assert_eq!(intersect(&row, origin), Ok(CellAddr::new(0, 1, 3)));
    let miss = CellRange::from_a1(0, "A6:A10").unwrap();
    assert_eq!(intersect(&miss, origin), Err(ErrorKind::Value));
    let block = CellRange::from_a1(0, "A1:B10").unwrap();
    assert_eq!(intersect(&block, origin), Err(ErrorKind::Value));
    let single = CellRange::from_a1(0, "Z99").unwrap();
    assert_eq!(intersect(&single, origin), Ok(CellAddr::new(0, 98, 25)));
    // Another sheet: the origin's row is still used.
    let other = CellRange::from_a1(1, "B1:B10").unwrap();
    assert_eq!(intersect(&other, origin), Ok(CellAddr::new(1, 4, 1)));
}

#[test]
fn ranges_and_arrays_in_scalar_context() {
    let mut c = ctx();
    c.set(0, "A1", 10.0).set(0, "A2", "5").set(0, "A3", true);
    c.set_origin(0, "B2");
    let r = c.range(0, "A1:A3");
    assert_eq!(
        to_number(&r, &mut c),
        Ok(5.0),
        "intersects A2, text converts"
    );
    c.set_origin(0, "B9");
    assert_eq!(to_number(&r, &mut c), Err(ErrorKind::Value));
    let r1 = c.range(0, "A3");
    assert_eq!(to_number(&r1, &mut c), Ok(1.0));
    let a = arr(1, 2, vec![num(7.0), num(8.0)]);
    assert_eq!(to_number(&a, &mut c), Ok(7.0), "inline array: top-left");
    assert_eq!(
        scalar_value(&arr(0, 0, vec![]), &mut c),
        err(ErrorKind::Value)
    );
    assert_eq!(scalar_value(&missing(), &mut c), Value::Empty);
}

#[test]
fn to_text_rules() {
    let mut c = ctx();
    assert_eq!(to_text(&missing(), &mut c).as_deref(), Ok(""));
    assert_eq!(to_text(&n(1.5), &mut c).as_deref(), Ok("1.5"));
    assert_eq!(to_text(&b(true), &mut c).as_deref(), Ok("TRUE"));
    assert_eq!(to_text(&b(false), &mut c).as_deref(), Ok("FALSE"));
    assert_eq!(to_text(&t("x"), &mut c).as_deref(), Ok("x"));
    assert_eq!(to_text(&e(ErrorKind::Div0), &mut c), Err(ErrorKind::Div0));
    c.set(0, "A1", 3.0);
    let r = c.range(0, "A1");
    assert_eq!(to_text(&r, &mut c).as_deref(), Ok("3"));
}

#[test]
fn general_number_text() {
    assert_eq!(number_to_text(0.1 + 0.2), "0.3");
    assert_eq!(number_to_text(-0.0), "0");
    assert_eq!(number_to_text(42.0), "42");
    assert_eq!(number_to_text(-1.25), "-1.25");
    assert_eq!(number_to_text(1.0 / 3.0), "0.333333333333333");
    assert_eq!(number_to_text(123_456_789_012_345.0), "123456789012345");
    assert_eq!(number_to_text(1e15), "1E+15");
    assert_eq!(number_to_text(1e20), "1E+20");
    assert_eq!(number_to_text(1.5e-5), "1.5E-05");
    assert_eq!(number_to_text(0.0001), "0.0001");
    assert_eq!(number_to_text(-2.5e300), "-2.5E+300");
}

#[test]
fn to_bool_rules() {
    let mut c = ctx();
    assert_eq!(to_bool(&missing(), &mut c), Ok(false));
    assert_eq!(to_bool(&n(0.0), &mut c), Ok(false));
    assert_eq!(to_bool(&n(-0.5), &mut c), Ok(true));
    assert_eq!(to_bool(&t("true"), &mut c), Ok(true));
    assert_eq!(to_bool(&t("False"), &mut c), Ok(false));
    assert_eq!(to_bool(&t("yes"), &mut c), Err(ErrorKind::Value));
    assert_eq!(to_bool(&t("1"), &mut c), Err(ErrorKind::Value));
    assert_eq!(to_bool(&e(ErrorKind::Ref), &mut c), Err(ErrorKind::Ref));
}

#[test]
fn to_integer_rules() {
    let mut c = ctx();
    assert_eq!(to_integer(&n(2.9), &mut c), Ok(2));
    assert_eq!(to_integer(&n(-2.9), &mut c), Ok(-2), "toward zero");
    assert_eq!(
        to_integer(&n(2.999_999_999_999_999_6), &mut c),
        Ok(3),
        "snapped first"
    );
    assert_eq!(to_integer(&t("4"), &mut c), Ok(4));
    assert_eq!(
        to_integer(&n(1e12), &mut c),
        Err(ErrorKind::InvalidArgument)
    );
    assert_eq!(coerce::approx_floor(-2.5), -3.0);
}

fn collect(arg: &EvalValue, c: &mut MockEvalContext) -> Result<Vec<f64>, ErrorKind> {
    let mut v = Vec::new();
    number_seq(arg, c, |x| v.push(x)).map(|()| v)
}

#[test]
fn number_seq_direct_args_coerce() {
    let mut c = ctx();
    assert_eq!(collect(&n(1.0), &mut c), Ok(vec![1.0]));
    assert_eq!(collect(&b(true), &mut c), Ok(vec![1.0]));
    assert_eq!(collect(&t("3"), &mut c), Ok(vec![3.0]));
    assert_eq!(collect(&t("x"), &mut c), Err(ErrorKind::Value));
    assert_eq!(collect(&missing(), &mut c), Ok(vec![0.0]));
    assert_eq!(collect(&e(ErrorKind::NA), &mut c), Err(ErrorKind::NA));
}

#[test]
fn number_seq_references_skip_text_count_bools() {
    let mut c = ctx();
    c.set(0, "A1", 1.0)
        .set(0, "A2", "2")
        .set(0, "A3", true)
        .set(0, "B1", 4.0)
        .set(0, "B3", false);
    let r = c.range(0, "A1:B3");
    // Column-major order; text "2" skipped, bools counted, empty B2 skipped.
    assert_eq!(collect(&r, &mut c), Ok(vec![1.0, 1.0, 4.0, 0.0]));
    c.set(0, "B2", Value::Error(ErrorKind::Div0));
    c.set(0, "A2", Value::Error(ErrorKind::NA));
    assert_eq!(
        collect(&r, &mut c),
        Err(ErrorKind::NA),
        "first error column-major"
    );
}

#[test]
fn number_seq_arrays() {
    let mut c = ctx();
    let a = arr(
        2,
        2,
        vec![num(1.0), txt("x"), Value::Bool(true), Value::Empty],
    );
    assert_eq!(collect(&a, &mut c), Ok(vec![1.0, 1.0]));
    let a = arr(1, 2, vec![num(1.0), err(ErrorKind::Num)]);
    assert_eq!(collect(&a, &mut c), Err(ErrorKind::Num));
}

#[test]
fn text_as_zero_and_values_seq() {
    let mut c = ctx();
    c.set(0, "A1", 2.0).set(0, "A2", "abc").set(0, "A4", true);
    let r = c.range(0, "A1:A4");
    let mut v = Vec::new();
    number_seq_with_text_as_zero(&r, &mut c, |x| v.push(x)).unwrap();
    assert_eq!(v, vec![2.0, 0.0, 1.0]);

    c.set(0, "A3", Value::Error(ErrorKind::Ref));
    let mut seen = Vec::new();
    values_seq(&r, &mut c, |v, src| seen.push((v.clone(), src)));
    assert_eq!(seen.len(), 4, "errors visited, not propagated");
    assert!(seen.iter().all(|(_, s)| *s == SeqSource::Reference));
    let mut direct = Vec::new();
    values_seq(&missing(), &mut c, |v, src| direct.push((v.clone(), src)));
    assert_eq!(direct, vec![(Value::Empty, SeqSource::Direct)]);
}

#[test]
fn text_to_number_setting_applies_to_direct_seq_args() {
    let mut c = ctx();
    c.settings.text_to_number = TextToNumber::TreatAsZero;
    assert_eq!(collect(&t("x"), &mut c), Ok(vec![0.0]));
    c.settings.text_to_number = TextToNumber::GenerateError;
    assert_eq!(collect(&t("3"), &mut c), Err(ErrorKind::Value));
}

mod props {
    use super::*;
    use proptest::prelude::*;
    use tally_types::round_to_15_digits;

    proptest! {
        #[test]
        fn general_text_round_trips(x in proptest::num::f64::NORMAL) {
            let s = number_to_text(x);
            let back = parse_unambiguous(&s, DateEpoch::D1899).unwrap();
            prop_assert_eq!(back, round_to_15_digits(x));
        }

        #[test]
        fn plain_decimals_parse(x in -1e12f64..1e12) {
            let s = format!("{x}");
            prop_assert_eq!(parse_unambiguous(&s, DateEpoch::D1899), Some(x));
        }
    }
}
