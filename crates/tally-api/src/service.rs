//! The [`DocumentService`] facade.
//!
//! Implemented by the engine (WP1) and wrapped 1:1 by the Tauri commands
//! (WP7, see `docs/api.md`) and later by `tally-wasm`. It is synchronous and
//! transport-agnostic; the shell decides threading (a `Mutex` in Tauri's
//! managed state).

use std::path::Path;

use tally_types::SheetId;

use crate::dto::{
    ApiError, ApplyResult, CellBlock, CellEditInfo, CellPos, ClipboardOut, Direction, DocId,
    FileFormat, FunctionInfo, RangeDto, SelectionDto, SelectionStats, SheetLayout, StyleDto,
    Suggestion, Transaction, WorkbookInfo,
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

    /// Every registered function.
    fn list_functions(&self) -> Vec<FunctionInfo>;

    /// ⌘K suggestions for a natural-language query.
    fn assist_suggest(
        &self,
        doc: DocId,
        query: &str,
        sel: &SelectionDto,
    ) -> Result<Vec<Suggestion>>;
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
