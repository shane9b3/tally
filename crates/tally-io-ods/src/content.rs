//! Streaming reader for `content.xml`.
//!
//! Rows and cells are parsed as run-length *runs* first and only expanded
//! into [`CellData`] after the materialized cell count has been checked
//! against [`Limits::max_cells`], so repeat-count bombs fail fast instead of
//! allocating (§5.4 requirement 3).

use std::io::BufRead;
use std::sync::Arc;

use tally_model::{CellContentData, CellData, CellRangeLocal, DimRun, SheetData, StyleRun};
use tally_types::{
    CalcSettings, CriteriaMode, DateEpoch, ErrorKind, IterSettings, Value, MAX_COLS, MAX_ROWS,
};

use crate::styles::{Origin, StyleSheet};
use crate::units::{date_value_to_serial, duration_to_days, epoch_from_null_date};
use crate::xml::{Elem, Ev, XmlReader};
use crate::{Limits, OdsError, DEFAULT_COL_WIDTH_HMM};

/// Maximum bytes of text in one cell.
const MAX_CELL_TEXT: usize = 16 * 1024 * 1024;
/// Maximum length of a `table:formula` attribute in characters (the engine's
/// parser limit; a longer formula could never be parsed anyway).
pub(crate) const MAX_FORMULA_LEN: usize = 8192;
/// Styled-but-empty runs larger than this (rows × columns) are kept as
/// [`StyleRun`]s (one per column) instead of being materialized as cells.
const MAX_STYLED_EMPTY_RUN: u64 = 4096;

/// What content.xml yields.
#[derive(Debug, Default)]
pub(crate) struct Content {
    pub sheets: Vec<SheetData>,
    pub calc: Option<CalcSettings>,
    pub is_spreadsheet: bool,
}

struct CellParsed {
    repeat: u32,
    content: CellContentData,
    style: Option<String>,
    span: (u32, u32),
}

struct Run {
    col: u32,
    len: u32,
    cell: CellParsed,
}

struct Parser<'a> {
    styles: &'a mut StyleSheet,
    limits: &'a Limits,
    cells_total: u64,
    /// Text and formula bytes of every materialized cell so far (a shared
    /// `Arc` counts once per cell, so this tracks the dense expansion).
    text_bytes: u64,
    sheets: usize,
    epoch: DateEpoch,
}

/// Bytes of text a cell's content carries (formula text plus text value).
fn content_bytes(c: &CellContentData) -> u64 {
    let value_bytes = |v: &Value| match v {
        Value::Text(t) => t.len() as u64,
        _ => 0,
    };
    match c {
        CellContentData::Value(v) => value_bytes(v),
        CellContentData::Formula { text, cached } => {
            text.len() as u64 + cached.as_ref().map_or(0, value_bytes)
        }
    }
}

/// Parses `content.xml`, adding its automatic styles to `styles`.
pub(crate) fn parse_content<R: BufRead>(
    xml: &mut XmlReader<R>,
    styles: &mut StyleSheet,
    limits: &Limits,
) -> Result<Content, OdsError> {
    let mut out = Content::default();
    let mut p = Parser {
        styles,
        limits,
        cells_total: 0,
        text_bytes: 0,
        sheets: 0,
        epoch: DateEpoch::default(),
    };
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "office:document-content" | "office:body" => {}
                "office:font-face-decls" => p.styles.parse_font_faces(xml)?,
                "office:automatic-styles" => p.styles.parse_block(xml, Origin::Content)?,
                "office:spreadsheet" => out.is_spreadsheet = true,
                "table:calculation-settings" if out.is_spreadsheet => {
                    let calc = parse_calc_settings(xml, &e)?;
                    p.epoch = calc.date_epoch;
                    out.calc = Some(calc);
                }
                "table:table" if out.is_spreadsheet => {
                    p.sheets += 1;
                    if p.sheets > p.limits.max_sheets {
                        return Err(OdsError::LimitExceeded {
                            what: "sheets",
                            limit: p.limits.max_sheets as u64,
                        });
                    }
                    let sheet = p.parse_table(xml, &e)?;
                    out.sheets.push(sheet);
                }
                _ => xml.skip()?,
            },
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn parse_count(e: &Elem, key: &str, max: u32) -> u32 {
    e.attr(key)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(1, |n| n.clamp(1, u64::from(max)) as u32)
}

fn push_run(runs: &mut Vec<DimRun>, start: u32, end: u32, size_hmm: u32, hidden: bool) {
    if let Some(last) = runs.last_mut() {
        if last.end.checked_add(1) == Some(start)
            && last.size_hmm == size_hmm
            && last.hidden == hidden
        {
            last.end = end;
            return;
        }
    }
    runs.push(DimRun {
        start,
        end,
        size_hmm,
        hidden,
    });
}

impl Parser<'_> {
    fn parse_table<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
    ) -> Result<SheetData, OdsError> {
        let mut sheet = SheetData {
            name: e.attr("table:name").unwrap_or_default().to_string(),
            ..SheetData::default()
        };
        if let Some(ts) = e
            .attr("table:style-name")
            .and_then(|n| self.styles.table_style(n))
        {
            sheet.hidden = ts.hidden;
            sheet.tab_color = ts.tab_color;
        }
        let mut col: u32 = 0;
        let mut col_defaults: Vec<(u32, u32, String)> = Vec::new();
        let mut col_runs_done = false;
        let mut row: u32 = 0;
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => match c.name.as_str() {
                    "table:table-column" => {
                        let rep = parse_count(&c, "table:number-columns-repeated", MAX_COLS);
                        if col < MAX_COLS {
                            let end = (u64::from(col) + u64::from(rep) - 1)
                                .min(u64::from(MAX_COLS - 1))
                                as u32;
                            let width = c
                                .attr("table:style-name")
                                .and_then(|n| self.styles.column_width(n))
                                .unwrap_or(DEFAULT_COL_WIDTH_HMM);
                            let hidden = c.attr("table:visibility").is_some_and(|v| v != "visible");
                            if width != DEFAULT_COL_WIDTH_HMM || hidden {
                                push_run(&mut sheet.col_widths, col, end, width, hidden);
                            }
                            if let Some(ds) = c
                                .attr("table:default-cell-style-name")
                                .filter(|n| *n != "Default")
                            {
                                col_defaults.push((col, end, ds.to_string()));
                            }
                        }
                        col = col.saturating_add(rep);
                        xml.skip()?;
                    }
                    "table:table-row" => {
                        if !col_runs_done {
                            // A column default cell style formats the whole
                            // column, empty cells included. Emitted before
                            // the row runs, which override it.
                            col_runs_done = true;
                            self.push_column_defaults(&mut sheet, &col_defaults)?;
                        }
                        self.parse_row(xml, &c, &mut row, &mut sheet, &col_defaults)?;
                    }
                    "table:table-header-columns"
                    | "table:table-columns"
                    | "table:table-column-group"
                    | "table:table-header-rows"
                    | "table:table-rows"
                    | "table:table-row-group" => {}
                    _ => xml.skip()?,
                },
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }
        if !col_runs_done {
            self.push_column_defaults(&mut sheet, &col_defaults)?;
        }
        Ok(sheet)
    }

    /// Counts `n` style runs against the materialized-cell limit.
    fn count_runs(&mut self, n: u64) -> Result<(), OdsError> {
        self.cells_total = self.cells_total.saturating_add(n);
        if self.cells_total > self.limits.max_cells {
            return Err(OdsError::LimitExceeded {
                what: "materialized cells",
                limit: self.limits.max_cells,
            });
        }
        Ok(())
    }

    fn push_column_defaults(
        &mut self,
        sheet: &mut SheetData,
        col_defaults: &[(u32, u32, String)],
    ) -> Result<(), OdsError> {
        for (c0, c1, name) in col_defaults {
            let Some(style) = self.styles.resolve(name) else {
                continue;
            };
            self.count_runs(u64::from(c1 - c0) + 1)?;
            for col in *c0..=*c1 {
                sheet.style_runs.push(StyleRun {
                    col,
                    start_row: 0,
                    end_row: MAX_ROWS - 1,
                    style,
                });
            }
        }
        Ok(())
    }

    fn parse_row<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
        row: &mut u32,
        sheet: &mut SheetData,
        col_defaults: &[(u32, u32, String)],
    ) -> Result<(), OdsError> {
        let rep = parse_count(e, "table:number-rows-repeated", MAX_ROWS);
        let row_default = e
            .attr("table:default-cell-style-name")
            .filter(|n| *n != "Default")
            .map(str::to_string);
        let height = e
            .attr("table:style-name")
            .and_then(|n| self.styles.row_style(n))
            .and_then(|rs| if rs.optimal { None } else { rs.height_hmm });
        let hidden = e.attr("table:visibility").is_some_and(|v| v != "visible");

        let mut runs: Vec<Run> = Vec::new();
        let mut col: u32 = 0;
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => match c.name.as_str() {
                    "table:table-cell" | "table:covered-table-cell" => {
                        let cell = self.parse_cell(xml, &c)?;
                        let crep = cell.repeat;
                        if col < MAX_COLS {
                            let len = crep.min(MAX_COLS - col);
                            let keep =
                                !matches!(cell.content, CellContentData::Value(Value::Empty))
                                    || cell.style.is_some()
                                    || cell.span != (1, 1);
                            if keep {
                                runs.push(Run { col, len, cell });
                            }
                        }
                        col = col.saturating_add(crep);
                    }
                    _ => xml.skip()?,
                },
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }

        if *row >= MAX_ROWS {
            return Ok(());
        }
        let rep = rep.min(MAX_ROWS - *row);
        let last = *row + (rep - 1);
        if height.is_some() || hidden {
            let h = height.unwrap_or(crate::DEFAULT_ROW_HEIGHT_HMM);
            push_run(&mut sheet.row_heights, *row, last, h, hidden);
        }

        // Resolve styles and decide which runs materialize.
        let mut kept: Vec<(u32, u32, CellContentData, Option<u32>)> = Vec::new();
        let mut per_row: u64 = 0;
        let mut per_row_bytes: u64 = 0;
        for run in runs {
            let non_empty = !matches!(run.cell.content, CellContentData::Value(Value::Empty));
            if run.cell.span != (1, 1) && rep == 1 && run.len == 1 {
                // Merges count against the cell cap like cells do.
                self.cells_total += 1;
                if self.cells_total > self.limits.max_cells {
                    return Err(OdsError::LimitExceeded {
                        what: "materialized cells",
                        limit: self.limits.max_cells,
                    });
                }
                let (cs, rs) = run.cell.span;
                sheet.merges.push(CellRangeLocal {
                    r0: *row,
                    c0: run.col,
                    r1: (*row).saturating_add(rs - 1).min(MAX_ROWS - 1),
                    c1: run.col.saturating_add(cs - 1).min(MAX_COLS - 1),
                });
            }
            let name = run.cell.style.clone().or_else(|| {
                if non_empty {
                    row_default.clone().or_else(|| {
                        col_defaults
                            .iter()
                            .find(|(s, e, _)| (*s..=*e).contains(&run.col))
                            .map(|(_, _, n)| n.clone())
                    })
                } else {
                    None
                }
            });
            let style = name.and_then(|n| self.styles.resolve(&n));
            if !non_empty {
                let Some(st) = style else {
                    continue;
                };
                if u64::from(run.len) * u64::from(rep) > MAX_STYLED_EMPTY_RUN {
                    // Formatting of a large blank area: keep it as runs.
                    self.count_runs(u64::from(run.len))?;
                    for col in run.col..run.col + run.len {
                        sheet.style_runs.push(StyleRun {
                            col,
                            start_row: *row,
                            end_row: last,
                            style: st,
                        });
                    }
                    continue;
                }
            }
            per_row += u64::from(run.len);
            per_row_bytes = per_row_bytes.saturating_add(
                u64::from(run.len).saturating_mul(content_bytes(&run.cell.content)),
            );
            kept.push((run.col, run.len, run.cell.content, style));
        }
        if per_row == 0 {
            *row = row.saturating_add(rep);
            return Ok(());
        }
        let eff = rep.min(self.limits.max_repeat.max(1));
        let total = per_row.saturating_mul(u64::from(eff));
        if self.cells_total.saturating_add(total) > self.limits.max_cells {
            return Err(OdsError::LimitExceeded {
                what: "materialized cells",
                limit: self.limits.max_cells,
            });
        }
        let bytes = per_row_bytes.saturating_mul(u64::from(eff));
        if self.text_bytes.saturating_add(bytes) > self.limits.max_text_bytes {
            return Err(OdsError::LimitExceeded {
                what: "materialized cell text bytes",
                limit: self.limits.max_text_bytes,
            });
        }
        self.cells_total += total;
        self.text_bytes += bytes;
        sheet.cells.reserve(total.min(1 << 20) as usize);
        for r in 0..eff {
            for (c0, len, content, style) in &kept {
                for k in 0..*len {
                    sheet.cells.push(CellData {
                        row: *row + r,
                        col: c0 + k,
                        content: content.clone(),
                        style: *style,
                    });
                }
            }
        }
        *row = row.saturating_add(rep);
        Ok(())
    }

    fn parse_cell<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
    ) -> Result<CellParsed, OdsError> {
        let repeat = parse_count(e, "table:number-columns-repeated", MAX_COLS);
        let span = (
            parse_count(e, "table:number-columns-spanned", MAX_COLS),
            parse_count(e, "table:number-rows-spanned", MAX_ROWS),
        );
        let mut paragraphs: Vec<String> = Vec::new();
        let mut budget = MAX_CELL_TEXT;
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => {
                    if c.name == "text:p" {
                        let p = parse_paragraph(xml, &mut budget)?;
                        paragraphs.push(p);
                    } else {
                        xml.skip()?;
                    }
                }
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }
        let text = (!paragraphs.is_empty()).then(|| paragraphs.join("\n"));
        let formula: Option<Arc<str>> = match e.attr("table:formula") {
            Some(f) if f.chars().count() > MAX_FORMULA_LEN => {
                return Err(OdsError::LimitExceeded {
                    what: "formula length",
                    limit: MAX_FORMULA_LEN as u64,
                });
            }
            Some(f) => Some(Arc::from(f)),
            None => None,
        };
        if e.attr("office:string-value")
            .is_some_and(|s| s.len() > MAX_CELL_TEXT)
        {
            return Err(OdsError::LimitExceeded {
                what: "cell text bytes",
                limit: MAX_CELL_TEXT as u64,
            });
        }
        let vt = e.attr("office:value-type");
        let number = |key: &str| e.attr(key).and_then(|v| v.trim().parse::<f64>().ok());
        let mut value = match vt {
            Some("float" | "percentage" | "currency") => match number("office:value") {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("date") => match e
                .attr("office:date-value")
                .and_then(|v| date_value_to_serial(v, self.epoch))
                .or_else(|| number("office:value"))
            {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("time") => match e
                .attr("office:time-value")
                .and_then(duration_to_days)
                .or_else(|| number("office:value"))
            {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("boolean") => match e.attr("office:boolean-value").map(str::trim) {
                Some("true" | "1") => Value::Bool(true),
                Some("false" | "0") => Value::Bool(false),
                _ => number("office:value").map_or(Value::Empty, |n| Value::Bool(n != 0.0)),
            },
            Some("string") => match e.attr("office:string-value") {
                Some(s) => Value::text(s),
                None => Value::text(text.as_deref().unwrap_or_default()),
            },
            _ => text.as_deref().map_or(Value::Empty, Value::text),
        };
        let display = text
            .as_deref()
            .or(e.attr("office:string-value"))
            .unwrap_or_default();
        let explicit_error = e.attr("calcext:value-type") == Some("error");
        let error = ErrorKind::parse(display.trim());
        let error_ok =
            explicit_error || (formula.is_some() && matches!(vt, None | Some("string" | "float")));
        if let Some(err) = error.filter(|_| error_ok) {
            value = Value::Error(err);
        }
        // NaN and infinities (`office:value="NaN"`, `1e309`) never reach the
        // model: AOO cannot store them, and they break `#NUM!` invariants.
        if matches!(value, Value::Number(n) if !n.is_finite()) {
            value = Value::Empty;
        }
        let content = match formula {
            Some(text) => CellContentData::Formula {
                text,
                cached: (!value.is_empty()).then_some(value),
            },
            None => CellContentData::Value(value),
        };
        Ok(CellParsed {
            repeat,
            content,
            style: e
                .attr("table:style-name")
                .filter(|n| *n != "Default")
                .map(str::to_string),
            span,
        })
    }
}

/// Collects the text of one `text:p`, applying ODF white-space collapsing and
/// `text:s` / `text:tab` / `text:line-break`. Annotations, notes and drawing
/// objects are skipped; other inline elements (spans, links, fields) are
/// transparent.
fn parse_paragraph<R: BufRead>(
    xml: &mut XmlReader<R>,
    budget: &mut usize,
) -> Result<String, OdsError> {
    let mut out = String::new();
    let mut last_space = true;
    let d = xml.depth();
    let over = || OdsError::LimitExceeded {
        what: "cell text bytes",
        limit: MAX_CELL_TEXT as u64,
    };
    loop {
        match xml.next()? {
            Ev::Text(t) => {
                if t.len() > *budget {
                    return Err(over());
                }
                let before = out.len();
                for ch in t.chars() {
                    if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                        if !last_space {
                            out.push(' ');
                            last_space = true;
                        }
                    } else {
                        out.push(ch);
                        last_space = false;
                    }
                }
                *budget -= out.len() - before;
            }
            Ev::Start(e) => match e.name.as_str() {
                "text:s" => {
                    let n = e
                        .attr("text:c")
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .unwrap_or(1)
                        .max(1);
                    if n > *budget as u64 {
                        return Err(over());
                    }
                    *budget -= n as usize;
                    out.extend(std::iter::repeat_n(' ', n as usize));
                    last_space = false;
                    xml.skip()?;
                }
                "text:tab" | "text:line-break" => {
                    if *budget == 0 {
                        return Err(over());
                    }
                    *budget -= 1;
                    out.push(if e.name == "text:tab" { '\t' } else { '\n' });
                    last_space = false;
                    xml.skip()?;
                }
                "text:note"
                | "office:annotation"
                | "text:ruby-text"
                | "text:bookmark-start"
                | "text:bookmark-end" => xml.skip()?,
                n if n.starts_with("draw:") || n.starts_with("svg:") => xml.skip()?,
                _ => {}
            },
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn parse_calc_settings<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
) -> Result<CalcSettings, OdsError> {
    // ODF attribute defaults apply when the element is present.
    let regex = e.attr("table:use-regular-expressions") != Some("false");
    let wildcards = e.attr("table:use-wildcards") == Some("true");
    let mut c = CalcSettings {
        case_sensitive: e.attr("table:case-sensitive") != Some("false"),
        precision_as_shown: e.attr("table:precision-as-shown") == Some("true"),
        whole_cell_match: e.attr("table:search-criteria-must-apply-to-whole-cell") != Some("false"),
        criteria_mode: if wildcards {
            CriteriaMode::Wildcards
        } else if regex {
            CriteriaMode::Regex
        } else {
            CriteriaMode::Literal
        },
        ..CalcSettings::default()
    };
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(ch) => {
                match ch.name.as_str() {
                    "table:null-date" => {
                        c.date_epoch = ch
                            .attr("table:date-value")
                            .map_or(DateEpoch::D1899, epoch_from_null_date);
                    }
                    "table:iteration" => {
                        if ch.attr("table:status") == Some("enable") {
                            let def = IterSettings::default();
                            c.iterations = Some(IterSettings {
                                steps: ch
                                    .attr("table:steps")
                                    .and_then(|v| v.trim().parse().ok())
                                    .unwrap_or(def.steps),
                                min_change: ch
                                    .attr("table:minimum-difference")
                                    .and_then(|v| v.trim().parse().ok())
                                    .unwrap_or(def.min_change),
                            });
                        }
                    }
                    _ => {}
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => return Ok(c),
            Ev::Eof => return Ok(c),
            _ => {}
        }
    }
}
