//! Applying [`Op`]s. Each op validates its arguments, records the inverse
//! [`Change`]s it needs for undo, and mutates the workbook; recalculation
//! happens once per transaction in [`crate::document`].
//!
//! Ops that would change only part of an array formula block are refused
//! with the error code [`ARRAY_BLOCK`]; ops that would split a merged area
//! with [`MERGED`].

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tally_api::{ApiError, ClearWhat, Op, PasteOptions, RangeDto};
use tally_formula::{adjust_for_structure_change, RefExpr, SheetRef, StructureChange};
use tally_model::CellRangeLocal;
use tally_numfmt::parse_format;
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::clipboard::ClipSnapshot;
use crate::formula_cells::FormulaCellId;
use crate::storage::{AutoFilter, StyleId};
use crate::undo::{
    apply_change, capture_range, clear_range_content, clear_range_styles, names_snapshot,
    remove_sheet, sheet_meta, Change, Effects,
};
use crate::units::{px_to_hmm, DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use crate::workbook::Workbook;

/// Most cells one op may write (paste, range inputs).
pub const MAX_OP_CELLS: u64 = 1_048_576;

/// Error code: the op would change part of an array formula block.
pub const ARRAY_BLOCK: &str = "array_block";
/// Error code: the op would change part of a merged area.
pub const MERGED: &str = "merged";

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
        nf.expr.walk_refs_mut(|r| {
            sheet_refs_mut(r, &mut |s| {
                if matches(s) {
                    *s = SheetRef::Id { id, abs: false };
                }
            });
        });
        let old = wb.replace_formula_ast(addr, Arc::new(nf));
        old_cells.push((addr, old));
    }
    Change::Cells(old_cells)
}

/// Characters not allowed in sheet names (as in LibreOffice).
const BAD_SHEET_CHARS: &[char] = &['[', ']', '*', '?', ':', '/', '\\'];

fn not_found_sheet(sheet: SheetId) -> ApiError {
    ApiError::not_found(format!("no sheet {sheet}"))
}

/// `Ok` if the sheet exists.
pub fn check_sheet(wb: &Workbook, sheet: SheetId) -> Result<(), ApiError> {
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

/// The error for changing part of an array.
pub fn array_block_error() -> ApiError {
    ApiError::new(ARRAY_BLOCK, "You cannot change only part of an array.")
}

/// Refuses when an array block intersects one of `ranges` without lying
/// entirely inside it.
pub fn check_array_blocks(wb: &Workbook, ranges: &[CellRange]) -> Result<(), ApiError> {
    for r in ranges {
        for b in wb.array_blocks_in(r) {
            if !r.contains_range(&b) {
                return Err(array_block_error());
            }
        }
    }
    Ok(())
}

/// Refuses when a merge intersects `range` without lying entirely inside it.
pub fn check_merges(wb: &Workbook, range: &CellRange) -> Result<(), ApiError> {
    let Some(s) = wb.sheet(range.sheet) else {
        return Ok(());
    };
    for m in &s.meta.merges {
        let mr = CellRange::new(range.sheet, m.r0, m.c0, m.r1, m.c1);
        if mr.intersects(range) && !range.contains_range(&mr) {
            return Err(ApiError::new(
                MERGED,
                "You cannot change only part of a merged range.",
            ));
        }
    }
    Ok(())
}

/// Applies `f(table, old)` to the style of every cell in `range`.
pub fn map_styles(
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

/// Writes a block of raw inputs with its top-left at `addr` (cells covered
/// by a merge are skipped). With `constants`, `=…` stays text.
pub fn write_inputs(
    wb: &mut Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
    rows: &[Vec<String>],
    constants: bool,
) {
    for (dr, row) in rows.iter().enumerate() {
        for (dc, input) in row.iter().enumerate() {
            let (rr, cc) = (r as u64 + dr as u64, c as u64 + dc as u64);
            if rr >= u64::from(MAX_ROWS) || cc >= u64::from(MAX_COLS) {
                continue;
            }
            let (rr, cc) = (rr as u32, cc as u32);
            if wb.sheet(sheet).is_some_and(|s| s.is_merge_covered(rr, cc)) {
                continue;
            }
            let addr = CellAddr::new(sheet, rr, cc);
            if constants {
                wb.set_constant_input(addr, input);
            } else {
                wb.set_input(addr, input);
            }
        }
    }
}

/// The rectangle a block of inputs covers (validated).
pub fn block_range(
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

fn ranges_of(sheet: SheetId, ranges: &[RangeDto]) -> Result<Vec<CellRange>, ApiError> {
    ranges.iter().map(|r| to_range(sheet, r)).collect()
}

fn local(r: &CellRange) -> CellRangeLocal {
    CellRangeLocal {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
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
            // Typing into a merged area edits its anchor.
            let (r, c) = match wb.sheet(*sheet).and_then(|s| s.merge_at(*r, *c)) {
                Some(m) => (m.r0, m.c0),
                None => (*r, *c),
            };
            let range = CellRange::new(*sheet, r, c, r, c);
            check_array_blocks(wb, &[range])?;
            let inv = capture_range(wb, range, true, true);
            wb.set_input(CellAddr::new(*sheet, r, c), input);
            Ok(vec![Change::Range(inv)])
        }
        Op::SetRangeInputs { sheet, r, c, rows } => {
            check_sheet(wb, *sheet)?;
            let range = block_range(*sheet, *r, *c, rows)?;
            check_array_blocks(wb, &[range])?;
            let inv = capture_range(wb, range, true, true);
            write_inputs(wb, *sheet, *r, *c, rows, false);
            Ok(vec![Change::Range(inv)])
        }
        Op::Clear {
            sheet,
            ranges,
            what,
        } => {
            check_sheet(wb, *sheet)?;
            let ranges = ranges_of(*sheet, ranges)?;
            let content = matches!(what, ClearWhat::Contents | ClearWhat::All);
            let style = matches!(what, ClearWhat::Formats | ClearWhat::All);
            if content {
                check_array_blocks(wb, &ranges)?;
            }
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
            let ranges = ranges_of(*sheet, ranges)?;
            let mut inv = Vec::with_capacity(ranges.len());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, false, true)));
                map_styles(wb, range, |t, old| t.patched(old, patch));
                if let Some(b) = &patch.borders {
                    crate::styles::apply_borders(wb, range, b);
                }
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
            let ranges = ranges_of(*sheet, ranges)?;
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
            let names = names_snapshot(wb);
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
                if adjust_for_structure_change(&mut nf, addr, &change) {
                    let old = wb.replace_formula_ast(addr, Arc::new(nf));
                    old_cells.push((addr, old));
                }
            }
            // Names scoped to the sheet go; names pointing at it become #REF!.
            wb.names.undefine_scope(*sheet);
            for def in wb.names.defs_mut() {
                let mut f = (*def.formula).clone();
                if adjust_for_structure_change(&mut f, def.base, &change) {
                    def.formula = Arc::new(f);
                }
            }
            let ins = remove_sheet(wb, *sheet, fx).ok_or_else(|| not_found_sheet(*sheet))?;
            // Bases on the deleted sheet move to the first sheet (same cell).
            let first = wb.name_base(None).sheet;
            for def in wb.names.defs_mut() {
                if def.base.sheet == *sheet {
                    def.base.sheet = first;
                }
            }
            wb.refresh_all_name_dependents();
            Ok(vec![names, Change::Cells(old_cells), ins])
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
            crate::clipboard::paste(
                wb,
                *sheet,
                target,
                payload,
                &PasteOptions::default(),
                clip,
                fx,
            )
        }
        Op::PasteSpecial {
            sheet,
            target,
            payload,
            options,
        } => {
            check_sheet(wb, *sheet)?;
            let target = to_range(*sheet, target)?;
            crate::clipboard::paste(wb, *sheet, target, payload, options, clip, fx)
        }
        Op::DefineName {
            name,
            scope,
            refers_to,
        } => {
            if let Some(s) = scope {
                check_sheet(wb, *s)?;
            }
            crate::names::validate_name(wb, name)?;
            let exists = wb
                .names
                .lookup(name, *scope)
                .and_then(|id| wb.names.get(id))
                .is_some_and(|e| e.def.is_some());
            if !exists && wb.names.len() >= crate::names::MAX_NAMES {
                return Err(ApiError::invalid_argument("too many names"));
            }
            let def = wb.parse_name_expr(refers_to, *scope)?;
            let inv = names_snapshot(wb);
            wb.names.define(name, *scope, def);
            wb.refresh_name_dependents(&crate::names::name_key(name));
            Ok(vec![inv])
        }
        Op::DeleteName { name, scope } => {
            let id = wb
                .names
                .lookup(name, *scope)
                .filter(|id| wb.names.get(*id).is_some_and(|e| e.def.is_some()))
                .ok_or_else(|| ApiError::not_found(format!("no name {name:?}")))?;
            let inv = names_snapshot(wb);
            wb.names.undefine(id);
            wb.refresh_name_dependents(&crate::names::name_key(name));
            Ok(vec![inv])
        }
        Op::SetArrayFormula {
            sheet,
            range,
            input,
        } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            if range.cell_count() > MAX_OP_CELLS {
                return Err(ApiError::invalid_argument("the array is too large"));
            }
            let text = input.trim();
            if !text.starts_with('=') || text.len() < 2 {
                return Err(ApiError::invalid_argument(
                    "an array formula must start with =",
                ));
            }
            let anchor = range.start();
            let f = wb
                .parse_formula(text, anchor)
                .map_err(|e| ApiError::invalid_argument(format!("bad formula: {e}")))?;
            check_array_blocks(wb, &[range])?;
            check_merges(wb, &range)?;
            let inv = capture_range(wb, range, true, false);
            wb.set_array_formula(anchor, Arc::new(f), range.rows(), range.cols());
            Ok(vec![Change::Range(inv)])
        }
        Op::MoveRange { sheet, from, to } => {
            check_sheet(wb, *sheet)?;
            let from = to_range(*sheet, from)?;
            check_cell(to.r, to.c)?;
            crate::moves::move_range(wb, from, *to, fx)
        }
        Op::MergeCells { sheet, range } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            if range.is_single() {
                return Err(ApiError::invalid_argument(
                    "a merge needs at least two cells",
                ));
            }
            check_merges(wb, &range)?;
            check_array_blocks(wb, &[range])?;
            let meta = sheet_meta(wb, *sheet).ok_or_else(|| not_found_sheet(*sheet))?;
            let content = capture_range(wb, range, true, false);
            // Only the top-left cell keeps its content.
            let anchor = range.start();
            let mut cells: Vec<CellAddr> = Vec::new();
            if let Some(s) = wb.sheet(*sheet) {
                for c in range.c0..=range.c1.min(s.columns.len().saturating_sub(1) as u32) {
                    if let Some(col) = s.column(c) {
                        use crate::storage::ColumnStore;
                        cells.extend(
                            col.cells
                                .iter_range(range.r0, range.r1)
                                .map(|(r, _)| CellAddr::new(*sheet, r, c))
                                .filter(|a| *a != anchor),
                        );
                    }
                }
            }
            for a in cells {
                wb.clear_content(a);
            }
            if let Some(s) = wb.sheet_mut(*sheet) {
                let l = local(&range);
                s.meta
                    .merges
                    .retain(|m| !(l.r0 <= m.r0 && m.r1 <= l.r1 && l.c0 <= m.c0 && m.c1 <= l.c1));
                s.meta.merges.push(l);
            }
            wb.pending.ranges.push(range);
            fx.layout(*sheet);
            Ok(vec![meta, Change::Range(content)])
        }
        Op::UnmergeCells { sheet, range } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            let meta = sheet_meta(wb, *sheet).ok_or_else(|| not_found_sheet(*sheet))?;
            let mut hit = Vec::new();
            if let Some(s) = wb.sheet_mut(*sheet) {
                s.meta.merges.retain(|m| {
                    let mr = CellRange::new(*sheet, m.r0, m.c0, m.r1, m.c1);
                    let keep = !mr.intersects(&range);
                    if !keep {
                        hit.push(mr);
                    }
                    keep
                });
            }
            wb.pending.display_only.extend(hit);
            fx.layout(*sheet);
            Ok(vec![meta])
        }
        Op::SetFrozen { sheet, rows, cols } => {
            check_sheet(wb, *sheet)?;
            if *rows >= MAX_ROWS || *cols >= MAX_COLS {
                return Err(ApiError::invalid_argument("cannot freeze the whole sheet"));
            }
            let meta = sheet_meta(wb, *sheet).ok_or_else(|| not_found_sheet(*sheet))?;
            if let Some(s) = wb.sheet_mut(*sheet) {
                s.meta.frozen = (*rows > 0 || *cols > 0).then_some((*rows, *cols));
            }
            fx.layout(*sheet);
            Ok(vec![meta])
        }
        Op::SetRowsHidden {
            sheet,
            start,
            end,
            hidden,
        } => {
            if start > end || *end >= MAX_ROWS {
                return Err(ApiError::invalid_argument("bad row range"));
            }
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let old = s.row_dims.clone();
            s.row_dims
                .set_hidden(*start, *end, *hidden, px_to_hmm(DEFAULT_ROW_HEIGHT_PX));
            fx.layout(*sheet);
            Ok(vec![Change::RowDims {
                sheet: *sheet,
                dims: old,
            }])
        }
        Op::SetColsHidden {
            sheet,
            start,
            end,
            hidden,
        } => {
            if start > end || *end >= MAX_COLS {
                return Err(ApiError::invalid_argument("bad column range"));
            }
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let old = s.col_dims.clone();
            s.col_dims
                .set_hidden(*start, *end, *hidden, px_to_hmm(DEFAULT_COL_WIDTH_PX));
            fx.layout(*sheet);
            Ok(vec![Change::ColDims {
                sheet: *sheet,
                dims: old,
            }])
        }
        Op::SetAutoFilter { sheet, range } => {
            check_sheet(wb, *sheet)?;
            let range = range.as_ref().map(|r| to_range(*sheet, r)).transpose()?;
            let meta = sheet_meta(wb, *sheet).ok_or_else(|| not_found_sheet(*sheet))?;
            if let Some(s) = wb.sheet_mut(*sheet) {
                s.meta.auto_filter = range.map(|r| AutoFilter {
                    range: local(&r),
                    columns: BTreeMap::new(),
                });
            }
            wb.reapply_filter(*sheet);
            fx.layout(*sheet);
            Ok(vec![meta])
        }
        Op::SetColumnFilter { sheet, col, filter } => {
            check_sheet(wb, *sheet)?;
            let meta = sheet_meta(wb, *sheet).ok_or_else(|| not_found_sheet(*sheet))?;
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let af = s
                .meta
                .auto_filter
                .as_mut()
                .ok_or_else(|| ApiError::invalid_argument("the sheet has no AutoFilter"))?;
            if *col < af.range.c0 || *col > af.range.c1 {
                return Err(ApiError::invalid_argument(
                    "the column is outside the AutoFilter range",
                ));
            }
            match filter {
                Some(f) if f.values.is_some() || f.condition.is_some() => {
                    af.columns.insert(*col, crate::filter::criteria_from_api(f));
                }
                _ => {
                    af.columns.remove(col);
                }
            }
            wb.reapply_filter(*sheet);
            fx.layout(*sheet);
            Ok(vec![meta])
        }
        Op::SortRange {
            sheet,
            range,
            keys,
            has_header,
            case_sensitive,
        } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            crate::sort::sort_range(wb, range, keys, *has_header, *case_sensitive)
        }
        Op::ReplaceAll {
            sheet,
            query,
            replace,
        } => {
            if let Some(s) = sheet {
                check_sheet(wb, *s)?;
            }
            crate::find::replace_all(wb, *sheet, query, replace, fx)
        }
        Op::FillRange {
            sheet,
            source,
            target,
            mode,
        } => {
            check_sheet(wb, *sheet)?;
            let source = to_range(*sheet, source)?;
            let target = to_range(*sheet, target)?;
            crate::fill::fill_range(wb, source, target, *mode)
        }
    }
}

/// Inserts or deletes rows/columns, recording: the names and sheet meta
/// state, the dimensions, the lost cells (deleted, or pushed off the end),
/// the rewritten formulas and the inverse structural change.
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
    let splits = wb.formulas.iter().any(|(_, f)| {
        f.array_range()
            .is_some_and(|b| crate::structure::splits_block(&shape, &b))
    });
    if splits {
        return Err(array_block_error());
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
    let names = names_snapshot(wb);
    let meta = sheet_meta(wb, shape.sheet).ok_or_else(|| not_found_sheet(shape.sheet))?;
    let lost = Change::Range(capture_range(wb, shape.lost(), true, true));
    let old = wb.structure_change(&change);
    wb.adjust_meta_for_structure(&change);
    fx.layout(shape.sheet);
    Ok(vec![
        names,
        meta,
        dims,
        lost,
        Change::Cells(old),
        Change::Structure(shape.inverse()),
    ])
}
