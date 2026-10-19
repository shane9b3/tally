//! Read-only queries behind the `DocumentService`: cell views, layout,
//! formula-bar text and hints, selection statistics, Ctrl+arrow navigation
//! and the name box.

use tally_api::{
    cell_flags, ApiError, CellBlock, CellEditInfo, CellPos, CellView, DimRunPx, Direction,
    RangeDto, SelectionDto, SelectionStats, SheetLayout, ValueKind,
};
use tally_formula::{explain, FuncId as AstFuncId, LabelSource};
use tally_functions::numeric::ApproxSum;
use tally_functions::{registry, FuncId};
use tally_numfmt::{
    format_value_with, general_string, parse_format, recognize_input_with, Formatted,
    RecognizeContext, SectionKind, EN_US,
};
use tally_types::{
    letters_to_col, CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS,
};

use crate::document::range_dto;
use crate::storage::{CellContent, ColumnStore, Dir, Sheet, StyleId};
use crate::styles::StyleEntry;
use crate::units::{hmm_to_px, DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use crate::workbook::Workbook;

/// Most cells one `cells()` call may cover.
pub const MAX_CELLS_PER_CALL: u64 = 20_000;

fn sheet_or_err(wb: &Workbook, sheet: SheetId) -> Result<&Sheet, ApiError> {
    wb.sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
}

/// Formats a value with a style (no width: the grid fits General numbers).
pub fn format_with(wb: &Workbook, v: &Value, style: &StyleEntry) -> Formatted {
    format_value_with(v, &style.format, &EN_US, None, wb.settings.date_epoch)
}

/// The display text of a cell.
pub fn display_text(wb: &Workbook, addr: CellAddr) -> String {
    let v = wb.value(addr);
    if v.is_empty() {
        return String::new();
    }
    let st = wb.styles.get(wb.style_at(addr));
    format_with(wb, &v, st).text
}

/// Sheet geometry.
pub fn sheet_layout(wb: &Workbook, sheet: SheetId) -> Result<SheetLayout, ApiError> {
    let s = sheet_or_err(wb, sheet)?;
    let runs = |d: &crate::storage::DimRuns| -> Vec<DimRunPx> {
        d.runs()
            .iter()
            .map(|r| DimRunPx {
                start: r.start,
                end: r.end,
                size: if r.hidden { 0.0 } else { hmm_to_px(r.size_hmm) },
                hidden: r.hidden,
            })
            .collect()
    };
    Ok(SheetLayout {
        sheet,
        used: s.used_range().map(|r| range_dto(&r)),
        default_col_width_px: DEFAULT_COL_WIDTH_PX,
        default_row_height_px: DEFAULT_ROW_HEIGHT_PX,
        col_runs: runs(&s.col_dims),
        row_runs: runs(&s.row_dims),
        frozen: None,
        merges: Vec::new(),
    })
}

fn view_of(
    wb: &Workbook,
    addr: CellAddr,
    content: Option<&CellContent>,
    style: StyleId,
) -> CellView {
    let mut flags = 0u8;
    let v = match content {
        None => Value::Empty,
        Some(CellContent::Value(v)) => v.clone(),
        Some(CellContent::Formula(id)) => {
            flags |= cell_flags::FORMULA;
            wb.formulas
                .get(*id)
                .map_or(Value::Empty, |f| f.cached.clone())
        }
    };
    let st = wb.styles.get(style);
    let kind = match &v {
        Value::Empty => ValueKind::Empty,
        Value::Number(_) => ValueKind::Number,
        Value::Text(_) => ValueKind::Text,
        Value::Bool(_) => ValueKind::Bool,
        Value::Error(_) => {
            flags |= cell_flags::ERROR;
            ValueKind::Error
        }
        Value::Array(_) => ValueKind::Number,
    };
    let (text, color) = if v.is_empty() {
        (String::new(), None)
    } else {
        let f = format_with(wb, &v, st);
        (f.text, f.color.map(|c| (c << 8) | 0xff))
    };
    let num = match &v {
        Value::Number(n) if st.is_general() => Some(*n),
        _ => None,
    };
    CellView {
        r: addr.row,
        c: addr.col,
        text,
        kind,
        num,
        style: style.0,
        color,
        flags,
    }
}

/// Cell views for the requested ranges (non-empty or styled cells only).
pub fn cells(
    wb: &Workbook,
    version: u64,
    sheet: SheetId,
    ranges: &[RangeDto],
) -> Result<Vec<CellBlock>, ApiError> {
    let s = sheet_or_err(wb, sheet)?;
    let total: u64 = ranges
        .iter()
        .map(|r| {
            if r.r0 > r.r1 || r.c0 > r.c1 {
                0
            } else {
                u64::from(r.r1 - r.r0 + 1) * u64::from(r.c1 - r.c0 + 1)
            }
        })
        .sum();
    if total > MAX_CELLS_PER_CALL {
        return Err(ApiError::invalid_argument(format!(
            "{total} cells requested; at most {MAX_CELLS_PER_CALL} per call"
        )));
    }
    let mut out = Vec::with_capacity(ranges.len());
    for r in ranges {
        if r.r0 > r.r1 || r.c0 > r.c1 || r.r1 >= MAX_ROWS || r.c1 >= MAX_COLS {
            return Err(ApiError::invalid_argument("range outside the grid"));
        }
        let mut views = Vec::new();
        let c1 = r.c1.min(s.columns.len().saturating_sub(1) as u32);
        if !s.columns.is_empty() {
            for c in r.c0..=c1 {
                let Some(col) = s.column(c) else {
                    continue;
                };
                let mut rows: Vec<u32> = col
                    .cells
                    .iter_range(r.r0, r.r1)
                    .map(|(row, _)| row)
                    .collect();
                for (a, b, _) in col.styles.runs_in(r.r0, r.r1) {
                    rows.extend(a..=b);
                }
                rows.sort_unstable();
                rows.dedup();
                for row in rows {
                    let addr = CellAddr::new(sheet, row, c);
                    views.push(view_of(wb, addr, col.cells.get(row), col.styles.get(row)));
                }
            }
        }
        views.sort_by_key(|v| (v.r, v.c));
        out.push(CellBlock {
            sheet,
            range: *r,
            version,
            cells: views,
        });
    }
    Ok(out)
}

/// Labels for `explain`: the text header at the top of a column's data
/// block, or at the left of a row of numbers.
pub struct HeaderLabels<'a> {
    wb: &'a Workbook,
}

impl<'a> HeaderLabels<'a> {
    /// Labels from `wb`.
    pub fn new(wb: &'a Workbook) -> Self {
        Self { wb }
    }

    fn text_at(&self, sheet: &Sheet, r: u32, c: u32) -> Option<Option<String>> {
        // Some(Some(text)) for text, Some(None) for other content, None for empty.
        match sheet.get(r, c)? {
            CellContent::Value(Value::Text(t)) => Some(Some(t.to_string())),
            CellContent::Formula(id) => match self.wb.formulas.get(*id).map(|f| &f.cached) {
                Some(Value::Text(t)) => Some(Some(t.to_string())),
                _ => Some(None),
            },
            CellContent::Value(_) => Some(None),
        }
    }

    fn walk(&self, sheet: &Sheet, addr: CellAddr, vertical: bool) -> Option<String> {
        let mut found: Option<String> = None;
        let mut steps = 0;
        let (mut r, mut c) = (addr.row, addr.col);
        loop {
            if vertical {
                if r == 0 {
                    break;
                }
                r -= 1;
            } else {
                if c == 0 {
                    break;
                }
                c -= 1;
            }
            steps += 1;
            if steps > 2000 {
                return None;
            }
            match self.text_at(sheet, r, c) {
                None => break,
                Some(Some(t)) => {
                    if found.is_some() {
                        // Text between the header and the cell: not a header.
                        return None;
                    }
                    found = Some(t);
                }
                Some(None) => {
                    if found.is_some() {
                        return None;
                    }
                }
            }
        }
        found.filter(|t| !t.trim().is_empty())
    }
}

impl LabelSource for HeaderLabels<'_> {
    fn label_for(&self, addr: CellAddr) -> Option<String> {
        let sheet = self.wb.sheet(addr.sheet)?;
        self.walk(sheet, addr, true)
            .or_else(|| self.walk(sheet, addr, false))
    }

    fn range_label(&self, r: &CellRange) -> Option<String> {
        let sheet = self.wb.sheet(r.sheet)?;
        if r.c0 == r.c1 {
            self.walk(sheet, r.start(), true)
        } else if r.r0 == r.r1 {
            self.walk(sheet, r.start(), false)
        } else {
            None
        }
    }

    fn function_name(&self, id: AstFuncId) -> Option<std::sync::Arc<str>> {
        registry()
            .get(FuncId(id.0))
            .map(|d| std::sync::Arc::from(d.name))
    }
}

/// The text to edit for a constant.
fn edit_text(wb: &Workbook, v: &Value, st: &StyleEntry) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => {
            let fmt = &st.format;
            if fmt.is_date() {
                // Whole milliseconds of the time part; a non-zero remainder
                // mod 1000 means fractional seconds, which must survive
                // re-entry.
                let ms = (n.abs() * 86_400_000.0).round();
                let frac_secs = ms % 1000.0 != 0.0;
                let code: String = match fmt.sections.first().map(|s| &s.kind) {
                    Some(SectionKind::Date(d))
                        if d.has_date && (d.has_time || n.fract() != 0.0) =>
                    {
                        format!("MM/DD/YYYY HH:MM:SS{}", if frac_secs { ".000" } else { "" })
                    }
                    Some(SectionKind::Date(d)) if !d.has_date => {
                        // AOO's GetInputLineString: durations (below 0 or a
                        // day or more) edit as elapsed hours so the days
                        // are kept.
                        let hours = if *n < 0.0 || *n >= 1.0 { "[HH]" } else { "HH" };
                        format!("{hours}:MM:SS{}", if frac_secs { ".000" } else { "" })
                    }
                    _ => "MM/DD/YYYY".to_owned(),
                };
                if let Ok(ast) = parse_format(&code) {
                    return format_value_with(v, &ast, &EN_US, None, wb.settings.date_epoch).text;
                }
            }
            if let Some(SectionKind::Number(l)) = fmt.sections.first().map(|s| &s.kind) {
                if l.percent > 0 {
                    return format!("{}%", general_string(n * 100.0, None, &EN_US));
                }
            }
            general_string(*n, None, &EN_US)
        }
        Value::Text(s) => {
            let ctx = RecognizeContext::now(wb.settings.date_epoch);
            let looks_like_value = s.starts_with('=')
                || s.starts_with('\'')
                || !matches!(recognize_input_with(s, &EN_US, &ctx).value, Value::Text(_));
            if looks_like_value && !st.is_text_format() {
                format!("'{s}")
            } else {
                s.to_string()
            }
        }
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Value::Error(e) => e.display().into_owned(),
        Value::Array(m) => m
            .get(0, 0)
            .map_or_else(String::new, |x| edit_text(wb, x, st)),
    }
}

/// Formula-bar text and hint.
pub fn cell_edit_info(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Result<CellEditInfo, ApiError> {
    let s = sheet_or_err(wb, sheet)?;
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    let addr = CellAddr::new(sheet, r, c);
    match s.get(r, c) {
        None => Ok(CellEditInfo {
            input: String::new(),
            hint: None,
        }),
        Some(CellContent::Value(v)) => Ok(CellEditInfo {
            input: edit_text(wb, v, wb.styles.get(s.style(r, c))),
            hint: None,
        }),
        Some(CellContent::Formula(id)) => {
            let input = wb.formula_text(*id).unwrap_or_default();
            let hint = wb
                .formulas
                .get(*id)
                .filter(|f| f.invalid_text.is_none())
                .and_then(|f| explain(&f.formula, addr, &HeaderLabels::new(wb)));
            Ok(CellEditInfo { input, hint })
        }
    }
}

/// Status bar statistics.
pub fn selection_stats(wb: &Workbook, sel: &SelectionDto) -> Result<SelectionStats, ApiError> {
    let s = sheet_or_err(wb, sel.sheet)?;
    let mut count = 0u64;
    let mut nums = ApproxSum::new();
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    // The first error in the selection: AOO's status bar then shows the
    // error instead of the sum.
    let mut error: Option<ErrorKind> = None;
    let ranges: Vec<RangeDto> = if sel.ranges.is_empty() {
        vec![RangeDto {
            r0: sel.active.r,
            c0: sel.active.c,
            r1: sel.active.r,
            c1: sel.active.c,
        }]
    } else {
        sel.ranges.clone()
    };
    for r in &ranges {
        if r.r0 > r.r1 || r.c0 > r.c1 {
            continue;
        }
        let c1 = r.c1.min(s.columns.len().saturating_sub(1) as u32);
        if s.columns.is_empty() {
            continue;
        }
        for c in r.c0..=c1 {
            let Some(col) = s.column(c) else {
                continue;
            };
            for (_, content) in col.cells.iter_range(r.r0, r.r1) {
                let v = match content {
                    CellContent::Value(v) => v.clone(),
                    CellContent::Formula(id) => wb
                        .formulas
                        .get(*id)
                        .map_or(Value::Empty, |f| f.cached.clone()),
                };
                count += 1;
                // Booleans are the numbers 1 and 0 (AOO's storage model).
                let n = match v {
                    Value::Number(n) => n,
                    Value::Bool(b) => f64::from(u8::from(b)),
                    Value::Error(e) => {
                        error.get_or_insert(e);
                        continue;
                    }
                    _ => continue,
                };
                nums.add(n);
                min = min.min(n);
                max = max.max(n);
            }
        }
    }
    let n = nums.count();
    if let Some(e) = error {
        return Ok(SelectionStats {
            count,
            count_numbers: n,
            formatted_sum: Some(e.display().into_owned()),
            ..SelectionStats::default()
        });
    }
    if n == 0 {
        return Ok(SelectionStats {
            count,
            ..SelectionStats::default()
        });
    }
    let sum = nums.total();
    let st = wb.styles.get(s.style(sel.active.r, sel.active.c));
    let formatted_sum = if st.is_general() {
        general_string(sum, None, &EN_US)
    } else {
        format_with(wb, &Value::Number(sum), st).text
    };
    Ok(SelectionStats {
        sum: Some(sum),
        count,
        count_numbers: n,
        average: Some(sum / n as f64),
        min: Some(min),
        max: Some(max),
        formatted_sum: Some(formatted_sum),
    })
}

/// Ctrl+arrow: the data edge from `from` in `dir`.
pub fn navigate_edge(
    wb: &Workbook,
    sheet: SheetId,
    from: CellPos,
    dir: Direction,
) -> Result<CellPos, ApiError> {
    let s = sheet_or_err(wb, sheet)?;
    let (r, c) = (from.r.min(MAX_ROWS - 1), from.c.min(MAX_COLS - 1));
    let filled = |r: u32, c: u32| s.is_non_empty(r, c);
    let pos = match dir {
        Direction::Up | Direction::Down => {
            let down = dir == Direction::Down;
            let edge = if down { MAX_ROWS - 1 } else { 0 };
            if r == edge {
                return Ok(CellPos { r, c });
            }
            let step = |x: u32| if down { x + 1 } else { x - 1 };
            let col = s.column(c);
            if filled(r, c) && filled(step(r), c) {
                // Last non-empty of the run.
                let mut x = step(r);
                while x != edge && filled(step(x), c) {
                    x = step(x);
                }
                CellPos { r: x, c }
            } else {
                let d = if down { Dir::Down } else { Dir::Up };
                let next = col.and_then(|col| col.cells.next_non_empty(r, d));
                CellPos {
                    r: next.unwrap_or(edge),
                    c,
                }
            }
        }
        Direction::Left | Direction::Right => {
            let right = dir == Direction::Right;
            let edge = if right { MAX_COLS - 1 } else { 0 };
            if c == edge {
                return Ok(CellPos { r, c });
            }
            let step = |x: u32| if right { x + 1 } else { x - 1 };
            if filled(r, c) && filled(r, step(c)) {
                let mut x = step(c);
                while x != edge && filled(r, step(x)) {
                    x = step(x);
                }
                CellPos { r, c: x }
            } else {
                let last_col = s.columns.len() as u32;
                let mut x = c;
                let mut found = None;
                loop {
                    if x == edge {
                        break;
                    }
                    x = step(x);
                    if right && x >= last_col {
                        break;
                    }
                    if filled(r, x) {
                        found = Some(x);
                        break;
                    }
                }
                CellPos {
                    r,
                    c: found.unwrap_or(edge),
                }
            }
        }
    };
    Ok(pos)
}

/// Splits `Sheet.A1`, `Sheet!A1` or `'My Sheet'.A1` into sheet name and reference.
fn split_sheet(text: &str) -> Option<(Option<String>, &str)> {
    if let Some(rest) = text.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch == '\'' {
                if chars.peek().map(|(_, c)| *c) == Some('\'') {
                    chars.next();
                    name.push('\'');
                    continue;
                }
                let after = rest.get(i + 1..)?;
                let r = after
                    .strip_prefix('.')
                    .or_else(|| after.strip_prefix('!'))?;
                return Some((Some(name), r));
            }
            name.push(ch);
        }
        return None;
    }
    match text.rfind(['.', '!']) {
        Some(i) => Some((
            Some(text[..i].trim_start_matches('$').to_owned()),
            &text[i + 1..],
        )),
        None => Some((None, text)),
    }
}

fn parse_local_range(sheet: SheetId, s: &str) -> Option<CellRange> {
    if let Some(r) = CellRange::from_a1(sheet, s) {
        return Some(r);
    }
    let (a, b) = s.split_once(':')?;
    let (a, b) = (a.trim_start_matches('$'), b.trim_start_matches('$'));
    if let (Some(c0), Some(c1)) = (letters_to_col(a), letters_to_col(b)) {
        return Some(CellRange::new(sheet, 0, c0, MAX_ROWS - 1, c1));
    }
    let row = |x: &str| -> Option<u32> {
        let n: u32 = x.parse().ok()?;
        (1..=MAX_ROWS).contains(&n).then(|| n - 1)
    };
    if let (Some(r0), Some(r1)) = (row(a), row(b)) {
        return Some(CellRange::new(sheet, r0, 0, r1, MAX_COLS - 1));
    }
    None
}

/// Name box text → selection.
pub fn resolve_name_box(wb: &Workbook, text: &str) -> Result<SelectionDto, ApiError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(ApiError::invalid_argument("empty reference"));
    }
    let (name, rest) =
        split_sheet(t).ok_or_else(|| ApiError::invalid_argument(format!("bad reference {t:?}")))?;
    let sheet = match name {
        Some(n) => wb
            .sheet_by_name_ci(&n)
            .ok_or_else(|| ApiError::not_found(format!("no sheet named {n:?}")))?,
        None => wb.active_sheet,
    };
    let range = parse_local_range(sheet, rest)
        .ok_or_else(|| ApiError::not_found(format!("{t:?} is not a cell, range or known name")))?;
    Ok(SelectionDto {
        sheet,
        ranges: vec![range_dto(&range)],
        active: CellPos {
            r: range.r0,
            c: range.c0,
        },
    })
}
