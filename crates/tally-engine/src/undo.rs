//! Undo/redo (plan §4.4): every transaction records the inverse of what it
//! did as low-level [`Change`]s (snapshots of the affected cells, styles,
//! dimensions or sheets). Applying a change returns its own inverse, so undo
//! produces the redo record and vice versa.

use std::sync::Arc;

use tally_api::SelectionDto;
use tally_types::{CellAddr, CellRange, SheetId, MAX_ROWS};

use crate::storage::{CellContent, ColumnStore, DimRuns, MetaKind, MetaPart, Sheet, StyleId};
use crate::workbook::{SnapContent, Workbook};

/// Undo stack depth (the OOo default).
pub const UNDO_DEPTH: usize = 100;

/// Style runs `(first row, last row, style)` of one column.
pub type ColumnRuns = (u32, Vec<(u32, u32, StyleId)>);

/// Content and/or styles of a rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeSnapshot {
    /// The rectangle.
    pub range: CellRange,
    /// Non-empty cells `(row, col, content)` when content was captured.
    pub contents: Option<Vec<(u32, u32, SnapContent)>>,
    /// Style runs per column when styles were captured.
    pub styles: Option<Vec<ColumnRuns>>,
}

/// One invertible low-level change.
#[derive(Clone, Debug)]
pub enum Change {
    /// Restore a rectangle.
    Range(RangeSnapshot),
    /// Restore individual cells' content.
    Cells(Vec<(CellAddr, Option<SnapContent>)>),
    /// Restore column widths.
    ColDims {
        /// Sheet.
        sheet: SheetId,
        /// Widths.
        dims: DimRuns,
    },
    /// Restore row heights.
    RowDims {
        /// Sheet.
        sheet: SheetId,
        /// Heights.
        dims: DimRuns,
    },
    /// Remove a sheet.
    RemoveSheet {
        /// Sheet.
        id: SheetId,
    },
    /// Re-insert a removed sheet with its id.
    InsertSheet {
        /// The sheet without its formula cells.
        sheet: Box<Sheet>,
        /// Tab index.
        index: usize,
        /// Its formula cells.
        formulas: Vec<(CellAddr, SnapContent)>,
        /// It was the active sheet.
        active: bool,
    },
    /// Rename a sheet.
    RenameSheet {
        /// Sheet.
        id: SheetId,
        /// Name.
        name: Arc<str>,
    },
    /// Move a sheet to a tab index.
    MoveSheet {
        /// Sheet.
        id: SheetId,
        /// Tab index.
        to: usize,
    },
    /// Insert or delete rows/columns. Its inverse is the opposite change;
    /// the records that use it also carry snapshots of the lost cells, the
    /// dimensions and the rewritten formulas, which undo restores afterwards.
    Structure(tally_formula::StructureChange),
    /// Restore parts of a sheet's meta (merges, frozen panes, the
    /// AutoFilter, conditional formats, notes, …): only the collections an
    /// op changed, as `Arc` snapshots or per-entry lists (see
    /// [`crate::storage::SheetMeta`]). The filtered rows are recomputed
    /// when the AutoFilter is restored.
    Meta {
        /// Sheet.
        sheet: SheetId,
        /// The parts, each restoring one field; applied last to first.
        parts: Vec<MetaPart>,
    },
    /// Restore the whole name table (dependents are refreshed).
    Names(Box<crate::names::NameTable>),
    /// Restore the document metadata (properties).
    DocMeta(Box<tally_model::DocMeta>),
    /// Restore the document structure protection.
    DocProtection(Option<tally_model::SheetProtectionData>),
    /// Restore the rows an AutoFilter hides.
    FilteredRows {
        /// Sheet.
        sheet: SheetId,
        /// The rows.
        rows: crate::storage::Spans,
    },
}

/// Side effects the API reports besides changed cells.
#[derive(Clone, Debug, Default)]
pub struct Effects {
    /// Sheets whose layout changed.
    pub layout_changed: Vec<SheetId>,
    /// Sheets were added, removed, renamed or moved.
    pub sheets_changed: bool,
    /// Non-fatal problems of a successful op (e.g. replacements that would
    /// not parse), reported as `OpError`s with the op's index.
    pub warnings: Vec<tally_api::ApiError>,
    /// Notices for `ApplyResult.warnings` (validation warnings and
    /// information messages of accepted input).
    pub notices: Vec<tally_api::ApiError>,
    /// Row heights computed by `AutoFitRows`.
    pub fitted_rows: Option<Vec<tally_api::DimRunPx>>,
    /// Cells changed by `ReplaceAll` ops.
    pub replaced: Option<u32>,
    /// `notices` with their op index (collected by the document).
    pub notice_errors: Vec<tally_api::OpError>,
    /// Charts whose data changed (v0.4; filled by the chart index).
    pub charts_changed: Option<Vec<tally_api::ChartRef>>,
    /// Sheets whose chart/picture list changed (v0.4).
    pub objects_changed: Option<Vec<SheetId>>,
    /// Ids allocated by `Add*`/`Create*` ops, in op order (v0.4; the op
    /// index is filled in by the document).
    pub created: Option<Vec<tally_api::CreatedId>>,
    /// The pivot list changed (v0.5).
    pub pivots_changed: Option<bool>,
    /// Per sheet, the [`MetaKind`]s whose whole-collection snapshot this
    /// transaction already holds in an earlier [`Change::Meta`] (a bit
    /// mask of [`MetaKind::bit`]). Later ops of the same transaction skip
    /// re-recording them: undo restores the earliest state anyway, and
    /// not holding the `Arc` lets `Arc::make_mut` edit in place.
    pub meta_recorded: std::collections::HashMap<SheetId, u32>,
    /// A protection change happened: the undo and redo stacks are cleared
    /// after the transaction (protection cannot be undone around; Excel
    /// and AOO do the same).
    pub clear_undo: bool,
}

impl Effects {
    /// Records a layout change.
    pub fn layout(&mut self, sheet: SheetId) {
        if !self.layout_changed.contains(&sheet) {
            self.layout_changed.push(sheet);
        }
    }

    /// Records that a sheet's chart/picture list changed (v0.4).
    pub fn objects(&mut self, sheet: SheetId) {
        let v = self.objects_changed.get_or_insert_with(Vec::new);
        if !v.contains(&sheet) {
            v.push(sheet);
        }
    }

    /// Records an id allocated by the op at `op_index` (v0.4).
    pub fn created(&mut self, op_index: u32, id: u32) {
        self.created
            .get_or_insert_with(Vec::new)
            .push(tally_api::CreatedId { op_index, id });
    }

    /// `true` if this transaction already holds a whole-collection
    /// snapshot of `kind` for `sheet`.
    pub fn meta_recorded(&self, sheet: SheetId, kind: MetaKind) -> bool {
        self.meta_recorded
            .get(&sheet)
            .is_some_and(|m| m & kind.bit() != 0)
    }

    /// Marks `kind` of `sheet` as recorded in this transaction.
    pub fn record_meta(&mut self, sheet: SheetId, kind: MetaKind) {
        *self.meta_recorded.entry(sheet).or_default() |= kind.bit();
    }
}

/// One undoable step. `changes` are applied in reverse order.
#[derive(Clone, Debug)]
pub struct UndoRecord {
    /// Menu label.
    pub label: Option<String>,
    /// Selection before the transaction.
    pub selection_before: Option<SelectionDto>,
    /// Selection after the transaction.
    pub selection_after: Option<SelectionDto>,
    /// The inverse changes, applied last to first.
    pub changes: Vec<Change>,
}

/// Undo and redo stacks.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<UndoRecord>,
    redo: Vec<UndoRecord>,
}

impl UndoStack {
    /// Pushes a new step and clears redo.
    pub fn push(&mut self, rec: UndoRecord) {
        self.undo.push(rec);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Pops the next undo step.
    pub fn pop_undo(&mut self) -> Option<UndoRecord> {
        self.undo.pop()
    }

    /// Pops the next redo step.
    pub fn pop_redo(&mut self) -> Option<UndoRecord> {
        self.redo.pop()
    }

    /// Pushes onto undo without clearing redo (after a redo).
    pub fn push_undo_keep_redo(&mut self, rec: UndoRecord) {
        self.undo.push(rec);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Pushes onto redo (after an undo).
    pub fn push_redo(&mut self, rec: UndoRecord) {
        self.redo.push(rec);
    }

    /// The API state.
    pub fn state(&self) -> tally_api::UndoState {
        tally_api::UndoState {
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            undo_label: self.undo.last().and_then(|r| r.label.clone()),
            redo_label: self.redo.last().and_then(|r| r.label.clone()),
        }
    }

    /// Number of undo steps.
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// The undo records, oldest first (tests inspect what they hold).
    pub fn undo_records(&self) -> &[UndoRecord] {
        &self.undo
    }

    /// Drops both stacks.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Captures a rectangle.
pub fn capture_range(wb: &Workbook, range: CellRange, content: bool, style: bool) -> RangeSnapshot {
    let mut contents = content.then(Vec::new);
    let mut styles = style.then(Vec::new);
    if let Some(sheet) = wb.sheet(range.sheet) {
        let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
        if !sheet.columns.is_empty() {
            for c in range.c0..=c1 {
                let Some(col) = sheet.column(c) else {
                    continue;
                };
                if let Some(out) = contents.as_mut() {
                    for (r, _) in col.cells.iter_range(range.r0, range.r1) {
                        if let Some(s) = wb.snapshot_content(CellAddr::new(range.sheet, r, c)) {
                            out.push((r, c, s));
                        }
                    }
                }
                if let Some(out) = styles.as_mut() {
                    let runs = col.styles.runs_in(range.r0, range.r1);
                    if !runs.is_empty() {
                        out.push((c, runs));
                    }
                }
            }
        }
    }
    RangeSnapshot {
        range,
        contents,
        styles,
    }
}

/// Clears the content of every non-empty cell in `range`.
pub fn clear_range_content(wb: &mut Workbook, range: CellRange) {
    let Some(sheet) = wb.sheet(range.sheet) else {
        return;
    };
    let mut cells: Vec<(u32, u32)> = Vec::new();
    let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
    if !sheet.columns.is_empty() {
        for c in range.c0..=c1 {
            if let Some(col) = sheet.column(c) {
                cells.extend(
                    col.cells
                        .iter_range(range.r0, range.r1)
                        .map(|(r, _)| (r, c)),
                );
            }
        }
    }
    for (r, c) in cells {
        wb.clear_content(CellAddr::new(range.sheet, r, c));
    }
    wb.pending.ranges.push(range);
}

/// Resets the styles of `range` to the default.
pub fn clear_range_styles(wb: &mut Workbook, range: CellRange) {
    let Some(sheet) = wb.sheet_mut(range.sheet) else {
        return;
    };
    let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
    if sheet.columns.is_empty() {
        return;
    }
    for c in range.c0..=c1 {
        if let Some(col) = sheet.columns.get_mut(c as usize) {
            col.styles.set(range.r0, range.r1, StyleId::DEFAULT);
        }
    }
    wb.pending.display_only.push(range);
}

fn restore_range(wb: &mut Workbook, snap: RangeSnapshot) {
    let range = snap.range;
    if let Some(contents) = snap.contents {
        clear_range_content(wb, range);
        for (r, c, content) in contents {
            wb.restore_content(CellAddr::new(range.sheet, r, c), Some(content));
        }
    }
    if let Some(styles) = snap.styles {
        clear_range_styles(wb, range);
        if let Some(sheet) = wb.sheet_mut(range.sheet) {
            for (c, runs) in styles {
                if let Some(col) = sheet.column_mut(c) {
                    for (s, e, id) in runs {
                        col.styles.set(s, e, id);
                    }
                }
            }
        }
    }
}

fn whole_sheet(id: SheetId) -> CellRange {
    CellRange::new(id, 0, 0, MAX_ROWS - 1, tally_types::MAX_COLS - 1)
}

/// Removes a sheet, returning the change that re-inserts it.
pub fn remove_sheet(wb: &mut Workbook, id: SheetId, fx: &mut Effects) -> Option<Change> {
    let index = wb.tab_of(id)?;
    let active = wb.active_sheet == id;
    let mut formulas = Vec::new();
    if let Some(sheet) = wb.sheet(id) {
        for (ci, col) in sheet.columns.iter().enumerate() {
            for r in &col.formula_rows {
                let addr = CellAddr::new(id, *r, ci as u32);
                if let Some(s) = wb.snapshot_content(addr) {
                    formulas.push((addr, s));
                }
            }
        }
    }
    let mut sheet = wb.remove_sheet(id)?;
    for (addr, _) in &formulas {
        sheet.remove(addr.row, addr.col);
    }
    wb.pending.ranges.push(whole_sheet(id));
    wb.refresh_all_precedents();
    fx.sheets_changed = true;
    Some(Change::InsertSheet {
        sheet: Box::new(sheet),
        index,
        formulas,
        active,
    })
}

/// Applies a change and returns its inverse.
pub fn apply_change(wb: &mut Workbook, change: Change, fx: &mut Effects) -> Option<Change> {
    match change {
        Change::Range(snap) => {
            let inv = capture_range(
                wb,
                snap.range,
                snap.contents.is_some(),
                snap.styles.is_some(),
            );
            restore_range(wb, snap);
            Some(Change::Range(inv))
        }
        Change::Cells(list) => {
            let inv: Vec<(CellAddr, Option<SnapContent>)> = list
                .iter()
                .map(|(a, _)| (*a, wb.snapshot_content(*a)))
                .collect();
            for (a, c) in list {
                wb.restore_content(a, c);
            }
            Some(Change::Cells(inv))
        }
        Change::ColDims { sheet, dims } => {
            let s = wb.sheet_mut(sheet)?;
            let old = std::mem::replace(&mut s.col_dims, dims);
            fx.layout(sheet);
            Some(Change::ColDims { sheet, dims: old })
        }
        Change::RowDims { sheet, dims } => {
            let s = wb.sheet_mut(sheet)?;
            let old = std::mem::replace(&mut s.row_dims, dims);
            fx.layout(sheet);
            Some(Change::RowDims { sheet, dims: old })
        }
        Change::RemoveSheet { id } => remove_sheet(wb, id, fx),
        Change::InsertSheet {
            sheet,
            index,
            formulas,
            active,
        } => {
            let id = sheet.id;
            wb.insert_sheet(*sheet, index);
            if active {
                wb.active_sheet = id;
            }
            for (addr, content) in formulas {
                wb.restore_content(addr, Some(content));
            }
            wb.pending.ranges.push(whole_sheet(id));
            wb.refresh_all_precedents();
            fx.sheets_changed = true;
            Some(Change::RemoveSheet { id })
        }
        Change::RenameSheet { id, name } => {
            let s = wb.sheet_mut(id)?;
            let old = std::mem::replace(&mut s.name, name);
            fx.sheets_changed = true;
            Some(Change::RenameSheet { id, name: old })
        }
        Change::Structure(ch) => {
            let shape = crate::structure::Shape::of(&ch)?;
            wb.structure_change(&ch);
            fx.layout(shape.sheet);
            Some(Change::Structure(shape.inverse()))
        }
        Change::MoveSheet { id, to } => {
            let from = wb.move_sheet(id, to)?;
            wb.refresh_all_precedents();
            fx.sheets_changed = true;
            Some(Change::MoveSheet { id, to: from })
        }
        Change::Meta { sheet, parts } => {
            let s = wb.sheet_mut(sheet)?;
            let mut filter_changed = false;
            let mut inv = Vec::with_capacity(parts.len());
            for p in parts.into_iter().rev() {
                filter_changed |=
                    p.kind() == Some(MetaKind::AutoFilter) && !p.unchanged_in(&s.meta);
                inv.push(p.swap_into(&mut s.meta));
            }
            // Only a change of the filter itself re-applies it.
            if filter_changed {
                wb.reapply_filter(sheet);
            }
            wb.pending.display_only.push(whole_sheet(sheet));
            fx.layout(sheet);
            Some(Change::Meta { sheet, parts: inv })
        }
        Change::Names(table) => {
            let old = std::mem::replace(&mut wb.names, *table);
            wb.refresh_all_name_dependents();
            Some(Change::Names(Box::new(old)))
        }
        Change::DocMeta(meta) => {
            let old = std::mem::replace(&mut wb.meta, *meta);
            Some(Change::DocMeta(Box::new(old)))
        }
        Change::FilteredRows { sheet, rows } => {
            let s = wb.sheet_mut(sheet)?;
            let old = std::mem::replace(&mut s.filtered_rows, rows);
            fx.layout(sheet);
            Some(Change::FilteredRows { sheet, rows: old })
        }
        Change::DocProtection(p) => {
            let old = std::mem::replace(&mut wb.structure_protection, p);
            fx.sheets_changed = true;
            Some(Change::DocProtection(old))
        }
    }
}

/// A snapshot of a sheet's whole meta state (for undo): every field as an
/// `Arc` snapshot. Cheap to take, but the op's edits then copy each
/// collection they touch once (copy-on-write), so prefer [`meta_parts`]
/// with the fields the op changes.
pub fn sheet_meta(wb: &Workbook, sheet: SheetId) -> Option<Change> {
    Some(Change::Meta {
        sheet,
        parts: wb.sheet(sheet)?.meta.all_parts(),
    })
}

/// A snapshot of the given fields of a sheet's meta (for undo).
pub fn meta_parts(wb: &Workbook, sheet: SheetId, kinds: &[MetaKind]) -> Option<Change> {
    let meta = &wb.sheet(sheet)?.meta;
    Some(Change::Meta {
        sheet,
        parts: kinds.iter().map(|k| meta.part(*k)).collect(),
    })
}

/// An undo entry restoring the given entries of a sheet's notes or links.
pub fn meta_entries(sheet: SheetId, part: MetaPart) -> Change {
    Change::Meta {
        sheet,
        parts: vec![part],
    }
}

/// A snapshot of the name table (for undo).
pub fn names_snapshot(wb: &Workbook) -> Change {
    Change::Names(Box::new(wb.names.clone()))
}

/// `true` if the content of a cell is a formula (used by tests).
pub fn is_formula(wb: &Workbook, addr: CellAddr) -> bool {
    matches!(
        wb.sheet(addr.sheet).and_then(|s| s.get(addr.row, addr.col)),
        Some(CellContent::Formula(_))
    )
}
