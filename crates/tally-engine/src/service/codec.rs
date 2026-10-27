//! File codecs behind a trait, so the engine builds while the IO crates are
//! being written. [`UnsupportedCodec`] is the default; with the `io` feature
//! [`IoCodec`] reads ODS/OTS, SXC (OOo 1.x), XLSX/XLSM, XLS (BIFF), HTML,
//! dBase and CSV/TSV, and writes ODS, XLSX, CSV, HTML and dBase through the
//! `tally-io-*` crates. Formats are sniffed by magic bytes first (zip
//! `mimetype`, OLE2 signature, dBase header, HTML tags), then by extension.
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

#[cfg(feature = "io")]
impl FileCodec for IoCodec {
    fn read(&self, bytes: &[u8], ext: &str) -> Result<Decoded, ApiError> {
        if tally_io_ods::sniff_is_ods(bytes) || ext == "ods" || ext == "ots" {
            let data = tally_io_ods::read_ods(
                std::io::Cursor::new(bytes),
                &tally_io_ods::Limits::default(),
            )
            .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            return Ok(Decoded {
                data,
                recognize_text: false,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if tally_io_xlsx::sniff_is_xlsx(bytes) || ext == "xlsx" || ext == "xlsm" {
            let data = tally_io_xlsx::read_xlsx(
                std::io::Cursor::new(bytes),
                &tally_io_xlsx::Limits::default(),
            )
            .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            return Ok(Decoded {
                data,
                recognize_text: false,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if tally_io_ods::sniff_is_sxc(bytes) || ext == "sxc" || ext == "stc" {
            let data = tally_io_ods::read_sxc(
                std::io::Cursor::new(bytes),
                &tally_io_ods::Limits::default(),
            )
            .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            return Ok(Decoded {
                data,
                recognize_text: false,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if tally_io_xls::sniff_is_xls(bytes) || ext == "xls" || ext == "xlt" {
            let data = tally_io_xls::read_xls(bytes, &tally_io_xls::Limits::default())
                .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            return Ok(Decoded {
                data,
                recognize_text: false,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if ext == "dbf" || (ext.is_empty() && tally_io_dbf::sniff_is_dbf(bytes)) {
            let data = tally_io_dbf::read_dbf(
                bytes,
                &tally_io_dbf::DbfOptions::default(),
                &tally_io_dbf::Limits::default(),
            )
            .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            return Ok(Decoded {
                data,
                recognize_text: false,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if matches!(ext, "html" | "htm" | "xhtml")
            || (matches!(ext, "txt" | "") && tally_io_html::sniff_is_html(bytes))
        {
            let (mut data, warnings) = tally_io_html::read_html(bytes)
                .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
            data.warnings.extend(warnings);
            // Cells are text: the document locale's recognizer makes
            // numbers and dates of them (formulas stay text).
            return Ok(Decoded {
                data,
                recognize_text: true,
                evaluate_formulas: false,
                csv_options: None,
            });
        }
        if matches!(ext, "csv" | "tsv" | "txt" | "tab" | "") {
            let delimiter = if ext == "tsv" || ext == "tab" {
                b'\t'
            } else {
                tally_io_csv::detect_delimiter(&bytes[..bytes.len().min(64 * 1024)])
            };
            let opts = tally_io_csv::CsvOptions {
                delimiter,
                ..tally_io_csv::CsvOptions::default()
            };
            let sheet = tally_io_csv::read_csv(bytes, &opts)
                .map_err(|e| ApiError::new(ApiError::BAD_FILE, e.to_string()))?;
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
            return Ok(Decoded {
                data: WorkbookData {
                    sheets: vec![sheet],
                    ..WorkbookData::default()
                },
                recognize_text: true,
                evaluate_formulas: opts.evaluate_formulas,
                csv_options,
            });
        }
        Err(ApiError::unsupported(format!(
            "unsupported file type .{ext}"
        )))
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
