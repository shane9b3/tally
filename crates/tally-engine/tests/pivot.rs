//! DataPilot (wave 5, W5-2): layout, aggregation, show-as, filters,
//! refresh, undo, structure, grouping, drill-down and GETPIVOTDATA on the
//! AOO help dataset (`tests/fixtures/pivot/sales.csv`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    AggFn, ApiError, AutoShow, BaseItem, CellPos, DataLayout, DatePart, DocumentService, Op,
    PivotCellRole, PivotDataFieldDto, PivotDto, PivotFieldDto, PivotGroupDto, PivotLayout,
    PivotSort, PivotSource, PivotSubtotals, ShowAs, ShowAsKind,
};
use tally_types::{CellAddr, Value};

fn sales_rows() -> Vec<Vec<String>> {
    let csv = include_str!("../../../tests/fixtures/pivot/sales.csv");
    csv.lines()
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect()
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    let rows = sales_rows();
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    f
}

fn field(col: u32, name: &str) -> PivotFieldDto {
    PivotFieldDto {
        source_col: col,
        name: name.into(),
        subtotals: PivotSubtotals::Auto,
        show_empty: false,
        sort: PivotSort::Name { ascending: true },
        hidden_items: Vec::new(),
        page_item: None,
        layout: PivotLayout::Tabular,
        empty_line_after: false,
        auto_show: None,
        group: None,
    }
}

fn data(col: u32, function: AggFn) -> PivotDataFieldDto {
    PivotDataFieldDto {
        source_col: col,
        function,
        caption: None,
        show_as: None,
    }
}

fn pivot(f: &Fixture) -> PivotDto {
    PivotDto {
        id: 0,
        name: String::new(),
        source: PivotSource::Range {
            sheet: f.sheet,
            range: range("A1:F25"),
        },
        target_sheet: f.sheet,
        target: CellPos { r: 0, c: 7 },
        output: None,
        row_fields: vec![field(0, "Region")],
        col_fields: vec![field(3, "Quarter")],
        page_fields: Vec::new(),
        data_fields: vec![data(5, AggFn::Sum)],
        data_layout: DataLayout::Row,
        row_totals: true,
        col_totals: true,
        ignore_empty_rows: false,
        identify_categories: false,
        show_filter_button: false,
        drill_down: true,
        source_filter: None,
    }
}

fn create_default(f: &mut Fixture) -> u32 {
    let p = pivot(f);
    create(f, p)
}

fn create(f: &mut Fixture, p: PivotDto) -> u32 {
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.pivots_changed, Some(true));
    r.created.unwrap()[0].id
}

#[test]
fn pivot_sum_region_by_quarter() {
    let mut f = fixture();
    let id = create_default(&mut f);
    // Header block.
    assert_eq!(f.value("H1"), Value::text("Sum - Revenue"));
    assert_eq!(f.value("I1"), Value::text("Quarter"));
    assert_eq!(f.value("H2"), Value::text("Region"));
    assert_eq!(f.value("I2"), Value::text("Q1"));
    assert_eq!(f.value("L2"), Value::text("Q4"));
    assert_eq!(f.value("M2"), Value::text("Total Result"));
    // Rows and data.
    assert_eq!(f.value("H3"), Value::text("East"));
    assert_eq!(f.value("I3"), Value::Number(190.0));
    assert_eq!(f.value("M3"), Value::Number(850.0));
    assert_eq!(f.value("H4"), Value::text("West"));
    assert_eq!(f.value("M4"), Value::Number(1465.0));
    assert_eq!(f.value("H5"), Value::text("Total Result"));
    assert_eq!(f.value("M5"), Value::Number(2315.0));
    assert_eq!(f.value("I5"), Value::Number(535.0));
    let pivots = f.svc.pivots(f.doc).unwrap();
    assert_eq!(pivots.len(), 1);
    assert_eq!(pivots[0].id, id);
    assert_eq!(pivots[0].name, "DataPilot1");
    assert_eq!(pivots[0].output, Some(range("H1:M5")));
    // Roles.
    let hit = f.svc.pivot_at(f.doc, f.sheet, 2, 7).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::RowItem);
    assert_eq!(hit.item.as_deref(), Some("East"));
    let hit = f.svc.pivot_at(f.doc, f.sheet, 2, 8).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::Data);
    let hit = f.svc.pivot_at(f.doc, f.sheet, 4, 12).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::Total);
    assert!(f.svc.pivot_at(f.doc, f.sheet, 10, 10).unwrap().is_none());
    // GETPIVOTDATA (AOO and Excel forms).
    f.set(
        "A30",
        "=GETPIVOTDATA(\"Sum - Revenue\",H1,\"Region\",\"East\")",
    );
    assert_eq!(f.value("A30"), Value::Number(850.0));
    f.set("A31", "=GETPIVOTDATA(\"Revenue\",H1)");
    assert_eq!(f.value("A31"), Value::Number(2315.0));
    f.set(
        "A32",
        "=GETPIVOTDATA(\"Revenue\",H1,\"Region\",\"West\",\"Quarter\",\"Q2\")",
    );
    assert_eq!(f.value("A32"), Value::Number(385.0));
    f.set("A33", "=GETPIVOTDATA(H1,\"Revenue East Quarter[Q1]\")");
    assert_eq!(f.value("A33"), Value::Number(190.0));
    f.set("A34", "=GETPIVOTDATA(\"Revenue\",H1,\"Nope\",\"x\")");
    assert!(matches!(f.value("A34"), Value::Error(_)));
}

#[test]
fn pivot_count_and_countnums() {
    let mut f = fixture();
    f.set("F3", "x");
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.data_fields = vec![data(5, AggFn::Count), data(5, AggFn::CountNums)];
    create(&mut f, p);
    // Two data fields: the Data pseudo-field as a row level.
    assert_eq!(f.value("H1"), Value::text("Region"));
    assert_eq!(f.value("I1"), Value::text("Data"));
    assert_eq!(f.value("J1"), Value::text("Data"));
    assert_eq!(f.value("H2"), Value::text("East"));
    assert_eq!(f.value("I2"), Value::text("Count - Revenue"));
    assert_eq!(f.value("J2"), Value::Number(12.0));
    // AOO names both functions "Count" in the output (`nFuncStrIds`).
    assert_eq!(f.value("I3"), Value::text("Count - Revenue"));
    assert_eq!(f.value("J3"), Value::Number(11.0));
}

#[test]
fn pivot_stdev_var_match_functions() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.row_fields.clear();
    p.data_fields = vec![data(5, AggFn::StDev), data(5, AggFn::Var)];
    p.row_totals = false;
    p.col_totals = false;
    create(&mut f, p);
    f.set("A30", "=STDEV(F2:F25)");
    f.set("A31", "=VAR(F2:F25)");
    let st = match f.value("I2") {
        Value::Number(n) => n,
        other => panic!("{other:?}"),
    };
    let va = match f.value("I3") {
        Value::Number(n) => n,
        other => panic!("{other:?}"),
    };
    assert!(matches!(f.value("A30"), Value::Number(n) if (n - st).abs() < 1e-9));
    assert!(matches!(f.value("A31"), Value::Number(n) if (n - va).abs() < 1e-9));
}

#[test]
fn pivot_two_row_fields_auto_subtotals() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Region"), field(2, "Product")];
    create(&mut f, p);
    assert_eq!(f.value("H1"), Value::text("Region"));
    assert_eq!(f.value("I1"), Value::text("Product"));
    assert_eq!(f.value("J1"), Value::text("Sum - Revenue"));
    assert_eq!(f.value("H2"), Value::text("East"));
    assert_eq!(f.value("I2"), Value::text("Apple"));
    assert_eq!(f.value("J2"), Value::Number(450.0));
    assert_eq!(f.value("I3"), Value::text("Pear"));
    assert_eq!(f.value("I4"), Value::text("Plum"));
    assert_eq!(f.value("H5"), Value::text("East Result"));
    assert_eq!(f.value("J5"), Value::Number(850.0));
    assert_eq!(f.value("H6"), Value::text("West"));
    assert_eq!(f.value("H9"), Value::text("West Result"));
    assert_eq!(f.value("H10"), Value::text("Total Result"));
    assert_eq!(f.value("J10"), Value::Number(2315.0));
}

#[test]
fn pivot_no_grand_totals() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.row_totals = false;
    p.col_totals = false;
    create(&mut f, p);
    assert_eq!(f.value("L2"), Value::text("Q4"));
    assert_eq!(f.value("M2"), Value::Empty);
    assert_eq!(f.value("H5"), Value::Empty);
    assert_eq!(f.svc.pivots(f.doc).unwrap()[0].output, Some(range("H1:L4")));
}

#[test]
fn pivot_page_field_filters() {
    let mut f = fixture();
    let mut p = pivot(&f);
    let mut page = field(2, "Product");
    page.page_item = Some("Apple".into());
    p.page_fields = vec![page];
    p.col_fields.clear();
    create(&mut f, p);
    assert_eq!(f.value("H1"), Value::text("Product"));
    assert_eq!(f.value("I1"), Value::text("Apple"));
    assert_eq!(f.value("H2"), Value::Empty);
    assert_eq!(f.value("H3"), Value::text("Region"));
    assert_eq!(f.value("H4"), Value::text("East"));
    assert_eq!(f.value("I4"), Value::Number(450.0));
    assert_eq!(f.value("I5"), Value::Number(850.0));
    assert_eq!(f.value("I6"), Value::Number(1300.0));
}

#[test]
fn pivot_hidden_items_and_manual_sort() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.row_fields = vec![field(2, "Product")];
    p.row_fields[0].hidden_items = vec!["Pear".into()];
    p.row_fields[0].sort = PivotSort::Manual {
        order: vec!["Plum".into(), "Apple".into()],
    };
    create(&mut f, p);
    assert_eq!(f.value("H2"), Value::text("Plum"));
    assert_eq!(f.value("H3"), Value::text("Apple"));
    assert_eq!(f.value("H4"), Value::text("Total Result"));
    assert_eq!(f.value("I4"), Value::Number(290.0 + 1300.0));
}

#[test]
fn pivot_sort_desc_by_data_and_autoshow() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.row_fields = vec![field(2, "Product")];
    p.row_fields[0].sort = PivotSort::Data {
        data_field: 0,
        ascending: false,
    };
    create(&mut f, p);
    assert_eq!(f.value("H2"), Value::text("Apple"));
    assert_eq!(f.value("H3"), Value::text("Pear"));
    assert_eq!(f.value("H4"), Value::text("Plum"));
    let mut p2 = pivot(&f);
    p2.target = CellPos { r: 20, c: 7 };
    p2.col_fields.clear();
    p2.row_fields = vec![field(2, "Product")];
    p2.row_fields[0].auto_show = Some(AutoShow {
        top: true,
        count: 1,
        data_field: 0,
    });
    create(&mut f, p2);
    assert_eq!(f.value("H22"), Value::text("Apple"));
    assert_eq!(f.value("H23"), Value::text("Total Result"));
}

#[test]
fn pivot_show_as_percent_of_column_and_running_total() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.data_fields[0].show_as = Some(ShowAs {
        kind: ShowAsKind::PercentOfColumn,
        base_field: None,
        base_item: None,
    });
    create(&mut f, p);
    assert!(matches!(f.value("I2"), Value::Number(n) if (n - 850.0 / 2315.0).abs() < 1e-12));
    assert!(matches!(f.value("I4"), Value::Number(n) if (n - 1.0).abs() < 1e-12));
    let mut p2 = pivot(&f);
    p2.target = CellPos { r: 20, c: 7 };
    p2.col_fields.clear();
    p2.data_fields[0].show_as = Some(ShowAs {
        kind: ShowAsKind::RunningTotal,
        base_field: Some(0),
        base_item: None,
    });
    create(&mut f, p2);
    assert_eq!(f.value("I22"), Value::Number(850.0));
    assert_eq!(f.value("I23"), Value::Number(2315.0));
}

#[test]
fn pivot_two_data_fields_in_columns() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.col_fields.clear();
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    p.data_layout = DataLayout::Column;
    create(&mut f, p);
    assert_eq!(f.value("I1"), Value::text("Data"));
    assert_eq!(f.value("I2"), Value::text("Sum - Revenue"));
    assert_eq!(f.value("J2"), Value::text("Sum - Units"));
    assert_eq!(f.value("I3"), Value::Number(850.0));
    assert_eq!(f.value("J3"), Value::Number(77.0));
}

#[test]
fn pivot_target_not_empty_refused_then_overwrite() {
    let mut f = fixture();
    f.set("J3", "keep");
    let p = pivot(&f);
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: false,
    }]);
    assert_eq!(r.errors[0].code, "target_not_empty");
    assert_eq!(f.value("J3"), Value::text("keep"));
    let p = pivot(&f);
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: true,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("J3"), Value::Number(230.0));
    // A pivot over its own source is refused.
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 0 };
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: true,
    }]);
    assert_eq!(r.errors[0].code, ApiError::INVALID_ARGUMENT);
}

#[test]
fn pivot_refresh_after_source_edit_and_update() {
    let mut f = fixture();
    let id = create_default(&mut f);
    f.set("F2", "1100");
    // AOO never refreshes automatically.
    assert_eq!(f.value("I3"), Value::Number(190.0));
    let r = f.apply(vec![Op::RefreshPivot {
        id,
        overwrite: false,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("I3"), Value::Number(1190.0));
    // Update: fewer columns; the old output is cleared.
    let mut p = pivot(&f);
    p.col_fields.clear();
    let r = f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p,
        overwrite: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("I2"), Value::Number(1850.0));
    assert_eq!(f.value("M3"), Value::Empty);
    let r = f.apply(vec![Op::DeletePivot {
        id,
        clear_output: true,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("H1"), Value::Empty);
    assert!(f.svc.pivots(f.doc).unwrap().is_empty());
}

#[test]
fn pivot_undo_restores_output_and_definition() {
    let mut f = fixture();
    f.set("H1", "old");
    let p = pivot(&f);
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: true,
    }]);
    assert!(r.errors.is_empty());
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("H1"), Value::text("old"));
    assert_eq!(f.value("I3"), Value::Empty);
    assert!(f.svc.pivots(f.doc).unwrap().is_empty());
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.value("I3"), Value::Number(190.0));
    assert_eq!(f.svc.pivots(f.doc).unwrap().len(), 1);
}

#[test]
fn pivot_insert_rows_moves_source_and_target() {
    let mut f = fixture();
    create_default(&mut f);
    f.apply(vec![Op::InsertRows {
        sheet: f.sheet,
        at: 0,
        count: 2,
    }]);
    let p = &f.svc.pivots(f.doc).unwrap()[0];
    assert_eq!(p.target, CellPos { r: 2, c: 7 });
    assert_eq!(p.output, Some(range("H3:M7")));
    match &p.source {
        PivotSource::Range { range, .. } => assert_eq!(*range, common::range("A3:F27")),
        other => panic!("{other:?}"),
    }
    assert!(f.svc.pivot_at(f.doc, f.sheet, 4, 7).unwrap().is_some());
    let r = f.apply(vec![Op::RefreshPivot {
        id: p.id,
        overwrite: false,
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("I5"), Value::Number(190.0));
}

#[test]
fn pivot_group_months_and_numeric_step() {
    let mut f = Fixture::new();
    let rows: Vec<Vec<String>> = vec![
        vec!["Date".into(), "Amount".into()],
        vec!["2024-01-05".into(), "10".into()],
        vec!["2024-01-20".into(), "5".into()],
        vec!["2024-02-03".into(), "7".into()],
        vec!["2024-04-11".into(), "1".into()],
    ];
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range("A1:B5"),
    };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Date")];
    p.row_fields[0].group = Some(PivotGroupDto::Date {
        start: None,
        end: None,
        parts: vec![DatePart::Months],
    });
    p.data_fields = vec![data(1, AggFn::Sum)];
    let id = create(&mut f, p);
    assert_eq!(f.value("H2"), Value::text("Jan"));
    assert_eq!(f.value("I2"), Value::Number(15.0));
    assert_eq!(f.value("H3"), Value::text("Feb"));
    assert_eq!(f.value("H4"), Value::text("Apr"));
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range("A1:B5"),
    };
    p.col_fields.clear();
    p.row_fields = vec![field(1, "Amount")];
    p.row_fields[0].group = Some(PivotGroupDto::Numeric {
        start: Some(0.0),
        end: None,
        step: 5.0,
    });
    p.data_fields = vec![data(1, AggFn::Count)];
    let r = f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p,
        overwrite: true,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // AOO `dpgroup.cxx:226-268`: the automatic end is the largest value
    // (10), which joins the last group, shown including the end.
    assert_eq!(f.value("H2"), Value::text("0-4"));
    assert_eq!(f.value("I2"), Value::Number(1.0));
    assert_eq!(f.value("H3"), Value::text("5-10"));
    assert_eq!(f.value("I3"), Value::Number(3.0));
    assert_eq!(f.value("H4"), Value::text("Total Result"));
}

#[test]
fn pivot_drill_down_sheet() {
    let mut f = fixture();
    let id = create_default(&mut f);
    let r = f.apply(vec![Op::PivotDrillDown { id, r: 2, c: 8 }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r.sheets_changed);
    let new = r.created.unwrap()[0].id;
    assert_eq!(f.value_on(new, "A1"), Value::text("Region"));
    assert_eq!(f.value_on(new, "A2"), Value::text("East"));
    assert_eq!(f.value_on(new, "D2"), Value::text("Q1"));
    assert_eq!(f.value_on(new, "A4"), Value::text("East"));
    assert_eq!(f.value_on(new, "A5"), Value::Empty, "three East/Q1 rows");
    f.svc.undo(f.doc).unwrap();
    assert!(f.svc.document(f.doc).unwrap().wb.sheet(new).is_none());
}

#[test]
fn pivot_field_items_scans_the_source() {
    let f = fixture();
    let items = f
        .svc
        .pivot_field_items(
            f.doc,
            &PivotSource::Range {
                sheet: f.sheet,
                range: range("A1:F25"),
            },
        )
        .unwrap();
    assert_eq!(items.len(), 6);
    assert_eq!(items[0].name, "Region");
    assert_eq!(items[0].items.len(), 2);
    assert_eq!(items[0].items[0].text, "East");
    assert_eq!(items[0].items[0].count, 12);
    assert!(items[4].numeric);
    assert!(!items[0].numeric);
}

#[test]
fn pivot_budget_refuses_huge_sources() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range("A1:K1000000"),
    };
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: false,
    }]);
    assert_eq!(r.errors[0].code, ApiError::UNSUPPORTED);
    let _ = CellAddr::new(f.sheet, 0, 0);
}

// ---------------------------------------------------------------------------
// Correctness review 4 (H3-H7, M8-M14) and security review 4 (H7, H8, M7-M9)
// ---------------------------------------------------------------------------

fn load(f: &mut Fixture, rows: &[&[&str]]) {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
}

/// A pivot over `src` at K1 with one row field (no sorting).
fn simple(f: &Fixture, src: &str, row: u32, data_col: u32, fun: AggFn) -> PivotDto {
    let mut p = pivot(f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range(src),
    };
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(row, "")];
    p.row_fields[0].sort = PivotSort::None;
    p.data_fields = vec![data(data_col, fun)];
    p
}

fn err() -> Value {
    Value::Error(tally_types::ErrorKind::Value)
}

fn clear_pivots(f: &mut Fixture) {
    let ids: Vec<u32> = f.svc.pivots(f.doc).unwrap().iter().map(|p| p.id).collect();
    for id in ids {
        f.apply(vec![Op::DeletePivot {
            id,
            clear_output: true,
        }]);
    }
}

/// H3: AOO `ScDPAggData::Update/Calculate` (`dptabres.cxx:415-597`):
/// strings are ignored except by Count, no data is empty, TRUE is a value,
/// errors do not stop Count and are output as #VALUE!
/// (`dpoutput.cxx:573-575`), StDev needs two values.
#[test]
fn pivot_aggregates_text_bool_and_errors_as_aoo() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[
            &["G", "V"],
            &["txt", "a"],
            &["txt", "b"],
            &["bool", "TRUE"],
            &["bool", "TRUE"],
            &["one", "5"],
            &["err", "=1/0"],
            &["err", "3"],
            &["mix", "4"],
            &["mix", "x"],
        ],
    );
    let col = |f: &Fixture| -> Vec<Value> { (2..=7).map(|r| f.value(&format!("L{r}"))).collect() };
    let run = |f: &mut Fixture, fun: AggFn| -> Vec<Value> {
        let p = simple(f, "A1:B10", 0, 1, fun);
        create(f, p);
        assert_eq!(f.value("K2"), Value::text("txt"));
        assert_eq!(f.value("K7"), Value::text("Total Result"));
        let v = col(f);
        clear_pivots(f);
        v
    };
    let n = Value::Number;
    assert_eq!(
        run(&mut f, AggFn::Sum),
        [Value::Empty, n(2.0), n(5.0), err(), n(4.0), err()]
    );
    assert_eq!(
        run(&mut f, AggFn::Count),
        [n(2.0), n(2.0), n(1.0), n(2.0), n(2.0), n(9.0)]
    );
    assert_eq!(
        run(&mut f, AggFn::CountNums),
        [Value::Empty, n(2.0), n(1.0), err(), n(1.0), err()]
    );
    assert_eq!(
        run(&mut f, AggFn::Min),
        [Value::Empty, n(1.0), n(5.0), err(), n(4.0), err()]
    );
    assert_eq!(
        run(&mut f, AggFn::Average)[0..3],
        [Value::Empty, n(1.0), n(5.0)]
    );
    assert_eq!(
        run(&mut f, AggFn::StDev)[0..3],
        [Value::Empty, n(0.0), err()]
    );
}

/// H4 and M8: items are identified by value or case-insensitive text
/// (`dptablecache.cxx:265-295`); numeric captions are written as numbers
/// (`dpoutput.cxx:619-629`); the empty item is "(empty)".
#[test]
fn pivot_items_by_value_and_case_insensitive_text() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[
            &["K", "V"],
            &["apple", "1"],
            &["Apple", "2"],
            &["10", "4"],
            &["2", "8"],
            &["'2", "16"],
            &["", "32"],
            &["banana", "64"],
            &["1.2", "128"],
            &["1.4", "256"],
            &["TRUE", "512"],
        ],
    );
    f.apply(vec![Op::SetNumberFormat {
        sheet: f.sheet,
        ranges: vec![range("A9:A10")],
        code: "0".into(),
    }]);
    let mut p = simple(&f, "A1:B11", 0, 1, AggFn::Sum);
    p.row_fields[0].sort = PivotSort::Name { ascending: true };
    create(&mut f, p);
    let got: Vec<(Value, Value)> = (2..=11)
        .map(|r| (f.value(&format!("K{r}")), f.value(&format!("L{r}"))))
        .collect();
    let n = Value::Number;
    assert_eq!(
        got,
        [
            (Value::Bool(true), n(512.0)),
            (n(1.0), n(128.0)),
            (n(1.0), n(256.0)),
            (n(2.0), n(8.0)),
            (n(10.0), n(4.0)),
            (Value::text("2"), n(16.0)),
            (Value::text("apple"), n(3.0)),
            (Value::text("banana"), n(64.0)),
            (Value::text("(empty)"), n(32.0)),
            (Value::text("Total Result"), n(1023.0)),
        ]
    );
    // GETPIVOTDATA matches the item text; "1" names two items: #REF!.
    f.set("D1", "=GETPIVOTDATA(\"V\";K1;\"K\";\"APPLE\")");
    assert_eq!(f.value("D1"), n(3.0));
    f.set("D2", "=GETPIVOTDATA(\"V\";K1;\"K\";1)");
    assert!(matches!(
        f.value("D2"),
        Value::Error(tally_types::ErrorKind::Ref)
    ));
    f.set("D3", "=GETPIVOTDATA(\"V\";K1;\"K\";\"(empty)\")");
    assert_eq!(f.value("D3"), n(32.0));
}

fn region_by_quarter(f: &Fixture, sa: ShowAs) -> PivotDto {
    let mut p = pivot(f);
    p.target = CellPos { r: 0, c: 10 };
    p.data_fields[0].show_as = Some(sa);
    p
}

fn sa(kind: ShowAsKind, base_field: Option<u32>, base_item: Option<BaseItem>) -> ShowAs {
    ShowAs {
        kind,
        base_field,
        base_item,
    }
}

/// H5 and M10(a): reference modes on subtotals and totals
/// (`dptabres.cxx:2330-2530`); GETPIVOTDATA returns the displayed value
/// (`dpoutput.cxx:1820-1828`).
#[test]
fn pivot_show_as_totals_and_missing_bases() {
    let mut f = fixture();
    let n = Value::Number;
    // Difference from the previous quarter: Q1 and the Total Result
    // column (a total in the reference dimension) are empty; the Total
    // Result row is computed.
    let p = region_by_quarter(
        &f,
        sa(ShowAsKind::Difference, Some(3), Some(BaseItem::Previous)),
    );
    create(&mut f, p);
    assert_eq!(f.value("L3"), Value::Empty);
    assert_eq!(f.value("M3"), n(40.0));
    assert_eq!(f.value("P3"), Value::Empty);
    assert_eq!(f.value("M5"), n(80.0));
    f.set(
        "A40",
        "=GETPIVOTDATA(\"Revenue\";K1;\"Region\";\"East\";\"Quarter\";\"Q2\")",
    );
    assert_eq!(f.value("A40"), n(40.0));
    clear_pivots(&mut f);
    // Percentage of the previous item: the first is empty, not #N/A.
    let p = region_by_quarter(
        &f,
        sa(ShowAsKind::PercentOf, Some(3), Some(BaseItem::Previous)),
    );
    create(&mut f, p);
    assert_eq!(f.value("L3"), Value::Empty);
    assert!(matches!(f.value("M3"), Value::Number(x) if (x - 230.0 / 190.0).abs() < 1e-12));
    clear_pivots(&mut f);
    // A named base item that does not exist is an error.
    for kind in [ShowAsKind::Difference, ShowAsKind::PercentOf] {
        let p = region_by_quarter(
            &f,
            sa(kind, Some(3), Some(BaseItem::Named { name: "Q9".into() })),
        );
        create(&mut f, p);
        assert_eq!(f.value("L3"), err());
        clear_pivots(&mut f);
    }
    // A base field outside the layout is an error.
    let p = region_by_quarter(&f, sa(ShowAsKind::RunningTotal, Some(1), None));
    create(&mut f, p);
    assert_eq!(f.value("L3"), err());
    clear_pivots(&mut f);
    // Region > Product, difference from Region = East: the subtotal rows
    // are computed, the reference item itself and the grand total empty.
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Region"), field(2, "Product")];
    p.data_fields[0].show_as = Some(sa(
        ShowAsKind::Difference,
        Some(0),
        Some(BaseItem::Named {
            name: "East".into(),
        }),
    ));
    create(&mut f, p);
    assert_eq!(f.value("K5"), Value::text("East Result"));
    assert_eq!(f.value("M2"), Value::Empty);
    assert_eq!(f.value("M5"), Value::Empty);
    assert_eq!(f.value("M6"), n(400.0));
    assert_eq!(f.value("K9"), Value::text("West Result"));
    assert_eq!(f.value("M9"), n(615.0));
    assert_eq!(f.value("M10"), Value::Empty);
}

/// H6: AutoShow keeps members tied with the last one and the totals
/// exclude the hidden members (`dptabres.cxx:3164-3184`,
/// `dptabsrc.cxx:986-995`).
#[test]
fn pivot_autoshow_keeps_ties_and_excludes_hidden_from_totals() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[
            &["K", "V"],
            &["a", "10"],
            &["b", "5"],
            &["c", "5"],
            &["d", "1"],
        ],
    );
    let mut p = simple(&f, "A1:B5", 0, 1, AggFn::Sum);
    p.row_fields[0].auto_show = Some(AutoShow {
        top: true,
        count: 2,
        data_field: 0,
    });
    p.row_fields[0].sort = PivotSort::Name { ascending: true };
    create(&mut f, p);
    let col: Vec<Value> = (2..=5).map(|r| f.value(&format!("K{r}"))).collect();
    assert_eq!(
        col,
        [
            Value::text("a"),
            Value::text("b"),
            Value::text("c"),
            Value::text("Total Result")
        ]
    );
    assert_eq!(f.value("L5"), Value::Number(20.0));
}

/// M9: no subtotals for the innermost field even when only the Data
/// pseudo-field follows it (AOO `lcl_TestSubTotal`, `dptabsrc.cxx:229-245`).
#[test]
fn pivot_no_innermost_subtotals_before_data_field() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Region"), field(2, "Product")];
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    create(&mut f, p);
    let rows: Vec<(Value, Value, Value, Value)> = (2..=9)
        .map(|r| {
            (
                f.value(&format!("K{r}")),
                f.value(&format!("L{r}")),
                f.value(&format!("M{r}")),
                f.value(&format!("N{r}")),
            )
        })
        .collect();
    let t = Value::text;
    let n = Value::Number;
    assert_eq!(
        rows[0],
        (t("East"), t("Apple"), t("Sum - Revenue"), n(450.0))
    );
    assert_eq!(
        rows[1],
        (Value::Empty, Value::Empty, t("Sum - Units"), n(45.0))
    );
    assert_eq!(
        rows[2],
        (Value::Empty, t("Pear"), t("Sum - Revenue"), n(270.0))
    );
    // L6: a subtotal per data field, captioned with the data field; the
    // Data column stays empty (AOO `FillMemberResults`, `dptabres.cxx:1484-1534`).
    assert_eq!(
        rows[6],
        (
            t("East Sum - Revenue"),
            Value::Empty,
            Value::Empty,
            n(850.0)
        )
    );
    assert_eq!(
        rows[7],
        (t("East Sum - Units"), Value::Empty, Value::Empty, n(77.0))
    );
}

/// M10: GETPIVOTDATA deviations (`dpoutput.cxx:1388-1410,1743-1749`).
#[test]
fn pivot_getpivotdata_follows_aoo() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Region"), field(2, "Product")];
    p.page_fields = vec![field(1, "Rep")];
    create(&mut f, p);
    let is_ref = |v: Value| matches!(v, Value::Error(tally_types::ErrorKind::Ref));
    // (b) An item name is not a data field in the AOO form...
    f.set("A40", "=GETPIVOTDATA(\"East\";K1)");
    assert!(is_ref(f.value("A40")));
    // ...but the Excel form may omit the single data field.
    f.set("A41", "=GETPIVOTDATA(K1;\"East\")");
    assert_eq!(f.value("A41"), Value::Number(850.0));
    f.set("A42", "=GETPIVOTDATA(K1;\"Revenue East Product[Apple]\")");
    assert_eq!(f.value("A42"), Value::Number(450.0));
    // (c) A page field at "- all -" cannot be filtered.
    f.set("A43", "=GETPIVOTDATA(\"Revenue\";K1;\"Rep\";\"Ann\")");
    assert!(is_ref(f.value("A43")));
    f.set("A44", "=GETPIVOTDATA(\"Revenue\";K1;\"Region\";\"West\")");
    assert_eq!(f.value("A44"), Value::Number(1465.0));
    // (d) An error result is #REF!.
    let mut g = Fixture::new();
    load(
        &mut g,
        &[&["K", "V"], &["a", "1"], &["b", "2"], &["b", "3"]],
    );
    let p = simple(&g, "A1:B4", 0, 1, AggFn::StDev);
    create(&mut g, p);
    assert_eq!(g.value("L2"), err());
    g.set("D1", "=GETPIVOTDATA(\"V\";K1;\"K\";\"a\")");
    assert!(is_ref(g.value("D1")));
    g.set("D2", "=GETPIVOTDATA(\"V\";K1;\"K\";\"b\")");
    assert!(matches!(g.value("D2"), Value::Number(x) if (x - 0.5f64.sqrt()).abs() < 1e-12));
}

/// M11 and M12: numeric and date grouping (`dpgroup.cxx`).
#[test]
fn pivot_numeric_and_date_grouping_follow_aoo() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[
            &["N", "V"],
            &["3", "1"],
            &["7", "1"],
            &["12", "1"],
            &["17", "1"],
            &["20", "1"],
            &["30", "1"],
            &["-4", "1"],
        ],
    );
    let mut p = simple(&f, "A1:B8", 0, 1, AggFn::Count);
    p.row_fields[0].sort = PivotSort::Name { ascending: true };
    p.row_fields[0].group = Some(PivotGroupDto::Numeric {
        start: Some(0.0),
        end: Some(20.0),
        step: 10.0,
    });
    create(&mut f, p);
    let got: Vec<(Value, Value)> = (2..=5)
        .map(|r| (f.value(&format!("K{r}")), f.value(&format!("L{r}"))))
        .collect();
    let t = Value::text;
    let n = Value::Number;
    assert_eq!(
        got,
        [
            (t("<0"), n(1.0)),
            (t("0-9"), n(2.0)),
            (t("10-20"), n(3.0)),
            (t(">20"), n(1.0))
        ]
    );
    let mut g = Fixture::new();
    load(
        &mut g,
        &[
            &["D", "V"],
            &["2024-01-20", "2"],
            &["2024-02-03", "4"],
            &["2024-04-11", "8"],
            &["2024-12-31", "16"],
        ],
    );
    let start = match g.value("A3") {
        Value::Number(x) => x,
        other => panic!("{other:?}"),
    };
    let mut p = simple(&g, "A1:B5", 0, 1, AggFn::Sum);
    p.row_fields[0].sort = PivotSort::Name { ascending: true };
    p.row_fields[0].group = Some(PivotGroupDto::Date {
        start: Some(start),
        end: Some(start + 100.0),
        parts: vec![DatePart::Months],
    });
    create(&mut g, p);
    let got: Vec<(Value, Value)> = (2..=5)
        .map(|r| (g.value(&format!("K{r}")), g.value(&format!("L{r}"))))
        .collect();
    assert_eq!(
        got,
        [
            (t("<02/03/2024"), n(2.0)),
            (t("Feb"), n(4.0)),
            (t("Apr"), n(8.0)),
            (t(">05/13/2024"), n(16.0))
        ]
    );
}

/// M13: Refresh asks before overwriting cells outside the old output
/// (`dbdocfun.cxx:1349-1371`).
#[test]
fn pivot_refresh_refuses_to_overwrite_without_flag() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[
            &["K", "V"],
            &["a", "1"],
            &["b", "2"],
            &["a", "3"],
            &["a", "4"],
        ],
    );
    let p = simple(&f, "A1:B5", 0, 1, AggFn::Sum);
    let id = create(&mut f, p);
    f.set("K5", "precious");
    f.set("A4", "c");
    f.set("A5", "d");
    let r = f.apply(vec![Op::RefreshPivot {
        id,
        overwrite: false,
    }]);
    assert_eq!(r.errors[0].code, "target_not_empty");
    assert_eq!(f.value("K5"), Value::text("precious"));
    let r = f.apply(vec![Op::RefreshPivot {
        id,
        overwrite: true,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("K5"), Value::text("d"));
}

/// M14 (engine side): captions and date groups survive an ODS round trip.
#[test]
fn pivot_ods_round_trip_keeps_captions_and_date_groups() {
    let mut f = Fixture::new();
    load(
        &mut f,
        &[&["D", "V"], &["2024-01-05", "1"], &["2024-03-05", "2"]],
    );
    let mut p = simple(&f, "A1:B3", 0, 1, AggFn::Sum);
    p.data_fields[0].caption = Some("Rev total".into());
    p.row_fields[0].group = Some(PivotGroupDto::Date {
        start: Some(45292.0),
        end: Some(45400.0),
        parts: vec![DatePart::Months],
    });
    create(&mut f, p);
    let before = f.svc.pivots(f.doc).unwrap()[0].clone();
    let data = f.svc.document(f.doc).unwrap().wb.to_data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    bytes.set_position(0);
    let back = tally_io_ods::read_ods(bytes, &tally_io_ods::Limits::default()).unwrap();
    let wb = tally_engine::Workbook::from_data(&back);
    let p = &wb.sheets[0].meta.pivots[0];
    let after = tally_engine::pivot::to_api(wb.sheets[0].id, p);
    assert_eq!(after.data_fields, before.data_fields);
    assert_eq!(after.row_fields[0].group, before.row_fields[0].group);
}

/// Security H7: pivots read from files get the API's field cap.
#[test]
fn pivot_from_file_with_too_many_fields_is_refused() {
    let f = fixture();
    let mut data = f.svc.document(f.doc).unwrap().wb.to_data();
    let field = tally_model::PivotFieldData::default();
    data.sheets[0].pivots.push(tally_model::PivotData {
        name: "Big".into(),
        row_fields: vec![field; 2000],
        data_fields: vec![tally_model::PivotDataFieldData::default(); 2000],
        ..tally_model::PivotData::default()
    });
    let wb = tally_engine::Workbook::from_data(&data);
    assert!(wb.sheets[0].meta.pivots.is_empty());
    assert!(
        wb.import_warnings
            .iter()
            .any(|w| w.contains("Big") && w.contains("4000 fields")),
        "{:?}",
        wb.import_warnings
    );
}

/// Security H8: "show items without data" does not build the cross
/// product: the axis trees are bounded by the sheet before the layout.
#[test]
fn pivot_show_empty_cross_product_is_refused_quickly() {
    let mut f = Fixture::new();
    let mut rows: Vec<Vec<String>> = vec![vec!["a".into(), "b".into(), "c".into(), "v".into()]];
    for i in 0..1000u32 {
        rows.push(vec![
            format!("x{i}"),
            format!("y{}", (i * 7) % 1000),
            format!("z{}", (i * 13) % 1000),
            "1".into(),
        ]);
    }
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range("A1:D1001"),
    };
    p.target = CellPos { r: 0, c: 10 };
    p.row_fields = vec![field(0, "a"), field(1, "b"), field(2, "c")];
    p.col_fields.clear();
    for fl in &mut p.row_fields {
        fl.show_empty = true;
    }
    p.data_fields = vec![data(3, AggFn::Sum)];
    let t = std::time::Instant::now();
    let r = f.apply(vec![Op::CreatePivot {
        pivot: p,
        overwrite: true,
    }]);
    assert_eq!(r.errors[0].code, ApiError::UNSUPPORTED);
    assert!(t.elapsed().as_secs() < 20, "{:?}", t.elapsed());
}

/// Security M7/M8: hidden items, manual order and AutoShow are hash or
/// sort based, GETPIVOTDATA is indexed.
#[test]
fn pivot_large_options_and_getpivotdata_are_not_quadratic() {
    let mut f = Fixture::new();
    let n = 20_000u32;
    let mut rows: Vec<Vec<String>> = vec![vec!["k".into(), "v".into()]];
    for i in 0..n {
        rows.push(vec![format!("i{i}"), format!("{}", i % 97)]);
    }
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range(&format!("A1:B{}", n + 1)),
    };
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "k")];
    p.row_fields[0].hidden_items = (0..10_000).map(|i| format!("zz{i}")).collect();
    p.row_fields[0].sort = PivotSort::Manual {
        order: (0..10_000).map(|i| format!("i{i}")).collect(),
    };
    p.row_fields[0].auto_show = Some(AutoShow {
        top: true,
        count: 15_000,
        data_field: 0,
    });
    p.data_fields = vec![data(1, AggFn::Sum)];
    let t = std::time::Instant::now();
    create(&mut f, p);
    let formulas: Vec<Vec<String>> = (0..2000)
        .map(|i| vec![format!("=GETPIVOTDATA(\"v\";$K$1;\"k\";\"i{i}\")")])
        .collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 3,
        rows: formulas,
    }]);
    // i96 (v = 96) is in the top 15,000; i0 (v = 0) is auto-hidden.
    assert_eq!(f.value("D97"), Value::Number(96.0));
    assert!(matches!(
        f.value("D1"),
        Value::Error(tally_types::ErrorKind::Ref)
    ));
    assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
}

// ---------------------------------------------------------------------------
// T17: the "Filter" button and the source filter (AOO `ScPivotFilterDlg`)
// ---------------------------------------------------------------------------

fn cond(
    connector: tally_api::StdConnector,
    col: u32,
    op: tally_api::StdFilterOp,
    value: &str,
) -> tally_api::StdCondition {
    tally_api::StdCondition {
        connector,
        col,
        op,
        value: value.into(),
    }
}

fn by_region(f: &Fixture) -> PivotDto {
    let mut p = pivot(f);
    p.col_fields.clear();
    p.show_filter_button = true;
    p
}

#[test]
fn pivot_filter_button_cell_and_source_filter() {
    use tally_api::{PivotSourceFilterDto, StdConnector, StdFilterOp};
    let mut f = fixture();
    let p = by_region(&f);
    let id = create(&mut f, p);
    // The button above the table, a blank row, then the table.
    assert_eq!(f.value("H1"), Value::text("Filter"));
    assert_eq!(f.value("H2"), Value::Empty);
    assert_eq!(f.value("H4"), Value::text("East"));
    assert_eq!(f.value("H5"), Value::text("West"));
    assert_eq!(f.value("I6"), Value::Number(2315.0));
    let hit = f.svc.pivot_at(f.doc, f.sheet, 0, 7).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::FilterButton);
    assert!(f.svc.pivots(f.doc).unwrap()[0].source_filter.is_none());

    // Region = east (not case-sensitive): only the East rows are read.
    let mut p = f.svc.pivots(f.doc).unwrap()[0].clone();
    p.source_filter = Some(PivotSourceFilterDto {
        conditions: vec![cond(StdConnector::And, 0, StdFilterOp::Eq, "east")],
        ..PivotSourceFilterDto::default()
    });
    let r = f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p.clone(),
        overwrite: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("H4"), Value::text("East"));
    assert_eq!(f.value("I4"), Value::Number(850.0));
    assert_eq!(f.value("H5"), Value::text("Total Result"));
    assert_eq!(f.value("I5"), Value::Number(850.0));
    assert_eq!(f.value("H6"), Value::Empty);
    let back = f.svc.pivots(f.doc).unwrap()[0].source_filter.clone();
    assert_eq!(back, p.source_filter);
    // GETPIVOTDATA sees the filtered result.
    f.set("A30", "=GETPIVOTDATA(\"Revenue\",H3)");
    assert_eq!(f.value("A30"), Value::Number(850.0));

    // Undo puts the unfiltered table back; redo filters again.
    f.svc.undo(f.doc).unwrap(); // A30
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("H5"), Value::text("West"));
    assert!(f.svc.pivots(f.doc).unwrap()[0].source_filter.is_none());
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.value("H5"), Value::text("Total Result"));

    // Refresh re-applies the filter to the changed source.
    f.set("F2", "1000");
    let r = f.apply(vec![Op::RefreshPivot {
        id,
        overwrite: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("I4"), Value::Number(1750.0));
    f.set("A2", "West");
    f.apply(vec![Op::RefreshPivot {
        id,
        overwrite: false,
    }]);
    assert_eq!(f.value("I4"), Value::Number(750.0));
}

#[test]
fn pivot_source_filter_and_or_regex_case() {
    use tally_api::{PivotSourceFilterDto, StdConnector, StdFilterOp};
    let mut f = fixture();
    let mut p = by_region(&f);
    // Region = West AND Quarter = Q1.
    p.source_filter = Some(PivotSourceFilterDto {
        conditions: vec![
            cond(StdConnector::And, 0, StdFilterOp::Eq, "West"),
            cond(StdConnector::And, 3, StdFilterOp::Eq, "Q1"),
        ],
        ..PivotSourceFilterDto::default()
    });
    let id = create(&mut f, p.clone());
    let rows = sales_rows();
    let sum = |keep: &dyn Fn(&[String]) -> bool| -> f64 {
        rows[1..]
            .iter()
            .filter(|r| keep(r))
            .map(|r| r[5].parse::<f64>().unwrap())
            .sum()
    };
    let west_q1 = sum(&|r| r[0] == "West" && r[3] == "Q1");
    assert_eq!(f.value("H4"), Value::text("West"));
    assert_eq!(f.value("I4"), Value::Number(west_q1));
    // AND binds tighter than OR: East OR (West AND Revenue > 200).
    p.source_filter = Some(PivotSourceFilterDto {
        conditions: vec![
            cond(StdConnector::And, 0, StdFilterOp::Eq, "East"),
            cond(StdConnector::Or, 0, StdFilterOp::Eq, "West"),
            cond(StdConnector::And, 5, StdFilterOp::Gt, "200"),
        ],
        ..PivotSourceFilterDto::default()
    });
    f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p.clone(),
        overwrite: false,
    }]);
    let west_big = sum(&|r| r[0] == "West" && r[5].parse::<f64>().unwrap() > 200.0);
    assert_eq!(f.value("I4"), Value::Number(850.0));
    assert_eq!(f.value("I5"), Value::Number(west_big));
    // A case-sensitive regular expression keeps East only.
    p.source_filter = Some(PivotSourceFilterDto {
        conditions: vec![cond(StdConnector::And, 0, StdFilterOp::Eq, "^[E].*")],
        case_sensitive: true,
        regex: true,
        no_duplicates: true,
    });
    let r = f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p.clone(),
        overwrite: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("H4"), Value::text("East"));
    assert_eq!(f.value("H5"), Value::text("Total Result"));
    // "No duplications" is kept but not applied (AOO): all East rows count.
    assert_eq!(f.value("I4"), Value::Number(850.0));
    assert!(
        f.svc.pivots(f.doc).unwrap()[0]
            .source_filter
            .as_ref()
            .unwrap()
            .no_duplicates
    );
    // A condition outside the source is refused.
    p.source_filter = Some(PivotSourceFilterDto {
        conditions: vec![cond(StdConnector::And, 6, StdFilterOp::Eq, "x")],
        ..PivotSourceFilterDto::default()
    });
    let r = f.apply(vec![Op::UpdatePivot {
        id,
        pivot: p,
        overwrite: false,
    }]);
    assert_eq!(r.errors.len(), 1);
}

/// A source filter read from a file is cut to the API's limits (8
/// conditions, 4,096-byte values) so the pivot can be edited and sent back.
#[test]
fn pivot_source_filter_from_file_is_bounded() {
    let f = fixture();
    let mut data = f.svc.document(f.doc).unwrap().wb.to_data();
    let cond = |value: String| tally_model::StdConditionData {
        value,
        ..tally_model::StdConditionData::default()
    };
    let mut conditions = vec![cond("x".repeat(5000))];
    conditions.extend((0..20).map(|i| cond(format!("v{i}"))));
    data.sheets[0].pivots.push(tally_model::PivotData {
        name: "Filtered".into(),
        data_fields: vec![tally_model::PivotDataFieldData::default()],
        source_filter: Some(tally_model::PivotSourceFilterData {
            conditions,
            ..tally_model::PivotSourceFilterData::default()
        }),
        ..tally_model::PivotData::default()
    });
    let wb = tally_engine::Workbook::from_data(&data);
    let p = &wb.sheets[0].meta.pivots[0];
    let f = p.data.source_filter.as_ref().unwrap();
    assert_eq!(f.conditions.len(), 8);
    assert_eq!(f.conditions[0].value, "v0");
}

// ---------------------------------------------------------------------------
// Layout parity (review 4, L6 and L7). The expected tables are derived
// from the AOO 4.1 source (`ScDPResultMember::FillMemberResults` /
// `FillDataResults` / `GetSize` / `IsSubTotalInTitle`, `dptabres.cxx`;
// `ScDPResultData::GetMeasureString`; `ScDPSource::CreateRes_Impl`,
// `dptabsrc.cxx`), not from a run of AOO.
// ---------------------------------------------------------------------------

/// The cells of columns `cols` on rows `r0..=r1` as `a | b | c` lines.
fn block(f: &Fixture, cols: &[&str], r0: u32, r1: u32) -> Vec<String> {
    (r0..=r1)
        .map(|r| {
            cols.iter()
                .map(|c| match f.value(&format!("{c}{r}")) {
                    Value::Empty => String::new(),
                    Value::Number(n) => format!("{n}"),
                    Value::Text(t) => t.to_string(),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect()
}

/// Region × Product at K1, Sum - Revenue, no column fields.
fn region_product(f: &Fixture) -> PivotDto {
    let mut p = pivot(f);
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "Region"), field(2, "Product")];
    p
}

/// L6: several data fields in rows: subtotals and grand totals per data
/// field, "<item> <data field>" / "Total <data field>", Data column empty.
#[test]
fn pivot_layout_captions_with_several_data_fields() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M", "N"], 2, 19);
    let want = [
        "East | Apple | Sum - Revenue | 450",
        " |  | Sum - Units | 45",
        " | Pear | Sum - Revenue | 270",
        " |  | Sum - Units | 22",
        " | Plum | Sum - Revenue | 130",
        " |  | Sum - Units | 10",
        "East Sum - Revenue |  |  | 850",
        "East Sum - Units |  |  | 77",
        "West | Apple | Sum - Revenue | 850",
        " |  | Sum - Units | 85",
        " | Pear | Sum - Revenue | 455",
        " |  | Sum - Units | 38",
        " | Plum | Sum - Revenue | 160",
        " |  | Sum - Units | 14",
        "West Sum - Revenue |  |  | 1465",
        "West Sum - Units |  |  | 137",
        "Total Sum - Revenue |  |  | 2315",
        "Total Sum - Units |  |  | 214",
    ];
    assert_eq!(got, want);
    // GETPIVOTDATA still finds the subtotal and grand total lines.
    f.set(
        "A40",
        "=GETPIVOTDATA(\"Sum - Units\";K1;\"Region\";\"West\")",
    );
    assert_eq!(f.value("A40"), Value::Number(137.0));
    f.set("A41", "=GETPIVOTDATA(\"Sum - Revenue\";K1)");
    assert_eq!(f.value("A41"), Value::Number(2315.0));
    // Data fields in columns: the row subtotals measure "any" data field.
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    p.data_layout = DataLayout::Column;
    create(&mut f, p);
    assert_eq!(f.value("M1"), Value::text("Data"));
    assert_eq!(f.value("M2"), Value::text("Sum - Revenue"));
    assert_eq!(f.value("N2"), Value::text("Sum - Units"));
    assert_eq!(
        block(&f, &["K", "L", "M", "N"], 6, 6),
        ["East Result |  | 850 | 77"]
    );
    assert_eq!(
        block(&f, &["K", "L", "M", "N"], 11, 11),
        ["Total Result |  | 2315 | 214"]
    );
}

/// L6: forced subtotal functions name the data field when there is one
/// data field or the Data field is on the same axis ("East Max -
/// Revenue"), only the function otherwise ("East Max").
#[test]
fn pivot_layout_captions_of_custom_subtotals() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.row_fields[0].subtotals = PivotSubtotals::Custom {
        fns: vec![AggFn::Max, AggFn::Min],
    };
    create(&mut f, p);
    assert_eq!(
        block(&f, &["K", "L", "M"], 5, 6),
        ["East Max - Revenue |  | 150", "East Min - Revenue |  | 10"]
    );
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[0].subtotals = PivotSubtotals::Custom {
        fns: vec![AggFn::Max],
    };
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    p.data_layout = DataLayout::Column;
    create(&mut f, p);
    assert_eq!(
        block(&f, &["K", "L", "M", "N"], 6, 6),
        ["East Max |  | 150 | 15"]
    );
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[0].subtotals = PivotSubtotals::Custom {
        fns: vec![AggFn::Max],
    };
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    create(&mut f, p);
    assert_eq!(
        block(&f, &["K", "L", "M", "N"], 8, 9),
        [
            "East Max - Revenue |  |  | 150",
            "East Max - Units |  |  | 15"
        ]
    );
}

/// L7: outline layout, subtotals at the top: with a single subtotal row
/// the subtotal is in the item's title row (captioned with the item).
#[test]
fn pivot_layout_outline_subtotals_at_top() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.row_fields[0].layout = PivotLayout::OutlineTop;
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M"], 1, 10);
    let want = [
        "Region | Product | Sum - Revenue",
        "East |  | 850",
        " | Apple | 450",
        " | Pear | 270",
        " | Plum | 130",
        "West |  | 1465",
        " | Apple | 850",
        " | Pear | 455",
        " | Plum | 160",
        "Total Result |  | 2315",
    ];
    assert_eq!(got, want);
    assert_eq!(
        f.svc.pivots(f.doc).unwrap()[0].output,
        Some(range("K1:M10"))
    );
    // The title row is an item row. GETPIVOTDATA reads its subtotal: a
    // deliberate deviation (parity-deviations T5). AOO 4.1 gives #REF!
    // here: the title row has no SUBTOTAL flag (`dptabres.cxx:1465`), so
    // `lcl_FilterInclude` (`dpoutput.cxx:1517-1630`) keeps the title row
    // and its three child rows and `GetPivotData` fails on more than one
    // row (`dpoutput.cxx:1811`).
    let hit = f.svc.pivot_at(f.doc, f.sheet, 1, 10).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::RowItem);
    assert_eq!(hit.item.as_deref(), Some("East"));
    f.set(
        "A40",
        "=GETPIVOTDATA(\"Sum - Revenue\";K1;\"Region\";\"East\")",
    );
    assert_eq!(f.value("A40"), Value::Number(850.0));
    f.set(
        "A41",
        "=GETPIVOTDATA(\"Sum - Revenue\";K1;\"Region\";\"West\";\"Product\";\"Pear\")",
    );
    assert_eq!(f.value("A41"), Value::Number(455.0));
    // Two subtotal rows (two data fields in rows) do not fit the title
    // row: they follow the children, and the title row has no values.
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[0].layout = PivotLayout::OutlineTop;
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M", "N"], 2, 11);
    let want = [
        "East |  |  | ",
        " | Apple | Sum - Revenue | 450",
        " |  | Sum - Units | 45",
        " | Pear | Sum - Revenue | 270",
        " |  | Sum - Units | 22",
        " | Plum | Sum - Revenue | 130",
        " |  | Sum - Units | 10",
        "East Sum - Revenue |  |  | 850",
        "East Sum - Units |  |  | 77",
        "West |  |  | ",
    ];
    assert_eq!(got, want);
    f.set(
        "A42",
        "=GETPIVOTDATA(\"Sum - Units\";K1;\"Region\";\"East\")",
    );
    assert_eq!(f.value("A42"), Value::Number(77.0));
    // The title row is not a data cell for drill-down.
    let id = f.svc.pivots(f.doc).unwrap()[0].id;
    let r = f.apply(vec![Op::PivotDrillDown { id, r: 1, c: 13 }]);
    assert!(!r.errors.is_empty());
}

/// L7: outline layout with subtotals at the bottom and an empty line
/// after each item.
#[test]
fn pivot_layout_outline_bottom_with_empty_lines() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.row_fields[0].layout = PivotLayout::OutlineBottom;
    p.row_fields[0].empty_line_after = true;
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M"], 2, 14);
    let want = [
        "East |  | ",
        " | Apple | 450",
        " | Pear | 270",
        " | Plum | 130",
        "East Result |  | 850",
        " |  | ",
        "West |  | ",
        " | Apple | 850",
        " | Pear | 455",
        " | Plum | 160",
        "West Result |  | 1465",
        " |  | ",
        "Total Result |  | 2315",
    ];
    assert_eq!(got, want);
    // Tally returns the subtotal: a deliberate deviation (T5). AOO 4.1
    // returns 0: the empty line is a CONTINUE entry that inherits the
    // subtotal entry (`lcl_FilterInclude`, `dpoutput.cxx:1523-1527`),
    // `lcl_StripSubTotals` then drops the real subtotal row and only the
    // empty row is left.
    f.set(
        "A40",
        "=GETPIVOTDATA(\"Sum - Revenue\";K1;\"Region\";\"West\")",
    );
    assert_eq!(f.value("A40"), Value::Number(1465.0));
    // Outline without subtotals: just the title row.
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[0].layout = PivotLayout::OutlineTop;
    p.row_fields[0].subtotals = PivotSubtotals::None;
    create(&mut f, p);
    assert_eq!(
        block(&f, &["K", "L", "M"], 2, 6),
        [
            "East |  | ",
            " | Apple | 450",
            " | Pear | 270",
            " | Plum | 130",
            "West |  | "
        ]
    );
}

/// L7: the layout options do nothing on the innermost row level or on
/// column fields (AOO `dptabsrc.cxx:690,935`); with the Data field
/// innermost on the rows the last real row field does get them.
#[test]
fn pivot_layout_options_only_on_outer_row_levels() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.row_fields[1].layout = PivotLayout::OutlineTop;
    p.row_fields[1].empty_line_after = true;
    p.col_fields = vec![field(3, "Quarter")];
    p.col_fields[0].layout = PivotLayout::OutlineBottom;
    p.col_fields[0].empty_line_after = true;
    create(&mut f, p);
    // Region | Product | Q1..Q4 | Total Result: no title rows, blank
    // rows or blank columns.
    assert_eq!(
        f.svc.pivots(f.doc).unwrap()[0].output,
        Some(range("K1:Q11"))
    );
    assert_eq!(
        block(&f, &["K", "L", "M", "Q"], 3, 6),
        [
            "East | Apple | 100 | 450",
            " | Pear | 60 | 270",
            " | Plum | 30 | 130",
            "East Result |  | 190 | 850"
        ]
    );
    // Data innermost on the rows: Product has a title row and an empty
    // line; Product has no subtotals (only the Data field follows it).
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[1].layout = PivotLayout::OutlineTop;
    p.row_fields[1].empty_line_after = true;
    p.data_fields = vec![data(5, AggFn::Sum), data(4, AggFn::Sum)];
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M", "N"], 2, 14);
    let want = [
        "East | Apple |  | ",
        " |  | Sum - Revenue | 450",
        " |  | Sum - Units | 45",
        " |  |  | ",
        " | Pear |  | ",
        " |  | Sum - Revenue | 270",
        " |  | Sum - Units | 22",
        " |  |  | ",
        " | Plum |  | ",
        " |  | Sum - Revenue | 130",
        " |  | Sum - Units | 10",
        " |  |  | ",
        "East Sum - Revenue |  |  | 850",
    ];
    assert_eq!(got, want);
    f.set(
        "A40",
        "=GETPIVOTDATA(\"Sum - Units\";K1;\"Region\";\"East\";\"Product\";\"Pear\")",
    );
    assert_eq!(f.value("A40"), Value::Number(22.0));
}

/// The lines of a large outline layout with empty lines stay linear and
/// within the output budget.
#[test]
fn pivot_layout_outline_lines_are_bounded() {
    let mut f = Fixture::new();
    let mut rows: Vec<Vec<String>> = vec![vec!["A".into(), "B".into(), "V".into()]];
    for i in 0..20_000 {
        rows.push(vec![format!("a{}", i % 2000), format!("b{i}"), "1".into()]);
    }
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
    let mut p = pivot(&f);
    p.source = PivotSource::Range {
        sheet: f.sheet,
        range: range("A1:C20001"),
    };
    p.target = CellPos { r: 0, c: 5 };
    p.col_fields.clear();
    p.row_fields = vec![field(0, "A"), field(1, "B")];
    p.row_fields[0].layout = PivotLayout::OutlineBottom;
    p.row_fields[0].empty_line_after = true;
    p.data_fields = vec![data(2, AggFn::Sum)];
    create(&mut f, p);
    // Header, 20,000 items, 2,000 × (title + subtotal + empty), total.
    let out = f.svc.pivots(f.doc).unwrap()[0].output.unwrap();
    assert_eq!(out.r1 - out.r0 + 1, 1 + 20_000 + 3 * 2_000 + 1);
}

/// Output captions use AOO's short function names (`nFuncStrIds`,
/// `dptabres.cxx:71-85`: Count for Count and Count (numbers only), StDev
/// and Var for both variants); GETPIVOTDATA accepts the caption, the
/// source name, AOO's given name (`lcl_GetDataFieldName`) and the long
/// names Tally wrote before.
#[test]
fn pivot_layout_short_function_names_in_captions() {
    let mut f = fixture();
    let mut p = region_product(&f);
    p.data_fields = vec![data(5, AggFn::StDevP), data(5, AggFn::CountNums)];
    create(&mut f, p);
    assert_eq!(
        block(&f, &["K", "L", "M"], 2, 3),
        ["East | Apple | StDev - Revenue", " |  | Count - Revenue"]
    );
    assert_eq!(
        block(&f, &["K", "L", "M"], 8, 9),
        ["East StDev - Revenue |  | ", "East Count - Revenue |  | "]
    );
    assert_eq!(
        block(&f, &["K", "L", "M"], 18, 19),
        ["Total StDev - Revenue |  | ", "Total Count - Revenue |  | "]
    );
    let east_sd = match f.value("N8") {
        Value::Number(n) => n,
        other => panic!("{other:?}"),
    };
    for name in [
        "StDev - Revenue",
        "StDevP (Population) - Revenue",
        "Revenue",
    ] {
        f.set(
            "A40",
            &format!("=GETPIVOTDATA(\"{name}\";K1;\"Region\";\"East\")"),
        );
        assert_eq!(f.value("A40"), Value::Number(east_sd), "{name}");
    }
    f.set(
        "A41",
        "=GETPIVOTDATA(\"Count - Revenue\";K1;\"Region\";\"East\")",
    );
    assert_eq!(f.value("A41"), Value::Number(12.0));
    f.set(
        "A42",
        "=GETPIVOTDATA(\"Count (Numbers only) - Revenue\";K1;\"Region\";\"East\")",
    );
    assert_eq!(f.value("A42"), Value::Number(12.0));
    // A forced subtotal function with one data field.
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.row_fields[0].subtotals = PivotSubtotals::Custom {
        fns: vec![AggFn::VarP],
    };
    create(&mut f, p);
    assert_eq!(f.value("K5"), Value::text("East Var - Revenue"));
    // A custom caption is still matched as well as the given name.
    clear_pivots(&mut f);
    let mut p = region_product(&f);
    p.data_fields[0].caption = Some("Takings".into());
    create(&mut f, p);
    assert_eq!(f.value("M1"), Value::text("Takings"));
    f.set("A43", "=GETPIVOTDATA(\"Takings\";K1;\"Region\";\"East\")");
    assert_eq!(f.value("A43"), Value::Number(850.0));
    f.set(
        "A44",
        "=GETPIVOTDATA(\"Sum - Revenue\";K1;\"Region\";\"East\")",
    );
    assert_eq!(f.value("A44"), Value::Number(850.0));
}

/// An item whose first line is a subtotal (its children have no data
/// under "show items without data") is captioned on that line, with any
/// outer item not written yet: AOO writes every member's caption on its
/// first row (`ScDPResultMember::FillMemberResults`, HASMEMBER at `rPos`,
/// `dptabres.cxx:1427-1434`), and the subtotal caption replaces only the
/// member's own cell.
#[test]
fn pivot_layout_subtotal_first_line_labels_outer_items() {
    let mut f = fixture();
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 10 };
    p.col_fields.clear();
    p.row_fields = vec![
        field(0, "Region"),
        field(1, "Rep"),
        field(2, "Product"),
        field(3, "Quarter"),
    ];
    p.row_fields[1].show_empty = true;
    p.row_fields[2].show_empty = true;
    create(&mut f, p);
    let got = block(&f, &["K", "L", "M", "N", "O"], 13, 40);
    let want = [
        " | Ann Result |  |  | 720",
        " | Bob | Apple Result |  | ",
        " |  | Pear Result |  | ",
        " |  | Plum | Q1 | 30",
        " |  |  | Q2 | 40",
        " |  |  | Q3 | 10",
        " |  |  | Q4 | 50",
        " |  | Plum Result |  | 130",
        " | Bob Result |  |  | 130",
        " | Cid | Apple Result |  | ",
        " |  | Pear Result |  | ",
        " |  | Plum Result |  | ",
        " | Cid Result |  |  | ",
        " | Dan | Apple Result |  | ",
        " |  | Pear Result |  | ",
        " |  | Plum Result |  | ",
        " | Dan Result |  |  | ",
        "East Result |  |  |  | 850",
        "West | Ann | Apple Result |  | ",
        " |  | Pear Result |  | ",
        " |  | Plum Result |  | ",
        " | Ann Result |  |  | ",
        " | Bob | Apple Result |  | ",
        " |  | Pear Result |  | ",
        " |  | Plum Result |  | ",
        " | Bob Result |  |  | ",
        " | Cid | Apple | Q1 | 200",
        " |  |  | Q2 | 220",
    ];
    assert_eq!(got, want);
    // The item cells carry their role and item.
    let hit = f.svc.pivot_at(f.doc, f.sheet, 30, 10).unwrap().unwrap();
    assert_eq!(hit.role, PivotCellRole::RowItem);
    assert_eq!(hit.item.as_deref(), Some("West"));
    // Columns: Quarter × Product with Product shown without data, Rep
    // filtered to Bob (only Plum has data): the first column of Q1 is the
    // "Apple Result" subtotal and carries the Q1 caption.
    clear_pivots(&mut f);
    let mut p = pivot(&f);
    p.target = CellPos { r: 0, c: 10 };
    p.row_fields = vec![field(0, "Region")];
    p.col_fields = vec![field(3, "Quarter"), field(2, "Product"), field(4, "Units")];
    p.col_fields[1].show_empty = true;
    let mut rep = field(1, "Rep");
    rep.page_item = Some("Bob".into());
    p.page_fields = vec![rep];
    create(&mut f, p);
    // Rows: page field, blank, header (corner + field names), Quarter,
    // Product, Units.
    assert_eq!(f.value("L4"), Value::text("Q1"));
    assert_eq!(f.value("L5"), Value::text("Apple Result"));
    assert_eq!(f.value("M5"), Value::text("Pear Result"));
    assert_eq!(f.value("N5"), Value::text("Plum"));
    assert_eq!(f.value("N6"), Value::Number(2.0));
}
