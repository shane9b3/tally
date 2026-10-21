//! Outline groups (wave 5, W5-2): group/ungroup nesting, collapse as a
//! third hidden source, level buttons, AutoOutline, clear, structure
//! changes and undo.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{ApiError, DocumentService, Op, OutlineAxis};
use tally_types::Value;

fn group(f: &mut Fixture, axis: OutlineAxis, start: u32, end: u32) -> tally_api::ApplyResult {
    f.apply(vec![Op::Group {
        sheet: f.sheet,
        axis,
        start,
        end,
    }])
}

fn hidden_rows(f: &Fixture) -> Vec<(u32, u32, bool)> {
    f.svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .row_runs
        .iter()
        .filter(|r| r.hidden)
        .map(|r| (r.start, r.end, r.filtered.unwrap_or(false)))
        .collect()
}

#[test]
fn outline_nesting_and_levels() {
    let mut f = Fixture::new();
    let r = group(&mut f, OutlineAxis::Rows, 1, 10);
    assert!(r.errors.is_empty());
    group(&mut f, OutlineAxis::Rows, 2, 4);
    group(&mut f, OutlineAxis::Rows, 0, 20);
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(o.row_levels, 3);
    assert_eq!(
        o.rows
            .iter()
            .map(|g| (g.start, g.end, g.level))
            .collect::<Vec<_>>(),
        vec![(0, 20, 1), (1, 10, 2), (2, 4, 3)]
    );
    // Partial overlap is refused.
    let r = group(&mut f, OutlineAxis::Rows, 3, 12);
    assert_eq!(r.errors[0].code, ApiError::INVALID_ARGUMENT);
    // Ungroup the innermost covering group.
    f.apply(vec![Op::Ungroup {
        sheet: f.sheet,
        axis: OutlineAxis::Rows,
        start: 2,
        end: 4,
    }]);
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(o.rows.len(), 2);
    assert_eq!(o.row_levels, 2);
    // Eight levels at most.
    let mut g = Fixture::new();
    for i in 0..8u32 {
        assert!(group(&mut g, OutlineAxis::Cols, i, 30 - i)
            .errors
            .is_empty());
    }
    let r = group(&mut g, OutlineAxis::Cols, 10, 12);
    assert_eq!(r.errors[0].code, ApiError::INVALID_ARGUMENT);
}

#[test]
fn collapse_hides_rows_as_a_third_source() {
    let mut f = Fixture::new();
    f.set("A3", "x");
    group(&mut f, OutlineAxis::Rows, 1, 4);
    f.apply(vec![Op::SetRowsHidden {
        sheet: f.sheet,
        start: 3,
        end: 3,
        hidden: true,
    }]);
    let r = f.apply(vec![Op::SetOutlineCollapsed {
        sheet: f.sheet,
        axis: OutlineAxis::Rows,
        start: 1,
        end: 4,
        collapsed: true,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(hidden_rows(&f), vec![(1, 4, false)]);
    // SUBTOTAL 109 skips rows hidden by the outline.
    f.set("A1", "=SUBTOTAL(109,A2:A5)");
    f.set("A3", "5");
    assert_eq!(f.value("A1"), Value::Number(0.0));
    f.apply(vec![Op::SetOutlineCollapsed {
        sheet: f.sheet,
        axis: OutlineAxis::Rows,
        start: 1,
        end: 4,
        collapsed: false,
    }]);
    // Expanding shows what collapsing hid; the user-hidden row stays.
    assert_eq!(hidden_rows(&f), vec![(3, 3, false)]);
    assert_eq!(f.value("A1"), Value::Number(5.0));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), vec![(1, 4, false)]);
    f.svc.redo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), vec![(3, 3, false)]);
}

#[test]
fn show_level_and_clear() {
    let mut f = Fixture::new();
    group(&mut f, OutlineAxis::Rows, 1, 10);
    group(&mut f, OutlineAxis::Rows, 2, 4);
    f.apply(vec![Op::ShowOutlineLevel {
        sheet: f.sheet,
        axis: OutlineAxis::Rows,
        level: 1,
    }]);
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(
        o.rows.iter().map(|g| g.collapsed).collect::<Vec<_>>(),
        vec![false, true]
    );
    assert_eq!(hidden_rows(&f), vec![(2, 4, false)]);
    f.apply(vec![Op::ShowOutlineLevel {
        sheet: f.sheet,
        axis: OutlineAxis::Rows,
        level: 0,
    }]);
    assert_eq!(hidden_rows(&f), vec![(1, 10, false)]);
    f.apply(vec![Op::ClearOutline {
        sheet: f.sheet,
        axis: None,
    }]);
    assert!(f
        .svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .outline
        .is_none());
    assert!(hidden_rows(&f).is_empty());
}

#[test]
fn auto_outline_from_formulas() {
    let mut f = Fixture::new();
    for r in 1..=3 {
        f.set(&format!("B{r}"), "1");
        f.set(&format!("C{r}"), "2");
    }
    f.set("B4", "=SUM(B1:B3)");
    f.set("D1", "=SUM(B1:C1)");
    let r = f.apply(vec![Op::AutoOutline {
        sheet: f.sheet,
        range: None,
    }]);
    assert!(r.errors.is_empty());
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(
        o.rows.iter().map(|g| (g.start, g.end)).collect::<Vec<_>>(),
        vec![(0, 2)]
    );
    assert_eq!(
        o.cols.iter().map(|g| (g.start, g.end)).collect::<Vec<_>>(),
        vec![(1, 2)]
    );
}

#[test]
fn groups_follow_structure_changes() {
    let mut f = Fixture::new();
    group(&mut f, OutlineAxis::Rows, 5, 8);
    f.apply(vec![Op::InsertRows {
        sheet: f.sheet,
        at: 0,
        count: 2,
    }]);
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!((o.rows[0].start, o.rows[0].end), (7, 10));
    f.apply(vec![Op::DeleteRows {
        sheet: f.sheet,
        at: 7,
        count: 4,
    }]);
    assert!(f
        .svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .outline
        .is_none());
    f.svc.undo(f.doc).unwrap();
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!((o.rows[0].start, o.rows[0].end), (7, 10));
    let _ = range("A1");
}
