//! T16: the fill handle's copy modifier (`FillMode::AutoCopy`, AOO
//! `nScFillModeMouseModifier & KEY_MOD1`) and AutoFill on a filtered sheet
//! (AOO i89232: `ScTable::FillAnalyse` / `FillAuto` with `IsDataFiltered`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{ColumnFilter, FillMode, Op};
use tally_types::Value;

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn t(s: &str) -> Value {
    Value::text(s)
}

fn fill(f: &mut Fixture, source: &str, target: &str, mode: FillMode) {
    let s = f.sheet;
    let r = f.apply(vec![Op::FillRange {
        sheet: s,
        source: range(source),
        target: range(target),
        mode,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

fn input_of(f: &Fixture, a1: &str) -> String {
    use tally_api::DocumentService;
    let a = tally_types::CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn vals(f: &Fixture, cells: &[&str]) -> Vec<Value> {
    cells.iter().map(|c| f.value(c)).collect()
}

#[test]
fn copy_modifier_copies_every_kind_of_source() {
    let mut f = Fixture::new();
    // A single number: AutoFill counts, the modifier copies.
    f.set("A1", "5");
    fill(&mut f, "A1", "A1:A3", FillMode::Auto);
    assert_eq!(vals(&f, &["A2", "A3"]), [n(6.0), n(7.0)]);
    fill(&mut f, "A1", "A1:A3", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["A2", "A3"]), [n(5.0), n(5.0)]);
    // A linear series repeats instead of extending.
    f.set("B1", "1");
    f.set("B2", "3");
    fill(&mut f, "B1:B2", "B1:B5", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["B3", "B4", "B5"]), [n(1.0), n(3.0), n(1.0)]);
    // Numbered text, day names and dates copy too (FillAnalyse returns
    // FILL_SIMPLE before looking at lists, and nDelta is 0).
    f.set("C1", "Item 1");
    f.set("D1", "Monday");
    f.set("E1", "2024-01-31");
    fill(&mut f, "C1:E1", "C1:E3", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["C3", "D3"]), [t("Item 1"), t("Monday")]);
    assert_eq!(f.value("E3"), f.value("E1"));
    // Formulas stay relative.
    f.set("F1", "=A1*2");
    fill(&mut f, "F1", "F1:F3", FillMode::AutoCopy);
    assert_eq!(input_of(&f, "F3"), "=A3*2");
}

#[test]
fn copy_modifier_in_rows_and_upwards() {
    let mut f = Fixture::new();
    f.set("B2", "1");
    f.set("C2", "2");
    fill(&mut f, "B2:C2", "B2:F2", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["D2", "E2", "F2"]), [n(1.0), n(2.0), n(1.0)]);
    f.set("A5", "Item 5");
    fill(&mut f, "A5", "A3:A5", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["A3", "A4"]), [t("Item 5"), t("Item 5")]);
    f.set("H2", "7");
    fill(&mut f, "H2", "G2:H2", FillMode::AutoCopy);
    assert_eq!(f.value("G2"), n(7.0));
}

/// A1 header, A2..A7 = x, y, x, y, x, y; the AutoFilter keeps x (rows 2,
/// 4, 6 visible; 3, 5, 7 filtered).
fn filtered_sheet() -> Fixture {
    let mut f = Fixture::new();
    f.set("A1", "Key");
    for (i, k) in ["x", "y", "x", "y", "x", "y"].iter().enumerate() {
        f.set(&format!("A{}", i + 2), k);
        f.set(&format!("B{}", i + 2), "old");
    }
    let r = f.apply(vec![Op::SetAutoFilter {
        sheet: f.sheet,
        range: Some(range("A1:B7")),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f.apply(vec![Op::SetColumnFilter {
        sheet: f.sheet,
        col: 0,
        filter: Some(ColumnFilter {
            values: Some(vec!["x".into()]),
            condition: None,
        }),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    f
}

#[test]
fn autofill_on_a_filtered_sheet_copies_into_visible_rows_only() {
    let mut f = filtered_sheet();
    f.set("C2", "1");
    f.set("D2", "Item 1");
    fill(&mut f, "B2:D2", "B2:D7", FillMode::Auto);
    // Visible rows get copies (no series); filtered rows keep their content.
    assert_eq!(vals(&f, &["C4", "C6"]), [n(1.0), n(1.0)]);
    assert_eq!(vals(&f, &["D4", "D6"]), [t("Item 1"), t("Item 1")]);
    assert_eq!(
        vals(&f, &["C3", "C5", "C7"]),
        [Value::Empty, Value::Empty, Value::Empty]
    );
    assert_eq!(
        vals(&f, &["B3", "B5", "B7"]),
        [t("old"), t("old"), t("old")]
    );
    assert_eq!(vals(&f, &["B4", "B6"]), [t("old"), t("old")]);
}

#[test]
fn filtered_fill_advances_the_pattern_over_written_rows_only() {
    let mut f = filtered_sheet();
    // A two-cell source (B2 visible, B3 filtered) repeats over the
    // visible rows only: B4 ← B2, B6 ← B3.
    f.set("C2", "a");
    f.set("C3", "b");
    fill(&mut f, "C2:C3", "C2:C7", FillMode::Auto);
    assert_eq!(vals(&f, &["C4", "C6"]), [t("a"), t("b")]);
    assert_eq!(vals(&f, &["C5", "C7"]), [Value::Empty, Value::Empty]);
    // Upwards too, and with the modifier.
    f.set("E7", "z");
    fill(&mut f, "E7", "E2:E7", FillMode::AutoCopy);
    assert_eq!(vals(&f, &["E2", "E4", "E6"]), [t("z"), t("z"), t("z")]);
    assert_eq!(vals(&f, &["E3", "E5"]), [Value::Empty, Value::Empty]);
}

#[test]
fn filtered_fill_right_leaves_filtered_rows_alone() {
    let mut f = filtered_sheet();
    f.set("C2", "1");
    f.set("C3", "1");
    f.set("D2", "2");
    f.set("D3", "2");
    fill(&mut f, "C2:D3", "C2:F3", FillMode::Auto);
    // Row 2 (visible) copies the pattern, row 3 (filtered) is untouched.
    assert_eq!(vals(&f, &["E2", "F2"]), [n(1.0), n(2.0)]);
    assert_eq!(vals(&f, &["E3", "F3"]), [Value::Empty, Value::Empty]);
}

#[test]
fn fill_down_and_series_ignore_the_filter() {
    // ⌘D (AOO FillSimple → FillSeries FILL_SIMPLE) writes filtered rows.
    let mut f = filtered_sheet();
    f.set("C2", "1");
    fill(&mut f, "C2", "C2:C5", FillMode::Copy);
    assert_eq!(vals(&f, &["C3", "C4", "C5"]), [n(1.0), n(1.0), n(1.0)]);
    fill(&mut f, "C2", "C2:C4", FillMode::LinearSeries);
    assert_eq!(vals(&f, &["C3", "C4"]), [n(2.0), n(3.0)]);
}

#[test]
fn autofilter_without_criteria_is_not_filtered() {
    let mut f = Fixture::new();
    f.set("A1", "Key");
    f.set("A2", "1");
    let r = f.apply(vec![Op::SetAutoFilter {
        sheet: f.sheet,
        range: Some(range("A1:A4")),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    fill(&mut f, "A2", "A2:A4", FillMode::Auto);
    assert_eq!(vals(&f, &["A3", "A4"]), [n(2.0), n(3.0)]);
}
