//! The solver through the service (wave 5, W5-3).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    DocumentService, SheetCell, SheetRange, SolverConstraint, SolverGoal, SolverOp, SolverOptions,
    SolverRequest, SolverStatus,
};
use tally_types::Value;

fn sr(f: &Fixture, a1: &str) -> SheetRange {
    SheetRange {
        sheet: f.sheet,
        range: range(a1),
    }
}

#[test]
fn solver_max_profit_via_service_and_document_unchanged() {
    let mut f = Fixture::new();
    // AOO help "maximum profit": three products with unit profits 5, 4, 6,
    // material ≤ 100 units, hours ≤ 180, product A ≤ 40.
    f.set("B1", "0");
    f.set("B2", "0");
    f.set("B3", "0");
    f.set("C1", "=5*B1+4*B2+6*B3");
    f.set("C2", "=B1+B2+B3");
    f.set("C3", "=2*B1+B2+3*B3");
    let req = SolverRequest {
        objective: SheetCell {
            sheet: f.sheet,
            r: 0,
            c: 2,
        },
        goal: SolverGoal::Maximize,
        variables: vec![sr(&f, "B1:B3")],
        constraints: vec![
            SolverConstraint {
                lhs: sr(&f, "C2"),
                op: SolverOp::Le,
                rhs: "100".into(),
            },
            SolverConstraint {
                lhs: sr(&f, "C3"),
                op: SolverOp::Le,
                rhs: "180".into(),
            },
            SolverConstraint {
                lhs: sr(&f, "B1"),
                op: SolverOp::Le,
                rhs: "40".into(),
            },
        ],
        options: SolverOptions {
            assume_non_negative: true,
            ..SolverOptions::default()
        },
    };
    let before = f.data();
    let r = f.svc.solver(f.doc, &req).unwrap();
    assert_eq!(r.status, SolverStatus::Optimal);
    assert!((r.objective - 480.0).abs() < 1e-6, "{r:?}");
    assert_eq!(f.data(), before, "the document is not changed");
    assert_eq!(f.value("B1"), Value::Number(0.0));
    // Integer variables: B1 ≤ 2.5 → 2 when integer.
    let mut int = req.clone();
    int.constraints[2].rhs = "2.5".into();
    int.constraints.push(SolverConstraint {
        lhs: sr(&f, "B1:B3"),
        op: SolverOp::Integer,
        rhs: String::new(),
    });
    let r = f.svc.solver(f.doc, &int).unwrap();
    assert_eq!(r.status, SolverStatus::Optimal);
    assert!(r.values.iter().all(|v| v.value.fract() == 0.0), "{r:?}");
    // Non-linear models are refused.
    f.set("C1", "=B1*B2");
    let e = f.svc.solver(f.doc, &req).unwrap_err();
    assert!(e.message.contains("not linear"), "{e:?}");
}
