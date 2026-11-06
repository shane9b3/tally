//! DIF (Data Interchange Format, `.dif`) import, as AOO 4.1's
//! `ScFormatFilterPluginImpl::ScImportDif` and `DifParser`
//! (`main/sc/source/filter/dif/difimp.cxx`).
//!
//! A DIF file is a header of topics, each three lines (the keyword, then
//! `vector,value`, then a quoted string): `TABLE` (its string names the
//! sheet), `VECTORS` (columns), `TUPLES` (rows), … up to `DATA`. Then the
//! data, as two-line entries:
//!
//! - `-1,0` + `BOT`: a new row; `-1,0` + `EOD`: the end.
//! - `0,<number>` + `V`: a number; + `TRUE` / `FALSE`: a boolean (AOO: the
//!   number with a logical format); + `NA` / `ERROR`: that text; anything
//!   else: the text `#IND: <indicator>?`. A number that does not parse
//!   gives the text `#ERR: <number> (<indicator>)`.
//! - `1,0` + `"text"`: text (`""` is a quote; a string may run over
//!   several lines, ending at a line followed by the start of an entry).
//!   An empty string leaves the cell empty.
//!
//! Unknown entries and topics are skipped. Deviation: AOO's default
//! (non-"plain") import runs numbers through the number formatter, which
//! also recognises locale numbers and gives them that format; Tally reads
//! the number syntax only (AOO's "plain" `ScanFloatVal`: `.` or `,` as the
//! decimal separator). Text is UTF-8 when valid, else Windows-1252 (AOO
//! asks for the character set).

use std::sync::Arc;

use tally_model::{CellContentData, CellData, SheetData, WorkbookData};
use tally_types::{Value, MAX_COLS, MAX_ROWS};

/// Limits for [`read_dif`].
#[derive(Clone, Debug)]
pub struct Limits {
    /// Largest file read.
    pub max_bytes: u64,
    /// Most cells kept.
    pub max_cells: u64,
    /// Longest line in bytes.
    pub max_line_bytes: usize,
    /// Longest (multi-line) string in bytes.
    pub max_string_bytes: usize,
    /// Total bytes of text cell content.
    pub max_text_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024,
            max_cells: 10_000_000,
            max_line_bytes: 1024 * 1024,
            max_string_bytes: 1024 * 1024,
            max_text_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// Why a DIF file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum DifError {
    /// No `DATA` topic (AOO: `eERR_FORMAT`).
    #[error("not a DIF file (no DATA section)")]
    NotDif,
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

/// Returns `true` if `bytes` start with a DIF header: the `TABLE` topic
/// (`TABLE`, then `0,1`) after an optional UTF-8 byte order mark.
pub fn sniff_is_dif(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let head = String::from_utf8_lossy(&b[..b.len().min(256)]);
    let mut lines = head.lines();
    lines.next().map(str::trim_end) == Some("TABLE")
        && lines.next().is_some_and(|l| {
            l.trim()
                .strip_prefix("0,")
                .is_some_and(|v| v.trim().parse::<u32>().is_ok())
        })
}

/// AOO's `ScanFloatVal` (the "plain" number syntax): optional sign and
/// blanks, digits, `.` or `,` as the decimal separator, an exponent.
fn scan_float(s: &str) -> Option<f64> {
    let t = s.trim().trim_start_matches('+');
    if t.is_empty() || t.len() > 400 {
        return None;
    }
    let t = if t.contains('.') {
        t.to_string()
    } else {
        t.replacen(',', ".", 1)
    };
    t.parse::<f64>().ok().filter(|n| n.is_finite()).filter(|_| {
        !t.chars()
            .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    })
}

/// AOO's `LookAhead`: does `line` start an entry (`-1,0`, `0,<number>`,
/// `1,0`)?
fn starts_entry(line: Option<&str>) -> bool {
    let Some(l) = line else {
        return false;
    };
    if let Some(rest) = l.strip_prefix('-') {
        return rest.starts_with("1,0");
    }
    if let Some(rest) = l.strip_prefix("0,") {
        return scan_float(rest).is_some();
    }
    l.starts_with("1,0")
}

/// AOO's `lcl_DeEscapeQuotesDif`: `""` → `"` (single quotes stay).
fn dequote(s: &str) -> String {
    s.replace("\"\"", "\"")
}

enum Entry {
    Bot,
    Eod,
    Value(Value),
    Text(String),
    Unknown,
}

/// The lines of the text, split lazily (a file of newlines never becomes
/// a vector of millions of slices), with one line of look-ahead.
struct Lines<'a> {
    it: std::iter::Peekable<std::str::Split<'a, char>>,
    max_line: usize,
    /// A line over `max_line` was met: reading stops there.
    too_long: bool,
}

fn strip_cr(l: &str) -> &str {
    l.strip_suffix('\r').unwrap_or(l)
}

impl<'a> Lines<'a> {
    fn new(text: &'a str, max_line: usize) -> Self {
        Self {
            it: text.split('\n').peekable(),
            max_line,
            too_long: false,
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        if self.too_long {
            return None;
        }
        let l = strip_cr(self.it.next()?);
        if l.len() > self.max_line {
            self.too_long = true;
            return None;
        }
        Some(l)
    }

    fn peek(&mut self) -> Option<&'a str> {
        if self.too_long {
            return None;
        }
        self.it.peek().map(|l| strip_cr(l))
    }

    /// AOO's `GetNextTopic`: the topic keyword, its value and its string;
    /// `None` at the end.
    fn topic(&mut self) -> Option<(&'a str, u32, String)> {
        let key = self.next()?;
        const KNOWN: [&str; 13] = [
            "TABLE",
            "VECTORS",
            "TUPLES",
            "DATA",
            "LABEL",
            "COMMENT",
            "SIZE",
            "PERIODICITY",
            "MAJORSTART",
            "MINORSTART",
            "TRUELENGTH",
            "UINITS",
            "DISPLAYUNITS",
        ];
        let key = key.trim_end();
        if !KNOWN.contains(&key) {
            self.next();
            self.next();
            return Some(("", 0, String::new()));
        }
        let vv = self.next().unwrap_or_default();
        let Some((_, val)) = vv.trim_start().split_once(',') else {
            self.next();
            return Some((key, 0, String::new()));
        };
        let val: u32 = val
            .trim()
            .chars()
            .take_while(char::is_ascii_digit)
            .take(9)
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        let data = self.next().unwrap_or_default();
        let data = if data.len() > 2 {
            data.get(1..data.len() - 1).unwrap_or_default().to_string()
        } else {
            String::new()
        };
        Some((key, val, data))
    }

    /// AOO's `GetNextDataset`.
    fn entry(&mut self, max_string: usize) -> Result<Entry, DifError> {
        let Some(line) = self.next() else {
            return Ok(Entry::Eod);
        };
        let mut out = Entry::Unknown;
        if let Some(rest) = line.strip_prefix('-') {
            if rest.starts_with("1,0") {
                let ind = self.next().unwrap_or_default().trim_end();
                if ind == "BOT" {
                    return Ok(Entry::Bot);
                }
                if ind == "EOD" {
                    return Ok(Entry::Eod);
                }
            }
        } else if let Some(num) = line.strip_prefix("0,") {
            let ind = self.next().unwrap_or_default().trim_end();
            return Ok(Entry::Value(match scan_float(num) {
                Some(n) if ind == "V" => Value::Number(n),
                Some(n) if ind == "TRUE" || ind == "FALSE" => Value::Bool(n != 0.0),
                Some(_) if ind == "NA" || ind == "ERROR" => Value::Text(Arc::from(ind)),
                Some(_) => Value::Text(Arc::from(format!("#IND: {ind}?"))),
                None => Value::Text(Arc::from(format!("#ERR: {num} ({ind})"))),
            }));
        } else if line.starts_with("1,0") {
            let s = self.next().unwrap_or_default();
            if let Some(body) = s.strip_prefix('"') {
                if starts_entry(self.peek()) {
                    // One line.
                    if s.len() >= 2 && s.ends_with('"') {
                        return Ok(Entry::Text(dequote(&body[..body.len() - 1])));
                    }
                } else {
                    // Several lines, up to one followed by an entry.
                    let mut data = body.to_string();
                    loop {
                        data.push('\n');
                        let Some(l) = self.next() else {
                            break;
                        };
                        if data.len() + l.len() > max_string {
                            return Err(DifError::LimitExceeded {
                                what: "string bytes",
                                limit: max_string as u64,
                            });
                        }
                        if !starts_entry(self.peek()) {
                            data.push_str(l);
                        } else {
                            if let Some(last) = l.strip_suffix('"') {
                                data.push_str(last);
                                out = Entry::Text(dequote(&data));
                            }
                            break;
                        }
                    }
                }
            }
        }
        if matches!(out, Entry::Unknown) {
            self.next();
        }
        Ok(out)
    }
}

/// Reads a DIF file into a one-sheet workbook. The sheet is named by the
/// `TABLE` topic's string, else (unnamed) by the file name.
pub fn read_dif(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, DifError> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(DifError::LimitExceeded {
            what: "file bytes",
            limit: limits.max_bytes,
        });
    }
    let text = tally_io_csv::decode(bytes, tally_io_csv::Encoding::Auto);
    let too_long = || DifError::LimitExceeded {
        what: "line bytes",
        limit: limits.max_line_bytes as u64,
    };
    let mut lines = Lines::new(&text, limits.max_line_bytes);
    let mut name = String::new();
    let mut data = false;
    while let Some((key, _, s)) = lines.topic() {
        match key {
            "TABLE" if !s.is_empty() => name = s.chars().take(255).collect(),
            "DATA" => {
                data = true;
                break;
            }
            _ => {}
        }
    }
    if lines.too_long {
        return Err(too_long());
    }
    if !data {
        return Err(DifError::NotDif);
    }
    let mut cells = Vec::new();
    let mut text_bytes = 0u64;
    // AOO: `nColCnt` starts past the last column, so the first BOT does
    // not advance the row.
    let mut col: Option<u32> = None;
    let mut row: u64 = 0;
    let mut overflow = 0u64;
    loop {
        let value = match lines.entry(limits.max_string_bytes)? {
            Entry::Eod if lines.too_long => return Err(too_long()),
            Entry::Eod => break,
            Entry::Bot => {
                if col.is_some() {
                    row += 1;
                }
                col = Some(0);
                continue;
            }
            Entry::Unknown => continue,
            Entry::Value(v) => Some(v),
            Entry::Text(s) => (!s.is_empty()).then(|| Value::Text(Arc::from(s))),
        };
        let c = col.unwrap_or(0);
        col = Some(c.saturating_add(1));
        let Some(v) = value else {
            continue;
        };
        if c >= MAX_COLS || row >= u64::from(MAX_ROWS) {
            overflow += 1;
            continue;
        }
        if let Value::Text(t) = &v {
            text_bytes += t.len() as u64;
            if text_bytes > limits.max_text_bytes {
                return Err(DifError::LimitExceeded {
                    what: "text bytes",
                    limit: limits.max_text_bytes,
                });
            }
        }
        if cells.len() as u64 >= limits.max_cells {
            return Err(DifError::LimitExceeded {
                what: "cells",
                limit: limits.max_cells,
            });
        }
        cells.push(CellData {
            row: row as u32,
            col: c,
            content: CellContentData::Value(v),
            style: None,
        });
    }
    let mut warnings = Vec::new();
    if overflow > 0 {
        // AOO: eERR_RNGOVRFLW.
        warnings.push(format!(
            "{overflow} DIF values outside the sheet ({MAX_COLS} columns, {MAX_ROWS} rows) were not imported."
        ));
    }
    Ok(WorkbookData {
        sheets: vec![SheetData {
            name,
            cells,
            ..SheetData::default()
        }],
        warnings,
        ..WorkbookData::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats() {
        assert_eq!(scan_float(" 1.5"), Some(1.5));
        assert_eq!(scan_float("+2,5"), Some(2.5));
        assert_eq!(scan_float("-1E3"), Some(-1000.0));
        assert_eq!(scan_float("abc"), None);
        assert_eq!(scan_float("inf"), None);
        assert_eq!(scan_float("1e999"), None);
        assert_eq!(scan_float(""), None);
    }

    #[test]
    fn entry_starts() {
        assert!(starts_entry(Some("-1,0")));
        assert!(starts_entry(Some("0,12")));
        assert!(starts_entry(Some("1,0")));
        assert!(!starts_entry(Some("0,x")));
        assert!(!starts_entry(Some("hello")));
        assert!(!starts_entry(None));
    }
}
