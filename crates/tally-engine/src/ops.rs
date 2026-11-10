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
use tally_formula::{
    adjust_for_structure_change, retarget_3d_for_sheet_delete, RefExpr, SheetRef, StructureChange,
};
use tally_model::CellRangeLocal;
use tally_numfmt::parse_format;
use tally_types::{CellAddr, CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::clipboard::ClipSnapshot;
use crate::formula_cells::FormulaCellId;
use crate::storage::MetaKind;
use crate::storage::{AutoFilter, StyleId};
use crate::undo::{
    apply_change, capture_range, clear_range_content, clear_range_styles, meta_parts,
    names_snapshot, remove_sheet, sheet_meta, Change, Effects,
};
use crate::units::{px_to_hmm, DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX};
use crate::workbook::Workbook;

/// Most cells one op may write (paste, range inputs).
pub const MAX_OP_CELLS: u64 = 1_048_576;

/// Most cells one transaction may touch, summed over its ops with
/// [`op_cost`]; an op that would go past it fails (`unsupported`) and the
/// rest of the transaction still applies (security review 2, MEDIUM-4:
/// 64 × `SetArrayFormula` of 1M cells took 11 s and 1.7 GB in one apply).
pub const MAX_TX_CELLS: u64 = 50_000_000;

/// The cells an op touches, for [`MAX_TX_CELLS`]: the cells written or
/// moved; for clears and sorts the used part of the range; for style ops
/// one per column (styles are stored as column runs); Replace All counts
/// its per-op maximum ([`crate::find::MAX_REPLACE`]).
pub fn op_cost(wb: &Workbook, op: &Op, clip: Option<&ClipSnapshot>) -> u64 {
    let area = |r: &RangeDto| -> u64 {
        if r.r1 < r.r0 || r.c1 < r.c0 {
            return 0;
        }
        (u64::from(r.r1 - r.r0) + 1) * (u64::from(r.c1 - r.c0) + 1)
    };
    let used = |sheet: SheetId, r: &RangeDto| -> u64 {
        if r.r1 < r.r0 || r.c1 < r.c0 {
            return 0;
        }
        wb.sheet(sheet)
            .and_then(|s| s.used_range())
            .and_then(|u| u.intersect(&CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1)))
            .map_or(0, |x| x.cell_count())
    };
    let cols = |r: &RangeDto| u64::from(r.c1.saturating_sub(r.c0)) + 1;
    let cost = match op {
        Op::SetRangeInputs { rows, .. } => rows.iter().map(|r| r.len() as u64).sum(),
        Op::Clear { sheet, ranges, .. } => ranges.iter().map(|r| used(*sheet, r)).sum(),
        Op::ApplyStyle { ranges, .. } | Op::SetNumberFormat { ranges, .. } => {
            ranges.iter().map(cols).sum()
        }
        Op::SetArrayFormula { range, .. } => area(range),
        Op::MoveRange { from, .. } => area(from).saturating_mul(2),
        Op::SortRange { sheet, range, .. } => used(*sheet, range),
        // A small target still receives the whole block (review M3).
        Op::Paste {
            target, payload, ..
        } => area(target)
            .min(MAX_OP_CELLS)
            .max(paste_block_cells(payload, clip)),
        Op::PasteSpecial {
            sheet,
            target,
            payload,
            options,
        } => paste_special_cost(wb, *sheet, target, payload, options, clip)
            .max(area(target).min(MAX_OP_CELLS)),
        Op::FillRange { target, .. } => area(target),
        Op::FillSheets {
            sheet, ranges, to, ..
        } => crate::group::fill_sheets_cost(wb, *sheet, ranges, to),
        // At most MAX_REPLACE cells, and never more than the sheet holds,
        // so a Replace over a large group is not "too large" (review L3).
        Op::ReplaceAll { sheet, .. } => {
            let cells = match sheet {
                Some(s) => wb.sheet(*s).map_or(0, |s| s.cell_count() as u64),
                None => wb.sheets.iter().map(|s| s.cell_count() as u64).sum(),
            };
            cells.min(crate::find::MAX_REPLACE as u64)
        }
        _ => crate::tools::tool_op_cost(wb, op).unwrap_or(1),
    };
    cost.max(1)
}

/// Paste Special's cost: the cells written (Link with Paste all writes a
/// formula into every cell of the pasted area, empty or not), plus the
/// band Shift cells moves (as `Op::InsertCells` counts it).
fn paste_special_cost(
    wb: &Workbook,
    sheet: SheetId,
    target: &RangeDto,
    payload: &tally_api::PastePayload,
    options: &PasteOptions,
    clip: Option<&ClipSnapshot>,
) -> u64 {
    let block = paste_block_cells(payload, clip);
    let Ok(t) = to_range(sheet, target) else {
        return block;
    };
    let Ok(Some(pasted)) = crate::clipboard::paste_area(sheet, t, payload, options, clip) else {
        return block;
    };
    let link_all = options.link == Some(true)
        && options.selection.is_none()
        && !options.values_only
        && !options.formats_only;
    let written = if link_all { pasted.cell_count() } else { block };
    let shifted = options.shift.map_or(0, |shift| {
        crate::tools::tool_op_cost(
            wb,
            &Op::InsertCells {
                sheet,
                range: crate::document::range_dto(&pasted),
                shift,
            },
        )
        .unwrap_or(0)
    });
    written.saturating_add(shifted)
}

/// The cells of a paste payload's block: the snapshot's cells for the
/// engine's own clipboard, rows × columns for TSV (counted without parsing:
/// lines × the most tabs on one line).
fn paste_block_cells(payload: &tally_api::PastePayload, clip: Option<&ClipSnapshot>) -> u64 {
    match payload {
        tally_api::PastePayload::Internal { token } => clip
            .filter(|c| c.token == *token)
            .map_or(0, |c| c.cells.len() as u64),
        tally_api::PastePayload::Tsv { text } => {
            let mut rows: u64 = 0;
            let mut cols: u64 = 0;
            for line in text.split('\n') {
                rows += 1;
                cols = cols.max(line.bytes().filter(|b| *b == b'\t').count() as u64 + 1);
            }
            rows.saturating_mul(cols)
        }
    }
}

/// Error code: the op would change part of an array formula block.
pub const ARRAY_BLOCK: &str = "array_block";

/// `(row, col)` keys of a per-cell map.
type CellKeys = Vec<(u32, u32)>;
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
pub(crate) fn resolve_sheet_refs(wb: &mut Workbook, id: SheetId) -> Vec<Change> {
    let Some(name) = wb.sheet(id).map(|s| s.name.to_string()) else {
        return Vec::new();
    };
    let name = name.to_lowercase();
    let matches = |s: &SheetRef| matches!(s, SheetRef::Unresolved(n) if n.to_lowercase() == name);
    let hits = |f: &tally_formula::Formula| {
        let mut hit = false;
        f.expr.walk_refs(|r| {
            hit |= match r {
                RefExpr::Cell(c) => matches(&c.sheet),
                RefExpr::Area(a, b) => matches(&a.sheet) || matches(&b.sheet),
                RefExpr::Cols { sheet, .. } | RefExpr::Rows { sheet, .. } => matches(sheet),
                RefExpr::Invalid => false,
            };
        });
        hit
    };
    let rewrite = |f: &tally_formula::Formula| {
        let mut nf = f.clone();
        nf.expr.walk_refs_mut(|r| {
            sheet_refs_mut(r, &mut |s| {
                if matches(s) {
                    *s = SheetRef::Id { id, abs: false };
                }
            });
        });
        nf
    };
    let mut out = Vec::new();
    // Names typed before their sheet existed (a reload resolves them too).
    if wb.names.defs().any(|d| hits(&d.formula)) {
        out.push(names_snapshot(wb));
        for def in wb.names.defs_mut() {
            if hits(&def.formula) {
                def.formula = Arc::new(rewrite(&def.formula));
            }
        }
        wb.refresh_all_name_dependents();
    }
    let targets: Vec<CellAddr> = wb
        .formulas
        .iter()
        .filter(|(_, f)| hits(&f.formula))
        .map(|(_, f)| f.addr)
        .collect();
    let mut old_cells = Vec::with_capacity(targets.len());
    for addr in targets {
        let Some((_, f)) = wb.formula_at(addr) else {
            continue;
        };
        let nf = rewrite(&f.formula);
        let old = wb.replace_formula_ast(addr, Arc::new(nf));
        old_cells.push((addr, old));
    }
    out.push(Change::Cells(old_cells));
    out
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
    for m in s.meta.merges.iter() {
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

// TEXT() caps formula format codes with tally-numfmt's copy of the limit.
const _: () = assert!(tally_numfmt::MAX_CODE_LEN == tally_model::MAX_NUMBER_FORMAT_LEN);

/// Refuses number format codes over [`tally_model::MAX_NUMBER_FORMAT_LEN`].
pub(crate) fn check_format_len(code: &str) -> Result<(), ApiError> {
    if code.chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN {
        return Err(ApiError::invalid_argument(format!(
            "number format is longer than {} characters",
            tally_model::MAX_NUMBER_FORMAT_LEN
        )));
    }
    Ok(())
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
///
/// v0.3: ops on a protected sheet are checked first ([`precheck`]), and
/// typed or pasted values are validated afterwards ([`postcheck`]; a
/// failing op is reverted).
pub fn apply_op(
    wb: &mut Workbook,
    op: &Op,
    clip: Option<&ClipSnapshot>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    if let Op::FillSheets {
        sheet,
        ranges,
        to,
        options,
    } = op
    {
        return crate::group::fill_sheets(wb, *sheet, ranges, to, options, fx);
    }
    if let Some(r) = crate::ops_v03::apply(wb, op, fx) {
        return r;
    }
    if let Some(r) = crate::ops_v04::apply(wb, op, fx) {
        return r;
    }
    if let Some(r) = crate::ops_v05::apply(wb, op, fx) {
        return r;
    }
    precheck(wb, op)?;
    let changes = apply_op_inner(wb, op, clip, fx)?;
    postcheck(wb, op, changes, fx)
}

fn ranges_or_err(sheet: SheetId, ranges: &[RangeDto]) -> Result<Vec<CellRange>, ApiError> {
    ranges.iter().map(|r| to_range(sheet, r)).collect()
}

/// Protection checks that can be made before an op runs.
pub fn precheck(wb: &Workbook, op: &Op) -> Result<(), ApiError> {
    use crate::protection::{check_edit, check_format, check_sheet_action, check_structure};
    let cell = |sheet: SheetId, r: u32, c: u32| CellRange::new(sheet, r, c, r, c);
    match op {
        Op::SetCellInput { sheet, r, c, .. } => {
            if *r < MAX_ROWS && *c < MAX_COLS {
                check_edit(wb, &[cell(*sheet, *r, *c)])?;
            }
        }
        Op::SetRangeInputs { sheet, r, c, rows } => {
            if let Ok(range) = block_range(*sheet, *r, *c, rows) {
                check_edit(wb, &[range])?;
            }
        }
        Op::Clear {
            sheet,
            ranges,
            what,
        } => {
            let ranges = ranges_or_err(*sheet, ranges)?;
            if matches!(what, ClearWhat::Contents | ClearWhat::All) {
                check_edit(wb, &ranges)?;
            }
            if matches!(what, ClearWhat::Formats | ClearWhat::All) {
                check_format(wb, &ranges)?;
            }
        }
        Op::ApplyStyle { sheet, ranges, .. } | Op::SetNumberFormat { sheet, ranges, .. } => {
            check_format(wb, &ranges_or_err(*sheet, ranges)?)?;
        }
        Op::InsertRows { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.insert_rows)?,
        Op::DeleteRows { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.delete_rows)?,
        Op::InsertCols { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.insert_cols)?,
        Op::DeleteCols { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.delete_cols)?,
        Op::SetColWidth { sheet, .. }
        | Op::SetRowHeight { sheet, .. }
        | Op::SetRowsHidden { sheet, .. }
        | Op::SetColsHidden { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.format_cells)?,
        Op::AddSheet { .. }
        | Op::RenameSheet { .. }
        | Op::DeleteSheet { .. }
        | Op::MoveSheet { .. } => check_structure(wb)?,
        Op::SetArrayFormula { sheet, range, .. } => check_edit(wb, &[to_range(*sheet, range)?])?,
        Op::MoveRange { sheet, from, to } => {
            let from = to_range(*sheet, from)?;
            check_edit(wb, &[from])?;
            if let Some(dest) = (BlockDest { from, to: *to }).range() {
                check_edit(wb, &[dest])?;
            }
        }
        Op::MergeCells { sheet, .. }
        | Op::UnmergeCells { sheet, .. }
        | Op::SetAutoFilter { sheet, .. } => {
            check_sheet_action(wb, *sheet, |_| false)?;
        }
        Op::SetColumnFilter { sheet, .. } => check_sheet_action(wb, *sheet, |p| p.auto_filter)?,
        Op::SortRange { sheet, range, .. } => {
            check_sheet_action(wb, *sheet, |p| p.sort)?;
            check_edit(wb, &[to_range(*sheet, range)?])?;
        }
        Op::FillRange { sheet, target, .. } => check_edit(wb, &[to_range(*sheet, target)?])?,
        _ => {}
    }
    Ok(())
}

/// The destination of a move.
struct BlockDest {
    from: CellRange,
    to: tally_api::CellPos,
}

impl BlockDest {
    fn range(&self) -> Option<CellRange> {
        let r1 = u64::from(self.to.r) + u64::from(self.from.r1 - self.from.r0);
        let c1 = u64::from(self.to.c) + u64::from(self.from.c1 - self.from.c0);
        (r1 < u64::from(MAX_ROWS) && c1 < u64::from(MAX_COLS))
            .then(|| CellRange::new(self.from.sheet, self.to.r, self.to.c, r1 as u32, c1 as u32))
    }
}

/// Undoes `changes` (an op that must not stand).
pub(crate) fn revert(wb: &mut Workbook, changes: Vec<Change>, fx: &mut Effects) {
    for ch in changes.into_iter().rev() {
        let _ = apply_change(wb, ch, fx);
    }
}

/// Checks made after an op wrote cells: protection of what a paste or
/// replace wrote, and data validation of typed and pasted values.
fn postcheck(
    wb: &mut Workbook,
    op: &Op,
    changes: Vec<Change>,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let written: Vec<CellRange> = changes
        .iter()
        .flat_map(|c| match c {
            Change::Range(s) if s.contents.is_some() => vec![s.range],
            Change::Cells(list) if matches!(op, Op::ReplaceAll { .. }) => {
                list.iter().map(|(a, _)| CellRange::single(*a)).collect()
            }
            _ => Vec::new(),
        })
        .collect();
    if matches!(
        op,
        Op::Paste { .. } | Op::PasteSpecial { .. } | Op::ReplaceAll { .. }
    ) {
        if let Err(e) = crate::protection::check_edit(wb, &written) {
            revert(wb, changes, fx);
            return Err(e);
        }
    }
    let validate = matches!(
        op,
        Op::SetCellInput { .. }
            | Op::SetRangeInputs { .. }
            | Op::Paste { .. }
            | Op::PasteSpecial { .. }
            | Op::FillRange { .. }
    );
    if validate {
        // Typed input: the cells as typed (even when now empty) and the
        // text typed, for blank and text-length rules (review 3, M13/LOW).
        let mut typed: BTreeMap<SheetId, std::collections::HashMap<(u32, u32), String>> =
            BTreeMap::new();
        match op {
            Op::SetCellInput { sheet, r, c, input } => {
                typed
                    .entry(*sheet)
                    .or_default()
                    .insert((*r, *c), input.clone());
            }
            Op::SetRangeInputs { sheet, r, c, rows } => {
                let m = typed.entry(*sheet).or_default();
                for (dr, row) in rows.iter().enumerate() {
                    for (dc, text) in row.iter().enumerate() {
                        let (rr, cc) = (r.saturating_add(dr as u32), c.saturating_add(dc as u32));
                        if rr < MAX_ROWS && cc < MAX_COLS {
                            m.insert((rr, cc), text.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        let mut by_sheet: BTreeMap<SheetId, Vec<(u32, u32)>> = BTreeMap::new();
        for (sheet, m) in &typed {
            if wb
                .sheet(*sheet)
                .is_some_and(|s| !s.meta.validations.is_empty())
            {
                by_sheet
                    .entry(*sheet)
                    .or_default()
                    .extend(m.keys().copied());
            }
        }
        for r in &written {
            let Some(s) = wb.sheet(r.sheet) else {
                continue;
            };
            if s.meta.validations.is_empty() {
                continue;
            }
            let cells = by_sheet.entry(r.sheet).or_default();
            for v in s.meta.validations.iter() {
                for vr in &v.ranges {
                    let vr = CellRange::new(r.sheet, vr.r0, vr.c0, vr.r1, vr.c1);
                    let Some(x) = vr.intersect(r) else { continue };
                    for c in x.c0..=x.c1.min(s.columns.len().saturating_sub(1) as u32) {
                        if let Some(col) = s.column(c) {
                            use crate::storage::ColumnStore;
                            cells.extend(col.cells.iter_range(x.r0, x.r1).map(|(row, _)| (row, c)));
                        }
                    }
                }
            }
        }
        for (sheet, mut cells) in by_sheet {
            cells.sort_unstable();
            cells.dedup();
            let checked = match typed.get(&sheet) {
                Some(inputs) => crate::validation::check_cells_inputs(wb, sheet, &cells, inputs),
                None => crate::validation::check_cells(wb, sheet, &cells),
            };
            match checked {
                Ok(n) => fx.notices.extend(n),
                Err(e) => {
                    revert(wb, changes, fx);
                    return Err(e);
                }
            }
        }
    }
    Ok(changes)
}

fn apply_op_inner(
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
            wb.set_typed_input(CellAddr::new(*sheet, r, c), input);
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
            let mut inv = Vec::with_capacity(ranges.len() + 1);
            // "Delete all" also removes notes and hyperlinks (Calc); undo
            // keeps the removed entries, not the whole maps.
            if *what == ClearWhat::All {
                let (note_keys, link_keys): (CellKeys, CellKeys) = wb
                    .sheet(*sheet)
                    .map(|s| {
                        (
                            ranges
                                .iter()
                                .flat_map(|r| crate::annotations::entries_in(&s.meta.notes, r))
                                .map(|(k, _)| k)
                                .collect(),
                            ranges
                                .iter()
                                .flat_map(|r| crate::annotations::entries_in(&s.meta.links, r))
                                .map(|(k, _)| k)
                                .collect(),
                        )
                    })
                    .unwrap_or_default();
                if !note_keys.is_empty() || !link_keys.is_empty() {
                    if let Some(s) = wb.sheet_mut(*sheet) {
                        let mut parts = Vec::new();
                        if !note_keys.is_empty() {
                            parts.push(crate::storage::MetaPart::NoteEntries(
                                crate::storage::map_entries(&s.meta.notes, &note_keys),
                            ));
                            let map = Arc::make_mut(&mut s.meta.notes);
                            for k in &note_keys {
                                map.remove(k);
                            }
                        }
                        if !link_keys.is_empty() {
                            parts.push(crate::storage::MetaPart::LinkEntries(
                                crate::storage::map_entries(&s.meta.links, &link_keys),
                            ));
                            let map = Arc::make_mut(&mut s.meta.links);
                            for k in &link_keys {
                                map.remove(k);
                            }
                        }
                        inv.push(Change::Meta {
                            sheet: *sheet,
                            parts,
                        });
                    }
                    wb.pending.display_only.extend(ranges.iter().copied());
                    fx.layout(*sheet);
                }
                // "Delete all" removes the cell attributes, conditional
                // formats and validations included (AOO `IDF_ATTRIB`,
                // review 3, LOW).
                let touches = wb.sheet(*sheet).is_some_and(|s| {
                    ranges.iter().any(|r| {
                        let l = local(r);
                        s.meta.cond_formats.iter().any(|cf| {
                            cf.ranges
                                .iter()
                                .any(|x| crate::condfmt::intersect_rect(x, &l).is_some())
                        }) || s.meta.validations.iter().any(|v| {
                            v.ranges
                                .iter()
                                .any(|x| crate::condfmt::intersect_rect(x, &l).is_some())
                        })
                    })
                });
                if touches {
                    if let Some(parts) =
                        meta_parts(wb, *sheet, &[MetaKind::CondFormats, MetaKind::Validations])
                    {
                        inv.push(parts);
                    }
                    if let Some(s) = wb.sheet_mut(*sheet) {
                        for r in &ranges {
                            crate::condfmt::remove_area(&mut s.meta, &local(r));
                        }
                    }
                    fx.layout(*sheet);
                }
            }
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
                check_format_len(code)?;
                parse_format(code)
                    .map_err(|e| ApiError::invalid_argument(format!("bad number format: {e}")))?;
            }
            let ranges = ranges_of(*sheet, ranges)?;
            let mut inv = Vec::with_capacity(ranges.len());
            // The UI shows the document font as the family of unstyled
            // cells; choosing it (or clearing the family) means "the
            // document font", so it is not baked into the styles.
            let doc_font =
                crate::fonts::patch_picks_document_font(patch, wb.default_font.as_deref());
            for range in ranges {
                inv.push(Change::Range(capture_range(wb, range, false, true)));
                map_styles(wb, range, |t, old| {
                    let id = t.patched(old, patch);
                    if doc_font {
                        t.without_family(id)
                    } else {
                        id
                    }
                });
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
            check_format_len(code)?;
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
            let mut out = vec![Change::RemoveSheet { id }];
            out.extend(resolve_sheet_refs(wb, id));
            Ok(out)
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
            out.extend(resolve_sheet_refs(wb, *sheet));
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
            // Rule formulas of the other sheets that point at this one are
            // restored by undo (LOW-12): their collections are cheap
            // `Arc` snapshots.
            let other_metas: Vec<Change> = wb
                .sheets
                .iter()
                .filter(|s| {
                    s.id != *sheet
                        && (!s.meta.cond_formats.is_empty() || !s.meta.validations.is_empty())
                })
                .map(|s| s.id)
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|id| {
                    meta_parts(wb, id, &[MetaKind::CondFormats, MetaKind::Validations])
                })
                .collect();
            // Formulas elsewhere that reference the sheet become #REF!.
            let change = StructureChange::DeleteSheet { sheet: *sheet };
            let mut adjusted: Vec<(FormulaCellId, CellAddr)> = Vec::new();
            for (id, f) in &wb.formulas {
                if f.addr.sheet != *sheet && wb.reads_sheet(f, &[*sheet]) {
                    adjusted.push((id, f.addr));
                }
            }
            // 3-D spans with an end on the sheet shrink to its neighbour
            // (AOO `UpdateDeleteTab`) instead of becoming #REF!.
            let order: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
            let mut old_cells = Vec::with_capacity(adjusted.len());
            for (id, addr) in adjusted {
                let Some(f) = wb.formulas.get(id) else {
                    continue;
                };
                let mut nf = (*f.formula).clone();
                let moved = retarget_3d_for_sheet_delete(&mut nf, addr.sheet, *sheet, &order);
                if adjust_for_structure_change(&mut nf, addr, &change) | moved {
                    let old = wb.replace_formula_ast(addr, Arc::new(nf));
                    old_cells.push((addr, old));
                }
            }
            // Names scoped to the sheet go; names pointing at it become #REF!.
            wb.names.undefine_scope(*sheet);
            for def in wb.names.defs_mut() {
                let mut f = (*def.formula).clone();
                let moved = retarget_3d_for_sheet_delete(&mut f, def.base.sheet, *sheet, &order);
                if adjust_for_structure_change(&mut f, def.base, &change) | moved {
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
            let mut out = vec![names];
            out.extend(other_metas);
            out.extend([Change::Cells(old_cells), ins]);
            Ok(out)
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
            // Names are workbook state: allowed unless the document
            // structure is protected (LOW-9).
            crate::protection::check_structure(wb)?;
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
            let inv = crate::undo::names_snapshot_tx(wb, fx);
            wb.names.define(name, *scope, def);
            wb.refresh_name_dependents(&crate::names::name_key(name));
            Ok(inv.into_iter().collect())
        }
        Op::DeleteName { name, scope } => {
            crate::protection::check_structure(wb)?;
            let id = wb
                .names
                .lookup(name, *scope)
                .filter(|id| wb.names.get(*id).is_some_and(|e| e.def.is_some()))
                .ok_or_else(|| ApiError::not_found(format!("no name {name:?}")))?;
            let inv = crate::undo::names_snapshot_tx(wb, fx);
            wb.names.undefine(id);
            wb.refresh_name_dependents(&crate::names::name_key(name));
            Ok(inv.into_iter().collect())
        }
        Op::SetNameUsage { name, scope, usage } => {
            // The flags only list the name in dialogs (AOO `ScRangeData`
            // types); no formula depends on them.
            crate::protection::check_structure(wb)?;
            let id = wb
                .names
                .lookup(name, *scope)
                .filter(|id| wb.names.get(*id).is_some_and(|e| e.def.is_some()))
                .ok_or_else(|| ApiError::not_found(format!("no name {name:?}")))?;
            let usage = tally_model::NameUsage {
                print_range: usage.print_range,
                filter: usage.filter,
                repeat_row: usage.repeat_row,
                repeat_column: usage.repeat_column,
            };
            if wb.names.get(id).is_some_and(|e| e.usage == usage) {
                return Ok(Vec::new());
            }
            let inv = crate::undo::names_snapshot_tx(wb, fx);
            wb.names.set_usage(id, usage);
            Ok(inv.into_iter().collect())
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
            let meta = meta_parts(wb, *sheet, &[MetaKind::Merges])
                .ok_or_else(|| not_found_sheet(*sheet))?;
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
            // The covered cells lose their contents (AOO asks first; the
            // UI prompts, the result reports what was cleared; review 3, M3).
            let cleared = cells.iter().filter(|a| !wb.value(**a).is_empty()).count();
            for a in cells {
                wb.clear_content(a);
            }
            if cleared > 0 {
                fx.notices.push(ApiError::new(
                    "merge_cleared",
                    format!(
                        "Merging cleared the contents of {cleared} covered cell{}.",
                        if cleared == 1 { "" } else { "s" }
                    ),
                ));
            }
            if let Some(s) = wb.sheet_mut(*sheet) {
                let l = local(&range);
                let merges = Arc::make_mut(&mut s.meta.merges);
                merges.retain(|m| !(l.r0 <= m.r0 && m.r1 <= l.r1 && l.c0 <= m.c0 && m.c1 <= l.c1));
                merges.push(l);
            }
            wb.pending.ranges.push(range);
            fx.layout(*sheet);
            Ok(vec![meta, Change::Range(content)])
        }
        Op::UnmergeCells { sheet, range } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            let meta = meta_parts(wb, *sheet, &[MetaKind::Merges])
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let mut hit = Vec::new();
            if let Some(s) = wb.sheet_mut(*sheet) {
                let touched = s
                    .meta
                    .merges
                    .iter()
                    .any(|m| CellRange::new(*sheet, m.r0, m.c0, m.r1, m.c1).intersects(&range));
                if !touched {
                    return Ok(Vec::new());
                }
                Arc::make_mut(&mut s.meta.merges).retain(|m| {
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
            let meta = meta_parts(wb, *sheet, &[MetaKind::Frozen])
                .ok_or_else(|| not_found_sheet(*sheet))?;
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
            let meta = meta_parts(wb, *sheet, &[MetaKind::AutoFilter])
                .ok_or_else(|| not_found_sheet(*sheet))?;
            if let Some(s) = wb.sheet_mut(*sheet) {
                s.meta.auto_filter = range.map(|r| {
                    Arc::new(AutoFilter {
                        range: local(&r),
                        columns: BTreeMap::new(),
                    })
                });
            }
            wb.reapply_filter(*sheet);
            fx.layout(*sheet);
            Ok(vec![meta])
        }
        Op::SetColumnFilter { sheet, col, filter } => {
            check_sheet(wb, *sheet)?;
            if let Some(f) = filter {
                crate::filter::check_filter_size(f)?;
            }
            let meta = meta_parts(wb, *sheet, &[MetaKind::AutoFilter])
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let s = wb
                .sheet_mut(*sheet)
                .ok_or_else(|| not_found_sheet(*sheet))?;
            let af = s
                .meta
                .auto_filter
                .as_mut()
                .map(Arc::make_mut)
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
            options,
        } => {
            check_sheet(wb, *sheet)?;
            let range = to_range(*sheet, range)?;
            let mut changes = crate::sort::sort_range_with(
                wb,
                range,
                keys,
                *has_header,
                *case_sensitive,
                options.as_ref(),
            )?;
            changes.extend(crate::tools::remember_sort(
                wb,
                range,
                keys,
                *has_header,
                *case_sensitive,
                options.as_ref(),
                fx,
            )?);
            Ok(changes)
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
        // Handled by `ops_v03::apply`, `ops_v04::apply` or `ops_v05::apply`.
        _ => Err(ApiError::new(ApiError::INTERNAL, "unhandled op")),
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
    // Rule formulas on other sheets may point at this one.
    let other_metas: Vec<Change> = wb
        .sheets
        .iter()
        .filter(|s| {
            s.id != shape.sheet
                && (!s.meta.cond_formats.is_empty()
                    || !s.meta.validations.is_empty()
                    || !s.meta.charts.is_empty())
        })
        .map(|s| s.id)
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|id| {
            meta_parts(
                wb,
                id,
                &[
                    MetaKind::CondFormats,
                    MetaKind::Validations,
                    MetaKind::Charts,
                ],
            )
        })
        .collect();
    let lost = Change::Range(capture_range(wb, shape.lost(), true, true));
    let old = wb.structure_change(&change);
    wb.adjust_meta_for_structure(&change);
    fx.layout(shape.sheet);
    let mut out = vec![names, meta];
    out.extend(other_metas);
    out.extend([
        dims,
        lost,
        Change::Cells(old),
        Change::Structure(shape.inverse()),
    ]);
    Ok(out)
}
