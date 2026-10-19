//! Property tests: the parser never panics, and accepted input round-trips.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use proptest::prelude::*;
use tally_formula::{parse, print, tokenize, Dialect};

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

    #[test]
    fn unicode_never_panics(s in "\\PC{0,24}") {
        check(&s);
    }
}
