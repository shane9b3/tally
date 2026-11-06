//! `ss:Worksheet` → [`SheetData`]: `ss:Table` (columns, rows, cells),
//! `ss:Names`, `x:WorksheetOptions`, `x:AutoFilter`.
//!
//! Positions follow templates `ss:Row` (`create-rows`) and `ss:Cell`
//! (`selected`) of `spreadsheetml2ooo.xsl`: a row or cell without
//! `ss:Index` follows the previous one; `ss:Index` (1-based) jumps ahead;
//! a cell with `ss:MergeAcross` occupies the merged columns, so the next
//! cell starts after them; `ss:Span` on a row or column repeats it
//! `Span + 1` times. An `ss:Index` that does not move forward is ignored
//! (the implied position is used), so positions strictly increase and the
//! cells come out sorted; an index past the sheet drops the rest of the
//! row or table with a warning.

use std::io::BufRead;
use std::sync::Arc;

use tally_model::{
    AutoFilterData, CellContentData, CellData, CellRangeLocal, DimRun, LinkData, NoteData,
    SheetData, SheetProtectionData, StyleRun,
};
use tally_types::{ErrorKind, Value, MAX_COLS, MAX_ROWS};

use crate::styles::parse_bool;
use crate::xml::{Elem, Ev, XmlReader};
use crate::{refs, Ctx, XmlssError};

/// Longest formula text accepted (the formula parser's limit, as ODS/XLSX).
pub(crate) const MAX_FORMULA_TEXT: usize = 8192;

/// Most text one cell holds (as ODS and XLSX).
pub(crate) const MAX_CELL_TEXT: usize = 16 * 1024 * 1024;

/// Most merged areas per sheet (as ODS and XLSX; later ones are dropped
/// with a warning).
pub(crate) const MAX_MERGES_PER_SHEET: usize = 100_000;

/// Most hyperlinks per sheet (as XLSX).
pub(crate) const MAX_LINKS_PER_SHEET: usize = 100_000;

/// Longest hyperlink target kept (as XLSX relationship targets).
pub(crate) const MAX_HREF_BYTES: usize = 8 * 1024;

/// Longest note text kept (as XLSX; longer notes are cut).
pub(crate) const MAX_NOTE_TEXT: usize = 32 * 1024;

/// Most notes per sheet.
pub(crate) const MAX_NOTES_PER_SHEET: usize = 100_000;

/// Largest column width or row height accepted, in points (Excel's own
/// maxima are 255 characters ≈ 1,790 pt and 409 pt).
const MAX_DIM_PT: f64 = 2000.0;

/// What [`crate::read_xmlss`] still has to do for a sheet once every sheet
/// name is known.
#[derive(Debug, Default)]
pub(crate) struct SheetExtra {
    /// Formula cells: (index into `cells`, R1C1 text).
    pub formulas: Vec<(usize, String)>,
    /// Cells whose value came from `ss:Type="DateTime"` (index into
    /// `cells`; the value is already a calendar-true serial).
    pub date_cells: Vec<usize>,
    /// `x:WorksheetOptions/x:Selected`.
    pub selected: bool,
    /// The per-sheet caps on merges, links and notes were reached (their
    /// warning is said once).
    merges_capped: bool,
    links_capped: bool,
    notes_capped: bool,
}

/// A cell as read, before it is placed (rows with `ss:Span` place it
/// several times).
struct RawCell {
    col: u32,
    value: Value,
    is_date: bool,
    formula: Option<String>,
    array: Option<(u32, u32)>,
    style: Option<u32>,
    merge: Option<(u32, u32)>,
    href: Option<Arc<str>>,
    note: Option<NoteData>,
}

/// `pt` → 1/100 mm, `None` for junk; clamped to [`MAX_DIM_PT`].
fn pt_attr(e: &Elem, key: &str) -> Option<u32> {
    let v: f64 = e.attr(key)?.trim().parse().ok()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    Some((v.min(MAX_DIM_PT) * 2540.0 / 72.0).round() as u32)
}

/// A 1-based `ss:Index`-like attribute → 0-based, `None` when absent or
/// junk, `Some(u64)` possibly past the sheet.
fn index_attr(e: &Elem, key: &str) -> Option<u64> {
    let v: u64 = e.attr(key)?.trim().parse().ok()?;
    (v >= 1).then(|| v - 1)
}

/// A count attribute (`ss:Span`, `ss:MergeAcross`), clamped to `max`.
fn count_attr(e: &Elem, key: &str, max: u32) -> u32 {
    e.attr(key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(0, |v| v.min(f64::from(max)) as u32)
}

/// Parses one `ss:Worksheet` (its start tag was just read) into
/// `ctx.sheets`.
pub(crate) fn worksheet<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    ctx: &mut Ctx,
) -> Result<(), XmlssError> {
    if ctx.sheets.len() >= ctx.limits.max_sheets {
        return Err(XmlssError::LimitExceeded {
            what: "sheets",
            limit: ctx.limits.max_sheets as u64,
        });
    }
    let index = ctx.sheets.len();
    let name = ctx.unique_sheet_name(e.attr("Name").unwrap_or(""), index);
    let mut sheet = SheetData {
        name,
        ..SheetData::default()
    };
    let mut extra = SheetExtra::default();
    let mut tables = 0;
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => match c.local() {
                // AOO's XSLT reads the first `ss:Table` of a worksheet
                // (Excel writes one); later ones would repeat columns and
                // put cells out of order.
                "Table" if tables > 0 => {
                    ctx.warn("Only the first table of a worksheet was read.");
                    xr.skip()?;
                }
                "Table" => {
                    tables += 1;
                    table(xr, &c, ctx, &mut sheet, &mut extra)?;
                }
                "Names" => crate::names(xr, ctx, Some(index))?,
                "WorksheetOptions" => options(xr, &mut sheet, &mut extra)?,
                "AutoFilter" => {
                    if let Some((_, range)) = c.attr("Range").and_then(|r| refs::range(r, (0, 0))) {
                        sheet.auto_filter = Some(AutoFilterData {
                            range,
                            ..AutoFilterData::default()
                        });
                    }
                    if has_child(xr, "AutoFilterColumn")? {
                        ctx.warn("AutoFilter criteria were dropped (the filter buttons are kept).");
                    }
                }
                "ConditionalFormatting" => {
                    ctx.warn("Conditional formatting was dropped.");
                    xr.skip()?;
                }
                "DataValidation" => {
                    ctx.warn("Validity (data validation) rules were dropped.");
                    xr.skip()?;
                }
                "PivotTable" => {
                    ctx.warn("Pivot tables were dropped (their cells are kept).");
                    xr.skip()?;
                }
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < base => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    ctx.sheets.push(sheet);
    ctx.extras.push(extra);
    Ok(())
}

/// Skips the current element; `true` if it has a descendant `local`.
fn has_child<R: BufRead>(xr: &mut XmlReader<R>, local: &str) -> Result<bool, XmlssError> {
    let base = xr.depth();
    let mut found = false;
    loop {
        match xr.next()? {
            Ev::Start(c) => found |= c.local() == local,
            Ev::End if xr.depth() < base => return Ok(found),
            Ev::Eof => return Ok(found),
            _ => {}
        }
    }
}

/// `x:WorksheetOptions`: `x:Visible` (`SheetHidden`/`SheetVeryHidden`),
/// `x:Selected`, `x:FreezePanes` with `x:SplitHorizontal` (rows) and
/// `x:SplitVertical` (columns), `x:ProtectContents` (template
/// `x:ExcelWorkbook`, the `config:` view settings). Unfrozen splits, zoom,
/// gridlines, page setup and print settings are dropped.
fn options<R: BufRead>(
    xr: &mut XmlReader<R>,
    sheet: &mut SheetData,
    extra: &mut SheetExtra,
) -> Result<(), XmlssError> {
    let mut freeze = false;
    let (mut rows, mut cols) = (0u32, 0u32);
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => {
                let num = |t: &str| t.trim().parse::<u32>().ok();
                match c.local() {
                    "Visible" => {
                        let t = xr.text_truncated(64)?;
                        sheet.hidden = matches!(t.trim(), "SheetHidden" | "SheetVeryHidden");
                    }
                    "Selected" => {
                        extra.selected = true;
                        xr.skip()?;
                    }
                    "FreezePanes" | "FrozenNoSplit" => {
                        freeze = true;
                        xr.skip()?;
                    }
                    "SplitHorizontal" => {
                        rows = num(&xr.text_truncated(64)?).unwrap_or(0).min(MAX_ROWS - 1);
                    }
                    "SplitVertical" => {
                        cols = num(&xr.text_truncated(64)?).unwrap_or(0).min(MAX_COLS - 1);
                    }
                    "ProtectContents" => {
                        let t = xr.text_truncated(64)?;
                        if parse_bool(&t) {
                            sheet.protection = Some(SheetProtectionData::default());
                        }
                    }
                    _ => xr.skip()?,
                }
            }
            Ev::End if xr.depth() < base => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    if freeze && (rows > 0 || cols > 0) {
        sheet.frozen = Some((rows, cols));
    }
    Ok(())
}

/// Per-table parse state.
struct Table {
    next_row: u32,
    next_col_def: u32,
    /// Explicit column runs: (first, last, width, hidden).
    cols: Vec<(u32, u32, Option<u32>, bool)>,
    /// Column style runs (first, last, style id) from `ss:Column
    /// ss:StyleID`, sorted and disjoint (`ss:Index` only moves forward).
    col_style: Vec<(u32, u32, u32)>,
    table_style: Option<u32>,
    default_col_hmm: Option<u32>,
    /// Rows were cut at the sheet's end.
    done: bool,
}

/// `ss:Table` (templates `ss:Table`, `create-columns`, `create-rows`).
fn table<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    ctx: &mut Ctx,
    sheet: &mut SheetData,
    extra: &mut SheetExtra,
) -> Result<(), XmlssError> {
    let mut t = Table {
        next_row: 0,
        next_col_def: 0,
        cols: Vec::new(),
        col_style: Vec::new(),
        table_style: match e.attr("StyleID") {
            Some(id) => ctx.style_id(id)?,
            None => None,
        },
        default_col_hmm: pt_attr(e, "DefaultColumnWidth").filter(|w| *w > 0),
        done: false,
    };
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => match c.local() {
                "Column" => {
                    column(&c, ctx, sheet, &mut t)?;
                    xr.skip()?;
                }
                "Row" if t.done => {
                    ctx.warn("Rows past the last row of the sheet were dropped.");
                    xr.skip()?;
                }
                "Row" => row(xr, &c, ctx, sheet, extra, &mut t)?,
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < base => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    // Column widths: explicit runs, and `ss:DefaultColumnWidth` for the
    // columns between and after them.
    let default = t.default_col_hmm;
    let mut next = 0u32;
    for (a, b, w, hidden) in std::mem::take(&mut t.cols) {
        if let (Some(d), true) = (default, a > next) {
            push_run(&mut sheet.col_widths, next, a - 1, d, false);
        }
        if let Some(size) = w.or(default).or(hidden.then_some(DEFAULT_COL_HMM)) {
            push_run(&mut sheet.col_widths, a, b, size, hidden);
        }
        next = b.saturating_add(1);
    }
    if let (Some(d), true) = (default, next < MAX_COLS) {
        push_run(&mut sheet.col_widths, next, MAX_COLS - 1, d, false);
    }
    Ok(())
}

/// Excel's default column width (48 pt) for hidden columns without one.
const DEFAULT_COL_HMM: u32 = 1693;

/// Appends a dimension run, merging it with the previous one when equal
/// and adjacent.
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

/// `ss:Column`: `ss:Index`, `ss:Span`, `ss:Width` (points), `ss:Hidden`,
/// `ss:StyleID` (a whole-column style run).
fn column(e: &Elem, ctx: &mut Ctx, sheet: &mut SheetData, t: &mut Table) -> Result<(), XmlssError> {
    let start = match index_attr(e, "Index") {
        Some(i) if i >= u64::from(t.next_col_def) => i,
        _ => u64::from(t.next_col_def),
    };
    if start >= u64::from(MAX_COLS) {
        ctx.warn("Columns past the last column of the sheet were dropped.");
        return Ok(());
    }
    let start = start as u32;
    let end = start
        .saturating_add(count_attr(e, "Span", MAX_COLS))
        .min(MAX_COLS - 1);
    t.next_col_def = end.saturating_add(1);
    let hidden = e.attr("Hidden").is_some_and(parse_bool);
    let width = pt_attr(e, "Width");
    t.cols.push((start, end, width, hidden));
    if let Some(id) = e.attr("StyleID") {
        if let Some(s) = ctx.style_id(id)? {
            // The model keeps one run per column: each counts against the
            // cell budget (as XLSX's column styles do) before any is made.
            // One table per sheet and forward-only columns bound a sheet
            // to 16,384 runs.
            ctx.add_cells(u64::from(end - start) + 1)?;
            t.col_style.push((start, end, s));
            sheet.style_runs.extend((start..=end).map(|c| StyleRun {
                col: c,
                start_row: 0,
                end_row: MAX_ROWS - 1,
                style: s,
            }));
        }
    }
    Ok(())
}

impl Table {
    /// The `ss:Column` style of column `c`.
    fn column_style(&self, c: u32) -> Option<u32> {
        let i = self.col_style.partition_point(|&(_, end, _)| end < c);
        self.col_style
            .get(i)
            .filter(|&&(start, _, _)| start <= c)
            .map(|&(_, _, s)| s)
    }
}

/// `ss:Row` and its cells.
fn row<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    ctx: &mut Ctx,
    sheet: &mut SheetData,
    extra: &mut SheetExtra,
    t: &mut Table,
) -> Result<(), XmlssError> {
    let r0 = match index_attr(e, "Index") {
        Some(i) if i >= u64::from(t.next_row) => i,
        _ => u64::from(t.next_row),
    };
    if r0 >= u64::from(MAX_ROWS) {
        ctx.warn("Rows past the last row of the sheet were dropped.");
        t.done = true;
        return xr.skip();
    }
    let r0 = r0 as u32;
    let r1 = r0
        .saturating_add(count_attr(e, "Span", MAX_ROWS))
        .min(MAX_ROWS - 1);
    t.next_row = r1.saturating_add(1);
    if r1 == MAX_ROWS - 1 {
        t.done = true;
    }
    // Template `get-row-style-name`: a height is fixed unless
    // `ss:AutoFitHeight="1"`.
    let hidden = e.attr("Hidden").is_some_and(parse_bool);
    let auto = e.attr("AutoFitHeight").is_some_and(parse_bool);
    let height = pt_attr(e, "Height").filter(|_| !auto || hidden);
    if hidden || height.is_some() {
        let size = height.unwrap_or(crate::DEFAULT_ROW_HMM);
        push_run(&mut sheet.row_heights, r0, r1, size, hidden);
    }
    let row_style = match e.attr("StyleID") {
        Some(id) => ctx.style_id(id)?,
        None => None,
    };
    // Cells.
    let mut raw: Vec<RawCell> = Vec::new();
    let mut next_col: u32 = 0;
    let mut cut = false;
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) if c.local() == "Cell" => {
                let col = match index_attr(&c, "Index") {
                    Some(i) if i >= u64::from(next_col) => i,
                    _ => u64::from(next_col),
                };
                if cut || col >= u64::from(MAX_COLS) {
                    cut = true;
                    xr.skip()?;
                    continue;
                }
                let col = col as u32;
                let across = count_attr(&c, "MergeAcross", MAX_COLS).min(MAX_COLS - 1 - col);
                let down = count_attr(&c, "MergeDown", MAX_ROWS).min(MAX_ROWS - 1 - r0);
                next_col = col.saturating_add(across).saturating_add(1);
                if next_col >= MAX_COLS {
                    cut = true;
                }
                let style = match c.attr("StyleID") {
                    Some(id) => ctx.style_id(id)?,
                    None => None,
                }
                .or(row_style)
                .or_else(|| t.column_style(col))
                .or(t.table_style);
                let mut rc = RawCell {
                    col,
                    value: Value::Empty,
                    is_date: false,
                    formula: None,
                    array: None,
                    style,
                    merge: (across > 0 || down > 0).then_some((across, down)),
                    href: None,
                    note: None,
                };
                cell(xr, &c, ctx, &mut rc, r0)?;
                raw.push(rc);
            }
            Ev::Start(_) => xr.skip()?,
            Ev::End if xr.depth() < base => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    if cut {
        ctx.warn("Cells past the last column of the sheet were dropped.");
    }
    // Cells with nothing to place (`<Cell/>`) cost nothing on any row.
    raw.retain(|rc| {
        rc.formula.is_some()
            || !rc.value.is_empty()
            || rc.style.is_some()
            || rc.merge.is_some()
            || rc.href.is_some()
            || rc.note.is_some()
    });
    if raw.is_empty() {
        return Ok(());
    }
    // Place the cells on every row of the span, after checking the whole
    // span against the budgets (so a spanned row cannot materialize
    // millions of cells, links or notes before the limit trips). Every
    // placed item counts against the cell budget, so rows × cells of work
    // is bounded by it too.
    let rows = u64::from(r1 - r0) + 1;
    let (mut per_row, mut per_row_text) = (0u64, 0u64);
    for rc in &raw {
        if rc.formula.is_some() || !rc.value.is_empty() || rc.style.is_some() {
            per_row += 1;
        }
        per_row += u64::from(rc.merge.is_some());
        per_row += u64::from(rc.href.is_some());
        per_row += u64::from(rc.note.is_some());
        per_row_text += rc.href.as_ref().map_or(0, |h| h.len() as u64);
        per_row_text += rc.note.as_ref().map_or(0, |n| n.text.len() as u64);
        if let Some((dr, dc)) = rc.array {
            per_row += (u64::from(dr) + 1) * (u64::from(dc) + 1);
        }
        if let Value::Text(t) = &rc.value {
            per_row_text += t.len() as u64;
        }
        per_row_text += rc.formula.as_ref().map_or(0, |f| f.len() as u64);
    }
    if ctx.cells.saturating_add(per_row.saturating_mul(rows)) > ctx.limits.max_cells {
        return Err(XmlssError::LimitExceeded {
            what: "materialized cells",
            limit: ctx.limits.max_cells,
        });
    }
    if ctx
        .text_bytes
        .saturating_add(per_row_text.saturating_mul(rows))
        > ctx.limits.max_text_bytes
    {
        return Err(XmlssError::LimitExceeded {
            what: "materialized cell text bytes",
            limit: ctx.limits.max_text_bytes,
        });
    }
    for r in r0..=r1 {
        for rc in &raw {
            place(ctx, sheet, extra, rc, r)?;
        }
    }
    Ok(())
}

/// The contents of one `ss:Cell` (its start tag `e` was just read).
fn cell<R: BufRead>(
    xr: &mut XmlReader<R>,
    e: &Elem,
    ctx: &mut Ctx,
    rc: &mut RawCell,
    row: u32,
) -> Result<(), XmlssError> {
    if let Some(f) = e.attr("Formula") {
        if f.len() > MAX_FORMULA_TEXT {
            return Err(XmlssError::LimitExceeded {
                what: "formula length",
                limit: MAX_FORMULA_TEXT as u64,
            });
        }
        if !f.trim().is_empty() {
            rc.formula = Some(f.to_string());
            if let Some((_, a)) = e
                .attr("ArrayRange")
                .and_then(|a| refs::range(a, (row, rc.col)))
            {
                rc.array = Some((a.r1.saturating_sub(row), a.c1.saturating_sub(rc.col)));
            }
        }
    }
    if let Some(h) = e.attr("HRef").map(str::trim).filter(|h| !h.is_empty()) {
        if h.len() > MAX_HREF_BYTES {
            ctx.warn(format!(
                "Hyperlink targets longer than {} KiB were dropped.",
                MAX_HREF_BYTES / 1024
            ));
        } else {
            rc.href = Some(Arc::from(h));
        }
    }
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) => match c.local() {
                "Data" => {
                    let ty = c.attr("Type").map(str::to_string);
                    let text = xr.text_content(MAX_CELL_TEXT)?;
                    let (v, is_date) = value(ty.as_deref(), text);
                    rc.value = v;
                    rc.is_date = is_date;
                }
                "Comment" => rc.note = Some(comment(xr, &c)?),
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < base => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// `ss:Data` by `ss:Type` (template `ss:Data`): `Number`, `String`,
/// `Boolean`, `DateTime` (ISO 8601 → serial; `true` in the result) and
/// `Error`. A value that does not parse is kept as text.
fn value(ty: Option<&str>, text: String) -> (Value, bool) {
    match ty.map(str::trim) {
        Some("Number") => match text.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => (Value::Number(n), false),
            Ok(_) => (Value::Empty, false),
            Err(_) if text.trim().is_empty() => (Value::Empty, false),
            Err(_) => (Value::text(text), false),
        },
        Some("Boolean") => match text.trim() {
            "1" | "true" | "TRUE" | "True" => (Value::Bool(true), false),
            "0" | "false" | "FALSE" | "False" | "" => (Value::Bool(false), false),
            _ => (Value::text(text), false),
        },
        Some("DateTime") => match iso_to_serial(&text) {
            Some(s) => (Value::Number(s), true),
            None if text.trim().is_empty() => (Value::Empty, false),
            None => (Value::text(text), false),
        },
        Some("Error") => {
            let t = text.trim();
            (
                Value::Error(ErrorKind::parse(t).unwrap_or(ErrorKind::Value)),
                false,
            )
        }
        _ if text.is_empty() => (Value::Empty, false),
        _ => (Value::text(text), false),
    }
}

/// `YYYY-MM-DD[THH:MM[:SS[.fff]]]` → a serial in Tally's default epoch
/// (D1899, 1899-12-30 = 0). The calendar date is taken as written (Excel
/// writes time-only values on 1899-12-31, as AOO imports them).
pub(crate) fn iso_to_serial(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut it = date.splitn(3, '-');
    let y: i32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if !(-99_999..=99_999).contains(&y) {
        return None;
    }
    let mut serial = tally_numfmt::date::serial_from_ymd(tally_types::DateEpoch::D1899, y, m, d)?;
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let mut parts = t.splitn(3, ':');
        let h: f64 = parts.next()?.parse().ok()?;
        let mi: f64 = parts.next().unwrap_or("0").parse().ok()?;
        let sec: f64 = parts.next().unwrap_or("0").parse().ok()?;
        if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&mi) || !(0.0..61.0).contains(&sec) {
            return None;
        }
        serial += (h * 3600.0 + mi * 60.0 + sec) / 86_400.0;
    }
    serial.is_finite().then_some(serial)
}

/// `ss:Comment` (template `ss:Comment` mode `body`): `ss:Author`,
/// `ss:ShowAlways`, the text of its `ss:Data`.
fn comment<R: BufRead>(xr: &mut XmlReader<R>, e: &Elem) -> Result<NoteData, XmlssError> {
    let author = e
        .attr("Author")
        .map(|a| a.chars().take(255).collect::<String>())
        .filter(|a| !a.is_empty());
    let shown = e.attr("ShowAlways").is_some_and(parse_bool);
    let mut text = String::new();
    let base = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(c) if c.local() == "Data" => {
                let t = xr.text_truncated(MAX_NOTE_TEXT.saturating_sub(text.len()))?;
                text.push_str(&t);
            }
            Ev::Start(_) => xr.skip()?,
            Ev::End if xr.depth() < base => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok(NoteData {
        row: 0,
        col: 0,
        text,
        author,
        created: None,
        shown,
    })
}

/// Materializes `rc` on row `r`.
fn place(
    ctx: &mut Ctx,
    sheet: &mut SheetData,
    extra: &mut SheetExtra,
    rc: &RawCell,
    r: u32,
) -> Result<(), XmlssError> {
    let c = rc.col;
    if let Some((across, down)) = rc.merge {
        ctx.add_cells(1)?;
        if sheet.merges.len() < MAX_MERGES_PER_SHEET {
            sheet.merges.push(CellRangeLocal {
                r0: r,
                c0: c,
                r1: r.saturating_add(down).min(MAX_ROWS - 1),
                c1: c + across,
            });
        } else if !extra.merges_capped {
            extra.merges_capped = true;
            ctx.warn(format!(
                "Only the first {MAX_MERGES_PER_SHEET} merged areas of a sheet were kept."
            ));
        }
    }
    if let Some(url) = &rc.href {
        ctx.add_cells(1)?;
        if sheet.links.len() < MAX_LINKS_PER_SHEET {
            ctx.add_text(url.len())?;
            sheet.links.push(LinkData {
                row: r,
                col: c,
                url: url.clone(),
            });
        } else if !extra.links_capped {
            extra.links_capped = true;
            ctx.warn(format!(
                "Only the first {MAX_LINKS_PER_SHEET} hyperlinks of a sheet are kept."
            ));
        }
    }
    if let Some(n) = &rc.note {
        ctx.add_cells(1)?;
        if sheet.notes.len() < MAX_NOTES_PER_SHEET {
            ctx.add_text(n.text.len())?;
            sheet.notes.push(NoteData {
                row: r,
                col: c,
                ..n.clone()
            });
        } else if !extra.notes_capped {
            extra.notes_capped = true;
            ctx.warn(format!(
                "Only the first {MAX_NOTES_PER_SHEET} notes of a sheet were kept."
            ));
        }
    }
    let text_len = match &rc.value {
        Value::Text(t) => t.len(),
        _ => 0,
    } + rc.formula.as_ref().map_or(0, String::len);
    if rc.formula.is_none() && rc.value.is_empty() && rc.style.is_none() {
        return Ok(());
    }
    ctx.add_cells(1)?;
    ctx.add_text(text_len)?;
    let idx = sheet.cells.len();
    let content = match &rc.formula {
        Some(f) => {
            extra.formulas.push((idx, f.clone()));
            if let Some((dr, dc)) = rc.array {
                let area = CellRangeLocal {
                    r0: r,
                    c0: c,
                    r1: r.saturating_add(dr).min(MAX_ROWS - 1),
                    c1: c.saturating_add(dc).min(MAX_COLS - 1),
                };
                let n = (u64::from(area.r1 - area.r0) + 1) * (u64::from(area.c1 - area.c0) + 1);
                ctx.add_cells(n)?;
                sheet.array_formulas.push(area);
            }
            CellContentData::Formula {
                text: Arc::from(""),
                cached: (!rc.value.is_empty()).then(|| rc.value.clone()),
            }
        }
        None => CellContentData::Value(rc.value.clone()),
    };
    if rc.is_date {
        extra.date_cells.push(idx);
    }
    sheet.cells.push(CellData {
        row: r,
        col: c,
        content,
        style: rc.style,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso_to_serial("1899-12-30T00:00:00.000"), Some(0.0));
        assert_eq!(iso_to_serial("1899-12-31T12:00:00.000"), Some(1.5));
        assert_eq!(iso_to_serial("2026-10-19T00:00:00.000"), Some(46314.0));
        assert_eq!(iso_to_serial("2026-10-19"), Some(46314.0));
        assert_eq!(iso_to_serial("2026-13-01"), None);
        assert_eq!(iso_to_serial("9999999999-01-01"), None);
        assert_eq!(iso_to_serial("2026-01-01T25:00"), None);
        for junk in ["", "T", "--", "2026-1-1T:", "é-é-é"] {
            let _ = iso_to_serial(junk);
        }
    }

    #[test]
    fn values() {
        assert_eq!(value(Some("Number"), "1.5".into()).0, Value::Number(1.5));
        assert_eq!(value(Some("Number"), "1e999".into()).0, Value::Empty);
        assert_eq!(value(Some("Boolean"), "1".into()).0, Value::Bool(true));
        assert_eq!(
            value(Some("Error"), "#N/A".into()).0,
            Value::Error(ErrorKind::NA)
        );
        assert_eq!(
            value(Some("Error"), "#WHAT".into()).0,
            Value::Error(ErrorKind::Value)
        );
        assert!(value(Some("DateTime"), "2020-01-01T00:00:00.000".into()).1);
    }
}
