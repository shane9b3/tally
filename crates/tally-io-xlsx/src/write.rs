//! `write_xlsx`: [`WorkbookData`] → XLSX via `rust_xlsxwriter`.
//!
//! Formulas are translated OpenFormula → ExcelA1 (with OOXML function
//! spellings) and written with their cached results. Date-formatted serials
//! are mapped to Excel's 1900 date system (see [`crate::dates`]); D1904
//! workbooks keep their serials and get `date1904="1"`, which
//! `rust_xlsxwriter` cannot write itself, so the finished package is
//! re-zipped with `xl/workbook.xml` patched.

use std::collections::HashSet;
use std::io::{Seek, Write};

use rust_xlsxwriter::{
    Color, DocProperties, Format, FormatAlign, FormatBorder, FormatPattern, FormatUnderline,
    Formula, Workbook, Worksheet,
};
use tally_model::{
    BorderLine, BorderStyle, CellContentData, CellData, HAlign, SheetData, StyleData, VAlign,
    WorkbookData, APP_DEFAULT_COL_WIDTH_HMM, APP_DEFAULT_ROW_HEIGHT_HMM,
};
use tally_types::{CellAddr, DateEpoch, ErrorKind, Value, MAX_COLS, MAX_ROWS};

use crate::colors::model_to_rgb;
use crate::dates::tally_serial_to_excel;
use crate::formula::{of_to_excel, XlsxResolver};
use crate::numfmt::{is_date_format, ooo_to_excel};
use crate::units::hmm_to_px;
use crate::{WriteReport, XlsxError};

/// Excel's cell text limit (characters).
const MAX_EXCEL_TEXT: usize = 32_767;
/// Styled empty runs longer than this are written as a column format.
const MAX_STYLED_EMPTY_RUN: u32 = 4096;
/// Most individually sized rows written per sheet.
const MAX_SIZED_ROWS: u32 = 100_000;
/// Most warnings kept (the rest are counted).
const MAX_WARNINGS: usize = 200;

/// What a row or column is, as far as its size goes: `None` has no run
/// (the application's default size; a row is fitted to its content),
/// `Some((size, hidden))` has one.
type Sized = Option<(u32, bool)>;

/// The runs as segments `(first, last, what)` that cover `0..count` once:
/// ascending, the rows or columns without a run included. Runs that
/// overlap (no well-formed sheet has them) are cut where the previous one
/// ends.
fn segments(runs: &[tally_model::DimRun], count: u32) -> Vec<(u32, u32, Sized)> {
    let mut sorted: Vec<&tally_model::DimRun> = runs
        .iter()
        .filter(|r| r.start <= r.end && r.start < count)
        .collect();
    sorted.sort_by_key(|r| r.start);
    let mut out = Vec::with_capacity(2 * sorted.len() + 1);
    let mut next = 0u32;
    for r in sorted {
        let (start, end) = (r.start.max(next), r.end.min(count - 1));
        if start > end {
            continue;
        }
        if start > next {
            out.push((next, start - 1, None));
        }
        out.push((start, end, Some((r.size_hmm, r.hidden))));
        next = end + 1;
    }
    if next < count {
        out.push((next, count - 1, None));
    }
    out
}

/// What most rows (or columns) of the sheet are: the default the sheet
/// is written with, so that as few as possible are written one by one. A
/// sheet read from an Excel file keeps the file's default this way, also
/// after rows were inserted or deleted (which leaves a few without a
/// run). On a tie the application's default wins, then the smaller size.
/// A shown size that rounds to 0 px cannot be a default, nor can hidden
/// columns (`hidden`: rows only, `zeroHeight`).
fn dominant(segments: &[(u32, u32, Sized)], hidden: bool) -> Sized {
    let mut counts: std::collections::HashMap<Sized, u64> = std::collections::HashMap::new();
    for &(start, end, what) in segments {
        match what {
            Some((_, true)) if !hidden => continue,
            Some((size, false)) if hmm_to_px(size) == 0 => continue,
            _ => {}
        }
        *counts.entry(what).or_default() += u64::from(end - start) + 1;
    }
    let mut totals: Vec<(Sized, u64)> = counts.into_iter().collect();
    totals.sort_by_key(|&(what, n)| (std::cmp::Reverse(n), what));
    totals.first().and_then(|t| t.0)
}

/// The default sizes a sheet is written with (`sheetFormatPr`): what the
/// rows and columns that are not written one by one have, see
/// [`dominant`].
///
/// When most rows have no run, the default is the application's (24 px,
/// 100 px): the file says what Tally shows, and the reader takes such a
/// default back as "no run". When most rows have one height, that height
/// is the default; the rows without a run are then written as rows of
/// 24 px (the file has no other way to say "automatic" under a default
/// that is a set height) and read back with that height set. When most
/// rows are hidden, the default is hidden (`zeroHeight`).
struct SheetDefaults {
    /// What the rows that are not written have.
    rows: Sized,
    /// What the columns that are not written have (never hidden).
    cols: Sized,
    /// `rust_xlsxwriter`'s own default column width, pixels: what it
    /// writes and lays objects out with for a column it was told
    /// nothing about.
    writer_col_px: u32,
}

impl SheetDefaults {
    fn new(data: &WorkbookData, sheet: &SheetData) -> Self {
        SheetDefaults {
            rows: dominant(&segments(&sheet.row_heights, MAX_ROWS), true),
            cols: dominant(&segments(&sheet.col_widths, MAX_COLS), false),
            writer_col_px: match default_font(data) {
                Some(_) => default_col_width_px(data),
                None => 64,
            },
        }
    }

    /// The default row height, pixels (of hidden rows: at least 1).
    fn row_px(&self) -> u32 {
        match self.rows {
            None => hmm_to_px(APP_DEFAULT_ROW_HEIGHT_HMM),
            Some((size, _)) => hmm_to_px(size).max(1),
        }
    }

    /// The default column width, pixels.
    fn col_px(&self) -> u32 {
        hmm_to_px(self.cols.map_or(APP_DEFAULT_COL_WIDTH_HMM, |c| c.0))
    }

    /// A width in pixels as `<col width>` counts it: characters of the
    /// default font, so that it reads back as `px` exactly.
    fn col_chars(data: &WorkbookData, px: u32) -> f64 {
        let mdw = crate::units::max_digit_width(default_font(data), Some(11.0));
        (f64::from(px) * 256.0 / mdw).trunc() / 256.0
    }

    /// `sheetFormatPr/@defaultColWidth` when the default is not the one
    /// `rust_xlsxwriter` implies.
    fn default_col_width_attr(&self, data: &WorkbookData) -> Option<String> {
        (self.col_px() != self.writer_col_px).then(|| {
            format!(
                r#"defaultColWidth="{}""#,
                Self::col_chars(data, self.col_px())
            )
        })
    }

    /// The attributes `rust_xlsxwriter` gives a `<row>` that only has the
    /// default height, once the default is not its own (20 px). They are
    /// taken out of a sheet whose default rows are automatic: a `<row>`
    /// with a height is a row Excel no longer fits to its content.
    fn default_row_attrs(&self) -> Option<String> {
        let px = self.row_px();
        (self.rows.is_none() && px != 20)
            .then(|| format!(r#" ht="{}" customHeight="1""#, f64::from(px) * 0.75))
    }

    /// The real width of the hidden columns that are as wide as
    /// `rust_xlsxwriter`'s default: it writes those with `width="0"`
    /// (as Excel does for a column hidden by dragging), which would read
    /// back as the sheet's default width.
    fn hidden_col_width(&self, data: &WorkbookData, sheet: &SheetData) -> Option<f64> {
        sheet
            .col_widths
            .iter()
            .any(|r| r.hidden && hmm_to_px(r.size_hmm) == self.writer_col_px)
            .then(|| Self::col_chars(data, self.writer_col_px))
    }
}

/// The columns that need a width although they have the sheet's default:
/// `rust_xlsxwriter` writes a `<col>` for a column with a format or an
/// outline level, and places charts, pictures and notes by the widths it
/// knows, in both cases with its own default width for a column it was
/// not given one for. Ranges, ascending.
fn default_width_columns(sheet: &SheetData) -> Vec<(u16, u16)> {
    if !sheet.charts.is_empty() || !sheet.images.is_empty() || !sheet.notes.is_empty() {
        return vec![(0, col_num(MAX_COLS - 1))];
    }
    let mut need = vec![false; MAX_COLS as usize];
    for run in &sheet.style_runs {
        // The runs written as a column format (`write_sheet`).
        if run.col < MAX_COLS
            && run.start_row <= run.end_row
            && run.end_row - run.start_row >= MAX_STYLED_EMPTY_RUN
        {
            need[run.col as usize] = true;
        }
    }
    for g in &sheet.col_groups {
        if g.start <= g.end {
            for c in g.start..=g.end.min(MAX_COLS - 1) {
                need[c as usize] = true;
            }
        }
    }
    let mut out: Vec<(u16, u16)> = Vec::new();
    for (c, _) in need.iter().enumerate().filter(|(_, n)| **n) {
        let c = c as u16;
        match out.last_mut() {
            Some(last) if last.1 + 1 == c => last.1 = c,
            _ => out.push((c, c)),
        }
    }
    out
}

/// The `sheetFormatPr` and `<row>` edits of [`SheetDefaults`] that
/// `rust_xlsxwriter` has no setter for.
fn default_size_patches(data: &WorkbookData, patches: &mut crate::write_v03::Patches) {
    // A workbook without a sheet is written with one empty sheet.
    let empty = [SheetData::default()];
    let sheets: &[SheetData] = if data.sheets.is_empty() {
        &empty
    } else {
        &data.sheets
    };
    for (i, sheet) in sheets.iter().enumerate() {
        let defaults = SheetDefaults::new(data, sheet);
        let part = format!("xl/worksheets/sheet{}.xml", i + 1);
        if let Some(attr) = defaults.default_col_width_attr(data) {
            patches.sheet_format_attrs.push((part.clone(), attr));
        }
        if let Some(attrs) = defaults.default_row_attrs() {
            patches.row_attr_strips.push((part.clone(), attrs));
        }
        if let Some(chars) = defaults.hidden_col_width(data, sheet) {
            patches.hidden_col_widths.push((part, chars));
        }
    }
}

struct Report {
    warnings: Vec<String>,
    dropped: usize,
}

impl crate::write_v03::Warn for Report {
    fn warn(&mut self, w: String) {
        Report::warn(self, w);
    }
}

impl Report {
    /// [`Report::warn`] unless the same warning was already given.
    fn warn_once(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warn(w);
        }
    }

    fn warn(&mut self, w: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(w.into());
        } else {
            self.dropped += 1;
        }
    }

    fn finish(mut self) -> WriteReport {
        if self.dropped > 0 {
            self.warnings
                .push(format!("… and {} more warnings", self.dropped));
        }
        WriteReport {
            warnings: self.warnings,
        }
    }
}

/// A valid, unique Excel sheet name for `name` (≤ 31 chars, none of
/// `[]:*?/\`, no leading/trailing `'`, not "History").
fn excel_sheet_name(name: &str, index: usize, taken: &mut HashSet<String>) -> String {
    let mut base: String = name
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    base = base.trim_matches('\'').to_string();
    if base.trim().is_empty() || base.eq_ignore_ascii_case("History") {
        base = format!("Sheet{}", index + 1);
    }
    let truncate = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let mut candidate = truncate(&base, 31);
    let mut k = 2;
    while taken.contains(&candidate.to_lowercase()) {
        let suffix = format!(" ({k})");
        candidate = format!("{}{suffix}", truncate(&base, 31 - suffix.chars().count()));
        k += 1;
    }
    taken.insert(candidate.to_lowercase());
    candidate
}

fn border(style: &BorderLine) -> FormatBorder {
    match style.style {
        BorderStyle::Solid if style.width_hmm <= 35 => FormatBorder::Thin,
        BorderStyle::Solid if style.width_hmm <= 66 => FormatBorder::Medium,
        BorderStyle::Solid => FormatBorder::Thick,
        BorderStyle::Dashed if style.width_hmm <= 35 => FormatBorder::Dashed,
        BorderStyle::Dashed => FormatBorder::MediumDashed,
        BorderStyle::Dotted if style.width_hmm < 18 => FormatBorder::Hair,
        BorderStyle::Dotted => FormatBorder::Dotted,
        BorderStyle::Double => FormatBorder::Double,
    }
}

fn rgb(c: u32) -> Color {
    Color::RGB(model_to_rgb(c))
}

/// The document's default font when it is not `rust_xlsxwriter`'s own
/// (Calibri).
fn default_font(data: &WorkbookData) -> Option<&str> {
    data.settings
        .default_font
        .as_deref()
        .filter(|f| !f.eq_ignore_ascii_case("Calibri"))
}

/// A font name as Excel accepts it (at most 31 characters).
fn excel_font_name(name: &str) -> String {
    name.chars().take(31).collect()
}

/// The format of an unstyled cell: the default font.
fn base_format(data: &WorkbookData) -> Format {
    match default_font(data) {
        Some(f) => Format::new().set_font_name(excel_font_name(f)),
        None => Format::new(),
    }
}

/// `set_default_format`'s default column width (px) for the default font:
/// 8.43 digits of its maximum digit width plus padding, one of the widths
/// `rust_xlsxwriter` supports, so `<col width>` counts in that font's
/// digits as Excel will (Calibri 11: 64 px, Arial 11: 72 px).
fn default_col_width_px(data: &WorkbookData) -> u32 {
    match crate::units::max_digit_width(default_font(data), Some(11.0)) as u32 {
        0..=6 => 56,
        7 => 64,
        8 => 72,
        9 | 10 => 80,
        11 => 96,
        12 => 104,
        _ => 120,
    }
}

/// The `rust_xlsxwriter` format of a model style (`base`: the format of
/// the default look).
fn format_of(s: &StyleData, base: &Format, report: &mut Report) -> Format {
    let mut f = base.clone();
    if s.font.bold {
        f = f.set_bold();
    }
    if s.font.italic {
        f = f.set_italic();
    }
    if s.font.underline {
        f = f.set_underline(match s.font.underline_style {
            tally_model::UnderlineStyleData::Double => FormatUnderline::Double,
            tally_model::UnderlineStyleData::SingleAccounting => FormatUnderline::SingleAccounting,
            tally_model::UnderlineStyleData::DoubleAccounting => FormatUnderline::DoubleAccounting,
            _ => FormatUnderline::Single,
        });
    }
    if s.font.strike {
        f = f.set_font_strikethrough();
    }
    if let Some(family) = &s.font.family {
        f = f.set_font_name(excel_font_name(family));
    }
    if let Some(size) = s.font.size_pt {
        f = f.set_font_size(f64::from(size));
    }
    if let Some(c) = s.font.color {
        f = f.set_font_color(rgb(c));
    }
    // A chip renders as its background colour in other applications.
    let fill = s.fill.or(s
        .display
        .as_ref()
        .map(|tally_model::DisplayData::Chip { bg, .. }| *bg));
    if let Some(c) = fill {
        f = f
            .set_pattern(FormatPattern::Solid)
            .set_background_color(rgb(c));
    }
    f = match s.align.h {
        HAlign::General => f,
        HAlign::Left => f.set_align(FormatAlign::Left),
        HAlign::Center => f.set_align(FormatAlign::Center),
        HAlign::Right => f.set_align(FormatAlign::Right),
        HAlign::Justify => f.set_align(FormatAlign::Justify),
    };
    f = match s.align.v {
        VAlign::Bottom => f,
        VAlign::Middle => f.set_align(FormatAlign::VerticalCenter),
        VAlign::Top => f.set_align(FormatAlign::Top),
    };
    if s.align.wrap {
        f = f.set_text_wrap();
    }
    if s.align.indent > 0 {
        f = f.set_indent(s.align.indent);
    }
    if s.align.stacked {
        f = f.set_rotation(270);
    } else if s.align.rotation_deg != 0 {
        f = f.set_rotation(s.align.rotation_deg.clamp(-90, 90));
    }
    if s.align.shrink_to_fit {
        f = f.set_shrink();
    }
    match (&s.borders.diagonal_down, &s.borders.diagonal_up) {
        (None, None) => {}
        (down, up) => {
            let l = down.as_ref().or(up.as_ref()).expect("one diagonal");
            f = f
                .set_border_diagonal(border(l))
                .set_border_diagonal_color(rgb(l.color))
                .set_border_diagonal_type(match (down.is_some(), up.is_some()) {
                    (true, true) => rust_xlsxwriter::FormatDiagonalBorder::BorderUpDown,
                    (true, false) => rust_xlsxwriter::FormatDiagonalBorder::BorderDown,
                    _ => rust_xlsxwriter::FormatDiagonalBorder::BorderUp,
                });
        }
    }
    let b = &s.borders;
    if let Some(l) = &b.top {
        f = f
            .set_border_top(border(l))
            .set_border_top_color(rgb(l.color));
    }
    if let Some(l) = &b.bottom {
        f = f
            .set_border_bottom(border(l))
            .set_border_bottom_color(rgb(l.color));
    }
    if let Some(l) = &b.left {
        f = f
            .set_border_left(border(l))
            .set_border_left_color(rgb(l.color));
    }
    if let Some(l) = &b.right {
        f = f
            .set_border_right(border(l))
            .set_border_right_color(rgb(l.color));
    }
    if let Some(p) = s.protection {
        if !p.locked {
            f = f.set_unlocked();
        }
        // AOO `XclExpCellProt` (xestyle.cxx): `hidden` = hide formula or
        // hide all; Excel has no "Hide all" or "Hide when printing".
        if p.hidden_formula || p.hidden {
            f = f.set_hidden();
        }
        if p.hidden {
            report.warn_once(
                "the cell protection \"Hide all\" has no Excel equivalent; written as \"Hide formula\"",
            );
        }
        if p.hide_print {
            report.warn_once(
                "the cell protection \"Hide when printing\" has no Excel equivalent and was dropped",
            );
        }
    }
    if let Some(code) = &s.number_format {
        match ooo_to_excel(code) {
            Some(x) => f = f.set_num_format(x),
            None => report.warn(format!(
                "number format {code:?} has no Excel equivalent; written as General"
            )),
        }
    }
    f
}

/// The text `rust_xlsxwriter` stores as a formula's cached result.
fn result_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::Number(n) if n.is_finite() => format!("{n}"),
        Value::Text(t) => t.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Error(k) => excel_error(*k).into(),
        _ => return None,
    })
}

fn excel_error(k: ErrorKind) -> &'static str {
    match k {
        ErrorKind::Num => "#NUM!",
        ErrorKind::Null => "#NULL!",
        ErrorKind::Ref => "#REF!",
        ErrorKind::Name => "#NAME?",
        ErrorKind::Div0 => "#DIV/0!",
        ErrorKind::NA => "#N/A",
        _ => "#VALUE!",
    }
}

struct SheetWriter<'a> {
    data: &'a WorkbookData,
    formats: &'a [Format],
    date_styles: &'a [bool],
    epoch: DateEpoch,
    resolver: &'a XlsxResolver,
    sheet_index: u32,
}

impl SheetWriter<'_> {
    fn fmt(&self, style: Option<u32>) -> Option<&Format> {
        style.and_then(|s| self.formats.get(s as usize))
    }

    fn is_date(&self, style: Option<u32>) -> bool {
        style.is_some_and(|s| self.date_styles.get(s as usize).copied().unwrap_or(false))
    }

    fn number(&self, n: f64, style: Option<u32>) -> f64 {
        if self.is_date(style) {
            tally_serial_to_excel(n, self.epoch)
        } else {
            n
        }
    }

    fn write_value(
        &self,
        ws: &mut Worksheet,
        c: &CellData,
        v: &Value,
        report: &mut Report,
    ) -> Result<(), XlsxError> {
        let (row, col) = (c.row, col_num(c.col));
        let fmt = self.fmt(c.style);
        match v {
            Value::Empty => {
                if let Some(f) = fmt {
                    ws.write_blank(row, col, f)?;
                }
            }
            Value::Number(n) => {
                let n = self.number(*n, c.style);
                match fmt {
                    Some(f) => ws.write_number_with_format(row, col, n, f)?,
                    None => ws.write_number(row, col, n)?,
                };
            }
            Value::Text(t) => {
                let text: &str = t;
                let owned;
                let text = if text.chars().count() > MAX_EXCEL_TEXT {
                    report.warn(format!(
                        "{}: text longer than 32,767 characters was truncated",
                        cell_name(self.sheet_index, c, self.data)
                    ));
                    owned = text.chars().take(MAX_EXCEL_TEXT).collect::<String>();
                    owned.as_str()
                } else {
                    text
                };
                match fmt {
                    Some(f) => ws.write_string_with_format(row, col, text, f)?,
                    None => ws.write_string(row, col, text)?,
                };
            }
            Value::Bool(b) => {
                match fmt {
                    Some(f) => ws.write_boolean_with_format(row, col, *b, f)?,
                    None => ws.write_boolean(row, col, *b)?,
                };
            }
            Value::Error(k) => {
                // OOXML error cells are written as an error formula with the
                // same cached error.
                let e = excel_error(*k);
                let formula = Formula::new(e).set_result(e);
                match fmt {
                    Some(f) => ws.write_formula_with_format(row, col, formula, f)?,
                    None => ws.write_formula(row, col, formula)?,
                };
            }
            Value::Array(m) => {
                report.warn(format!(
                    "{}: an inline array value was written as its first element",
                    cell_name(self.sheet_index, c, self.data)
                ));
                if let Some(first) = m.get(0, 0) {
                    if !matches!(first, Value::Array(_)) {
                        let first = first.clone();
                        self.write_value(ws, c, &first, report)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn write_formula(
        &self,
        ws: &mut Worksheet,
        c: &CellData,
        text: &str,
        cached: Option<&Value>,
        array: Option<tally_model::CellRangeLocal>,
        report: &mut Report,
    ) -> Result<(), XlsxError> {
        let origin = CellAddr::new(self.sheet_index, c.row, c.col);
        let Some(excel) = of_to_excel(text, origin, self.resolver) else {
            report.warn(format!(
                "{}: formula {text:?} could not be translated; its cached value was written",
                cell_name(self.sheet_index, c, self.data)
            ));
            if let Some(v) = cached {
                self.write_value(ws, c, v, report)?;
            }
            return Ok(());
        };
        let mut formula = Formula::new(&excel);
        if let Some(v) = cached {
            let v = match v {
                Value::Number(n) => Value::Number(self.number(*n, c.style)),
                Value::Array(m) => m.get(0, 0).cloned().unwrap_or(Value::Empty),
                other => other.clone(),
            };
            if let Some(r) = result_text(&v) {
                formula = formula.set_result(r);
            }
        }
        let (row, col) = (c.row, col_num(c.col));
        match (array, self.fmt(c.style)) {
            (Some(a), Some(f)) => ws.write_array_formula_with_format(
                a.r0,
                col_num(a.c0),
                a.r1,
                col_num(a.c1),
                formula,
                f,
            )?,
            (Some(a), None) => {
                ws.write_array_formula(a.r0, col_num(a.c0), a.r1, col_num(a.c1), formula)?
            }
            (None, Some(f)) => ws.write_formula_with_format(row, col, formula, f)?,
            (None, None) => ws.write_formula(row, col, formula)?,
        };
        Ok(())
    }

    fn write_sheet(
        &self,
        ws: &mut Worksheet,
        sheet: &SheetData,
        report: &mut Report,
    ) -> Result<(), XlsxError> {
        let defaults = SheetDefaults::new(self.data, sheet);
        // Column widths and hidden columns. The columns of the default
        // width are not written (`sheetFormatPr` has it), except where
        // `rust_xlsxwriter` would use its own default instead.
        let col_px = defaults.col_px();
        if col_px != defaults.writer_col_px {
            for (c0, c1) in default_width_columns(sheet) {
                ws.set_column_range_width_pixels(c0, c1, col_px)?;
            }
        }
        for (start, end, what) in segments(&sheet.col_widths, MAX_COLS) {
            if what == defaults.cols {
                continue;
            }
            // A column without a run under another default: 100 px.
            let (size, hidden) = what.unwrap_or((APP_DEFAULT_COL_WIDTH_HMM, false));
            if !hidden && hmm_to_px(size) == col_px {
                continue;
            }
            let (c0, c1) = (col_num(start), col_num(end));
            ws.set_column_range_width_pixels(c0, c1, hmm_to_px(size))?;
            if hidden {
                ws.set_column_range_hidden(c0, c1)?;
            }
        }
        // Row heights and hidden rows: the rows that are not what the
        // sheet's default says (see `SheetDefaults`), one by one.
        let row_segments = segments(&sheet.row_heights, MAX_ROWS);
        let row_px = defaults.row_px();
        ws.set_default_row_height_pixels(row_px);
        let default_hidden = matches!(defaults.rows, Some((_, true)));
        if default_hidden {
            ws.hide_unused_rows(true);
        }
        let mut sized = 0u32;
        let mut capped = false;
        let mut count = |report: &mut Report| {
            if sized >= MAX_SIZED_ROWS {
                if !capped {
                    report.warn(format!(
                        "sheet {:?}: only the first {MAX_SIZED_ROWS} row heights were written",
                        sheet.name
                    ));
                }
                capped = true;
                return false;
            }
            sized += 1;
            true
        };
        'rows: for &(start, end, what) in &row_segments {
            if what == defaults.rows {
                continue;
            }
            // A row without a run under another default: a row of 24 px.
            let (size, hidden) = what.unwrap_or((APP_DEFAULT_ROW_HEIGHT_HMM, false));
            if defaults.rows.is_none() && !hidden && size == APP_DEFAULT_ROW_HEIGHT_HMM {
                // Under the automatic default a set height of 24 px is not
                // kept apart from an automatic row.
                continue;
            }
            for row in start..=end {
                if !count(report) {
                    break 'rows;
                }
                ws.set_row_height_pixels(row, hmm_to_px(size))?;
                if hidden {
                    ws.set_row_hidden(row)?;
                }
            }
        }
        if default_hidden {
            // A `<row>` element is a shown row unless it says otherwise,
            // and the rows with cells, formatted blanks or an outline
            // level get one: those that take the hidden default say so.
            let is_default = |row: u32| {
                let at = row_segments.partition_point(|s| s.1 < row);
                row_segments.get(at).is_some_and(|s| s.2 == defaults.rows)
            };
            let cells = sheet.cells.iter().map(|c| (c.row, c.row));
            let blanks = sheet
                .style_runs
                .iter()
                .filter(|r| {
                    r.start_row <= r.end_row && r.end_row - r.start_row < MAX_STYLED_EMPTY_RUN
                })
                .map(|r| (r.start_row, r.end_row));
            let groups = sheet.row_groups.iter().map(|g| (g.start, g.end));
            let mut last = None;
            'hidden: for (start, end) in cells.chain(blanks).chain(groups) {
                for row in start..=end.min(MAX_ROWS - 1) {
                    if last == Some(row) || !is_default(row) {
                        continue;
                    }
                    last = Some(row);
                    if !count(report) {
                        break 'hidden;
                    }
                    ws.set_row_hidden(row)?;
                }
            }
        }
        // Rows hidden by the AutoFilter are hidden rows in Excel.
        if let Some(af) = &sheet.auto_filter {
            let mut n = 0u32;
            'spans: for &(a, b) in &af.filtered_rows {
                for row in a..=b.min(MAX_ROWS - 1) {
                    if n >= MAX_SIZED_ROWS {
                        report.warn(format!(
                            "sheet {:?}: only the first {MAX_SIZED_ROWS} filtered rows were \
                             written hidden",
                            sheet.name
                        ));
                        break 'spans;
                    }
                    n += 1;
                    ws.set_row_hidden(row)?;
                }
            }
        }
        // Formatting of blank areas.
        for run in &sheet.style_runs {
            let Some(fmt) = self.fmt(Some(run.style)) else {
                continue;
            };
            if run.col >= MAX_COLS || run.start_row > run.end_row {
                continue;
            }
            let col = col_num(run.col);
            if run.end_row - run.start_row >= MAX_STYLED_EMPTY_RUN {
                ws.set_column_range_format(col, col, fmt)?;
            } else {
                for row in run.start_row..=run.end_row.min(MAX_ROWS - 1) {
                    ws.write_blank(row, col, fmt)?;
                }
            }
        }
        // Merges first: `merge_range` writes the anchor; the real anchor
        // content is written over it below.
        for m in &sheet.merges {
            if m.r0 == m.r1 && m.c0 == m.c1 {
                continue;
            }
            if m.r1 >= MAX_ROWS || m.c1 >= MAX_COLS {
                continue;
            }
            let anchor_style = sheet
                .cells
                .iter()
                .find(|c| c.row == m.r0 && c.col == m.c0)
                .and_then(|c| c.style);
            let default = base_format(self.data);
            let fmt = self.fmt(anchor_style).unwrap_or(&default);
            if let Err(e) = ws.merge_range(m.r0, col_num(m.c0), m.r1, col_num(m.c1), "", fmt) {
                report.warn(format!("sheet {:?}: merge skipped ({e})", sheet.name));
            }
        }
        for c in &sheet.cells {
            if c.row >= MAX_ROWS || c.col >= MAX_COLS {
                continue;
            }
            match &c.content {
                CellContentData::Value(v) => self.write_value(ws, c, v, report)?,
                CellContentData::Formula { text, cached } => {
                    let array = sheet
                        .array_formulas
                        .iter()
                        .find(|a| a.r0 == c.row && a.c0 == c.col)
                        .copied();
                    self.write_formula(ws, c, text, cached.as_ref(), array, report)?;
                }
                CellContentData::SharedFormula {
                    text,
                    anchor_row,
                    anchor_col,
                    cached,
                } => {
                    // Re-anchor the group's formula at this cell.
                    let parsed_at = CellAddr::new(self.sheet_index, *anchor_row, *anchor_col);
                    let printed_at = CellAddr::new(self.sheet_index, c.row, c.col);
                    let moved =
                        crate::formula::of_to_excel_at(text, parsed_at, printed_at, self.resolver)
                            .and_then(|x| {
                                let f = crate::formula::parse_excel(&x, printed_at, self.resolver)
                                    .ok()?;
                                Some(crate::formula::print_of(&f, printed_at, self.resolver))
                            });
                    match moved {
                        Some(t) => self.write_formula(ws, c, &t, cached.as_ref(), None, report)?,
                        None => {
                            if let Some(v) = cached {
                                self.write_value(ws, c, v, report)?;
                            }
                        }
                    }
                }
            }
        }
        self.write_links(ws, sheet, report)?;
        let warn: &mut dyn crate::write_v03::Warn = report;
        crate::write_v03::write_conditional_formats(
            ws,
            sheet,
            self.sheet_index,
            self.resolver,
            warn,
        )?;
        crate::write_v03::write_validations(ws, sheet, self.sheet_index, self.resolver, warn)?;
        crate::write_v03::write_notes(ws, sheet, warn);
        crate::write_v03::write_page_setup(ws, sheet, warn)?;
        // Wave 5 (placeholders until W5-1/W5-2 land).
        crate::write_v04::write_outline(ws, sheet, warn)?;
        crate::write_v04::write_charts(ws, sheet, self.sheet_index, self.resolver, warn)?;
        crate::write_v04::write_images(ws, sheet, warn)?;
        if let Some(p) = &sheet.protection {
            crate::write_v03::write_protection(ws, p);
        }
        if let Some((rows, cols)) = sheet.frozen {
            if rows > 0 || cols > 0 {
                ws.set_freeze_panes(rows.min(MAX_ROWS - 1), col_num(cols.min(MAX_COLS - 1)))?;
            }
        }
        if sheet
            .view_options
            .as_ref()
            .is_some_and(|v| !v.show_gridlines)
        {
            ws.set_screen_gridlines(false);
        }
        if let Some(c) = sheet.tab_color {
            ws.set_tab_color(rgb(c));
        }
        Ok(())
    }
}

impl SheetWriter<'_> {
    /// Hyperlinks: written as URL cells over text (or empty) cells; links on
    /// numbers and formulas are dropped with a warning (rust_xlsxwriter
    /// stores hyperlinks as string cells).
    fn write_links(
        &self,
        ws: &mut Worksheet,
        sheet: &SheetData,
        report: &mut Report,
    ) -> Result<(), XlsxError> {
        for l in &sheet.links {
            if l.row >= MAX_ROWS || l.col >= MAX_COLS {
                continue;
            }
            let cell = sheet
                .cells
                .iter()
                .find(|c| c.row == l.row && c.col == l.col);
            let text = match cell.map(|c| &c.content) {
                None | Some(CellContentData::Value(Value::Empty)) => l.url.to_string(),
                Some(CellContentData::Value(Value::Text(t))) => t.to_string(),
                Some(_) => {
                    report.warn(format!(
                        "sheet {:?}: the hyperlink on {} (not a text cell) was not written",
                        sheet.name,
                        cell_name(
                            self.sheet_index,
                            cell.unwrap_or(&CellData {
                                row: l.row,
                                col: l.col,
                                content: CellContentData::Value(Value::Empty),
                                style: None,
                            }),
                            self.data
                        )
                    ));
                    continue;
                }
            };
            let Some(url) = crate::write_v03::excel_url(&l.url) else {
                report.warn(format!(
                    "sheet {:?}: the hyperlink {:?} has a type Excel does not support",
                    sheet.name, l.url
                ));
                continue;
            };
            // An unstyled link is in the default font (not Calibri).
            let link_format =
                default_font(self.data).map(|_| base_format(self.data).set_hyperlink());
            let fmt = self
                .fmt(cell.and_then(|c| c.style))
                .or(link_format.as_ref());
            if let Err(e) = ws.write_url_with_options(l.row, col_num(l.col), url, text, "", fmt) {
                report.warn(format!(
                    "sheet {:?}: a hyperlink was skipped ({e})",
                    sheet.name
                ));
            }
        }
        Ok(())
    }
}

fn col_num(c: u32) -> u16 {
    u16::try_from(c.min(MAX_COLS - 1)).unwrap_or(u16::MAX)
}

fn cell_name(sheet: u32, c: &CellData, data: &WorkbookData) -> String {
    let name = data
        .sheets
        .get(sheet as usize)
        .map_or("?", |s| s.name.as_str());
    format!(
        "{name}!{}{}",
        tally_types::col_to_letters(c.col),
        u64::from(c.row) + 1
    )
}

/// Quotes a sheet name for a defined-name scope (`'My Sheet'!Name`).
fn scoped(sheet: &str, name: &str) -> String {
    let plain = sheet.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !sheet.chars().next().is_some_and(|c| c.is_ascii_digit());
    if plain {
        format!("{sheet}!{name}")
    } else {
        format!("'{}'!{name}", sheet.replace('\'', "''"))
    }
}

pub(crate) fn write(
    data: &WorkbookData,
    mut writer: impl Write + Seek,
) -> Result<WriteReport, XlsxError> {
    let mut report = Report {
        warnings: Vec::new(),
        dropped: 0,
    };
    let epoch = data.settings.calc_settings.date_epoch;
    let mut wb = Workbook::new();
    // The sheets whose default sizes need an edit of their XML (nearly
    // every sheet made in Tally) are compressed after the edit. So that
    // they are not compressed at full cost twice, `rust_xlsxwriter` then
    // writes the package at its fastest level (it has no "stored") and
    // the edit pass compresses every part at the normal one: about 15 %
    // on top of a save without edits, instead of 80 %.
    let mut size_patches = crate::write_v03::Patches::default();
    default_size_patches(data, &mut size_patches);
    let compress_later = !size_patches.is_empty();
    if compress_later {
        wb.set_zip_compression_level(1);
    }
    // The default font becomes the `Normal` style's (fonts[0]); Calibri
    // 11's column metrics (64 px) and row height (20 px) are kept.
    let base = base_format(data);
    if default_font(data).is_some() {
        wb.set_default_format(&base, 20, default_col_width_px(data))?;
    }

    let formats: Vec<Format> = data
        .styles
        .iter()
        .map(|s| format_of(s, &base, &mut report))
        .collect();
    let date_styles: Vec<bool> = data
        .styles
        .iter()
        .map(|s| s.number_format.as_deref().is_some_and(is_date_format))
        .collect();

    let mut taken = HashSet::new();
    let original: Vec<String> = data.sheets.iter().map(|s| s.name.clone()).collect();
    let excel_names: Vec<String> = original
        .iter()
        .enumerate()
        .map(|(i, n)| excel_sheet_name(n, i, &mut taken))
        .collect();
    for (o, e) in original.iter().zip(&excel_names) {
        if o != e {
            report.warn(format!(
                "sheet {o:?} was renamed to {e:?} (Excel naming rules)"
            ));
        }
    }
    let resolver = XlsxResolver::renaming(&original, &excel_names);

    let sheets: &[SheetData] = if data.sheets.is_empty() {
        &[]
    } else {
        &data.sheets
    };
    let active = data.settings.active_sheet as usize;
    for (i, sheet) in sheets.iter().enumerate() {
        let ws = wb.add_worksheet();
        ws.set_name(excel_names[i].as_str())?;
        let w = SheetWriter {
            data,
            formats: &formats,
            date_styles: &date_styles,
            epoch,
            resolver: &resolver,
            sheet_index: u32::try_from(i).unwrap_or(u32::MAX),
        };
        w.write_sheet(ws, sheet, &mut report)?;
        if sheet.hidden && i != active {
            ws.set_hidden(true);
        }
        if i == active {
            ws.set_active(true);
        }
    }
    if sheets.is_empty() {
        // An empty sheet, with the sizes of one (`default_size_patches`).
        let defaults = SheetDefaults::new(data, &SheetData::default());
        wb.add_worksheet()
            .set_default_row_height_pixels(defaults.row_px());
    }

    // Defined names.
    for n in &data.names {
        let scope = n.sheet.and_then(|s| excel_names.get(s as usize));
        let origin = CellAddr::new(n.sheet.unwrap_or(0), 0, 0);
        let Some(text) = of_to_excel(&n.formula, origin, &resolver) else {
            report.warn(format!(
                "name {:?}: formula could not be translated",
                n.name
            ));
            continue;
        };
        if n.name == "Print_Area" {
            if let (Some(sheet_idx), Some(r)) = (n.sheet, print_area(&n.formula, origin, &resolver))
            {
                if let Ok(ws) = wb.worksheet_from_index(sheet_idx as usize) {
                    ws.set_print_area(r.r0, col_num(r.c0), r.r1, col_num(r.c1))?;
                    continue;
                }
            }
            report.warn("a print area that is not a single range was dropped");
            continue;
        }
        let full = match scope {
            Some(sheet) => scoped(sheet, &n.name),
            None => n.name.clone(),
        };
        if let Err(e) = wb.define_name(full, &format!("={text}")) {
            report.warn(format!("name {:?} was dropped ({e})", n.name));
        }
    }

    let mut props = DocProperties::new();
    let m = &data.meta;
    if let Some(t) = &m.title {
        props = props.set_title(t.as_str());
    }
    if let Some(t) = &m.subject {
        props = props.set_subject(t.as_str());
    }
    if let Some(t) = &m.author {
        props = props.set_author(t.as_str());
    }
    if !m.keywords.is_empty() {
        props = props.set_keywords(m.keywords.join(", "));
    }
    if let Some(t) = &m.description {
        props = props.set_comment(t.as_str());
    }
    wb.set_properties(&props);

    let bytes = wb.save_to_buffer()?;
    let mut patches = crate::write_v03::patches(data, &mut report);
    patches.date1904 = epoch == DateEpoch::D1904;
    patches.chart_literals = crate::chart_write::take_literals();
    // Wave 5 parts added through the patches pass (placeholders).
    crate::write_v04::scenario_patches(data, &mut patches, &mut report);
    crate::autofilter::autofilter_patches(data, &mut patches, &mut report);
    crate::write_v04::pivot_patches(data, &mut patches, &mut report);
    crate::tools::style_patches(data, &mut patches);
    crate::tools::view_patches(data, &mut patches);
    patches.sheet_format_attrs = size_patches.sheet_format_attrs;
    patches.row_attr_strips = size_patches.row_attr_strips;
    patches.hidden_col_widths = size_patches.hidden_col_widths;
    patches.compress = compress_later;
    let bytes = if patches.is_empty() {
        bytes
    } else {
        crate::write_v03::apply_patches(&bytes, &patches)?
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(report.finish())
}

/// The single area a `Print_Area` name covers.
fn print_area(
    formula: &str,
    origin: CellAddr,
    r: &XlsxResolver,
) -> Option<tally_model::CellRangeLocal> {
    let f =
        tally_formula::parse(formula, origin, tally_formula::Dialect::open_formula(), r).ok()?;
    let tally_formula::Expr::Ref(re) = &f.expr else {
        return None;
    };
    match re.resolve(origin) {
        tally_formula::ResolvedRef::Range(cr) => Some(tally_model::CellRangeLocal {
            r0: cr.r0,
            c0: cr.c0,
            r1: cr.r1,
            c1: cr.c1,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_names() {
        let mut t = HashSet::new();
        assert_eq!(excel_sheet_name("Data", 0, &mut t), "Data");
        assert_eq!(excel_sheet_name("data", 1, &mut t), "data (2)");
        assert_eq!(excel_sheet_name("a/b:c", 2, &mut t), "a_b_c");
        assert_eq!(excel_sheet_name("", 3, &mut t), "Sheet4");
        assert_eq!(excel_sheet_name("History", 4, &mut t), "Sheet5");
        let long = "x".repeat(40);
        assert_eq!(excel_sheet_name(&long, 5, &mut t).chars().count(), 31);
        assert_eq!(
            excel_sheet_name(&long, 6, &mut t),
            format!("{} (2)", "x".repeat(27))
        );
    }

    #[test]
    fn scopes() {
        assert_eq!(scoped("Sheet1", "Tax"), "Sheet1!Tax");
        assert_eq!(scoped("My Sheet", "Tax"), "'My Sheet'!Tax");
        assert_eq!(scoped("It's", "Tax"), "'It''s'!Tax");
    }
}
