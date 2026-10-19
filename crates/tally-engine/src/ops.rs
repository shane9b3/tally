//! Applying [`Op`]s. Each op validates its arguments, records the inverse
//! [`Change`]s it needs for undo, and mutates the workbook; recalculation
//! happens once per transaction in [`crate::document`].

use std::collections::HashMap;
use std::sync::Arc;

use tally_api::{ApiError, ClearWhat, Op, PastePayload, RangeDto};
use tally_formula::{adjust_for_structure_change, RefExpr, SheetRef, StructureChange};
use tally_numfmt::parse_format;
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::clipboard::{parse_tsv, ClipSnapshot};
use crate::formula_cells::FormulaCellId;
use crate::storage::StyleId;
use crate::undo::{
    apply_change, capture_range, clear_range_content, clear_range_styles, remove_sheet, Change,
    Effects,
};
use crate::units::px_to_hmm;
use crate::workbook::{SnapContent, Workbook};

/// Most cells one op may write (paste, range inputs).
pub const MAX_OP_CELLS: u64 = 1_048_576;

/// Calls `f` on every sheet qualifier of `r`.
fn sheet_refs_mut(r: &mut RefExpr, f: &mut impl FnMut(&mut SheetRef)) {
    match r {
        RefExpr::Cell(c) => f(&mut c.sheet),
        RefExpr::Area(a, b) => {
            f(&mut a.sheet);
            f(&mut b.sheet);
        }
        RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => f(sheet),
        RefExpr::Invalid => {}
    }
}

/// Points references to an unknown sheet named like sheet `id` (added or
/// renamed just now) at that sheet, as reloading the file would. Returns
/// the change that restores the old formulas.
fn resolve_sheet_refs(wb: &mut Workbook, id: SheetId) -> Change {
    let Some(name) = wb.sheet(id).map(|s| s.name.to_string()) else {
        return Change::Cells(Vec::new());
    };
    let name = name.to_lowercase();
    let matches = |s: &SheetRef| matches!(s, SheetRef::Unresolved(n) if n.to_lowercase() == name);
    let mut targets: Vec<CellAddr> = Vec::new();
    for (_, f) in &wb.formulas {
        let mut hit = false;
        f.formula.expr.walk_refs(|r| {
            hit |= match r {
                RefExpr::Cell(c) => matches(&c.sheet),
                RefExpr::Area(a, b) => matches(&a.sheet) || matches(&b.sheet),
                RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => matches(sheet),
                RefExpr::Invalid => false,
            };
        });
        if hit {
            targets.push(f.addr);
        }
    }
    let mut old_cells = Vec::with_capacity(targets.len());
    for addr in targets {
        let Some((_, f)) = wb.formula_at(addr) else {
            continue;
        };
        let mut nf = (*f.formula).clone();
        let invalid = f.invalid_text.clone();
        nf.expr.walk_refs_mut(|r| {
            sheet_refs_mut(r, &mut |s| {
                if matches(s) {
                    *s = SheetRef::Id { id, abs: false };
                }
            });
        });
        old_cells.push((addr, wb.snapshot_content(addr)));
        wb.restore_content(
            addr,
            Some(SnapContent::Formula {
                formula: Arc::new(nf),
                invalid_text: invalid,
            }),
        );
    }
    Change::Cells(old_cells)
}

/// Characters not allowed in sheet names (as in LibreOffice).
const BAD_SHEET_CHARS: &[char] = &['[', ']', '*', '?', ':', '/', '\\'];

fn not_found_sheet(sheet: SheetId) -> ApiError {
    ApiError::not_found(format!("no sheet {sheet}"))
}

fn check_sheet(wb: &Workbook, sheet: SheetId) -> Result<(), ApiError> {
    if wb.sheet(sheet).is_some() {
        Ok(())
    } else {
        Err(not_found_sheet(sheet))
    }
}

/// Validates a range DTO and converts it.
pub fn to_range(sheet: SheetId, r: &RangeDto) -> Result<CellRange, ApiError> {
    if r.r0 > r.r1 || r.c0 > r.c1 || r.r1 >= MAX_ROWS || r.c1 >= MAX_COLS {
        return Err(ApiError::invalid_argument(format!(
            "range {}:{}-{}:{} is outside the grid",
            r.r0, r.c0, r.r1, r.c1
        )));
    }
    Ok(CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1))
}

fn check_cell(r: u32, c: u32) -> Result<(), ApiError> {
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument(format!(
            "cell {r}:{c} is outside the grid"
        )));
    }
    Ok(())
}

/// Applies `f(table, old)` to the style of every cell in `range`.
fn map_styles(
    wb: &mut Workbook,
    range: CellRange,
    mut f: impl FnMut(&mut crate::styles::StyleTable, StyleId) -> StyleId,
) {
    let Some(idx) = wb.tab_of(range.sheet) else {
        return;
    };
    let Workbook { sheets, styles, .. } = wb;
    let Some(sheet) = sheets.get_mut(idx) else {
        return;
    };
    let mut cache: HashMap<StyleId, StyleId> = HashMap::new();
    for c in range.c0..=range.c1 {
        let Some(col) = sheet.column_mut(c) else {
            continue;
        };
        col.styles.map(range.r0, range.r1, |old| {
            *cache.entry(old).or_insert_with(|| f(styles, old))
        });
    }
    wb.pending.display_only.push(range);
}

/// Writes a block of raw inputs with its top-left at `addr`.
fn write_inputs(wb: &mut Workbook, sheet: SheetId, r: u32, c: u32, rows: &[Vec<String>]) {
    for (dr, row) in rows.iter().enumerate() {
        for (dc, input) in row.iter().enumerate() {
            let (rr, cc) = (r as u64 + dr as u64, c as u64 + dc as u64);
            if rr >= u64::from(MAX_ROWS) || cc >= u64::from(MAX_COLS) {
                continue;
            }
            wb.set_input(CellAddr::new(sheet, rr as u32, cc as u32), input);
        }
    }
}

fn block_range(
    sheet: SheetId,
    r: u32,
    c: u32,
    rows: &[Vec<String>],
) -> Result<CellRange, ApiError> {
    check_cell(r, c)?;
    let h = rows.len().max(1) as u64;
    let w = rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as u64;
    if h * w > MAX_OP_CELLS {
        return Err(ApiError::invalid_argument(format!(
            "block of {h}×{w} cells is too large"
        )));
    }
    let r1 = (u64::from(r) + h - 1).min(u64::from(MAX_ROWS - 1)) as u32;
    let c1 = (u64::from(c) + w - 1).min(u64::from(MAX_COLS - 1)) as u32;
    Ok(CellRange::new(sheet, r, c, r1, c1))
}

/// Validates a sheet name.
pub fn check_sheet_name(name: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::invalid_argument("sheet name is empty"));
    }
    if name.chars().count() > 255 {
        return Err(ApiError::invalid_argument("sheet name is too long"));
    }
    if name.contains(BAD_SHEET_CHARS) || name.starts_with('\'') || name.ends_with('\'') {
        return Err(ApiError::invalid_argument(format!(
            "sheet name {name:?} contains a character that is not allowed"
        )));
    }
    Ok(())
}

/// Applies one op, returning the inverse changes (in apply order).
pub fn apply_op(
    wb: &mut Workbook,
    op: &Op,
    clip: Option<&ClipSnapshot>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    match op {
        Op::SetCellInput { sheet, r, c, input } => {
            check_sheet(wb, *sheet)?;
            check_cell(*r, *c)?;
            let range = CellRange::new(*sheet, *r, *c, *r, *c);
            let inv = capture_range(wb, range, true, true);
            wb.set_input(CellAddr::new(*sheet, *r, *c), input);
            Ok(vec![Change::Range(inv)])
        }
        Op::SetRangeInputs { sheet, r, c, rows } => {
            check_sheet(wb, *sheet)?;
            let range = block_range(*sheet, *r, *c, rows)?;
            let inv = capture_range(wb, range, true, true);
            write_inputs(wb, *sheet, *r, *c, rows);
            Ok(vec![Change::Range(inv)])
        }
        Op::Clear {
            sheet,
            ranges,
            what,
        } => {
            check_sheet(wb, *sheet)?;
            let ranges: Vec<CellRange> = ranges
                .iter()
                .map(|r| to_range(*sheet, r))
                .collect::<Result<_, _>>()?;
            let content = matches!(what, ClearWhat::Contents | ClearWhat::All);
            let style = matches!(what, ClearWhat::Formats | ClearWhat::All);
            let mut inv = Vec::with_capacity(ranges.len());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, content, style)));
                if content {
                    clear_range_content(wb, range);
                }
                if style {
                    clear_range_styles(wb, range);
                }
            }
            Ok(inv)
        }
        Op::ApplyStyle {
            sheet,
            ranges,
            patch,
        } => {
            check_sheet(wb, *sheet)?;
            if let Some(code) = &patch.number_format {
                parse_format(code)
                    .map_err(|e| ApiError::invalid_argument(format!("bad number format: {e}")))?;
            }
            let ranges: Vec<CellRange> = ranges
                .iter()
                .map(|r| to_range(*sheet, r))
                .collect::<Result<_, _>>()?;
            let mut inv = Vec::with_capacity(ranges.len());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, false, true)));
                map_styles(wb, range, |t, old| t.patched(old, patch));
            }
            Ok(inv)
        }
        Op::SetNumberFormat {
            sheet,
            ranges,
            code,
        } => {
            check_sheet(wb, *sheet)?;
            parse_format(code)
                .map_err(|e| ApiError::invalid_argument(format!("bad number format: {e}")))?;
            let ranges: Vec<CellRange> = ranges
                .iter()
                .map(|r| to_range(*sheet, r))
                .collect::<Result<_, _>>()?;
            let mut inv = Vec::with_capacity(ranges.len());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, false, true)));
                map_styles(wb, range, |t, old| t.with_format(old, code));
            }
            Ok(inv)
        }
        Op::InsertRows { sheet, at, count } => {
            let (sheet, at, count) = (*sheet, *at, *count);
            structure(wb, StructureChange::InsertRows { sheet, at, count }, fx)
        }
        Op::DeleteRows { sheet, at, count } => {
            let (sheet, at, count) = (*sheet, *at, *count);
            structure(wb, StructureChange::DeleteRows { sheet, at, count }, fx)
        }
        Op::InsertCols { sheet, at, count } => {
            let (sheet, at, count) = (*sheet, *at, *count);
            structure(wb, StructureChange::InsertCols { sheet, at, count }, fx)
        }
        Op::DeleteCols { sheet, at, count } => {
            let (sheet, at, count) = (*sheet, *at, *count);
            structure(wb, StructureChange::DeleteCols { sheet, at, count }, fx)
        }
        Op::SetColWidth {
            sheet,
            start,
            end,
            px,
        } => {
            if start > end || *end >= MAX_COLS || !px.is_finite() || *px < 0.0 {
                return Err(ApiError::invalid_argument("bad column range or width"));
            }
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let old = s.col_dims.clone();
            s.col_dims.set(*start, *end, px_to_hmm(*px), *px == 0.0);
            fx.layout(*sheet);
            Ok(vec![Change::ColDims {
                sheet: *sheet,
                dims: old,
            }])
        }
        Op::SetRowHeight {
            sheet,
            start,
            end,
            px,
        } => {
            if start > end || *end >= MAX_ROWS || !px.is_finite() || *px < 0.0 {
                return Err(ApiError::invalid_argument("bad row range or height"));
            }
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let old = s.row_dims.clone();
            s.row_dims.set(*start, *end, px_to_hmm(*px), *px == 0.0);
            fx.layout(*sheet);
            Ok(vec![Change::RowDims {
                sheet: *sheet,
                dims: old,
            }])
        }
        Op::AddSheet { name, index } => {
            if let Some(n) = name {
                check_sheet_name(n)?;
            }
            if wb.sheets.len() >= crate::workbook::MAX_SHEETS {
                return Err(ApiError::invalid_argument(format!(
                    "a workbook can have at most {} sheets",
                    crate::workbook::MAX_SHEETS
                )));
            }
            let id = wb.add_sheet(name.as_deref(), index.map(|i| i as usize));
            wb.refresh_all_precedents();
            fx.sheets_changed = true;
            // Undo restores the formulas first, then removes the sheet.
            let resolved = resolve_sheet_refs(wb, id);
            Ok(vec![Change::RemoveSheet { id }, resolved])
        }
        Op::RenameSheet { sheet, name } => {
            check_sheet(wb, *sheet)?;
            check_sheet_name(name)?;
            if wb.sheet_by_name_ci(name).is_some_and(|id| id != *sheet) {
                return Err(ApiError::invalid_argument(format!(
                    "a sheet named {name:?} already exists"
                )));
            }
            let inv = apply_change(
                wb,
                Change::RenameSheet {
                    id: *sheet,
                    name: Arc::from(name.as_str()),
                },
                fx,
            );
            let mut out: Vec<Change> = inv.into_iter().collect();
            out.push(resolve_sheet_refs(wb, *sheet));
            Ok(out)
        }
        Op::DeleteSheet { sheet } => {
            check_sheet(wb, *sheet)?;
            let visible = wb.sheets.iter().filter(|s| !s.hidden).count();
            let this_visible = wb.sheet(*sheet).is_some_and(|s| !s.hidden);
            if wb.sheets.len() <= 1 || (this_visible && visible <= 1) {
                return Err(ApiError::invalid_argument(
                    "the last sheet cannot be deleted",
                ));
            }
            // Formulas elsewhere that reference the sheet become #REF!.
            let change = StructureChange::DeleteSheet { sheet: *sheet };
            let mut adjusted: Vec<(FormulaCellId, CellAddr)> = Vec::new();
            for (id, f) in &wb.formulas {
                if f.addr.sheet != *sheet && f.precedents.iter().any(|p| p.sheet == *sheet) {
                    adjusted.push((id, f.addr));
                }
            }
            let mut old_cells = Vec::with_capacity(adjusted.len());
            for (id, addr) in adjusted {
                let Some(f) = wb.formulas.get(id) else {
                    continue;
                };
                let mut nf = (*f.formula).clone();
                let invalid = f.invalid_text.clone();
                if adjust_for_structure_change(&mut nf, addr, &change) {
                    old_cells.push((addr, wb.snapshot_content(addr)));
                    wb.restore_content(
                        addr,
                        Some(SnapContent::Formula {
                            formula: Arc::new(nf),
                            invalid_text: invalid,
                        }),
                    );
                }
            }
            let ins = remove_sheet(wb, *sheet, fx).ok_or_else(|| not_found_sheet(*sheet))?;
            Ok(vec![Change::Cells(old_cells), ins])
        }
        Op::MoveSheet { sheet, to } => {
            check_sheet(wb, *sheet)?;
            let inv = apply_change(
                wb,
                Change::MoveSheet {
                    id: *sheet,
                    to: *to as usize,
                },
                fx,
            );
            Ok(inv.into_iter().collect())
        }
        Op::Paste {
            sheet,
            target,
            payload,
        } => {
            check_sheet(wb, *sheet)?;
            let target = to_range(*sheet, target)?;
            match payload {
                PastePayload::Tsv { text } => {
                    let rows = parse_tsv(text);
                    if rows.is_empty() {
                        return Ok(Vec::new());
                    }
                    let range = block_range(*sheet, target.r0, target.c0, &rows)?;
                    let inv = capture_range(wb, range, true, true);
                    write_inputs(wb, *sheet, target.r0, target.c0, &rows);
                    Ok(vec![Change::Range(inv)])
                }
                PastePayload::Internal { token } => {
                    let clip = clip.filter(|c| c.token == *token).ok_or_else(|| {
                        ApiError::not_found("the clipboard token is no longer valid")
                    })?;
                    paste_internal(wb, *sheet, target, clip)
                }
            }
        }
    }
}

/// Inserts or deletes rows/columns, recording: the dimensions, the lost
/// cells (deleted, or pushed off the end), the rewritten formulas and the
/// inverse structural change.
fn structure(
    wb: &mut Workbook,
    change: StructureChange,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let shape = crate::structure::Shape::of(&change)
        .ok_or_else(|| ApiError::invalid_argument("not a row/column change"))?;
    check_sheet(wb, shape.sheet)?;
    let limit = shape.limit();
    if shape.count == 0 || shape.at >= limit || shape.count > limit - shape.at {
        return Err(ApiError::invalid_argument(format!(
            "cannot {} {} {} at {}",
            if shape.insert { "insert" } else { "delete" },
            shape.count,
            if shape.rows { "rows" } else { "columns" },
            shape.at
        )));
    }
    if shape.insert && shape.count >= limit {
        return Err(ApiError::invalid_argument(
            "cannot insert a whole sheet of rows/columns",
        ));
    }
    let sheet = wb
        .sheet(shape.sheet)
        .ok_or_else(|| not_found_sheet(shape.sheet))?;
    let dims = if shape.rows {
        Change::RowDims {
            sheet: shape.sheet,
            dims: sheet.row_dims.clone(),
        }
    } else {
        Change::ColDims {
            sheet: shape.sheet,
            dims: sheet.col_dims.clone(),
        }
    };
    let lost = Change::Range(capture_range(wb, shape.lost(), true, true));
    let old = wb.structure_change(&change);
    fx.layout(shape.sheet);
    Ok(vec![
        dims,
        lost,
        Change::Cells(old),
        Change::Structure(shape.inverse()),
    ])
}

fn paste_internal(
    wb: &mut Workbook,
    sheet: SheetId,
    target: CellRange,
    clip: &ClipSnapshot,
) -> Result<Vec<Change>, ApiError> {
    let (rows, cols) = (clip.rows.max(1), clip.cols.max(1));
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
    let inv = capture_range(wb, area, true, true);
    clear_range_content(wb, area);
    clear_range_styles(wb, area);
    let style_ids: Vec<StyleId> = clip
        .styles
        .iter()
        .map(|s| wb.styles.intern(s.clone()))
        .collect();
    for rr in 0..reps_r {
        for rc in 0..reps_c {
            for (dr, dc, content, style) in &clip.cells {
                let row = u64::from(target.r0) + u64::from(rr * rows + dr);
                let col = u64::from(target.c0) + u64::from(rc * cols + dc);
                if row > u64::from(r1) || col > u64::from(c1) {
                    continue;
                }
                let addr = CellAddr::new(sheet, row as u32, col as u32);
                if let Some(content) = content {
                    wb.restore_content(addr, Some(content.clone()));
                }
                if let Some(st) = style.and_then(|i| style_ids.get(i)) {
                    if let Some(c) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(addr.col)) {
                        c.styles.set(addr.row, addr.row, *st);
                    }
                }
            }
        }
    }
    Ok(vec![Change::Range(inv)])
}
