//! Timing check: `cargo test -p tally-formula --release --test perf -- --ignored`.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use std::time::Instant;
use tally_formula::parse;

#[test]
#[ignore = "timing test; run in release"]
fn parse_100k_formulas_under_a_second() {
    let r = resolver();
    let d = calc();
    let start = Instant::now();
    let mut n = 0usize;
    for i in 0..100_000u32 {
        let origin = tally_types::CellAddr::new(0, i % 1000, 5);
        let f = parse("=SUM(A1:A10)*2+B1", origin, d, &r).unwrap();
        n += usize::from(matches!(f.expr, tally_formula::Expr::Binary(..)));
    }
    let elapsed = start.elapsed();
    assert_eq!(n, 100_000);
    eprintln!("parsed 100k formulas in {elapsed:?}");
    assert!(elapsed.as_secs_f64() < 1.0, "{elapsed:?}");
}
