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
    WorkbookData,
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

/// The `rust_xlsxwriter` format of a model style.
fn format_of(s: &StyleData, report: &mut Report) -> Format {
    let mut f = Format::new();
    if s.font.bold {
        f = f.set_bold();
    }
    if s.font.italic {
        f = f.set_italic();
    }
    if s.font.underline {
        f = f.set_underline(FormatUnderline::Single);
    }
    if s.font.strike {
        f = f.set_font_strikethrough();
    }
    if let Some(family) = &s.font.family {
        f = f.set_font_name(family.as_str());
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
        if p.hidden_formula || p.hidden {
            f = f.set_hidden();
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
        // Column widths and hidden columns.
        for run in &sheet.col_widths {
            if run.start >= MAX_COLS || run.start > run.end {
                continue;
            }
            let (c0, c1) = (col_num(run.start), col_num(run.end.min(MAX_COLS - 1)));
            ws.set_column_range_width_pixels(c0, c1, hmm_to_px(run.size_hmm))?;
            if run.hidden {
                ws.set_column_range_hidden(c0, c1)?;
            }
        }
        // Row heights and hidden rows.
        let mut sized = 0u32;
        for run in &sheet.row_heights {
            if run.start >= MAX_ROWS || run.start > run.end {
                continue;
            }
            let end = run.end.min(MAX_ROWS - 1);
            if run.start == 0 && end == MAX_ROWS - 1 && !run.hidden {
                ws.set_default_row_height_pixels(hmm_to_px(run.size_hmm));
                continue;
            }
            for row in run.start..=end {
                if sized >= MAX_SIZED_ROWS {
                    report.warn(format!(
                        "sheet {:?}: only the first {MAX_SIZED_ROWS} row heights were written",
                        sheet.name
                    ));
                    break;
                }
                sized += 1;
                ws.set_row_height_pixels(row, hmm_to_px(run.size_hmm))?;
                if run.hidden {
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
            let default = Format::new();
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
        crate::write_v04::write_outline(ws, sheet)?;
        crate::write_v04::write_charts(ws, sheet, self.sheet_index, warn)?;
        crate::write_v04::write_images(ws, sheet, warn)?;
        if let Some(p) = &sheet.protection {
            crate::write_v03::write_protection(ws, p);
        }
        if let Some((rows, cols)) = sheet.frozen {
            if rows > 0 || cols > 0 {
                ws.set_freeze_panes(rows.min(MAX_ROWS - 1), col_num(cols.min(MAX_COLS - 1)))?;
            }
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
            let fmt = self.fmt(cell.and_then(|c| c.style));
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

    let formats: Vec<Format> = data
        .styles
        .iter()
        .map(|s| format_of(s, &mut report))
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
        wb.add_worksheet();
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
    // Wave 5 parts added through the patches pass (placeholders).
    crate::write_v04::scenario_patches(data, &mut patches, &mut report);
    crate::write_v04::pivot_patches(data, &mut patches, &mut report);
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
