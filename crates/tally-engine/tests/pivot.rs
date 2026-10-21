//! DataPilot (wave 5, W5-2): layout, aggregation, show-as, filters,
//! refresh, undo, structure, grouping, drill-down and GETPIVOTDATA on the
//! AOO help dataset (`tests/fixtures/pivot/sales.csv`).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    AggFn, ApiError, AutoShow, CellPos, DataLayout, DatePart, DocumentService, Op, PivotCellRole,
    PivotDataFieldDto, PivotDto, PivotFieldDto, PivotGroupDto, PivotLayout, PivotSort, PivotSource,
    PivotSubtotals, ShowAs, ShowAsKind,
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
    assert_eq!(f.value("I3"), Value::text("Count (Numbers only) - Revenue"));
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
    let r = f.apply(vec![Op::RefreshPivot { id }]);
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
    let r = f.apply(vec![Op::RefreshPivot { id: p.id }]);
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
    assert_eq!(f.value("H2"), Value::text("0-4"));
    assert_eq!(f.value("I2"), Value::Number(1.0));
    assert_eq!(f.value("H3"), Value::text("5-9"));
    assert_eq!(f.value("I3"), Value::Number(2.0));
    assert_eq!(f.value("H4"), Value::text("10-14"));
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
