//! ODS (OpenDocument Spreadsheet) import and export for Tally (WP5).
//!
//! [`read_ods`] converts an ODS package into [`tally_model::WorkbookData`];
//! [`write_ods`] does the reverse. The Phase A subset is supported (values,
//! formulas with cached results, cell styles incl. number formats, column
//! widths, sheet names/order, active sheet); everything else is skipped.
//!
//! Reading untrusted files is hardened as described in `docs/security.md`:
//! zip-bomb, ratio and entry-count limits enforced while streaming, DOCTYPE
//! rejection, XML depth/attribute caps, a cap on materialized cells for
//! repeated rows/columns, and path-traversal-proof entry lookup (nothing is
//! ever extracted to disk).
//!
//! Formulas are kept exactly as stored (`of:=SUM([.A1:.A3])`, or legacy
//! `oooc:=…`); the engine's OpenFormula parser handles the namespace prefix.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod archive;
mod content;
mod numfmt;
mod read;
mod styles;
mod units;
mod write;
mod xml;

use std::io::{Cursor, Read, Seek, Write};

use tally_model::WorkbookData;

/// The ODS media type stored in the `mimetype` entry.
pub const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// LibreOffice's default column width (0.889 in), in 1/100 mm. Columns with
/// this width are not recorded in [`tally_model::SheetData::col_widths`].
pub const DEFAULT_COL_WIDTH_HMM: u32 = 2258;

/// LibreOffice's default row height (0.452 cm), in 1/100 mm.
pub const DEFAULT_ROW_HEIGHT_HMM: u32 = 452;

/// Resource limits applied by [`read_ods`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of entries in the zip archive.
    pub max_entries: usize,
    /// Maximum uncompressed bytes of `content.xml`.
    pub max_content_xml_bytes: u64,
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
    /// Maximum copies materialized for one repeated non-empty row.
    pub max_repeat: u32,
    /// Maximum bytes of formula and string text materialized into
    /// [`WorkbookData`] (all sheets). Each materialized cell counts its full
    /// text length, even when repeated cells share one allocation, so the
    /// budget reflects what the engine will hold.
    pub max_text_bytes: u64,
    /// Maximum number of sheets (`table:table` elements).
    pub max_sheets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_content_xml_bytes: 512 * 1024 * 1024,
            max_entry_bytes: 64 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_ratio: 200,
            max_xml_depth: 256,
            max_attrs_per_element: 256,
            max_cells: 10_000_000,
            max_repeat: 1_048_576,
            max_text_bytes: 1024 * 1024 * 1024,
            max_sheets: 10_000,
        }
    }
}

/// Errors from reading or writing ODS.
#[derive(Debug, thiserror::Error)]
pub enum OdsError {
    /// The zip container is malformed.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// An XML stream is malformed.
    #[error("XML error: {0}")]
    Xml(String),
    /// The document contains a `<!DOCTYPE>`, which ODF never needs.
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
    /// The input is not an ODS spreadsheet.
    #[error("not an OpenDocument spreadsheet")]
    NotOds,
    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A feature we cannot read (e.g. encryption).
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Extra information from [`write_ods_with_report`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Things that could not be written faithfully (unsupported number
    /// formats fall back to General, merges, named expressions, …).
    pub warnings: Vec<String>,
}

/// Reads an ODS package.
///
/// The workbook is always meant to be recalculated after loading; cached
/// formula results are returned in [`tally_model::CellContentData::Formula`]
/// for display before recalculation.
pub fn read_ods(reader: impl Read + Seek, limits: &Limits) -> Result<WorkbookData, OdsError> {
    read::read(reader, limits)
}

/// Writes an ODS package.
pub fn write_ods(data: &WorkbookData, writer: impl Write + Seek) -> Result<(), OdsError> {
    write::write(data, writer).map(|_| ())
}

/// Writes an ODS package and reports what could not be written faithfully.
pub fn write_ods_with_report(
    data: &WorkbookData,
    writer: impl Write + Seek,
) -> Result<WriteReport, OdsError> {
    write::write(data, writer)
}

/// Returns `true` if `bytes` look like an ODS package: a zip whose
/// `mimetype` entry is the ODS media type (or the ODS template type).
///
/// The fast path reads only the first local file header (a conforming file
/// starts with a stored `mimetype`). Otherwise the end-of-central-directory
/// records are checked against the default [`Limits`] first (entry count and
/// central-directory size, see `archive::precheck`), so the zip index is
/// only built for an archive that `read_ods` would also accept.
pub fn sniff_is_ods(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let ok = |mt: &[u8]| {
        mt == MIMETYPE.as_bytes()
            || mt == b"application/vnd.oasis.opendocument.spreadsheet-template"
    };
    // Fast path: a conforming file starts with a stored `mimetype` entry.
    if bytes.len() >= 30 {
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let u32_at = |i: usize| {
            u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize
        };
        let method = u16_at(8);
        let size = u32_at(18);
        let name_len = u16_at(26);
        let extra_len = u16_at(28);
        let name_end = 30 + name_len;
        if method == 0 && bytes.get(30..name_end) == Some(b"mimetype".as_slice()) {
            let start = name_end + extra_len;
            if let Some(mt) = bytes.get(start..start.saturating_add(size)) {
                return ok(mt);
            }
        }
    }
    // Slow path (any entry order), bounded by the EOCD precheck.
    let mut cur = Cursor::new(bytes);
    if archive::precheck(&mut cur, &Limits::default()).is_err() {
        return false;
    }
    let Ok(mut zip) = zip::ZipArchive::new(cur) else {
        return false;
    };
    let Ok(file) = zip.by_name("mimetype") else {
        return false;
    };
    let mut mt = Vec::new();
    if file.take(128).read_to_end(&mut mt).is_err() {
        return false;
    }
    ok(mt.trim_ascii())
}
