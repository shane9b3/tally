//! SYLK (Symbolic Link, `.slk`) import, as AOO 4.1's
//! `ScImportExport::Sylk2Doc` (`main/sc/source/ui/docshell/impex.cxx`).
//!
//! A SYLK file is a list of text records, one per line, whose fields are
//! separated by `;`:
//!
//! - `ID;P<program>`: the producer. `ID;PCALCOOO32` (AOO/OOo 3.2+) and
//!   `ID;PSCALC3` (StarCalc 3, older OOo) are AOO's own files: their
//!   formulas are AOO's native A1 syntax. Every other producer (Excel's
//!   `ID;PWXL`, Multiplan, …) writes R1C1 formulas.
//! - `C;X<col>;Y<row>;K<value>[;E<formula>]`: a cell. `X`/`Y` are 1-based
//!   and persist from record to record. `K"text"` is text, `K1.5` a
//!   number. `;M<formula>` with `;R<row>;C<col>` is a matrix (array)
//!   formula over that area. A value followed by `;I` is ignored.
//! - `P;P<code>`: appends an (Excel, en-US) number format code to the
//!   format table; `F;P<n>[;X<col>][;Y<row>]` gives a cell format `n`.
//! - `E`: the end.
//!
//! Strings and formulas end at an unescaped `;` (`;;` is a literal `;`);
//! `ESC :` is a line break in text (`SYLK_LF`). Everything else (`F;W`
//! column widths, `P;F` fonts, `B` bounds, `O` options, `F;S` styles,
//! `NN` names, …) is ignored, as AOO does.
//!
//! Deviations from AOO: AOO drops the formulas of foreign files and keeps
//! their values (`if( !bMyDoc || !bData ) break;`); Tally converts R1C1
//! formulas (`tally_io_xlsx::r1c1_formula_to_of`) and keeps the value as the
//! cached result, falling back to the value alone when a formula does not
//! convert. Unquoted `KTRUE` / `KFALSE` become booleans and `K#N/A`-style
//! values errors (AOO reads them as the number 0).

use std::sync::Arc;

use tally_model::{CellContentData, CellData, CellRangeLocal, SheetData, StyleData, WorkbookData};
use tally_types::{ErrorKind, Value, MAX_COLS, MAX_ROWS};

/// Limits for [`read_sylk`].
#[derive(Clone, Debug)]
pub struct Limits {
    /// Largest file read.
    pub max_bytes: u64,
    /// Most cells kept.
    pub max_cells: u64,
    /// Longest record (line) in bytes.
    pub max_line_bytes: usize,
    /// Most `P;P` number formats.
    pub max_formats: usize,
    /// Total bytes of text cell content.
    pub max_text_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024,
            max_cells: 10_000_000,
            max_line_bytes: 1024 * 1024,
            max_formats: 10_000,
            max_text_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// Why a SYLK file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum SylkError {
    /// Not a SYLK file (no `ID` record first).
    #[error("not a SYLK file")]
    NotSylk,
    /// A record is malformed (`C`/`F` not followed by `;`), as AOO refuses.
    #[error("malformed SYLK record on line {0}")]
    Malformed(usize),
    /// A limit was hit.
    #[error(
        "limit exceeded: the file is too large or complex to open safely ({what} over {limit})"
    )]
    LimitExceeded {
        /// What.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
}

/// Longest `P;P` number format code converted, in bytes: the model's
/// `MAX_NUMBER_FORMAT_LEN` (a longer code would be replaced by General).
const MAX_CODE_BYTES: usize = tally_model::MAX_NUMBER_FORMAT_LEN;

/// Who wrote the file (AOO's `SylkVersion`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Version {
    /// StarCalc 3 / old OOo: badly quoted strings (`""` escapes).
    Scalc3,
    /// OOo 3.2+ (`CALCOOO32`).
    Ooo32,
    /// Anyone else (assumed to write correct strings).
    Other,
}

impl Version {
    fn own(self) -> bool {
        self != Self::Other
    }
}

/// Returns `true` if `bytes` start with a SYLK `ID;` record (after an
/// optional UTF-8 byte order mark), as AOO's type detection does for
/// `ID;P`.
pub fn sniff_is_sylk(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    b.starts_with(b"ID;")
}

/// AOO's `lcl_ScanSylkString`: the string starting at the opening quote
/// `s[0]`; returns it (unescaped) and the index after it (at its closing
/// quote, or the end).
fn scan_string(s: &[u8], v: Version) -> (String, usize) {
    let mut p = 0;
    let mut end_quote = None;
    loop {
        p += 1;
        let Some(&c) = s.get(p) else { break };
        if c != b'"' {
            continue;
        }
        end_quote = Some(p);
        if v != Version::Scalc3 {
            if s.get(p + 1) == Some(&b';') {
                if s.get(p + 2) == Some(&b';') {
                    p += 2; // escaped ';'
                    end_quote = None;
                } else {
                    break;
                }
            }
        } else if s.get(p + 1) == Some(&b'"') {
            p += 1; // escaped '"'
            end_quote = None;
        } else if s.get(p + 1) == Some(&b';') {
            break;
        }
    }
    let end = end_quote.unwrap_or(p.min(s.len())).max(1);
    let raw = String::from_utf8_lossy(s.get(1..end).unwrap_or_default());
    (unescape(&raw, v), p.min(s.len()))
}

/// AOO's `lcl_UnescapeSylk`.
fn unescape(s: &str, v: Version) -> String {
    let s = if v == Version::Scalc3 {
        s.replace("\"\"", "\"")
    } else {
        s.replace(";;", ";")
    };
    s.replace("\u{1b} :", "\n")
}

/// AOO's `lcl_ScanSylkFormula`: the formula at `s`, and the index after it.
fn scan_formula(s: &[u8], v: Version) -> (String, usize) {
    if v != Version::Scalc3 {
        let mut p = 0;
        while p < s.len() {
            if s[p] == b';' {
                if s.get(p + 1) == Some(&b';') {
                    p += 1;
                } else {
                    break;
                }
            }
            p += 1;
        }
        let p = p.min(s.len());
        return (unescape(&String::from_utf8_lossy(&s[..p]), v), p);
    }
    // StarCalc 3: a formula holding a `;` was quoted (doubled quotes).
    let mut quoted = false;
    if s.first() == Some(&b'"') {
        let mut p = 0;
        loop {
            p += 1;
            match s.get(p) {
                None => break,
                Some(b'"') if s.get(p + 1) == Some(&b'"') => p += 1,
                Some(b'"') => break,
                Some(b';') => {
                    quoted = true;
                    break;
                }
                Some(_) => {}
            }
        }
    }
    if quoted {
        scan_string(s, v)
    } else {
        let p = s.iter().position(|&c| c == b';').unwrap_or(s.len());
        (String::from_utf8_lossy(&s[..p]).into_owned(), p)
    }
}

/// `String::ToInt32` of the field at `s`: leading digits (with a sign).
fn int_at(s: &[u8]) -> i64 {
    let mut i = 0;
    while s.get(i) == Some(&b' ') {
        i += 1;
    }
    let neg = s.get(i) == Some(&b'-');
    if neg || s.get(i) == Some(&b'+') {
        i += 1;
    }
    let mut n: i64 = 0;
    while let Some(d) = s.get(i).filter(|c| c.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    if neg {
        -n
    } else {
        n
    }
}

/// A 1-based SYLK coordinate as a 0-based index below `max`.
fn index(n: i64, max: u32) -> Option<u32> {
    (1..=i64::from(max)).contains(&n).then(|| (n - 1) as u32)
}

/// An unquoted `K` value: a number (`rtl_math_uStringToDouble` with `.`),
/// or a boolean or an error (Tally; AOO reads those as 0).
fn value_of(field: &str) -> Value {
    let t = field.trim();
    if t.eq_ignore_ascii_case("TRUE") {
        return Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("FALSE") {
        return Value::Bool(false);
    }
    if t.starts_with('#') {
        if let Some(e) = ErrorKind::parse(t) {
            return Value::Error(e);
        }
    }
    // The longest numeric prefix, as rtl_math does (0 when there is none);
    // at most 64 characters are tried.
    let cand: String = t
        .chars()
        .take(64)
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    for end in (1..=cand.len()).rev() {
        if let Ok(n) = cand[..end].parse::<f64>() {
            if n.is_finite() {
                return Value::Number(n);
            }
            break;
        }
    }
    Value::Number(0.0)
}

/// Fields of a record after its tag and `;`: calls `f(letter, rest)` for
/// each field start; `f` returns how far it consumed into `rest` (the
/// scanner then skips to the next `;`, as AOO's loops do).
fn for_fields(line: &[u8], mut f: impl FnMut(u8, &[u8]) -> usize) {
    let mut p = 0;
    while p < line.len() {
        let ch = line[p].to_ascii_uppercase();
        p += 1;
        p += f(ch, &line[p..]).min(line.len() - p);
        while p < line.len() && line[p] != b';' {
            p += 1;
        }
        if p < line.len() {
            p += 1;
        }
    }
}

struct Reader<'a> {
    limits: &'a Limits,
    version: Version,
    cells: std::collections::BTreeMap<(u32, u32), CellData>,
    arrays: Vec<CellRangeLocal>,
    formats: Vec<Option<u32>>,
    styles: Vec<StyleData>,
    /// Number format code → index into `styles`.
    style_of: std::collections::HashMap<String, u32>,
    /// A `P;P` code over [`MAX_CODE_BYTES`] was refused.
    long_code: bool,
    text_bytes: u64,
    dropped_outside: u64,
    sheet_names: Vec<String>,
}

impl Reader<'_> {
    fn put(&mut self, row: u32, col: u32, content: CellContentData) -> Result<(), SylkError> {
        if !self.cells.contains_key(&(row, col)) && self.cells.len() as u64 >= self.limits.max_cells
        {
            return Err(SylkError::LimitExceeded {
                what: "cells",
                limit: self.limits.max_cells,
            });
        }
        let style = self.cells.get(&(row, col)).and_then(|c| c.style);
        self.cells.insert(
            (row, col),
            CellData {
                row,
                col,
                content,
                style,
            },
        );
        Ok(())
    }

    fn text(&mut self, s: String) -> Result<Value, SylkError> {
        self.text_bytes += s.len() as u64;
        if self.text_bytes > self.limits.max_text_bytes {
            return Err(SylkError::LimitExceeded {
                what: "text bytes",
                limit: self.limits.max_text_bytes,
            });
        }
        Ok(Value::Text(Arc::from(s)))
    }

    /// A `C` record.
    fn content(&mut self, line: &[u8], pos: &mut (i64, i64)) -> Result<(), SylkError> {
        let mut ref_pos = (1i64, 1i64);
        let mut value: Option<Value> = None;
        let mut formula: Option<(String, bool)> = None;
        let mut err = None;
        for_fields(line, |ch, rest| match ch {
            b'X' => {
                pos.1 = int_at(rest);
                0
            }
            b'Y' => {
                pos.0 = int_at(rest);
                0
            }
            b'C' => {
                ref_pos.1 = int_at(rest);
                0
            }
            b'R' => {
                ref_pos.0 = int_at(rest);
                0
            }
            b'K' => {
                let (v, used) = if rest.first() == Some(&b'"') {
                    let (s, used) = scan_string(rest, self.version);
                    match self.text(s) {
                        Ok(v) => (v, used),
                        Err(e) => {
                            err = Some(e);
                            return rest.len();
                        }
                    }
                } else {
                    let end = rest.iter().position(|&c| c == b';').unwrap_or(rest.len());
                    (value_of(&String::from_utf8_lossy(&rest[..end])), 0)
                };
                // `…;I`: the value is ignored.
                let after = &rest[used.min(rest.len())..];
                let q = after.iter().position(|&c| c == b';').unwrap_or(after.len());
                if after.get(q + 1) != Some(&b'I') {
                    value = Some(v);
                }
                used
            }
            b'E' | b'M' => {
                let (f, used) = scan_formula(rest, self.version);
                formula = Some((f, ch == b'M'));
                used
            }
            _ => 0,
        });
        if let Some(e) = err {
            return Err(e);
        }
        let (Some(row), Some(col)) = (index(pos.0, MAX_ROWS), index(pos.1, MAX_COLS)) else {
            if value.is_some() || formula.is_some() {
                self.dropped_outside += 1;
            }
            return Ok(());
        };
        let of = formula.as_ref().and_then(|(f, _)| {
            if self.version.own() {
                Some(tally_io_ods::native_formula_to_of(f))
            } else {
                tally_io_xlsx::r1c1_formula_to_of(f, &self.sheet_names, 0, row, col)
            }
        });
        match of {
            Some(text) => {
                if let Some((_, true)) = formula {
                    // AOO: `nRefCol/Row` at least the anchor.
                    let r1 = index(ref_pos.0.max(pos.0), MAX_ROWS).unwrap_or(row);
                    let c1 = index(ref_pos.1.max(pos.1), MAX_COLS).unwrap_or(col);
                    if self.arrays.len() < 100_000 {
                        self.arrays.push(CellRangeLocal {
                            r0: row,
                            c0: col,
                            r1,
                            c1,
                        });
                    }
                }
                self.put(
                    row,
                    col,
                    CellContentData::Formula {
                        text: Arc::from(text),
                        cached: value,
                    },
                )?;
            }
            None => {
                if let Some(v) = value {
                    self.put(row, col, CellContentData::Value(v))?;
                }
            }
        }
        Ok(())
    }

    /// An `F` record: `P<n>` applies the n-th `P;P` format at X/Y.
    fn format(&mut self, line: &[u8], pos: &mut (i64, i64)) -> Result<(), SylkError> {
        let mut fmt: Option<i64> = None;
        for_fields(line, |ch, rest| {
            match ch {
                b'X' => pos.1 = int_at(rest),
                b'Y' => pos.0 = int_at(rest),
                b'P' => fmt = Some(int_at(rest)),
                _ => {}
            }
            0
        });
        let Some(n) = fmt.and_then(|n| usize::try_from(n).ok()) else {
            return Ok(());
        };
        let Some(&style) = self.formats.get(n) else {
            return Ok(());
        };
        let (Some(row), Some(col)) = (index(pos.0, MAX_ROWS), index(pos.1, MAX_COLS)) else {
            return Ok(());
        };
        if let Some(c) = self.cells.get_mut(&(row, col)) {
            c.style = style;
        } else if style.is_some() {
            self.put(row, col, CellContentData::Value(Value::Empty))?;
            if let Some(c) = self.cells.get_mut(&(row, col)) {
                c.style = style;
            }
        }
        Ok(())
    }

    /// A `P;P<code>` record.
    fn picture(&mut self, code: &[u8]) -> Result<(), SylkError> {
        if self.formats.len() >= self.limits.max_formats {
            return Err(SylkError::LimitExceeded {
                what: "number formats",
                limit: self.limits.max_formats as u64,
            });
        }
        // A code longer than any the model keeps is refused before it is
        // converted (review M2: 1 MiB codes cost the converter ~33 MB each);
        // the cells using it stay General.
        if code.len() > MAX_CODE_BYTES {
            self.long_code = true;
            self.formats.push(None);
            return Ok(());
        }
        let code = String::from_utf8_lossy(code)
            .replace(";;", ";")
            .replace('\u{1b}', "");
        let style = tally_io_xlsx::excel_to_ooo(&code)
            .filter(|c| c.chars().count() <= tally_model::MAX_NUMBER_FORMAT_LEN)
            .map(|c| match self.style_of.get(&c) {
                Some(&i) => i,
                None => {
                    let i = self.styles.len() as u32;
                    self.style_of.insert(c.clone(), i);
                    self.styles.push(StyleData {
                        number_format: Some(c),
                        ..StyleData::default()
                    });
                    i
                }
            });
        self.formats.push(style);
        Ok(())
    }
}

/// Reads a SYLK file into a one-sheet workbook (the sheet is unnamed, so it
/// takes the file name). Text is UTF-8 when the bytes are valid UTF-8,
/// else Windows-1252 (AOO uses the system character set).
pub fn read_sylk(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, SylkError> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(SylkError::LimitExceeded {
            what: "file bytes",
            limit: limits.max_bytes,
        });
    }
    if !sniff_is_sylk(bytes) {
        return Err(SylkError::NotSylk);
    }
    let text = tally_io_csv::decode(bytes, tally_io_csv::Encoding::Auto);
    let mut r = Reader {
        limits,
        version: Version::Other,
        cells: std::collections::BTreeMap::new(),
        arrays: Vec::new(),
        formats: Vec::new(),
        styles: Vec::new(),
        style_of: std::collections::HashMap::new(),
        long_code: false,
        text_bytes: 0,
        dropped_outside: 0,
        sheet_names: vec![String::new()],
    };
    // (row, col), 1-based, persisting from record to record.
    let mut pos = (1i64, 1i64);
    for (n, line) in text.split(['\n', '\r']).enumerate() {
        let line = line.as_bytes();
        if line.len() > limits.max_line_bytes {
            return Err(SylkError::LimitExceeded {
                what: "record bytes",
                limit: limits.max_line_bytes as u64,
            });
        }
        match line.first() {
            Some(b'C') => {
                if line.get(1) != Some(&b';') {
                    return Err(SylkError::Malformed(n + 1));
                }
                r.content(&line[2..], &mut pos)?;
            }
            Some(b'F') => {
                if line.get(1) != Some(&b';') {
                    return Err(SylkError::Malformed(n + 1));
                }
                r.format(&line[2..], &mut pos)?;
            }
            Some(b'P') if line.starts_with(b"P;P") => r.picture(&line[3..])?,
            Some(b'I') if line.get(1) == Some(&b'D') => {
                let id = line.get(4..).unwrap_or_default();
                r.version = match id {
                    b"CALCOOO32" => Version::Ooo32,
                    b"SCALC3" => Version::Scalc3,
                    _ => Version::Other,
                };
            }
            Some(b'E') => break,
            _ => {}
        }
    }
    let mut warnings = Vec::new();
    if r.long_code {
        warnings.push(tally_model::long_number_format_warning());
    }
    if r.dropped_outside > 0 {
        warnings.push(format!(
            "{} SYLK cells outside the sheet ({MAX_COLS} columns, {MAX_ROWS} rows) were not imported.",
            r.dropped_outside
        ));
    }
    let sheet = SheetData {
        cells: r.cells.into_values().collect(),
        array_formulas: r.arrays,
        ..SheetData::default()
    };
    Ok(WorkbookData {
        sheets: vec![sheet],
        styles: r.styles,
        warnings,
        ..WorkbookData::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_follow_aoo_escapes() {
        assert_eq!(
            scan_string(br#""a;;b";X1"#, Version::Ooo32),
            ("a;b".to_string(), 5)
        );
        assert_eq!(
            scan_string(br#""say ""hi""";X1"#, Version::Scalc3).0,
            "say \"hi\""
        );
        assert_eq!(
            scan_string(b"\"line\x1b :two", Version::Other).0,
            "line\ntwo"
        );
        // Unterminated: everything is the string.
        assert_eq!(scan_string(b"\"abc", Version::Other).0, "abc");
        assert_eq!(scan_string(b"\"", Version::Other).0, "");
    }

    #[test]
    fn formulas_end_at_an_unescaped_semicolon() {
        assert_eq!(
            scan_formula(b"IF(A1;;1;;2);X3", Version::Ooo32),
            ("IF(A1;1;2)".to_string(), 12)
        );
        assert_eq!(scan_formula(b"SUM(A1)", Version::Scalc3).0, "SUM(A1)");
        assert_eq!(
            scan_formula(br#""IF(A1;1;2)";X1"#, Version::Scalc3).0,
            "IF(A1;1;2)"
        );
    }

    #[test]
    fn values() {
        assert_eq!(value_of("1.5"), Value::Number(1.5));
        assert_eq!(value_of("-2E3"), Value::Number(-2000.0));
        assert_eq!(value_of("TRUE"), Value::Bool(true));
        assert_eq!(value_of("#N/A"), Value::Error(ErrorKind::NA));
        assert_eq!(value_of("abc"), Value::Number(0.0));
        assert_eq!(value_of("1e999"), Value::Number(0.0));
        assert_eq!(int_at(b"12;Y"), 12);
        assert_eq!(int_at(b"99999999999999999999999"), i64::MAX);
    }
}
