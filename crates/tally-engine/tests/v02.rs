//! v0.2 features through the `DocumentService`: names, array formulas,
//! move, merges, freeze, sort, AutoFilter, hidden rows, find & replace,
//! borders, paste special, fill, hard recalc. Every op is checked for
//! undo/redo with `to_data()` snapshots.
#![allow(clippy::unwrap_used)]

mod common;

use common::{input, range, sel, Fixture};
use tally_api::{
    BorderEdgePatch, BorderLineDto, BorderStyleDto, BordersPatch, CellPos, ColumnFilter,
    DocumentService, FillMode, FilterCondition, FilterOp, FindMode, FindQuery, Op, PasteOperation,
    PasteOptions, PastePayload, SortKey, StylePatch,
};
use tally_types::{ErrorKind, Value};

fn n(x: f64) -> Value {
    Value::Number(x)
}

/// Applies `ops`, asserts no errors, and checks undo/redo restore `to_data()`.
fn round_trip(f: &mut Fixture, ops: Vec<Op>) {
    let before = f.data();
    let res = f.apply(ops.clone());
    assert!(res.errors.is_empty(), "{ops:?}: {:?}", res.errors);
    let after = f.data();
    assert_ne!(before, after, "{ops:?} changed nothing");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.data(), before, "undo of {ops:?}");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(f.data(), after, "redo of {ops:?}");
}

fn formula(f: &Fixture, a1: &str) -> String {
    let a = tally_types::CellAddr::from_a1(f.sheet, a1).unwrap();
    f.svc
        .cell_edit_info(f.doc, f.sheet, a.row, a.col)
        .unwrap()
        .input
}

fn error_code(f: &mut Fixture, ops: Vec<Op>) -> String {
    let res = f.apply(ops);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    res.errors[0].code.clone()
}

// ----- a. names --------------------------------------------------------------

#[test]
fn named_ranges() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for (a, v) in [("A1", "1"), ("A2", "2"), ("A3", "3")] {
        f.set(a, v);
    }
    f.set("B1", "=SUM(Data)");
    assert_eq!(f.value("B1"), Value::Error(ErrorKind::Name));
    round_trip(
        &mut f,
        vec![Op::DefineName {
            name: "Data".into(),
            scope: None,
            refers_to: "$Sheet1.$A$1:$A$3".into(),
        }],
    );
    assert_eq!(f.value("B1"), n(6.0), "defining a name dirties its users");
    assert_eq!(formula(&f, "B1"), "=SUM(Data)");
    // Redefinition.
    round_trip(
        &mut f,
        vec![Op::DefineName {
            name: "Data".into(),
            scope: None,
            refers_to: "$Sheet1.$A$1:$A$2".into(),
        }],
    );
    assert_eq!(f.value("B1"), n(3.0));
    // Formulas typed after the definition use it too, and print it.
    f.set("B2", "=DATA*2");
    assert_eq!(formula(&f, "B2"), "=Data*2");
    // Edits inside the range propagate.
    f.set("A2", "10");
    assert_eq!(f.value("B1"), n(11.0));
    // Query.
    let names = f.svc.names(f.doc).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name, "Data");
    assert_eq!(names[0].refers_to, "$Sheet1.$A$1:$A$2");
    // Name box.
    let nb = f.svc.resolve_name_box(f.doc, "DATA").unwrap();
    assert_eq!(nb.ranges[0], range("A1:A2"));
    // Invalid names are rejected.
    for bad in ["A1", "1x", "has space", "TRUE"] {
        let code = error_code(
            &mut f,
            vec![Op::DefineName {
                name: bad.into(),
                scope: None,
                refers_to: "1".into(),
            }],
        );
        assert_eq!(code, "invalid_argument", "{bad}");
    }
    // Sheet scope shadows the workbook name.
    f.apply(vec![Op::DefineName {
        name: "Data".into(),
        scope: Some(s),
        refers_to: "$Sheet1.$A$3".into(),
    }]);
    f.set("C1", "=Data");
    assert_eq!(f.value("C1"), n(3.0));
    // Delete: users show #NAME? and keep the text.
    round_trip(
        &mut f,
        vec![Op::DeleteName {
            name: "Data".into(),
            scope: None,
        }],
    );
    assert_eq!(f.value("B2"), Value::Error(ErrorKind::Name));
    assert_eq!(formula(&f, "B2"), "=Data*2");
    // B1 was typed before any definition, so it resolves by text: now the
    // sheet-scoped Data.
    assert_eq!(f.value("B1"), n(3.0));
    // Inserting rows above moves the name's references.
    f.apply(vec![Op::DefineName {
        name: "Total".into(),
        scope: None,
        refers_to: "$Sheet1.$A$1:$A$3".into(),
    }]);
    f.set("D1", "=SUM(Total)");
    assert_eq!(f.value("D1"), n(14.0));
    round_trip(
        &mut f,
        vec![Op::InsertRows {
            sheet: s,
            at: 0,
            count: 2,
        }],
    );
    let names = f.svc.names(f.doc).unwrap();
    let total = names.iter().find(|x| x.name == "Total").unwrap();
    assert_eq!(total.refers_to, "$Sheet1.$A$3:$A$5");
    assert_eq!(f.value("D3"), n(14.0));
}

#[test]
fn names_survive_ods_round_trip() {
    let mut f = Fixture::new();
    f.set("A1", "4");
    f.apply(vec![
        Op::DefineName {
            name: "Rate".into(),
            scope: None,
            refers_to: "0.25".into(),
        },
        Op::DefineName {
            name: "Cell".into(),
            scope: Some(f.sheet),
            refers_to: "$Sheet1.$A$1".into(),
        },
    ]);
    f.set("B1", "=Cell*Rate");
    assert_eq!(f.value("B1"), n(1.0));
    let data = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    assert_eq!(back.names.len(), 2);
    let mut wb = tally_engine::Workbook::from_data(&back);
    tally_engine::recalc::recalc(&mut wb);
    let s = wb.sheets[0].id;
    assert_eq!(wb.value(tally_types::CellAddr::new(s, 0, 1)), n(1.0));
    assert_eq!(wb.to_data().names, data.names);
}

// ----- b. array formulas --------------------------------------------------------

#[test]
fn array_formulas() {
    let mut f = Fixture::new();
    let s = f.sheet;
    for (a, v) in [("A1", "1"), ("A2", "2"), ("A3", "3")] {
        f.set(a, v);
    }
    round_trip(
        &mut f,
        vec![Op::SetArrayFormula {
            sheet: s,
            range: range("B1:B3"),
            input: "=A1:A3*A1:A3".into(),
        }],
    );
    assert_eq!(f.value("B3"), n(9.0));
    let info = f.svc.cell_edit_info(f.doc, s, 1, 1).unwrap();
    assert_eq!(info.input, "{=A1:A3*A1:A3}");
    assert_eq!(info.array, Some(true));
    assert_eq!(info.array_range, Some(range("B1:B3")));
    let cells = f.svc.cells(f.doc, s, &[range("B2")]).unwrap();
    assert_eq!(cells[0].cells[0].flags & tally_api::cell_flags::ARRAY, 4);
    // Partial edits are refused.
    assert_eq!(error_code(&mut f, vec![input(s, "B2", "5")]), "array_block");
    assert_eq!(
        error_code(
            &mut f,
            vec![Op::Clear {
                sheet: s,
                ranges: vec![range("B1:B2")],
                what: tally_api::ClearWhat::Contents,
            }]
        ),
        "array_block"
    );
    assert_eq!(
        error_code(
            &mut f,
            vec![Op::InsertRows {
                sheet: s,
                at: 1,
                count: 1
            }]
        ),
        "array_block"
    );
    // Whole-block ops work.
    f.set("A3", "4");
    assert_eq!(f.value("B3"), n(16.0));
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 0,
        count: 1,
    }]);
    assert_eq!(f.value("B4"), n(16.0));
    assert_eq!(formula(&f, "B2"), "{=A2:A4*A2:A4}");
    // ODS round trip keeps the block.
    let data = f.data();
    assert_eq!(data.sheets[0].array_formulas.len(), 1);
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    let mut wb = tally_engine::Workbook::from_data(&back);
    tally_engine::recalc::recalc(&mut wb);
    assert_eq!(wb.to_data().sheets, data.sheets);
    // Clearing the whole block works and is undoable.
    round_trip(
        &mut f,
        vec![Op::Clear {
            sheet: s,
            ranges: vec![range("B2:B4")],
            what: tally_api::ClearWhat::Contents,
        }],
    );
}

// ----- d. move ------------------------------------------------------------------

#[test]
fn move_range_rewrites_references() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("B1", "=A1+A2");
    f.set("Z1", "100");
    f.set("A3", "=Z1*2");
    f.set("E5", "9");
    f.set("F1", "=E5");
    f.set("G1", "=SUM(A1:A3)");
    round_trip(
        &mut f,
        vec![Op::MoveRange {
            sheet: s,
            from: range("A1:A3"),
            to: CellPos { r: 3, c: 4 },
        }],
    );
    assert_eq!(formula(&f, "B1"), "=E4+E5");
    assert_eq!(f.value("B1"), n(3.0));
    assert_eq!(
        formula(&f, "E6"),
        "=Z1*2",
        "moved formula keeps outside target"
    );
    assert_eq!(
        formula(&f, "F1"),
        "=#REF!",
        "E5 was overwritten by the move"
    );
    assert_eq!(formula(&f, "G1"), "=SUM(E4:E6)");
    assert_eq!(f.value("A1"), Value::Empty);
}

#[test]
fn move_onto_referenced_cells_gives_ref_errors() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("C1", "5");
    f.set("D1", "=C1");
    f.apply(vec![Op::MoveRange {
        sheet: s,
        from: range("A1"),
        to: CellPos { r: 0, c: 2 },
    }]);
    assert_eq!(formula(&f, "D1"), "=#REF!");
    assert_eq!(f.value("C1"), n(1.0));
}

// ----- e. merges ----------------------------------------------------------------

#[test]
fn merges() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "keep");
    f.set("B1", "drop");
    round_trip(
        &mut f,
        vec![Op::MergeCells {
            sheet: s,
            range: range("A1:B2"),
        }],
    );
    assert_eq!(f.value("B1"), Value::Empty);
    let layout = f.svc.sheet_layout(f.doc, s).unwrap();
    assert_eq!(layout.merges, vec![range("A1:B2")]);
    // Typing into a covered cell edits the anchor.
    f.apply(vec![input(s, "B2", "new")]);
    assert_eq!(f.value("A1"), Value::text("new"));
    assert_eq!(f.value("B2"), Value::Empty);
    // Inserting rows inside grows the merge; deleting shrinks it.
    f.apply(vec![Op::InsertRows {
        sheet: s,
        at: 1,
        count: 2,
    }]);
    assert_eq!(
        f.svc.sheet_layout(f.doc, s).unwrap().merges,
        vec![range("A1:B4")]
    );
    f.apply(vec![Op::DeleteCols {
        sheet: s,
        at: 1,
        count: 1,
    }]);
    assert_eq!(
        f.svc.sheet_layout(f.doc, s).unwrap().merges,
        vec![range("A1:A4")]
    );
    // Copy/paste carries the merge.
    let clip = f.svc.copy(f.doc, &sel(s, "A1:A4")).unwrap();
    f.apply(vec![Op::Paste {
        sheet: s,
        target: range("D1"),
        payload: PastePayload::Internal { token: clip.token },
    }]);
    let merges = f.svc.sheet_layout(f.doc, s).unwrap().merges;
    assert!(merges.contains(&range("D1:D4")), "{merges:?}");
    assert_eq!(f.value("D1"), Value::text("new"));
    // Partial overlap is refused; unmerge works.
    assert_eq!(
        error_code(
            &mut f,
            vec![Op::MergeCells {
                sheet: s,
                range: range("A3:B6"),
            }]
        ),
        "merged"
    );
    round_trip(
        &mut f,
        vec![Op::UnmergeCells {
            sheet: s,
            range: range("A2"),
        }],
    );
    // ODS round trip (via the model).
    let data = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    assert_eq!(back.sheets[0].merges, data.sheets[0].merges);
}

// ----- f. freeze, i. hidden -------------------------------------------------------

#[test]
fn freeze_and_hidden_rows() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "x");
    round_trip(
        &mut f,
        vec![Op::SetFrozen {
            sheet: s,
            rows: 1,
            cols: 2,
        }],
    );
    let l = f.svc.sheet_layout(f.doc, s).unwrap();
    assert_eq!(l.frozen, Some(CellPos { r: 1, c: 2 }));
    f.apply(vec![Op::SetFrozen {
        sheet: s,
        rows: 0,
        cols: 0,
    }]);
    assert_eq!(f.svc.sheet_layout(f.doc, s).unwrap().frozen, None);
    // Hide rows keeping a custom height.
    f.apply(vec![Op::SetRowHeight {
        sheet: s,
        start: 3,
        end: 3,
        px: 40.0,
    }]);
    round_trip(
        &mut f,
        vec![Op::SetRowsHidden {
            sheet: s,
            start: 2,
            end: 4,
            hidden: true,
        }],
    );
    let l = f.svc.sheet_layout(f.doc, s).unwrap();
    assert!(l.row_runs.iter().all(|r| r.hidden && r.size == 0.0));
    assert_eq!(l.row_runs.first().map(|r| r.start), Some(2));
    f.apply(vec![Op::SetRowsHidden {
        sheet: s,
        start: 2,
        end: 4,
        hidden: false,
    }]);
    let l = f.svc.sheet_layout(f.doc, s).unwrap();
    assert_eq!(
        l.row_runs.len(),
        1,
        "the custom height survives: {:?}",
        l.row_runs
    );
    assert!((l.row_runs[0].size - 40.0).abs() < 0.1);
    round_trip(
        &mut f,
        vec![Op::SetColsHidden {
            sheet: s,
            start: 1,
            end: 1,
            hidden: true,
        }],
    );
}

// ----- g. sort --------------------------------------------------------------------

#[test]
fn sort_rows() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let rows = [
        ["Name", "Qty", ""],
        ["pear", "3", "=B2*10"],
        ["Apple", "", "=B3*10"],
        ["fig", "1", "=B4*$E$1"],
        ["apple", "2", "=B5*10"],
    ];
    f.set("E1", "100");
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if !v.is_empty() {
                let a = format!("{}{}", ["A", "B", "C"][c], r + 1);
                f.set(&a, v);
            }
        }
    }
    let header = f.svc.detect_header(f.doc, s, range("A1:C5")).unwrap();
    assert!(header);
    round_trip(
        &mut f,
        vec![Op::SortRange {
            sheet: s,
            range: range("A1:C5"),
            keys: vec![SortKey {
                col: 1,
                ascending: true,
            }],
            has_header: true,
            case_sensitive: false,
        }],
    );
    let col = |f: &Fixture, c: &str| -> Vec<Value> {
        (1..=5).map(|r| f.value(&format!("{c}{r}"))).collect()
    };
    assert_eq!(
        col(&f, "A"),
        vec![
            Value::text("Name"),
            Value::text("fig"),
            Value::text("apple"),
            Value::text("pear"),
            Value::text("Apple"),
        ],
        "empty key last"
    );
    // Formulas follow their row; the absolute reference stays.
    assert_eq!(formula(&f, "C2"), "=B2*$E$1");
    assert_eq!(f.value("C2"), n(100.0));
    assert_eq!(f.value("C4"), n(30.0));
    // Text key, descending, case-insensitive (stable for apple/Apple).
    f.apply(vec![Op::SortRange {
        sheet: s,
        range: range("A2:C5"),
        keys: vec![SortKey {
            col: 0,
            ascending: false,
        }],
        has_header: false,
        case_sensitive: false,
    }]);
    assert_eq!(
        col(&f, "A")[1..].to_vec(),
        vec![
            Value::text("pear"),
            Value::text("fig"),
            Value::text("apple"),
            Value::text("Apple"),
        ]
    );
    // Merges inside are refused.
    f.apply(vec![Op::MergeCells {
        sheet: s,
        range: range("A7:B7"),
    }]);
    assert_eq!(
        error_code(
            &mut f,
            vec![Op::SortRange {
                sheet: s,
                range: range("A6:C8"),
                keys: vec![SortKey {
                    col: 0,
                    ascending: true
                }],
                has_header: false,
                case_sensitive: false,
            }]
        ),
        "merged"
    );
}

// ----- h. AutoFilter ----------------------------------------------------------------

#[test]
fn autofilter() {
    let mut f = Fixture::new();
    let s = f.sheet;
    let data = [("A1", "Status"), ("B1", "Amount")];
    for (a, v) in data {
        f.set(a, v);
    }
    for (i, (st, amt)) in [("Paid", "10"), ("Due", "20"), ("Paid", "30"), ("", "40")]
        .iter()
        .enumerate()
    {
        if !st.is_empty() {
            f.set(&format!("A{}", i + 2), st);
        }
        f.set(&format!("B{}", i + 2), amt);
    }
    f.set("D1", "=SUM(B2:B5)");
    round_trip(
        &mut f,
        vec![Op::SetAutoFilter {
            sheet: s,
            range: Some(range("A1:B5")),
        }],
    );
    let info = f.svc.filter_info(f.doc, s).unwrap();
    assert_eq!(info.range, Some(range("A1:B5")));
    assert_eq!(info.columns[0].unique, vec!["Due", "Paid", ""]);
    assert_eq!(info.columns[0].counts, vec![1, 2, 1]);
    assert_eq!(info.columns[1].unique, vec!["10", "20", "30", "40"]);
    round_trip(
        &mut f,
        vec![Op::SetColumnFilter {
            sheet: s,
            col: 0,
            filter: Some(ColumnFilter {
                values: Some(vec!["Paid".into()]),
                condition: None,
            }),
        }],
    );
    let l = f.svc.sheet_layout(f.doc, s).unwrap();
    let filtered: Vec<(u32, u32)> = l
        .row_runs
        .iter()
        .filter(|r| r.filtered == Some(true))
        .map(|r| (r.start, r.end))
        .collect();
    assert_eq!(filtered, vec![(2, 2), (4, 4)]);
    assert_eq!(
        f.value("D1"),
        n(100.0),
        "SUM still includes filtered rows (AOO)"
    );
    // Second column condition combines with AND.
    f.apply(vec![Op::SetColumnFilter {
        sheet: s,
        col: 1,
        filter: Some(ColumnFilter {
            values: None,
            condition: Some(FilterCondition {
                op: FilterOp::Gt,
                value: "15".into(),
            }),
        }),
    }]);
    let hidden: Vec<u32> = hidden_rows(&f);
    assert_eq!(hidden, vec![1, 2, 4]);
    // Editing a cell does not re-apply the filter (AOO); refreshing does.
    let res = f.set("B2", "50");
    assert!(!res.layout_changed.contains(&s));
    assert_eq!(hidden_rows(&f), vec![1, 2, 4]);
    let res = f.apply(vec![Op::RefreshFilter { sheet: s }]);
    assert!(res.layout_changed.contains(&s));
    assert_eq!(hidden_rows(&f), vec![2, 4]);
    f.svc.undo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), vec![1, 2, 4], "refresh is undoable");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), vec![2, 4]);
    // User-hidden rows are distinct and survive removing the filter.
    f.apply(vec![Op::SetRowsHidden {
        sheet: s,
        start: 9,
        end: 9,
        hidden: true,
    }]);
    // ODS keeps the filter, its criteria and the filtered rows.
    let d = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&d, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    assert_eq!(back.sheets[0].auto_filter, d.sheets[0].auto_filter);
    round_trip(
        &mut f,
        vec![Op::SetAutoFilter {
            sheet: s,
            range: None,
        }],
    );
    assert_eq!(hidden_rows(&f), vec![9]);
}

fn hidden_rows(f: &Fixture) -> Vec<u32> {
    let l = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
    l.row_runs
        .iter()
        .filter(|r| r.hidden)
        .flat_map(|r| r.start..=r.end)
        .collect()
}

// ----- j. find & replace -----------------------------------------------------------

fn query(text: &str) -> FindQuery {
    FindQuery {
        text: text.into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: FindMode::Literal,
        from: None,
    }
}

#[test]
fn find_and_replace() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "apple pie");
    f.set("A2", "Apple");
    f.set("B1", "=SUM(A5:A6)");
    f.set("C3", "1234");
    let hits = f.svc.find(f.doc, &query("apple")).unwrap();
    assert_eq!(hits.hits.len(), 2);
    assert!(!hits.truncated);
    let whole = f
        .svc
        .find(
            f.doc,
            &FindQuery {
                whole_cell: true,
                match_case: true,
                ..query("Apple")
            },
        )
        .unwrap();
    assert_eq!(whole.hits.len(), 1);
    assert_eq!((whole.hits[0].r, whole.hits[0].c), (1, 0));
    let wild = f
        .svc
        .find(
            f.doc,
            &FindQuery {
                mode: FindMode::Wildcards,
                whole_cell: true,
                ..query("a*e")
            },
        )
        .unwrap();
    assert_eq!(wild.hits.len(), 2, "'apple pie' and 'Apple' both match a*e");
    let in_formulas = f
        .svc
        .find(
            f.doc,
            &FindQuery {
                in_formulas: true,
                ..query("A5")
            },
        )
        .unwrap();
    assert_eq!(in_formulas.hits.len(), 1);
    // `from` rotates the hits.
    let rot = f
        .svc
        .find(
            f.doc,
            &FindQuery {
                from: Some(CellPos { r: 0, c: 0 }),
                ..query("apple")
            },
        )
        .unwrap();
    assert_eq!((rot.hits[0].r, rot.hits[0].c), (1, 0));
    // Regex replace in values.
    round_trip(
        &mut f,
        vec![Op::ReplaceAll {
            sheet: Some(s),
            query: FindQuery {
                mode: FindMode::Regex,
                ..query("(a)pple")
            },
            replace: "$1pricot".into(),
        }],
    );
    assert_eq!(f.value("A1"), Value::text("apricot pie"));
    assert_eq!(f.value("A2"), Value::text("Apricot"));
    // Replace in formulas, with a failure reported and the rest applied.
    f.set("B2", "=A5+1");
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: None,
        query: FindQuery {
            in_formulas: true,
            ..query("A5")
        },
        replace: "A7".into(),
    }]);
    assert!(res.errors.is_empty());
    assert_eq!(formula(&f, "B1"), "=SUM(A6:A7)", "reversed area normalized");
    assert_eq!(formula(&f, "B2"), "=A7+1");
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: None,
        query: FindQuery {
            in_formulas: true,
            ..query("+1")
        },
        replace: "+(".into(),
    }]);
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert_eq!(formula(&f, "B2"), "=A7+1", "kept");
    // Numbers are replaced through the recognizer.
    f.apply(vec![Op::ReplaceAll {
        sheet: None,
        query: query("23"),
        replace: "99".into(),
    }]);
    assert_eq!(f.value("C3"), n(1994.0));
}

// ----- k. borders ---------------------------------------------------------------------

#[test]
fn borders_patch_and_ods() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("B2", "x");
    let line = BorderLineDto {
        style: BorderStyleDto::Solid,
        width_px: 1.0,
        color: 0x0000_00ff,
    };
    let edge = Some(BorderEdgePatch {
        line: Some(line.clone()),
    });
    let patch = StylePatch {
        borders: Some(BordersPatch {
            top: edge.clone(),
            bottom: edge.clone(),
            left: edge.clone(),
            right: edge.clone(),
            inner_h: None,
            inner_v: edge,
        }),
        ..StylePatch::default()
    };
    round_trip(
        &mut f,
        vec![Op::ApplyStyle {
            sheet: s,
            ranges: vec![range("B2:C3")],
            patch,
        }],
    );
    let cells = f.svc.cells(f.doc, s, &[range("B2:C3")]).unwrap();
    let ids: Vec<u32> = cells[0].cells.iter().map(|c| c.style).collect();
    let styles = f.svc.styles(f.doc, &ids).unwrap();
    let b2 = &styles[0].borders;
    assert!(b2.top.is_some() && b2.left.is_some() && b2.right.is_some());
    assert!(b2.bottom.is_none(), "no inner horizontal line requested");
    let data = f.data();
    let mut bytes = std::io::Cursor::new(Vec::new());
    tally_io_ods::write_ods(&data, &mut bytes).unwrap();
    let back = tally_io_ods::read_ods(
        std::io::Cursor::new(bytes.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap();
    let mut wb = tally_engine::Workbook::from_data(&back);
    tally_engine::recalc::recalc(&mut wb);
    assert_eq!(wb.to_data().styles, data.styles);
}

// ----- l. paste special ------------------------------------------------------------------

#[test]
fn paste_special() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "2");
    f.set("A2", "=A1*5");
    f.apply(vec![Op::ApplyStyle {
        sheet: s,
        ranges: vec![range("A1:A2")],
        patch: StylePatch {
            bold: Some(true),
            ..StylePatch::default()
        },
    }]);
    let clip = f.svc.copy(f.doc, &sel(s, "A1:A2")).unwrap();
    let paste = |target: &str, options: PasteOptions| Op::PasteSpecial {
        sheet: s,
        target: range(target),
        payload: PastePayload::Internal {
            token: clip.token.clone(),
        },
        options,
    };
    // Values only, transposed.
    round_trip(
        &mut f,
        vec![paste(
            "C1",
            PasteOptions {
                values_only: true,
                transpose: true,
                ..PasteOptions::default()
            },
        )],
    );
    assert_eq!(f.value("C1"), n(2.0));
    assert_eq!(f.value("D1"), n(10.0));
    assert_eq!(formula(&f, "D1"), "10");
    let st = f.svc.cells(f.doc, s, &[range("D1")]).unwrap()[0].cells[0].style;
    assert_eq!(st, 0, "values only keeps the target format");
    // Formats only.
    f.set("F1", "7");
    f.apply(vec![paste(
        "F1",
        PasteOptions {
            formats_only: true,
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(f.value("F1"), n(7.0));
    let st = f.svc.cells(f.doc, s, &[range("F1")]).unwrap()[0].cells[0].style;
    assert!(f.svc.styles(f.doc, &[st]).unwrap()[0].bold);
    // Arithmetic.
    f.set("H1", "1");
    f.set("H2", "=3");
    f.apply(vec![paste(
        "H1",
        PasteOptions {
            values_only: true,
            operation: PasteOperation::Add,
            ..PasteOptions::default()
        },
    )]);
    assert_eq!(f.value("H1"), n(3.0));
    assert_eq!(formula(&f, "H2"), "=(3)+10");
    assert_eq!(f.value("H2"), n(13.0));
    // Skip empty (TSV).
    f.set("J1", "keep");
    f.apply(vec![Op::PasteSpecial {
        sheet: s,
        target: range("J1"),
        payload: PastePayload::Tsv {
            text: "\tx\n=1+1\t".into(),
        },
        options: PasteOptions {
            skip_empty: true,
            values_only: true,
            ..PasteOptions::default()
        },
    }]);
    assert_eq!(f.value("J1"), Value::text("keep"));
    assert_eq!(f.value("K1"), Value::text("x"));
    assert_eq!(
        f.value("J2"),
        Value::text("=1+1"),
        "values only: no formulas"
    );
}

// ----- m. fill ------------------------------------------------------------------------

fn fill(f: &mut Fixture, source: &str, target: &str, mode: FillMode) {
    let s = f.sheet;
    round_trip(
        f,
        vec![Op::FillRange {
            sheet: s,
            source: range(source),
            target: range(target),
            mode,
        }],
    );
}

#[test]
fn fill_series() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    fill(&mut f, "A1", "A1:A4", FillMode::Auto);
    assert_eq!(f.value("A4"), n(4.0));
    f.set("B1", "2");
    f.set("B2", "5");
    fill(&mut f, "B1:B2", "B1:B4", FillMode::Auto);
    assert_eq!(f.value("B4"), n(11.0));
    // Up continues backwards.
    f.set("C5", "1");
    f.set("C6", "2");
    fill(&mut f, "C5:C6", "C3:C6", FillMode::Auto);
    assert_eq!(f.value("C4"), n(0.0));
    assert_eq!(f.value("C3"), n(-1.0));
    // Day and month names, keeping case.
    f.set("D1", "Mon");
    fill(&mut f, "D1", "D1:D3", FillMode::Auto);
    assert_eq!(f.value("D3"), Value::text("Wed"));
    f.set("E1", "DECEMBER");
    fill(&mut f, "E1", "E1:F1", FillMode::Auto);
    assert_eq!(f.value("F1"), Value::text("JANUARY"));
    // Numbered text keeps zeros.
    f.set("G1", "Item 09");
    fill(&mut f, "G1", "G1:G2", FillMode::Auto);
    assert_eq!(f.value("G2"), Value::text("Item 10"));
    // Plain text copies; formulas copy relatively.
    f.set("H1", "x");
    f.set("I1", "=H1&\"!\"");
    fill(&mut f, "H1:I1", "H1:I3", FillMode::Auto);
    assert_eq!(f.value("H3"), Value::text("x"));
    assert_eq!(formula(&f, "I3"), "=H3&\"!\"");
    // Dates: month steps.
    f.set("J1", "01/31/2024");
    f.set("J2", "02/29/2024");
    fill(&mut f, "J1:J2", "J1:J3", FillMode::Auto);
    // The day of month differs: a constant step in days (29).
    assert_eq!(f.text("J3"), "03/29/2024");
}

#[test]
fn fill_dates_and_modes() {
    let mut f = Fixture::new();
    f.set("A1", "01/15/2024");
    f.set("A2", "02/15/2024");
    fill(&mut f, "A1:A2", "A1:A4", FillMode::Auto);
    assert_eq!(f.text("A4"), "04/15/2024");
    f.set("B1", "03/01/2024");
    fill(&mut f, "B1", "B1:B2", FillMode::Auto);
    assert_eq!(f.text("B2"), "03/02/2024");
    f.set("C1", "3");
    f.set("C2", "6");
    fill(&mut f, "C1:C2", "C1:C4", FillMode::GrowthSeries);
    assert_eq!(f.value("C4"), n(24.0));
    f.set("D1", "5");
    fill(&mut f, "D1", "D1:D3", FillMode::Copy);
    assert_eq!(f.value("D3"), n(5.0));
    let s = f.sheet;
    let res = f.apply(vec![Op::FillRange {
        sheet: s,
        source: range("D1"),
        target: range("E2"),
        mode: FillMode::Auto,
    }]);
    assert_eq!(res.errors[0].code, "invalid_argument");
}

// ----- n. hard recalc -------------------------------------------------------------------

#[test]
fn recalc_all_recomputes() {
    let mut f = Fixture::new();
    f.set("A1", "2");
    f.set("B1", "=A1*2");
    // Corrupt a cached value behind the engine's back.
    {
        let d = f.svc.document_mut(f.doc).unwrap();
        let a = tally_types::CellAddr::new(f.sheet, 0, 1);
        let (id, _) = d.wb.formula_at(a).unwrap();
        d.wb.formulas.get_mut(id).unwrap().cached = n(0.0);
        d.recalc_incomplete = true;
    }
    assert_eq!(f.value("B1"), n(0.0));
    let res = f.svc.recalc_all(f.doc).unwrap();
    assert!(res.errors.is_empty());
    assert_eq!(f.value("B1"), n(4.0));
    assert!(!f.svc.document(f.doc).unwrap().recalc_incomplete);
}

// ----- p. XLSX codec ---------------------------------------------------------------------

#[cfg(feature = "io")]
#[test]
fn xlsx_save_and_open_through_the_service() {
    let mut svc = tally_engine::EngineService::with_default_codec();
    let info = svc.new_workbook(None).unwrap();
    let s = info.sheets[0].id;
    svc.apply(
        info.doc,
        common::tx(vec![input(s, "A1", "2"), input(s, "A2", "=A1*21")]),
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("tally-v02-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("book.xlsx");
    svc.save(info.doc, Some(&path), tally_api::FileFormat::Xlsx)
        .unwrap();
    let back = svc.open(&path).unwrap();
    let wb = &svc.document(back.doc).unwrap().wb;
    let s2 = wb.sheets[0].id;
    assert_eq!(wb.value(tally_types::CellAddr::new(s2, 1, 0)), n(42.0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_sheet_drops_its_names_and_undo_restores_them() {
    let mut f = Fixture::new();
    let res = f.apply(vec![Op::AddSheet {
        name: Some("Data".into()),
        index: None,
    }]);
    let data = res.info.unwrap().sheets[1].id;
    f.apply(vec![
        input(data, "A1", "5"),
        Op::DefineName {
            name: "Five".into(),
            scope: None,
            refers_to: "$Data.$A$1".into(),
        },
        Op::DefineName {
            name: "Local".into(),
            scope: Some(data),
            refers_to: "1".into(),
        },
    ]);
    f.set("A1", "=Five*2");
    assert_eq!(f.value("A1"), n(10.0));
    round_trip(&mut f, vec![Op::DeleteSheet { sheet: data }]);
    assert_eq!(f.value("A1"), Value::Error(ErrorKind::Ref));
    let names = f.svc.names(f.doc).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].refers_to, "#REF!");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("A1"), n(10.0));
    assert_eq!(f.svc.names(f.doc).unwrap().len(), 2);
}

#[test]
fn move_carries_merges_and_array_blocks() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "1");
    f.set("A2", "2");
    f.apply(vec![
        Op::MergeCells {
            sheet: s,
            range: range("C1:D1"),
        },
        Op::SetArrayFormula {
            sheet: s,
            range: range("B1:B2"),
            input: "=A1:A2*2".into(),
        },
    ]);
    // Moving part of the array is refused.
    assert_eq!(
        error_code(
            &mut f,
            vec![Op::MoveRange {
                sheet: s,
                from: range("B1"),
                to: CellPos { r: 9, c: 1 },
            }]
        ),
        "array_block"
    );
    round_trip(
        &mut f,
        vec![Op::MoveRange {
            sheet: s,
            from: range("A1:D2"),
            to: CellPos { r: 9, c: 0 },
        }],
    );
    assert_eq!(f.value("B11"), n(4.0));
    assert_eq!(formula(&f, "B10"), "{=A10:A11*2}");
    assert_eq!(
        f.svc.sheet_layout(f.doc, s).unwrap().merges,
        vec![range("C10:D10")]
    );
    // A copy of part of the block pastes values.
    let clip = f.svc.copy(f.doc, &sel(s, "B11")).unwrap();
    f.apply(vec![Op::Paste {
        sheet: s,
        target: range("F1"),
        payload: PastePayload::Internal { token: clip.token },
    }]);
    assert_eq!(f.value("F1"), n(4.0));
    assert_eq!(formula(&f, "F1"), "4");
}

#[test]
fn deleting_rows_through_a_merge_and_undo() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.apply(vec![Op::MergeCells {
        sheet: s,
        range: range("A2:B3"),
    }]);
    round_trip(
        &mut f,
        vec![Op::DeleteRows {
            sheet: s,
            at: 1,
            count: 2,
        }],
    );
    assert!(f.svc.sheet_layout(f.doc, s).unwrap().merges.is_empty());
    f.svc.undo(f.doc).unwrap();
    assert_eq!(
        f.svc.sheet_layout(f.doc, s).unwrap().merges,
        vec![range("A2:B3")]
    );
}

#[test]
fn moving_a_block_onto_itself_shifted_keeps_its_merge_and_array() {
    let mut f = Fixture::new();
    let s = f.sheet;
    f.set("A1", "3");
    f.apply(vec![
        Op::MergeCells {
            sheet: s,
            range: range("A1:B1"),
        },
        Op::SetArrayFormula {
            sheet: s,
            range: range("A2:B2"),
            input: "=A1*2".into(),
        },
    ]);
    round_trip(
        &mut f,
        vec![Op::MoveRange {
            sheet: s,
            from: range("A1:B2"),
            to: CellPos { r: 0, c: 1 },
        }],
    );
    assert_eq!(
        f.svc.sheet_layout(f.doc, s).unwrap().merges,
        vec![range("B1:C1")]
    );
    assert_eq!(f.value("B1"), n(3.0));
    assert_eq!(f.value("C2"), n(6.0));
    assert_eq!(formula(&f, "B2"), "{=B1*2}");
}

#[test]
fn volatile_through_a_name() {
    let mut f = Fixture::new();
    f.apply(vec![Op::DefineName {
        name: "Now".into(),
        scope: None,
        refers_to: "1".into(),
    }]);
    f.set("A1", "=Now");
    let id = {
        let d = f.svc.document(f.doc).unwrap();
        d.wb.formula_at(tally_types::CellAddr::new(f.sheet, 0, 0))
            .unwrap()
            .0
    };
    let vol = |f: &Fixture| f.svc.document(f.doc).unwrap().wb.formulas[id].volatile;
    assert!(!vol(&f));
    f.apply(vec![Op::DefineName {
        name: "Now".into(),
        scope: None,
        refers_to: "RAND()".into(),
    }]);
    assert!(
        vol(&f),
        "redefining the name with RAND() makes its users volatile"
    );
}
