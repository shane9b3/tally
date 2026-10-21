//! dBase (`.dbf`) import and export (wave 5, W5-3): dBase III/IV/7,
//! FoxPro and Visual FoxPro tables with the field types `C`/`V` (text),
//! `N`/`F` (numbers), `D` (dates, `YYYYMMDD`), `L` (logicals), the
//! FoxPro binary types `Y` (currency), `I` (integer), `B` (Visual FoxPro
//! double) and `T` (date-time), and `M`/`G` (memos and binaries,
//! ignored: the cells stay empty). The first row of the sheet holds the
//! field names with AOO's type descriptors (`NAME,C,12`, `PRICE,N,10,2`,
//! `OK,L`, `DAY,D`), as AOO imports a table; export reads them back.
//!
//! Text is decoded with the code page named by the header's language
//! driver id (cp437, cp850, cp852, cp866, windows-125x, …) unless
//! [`DbfOptions::encoding`] overrides it; the default for no or an unknown
//! id is IBM 850, as in AOO. Export writes a dBase III header, infers each column's
//! type from its values (numbers → `N` with the widest decimals, dates →
//! `D`, booleans → `L`, else `C` up to 254 characters) and encodes text in
//! the chosen code page (unencodable characters become `?`).
//!
//! Limits (docs/security.md): at most [`Limits::max_records`] records,
//! [`Limits::max_fields`] fields and [`Limits::max_cells`] cells; a header
//! that does not match the file size is rejected.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod oem;

use encoding_rs::Encoding;
use tally_model::{CellContentData, CellData, SheetData, StyleData, WorkbookData};
use tally_numfmt::date::{serial_from_ymd, ymd_from_serial};
use tally_types::{DateEpoch, Value, MAX_COLS, MAX_ROWS};

/// Limits of [`read_dbf`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest file (256 MiB).
    pub max_bytes: u64,
    /// Most records read.
    pub max_records: u32,
    /// Most fields.
    pub max_fields: usize,
    /// Most cells materialized.
    pub max_cells: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
            max_records: 2_000_000,
            max_fields: 1024,
            max_cells: 10_000_000,
        }
    }
}

/// Options shared by import and export.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbfOptions {
    /// An encoding label (`windows-1252`, `cp850`, `utf-8`, …); `None`
    /// means the file's language driver on import and windows-1252 on
    /// export.
    pub encoding: Option<String>,
}

/// Errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum DbfError {
    /// Not a dBase file.
    #[error("not a dBase table")]
    NotDbf,
    /// The header is inconsistent with the data.
    #[error("malformed dBase table: {0}")]
    Malformed(String),
    /// A limit was hit.
    #[error("limit exceeded: {what} (limit {limit})")]
    LimitExceeded {
        /// What.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
    /// The encoding label is unknown.
    #[error("unknown encoding {0:?}")]
    UnknownEncoding(String),
    /// Nothing to export.
    #[error("the sheet is empty")]
    Empty,
}

/// dBase version bytes this reader accepts.
const VERSIONS: &[u8] = &[
    0x02, 0x03, 0x04, 0x05, 0x07, 0x30, 0x31, 0x32, 0x43, 0x7B, 0x83, 0x8B, 0x8E, 0xCB, 0xE5, 0xF5,
    0xFB,
];

/// `true` if `bytes` look like a dBase table (version byte, plausible
/// header length and record length).
pub fn sniff_is_dbf(bytes: &[u8]) -> bool {
    if bytes.len() < 32 || !VERSIONS.contains(&bytes[0]) {
        return false;
    }
    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let record_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    // Visual FoxPro (0x30-0x32) appends a 263-byte backlink after the
    // field terminator (correctness review 4, M32).
    let fields_ok = (header_len - 1).is_multiple_of(32)
        || (is_vfp(bytes[0]) && header_len >= 33 + 263 && (header_len - 264).is_multiple_of(32));
    header_len >= 33 && fields_ok && record_len >= 1 && header_len <= bytes.len()
}

/// Visual FoxPro version bytes.
fn is_vfp(version: u8) -> bool {
    matches!(version, 0x30..=0x32)
}

/// A text encoding: one of `encoding_rs`'s, or an OEM code page table.
#[derive(Clone, Copy)]
pub enum Enc {
    /// A WHATWG encoding.
    Rs(&'static Encoding),
    /// cp437/cp850/cp852 (table, label, language driver id).
    Oem(&'static oem::Table, &'static str, u8),
}

impl Enc {
    fn decode(&self, bytes: &[u8]) -> String {
        match self {
            Enc::Rs(e) => e.decode(bytes).0.into_owned(),
            Enc::Oem(t, _, _) => oem::decode(t, bytes),
        }
    }

    fn encode(&self, text: &str) -> Vec<u8> {
        match self {
            Enc::Rs(e) => e.encode(text).0.into_owned(),
            Enc::Oem(t, _, _) => oem::encode(t, text),
        }
    }

    fn driver(&self) -> u8 {
        match self {
            Enc::Rs(e) => driver_for(e),
            Enc::Oem(_, _, d) => *d,
        }
    }

    /// The encoding for a label, `None` if unknown.
    pub fn for_label(label: &str) -> Option<Enc> {
        if let Some((t, n, d)) = oem::table_for(label) {
            return Some(Enc::Oem(t, n, d));
        }
        Encoding::for_label(label.trim().as_bytes()).map(Enc::Rs)
    }
}

/// The encoding of a language driver id (dBase/FoxPro table).
pub fn encoding_for_driver(id: u8) -> Enc {
    let label: &str = match id {
        0x01 | 0x69 => return Enc::Oem(&oem::CP437, "cp437", 0x01),
        0x02 | 0x6A => return Enc::Oem(&oem::CP850, "cp850", 0x02),
        0x64 | 0x1B | 0x87 => return Enc::Oem(&oem::CP852, "cp852", 0x64),
        0x03 | 0x57 | 0x58 | 0x59 => "windows-1252",
        0x04 => "macintosh",
        0x65 | 0x26 => "ibm866",
        0x66 => "ibm865",
        0x67 => "ibm861",
        0x6B => "ibm857",
        0x78 => "big5",
        0x79 => "euc-kr",
        0x7A => "gbk",
        0x7B => "shift_jis",
        0x7C => "windows-874",
        0x7D => "windows-1255",
        0x7E => "windows-1256",
        0x86 => "ibm737",
        0x88 => "ibm857",
        0xC8 => "windows-1250",
        0xC9 => "windows-1251",
        0xCA => "windows-1254",
        0xCB => "windows-1253",
        0xCC => "windows-1257",
        // No or an unknown language driver: AOO's dBase import defaults to
        // IBM 850 (sc/source/ui/docshell/docsh.cxx:1247-1253; correctness
        // review 4, M34).
        _ => return Enc::Oem(&oem::CP850, "cp850", 0x02),
    };
    Enc::Rs(Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252))
}

/// Resolves an option label.
fn encoding_of(opts: &DbfOptions, fallback: Enc) -> Result<Enc, DbfError> {
    match opts.encoding.as_deref() {
        None => Ok(fallback),
        Some(l) => Enc::for_label(l).ok_or_else(|| DbfError::UnknownEncoding(l.to_string())),
    }
}

struct Field {
    name: String,
    kind: u8,
    len: usize,
    decimals: u8,
    /// A FoxPro system field (`_NullFlags`): read past, not shown.
    system: bool,
}

impl Field {
    /// The header text AOO writes for a field: `NAME,C,12`, `NAME,N,10,2`,
    /// `NAME,L`, `NAME,D`, `NAME,M` (sc/source/ui/docshell/docsh8.cxx:
    /// 320-351; correctness review 4, M35). Other types keep the name.
    fn header(&self) -> String {
        match self.kind {
            b'C' | b'V' => format!("{},C,{}", self.name, self.len),
            b'N' | b'F' => format!("{},N,{},{}", self.name, self.len, self.decimals),
            b'L' => format!("{},L", self.name),
            b'D' => format!("{},D", self.name),
            b'M' => format!("{},M", self.name),
            _ => self.name.clone(),
        }
    }
}

/// Reads a dBase table into one sheet (field names in the first row).
pub fn read_dbf(
    bytes: &[u8],
    opts: &DbfOptions,
    limits: &Limits,
) -> Result<WorkbookData, DbfError> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(DbfError::LimitExceeded {
            what: "file size",
            limit: limits.max_bytes,
        });
    }
    if !sniff_is_dbf(bytes) {
        return Err(DbfError::NotDbf);
    }
    let records = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let record_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    let driver = bytes[29];
    let enc = encoding_of(opts, encoding_for_driver(driver))?;
    // Field descriptors: 32 bytes each from offset 32 until 0x0D.
    let mut fields: Vec<Field> = Vec::new();
    let mut pos = 32;
    while pos + 32 <= header_len && bytes[pos] != 0x0D {
        let d = &bytes[pos..pos + 32];
        let name_end = d[..11].iter().position(|b| *b == 0).unwrap_or(11);
        let name = enc.decode(&d[..name_end]);
        let kind = d[11];
        let len = if matches!(kind, b'C') {
            // Character fields may use the decimals byte as the high byte.
            usize::from(d[16]) | (usize::from(d[17]) << 8)
        } else {
            usize::from(d[16])
        };
        fields.push(Field {
            name: name.trim().to_string(),
            kind,
            len,
            decimals: d[17],
            system: is_vfp(bytes[0]) && (d[18] & 0x01 != 0 || kind == b'0'),
        });
        pos += 32;
        if fields.len() > limits.max_fields {
            return Err(DbfError::LimitExceeded {
                what: "fields",
                limit: limits.max_fields as u64,
            });
        }
    }
    if fields.is_empty() {
        return Err(DbfError::Malformed("no fields".into()));
    }
    let sum: usize = 1 + fields.iter().map(|f| f.len).sum::<usize>();
    if sum != record_len {
        return Err(DbfError::Malformed(format!(
            "record length {record_len} does not match the fields ({sum})"
        )));
    }
    let available = bytes.len().saturating_sub(header_len) / record_len.max(1);
    let n = (records as usize).min(available);
    if records as usize > available + 1 {
        return Err(DbfError::Malformed(format!(
            "header claims {records} records, the file holds {available}"
        )));
    }
    if n as u64 > u64::from(limits.max_records) {
        return Err(DbfError::LimitExceeded {
            what: "records",
            limit: u64::from(limits.max_records),
        });
    }
    if (n as u64 + 1) * fields.len() as u64 > limits.max_cells {
        return Err(DbfError::LimitExceeded {
            what: "cells",
            limit: limits.max_cells,
        });
    }
    let mut styles: Vec<StyleData> = Vec::new();
    let mut date_style: Option<u32> = None;
    let mut sheet = SheetData {
        name: String::new(),
        ..SheetData::default()
    };
    // Sheet column of each field (system fields have none).
    let mut cols: Vec<Option<u32>> = Vec::with_capacity(fields.len());
    let mut next_col = 0u32;
    for f in &fields {
        if f.system || next_col >= MAX_COLS {
            cols.push(None);
            continue;
        }
        sheet.cells.push(CellData {
            row: 0,
            col: next_col,
            content: CellContentData::Value(Value::text(f.header().as_str())),
            style: None,
        });
        cols.push(Some(next_col));
        next_col += 1;
    }
    let vfp = is_vfp(bytes[0]);
    let dbase7 = bytes[0] == 0x04;
    let mut datetime_style: Option<u32> = None;
    let mut row = 1u32;
    let epoch = DateEpoch::D1899;
    for i in 0..n {
        let start = header_len + i * record_len;
        let rec = &bytes[start..start + record_len];
        if rec[0] == b'*' {
            continue; // deleted
        }
        if row >= MAX_ROWS {
            break;
        }
        let mut off = 1;
        for (f, col) in fields.iter().zip(&cols) {
            let raw = &rec[off..off + f.len];
            off += f.len;
            let Some(col) = *col else {
                continue;
            };
            let (value, style) = match f.kind {
                b'C' | b'V' => {
                    let s = enc.decode(raw);
                    let t = s.trim_end_matches([' ', '\0']);
                    if t.is_empty() {
                        continue;
                    }
                    (Value::text(t), None)
                }
                // FoxPro currency: a little-endian i64 in 1/10,000
                // (correctness review 4, M33).
                b'Y' if f.len == 8 => {
                    let v = i64::from_le_bytes(raw.try_into().unwrap_or([0; 8]));
                    (Value::Number(v as f64 / 10_000.0), None)
                }
                // FoxPro integer (little-endian); dBase 7 long (big-endian,
                // sign bit flipped).
                b'I' if f.len == 4 => {
                    let b: [u8; 4] = raw.try_into().unwrap_or([0; 4]);
                    let v = if dbase7 {
                        (u32::from_be_bytes(b) ^ 0x8000_0000) as i32
                    } else {
                        i32::from_le_bytes(b)
                    };
                    (Value::Number(f64::from(v)), None)
                }
                // Visual FoxPro double.
                b'B' if vfp && f.len == 8 => {
                    let v = f64::from_le_bytes(raw.try_into().unwrap_or([0; 8]));
                    if !v.is_finite() {
                        continue;
                    }
                    (Value::Number(v), None)
                }
                // Visual FoxPro date-time: Julian day number and
                // milliseconds since midnight.
                b'T' if f.len == 8 => {
                    let jdn = i32::from_le_bytes(raw[..4].try_into().unwrap_or([0; 4]));
                    let ms = i32::from_le_bytes(raw[4..].try_into().unwrap_or([0; 4]));
                    if jdn <= 0 {
                        continue;
                    }
                    // JDN 2415019 is 1899-12-30, serial 0 in D1899.
                    let serial = f64::from(jdn - 2_415_019)
                        + f64::from(ms.clamp(0, 86_400_000)) / 86_400_000.0;
                    let st = *datetime_style.get_or_insert_with(|| {
                        styles.push(StyleData {
                            number_format: Some("YYYY-MM-DD HH:MM:SS".into()),
                            ..StyleData::default()
                        });
                        (styles.len() - 1) as u32
                    });
                    (Value::Number(serial), Some(st))
                }
                b'N' | b'F' | b'Y' => {
                    let s = String::from_utf8_lossy(raw);
                    let t = s.trim();
                    if t.is_empty() {
                        continue;
                    }
                    match t.parse::<f64>() {
                        Ok(v) if v.is_finite() => (Value::Number(v), None),
                        _ => (Value::text(t), None),
                    }
                }
                b'D' => {
                    let t: Vec<u8> = raw
                        .iter()
                        .copied()
                        .filter(|b| !b.is_ascii_whitespace())
                        .collect();
                    if t.len() < 8 || !t[..8].iter().all(u8::is_ascii_digit) {
                        continue;
                    }
                    let t = String::from_utf8_lossy(&t[..8]).into_owned();
                    let y = t[..4].parse::<i32>().ok();
                    let m = t[4..6].parse::<u32>().ok();
                    let d = t[6..8].parse::<u32>().ok();
                    match (y, m, d) {
                        (Some(y), Some(m), Some(d)) => match serial_from_ymd(epoch, y, m, d) {
                            Some(serial) => {
                                let st = *date_style.get_or_insert_with(|| {
                                    styles.push(StyleData {
                                        number_format: Some("YYYY-MM-DD".into()),
                                        ..StyleData::default()
                                    });
                                    (styles.len() - 1) as u32
                                });
                                (Value::Number(serial), Some(st))
                            }
                            None => (Value::text(t), None),
                        },
                        _ => continue,
                    }
                }
                b'L' => match raw.first().copied().unwrap_or(b'?') {
                    b'T' | b't' | b'Y' | b'y' => (Value::Bool(true), None),
                    b'F' | b'f' | b'N' | b'n' => (Value::Bool(false), None),
                    _ => continue,
                },
                // Memo, binary, general, timestamp, autoincrement: not read.
                _ => continue,
            };
            sheet.cells.push(CellData {
                row,
                col,
                content: CellContentData::Value(value),
                style,
            });
        }
        row += 1;
    }
    let _ = f64::from(fields[0].decimals);
    Ok(WorkbookData {
        sheets: vec![sheet],
        styles,
        ..WorkbookData::default()
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColKind {
    Unknown,
    Number,
    Date,
    Logical,
    Text,
}

/// A valid dBase field name from a header text.
fn field_name(text: &str, index: usize, used: &mut Vec<String>) -> String {
    let mut n: String = text
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(10)
        .collect();
    if n.is_empty() || n.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        n = format!("FIELD{}", index + 1);
    }
    let base = n.clone();
    let mut k = 1;
    while used.contains(&n) {
        k += 1;
        let suffix = k.to_string();
        n = format!("{}{}", &base[..base.len().min(10 - suffix.len())], suffix);
    }
    used.push(n.clone());
    n
}

/// A declared field type with its optional width and precision.
type Descriptor = (u8, Option<usize>, Option<usize>);

/// Splits an AOO header text `NAME[,T[,W[,P]]]` into the name and the
/// declared type (`L`, `D`, `M`, `C`, `N`), width and precision.
fn parse_descriptor(text: &str) -> (&str, Option<Descriptor>) {
    let mut parts = text.split(',');
    let name = parts.next().unwrap_or("");
    let Some(t) = parts.next() else {
        return (name, None);
    };
    let kind = t.trim().to_ascii_uppercase().bytes().next();
    let Some(kind @ (b'L' | b'D' | b'M' | b'C' | b'N')) = kind else {
        return (text, None);
    };
    let num = |p: Option<&str>| p.and_then(|v| v.trim().parse::<usize>().ok());
    let w = num(parts.next());
    let p = num(parts.next());
    (name, Some((kind, w, p)))
}

/// Writes the first sheet of the workbook as a dBase III table: the first
/// row supplies the field names (with AOO's optional type descriptors).
pub fn write_dbf(data: &WorkbookData, opts: &DbfOptions) -> Result<Vec<u8>, DbfError> {
    let Some(sheet) = data
        .sheets
        .iter()
        .find(|s| !s.hidden)
        .or(data.sheets.first())
    else {
        return Err(DbfError::Empty);
    };
    let enc = encoding_of(opts, Enc::Rs(encoding_rs::WINDOWS_1252))?;
    let (mut rows, mut cols) = (0u32, 0u32);
    let mut grid: std::collections::HashMap<(u32, u32), &CellData> =
        std::collections::HashMap::new();
    for c in &sheet.cells {
        rows = rows.max(c.row + 1);
        cols = cols.max(c.col + 1);
        grid.insert((c.row, c.col), c);
    }
    if rows == 0 || cols == 0 {
        return Err(DbfError::Empty);
    }
    if cols > 1024 {
        return Err(DbfError::LimitExceeded {
            what: "fields",
            limit: 1024,
        });
    }
    let value_of = |cd: &CellData| -> Value {
        match &cd.content {
            CellContentData::Value(v) => v.clone(),
            CellContentData::Formula { cached, .. }
            | CellContentData::SharedFormula { cached, .. } => {
                cached.clone().unwrap_or(Value::Empty)
            }
        }
    };
    let is_date = |cd: &CellData| {
        cd.style
            .and_then(|i| data.styles.get(i as usize))
            .and_then(|s| s.number_format.as_deref())
            .and_then(|c| tally_numfmt::parse_format(c).ok())
            .is_some_and(|ast| ast.is_date())
    };
    // Column analysis.
    let mut kinds = vec![ColKind::Unknown; cols as usize];
    let mut widths = vec![1usize; cols as usize];
    let mut decimals = vec![0u8; cols as usize];
    let mut texts: Vec<Vec<Option<String>>> = Vec::with_capacity(rows as usize);
    let epoch = data.settings.calc_settings.date_epoch;
    for r in 1..rows {
        let mut line: Vec<Option<String>> = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            let Some(cd) = grid.get(&(r, c)) else {
                line.push(None);
                continue;
            };
            let v = value_of(cd);
            let k = &mut kinds[c as usize];
            let s = match &v {
                Value::Empty => None,
                Value::Number(n) if is_date(cd) && *k != ColKind::Text => {
                    *k = if *k == ColKind::Unknown || *k == ColKind::Date {
                        ColKind::Date
                    } else {
                        ColKind::Text
                    };
                    ymd_from_serial(epoch, *n).map(|(y, m, d)| format!("{y:04}{m:02}{d:02}"))
                }
                Value::Number(n) => {
                    if *k == ColKind::Unknown || *k == ColKind::Number {
                        *k = ColKind::Number;
                    } else {
                        *k = ColKind::Text;
                    }
                    let mut s = format!("{n}");
                    if s.contains('e') || s.contains('E') {
                        s = format!("{n:.6}");
                    }
                    let dec = s.split('.').nth(1).map_or(0, |f| f.len().min(15)) as u8;
                    decimals[c as usize] = decimals[c as usize].max(dec);
                    // Width of the integer part (with sign); the decimals
                    // are added once the column's precision is known.
                    let int_w = s.split('.').next().map_or(1, str::len);
                    widths[c as usize] = widths[c as usize].max(int_w);
                    line.push(Some(s));
                    continue;
                }
                Value::Bool(b) => {
                    if *k == ColKind::Unknown || *k == ColKind::Logical {
                        *k = ColKind::Logical;
                    } else {
                        *k = ColKind::Text;
                    }
                    Some(if *b { "T".into() } else { "F".into() })
                }
                Value::Text(t) => {
                    *k = ColKind::Text;
                    Some(t.to_string())
                }
                Value::Error(e) => {
                    *k = ColKind::Text;
                    Some(e.display().into_owned())
                }
                Value::Array(_) => None,
            };
            if let Some(s) = &s {
                widths[c as usize] = widths[c as usize].max(enc.encode(s).len());
            }
            line.push(s);
        }
        texts.push(line);
    }
    // Field descriptors.
    let mut used = Vec::new();
    let mut fields: Vec<(String, u8, usize, u8)> = Vec::new();
    for c in 0..cols {
        let header = grid
            .get(&(0, c))
            .map(|cd| match value_of(cd) {
                Value::Text(t) => t.to_string(),
                Value::Number(n) => format!("{n}"),
                _ => String::new(),
            })
            .unwrap_or_default();
        // AOO header descriptors: `NAME,C,12`, `NAME,N,10,2`, `NAME,L`,
        // `NAME,D` (sc/source/ui/docshell/docsh8.cxx:445-490).
        let (header_name, declared) = parse_descriptor(&header);
        let name = field_name(header_name, c as usize, &mut used);
        let inferred = kinds[c as usize];
        let kind_of = match declared.as_ref().map(|d| d.0) {
            Some(b'C') | Some(b'M') => ColKind::Text,
            Some(b'N') => ColKind::Number,
            Some(b'D') => ColKind::Date,
            Some(b'L') => ColKind::Logical,
            _ => inferred,
        };
        if let Some((k, w, p)) = declared {
            let clamp_w = |w: usize, max: usize| w.clamp(1, max);
            match k {
                b'C' | b'M' => {
                    if let Some(w) = w {
                        fields.push((name, b'C', clamp_w(w, 254), 0));
                        continue;
                    }
                }
                b'N' => {
                    if let Some(w) = w {
                        let dec = p.unwrap_or(0).min(15) as u8;
                        fields.push((name, b'N', clamp_w(w, 20), dec));
                        continue;
                    }
                }
                _ => {}
            }
        }
        let (kind, len, dec) = match kind_of {
            ColKind::Date => (b'D', 8, 0),
            ColKind::Logical => (b'L', 1, 0),
            ColKind::Number => {
                let dec = decimals[c as usize];
                let len =
                    (widths[c as usize] + if dec > 0 { usize::from(dec) + 1 } else { 0 }).min(20);
                (b'N', len, dec)
            }
            ColKind::Text | ColKind::Unknown => (b'C', widths[c as usize].clamp(1, 254), 0),
        };
        fields.push((name, kind, len, dec));
    }
    let record_len: usize = 1 + fields.iter().map(|f| f.2).sum::<usize>();
    let header_len = 32 + 32 * fields.len() + 1;
    let mut out = Vec::with_capacity(header_len + record_len * texts.len() + 1);
    out.push(0x03);
    let now = tally_numfmt::date::now_serial(DateEpoch::D1899);
    let (y, m, d) = ymd_from_serial(DateEpoch::D1899, now).unwrap_or((2000, 1, 1));
    out.push(((y - 1900).clamp(0, 255)) as u8);
    out.push(m as u8);
    out.push(d as u8);
    out.extend_from_slice(&(texts.len() as u32).to_le_bytes());
    out.extend_from_slice(&(header_len as u16).to_le_bytes());
    out.extend_from_slice(&(record_len as u16).to_le_bytes());
    out.extend_from_slice(&[0u8; 17]);
    out.push(enc.driver());
    out.extend_from_slice(&[0u8; 2]);
    for (name, kind, len, dec) in &fields {
        let mut d = [0u8; 32];
        let nb = enc.encode(name);
        d[..nb.len().min(10)].copy_from_slice(&nb[..nb.len().min(10)]);
        d[11] = *kind;
        d[16] = (*len & 0xff) as u8;
        d[17] = if *kind == b'C' {
            (*len >> 8) as u8
        } else {
            *dec
        };
        out.extend_from_slice(&d);
    }
    out.push(0x0D);
    for line in &texts {
        out.push(b' ');
        for (c, (_, kind, len, dec)) in fields.iter().enumerate() {
            let cell = line.get(c).and_then(|s| s.as_deref());
            let mut field = vec![b' '; *len];
            match (kind, cell) {
                (_, None) => {}
                // A declared type the value does not fit stays blank.
                (b'N', Some(s)) if s.parse::<f64>().is_err() => {}
                (b'D', Some(s)) if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) => {}
                (b'N', Some(s)) => {
                    let n: f64 = s.parse().unwrap_or(0.0);
                    let text = format!(
                        "{:>width$.prec$}",
                        n,
                        width = *len,
                        prec = usize::from(*dec)
                    );
                    let b = text.as_bytes();
                    if b.len() <= *len {
                        field[*len - b.len()..].copy_from_slice(b);
                    } else {
                        field.fill(b'*');
                    }
                }
                (b'L', Some(s)) => field[0] = s.as_bytes().first().copied().unwrap_or(b'?'),
                (b'D', Some(s)) => {
                    let b = s.as_bytes();
                    field[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
                }
                (_, Some(s)) => {
                    let b = enc.encode(s);
                    let n = b.len().min(*len);
                    field[..n].copy_from_slice(&b[..n]);
                }
            }
            out.extend_from_slice(&field);
        }
    }
    out.push(0x1A);
    Ok(out)
}

fn driver_for(enc: &'static Encoding) -> u8 {
    match enc.name() {
        "IBM437" => 0x01,
        "IBM850" => 0x02,
        "windows-1252" => 0x03,
        "macintosh" => 0x04,
        "IBM852" => 0x64,
        "IBM866" => 0x65,
        "windows-1250" => 0xC8,
        "windows-1251" => 0xC9,
        "windows-1254" => 0xCA,
        "windows-1253" => 0xCB,
        "windows-1257" => 0xCC,
        _ => 0x00,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> WorkbookData {
        let mut wb = WorkbookData::default();
        wb.styles.push(StyleData {
            number_format: Some("YYYY-MM-DD".into()),
            ..StyleData::default()
        });
        let mut s = SheetData {
            name: "T".into(),
            ..SheetData::default()
        };
        let mut push = |r, c, v: Value, st: Option<u32>| {
            s.cells.push(CellData {
                row: r,
                col: c,
                content: CellContentData::Value(v),
                style: st,
            })
        };
        push(0, 0, Value::text("Name"), None);
        push(0, 1, Value::text("Amount"), None);
        push(0, 2, Value::text("When"), None);
        push(0, 3, Value::text("Ok"), None);
        push(1, 0, Value::text("Müller & Söhne"), None);
        push(1, 1, Value::Number(1234.5), None);
        push(1, 2, Value::Number(46305.0), Some(0)); // 2026-10-11
        push(1, 3, Value::Bool(true), None);
        push(2, 0, Value::text("Çà"), None);
        push(2, 1, Value::Number(-7.25), None);
        push(2, 3, Value::Bool(false), None);
        wb.sheets.push(s);
        wb
    }

    #[test]
    fn dbf_roundtrip_cp1252() {
        let wb = book();
        let bytes = write_dbf(&wb, &DbfOptions::default()).unwrap();
        assert!(sniff_is_dbf(&bytes));
        assert_eq!(bytes[0], 0x03);
        assert_eq!(bytes[29], 0x03, "windows-1252 driver");
        let back = read_dbf(&bytes, &DbfOptions::default(), &Limits::default()).unwrap();
        let s = &back.sheets[0];
        let get = |r, c| {
            s.cells
                .iter()
                .find(|x| x.row == r && x.col == c)
                .map(|x| match &x.content {
                    CellContentData::Value(v) => v.clone(),
                    _ => Value::Empty,
                })
                .unwrap_or(Value::Empty)
        };
        // AOO type descriptors (docsh8.cxx:320-351).
        assert_eq!(get(0, 0), Value::text("NAME,C,14"));
        assert_eq!(get(0, 1), Value::text("AMOUNT,N,7,2"));
        assert_eq!(get(0, 2), Value::text("WHEN,D"));
        assert_eq!(get(0, 3), Value::text("OK,L"));
        // Re-exporting the imported sheet keeps names and types.
        let again = write_dbf(&back, &DbfOptions::default()).unwrap();
        assert_eq!(&again[32..36], b"NAME");
        assert_eq!(again[32 + 11], b'C');
        assert_eq!(again[32 + 16], 14);
        assert_eq!(&again[64..70], b"AMOUNT");
        assert_eq!(
            (again[64 + 11], again[64 + 16], again[64 + 17]),
            (b'N', 7, 2)
        );
        assert_eq!(get(1, 0), Value::text("Müller & Söhne"));
        assert_eq!(get(1, 1), Value::Number(1234.5));
        assert_eq!(get(2, 1), Value::Number(-7.25));
        assert_eq!(get(1, 2), Value::Number(46305.0));
        assert_eq!(get(1, 3), Value::Bool(true));
        assert_eq!(get(2, 3), Value::Bool(false));
        assert_eq!(get(2, 0), Value::text("Çà"));
        assert_eq!(
            back.styles[s
                .cells
                .iter()
                .find(|x| x.row == 1 && x.col == 2)
                .unwrap()
                .style
                .unwrap() as usize]
                .number_format
                .as_deref(),
            Some("YYYY-MM-DD")
        );
        // cp850 in, cp850 out.
        let opts = DbfOptions {
            encoding: Some("cp850".into()),
        };
        let b2 = write_dbf(&wb, &opts).unwrap();
        assert_eq!(b2[29], 0x02);
        let back2 = read_dbf(&b2, &DbfOptions::default(), &Limits::default()).unwrap();
        assert_eq!(
            back2.sheets[0]
                .cells
                .iter()
                .find(|x| x.row == 1 && x.col == 0)
                .map(|x| x.content.clone()),
            Some(CellContentData::Value(Value::text("Müller & Söhne")))
        );
        assert!(matches!(
            write_dbf(
                &wb,
                &DbfOptions {
                    encoding: Some("klingon".into())
                }
            ),
            Err(DbfError::UnknownEncoding(_))
        ));
    }

    #[test]
    fn malformed_and_limits() {
        let wb = book();
        let bytes = write_dbf(&wb, &DbfOptions::default()).unwrap();
        assert!(matches!(
            read_dbf(b"not a dbf", &DbfOptions::default(), &Limits::default()),
            Err(DbfError::NotDbf)
        ));
        let mut lying = bytes.clone();
        lying[4..8].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(matches!(
            read_dbf(&lying, &DbfOptions::default(), &Limits::default()),
            Err(DbfError::Malformed(_))
        ));
        let mut bad_len = bytes.clone();
        bad_len[10] = 5;
        assert!(matches!(
            read_dbf(&bad_len, &DbfOptions::default(), &Limits::default()),
            Err(DbfError::Malformed(_))
        ));
        let tiny = Limits {
            max_records: 1,
            ..Limits::default()
        };
        assert!(matches!(
            read_dbf(&bytes, &DbfOptions::default(), &tiny),
            Err(DbfError::LimitExceeded { .. })
        ));
        let truncated = &bytes[..bytes.len() - 10];
        // A truncated last record is dropped, not an error.
        let back = read_dbf(truncated, &DbfOptions::default(), &Limits::default()).unwrap();
        assert!(back.sheets[0].cells.iter().all(|c| c.row <= 1));
        // Fuzz-ish: flipped bytes never panic.
        for i in (0..bytes.len()).step_by(7) {
            let mut m = bytes.clone();
            m[i] ^= 0xff;
            let _ = read_dbf(&m, &DbfOptions::default(), &Limits::default());
        }
    }
}
