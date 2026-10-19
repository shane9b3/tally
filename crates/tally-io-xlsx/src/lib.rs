//! XLSX (Office Open XML spreadsheet) import and export for Tally.
//!
//! [`read_xlsx`] converts an `.xlsx` package into
//! [`tally_model::WorkbookData`] with its own streaming quick-xml reader
//! (workbook, relationships, shared strings, styles, worksheets);
//! [`write_xlsx`] writes one with `rust_xlsxwriter`. AOO 4.1 can import
//! `.xlsx` but never save it; Tally does both. See `docs/file-formats.md`
//! for what is supported in each direction, the date mapping and the known
//! gaps.
//!
//! Formulas are translated at the boundary: XLSX stores ExcelA1 text
//! (`SUM(Sheet1!A1:B2)`, `_xlfn.` prefixes), the model stores OpenFormula
//! (`of:=SUM([Sheet1.A1:.B2])`). Shared formulas (`t="shared"`) are
//! re-anchored for every member cell through the formula AST.
//!
//! Reading untrusted files is hardened exactly like `tally-io-ods` (see
//! `docs/security.md`): zip-bomb, ratio and entry-count limits enforced
//! while streaming, EOCD precheck before indexing, DOCTYPE rejection, XML
//! depth/attribute caps, a cap on materialized cells and text bytes, and
//! path-traversal-proof entry lookup (nothing is ever extracted to disk).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod archive;
mod colors;
mod dates;
mod formula;
mod numfmt;
mod read;
mod sheet;
mod sheet_v03;
mod sst;
mod styles;
mod units;
mod workbook;
mod write;
mod write_v03;
mod xml;

use std::io::{Cursor, Read, Seek, Write};

use tally_model::WorkbookData;

pub use dates::{excel_serial_to_tally, tally_serial_to_excel};
pub use numfmt::{builtin_format_code, is_date_format};

/// Excel's default column width (8.43 characters, 64 px), in 1/100 mm.
/// Columns without a `<col>` entry use it; they are not recorded in
/// [`tally_model::SheetData::col_widths`].
pub const DEFAULT_COL_WIDTH_HMM: u32 = 1693;

/// Excel's default row height (15 pt, 20 px), in 1/100 mm.
pub const DEFAULT_ROW_HEIGHT_HMM: u32 = 529;

/// The OOXML main content type of a workbook part.
pub const WORKBOOK_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";

/// Resource limits applied by [`read_xlsx`]. The defaults are the same as
/// `tally_io_ods::Limits`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of entries in the zip archive.
    pub max_entries: usize,
    /// Maximum uncompressed bytes of one worksheet or `sharedStrings.xml`
    /// (the parts that grow with the data, like ODS `content.xml`).
    pub max_sheet_xml_bytes: u64,
    /// Maximum uncompressed bytes of any other entry we read.
    pub max_entry_bytes: u64,
    /// Maximum uncompressed bytes read from the whole package.
    pub max_total_bytes: u64,
    /// Maximum compression ratio, enforced once an entry inflates past 10 MiB.
    pub max_ratio: u32,
    /// Maximum XML element nesting depth.
    pub max_xml_depth: u32,
    /// Maximum attributes on one XML element.
    pub max_attrs_per_element: u32,
    /// Maximum cells materialized into [`WorkbookData`] (all sheets).
    pub max_cells: u64,
    /// Maximum bytes of formula and string text materialized into
    /// [`WorkbookData`] (all sheets), counting every cell's full text even
    /// when cells share one shared-string allocation.
    pub max_text_bytes: u64,
    /// Maximum number of sheets.
    pub max_sheets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_sheet_xml_bytes: 512 * 1024 * 1024,
            max_entry_bytes: 64 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_ratio: 200,
            max_xml_depth: 256,
            max_attrs_per_element: 256,
            max_cells: 10_000_000,
            max_text_bytes: 1024 * 1024 * 1024,
            max_sheets: 10_000,
        }
    }
}

/// Errors from reading or writing XLSX.
#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    /// The zip container is malformed.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// An XML stream is malformed.
    #[error("XML error: {0}")]
    Xml(String),
    /// The document contains a `<!DOCTYPE>`, which OOXML never needs.
    #[error("XML DOCTYPE declarations are not allowed")]
    ForbiddenDoctype,
    /// A resource limit was hit.
    #[error("limit exceeded: {what} (limit {limit})")]
    LimitExceeded {
        /// What was limited.
        what: &'static str,
        /// The limit value.
        limit: u64,
    },
    /// The input is not an XLSX workbook.
    #[error("not an XLSX workbook")]
    NotXlsx,
    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A feature we cannot read (e.g. encryption, `.xlsb`).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The writer library rejected the workbook.
    #[error("XLSX writer error: {0}")]
    Writer(String),
}

impl From<rust_xlsxwriter::XlsxError> for XlsxError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        XlsxError::Writer(e.to_string())
    }
}

/// What [`write_xlsx`] could not write faithfully.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Human-readable warnings (formulas that could not be translated,
    /// number formats with no Excel equivalent, names that were dropped, …).
    pub warnings: Vec<String>,
}

/// Reads an XLSX package.
///
/// Cached formula results are returned in
/// [`tally_model::CellContentData::Formula`] for display before the
/// engine recalculates. Formulas that cannot be translated from ExcelA1 are
/// kept with their original text after `of:=` (the engine shows Err:5xx
/// for them) and their cached value.
pub fn read_xlsx(reader: impl Read + Seek, limits: &Limits) -> Result<WorkbookData, XlsxError> {
    read::read(reader, limits)
}

/// Replaces every [`tally_model::CellContentData::SharedFormula`] member
/// with a plain formula printed at its own cell (for consumers other than
/// the engine, which resolves shared groups itself with one parse per
/// group). Members whose group formula does not parse keep their cached
/// value.
pub fn expand_shared_formulas(wb: &mut WorkbookData) {
    use tally_model::CellContentData;
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    let r = formula::XlsxResolver::new(&names);
    for (i, sheet) in wb.sheets.iter_mut().enumerate() {
        let id = u32::try_from(i).unwrap_or(u32::MAX);
        for c in &mut sheet.cells {
            let CellContentData::SharedFormula {
                text,
                anchor_row,
                anchor_col,
                cached,
            } = &c.content
            else {
                continue;
            };
            let at = tally_types::CellAddr::new(id, *anchor_row, *anchor_col);
            let here = tally_types::CellAddr::new(id, c.row, c.col);
            let f = tally_formula::parse(text, at, tally_formula::Dialect::open_formula(), &r);
            c.content = match f {
                Ok(f) => CellContentData::Formula {
                    text: formula::print_of(&f, here, &r).into(),
                    cached: cached.clone(),
                },
                Err(_) => CellContentData::Value(cached.clone().unwrap_or_default()),
            };
        }
    }
}

/// Writes an XLSX package and reports what could not be written faithfully.
pub fn write_xlsx(
    data: &WorkbookData,
    writer: impl Write + Seek,
) -> Result<WriteReport, XlsxError> {
    write::write(data, writer)
}

/// Returns `true` if `bytes` look like an XLSX workbook: a zip whose
/// `[Content_Types].xml` and `xl/workbook.xml` entries exist (or whose
/// content types name a spreadsheet main part elsewhere).
///
/// The end-of-central-directory records are checked against the default
/// [`Limits`] first, so the zip index is only built for an archive that
/// [`read_xlsx`] would also accept. At most 1 MiB of `[Content_Types].xml`
/// is read.
pub fn sniff_is_xlsx(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let mut cur = Cursor::new(bytes);
    if archive::precheck(&mut cur, &Limits::default()).is_err() {
        return false;
    }
    let Ok(mut zip) = zip::ZipArchive::new(cur) else {
        return false;
    };
    if zip.index_for_name("xl/workbook.xml").is_some() {
        return true;
    }
    let Ok(file) = zip.by_name("[Content_Types].xml") else {
        return false;
    };
    let mut ct = Vec::new();
    if file.take(1024 * 1024).read_to_end(&mut ct).is_err() {
        return false;
    }
    let ct = String::from_utf8_lossy(&ct);
    ct.contains("spreadsheetml.sheet.main+xml") || ct.contains("spreadsheetml.template.main+xml")
}
