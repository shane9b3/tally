//! Regression tests for the correctness review (docs/reviews/correctness-review-2026-10-19.md).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, sel, Fixture};
use tally_api::{DocumentService, Op};
use tally_types::{CellAddr, ErrorKind, Value};

fn edit(f: &Fixture, a1: &str) -> String {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn num_format(f: &mut Fixture, a1: &str, code: &str) {
    let s = f.sheet;
    f.apply(vec![Op::SetNumberFormat {
        sheet: s,
        ranges: vec![range(a1)],
        code: code.into(),
    }]);
}

fn num(f: &Fixture, a1: &str) -> f64 {
    match f.value(a1) {
        Value::Number(n) => n,
        v => panic!("{a1}: expected a number, got {v:?}"),
    }
}

// ---- H1: time / duration edit text ----

#[test]
fn h1_duration_edit_text_keeps_days() {
    let mut f = Fixture::new();
    f.set("A1", "36:00:00");
    assert_eq!(f.value("A1"), Value::Number(1.5));
    assert_eq!(edit(&f, "A1"), "36:00:00");
    let t = edit(&f, "A1");
    f.set("A1", &t);
    assert_eq!(f.value("A1"), Value::Number(1.5));

    // A plain HH:MM format on a value of a day or more.
    f.set("A2", "1.5");
    num_format(&mut f, "A2", "HH:MM");
    assert_eq!(edit(&f, "A2"), "36:00:00");
    let t = edit(&f, "A2");
    f.set("A2", &t);
    assert_eq!(f.value("A2"), Value::Number(1.5));
}

#[test]
fn h1_negative_time_round_trips() {
    let mut f = Fixture::new();
    f.set("A1", "-0.25");
    num_format(&mut f, "A1", "HH:MM:SS");
    assert_eq!(edit(&f, "A1"), "-06:00:00");
    let t = edit(&f, "A1");
    f.set("A1", &t);
    assert_eq!(f.value("A1"), Value::Number(-0.25));
}

#[test]
fn h1_fractional_seconds_round_trip() {
    let mut f = Fixture::new();
    let v = 0.5 + 1.0 / 86_400.0 / 2.0;
    f.set("A1", &format!("{v}"));
    num_format(&mut f, "A1", "HH:MM:SS");
    assert_eq!(edit(&f, "A1"), "12:00:00.500");
    let t = edit(&f, "A1");
    f.set("A1", &t);
    assert!((num(&f, "A1") - v).abs() < 1e-12, "{:?}", f.value("A1"));

    f.set("A2", "1:30:15.5");
    let t = edit(&f, "A2");
    assert_eq!(t, "01:30:15.500");
    let before = num(&f, "A2");
    f.set("A2", &t);
    assert!((num(&f, "A2") - before).abs() < 1e-12);
}

// ---- H2: criteria with locale numbers and dates ----

#[test]
fn h2_criteria_recognize_locale_input() {
    let mut f = Fixture::new();
    f.set("A1", "1/5/2026");
    f.set("A2", "10/19/2026");
    f.set("A3", "12/1/2026");
    f.set("Z1", "=COUNTIF(A1:A3,\">10/1/2026\")");
    f.set("Z2", "=COUNTIF(A1:A3,\"10/19/2026\")");
    f.set("Z3", "=COUNTIF(A1:A3,\">2026-10-01\")");
    assert_eq!(f.value("Z1"), Value::Number(2.0));
    assert_eq!(f.value("Z2"), Value::Number(1.0));
    assert_eq!(f.value("Z3"), Value::Number(2.0));

    f.set("B1", "500");
    f.set("B2", "1500");
    f.set("B3", "2500");
    f.set("Z4", "=COUNTIF(B1:B3,\">1,000\")");
    assert_eq!(f.value("Z4"), Value::Number(2.0));

    f.set("C1", "0.2");
    f.set("C2", "0.6");
    f.set("Z5", "=COUNTIF(C1:C2,\">50%\")");
    assert_eq!(f.value("Z5"), Value::Number(1.0));

    f.set("D1", "1");
    f.set("D2", "10");
    f.set("Z6", "=SUMIF(D1:D2,\">$5\")");
    assert_eq!(f.value("Z6"), Value::Number(10.0));

    // Booleans and errors keep their meaning.
    f.set("E1", "TRUE");
    f.set("E2", "=NA()");
    f.set("Z7", "=COUNTIF(E1:E2,\"TRUE\")");
    f.set("Z8", "=COUNTIF(E1:E2,\"#N/A\")");
    assert_eq!(f.value("Z7"), Value::Number(1.0));
    assert_eq!(f.value("Z8"), Value::Number(1.0));
}

// ---- H4: long style runs survive to_data / from_data ----

#[test]
fn h4_long_style_runs_round_trip_through_data() {
    use tally_api::StylePatch;
    use tally_engine::storage::StyleId;
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("C100:C20100")],
        patch: StylePatch {
            italic: Some(true),
            ..StylePatch::default()
        },
    }]);
    f.set("A1", "x");
    let d = f.data();
    // The run is kept as a run, not 20,001 styled empty cells.
    assert_eq!(d.sheets[0].cells.len(), 1);
    assert_eq!(d.sheets[0].style_runs.len(), 1);
    let wb = tally_engine::Workbook::from_data(&d);
    let sid = wb.sheets[0].id;
    for r in [99, 5000, 20_099] {
        let st = wb.style_at(CellAddr::new(sid, r, 2));
        assert_ne!(st, StyleId::DEFAULT, "C{}", r + 1);
        assert!(wb.styles.get(st).data.font.italic);
    }
    assert_eq!(wb.style_at(CellAddr::new(sid, 98, 2)), StyleId::DEFAULT);
    assert_eq!(wb.style_at(CellAddr::new(sid, 20_100, 2)), StyleId::DEFAULT);
    assert_eq!(wb.to_data(), d);
}

// ---- M1: cycle detection follows evaluation ----

#[test]
fn m1_address_only_and_untaken_branches_are_not_cycles() {
    let mut f = Fixture::new();
    f.set("A5", "=ROW(A5)-1");
    assert_eq!(f.value("A5"), Value::Number(4.0));
    f.set("B2", "=COLUMN(B2)");
    assert_eq!(f.value("B2"), Value::Number(2.0));
    f.set("A1", "=IF(TRUE(),5,A1)");
    assert_eq!(f.value("A1"), Value::Number(5.0));
    f.set("A1", "=IF(FALSE(),A1,5)");
    assert_eq!(f.value("A1"), Value::Number(5.0));
    f.set("A1", "=IFERROR(1,A1)");
    assert_eq!(f.value("A1"), Value::Number(1.0));
    f.set("A1", "=IFNA(1,A1)");
    assert_eq!(f.value("A1"), Value::Number(1.0));
}

#[test]
fn m1_real_cycles_are_still_522() {
    let circ = Value::Error(ErrorKind::Circular);
    let mut f = Fixture::new();
    f.set("A1", "=IF(TRUE(),A1,5)");
    assert_eq!(f.value("A1"), circ);
    // A followed cycle is circular even when IFERROR would hide the error.
    f.set("A2", "=IFERROR(A2,0)");
    assert_eq!(f.value("A2"), circ);

    let mut f = Fixture::new();
    f.set("A1", "=B1");
    f.set("B1", "=A1");
    assert_eq!(f.value("A1"), circ);
    assert_eq!(f.value("B1"), circ);

    let mut f = Fixture::new();
    f.set("B1", "=C1");
    f.set("C1", "=D1");
    f.set("D1", "=SUM(B1:B2)");
    for a in ["B1", "C1", "D1"] {
        assert_eq!(f.value(a), circ, "{a}");
    }

    // A cycle through a taken IF branch on another cell.
    let mut f = Fixture::new();
    f.set("C1", "1");
    f.set("A1", "=B1+1");
    f.set("B1", "=IF(C1=1,A1,7)");
    assert_eq!(f.value("A1"), circ);
    assert_eq!(f.value("B1"), circ);
    // Not taken: no cycle.
    f.set("C1", "0");
    assert_eq!(f.value("B1"), Value::Number(7.0));
    assert_eq!(f.value("A1"), Value::Number(8.0));
    // Taken again.
    f.set("C1", "1");
    assert_eq!(f.value("A1"), circ);

    // A followed cycle longer than the on-demand nesting limit.
    let mut f = Fixture::new();
    let s = f.sheet;
    let mut ops = Vec::new();
    for r in 1..100u32 {
        ops.push(common::input(
            s,
            &format!("D{r}"),
            &format!("=IF(TRUE(),D{},0)", r + 1),
        ));
    }
    ops.push(common::input(s, "D100", "=IF(TRUE(),D1,0)"));
    f.apply(ops);
    for a in ["D1", "D50", "D100"] {
        assert_eq!(f.value(a), circ, "{a}");
    }
}

#[test]
fn m1_untaken_branch_precedents_still_dirty_the_formula() {
    let mut f = Fixture::new();
    f.set("C1", "0");
    f.set("B1", "10");
    f.set("A1", "=IF(C1=1,B1,0)");
    assert_eq!(f.value("A1"), Value::Number(0.0));
    f.set("B1", "20");
    f.set("C1", "1");
    assert_eq!(f.value("A1"), Value::Number(20.0));
    f.set("B1", "30");
    assert_eq!(f.value("A1"), Value::Number(30.0));
}

#[test]
fn m1_running_balance_inside_if_is_not_circular() {
    // The common `=IF(A2="","",B1+A2)` pattern: each row reads the row above
    // through an IF branch.
    let mut f = Fixture::new();
    f.set("B1", "0");
    for r in (2..=300).rev() {
        f.set(&format!("A{r}"), "1");
        f.set(
            &format!("B{r}"),
            &format!("=IF(A{r}=\"\",\"\",B{}+A{r})", r - 1),
        );
    }
    assert_eq!(f.value("B300"), Value::Number(299.0));
    f.set("A2", "5");
    assert_eq!(f.value("B300"), Value::Number(303.0));
}

#[test]
fn m1_deep_on_demand_chain_fits_the_stack() {
    // A1..A2000 and B1 form one component through IF branches (A2000's
    // untaken branch names B1). B1 is evaluated first and reads A1, which
    // needs the whole chain on demand: 2000 levels, far past
    // MAX_ON_DEMAND_DEPTH, so the work-stack deferral must kick in. Runs on
    // a 256 KiB stack.
    let n: u32 = 2000;
    let h = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let mut f = Fixture::new();
            let s = f.sheet;
            let mut ops = Vec::new();
            for r in 1..n {
                ops.push(common::input(
                    s,
                    &format!("A{r}"),
                    &format!("=IF(TRUE(),A{}+1,0)", r + 1),
                ));
            }
            ops.push(common::input(s, &format!("A{n}"), "=IF(FALSE(),B1,5)"));
            ops.push(common::input(s, "B1", "=IF(TRUE(),A1,0)"));
            f.apply(ops);
            (f.value("A1"), f.value("B1"))
        })
        .unwrap();
    let want = Value::Number(f64::from(n) + 4.0);
    assert_eq!(h.join().unwrap(), (want.clone(), want));
}

// ---- M3: Text-formatted cells keep formulas as text ----

#[test]
fn m3_formula_in_text_formatted_cell_stays_text() {
    let mut f = Fixture::new();
    num_format(&mut f, "A1:A2", "@");
    f.set("A1", "=1+1");
    assert_eq!(f.value("A1"), Value::text("=1+1"));
    assert_eq!(edit(&f, "A1"), "=1+1");
    f.set("A2", "007");
    assert_eq!(f.value("A2"), Value::text("007"));
    // Without the text format it is a formula.
    f.set("B1", "=1+1");
    assert_eq!(f.value("B1"), Value::Number(2.0));
}

// ---- M6: inserted rows take the style of the row above ----

#[test]
fn m6_rows_inserted_below_a_styled_row_inherit_it() {
    use tally_api::StylePatch;
    use tally_engine::storage::StyleId;
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("A1:C1")],
        patch: StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    }]);
    f.set("A1", "Header");
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 1,
        count: 2,
    }]);
    let style = |f: &Fixture, a1: &str| {
        f.svc
            .document(f.doc)
            .unwrap()
            .wb
            .style_at(CellAddr::from_a1(s, a1).unwrap())
    };
    let bold = style(&f, "A1");
    assert_ne!(bold, StyleId::DEFAULT);
    assert_eq!(style(&f, "A2"), bold);
    assert_eq!(style(&f, "C3"), bold);
    assert_eq!(style(&f, "A4"), StyleId::DEFAULT);
    assert_eq!(style(&f, "D2"), StyleId::DEFAULT);
    // Inserting at the top has no row above: the new row is unstyled.
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 0,
        count: 1,
    }]);
    assert_eq!(style(&f, "A1"), StyleId::DEFAULT);
    assert_eq!(style(&f, "A2"), bold);
    // Undo restores the original layout.
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(style(&f, "A1"), bold);
    assert_eq!(style(&f, "A2"), StyleId::DEFAULT);
}

// ---- M7: selection statistics ----

#[test]
fn m7_stats_count_booleans_and_show_errors() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "x");
    f.set("A4", "TRUE");
    f.set("A5", "2");
    let st = f.svc.selection_stats(f.doc, &sel(s, "A1:A6")).unwrap();
    assert_eq!(st.sum, Some(4.0));
    assert_eq!(st.count, 4);
    assert_eq!(st.count_numbers, 3);
    assert_eq!(st.min, Some(1.0));
    assert_eq!(st.formatted_sum.as_deref(), Some("4"));

    f.set("A3", "=1/0");
    let st = f.svc.selection_stats(f.doc, &sel(s, "A1:A6")).unwrap();
    assert_eq!(st.count, 5);
    assert_eq!(st.sum, None);
    assert_eq!(st.average, None);
    assert_eq!(st.min, None);
    assert_eq!(st.max, None);
    assert_eq!(st.formatted_sum.as_deref(), Some("#DIV/0!"));
}

// ---- L: unknown sheet references resolve when the sheet appears ----

#[test]
fn l16_unresolved_sheet_refs_resolve_on_add_and_rename() {
    let mut f = Fixture::new();
    f.set("A1", "=Nosuch.A1+1");
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Ref));
    let r = f.apply(vec![Op::AddSheet {
        name: Some("Nosuch".into()),
        index: None,
    }]);
    let t = r.info.unwrap().sheets[1].id;
    assert_eq!(f.value("A1"), Value::Number(1.0));
    f.apply(vec![common::input(t, "A1", "41")]);
    assert_eq!(f.value("A1"), Value::Number(42.0));
    assert_eq!(edit(&f, "A1"), "=Nosuch.A1+1");
    // Renaming keeps the reference on the sheet (by id).
    f.apply(vec![Op::RenameSheet {
        sheet: t,
        name: "Data".into(),
    }]);
    assert_eq!(edit(&f, "A1"), "=Data.A1+1");
    assert_eq!(f.value("A1"), Value::Number(42.0));

    // A rename to a name that formulas were waiting for.
    f.set("B1", "=SUM(Later.A1:A2)");
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::Ref));
    f.apply(vec![Op::RenameSheet {
        sheet: t,
        name: "later".into(),
    }]);
    assert_eq!(f.value("B1"), Value::Number(41.0));
    // Undo of the rename restores the unresolved formula.
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::Ref));
    assert_eq!(edit(&f, "B1"), "=SUM(Later.A1:A2)");
}

#[test]
fn l16_undo_add_sheet_unresolves_again() {
    let mut f = Fixture::new();
    f.set("A1", "=Nosuch.A1+1");
    f.apply(vec![Op::AddSheet {
        name: Some("Nosuch".into()),
        index: None,
    }]);
    assert_eq!(f.value("A1"), Value::Number(1.0));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Ref));
    assert_eq!(edit(&f, "A1"), "=Nosuch.A1+1");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.value("A1"), Value::Number(1.0));
}

// ---- L17: invalid formulas are saved as their text ----

#[test]
fn l17_invalid_formula_saved_as_text() {
    use tally_model::CellContentData;
    let mut f = Fixture::new();
    f.set("A1", "=SUM(A2:B3 C1)");
    assert!(matches!(f.value("A1"), Value::Error(_)));
    let d = f.data();
    assert_eq!(
        d.sheets[0].cells[0].content,
        CellContentData::Value(Value::text("=SUM(A2:B3 C1)"))
    );
    let wb = tally_engine::Workbook::from_data(&d);
    let a = CellAddr::new(wb.sheets[0].id, 0, 0);
    assert_eq!(wb.value(a), Value::text("=SUM(A2:B3 C1)"));
    assert!(wb.formula_at(a).is_none());
}

// ---- L15 / M8: printing reversed ranges and whole columns ----

#[test]
fn l15_reversed_ranges_and_whole_columns_on_save() {
    use tally_model::CellContentData;
    let mut f = Fixture::new();
    f.set("C1", "=SUM(B2:A1)");
    f.set("C2", "=SUM(A:A)");
    assert_eq!(edit(&f, "C1"), "=SUM(A1:B2)");
    assert_eq!(edit(&f, "C2"), "=SUM(A:A)");
    let d = f.data();
    let texts: Vec<String> = d.sheets[0]
        .cells
        .iter()
        .filter_map(|c| match &c.content {
            CellContentData::Formula { text, .. } => Some(text.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec!["of:=SUM([.A1:.B2])", "of:=SUM([.A$1:.A$1048576])"]
    );
    // Reloading gives back the whole column.
    let mut wb = tally_engine::Workbook::from_data(&d);
    tally_engine::recalc::recalc(&mut wb);
    assert_eq!(wb.to_data(), d);
}
