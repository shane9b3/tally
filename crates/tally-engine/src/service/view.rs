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
    RecognizeContext, SectionKind,
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

/// Longest display text `cells()` returns per cell, in characters; longer
/// text ends in `…` (the grid cannot show more; the full value stays in the
/// cell, the edit info and exports). Bounds every viewport call to
/// 20,000 × 1,024 characters (security review 2, HIGH-4).
pub const MAX_VIEW_TEXT: usize = 1024;

/// `text` cut to [`MAX_VIEW_TEXT`] characters.
pub fn cap_view_text(mut text: String) -> String {
    if text.len() <= MAX_VIEW_TEXT {
        return text;
    }
    if let Some((i, _)) = text.char_indices().nth(MAX_VIEW_TEXT - 1) {
        text.truncate(i);
        text.push('…');
    }
    text
}

fn sheet_or_err(wb: &Workbook, sheet: SheetId) -> Result<&Sheet, ApiError> {
    wb.sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
}

/// Formats a value with a style (no width: the grid fits General numbers).
pub fn format_with(wb: &Workbook, v: &Value, style: &StyleEntry) -> Formatted {
    format_value_with(v, &style.format, wb.locale, None, wb.settings.date_epoch)
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

/// Pixel runs of one axis; `filtered` spans (rows hidden by the AutoFilter)
/// are split out and reported as hidden with `filtered: true`; `outline`
/// spans (rows/columns of collapsed groups) are reported hidden without
/// the flag.
fn px_runs(
    d: &crate::storage::DimRuns,
    filtered: &[(u32, u32)],
    outline: &[(u32, u32)],
    default_hmm: u32,
) -> Vec<DimRunPx> {
    let mut out: Vec<DimRunPx> = Vec::new();
    let mut push = |start: u32, end: u32, hmm: u32, hidden: bool, filt: bool| {
        let hidden = hidden || filt;
        let size = if hidden { 0.0 } else { hmm_to_px(hmm) };
        if let Some(last) = out.last_mut() {
            if last.end.checked_add(1) == Some(start)
                && last.hidden == hidden
                && last.size == size
                && last.filtered.unwrap_or(false) == filt
            {
                last.end = end;
                return;
            }
        }
        out.push(DimRunPx {
            start,
            end,
            size,
            hidden,
            filtered: filt.then_some(true),
        });
    };
    if filtered.is_empty() && outline.is_empty() {
        for r in d.runs() {
            push(r.start, r.end, r.size_hmm, r.hidden, false);
        }
        return out;
    }
    // Merge the explicit runs with the hidden spans: walk the boundaries.
    let mut points: Vec<u32> = Vec::new();
    for r in d.runs() {
        points.push(r.start);
        points.push(r.end.saturating_add(1));
    }
    for (a, b) in filtered.iter().chain(outline) {
        points.push(*a);
        points.push(b.saturating_add(1));
    }
    points.sort_unstable();
    points.dedup();
    let covers = |spans: &[(u32, u32)], a: u32| {
        let i = spans.partition_point(|s| s.1 < a);
        spans.get(i).is_some_and(|s| s.0 <= a)
    };
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1] - 1);
        let run = d.get(a);
        let filt = covers(filtered, a);
        let outl = covers(outline, a);
        match run {
            Some(r) => push(a, b, r.size_hmm, r.hidden || outl, filt),
            None if filt || outl => push(a, b, default_hmm, outl, filt),
            None => {}
        }
    }
    out
}

/// Sheet geometry.
pub fn sheet_layout(wb: &Workbook, sheet: SheetId) -> Result<SheetLayout, ApiError> {
    let s = sheet_or_err(wb, sheet)?;
    Ok(SheetLayout {
        sheet,
        used: s.used_range().map(|r| range_dto(&r)),
        default_col_width_px: DEFAULT_COL_WIDTH_PX,
        default_row_height_px: DEFAULT_ROW_HEIGHT_PX,
        col_runs: px_runs(
            &s.col_dims,
            &[],
            s.outline_cols.spans(),
            crate::units::px_to_hmm(DEFAULT_COL_WIDTH_PX),
        ),
        row_runs: px_runs(
            &s.row_dims,
            s.filtered_rows.spans(),
            s.outline_rows.spans(),
            crate::units::px_to_hmm(DEFAULT_ROW_HEIGHT_PX),
        ),
        frozen: s
            .meta
            .frozen
            .filter(|(r, c)| *r > 0 || *c > 0)
            .map(|(r, c)| CellPos { r, c }),
        merges: s
            .meta
            .merges
            .iter()
            .map(|m| RangeDto {
                r0: m.r0,
                c0: m.c0,
                r1: m.r1,
                c1: m.c1,
            })
            .collect(),
        outline: crate::outline::to_api(&s.meta),
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
            match wb.formulas.get(*id) {
                Some(f) => {
                    if f.array.is_some() {
                        flags |= cell_flags::ARRAY;
                    }
                    f.cached.clone()
                }
                None => Value::Empty,
            }
        }
        Some(CellContent::ArrayPart(_)) => {
            flags |= cell_flags::FORMULA | cell_flags::ARRAY;
            wb.value(addr)
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
        (cap_view_text(f.text), f.color.map(|c| (c << 8) | 0xff))
    };
    let num = match &v {
        Value::Number(n) if st.is_general() => Some(*n),
        _ => None,
    };
    let link = match content {
        Some(CellContent::Formula(id)) => {
            wb.formulas.get(*id).and_then(|f| hyperlink_target(wb, f))
        }
        _ => None,
    };
    if link.is_some() {
        flags |= cell_flags::LINK;
    }
    CellView {
        r: addr.row,
        c: addr.col,
        text,
        kind,
        num,
        style: style.0,
        color,
        flags,
        cf_style: None,
        cf_bar: None,
        cf_icon: None,
        link,
    }
}

/// The target of a formula whose top-level call is `HYPERLINK(url; …)`.
fn hyperlink_target(wb: &Workbook, f: &crate::formula_cells::FormulaCell) -> Option<String> {
    use tally_formula::{Expr, FuncRef};
    let mut e = &f.formula.expr;
    while let Expr::Paren(x) = e {
        e = x;
    }
    let Expr::Call {
        func: FuncRef::Known(id),
        args,
    } = e
    else {
        return None;
    };
    let def = registry().get(FuncId(id.0))?;
    if !def.name.eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let arg = args.first()?;
    let expr = crate::fexpr::FExpr {
        formula: std::sync::Arc::new(tally_formula::Formula::new(arg.clone())),
        invalid: None,
    };
    match expr.eval(wb, f.addr) {
        Value::Text(t) if !t.trim().is_empty() => Some(t.to_string()),
        _ => None,
    }
}

/// `true` if the sheet has state that decorates cell views.
fn has_decorations(s: &Sheet) -> bool {
    let m = &s.meta;
    !m.cond_formats.is_empty()
        || !m.notes.is_empty()
        || !m.links.is_empty()
        || m.validations.iter().any(|v| v.has_dropdown())
}

/// Adds notes, links, validation dropdowns and conditional formats to the
/// views of `req` (adding views for otherwise empty cells that get one).
/// `eval` is the call's conditional-format evaluator (one work budget per
/// `cells()` call).
fn decorate(
    wb: &Workbook,
    s: &Sheet,
    eval: &crate::condfmt::CfEval<'_>,
    req: &CellRange,
    views: &mut Vec<CellView>,
) {
    use std::collections::HashSet;
    let m = &s.meta;
    let have: HashSet<(u32, u32)> = views.iter().map(|v| (v.r, v.c)).collect();
    // Cells of `req` that need a view: a bitmap over the request (at most
    // MAX_CELLS_PER_CALL cells), marked per distinct rectangle, so a
    // thousand formats on one range cost one pass (CRITICAL-1).
    let cols = u64::from(req.c1 - req.c0) + 1;
    let area = (u64::from(req.r1 - req.r0) + 1) * cols;
    let mut extra = vec![false; area as usize];
    let mut any_extra = false;
    let mark = |extra: &mut Vec<bool>, any: &mut bool, r: u32, c: u32| {
        let i = (u64::from(r - req.r0) * cols + u64::from(c - req.c0)) as usize;
        extra[i] = true;
        *any = true;
    };
    for (k, _) in crate::annotations::entries_in(&m.notes, req) {
        mark(&mut extra, &mut any_extra, k.0, k.1);
    }
    for (k, _) in crate::annotations::entries_in(&m.links, req) {
        mark(&mut extra, &mut any_extra, k.0, k.1);
    }
    let local = tally_model::CellRangeLocal {
        r0: req.r0,
        c0: req.c0,
        r1: req.r1,
        c1: req.c1,
    };
    let mut rects: Vec<tally_model::CellRangeLocal> = Vec::new();
    for v in m.validations.iter().filter(|v| v.has_dropdown()) {
        for r in &v.ranges {
            let x = CellRange::new(req.sheet, r.r0, r.c0, r.r1, r.c1);
            if let Some(i) = x.intersect(req) {
                rects.push(tally_model::CellRangeLocal {
                    r0: i.r0,
                    c0: i.c0,
                    r1: i.r1,
                    c1: i.c1,
                });
            }
        }
    }
    if !m.cond_formats.is_empty() {
        rects.extend(eval.rects_in(&local));
    }
    rects.sort_unstable_by_key(|r| (r.r0, r.c0, r.r1, r.c1));
    rects.dedup();
    for r in rects {
        for row in r.r0..=r.r1 {
            for col in r.c0..=r.c1 {
                mark(&mut extra, &mut any_extra, row, col);
            }
        }
    }
    let first_new = views.len();
    if any_extra {
        for (i, flag) in extra.iter().enumerate() {
            if !*flag {
                continue;
            }
            let r = req.r0 + (i as u64 / cols) as u32;
            let c = req.c0 + (i as u64 % cols) as u32;
            if !have.contains(&(r, c)) {
                let addr = CellAddr::new(req.sheet, r, c);
                views.push(view_of(wb, addr, None, s.style(r, c)));
            }
        }
    }
    let has_dropdowns = m.validations.iter().any(|v| v.has_dropdown());
    for v in views.iter_mut() {
        let key = (v.r, v.c);
        if m.notes.contains_key(&key) {
            v.flags |= cell_flags::COMMENT;
        }
        if let Some(u) = m.links.get(&key) {
            v.flags |= cell_flags::LINK;
            v.link = Some(u.to_string());
        }
        if has_dropdowns
            && m.validations
                .iter()
                .any(|x| x.has_dropdown() && x.contains(v.r, v.c))
        {
            v.flags |= cell_flags::VALIDATION_DROPDOWN;
        }
        if m.cond_formats.is_empty() {
            continue;
        }
        let Some(d) = eval.decor(v.r, v.c, StyleId(v.style)) else {
            continue;
        };
        if let Some(id) = d.style {
            v.cf_style = Some(id);
            if d.number_format {
                if let Some(entry) = wb.transient.style(id) {
                    let val = wb.value(CellAddr::new(req.sheet, v.r, v.c));
                    if !val.is_empty() {
                        let f = format_value_with(
                            &val,
                            &entry.format,
                            wb.locale,
                            None,
                            wb.settings.date_epoch,
                        );
                        v.text = cap_view_text(f.text);
                        v.color = f.color.map(|c| (c << 8) | 0xff);
                        v.num = match val {
                            Value::Number(n) if entry.format.is_general() => Some(n),
                            _ => None,
                        };
                    }
                }
            }
        }
        v.cf_bar = d.bar;
        v.cf_icon = d.icon;
        if d.hide_value {
            v.text.clear();
        }
    }
    // Added views that nothing applied to are dropped again.
    let mut i = first_new;
    while i < views.len() {
        let v = &views[i];
        let plain =
            v.flags == 0 && v.cf_style.is_none() && v.cf_bar.is_none() && v.cf_icon.is_none();
        if plain && v.style == 0 && v.text.is_empty() {
            views.swap_remove(i);
        } else {
            i += 1;
        }
    }
}

/// Cell views for the requested ranges (non-empty or styled cells only).
pub fn cells(
    wb: &Workbook,
    version: u64,
    sheet: SheetId,
    ranges: &[RangeDto],
) -> Result<Vec<CellBlock>, ApiError> {
    cells_with_cf_budget(
        wb,
        version,
        sheet,
        ranges,
        crate::condfmt::MAX_CF_WORK_PER_CALL,
    )
}

/// [`cells`] with a custom conditional-format work budget (tests).
pub fn cells_with_cf_budget(
    wb: &Workbook,
    version: u64,
    sheet: SheetId,
    ranges: &[RangeDto],
    cf_budget: u64,
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
    let decorations = has_decorations(s);
    let eval = crate::condfmt::CfEval::with_budget(wb, sheet, version, cf_budget);
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
        let truncated_before = eval.truncated();
        if decorations {
            let req = CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1);
            decorate(wb, s, &eval, &req, &mut views);
        }
        views.sort_by_key(|v| (v.r, v.c));
        out.push(CellBlock {
            sheet,
            range: *r,
            version,
            cells: views,
            cf_truncated: (decorations && (truncated_before || eval.truncated())).then_some(true),
        });
    }
    Ok(out)
}

/// AOO value highlighting: constant numbers (and booleans).
pub const HIGHLIGHT_VALUE: u32 = 0x0000_ffff;
/// AOO value highlighting: formulas.
pub const HIGHLIGHT_FORMULA: u32 = 0x0080_00ff;
/// AOO value highlighting: text constants.
pub const HIGHLIGHT_TEXT: u32 = 0x0000_00ff;

/// Applies the View options of `cells_with` to blocks from [`cells`]:
/// formula text instead of results, value highlighting colours (RGBA) and
/// blank zero values.
pub fn apply_cells_options(
    wb: &Workbook,
    sheet: SheetId,
    blocks: &mut [CellBlock],
    opts: &tally_api::CellsOptions,
) {
    if !opts.show_formulas && !opts.value_highlighting && !opts.hide_zero_values {
        return;
    }
    let Some(s) = wb.sheet(sheet) else {
        return;
    };
    let protected = s.meta.protection.is_some();
    for b in blocks.iter_mut() {
        for v in &mut b.cells {
            let content = s.get(v.r, v.c);
            let formula_id = match content {
                Some(CellContent::Formula(id) | CellContent::ArrayPart(id)) => Some(*id),
                _ => None,
            };
            if opts.value_highlighting && v.kind != ValueKind::Empty {
                v.color = Some(if formula_id.is_some() {
                    HIGHLIGHT_FORMULA
                } else if v.kind == ValueKind::Text {
                    HIGHLIGHT_TEXT
                } else {
                    HIGHLIGHT_VALUE
                });
            }
            if opts.show_formulas {
                if let Some(id) = formula_id {
                    let hidden = protected
                        && crate::protection::cell_protection(wb, s.style(v.r, v.c)).hidden_formula;
                    if !hidden {
                        if let Some(text) = wb.formula_text(id) {
                            let array = wb.formulas.get(id).is_some_and(|f| f.array.is_some())
                                || matches!(content, Some(CellContent::ArrayPart(_)));
                            v.text =
                                cap_view_text(if array { format!("{{{text}}}") } else { text });
                            v.kind = ValueKind::Text;
                            v.num = None;
                            if !opts.value_highlighting {
                                v.color = None;
                            }
                            continue;
                        }
                    }
                }
            }
            if opts.hide_zero_values && v.kind == ValueKind::Number {
                let addr = CellAddr::new(sheet, v.r, v.c);
                if matches!(wb.value(addr), Value::Number(n) if n == 0.0) {
                    v.text.clear();
                    v.num = None;
                }
            }
        }
    }
}

/// The Function Wizard's live result: `text` evaluated at `(r, c)` and
/// formatted with that cell's number format; `None` when it does not
/// parse.
pub fn preview_formula(
    d: &crate::document::Document,
    sheet: SheetId,
    r: u32,
    c: u32,
    text: &str,
) -> Result<Option<String>, ApiError> {
    let wb = &d.wb;
    sheet_or_err(wb, sheet)?;
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    let t = text.trim();
    if t.is_empty() || t == "=" {
        return Ok(None);
    }
    let src = if t.starts_with('=') {
        t.to_owned()
    } else {
        format!("={t}")
    };
    let addr = CellAddr::new(sheet, r, c);
    let Some(v) = d.eval_formula_text_at(addr, &src) else {
        return Ok(None);
    };
    let v = match v {
        Value::Array(m) => m.get(0, 0).cloned().unwrap_or(Value::Empty),
        other => other,
    };
    if v.is_empty() {
        return Ok(Some(String::new()));
    }
    let st = wb.styles.get(wb.style_at(addr));
    Ok(Some(cap_view_text(format_with(wb, &v, st).text)))
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
            CellContent::ArrayPart(_) => match self.wb.value(CellAddr::new(sheet.id, r, c)) {
                Value::Text(t) => Some(Some(t.to_string())),
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
/// The input string of a cell as AOO's `ScCellFormat::GetInputString`
/// gives it: the edit text of the value (of the result, for a formula),
/// not the displayed text (used by the standard filter's text
/// conditions, correctness review 4, M3).
pub(crate) fn input_string(wb: &Workbook, addr: tally_types::CellAddr) -> String {
    let Some(s) = wb.sheet(addr.sheet) else {
        return String::new();
    };
    let v = wb.value(addr);
    if v.is_empty() {
        return String::new();
    }
    edit_text(wb, &v, wb.styles.get(s.style(addr.row, addr.col)))
}

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
                    _ => wb.locale.short_date.to_owned(),
                };
                let code = code.replacen("MM/DD/YYYY", wb.locale.short_date, 1);
                if let Ok(ast) = parse_format(&code) {
                    return format_value_with(v, &ast, wb.locale, None, wb.settings.date_epoch)
                        .text;
                }
            }
            if let Some(SectionKind::Number(l)) = fmt.sections.first().map(|s| &s.kind) {
                if l.percent > 0 {
                    return format!("{}%", general_string(n * 100.0, None, wb.locale));
                }
            }
            general_string(*n, None, wb.locale)
        }
        Value::Text(s) => {
            let ctx = RecognizeContext::now(wb.settings.date_epoch);
            let looks_like_value = s.starts_with('=')
                || s.starts_with('\'')
                || !matches!(
                    recognize_input_with(s, wb.locale, &ctx).value,
                    Value::Text(_)
                );
            if looks_like_value && !st.is_text_format() {
                format!("'{s}")
            } else {
                s.to_string()
            }
        }
        Value::Bool(b) => if *b {
            wb.locale.true_name
        } else {
            wb.locale.false_name
        }
        .to_owned(),
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
    let plain = |input: String| CellEditInfo {
        input,
        hint: None,
        array: None,
        array_range: None,
    };
    match s.get(r, c) {
        None => Ok(plain(String::new())),
        Some(CellContent::Value(v)) => Ok(plain(edit_text(wb, v, wb.styles.get(s.style(r, c))))),
        Some(CellContent::Formula(id) | CellContent::ArrayPart(id)) => {
            let Some(f) = wb.formulas.get(*id) else {
                return Ok(plain(String::new()));
            };
            let hidden = s.meta.protection.is_some()
                && crate::protection::cell_protection(wb, s.style(r, c)).hidden_formula;
            if hidden {
                return Ok(plain(String::new()));
            }
            let text = wb.formula_text(*id).unwrap_or_default();
            let hint = (f.invalid_text.is_none())
                .then(|| explain(&f.formula, f.addr, &HeaderLabels::new(wb)))
                .flatten();
            match f.array_range() {
                Some(b) => Ok(CellEditInfo {
                    input: format!("{{{text}}}"),
                    hint,
                    array: Some(true),
                    array_range: Some(range_dto(&b)),
                }),
                None => Ok(CellEditInfo {
                    input: text,
                    hint,
                    array: None,
                    array_range: None,
                }),
            }
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
            for (row, content) in col.cells.iter_range(r.r0, r.r1) {
                let v = match content {
                    CellContent::Value(v) => v.clone(),
                    CellContent::Formula(id) => wb
                        .formulas
                        .get(*id)
                        .map_or(Value::Empty, |f| f.cached.clone()),
                    CellContent::ArrayPart(_) => wb.value(CellAddr::new(sel.sheet, row, c)),
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
        general_string(sum, None, wb.locale)
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

/// Name box text → selection: a reference (optionally sheet-qualified) or
/// a named range (sheet-scoped names of the active sheet first).
pub fn resolve_name_box(wb: &Workbook, text: &str) -> Result<SelectionDto, ApiError> {
    resolve_name_box_in(wb, wb.active_sheet, text)
}

/// [`resolve_name_box`] relative to `current` instead of the active sheet.
pub fn resolve_name_box_in(
    wb: &Workbook,
    current: SheetId,
    text: &str,
) -> Result<SelectionDto, ApiError> {
    sheet_or_err(wb, current)?;
    let t = text.trim();
    if t.is_empty() {
        return Err(ApiError::invalid_argument("empty reference"));
    }
    if let Some(id) = wb.names.visible(t, current) {
        let def = wb.names.get(id).and_then(|e| e.def.as_ref());
        let range = def.and_then(|_| wb.parse_reference_at(t, CellAddr::new(current, 0, 0)));
        return match range {
            Some(range) => Ok(SelectionDto {
                sheet: range.sheet,
                ranges: vec![range_dto(&range)],
                active: CellPos {
                    r: range.r0,
                    c: range.c0,
                },
            }),
            None => Err(ApiError::not_found(format!(
                "the name {t:?} does not refer to a cell range"
            ))),
        };
    }
    let (name, rest) =
        split_sheet(t).ok_or_else(|| ApiError::invalid_argument(format!("bad reference {t:?}")))?;
    let sheet = match name {
        Some(n) => wb
            .sheet_by_name_ci(&n)
            .ok_or_else(|| ApiError::not_found(format!("no sheet named {n:?}")))?,
        None => current,
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
