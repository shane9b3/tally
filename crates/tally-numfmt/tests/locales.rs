//! Formatting and input recognition for the v0.3 locales.
#![allow(clippy::unwrap_used)]

use tally_numfmt::{
    builtin_format, format_value_with, general_string, parse_format, recognize_input_with, Locale,
    QuickFormat, RecognizeContext, DE_DE, EN_GB, EN_US, ES_ES, FR_FR, JA_JP,
};
use tally_types::{DateEpoch, Value};

fn ctx() -> RecognizeContext {
    RecognizeContext {
        epoch: DateEpoch::D1899,
        today: (2026, 10, 19),
    }
}

fn fmt(v: f64, code: &str, l: &Locale) -> String {
    let ast = parse_format(code).unwrap();
    format_value_with(&Value::Number(v), &ast, l, None, DateEpoch::D1899).text
}

fn rec(s: &str, l: &Locale) -> (Value, Option<String>) {
    let r = recognize_input_with(s, l, &ctx());
    (r.value, r.implied_format)
}

fn num(s: &str, l: &Locale) -> Option<f64> {
    match rec(s, l).0 {
        Value::Number(n) => Some(n),
        _ => None,
    }
}

/// 2026-10-19 as a 1899-12-30 serial.
const OCT19: f64 = 46314.0;

#[test]
fn de_de() {
    let l = &DE_DE;
    assert_eq!(fmt(1234.5, "#,##0.00", l), "1.234,50");
    assert_eq!(general_string(0.5, None, l), "0,5");
    assert_eq!(
        fmt(1234.5, &builtin_format(QuickFormat::Currency, l), l),
        "1.234,50 €"
    );
    assert_eq!(fmt(OCT19, "DD.MM.YYYY", l), "19.10.2026");
    assert_eq!(
        fmt(OCT19, "NNNND. MMMM YYYY", l),
        "Montag, 19. Oktober 2026"
    );
    assert_eq!(fmt(OCT19 - 214.0, "MMM", l), "Mär");
    assert_eq!(num("1.234,5", l), Some(1234.5));
    assert_eq!(num("0,25", l), Some(0.25));
    assert_eq!(num("12,50 €", l), Some(12.5));
    assert_eq!(num("12,50€", l), Some(12.5));
    assert_eq!(num("-3,5", l), Some(-3.5));
    assert_eq!(num("50%", l), Some(0.5));
    assert_eq!(
        rec("19.10.2026", l),
        (Value::Number(OCT19), Some("DD.MM.YYYY".into()))
    );
    assert_eq!(num("19.10.", l), Some(OCT19));
    assert_eq!(num("19. Oktober 2026", l), Some(OCT19));
    assert_eq!(rec("WAHR", l).0, Value::Bool(true));
    assert_eq!(rec("true", l).0, Value::Bool(true), "English booleans too");
    assert!(matches!(rec("1,2,3", l).0, Value::Text(_)));
    assert_eq!(
        num("14:30:15,5", l).map(|v| (v * 86400.0).round()),
        Some(52215.0 + 1.0)
    );
}

#[test]
fn fr_fr() {
    let l = &FR_FR;
    assert_eq!(fmt(1234567.891, "#,##0.00", l), "1\u{a0}234\u{a0}567,89");
    assert_eq!(
        fmt(5.0, &builtin_format(QuickFormat::Currency, l), l),
        "5,00 €"
    );
    assert_eq!(fmt(OCT19, "DD MMMM YYYY", l), "19 octobre 2026");
    assert_eq!(fmt(OCT19, "NN", l), "lun.");
    assert_eq!(num("1 234,5", l), Some(1234.5));
    assert_eq!(num("1\u{a0}234,5", l), Some(1234.5));
    assert_eq!(num("1\u{202f}234,5", l), Some(1234.5));
    assert_eq!(
        rec("19/10/2026", l),
        (Value::Number(OCT19), Some("DD/MM/YYYY".into()))
    );
    assert_eq!(num("19 oct. 2026", l), Some(OCT19));
    assert_eq!(num("19 octobre 2026", l), Some(OCT19));
    assert!(num("19 févr. 2026", l).is_some());
    assert_eq!(rec("VRAI", l).0, Value::Bool(true));
    assert_eq!(rec("faux", l).0, Value::Bool(false));
}

#[test]
fn es_es() {
    let l = &ES_ES;
    assert_eq!(fmt(1234.5, "#,##0.00", l), "1.234,50");
    assert_eq!(fmt(OCT19, "DD MMMM YYYY", l), "19 octubre 2026");
    assert_eq!(fmt(0.75, "HH:MM AM/PM", l), "06:00 p. m.");
    assert_eq!(num("1.234,5", l), Some(1234.5));
    assert_eq!(num("19/10/2026", l), Some(OCT19));
    assert_eq!(num("19 oct 2026", l), Some(OCT19));
    assert_eq!(num("6:00 p. m.", l), Some(0.75));
    assert_eq!(rec("VERDADERO", l).0, Value::Bool(true));
    assert_eq!(
        fmt(3.0, &builtin_format(QuickFormat::Currency, l), l),
        "3,00 €"
    );
}

#[test]
fn en_gb() {
    let l = &EN_GB;
    assert_eq!(fmt(1234.5, "#,##0.00", l), "1,234.50");
    assert_eq!(
        fmt(1234.5, &builtin_format(QuickFormat::Currency, l), l),
        "£1,234.50"
    );
    assert_eq!(num("£12.50", l), Some(12.5));
    assert_eq!(
        rec("19/10/2026", l),
        (Value::Number(OCT19), Some("DD/MM/YYYY".into()))
    );
    assert!(
        matches!(rec("10/19/2026", l).0, Value::Text(_)),
        "month 19 does not exist"
    );
    assert_eq!(num("19 Oct 2026", l), Some(OCT19));
    assert_eq!(
        fmt(OCT19, &builtin_format(QuickFormat::Date, l), l),
        "19/10/2026"
    );
}

#[test]
fn ja_jp() {
    let l = &JA_JP;
    assert_eq!(fmt(1234.5, "#,##0.00", l), "1,234.50");
    assert_eq!(
        fmt(1234.0, &builtin_format(QuickFormat::Currency, l), l),
        "￥1,234"
    );
    assert_eq!(
        fmt(OCT19, &builtin_format(QuickFormat::Date, l), l),
        "2026/10/19"
    );
    assert_eq!(fmt(OCT19, "MMMM", l), "10月");
    assert_eq!(fmt(OCT19, "NNN", l), "月曜日");
    assert_eq!(fmt(0.75, "AM/PM HH:MM", l), "午後 06:00");
    assert_eq!(num("2026/10/19", l), Some(OCT19));
    assert_eq!(num("¥1,234", l), Some(1234.0));
    assert_eq!(num("￥1,234", l), Some(1234.0));
    assert_eq!(num("午後 6:00", l), Some(0.75));
}

#[test]
fn en_us_unchanged() {
    let l = &EN_US;
    assert_eq!(num("1,234.5", l), Some(1234.5));
    assert_eq!(num("10/19/2026", l), Some(OCT19));
    assert!(matches!(rec("1.234,5", l).0, Value::Text(_)));
    assert!(matches!(rec("5€", l).0, Value::Text(_)));
}
