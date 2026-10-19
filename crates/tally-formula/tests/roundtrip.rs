//! Round trips: CalcA1 → AST → CalcA1 identity, and CalcA1 ↔ OpenFormula ↔
//! CalcA1 preserving the AST.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use tally_formula::{parse, print, ArgSep, Dialect, SheetSep};

/// Canonical CalcA1 (en-US: `,` arguments, `.` sheets), parsed and printed
/// at C3.
const CANONICAL: &[&str] = &[
    "=1",
    "=0",
    "=12345",
    "=1.5",
    "=0.5",
    "=0.1",
    "=123.456",
    "=1E+20",
    "=1.5E-7",
    "=1E+15",
    "=\"\"",
    "=\"abc\"",
    "=\"a\"\"b\"",
    "=\"\"\"\"",
    "=\"héllo\"",
    "=TRUE",
    "=FALSE",
    "=#REF!",
    "=#N/A",
    "=#NAME?",
    "=#DIV/0!",
    "=#VALUE!",
    "=#NUM!",
    "=#NULL!",
    "=Err:502",
    "=Err:522",
    "=Err:504",
    "=10%",
    "=50%%",
    "=A1",
    "=C3",
    "=$A$1",
    "=$A1",
    "=A$1",
    "=XFD1048576",
    "=$XFD$1048576",
    "=A1:B2",
    "=$A$1:$B$2",
    "=B2:A1",
    "=A:A",
    "=$A:$C",
    "=B:A",
    "=1:1",
    "=$1:$3",
    "=10:2",
    "=Sheet1.A1",
    "=Sheet2.B2",
    "=$Sheet2.$B$2",
    "='My Sheet'.A1",
    "='It''s'.A1",
    "=$'My Sheet'.$A$1",
    "=Sheet2.A1:B2",
    "=Sheet2.A1:Sheet2.B2",
    "=Sheet1.A1:Sheet2.B2",
    "='My Sheet'.A1:C3",
    "=Sheet2.A:A",
    "=Sheet2.1:2",
    "=Nowhere.A1",
    "='Missing Sheet'.A1",
    "=A1:A10~B1:B10",
    "=A1:B2!B1:C3",
    "=A1!B1",
    "=(A1:B2)!(B1:C3)",
    "=A1:B2:C3",
    "=1+2",
    "=1-2",
    "=2*3",
    "=6/3",
    "=2^3",
    "=\"a\"&\"b\"",
    "=1=1",
    "=1<>2",
    "=1<2",
    "=1<=2",
    "=1>2",
    "=1>=2",
    "=1+2*3",
    "=(1+2)*3",
    "=2^3^2",
    "=2^(3^2)",
    "=-2^2",
    "=-(2^2)",
    "=2^-2",
    "=--1",
    "=+1",
    "=-+-1",
    "=1-+1",
    "=-A1",
    "=-A1:B2",
    "=-50%",
    "=2^50%",
    "=A1%",
    "=1+2&3",
    "=1=2=3",
    "=1-2-3",
    "=A1+B1*C1^D1",
    "=((1))",
    "=-(-(1))",
    "=1+-2*3",
    "=A1~B1~C1",
    "=SUM(1)",
    "=SUM()",
    "=SUM(A1:B2,3)",
    "=IF(A1,,1)",
    "=IF(,,)",
    "=IF(A1,)",
    "=ADDRESS(1,1,,0)",
    "=NOW()",
    "=FOO(1)",
    "=_XLFN.CONCAT(1,2)",
    "=ERROR.TYPE(#N/A)",
    "=LOG10(100)",
    "=IF(A1>0,\"pos\",IF(A1<0,\"neg\",\"zero\"))",
    "=VLOOKUP(A1,Sheet2.A1:B10,2,FALSE)",
    "=SUMIF(G2:G15,\"<>Paid\",F2:F15)",
    "=ROUND(A1*1.2,2)",
    "=AVERAGE(A:A)",
    "=SUM(1:1)",
    "=MAX(A1:A10)-MIN(A1:A10)",
    "=SUM(A1:A3)*2+B1",
    "=SUM((A1:A3~C1:C3))",
    "={1}",
    "={1;2;3}",
    "={1|2|3}",
    "={1;2|3;4}",
    "={-1;2;3}",
    "={\"a\";TRUE|#N/A;1.5}",
    "=SUM({1;2|3;4})*2",
    "=MyRange",
    "=Tax",
    "=MyRange*Tax",
    "=Unknown",
    "=Foo_Bar",
    "=(MyRange)!A1",
    "=Größe",
    "=Data.A1:Data.B5",
    "=IFERROR(1/0,\"x\")",
];

#[test]
fn calc_a1_identity() {
    assert!(CANONICAL.len() >= 100, "{}", CANONICAL.len());
    let r = resolver();
    for src in CANONICAL {
        let f = parse(src, a1("C3"), calc(), &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), calc(), &r), *src, "{src}");
    }
}

#[test]
fn identity_modulo_whitespace_and_leniency() {
    let cases = [
        ("= 1 + 2 ", "=1+2"),
        ("=  SUM( A1 , B1 )", "=SUM(A1,B1)"),
        ("=\tA1\n*\r\nB1", "=A1*B1"),
        ("= - 2 ^ 2", "=-2^2"),
        ("=IF( A1 , , 1 )", "=IF(A1,,1)"),
        ("=SUM(A1;B1)", "=SUM(A1,B1)"),
        ("=IF(A1;;1)", "=IF(A1,,1)"),
        ("=sum(a1:b2)", "=SUM(A1:B2)"),
        ("=Sheet2!A1", "=Sheet2.A1"),
        ("='My Sheet'!A1:B2", "='My Sheet'.A1:B2"),
        ("=sheet2.a1", "=Sheet2.A1"),
        ("=1.", "=1"),
        ("=.5", "=0.5"),
        ("=007", "=7"),
        ("=1e3", "=1000"),
        ("=true", "=TRUE"),
        ("=TRUE()", "=TRUE"),
        ("=false()", "=FALSE"),
        ("=#n/a", "=#N/A"),
        ("=err:519", "=#VALUE!"),
        ("=Err:524", "=#REF!"),
        ("=A1 : B2", "=A1:B2"),
        ("=SUM (1)", "=SUM(1)"),
        ("1+2", "=1+2"),
        ("={-1;+2;--3}", "={-1;2;3}"),
        ("=myrange", "=MyRange"),
        ("=MyRange!A1", "=MyRange.A1"),
    ];
    let r = resolver();
    for (src, want) in cases {
        let f = parse(src, a1("C3"), calc(), &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(print(&f, a1("C3"), calc(), &r), want, "{src}");
    }
}

#[test]
fn calc_open_formula_calc_preserves_semantics() {
    let r = resolver();
    for src in CANONICAL {
        let f = parse(src, a1("C3"), calc(), &r).unwrap();
        let of = print(&f, a1("C3"), Dialect::open_formula(), &r);
        assert!(of.starts_with("of:="), "{of}");
        let g = parse(&of, a1("C3"), calc(), &r).unwrap_or_else(|e| panic!("{src} -> {of}: {e}"));
        assert_eq!(f, g, "{src} -> {of}");
        let back = print(&g, a1("C3"), calc(), &r);
        assert_eq!(back, *src, "{src} -> {of} -> {back}");
    }
}

#[test]
fn other_calc_separators_preserve_semantics() {
    let r = resolver();
    for d in [
        Dialect::calc_a1(ArgSep::Semicolon, SheetSep::Dot),
        Dialect::calc_a1(ArgSep::Semicolon, SheetSep::Bang),
        Dialect::calc_a1(ArgSep::Comma, SheetSep::Bang),
    ] {
        for src in CANONICAL {
            let f = parse(src, a1("C3"), calc(), &r).unwrap();
            let text = print(&f, a1("C3"), d, &r);
            let g = parse(&text, a1("C3"), d, &r).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(f, g, "{src} -> {text}");
        }
    }
}

#[test]
fn open_formula_output_examples() {
    let cases = [
        ("=SUM(A1:B2,3)", "of:=SUM([.A1:.B2];3)"),
        ("=$Sheet2.A1", "of:=[$Sheet2.A1]"),
        ("=Sheet2.A1:B2", "of:=[Sheet2.A1:.B2]"),
        ("=$'My Sheet'.A1:B2", "of:=[$'My Sheet'.A1:.B2]"),
        ("=#REF!", "of:=[.#REF!]"),
        ("=A:A", "of:=[.A:.A]"),
        ("=TRUE", "of:=TRUE()"),
        ("={1;2|3;4}", "of:={1;2|3;4}"),
    ];
    for (src, want) in cases {
        assert_eq!(pr_of(&p(src), "A1"), want, "{src}");
    }
}

#[test]
fn print_with_separators() {
    let r = resolver();
    let f = p("=SUM(Sheet2.A1:B2,'My Sheet'.C1)");
    let d = Dialect::calc_a1(ArgSep::Semicolon, SheetSep::Bang);
    assert_eq!(
        print(&f, a1("A1"), d, &r),
        "=SUM(Sheet2!A1:B2;'My Sheet'!C1)"
    );
}

#[test]
fn sheet_name_quoting() {
    let mut r = resolver();
    r.add_sheet(10, "A1");
    r.add_sheet(11, "2020");
    r.add_sheet(12, "Q1-Sales");
    r.add_sheet(13, "Ünïcode");
    for (src, want) in [
        ("='A1'.B2", "='A1'.B2"),
        ("='2020'.A1", "='2020'.A1"),
        ("='Q1-Sales'.A1", "='Q1-Sales'.A1"),
        ("=Ünïcode.A1", "=Ünïcode.A1"),
    ] {
        let f = parse(src, a1("A1"), calc(), &r).unwrap();
        assert_eq!(print(&f, a1("A1"), calc(), &r), want);
    }
}

#[test]
fn deleted_or_unknown_sheet_id_prints_ref() {
    let mut r = resolver();
    let f = parse("=Sheet2.A1+1", a1("A1"), calc(), &r).unwrap();
    r.remove_sheet(1);
    assert_eq!(print(&f, a1("A1"), calc(), &r), "=#REF!+1");
    assert_eq!(
        print(&f, a1("A1"), Dialect::open_formula(), &r),
        "of:=[.#REF!]+1"
    );
}

#[test]
fn renamed_sheet_prints_new_name() {
    let mut r = resolver();
    let f = parse("=Sheet2.A1", a1("A1"), calc(), &r).unwrap();
    r.add_sheet(1, "Totals 2026");
    assert_eq!(print(&f, a1("A1"), calc(), &r), "='Totals 2026'.A1");
}

#[test]
fn programmatic_asts_get_parentheses() {
    use tally_formula::{BinOp, Expr, Formula, UnOp};
    let n = |x| Box::new(Expr::Num(x));
    let r = resolver();
    let cases = [
        (
            Expr::Binary(
                BinOp::Mul,
                Box::new(Expr::Binary(BinOp::Add, n(1.0), n(2.0))),
                n(3.0),
            ),
            "=(1+2)*3",
        ),
        (
            Expr::Binary(
                BinOp::Sub,
                n(1.0),
                Box::new(Expr::Binary(BinOp::Sub, n(2.0), n(3.0))),
            ),
            "=1-(2-3)",
        ),
        (
            Expr::Unary(
                UnOp::Neg,
                Box::new(Expr::Binary(BinOp::Pow, n(2.0), n(2.0))),
            ),
            "=-(2^2)",
        ),
        (
            Expr::Binary(
                BinOp::Pow,
                n(2.0),
                Box::new(Expr::Binary(BinOp::Pow, n(3.0), n(2.0))),
            ),
            "=2^(3^2)",
        ),
    ];
    for (e, want) in cases {
        let f = Formula::new(e);
        let s = print(&f, a1("A1"), calc(), &r);
        assert_eq!(s, want);
        // Reparsing gives the same value structure (modulo explicit parens).
        assert!(parse(&s, a1("A1"), calc(), &r).is_ok());
    }
}

#[test]
fn numbers_print_shortest() {
    for (src, want) in [
        ("=0.1+0.2", "=0.1+0.2"),
        ("=1234567890123", "=1234567890123"),
        ("=1E15", "=1E+15"),
        ("=123456789012345678", "=1.2345678901234568E+17"),
        ("=0.00001", "=0.00001"),
        ("=0.000001", "=1E-6"),
        ("=3.14159", "=3.14159"),
    ] {
        let s = pr(&p(src), "A1");
        assert_eq!(s, want, "{src}");
        // The printed form reparses to the same number.
        assert_eq!(p(&s), p(src));
    }
}
