//! Recalculation: chains, diamonds, cycles, volatile cells.
#![allow(clippy::unwrap_used)]

mod common;

use common::Fixture;
use tally_engine::recalc::recalc;
use tally_engine::Workbook;
use tally_types::{CellAddr, ErrorKind, Value};

fn a(s: &str) -> CellAddr {
    CellAddr::from_a1(1, s).unwrap()
}

#[test]
fn chain_of_100k_on_a_256k_stack() {
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut wb = Workbook::new();
            wb.set_input(a("A1"), "1");
            for r in 2..=100_000u32 {
                wb.set_input(CellAddr::new(1, r - 1, 0), &format!("=A{}+1", r - 1));
            }
            recalc(&mut wb);
            let first = wb.value(CellAddr::new(1, 99_999, 0));
            wb.set_input(a("A1"), "10");
            let rep = recalc(&mut wb);
            (first, wb.value(CellAddr::new(1, 99_999, 0)), rep.evaluated)
        })
        .unwrap();
    let (first, second, evaluated) = h.join().unwrap();
    assert_eq!(first, Value::Number(100_000.0));
    assert_eq!(second, Value::Number(100_009.0));
    assert_eq!(evaluated, 99_999);
}

#[test]
fn diamond_evaluates_each_once() {
    let mut f = Fixture::new();
    f.set("A1", "2");
    f.set("B1", "=A1*10");
    f.set("B2", "=A1+1");
    f.set("C1", "=B1+B2");
    assert_eq!(f.value("C1"), Value::Number(23.0));
    let res = f.set("A1", "3");
    assert_eq!(f.value("C1"), Value::Number(34.0));
    // A1, B1:B2 and C1 are reported as changed.
    let ranges = &res.changed[0].ranges;
    let covers = |r: u32, c: u32| {
        ranges
            .iter()
            .any(|x| x.r0 <= r && r <= x.r1 && x.c0 <= c && c <= x.c1)
    };
    assert!(covers(0, 0) && covers(0, 1) && covers(1, 1) && covers(0, 2));
}

#[test]
fn cycle_of_two_and_recovery() {
    let mut f = Fixture::new();
    f.set("A1", "=B1+1");
    f.set("B1", "=A1+1");
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Circular));
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::Circular));
    assert_eq!(f.text("A1"), "Err:522");
    f.set("B1", "1");
    assert_eq!(f.value("A1"), Value::Number(2.0));
}

#[test]
fn cycle_of_fifty_and_recovery() {
    let mut f = Fixture::new();
    for r in 1..50 {
        f.set(&format!("A{r}"), &format!("=A{}", r + 1));
    }
    f.set("A50", "=A1");
    f.set("B1", "=A25*2");
    for r in 1..=50 {
        assert_eq!(
            f.value(&format!("A{r}")),
            Value::Error(ErrorKind::Circular),
            "A{r}"
        );
    }
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::Circular));
    f.set("A50", "7");
    for r in 1..=50 {
        assert_eq!(f.value(&format!("A{r}")), Value::Number(7.0), "A{r}");
    }
    assert_eq!(f.value("B1"), Value::Number(14.0));
}

#[test]
fn volatile_cells_recalculate_on_every_apply() {
    let mut f = Fixture::new();
    f.set("A1", "=RAND()");
    let v1 = f.value("A1");
    f.set("C9", "1");
    let v2 = f.value("A1");
    assert!(matches!(v1, Value::Number(_)));
    assert_ne!(v1, v2, "RAND() changes on an unrelated edit");
}

#[test]
fn whole_column_sum_tracks_edits() {
    let mut f = Fixture::new();
    f.set("B1", "=SUM(A:A)");
    f.set("A1000000", "5");
    assert_eq!(f.value("B1"), Value::Number(5.0));
    f.set("A3", "=2*3");
    assert_eq!(f.value("B1"), Value::Number(11.0));
}

#[test]
fn sumproduct_array_context() {
    let mut f = Fixture::new();
    for (c, v) in [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "4"),
        ("B2", "8"),
        ("B3", "16"),
    ] {
        f.set(c, v);
    }
    f.set("C1", "=SUMPRODUCT((A1:A3>1)*B1:B3)");
    assert_eq!(f.value("C1"), Value::Number(24.0));
}

#[test]
fn implicit_intersection_of_cell_result() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("A3", "3");
    f.set("C2", "=A1:A3");
    assert_eq!(f.value("C2"), Value::Number(2.0));
    f.set("C9", "=A1:A3");
    assert_eq!(f.value("C9"), Value::Error(ErrorKind::Value));
}

#[test]
fn invalid_formula_keeps_text_and_shows_error() {
    let mut f = Fixture::new();
    f.set("A1", "=SUM(1;2");
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Bracket));
    let info = tally_api::DocumentService::cell_edit_info(&f.svc, f.doc, f.sheet, 0, 0).unwrap();
    assert_eq!(info.input, "=SUM(1;2");
    f.set("A2", "=NOSUCHFN(1)");
    assert_eq!(f.value("A2"), Value::Error(ErrorKind::Name));
}

#[test]
fn cross_sheet_references_and_sheet_deletion() {
    let mut f = Fixture::new();
    let res = f.apply(vec![tally_api::Op::AddSheet {
        name: Some("Data".into()),
        index: None,
    }]);
    let info = res.info.unwrap();
    let data = info.sheets[1].id;
    f.apply(vec![common::input(data, "A1", "21")]);
    f.set("A1", "=Data.A1*2");
    f.set("A2", "=Data!A1+1");
    assert_eq!(f.value("A1"), Value::Number(42.0));
    assert_eq!(f.value("A2"), Value::Number(22.0));
    f.apply(vec![common::input(data, "A1", "1")]);
    assert_eq!(f.value("A1"), Value::Number(2.0));
    f.apply(vec![tally_api::Op::DeleteSheet { sheet: data }]);
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Ref));
    let edit = tally_api::DocumentService::cell_edit_info(&f.svc, f.doc, f.sheet, 0, 0).unwrap();
    assert_eq!(edit.input, "=#REF!*2");
    tally_api::DocumentService::undo(&mut f.svc, f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Number(2.0));
}

fn nested_if(depth: usize) -> String {
    let mut s = String::from("=");
    for _ in 0..depth {
        s.push_str("IF(TRUE(),");
    }
    s.push('1');
    for _ in 0..depth {
        s.push_str(",0)");
    }
    s
}

fn nested_iferror(depth: usize) -> String {
    let mut s = String::from("=");
    for _ in 0..depth {
        s.push_str("IFNA(");
    }
    s.push('1');
    for _ in 0..depth {
        s.push_str(",0)");
    }
    s
}

#[test]
fn nested_lazy_calls_are_bounded_on_a_256k_stack() {
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut wb = Workbook::new();
            wb.set_input(a("A1"), &nested_if(250));
            wb.set_input(a("A2"), &nested_iferror(100));
            wb.set_input(a("A3"), &nested_iferror(30));
            recalc(&mut wb);
            (wb.value(a("A1")), wb.value(a("A2")), wb.value(a("A3")))
        })
        .unwrap();
    let (ifs, deep, shallow) = h.join().unwrap();
    assert_eq!(ifs, Value::Number(1.0), "IF is evaluated on the work stack");
    assert_eq!(
        deep,
        Value::Error(ErrorKind::FormulaOverflow),
        "deeper than MAX_EVAL_DEPTH"
    );
    assert_eq!(shallow, Value::Number(1.0));
}

#[test]
fn three_d_references_track_tab_order() {
    let mut f = Fixture::new();
    let r = f.apply(vec![
        tally_api::Op::AddSheet {
            name: Some("S2".into()),
            index: None,
        },
        tally_api::Op::AddSheet {
            name: Some("S3".into()),
            index: None,
        },
    ]);
    let info = r.info.unwrap();
    let (s2, s3) = (info.sheets[1].id, info.sheets[2].id);
    f.set("B1", "=SUM(Sheet1.A1:S3.A1)");
    // Edits inside the span dirty the 3-D reference.
    assert_eq!(f.value("B1"), Value::Number(0.0));
    let res = f.apply(vec![common::input(s2, "A1", "5")]);
    assert!(
        res.changed.iter().any(|c| c.sheet == f.sheet),
        "B1 recalculated"
    );
    assert_eq!(f.value("B1"), Value::Number(5.0));
    f.apply(vec![tally_api::Op::MoveSheet { sheet: s3, to: 1 }]);
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let (_, fc) = wb.formula_at(CellAddr::new(f.sheet, 0, 1)).unwrap();
    assert_eq!(
        wb.precedents_of(&fc.formula, fc.addr).len(),
        2,
        "S2 is no longer inside Sheet1..S3"
    );
    assert_eq!(fc.spans().len(), 1, "one unexpanded span");
    let res = f.apply(vec![common::input(s2, "A1", "6")]);
    assert!(
        !res.changed.iter().any(|c| c.sheet == f.sheet),
        "B1 no longer reads S2"
    );
}

// ---- Review 7: the deferred-evaluation work stack ----

/// `A1..A15 =INDIRECT("A<n+1>")` nest 16 deep on demand; `A16` then reads
/// `k` dirty formulas through INDIRECT, all deferred in one attempt
/// (security review 7, H1; correctness review 7, finding 1). Before the
/// fix the deferring formula was retried once per sibling, each retry
/// re-walking the chain: k = 8,000 took 24 s in release, growing as k³.
#[test]
fn deferred_siblings_recalculate_in_linear_time() {
    let k = 30_000u32;
    let mut wb = Workbook::new();
    wb.threading = tally_engine::parallel::Threading::Sequential;
    for i in 1..16 {
        wb.set_input(a(&format!("A{i}")), &format!("=INDIRECT(\"A{}\")", i + 1));
    }
    wb.set_input(a("A16"), &format!("=SUM(INDIRECT(\"B1:B{k}\"))"));
    for r in 1..=k {
        wb.set_input(a(&format!("B{r}")), "=1");
    }
    let t = std::time::Instant::now();
    let rep = recalc(&mut wb);
    // Generous for an unoptimized build under load; the old code needed
    // hours here.
    assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
    assert_eq!(wb.value(a("A1")), Value::Number(f64::from(k)));
    assert_eq!(rep.circular, 0);
}

/// Each row has a 24-long chain of dynamic references; the first formula
/// sums the rows through INDIRECT (correctness review 7, finding 1).
#[test]
fn many_deferred_dynamic_chains_stay_linear() {
    let n = 600u32;
    let mut wb = Workbook::new();
    wb.threading = tally_engine::parallel::Threading::Sequential;
    let s = wb.sheets[0].id;
    wb.set_input(a("A1"), &format!("=SUM(INDIRECT(\"B1:B{n}\"))"));
    for r in 1..=n {
        for c in 1..=24u32 {
            // Column c (B = 1) reads column c + 1 of the same row.
            wb.set_input(
                CellAddr::new(s, r - 1, c),
                &format!("=INDIRECT(ADDRESS({r};{}))", c + 2),
            );
        }
        wb.set_input(CellAddr::new(s, r - 1, 25), "=1");
    }
    let t = std::time::Instant::now();
    let rep = recalc(&mut wb);
    assert!(t.elapsed().as_secs() < 60, "took {:?}", t.elapsed());
    assert_eq!(wb.value(a("A1")), Value::Number(f64::from(n)));
    assert_eq!(rep.circular, 0);
}

/// A cell deferred at depth 16 and then evaluated on demand at a shallower
/// depth in the same attempt is finished, not a cycle: the attempt made
/// progress and is repeated (correctness review 7, finding 3: A1 was
/// Err:522 for k = 11..16).
#[test]
fn a_deferred_cell_finished_later_in_the_attempt_is_not_circular() {
    for k in 2..=25u32 {
        let mut wb = Workbook::new();
        wb.threading = tally_engine::parallel::Threading::Sequential;
        for i in 1..=24u32 {
            wb.set_input(a(&format!("K{i}")), &format!("=INDIRECT(\"K{}\")", i + 1));
        }
        wb.set_input(a("K25"), "=7");
        wb.set_input(a("A1"), &format!("=INDIRECT(\"K1\")+INDIRECT(\"K{k}\")"));
        let rep = recalc(&mut wb);
        assert_eq!(wb.value(a("A1")), Value::Number(14.0), "k = {k}");
        assert_eq!(rep.circular, 0, "k = {k}");
    }
}

/// A real cycle through deferred cells is still Err:522, and formulas
/// outside it are not.
#[test]
fn a_cycle_through_deferred_cells_is_circular() {
    let mut wb = Workbook::new();
    wb.threading = tally_engine::parallel::Threading::Sequential;
    for i in 1..=40u32 {
        wb.set_input(a(&format!("K{i}")), &format!("=INDIRECT(\"K{}\")", i + 1));
    }
    wb.set_input(a("K41"), "=INDIRECT(\"K1\")+1");
    wb.set_input(a("B1"), "=5");
    for r in 2..=20u32 {
        wb.set_input(a(&format!("B{r}")), &format!("=B{}+1", r - 1));
    }
    recalc(&mut wb);
    for i in [1, 17, 33, 41] {
        assert_eq!(
            wb.value(a(&format!("K{i}"))),
            Value::Error(ErrorKind::Circular),
            "K{i}"
        );
    }
    assert_eq!(wb.value(a("B20")), Value::Number(24.0));
}
