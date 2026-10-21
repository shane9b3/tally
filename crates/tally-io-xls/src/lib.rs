//! Legacy Excel (BIFF5/BIFF8 `.xls`) import through
//! [calamine](https://crates.io/crates/calamine) (wave 5, W5-3).
//!
//! What is read: sheet names, order and visibility, cell values (numbers,
//! text, booleans, errors, dates and times as serials with a date or time
//! number format), the cached results of formulas together with the
//! formula text calamine decodes (ExcelA1, converted to OpenFormula by
//! `tally_io_xlsx::excel_formula_to_of`; a formula calamine cannot decode
//! keeps its cached value), merged areas and the 1904 date system.
//! Column widths, styles and names are not exposed by calamine and are
//! not read. Export is not supported (docs/parity-deviations.md).
//!
//! Hardening (docs/security.md): the file must start with the OLE2
//! signature and be at most [`Limits::max_bytes`]; sheet, cell and text
//! budgets are enforced while materializing; calamine's decoder runs under
//! `catch_unwind` so a malformed stream is an error, never a crash.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use calamine::{Data, Reader, Xls, XlsOptions};
use tally_model::{
    CellContentData, CellData, CellRangeLocal, DocSettings, SheetData, StyleData, WorkbookData,
};
use tally_types::{DateEpoch, ErrorKind, Value, MAX_COLS, MAX_ROWS};

/// OLE2 compound file signature.
pub const OLE2_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Resource limits of [`read_xls`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest file accepted (256 MiB).
    pub max_bytes: u64,
    /// Most sheets.
    pub max_sheets: usize,
    /// Most cells materialized (all sheets).
    pub max_cells: u64,
    /// Most bytes of text and formula text materialized.
    pub max_text_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_sheets: 1024,
            max_cells: 10_000_000,
            max_text_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Errors of [`read_xls`].
#[derive(Debug, thiserror::Error)]
pub enum XlsError {
    /// Not an OLE2 file.
    #[error("not an Excel 97-2003 workbook")]
    NotXls,
    /// A limit was hit.
    #[error("limit exceeded: {what} (limit {limit})")]
    LimitExceeded {
        /// What.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
    /// calamine could not decode the file.
    #[error("malformed workbook: {0}")]
    Malformed(String),
}

/// `true` if `bytes` start with the OLE2 signature (`.xls`, but also
/// `.doc`/`.ppt`; the reader checks for a workbook stream).
pub fn sniff_is_xls(bytes: &[u8]) -> bool {
    bytes.starts_with(&OLE2_MAGIC)
}

fn err_kind(e: calamine::CellErrorType) -> ErrorKind {
    use calamine::CellErrorType as C;
    match e {
        C::Div0 => ErrorKind::Div0,
        C::NA => ErrorKind::NA,
        C::Name => ErrorKind::Name,
        C::Null => ErrorKind::Null,
        C::Num => ErrorKind::Num,
        C::Ref => ErrorKind::Ref,
        C::Value => ErrorKind::Value,
        C::GettingData => ErrorKind::NA,
    }
}

/// Reads a `.xls` workbook.
pub fn read_xls(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, XlsError> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(XlsError::LimitExceeded {
            what: "file size",
            limit: limits.max_bytes,
        });
    }
    if !sniff_is_xls(bytes) {
        return Err(XlsError::NotXls);
    }
    let data = bytes.to_vec();
    let result = catch_unwind(AssertUnwindSafe(|| decode(data, limits)));
    match result {
        Ok(r) => r,
        Err(_) => Err(XlsError::Malformed(
            "the BIFF stream could not be decoded".into(),
        )),
    }
}

fn decode(data: Vec<u8>, limits: &Limits) -> Result<WorkbookData, XlsError> {
    let mut wb: Xls<Cursor<Vec<u8>>> =
        Xls::new_with_options(Cursor::new(data), XlsOptions::default())
            .map_err(|e| XlsError::Malformed(e.to_string()))?;
    let date1904 = wb.has_1904_epoch();
    let metas: Vec<calamine::Sheet> = wb.sheets_metadata().to_vec();
    let names: Vec<String> = metas.iter().map(|s| s.name.clone()).collect();
    if names.len() > limits.max_sheets {
        return Err(XlsError::LimitExceeded {
            what: "sheets",
            limit: limits.max_sheets as u64,
        });
    }
    let mut styles: Vec<StyleData> = Vec::new();
    let mut style_index = |code: &str| -> u32 {
        if let Some(i) = styles
            .iter()
            .position(|s| s.number_format.as_deref() == Some(code))
        {
            return i as u32;
        }
        styles.push(StyleData {
            number_format: Some(code.to_string()),
            ..StyleData::default()
        });
        (styles.len() - 1) as u32
    };
    let mut cells_total: u64 = 0;
    let mut text_total: u64 = 0;
    let mut sheets: Vec<SheetData> = Vec::with_capacity(names.len());
    let mut warnings: Vec<String> = Vec::new();
    let mut formula_failures = 0usize;
    for (si, meta) in metas.iter().enumerate() {
        let name = meta.name.clone();
        let range = wb
            .worksheet_range(&name)
            .map_err(|e| XlsError::Malformed(e.to_string()))?;
        let formulas = wb.worksheet_formula(&name).ok();
        let mut sheet = SheetData {
            name: name.clone(),
            hidden: !matches!(meta.visible, calamine::SheetVisible::Visible),
            ..SheetData::default()
        };
        for (r, c, value) in range.used_cells() {
            let (r, c) = (r as u32, c as u32);
            if r >= MAX_ROWS || c >= MAX_COLS {
                continue;
            }
            // calamine reports an undecodable formula as text starting
            // with "Unrecognised formula": keep the cached value then.
            let formula_text = formulas
                .as_ref()
                .and_then(|f| f.get_value((r, c)))
                .filter(|t| !t.is_empty())
                .filter(|t| {
                    if t.starts_with("Unrecognised formula") {
                        formula_failures += 1;
                        false
                    } else {
                        true
                    }
                });
            let (val, format): (Value, Option<&str>) = match value {
                Data::Empty => (Value::Empty, None),
                Data::Int(i) => (Value::Number(*i as f64), None),
                Data::Float(f) => (Value::Number(*f), None),
                Data::String(s) => (Value::text(s.as_str()), None),
                Data::Bool(b) => (Value::Bool(*b), None),
                Data::Error(e) => (Value::Error(err_kind(e.clone())), None),
                Data::DateTime(dt) => {
                    let serial = tally_io_xlsx::excel_serial_to_tally(dt.as_f64(), date1904);
                    let fmt = if dt.is_duration() {
                        "[HH]:MM:SS"
                    } else if serial.fract() == 0.0 {
                        "YYYY-MM-DD"
                    } else if serial.abs() < 1.0 {
                        "HH:MM:SS"
                    } else {
                        "YYYY-MM-DD HH:MM:SS"
                    };
                    (Value::Number(serial), Some(fmt))
                }
                Data::DateTimeIso(s) | Data::DurationIso(s) => (Value::text(s.as_str()), None),
            };
            if val.is_empty() && formula_text.is_none() {
                continue;
            }
            cells_total += 1;
            if cells_total > limits.max_cells {
                return Err(XlsError::LimitExceeded {
                    what: "cells",
                    limit: limits.max_cells,
                });
            }
            if let Value::Text(t) = &val {
                text_total += t.len() as u64;
            }
            let content = match formula_text {
                Some(f) => {
                    text_total += f.len() as u64;
                    match tally_io_xlsx::excel_formula_to_of(f, &names, si, r, c) {
                        Some(of) => CellContentData::Formula {
                            text: Arc::from(of.as_str()),
                            cached: (!val.is_empty()).then(|| val.clone()),
                        },
                        None => {
                            formula_failures += 1;
                            CellContentData::Value(val)
                        }
                    }
                }
                None => CellContentData::Value(val),
            };
            if text_total > limits.max_text_bytes {
                return Err(XlsError::LimitExceeded {
                    what: "text bytes",
                    limit: limits.max_text_bytes,
                });
            }
            sheet.cells.push(CellData {
                row: r,
                col: c,
                content,
                style: format.map(&mut style_index),
            });
        }
        if let Ok(merges) = wb.merge_cells_by_sheet_name(&name) {
            for d in merges.iter().take(100_000) {
                let (r0, c0) = d.start;
                let (r1, c1) = d.end;
                if r1 > r0 || c1 > c0 {
                    if r1 >= MAX_ROWS || c1 >= MAX_COLS {
                        continue;
                    }
                    sheet.merges.push(CellRangeLocal { r0, c0, r1, c1 });
                }
            }
        }
        sheets.push(sheet);
    }
    if formula_failures > 0 {
        warnings.push(format!(
            "{formula_failures} formula(s) could not be converted and were imported as values."
        ));
    }
    let mut settings = DocSettings::default();
    if date1904 {
        settings.calc_settings.date_epoch = DateEpoch::D1904;
    }
    Ok(WorkbookData {
        sheets,
        styles,
        settings,
        warnings,
        ..WorkbookData::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_ole_and_oversize() {
        assert!(matches!(
            read_xls(b"PK\x03\x04", &Limits::default()),
            Err(XlsError::NotXls)
        ));
        let small = Limits {
            max_bytes: 4,
            ..Default::default()
        };
        assert!(matches!(
            read_xls(&OLE2_MAGIC, &small),
            Err(XlsError::LimitExceeded { .. })
        ));
        // OLE2 magic alone is malformed, not a panic.
        assert!(matches!(
            read_xls(&OLE2_MAGIC, &Limits::default()),
            Err(XlsError::Malformed(_))
        ));
    }
}
