//! Cross-format: XLSX fixture → `WorkbookData` → ODS (tally-io-ods) →
//! `WorkbookData`: values, formulas and cached results agree, and the ODS
//! result writes back to XLSX with the same ExcelA1 formulas.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

mod common;

use std::io::Cursor;

use common::*;
use tally_model::{CellContentData, WorkbookData};
use tally_types::Value;

fn via_ods(wb: &WorkbookData) -> WorkbookData {
    let mut buf = Cursor::new(Vec::new());
    tally_io_ods::write_ods(wb, &mut buf).unwrap();
    tally_io_ods::read_ods(
        Cursor::new(buf.into_inner()),
        &tally_io_ods::Limits::default(),
    )
    .unwrap()
}

/// Values compare exactly, except that an empty string read from ODS may
/// come back as an empty cell.
fn same_content(a: &CellContentData, b: &CellContentData) -> bool {
    match (a, b) {
        (
            CellContentData::Formula { text, cached },
            CellContentData::Formula {
                text: t2,
                cached: c2,
            },
        ) => text == t2 && (cached == c2 || cached.is_none() || c2.is_none()),
        (x, y) => x == y,
    }
}

#[test]
fn xlsx_to_ods_and_back() {
    for fx in ["excel365-basic.xlsx", "libreoffice-style.xlsx"] {
        let wb = open(fx);
        let ods = via_ods(&wb);
        assert_eq!(ods.sheets.len(), wb.sheets.len(), "{fx}");
        let mut compared = 0;
        let mut formulas = 0;
        for (a, b) in wb.sheets.iter().zip(&ods.sheets) {
            assert_eq!(a.name, b.name, "{fx}");
            for c in &a.cells {
                if matches!(c.content, CellContentData::Value(Value::Empty)) {
                    continue;
                }
                let got = b
                    .cells
                    .iter()
                    .find(|x| x.row == c.row && x.col == c.col)
                    .unwrap_or_else(|| {
                        panic!("{fx}: {}!R{}C{} missing in ODS", a.name, c.row, c.col)
                    });
                assert!(
                    same_content(&c.content, &got.content),
                    "{fx}: {}!R{}C{}: {:?} vs {:?}",
                    a.name,
                    c.row,
                    c.col,
                    c.content,
                    got.content
                );
                if matches!(c.content, CellContentData::Formula { .. }) {
                    formulas += 1;
                }
                compared += 1;
            }
        }
        assert!(
            compared > 10 && formulas > 3,
            "{fx}: {compared} cells, {formulas} formulas"
        );
        // Number formats survive (styles may be re-ordered by the ODS writer).
        let fmt = |w: &WorkbookData, a1: &str| style(w, 0, a1).and_then(|s| s.number_format);
        assert_eq!(fmt(&wb, "B2"), fmt(&ods, "B2"), "{fx}");
        assert_eq!(
            wb.settings.calc_settings.date_epoch,
            ods.settings.calc_settings.date_epoch
        );

        // ... and the ODS-read workbook writes to XLSX with the same formulas.
        let again = round_trip(&ods);
        for (a, b) in ods.sheets.iter().zip(&again.sheets) {
            for c in &a.cells {
                if let CellContentData::Formula { text, .. } = &c.content {
                    let got = b
                        .cells
                        .iter()
                        .find(|x| x.row == c.row && x.col == c.col)
                        .unwrap();
                    match &got.content {
                        CellContentData::Formula { text: t2, .. } => assert_eq!(text, t2),
                        other => panic!("{fx}: {other:?}"),
                    }
                }
            }
        }
    }
}

#[test]
fn excel_1904_via_ods_keeps_dates() {
    let wb = open("excel-1904.xlsx");
    let ods = via_ods(&wb);
    assert_eq!(
        ods.settings.calc_settings.date_epoch,
        tally_types::DateEpoch::D1904
    );
    assert_eq!(value(&ods, 0, "B1"), num(44852.0));
}
