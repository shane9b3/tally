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
/// size to tile) with `opts`. TSV supports `values_only` (`=…` stays text),
/// `transpose` and `skip_empty`; internal payloads support every option.
pub fn paste(
    wb: &mut Workbook,
    sheet: SheetId,
    target: CellRange,
    payload: &PastePayload,
    opts: &PasteOptions,
    clip: Option<&ClipSnapshot>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    match payload {
        PastePayload::Tsv { text } => {
            let mut rows = parse_tsv(text);
            if rows.is_empty() {
                return Ok(Vec::new());
            }
            if opts.transpose {
                rows = transpose_rows(&rows);
            }
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
                            opts.values_only,
                        );
                    }
                }
            } else {
                write_inputs(wb, sheet, target.r0, target.c0, &rows, opts.values_only);
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
            let reps_r = if target.rows().is_multiple_of(rows) {
                target.rows() / rows
            } else {
                1
            };
            let reps_c = if target.cols().is_multiple_of(cols) {
                target.cols() / cols
            } else {
                1
            };
            let r1 = (u64::from(target.r0) + u64::from(rows * reps_r) - 1)
                .min(u64::from(MAX_ROWS - 1)) as u32;
            let c1 = (u64::from(target.c0) + u64::from(cols * reps_c) - 1)
                .min(u64::from(MAX_COLS - 1)) as u32;
            Ok(Some(CellRange::new(sheet, target.r0, target.c0, r1, c1)))
        }
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

/// Internal paste: formulas re-anchored (the AST is relative), styles and
/// merges carried, with the paste-special options.
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
    let reps_r = if target.rows().is_multiple_of(rows) {
        target.rows() / rows
    } else {
        1
    };
    let reps_c = if target.cols().is_multiple_of(cols) {
        target.cols() / cols
    } else {
        1
    };
    let total = u64::from(rows) * u64::from(cols) * u64::from(reps_r) * u64::from(reps_c);
    if total > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument("paste area is too large"));
    }
    let r1 =
        (u64::from(target.r0) + u64::from(rows * reps_r) - 1).min(u64::from(MAX_ROWS - 1)) as u32;
    let c1 =
        (u64::from(target.c0) + u64::from(cols * reps_c) - 1).min(u64::from(MAX_COLS - 1)) as u32;
    let area = CellRange::new(sheet, target.r0, target.c0, r1, c1);
    let contents = !opts.formats_only;
    let formats = !opts.values_only;
    if contents {
        check_array_blocks(wb, &[area])?;
    }
    // Merges are carried unless pasting values only.
    let carry_merges = formats && (!clip.merges.is_empty() || area_has_merges(wb, &area));
    if carry_merges {
        check_merges(wb, &area)?;
    }
    let mut changes = Vec::new();
    if carry_merges {
        changes.extend(meta_parts(wb, sheet, &[crate::storage::MetaKind::Merges]));
    }
    // Conditional formats and validations are attributes: pasted with the
    // formats (not transposed); notes with the contents.
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
    let carry_notes = contents
        && !opts.skip_empty
        && (!clip.notes.is_empty()
            || wb.sheet(sheet).is_some_and(|s| {
                crate::annotations::entries_in(&s.meta.notes, &area)
                    .next()
                    .is_some()
            }));
    if carry_notes {
        changes.extend(meta_parts(wb, sheet, &[crate::storage::MetaKind::Notes]));
    }
    changes.push(Change::Range(capture_range(wb, area, contents, formats)));
    let place = |dr: u32, dc: u32| if opts.transpose { (dc, dr) } else { (dr, dc) };
    // Target values before the paste (for arithmetic).
    let arithmetic = opts.operation != PasteOperation::None && contents;
    let mut writes: Vec<(CellAddr, &ClipCell)> = Vec::new();
    for rr in 0..reps_r {
        for rc in 0..reps_c {
            for cell in &clip.cells {
                let (dr, dc) = place(cell.dr, cell.dc);
                let row = u64::from(target.r0) + u64::from(rr * rows + dr);
                let col = u64::from(target.c0) + u64::from(rc * cols + dc);
                if row > u64::from(r1) || col > u64::from(c1) {
                    continue;
                }
                writes.push((CellAddr::new(sheet, row as u32, col as u32), cell));
            }
        }
    }
    if contents && !opts.skip_empty && !arithmetic {
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
    for (addr, cell) in writes {
        let source_empty = cell.value.is_empty() && cell.content.is_none();
        if contents && !(opts.skip_empty && source_empty) {
            if arithmetic {
                paste_arithmetic(wb, addr, cell, opts.operation);
            } else if opts.values_only || (opts.transpose && is_array(&cell.content)) {
                // The area was cleared (or empty sources are skipped).
                if !cell.value.is_empty() {
                    wb.set_value(addr, cell.value.clone());
                }
            } else if let Some(content) = &cell.content {
                let content = if opts.transpose {
                    transposed(content)
                } else {
                    content.clone()
                };
                wb.restore_content(addr, Some(content));
            }
        }
        if formats {
            if let Some(st) = cell.style.and_then(|i| style_ids.get(i)) {
                if let Some(c) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(addr.col)) {
                    c.styles.set(addr.row, addr.row, *st);
                }
            }
        }
    }
    if carry_rules || carry_notes {
        if let Some(s) = wb.sheet_mut(sheet) {
            let l = local(&area);
            if carry_rules {
                crate::condfmt::remove_area(&mut s.meta, &l);
            }
            if carry_notes {
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
                    if carry_notes {
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

/// Combines a pasted cell with the target (Paste Special › Operations).
/// Empty sources leave the target alone; a numeric source combines with a
/// number (or empty) target, and with a formula target as
/// `=(formula) op source`; text targets are kept; non-numeric sources
/// replace the target.
fn paste_arithmetic(wb: &mut Workbook, addr: CellAddr, cell: &ClipCell, op: PasteOperation) {
    if cell.value.is_empty() {
        return;
    }
    let Some(s) = number_of(&cell.value) else {
        wb.set_value(addr, cell.value.clone());
        return;
    };
    let is_formula = matches!(
        wb.sheet(addr.sheet)
            .and_then(|sh| sh.get(addr.row, addr.col)),
        Some(CellContent::Formula(_))
    );
    if is_formula {
        let Some((id, f)) = wb.formula_at(addr) else {
            return;
        };
        if f.invalid_text.is_some() || f.array.is_some() {
            return;
        }
        let Some(text) = wb.formula_text(id) else {
            return;
        };
        let body = text.strip_prefix('=').unwrap_or(&text);
        let num = tally_numfmt::general_string(s, None, &tally_numfmt::EN_US);
        let new = format!("=({body}){}{num}", op_symbol(op));
        if let Ok(nf) = wb.parse_formula(&new, addr) {
            wb.set_formula(addr, Arc::new(nf), None);
        }
        return;
    }
    let t = wb.value(addr);
    // Text targets are kept.
    if let Some(tn) = number_of(&t) {
        wb.set_value(addr, combine(op, tn, s));
    }
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
