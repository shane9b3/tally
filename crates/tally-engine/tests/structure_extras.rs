//! Insert/delete cells with shift, copy sheet, hide sheet, tab colour and
//! the name fixes of review 3 (wave 5, W5-2).
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{ApiError, DocumentService, Op, ShiftDir};
use tally_types::{CellAddr, Value};

fn formula(f: &Fixture, a1: &str) -> String {
    let a = CellAddr::from_a1(f.sheet, a1).unwrap();
    let wb = &f.svc.document(f.doc).unwrap().wb;
    let (id, _) = wb.formula_at(a).unwrap();
    wb.formula_text(id).unwrap()
}

#[test]
fn insert_cells_shift_down_adjusts_refs() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A2", "2");
    f.set("A3", "3");
    f.set("B1", "10");
    f.set("B2", "20");
    f.set("C1", "=A2");
    f.set("C2", "=SUM(A1:A3)");
    f.set("C3", "=B2");
    f.set("C4", "=SUM(A1:B3)");
    let r = f.apply(vec![Op::InsertCells {
        sheet: f.sheet,
        range: range("A2:A2"),
        shift: ShiftDir::Down,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("A2"), Value::Empty);
    assert_eq!(f.value("A3"), Value::Number(2.0));
    assert_eq!(f.value("A4"), Value::Number(3.0));
    assert_eq!(
        f.value("B2"),
        Value::Number(20.0),
        "outside the band: unchanged"
    );
    assert_eq!(formula(&f, "C1"), "=A3", "a reference into the block moves");
    assert_eq!(
        formula(&f, "C2"),
        "=SUM(A1:A4)",
        "a range over the insertion grows"
    );
    assert_eq!(formula(&f, "C3"), "=B2", "outside the band: unchanged");
    assert_eq!(
        formula(&f, "C4"),
        "=SUM(A1:B3)",
        "partially overlapping: unchanged"
    );
    assert_eq!(f.value("C2"), Value::Number(6.0));
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.value("A2"), Value::Number(2.0));
    assert_eq!(formula(&f, "C1"), "=A2");
    assert_eq!(formula(&f, "C2"), "=SUM(A1:A3)");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(formula(&f, "C1"), "=A3");
}

#[test]
fn delete_cells_shift_left_and_refs() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("B1", "2");
    f.set("C1", "3");
    f.set("A3", "=B1");
    f.set("A4", "=SUM(A1:C1)");
    f.set("A5", "=C1");
    let r = f.apply(vec![Op::DeleteCells {
        sheet: f.sheet,
        range: range("B1:B1"),
        shift: ShiftDir::Left,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(f.value("B1"), Value::Number(3.0));
    assert_eq!(f.value("C1"), Value::Empty);
    assert_eq!(formula(&f, "A3"), "=#REF!");
    assert_eq!(formula(&f, "A4"), "=SUM(A1:B1)");
    assert_eq!(formula(&f, "A5"), "=B1");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(formula(&f, "A3"), "=B1");
    assert_eq!(f.value("C1"), Value::Number(3.0));
}

#[test]
fn copy_sheet_formulas_point_at_the_copy() {
    let mut f = Fixture::new();
    f.set("A1", "5");
    f.set("A2", "=A1*2");
    f.set("A3", "=Sheet1.A1+1");
    f.apply(vec![Op::DefineName {
        name: "Local".into(),
        scope: Some(f.sheet),
        refers_to: "$Sheet1.$A$1".into(),
    }]);
    f.set("A4", "=Local");
    let r = f.apply(vec![Op::CopySheet {
        sheet: f.sheet,
        name: None,
        index: None,
    }]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r.sheets_changed);
    let copy = r.created.unwrap()[0].id;
    let info = f.svc.document(f.doc).unwrap().info();
    assert_eq!(info.sheets[1].name, "Sheet1_2");
    f.set("A1", "100");
    assert_eq!(f.value_on(copy, "A1"), Value::Number(5.0));
    assert_eq!(f.value_on(copy, "A2"), Value::Number(10.0));
    assert_eq!(
        f.value_on(copy, "A3"),
        Value::Number(6.0),
        "points at the copy"
    );
    assert_eq!(
        f.value_on(copy, "A4"),
        Value::Number(5.0),
        "scoped name copied"
    );
    assert_eq!(f.value("A3"), Value::Number(101.0));
    f.svc.undo(f.doc).unwrap();
    f.svc.undo(f.doc).unwrap();
    assert_eq!(f.svc.document(f.doc).unwrap().info().sheets.len(), 1);
}

#[test]
fn hide_sheet_and_tab_colour() {
    let mut f = Fixture::new();
    let r = f.apply(vec![Op::SetSheetHidden {
        sheet: f.sheet,
        hidden: true,
    }]);
    assert_eq!(
        r.errors[0].code,
        ApiError::INVALID_ARGUMENT,
        "last visible sheet"
    );
    f.apply(vec![Op::AddSheet {
        name: None,
        index: None,
    }]);
    let r = f.apply(vec![
        Op::SetSheetHidden {
            sheet: f.sheet,
            hidden: true,
        },
        Op::SetTabColor {
            sheet: f.sheet,
            color: Some(0xff00_00ff),
        },
    ]);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let info = f.svc.document(f.doc).unwrap().info();
    assert!(info.sheets[0].hidden);
    assert_eq!(info.sheets[0].tab_color.as_deref(), Some("#ff0000"));
    f.svc.undo(f.doc).unwrap();
    let info = f.svc.document(f.doc).unwrap().info();
    assert!(!info.sheets[0].hidden);
    assert_eq!(info.sheets[0].tab_color, None);
}

#[test]
fn deleting_a_sheet_name_falls_back_to_the_global_one() {
    let mut f = Fixture::new();
    f.set("A1", "1");
    f.set("A3", "3");
    f.apply(vec![
        Op::DefineName {
            name: "X".into(),
            scope: None,
            refers_to: "$Sheet1.$A$1".into(),
        },
        Op::DefineName {
            name: "X".into(),
            scope: Some(f.sheet),
            refers_to: "$Sheet1.$A$3".into(),
        },
    ]);
    f.set("G1", "=X");
    assert_eq!(f.value("G1"), Value::Number(3.0));
    let r = f.apply(vec![Op::DeleteName {
        name: "X".into(),
        scope: Some(f.sheet),
    }]);
    assert!(r.errors.is_empty());
    assert_eq!(f.value("G1"), Value::Number(1.0));
    f.set("A1", "7");
    assert_eq!(f.value("G1"), Value::Number(7.0));
}

#[test]
fn imported_names_use_base_cells_and_survive_wide_grid_collisions() {
    use tally_model::{CellContentData, CellData, NameData, SheetData, WorkbookData};
    let mut data = WorkbookData::default();
    data.sheets.push(SheetData {
        name: "Sheet1".into(),
        cells: vec![
            CellData {
                row: 0,
                col: 1,
                content: CellContentData::Value(Value::Number(10.0)),
                style: None,
            },
            CellData {
                row: 1,
                col: 1,
                content: CellContentData::Value(Value::Number(20.0)),
                style: None,
            },
            CellData {
                row: 0,
                col: 2,
                content: CellContentData::Formula {
                    text: "of:=Rel".into(),
                    cached: None,
                },
                style: None,
            },
            CellData {
                row: 1,
                col: 2,
                content: CellContentData::Formula {
                    text: "of:=Rel".into(),
                    cached: None,
                },
                style: None,
            },
            CellData {
                row: 3,
                col: 1,
                content: CellContentData::Formula {
                    text: "of:=Rel2".into(),
                    cached: None,
                },
                style: None,
            },
        ],
        ..SheetData::default()
    });
    // `Rel` = the cell to the left, relative to C1.
    data.names.push(NameData {
        name: "Rel".into(),
        sheet: None,
        formula: "of:=[.B1]".into(),
        base_cell: Some((0, 0, 2)),
        usage: Default::default(),
    });
    // `Rel2` is a cell reference in a 16k-column grid, a name in AOO.
    data.names.push(NameData {
        name: "Rel2".into(),
        sheet: None,
        formula: "of:=[.$B$1]*100".into(),
        base_cell: None,
        usage: Default::default(),
    });
    data.names.push(NameData {
        name: "bad name".into(),
        sheet: None,
        formula: "of:=1".into(),
        base_cell: None,
        usage: Default::default(),
    });
    let doc = tally_engine::Document::new(
        tally_api::DocId(1),
        tally_engine::Workbook::from_data(&data),
        "t",
    );
    let info = doc.info();
    let sheet = info.sheets[0].id;
    let wb = &doc.wb;
    assert_eq!(wb.value(CellAddr::new(sheet, 0, 2)), Value::Number(10.0));
    assert_eq!(wb.value(CellAddr::new(sheet, 1, 2)), Value::Number(20.0));
    assert_eq!(wb.value(CellAddr::new(sheet, 3, 1)), Value::Number(1000.0));
    assert!(wb.names.defined().any(|(_, e)| &*e.name == "Rel2_"));
    assert!(info.warnings.iter().any(|w| w.contains("Rel2")));
    assert!(info.warnings.iter().any(|w| w.contains("bad name")));
    // The base cell round-trips.
    let out = wb.to_data();
    let rel = out.names.iter().find(|n| n.name == "Rel").unwrap();
    assert_eq!(rel.base_cell, Some((0, 0, 2)));
    assert_eq!(rel.formula, "of:=[.B1]");
}
