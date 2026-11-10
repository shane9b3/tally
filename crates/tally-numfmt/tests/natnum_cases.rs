//! `[NatNumN]` / `[DBNumN]` / `[~calendar]` / era-code cases.
//!
//! The table (`tests/data/natnum_cases.json`) is shared with the Format
//! Cells preview (`packages/app/src/natnum.test.ts`) so both render the same.
//! Expectations are transcribed from AOO 4.1 (`nativenumbersupplier.cxx`,
//! `calendar_gregorian.cxx`, `calendar_hijri.cxx`, `calendar_jewish.cxx`,
//! `zformat.cxx`); see `docs/numfmt-natnum.md` for the deliberate
//! differences (Reiwa).

use tally_numfmt::{format_value_with, locale_by_tag, parse_format, print_format};
use tally_types::{DateEpoch, Value};

#[test]
fn natnum_table() {
    let data = include_str!("data/natnum_cases.json");
    let cases: serde_json::Value = serde_json::from_str(data).expect("valid JSON");
    let mut failures = Vec::new();
    let cases = cases.as_array().expect("array");
    assert!(cases.len() > 70, "{}", cases.len());
    for c in cases {
        let value = c["value"].as_f64().expect("value");
        let code = c["code"].as_str().expect("code");
        let tag = c["locale"].as_str().expect("locale");
        let want = c["want"].as_str().expect("want");
        let locale = locale_by_tag(tag).expect("locale");
        let ast = match parse_format(code) {
            Ok(a) => a,
            Err(e) => {
                failures.push(format!("{code}: parse error {e}"));
                continue;
            }
        };
        // Codes print back unchanged.
        if print_format(&ast) != code {
            failures.push(format!("{code}: prints as {}", print_format(&ast)));
        }
        let got = format_value_with(&Value::Number(value), &ast, locale, None, DateEpoch::D1899);
        if got.text != want {
            failures.push(format!(
                "{value} {code} [{tag}]: got {:?}, want {want:?}",
                got.text
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn era_r_is_a_literal_in_number_sections() {
    // `R` (rand) before a number is text, as it was before era codes.
    let ast = parse_format("R #,##0.00").unwrap();
    let got = format_value_with(
        &Value::Number(1234.5),
        &ast,
        &tally_numfmt::EN_US,
        None,
        DateEpoch::D1899,
    );
    assert_eq!(got.text, "R 1,234.50");
    assert_eq!(print_format(&ast), "R #,##0.00");
    let ast = parse_format("RR 0").unwrap();
    assert_eq!(print_format(&ast), "RR 0");
}

#[test]
fn modifier_limits() {
    assert!(parse_format("[NatNum19]0").is_ok());
    assert!(parse_format("[NatNum20]0").is_err());
    assert!(parse_format("[DBNum0]0").is_err());
    assert!(parse_format("[DBNum9]0").is_ok());
    // A NatNum that is not valid for the language changes nothing.
    let ast = parse_format("[NatNum12][$-411]0").unwrap();
    let got = format_value_with(
        &Value::Number(12.0),
        &ast,
        &tally_numfmt::EN_US,
        None,
        DateEpoch::D1899,
    );
    assert_eq!(got.text, "12");
    // The column width counts the transliterated text.
    let ast = parse_format("[NatNum4][$-411]0").unwrap();
    let got = format_value_with(
        &Value::Number(123_456.0),
        &ast,
        &tally_numfmt::EN_US,
        Some(5),
        DateEpoch::D1899,
    );
    assert_eq!(got.text, "###");
    // TEXT(): the document language is the calendar language, a tag only
    // sets the NatNum language.
    let ast = parse_format("[NatNum1][$-411]0").unwrap();
    let got = tally_numfmt::format_value_in_locale(
        &Value::Number(12.0),
        &ast,
        &tally_numfmt::EN_US,
        None,
        DateEpoch::D1899,
    );
    assert_eq!(got.text, "一二");
}
