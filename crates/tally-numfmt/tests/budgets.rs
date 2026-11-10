//! Time budgets for hostile format codes (security review 6, H2 / H3).
//!
//! The debug-build tests use generous limits; the `_release` variants hold
//! the render budget of `docs/security.md` §8 and run with
//! `cargo test --release -p tally-numfmt --test budgets -- --ignored`.

use std::time::{Duration, Instant};

use tally_numfmt::{format_value_with, parse_format, print_format, EN_US};
use tally_types::{DateEpoch, Value};

/// `[$-40D]` + `tag` repeated up to 4,096 characters + `D`.
fn repeated_tag_code(tag: &str) -> String {
    let mut code = String::from("[$-40D]");
    while code.len() + tag.len() < 4096 {
        code.push_str(tag);
    }
    code.push('D');
    code
}

/// Formats `cells` serials (one per day from 2000-01-01) with `code`.
fn render(code: &str, cells: u32) -> Duration {
    let ast = parse_format(code).expect("valid code");
    let t = Instant::now();
    for i in 0..cells {
        let v = Value::Number(36_526.0 + f64::from(i % 20_000));
        let f = format_value_with(&v, &ast, &EN_US, None, DateEpoch::D1899);
        assert!(!f.text.is_empty());
    }
    t.elapsed()
}

fn check_calendar_render(cells: u32, limit: Duration) {
    for code in [
        "[$-40D][~jewish]D".to_string(),
        "[$-40D]EE".to_string(),
        "G E".to_string(),
        repeated_tag_code("[~jewish]"),
        repeated_tag_code("[~jewish][~gregorian]"),
    ] {
        let took = render(&code, cells);
        assert!(
            took < limit,
            "{} chars: {cells} cells took {took:?} (limit {limit:?})",
            code.len()
        );
    }
    // 408 Hebrew-numeral fields per cell: linear in the output (about
    // 35 µs per cell in release), so it gets 10× the budget.
    let code = repeated_tag_code("[~jewish]D");
    let took = render(&code, cells);
    assert!(
        took < limit * 10,
        "{cells} cells of 408 fields took {took:?}"
    );
}

/// The Jewish calendar cost ~50 µs per cell (a year search starting ~7,500
/// years low) and ~22 ms with 450 `[~jewish]` tags (no memo).
#[test]
fn jewish_calendar_renders_fast() {
    check_calendar_render(2_000, Duration::from_secs(3));
}

/// Release-only timing: a screen of 20,000 cells in < 100 ms.
#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn jewish_calendar_renders_100ms_release() {
    check_calendar_render(20_000, Duration::from_millis(100));
}

/// `RR` next to a digit is rewritten to literals in one pass, and keyword
/// runs are scanned at most 255 characters ahead (both were O(n²): 262K
/// characters took 5.6 s / 11.5 s in release).
fn check_parse(chars: usize, limit: Duration) {
    let rr = format!("{}0", "RR ".repeat(chars / 3));
    let r_run = format!("{} 0", "R".repeat(chars));
    let e_run = format!("{} 0", "E".repeat(chars));
    for code in [rr, r_run, e_run] {
        let t = Instant::now();
        let ast = parse_format(&code);
        let took = t.elapsed();
        assert!(
            took < limit,
            "{} chars: {took:?} (limit {limit:?})",
            code.len()
        );
        drop(ast);
    }
}

#[test]
fn long_r_codes_parse_linearly() {
    check_parse(65_536, Duration::from_secs(3));
}

#[test]
#[ignore = "release-only timing; run with --release -- --ignored"]
fn long_r_codes_parse_linearly_release() {
    check_parse(262_144, Duration::from_millis(200));
}

#[test]
fn lowercase_r_literal_keeps_its_case() {
    let ast = parse_format("r0").unwrap();
    assert_eq!(print_format(&ast), "r0");
    let f = format_value_with(&Value::Number(1.0), &ast, &EN_US, None, DateEpoch::D1899);
    assert_eq!(f.text, "r1");
    let ast = parse_format("rR #").unwrap();
    assert_eq!(print_format(&ast), "rR #");
}
