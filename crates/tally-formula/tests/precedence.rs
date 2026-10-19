//! Operator precedence and associativity (AOO rules).

#![allow(clippy::unwrap_used)] // Test code may unwrap.

mod common;
use common::*;

fn s(src: &str) -> String {
    sx(&p(src).expr)
}

#[test]
fn unary_minus_binds_tighter_than_pow() {
    assert_eq!(s("=-2^2"), "(^ (neg 2.0) 2.0)");
    assert_eq!(s("=-A1^2"), "(^ (neg (cell R[0]C[0])) 2.0)");
    assert_eq!(s("=2^-2"), "(^ 2.0 (neg 2.0))");
}

#[test]
fn pow_is_left_associative() {
    assert_eq!(s("=2^3^2"), "(^ (^ 2.0 3.0) 2.0)");
}

#[test]
fn arithmetic() {
    assert_eq!(s("=1+2*3"), "(+ 1.0 (* 2.0 3.0))");
    assert_eq!(s("=1*2+3"), "(+ (* 1.0 2.0) 3.0)");
    assert_eq!(s("=1-2-3"), "(- (- 1.0 2.0) 3.0)");
    assert_eq!(s("=2*3^2"), "(* 2.0 (^ 3.0 2.0))");
    assert_eq!(s("=1+2&3+4"), "(& (+ 1.0 2.0) (+ 3.0 4.0))");
    assert_eq!(s("=1&2=3&4"), "(= (& 1.0 2.0) (& 3.0 4.0))");
    assert_eq!(s("=1<2=TRUE"), "(= (< 1.0 2.0) TRUE)");
}

#[test]
fn percent() {
    assert_eq!(s("=10%"), "(% 10.0)");
    assert_eq!(s("=-10%"), "(neg (% 10.0))");
    assert_eq!(s("=2^10%"), "(^ 2.0 (% 10.0))");
    assert_eq!(s("=A1:A2%"), "(% (area R[0]C[0] R[1]C[0]))");
}

#[test]
fn reference_operators_bind_tightest() {
    assert_eq!(
        s("=A1:A10~B1:B10"),
        "(union (area R[0]C[0] R[9]C[0]) (area R[0]C[1] R[9]C[1]))"
    );
    assert_eq!(
        s("=A1~B1!C1"),
        "(union (cell R[0]C[0]) (isect (cell R[0]C[1]) (cell R[0]C[2])))"
    );
    assert_eq!(s("=-A1:B2"), "(neg (area R[0]C[0] R[1]C[1]))");
    assert_eq!(s("=A1:B2*2"), "(* (area R[0]C[0] R[1]C[1]) 2.0)");
    assert_eq!(
        s("=(A1):B2"),
        "(range (paren (cell R[0]C[0])) (cell R[1]C[1]))"
    );
}

#[test]
fn second_endpoint_sheet_inherits() {
    use tally_formula::{RefExpr, ResolvedRef, SheetRef};
    use tally_types::CellRange;
    let f = p("=Sheet2.A1:B2");
    let tally_formula::Expr::Ref(RefExpr::Area(a, b)) = &f.expr else {
        panic!()
    };
    assert_eq!(a.sheet, SheetRef::Id { id: 1, abs: false });
    assert_eq!(b.sheet, SheetRef::Current);
    assert_eq!(
        f.static_precedents(a1("A1")),
        vec![ResolvedRef::Range(CellRange::new(1, 0, 0, 1, 1))]
    );
}
