//! Error classification, limits and hostile input.

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;
use tally_formula::{parse, MAX_NESTING};

#[test]
fn err_501_invalid_char() {
    for s in [
        "=@", "=1+`", "=A1#", "=\"abc", "=[.A1]", "=$", "='abc'", "=#FOO!", "=Err:999", "=1€",
    ] {
        assert_eq!(code(s), 501, "{s}");
    }
}

#[test]
fn err_504_param_list() {
    for s in [
        "=1,2", "=(1,2)", "=1;2", "={1,2}", "={1;2|3}", "={1+2}", "=1}", "=1|2",
    ] {
        assert_eq!(code(s), 504, "{s}");
    }
    let many = format!("=SUM({})", vec!["1"; 256].join(","));
    assert_eq!(code(&many), 504);
    let ok = format!("=SUM({})", vec!["1"; 255].join(","));
    p(&ok);
}

#[test]
fn err_508_bracket() {
    for s in [
        "=(1", "=SUM(1", "=SUM(1,", "=SUM(", "=((1)", "=1)", "=(1))", "={1;2", "=IF(A1;1",
    ] {
        assert_eq!(code(s), 508, "{s}");
    }
}

#[test]
fn err_509_missing_operator() {
    for s in [
        "=1 2",
        "=A1 B1",
        "=(1)(2)",
        "=1(2)",
        "=SUM(1 2)",
        "=\"a\"\"b\" 1",
        "=1E",
        "=1 SUM(1)",
    ] {
        assert_eq!(code(s), 509, "{s}");
    }
}

#[test]
fn err_510_missing_variable() {
    for s in [
        "=",
        "=1+",
        "=*2",
        "=()",
        "=1++",
        "=+",
        "=-",
        "=IF(1+,2)",
        "=%",
        "=1:2:",
        "={1;;2}",
        "=1*/2",
    ] {
        assert_eq!(code(s), 510, "{s}");
    }
}

#[test]
fn err_512_too_long() {
    let long = format!("={}", "1+".repeat(4096)) + "1";
    assert!(long.chars().count() > 8192);
    assert_eq!(code(&long), 512);
    // Exactly at the limit is fine (chars, not bytes).
    let s = format!("=\"{}\"", "é".repeat(8189));
    assert_eq!(s.chars().count(), 8192);
    p(&s);
}

#[test]
fn err_512_nesting() {
    let ok = format!("={}1{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
    p(&ok);
    let deep = format!(
        "={}1{}",
        "(".repeat(MAX_NESTING + 1),
        ")".repeat(MAX_NESTING + 1)
    );
    assert_eq!(code(&deep), 512);
    let calls = format!(
        "={}1{}",
        "SUM(".repeat(MAX_NESTING + 1),
        ")".repeat(MAX_NESTING + 1)
    );
    assert_eq!(code(&calls), 512);
    let ok_calls = format!(
        "={}1{}",
        "ABS(".repeat(MAX_NESTING),
        ")".repeat(MAX_NESTING)
    );
    p(&ok_calls);
}

#[test]
fn err_512_tree_depth() {
    // 2000 prefix minuses: no recursion, but the tree would be too deep.
    let s = format!("={}1", "-".repeat(2000));
    assert_eq!(code(&s), 512);
    // A long flat chain within the limits is fine.
    p(&format!("={}1", "1+".repeat(500)));
}

#[test]
fn error_spans() {
    let r = resolver();
    let e = parse("=1+@", a1("A1"), calc(), &r).unwrap_err();
    assert_eq!(e.span, 3..4);
    let e = parse("=SUM(1", a1("A1"), calc(), &r).unwrap_err();
    assert_eq!(e.span, 1..4);
    let e = parse("=1+", a1("A1"), calc(), &r).unwrap_err();
    assert_eq!(e.span, 3..3);
    assert_eq!(e.to_string(), "Err:510 at 3..3");
}

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn depth_bomb_on_256k_stack() {
    on_small_stack(|| {
        let r = resolver();
        let origin = a1("A1");
        let d = calc();
        // 10,000 deep: rejected (it is also over the length limit).
        let bomb = format!("={}", "(".repeat(10_000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        let bomb = format!("={}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        // Within the length limit, so the nesting guard is what fires.
        let bomb = format!("={}1", "(".repeat(8000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        let bomb = format!("={}1", "SUM(".repeat(2000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        let bomb = format!("={}1", "-".repeat(8000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        let bomb = format!("={}1", "-(".repeat(4000));
        assert_eq!(parse(&bomb, origin, d, &r).unwrap_err().kind.code(), 512);
        // The deepest accepted formulas parse, print and drop on this stack.
        let ok = format!("={}1{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
        let f = parse(&ok, origin, d, &r).unwrap();
        assert_eq!(tally_formula::print(&f, origin, d, &r), ok);
        let ok = format!("={}1", "-".repeat(1000));
        let f = parse(&ok, origin, d, &r).unwrap();
        assert_eq!(tally_formula::print(&f, origin, d, &r), ok);
        drop(f);
        let _ = tally_formula::tokenize(&"(".repeat(100_000), d);
    });
}
