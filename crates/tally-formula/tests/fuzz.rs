//! Property tests: the parser never panics, and accepted input round-trips.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use proptest::prelude::*;
use tally_formula::{parse, print, tokenize, BinOp, Dialect, Expr, NameRef};

fn check(src: &str) {
    let r = resolver();
    for d in [calc(), Dialect::open_formula()] {
        let toks = tokenize(src, d);
        // Spans tile the input exactly.
        let mut at = 0;
        for t in &toks {
            assert_eq!(t.span.start, at);
            assert!(t.span.end > t.span.start);
            at = t.span.end;
        }
        assert_eq!(at, src.len());
        if let Ok(f) = parse(src, a1("C3"), d, &r) {
            let printed = print(&f, a1("C3"), calc(), &r);
            let g = parse(&printed, a1("C3"), calc(), &r)
                .unwrap_or_else(|e| panic!("{src:?} -> {printed:?}: {e}"));
            // Printing is a fixed point for CalcA1 input. (OpenFormula input
            // can contain names such as `z9` that CalcA1 reads as cells.)
            if !d.is_open_formula() && !src.starts_with("of:=") && !src.starts_with("oooc:=") {
                // Same AST, modulo parentheses the printer adds for `(Name)!A1`.
                assert_eq!(
                    strip_parens(&g.expr),
                    strip_parens(&f.expr),
                    "{src:?} -> {printed:?}"
                );
                assert_eq!(print(&g, a1("C3"), calc(), &r), printed, "{src:?}");
            }
            let of = print(&f, a1("C3"), Dialect::open_formula(), &r);
            assert!(
                parse(&of, a1("C3"), calc(), &r).is_ok(),
                "{src:?} -> {of:?}"
            );
        }
    }
}

/// Inputs the property tests found whose printed form did not re-parse, or
/// re-parsed to a different tree. In CalcA1 `g:W.rq` used to lex as one
/// (invalid) column range with a sheet on its end only; it is the names `g`
/// and `W.rq`, like AOO, which splits a symbol at `:` when the whole is no
/// reference. Operands of `:` and `!` that would merge with the operator
/// into one reference token print in parentheses.
const REGRESSIONS: &[&str] = &[
    "g:W.rq",
    "g:NW.rq",
    "kg:NW.rq",
    "u:jdM.U",
    "a :a",
    "A :a",
    "1 :1",
    "1:1.",
    "A:a !1",
    "a:A !A",
    "a!A !a1!A",
    "A:A !A",
    "a:A1 :A1 ",
    "TRUE:A1 :A1 ",
    "A:A1: A1!_.",
    "a:A1: A1:TRUE.",
    "A1: B2:C3",
    "x:A1: B2:C3",
];

#[test]
fn round_trip_regressions() {
    for s in REGRESSIONS {
        check(s);
        check(&format!("={s}"));
        check(&format!("of:={s}"));
    }
}

#[test]
fn round_trip_regression_output() {
    let r = resolver();
    let of = Dialect::open_formula();
    let excel = Dialect::excel_a1();
    for (src, d, out, want) in [
        ("=g:W.rq", calc(), calc(), "=g:W.rq"),
        ("g:W.rq", of, calc(), "=g:W.rq"),
        ("=kg:NW.rq", calc(), calc(), "=kg:NW.rq"),
        ("=u:jdM.U", calc(), calc(), "=u:jdM.U"),
        ("=a :a", calc(), calc(), "=a:(a)"),
        ("=1 :1", calc(), calc(), "=1:(1)"),
        ("=A:A !A", calc(), calc(), "=(A:A)!A"),
        ("=a!A !a1", calc(), calc(), "=((a)!A)!A1"),
        ("=a:A1 :A1", calc(), calc(), "=a:A1:(A1)"),
        ("=A1: B2:C3", calc(), calc(), "=A1:(B2:C3)"),
        ("of:=a:[.B:.B]", of, calc(), "=a:(B:B)"),
        // Operands that cannot join keep no parentheses.
        ("=MyRange:A1", calc(), calc(), "=MyRange:A1"),
        ("=MyRange:Tax", calc(), calc(), "=MyRange:Tax"),
        ("=Tax:MyRange", calc(), calc(), "=Tax:MyRange"),
        ("of:=SUM(Start:End)", of, calc(), "=SUM(Start:End)"),
        ("of:=SUM(Start:End)", of, excel, "=SUM(Start:End)"),
        ("of:=SUM(g:W.rq)", of, excel, "=SUM(g:W.rq)"),
        ("of:=SUM(a:a)", of, excel, "=SUM(a:(a))"),
        ("=a:1", calc(), calc(), "=a:1"),
        ("=Tax:A1", calc(), calc(), "=Tax:A1"),
        ("=A:A:b", calc(), calc(), "=A:A:b"),
    ] {
        let f = parse(src, a1("C3"), d, &r).unwrap_or_else(|e| panic!("{src}: {e}"));
        let printed = print(&f, a1("C3"), out, &r);
        assert_eq!(printed, want, "{src}");
        let g = parse(&printed, a1("C3"), out, &r).unwrap_or_else(|e| panic!("{printed}: {e}"));
        assert_eq!(strip_parens(&g.expr), strip_parens(&f.expr), "{src}");
    }
}

/// `A:Sheet2.C` is no reference (a whole-column range cannot change
/// sheets), so CalcA1 reads it as the names `A` and `Sheet2.C`, which
/// evaluate to #NAME?, as AOO does; it used to be Err:501.
#[test]
fn column_range_across_sheets_reads_as_names() {
    let f = p("=SUM(A:Sheet2.C)");
    let Expr::Call { args, .. } = &f.expr else {
        panic!("{:?}", f.expr);
    };
    assert_eq!(
        args.as_slice(),
        [Expr::Binary(
            BinOp::Range,
            Box::new(Expr::Name(NameRef::Unresolved("A".into()))),
            Box::new(Expr::Name(NameRef::Unresolved("Sheet2.C".into()))),
        )]
    );
    assert_eq!(pr(&f, "A1"), "=SUM(A:Sheet2.C)");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn printable_ascii_never_panics(s in "[ -~]{0,64}") {
        check(&s);
        check(&format!("={s}"));
    }

    #[test]
    fn formula_alphabet_never_panics(s in "[=A-C1-3$:.!;,'\"()%^*/+&<>~|{}#\\[\\]-]{0,48}") {
        check(&s);
        check(&format!("={s}"));
    }

    /// Names, numbers and references joined by `:`, `!`, `~`, `.` and
    /// spaces: the shapes where printing can merge tokens.
    #[test]
    fn reference_like_round_trips(s in "(([a-zA-Z_][a-zA-Z0-9_.]{0,4}|[0-9]{1,3}|\\$?[A-Z]{1,2}\\$?[0-9]?|TRUE|'[a-z ]{0,3}')[ ]?[:!~ .][ ]?){1,6}") {
        check(&s);
        check(&format!("={s}"));
    }

    #[test]
    fn unicode_never_panics(s in "\\PC{0,24}") {
        check(&s);
    }
}
