//! AOO 4.1 parity items: Validity "Sort entries ascending" (its order and
//! the ODS `table:display-list` round trip), Data ▸ Refresh Range (AOO
//! `ScDBDocFunc::RepeatDB`) and Edit ▸ Fill ▸ Up / Left (`FillSimple`).
#![cfg(feature = "io")]
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, tx, Fixture};
use tally_api::{
    AggFn, DocumentService, FileFormat, FillMode, ListSource, Op, SortKey, StdCondition,
    StdConnector, StdFilterOp, StdFilterOptions, SubtotalGroup, SubtotalOptions, Transaction,
    ValidationDto, ValidationKind,
};
use tally_engine::EngineService;
use tally_types::{CellAddr, Value};

fn list_rule(a1: &str, source: ListSource, show: bool, sorted: bool) -> ValidationDto {
    ValidationDto {
        id: 0,
        ranges: vec![range(a1)],
        kind: ValidationKind::List { source },
        allow_blank: true,
        show_dropdown: show,
        sort_ascending: sorted,
        input_help: None,
        error: None,
    }
}

fn items(f: &Fixture, a1: &str) -> Vec<String> {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .validation_at(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .unwrap()
        .list_items
}

/// AOO `TypedScStrCollection` order: values first by number, then strings
/// case-sensitively in code order; equal entries once.
#[test]
fn sorted_list_puts_numbers_first_and_drops_duplicates() {
    let mut f = Fixture::new();
    for (i, v) in [
        "pear", "10", "Apple", "2", "apple", "TRUE", "pear", "", "2.0", "Zebra",
    ]
    .iter()
    .enumerate()
    {
        if !v.is_empty() {
            f.set(&format!("A{}", i + 1), v);
        }
    }
    let r = f.apply(vec![
        Op::AddValidation {
            sheet: f.sheet,
            validation: list_rule(
                "C1",
                ListSource::Range {
                    reference: "$A$1:$A$10".into(),
                },
                true,
                true,
            ),
        },
        Op::AddValidation {
            sheet: f.sheet,
            validation: list_rule(
                "C2",
                ListSource::Range {
                    reference: "$A$1:$A$10".into(),
                },
                true,
                false,
            ),
        },
        Op::AddValidation {
            sheet: f.sheet,
            validation: list_rule(
                "C3",
                ListSource::Values {
                    items: vec!["b".into(), "10".into(), "a".into(), "9".into(), "b".into()],
                },
                true,
                true,
            ),
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // TRUE is the value 1; 2 and 2.0 are one value; "Apple" < "Zebra" < "apple".
    assert_eq!(
        items(&f, "C1"),
        ["TRUE", "2", "10", "Apple", "Zebra", "apple", "pear"]
    );
    // Unsorted: source order, duplicates kept.
    assert_eq!(
        items(&f, "C2"),
        ["pear", "10", "Apple", "2", "apple", "TRUE", "pear", "2", "Zebra"]
    );
    // Inline items are recognised like typed input: 9 < 10 as numbers.
    assert_eq!(items(&f, "C3"), ["9", "10", "a", "b"]);
    // The rule reports its flag; a hidden list is never sorted (one AOO list type).
    let v = &f.svc.validations(f.doc, f.sheet).unwrap()[0];
    assert!(v.sort_ascending);
    let mut hidden = v.clone();
    hidden.show_dropdown = false;
    let r = f.apply(vec![Op::UpdateValidation {
        sheet: f.sheet,
        id: v.id,
        validation: hidden,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!f.svc.validations(f.doc, f.sheet).unwrap()[0].sort_ascending);
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tally-data-small-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// ODS `table:display-list`: `sorted-ascending`, `unsorted` and `no`
/// (AOO XMLStylesExportHelper.cxx:373-383, xmlcvali.cxx:261-275); XLSX
/// has no sorted dropdown, so the flag is dropped there.
#[test]
fn display_list_round_trips_through_ods_and_is_dropped_by_xlsx() {
    let mut svc = EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let (doc, sheet) = (info.doc, info.sheets[0].id);
    let src = || ListSource::Values {
        items: vec!["b".into(), "a".into()],
    };
    let r = svc
        .apply(
            doc,
            tx(vec![
                Op::AddValidation {
                    sheet,
                    validation: list_rule("A1", src(), true, true),
                },
                Op::AddValidation {
                    sheet,
                    validation: list_rule("A2", src(), true, false),
                },
                Op::AddValidation {
                    sheet,
                    validation: list_rule("A3", src(), false, false),
                },
            ]),
        )
        .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let flags = |svc: &EngineService, doc| -> Vec<(bool, bool)> {
        let mut v: Vec<(u32, bool, bool)> = svc
            .validations(doc, svc.info(doc).unwrap().sheets[0].id)
            .unwrap()
            .iter()
            .map(|v| (v.ranges[0].r0, v.show_dropdown, v.sort_ascending))
            .collect();
        v.sort_unstable();
        v.into_iter().map(|(_, s, o)| (s, o)).collect()
    };
    let ods = temp("display-list.ods");
    svc.save(doc, Some(&ods), FileFormat::Ods).unwrap();
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let mut z = zip::ZipArchive::new(std::fs::File::open(&ods).unwrap()).unwrap();
        z.by_name("content.xml")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
    }
    let xml = String::from_utf8(bytes).unwrap();
    assert!(
        xml.contains(r#"table:display-list="sorted-ascending""#),
        "{xml}"
    );
    assert!(xml.contains(r#"table:display-list="unsorted""#));
    assert!(xml.contains(r#"table:display-list="no""#));
    let back = svc.open(&ods).unwrap();
    assert_eq!(
        flags(&svc, back.doc),
        [(true, true), (true, false), (false, false)]
    );
    // Dropdown order after the round trip.
    let at = svc
        .validation_at(back.doc, back.sheets[0].id, 0, 0)
        .unwrap();
    assert_eq!(at.unwrap().list_items, ["a", "b"]);
    let xlsx = temp("display-list.xlsx");
    svc.save(doc, Some(&xlsx), FileFormat::Xlsx).unwrap();
    let back = svc.open(&xlsx).unwrap();
    assert_eq!(
        flags(&svc, back.doc),
        [(true, false), (true, false), (false, false)]
    );
    let _ = std::fs::remove_dir_all(ods.parent().unwrap());
}

fn table(f: &mut Fixture) {
    let rows: Vec<Vec<String>> = [
        ["Name", "Dept", "Pay"],
        ["Cid", "Sales", "20"],
        ["Ann", "Ops", "10"],
        ["Eve", "HR", "50"],
        ["Bob", "Ops", "30"],
    ]
    .iter()
    .map(|r| r.iter().map(|s| s.to_string()).collect())
    .collect();
    f.apply(vec![
        Op::SetRangeInputs {
            sheet: f.sheet,
            r: 0,
            c: 0,
            rows,
        },
        Op::DefineDatabaseRange {
            name: "Staff".into(),
            sheet: f.sheet,
            range: range("A1:C5"),
            has_header: true,
        },
    ]);
}

fn sort_by_name(f: &mut Fixture, a1: &str) {
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range(a1),
        keys: vec![SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: true,
        case_sensitive: false,
        options: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

fn refresh(f: &mut Fixture) -> tally_api::ApplyResult {
    f.apply(vec![Op::RefreshDatabaseRange {
        name: "staff".into(),
    }])
}

fn column(f: &Fixture, col: char, rows: std::ops::RangeInclusive<u32>) -> Vec<String> {
    rows.map(|r| f.text(&format!("{col}{r}"))).collect()
}

/// Refresh Range re-sorts with the sort the database range keeps.
#[test]
fn refresh_range_sorts_again() {
    let mut f = Fixture::new();
    table(&mut f);
    // Nothing kept yet: AOO STR_MSSG_REPEATDB_0.
    let r = refresh(&mut f);
    assert_eq!(r.errors[0].message, "No operations to execute");
    // A sort of another area is not kept.
    sort_by_name(&mut f, "A1:C4");
    assert!(!f.svc.database_ranges(f.doc).unwrap()[0].has_sort);
    sort_by_name(&mut f, "A1:C5");
    let d = &f.svc.database_ranges(f.doc).unwrap()[0];
    assert!(d.has_sort && !d.has_subtotals && d.filter.is_none());
    assert_eq!(column(&f, 'A', 2..=5), ["Ann", "Bob", "Cid", "Eve"]);
    f.set("A2", "Zed");
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(column(&f, 'A', 2..=5), ["Bob", "Cid", "Eve", "Zed"]);
    // One undo step puts the edit's order back.
    f.svc.undo(f.doc).unwrap();
    assert_eq!(column(&f, 'A', 2..=5), ["Zed", "Bob", "Cid", "Eve"]);
    // The sort is saved with the range (ODS `table:sort`) and still works after a reload.
    let data = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    assert_eq!(
        back.sheets[0].database_ranges[0].sort,
        data.sheets[0].database_ranges[0].sort
    );
    assert_eq!(
        back.sheets[0].database_ranges[0]
            .sort
            .as_ref()
            .unwrap()
            .keys,
        vec![(0, true)]
    );
}

/// Refresh Range re-applies the kept filter to the current data.
#[test]
fn refresh_range_filters_again() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C5"),
        conditions: vec![StdCondition {
            connector: StdConnector::And,
            col: 1,
            op: StdFilterOp::Eq,
            value: "Ops".into(),
        }],
        options: StdFilterOptions {
            has_header: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(f.svc.database_ranges(f.doc).unwrap()[0].filter.is_some());
    let hidden = |f: &Fixture| -> Vec<u32> {
        let layout = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
        layout
            .row_runs
            .iter()
            .filter(|r| r.hidden)
            .flat_map(|r| r.start..=r.end)
            .collect()
    };
    assert_eq!(hidden(&f), vec![1, 3]);
    // Row 2 (Ann, Ops) becomes HR; row 4 (Eve) becomes Ops.
    f.set("B3", "HR");
    f.set("B4", "Ops");
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(hidden(&f), vec![1, 2]);
}

/// With a sort and subtotals the subtotal rows go first, the data is
/// sorted, and the subtotals are built again (dbdocfun.cxx:334-365).
#[test]
fn refresh_range_redoes_sort_and_subtotals() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_name(&mut f, "A1:C5");
    let r = f.apply(vec![Op::Subtotals {
        sheet: f.sheet,
        range: range("A1:C5"),
        groups: vec![SubtotalGroup {
            group_by: 1,
            columns: vec![2],
            function: AggFn::Sum,
        }],
        options: SubtotalOptions {
            presort: false,
            ..SubtotalOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = &f.svc.database_ranges(f.doc).unwrap()[0];
    assert!(d.has_sort && d.has_subtotals, "{d:?}");
    // By name: Ann Ops, Bob Ops, Cid Sales, Eve HR → Ops (2), Sales, HR.
    assert_eq!(f.text("B4"), "Ops Sum");
    assert_eq!(f.value("C4"), Value::Number(40.0));
    // Ann earns 100 and moves to Sales; the refresh regroups.
    f.set("B2", "Sales");
    f.set("C2", "100");
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        column(&f, 'B', 2..=10),
        [
            "Sales",
            "Sales Sum",
            "Ops",
            "Ops Sum",
            "Sales",
            "Sales Sum",
            "HR",
            "HR Sum",
            "Grand Total"
        ]
    );
    assert_eq!(f.value("C3"), Value::Number(100.0));
    assert_eq!(f.value("C10"), Value::Number(200.0));
    assert!(f.svc.database_ranges(f.doc).unwrap()[0].has_subtotals);
    // Undo restores the old layout in one step.
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.text("B4"), "Ops Sum");
    assert_eq!(f.text("B2"), "Sales");
}

fn formula(f: &Fixture, sheet: tally_types::SheetId, a1: &str) -> Option<String> {
    let a = CellAddr::from_a1(sheet, a1).unwrap();
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let (id, _) = wb.formula_at(a)?;
    wb.formula_text(id)
}

fn fill(sheet: tally_types::SheetId, source: &str, target: &str) -> Op {
    Op::FillRange {
        sheet,
        source: range(source),
        target: range(target),
        mode: FillMode::Copy,
    }
}

/// Edit ▸ Fill ▸ Up copies the bottom row, Left the right column, with
/// relative references adjusted and numbers not counted (`FILL_SIMPLE`).
#[test]
fn fill_up_and_left_copy_with_relative_references() {
    let mut f = Fixture::new();
    f.set("A5", "5");
    f.set("B5", "=A5*2");
    f.set("C5", "=$A$5+A5");
    let r = f.apply(vec![fill(f.sheet, "A5:C5", "A2:C5")]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    for row in 2..=4 {
        assert_eq!(f.value(&format!("A{row}")), Value::Number(5.0));
        assert_eq!(
            formula(&f, f.sheet, &format!("B{row}")).unwrap(),
            format!("=A{row}*2")
        );
        assert_eq!(
            formula(&f, f.sheet, &format!("C{row}")).unwrap(),
            format!("=$A$5+A{row}")
        );
    }
    f.set("H1", "7");
    f.set("H2", "=H1+1");
    let r = f.apply(vec![fill(f.sheet, "H1:H2", "E1:H2")]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("E1"), Value::Number(7.0));
    assert_eq!(formula(&f, f.sheet, "E2").unwrap(), "=E1+1");
    assert_eq!(f.value("F2"), Value::Number(8.0));
}

/// Fill ops are group ops: Up / Left fill every selected sheet.
#[test]
fn fill_up_and_left_fill_every_grouped_sheet() {
    let mut f = Fixture::new();
    f.svc
        .apply(
            f.doc,
            tx(vec![Op::AddSheet {
                name: Some("S2".into()),
                index: None,
            }]),
        )
        .unwrap();
    let ids: Vec<_> = f
        .svc
        .info(f.doc)
        .unwrap()
        .sheets
        .iter()
        .map(|s| s.id)
        .collect();
    for (i, s) in ids.iter().enumerate() {
        f.svc
            .apply(
                f.doc,
                tx(vec![
                    common::input(*s, "A3", &format!("{}", i + 1)),
                    common::input(*s, "B3", "=A3*10"),
                    common::input(*s, "C1", "x"),
                ]),
            )
            .unwrap();
    }
    let group = |op: Op| Transaction {
        ops: vec![op],
        label: Some("fill".into()),
        selection_before: None,
        selection_after: None,
        group: Some(ids.clone()),
    };
    let r = f
        .svc
        .apply(f.doc, group(fill(ids[0], "A3:B3", "A1:B3")))
        .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f
        .svc
        .apply(f.doc, group(fill(ids[0], "C1", "A1:C1")))
        .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    for (i, s) in ids.iter().enumerate() {
        assert_eq!(formula(&f, *s, "B2").unwrap(), "=A2*10");
        assert_eq!(f.value_on(*s, "A2"), Value::Number((i + 1) as f64));
        assert_eq!(f.value_on(*s, "B2"), Value::Number(((i + 1) * 10) as f64));
        assert_eq!(f.value_on(*s, "A1"), Value::text("x"));
        assert_eq!(f.value_on(*s, "B1"), Value::text("x"));
    }
    // One undo step takes the Left fill back on both sheets.
    f.svc.undo(f.doc).unwrap();
    for (i, s) in ids.iter().enumerate() {
        assert_eq!(formula(&f, *s, "B1").unwrap(), "=A1*10");
        assert_eq!(f.value_on(*s, "B1"), Value::Number(((i + 1) * 10) as f64));
    }
}

fn db(f: &Fixture) -> tally_api::DatabaseRangeDto {
    f.svc.database_ranges(f.doc).unwrap().remove(0)
}

fn sort_by_pay_desc(f: &mut Fixture) {
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("A1:C5"),
        keys: vec![SortKey {
            col: 2,
            ascending: false,
        }],
        has_header: true,
        case_sensitive: false,
        options: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// Pairs (name, pay) of rows `rows`, from columns `name` and `pay`.
fn pairs(
    f: &Fixture,
    name: char,
    pay: char,
    rows: std::ops::RangeInclusive<u32>,
) -> Vec<(String, String)> {
    rows.map(|r| (f.text(&format!("{name}{r}")), f.text(&format!("{pay}{r}"))))
        .collect()
}

/// Review H1: whole columns / rows inserted before and inside the range
/// move it and its kept sort (AOO `ScDBCollection::UpdateReference`), so
/// Refresh sorts the right column and keeps rows together.
#[test]
fn refresh_after_inserted_columns_and_rows_sorts_the_moved_range() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_pay_desc(&mut f);
    assert_eq!(column(&f, 'A', 2..=5), ["Eve", "Bob", "Cid", "Ann"]);
    let r = f.apply(vec![
        Op::InsertCols {
            sheet: f.sheet,
            at: 0,
            count: 1,
        },
        Op::InsertRows {
            sheet: f.sheet,
            at: 2,
            count: 1,
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = db(&f);
    assert_eq!(d.range, range("B1:D6"));
    // Fill the inserted row and change a pay.
    f.set("B3", "Kim");
    f.set("C3", "Ops");
    f.set("D3", "35");
    f.set("D6", "99"); // Ann
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        pairs(&f, 'B', 'D', 2..=6),
        [
            ("Ann", "99"),
            ("Eve", "50"),
            ("Kim", "35"),
            ("Bob", "30"),
            ("Cid", "20")
        ]
        .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert_eq!(column(&f, 'A', 1..=6), ["", "", "", "", "", ""]);
    // Undo takes the refresh back; undoing the inserts puts the range back.
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.text("B2"), "Eve");
    for _ in 0..5 {
        f.svc.undo(f.doc).unwrap();
    }
    assert_eq!(db(&f).range, range("A1:C5"));
}

/// Review H1 with subtotals: the group and total columns move too.
#[test]
fn subtotal_columns_move_with_inserted_columns() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::Subtotals {
        sheet: f.sheet,
        range: range("A1:C5"),
        groups: vec![SubtotalGroup {
            group_by: 1,
            columns: vec![2],
            function: AggFn::Sum,
        }],
        options: SubtotalOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // A column inside the range (before Dept) and one before it.
    let r = f.apply(vec![
        Op::InsertCols {
            sheet: f.sheet,
            at: 1,
            count: 1,
        },
        Op::InsertCols {
            sheet: f.sheet,
            at: 0,
            count: 1,
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let data = f.data();
    let d = &data.sheets[0].database_ranges[0];
    assert_eq!((d.range.c0, d.range.c1), (1, 4));
    let st = d.subtotals.as_ref().unwrap();
    assert_eq!(
        (st.groups[0].group_by, st.groups[0].columns.clone()),
        (3, vec![4])
    );
    let sheet_st = data.sheets[0].subtotals.as_ref().unwrap();
    assert_eq!(sheet_st.groups[0].group_by, 3);
    // Refresh regroups by Dept (now D) and totals Pay (now E): Eve moves
    // from HR to Sales.
    assert_eq!(f.text("B2"), "Eve");
    f.set("D2", "Sales");
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        column(&f, 'D', 2..=8),
        [
            "Ops",
            "Ops",
            "Ops Sum",
            "Sales",
            "Sales",
            "Sales Sum",
            "Grand Total"
        ]
    );
    assert_eq!(f.value("E7"), Value::Number(70.0));
    assert_eq!(f.value("E8"), Value::Number(110.0));
}

/// Deleting every row of a database range leaves it as one row at the
/// deletion point (AOO `ScRefUpdate::Update`); deleting a key column drops
/// that key, and the undo restores both.
#[test]
fn deleted_rows_and_columns_of_a_database_range() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_pay_desc(&mut f);
    let r = f.apply(vec![Op::DeleteCols {
        sheet: f.sheet,
        at: 2,
        count: 1,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = db(&f);
    assert_eq!(d.range, range("A1:B5"));
    assert!(!d.has_sort, "the Pay key went with its column");
    f.svc.undo(f.doc).unwrap();
    assert!(db(&f).has_sort);
    let r = f.apply(vec![Op::DeleteRows {
        sheet: f.sheet,
        at: 0,
        count: 6,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(db(&f).range, range("A1:C1"));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(db(&f).range, range("A1:C5"));
}

/// Inserted cells shifting the range right, and a block move, carry the
/// range and its kept sort.
#[test]
fn cell_shifts_and_moves_carry_the_range() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_pay_desc(&mut f);
    let r = f.apply(vec![Op::InsertCells {
        sheet: f.sheet,
        range: range("A1:A5"),
        shift: tally_api::ShiftDir::Right,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(db(&f).range, range("B1:D5"));
    let r = f.apply(vec![Op::MoveRange {
        sheet: f.sheet,
        from: range("B1:D5"),
        to: tally_api::CellPos { r: 10, c: 4 },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(db(&f).range, range("E11:G15"));
    f.set("G12", "1"); // Eve's pay
    let r = refresh(&mut f);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(column(&f, 'E', 12..=15), ["Bob", "Cid", "Ann", "Eve"]);
    // Undo the refresh and the edit, then the move.
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(db(&f).range, range("E11:G15"));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(db(&f).range, range("B1:D5"));
    assert_eq!(f.text("B2"), "Eve");
}

/// Review L4: a copy or left-to-right sort clears the kept sort; the
/// sort's header flag becomes the range's.
#[test]
fn copy_and_left_to_right_sorts_clear_the_kept_sort() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_pay_desc(&mut f);
    assert!(db(&f).has_sort);
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("A1:C5"),
        keys: vec![SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: true,
        case_sensitive: false,
        options: Some(tally_api::SortOptions {
            copy_to: Some(tally_api::SheetCell {
                sheet: f.sheet,
                r: 20,
                c: 0,
            }),
            include_formats: true,
            ..tally_api::SortOptions::default()
        }),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!db(&f).has_sort);
    assert_eq!(
        refresh(&mut f).errors[0].message,
        "No operations to execute"
    );
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("A1:C5"),
        keys: vec![SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let d = db(&f);
    assert!(d.has_sort && !d.has_header);
}

/// Review L5: Fill copies no merges (AOO table4.cxx:606-614 clears
/// ATTR_MERGE / ATTR_MERGE_FLAG from the copied pattern).
#[test]
fn fill_up_does_not_copy_merges() {
    let mut f = Fixture::new();
    f.set("A4", "x");
    let r = f.apply(vec![Op::MergeCells {
        sheet: f.sheet,
        range: range("A4:B4"),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r = f.apply(vec![fill(f.sheet, "A4:B4", "A2:B4")]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.data().sheets[0].merges.len(), 1);
    assert_eq!(f.value("A2"), Value::text("x"));
}

/// Safety net: a range whose kept parameters name a column outside it
/// (here from a file) is refused instead of sorting the wrong data.
#[test]
fn refresh_refuses_parameters_outside_the_range() {
    let mut f = Fixture::new();
    table(&mut f);
    sort_by_pay_desc(&mut f);
    let mut data = f.data();
    data.sheets[0].database_ranges[0].subtotals = Some(tally_model::SubtotalsData {
        range: data.sheets[0].database_ranges[0].range,
        groups: vec![tally_model::SubtotalGroupData {
            group_by: 9,
            columns: vec![2],
            function: tally_model::AggFnData::Sum,
        }],
        ..tally_model::SubtotalsData::default()
    });
    let p = std::env::temp_dir().join(format!(
        "tally-data-small-outside-{}.ods",
        std::process::id()
    ));
    let mut file = std::fs::File::create(&p).unwrap();
    tally_io_ods::write_ods(&data, &mut file).unwrap();
    drop(file);
    let mut svc = EngineService::with_default_codec();
    let info = svc.open(&p).unwrap();
    let r = svc
        .apply(
            info.doc,
            tx(vec![Op::RefreshDatabaseRange {
                name: "Staff".into(),
            }]),
        )
        .unwrap();
    assert!(
        r.errors[0].message.contains("outside the range"),
        "{:?}",
        r.errors
    );
    let _ = std::fs::remove_file(&p);
}
