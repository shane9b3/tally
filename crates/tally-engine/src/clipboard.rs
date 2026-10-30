//! The internal clipboard (plan §4.3): `copy` stores a snapshot of the
//! selection (contents with relative formulas, and styles) under a random
//! token and returns TSV and HTML renderings; pasting that token re-anchors
//! the formulas at the target (the AST is relative, so the same formula is
//! simply placed at the new address) and carries the styles, the
//! conditional formats and validations covering the block (AOO
//! `IDF_ATTRIB`: they are cell attributes) and the notes (`IDF_NOTE`),
//! all re-anchored at the target (review 3, M4). Only the last snapshot
//! is kept.

use std::sync::Arc;

use tally_api::{ApiError, PasteOperation, PasteOptions, PastePayload};
use tally_model::{CellRangeLocal, StyleData};
use tally_types::{CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::ops::{block_range, check_array_blocks, check_merges, write_inputs, MAX_OP_CELLS};
use crate::storage::{CellContent, ColumnStore, StyleId};
use crate::undo::{
    capture_range, clear_range_content, clear_range_styles, meta_parts, Change, Effects,
};
use crate::workbook::{Rng, SnapContent, Workbook};

/// A copied block.
#[derive(Clone, Debug)]
pub struct ClipSnapshot {
    /// Token embedded in the HTML (`data-tally-clip`).
    pub token: String,
    /// Rows of the block.
    pub rows: u32,
    /// Columns of the block.
    pub cols: u32,
    /// Non-empty or styled cells.
    pub cells: Vec<ClipCell>,
    /// Styles referenced by `cells`.
    pub styles: Vec<StyleData>,
    /// Merged areas inside the block, relative to its top-left.
    pub merges: Vec<CellRangeLocal>,
    /// Conditional formats intersecting the block: the intersecting
    /// ranges relative to the block, the origin relative to the block
    /// (may be negative) and the rules.
    pub cond_formats: Vec<ClipRules<crate::condfmt::CondFormat>>,
    /// Validations intersecting the block, likewise.
    pub validations: Vec<ClipRules<crate::validation::Validation>>,
    /// Notes inside the block `(dr, dc, note)`.
    pub notes: Vec<(u32, u32, crate::annotations::Note)>,
    /// The copied block's top-left (sheet and cell), for Paste Special ▸
    /// Link.
    pub source: CellAddr,
    /// The document copied from (set by the service), for Link.
    pub source_doc: Option<tally_api::DocId>,
}

/// A conditional format or validation as copied: its ranges clipped to
/// the block and made relative, plus the item itself (whose own ranges
/// and origin are rewritten on paste).
#[derive(Clone, Debug)]
pub struct ClipRules<T> {
    /// Ranges relative to the block's top-left.
    pub ranges: Vec<CellRangeLocal>,
    /// Origin relative to the block's top-left.
    pub origin: (i64, i64),
    /// The item (ranges/origin as in the source sheet).
    pub item: T,
}

fn clip_rules<T: Clone>(
    range: &CellRange,
    ranges: &[CellRangeLocal],
    origin: CellAddr,
    item: &T,
) -> Option<ClipRules<T>> {
    let block = local(range);
    let rel: Vec<CellRangeLocal> = ranges
        .iter()
        .filter_map(|r| crate::condfmt::intersect_rect(r, &block))
        .map(|r| CellRangeLocal {
            r0: r.r0 - range.r0,
            c0: r.c0 - range.c0,
            r1: r.r1 - range.r0,
            c1: r.c1 - range.c0,
        })
        .collect();
    if rel.is_empty() {
        return None;
    }
    Some(ClipRules {
        ranges: rel,
        origin: (
            i64::from(origin.row) - i64::from(range.r0),
            i64::from(origin.col) - i64::from(range.c0),
        ),
        item: item.clone(),
    })
}

fn local(r: &CellRange) -> CellRangeLocal {
    CellRangeLocal {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

/// One copied cell.
#[derive(Clone, Debug)]
pub struct ClipCell {
    /// Row offset.
    pub dr: u32,
    /// Column offset.
    pub dc: u32,
    /// Content (formulas relative; array anchors whose block was copied
    /// whole keep the block; other array cells become their values).
    pub content: Option<SnapContent>,
    /// Index into [`ClipSnapshot::styles`].
    pub style: Option<usize>,
    /// The value at copy time (for "values only").
    pub value: Value,
}

/// Copies `range` of `wb`. Returns the snapshot and its TSV/HTML text.
pub fn copy_range(
    wb: &Workbook,
    range: CellRange,
    rng: &mut Rng,
) -> (ClipSnapshot, String, String) {
    let token = format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64());
    let mut cells = Vec::new();
    let mut styles: Vec<StyleData> = Vec::new();
    let mut style_idx: Vec<(StyleId, usize)> = Vec::new();
    if let Some(sheet) = wb.sheet(range.sheet) {
        for c in range.c0..=range.c1.min(sheet.columns.len().saturating_sub(1) as u32) {
            let Some(col) = sheet.column(c) else {
                continue;
            };
            let mut rows: Vec<u32> = col
                .cells
                .iter_range(range.r0, range.r1)
                .map(|(r, _)| r)
                .collect();
            for (s, e, _) in col.styles.runs_in(range.r0, range.r1) {
                rows.extend(s..=e);
            }
            rows.sort_unstable();
            rows.dedup();
            for r in rows {
                let addr = CellAddr::new(range.sheet, r, c);
                let value = wb.value(addr);
                let content = match wb.array_at(addr) {
                    Some((_, f)) if f.array_range().is_some_and(|b| !range.contains_range(&b)) => {
                        (!value.is_empty()).then(|| SnapContent::Value(value.clone()))
                    }
                    _ => wb.snapshot_content(addr),
                };
                let st = col.styles.get(r);
                let si = (st != StyleId::DEFAULT).then(|| {
                    if let Some((_, i)) = style_idx.iter().find(|(s, _)| *s == st) {
                        *i
                    } else {
                        styles.push(wb.styles.get(st).data.clone());
                        style_idx.push((st, styles.len() - 1));
                        styles.len() - 1
                    }
                });
                cells.push(ClipCell {
                    dr: r - range.r0,
                    dc: c - range.c0,
                    content,
                    style: si,
                    value,
                });
            }
        }
    }
    let merges = wb
        .sheet(range.sheet)
        .map(|s| {
            s.meta
                .merges
                .iter()
                .filter(|m| {
                    range.r0 <= m.r0 && m.r1 <= range.r1 && range.c0 <= m.c0 && m.c1 <= range.c1
                })
                .map(|m| CellRangeLocal {
                    r0: m.r0 - range.r0,
                    c0: m.c0 - range.c0,
                    r1: m.r1 - range.r0,
                    c1: m.c1 - range.c0,
                })
                .collect()
        })
        .unwrap_or_default();
    let (cond_formats, validations, notes) = wb
        .sheet(range.sheet)
        .map(|s| {
            let cf: Vec<_> = s
                .meta
                .cond_formats
                .iter()
                .filter_map(|cf| clip_rules(&range, &cf.ranges, cf.origin, &**cf))
                .collect();
            let va: Vec<_> = s
                .meta
                .validations
                .iter()
                .filter_map(|v| clip_rules(&range, &v.ranges, v.origin, &**v))
                .collect();
            let notes: Vec<_> = crate::annotations::entries_in(&s.meta.notes, &range)
                .map(|((r, c), n)| (r - range.r0, c - range.c0, n.clone()))
                .collect();
            (cf, va, notes)
        })
        .unwrap_or_default();
    let tsv = render_tsv(wb, range);
    let html = render_html(wb, range, &token);
    (
        ClipSnapshot {
            token,
            rows: range.rows(),
            cols: range.cols(),
            cells,
            styles,
            merges,
            cond_formats,
            validations,
            notes,
            source: CellAddr::new(range.sheet, range.r0, range.c0),
            source_doc: None,
        },
        tsv,
        html,
    )
}

fn display(wb: &Workbook, addr: CellAddr) -> String {
    crate::service::view::display_text(wb, addr)
}

fn render_tsv(wb: &Workbook, range: CellRange) -> String {
    let mut out = String::new();
    for r in range.r0..=range.r1 {
        let mut line: Vec<String> = Vec::with_capacity(range.cols() as usize);
        for c in range.c0..=range.c1 {
            line.push(display(wb, CellAddr::new(range.sheet, r, c)));
        }
        out.push_str(line.join("\t").trim_end_matches('\t'));
        out.push('\n');
    }
    out
}

/// Escapes text for HTML.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn render_html(wb: &Workbook, range: CellRange, token: &str) -> String {
    let mut out = format!("<table data-tally-clip=\"{}\">", escape_html(token));
    for r in range.r0..=range.r1 {
        out.push_str("<tr>");
        for c in range.c0..=range.c1 {
            let addr = CellAddr::new(range.sheet, r, c);
            let st = wb.styles.get(wb.style_at(addr));
            let mut css = String::new();
            if st.data.font.bold {
                css.push_str("font-weight:bold;");
            }
            if st.data.font.italic {
                css.push_str("font-style:italic;");
            }
            if css.is_empty() {
                out.push_str("<td>");
            } else {
                out.push_str(&format!("<td style=\"{css}\">"));
            }
            out.push_str(&escape_html(&display(wb, addr)));
            out.push_str("</td>");
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

/// Pastes `payload` at `target` (the top-left, or a multiple of the block
/// size to tile) with `opts`. TSV supports `values_only` (`=…` stays text;
/// also when `selection` leaves out formulas), `transpose`, `skip_empty`
/// and `shift`; internal payloads support every option. With `shift`, cells
/// are first inserted for the pasted block (AOO `PasteFromClip` with
/// `eMoveMode`: `ScDocFunc::InsertCells`), in the same undo step.
pub fn paste(
    wb: &mut Workbook,
    sheet: SheetId,
    target: CellRange,
    payload: &PastePayload,
    opts: &PasteOptions,
    clip: Option<&ClipSnapshot>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    if opts.link == Some(true) && matches!(payload, PastePayload::Tsv { .. }) {
        return Err(ApiError::invalid_argument(
            "Link needs cells copied from this document",
        ));
    }
    let mut changes = Vec::new();
    if let Some(dir) = opts.shift {
        if !matches!(dir, tally_api::ShiftDir::Down | tally_api::ShiftDir::Right) {
            return Err(ApiError::invalid_argument(
                "Paste Special shifts cells down or right",
            ));
        }
        let Some(area) = paste_area(sheet, target, payload, opts, clip)? else {
            return Ok(changes);
        };
        let dto = crate::document::range_dto(&area);
        changes.extend(crate::tools::shift_range(wb, sheet, &dto, dir, fx)?);
    }
    match paste_unshifted(wb, sheet, target, payload, opts, clip, fx) {
        Ok(c) => {
            changes.extend(c);
            Ok(changes)
        }
        Err(e) => {
            crate::ops::revert(wb, changes, fx);
            Err(e)
        }
    }
}

/// Refuses a Paste Special ▸ Link of a block copied from another document
/// (AOO would make a DDE link; Tally has none).
pub fn check_link_source(
    op: &tally_api::Op,
    clip: Option<&ClipSnapshot>,
    doc: tally_api::DocId,
) -> Result<(), ApiError> {
    if let tally_api::Op::PasteSpecial {
        payload: PastePayload::Internal { token },
        options,
        ..
    } = op
    {
        if options.link == Some(true) {
            if let Some(c) = clip.filter(|c| c.token == *token) {
                if c.source_doc.is_some_and(|d| d != doc) {
                    return Err(ApiError::invalid_argument(
                        "Link needs cells copied from this document",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn paste_unshifted(
    wb: &mut Workbook,
    sheet: SheetId,
    target: CellRange,
    payload: &PastePayload,
    opts: &PasteOptions,
    clip: Option<&ClipSnapshot>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    // AOO tests the whole area for cell protection whatever is pasted
    // (`ScEditableTester` in `PasteFromClip`, viewfun3.cxx:1258), so a
    // Notes- or Formats-only paste is refused too.
    if let Some(area) = paste_area(sheet, target, payload, opts, clip)? {
        crate::protection::check_edit(wb, &[area])?;
    }
    match payload {
        PastePayload::Tsv { text } => {
            let mut rows = parse_tsv(text);
            if rows.is_empty() {
                return Ok(Vec::new());
            }
            if opts.transpose {
                rows = transpose_rows(&rows);
            }
            let values_only = opts.values_only || opts.selection.is_some_and(|s| !s.formulas);
            let range = block_range(sheet, target.r0, target.c0, &rows)?;
            check_array_blocks(wb, &[range])?;
            let inv = capture_range(wb, range, true, true);
            if opts.skip_empty {
                for (dr, row) in rows.iter().enumerate() {
                    for (dc, field) in row.iter().enumerate() {
                        if field.is_empty() {
                            continue;
                        }
                        write_inputs(
                            wb,
                            sheet,
                            target.r0 + dr as u32,
                            target.c0 + dc as u32,
                            &[vec![field.clone()]],
                            values_only,
                        );
                    }
                }
            } else {
                write_inputs(wb, sheet, target.r0, target.c0, &rows, values_only);
            }
            Ok(vec![Change::Range(inv)])
        }
        PastePayload::Internal { token } => {
            let clip = clip
                .filter(|c| c.token == *token)
                .ok_or_else(|| ApiError::not_found("the clipboard token is no longer valid"))?;
            paste_internal(wb, sheet, target, clip, opts, fx)
        }
    }
}

/// The cells [`paste`] writes for `payload` at `target`: a TSV block at the
/// top-left (never tiled), an internal block tiled over a target that is a
/// whole multiple of its size.
pub fn paste_area(
    sheet: SheetId,
    target: CellRange,
    payload: &PastePayload,
    opts: &PasteOptions,
    clip: Option<&ClipSnapshot>,
) -> Result<Option<CellRange>, ApiError> {
    match payload {
        PastePayload::Tsv { text } => {
            let mut rows = parse_tsv(text);
            if rows.is_empty() {
                return Ok(None);
            }
            if opts.transpose {
                rows = transpose_rows(&rows);
            }
            block_range(sheet, target.r0, target.c0, &rows).map(Some)
        }
        PastePayload::Internal { token } => {
            let clip = clip
                .filter(|c| c.token == *token)
                .ok_or_else(|| ApiError::not_found("the clipboard token is no longer valid"))?;
            let (crows, ccols) = (clip.rows.max(1), clip.cols.max(1));
            let (rows, cols) = if opts.transpose {
                (ccols, crows)
            } else {
                (crows, ccols)
            };
            let t = Tiling::of(target, rows, cols);
            let (r1, c1) = t.ends(target);
            Ok(Some(CellRange::new(sheet, target.r0, target.c0, r1, c1)))
        }
    }
}

/// How a `rows` × `cols` block covers `target` (AOO `CopyFromClip`): a
/// selection larger than the block in a direction is filled with copies
/// of it, the last one cut at the selection's edge; otherwise the block is
/// pasted once (beyond a smaller selection).
#[derive(Clone, Copy, Debug)]
struct Tiling {
    rows: u32,
    cols: u32,
    reps_r: u32,
    reps_c: u32,
}

impl Tiling {
    fn of(target: CellRange, rows: u32, cols: u32) -> Self {
        let rows_out = target.rows().max(rows);
        let cols_out = target.cols().max(cols);
        Tiling {
            rows: rows_out,
            cols: cols_out,
            reps_r: rows_out.div_ceil(rows),
            reps_c: cols_out.div_ceil(cols),
        }
    }

    /// The last row and column of the pasted area (clamped to the sheet).
    fn ends(&self, target: CellRange) -> (u32, u32) {
        let r1 = (u64::from(target.r0) + u64::from(self.rows) - 1).min(u64::from(MAX_ROWS - 1));
        let c1 = (u64::from(target.c0) + u64::from(self.cols) - 1).min(u64::from(MAX_COLS - 1));
        (r1 as u32, c1 as u32)
    }
}

fn transpose_rows(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let w = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..w)
        .map(|c| {
            rows.iter()
                .map(|r| r.get(c).cloned().unwrap_or_default())
                .collect()
        })
        .collect()
}

fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Empty => Some(0.0),
        _ => None,
    }
}

fn combine(op: PasteOperation, t: f64, s: f64) -> Value {
    let r = match op {
        PasteOperation::None => s,
        PasteOperation::Add => t + s,
        PasteOperation::Subtract => t - s,
        PasteOperation::Multiply => t * s,
        PasteOperation::Divide => {
            if s == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            t / s
        }
    };
    if r.is_finite() {
        Value::Number(r)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

fn op_symbol(op: PasteOperation) -> &'static str {
    match op {
        PasteOperation::None => "",
        PasteOperation::Add => "+",
        PasteOperation::Subtract => "-",
        PasteOperation::Multiply => "*",
        PasteOperation::Divide => "/",
    }
}

/// The effective Paste Special selection (AOO's `IDF_*` insert flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PasteFlags {
    /// "Paste all" (`IDF_ALL`): Link then also references empty cells.
    pub all: bool,
    pub text: bool,
    pub numbers: bool,
    pub date_time: bool,
    pub formulas: bool,
    pub notes: bool,
    pub formats: bool,
}

impl PasteFlags {
    /// `selection` wins; else the older `values_only` (text, numbers and
    /// dates: formula results as values) and `formats_only`; else all.
    pub(crate) fn of(o: &PasteOptions) -> Self {
        let kinds = |text, numbers, date_time, formulas, notes, formats| PasteFlags {
            all: false,
            text,
            numbers,
            date_time,
            formulas,
            notes,
            formats,
        };
        if let Some(s) = o.selection {
            kinds(
                s.text,
                s.numbers,
                s.date_time,
                s.formulas,
                s.notes,
                s.formats,
            )
        } else if o.values_only {
            kinds(true, true, true, false, false, false)
        } else if o.formats_only {
            kinds(false, false, false, false, false, true)
        } else {
            PasteFlags {
                all: true,
                ..kinds(true, true, true, true, true, true)
            }
        }
    }

    /// Any cell content kind (`IDF_CONTENTS` without the notes).
    fn cells(&self) -> bool {
        self.text || self.numbers || self.date_time || self.formulas
    }

    /// AOO `lclCanCloneValue`: whether a number with a date/time format
    /// (`date`) or another format is pasted.
    fn number(&self, date: bool) -> bool {
        if self.numbers == self.date_time {
            self.numbers
        } else if self.date_time {
            date
        } else {
            !date
        }
    }

    /// Whether a source constant of this kind is selected (AOO
    /// `ScColumn::CreateRefCell` / `CloneCell` for value and string cells).
    fn constant(&self, v: &Value, date: bool) -> bool {
        match v {
            Value::Empty => false,
            Value::Text(_) => self.text,
            Value::Number(_) | Value::Bool(_) => self.number(date),
            _ => self.text || self.numbers,
        }
    }
}

/// What lands in one target cell, before an operation is applied.
fn pasted_content(
    cell: &ClipCell,
    f: &PasteFlags,
    date: bool,
    transpose: bool,
    wb: &Workbook,
) -> Option<SnapContent> {
    // Array parts copied with their whole block have no content of their
    // own (the anchor writes them) but carry their value.
    let is_formula = matches!(cell.content, Some(SnapContent::Formula { .. }))
        || (cell.content.is_none() && !cell.value.is_empty());
    if !is_formula {
        let content = cell.content.as_ref()?;
        let SnapContent::Value(v) = content else {
            return None;
        };
        return f.constant(v, date).then(|| content.clone());
    }
    if f.formulas {
        let value = || (!cell.value.is_empty()).then(|| SnapContent::Value(cell.value.clone()));
        return match &cell.content {
            // A transposed array is pasted as values (its block cannot
            // turn); otherwise array parts are written by their anchor.
            None if transpose => value(),
            None => None,
            Some(c) if transpose && is_array(&Some(c.clone())) => value(),
            Some(c) => Some(without_missing_sheets(
                wb,
                if transpose { transposed(c) } else { c.clone() },
            )),
        };
    }
    // Formulas off: the result as a value, by its kind (AOO `CloneCell`:
    // errors go with Numbers, empty texts are not pasted).
    let ok = match &cell.value {
        Value::Empty => false,
        Value::Text(t) => f.text && !t.is_empty(),
        Value::Number(_) | Value::Bool(_) => f.number(date),
        _ => f.numbers,
    };
    ok.then(|| SnapContent::Value(cell.value.clone()))
}

/// A reference to source cell `src` (AOO `ScColumn::CreateRefCell`: every
/// part absolute, sheet shown: `=$Sheet1.$A$1`).
fn link_formula(wb: &Workbook, src: CellAddr) -> SnapContent {
    use tally_formula::{CellRefExpr, Expr, Formula, RefCoord, RefExpr, SheetRef};
    let sheet = if wb.sheet(src.sheet).is_some() {
        SheetRef::Id {
            id: src.sheet,
            abs: true,
        }
    } else {
        SheetRef::Deleted
    };
    let r = Expr::Ref(RefExpr::Cell(CellRefExpr {
        sheet,
        row: RefCoord::absolute(src.row as i32),
        col: RefCoord::absolute(src.col as i32),
    }));
    SnapContent::formula(Arc::new(Formula::new(r)), None)
}

/// A target cell's content as it was before the paste (for operations).
#[derive(Clone, Debug)]
pub(crate) enum Orig {
    Empty,
    /// A number or boolean constant: its number and value.
    Number(f64, Value),
    /// Text, errors and array cells: never combined.
    Other(SnapContent),
    /// A formula: its text (`=…`) and content.
    Formula(String, SnapContent),
}

fn orig_of(wb: &Workbook, addr: CellAddr) -> Orig {
    match wb.sheet(addr.sheet).and_then(|s| s.get(addr.row, addr.col)) {
        None => Orig::Empty,
        Some(CellContent::Value(v)) => match number_of(v) {
            Some(n) => Orig::Number(n, v.clone()),
            _ => Orig::Other(SnapContent::Value(v.clone())),
        },
        Some(CellContent::ArrayPart(_)) => Orig::Other(SnapContent::Value(wb.value(addr))),
        Some(CellContent::Formula(id)) => {
            let content = wb.snapshot_content(addr);
            let plain = wb
                .formulas
                .get(*id)
                .is_some_and(|f| f.array.is_none() && f.invalid_text.is_none());
            match (plain, wb.formula_text(*id), content) {
                (true, Some(t), Some(c)) => Orig::Formula(t, c),
                (_, _, Some(c)) => Orig::Other(c),
                _ => Orig::Empty,
            }
        }
    }
}

/// AOO `ScColumn::MixData` for one cell: `orig` (the target before the
/// paste) combined with what was pasted there by `op`. Both numbers (empty
/// counts as 0): the result, or `#VALUE!` on overflow or division by
/// zero. Either text: the original stays. Otherwise (a formula on either
/// side) the formula `=(orig) op (pasted)`.
fn mix_cell(wb: &mut Workbook, addr: CellAddr, orig: &Orig, op: PasteOperation) {
    let dest = orig_of(wb, addr);
    let restore = |wb: &mut Workbook, o: &Orig| match o {
        Orig::Empty => wb.restore_content(addr, None),
        Orig::Number(_, v) => wb.set_value(addr, v.clone()),
        Orig::Other(c) | Orig::Formula(_, c) => wb.restore_content(addr, Some(c.clone())),
    };
    match (orig, &dest) {
        (Orig::Empty, Orig::Empty) => {}
        (Orig::Empty | Orig::Number(..), Orig::Empty | Orig::Number(..)) => {
            let a = if let Orig::Number(n, _) = orig {
                *n
            } else {
                0.0
            };
            let b = if let Orig::Number(n, _) = dest {
                n
            } else {
                0.0
            };
            let v = match combine(op, a, b) {
                Value::Error(_) => Value::Error(ErrorKind::Value),
                v => v,
            };
            wb.set_value(addr, v);
        }
        (Orig::Other(_), _) | (_, Orig::Other(_)) => restore(wb, orig),
        _ => {
            let side = |o: &Orig| match o {
                Orig::Formula(t, _) => format!("({})", t.strip_prefix('=').unwrap_or(t)),
                // Shortest round-trip form: no digits are lost.
                Orig::Number(n, _) => format!("{n}"),
                _ => "0".to_owned(),
            };
            let text = format!("={}{}{}", side(orig), op_symbol(op), side(&dest));
            if let Ok(nf) = wb.parse_formula(&text, addr) {
                wb.set_formula(addr, Arc::new(nf), None);
            }
        }
    }
}

/// The non-empty cells of `area` as they are (AOO's mix document).
pub(crate) fn originals(wb: &Workbook, area: CellRange) -> Vec<(CellAddr, Orig)> {
    let mut out = Vec::new();
    if let Some(s) = wb.sheet(area.sheet) {
        for c in area.c0..=area.c1.min(s.columns.len().saturating_sub(1) as u32) {
            if let Some(col) = s.column(c) {
                for (r, _) in col.cells.iter_range(area.r0, area.r1) {
                    out.push(CellAddr::new(area.sheet, r, c));
                }
            }
        }
    }
    out.into_iter().map(|a| (a, orig_of(wb, a))).collect()
}

/// AOO `MixData` over a block: every original cell and every cell written
/// by the paste (`written`); with `skip_empty` only the written ones (the
/// others keep their contents).
pub(crate) fn mix_all(
    wb: &mut Workbook,
    origs: Vec<(CellAddr, Orig)>,
    mut written: std::collections::HashSet<CellAddr>,
    skip_empty: bool,
    op: PasteOperation,
) {
    for (addr, orig) in &origs {
        if skip_empty && !written.contains(addr) {
            continue;
        }
        written.remove(addr);
        mix_cell(wb, *addr, orig, op);
    }
    let mut rest: Vec<CellAddr> = written.into_iter().collect();
    rest.sort_by_key(|a| (a.col, a.row));
    for addr in rest {
        mix_cell(wb, addr, &Orig::Empty, op);
    }
}

/// Internal paste (AOO `ScViewFunc::PasteFromClip` → `ScDocument::
/// CopyFromClip` / `MixDocument`): formulas re-anchored (the AST is
/// relative), styles and merges carried, by the Paste Special options.
///
/// - Selection: unselected kinds are not pasted; with Formulas off,
///   formula cells paste their results by kind (`CloneCell`).
/// - Without Skip empty cells, the target block is first cleared of all
///   contents and notes when any cell kind is pasted, of its notes when
///   only notes are, and of its formats when formats are (`nDelFlag`).
///   With it, cells where nothing is pasted keep their contents, and
///   formats come only from non-empty source cells.
/// - An operation combines the target as it was with what was pasted
///   ([`mix_cell`]).
/// - Link: references to the source cells instead of their contents, for
///   the selected kinds (every cell of the block with Paste all).
pub fn paste_internal(
    wb: &mut Workbook,
    sheet: SheetId,
    target: CellRange,
    clip: &ClipSnapshot,
    opts: &PasteOptions,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let (crows, ccols) = (clip.rows.max(1), clip.cols.max(1));
    let (rows, cols) = if opts.transpose {
        (ccols, crows)
    } else {
        (crows, ccols)
    };
    let tiling = Tiling::of(target, rows, cols);
    let (reps_r, reps_c) = (tiling.reps_r, tiling.reps_c);
    if u64::from(tiling.rows) * u64::from(tiling.cols) > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("paste area is too large"));
    }
    let (r1, c1) = tiling.ends(target);
    let area = CellRange::new(sheet, target.r0, target.c0, r1, c1);
    let f = PasteFlags::of(opts);
    let link = opts.link == Some(true);
    let cells = f.cells();
    let contents = cells || f.notes;
    let formats = f.formats;
    // Notes are not linked (a reference cell has none).
    let paste_notes = f.notes && !link;
    let arithmetic = opts.operation != PasteOperation::None && cells;
    if cells {
        check_array_blocks(wb, &[area])?;
    }
    // Merges are carried with the formats.
    let carry_merges = formats && (!clip.merges.is_empty() || area_has_merges(wb, &area));
    if carry_merges {
        check_merges(wb, &area)?;
    }
    let mut changes = Vec::new();
    if carry_merges {
        changes.extend(meta_parts(wb, sheet, &[crate::storage::MetaKind::Merges]));
    }
    // Conditional formats and validations are attributes: pasted with the
    // formats (not transposed).
    let carry_rules = formats
        && !opts.transpose
        && (!clip.cond_formats.is_empty()
            || !clip.validations.is_empty()
            || wb.sheet(sheet).is_some_and(|s| {
                let l = local(&area);
                s.meta.cond_formats.iter().any(|cf| {
                    cf.ranges
                        .iter()
                        .any(|r| crate::condfmt::intersect_rect(r, &l).is_some())
                }) || s.meta.validations.iter().any(|v| {
                    v.ranges
                        .iter()
                        .any(|r| crate::condfmt::intersect_rect(r, &l).is_some())
                })
            }));
    if carry_rules {
        changes.extend(meta_parts(
            wb,
            sheet,
            &[
                crate::storage::MetaKind::CondFormats,
                crate::storage::MetaKind::Validations,
            ],
        ));
    }
    // Without Skip empty, any content kind clears the target's notes
    // (AOO `IDF_CONTENTS` includes `IDF_NOTE`).
    let clear_notes = contents && !opts.skip_empty;
    let carry_notes = (paste_notes && !clip.notes.is_empty())
        || (clear_notes
            && wb.sheet(sheet).is_some_and(|s| {
                crate::annotations::entries_in(&s.meta.notes, &area)
                    .next()
                    .is_some()
            }));
    if carry_notes {
        changes.extend(meta_parts(wb, sheet, &[crate::storage::MetaKind::Notes]));
    }
    changes.push(Change::Range(capture_range(wb, area, cells, formats)));
    let place = |dr: u32, dc: u32| if opts.transpose { (dc, dr) } else { (dr, dc) };
    // (target, offset in the source block, source cell).
    let mut writes: Vec<(CellAddr, (u32, u32), Option<&ClipCell>)> = Vec::new();
    let mut by_pos: std::collections::HashMap<(u32, u32), &ClipCell> =
        std::collections::HashMap::new();
    if link && f.all {
        by_pos.extend(clip.cells.iter().map(|c| ((c.dr, c.dc), c)));
    }
    for rr in 0..reps_r {
        for rc in 0..reps_c {
            let at = |dr: u32, dc: u32| {
                let (pr, pc) = place(dr, dc);
                let row = u64::from(target.r0) + u64::from(rr * rows + pr);
                let col = u64::from(target.c0) + u64::from(rc * cols + pc);
                (row <= u64::from(r1) && col <= u64::from(c1))
                    .then(|| CellAddr::new(sheet, row as u32, col as u32))
            };
            if link && f.all {
                // AOO: with "Paste all", empty cells are referenced too.
                for dr in 0..crows {
                    for dc in 0..ccols {
                        if let Some(a) = at(dr, dc) {
                            writes.push((a, (dr, dc), by_pos.get(&(dr, dc)).copied()));
                        }
                    }
                }
            } else {
                for cell in &clip.cells {
                    if let Some(a) = at(cell.dr, cell.dc) {
                        writes.push((a, (cell.dr, cell.dc), Some(cell)));
                    }
                }
            }
        }
    }
    // The target as it was, for an operation (AOO's mix document).
    let origs = if arithmetic {
        originals(wb, area)
    } else {
        Vec::new()
    };
    if cells && !opts.skip_empty {
        clear_range_content(wb, area);
    }
    if formats && !opts.skip_empty {
        clear_range_styles(wb, area);
    }
    if carry_merges {
        if let Some(s) = wb.sheet_mut(sheet) {
            let merges = std::sync::Arc::make_mut(&mut s.meta.merges);
            merges.retain(|m| {
                !(area.r0 <= m.r0 && m.r1 <= area.r1 && area.c0 <= m.c0 && m.c1 <= area.c1)
            });
            for rr in 0..reps_r {
                for rc in 0..reps_c {
                    for m in &clip.merges {
                        let (a0, b0) = place(m.r0, m.c0);
                        let (a1, b1) = place(m.r1, m.c1);
                        let n = CellRangeLocal {
                            r0: target.r0 + rr * rows + a0,
                            c0: target.c0 + rc * cols + b0,
                            r1: target.r0 + rr * rows + a1,
                            c1: target.c0 + rc * cols + b1,
                        };
                        if n.r1 <= area.r1 && n.c1 <= area.c1 {
                            merges.push(n);
                        }
                    }
                }
            }
        }
        fx.layout(sheet);
    }
    let style_ids: Vec<StyleId> = clip
        .styles
        .iter()
        .map(|s| wb.styles.intern(s.clone()))
        .collect();
    let is_date = |wb: &Workbook, cell: &ClipCell| {
        cell.style
            .and_then(|i| style_ids.get(i))
            .is_some_and(|st| wb.styles.get(*st).format.is_date())
    };
    // Targets written with an operation pending (AOO mixes every cell of
    // the block where either side has content).
    let mut written: std::collections::HashSet<CellAddr> = std::collections::HashSet::new();
    for (addr, (dr, dc), cell) in writes {
        let source_empty = cell.is_none_or(|c| c.value.is_empty() && c.content.is_none());
        if cells {
            let content = match cell {
                _ if link && f.all => Some(link_formula(
                    wb,
                    CellAddr::new(
                        clip.source.sheet,
                        clip.source.row + dr,
                        clip.source.col + dc,
                    ),
                )),
                None => None,
                Some(cell) if link => {
                    let date = is_date(wb, cell);
                    let kind_on = match &cell.content {
                        Some(SnapContent::Value(v)) => f.constant(v, date),
                        Some(SnapContent::Formula { .. }) => f.formulas,
                        None => f.formulas && !cell.value.is_empty(),
                    };
                    kind_on.then(|| {
                        link_formula(
                            wb,
                            CellAddr::new(
                                clip.source.sheet,
                                clip.source.row + dr,
                                clip.source.col + dc,
                            ),
                        )
                    })
                }
                Some(cell) => {
                    let date = is_date(wb, cell);
                    pasted_content(cell, &f, date, opts.transpose, wb)
                }
            };
            if let Some(content) = content {
                wb.restore_content(addr, Some(content));
                if arithmetic {
                    written.insert(addr);
                }
            }
        }
        if formats {
            let style = cell.and_then(|c| c.style).and_then(|i| style_ids.get(i));
            // Skip empty: attributes only from non-empty source cells
            // (AOO `bSkipAttrForEmpty`).
            let set = if opts.skip_empty {
                (!source_empty).then(|| style.copied().unwrap_or(StyleId::DEFAULT))
            } else {
                style.copied()
            };
            if let Some(st) = set {
                if let Some(c) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(addr.col)) {
                    c.styles.set(addr.row, addr.row, st);
                }
            }
        }
    }
    if arithmetic {
        mix_all(wb, origs, written, opts.skip_empty, opts.operation);
    }
    if carry_rules || carry_notes {
        if let Some(s) = wb.sheet_mut(sheet) {
            let l = local(&area);
            if carry_rules {
                crate::condfmt::remove_area(&mut s.meta, &l);
            }
            if carry_notes && clear_notes {
                let map = Arc::make_mut(&mut s.meta.notes);
                crate::annotations::clear_in(map, &area);
            }
            for rr in 0..reps_r {
                for rc in 0..reps_c {
                    let (tr, tc) = (target.r0 + rr * rows, target.c0 + rc * cols);
                    let place_rect = |r: &CellRangeLocal| -> Option<CellRangeLocal> {
                        let n = CellRangeLocal {
                            r0: tr + r.r0,
                            c0: tc + r.c0,
                            r1: tr + r.r1,
                            c1: tc + r.c1,
                        };
                        crate::condfmt::intersect_rect(&n, &l)
                    };
                    let origin_at = |o: (i64, i64)| {
                        CellAddr::new(
                            sheet,
                            (i64::from(tr) + o.0).clamp(0, i64::from(MAX_ROWS - 1)) as u32,
                            (i64::from(tc) + o.1).clamp(0, i64::from(MAX_COLS - 1)) as u32,
                        )
                    };
                    if carry_rules {
                        for cf in &clip.cond_formats {
                            let ranges: Vec<CellRangeLocal> =
                                cf.ranges.iter().filter_map(place_rect).collect();
                            if ranges.is_empty() {
                                continue;
                            }
                            let mut item = cf.item.clone();
                            item.id = s.meta.alloc_id();
                            item.ranges = ranges;
                            item.origin = origin_at(cf.origin);
                            Arc::make_mut(&mut s.meta.cond_formats).push(Arc::new(item));
                        }
                        for v in &clip.validations {
                            let ranges: Vec<CellRangeLocal> =
                                v.ranges.iter().filter_map(place_rect).collect();
                            if ranges.is_empty() {
                                continue;
                            }
                            let mut item = v.item.clone();
                            item.id = s.meta.alloc_id();
                            item.ranges = ranges;
                            item.origin = origin_at(v.origin);
                            Arc::make_mut(&mut s.meta.validations).push(Arc::new(item));
                        }
                    }
                    if carry_notes && paste_notes {
                        let map = Arc::make_mut(&mut s.meta.notes);
                        for (dr, dc, note) in &clip.notes {
                            let (a, b) = place(*dr, *dc);
                            let (r, c) = (tr + a, tc + b);
                            if r <= area.r1 && c <= area.c1 {
                                map.insert((r, c), note.clone());
                            }
                        }
                    }
                }
            }
        }
        fx.layout(sheet);
    }
    wb.pending.ranges.push(area);
    Ok(changes)
}

/// A formula with the relative row and column offsets of its references
/// swapped, for Paste Special ▸ Transpose (`=A1*5` copied from B1 and
/// transposed reads the cell left of it instead of above). Only references
/// whose row and column are both relative are transposed; mixed and
/// absolute ones keep their target.
fn transposed(content: &SnapContent) -> SnapContent {
    use tally_formula::{CellRefExpr, RefExpr};
    let SnapContent::Formula {
        formula,
        invalid_text: None,
        array,
    } = content
    else {
        return content.clone();
    };
    let swap = |c: &mut CellRefExpr| {
        if !c.row.abs && !c.col.abs {
            std::mem::swap(&mut c.row, &mut c.col);
        }
    };
    let mut f = (**formula).clone();
    f.expr.walk_refs_mut(|r| match r {
        RefExpr::Cell(c) => swap(c),
        RefExpr::Area(a, b) => {
            swap(a);
            swap(b);
        }
        _ => {}
    });
    SnapContent::Formula {
        formula: std::sync::Arc::new(f),
        invalid_text: None,
        array: *array,
    }
}

fn is_array(c: &Option<SnapContent>) -> bool {
    matches!(c, Some(SnapContent::Formula { array: Some(_), .. }))
}

fn area_has_merges(wb: &Workbook, area: &CellRange) -> bool {
    wb.sheet(area.sheet).is_some_and(|s| {
        s.meta
            .merges
            .iter()
            .any(|m| CellRange::new(area.sheet, m.r0, m.c0, m.r1, m.c1).intersects(area))
    })
}

/// Splits TSV text into rows of fields (`\r\n` or `\n`; a trailing newline
/// adds no row; quoted fields may contain tabs, newlines and `""`).
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut at_field_start = true;
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if at_field_start => {
                in_quotes = true;
                at_field_start = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut field));
                at_field_start = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                at_field_start = true;
            }
            c => {
                field.push(c);
                at_field_start = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// A copied formula whose sheets were deleted since the copy: those
/// references become `#REF!` (`SheetRef::Deleted`), as they print, instead
/// of reading an empty sheet that no longer exists.
fn without_missing_sheets(wb: &Workbook, content: SnapContent) -> SnapContent {
    use tally_formula::{RefExpr, SheetRef};
    let SnapContent::Formula {
        formula,
        invalid_text,
        array,
    } = content
    else {
        return content;
    };
    let gone = |s: &SheetRef| matches!(s, SheetRef::Id { id, .. } if wb.sheet(*id).is_none());
    let mut hit = false;
    formula.expr.walk_refs(|r| {
        hit |= match r {
            RefExpr::Cell(c) => gone(&c.sheet),
            RefExpr::Area(a, b) => gone(&a.sheet) || gone(&b.sheet),
            RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => gone(sheet),
            RefExpr::Invalid => false,
        };
    });
    if !hit {
        return SnapContent::Formula {
            formula,
            invalid_text,
            array,
        };
    }
    let mut nf = (*formula).clone();
    nf.expr.walk_refs_mut(|r| {
        let mark = |s: &mut SheetRef| {
            if gone(s) {
                *s = SheetRef::Deleted;
            }
        };
        match r {
            RefExpr::Cell(c) => mark(&mut c.sheet),
            RefExpr::Area(a, b) => {
                mark(&mut a.sheet);
                mark(&mut b.sheet);
            }
            RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => mark(sheet),
            RefExpr::Invalid => {}
        }
    });
    SnapContent::Formula {
        formula: Arc::new(nf),
        invalid_text,
        array,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_parsing() {
        assert_eq!(
            parse_tsv("1\t2\n3\t4\n"),
            vec![vec!["1", "2"], vec!["3", "4"]]
        );
        assert_eq!(parse_tsv("a\r\nb"), vec![vec!["a"], vec!["b"]]);
        assert_eq!(parse_tsv("\"x\ty\"\t\"q\"\"\""), vec![vec!["x\ty", "q\""]]);
        assert_eq!(parse_tsv("a\t\tb"), vec![vec!["a", "", "b"]]);
        assert!(parse_tsv("").is_empty());
    }

    #[test]
    fn html_escape() {
        assert_eq!(escape_html("<a & 'b'>"), "&lt;a &amp; &#39;b&#39;&gt;");
    }
}
