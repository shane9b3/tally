//! The [`DocumentService`] facade.
//!
//! Implemented by the engine (WP1) and wrapped 1:1 by the Tauri commands
//! (WP7, see `docs/api.md`) and later by `tally-wasm`. It is synchronous and
//! transport-agnostic; the shell decides threading (a `Mutex` in Tauri's
//! managed state).

use std::path::Path;

use tally_types::SheetId;

use crate::charts::{ChartDataDto, ChartDto, ChartKind, ImageDto};
use crate::dto::{
    ApiError, ApplyResult, AutoFilterInfo, CellBlock, CellEditInfo, CellPos, ClipboardOut,
    Direction, DocId, FileFormat, FindQuery, FindResult, FunctionInfo, NameInfo, RangeDto,
    SelectionDto, SelectionStats, SheetLayout, StyleDto, Suggestion, Transaction, WorkbookInfo,
};
use crate::features::{
    ConditionalFormatDto, DocPropertiesDto, GoalSeekRequest, GoalSeekResult, LocaleInfo, NoteDto,
    PageSetupDto, ProtectionInfo, ValidationDto, ValidationInfo,
};
use crate::pivot::{PivotDto, PivotFieldItemsDto, PivotHit, PivotSource};
use crate::print::{
    CalcSettingsDto, ChartSvgDto, PageBreaksDto, PrintJobInfo, PrintLayoutDto, PrintOptionsDto,
    SolverRequest, SolverResult,
};
use crate::tools::{
    AutocompleteDto, CellStyleDto, DatabaseRangeDto, DocumentStatsDto, NavigatorDto, ScenarioDto,
    TraceDto,
};

/// Result type of every fallible [`DocumentService`] method.
pub type Result<T> = std::result::Result<T, ApiError>;

/// Everything the UI can ask of the engine.
///
/// Methods taking `&self` are queries; `&mut self` methods change state.
/// Unknown `doc` or `sheet` ids yield [`ApiError::NOT_FOUND`].
pub trait DocumentService {
    /// Creates a workbook, optionally from a bundled template id.
    fn new_workbook(&mut self, template: Option<&str>) -> Result<WorkbookInfo>;

    /// Opens a file from disk (format detected from content and extension).
    fn open(&mut self, path: &Path) -> Result<WorkbookInfo>;

    /// Saves `doc` to `path` (or its current path when `None`) in `format`.
    fn save(&mut self, doc: DocId, path: Option<&Path>, format: FileFormat)
        -> Result<WorkbookInfo>;

    /// Closes a document and frees its memory.
    ///
    /// Returns `Result` (the plan had `()`) so that closing an unknown
    /// document is reported instead of silently ignored.
    fn close(&mut self, doc: DocId) -> Result<()>;

    /// Row/column geometry of a sheet.
    fn sheet_layout(&self, doc: DocId, sheet: SheetId) -> Result<SheetLayout>;

    /// Cells of the requested ranges, one block per range (≤ 20k cells per call).
    fn cells(&self, doc: DocId, sheet: SheetId, ranges: &[RangeDto]) -> Result<Vec<CellBlock>>;

    /// Resolves style ids returned in [`crate::dto::CellView::style`].
    fn styles(&self, doc: DocId, ids: &[u32]) -> Result<Vec<StyleDto>>;

    /// Formula-bar text and hint for a cell.
    fn cell_edit_info(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<CellEditInfo>;

    /// Applies a transaction as one undo step.
    fn apply(&mut self, doc: DocId, tx: Transaction) -> Result<ApplyResult>;

    /// Undoes the last transaction.
    fn undo(&mut self, doc: DocId) -> Result<ApplyResult>;

    /// Redoes the last undone transaction.
    fn redo(&mut self, doc: DocId) -> Result<ApplyResult>;

    /// Sum/count/average for the status bar.
    fn selection_stats(&self, doc: DocId, sel: &SelectionDto) -> Result<SelectionStats>;

    /// Copies the selection into the engine's internal clipboard snapshot.
    fn copy(&mut self, doc: DocId, sel: &SelectionDto) -> Result<ClipboardOut>;

    /// Target of Ctrl+arrow from `from` in `dir` (data edge jump).
    fn navigate_edge(
        &self,
        doc: DocId,
        sheet: SheetId,
        from: CellPos,
        dir: Direction,
    ) -> Result<CellPos>;

    /// Resolves name box text: `B12`, `A1:C5`, `Sheet2.A1`, a named range.
    fn resolve_name_box(&self, doc: DocId, text: &str) -> Result<SelectionDto>;

    /// Like [`Self::resolve_name_box`], with unqualified references and
    /// sheet-scoped names resolved against `sheet` (the sheet the UI
    /// shows) instead of the engine's active sheet.
    fn resolve_name_box_in(&self, doc: DocId, sheet: SheetId, text: &str) -> Result<SelectionDto> {
        let _ = sheet;
        self.resolve_name_box(doc, text)
    }

    /// Every registered function.
    fn list_functions(&self) -> Vec<FunctionInfo>;

    /// ⌘K suggestions for a natural-language query.
    fn assist_suggest(
        &self,
        doc: DocId,
        query: &str,
        sel: &SelectionDto,
    ) -> Result<Vec<Suggestion>>;

    // ----- v0.2 (default implementations report `unsupported`) -----------

    /// Every defined name (workbook scope first, then by sheet), sorted by name.
    fn names(&self, doc: DocId) -> Result<Vec<NameInfo>> {
        let _ = doc;
        Err(ApiError::unsupported("names are not supported"))
    }

    /// The sheet's AutoFilter with each column's distinct values.
    fn filter_info(&self, doc: DocId, sheet: SheetId) -> Result<AutoFilterInfo> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("filters are not supported"))
    }

    /// Find (at most 10,000 hits).
    fn find(&self, doc: DocId, query: &FindQuery) -> Result<FindResult> {
        let _ = (doc, query);
        Err(ApiError::unsupported("find is not supported"))
    }

    /// `true` if the first row of `range` looks like column labels (text
    /// over a row that is not all text), as AOO's sort dialog guesses.
    fn detect_header(&self, doc: DocId, sheet: SheetId, range: RangeDto) -> Result<bool> {
        let _ = (doc, sheet, range);
        Err(ApiError::unsupported("header detection is not supported"))
    }

    /// Hard recalculation (⇧⌘F9): rebuilds dependencies and recomputes
    /// every formula; clears the "recalculation incomplete" state.
    fn recalc_all(&mut self, doc: DocId) -> Result<ApplyResult> {
        let _ = doc;
        Err(ApiError::unsupported("recalc_all is not supported"))
    }

    // ----- v0.3 (default implementations report `unsupported`) -----------

    /// The sheet's conditional formats in priority order.
    fn conditional_formats(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ConditionalFormatDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported(
            "conditional formats are not supported",
        ))
    }

    /// The sheet's data validations.
    fn validations(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ValidationDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("validation is not supported"))
    }

    /// The validation of one cell with its list items resolved.
    fn validation_at(
        &self,
        doc: DocId,
        sheet: SheetId,
        r: u32,
        c: u32,
    ) -> Result<Option<ValidationInfo>> {
        let _ = (doc, sheet, r, c);
        Err(ApiError::unsupported("validation is not supported"))
    }

    /// The note of a cell.
    fn note(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<Option<NoteDto>> {
        let _ = (doc, sheet, r, c);
        Err(ApiError::unsupported("notes are not supported"))
    }

    /// Every note of a sheet, row-major.
    fn notes(&self, doc: DocId, sheet: SheetId) -> Result<Vec<NoteDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("notes are not supported"))
    }

    /// Protection state of a sheet and the document structure.
    fn protection_info(&self, doc: DocId, sheet: SheetId) -> Result<ProtectionInfo> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("protection is not supported"))
    }

    /// A sheet's page setup (with its manual breaks).
    fn page_setup(&self, doc: DocId, sheet: SheetId) -> Result<PageSetupDto> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("page setup is not supported"))
    }

    /// Document properties.
    fn document_properties(&self, doc: DocId) -> Result<DocPropertiesDto> {
        let _ = doc;
        Err(ApiError::unsupported(
            "document properties are not supported",
        ))
    }

    /// Goal seek, evaluated in a sandbox (the document is not changed).
    fn goal_seek(&self, doc: DocId, req: &GoalSeekRequest) -> Result<GoalSeekResult> {
        let _ = (doc, req);
        Err(ApiError::unsupported("goal seek is not supported"))
    }

    /// Supported locales.
    fn list_locales(&self) -> Vec<LocaleInfo> {
        Vec::new()
    }

    /// Sets the document's locale (number formatting and input
    /// recognition; persisted with the document, not undoable). The
    /// result reports every sheet's used area as changed.
    fn set_locale(&mut self, doc: DocId, locale: &str) -> Result<ApplyResult> {
        let _ = (doc, locale);
        Err(ApiError::unsupported("locales are not supported"))
    }

    // ----- v0.4 charts and pictures (W5-1; defaults report `unsupported`) --

    /// The charts of a sheet, in id order.
    fn charts(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ChartDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("charts are not supported"))
    }

    /// The resolved data of a chart, read from cached values without
    /// recalculating (≤ 255 series, ≤ 32,000 points per series).
    fn chart_data(&self, doc: DocId, sheet: SheetId, id: u32) -> Result<ChartDataDto> {
        let _ = (doc, sheet, id);
        Err(ApiError::unsupported("charts are not supported"))
    }

    /// A chart proposal for a selection: detects the data region around
    /// it, guesses labels and series orientation and anchors the chart to
    /// the right of the region at 16 × 9 cm. Nothing is added.
    fn suggest_chart(
        &self,
        doc: DocId,
        sheet: SheetId,
        sel: RangeDto,
        kind: ChartKind,
    ) -> Result<ChartDto> {
        let _ = (doc, sheet, sel, kind);
        Err(ApiError::unsupported("charts are not supported"))
    }

    /// The pictures of a sheet (without their bytes).
    fn images(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ImageDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("pictures are not supported"))
    }

    // ----- v0.5 DataPilot and data tools (W5-2) ----------------------------

    /// Every pivot table of the document.
    fn pivots(&self, doc: DocId) -> Result<Vec<PivotDto>> {
        let _ = doc;
        Err(ApiError::unsupported("pivot tables are not supported"))
    }

    /// The fields and distinct items of a pivot source (which may be
    /// scanned before any pivot exists).
    fn pivot_field_items(
        &self,
        doc: DocId,
        source: &PivotSource,
    ) -> Result<Vec<PivotFieldItemsDto>> {
        let _ = (doc, source);
        Err(ApiError::unsupported("pivot tables are not supported"))
    }

    /// The pivot output a cell belongs to, if any.
    fn pivot_at(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<Option<PivotHit>> {
        let _ = (doc, sheet, r, c);
        Err(ApiError::unsupported("pivot tables are not supported"))
    }

    /// AutoInput: the column's text completions for `prefix` at a cell.
    fn autocomplete(
        &self,
        doc: DocId,
        sheet: SheetId,
        r: u32,
        c: u32,
        prefix: &str,
    ) -> Result<AutocompleteDto> {
        let _ = (doc, sheet, r, c, prefix);
        Err(ApiError::unsupported("AutoInput is not supported"))
    }

    /// Named cell styles, `Default` first.
    fn cell_styles(&self, doc: DocId) -> Result<Vec<CellStyleDto>> {
        let _ = doc;
        Err(ApiError::unsupported("cell styles are not supported"))
    }

    /// The scenarios of a (base) sheet.
    fn scenarios(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ScenarioDto>> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("scenarios are not supported"))
    }

    /// Every named database range.
    fn database_ranges(&self, doc: DocId) -> Result<Vec<DatabaseRangeDto>> {
        let _ = doc;
        Err(ApiError::unsupported("database ranges are not supported"))
    }

    /// The Navigator's entries.
    fn navigator(&self, doc: DocId) -> Result<NavigatorDto> {
        let _ = doc;
        Err(ApiError::unsupported("the Navigator is not supported"))
    }

    /// Document statistics.
    fn document_statistics(&self, doc: DocId) -> Result<DocumentStatsDto> {
        let _ = doc;
        Err(ApiError::unsupported(
            "document statistics are not supported",
        ))
    }

    // ----- v0.6 print, PDF, calculation semantics and detective (W5-3) -----

    /// Pagination of a print (no rendering).
    fn print_layout(&self, doc: DocId, opts: &PrintOptionsDto) -> Result<PrintLayoutDto> {
        let _ = (doc, opts);
        Err(ApiError::unsupported("printing is not supported"))
    }

    /// Automatic and manual page breaks of a sheet (Page Break Preview).
    fn page_breaks(&self, doc: DocId, sheet: SheetId) -> Result<PageBreaksDto> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("printing is not supported"))
    }

    /// Builds a print job (cells, charts as the given SVGs) and keeps it in
    /// an LRU of 4 jobs.
    fn prepare_print_job(
        &mut self,
        doc: DocId,
        opts: &PrintOptionsDto,
        charts: &[ChartSvgDto],
    ) -> Result<PrintJobInfo> {
        let _ = (doc, opts, charts);
        Err(ApiError::unsupported("printing is not supported"))
    }

    /// One page of a job rasterized to PNG, base64.
    fn print_job_page_png(&self, job: u32, page: u32, dpi: u32) -> Result<String> {
        let _ = (job, page, dpi);
        Err(ApiError::unsupported("printing is not supported"))
    }

    /// The PDF of a job.
    fn print_job_pdf(&self, job: u32) -> Result<Vec<u8>> {
        let _ = job;
        Err(ApiError::unsupported("PDF export is not supported"))
    }

    /// The document's calculation settings.
    fn calc_settings(&self, doc: DocId) -> Result<CalcSettingsDto> {
        let _ = doc;
        Err(ApiError::unsupported(
            "calculation settings are not supported",
        ))
    }

    /// The linear solver, run in a sandbox (the document is not changed).
    fn solver(&self, doc: DocId, req: &SolverRequest) -> Result<SolverResult> {
        let _ = (doc, req);
        Err(ApiError::unsupported("the solver is not supported"))
    }

    /// Detective: the cells a formula reads (direct precedents).
    fn precedents(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<Vec<TraceDto>> {
        let _ = (doc, sheet, r, c);
        Err(ApiError::unsupported("the detective is not supported"))
    }

    /// Detective: the formulas that read a cell (direct dependents).
    fn dependents(&self, doc: DocId, sheet: SheetId, r: u32, c: u32) -> Result<Vec<TraceDto>> {
        let _ = (doc, sheet, r, c);
        Err(ApiError::unsupported("the detective is not supported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trait must stay object-safe: the Tauri shell stores a
    /// `Box<dyn DocumentService + Send>`.
    #[test]
    fn trait_is_object_safe() {
        fn _assert(_: &dyn DocumentService) {}
    }
}
