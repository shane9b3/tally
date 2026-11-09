//! The [`DocumentService`] facade.
//!
//! Implemented by the engine (WP1) and wrapped 1:1 by the Tauri commands
//! (WP7, see `docs/api.md`) and later by `tally-wasm`. It is synchronous and
//! transport-agnostic; the shell decides threading (the desktop shell keeps
//! one engine service per document, each behind its own `Mutex`).

use std::path::Path;

use tally_types::SheetId;

use crate::charts::{ChartDataDto, ChartDto, ChartKind, ImageDto};
use crate::dto::{
    ApiError, ApplyResult, AutoFilterInfo, CellBlock, CellEditInfo, CellPos, ClipboardOut,
    Direction, DocId, FileFormat, FindQuery, FindResult, FunctionInfo, NameInfo, PasteOptions,
    PastePayload, RangeDto, SelectionDto, SelectionStats, SheetLayout, StyleDto, Suggestion,
    Transaction, UndoHistory, WorkbookInfo,
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
    AutocompleteDto, CellStyleDto, CellsOptions, DatabaseRangeDto, DetectiveAction, DetectiveState,
    DocumentStatsDto, NavigatorDto, ScenarioDto, TraceDto, ViewOptionsDto,
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

    /// Opens a file as a new untitled document: no path (so Save asks
    /// where), title `Untitled`, dirty when `dirty` (recovery snapshots,
    /// user templates).
    fn open_untitled(&mut self, path: &Path, dirty: bool) -> Result<WorkbookInfo> {
        let _ = dirty;
        self.open(path)
    }

    /// The current summary of an open document (the desktop shell hands it
    /// to a new window, so the window never trusts a summary from a webview).
    fn info(&self, doc: DocId) -> Result<WorkbookInfo> {
        let _ = doc;
        Err(ApiError::unsupported(
            "document summaries are not available",
        ))
    }

    /// Sets `doc` read-only or editable (AOO's Edit ▸ Edit Mode). While it
    /// is read-only every edit (`apply`, `undo`, `redo`, `set_locale`) is
    /// refused with [`ApiError::READ_ONLY`], and so is a save to the
    /// document's own file; a save to another file makes it editable.
    fn set_read_only(&mut self, doc: DocId, read_only: bool) -> Result<WorkbookInfo> {
        let _ = (doc, read_only);
        Err(ApiError::unsupported("read-only mode is not supported"))
    }

    /// Reads `doc`'s file again into the same document id (discarding
    /// unsaved changes and the undo history), keeping its read-only flag.
    fn reload(&mut self, doc: DocId) -> Result<WorkbookInfo> {
        let _ = doc;
        Err(ApiError::unsupported("reload is not supported"))
    }

    /// Makes `doc` an untitled copy: no path (Save asks where), title
    /// `Untitled`, editable (AOO's "edit a copy of the document").
    fn detach_path(&mut self, doc: DocId) -> Result<WorkbookInfo> {
        let _ = doc;
        Err(ApiError::unsupported("untitled copies are not supported"))
    }

    /// Whether `doc` has unsaved changes (`WorkbookInfo.dirty`), for the
    /// shell's exit snapshot, which must not copy a clean document.
    fn is_dirty(&self, doc: DocId) -> Result<bool> {
        let _ = doc;
        Err(ApiError::unsupported("the dirty flag is not available"))
    }

    /// Writes a copy of `doc` to `path` in `format` without changing the
    /// document's path, title or dirty flag (autosave, Save as Template).
    fn save_copy(&self, doc: DocId, path: &Path, format: FileFormat) -> Result<()> {
        let _ = (doc, path, format);
        Err(ApiError::unsupported("save_copy is not supported"))
    }

    /// Sets the options of `doc`'s CSV exports (AOO's Text Export dialog);
    /// later CSV saves of the document use them until changed.
    fn set_csv_options(&mut self, doc: DocId, options: &crate::CsvExportOptions) -> Result<()> {
        let _ = (doc, options);
        Err(ApiError::unsupported(
            "CSV export options are not supported",
        ))
    }

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

    /// Undoes the last `count` transactions as one user action (AOO's
    /// toolbar Undo list: `SID_UNDO` with a count, run by
    /// `ScTabViewShell::ExecuteUndo`). Fewer when the stack holds fewer;
    /// 0 is a no-op.
    fn undo_steps(&mut self, doc: DocId, count: u32) -> Result<ApplyResult> {
        let _ = (doc, count);
        Err(ApiError::unsupported("undo_steps is not supported"))
    }

    /// Redoes the next `count` undone transactions as one user action.
    fn redo_steps(&mut self, doc: DocId, count: u32) -> Result<ApplyResult> {
        let _ = (doc, count);
        Err(ApiError::unsupported("redo_steps is not supported"))
    }

    /// The labels of the undo and redo steps, next one first.
    fn undo_history(&self, doc: DocId) -> Result<UndoHistory> {
        let _ = doc;
        Err(ApiError::unsupported("undo_history is not supported"))
    }

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

    /// Tab / Enter on a protected sheet: the next unlocked, visible,
    /// non-covered cell from `from` in reading order (`Right`/`Left`: along
    /// rows, `Down`/`Up`: down columns), wrapping, optionally inside
    /// `within`. `None` when the sheet is not protected or has no such cell.
    fn next_unlocked(
        &self,
        doc: DocId,
        sheet: SheetId,
        from: CellPos,
        dir: Direction,
        within: Option<RangeDto>,
    ) -> Result<Option<CellPos>>;

    /// `range` grown while any cell on the border just outside it holds data
    /// (AOO's `GetDataArea`): the block around one cell (Ctrl+*), the sort
    /// area a selection would extend to.
    fn data_region(&self, doc: DocId, sheet: SheetId, range: RangeDto) -> Result<RangeDto>;

    /// Whether a paste of `payload` at `target` with `options` would write
    /// over at least one cell that holds data (the paste-over-data warning).
    fn paste_overwrites(
        &self,
        doc: DocId,
        sheet: SheetId,
        target: RangeDto,
        payload: PastePayload,
        options: PasteOptions,
    ) -> Result<bool>;

    /// Resolves name box text: `B12`, `A1:C5`, `Sheet2.A1`, a named range.
    fn resolve_name_box(&self, doc: DocId, text: &str) -> Result<SelectionDto>;

    /// Like [`Self::resolve_name_box`], with unqualified references and
    /// sheet-scoped names resolved against `sheet` (the sheet the UI
    /// shows) instead of the engine's active sheet.
    fn resolve_name_box_in(&self, doc: DocId, sheet: SheetId, text: &str) -> Result<SelectionDto> {
        let _ = sheet;
        self.resolve_name_box(doc, text)
    }

    /// Records the sheet the user is on (view state, like AOO's view
    /// settings): the next save writes it as the active sheet, and calls that
    /// default to the active sheet (CSV save, Find across sheets,
    /// `PrintSheets::Active`) start there. Not undoable; does not mark the
    /// document modified. `not_found` for an unknown sheet.
    fn set_active_sheet(&mut self, doc: DocId, sheet: SheetId) -> Result<()> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("set_active_sheet is not supported"))
    }

    /// The view zoom in percent (20–400, AOO `MINZOOM`..`MAXZOOM`) for
    /// every sheet (AOO's default "synchronize sheets"), saved with the
    /// document's view settings (ODS `ZoomValue`, XLSX `zoomScale`). View
    /// state: not undoable, does not mark the document modified.
    /// `invalid_argument` outside 20–400.
    fn set_view_zoom(&mut self, doc: DocId, zoom: u16) -> Result<()> {
        let _ = (doc, zoom);
        Err(ApiError::unsupported("set_view_zoom is not supported"))
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

    /// The bytes of a picture, base64 (≤ 16 MiB decoded), so the UI can
    /// show pictures of reopened files.
    fn image_data(&self, doc: DocId, sheet: SheetId, id: u32) -> Result<String> {
        let _ = (doc, sheet, id);
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

    /// The custom sort lists (Tools ▸ Options ▸ Sort Lists; v0.5, W5-2).
    fn custom_sort_lists(&self, doc: DocId) -> Result<Vec<Vec<String>>> {
        let _ = doc;
        Err(ApiError::unsupported("custom sort lists are not supported"))
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

    /// Tools ▸ Detective: runs `action` on cell `(r, c)` of `sheet` as AOO
    /// does (each Trace adds one level, each Remove takes the outermost
    /// one away) and records it in the document's operation list, which
    /// Refresh Traces replays and ODS saves. Not undoable; allowed on a
    /// read-only document. Returns the sheet's arrows.
    fn detective(
        &mut self,
        doc: DocId,
        sheet: SheetId,
        r: u32,
        c: u32,
        action: DetectiveAction,
    ) -> Result<DetectiveState> {
        let _ = (doc, sheet, r, c, action);
        Err(ApiError::unsupported("the detective is not supported"))
    }

    /// The detective arrows of a sheet.
    fn detective_arrows(&self, doc: DocId, sheet: SheetId) -> Result<DetectiveState> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("the detective is not supported"))
    }

    /// Tools ▸ Detective ▸ AutoRefresh (an application option in AOO,
    /// `Input/DetectiveAuto`, on by default): when on, the arrows are
    /// redrawn after a change that touched formulas (or any change while
    /// the list has a Trace Error), and `ApplyResult::detective_changed`
    /// says so.
    fn set_detective_auto(&mut self, doc: DocId, on: bool) -> Result<()> {
        let _ = (doc, on);
        Err(ApiError::unsupported("the detective is not supported"))
    }

    /// A sheet's view options, with the persisted split position (v0.6).
    fn view_options(&self, doc: DocId, sheet: SheetId) -> Result<ViewOptionsDto> {
        let _ = (doc, sheet);
        Err(ApiError::unsupported("view options are not supported"))
    }

    /// [`DocumentService::cells`] rendered with view options: formula text
    /// instead of results, value highlighting colours, hidden zero values
    /// (v0.6).
    fn cells_with(
        &self,
        doc: DocId,
        sheet: SheetId,
        ranges: &[RangeDto],
        opts: &CellsOptions,
    ) -> Result<Vec<CellBlock>> {
        let _ = opts;
        self.cells(doc, sheet, ranges)
    }

    /// The Function Wizard's live result: formula `text` (with or without
    /// the leading `=`) evaluated at `(r, c)` without committing it,
    /// formatted with that cell's number format; `None` when it does not
    /// parse (v0.6).
    fn preview_formula(
        &self,
        doc: DocId,
        sheet: SheetId,
        r: u32,
        c: u32,
        text: &str,
    ) -> Result<Option<String>> {
        let _ = (doc, sheet, r, c, text);
        Err(ApiError::unsupported("formula preview is not supported"))
    }

    /// Detective ▸ Mark Invalid Data: the cells of `range` (or the whole
    /// sheet) whose values fail their validation rule, at most 10,000,
    /// checked within the validation work budget (v0.6).
    fn invalid_cells(
        &self,
        doc: DocId,
        sheet: SheetId,
        range: Option<RangeDto>,
    ) -> Result<Vec<CellPos>> {
        let _ = (doc, sheet, range);
        Err(ApiError::unsupported("validation marking is not supported"))
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
