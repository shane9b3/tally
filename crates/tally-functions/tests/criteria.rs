//! Criterion parsing and matching across Wildcards, Regex and Literal modes.

#![allow(clippy::unwrap_used)]

mod common;
use common::*;

use tally_functions::{CmpOp, Criterion};
use tally_types::{CalcSettings, CriteriaMode};

#[derive(Copy, Clone)]
struct Mode {
    mode: CriteriaMode,
    case: bool,
    whole: bool,
}

const WILD: Mode = Mode {
    mode: CriteriaMode::Wildcards,
    case: false,
    whole: true,
};
const REGEX: Mode = Mode {
    mode: CriteriaMode::Regex,
    case: false,
    whole: true,
};
const LIT: Mode = Mode {
    mode: CriteriaMode::Literal,
    case: false,
    whole: true,
};

fn settings(m: Mode) -> CalcSettings {
    CalcSettings {
        criteria_mode: m.mode,
        case_sensitive: m.case,
        whole_cell_match: m.whole,
        ..CalcSettings::default()
    }
}

fn check(m: Mode, crit: Value, cell: Value, expect: bool) {
    let c = Criterion::parse(&crit, &settings(m));
    assert_eq!(
        c.matches(&cell),
        expect,
        "mode {:?} case {} whole {}: criterion {crit:?} vs cell {cell:?}",
        m.mode,
        m.case,
        m.whole
    );
}

fn s(x: &str) -> Value {
    txt(x)
}

#[test]
fn defaults_documented() {
    let d = CalcSettings::default();
    assert_eq!(d.criteria_mode, CriteriaMode::Wildcards);
    assert!(!d.case_sensitive);
    assert!(d.whole_cell_match);
}

#[test]
fn operators_parse() {
    let d = CalcSettings::default();
    for (text, op) in [
        ("=1", CmpOp::Eq),
        ("<>1", CmpOp::Ne),
        ("<1", CmpOp::Lt),
        ("<=1", CmpOp::Le),
        (">1", CmpOp::Gt),
        (">=1", CmpOp::Ge),
        ("1", CmpOp::Eq),
        ("abc", CmpOp::Eq),
    ] {
        assert_eq!(Criterion::parse(&s(text), &d).op(), op, "{text}");
    }
    assert!(Criterion::parse(&s(">=5"), &d).is_numeric());
    assert!(!Criterion::parse(&s(">=x"), &d).is_numeric());
}

#[test]
fn numeric_criteria() {
    let cases: &[(&str, Value, bool)] = &[
        ("5", num(5.0), true),
        ("5", num(5.000_000_000_000_001), true),
        ("5", num(6.0), false),
        ("5", s("5"), true),
        ("5", s("5.0"), true),
        ("=5", s("five"), false),
        (">5", num(6.0), true),
        (">5", num(5.0), false),
        (">5", s("6"), false),
        (">=5", num(5.0), true),
        ("<5", num(-1.0), true),
        ("<5", Value::Empty, false),
        ("<=5", num(5.0), true),
        ("<>5", num(5.0), false),
        ("<>5", num(4.0), true),
        ("<>5", s("x"), true),
        ("<>5", Value::Empty, true),
        ("=1", Value::Bool(true), true),
        (">0", Value::Bool(false), false),
        (">2024-01-01", num(45_306.0), true),
        ("=2024-01-15", num(45_306.0), true),
        ("1e3", num(1000.0), true),
    ];
    for (crit, cell, expect) in cases {
        for m in [WILD, REGEX, LIT] {
            check(m, s(crit), cell.clone(), *expect);
        }
    }
}

#[test]
fn non_text_criterion_values() {
    let d = WILD;
    check(d, num(3.0), num(3.0), true);
    check(d, num(3.0), s("3"), true);
    check(d, num(3.0), num(4.0), false);
    check(d, Value::Bool(true), Value::Bool(true), true);
    check(d, Value::Bool(true), num(1.0), true);
    check(d, Value::Bool(false), Value::Bool(true), false);
    check(d, Value::Empty, num(0.0), true);
    check(d, Value::Empty, Value::Empty, false);
    check(d, err(ErrorKind::NA), err(ErrorKind::NA), true);
    check(d, err(ErrorKind::NA), err(ErrorKind::Div0), false);
    check(d, s("#N/A"), err(ErrorKind::NA), true);
    check(d, s("<>#N/A"), num(1.0), true);
    check(d, s("TRUE"), Value::Bool(true), true);
    check(d, s("false"), Value::Bool(true), false);
}

#[test]
fn empty_and_blank_criteria() {
    for m in [WILD, REGEX, LIT] {
        check(m, s(""), Value::Empty, true);
        check(m, s(""), s(""), true);
        check(m, s(""), num(0.0), false);
        check(m, s("="), Value::Empty, true);
        check(m, s("="), s(""), false);
        check(m, s("="), s("a"), false);
        check(m, s("<>"), Value::Empty, false);
        check(m, s("<>"), s("a"), true);
        check(m, s("<>"), num(0.0), true);
        check(m, s("<>"), s(""), true);
    }
}

#[test]
fn wildcard_mode() {
    let m = WILD;
    check(m, s("a*"), s("apple"), true);
    check(m, s("a*"), s("banana"), false);
    check(m, s("*an*"), s("banana"), true);
    check(m, s("b?n*"), s("banana"), true);
    check(m, s("?"), s("ab"), false);
    check(m, s("??"), s("ab"), true);
    check(m, s("*"), s(""), true);
    check(m, s("*"), Value::Empty, false);
    check(m, s("*"), num(1.0), false);
    check(m, s("a*b*c"), s("aXXbYYc"), true);
    check(m, s("a*b*c"), s("aXXbYY"), false);
    check(m, s("Apple"), s("APPLE"), true);
    check(m, s("<>a*"), s("apple"), false);
    check(m, s("<>a*"), s("pear"), true);
    check(m, s("<>a*"), Value::Empty, true);
    check(m, s("<>a*"), num(3.0), true);
    // Tilde escapes.
    check(m, s("~*"), s("*"), true);
    check(m, s("~*"), s("x"), false);
    check(m, s("what~?"), s("what?"), true);
    check(m, s("what~?"), s("whats"), false);
    check(m, s("a~~b"), s("a~b"), true);
    check(m, s("a~b"), s("a~b"), true);
    check(m, s("100%~"), s("100%~"), true);
    // Regex metacharacters are literal in wildcard mode.
    check(m, s("a.c"), s("abc"), false);
    check(m, s("a.c"), s("a.c"), true);
    check(m, s("(x)"), s("(x)"), true);
}

#[test]
fn regex_mode() {
    let m = REGEX;
    check(m, s("a.c"), s("abc"), true);
    check(m, s("a.c"), s("abcd"), false);
    check(m, s("^b"), s("banana"), false);
    check(m, s("b.*"), s("banana"), true);
    check(m, s("[0-9]+"), s("123"), true);
    check(m, s("[0-9]+"), s("12a"), false);
    check(m, s("APP.*"), s("apple"), true);
    check(m, s("a|b"), s("b"), true);
    check(m, s("a*"), s("aaa"), true);
    check(m, s("(unclosed"), s("(unclosed"), false);
    check(m, s("<>(unclosed"), s("x"), false);
    check(m, s("<>a.c"), s("abc"), false);
    check(m, s("<>a.c"), s("xyz"), true);
    // Huge patterns are refused by the size limit rather than compiled.
    let huge = "(a{1000}){1000}";
    check(m, s(huge), s("a"), false);
}

#[test]
fn literal_mode() {
    let m = LIT;
    check(m, s("a*"), s("a*"), true);
    check(m, s("a*"), s("apple"), false);
    check(m, s("a.c"), s("abc"), false);
    check(m, s("ABC"), s("abc"), true);
    check(m, s("<>abc"), s("ABC"), false);
    check(m, s("~*"), s("~*"), true);
}

#[test]
fn whole_cell_off() {
    for mode in [
        CriteriaMode::Wildcards,
        CriteriaMode::Regex,
        CriteriaMode::Literal,
    ] {
        let m = Mode {
            mode,
            case: false,
            whole: false,
        };
        check(m, s("an"), s("banana"), true);
        check(m, s("an"), s("pear"), false);
        check(m, s("<>an"), s("banana"), false);
        check(m, s("<>an"), s("pear"), true);
    }
    let m = Mode {
        mode: CriteriaMode::Wildcards,
        case: false,
        whole: false,
    };
    check(m, s("b?n"), s("abanana"), true);
    let m = Mode {
        mode: CriteriaMode::Regex,
        case: false,
        whole: false,
    };
    check(m, s("^ban"), s("banana"), true);
    check(m, s("^ban"), s("abanana"), false);
}

#[test]
fn case_sensitivity() {
    for mode in [
        CriteriaMode::Wildcards,
        CriteriaMode::Regex,
        CriteriaMode::Literal,
    ] {
        let on = Mode {
            mode,
            case: true,
            whole: true,
        };
        let off = Mode {
            mode,
            case: false,
            whole: true,
        };
        check(on, s("Apple"), s("apple"), false);
        check(on, s("Apple"), s("Apple"), true);
        check(off, s("Apple"), s("aPPLE"), true);
        check(on, s("<>Apple"), s("apple"), true);
    }
    // Non-ASCII folding.
    check(WILD, s("ÄPFEL"), s("äpfel"), true);
    check(Mode { case: true, ..WILD }, s("ÄPFEL"), s("äpfel"), false);
}

#[test]
fn text_ordering() {
    let m = WILD;
    check(m, s(">b"), s("c"), true);
    check(m, s(">b"), s("a"), false);
    check(m, s(">b"), s("B"), false);
    check(m, s(">=b"), s("B"), true);
    check(m, s("<b"), s("Apple"), true);
    check(m, s("<b"), num(1.0), false);
    check(m, s(">a*"), s("a*b"), true);
    check(Mode { case: true, ..m }, s("<b"), s("B"), true);
}

#[test]
fn array_values_use_top_left() {
    let d = CalcSettings::default();
    let m = std::sync::Arc::new(Matrix::from_vec(1, 2, vec![num(2.0), num(3.0)]).unwrap());
    let c = Criterion::parse(&Value::Array(m.clone()), &d);
    assert!(c.matches(&num(2.0)));
    assert!(Criterion::parse(&num(2.0), &d).matches(&Value::Array(m)));
}

/// Security review M2: a 4,000-character wildcard pattern against 1 MB of
/// text (3.2 s per cell with the old backtracking matcher).
#[test]
fn wildcard_matching_is_linear() {
    let text = s(&"a".repeat(1_000_000));
    let c = Criterion::parse(&s(&format!("*{}b", "a".repeat(4000))), &settings(WILD));
    let t = std::time::Instant::now();
    assert!(!c.matches(&text));
    let c2 = Criterion::parse(&s(&format!("*{}?b*", "a".repeat(4000))), &settings(WILD));
    assert!(!c2.matches(&text));
    let c3 = Criterion::parse(&s(&format!("*{}*", "a?".repeat(2000))), &settings(WILD));
    assert!(c3.matches(&text));
    let took = t.elapsed();
    // Generous: shared CI runners took 2.1 s in a debug build. The old
    // matcher needed minutes in debug for these three patterns.
    let budget = if cfg!(debug_assertions) {
        std::time::Duration::from_secs(15)
    } else {
        std::time::Duration::from_secs(1)
    };
    assert!(took < budget, "took {took:?}");
}

/// Review 3 H2: `parse_in` reads numeric operands in the document locale;
/// `parse` stays en-US.
#[test]
fn operands_in_document_locale() {
    use tally_numfmt::{DE_DE, EN_US};
    let d = settings(WILD);
    let de = |text: &str| Criterion::parse_in(&s(text), &d, &DE_DE);
    assert!(de(">1,5").is_numeric());
    assert!(de(">1,5").matches(&num(2.0)));
    assert!(!de(">1,5").matches(&num(1.0)));
    assert!(de("1.000").matches(&num(1000.0)));
    assert!(de("<19.10.2026").matches(&num(46_204.0)));
    assert!(!de("<19.10.2026").matches(&num(46_314.0)));
    assert!(de("<2026-10-19").matches(&num(46_204.0)));
    // `1.5` is 1 May under de-DE, a number nonetheless.
    assert!(de("1.5").is_numeric());
    assert!(de("1.5").matches(&num(46_143.0)));
    assert!(de("WAHR").is_numeric());
    assert!(de("WAHR").matches(&Value::Bool(true)));
    // Text under en-US.
    assert!(!Criterion::parse(&s(">1,5"), &d).is_numeric());
    assert!(!Criterion::parse_in(&s("19.10.2026"), &d, &EN_US).is_numeric());
    assert!(Criterion::parse(&s("10/19/2026"), &d).matches(&num(46_314.0)));
}
