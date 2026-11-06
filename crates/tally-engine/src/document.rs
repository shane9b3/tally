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
    /// The opened file held macros Tally drops; reported once with the
    /// open result, like [`Self::warnings`].
    pub has_macros: bool,
    /// Options of the document's CSV exports (the last Text Export
    /// dialog choices; defaults until set).
    pub csv_options: tally_api::CsvExportOptions,
    /// Read-only (AOO's read-only medium / Edit Mode off): the service
    /// refuses every edit with [`tally_api::ApiError::READ_ONLY`].
    pub read_only: bool,
    /// Tools ▸ Detective ▸ AutoRefresh (AOO's application option
    /// `Input/DetectiveAuto`, on by default; the app mirrors it here).
    pub detective_auto: bool,
    /// The dependency generation the arrows were last checked against
    /// (AOO `IsDetectiveDirty`).
    detective_gen: u64,
    /// Identity of each sheet's detective state at the last result, to
    /// report `detective_changed` when rows or columns or undo moved them.
    detective_ptrs: Vec<(SheetId, usize, u64)>,
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
        // Detective arrows from the file's operation list.
        crate::detective::refresh_all(&mut wb);
        let detective_gen = wb.deps.generation();
        let detective_ptrs = detective_ptrs(&wb);
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
            has_macros: false,
            csv_options: tally_api::CsvExportOptions::default(),
            read_only: false,
            detective_auto: true,
            detective_gen,
            detective_ptrs,
        }
    }

    /// Detective AutoRefresh after a change (`ScDocShell::SetDocumentModified`):
    /// with the option on and a non-empty operation list, the arrows are
    /// replayed when formulas were added, removed or re-pointed since the
    /// last check, or always while the list holds a Trace Error. Returns
    /// whether any sheet's arrows changed (replayed, or moved by a
    /// structure change or undo).
    fn detective_after_change(&mut self) -> bool {
        let gen = self.wb.deps.generation();
        let formulas_changed = gen != self.detective_gen;
        if self.detective_auto
            && crate::detective::has_ops(&self.wb)
            && (formulas_changed || crate::detective::has_add_error(&self.wb))
        {
            crate::detective::refresh_all(&mut self.wb);
        }
        // Replaying reads but never re-registers formulas.
        self.detective_gen = self.wb.deps.generation();
        let ptrs = detective_ptrs(&self.wb);
        let changed = ptrs != self.detective_ptrs;
        self.detective_ptrs = ptrs;
        changed
    }

    /// Runs a Tools ▸ Detective command (not undoable; the document becomes
    /// dirty when it changed something, unless it is read-only).
    pub fn detective(
        &mut self,
        sheet: SheetId,
        r: u32,
        c: u32,
        action: tally_api::DetectiveAction,
    ) -> Result<tally_api::DetectiveState, tally_api::ApiError> {
        let done = crate::detective::run(&mut self.wb, sheet, r, c, action)?;
        if done && !self.read_only {
            self.dirty = true;
        }
        self.detective_gen = self.wb.deps.generation();
        self.detective_ptrs = detective_ptrs(&self.wb);
        let mut st = crate::detective::state(&self.wb, sheet, done);
        st.dirty = self.dirty;
        Ok(st)
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
            external_links: (!self.wb.external_links.is_empty())
                .then(|| self.wb.external_links.clone()),
            read_only: self.read_only,
            load_readonly: self.wb.load_readonly,
            has_macros: self.has_macros,
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
        // Charts whose plotted cells changed (v0.4, W5-1).
        if fx.charts_changed.is_none() {
            fx.charts_changed =
                crate::charts::changed_charts(&self.wb, &report.changed, &fx.layout_changed);
        }
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
        let detective_changed = self.detective_after_change().then_some(true);
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
            charts_changed: fx.charts_changed,
            objects_changed: fx.objects_changed,
            created: fx.created,
            pivots_changed: fx.pivots_changed,
            detective_changed,
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
    ///
    /// With a `group` of two or more sheets the group ops run on every
    /// sheet of the group ([`crate::group`]) and the transaction is
    /// all-or-nothing: when an op fails (an error, not a warning or
    /// notice), every change is reverted and only the errors are
    /// returned (op indexes are those of `tx.ops`).
    pub fn apply(&mut self, tx: Transaction, clip: Option<&ClipSnapshot>) -> ApplyResult {
        let mut fx = Effects::default();
        let mut errors = Vec::new();
        let mut changes = Vec::new();
        let mut spent: u64 = 0;
        let mut meta_ops: u64 = 0;
        let mut note_bytes: u64 = 0;
        let group = match crate::group::group_sheets(&self.wb, &tx) {
            Ok(g) => g,
            Err(e) => {
                let report = recalc(&mut self.wb);
                let err = OpError {
                    op_index: 0,
                    code: e.code,
                    message: e.message,
                };
                return self.result(report, fx, vec![err], None);
            }
        };
        let atomic = group.is_some();
        let active_before = self.wb.active_sheet;
        let mut failed = false;
        // Indexes into `errors` of the failures (not warnings).
        let mut hard: Vec<usize> = Vec::new();
        for step in crate::group::expand(&tx, group.as_deref()) {
            let (i, op) = (step.index as usize, step.op.as_ref());
            if atomic && failed {
                break;
            }
            let cost = crate::ops::op_cost(&self.wb, op, clip);
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
                failed = true;
                hard.push(errors.len() - 1);
                continue;
            }
            if let Some(bytes) = crate::ops_v03::meta_op_cost(op) {
                meta_ops += 1;
                note_bytes = note_bytes.saturating_add(bytes);
                if meta_ops > crate::ops_v03::MAX_META_OPS_PER_TX
                    || note_bytes > crate::ops_v03::MAX_NOTE_BYTES_PER_TX
                {
                    let e = tally_api::ApiError::unsupported(format!(
                        "the change is too large for one step (more than {} \
                         formatting, validation, note or link changes, or more \
                         than {} MiB of note text); split it into smaller steps",
                        crate::ops_v03::MAX_META_OPS_PER_TX,
                        crate::ops_v03::MAX_NOTE_BYTES_PER_TX >> 20
                    ));
                    errors.push(OpError {
                        op_index: i as u32,
                        code: e.code,
                        message: e.message,
                    });
                    failed = true;
                    hard.push(errors.len() - 1);
                    continue;
                }
            }
            spent += cost;
            let applied = crate::clipboard::check_link_source(op, clip, self.id)
                .and_then(|()| apply_op(&mut self.wb, op, clip, &mut fx));
            match applied {
                Ok(c) => {
                    // Ids allocated by this op (recorded with a placeholder
                    // index) belong to op `i`.
                    if let Some(v) = fx.created.as_mut() {
                        for c in v.iter_mut().filter(|c| c.op_index == u32::MAX) {
                            c.op_index = i as u32;
                        }
                    }
                    changes.extend(c);
                    // A row/column change made on every group sheet moves
                    // the 3D references over those sheets once.
                    if let Some(sheets) = step.ends_group {
                        changes.extend(crate::group::structure_3d(&mut self.wb, op, sheets));
                    }
                }
                Err(e) => {
                    failed = true;
                    errors.push(OpError {
                        op_index: i as u32,
                        code: e.code,
                        message: e.message,
                    });
                    hard.push(errors.len() - 1);
                }
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
        if atomic && failed {
            // All-or-nothing: undo what the transaction did so far.
            for ch in std::mem::take(&mut changes).into_iter().rev() {
                let _ = apply_change(&mut self.wb, ch, &mut fx);
            }
            self.wb.active_sheet = active_before;
            fx.created = None;
            fx.notice_errors.clear();
            fx.replaced = None;
            fx.fitted_rows = None;
            let failures: Vec<OpError> = errors
                .into_iter()
                .enumerate()
                .filter(|(k, _)| hard.contains(k))
                .map(|(_, e)| e)
                .collect();
            // Nothing stands, so a reverted protection change neither clears
            // the undo history nor dirties the document.
            fx.clear_undo = false;
            let report = self.recalc_and_filter(&mut fx);
            return self.result(report, fx, failures, None);
        }
        let report = self.recalc_and_filter(&mut fx);
        if fx.clear_undo {
            // Protection changes are not undoable: nothing before them can
            // be undone around them either (security review 3, LOW-9).
            self.undo.clear();
            self.dirty = true;
        } else if !changes.is_empty() {
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

/// The identity of every sheet's detective state.
fn detective_ptrs(wb: &Workbook) -> Vec<(SheetId, usize, u64)> {
    wb.sheets
        .iter()
        .map(|s| {
            let d = &s.meta.detective;
            (s.id, std::sync::Arc::as_ptr(d) as usize, d.moves)
        })
        .collect()
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
