#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)
#![allow(dead_code)]

use std::io::Cursor;
use std::path::PathBuf;

use tally_io_xlsx::{read_xlsx, write_xlsx, Limits};
use tally_model::{CellContentData, CellData, StyleData, WorkbookData};
use tally_types::Value;

pub fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/xlsx")
        .join(rel)
}

pub fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixture(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

pub fn open(rel: &str) -> WorkbookData {
    read_xlsx(Cursor::new(read_fixture(rel)), &Limits::default())
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

pub fn to_bytes(wb: &WorkbookData) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    write_xlsx(wb, &mut out).unwrap();
    out.into_inner()
}

pub fn round_trip(wb: &WorkbookData) -> WorkbookData {
    read_xlsx(Cursor::new(to_bytes(wb)), &Limits::default()).unwrap()
}

pub fn cell(wb: &WorkbookData, sheet: usize, a1: &str) -> Option<CellData> {
    let (row, col, _, _) = tally_types::parse_a1(a1).unwrap();
    wb.sheets[sheet]
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .cloned()
}

pub fn content(wb: &WorkbookData, sheet: usize, a1: &str) -> CellContentData {
    cell(wb, sheet, a1)
        .unwrap_or_else(|| panic!("no cell at {sheet}!{a1}"))
        .content
}

pub fn value(wb: &WorkbookData, sheet: usize, a1: &str) -> Value {
    match content(wb, sheet, a1) {
        CellContentData::Value(v) => v,
        other => panic!("{a1}: not a value: {other:?}"),
    }
}

pub fn formula(wb: &WorkbookData, sheet: usize, a1: &str) -> (String, Option<Value>) {
    match content(wb, sheet, a1) {
        CellContentData::Formula { text, cached } => (text.to_string(), cached),
        other => panic!("{a1}: not a formula: {other:?}"),
    }
}

pub fn style(wb: &WorkbookData, sheet: usize, a1: &str) -> Option<StyleData> {
    cell(wb, sheet, a1)
        .and_then(|c| c.style)
        .map(|i| wb.styles[i as usize].clone())
}

pub fn num(v: f64) -> Value {
    Value::Number(v)
}

pub fn text(s: &str) -> Value {
    Value::text(s)
}
