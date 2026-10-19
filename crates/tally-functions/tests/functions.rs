//! The six WP3-core functions: SUM, ROUND, AVERAGE, COUNT, IF, CONCATENATE.

#![allow(clippy::unwrap_used)]

mod common;
use common::*;

use tally_types::TextToNumber;

/// A1=1, A2="x", A3=TRUE, A4=#DIV/0!, A5 empty, A6=2.5, B1="3", B2=FALSE.
fn sheet() -> MockEvalContext {
    let mut c = MockEvalContext::new();
    c.set(0, "A1", 1.0)
        .set(0, "A2", "x")
        .set(0, "A3", true)
        .set(0, "A4", Value::Error(ErrorKind::Div0))
        .set(0, "A6", 2.5)
        .set(0, "B1", "3")
        .set(0, "B2", false);
    c
}

// ---------------------------------------------------------------- SUM

#[test]
fn sum_cases() {
    let mut c = sheet();
    assert_eq!(call(&mut c, "SUM", vec![n(1.0), n(2.0), n(3.5)]), num(6.5));
    assert_eq!(
        call(&mut c, "SUM", vec![rg("A1:A3")]),
        num(2.0),
        "text skipped, bool counted"
    );
    assert_eq!(call(&mut c, "SUM", vec![rg("A1:A6")]), err(ErrorKind::Div0));
    assert_eq!(call(&mut c, "SUM", vec![rg("A5")]), num(0.0), "empty cell");
    assert_eq!(
        call(&mut c, "SUM", vec![rg("B1:B2")]),
        num(0.0),
        "text number in ref skipped"
    );
    assert_eq!(
        call(&mut c, "SUM", vec![t("3"), b(true)]),
        num(4.0),
        "direct args coerced"
    );
    assert_eq!(call(&mut c, "SUM", vec![t("x")]), err(ErrorKind::Value));
    assert_eq!(
        call(&mut c, "SUM", vec![n(1.0), missing(), n(2.0)]),
        num(3.0)
    );
    assert_eq!(
        call(&mut c, "SUM", vec![n(0.1), n(0.2), n(-0.3)]),
        num(0.0),
        "approxAdd"
    );
    assert_eq!(
        call(
            &mut c,
            "SUM",
            vec![arr(1, 3, vec![num(1.0), txt("9"), Value::Bool(true)])]
        ),
        num(2.0)
    );
    assert_eq!(
        call(&mut c, "SUM", vec![e(ErrorKind::NA), n(1.0)]),
        err(ErrorKind::NA)
    );
    c.settings.text_to_number = TextToNumber::GenerateError;
    assert_eq!(call(&mut c, "SUM", vec![t("3")]), err(ErrorKind::Value));
}

// ---------------------------------------------------------------- ROUND

#[test]
fn round_cases() {
    let mut c = sheet();
    let round = |c: &mut MockEvalContext, x: EvalValue, d: EvalValue| call(c, "ROUND", vec![x, d]);
    assert_eq!(round(&mut c, n(2.675), n(2.0)), num(2.68));
    assert_eq!(round(&mut c, n(1.005), n(2.0)), num(1.01));
    assert_eq!(round(&mut c, n(-2.5), n(0.0)), num(-3.0));
    assert_eq!(
        call(&mut c, "ROUND", vec![n(2.5)]),
        num(3.0),
        "count defaults to 0"
    );
    assert_eq!(
        round(&mut c, n(2.5), missing()),
        num(3.0),
        "explicitly empty count"
    );
    assert_eq!(round(&mut c, n(1234.567), n(-2.0)), num(1200.0));
    assert_eq!(
        round(&mut c, n(1234.567), n(1.9)),
        num(1234.6),
        "count floored"
    );
    assert_eq!(
        round(&mut c, n(15.0), n(-1.5)),
        num(0.0),
        "count -1.5 floors to -2"
    );
    assert_eq!(
        round(&mut c, n(15.0), n(-0.5)),
        num(20.0),
        "count -0.5 floors to -1"
    );
    assert_eq!(
        round(&mut c, n(1.0), n(21.0)),
        err(ErrorKind::InvalidArgument)
    );
    assert_eq!(round(&mut c, t("2.45"), n(1.0)), num(2.5), "text converts");
    assert_eq!(round(&mut c, t("x"), n(1.0)), err(ErrorKind::Value));
    assert_eq!(round(&mut c, b(true), n(0.0)), num(1.0));
    let a5 = c.range(0, "A5");
    assert_eq!(round(&mut c, a5, n(0.0)), num(0.0), "empty cell");
    let a4 = c.range(0, "A4");
    assert_eq!(round(&mut c, a4, n(0.0)), err(ErrorKind::Div0));
    assert_eq!(round(&mut c, n(1.0), e(ErrorKind::NA)), err(ErrorKind::NA));
}

// ---------------------------------------------------------------- AVERAGE

#[test]
fn average_cases() {
    let mut c = sheet();
    assert_eq!(
        call(&mut c, "AVERAGE", vec![n(1.0), n(2.0), n(6.0)]),
        num(3.0)
    );
    assert_eq!(
        call(&mut c, "AVERAGE", vec![rg("A1:A3")]),
        num(1.0),
        "(1+TRUE)/2"
    );
    assert_eq!(
        call(&mut c, "AVERAGE", vec![rg("A5")]),
        err(ErrorKind::Div0)
    );
    assert_eq!(
        call(&mut c, "AVERAGE", vec![rg("A2")]),
        err(ErrorKind::Div0),
        "text only"
    );
    assert_eq!(
        call(&mut c, "AVERAGE", vec![rg("A1:A6")]),
        err(ErrorKind::Div0),
        "#DIV/0! cell"
    );
    assert_eq!(call(&mut c, "AVERAGE", vec![rg("A5:A6"), n(0.5)]), num(1.5));
    assert_eq!(call(&mut c, "AVERAGE", vec![t("4"), n(2.0)]), num(3.0));
    assert_eq!(
        call(&mut c, "AVERAGE", vec![t("four")]),
        err(ErrorKind::Value)
    );
    assert_eq!(
        call(&mut c, "AVERAGE", vec![n(1.0), missing()]),
        num(0.5),
        "missing counts as 0"
    );
    assert_eq!(call(&mut c, "AVERAGE", vec![b(true), b(false)]), num(0.5));
    assert_eq!(
        call(&mut c, "AVERAGE", vec![e(ErrorKind::Ref)]),
        err(ErrorKind::Ref)
    );
}

// ---------------------------------------------------------------- COUNT

#[test]
fn count_cases() {
    let mut c = sheet();
    assert_eq!(
        call(&mut c, "COUNT", vec![rg("A1:A6")]),
        num(3.0),
        "1, TRUE, 2.5"
    );
    assert_eq!(
        call(&mut c, "COUNT", vec![rg("B1:B2")]),
        num(1.0),
        "FALSE counts, text not"
    );
    assert_eq!(call(&mut c, "COUNT", vec![rg("A5")]), num(0.0));
    assert_eq!(
        call(&mut c, "COUNT", vec![n(1.0), t("x"), t("2"), b(true)]),
        num(3.0)
    );
    assert_eq!(
        call(&mut c, "COUNT", vec![e(ErrorKind::NA), n(1.0)]),
        num(1.0),
        "errors ignored"
    );
    assert_eq!(call(&mut c, "COUNT", vec![t("2024-01-15")]), num(1.0));
    assert_eq!(call(&mut c, "COUNT", vec![n(1.0), missing()]), num(2.0));
    assert_eq!(
        call(
            &mut c,
            "COUNT",
            vec![arr(
                2,
                2,
                vec![num(1.0), txt("2"), err(ErrorKind::NA), Value::Empty]
            )]
        ),
        num(1.0)
    );
    assert_eq!(call(&mut c, "COUNT", vec![rg("A1:B6"), rg("A1")]), num(5.0));
}

// ---------------------------------------------------------------- IF

#[test]
fn if_cases() {
    let mut c = sheet();
    assert_eq!(call(&mut c, "IF", vec![b(true), n(1.0), n(2.0)]), num(1.0));
    assert_eq!(call(&mut c, "IF", vec![b(false), n(1.0), n(2.0)]), num(2.0));
    assert_eq!(
        call(&mut c, "IF", vec![b(true)]),
        Value::Bool(true),
        "then absent"
    );
    assert_eq!(
        call(&mut c, "IF", vec![b(false), n(1.0)]),
        Value::Bool(false),
        "else absent"
    );
    assert_eq!(
        call(&mut c, "IF", vec![b(true), missing(), n(2.0)]),
        num(0.0),
        "then empty"
    );
    assert_eq!(call(&mut c, "IF", vec![n(0.0), t("y"), t("n")]), txt("n"));
    assert_eq!(call(&mut c, "IF", vec![n(-3.0), t("y"), t("n")]), txt("y"));
    assert_eq!(
        call(&mut c, "IF", vec![t("TRUE"), n(1.0), n(2.0)]),
        num(1.0)
    );
    assert_eq!(
        call(&mut c, "IF", vec![t("abc"), n(1.0), n(2.0)]),
        err(ErrorKind::Value)
    );
    assert_eq!(
        call(&mut c, "IF", vec![e(ErrorKind::NA), n(1.0), n(2.0)]),
        err(ErrorKind::NA)
    );
    let a5 = c.range(0, "A5");
    assert_eq!(
        call(&mut c, "IF", vec![a5, n(1.0), n(2.0)]),
        num(2.0),
        "empty cell is FALSE"
    );
    // Condition from a column range uses implicit intersection with the origin row.
    c.set_origin(0, "D3");
    let col = c.range(0, "A1:A6");
    assert_eq!(
        call(&mut c, "IF", vec![col.clone(), t("y"), t("n")]),
        txt("y"),
        "A3 = TRUE"
    );
    c.set_origin(0, "D2");
    assert_eq!(
        call(&mut c, "IF", vec![col, t("y"), t("n")]),
        err(ErrorKind::Value),
        "A2 = \"x\""
    );
    // Errors in the chosen branch are returned, the other branch's are not.
    assert_eq!(
        call(&mut c, "IF", vec![b(true), e(ErrorKind::Ref), n(2.0)]),
        err(ErrorKind::Ref)
    );
}

// ---------------------------------------------------------------- CONCATENATE

#[test]
fn concatenate_cases() {
    let mut c = sheet();
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![t("a"), t("b"), t("c")]),
        txt("abc")
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![t("n="), n(1.5)]),
        txt("n=1.5")
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![n(0.1 + 0.2)]),
        txt("0.3"),
        "15 digits"
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![b(true), b(false)]),
        txt("TRUEFALSE")
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![t("a"), missing(), t("b")]),
        txt("ab")
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![rg("A5"), t("!")]),
        txt("!"),
        "empty cell"
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![rg("A1"), rg("A3")]),
        txt("1TRUE")
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![rg("A4")]),
        err(ErrorKind::Div0)
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![t("x"), e(ErrorKind::NA)]),
        err(ErrorKind::NA)
    );
    assert_eq!(call(&mut c, "CONCATENATE", vec![n(1e20)]), txt("1E+20"));
    c.set_origin(0, "C6");
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![rg("A1:A6")]),
        txt("2.5"),
        "intersects A6"
    );
    c.set_origin(0, "C9");
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![rg("A1:A6")]),
        err(ErrorKind::Value)
    );
    assert_eq!(
        call(
            &mut c,
            "CONCATENATE",
            vec![arr(1, 2, vec![txt("p"), txt("q")])]
        ),
        txt("p"),
        "inline array: top-left"
    );
}
