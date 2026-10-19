//! One open document: the workbook, its undo stack, version and file state.

use std::path::PathBuf;

use tally_api::{
    ApplyResult, DocId, OpError, RangeDto, SelectionDto, SheetInfo, SheetRanges, Transaction,
    WorkbookInfo,
};
use tally_types::{CellRange, SheetId};

use crate::clipboard::ClipSnapshot;
use crate::ops::apply_op;
use crate::recalc::{hard_recalc_with, recalc, RecalcBudget, RecalcReport};
use crate::undo::{apply_change, Effects, UndoRecord, UndoStack};
use crate::workbook::Workbook;

/// An open document.
#[derive(Clone, Debug)]
pub struct Document {
    /// Handle.
    pub id: DocId,
    /// The workbook.
    pub wb: Workbook,
    /// Undo/redo.
    pub undo: UndoStack,
    /// Monotonic version.
    pub version: u64,
    /// File path, if any.
    pub path: Option<PathBuf>,
    /// Window title.
    pub title: String,
    /// Unsaved changes exist.
    pub dirty: bool,
    /// The recalc on open hit its [`RecalcBudget`]; some formulas show the
    /// values cached in the file.
    pub recalc_incomplete: bool,
    /// Warnings for the next [`Self::info`] (taken by the service once).
    pub warnings: Vec<String>,
}

/// Formats a colour `0xRRGGBBAA` as `#rrggbb`.
fn css_color(c: u32) -> String {
    format!("#{:06x}", c >> 8)
}

impl Document {
    /// Wraps a workbook (recalculating it fully).
    pub fn new(id: DocId, wb: Workbook, title: &str) -> Self {
        Self::with_budget(id, wb, title, &RecalcBudget::unbounded())
    }

    /// Wraps a workbook, recalculating it fully within `budget`. If the
    /// budget runs out, the remaining formulas keep their cached values,
    /// [`Self::recalc_incomplete`] is set and a warning is queued.
    pub fn with_budget(id: DocId, mut wb: Workbook, title: &str, budget: &RecalcBudget) -> Self {
        let report = hard_recalc_with(&mut wb, budget);
        let mut warnings = Vec::new();
        if report.incomplete {
            warnings.push(format!(
                "Recalculation stopped after {} formulas to keep the file responsive; \
                 {} formulas show the values saved in the file and update when you \
                 edit the cells they depend on.",
                report.evaluated, report.skipped
            ));
        }
        Self {
            id,
            wb,
            undo: UndoStack::default(),
            version: 1,
            path: None,
            title: title.to_owned(),
            dirty: false,
            recalc_incomplete: report.incomplete,
            warnings,
        }
    }

    /// Evaluates formula `text` at `addr` without committing it
    /// (see [`Workbook::eval_formula_text_at`]).
    pub fn eval_formula_text_at(
        &self,
        addr: tally_types::CellAddr,
        text: &str,
    ) -> Option<tally_types::Value> {
        self.wb.eval_formula_text_at(addr, text)
    }

    /// The API summary.
    pub fn info(&self) -> WorkbookInfo {
        WorkbookInfo {
            doc: self.id,
            title: self.title.clone(),
            path: self.path.as_ref().map(|p| p.display().to_string()),
            sheets: self
                .wb
                .sheets
                .iter()
                .map(|s| SheetInfo {
                    id: s.id,
                    name: s.name.to_string(),
                    tab_color: s.tab_color.map(css_color),
                    hidden: s.hidden,
                })
                .collect(),
            active_sheet: self.wb.active_sheet,
            dirty: self.dirty,
            version: self.version,
            warnings: self.warnings.clone(),
        }
    }

    fn result(
        &mut self,
        report: RecalcReport,
        fx: Effects,
        errors: Vec<OpError>,
        selection: Option<SelectionDto>,
    ) -> ApplyResult {
        self.version += 1;
        let mut by_sheet: Vec<SheetRanges> = Vec::new();
        for r in report.changed {
            if self.wb.sheet(r.sheet).is_none() {
                continue;
            }
            let dto = range_dto(&r);
            match by_sheet.iter_mut().find(|s| s.sheet == r.sheet) {
                Some(s) => s.ranges.push(dto),
                None => by_sheet.push(SheetRanges {
                    sheet: r.sheet,
                    ranges: vec![dto],
                }),
            }
        }
        if let Some(sel) = &selection {
            if self.wb.sheet(sel.sheet).is_some() {
                self.wb.active_sheet = sel.sheet;
            }
        }
        if self.wb.sheet(self.wb.active_sheet).is_none() {
            self.wb.active_sheet = self.wb.sheets.first().map_or(0, |s| s.id);
        }
        let layout_changed: Vec<SheetId> = fx
            .layout_changed
            .into_iter()
            .filter(|s| self.wb.sheet(*s).is_some())
            .collect();
        ApplyResult {
            version: self.version,
            changed: by_sheet,
            layout_changed,
            sheets_changed: fx.sheets_changed,
            errors,
            undo: self.undo.state(),
            selection,
            info: fx.sheets_changed.then(|| self.info()),
        }
    }

    /// Applies a transaction as one undo step.
    pub fn apply(&mut self, tx: Transaction, clip: Option<&ClipSnapshot>) -> ApplyResult {
        let mut fx = Effects::default();
        let mut errors = Vec::new();
        let mut changes = Vec::new();
        for (i, op) in tx.ops.iter().enumerate() {
            match apply_op(&mut self.wb, op, clip, &mut fx) {
                Ok(c) => changes.extend(c),
                Err(e) => errors.push(OpError {
                    op_index: i as u32,
                    code: e.code,
                    message: e.message,
                }),
            }
        }
        let report = recalc(&mut self.wb);
        if !changes.is_empty() {
            self.undo.push(UndoRecord {
                label: tx.label.clone(),
                selection_before: tx.selection_before.clone(),
                selection_after: tx.selection_after.clone(),
                changes,
            });
            self.dirty = true;
        }
        self.result(report, fx, errors, tx.selection_after)
    }

    /// Undoes the last step (a no-op result when there is none).
    pub fn undo(&mut self) -> ApplyResult {
        let mut fx = Effects::default();
        let Some(rec) = self.undo.pop_undo() else {
            let report = recalc(&mut self.wb);
            return self.result(report, fx, Vec::new(), None);
        };
        let mut redo = Vec::with_capacity(rec.changes.len());
        for ch in rec.changes.into_iter().rev() {
            if let Some(inv) = apply_change(&mut self.wb, ch, &mut fx) {
                redo.push(inv);
            }
        }
        let report = recalc(&mut self.wb);
        let selection = rec.selection_before.clone();
        self.undo.push_redo(UndoRecord {
            label: rec.label,
            selection_before: rec.selection_before,
            selection_after: rec.selection_after,
            changes: redo,
        });
        self.dirty = true;
        self.result(report, fx, Vec::new(), selection)
    }

    /// Redoes the last undone step.
    pub fn redo(&mut self) -> ApplyResult {
        let mut fx = Effects::default();
        let Some(rec) = self.undo.pop_redo() else {
            let report = recalc(&mut self.wb);
            return self.result(report, fx, Vec::new(), None);
        };
        let mut undo = Vec::with_capacity(rec.changes.len());
        for ch in rec.changes.into_iter().rev() {
            if let Some(inv) = apply_change(&mut self.wb, ch, &mut fx) {
                undo.push(inv);
            }
        }
        let report = recalc(&mut self.wb);
        let selection = rec.selection_after.clone();
        self.undo.push_undo_keep_redo(UndoRecord {
            label: rec.label,
            selection_before: rec.selection_before,
            selection_after: rec.selection_after,
            changes: undo,
        });
        self.dirty = true;
        self.result(report, fx, Vec::new(), selection)
    }
}

/// A range as a DTO.
pub fn range_dto(r: &CellRange) -> RangeDto {
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}
