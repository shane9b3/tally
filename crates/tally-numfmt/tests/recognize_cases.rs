//! Table-driven input recognition cases (en-US, today = 2026-10-19).

use tally_numfmt::date::serial_from_ymd;
use tally_numfmt::{
    parse_number_unambiguous, parse_number_unambiguous_with, recognize_input, recognize_input_with,
    Locale, RecognizeContext,
};
use tally_types::{DateEpoch, Value};

const CUR: &str = "[$$-409]#,##0.00;[RED]-[$$-409]#,##0.00";

#[derive(Debug)]
enum Exp {
    /// Number with implied format.
    N(f64, Option<&'static str>),
    /// Date `(y, m, d)` plus seconds of day, with implied format.
    Dt(i32, u32, u32, f64, &'static str),
    /// Time of day / duration in seconds, with implied format.
    Tm(f64, &'static str),
    /// Text with the given content.
    T(&'static str),
    B(bool),
    Empty,
}

use Exp::*;

const CASES: &[(&str, Exp)] = &[
    // --- plain numbers ---
    ("123", N(123.0, None)),
    ("-123", N(-123.0, None)),
    ("+123", N(123.0, None)),
    // Blanks after the sign are skipped (AOO ScanStartString).
    ("- 5", N(-5.0, None)),
    ("+ 5", N(5.0, None)),
    ("-", T("-")),
    ("- ", T("- ")),
    ("  42  ", N(42.0, None)),
    ("1.5", N(1.5, None)),
    (".5", N(0.5, None)),
    ("5.", N(5.0, None)),
    ("-0.25", N(-0.25, None)),
    ("0", N(0.0, None)),
    ("007", N(7.0, None)),
    ("1.50", N(1.5, None)),
    // --- grouping ---
    ("1,234", N(1234.0, Some("#,##0"))),
    ("1,234,567", N(1_234_567.0, Some("#,##0"))),
    ("1,234.5", N(1234.5, Some("#,##0.00"))),
    ("-1,234.56", N(-1234.56, Some("#,##0.00"))),
    ("1,23", T("1,23")),
    ("12,34,567", T("12,34,567")),
    ("1234,567", T("1234,567")),
    (",123", T(",123")),
    ("1,234e5", T("1,234e5")),
    ("1,,234", T("1,,234")),
    // --- accounting negatives ---
    ("(123)", N(-123.0, None)),
    ("(1,234.50)", N(-1234.5, Some("#,##0.00"))),
    ("(-5)", T("(-5)")),
    ("(5", T("(5")),
    // --- percent ---
    ("15%", N(0.15, Some("0%"))),
    ("12.5%", N(0.125, Some("0.00%"))),
    ("-15%", N(-0.15, Some("0%"))),
    ("100%", N(1.0, Some("0%"))),
    ("%15", T("%15")),
    ("15%%", T("15%%")),
    // --- currency ---
    ("$5", N(5.0, Some(CUR))),
    ("$1,200.50", N(1200.5, Some(CUR))),
    ("-$5", N(-5.0, Some(CUR))),
    ("$-5", N(-5.0, Some(CUR))),
    ("($5.00)", N(-5.0, Some(CUR))),
    ("$", T("$")),
    ("$5%", T("$5%")),
    ("$1e5", T("$1e5")),
    ("5$", T("5$")),
    // --- scientific ---
    ("1e5", N(100_000.0, Some("0.00E+00"))),
    ("1.5E-3", N(0.0015, Some("0.00E+00"))),
    ("-2.5e+10", N(-2.5e10, Some("0.00E+00"))),
    ("1e", T("1e")),
    ("e5", T("e5")),
    ("1e400", T("1e400")),
    ("inf", T("inf")),
    ("nan", T("nan")),
    ("Infinity", T("Infinity")),
    ("0x10", T("0x10")),
    // --- booleans and forced text ---
    ("TRUE", B(true)),
    ("false", B(false)),
    ("True", B(true)),
    (" TRUE ", B(true)),
    ("TRUEX", T("TRUEX")),
    ("'123", T("123")),
    ("'=A1", T("=A1")),
    ("'", T("")),
    ("=A1+1", T("=A1+1")),
    ("", Empty),
    ("   ", Empty),
    ("abc", T("abc")),
    ("12abc", T("12abc")),
    ("1 2", T("1 2")),
    ("1-2-3-4", T("1-2-3-4")),
    ("--5", T("--5")),
    // --- fractions ---
    ("0 1/2", N(0.5, Some("# ?/?"))),
    ("1 1/2", N(1.5, Some("# ?/?"))),
    ("-1 1/2", N(-1.5, Some("# ?/?"))),
    ("2 3/16", N(2.1875, Some("# ??/??"))),
    ("1   3/4", N(1.75, Some("# ?/?"))),
    ("1 1/0", T("1 1/0")),
    ("1 a/2", T("1 a/2")),
    // --- dates ---
    ("1/2", Dt(2026, 1, 2, 0.0, "MM/DD/YYYY")),
    ("10/19/2026", Dt(2026, 10, 19, 0.0, "MM/DD/YYYY")),
    ("10/19/26", Dt(2026, 10, 19, 0.0, "MM/DD/YY")),
    ("10/19/30", Dt(1930, 10, 19, 0.0, "MM/DD/YY")),
    ("10/19/29", Dt(2029, 10, 19, 0.0, "MM/DD/YY")),
    ("1/1/2000", Dt(2000, 1, 1, 0.0, "MM/DD/YYYY")),
    ("2/29/2024", Dt(2024, 2, 29, 0.0, "MM/DD/YYYY")),
    ("2/29/2023", T("2/29/2023")),
    ("13/1/2026", T("13/1/2026")),
    ("0/1/2026", T("0/1/2026")),
    ("1/32/2026", T("1/32/2026")),
    ("10/19", Dt(2026, 10, 19, 0.0, "MM/DD/YYYY")),
    ("10-19-2026", Dt(2026, 10, 19, 0.0, "MM-DD-YYYY")),
    ("10/19-2026", T("10/19-2026")),
    ("2026-10-19", Dt(2026, 10, 19, 0.0, "YYYY-MM-DD")),
    ("2026/10/19", Dt(2026, 10, 19, 0.0, "YYYY/MM/DD")),
    ("2026-13-01", T("2026-13-01")),
    ("1/2/3", Dt(2003, 1, 2, 0.0, "MM/DD/YY")),
    ("5/6/7890", Dt(7890, 5, 6, 0.0, "MM/DD/YYYY")),
    ("1/2/10000", T("1/2/10000")),
    ("19 Oct 2026", Dt(2026, 10, 19, 0.0, "DD MMM YYYY")),
    ("19 October 2026", Dt(2026, 10, 19, 0.0, "DD MMMM YYYY")),
    ("19-Oct-26", Dt(2026, 10, 19, 0.0, "DD-MMM-YY")),
    ("19 oct", Dt(2026, 10, 19, 0.0, "DD MMM YYYY")),
    ("31 Feb 2026", T("31 Feb 2026")),
    ("Oct 19, 2026", Dt(2026, 10, 19, 0.0, "MMM DD, YYYY")),
    ("October 19 2026", Dt(2026, 10, 19, 0.0, "MMMM DD YYYY")),
    ("Oct 19", Dt(2026, 10, 19, 0.0, "MMM DD, YYYY")),
    ("Oct 2026", Dt(2026, 10, 1, 0.0, "MMM YYYY")),
    ("Foo 19, 2026", T("Foo 19, 2026")),
    ("May 5 2025", Dt(2025, 5, 5, 0.0, "MMMM DD YYYY")),
    // --- times ---
    ("14:30", Tm(52_200.0, "HH:MM")),
    ("2:30 PM", Tm(52_200.0, "HH:MM AM/PM")),
    ("2:30PM", Tm(52_200.0, "HH:MM AM/PM")),
    ("2:30 am", Tm(9000.0, "HH:MM AM/PM")),
    ("2:30 p", Tm(52_200.0, "HH:MM AM/PM")),
    ("12:00 AM", Tm(0.0, "HH:MM AM/PM")),
    ("12:00 PM", Tm(43_200.0, "HH:MM AM/PM")),
    ("12:30 am", Tm(1800.0, "HH:MM AM/PM")),
    ("14:30:15", Tm(52_215.0, "HH:MM:SS")),
    ("2:30:15.5 pm", Tm(52_215.5, "HH:MM:SS.00 AM/PM")),
    ("0:00", Tm(0.0, "HH:MM")),
    ("23:59:59", Tm(86_399.0, "HH:MM:SS")),
    ("2:5", Tm(7500.0, "HH:MM")),
    ("25:30", Tm(91_800.0, "[HH]:MM")),
    ("100:00:00", Tm(360_000.0, "[HH]:MM:SS")),
    // Negative durations (correctness review H1).
    ("-6:00:00", Tm(-21_600.0, "[HH]:MM:SS")),
    ("-1:00", Tm(-3600.0, "[HH]:MM")),
    ("-36:00:00", Tm(-129_600.0, "[HH]:MM:SS")),
    ("-00:00:00.500", Tm(-0.5, "[HH]:MM:SS.00")),
    ("-2:30 PM", T("-2:30 PM")),
    ("--1:00", T("--1:00")),
    ("13:00 PM", T("13:00 PM")),
    ("14:60", T("14:60")),
    ("14:30:60", T("14:30:60")),
    ("1:2:3:4", T("1:2:3:4")),
    ("14:", T("14:")),
    (":30", T(":30")),
    ("2:30 xm", T("2:30 xm")),
    // --- date + time ---
    (
        "2026-10-19 14:30",
        Dt(2026, 10, 19, 52_200.0, "YYYY-MM-DD HH:MM"),
    ),
    (
        "10/19/2026 2:30 PM",
        Dt(2026, 10, 19, 52_200.0, "MM/DD/YYYY HH:MM AM/PM"),
    ),
    (
        "2026-10-19T14:30",
        Dt(2026, 10, 19, 52_200.0, "YYYY-MM-DD HH:MM"),
    ),
    (
        "19 Oct 2026 14:30:15",
        Dt(2026, 10, 19, 52_215.0, "DD MMM YYYY HH:MM:SS"),
    ),
    ("2026-10-19 25:00", T("2026-10-19 25:00")),
    ("abc 14:30", T("abc 14:30")),
];

fn ctx(epoch: DateEpoch) -> RecognizeContext {
    RecognizeContext {
        epoch,
        today: (2026, 10, 19),
        year2000: 1930,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn recognizer_table() {
    let l = Locale::en_us();
    let c = ctx(DateEpoch::D1899);
    let mut failures = Vec::new();
    for (input, exp) in CASES {
        let r = recognize_input_with(input, &l, &c);
        let fmt = r.implied_format.as_deref();
        let ok = match exp {
            N(v, f) => matches!(r.value, Value::Number(x) if close(x, *v)) && fmt == *f,
            Dt(y, m, d, secs, f) => {
                let want = serial_from_ymd(DateEpoch::D1899, *y, *m, *d).unwrap() + secs / 86400.0;
                matches!(r.value, Value::Number(x) if close(x, want)) && fmt == Some(*f)
            }
            Tm(secs, f) => {
                matches!(r.value, Value::Number(x) if close(x, secs / 86400.0)) && fmt == Some(*f)
            }
            T(s) => r.value == Value::text(*s) && fmt.is_none(),
            B(b) => r.value == Value::Bool(*b) && fmt.is_none(),
            Empty => r.value == Value::Empty,
        };
        if !ok {
            failures.push(format!("{input:?}: got {r:?}, want {exp:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} failed:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
    assert!(CASES.len() >= 80);
}

#[test]
fn recognizer_epochs() {
    let l = Locale::en_us();
    let r = recognize_input_with("2026-10-19", &l, &ctx(DateEpoch::D1904));
    assert_eq!(r.value, Value::Number(44852.0));
    let r = recognize_input_with("2026-10-19", &l, &ctx(DateEpoch::D1900));
    assert_eq!(r.value, Value::Number(46312.0));
    let r = recognize_input_with("2026-10-19", &l, &ctx(DateEpoch::D1899));
    assert_eq!(r.value, Value::Number(46314.0));
    // The default entry point uses today's date and the 1899 epoch.
    let r = recognize_input("2026-10-19", &l);
    assert_eq!(r.value, Value::Number(46314.0));
}

/// The two-digit year window (P17: AOO Tools ▸ Options ▸ General ▸ "Year
/// (two digits)", the document's `table:null-year`).
#[test]
fn two_digit_year_window() {
    let l = Locale::en_us();
    let year = |input: &str, start: u16| -> i32 {
        let c = RecognizeContext {
            year2000: start,
            ..ctx(DateEpoch::D1899)
        };
        let Value::Number(x) = recognize_input_with(input, &l, &c).value else {
            panic!("{input} is not a date");
        };
        let (y, _, _) = tally_numfmt::date::ymd_from_serial(DateEpoch::D1899, x.floor()).unwrap();
        y
    };
    assert_eq!(year("1/1/45", 1930), 1945);
    assert_eq!(year("1/1/29", 1930), 2029);
    assert_eq!(year("1/1/30", 1930), 1930);
    assert_eq!(year("1/1/45", 1950), 2045);
    assert_eq!(year("1/1/50", 1950), 1950);
    assert_eq!(year("1/1/49", 1950), 2049);
    assert_eq!(year("1/1/45", 2000), 2045);
    assert_eq!(year("1/1/99", 2000), 2099);
    assert_eq!(year("1/1/00", 2000), 2000);
    // Four-digit years are never moved.
    assert_eq!(year("1/1/1945", 1950), 1945);
    // Month-name forms use the window too.
    assert_eq!(year("Jan 1, 45", 1950), 2045);
    // German order (DD.MM.YY).
    let de = tally_numfmt::locale_by_tag("de-DE").unwrap();
    let c = RecognizeContext {
        year2000: 1950,
        ..ctx(DateEpoch::D1899)
    };
    let Value::Number(x) = recognize_input_with("1.1.45", de, &c).value else {
        panic!("1.1.45 is not a date in de-DE");
    };
    assert_eq!(
        tally_numfmt::date::ymd_from_serial(DateEpoch::D1899, x).unwrap(),
        (2045, 1, 1)
    );
}

#[test]
fn unambiguous() {
    let e = DateEpoch::D1899;
    let cases: &[(&str, Option<f64>)] = &[
        ("1.5", Some(1.5)),
        (" -2e3 ", Some(-2000.0)),
        (".5", Some(0.5)),
        ("+5", Some(5.0)),
        ("1,000", None),
        ("$5", None),
        ("5%", None),
        ("(5)", None),
        ("2026-10-19", Some(46314.0)),
        ("2026-10-19 12:00", Some(46314.5)),
        ("2026-10-19T06:00:00", Some(46314.25)),
        ("12:00", Some(0.5)),
        ("25:00", Some(25.0 / 24.0)),
        ("12:00:30.5", Some((43_230.5) / 86400.0)),
        ("10/19/2026", None),
        ("19 Oct 2026", None),
        ("TRUE", None),
        ("", None),
        ("abc", None),
        ("1e400", None),
        ("2:30 PM", None),
        ("26-10-19", None),
        ("2026-02-30", None),
        ("1 1/2", None),
    ];
    for (s, want) in cases {
        let got = parse_number_unambiguous_with(s, e);
        let ok = match (got, want) {
            (Some(g), Some(w)) => close(g, *w),
            (None, None) => true,
            _ => false,
        };
        assert!(ok, "{s:?}: got {got:?}, want {want:?}");
    }
    assert_eq!(parse_number_unambiguous("2026-10-19"), Some(46314.0));
    assert_eq!(
        parse_number_unambiguous_with("2026-10-19", DateEpoch::D1904),
        Some(44852.0)
    );
}
