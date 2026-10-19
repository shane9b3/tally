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
        let mut warnings = std::mem::take(&mut wb.import_warnings);
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
        mut fx: Effects,
        errors: Vec<OpError>,
        selection: Option<SelectionDto>,
    ) -> ApplyResult {
        self.version += 1;
        let notices: Vec<OpError> = std::mem::take(&mut fx.notice_errors);
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
            warnings: (!notices.is_empty()).then_some(notices),
            fitted_rows: fx.fitted_rows,
            replaced: fx.replaced,
        }
    }

    /// Recalculates. AutoFilters are not re-applied (see [`crate::filter`]);
    /// SUBTOTAL-style formulas reading a sheet whose layout (row
    /// visibility) changed are recalculated.
    fn recalc_and_filter(&mut self, fx: &mut Effects) -> RecalcReport {
        self.wb.dirty_visibility_dependents(&fx.layout_changed);
        recalc(&mut self.wb)
    }

    /// Hard recalculation: rebuilds every formula's dependencies (dynamic
    /// ones are collected again) and recomputes everything without a
    /// budget. Clears [`Self::recalc_incomplete`].
    pub fn recalc_all(&mut self) -> ApplyResult {
        let ids: Vec<crate::formula_cells::FormulaCellId> = self.wb.formulas.keys().collect();
        for id in ids {
            if let Some(f) = self.wb.formulas.get_mut(id) {
                f.dynamic_deps.clear();
            }
            self.wb.refresh_formula(id);
        }
        let report = hard_recalc_with(&mut self.wb, &RecalcBudget::unbounded());
        self.recalc_incomplete = false;
        let mut fx = Effects::default();
        let sheets: Vec<SheetId> = self.wb.sheets.iter().map(|s| s.id).collect();
        for s in sheets {
            if self.wb.reapply_filter(s) {
                fx.layout(s);
            }
        }
        self.result(report, fx, Vec::new(), None)
    }

    /// Applies a transaction as one undo step.
    pub fn apply(&mut self, tx: Transaction, clip: Option<&ClipSnapshot>) -> ApplyResult {
        let mut fx = Effects::default();
        let mut errors = Vec::new();
        let mut changes = Vec::new();
        let mut spent: u64 = 0;
        for (i, op) in tx.ops.iter().enumerate() {
            let cost = crate::ops::op_cost(&self.wb, op);
            if spent.saturating_add(cost) > crate::ops::MAX_TX_CELLS {
                let e = tally_api::ApiError::unsupported(format!(
                    "the change is too large for one step (more than {} cells); \
                     split it into smaller steps",
                    crate::ops::MAX_TX_CELLS
                ));
                errors.push(OpError {
                    op_index: i as u32,
                    code: e.code,
                    message: e.message,
                });
                continue;
            }
            spent += cost;
            match apply_op(&mut self.wb, op, clip, &mut fx) {
                Ok(c) => changes.extend(c),
                Err(e) => errors.push(OpError {
                    op_index: i as u32,
                    code: e.code,
                    message: e.message,
                }),
            }
            for w in fx.warnings.drain(..) {
                errors.push(OpError {
                    op_index: i as u32,
                    code: w.code,
                    message: w.message,
                });
            }
            for w in fx.notices.drain(..) {
                fx.notice_errors.push(OpError {
                    op_index: i as u32,
                    code: w.code,
                    message: w.message,
                });
            }
        }
        let report = self.recalc_and_filter(&mut fx);
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

    /// Sets the locale (not undoable; the document becomes dirty). Every
    /// sheet's used area is reported as changed.
    pub fn set_locale(&mut self, tag: &str) -> Result<ApplyResult, tally_api::ApiError> {
        if !self.wb.set_locale(tag) {
            return Err(tally_api::ApiError::invalid_argument(format!(
                "unknown locale {tag:?}"
            )));
        }
        let changed: Vec<CellRange> = self
            .wb
            .sheets
            .iter()
            .filter_map(|s| s.used_range())
            .collect();
        self.dirty = true;
        let report = RecalcReport {
            changed,
            ..RecalcReport::default()
        };
        let mut fx = Effects::default();
        for s in self.wb.sheets.iter().map(|s| s.id).collect::<Vec<_>>() {
            fx.layout(s);
        }
        Ok(self.result(report, fx, Vec::new(), None))
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
        let report = self.recalc_and_filter(&mut fx);
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
        let report = self.recalc_and_filter(&mut fx);
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
