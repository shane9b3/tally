//! Data tools (wave 5, W5-2): standard/advanced filter, subtotals,
//! consolidate, text to columns, remove duplicates, delete contents,
//! fill series, names from labels, database ranges, AutoInput,
//! Navigator, statistics, sort options, custom sort lists.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    AggFn, CsvColumnType, CsvImportOptionsDto, DateUnit, DeleteContentsFlags, Direction,
    DocumentService, NameLabelFlags, Op, SeriesKind, SheetCell, SheetRange, SortKey, SortOptions,
    StdCondition, StdConnector, StdFilterOp, StdFilterOptions, SubtotalGroup, SubtotalOptions,
};
use tally_types::Value;

fn table(f: &mut Fixture) {
    let rows: Vec<Vec<String>> = [
        ["Name", "Dept", "Pay"],
        ["Ann", "Sales", "10"],
        ["Bob", "Ops", "30"],
        ["Cid", "Sales", "20"],
        ["Dan", "Ops", "40"],
        ["Eve", "HR", "50"],
        ["Ann", "Sales", "10"],
    ]
    .iter()
    .map(|r| r.iter().map(|s| s.to_string()).collect())
    .collect();
    f.apply(vec![Op::SetRangeInputs {
        sheet: f.sheet,
        r: 0,
        c: 0,
        rows,
    }]);
}

fn hidden_rows(f: &Fixture) -> Vec<(u32, u32)> {
    f.svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .row_runs
        .iter()
        .filter(|r| r.hidden && r.filtered == Some(true))
        .map(|r| (r.start, r.end))
        .collect()
}

fn cond(connector: StdConnector, col: u32, op: StdFilterOp, value: &str) -> StdCondition {
    StdCondition {
        connector,
        col,
        op,
        value: value.into(),
    }
}

#[test]
fn std_filter_or_conditions() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![
            cond(StdConnector::And, 1, StdFilterOp::Eq, "Sales"),
            cond(StdConnector::Or, 2, StdFilterOp::Gt, "35"),
        ],
        options: StdFilterOptions {
            has_header: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // Sales rows (2, 4, 7) and Pay > 35 (5, 6) stay: rows 3 hidden.
    assert_eq!(hidden_rows(&f), vec![(2, 2)]);
    f.set("E1", "=SUBTOTAL(9,C2:C7)");
    assert_eq!(f.value("E1"), Value::Number(130.0));
    // No duplicates + begins-with + case.
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![cond(StdConnector::And, 0, StdFilterOp::BeginsWith, "a")],
        options: StdFilterOptions {
            has_header: true,
            no_duplicates: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(hidden_rows(&f), vec![(2, 6)]);
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![cond(StdConnector::And, 0, StdFilterOp::BeginsWith, "a")],
        options: StdFilterOptions {
            has_header: true,
            case_sensitive: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(hidden_rows(&f), vec![(1, 6)]);
    // Largest 2 and a regex.
    f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![cond(StdConnector::And, 2, StdFilterOp::Largest, "2")],
        options: StdFilterOptions {
            has_header: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert_eq!(hidden_rows(&f), vec![(1, 3), (6, 6)]);
    f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![cond(StdConnector::And, 0, StdFilterOp::Eq, "^[BC].*")],
        options: StdFilterOptions {
            has_header: true,
            regex: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert_eq!(hidden_rows(&f), vec![(1, 1), (4, 6)]);
    let r = f.apply(vec![Op::ResetFilter { sheet: f.sheet }]);
    assert!(r.errors.is_empty());
    assert!(hidden_rows(&f).is_empty());
    f.svc.undo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), vec![(1, 1), (4, 6)]);
}

#[test]
fn std_filter_copy_to() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::StandardFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        conditions: vec![cond(StdConnector::And, 1, StdFilterOp::Eq, "Ops")],
        options: StdFilterOptions {
            has_header: true,
            copy_to: Some(SheetCell {
                sheet: f.sheet,
                r: 0,
                c: 5,
            }),
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(hidden_rows(&f).is_empty());
    assert_eq!(f.value("F1"), Value::text("Name"));
    assert_eq!(f.value("F2"), Value::text("Bob"));
    assert_eq!(f.value("H3"), Value::Number(40.0));
    assert_eq!(f.value("F4"), Value::Empty);
}

#[test]
fn advanced_filter_criteria_rows_or() {
    let mut f = Fixture::new();
    table(&mut f);
    // Criteria: Dept = Sales OR Pay > 35.
    f.set("E1", "Dept");
    f.set("F1", "Pay");
    f.set("E2", "Sales");
    f.set("F3", ">35");
    let r = f.apply(vec![Op::AdvancedFilter {
        sheet: f.sheet,
        range: range("A1:C7"),
        criteria: SheetRange {
            sheet: f.sheet,
            range: range("E1:F3"),
        },
        options: StdFilterOptions {
            has_header: true,
            ..StdFilterOptions::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(hidden_rows(&f), vec![(2, 2)]);
    let ranges = f.svc.database_ranges(f.doc).unwrap();
    assert!(ranges.is_empty());
    let data = f.data();
    assert!(data.sheets[0]
        .standard_filter
        .as_ref()
        .unwrap()
        .criteria
        .is_some());
}

#[test]
fn subtotals_insert_and_remove() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::Subtotals {
        sheet: f.sheet,
        range: range("A1:C7"),
        groups: vec![SubtotalGroup {
            group_by: 1,
            columns: vec![2],
            function: AggFn::Sum,
        }],
        options: SubtotalOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // Sorted by Dept: HR, Ops, Ops, Sales ×3; subtotal rows after each.
    assert_eq!(f.value("B2"), Value::text("HR"));
    assert_eq!(f.value("B3"), Value::text("HR Sum"));
    assert_eq!(f.value("C3"), Value::Number(50.0));
    assert_eq!(f.value("B6"), Value::text("Ops Sum"));
    assert_eq!(f.value("C6"), Value::Number(70.0));
    assert_eq!(f.value("B10"), Value::text("Sales Sum"));
    assert_eq!(f.value("C10"), Value::Number(40.0));
    assert_eq!(f.value("B11"), Value::text("Grand Total"));
    assert_eq!(f.value("C11"), Value::Number(160.0));
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(o.rows.len(), 3);
    assert_eq!((o.rows[1].start, o.rows[1].end), (3, 4));
    let r = f.apply(vec![Op::RemoveSubtotals {
        sheet: f.sheet,
        range: range("A1:C11"),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("B3"), Value::text("Ops"));
    assert_eq!(f.value("A8"), Value::Empty);
    assert!(f
        .svc
        .sheet_layout(f.doc, f.sheet)
        .unwrap()
        .outline
        .is_none());
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("B11"), Value::text("Grand Total"));
}

#[test]
fn subtotals_outline_levels_two_groups() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::Subtotals {
        sheet: f.sheet,
        range: range("A1:C7"),
        groups: vec![
            SubtotalGroup {
                group_by: 1,
                columns: vec![2],
                function: AggFn::Sum,
            },
            SubtotalGroup {
                group_by: 0,
                columns: vec![2],
                function: AggFn::Count,
            },
        ],
        options: SubtotalOptions::default(),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let o = f.svc.sheet_layout(f.doc, f.sheet).unwrap().outline.unwrap();
    assert_eq!(o.row_levels, 2);
    assert_eq!(f.value("B2"), Value::text("HR"));
    assert_eq!(f.value("A3"), Value::text("Eve Count"));
    assert_eq!(f.value("B4"), Value::text("HR Sum"));
    assert_eq!(f.value("B16"), Value::text("Grand Total"));
}

#[test]
fn consolidate_by_labels() {
    let mut f = Fixture::new();
    let rows = |f: &mut Fixture, c: u32, data: &[[&str; 2]]| {
        let rows: Vec<Vec<String>> = data
            .iter()
            .map(|r| r.iter().map(|s| s.to_string()).collect())
            .collect();
        f.apply(vec![Op::SetRangeInputs {
            sheet: f.sheet,
            r: 0,
            c,
            rows,
        }]);
    };
    rows(&mut f, 0, &[["", "Q1"], ["Apple", "10"], ["Pear", "5"]]);
    rows(&mut f, 3, &[["", "Q1"], ["Pear", "7"], ["Plum", "1"]]);
    let r = f.apply(vec![Op::Consolidate {
        function: AggFn::Sum,
        sources: vec![
            SheetRange {
                sheet: f.sheet,
                range: range("A1:B3"),
            },
            SheetRange {
                sheet: f.sheet,
                range: range("D1:E3"),
            },
        ],
        dest: SheetCell {
            sheet: f.sheet,
            r: 5,
            c: 0,
        },
        by_row_labels: true,
        by_col_labels: true,
        link: false,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("B6"), Value::text("Q1"));
    assert_eq!(f.value("A7"), Value::text("Apple"));
    assert_eq!(f.value("B7"), Value::Number(10.0));
    assert_eq!(f.value("A8"), Value::text("Pear"));
    assert_eq!(f.value("B8"), Value::Number(12.0));
    assert_eq!(f.value("A9"), Value::text("Plum"));
    // By position with links.
    let r = f.apply(vec![Op::Consolidate {
        function: AggFn::Max,
        sources: vec![
            SheetRange {
                sheet: f.sheet,
                range: range("B2:B3"),
            },
            SheetRange {
                sheet: f.sheet,
                range: range("E2:E3"),
            },
        ],
        dest: SheetCell {
            sheet: f.sheet,
            r: 12,
            c: 0,
        },
        by_row_labels: false,
        by_col_labels: false,
        link: true,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A13"), Value::Number(10.0));
    assert_eq!(f.value("A14"), Value::Number(5.0));
    f.set("E3", "99");
    assert_eq!(f.value("A14"), Value::Number(99.0));
}

#[test]
fn text_to_columns_semicolon() {
    let mut f = Fixture::new();
    f.set("A1", "a;1;2024-03-05");
    f.set("A2", "\"b;c\";2;x");
    f.set("B1", "gone");
    let r = f.apply(vec![Op::TextToColumns {
        sheet: f.sheet,
        range: range("A1:A2"),
        options: CsvImportOptionsDto {
            delimiters: ";".into(),
            column_types: vec![
                CsvColumnType::Standard,
                CsvColumnType::Text,
                CsvColumnType::DateYmd,
            ],
            ..CsvImportOptionsDto::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A1"), Value::text("a"));
    assert_eq!(f.value("B1"), Value::text("1"));
    assert_eq!(f.text("C1"), "2024-03-05");
    assert!(matches!(f.value("C1"), Value::Number(_)));
    assert_eq!(f.value("A2"), Value::text("b;c"));
    assert_eq!(f.value("C2"), Value::text("x"));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("B1"), Value::text("gone"));
}

#[test]
fn remove_duplicates_keeps_first() {
    let mut f = Fixture::new();
    table(&mut f);
    f.set("E7", "side");
    let r = f.apply(vec![Op::RemoveDuplicates {
        sheet: f.sheet,
        range: range("A1:C7"),
        cols: vec![0],
        has_header: true,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.replaced, Some(1));
    assert_eq!(f.value("A6"), Value::text("Eve"));
    assert_eq!(f.value("A7"), Value::Empty);
    assert_eq!(
        f.value("E7"),
        Value::text("side"),
        "outside the range: untouched"
    );
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("A7"), Value::text("Ann"));
}

#[test]
fn delete_contents_flags_and_fill_series() {
    let mut f = Fixture::new();
    f.set("A1", "text");
    f.set("A2", "5");
    f.set("A3", "=A2*2");
    f.set("A4", "2024-01-01");
    let r = f.apply(vec![Op::DeleteContents {
        sheet: f.sheet,
        ranges: vec![range("A1:A4")],
        what: DeleteContentsFlags {
            numbers: true,
            ..DeleteContentsFlags::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A1"), Value::text("text"));
    assert_eq!(f.value("A2"), Value::Empty);
    assert_eq!(f.value("A3"), Value::Number(0.0), "formulas stay");
    assert!(matches!(f.value("A4"), Value::Number(_)), "dates stay");
    // Fill series.
    let r = f.apply(vec![Op::FillSeries {
        sheet: f.sheet,
        range: range("C1:C5"),
        direction: Direction::Down,
        kind: SeriesKind::Linear,
        unit: None,
        start: Some("2".into()),
        end: Some("7".into()),
        increment: Some("2".into()),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("C1"), Value::Number(2.0));
    assert_eq!(f.value("C3"), Value::Number(6.0));
    assert_eq!(f.value("C4"), Value::Empty, "stops at the end value");
    f.set("D1", "2024-01-31");
    f.apply(vec![Op::FillSeries {
        sheet: f.sheet,
        range: range("D1:D3"),
        direction: Direction::Down,
        kind: SeriesKind::Date,
        unit: Some(DateUnit::Month),
        start: None,
        end: None,
        increment: None,
    }]);
    assert_eq!(f.text("D2"), "2024-02-29");
    assert_eq!(f.text("D3"), "2024-03-31");
    f.apply(vec![Op::FillSeries {
        sheet: f.sheet,
        range: range("E1:E3"),
        direction: Direction::Down,
        kind: SeriesKind::Growth,
        unit: None,
        start: Some("3".into()),
        end: None,
        increment: Some("2".into()),
    }]);
    assert_eq!(f.value("E3"), Value::Number(12.0));
}

#[test]
fn names_from_labels_and_database_ranges() {
    let mut f = Fixture::new();
    table(&mut f);
    let r = f.apply(vec![Op::CreateNamesFromLabels {
        sheet: f.sheet,
        range: range("A1:C7"),
        from: NameLabelFlags {
            top: true,
            ..NameLabelFlags::default()
        },
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let names = f.svc.names(f.doc).unwrap();
    assert_eq!(names.len(), 3);
    assert!(names
        .iter()
        .any(|n| n.name == "Pay" && n.refers_to.contains("$C$2:$C$7")));
    f.set("E1", "=SUM(Pay)");
    assert_eq!(f.value("E1"), Value::Number(160.0));
    let r = f.apply(vec![Op::DefineDatabaseRange {
        name: "Staff".into(),
        sheet: f.sheet,
        range: range("A1:C7"),
        has_header: true,
    }]);
    assert!(r.errors.is_empty());
    let db = f.svc.database_ranges(f.doc).unwrap();
    assert_eq!(db.len(), 1);
    assert_eq!(db[0].name, "Staff");
    let nav = f.svc.navigator(f.doc).unwrap();
    assert_eq!(nav.database_ranges.len(), 1);
    assert_eq!(nav.names.len(), 3);
    assert_eq!(nav.sheets.len(), 1);
    let stats = f.svc.document_statistics(f.doc).unwrap();
    assert_eq!(stats.names, 3);
    assert_eq!(stats.formulas, 1);
    f.apply(vec![Op::DeleteDatabaseRange {
        name: "staff".into(),
    }]);
    assert!(f.svc.database_ranges(f.doc).unwrap().is_empty());
}

#[test]
fn autoinput_offers_first_alphabetical_match() {
    let mut f = Fixture::new();
    f.set("A1", "Apple");
    f.set("A2", "apricot");
    f.set("A3", "Banana");
    let a = f.svc.autocomplete(f.doc, f.sheet, 3, 0, "Ap").unwrap();
    assert_eq!(a.completion.as_deref(), Some("Apple"));
    assert_eq!(a.items, vec!["Apple", "apricot"]);
    let a = f.svc.autocomplete(f.doc, f.sheet, 3, 0, "ap").unwrap();
    assert_eq!(a.completion.as_deref(), Some("apple"));
    let all = f.svc.autocomplete(f.doc, f.sheet, 3, 0, "").unwrap();
    assert_eq!(all.items.len(), 3);
    assert!(all.completion.is_none());
    // Another block: a gap row.
    f.set("A6", "Cherry");
    let a = f.svc.autocomplete(f.doc, f.sheet, 7, 0, "A").unwrap();
    assert!(a.completion.is_none());
}

#[test]
fn sort_options_left_to_right_custom_and_copy() {
    let mut f = Fixture::new();
    f.set("A1", "Q3");
    f.set("B1", "Q1");
    f.set("C1", "Q2");
    f.set("A2", "3");
    f.set("B2", "1");
    f.set("C2", "2");
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("A1:C2"),
        keys: vec![SortKey {
            col: 0,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: Some(SortOptions {
            left_to_right: true,
            include_formats: true,
            ..SortOptions::default()
        }),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A1"), Value::text("Q1"));
    assert_eq!(f.value("C2"), Value::Number(3.0));
    // Custom order and copy-to.
    f.set("E1", "low");
    f.set("E2", "high");
    f.set("E3", "mid");
    let r = f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("E1:E3"),
        keys: vec![SortKey {
            col: 4,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: Some(SortOptions {
            custom_order: Some(vec!["high".into(), "mid".into(), "low".into()]),
            copy_to: Some(SheetCell {
                sheet: f.sheet,
                r: 0,
                c: 6,
            }),
            include_formats: true,
            ..SortOptions::default()
        }),
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("E1"), Value::text("low"), "the range is unchanged");
    assert_eq!(f.value("G1"), Value::text("high"));
    assert_eq!(f.value("G2"), Value::text("mid"));
    assert_eq!(f.value("G3"), Value::text("low"));
    // Natural order.
    f.set("H1", "a10");
    f.set("H2", "a2");
    f.apply(vec![Op::SortRange {
        sheet: f.sheet,
        range: range("H1:H2"),
        keys: vec![SortKey {
            col: 7,
            ascending: true,
        }],
        has_header: false,
        case_sensitive: false,
        options: Some(SortOptions {
            natural: true,
            include_formats: true,
            ..SortOptions::default()
        }),
    }]);
    assert_eq!(f.value("H1"), Value::text("a2"));
    let r = f.apply(vec![Op::SetCustomSortLists {
        lists: vec![vec!["Mon".into(), "Tue".into()]],
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.svc.custom_sort_lists(f.doc).unwrap().len(), 1);
    f.svc.undo(f.doc).unwrap();
    assert!(f.svc.custom_sort_lists(f.doc).unwrap().is_empty());
}
