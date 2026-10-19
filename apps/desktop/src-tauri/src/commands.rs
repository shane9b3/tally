//! One `#[tauri::command]` per entry in docs/api.md. Each locks the service
//! and forwards to the matching [`DocumentService`] method. Errors are
//! [`ApiError`] (`{ code, message }`), which Tauri serializes as the
//! rejection value of the JS promise.
//!
//! Commands are `async` so they run on Tauri's async runtime instead of the
//! main (UI) thread.

use std::path::PathBuf;

use tally_api::{
    ApiError, ApplyResult, CellBlock, CellEditInfo, CellPos, ClipboardOut, Direction, DocId,
    DocumentService, FunctionInfo, RangeDto, SelectionDto, SelectionStats, SheetLayout, StyleDto,
    Suggestion, Transaction, WorkbookInfo,
};
use tally_types::SheetId;
use tauri::{AppHandle, Runtime, State};

use crate::files::{self, TemplateInfo};
use crate::state::{lock, AppState};
use tally_api::FileFormat;

/// Result of every command.
pub type CmdResult<T> = Result<T, ApiError>;

/// Runs `f` with the locked service.
pub fn with_service<T>(
    state: &AppState,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<T>,
) -> CmdResult<T> {
    let mut guard = lock(&state.service, "engine")?;
    f(guard.as_mut())
}

/// Remembers the document's file (for Save) from a returned info.
pub fn track_path(state: &AppState, info: &WorkbookInfo) -> CmdResult<()> {
    let mut paths = lock(&state.paths, "paths")?;
    match &info.path {
        Some(p) => {
            paths.insert(info.doc.0, PathBuf::from(p));
        }
        None => {
            paths.remove(&info.doc.0);
        }
    }
    Ok(())
}

/// Opens a path through the engine and remembers it for Save (no allowlist
/// check; callers check first).
pub fn open_path(state: &AppState, path: &std::path::Path) -> CmdResult<WorkbookInfo> {
    let info = with_service(state, |s| s.open(path))?;
    track_path(state, &info)?;
    Ok(info)
}

/// Opens an allowlisted path and records it in the recent list.
pub fn open_checked<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    path: &std::path::Path,
) -> CmdResult<WorkbookInfo> {
    let canon = files::check_open_allowed(state, path)?;
    let info = open_path(state, &canon)?;
    files::push_recent(app, state, &canon);
    Ok(info)
}

/// `wb_new`: a blank workbook, or one built by the engine from a template
/// id (`invoices`, `budget`, `stock`; unknown ids are `not_found`).
pub fn new_workbook(state: &AppState, template: Option<&str>) -> CmdResult<WorkbookInfo> {
    let info = with_service(state, |s| s.new_workbook(template))?;
    track_path(state, &info)?;
    Ok(info)
}

/// Saves `doc` to `path` (already chosen in a dialog or remembered) and
/// remembers it for the next Save.
pub fn save_to(
    state: &AppState,
    doc: DocId,
    path: &std::path::Path,
    format: FileFormat,
) -> CmdResult<WorkbookInfo> {
    let info = with_service(state, |s| s.save(doc, Some(path), format))?;
    track_path(state, &info)?;
    Ok(info)
}

#[tauri::command]
pub async fn wb_new(
    state: State<'_, AppState>,
    template: Option<String>,
) -> CmdResult<WorkbookInfo> {
    new_workbook(&state, template.as_deref())
}

#[tauri::command]
pub async fn wb_open(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<WorkbookInfo> {
    open_checked(&app, &state, std::path::Path::new(&path))
}

#[tauri::command]
pub async fn wb_close(state: State<'_, AppState>, doc: DocId) -> CmdResult<()> {
    with_service(&state, |s| s.close(doc))?;
    lock(&state.paths, "paths")?.remove(&doc.0);
    Ok(())
}

#[tauri::command]
pub async fn sheet_layout(
    state: State<'_, AppState>,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<SheetLayout> {
    with_service(&state, |s| s.sheet_layout(doc, sheet))
}

#[tauri::command]
pub async fn get_cells(
    state: State<'_, AppState>,
    doc: DocId,
    sheet: SheetId,
    ranges: Vec<RangeDto>,
) -> CmdResult<Vec<CellBlock>> {
    with_service(&state, |s| s.cells(doc, sheet, &ranges))
}

#[tauri::command]
pub async fn get_styles(
    state: State<'_, AppState>,
    doc: DocId,
    ids: Vec<u32>,
) -> CmdResult<Vec<StyleDto>> {
    with_service(&state, |s| s.styles(doc, &ids))
}

#[tauri::command]
pub async fn cell_edit_info(
    state: State<'_, AppState>,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<CellEditInfo> {
    with_service(&state, |s| s.cell_edit_info(doc, sheet, r, c))
}

#[tauri::command]
pub async fn apply(
    state: State<'_, AppState>,
    doc: DocId,
    tx: Transaction,
) -> CmdResult<ApplyResult> {
    with_service(&state, |s| s.apply(doc, tx))
}

#[tauri::command]
pub async fn undo(state: State<'_, AppState>, doc: DocId) -> CmdResult<ApplyResult> {
    with_service(&state, |s| s.undo(doc))
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>, doc: DocId) -> CmdResult<ApplyResult> {
    with_service(&state, |s| s.redo(doc))
}

#[tauri::command]
pub async fn selection_stats(
    state: State<'_, AppState>,
    doc: DocId,
    sel: SelectionDto,
) -> CmdResult<SelectionStats> {
    with_service(&state, |s| s.selection_stats(doc, &sel))
}

#[tauri::command]
pub async fn clipboard_copy(
    state: State<'_, AppState>,
    doc: DocId,
    sel: SelectionDto,
) -> CmdResult<ClipboardOut> {
    with_service(&state, |s| s.copy(doc, &sel))
}

#[tauri::command]
pub async fn navigate_edge(
    state: State<'_, AppState>,
    doc: DocId,
    sheet: SheetId,
    from: CellPos,
    dir: Direction,
) -> CmdResult<CellPos> {
    with_service(&state, |s| s.navigate_edge(doc, sheet, from, dir))
}

#[tauri::command]
pub async fn resolve_name_box(
    state: State<'_, AppState>,
    doc: DocId,
    text: String,
) -> CmdResult<SelectionDto> {
    with_service(&state, |s| s.resolve_name_box(doc, &text))
}

#[tauri::command]
pub async fn list_functions(state: State<'_, AppState>) -> CmdResult<Vec<FunctionInfo>> {
    with_service(&state, |s| Ok(s.list_functions()))
}

#[tauri::command]
pub async fn assist_suggest(
    state: State<'_, AppState>,
    doc: DocId,
    query: String,
    sel: SelectionDto,
) -> CmdResult<Vec<Suggestion>> {
    with_service(&state, |s| s.assist_suggest(doc, &query, &sel))
}

#[tauri::command]
pub async fn recent_files(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(lock(&state.recent, "recent")?.clone())
}

#[tauri::command]
pub async fn list_templates() -> CmdResult<Vec<TemplateInfo>> {
    Ok(files::list_templates())
}
