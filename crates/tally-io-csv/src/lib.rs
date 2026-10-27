//! CSV import and export for Tally (WP5).
//!
//! [`read_csv`] decodes bytes (BOM sniffing, strict UTF-8, Windows-1252
//! fallback), parses them with the `csv` crate and produces a
//! [`tally_model::SheetData`] whose cells are raw
//! `CellContentData::Value(Value::Text(..))` strings; the engine runs the
//! input recognizer on them. [`write_csv`] writes formatted cell text with the
//! formula-injection guard on by default (see `docs/security.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::borrow::Cow;
use std::io::Write;

use tally_model::{CellContentData, CellData, SheetData};
use tally_types::{Value, MAX_COLS, MAX_ROWS};

/// Text encoding of CSV input.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    /// BOM sniffing (UTF-8, UTF-16 LE/BE), then BOM-less UTF-16 detection,
    /// then strict UTF-8, else Windows-1252.
    #[default]
    Auto,
    /// UTF-8 (a BOM is removed; invalid sequences become U+FFFD).
    Utf8,
    /// ISO-8859-1: every byte is the code point of the same value.
    Latin1,
    /// Windows-1252 (Latin-1 plus `€`, curly quotes, … in 0x80–0x9F).
    Windows1252,
    /// UTF-16; endianness from the BOM, little-endian without one.
    Utf16,
}

/// Options for [`read_csv`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvOptions {
    /// Field separator (default `,`). See [`detect_delimiter`].
    pub delimiter: u8,
    /// Quote character (default `"`); quotes inside quoted fields are doubled.
    pub quote: u8,
    /// Whether the first row is a header. Informational only: the header row
    /// is imported as ordinary cells.
    pub has_header_row: bool,
    /// Input encoding.
    pub encoding: Encoding,
    /// Trim leading/trailing whitespace from fields.
    pub trim: bool,
    /// Drop rows whose fields are all empty (otherwise they stay as blank rows).
    pub skip_empty_rows: bool,
    /// Maximum number of rows.
    pub max_rows: u32,
    /// Maximum number of fields in one row.
    pub max_cols: u32,
    /// Maximum number of non-empty cells.
    pub max_cells: u64,
    /// Whether the importer should turn fields starting with `=` into live
    /// formulas (AOO's "Evaluate formulas", off by default). [`read_csv`]
    /// always returns raw text; the engine reads this flag when it runs the
    /// input recognizer, and with `false` `=1+2` stays the text `=1+2`.
    pub evaluate_formulas: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
            has_header_row: false,
            encoding: Encoding::Auto,
            trim: false,
            skip_empty_rows: false,
            max_rows: MAX_ROWS,
            max_cols: MAX_COLS,
            max_cells: 10_000_000,
            evaluate_formulas: false,
        }
    }
}

/// Errors from CSV import/export.
#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    /// The CSV parser or writer failed.
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    /// An I/O error while writing.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A limit from [`CsvOptions`] was exceeded.
    /// Displayed like the ODS/XLSX readers' limit errors
    /// (`limit exceeded: … (<what> over <limit>)`), which the app localises.
    #[error(
        "limit exceeded: the file is too large or complex to open safely ({what} over {limit})"
    )]
    LimitExceeded {
        /// What was limited.
        what: &'static str,
        /// The limit value.
        limit: u64,
    },
}

/// Decodes CSV bytes according to `encoding`.
pub fn decode(bytes: &[u8], encoding: Encoding) -> Cow<'_, str> {
    match encoding {
        Encoding::Utf8 => encoding_rs::UTF_8.decode_with_bom_removal(bytes).0,
        Encoding::Latin1 => Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect()),
        Encoding::Windows1252 => {
            encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(bytes)
                .0
        }
        Encoding::Utf16 => {
            if bytes.starts_with(&[0xfe, 0xff]) {
                encoding_rs::UTF_16BE.decode_with_bom_removal(bytes).0
            } else {
                encoding_rs::UTF_16LE.decode_with_bom_removal(bytes).0
            }
        }
        Encoding::Auto => {
            if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                return encoding_rs::UTF_8.decode_with_bom_removal(bytes).0;
            }
            if bytes.starts_with(&[0xff, 0xfe]) {
                return encoding_rs::UTF_16LE.decode_with_bom_removal(bytes).0;
            }
            if bytes.starts_with(&[0xfe, 0xff]) {
                return encoding_rs::UTF_16BE.decode_with_bom_removal(bytes).0;
            }
            if let Some(le) = looks_like_utf16(bytes) {
                let enc = if le {
                    encoding_rs::UTF_16LE
                } else {
                    encoding_rs::UTF_16BE
                };
                return enc.decode_without_bom_handling(bytes).0;
            }
            match std::str::from_utf8(bytes) {
                Ok(s) => Cow::Borrowed(s),
                Err(_) => {
                    encoding_rs::WINDOWS_1252
                        .decode_without_bom_handling(bytes)
                        .0
                }
            }
        }
    }
}

/// The encoding [`Encoding::Auto`] picks for `bytes`: UTF-8 (with a BOM or
/// valid), UTF-16 (a BOM or the NUL pattern), else Windows-1252. A later
/// CSV save of the document reuses it.
pub fn detect_encoding(bytes: &[u8]) -> Encoding {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Encoding::Utf8;
    }
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Encoding::Utf16;
    }
    if looks_like_utf16(bytes).is_some() {
        return Encoding::Utf16;
    }
    if std::str::from_utf8(bytes).is_ok() {
        Encoding::Utf8
    } else {
        Encoding::Windows1252
    }
}

/// BOM-less UTF-16 heuristic: mostly-ASCII text has a NUL in every other
/// byte. Returns `Some(true)` for little-endian.
fn looks_like_utf16(bytes: &[u8]) -> Option<bool> {
    let sample = &bytes[..bytes.len().min(512) & !1];
    if sample.len() < 4 {
        return None;
    }
    let pairs = sample.len() / 2;
    let even_nul = sample.iter().step_by(2).filter(|&&b| b == 0).count();
    let odd_nul = sample
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|&&b| b == 0)
        .count();
    if odd_nul * 10 >= pairs * 9 && even_nul == 0 {
        Some(true)
    } else if even_nul * 10 >= pairs * 9 && odd_nul == 0 {
        Some(false)
    } else {
        None
    }
}

/// Guesses the field separator from a sample (the first 20 lines are
/// examined): of `,` `;` tab and `|`, the one that occurs the same non-zero
/// number of times (outside quotes) on the most lines wins; ties go to the
/// higher count, then to that order. Defaults to `,`.
pub fn detect_delimiter(sample: &[u8]) -> u8 {
    const CANDIDATES: [char; 4] = [',', ';', '\t', '|'];
    let text = decode(sample, Encoding::Auto);
    let mut counts: Vec<[usize; 4]> = Vec::new();
    let mut cur = [0usize; 4];
    let mut in_quotes = false;
    let mut line_has_content = false;
    for ch in text.chars() {
        if counts.len() >= 20 {
            break;
        }
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                line_has_content = true;
            }
            '\n' if !in_quotes => {
                if line_has_content {
                    counts.push(cur);
                }
                cur = [0; 4];
                line_has_content = false;
            }
            '\r' => {}
            c => {
                line_has_content = true;
                if !in_quotes {
                    if let Some(i) = CANDIDATES.iter().position(|&d| d == c) {
                        cur[i] += 1;
                    }
                }
            }
        }
    }
    if line_has_content && counts.len() < 20 {
        counts.push(cur);
    }
    let mut best: Option<(usize, usize, usize)> = None; // (consistent lines, mode, index)
    for i in 0..CANDIDATES.len() {
        let mut freq: Vec<(usize, usize)> = Vec::new(); // (count value, lines)
        for line in &counts {
            let c = line[i];
            if c == 0 {
                continue;
            }
            match freq.iter_mut().find(|(v, _)| *v == c) {
                Some(slot) => slot.1 += 1,
                None => freq.push((c, 1)),
            }
        }
        let Some(&(mode, lines)) = freq.iter().max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
        else {
            continue;
        };
        let better = match best {
            None => true,
            Some((bl, bm, _)) => lines > bl || (lines == bl && mode > bm),
        };
        if better {
            best = Some((lines, mode, i));
        }
    }
    best.map_or(b',', |(_, _, i)| CANDIDATES[i] as u8)
}

/// Parses CSV bytes into a sheet of raw text cells.
///
/// Rows and columns are 0-based; empty fields produce no cell; blank lines
/// produce blank rows unless [`CsvOptions::skip_empty_rows`] is set. The
/// sheet name is left empty for the caller to fill in.
pub fn read_csv(bytes: &[u8], opts: &CsvOptions) -> Result<SheetData, CsvError> {
    let text = decode(bytes, opts.encoding);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(opts.delimiter)
        .quote(opts.quote)
        .has_headers(false)
        .flexible(true)
        .trim(if opts.trim {
            csv::Trim::Fields
        } else {
            csv::Trim::None
        })
        .from_reader(text.as_bytes());
    let mut sheet = SheetData::default();
    let mut row: u64 = 0;
    // csv-core silently drops blank lines; we restore them as blank rows by
    // counting the line breaks between the end of one record's content and
    // the start of the next.
    let raw = text.as_bytes();
    let is_break = |b: u8| b == b'\r' || b == b'\n';
    let mut prev_end = 0usize; // reader byte offset after the previous record
    let mut prev_start = 0usize; // content start of the previous record
    let mut first = true;
    let mut cells: u64 = 0;
    let mut record = csv::StringRecord::new();
    // Repeated field texts (categories, dates, codes) share one allocation.
    let mut texts = TextInterner::default();
    while reader.read_record(&mut record)? {
        let end = usize::try_from(reader.position().byte())
            .unwrap_or(usize::MAX)
            .min(raw.len());
        let mut start = prev_end.min(end);
        while start < end && is_break(raw[start]) {
            start += 1;
        }
        let mut gap_from = prev_end.min(start);
        if !first {
            while gap_from > prev_start && is_break(raw[gap_from - 1]) {
                gap_from -= 1;
            }
        }
        let breaks = count_line_breaks(&raw[gap_from..start]);
        let blank = if first {
            breaks
        } else {
            breaks.saturating_sub(1)
        };
        if !opts.skip_empty_rows {
            row += blank;
        }
        first = false;
        prev_end = end;
        prev_start = start;

        if opts.skip_empty_rows && record.iter().all(str::is_empty) {
            continue;
        }
        if row >= u64::from(opts.max_rows) {
            return Err(CsvError::LimitExceeded {
                what: "rows",
                limit: u64::from(opts.max_rows),
            });
        }
        if record.len() > opts.max_cols as usize {
            return Err(CsvError::LimitExceeded {
                what: "columns",
                limit: u64::from(opts.max_cols),
            });
        }
        for (col, field) in record.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            cells += 1;
            if cells > opts.max_cells {
                return Err(CsvError::LimitExceeded {
                    what: "cells",
                    limit: opts.max_cells,
                });
            }
            sheet.cells.push(CellData {
                row: row as u32,
                col: col as u32,
                content: CellContentData::Value(Value::Text(texts.get(field))),
                style: None,
            });
        }
        row += 1;
    }
    Ok(sheet)
}

/// Shares the allocation of repeated field texts while reading. At most
/// [`TextInterner::CAP`] distinct texts are remembered (after that, new
/// texts are allocated as usual), and texts longer than
/// [`TextInterner::MAX_LEN`] bytes are never remembered.
#[derive(Default)]
struct TextInterner {
    seen: std::collections::HashSet<std::sync::Arc<str>>,
}

impl TextInterner {
    const CAP: usize = 1 << 16;
    const MAX_LEN: usize = 64;

    fn get(&mut self, s: &str) -> std::sync::Arc<str> {
        if s.len() > Self::MAX_LEN {
            return s.into();
        }
        if let Some(t) = self.seen.get(s) {
            return t.clone();
        }
        let t: std::sync::Arc<str> = s.into();
        if self.seen.len() < Self::CAP {
            self.seen.insert(t.clone());
        }
        t
    }
}

/// Counts line breaks (`\r\n`, `\n` or a lone `\r`).
fn count_line_breaks(s: &[u8]) -> u64 {
    let mut n = 0;
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'\r' => {
                n += 1;
                if s.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'\n' => n += 1,
            _ => {}
        }
        i += 1;
    }
    n
}

/// One cell to export: the formatted text, and whether the cell holds a
/// text value (numbers, booleans, dates and errors are not text).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CsvCell {
    /// The text to write.
    pub text: String,
    /// `true` if the cell's value is text. Informational only: the injection
    /// guard decides on the output text, because a number format can make a
    /// number display as `=HYPERLINK(…)` (see [`needs_injection_guard`]).
    pub is_text: bool,
}

impl CsvCell {
    /// A text cell.
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            is_text: true,
        }
    }

    /// A non-text cell (number, date, boolean, error) shown as `s`.
    pub fn value(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            is_text: false,
        }
    }
}

/// Record terminator for [`write_csv`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// `\r\n` (RFC 4180).
    #[default]
    Crlf,
    /// `\n`.
    Lf,
}

/// Options for [`write_csv`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvWriteOptions {
    /// Field separator (default `,`).
    pub delimiter: u8,
    /// Quote character (default `"`).
    pub quote: u8,
    /// Quote every field, not just those that need it.
    pub quote_all: bool,
    /// Record terminator.
    pub line_ending: LineEnding,
    /// Prefix `'` to fields that a spreadsheet would run as a formula
    /// (default `true`; see `docs/security.md`). Turn it off for raw output.
    pub injection_guard: bool,
    /// Quote every text cell ([`CsvCell::is_text`]), as AOO's "Quote all
    /// text cells"; other fields are quoted only when they need it.
    pub quote_text: bool,
    /// Never quote (AOO's empty text delimiter): fields are written as
    /// they are, even when they hold the delimiter or a line break.
    pub no_quotes: bool,
}

impl Default for CsvWriteOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
            quote_all: false,
            line_ending: LineEnding::Crlf,
            injection_guard: true,
            quote_text: false,
            no_quotes: false,
        }
    }
}

/// Characters that make spreadsheet applications treat a CSV field as a
/// formula (or DDE command) when it is opened, including the full-width
/// forms some CJK-locale importers normalize (U+FF1D, U+FF0B, U+FF0D, U+FF20).
const INJECTION_TRIGGERS: [char; 8] = ['=', '+', '-', '@', '＝', '＋', '－', '＠'];

/// Whether the guard would prefix the output field `text` with `'`.
///
/// The decision is made on the text that is written, whatever the cell's
/// value type: a field is guarded when it starts with tab or CR, or when its
/// first character after any leading whitespace (including LF) and `"` is
/// one of `= + - @` or their full-width forms. A field that is entirely a
/// plain numeric literal ([`is_plain_number`], e.g. `-5`, `-1,234.50`,
/// `+1e-3`) is never guarded, since importers read it as a number.
pub fn needs_injection_guard(text: &str) -> bool {
    if text.starts_with(['\t', '\r']) {
        return true;
    }
    // Some importers trim spaces (LibreOffice "trim spaces") or strip
    // leading quotes before the formula check.
    let rest = text.trim_start_matches(|c: char| c.is_whitespace() || c == '"');
    let trigger = rest
        .chars()
        .next()
        .is_some_and(|c| INJECTION_TRIGGERS.contains(&c) || c == '\t' || c == '\r');
    trigger && !is_plain_number(text)
}

/// `true` for a plain numeric literal, optionally surrounded by spaces:
/// an optional sign, digits with optional `,` thousands separators, at most
/// one decimal point, an optional exponent and an optional trailing `%`.
pub fn is_plain_number(text: &str) -> bool {
    let b = text.trim_matches(' ').as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b',' && digits > 0)) {
        if b[i].is_ascii_digit() {
            digits += 1;
        }
        i += 1;
    }
    if i > 0 && b[i - 1] == b',' {
        return false; // trailing separator
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if b.get(i) == Some(&b'%') {
        i += 1;
    }
    i == b.len()
}

/// Writes rows of formatted cells as CSV.
///
/// Fields containing the delimiter, the quote character or a line break are
/// quoted, with embedded quotes doubled; [`CsvWriteOptions::quote_all`]
/// quotes every field, [`CsvWriteOptions::quote_text`] every text cell and
/// [`CsvWriteOptions::no_quotes`] none. A row holding one empty field is
/// written as `""` (unless quoting is off), an empty row as a bare line
/// break. With [`CsvWriteOptions::injection_guard`], every field for which
/// [`needs_injection_guard`] holds gets a `'` prefix, whatever the cell's
/// type (a number format can display a number as formula text).
pub fn write_csv<W: Write>(
    rows: impl Iterator<Item = Vec<CsvCell>>,
    opts: &CsvWriteOptions,
    w: W,
) -> Result<(), CsvError> {
    let mut w = std::io::BufWriter::new(w);
    let terminator: &[u8] = match opts.line_ending {
        LineEnding::Crlf => b"\r\n",
        LineEnding::Lf => b"\n",
    };
    let q = opts.quote;
    for row in rows {
        let single = row.len() == 1;
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                w.write_all(&[opts.delimiter])?;
            }
            let text: Cow<'_, str> = if opts.injection_guard && needs_injection_guard(&cell.text) {
                Cow::Owned(format!("'{}", cell.text))
            } else {
                Cow::Borrowed(cell.text.as_str())
            };
            let quote = !opts.no_quotes
                && (opts.quote_all
                    || (opts.quote_text && cell.is_text)
                    || (single && text.is_empty())
                    || text
                        .bytes()
                        .any(|b| b == opts.delimiter || b == q || b == b'\r' || b == b'\n'));
            if quote {
                w.write_all(&[q])?;
                let bytes = text.as_bytes();
                let mut from = 0;
                for (j, &b) in bytes.iter().enumerate() {
                    if b == q {
                        w.write_all(&bytes[from..=j])?;
                        w.write_all(&[q])?;
                        from = j + 1;
                    }
                }
                w.write_all(&bytes[from..])?;
                w.write_all(&[q])?;
            } else {
                w.write_all(text.as_bytes())?;
            }
        }
        w.write_all(terminator)?;
    }
    w.flush()?;
    Ok(())
}

/// Character set of CSV output.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputCharset {
    /// UTF-8 without a byte-order mark.
    #[default]
    Utf8,
    /// UTF-16 little-endian with a byte-order mark.
    Utf16Le,
    /// Windows-1252; unmappable characters become `?`.
    Windows1252,
    /// ISO-8859-1 (Latin-1); characters above U+00FF become `?`.
    Iso88591,
}

/// How many characters of `text` `charset` cannot hold (written as `?`).
pub fn unmappable_count(text: &str, charset: OutputCharset) -> usize {
    match charset {
        OutputCharset::Utf8 | OutputCharset::Utf16Le => 0,
        OutputCharset::Iso88591 => text.chars().filter(|c| u32::from(*c) > 0xFF).count(),
        OutputCharset::Windows1252 => {
            let mut buf = [0u8; 4];
            text.chars()
                .filter(|c| {
                    !c.is_ascii() && encoding_rs::WINDOWS_1252.encode(c.encode_utf8(&mut buf)).2
                })
                .count()
        }
    }
}

/// Encodes UTF-8 CSV output in `charset`.
pub fn encode_output(utf8: &[u8], charset: OutputCharset) -> Vec<u8> {
    let text = String::from_utf8_lossy(utf8);
    match charset {
        OutputCharset::Utf8 => text.into_owned().into_bytes(),
        OutputCharset::Utf16Le => {
            let mut out = Vec::with_capacity(2 + text.len() * 2);
            out.extend_from_slice(&[0xFF, 0xFE]);
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        OutputCharset::Iso88591 => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect(),
        OutputCharset::Windows1252 => {
            let mut out = Vec::with_capacity(text.len());
            let mut buf = [0u8; 4];
            for c in text.chars() {
                let s = c.encode_utf8(&mut buf);
                let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(s);
                if unmappable {
                    out.push(b'?');
                } else {
                    out.extend_from_slice(&bytes);
                }
            }
            out
        }
    }
}
