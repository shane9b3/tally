#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)
#![allow(dead_code)]

use std::collections::BTreeMap;
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
/// workbooks compare equal regardless of style table order, and with the
/// formatting compared as each column's effective styles: a blank styled
/// cell and a one-row style run are the same thing.
#[derive(Debug, PartialEq)]
pub struct Normalized {
    pub wb: WorkbookData,
    pub cell_styles: Vec<Vec<Option<StyleData>>>,
    /// Per sheet and column: `(first row, last row, style)` runs.
    pub formatting: Vec<BTreeMap<u32, ColumnFormat>>,
}

/// A column's effective styles: `(first row, last row, style)` runs.
pub type ColumnFormat = Vec<(u32, u32, StyleData)>;

/// Sets rows `r0..=r1` of a column's runs to `st` (`None` clears).
fn paint(
    runs: &mut BTreeMap<u32, (u32, Option<StyleData>)>,
    r0: u32,
    r1: u32,
    st: Option<StyleData>,
) {
    let cut: Vec<(u32, (u32, Option<StyleData>))> = runs
        .range(..=r1)
        .filter(|(_, (e, _))| *e >= r0)
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    for (a, (e, s)) in cut {
        runs.remove(&a);
        if a < r0 {
            runs.insert(a, (r0 - 1, s.clone()));
        }
        if e > r1 {
            runs.insert(r1 + 1, (e, s));
        }
    }
    runs.insert(r0, (r1, st));
}

pub fn normalize(wb: &WorkbookData) -> Normalized {
    let mut out = wb.clone();
    out.styles.clear();
    let data = |i: Option<u32>| {
        i.and_then(|i| wb.styles.get(i as usize))
            .filter(|s| **s != StyleData::default())
            .cloned()
    };
    let mut cell_styles = Vec::new();
    let mut formatting = Vec::new();
    for sheet in &mut out.sheets {
        // Runs first, then the cells' own styles (as the engine loads them).
        let mut cols: BTreeMap<u32, BTreeMap<u32, (u32, Option<StyleData>)>> = BTreeMap::new();
        for r in &sheet.style_runs {
            paint(
                cols.entry(r.col).or_default(),
                r.start_row,
                r.end_row,
                data(Some(r.style)),
            );
        }
        for c in &sheet.cells {
            if c.style.is_some() {
                paint(cols.entry(c.col).or_default(), c.row, c.row, data(c.style));
            }
        }
        let mut f: BTreeMap<u32, ColumnFormat> = BTreeMap::new();
        for (col, runs) in cols {
            let mut v: ColumnFormat = Vec::new();
            for (a, (e, st)) in runs {
                let Some(st) = st else { continue };
                match v.last_mut() {
                    Some(l) if l.2 == st && l.1 + 1 == a => l.1 = e,
                    _ => v.push((a, e, st)),
                }
            }
            if !v.is_empty() {
                f.insert(col, v);
            }
        }
        formatting.push(f);
        sheet.style_runs.clear();
        sheet
            .cells
            .retain(|c| !matches!(c.content, CellContentData::Value(tally_types::Value::Empty)));
        let mut v = Vec::new();
        for c in &mut sheet.cells {
            v.push(data(c.style));
            c.style = None;
        }
        cell_styles.push(v);
    }
    Normalized {
        wb: out,
        cell_styles,
        formatting,
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
