//! HTML tables: import and export (wave 5, W5-3).
//!
//! **Import** ([`read_html`]) uses its own tolerant tokenizer (no DOM, no
//! scripts): every `<table>` becomes a sheet (`Table1`, `Table2`, …, or the
//! table's `<caption>`), `<tr>` rows, `<td>`/`<th>` cells; `colspan` and
//! `rowspan` are honoured and produce merged areas; `<br>` breaks lines;
//! `<script>`, `<style>`, `<template>` and comments are skipped; entities
//! (`&amp;`, `&#NNN;`, `&#xHH;`, the common named ones) are decoded;
//! whitespace is collapsed. Cells are text: the engine's codec runs the
//! document locale's number recognition over them, so `1,234.5` and
//! `12.03.2026` become numbers and dates. Nested tables are flattened into
//! their parent cell's text. The same reader serves HTML clipboard paste.
//!
//! **Export** ([`write_html`]) writes one `<table>` per sheet with the
//! sheet name as `<caption>`, inline styles (bold, italic, underline,
//! colour, background, alignment) and the values formatted with their
//! number formats in the document locale; everything is escaped.
//!
//! Limits (docs/security.md): input at most [`MAX_INPUT_BYTES`], at most
//! [`MAX_TABLES`] tables, [`MAX_CELLS`] cells and [`MAX_CELL_BYTES`] bytes
//! per cell; the rest is cut with a warning.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use tally_model::{CellContentData, CellData, CellRangeLocal, HAlign, SheetData, WorkbookData};
use tally_numfmt::{format_value_with, locale_by_tag, parse_format, EN_US};
use tally_types::{Value, MAX_COLS, MAX_ROWS};

/// Largest HTML input (64 MiB).
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
/// Most tables imported.
pub const MAX_TABLES: usize = 1024;
/// Most cells imported (all tables).
pub const MAX_CELLS: usize = 10_000_000;
/// Longest cell text (bytes).
pub const MAX_CELL_BYTES: usize = 64 * 1024;
/// Most cells one export writes.
pub const MAX_EXPORT_CELLS: u64 = 10_000_000;

/// Errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    /// A limit was hit.
    #[error("limit exceeded: {what} (limit {limit})")]
    LimitExceeded {
        /// What.
        what: &'static str,
        /// The limit.
        limit: u64,
    },
    /// No table in the input.
    #[error("the HTML contains no table")]
    NoTable,
}

/// `true` if `bytes` look like HTML (a tag within the first 4 KiB).
pub fn sniff_is_html(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
    let t = lower.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    t.starts_with("<!doctype html")
        || t.starts_with("<html")
        || lower.contains("<table")
        || lower.contains("<html")
        || lower.contains("<body")
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Tok<'a> {
    Open {
        name: String,
        attrs: &'a str,
        self_closing: bool,
    },
    Close(String),
    Text(&'a str),
}

/// A tolerant tokenizer: tags, closing tags and text; comments, CDATA,
/// doctypes and processing instructions are dropped; `<script>`,
/// `<style>` and `<template>` bodies are skipped as raw text.
struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
    skip_until: Option<&'static str>,
}

impl<'a> Tokenizer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            skip_until: None,
        }
    }

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            if self.pos >= self.src.len() {
                return None;
            }
            if let Some(end_tag) = self.skip_until {
                // Raw text element: find `</name`.
                let rest = &self.src[self.pos..];
                let lower = rest.to_ascii_lowercase();
                let needle = format!("</{end_tag}");
                match lower.find(&needle) {
                    Some(i) => {
                        self.pos += i;
                        self.skip_until = None;
                        continue;
                    }
                    None => {
                        self.pos = self.src.len();
                        return None;
                    }
                }
            }
            let rest = &self.src[self.pos..];
            if let Some(after) = rest.strip_prefix('<') {
                if after.starts_with("!--") {
                    match after.find("-->") {
                        Some(i) => self.pos += 1 + i + 3,
                        None => self.pos = self.src.len(),
                    }
                    continue;
                }
                if after.starts_with('!') || after.starts_with('?') {
                    match after.find('>') {
                        Some(i) => self.pos += 1 + i + 1,
                        None => self.pos = self.src.len(),
                    }
                    continue;
                }
                let (closing, body) = match after.strip_prefix('/') {
                    Some(b) => (true, b),
                    None => (false, after),
                };
                let name_len = body
                    .char_indices()
                    .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-' || *c == ':'))
                    .map_or(body.len(), |(i, _)| i);
                if name_len == 0 {
                    // A lone `<`: text.
                    let end = rest[1..].find('<').map_or(rest.len(), |i| i + 1);
                    let t = &rest[..end];
                    self.pos += end;
                    return Some(Tok::Text(t));
                }
                let name = body[..name_len].to_ascii_lowercase();
                // Attributes up to `>` (quotes may hold `>`).
                let mut i = name_len;
                let b = body.as_bytes();
                let mut quote: Option<u8> = None;
                while i < b.len() {
                    match (quote, b[i]) {
                        (Some(q), c) if c == q => quote = None,
                        (Some(_), _) => {}
                        (None, b'"') | (None, b'\'') => quote = Some(b[i]),
                        (None, b'>') => break,
                        _ => {}
                    }
                    i += 1;
                }
                let attrs = &body[name_len..i.min(body.len())];
                let self_closing = attrs.trim_end().ends_with('/');
                self.pos += 1 + usize::from(closing) + i + usize::from(i < b.len());
                if closing {
                    return Some(Tok::Close(name));
                }
                if matches!(name.as_str(), "script" | "style" | "template") && !self_closing {
                    self.skip_until = Some(match name.as_str() {
                        "script" => "script",
                        "style" => "style",
                        _ => "template",
                    });
                }
                return Some(Tok::Open {
                    name,
                    attrs,
                    self_closing,
                });
            }
            let end = rest.find('<').unwrap_or(rest.len());
            let t = &rest[..end];
            self.pos += end;
            if t.is_empty() {
                continue;
            }
            return Some(Tok::Text(t));
        }
    }
}

fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(key) {
        let at = from + i;
        let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
        let after = &attrs[at + key.len()..];
        let after_t = after.trim_start();
        if before_ok {
            if let Some(v) = after_t.strip_prefix('=') {
                let v = v.trim_start();
                if let Some(q) = v.strip_prefix('"') {
                    return q.split('"').next();
                }
                if let Some(q) = v.strip_prefix('\'') {
                    return q.split('\'').next();
                }
                return Some(
                    v.split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                        .next()
                        .unwrap_or(""),
                );
            }
        }
        from = at + key.len();
    }
    None
}

/// Decodes HTML entities.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after
            .char_indices()
            .take(12)
            .find(|(_, c)| *c == ';')
            .map(|(j, _)| j);
        match end {
            Some(j) => {
                let name = &after[..j];
                let decoded = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    "euro" => Some('€'),
                    "pound" => Some('£'),
                    "yen" => Some('¥'),
                    "copy" => Some('©'),
                    "reg" => Some('®'),
                    "deg" => Some('°'),
                    "middot" => Some('·'),
                    "ndash" => Some('–'),
                    "mdash" => Some('—'),
                    "lsquo" => Some('‘'),
                    "rsquo" => Some('’'),
                    "ldquo" => Some('“'),
                    "rdquo" => Some('”'),
                    "hellip" => Some('…'),
                    "times" => Some('×'),
                    "divide" => Some('÷'),
                    "plusmn" => Some('±'),
                    "frac12" => Some('½'),
                    "frac14" => Some('¼'),
                    "frac34" => Some('¾'),
                    "szlig" => Some('ß'),
                    "auml" => Some('ä'),
                    "ouml" => Some('ö'),
                    "uuml" => Some('ü'),
                    "Auml" => Some('Ä'),
                    "Ouml" => Some('Ö'),
                    "Uuml" => Some('Ü'),
                    "eacute" => Some('é'),
                    "egrave" => Some('è'),
                    "agrave" => Some('à'),
                    "ccedil" => Some('ç'),
                    "ntilde" => Some('ñ'),
                    _ => {
                        let code = name
                            .strip_prefix("#x")
                            .or_else(|| name.strip_prefix("#X"))
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .or_else(|| name.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()));
                        code.and_then(char::from_u32).filter(|c| *c != '\0')
                    }
                };
                match decoded {
                    Some(c) => {
                        out.push(c);
                        rest = &after[j + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = after;
                    }
                }
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        if ch == '\n' {
            // Explicit line breaks (from <br>) are kept.
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            last_space = true;
        } else if ch.is_whitespace() && ch != '\u{a0}' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim_matches(|c: char| c == ' ' || c == '\n')
        .to_string()
}

struct TableBuilder {
    sheet: SheetData,
    row: i64,
    col: u32,
    /// `(row, col) → still covered` by a rowspan.
    covered: Vec<(u32, u32, u32)>, // (col, first_row, last_row)
    cell: Option<(u32, u32, u32, u32, String)>, // r, c, rowspan, colspan, text
    cells: usize,
}

impl TableBuilder {
    fn new(name: String) -> Self {
        Self {
            sheet: SheetData {
                name,
                ..SheetData::default()
            },
            row: -1,
            col: 0,
            covered: Vec::new(),
            cell: None,
            cells: 0,
        }
    }

    fn start_row(&mut self) {
        self.finish_cell();
        self.row += 1;
        self.col = 0;
    }

    fn skip_covered(&mut self) {
        let r = self.row.max(0) as u32;
        loop {
            let c = self.col;
            if self
                .covered
                .iter()
                .any(|(cc, r0, r1)| *cc == c && *r0 <= r && r <= *r1)
            {
                self.col += 1;
            } else {
                break;
            }
        }
    }

    fn start_cell(&mut self, attrs: &str) {
        self.finish_cell();
        if self.row < 0 {
            self.row = 0;
        }
        self.skip_covered();
        let span = |k: &str| {
            attr(attrs, k)
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(1)
                .clamp(1, 1000)
        };
        self.cell = Some((
            self.row as u32,
            self.col,
            span("rowspan"),
            span("colspan"),
            String::new(),
        ));
    }

    fn text(&mut self, t: &str) {
        if let Some(c) = self.cell.as_mut() {
            if c.4.len() < MAX_CELL_BYTES {
                c.4.push_str(t);
            }
        }
    }

    fn finish_cell(&mut self) {
        let Some((r, c, rs, cs, raw)) = self.cell.take() else {
            return;
        };
        let text = collapse_ws(&decode_entities(&raw));
        if r < MAX_ROWS && c < MAX_COLS {
            if !text.is_empty() {
                self.sheet.cells.push(CellData {
                    row: r,
                    col: c,
                    content: CellContentData::Value(Value::text(text.as_str())),
                    style: None,
                });
                self.cells += 1;
            }
            if rs > 1 || cs > 1 {
                let r1 = (r + rs - 1).min(MAX_ROWS - 1);
                let c1 = (c + cs - 1).min(MAX_COLS - 1);
                self.sheet.merges.push(CellRangeLocal {
                    r0: r,
                    c0: c,
                    r1,
                    c1,
                });
                if rs > 1 {
                    for cc in c..=c1 {
                        self.covered.push((cc, r + 1, r1));
                    }
                }
            }
        }
        self.col = c + cs;
    }

    fn finish(mut self) -> SheetData {
        self.finish_cell();
        self.sheet
    }
}

/// Reads every table of an HTML document into one sheet each.
pub fn read_html(bytes: &[u8]) -> Result<(WorkbookData, Vec<String>), HtmlError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(HtmlError::LimitExceeded {
            what: "input size",
            limit: MAX_INPUT_BYTES as u64,
        });
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut tok = Tokenizer::new(text);
    let mut sheets: Vec<SheetData> = Vec::new();
    let mut stack: Vec<TableBuilder> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut caption: Option<String> = None;
    let mut in_caption = false;
    let mut total_cells = 0usize;
    let mut tables_seen = 0usize;
    let mut truncated = false;
    while let Some(t) = tok.next() {
        match t {
            Tok::Open {
                name,
                attrs,
                self_closing,
            } => match name.as_str() {
                "table" => {
                    if !stack.is_empty() {
                        // Nested table: its text flows into the parent cell.
                        continue;
                    }
                    tables_seen += 1;
                    if sheets.len() >= MAX_TABLES {
                        truncated = true;
                        continue;
                    }
                    stack.push(TableBuilder::new(format!("Table{}", sheets.len() + 1)));
                    caption = None;
                }
                "caption" => {
                    in_caption = stack.len() == 1;
                    if in_caption {
                        caption = Some(String::new());
                    }
                }
                "tr" => {
                    if let Some(b) = stack.last_mut() {
                        b.start_row();
                    }
                }
                "td" | "th" => {
                    if let Some(b) = stack.last_mut() {
                        if total_cells + b.cells >= MAX_CELLS {
                            truncated = true;
                            continue;
                        }
                        b.start_cell(attrs);
                    }
                }
                "br" => {
                    if let Some(b) = stack.last_mut() {
                        b.text("\n");
                    }
                }
                "p" | "div" | "li" => {
                    if let Some(b) = stack.last_mut() {
                        b.text(" ");
                    }
                }
                _ => {
                    let _ = self_closing;
                }
            },
            Tok::Close(name) => match name.as_str() {
                "table" => {
                    if let Some(b) = stack.pop() {
                        let mut s = b.finish();
                        if let Some(c) = caption.take() {
                            let c = collapse_ws(&decode_entities(&c));
                            if !c.is_empty() {
                                s.name = c.chars().take(64).collect();
                            }
                        }
                        total_cells += s.cells.len();
                        sheets.push(s);
                    }
                }
                "caption" => in_caption = false,
                "td" | "th" => {
                    if let Some(b) = stack.last_mut() {
                        b.finish_cell();
                    }
                }
                "tr" => {
                    if let Some(b) = stack.last_mut() {
                        b.finish_cell();
                    }
                }
                _ => {}
            },
            Tok::Text(t) => {
                if in_caption {
                    if let Some(c) = caption.as_mut() {
                        if c.len() < 1024 {
                            c.push_str(t);
                        }
                    }
                } else if let Some(b) = stack.last_mut() {
                    b.text(t);
                }
            }
        }
    }
    if let Some(b) = stack.pop() {
        let s = b.finish();
        sheets.push(s);
    }
    // Tables without any cell content (layout tables, `<table></table>`)
    // are dropped.
    sheets.retain(|s| !s.cells.is_empty());
    if sheets.is_empty() {
        return Err(HtmlError::NoTable);
    }
    if truncated {
        warnings.push(format!(
            "Only the first {} tables and {} cells were imported.",
            MAX_TABLES, MAX_CELLS
        ));
    }
    let _ = tables_seen;
    // Unique sheet names.
    let mut seen: Vec<String> = Vec::new();
    for s in &mut sheets {
        let base = s.name.clone();
        let mut name = base.clone();
        let mut n = 1;
        while seen.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
            n += 1;
            name = format!("{base} ({n})");
        }
        seen.push(name.clone());
        s.name = name;
    }
    Ok((
        WorkbookData {
            sheets,
            ..WorkbookData::default()
        },
        warnings,
    ))
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Escapes text for HTML.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
    out
}

fn css_color(c: u32) -> String {
    format!("#{:06x}", c >> 8)
}

/// Writes the workbook as an HTML document, one `<table>` per sheet.
pub fn write_html(data: &WorkbookData) -> Result<String, HtmlError> {
    let locale = data
        .settings
        .locale
        .as_deref()
        .and_then(locale_by_tag)
        .unwrap_or(&EN_US);
    let epoch = data.settings.calc_settings.date_epoch;
    let formats: Vec<Option<tally_numfmt::FormatAst>> = data
        .styles
        .iter()
        .map(|s| {
            s.number_format
                .as_deref()
                .and_then(|c| parse_format(c).ok())
        })
        .collect();
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"generator\" content=\"Tally\">\n");
    if let Some(t) = &data.meta.title {
        out.push_str(&format!("<title>{}</title>\n", escape(t)));
    }
    out.push_str("<style>table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:2px 6px;vertical-align:bottom}</style>\n");
    out.push_str("</head>\n<body>\n");
    let mut written: u64 = 0;
    for sheet in &data.sheets {
        if sheet.hidden {
            continue;
        }
        let (mut rows, mut cols) = (0u32, 0u32);
        for c in &sheet.cells {
            rows = rows.max(c.row + 1);
            cols = cols.max(c.col + 1);
        }
        written += u64::from(rows) * u64::from(cols);
        if written > MAX_EXPORT_CELLS {
            return Err(HtmlError::LimitExceeded {
                what: "exported cells",
                limit: MAX_EXPORT_CELLS,
            });
        }
        let mut grid: std::collections::HashMap<(u32, u32), &CellData> =
            std::collections::HashMap::new();
        for c in &sheet.cells {
            grid.insert((c.row, c.col), c);
        }
        let covered = |r: u32, c: u32| {
            sheet.merges.iter().any(|m| {
                m.r0 <= r && r <= m.r1 && m.c0 <= c && c <= m.c1 && !(m.r0 == r && m.c0 == c)
            })
        };
        out.push_str(&format!(
            "<table>\n<caption>{}</caption>\n",
            escape(&sheet.name)
        ));
        for r in 0..rows {
            out.push_str("<tr>");
            for c in 0..cols {
                if covered(r, c) {
                    continue;
                }
                let mut tag = String::from("<td");
                if let Some(m) = sheet.merges.iter().find(|m| m.r0 == r && m.c0 == c) {
                    if m.c1 > m.c0 {
                        tag.push_str(&format!(" colspan=\"{}\"", m.c1 - m.c0 + 1));
                    }
                    if m.r1 > m.r0 {
                        tag.push_str(&format!(" rowspan=\"{}\"", m.r1 - m.r0 + 1));
                    }
                }
                let cell = grid.get(&(r, c));
                let style = cell
                    .and_then(|cd| cd.style)
                    .and_then(|i| data.styles.get(i as usize));
                let mut css = String::new();
                let mut text = String::new();
                let mut numeric = false;
                if let Some(cd) = cell {
                    let value = match &cd.content {
                        CellContentData::Value(v) => Some(v.clone()),
                        CellContentData::Formula { cached, .. }
                        | CellContentData::SharedFormula { cached, .. } => cached.clone(),
                    };
                    if let Some(v) = value {
                        numeric = matches!(v, Value::Number(_));
                        let fmt = cd
                            .style
                            .and_then(|i| formats.get(i as usize))
                            .and_then(|f| f.as_ref());
                        text = match (fmt, &v) {
                            (Some(ast), v) => format_value_with(v, ast, locale, None, epoch).text,
                            (None, Value::Number(n)) => {
                                tally_numfmt::general_string(*n, None, locale)
                            }
                            (None, Value::Text(t)) => t.to_string(),
                            (None, Value::Bool(b)) => {
                                if *b {
                                    "TRUE".into()
                                } else {
                                    "FALSE".into()
                                }
                            }
                            (None, Value::Error(e)) => e.display().into_owned(),
                            (None, _) => String::new(),
                        };
                    }
                }
                if let Some(st) = style {
                    if st.font.bold {
                        css.push_str("font-weight:bold;");
                    }
                    if st.font.italic {
                        css.push_str("font-style:italic;");
                    }
                    if st.font.underline {
                        css.push_str("text-decoration:underline;");
                    }
                    if let Some(c) = st.font.color {
                        css.push_str(&format!("color:{};", css_color(c)));
                    }
                    if let Some(c) = st.fill {
                        css.push_str(&format!("background:{};", css_color(c)));
                    }
                    if let Some(f) = &st.font.family {
                        css.push_str(&format!("font-family:{};", escape(f).replace(';', "")));
                    }
                    if let Some(s) = st.font.size_pt {
                        css.push_str(&format!("font-size:{s}pt;"));
                    }
                    match st.align.h {
                        HAlign::Left => css.push_str("text-align:left;"),
                        HAlign::Center => css.push_str("text-align:center;"),
                        HAlign::Right => css.push_str("text-align:right;"),
                        HAlign::Justify => css.push_str("text-align:justify;"),
                        HAlign::General => {}
                    }
                }
                if numeric && !css.contains("text-align") {
                    css.push_str("text-align:right;");
                }
                if !css.is_empty() {
                    tag.push_str(&format!(" style=\"{css}\""));
                }
                tag.push('>');
                out.push_str(&tag);
                out.push_str(&escape(&text));
                out.push_str("</td>");
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</table>\n");
    }
    out.push_str("</body>\n</html>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_and_skips_scripts() {
        let html = r#"<html><script>var x = "<table><tr><td>no</td></tr></table>";</script>
        <!-- <table><tr><td>comment</td></tr></table> -->
        <table><caption>Sales &amp; Co</caption><tr><th>A</th><th colspan="2">B</th></tr>
        <tr><td>1,5</td><td rowspan=2>x<br>y</td><td>&#65;&#x42;</td></tr><tr><td>2</td><td>3</td></tr></table>
        <p>text</p><table><tr><td> spaced   out </td></tr>"#;
        let (wb, w) = read_html(html.as_bytes()).unwrap();
        assert!(w.is_empty());
        assert_eq!(wb.sheets.len(), 2);
        let s = &wb.sheets[0];
        assert_eq!(s.name, "Sales & Co");
        let get = |r: u32, c: u32| {
            s.cells
                .iter()
                .find(|x| x.row == r && x.col == c)
                .map(|x| match &x.content {
                    CellContentData::Value(Value::Text(t)) => t.to_string(),
                    other => format!("{other:?}"),
                })
        };
        assert_eq!(get(0, 1).as_deref(), Some("B"));
        assert_eq!(get(1, 0).as_deref(), Some("1,5"));
        assert_eq!(get(1, 1).as_deref(), Some("x\ny"));
        assert_eq!(get(1, 2).as_deref(), Some("AB"));
        assert_eq!(get(2, 0).as_deref(), Some("2"));
        assert_eq!(get(2, 2).as_deref(), Some("3"), "rowspan skipped");
        assert_eq!(s.merges.len(), 2);
        assert_eq!(wb.sheets[1].name, "Table2");
        assert_eq!(
            wb.sheets[1].cells[0].content,
            CellContentData::Value(Value::text("spaced out"))
        );
        assert!(matches!(
            read_html(b"<p>no tables</p>"),
            Err(HtmlError::NoTable)
        ));
        assert!(sniff_is_html(b"  <!DOCTYPE html><html>"));
        assert!(!sniff_is_html(b"a,b,c\n1,2,3"));
    }

    #[test]
    fn export_escapes_and_round_trips() {
        let mut wb = WorkbookData::default();
        wb.styles.push(tally_model::StyleData {
            number_format: Some("0.00".into()),
            font: tally_model::FontData {
                bold: true,
                ..Default::default()
            },
            ..Default::default()
        });
        wb.sheets.push(SheetData {
            name: "S<1>".into(),
            cells: vec![
                CellData {
                    row: 0,
                    col: 0,
                    content: CellContentData::Value(Value::text("a&b")),
                    style: None,
                },
                CellData {
                    row: 0,
                    col: 1,
                    content: CellContentData::Value(Value::Number(1.5)),
                    style: Some(0),
                },
            ],
            ..SheetData::default()
        });
        let html = write_html(&wb).unwrap();
        assert!(html.contains("<caption>S&lt;1&gt;</caption>"));
        assert!(html.contains(">a&amp;b</td>"));
        assert!(html.contains("font-weight:bold;"));
        assert!(html.contains(">1.50</td>"));
        let (back, _) = read_html(html.as_bytes()).unwrap();
        assert_eq!(back.sheets[0].name, "S<1>");
        assert_eq!(back.sheets[0].cells.len(), 2);
    }
}
