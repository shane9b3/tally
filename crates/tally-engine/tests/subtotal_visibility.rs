//! SUBTOTAL and row visibility through the engine's `EvalContext`
//! (`hidden_row` / `filtered_row`): as in AOO, rows hidden by an AutoFilter
//! are skipped by every code, manually hidden rows only by 101 to 111.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{ColumnFilter, DocumentService, Op};
use tally_types::Value;

fn n(x: f64) -> Value {
    Value::Number(x)
}

/// A1 header, A2:A6 = 10, 20, 30, 40, 50; B2:B6 = keep/drop labels;
/// D1 = SUBTOTAL(9; A2:A6), D2 = SUBTOTAL(109; A2:A6), D3 = SUBTOTAL(2; A2:A6).
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.set("A1", "Amount");
    f.set("B1", "Tag");
    for (i, (v, tag)) in [
        (10, "keep"),
        (20, "drop"),
        (30, "keep"),
        (40, "drop"),
        (50, "keep"),
    ]
    .iter()
    .enumerate()
    {
        f.set(&format!("A{}", i + 2), &v.to_string());
        f.set(&format!("B{}", i + 2), tag);
    }
    f.set("D1", "=SUBTOTAL(9;A2:A6)");
    f.set("D2", "=SUBTOTAL(109;A2:A6)");
    f.set("D3", "=SUBTOTAL(2;A2:A6)");
    assert_eq!(f.value("D1"), n(150.0));
    assert_eq!(f.value("D2"), n(150.0));
    assert_eq!(f.value("D3"), n(5.0));
    f
}

/// The values after a hard recalc (in case the op did not dirty the
/// SUBTOTAL cells itself).
fn recalc_values(f: &mut Fixture) -> (Value, Value, Value) {
    f.svc.recalc_all(f.doc).unwrap();
    (f.value("D1"), f.value("D2"), f.value("D3"))
}

#[test]
fn manually_hidden_rows_are_skipped_by_101_to_111_only() {
    let mut f = fixture();
    let s = f.sheet;
    // Hide A3 (20) and A5 (40).
    for row in [2, 4] {
        let res = f.apply(vec![Op::SetRowsHidden {
            sheet: s,
            start: row,
            end: row,
            hidden: true,
        }]);
        assert!(res.errors.is_empty(), "{:?}", res.errors);
    }
    let (sum, sum_visible, count) = recalc_values(&mut f);
    assert_eq!(sum, n(150.0), "SUBTOTAL(9) includes manually hidden rows");
    assert_eq!(sum_visible, n(90.0), "SUBTOTAL(109) skips them");
    assert_eq!(count, n(5.0));
}

#[test]
fn filtered_rows_are_skipped_by_every_code() {
    let mut f = fixture();
    let s = f.sheet;
    let res = f.apply(vec![
        Op::SetAutoFilter {
            sheet: s,
            range: Some(range("A1:B6")),
        },
        Op::SetColumnFilter {
            sheet: s,
            col: 1,
            filter: Some(ColumnFilter {
                values: Some(vec!["keep".into()]),
                condition: None,
            }),
        },
    ]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let info = f.svc.filter_info(f.doc, s).unwrap();
    assert_eq!(info.range, Some(range("A1:B6")));
    let (sum, sum_visible, count) = recalc_values(&mut f);
    assert_eq!(sum, n(90.0), "SUBTOTAL(9) skips filtered rows (AOO)");
    assert_eq!(sum_visible, n(90.0));
    assert_eq!(count, n(3.0));

    // A manually hidden row on top of the filter: only 109 drops it.
    f.apply(vec![Op::SetRowsHidden {
        sheet: s,
        start: 1,
        end: 1,
        hidden: true,
    }]);
    let (sum, sum_visible, _) = recalc_values(&mut f);
    assert_eq!(sum, n(90.0));
    assert_eq!(sum_visible, n(80.0));

    // Removing the AutoFilter shows the rows again.
    f.apply(vec![Op::SetAutoFilter {
        sheet: s,
        range: None,
    }]);
    let (sum, sum_visible, count) = recalc_values(&mut f);
    assert_eq!(sum, n(150.0));
    assert_eq!(sum_visible, n(140.0));
    assert_eq!(count, n(5.0));
}

/// Whether hiding/filtering dirties SUBTOTAL without a hard recalc (AOO
/// recalculates SUBTOTAL when rows are hidden or filtered).
#[test]
fn filtering_updates_subtotal_without_hard_recalc() {
    let mut f = fixture();
    let s = f.sheet;
    f.apply(vec![
        Op::SetAutoFilter {
            sheet: s,
            range: Some(range("A1:B6")),
        },
        Op::SetColumnFilter {
            sheet: s,
            col: 1,
            filter: Some(ColumnFilter {
                values: Some(vec!["drop".into()]),
                condition: None,
            }),
        },
    ]);
    assert_eq!(f.value("D1"), n(60.0));
    assert_eq!(f.value("D2"), n(60.0));
}
