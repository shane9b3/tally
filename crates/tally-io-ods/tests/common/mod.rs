#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)
#![allow(dead_code)]

use std::path::PathBuf;

use tally_model::{CellContentData, CellData, StyleData, WorkbookData};

pub fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel)
}

pub fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A workbook with style indices replaced by the style data itself, so two
/// workbooks compare equal regardless of style table order.
#[derive(Debug, PartialEq)]
pub struct Normalized {
    pub wb: WorkbookData,
    pub cell_styles: Vec<Vec<Option<StyleData>>>,
}

pub fn normalize(wb: &WorkbookData) -> Normalized {
    let mut out = wb.clone();
    out.styles.clear();
    let mut cell_styles = Vec::new();
    for sheet in &mut out.sheets {
        let mut v = Vec::new();
        for c in &mut sheet.cells {
            let s = c
                .style
                .and_then(|i| wb.styles.get(i as usize))
                .filter(|s| **s != StyleData::default())
                .cloned();
            v.push(s);
            c.style = None;
        }
        cell_styles.push(v);
    }
    Normalized {
        wb: out,
        cell_styles,
    }
}

pub fn cell(wb: &WorkbookData, sheet: usize, row: u32, col: u32) -> Option<&CellData> {
    wb.sheets[sheet]
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
}

pub fn content(wb: &WorkbookData, sheet: usize, row: u32, col: u32) -> &CellContentData {
    &cell(wb, sheet, row, col)
        .unwrap_or_else(|| panic!("no cell at {sheet}!{row},{col}"))
        .content
}

pub fn style(wb: &WorkbookData, sheet: usize, row: u32, col: u32) -> Option<&StyleData> {
    cell(wb, sheet, row, col)
        .and_then(|c| c.style)
        .map(|i| &wb.styles[i as usize])
}
