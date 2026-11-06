//! File codecs behind a trait, so the engine builds while the IO crates are
//! being written. [`UnsupportedCodec`] is the default; with the `io` feature
//! [`IoCodec`] reads ODS/OTS, flat ODS (`.fods`), SXC/STC (OOo 1.x),
//! XLSX/XLSM/XLTX/XLTM, XLS/XLT (BIFF), Excel 2003 XML, SYLK, DIF, HTML,
//! dBase and CSV/TSV, and writes ODS, XLSX, CSV, HTML and dBase through the
//! `tally-io-*` crates. Formats are sniffed by their content first (zip
//! `mimetype`, OLE2 signature, XML roots, SYLK/DIF headers, dBase header,
//! HTML tags), then by extension ([`detect`]).
//!
//! Wiring (one line in the shell):
//! `let svc = EngineService::new().with_codec(Box::new(tally_engine::service::IoCodec));`
//! or simply `EngineService::with_default_codec()`.

use tally_api::{ApiError, FileFormat};
use tally_model::WorkbookData;

/// What a codec read from a file.
#[derive(Clone, Debug)]
pub struct Decoded {
    /// The workbook.
    pub data: WorkbookData,
    /// Text values are raw input to run through the recognizer (CSV).
    pub recognize_text: bool,
    /// With `recognize_text`: `=…` text becomes a formula (CSV
    /// `evaluate_formulas`; off by default, so `=1+2` stays text).
    pub evaluate_formulas: bool,
    /// CSV: the delimiters and character set found on import, which a
    /// later CSV save reuses (AOO keeps the import's filter options).
    pub csv_options: Option<tally_api::CsvExportOptions>,
    /// The file holds macros (VBA, Basic) that Tally drops.
    pub has_macros: bool,
}

/// What a codec needs to write a file.
#[derive(Clone, Debug)]
pub struct Export<'a> {
    /// The workbook.
    pub data: &'a WorkbookData,
    /// For CSV: the active sheet's rows as formatted text, with a flag
    /// telling whether the cell holds text (informational; the injection
    /// guard judges the text itself).
    pub csv_rows: Option<Vec<Vec<(String, bool)>>>,
    /// For CSV: the delimiters, quoting and character set.
    pub csv_options: Option<tally_api::CsvExportOptions>,
}

/// Reads and writes files.
pub trait FileCodec: Send + Sync {
    /// Decodes `bytes`; `ext` is the lowercase file extension (may be empty).
    fn read(&self, bytes: &[u8], ext: &str) -> Result<Decoded, ApiError>;
    /// Encodes the workbook in `format`.
    fn write(&self, export: &Export<'_>, format: FileFormat) -> Result<Vec<u8>, ApiError>;
}

/// A codec that supports nothing (the default until the IO crates are wired).
#[derive(Copy, Clone, Debug, Default)]
pub struct UnsupportedCodec;

impl FileCodec for UnsupportedCodec {
    fn read(&self, _bytes: &[u8], ext: &str) -> Result<Decoded, ApiError> {
        Err(ApiError::unsupported(format!(
            "opening .{ext} files is not wired in this build (enable tally-engine's `io` feature)"
        )))
    }

    fn write(&self, _export: &Export<'_>, format: FileFormat) -> Result<Vec<u8>, ApiError> {
        Err(ApiError::unsupported(format!(
            "saving {format:?} is not wired in this build (enable tally-engine's `io` feature)"
        )))
    }
}

/// ODS, XLSX and CSV through the IO crates.
#[cfg(feature = "io")]
#[derive(Copy, Clone, Debug, Default)]
pub struct IoCodec;

/// What [`IoCodec`] reads a file as ([`detect`]).
#[cfg(feature = "io")]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// ODS / OTS package.
    Ods,
    /// Flat XML ODF spreadsheet (`.fods`).
    Fods,
    /// XLSX / XLSM / XLTX / XLTM package.
    Xlsx,
    /// OOo 1.x package (`.sxc`, `.stc`).
    Sxc,
    /// Excel 97–2003 (BIFF in an OLE2 compound file).
    Xls,
    /// Excel 2003 XML (SpreadsheetML).
    Xmlss,
    /// SYLK.
    Sylk,
    /// Data Interchange Format.
    Dif,
    /// dBase.
    Dbf,
    /// HTML tables.
    Html,
    /// CSV / TSV / text.
    Csv,
}

/// What `bytes` with lowercase extension `ext` are read as: the content
/// first (zip `mimetype` / content types, the OLE2 signature, the flat ODF
/// and SpreadsheetML roots, the SYLK `ID;` and DIF `TABLE` headers), then
/// the extension (AOO's type detection also prefers the content). The SYLK
/// and DIF headers are not trusted on a CSV-family file: `ID;Price;…` is a
/// semicolon CSV. `None`: not a format Tally reads.
#[cfg(feature = "io")]
pub fn detect(bytes: &[u8], ext: &str) -> Option<Kind> {
    let text_family = matches!(ext, "csv" | "tsv" | "txt" | "tab");
    let sniffed = if tally_io_ods::sniff_is_ods(bytes) {
        Some(Kind::Ods)
    } else if tally_io_xlsx::sniff_is_xlsx(bytes) {
        Some(Kind::Xlsx)
    } else if tally_io_ods::sniff_is_sxc(bytes) {
        Some(Kind::Sxc)
    } else if tally_io_xls::sniff_is_xls(bytes) {
        Some(Kind::Xls)
    } else if tally_io_ods::sniff_is_fods(bytes) {
        Some(Kind::Fods)
    } else if tally_io_xmlss::sniff_is_xmlss(bytes) {
        Some(Kind::Xmlss)
    } else if !text_family
        && tally_io_sylk::sniff_is_sylk(bytes)
        && (!ext.is_empty() || has_sylk_cells(bytes))
    {
        Some(Kind::Sylk)
    } else if !text_family && tally_io_dif::sniff_is_dif(bytes) {
        Some(Kind::Dif)
    } else {
        None
    };
    if sniffed.is_some() {
        return sniffed;
    }
    Some(match ext {
        "ods" | "ots" => Kind::Ods,
        "fods" => Kind::Fods,
        "xlsx" | "xlsm" | "xltx" | "xltm" => Kind::Xlsx,
        "sxc" | "stc" => Kind::Sxc,
        "xls" | "xlt" | "xlw" => Kind::Xls,
        "xml" => Kind::Xmlss,
        "slk" => Kind::Sylk,
        "dif" => Kind::Dif,
        "dbf" => Kind::Dbf,
        "" if tally_io_dbf::sniff_is_dbf(bytes) => Kind::Dbf,
        "html" | "htm" | "xhtml" => Kind::Html,
        "txt" | "" if tally_io_html::sniff_is_html(bytes) => Kind::Html,
        "csv" | "tsv" | "txt" | "tab" | "" => Kind::Csv,
        _ => return None,
    })
}

/// An extension-less file starting `ID;` is SYLK only if a `C;` record
/// follows in its first 64 KiB: `ID;Name;Price` is a semicolon CSV.
#[cfg(feature = "io")]
fn has_sylk_cells(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(64 * 1024)]
        .windows(3)
        .any(|w| (w[0] == b'\n' || w[0] == b'\r') && w[1] == b'C' && w[2] == b';')
}

#[cfg(feature = "io")]
fn bad_file(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ApiError::BAD_FILE, e.to_string())
}

#[cfg(feature = "io")]
impl Decoded {
    /// A workbook read as stored (no text recognition).
    fn plain(data: WorkbookData, has_macros: bool) -> Self {
        Self {
            data,
            recognize_text: false,
            evaluate_formulas: false,
            csv_options: None,
            has_macros,
        }
    }
}

#[cfg(feature = "io")]
impl FileCodec for IoCodec {
    fn read(&self, bytes: &[u8], ext: &str) -> Result<Decoded, ApiError> {
        let Some(kind) = detect(bytes, ext) else {
            return Err(ApiError::unsupported(format!(
                "unsupported file type .{ext}"
            )));
        };
        match kind {
            Kind::Ods => {
                let data = tally_io_ods::read_ods(
                    std::io::Cursor::new(bytes),
                    &tally_io_ods::Limits::default(),
                )
                .map_err(bad_file)?;
                Ok(Decoded::plain(data, tally_io_ods::has_macros(bytes)))
            }
            Kind::Fods => {
                let data = tally_io_ods::read_fods(bytes, &tally_io_ods::Limits::default())
                    .map_err(bad_file)?;
                Ok(Decoded::plain(data, tally_io_ods::fods_has_macros(bytes)))
            }
            Kind::Xlsx => {
                let data = tally_io_xlsx::read_xlsx(
                    std::io::Cursor::new(bytes),
                    &tally_io_xlsx::Limits::default(),
                )
                .map_err(bad_file)?;
                Ok(Decoded::plain(data, tally_io_xlsx::has_macros(bytes)))
            }
            Kind::Sxc => {
                let data = tally_io_ods::read_sxc(
                    std::io::Cursor::new(bytes),
                    &tally_io_ods::Limits::default(),
                )
                .map_err(bad_file)?;
                Ok(Decoded::plain(data, tally_io_ods::has_macros(bytes)))
            }
            Kind::Xls => {
                let data = tally_io_xls::read_xls(bytes, &tally_io_xls::Limits::default())
                    .map_err(bad_file)?;
                Ok(Decoded::plain(data, tally_io_xls::has_macros(bytes)))
            }
            Kind::Xmlss => {
                let data = tally_io_xmlss::read_xmlss(bytes, &tally_io_xmlss::Limits::default())
                    .map_err(bad_file)?;
                Ok(Decoded::plain(data, false))
            }
            Kind::Sylk => {
                let data = tally_io_sylk::read_sylk(bytes, &tally_io_sylk::Limits::default())
                    .map_err(bad_file)?;
                Ok(Decoded::plain(data, false))
            }
            Kind::Dif => {
                let data = tally_io_dif::read_dif(bytes, &tally_io_dif::Limits::default())
                    .map_err(bad_file)?;
                Ok(Decoded::plain(data, false))
            }
            Kind::Dbf => {
                let data = tally_io_dbf::read_dbf(
                    bytes,
                    &tally_io_dbf::DbfOptions::default(),
                    &tally_io_dbf::Limits::default(),
                )
                .map_err(bad_file)?;
                Ok(Decoded::plain(data, false))
            }
            Kind::Html => {
                let (mut data, warnings) = tally_io_html::read_html(bytes).map_err(bad_file)?;
                data.warnings.extend(warnings);
                // Cells are text: the document locale's recognizer makes
                // numbers and dates of them (formulas stay text).
                Ok(Decoded {
                    recognize_text: true,
                    ..Decoded::plain(data, false)
                })
            }
            Kind::Csv => {
                let delimiter = if ext == "tsv" || ext == "tab" {
                    b'\t'
                } else {
                    tally_io_csv::detect_delimiter(&bytes[..bytes.len().min(64 * 1024)])
                };
                let opts = tally_io_csv::CsvOptions {
                    delimiter,
                    ..tally_io_csv::CsvOptions::default()
                };
                let sheet = tally_io_csv::read_csv(bytes, &opts).map_err(bad_file)?;
                let charset = match tally_io_csv::detect_encoding(bytes) {
                    tally_io_csv::Encoding::Utf16 => tally_api::CsvCharset::Utf16,
                    tally_io_csv::Encoding::Windows1252 => tally_api::CsvCharset::Windows1252,
                    tally_io_csv::Encoding::Latin1 => tally_api::CsvCharset::Iso88591,
                    _ => tally_api::CsvCharset::Utf8,
                };
                let csv_options = Some(tally_api::CsvExportOptions {
                    field_delimiter: char::from(delimiter).to_string(),
                    text_delimiter: char::from(opts.quote).to_string(),
                    charset,
                    ..tally_api::CsvExportOptions::default()
                })
                .filter(|o| o.delimiters().is_ok());
                Ok(Decoded {
                    data: WorkbookData {
                        sheets: vec![sheet],
                        ..WorkbookData::default()
                    },
                    recognize_text: true,
                    evaluate_formulas: opts.evaluate_formulas,
                    csv_options,
                    has_macros: false,
                })
            }
        }
    }

    fn write(&self, export: &Export<'_>, format: FileFormat) -> Result<Vec<u8>, ApiError> {
        match format {
            FileFormat::Ods => {
                let mut cur = std::io::Cursor::new(Vec::new());
                tally_io_ods::write_ods(export.data, &mut cur)
                    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
                Ok(cur.into_inner())
            }
            FileFormat::Csv => {
                let rows = export.csv_rows.clone().unwrap_or_default();
                let o = export.csv_options.clone().unwrap_or_default();
                let (delimiter, text) = o.delimiters()?;
                let opts = tally_io_csv::CsvWriteOptions {
                    delimiter,
                    quote: text.unwrap_or(b'"'),
                    no_quotes: text.is_none(),
                    quote_text: o.quote_all_text,
                    ..tally_io_csv::CsvWriteOptions::default()
                };
                let charset = match o.charset {
                    tally_api::CsvCharset::Utf8 => tally_io_csv::OutputCharset::Utf8,
                    tally_api::CsvCharset::Utf16 => tally_io_csv::OutputCharset::Utf16Le,
                    tally_api::CsvCharset::Windows1252 => tally_io_csv::OutputCharset::Windows1252,
                    tally_api::CsvCharset::Iso88591 => tally_io_csv::OutputCharset::Iso88591,
                };
                let mut out = Vec::new();
                tally_io_csv::write_csv(
                    rows.into_iter().map(|row| {
                        row.into_iter()
                            .map(|(text, is_text)| {
                                if is_text {
                                    tally_io_csv::CsvCell::text(text)
                                } else {
                                    tally_io_csv::CsvCell::value(text)
                                }
                            })
                            .collect()
                    }),
                    &opts,
                    &mut out,
                )
                .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
                Ok(match charset {
                    tally_io_csv::OutputCharset::Utf8 => out,
                    cs => tally_io_csv::encode_output(&out, cs),
                })
            }
            FileFormat::Xlsx => {
                let mut cur = std::io::Cursor::new(Vec::new());
                tally_io_xlsx::write_xlsx(export.data, &mut cur)
                    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
                Ok(cur.into_inner())
            }
            FileFormat::Html => tally_io_html::write_html(export.data)
                .map(String::into_bytes)
                .map_err(|e| ApiError::new(ApiError::IO, e.to_string())),
            FileFormat::Dbf => {
                tally_io_dbf::write_dbf(export.data, &tally_io_dbf::DbfOptions::default())
                    .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))
            }
        }
    }
}

/// Warnings for a CSV export with `o`: fields that break the file without a
/// text delimiter, and characters the character set cannot hold (`?`).
pub fn csv_warnings(rows: &[Vec<(String, bool)>], o: &tally_api::CsvExportOptions) -> Vec<String> {
    let mut out = Vec::new();
    if o.text_delimiter.is_empty() {
        let d = o.field_delimiter.chars().next().unwrap_or(',');
        let broken = rows
            .iter()
            .flatten()
            .filter(|(t, _)| t.contains(d) || t.contains(['\r', '\n']))
            .count();
        if broken > 0 {
            out.push(format!(
                "{broken} cells contain the field delimiter or a line break, and no text \
                 delimiter is set: the CSV file will not read back as this sheet."
            ));
        }
    }
    #[cfg(feature = "io")]
    {
        let charset = match o.charset {
            tally_api::CsvCharset::Windows1252 => Some(tally_io_csv::OutputCharset::Windows1252),
            tally_api::CsvCharset::Iso88591 => Some(tally_io_csv::OutputCharset::Iso88591),
            _ => None,
        };
        if let Some(cs) = charset {
            let n: usize = rows
                .iter()
                .flatten()
                .map(|(t, _)| tally_io_csv::unmappable_count(t, cs))
                .sum();
            if n > 0 {
                out.push(format!(
                    "{n} characters cannot be written in the chosen character set and were \
                     saved as \"?\"."
                ));
            }
        }
    }
    out
}
