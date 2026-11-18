//! Legacy Excel (BIFF5/BIFF8 `.xls`) import (wave 5, W5-3).
//!
//! Values and formulas are read through
//! [calamine](https://crates.io/crates/calamine): sheet names, order and
//! visibility, cell values (numbers, text, booleans, errors), the cached
//! results of formulas together with the formula text calamine decodes
//! (ExcelA1, converted to OpenFormula by
//! `tally_io_xlsx::excel_formula_to_of`; a formula calamine cannot decode
//! keeps its cached value), merged areas and the 1904 date system.
//!
//! The look of a BIFF8 workbook, which calamine does not expose, is read
//! by this crate's own formatting reader (`format.rs`) from the same
//! stream: the width of every column and the height of every row as
//! OpenOffice 4.1 computes them from `COLINFO`, `DEFCOLWIDTH`,
//! `STANDARDWIDTH`, `ROW` and `DEFAULTROWHEIGHT`, hidden rows and columns,
//! fonts, fills, borders, alignment, number formats and cell protection
//! (`FONT`, `FORMAT`, `XF`, `PALETTE` and the XF index of every cell
//! record, `BLANK` and `MULBLANK` included), column and row default
//! formats, sheet view settings (grid lines, headers, zero values, frozen
//! panes, split, zoom, tab colour, the active sheet), the page setup
//! (paper, orientation, scale or fit to pages, margins, header and footer,
//! print options, manual page breaks, print area and print titles), PNG
//! and JPEG pictures with their anchors, and cell notes. It maps onto the
//! model the way the XLSX reader maps the equivalent constructs and never
//! fails a file: what it cannot read is skipped with a warning in
//! [`WorkbookData::warnings`](tally_model::WorkbookData::warnings), and
//! empty cells that only carry a format get the room the value cells
//! leave under [`Limits::max_cells`]. The same pass reads the defined names
//! (`NAME`; `names.rs`) and the cell hyperlinks (`HLINK`; `hlink.rs`).
//! BIFF5 files open with their values only. Not read: rich text inside a
//! cell, defined names whose formula uses functions, named styles, charts
//! and shapes (docs/file-formats.md). Export is not supported
//! (docs/parity-deviations.md).
//!
//! Hardening (docs/security.md): the file must start with the OLE2
//! signature and be at most [`Limits::max_bytes`]. Before calamine runs,
//! [`prescan`] extracts the workbook stream with strict chain walks,
//! rebuilds a canonical compound file around it, neutralizes the record
//! fields calamine allocates from without a bound and refuses files over
//! the dense-area, cell, text and formula-work budgets (calamine
//! allocates eagerly and an allocation failure aborts). Sheet, cell and
//! text budgets are enforced again while materializing; calamine's
//! decoder runs under `catch_unwind` so a malformed stream is an error,
//! never a crash. The formatting reader works on the extracted stream
//! with the pre-scan's record framing, checks every length, caps every
//! count by a field of [`Limits`] and is linear in the stream size.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::io::Cursor;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use calamine::{Data, Reader, Xls, XlsOptions};

mod format;
mod hlink;
mod names;
pub mod prescan;
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
    /// Most `XF` (cell format) records read; later ones are the default
    /// look.
    pub max_xfs: usize,
    /// Most `FONT` records read.
    pub max_fonts: usize,
    /// Most `FORMAT` (number format) records read.
    pub max_number_formats: usize,
    /// Most `COLINFO` records read per sheet (a BIFF8 sheet has 256
    /// columns).
    pub max_col_infos: usize,
    /// Most `ROW` records read (all sheets).
    pub max_row_records: usize,
    /// Most cell records whose format is kept (all sheets), the empty
    /// cells a row's default format adds included; the cells beyond keep
    /// their values and get the default look.
    pub max_styled_cells: u64,
    /// Most bytes of number format codes and font names kept in the cell
    /// styles (every style owns a copy of its own); the formats beyond
    /// are read without them.
    pub max_style_text_bytes: u64,
    /// Most manual page breaks read per sheet and direction.
    pub max_page_breaks: usize,
    /// Most pictures placed on sheets (all sheets).
    pub max_images: usize,
    /// Largest single picture.
    pub max_image_bytes: u64,
    /// Most picture bytes (every placement counts its picture).
    pub max_total_image_bytes: u64,
    /// Most bytes of sheet drawing data (`MSODRAWING`) looked at (all
    /// sheets).
    pub max_drawing_bytes: usize,
    /// Most cell notes read (all sheets).
    pub max_notes: usize,
    /// Most `NAME` records read (defined names; OpenOffice reads 65,535).
    pub max_names: usize,
    /// Most bytes of defined names and their formulas kept.
    pub max_name_bytes: u64,
    /// Most cells with a hyperlink (all sheets).
    pub max_links: usize,
    /// Most bytes of hyperlink targets kept (a link over a range shares
    /// one target).
    pub max_link_text_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_sheets: 1024,
            max_cells: 10_000_000,
            max_text_bytes: 512 * 1024 * 1024,
            max_xfs: 65_536,
            max_fonts: 65_536,
            max_number_formats: 16_384,
            max_col_infos: 256,
            max_row_records: 1_048_576,
            max_styled_cells: 10_000_000,
            max_style_text_bytes: 16 * 1024 * 1024,
            max_page_breaks: 4_096,
            max_images: 1_000,
            max_image_bytes: 16 * 1024 * 1024,
            max_total_image_bytes: tally_model::MAX_TOTAL_IMAGE_BYTES as u64,
            max_drawing_bytes: 64 * 1024 * 1024,
            max_notes: 65_536,
            max_names: 65_535,
            max_name_bytes: 16 * 1024 * 1024,
            max_links: 1_000_000,
            max_link_text_bytes: 16 * 1024 * 1024,
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

/// Returns `true` if the workbook holds macros Tally drops: a VBA project
/// (a `_VBA_PROJECT_CUR` or `_VBA_PROJECT` storage in the compound file's
/// directory, walked like the workbook stream) or an Excel 4.0 macro sheet
/// (a `BOUNDSHEET` record of type 1 in the workbook globals). A file whose
/// container does not parse has none (it will not open either).
pub fn has_macros(bytes: &[u8]) -> bool {
    if !sniff_is_xls(bytes) {
        return false;
    }
    let Ok(names) = prescan::directory_names(bytes) else {
        return false;
    };
    if names.iter().any(|n| {
        n.eq_ignore_ascii_case("_VBA_PROJECT_CUR") || n.eq_ignore_ascii_case("_VBA_PROJECT")
    }) {
        return true;
    }
    let Ok(stream) = prescan::extract_workbook_stream(bytes) else {
        return false;
    };
    // The globals: records up to the first EOF (0x000A).
    let mut pos = 0usize;
    while let Some(h) = stream.get(pos..pos + 4) {
        let typ = u16::from_le_bytes([h[0], h[1]]);
        let len = usize::from(u16::from_le_bytes([h[2], h[3]]));
        let body = stream.get(pos + 4..pos + 4 + len).unwrap_or_default();
        match typ {
            0x000A => break,
            // BOUNDSHEET: lbPlyPos (4), hsState (1), dt (1): 1 = macro sheet.
            0x0085 if body.get(5) == Some(&1) => return true,
            _ => {}
        }
        pos += 4 + len;
    }
    false
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
    let scanned = catch_unwind(AssertUnwindSafe(|| {
        prescan::sanitize_with_offset(bytes, limits.max_sheets, limits.max_cells)
    }));
    let (data, stream_at, scan) = match scanned {
        Ok(Ok((data, at, scan))) => (data, at, scan),
        Ok(Err(prescan::Refusal::Malformed(m))) => return Err(XlsError::Malformed(m)),
        Ok(Err(prescan::Refusal::Budget { what, limit })) => {
            return Err(XlsError::LimitExceeded { what, limit })
        }
        Err(_) => {
            return Err(XlsError::Malformed(
                "the compound file could not be read".into(),
            ))
        }
    };
    // The look of the workbook, from the same validated stream. This pass
    // degrades instead of failing: at worst the file opens with its
    // values only and a warning.
    let formatting = catch_unwind(AssertUnwindSafe(|| {
        format::read(data.get(stream_at..).unwrap_or_default(), limits)
    }))
    .unwrap_or_else(|_| format::failed());
    // Empty cells that only carry a format, and column formats, get the
    // room the value cells leave (the pre-scan counted those, each record
    // once, so never fewer than are materialized): formatting never makes
    // a file exceed the cell limit.
    let blank_budget = limits.max_cells.saturating_sub(scan.cells);
    let result = catch_unwind(AssertUnwindSafe(|| {
        decode(data, formatting, limits, blank_budget)
    }));
    match result {
        // What the pre-scan had to leave out (damaged names and records).
        Ok(r) => r.map(|mut wb| {
            wb.warnings.extend(scan.warnings());
            wb
        }),
        Err(_) => Err(XlsError::Malformed(
            "the BIFF stream could not be decoded".into(),
        )),
    }
}

/// The style table while cells are materialized: the formatting pass's
/// styles plus the date formats guessed for cells without a usable one.
struct Styles {
    styles: Vec<StyleData>,
    /// (base style, format code) → style.
    guessed: std::collections::HashMap<(Option<u32>, &'static str), u32>,
    /// The default look as a style of its own, once a cell needs it.
    plain: Option<u32>,
}

impl Styles {
    /// The style `base` with the number format `code`.
    fn with_format(&mut self, base: Option<u32>, code: &'static str) -> u32 {
        if let Some(i) = self.guessed.get(&(base, code)) {
            return *i;
        }
        let mut style = base
            .and_then(|b| self.styles.get(b as usize).cloned())
            .unwrap_or_default();
        style.number_format = Some(code.to_string());
        self.styles.push(style);
        let i = (self.styles.len() - 1) as u32;
        self.guessed.insert((base, code), i);
        i
    }

    /// The default look, for a cell in a column with a format: a cell
    /// without a style would take the column's.
    fn plain(&mut self) -> u32 {
        if let Some(i) = self.plain {
            return i;
        }
        self.styles.push(StyleData::default());
        let i = (self.styles.len() - 1) as u32;
        self.plain = Some(i);
        i
    }

    /// The style of a cell with the format `info`; in a column with a
    /// format (`covered`) the default look is a style too.
    fn of(&mut self, info: Option<&format::XfInfo>, covered: bool) -> Option<u32> {
        info.and_then(|x| x.style)
            .or_else(|| covered.then(|| self.plain()))
    }
}

/// The empty cells that only carry a format, within the room the value
/// cells leave.
struct Blanks {
    left: u64,
    dropped: bool,
}

impl Blanks {
    /// Adds an empty cell with the format `info` (a `BLANK` or `MULBLANK`
    /// record, a value calamine dropped, or a cell of a row with a
    /// default format) if the format shows, or if the cell is in a column
    /// with a format (`covered`), whose look it must not take.
    fn add(
        &mut self,
        cells: &mut Vec<CellData>,
        styles: &mut Styles,
        info: Option<&format::XfInfo>,
        covered: bool,
        (row, col): (u32, u32),
    ) {
        if info.is_none_or(|x| x.plain) && !covered {
            return;
        }
        let Some(style) = styles.of(info, covered) else {
            return;
        };
        if self.left == 0 {
            self.dropped = true;
            return;
        }
        self.left -= 1;
        cells.push(CellData {
            row,
            col,
            content: CellContentData::Value(Value::Empty),
            style: Some(style),
        });
    }
}

fn decode(
    data: Vec<u8>,
    mut formatting: format::Formatting,
    limits: &Limits,
    blank_budget: u64,
) -> Result<WorkbookData, XlsError> {
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
    // The formatting pass lists one entry per BOUNDSHEET, calamine one
    // sheet per BOUNDSHEET: they pair up by index. If they ever disagree
    // the formatting cannot be attributed and is dropped.
    if formatting.sheets.len() != metas.len() && !formatting.sheets.is_empty() {
        formatting = format::unattributable();
    }
    let mut warnings: Vec<String> = std::mem::take(&mut formatting.warnings);
    let mut sheet_formats = std::mem::take(&mut formatting.sheets).into_iter();
    let mut styles = Styles {
        styles: std::mem::take(&mut formatting.styles),
        guessed: std::collections::HashMap::new(),
        plain: None,
    };
    let mut blanks = Blanks {
        left: blank_budget,
        dropped: false,
    };
    let mut cells_total: u64 = 0;
    let mut text_total: u64 = 0;
    let mut sheets: Vec<SheetData> = Vec::with_capacity(names.len());
    let mut formula_failures = 0usize;
    let mut links_dropped = false;
    for (si, meta) in metas.iter().enumerate() {
        let name = meta.name.clone();
        let range = wb
            .worksheet_range(&name)
            .map_err(|e| XlsError::Malformed(e.to_string()))?;
        let formulas = wb.worksheet_formula(&name).ok();
        let mut sf = sheet_formats.next().unwrap_or_default();
        let mut sheet = SheetData {
            name: name.clone(),
            hidden: !matches!(meta.visible, calamine::SheetVisible::Visible),
            ..SheetData::default()
        };
        let styled_cols = std::mem::take(&mut sf.styled_cols);
        let covered = |c: u32| styled_cols.get(c as usize).copied().unwrap_or(false);
        // The lists are row-major: the formats are merged in as the cells
        // go by.
        let mut formats = sf.cells.iter().peekable();
        // `used_cells()` yields positions relative to the range's start
        // (its first used row and column), not sheet coordinates; merges
        // and `Range::get_value` are absolute. Values and formulas are
        // walked together: a formula whose cell has no value (a string
        // result without its `STRING` record) is a cell too.
        let (row0, col0) = range.start().unwrap_or((0, 0));
        let at = |r: usize, c: usize, (r0, c0): (u32, u32)| {
            (
                (r as u64).saturating_add(u64::from(r0)),
                (c as u64).saturating_add(u64::from(c0)),
            )
        };
        let mut values = range
            .used_cells()
            .map(|(r, c, v)| (at(r, c, (row0, col0)), v))
            .peekable();
        let formula_start = formulas
            .as_ref()
            .and_then(calamine::Range::start)
            .unwrap_or((0, 0));
        let mut formula_cells = formulas
            .iter()
            .flat_map(calamine::Range::used_cells)
            .map(|(r, c, t)| (at(r, c, formula_start), t))
            .peekable();
        let empty = Data::Empty;
        loop {
            let ((r, c), value, formula) = match (
                values.peek().map(|v| v.0),
                formula_cells.peek().map(|f| f.0),
            ) {
                (None, None) => break,
                (Some(v), Some(f)) if v == f => (
                    v,
                    values.next().map_or(&empty, |x| x.1),
                    formula_cells.next().map(|x| x.1),
                ),
                (Some(v), Some(f)) if v < f => (v, values.next().map_or(&empty, |x| x.1), None),
                (Some(v), None) => (v, values.next().map_or(&empty, |x| x.1), None),
                (_, Some(f)) => (f, &empty, formula_cells.next().map(|x| x.1)),
            };
            if r >= u64::from(MAX_ROWS) || c >= u64::from(MAX_COLS) {
                continue;
            }
            let (r, c) = (r as u32, c as u32);
            while let Some(f) = formats.next_if(|f| (f.row, u32::from(f.col)) < (r, c)) {
                let col = u32::from(f.col);
                blanks.add(
                    &mut sheet.cells,
                    &mut styles,
                    formatting.xf(f.xf),
                    covered(col),
                    (f.row, col),
                );
            }
            let xf = formats
                .next_if(|f| (f.row, u32::from(f.col)) == (r, c))
                .and_then(|f| formatting.xf(f.xf))
                .copied();
            // calamine reports an undecodable formula as text starting
            // with "Unrecognised formula": keep the cached value then.
            let formula_text = formula.filter(|t| !t.is_empty()).filter(|t| {
                if t.starts_with("Unrecognised formula") {
                    formula_failures += 1;
                    false
                } else {
                    true
                }
            });
            // A number under a real number format: a date format makes it
            // a date serial (the 1900 leap-year shift), any other format
            // leaves it alone, as in the XLSX reader.
            let formatted = xf.filter(|x| x.has_format);
            let number = |raw: f64| match formatted {
                Some(x) if x.date => tally_io_xlsx::excel_serial_to_tally(raw, date1904),
                _ => raw,
            };
            let (val, guess): (Value, Option<&'static str>) = match value {
                Data::Empty => (Value::Empty, None),
                Data::Int(i) => (Value::Number(number(*i as f64)), None),
                Data::Float(f) => (Value::Number(number(*f)), None),
                Data::String(s) => (Value::text(s.as_str()), None),
                Data::Bool(b) => (Value::Bool(*b), None),
                Data::Error(e) => (Value::Error(err_kind(e.clone())), None),
                Data::DateTime(dt) if formatted.is_some() => {
                    (Value::Number(number(dt.as_f64())), None)
                }
                Data::DateTime(dt) => {
                    // No usable number format (a code Tally cannot
                    // translate, or a BIFF5 file): guess one from what
                    // calamine found. The 1900 leap-year shift applies to
                    // dates only: a time or duration below 1 day (or any
                    // duration) keeps its serial (correctness review 4,
                    // H10).
                    let raw = dt.as_f64();
                    let serial = if dt.is_duration() || raw < 1.0 {
                        raw
                    } else {
                        tally_io_xlsx::excel_serial_to_tally(raw, date1904)
                    };
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
                blanks.add(
                    &mut sheet.cells,
                    &mut styles,
                    xf.as_ref(),
                    covered(c),
                    (r, c),
                );
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
            let base = styles.of(xf.as_ref(), covered(c));
            sheet.cells.push(CellData {
                row: r,
                col: c,
                content,
                style: match guess {
                    Some(code) => Some(styles.with_format(base, code)),
                    None => base,
                },
            });
        }
        for f in formats {
            let col = u32::from(f.col);
            blanks.add(
                &mut sheet.cells,
                &mut styles,
                formatting.xf(f.xf),
                covered(col),
                (f.row, col),
            );
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
        sheet.col_widths = sf.col_widths;
        sheet.row_heights = sf.row_heights;
        sheet.view_options = sf.view_options;
        sheet.frozen = sf.frozen;
        sheet.split = sf.split;
        sheet.zoom = sf.zoom;
        sheet.tab_color = sf.tab_color;
        sheet.images = sf.images;
        sheet.notes = sf.notes;
        // Hyperlinks count as cells, like the XLSX reader's.
        let mut links = sf.links;
        if links.len() as u64 > blanks.left {
            links.truncate(blanks.left as usize);
            links_dropped = true;
        }
        blanks.left -= links.len() as u64;
        sheet.links = links;
        sheet.page_setup = sf.page_setup;
        sheet.row_breaks = sf.row_breaks;
        sheet.col_breaks = sf.col_breaks;
        // Column default formats count as cells, like the XLSX reader's.
        if sf.style_runs.len() as u64 <= blanks.left {
            blanks.left -= sf.style_runs.len() as u64;
            sheet.style_runs = sf.style_runs;
        } else {
            blanks.dropped = true;
        }
        sheets.push(sheet);
    }
    // Defined names, now that the sheets have their names.
    let (defined_names, names_skipped) = formatting.names.resolve(&names);
    if names_skipped > 0 {
        warnings.push(format!(
            "{names_skipped} defined name(s) with a formula Tally does not read from an .xls \
             file (functions, references to other workbooks or to deleted sheets) were left out."
        ));
    }
    if formula_failures > 0 {
        warnings.push(format!(
            "{formula_failures} formula(s) could not be converted and were imported as values."
        ));
    }
    if blanks.dropped {
        warnings.push(format!(
            "The file has more than {} cells with its formatted empty cells: the formatting of \
             some empty cells was left out.",
            limits.max_cells
        ));
    }
    if links_dropped {
        warnings.push(format!(
            "The file has more than {} cells with its hyperlinks: some hyperlinks were left out.",
            limits.max_cells
        ));
    }
    let mut settings = DocSettings::default();
    if date1904 {
        settings.calc_settings.date_epoch = DateEpoch::D1904;
    }
    settings.default_font = formatting.default_font.take();
    if let Some(active) = formatting
        .active_sheet
        .filter(|a| (*a as usize) < sheets.len())
    {
        settings.active_sheet = active;
    }
    Ok(WorkbookData {
        sheets,
        styles: styles.styles,
        names: defined_names,
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
