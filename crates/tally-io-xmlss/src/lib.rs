//! Excel 2003 XML Spreadsheet ("SpreadsheetML", `.xml`) import for Tally.
//!
//! [`read_xmlss`] converts a SpreadsheetML document into
//! [`tally_model::WorkbookData`], following what AOO 4.1 imports through
//! its XSLT filter (`main/filter/source/xslt/import/spreadsheetml/
//! spreadsheetml2ooo.xsl`; template names are cited where the behaviour
//! follows it). [`sniff_is_xmlss`] recognizes the format from its first
//! 4 KiB.
//!
//! Read: document properties, `x:ExcelWorkbook` (active sheet, 1904 date
//! system), styles with `ss:Parent` inheritance, workbook and sheet named
//! ranges (`Print_Area` / `Print_Titles` become the page setup), sheets
//! with columns, rows and cells (values, R1C1 formulas with cached values,
//! array ranges, merges, hyperlinks, comments), frozen panes, hidden and
//! protected sheets and the AutoFilter range. Conditional formatting,
//! data validation, AutoFilter criteria and pivot tables are dropped with
//! a warning; page setup, printing options, sorting, unfrozen splits,
//! zoom, tab colours and custom document properties silently.
//!
//! Reading untrusted input is hardened like the other XML readers (see
//! `docs/security.md`): a size cap, DOCTYPE rejection, depth, attribute,
//! per-event, cell, text, sheet, style and name budgets, clamped indexes
//! and spans, and no recursion on the input's structure.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod refs;
mod sheet;
mod styles;
mod xml;

use std::borrow::Cow;
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Arc;

use tally_model::{CellContentData, NameData, PageSetupData, SheetData, WorkbookData};
use tally_types::{DateEpoch, Value};

use crate::sheet::SheetExtra;
use crate::styles::{ModelStyles, StyleTable, MAX_STYLE_ID_BYTES};
use crate::xml::{Elem, Ev, XmlLimits, XmlReader};

/// Excel's default row height (12.75 pt) in 1/100 mm, used for hidden rows
/// without a height.
const DEFAULT_ROW_HMM: u32 = 450;

/// Longest named-range name kept (Excel's limit is 255 characters).
const MAX_NAME_CHARS: usize = 255;

/// Longest `ss:RefersTo` text kept.
const MAX_REFERS_TO: usize = 64 * 1024;

/// Longest sheet name kept, in characters (longer names are cut).
const MAX_SHEET_NAME_CHARS: usize = 255;

/// Longest document property kept (longer text is cut).
const MAX_META_TEXT: usize = 64 * 1024;

/// Resource limits applied by [`read_xmlss`]. Cell, text and sheet
/// defaults are those of `tally_io_ods::Limits` and `tally_io_xlsx::Limits`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum input size in bytes.
    pub max_bytes: u64,
    /// Maximum cells materialized into [`WorkbookData`] (all sheets;
    /// merges and array-formula areas count too).
    pub max_cells: u64,
    /// Maximum number of sheets.
    pub max_sheets: usize,
    /// Maximum bytes of cell text, formula text, note text and hyperlink
    /// targets materialized (all sheets).
    pub max_text_bytes: u64,
    /// Maximum XML element nesting depth.
    pub max_depth: u32,
    /// Maximum attributes on one XML element.
    pub max_attrs: u32,
    /// Maximum input bytes one XML event (a tag, a text run) may span.
    pub max_event_bytes: u64,
    /// Maximum distinct style ids (`ss:Style` definitions and references).
    pub max_styles: usize,
    /// Maximum named ranges.
    pub max_names: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024,
            max_cells: 10_000_000,
            max_sheets: 10_000,
            max_text_bytes: 1024 * 1024 * 1024,
            max_depth: 256,
            max_attrs: 256,
            max_event_bytes: 64 * 1024 * 1024,
            max_styles: 64_000,
            max_names: 100_000,
        }
    }
}

/// Errors from reading SpreadsheetML.
#[derive(Debug, thiserror::Error)]
pub enum XmlssError {
    /// The input is not an Excel 2003 XML spreadsheet.
    #[error("not an Excel 2003 XML spreadsheet")]
    NotXmlss,
    /// The workbook has no worksheet.
    #[error("the Excel 2003 XML workbook contains no worksheets")]
    NoSheets,
    /// The XML is malformed.
    #[error("XML error: {0}")]
    Xml(String),
    /// The document contains a `<!DOCTYPE>`, which SpreadsheetML never needs.
    #[error("XML DOCTYPE declarations are not allowed")]
    ForbiddenDoctype,
    /// The text encoding is unknown or the bytes do not match it.
    #[error("unsupported or invalid text encoding: {0}")]
    Encoding(String),
    /// A resource limit was hit.
    #[error(
        "limit exceeded: the file is too large or complex to open safely ({what} over {})",
        human_limit(what, *limit)
    )]
    LimitExceeded {
        /// What was limited.
        what: &'static str,
        /// The limit value.
        limit: u64,
    },
}

/// `limit` for messages: byte limits in KiB/MiB/GiB, counts as plain numbers.
fn human_limit(what: &str, limit: u64) -> String {
    const KIB: u64 = 1024;
    if !what.contains("bytes") || limit < KIB {
        return limit.to_string();
    }
    for (unit, size) in [("GiB", KIB * KIB * KIB), ("MiB", KIB * KIB), ("KiB", KIB)] {
        if limit >= size {
            let whole = limit / size;
            return if limit.is_multiple_of(size) {
                format!("{whole} {unit}")
            } else {
                format!("{:.1} {unit}", limit as f64 / size as f64)
            };
        }
    }
    limit.to_string()
}

const SS_NS: &[u8] = b"urn:schemas-microsoft-com:office:spreadsheet";

/// Returns `true` if `bytes` look like an Excel 2003 XML spreadsheet: after
/// an optional BOM, whitespace, the XML declaration, comments and other
/// processing instructions, either `<?mso-application progid="Excel.Sheet"?>`
/// or a root `Workbook` element (any prefix) declaring the
/// `urn:schemas-microsoft-com:office:spreadsheet` namespace. Only the first
/// 4 KiB are looked at.
pub fn sniff_is_xmlss(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let text: Cow<'_, [u8]> = match head {
        [0xFF, 0xFE, rest @ ..] => Cow::Owned(
            encoding_rs::UTF_16LE
                .decode_without_bom_handling(rest)
                .0
                .into_owned()
                .into_bytes(),
        ),
        [0xFE, 0xFF, rest @ ..] => Cow::Owned(
            encoding_rs::UTF_16BE
                .decode_without_bom_handling(rest)
                .0
                .into_owned()
                .into_bytes(),
        ),
        [0xEF, 0xBB, 0xBF, rest @ ..] => Cow::Borrowed(rest),
        _ => Cow::Borrowed(head),
    };
    sniff_text(&text)
}

fn sniff_text(s: &[u8]) -> bool {
    let mut i = 0;
    loop {
        while s.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let rest = s.get(i..).unwrap_or_default();
        if rest.starts_with(b"<?") {
            let Some(end) = find(rest, b"?>") else {
                return false;
            };
            let pi = &rest[..end];
            if pi.starts_with(b"<?mso-application") && find(pi, b"Excel.Sheet").is_some() {
                return true;
            }
            i += end + 2;
        } else if rest.starts_with(b"<!--") {
            let Some(end) = find(rest, b"-->") else {
                return false;
            };
            i += end + 3;
        } else if rest.starts_with(b"<") {
            let name_len = rest[1..]
                .iter()
                .position(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
                .unwrap_or(rest.len() - 1);
            let name = &rest[1..1 + name_len];
            let local = name.rsplit(|b| *b == b':').next().unwrap_or(name);
            if local != b"Workbook" {
                return false;
            }
            let tag_end = find(rest, b">").unwrap_or(rest.len());
            return find(&rest[..tag_end], SS_NS).is_some();
        } else {
            return false;
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The input as UTF-8: a UTF-8 BOM is skipped; UTF-16 (with a BOM, or
/// detected from `<` followed or preceded by a NUL) and a declared
/// non-UTF-8 `encoding=` are decoded with `encoding_rs`; malformed input is
/// an [`XmlssError::Encoding`].
fn to_utf8(bytes: &[u8]) -> Result<Cow<'_, str>, XmlssError> {
    let decode = |enc: &'static encoding_rs::Encoding,
                  b: &[u8]|
     -> Result<Cow<'static, str>, XmlssError> {
        enc.decode_without_bom_handling_and_without_replacement(b)
            .map(|c| Cow::Owned(c.into_owned()))
            .ok_or_else(|| XmlssError::Encoding(format!("the bytes are not valid {}", enc.name())))
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => return utf8(rest),
        [0xFF, 0xFE, rest @ ..] => return decode(encoding_rs::UTF_16LE, rest),
        [b'<', 0, ..] => return decode(encoding_rs::UTF_16LE, bytes),
        [0xFE, 0xFF, rest @ ..] => return decode(encoding_rs::UTF_16BE, rest),
        [0, b'<', ..] => return decode(encoding_rs::UTF_16BE, bytes),
        _ => {}
    }
    if let Some(label) = declared_encoding(bytes) {
        let l = label.to_ascii_lowercase();
        if !matches!(
            l.as_str(),
            "utf-8" | "utf8" | "us-ascii" | "ascii" | "utf-16" | "utf16"
        ) {
            let enc = encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or_else(|| XmlssError::Encoding(format!("unknown encoding \"{label}\"")))?;
            if enc != encoding_rs::UTF_8 {
                return decode(enc, bytes);
            }
        }
    }
    utf8(bytes)
}

fn utf8(b: &[u8]) -> Result<Cow<'_, str>, XmlssError> {
    std::str::from_utf8(b)
        .map(Cow::Borrowed)
        .map_err(|_| XmlssError::Encoding("the file is not valid UTF-8 text".into()))
}

/// The `encoding="…"` of a leading XML declaration (first 1 KiB).
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(1024)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let decl = &head[..find(head, b"?>")?];
    let at = find(decl, b"encoding")? + 8;
    let rest = &decl[at..];
    let q = rest.iter().position(|b| *b == b'"' || *b == b'\'')?;
    let quote = rest[q];
    let val = &rest[q + 1..];
    let end = val.iter().position(|b| *b == quote)?;
    let label = std::str::from_utf8(&val[..end]).ok()?.trim();
    (!label.is_empty() && label.len() <= 64).then(|| label.to_string())
}

/// Reads an Excel 2003 XML spreadsheet.
///
/// Formulas are converted from R1C1 to OpenFormula
/// ([`tally_io_xlsx::r1c1_formula_to_of`]); one that does not convert is
/// kept with its original text after `of:=` (the engine shows Err:5xx) and
/// its cached `ss:Data` value. What was dropped is listed in
/// [`WorkbookData::warnings`].
pub fn read_xmlss(bytes: &[u8], limits: &Limits) -> Result<WorkbookData, XmlssError> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(XmlssError::LimitExceeded {
            what: "input bytes",
            limit: limits.max_bytes,
        });
    }
    let text = to_utf8(bytes)?;
    if text.len() as u64 > limits.max_bytes.saturating_mul(2) {
        return Err(XmlssError::LimitExceeded {
            what: "input bytes",
            limit: limits.max_bytes,
        });
    }
    let mut xr = XmlReader::new(
        text.as_bytes(),
        XmlLimits {
            max_depth: limits.max_depth,
            max_attrs: limits.max_attrs,
            max_event: limits.max_event_bytes,
        },
    );
    let mut ctx = Ctx::new(limits);
    let root = loop {
        match xr.next()? {
            Ev::Start(e) => break e,
            Ev::Eof => return Err(XmlssError::NotXmlss),
            _ => {}
        }
    };
    if root.local() != "Workbook" {
        return Err(XmlssError::NotXmlss);
    }
    workbook(&mut xr, &mut ctx)?;
    finish(ctx)
}

/// Shared parse state.
pub(crate) struct Ctx<'a> {
    pub limits: &'a Limits,
    pub cells: u64,
    pub text_bytes: u64,
    pub warnings: Vec<String>,
    /// Style ids in first-reference order, and their index.
    style_ids: Vec<String>,
    style_index: HashMap<String, u32>,
    styles: StyleTable,
    pub sheets: Vec<SheetData>,
    pub extras: Vec<SheetExtra>,
    /// Lower-cased sheet names taken.
    taken: std::collections::HashSet<String>,
    /// Lower-cased base name → the next ` (n)` suffix to try, so repeated
    /// names cost O(1) each (review: 10,000 equal names were quadratic).
    next_suffix: HashMap<String, u32>,
    /// `warnings`, for an O(1) "already said" check.
    said: std::collections::HashSet<String>,
    /// Named ranges: (name, local sheet, `ss:RefersTo`).
    names: Vec<(String, Option<usize>, String)>,
    active_sheet: Option<u32>,
    date1904: bool,
    meta: tally_model::DocMeta,
}

/// Adds `w` to `warnings` once.
pub(crate) fn warn(warnings: &mut Vec<String>, w: String) {
    if !warnings.contains(&w) {
        warnings.push(w);
    }
}

impl<'a> Ctx<'a> {
    fn new(limits: &'a Limits) -> Self {
        Self {
            limits,
            cells: 0,
            text_bytes: 0,
            warnings: Vec::new(),
            style_ids: Vec::new(),
            style_index: HashMap::new(),
            styles: StyleTable::default(),
            sheets: Vec::new(),
            extras: Vec::new(),
            taken: Default::default(),
            next_suffix: HashMap::new(),
            said: Default::default(),
            names: Vec::new(),
            active_sheet: None,
            date1904: false,
            meta: Default::default(),
        }
    }

    /// Adds a warning once (an O(1) check).
    pub fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if self.said.insert(w.clone()) {
            self.warnings.push(w);
        }
    }

    pub fn add_cells(&mut self, n: u64) -> Result<(), XmlssError> {
        self.cells = self.cells.saturating_add(n);
        if self.cells > self.limits.max_cells {
            return Err(XmlssError::LimitExceeded {
                what: "materialized cells",
                limit: self.limits.max_cells,
            });
        }
        Ok(())
    }

    pub fn add_text(&mut self, n: usize) -> Result<(), XmlssError> {
        self.text_bytes = self.text_bytes.saturating_add(n as u64);
        if self.text_bytes > self.limits.max_text_bytes {
            return Err(XmlssError::LimitExceeded {
                what: "materialized cell text bytes",
                limit: self.limits.max_text_bytes,
            });
        }
        Ok(())
    }

    /// The interned index of style id `id` (`None` for an over-long id).
    pub fn style_id(&mut self, id: &str) -> Result<Option<u32>, XmlssError> {
        if id.len() > MAX_STYLE_ID_BYTES {
            return Ok(None);
        }
        if let Some(i) = self.style_index.get(id) {
            return Ok(Some(*i));
        }
        if self.style_ids.len() >= self.limits.max_styles {
            return Err(XmlssError::LimitExceeded {
                what: "styles",
                limit: self.limits.max_styles as u64,
            });
        }
        let i = u32::try_from(self.style_ids.len()).unwrap_or(u32::MAX);
        self.style_ids.push(id.to_string());
        self.style_index.insert(id.to_string(), i);
        Ok(Some(i))
    }

    /// A non-empty sheet name, unique (ignoring ASCII case) among the sheets
    /// read so far: control characters removed, at most
    /// [`MAX_SHEET_NAME_CHARS`] characters, `Sheet<n>` when empty, a ` (2)`
    /// suffix when taken.
    pub fn unique_sheet_name(&mut self, raw: &str, index: usize) -> String {
        let mut base: String = raw
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_SHEET_NAME_CHARS)
            .collect();
        base = base.trim().to_string();
        if base.is_empty() {
            base = format!("Sheet{}", index + 1);
        }
        let mut name = base.clone();
        if self.taken.contains(&name.to_lowercase()) {
            let key = base.to_lowercase();
            let mut n = self.next_suffix.get(&key).copied().unwrap_or(2);
            loop {
                name = format!("{base} ({n})");
                n += 1;
                if !self.taken.contains(&name.to_lowercase()) {
                    break;
                }
            }
            self.next_suffix.insert(key, n);
        }
        if name != raw {
            self.warn("Sheet names that were empty, too long or repeated were changed.");
        }
        self.taken.insert(name.to_lowercase());
        name
    }
}

/// The children of `ss:Workbook`.
fn workbook<R: BufRead>(xr: &mut XmlReader<R>, ctx: &mut Ctx) -> Result<(), XmlssError> {
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "DocumentProperties" => doc_properties(xr, ctx)?,
                "ExcelWorkbook" => excel_workbook(xr, ctx)?,
                "Styles" => styles_list(xr, ctx)?,
                "Names" => names(xr, ctx, None)?,
                "Worksheet" => sheet::worksheet(xr, &e, ctx)?,
                _ => xr.skip()?,
            },
            Ev::End | Ev::Eof => return Ok(()),
            Ev::Text(_) => {}
        }
    }
}

/// `o:DocumentProperties` (template `o:DocumentProperties`): `Title`,
/// `Subject`, `Author` (initial creator), `LastAuthor`, `Created`,
/// `LastSaved`, `Keywords`, `Description`.
fn doc_properties<R: BufRead>(xr: &mut XmlReader<R>, ctx: &mut Ctx) -> Result<(), XmlssError> {
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) => {
                let slot = match e.local() {
                    "Title" => &mut ctx.meta.title,
                    "Subject" => &mut ctx.meta.subject,
                    "Author" => &mut ctx.meta.author,
                    "LastAuthor" => &mut ctx.meta.last_author,
                    "Created" => &mut ctx.meta.created,
                    "LastSaved" => &mut ctx.meta.modified,
                    "Description" => &mut ctx.meta.description,
                    "Keywords" => {
                        let t = xr.text_truncated(MAX_META_TEXT)?;
                        ctx.meta.keywords = t
                            .split([',', ';'])
                            .map(str::trim)
                            .filter(|k| !k.is_empty())
                            .map(str::to_string)
                            .collect();
                        continue;
                    }
                    _ => {
                        xr.skip()?;
                        continue;
                    }
                };
                let t = xr.text_truncated(MAX_META_TEXT)?;
                let t = t.trim();
                *slot = (!t.is_empty()).then(|| t.to_string());
            }
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// `x:ExcelWorkbook`: `x:ActiveSheet` (0-based) and `x:Date1904`.
fn excel_workbook<R: BufRead>(xr: &mut XmlReader<R>, ctx: &mut Ctx) -> Result<(), XmlssError> {
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "ActiveSheet" => {
                    ctx.active_sheet = xr.text_truncated(64)?.trim().parse().ok();
                }
                "Date1904" => {
                    let t = xr.text_truncated(64)?;
                    ctx.date1904 = t.trim().is_empty() || styles::parse_bool(&t);
                }
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// `ss:Styles/ss:Style` (`ss:ID`).
fn styles_list<R: BufRead>(xr: &mut XmlReader<R>, ctx: &mut Ctx) -> Result<(), XmlssError> {
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) if e.local() == "Style" => {
                let st = styles::parse_style(xr, &e)?;
                let Some(id) = e.attr("ID").filter(|i| i.len() <= MAX_STYLE_ID_BYTES) else {
                    continue;
                };
                if ctx.styles.raw.len() >= ctx.limits.max_styles {
                    return Err(XmlssError::LimitExceeded {
                        what: "styles",
                        limit: ctx.limits.max_styles as u64,
                    });
                }
                ctx.styles.raw.insert(id.to_string(), st);
            }
            Ev::Start(_) => xr.skip()?,
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// `ss:Names/ss:NamedRange` (template `Names`), workbook-level or local to
/// sheet `sheet`.
pub(crate) fn names<R: BufRead>(
    xr: &mut XmlReader<R>,
    ctx: &mut Ctx,
    sheet: Option<usize>,
) -> Result<(), XmlssError> {
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) if e.local() == "NamedRange" => {
                named_range(&e, ctx, sheet)?;
                xr.skip()?;
            }
            Ev::Start(_) => xr.skip()?,
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn named_range(e: &Elem, ctx: &mut Ctx, sheet: Option<usize>) -> Result<(), XmlssError> {
    let (Some(name), Some(refers)) = (e.attr("Name"), e.attr("RefersTo")) else {
        return Ok(());
    };
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || refers.len() > MAX_REFERS_TO {
        ctx.warn("Named ranges with empty, over-long or unreadable definitions were dropped.");
        return Ok(());
    }
    if ctx.names.len() >= ctx.limits.max_names {
        return Err(XmlssError::LimitExceeded {
            what: "named ranges",
            limit: ctx.limits.max_names as u64,
        });
    }
    ctx.names
        .push((name.to_string(), sheet, refers.trim().to_string()));
    Ok(())
}

/// Resolves what needed every sheet name: formulas, styles, names, the
/// active sheet.
fn finish(mut ctx: Ctx) -> Result<WorkbookData, XmlssError> {
    if ctx.sheets.is_empty() {
        return Err(XmlssError::NoSheets);
    }
    let names: Vec<String> = ctx.sheets.iter().map(|s| s.name.clone()).collect();
    let mut wb = WorkbookData::default();
    // Formulas.
    let mut failed = 0usize;
    for (si, (sheet, extra)) in ctx.sheets.iter_mut().zip(&ctx.extras).enumerate() {
        for (idx, raw) in &extra.formulas {
            let Some(cell) = sheet.cells.get_mut(*idx) else {
                continue;
            };
            let of = tally_io_xlsx::r1c1_formula_to_of(raw, &names, si, cell.row, cell.col)
                .unwrap_or_else(|| {
                    failed += 1;
                    format!("of:={}", raw.trim().strip_prefix('=').unwrap_or(raw.trim()))
                });
            if let CellContentData::Formula { text, .. } = &mut cell.content {
                *text = Arc::from(of);
            }
        }
    }
    if failed > 0 {
        ctx.warn(format!(
            "{failed} formula(s) could not be translated and show an error; their original text is kept."
        ));
    }
    // Styles.
    let mut model = ModelStyles::default();
    let default_id = ctx.style_id("Default")?;
    let (map, default_font) =
        styles::build(&ctx.styles, &ctx.style_ids, &mut model, &mut ctx.warnings);
    let lookup = |i: Option<u32>| i.and_then(|i| map.get(i as usize).copied().flatten());
    let default_style = lookup(default_id);
    let mut date_styles: HashMap<(Option<u32>, &'static str), Option<u32>> = HashMap::new();
    let date1904 = ctx.date1904;
    for (sheet, extra) in ctx.sheets.iter_mut().zip(&ctx.extras) {
        let mut is_dt = vec![false; sheet.cells.len()];
        for &i in &extra.date_cells {
            if let Some(f) = is_dt.get_mut(i) {
                *f = true;
            }
        }
        for (cell, dt) in sheet.cells.iter_mut().zip(is_dt) {
            let mut style = lookup(cell.style).or(if cell.style.is_none() {
                default_style
            } else {
                None
            });
            let fmt = style
                .and_then(|s| model.styles.get(s as usize))
                .and_then(|s| s.number_format.clone());
            let value = match &mut cell.content {
                CellContentData::Value(v) => Some(v),
                CellContentData::Formula { cached, .. } => cached.as_mut(),
                CellContentData::SharedFormula { .. } => None,
            };
            if let Some(Value::Number(n)) = value {
                if dt {
                    // Time-only values sit on 1899-12-30/31.
                    let time_only = (0.0..2.0).contains(n) && n.fract() != 0.0;
                    let has_time = n.fract() != 0.0;
                    if date1904 {
                        *n -= 1462.0;
                    }
                    if fmt.is_none() {
                        // A DateTime value without a format shows as a date
                        // (AOO gives date-value cells the default date
                        // format).
                        let code: &'static str = if time_only {
                            "HH:MM:SS"
                        } else if has_time {
                            "YYYY-MM-DD HH:MM:SS"
                        } else {
                            "YYYY-MM-DD"
                        };
                        style = *date_styles.entry((style, code)).or_insert_with(|| {
                            let mut st = style
                                .and_then(|s| model.styles.get(s as usize).cloned())
                                .unwrap_or_default();
                            st.number_format = Some(code.to_string());
                            model.intern(st)
                        });
                    }
                } else if fmt.as_deref().is_some_and(tally_io_xlsx::is_date_format) {
                    *n = tally_io_xlsx::excel_serial_to_tally(*n, date1904);
                }
            }
            cell.style = style;
        }
        sheet
            .style_runs
            .retain_mut(|r| match lookup(Some(r.style)) {
                Some(s) => {
                    r.style = s;
                    true
                }
                None => false,
            });
    }
    if date1904 {
        wb.settings.calc_settings.date_epoch = DateEpoch::D1904;
    }
    wb.styles = model.styles;
    wb.settings.default_font = default_font;
    // Names.
    let raw_names = std::mem::take(&mut ctx.names);
    for (name, local, refers) in raw_names {
        resolve_name(&mut ctx, &mut wb, &names, &name, local, &refers);
    }
    // Active and hidden sheets.
    let selected = ctx.extras.iter().position(|e| e.selected);
    let count = ctx.sheets.len();
    let mut active = ctx
        .active_sheet
        .map(|a| a as usize)
        .filter(|a| *a < count)
        .or(selected)
        .unwrap_or(0);
    if ctx.sheets.get(active).is_some_and(|s| s.hidden) {
        match ctx.sheets.iter().position(|s| !s.hidden) {
            Some(v) => active = v,
            None => {
                if let Some(s) = ctx.sheets.get_mut(active) {
                    s.hidden = false;
                }
            }
        }
    }
    wb.settings.active_sheet = u32::try_from(active).unwrap_or(0);
    for s in &mut ctx.sheets {
        s.style_runs.sort_by_key(|r| (r.col, r.start_row));
    }
    wb.sheets = ctx.sheets;
    wb.meta = ctx.meta;
    wb.warnings = ctx.warnings;
    Ok(wb)
}

/// One named range → [`NameData`], or the page setup for `Print_Area` /
/// `Print_Titles`; `_FilterDatabase` and other hidden `_` names are
/// dropped (template `Names` skips them too; `x:AutoFilter` carries the
/// range).
fn resolve_name(
    ctx: &mut Ctx,
    wb: &mut WorkbookData,
    sheet_names: &[String],
    name: &str,
    local: Option<usize>,
    refers: &str,
) {
    let sheet_of = |n: &Option<String>| -> Option<usize> {
        match n {
            Some(n) => sheet_names.iter().position(|s| s.eq_ignore_ascii_case(n)),
            None => local,
        }
    };
    match name {
        "Print_Area" => {
            let parts = refs::split_list(refers.trim_start_matches('='));
            if let [one] = parts.as_slice() {
                if let Some((sheet, r)) = refs::range(one, (0, 0)) {
                    if let Some(s) = sheet_of(&sheet).and_then(|i| ctx.sheets.get_mut(i)) {
                        s.page_setup
                            .get_or_insert_with(PageSetupData::default)
                            .print_range = Some(r);
                        return;
                    }
                }
            }
        }
        "Print_Titles" => {
            for part in refs::split_list(refers.trim_start_matches('=')) {
                let Some((sheet, r)) = refs::range(part, (0, 0)) else {
                    continue;
                };
                let Some(s) = sheet_of(&sheet).and_then(|i| ctx.sheets.get_mut(i)) else {
                    continue;
                };
                let p = s.page_setup.get_or_insert_with(PageSetupData::default);
                let full_rows = r.c0 == 0 && r.c1 == tally_types::MAX_COLS - 1;
                let full_cols = r.r0 == 0 && r.r1 == tally_types::MAX_ROWS - 1;
                if full_rows && !full_cols {
                    p.repeat_rows = Some((r.r0, r.r1));
                } else if full_cols && !full_rows {
                    p.repeat_cols = Some((r.c0, r.c1));
                }
            }
            return;
        }
        n if n.starts_with('_') => return,
        _ => {}
    }
    let Some(a1) = tally_io_xlsx::r1c1_to_a1(refers, 0, 0) else {
        ctx.warn("Named ranges with empty, over-long or unreadable definitions were dropped.");
        return;
    };
    let body = a1.trim().strip_prefix('=').unwrap_or(a1.trim());
    let Some(of) = tally_io_xlsx::excel_formula_to_of(body, sheet_names, local.unwrap_or(0), 0, 0)
    else {
        ctx.warn("Named ranges with empty, over-long or unreadable definitions were dropped.");
        return;
    };
    if ctx.add_text(of.len()).is_err() {
        ctx.warn("Named ranges past the text budget were dropped.");
        return;
    }
    wb.names.push(NameData {
        name: name.to_string(),
        sheet: local.and_then(|l| u32::try_from(l).ok()),
        formula: refs::absolute_sheets(&of),
        base_cell: None,
        usage: Default::default(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff() {
        let ns = r#"<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet">"#;
        assert!(sniff_is_xmlss(ns.as_bytes()));
        assert!(sniff_is_xmlss(
            format!("\u{feff}<?xml version=\"1.0\"?>\n<!-- c -->\n{ns}").as_bytes()
        ));
        assert!(sniff_is_xmlss(
            br#"<?xml version="1.0"?><?mso-application progid="Excel.Sheet"?><x/>"#
        ));
        assert!(sniff_is_xmlss(
            br#"<ss:Workbook xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">"#
        ));
        let mut utf16 = vec![0xFF, 0xFE];
        for u in ns.encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        assert!(sniff_is_xmlss(&utf16));
        for neg in [
            &b""[..],
            b"<Workbook>",
            b"<html><body><table></table></body></html>",
            b"<?xml version=\"1.0\"?><office:document xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\">",
            b"<?xml version=\"1.0\"?><root/>",
            b"<!DOCTYPE x><Workbook xmlns=\"urn:schemas-microsoft-com:office:spreadsheet\">",
            b"PK\x03\x04",
            b"<?xml",
            b"<!--",
        ] {
            assert!(!sniff_is_xmlss(neg), "{}", String::from_utf8_lossy(neg));
        }
        // Only the first 4 KiB count.
        let late = format!("{}{ns}", " ".repeat(5000));
        assert!(!sniff_is_xmlss(late.as_bytes()));
    }

    #[test]
    fn encodings() {
        assert_eq!(
            declared_encoding(br#"<?xml version="1.0" encoding='windows-1252'?>"#).as_deref(),
            Some("windows-1252")
        );
        let latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xe9</a>";
        assert_eq!(
            to_utf8(latin).unwrap(),
            "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\u{e9}</a>"
        );
        assert!(matches!(
            to_utf8(b"<a>\xff</a>"),
            Err(XmlssError::Encoding(_))
        ));
        assert!(matches!(
            to_utf8(b"<?xml version=\"1.0\" encoding=\"klingon\"?><a/>"),
            Err(XmlssError::Encoding(_))
        ));
        assert_eq!(to_utf8(b"\xEF\xBB\xBF<a/>").unwrap(), "<a/>");
        assert_eq!(to_utf8(b"<\0a\0/\0>\0").unwrap(), "<a/>");
        assert_eq!(to_utf8(b"\0<\0a\0/\0>").unwrap(), "<a/>");
        // Odd-length UTF-16 is malformed, never a panic.
        assert!(to_utf8(b"\xFF\xFE<\0a").is_err());
    }

    #[test]
    fn limit_messages() {
        let e = XmlssError::LimitExceeded {
            what: "input bytes",
            limit: 512 * 1024 * 1024,
        };
        assert!(e.to_string().contains("512 MiB"));
    }
}
