//! One `#[tauri::command]` per entry in docs/api.md. Each locks the service
//! and forwards to the matching [`DocumentService`] method. Errors are
//! [`ApiError`] (`{ code, message }`), which Tauri serializes as the
//! rejection value of the JS promise.
//!
//! Commands are `async` so they run on Tauri's async runtime instead of the
//! main (UI) thread, and each engine call runs on a blocking worker
//! ([`blocking`]): the service mutex is a std `Mutex` and an open or recalc
//! can take seconds, which must not park the async runtime's workers.
//!
//! Every engine call runs under `catch_unwind` ([`with_service`]): a panic
//! becomes an `internal` error for that one call, and the other documents
//! (and their unsaved work) stay usable.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use tally_api::{
    ApiError, ApplyResult, AutoFilterInfo, CellBlock, CellEditInfo, CellPos, ClipboardOut,
    ConditionalFormatDto, Direction, DocId, DocPropertiesDto, DocumentService, FindQuery,
    FindResult, FunctionInfo, GoalSeekRequest, GoalSeekResult, LocaleInfo, NameInfo, NoteDto,
    PageSetupDto, ProtectionInfo, RangeDto, SelectionDto, SelectionStats, SheetLayout, StyleDto,
    Suggestion, Transaction, ValidationDto, ValidationInfo, WorkbookInfo,
};
use tally_types::SheetId;
use tauri::{AppHandle, Manager, Runtime};

use crate::files::{self, TemplateInfo};
use crate::limits;
use crate::links;
use crate::state::{lock, AppState};
use tally_api::FileFormat;

/// Result of every command.
pub type CmdResult<T> = Result<T, ApiError>;

/// Describes a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

/// Runs `f` with the locked service. A panic inside `f` is caught while the
/// guard is still held, so the mutex is not poisoned, and is returned as an
/// `internal` error.
pub fn with_service<T>(
    state: &AppState,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<T>,
) -> CmdResult<T> {
    let mut guard = lock(&state.service);
    let svc: &mut dyn DocumentService = guard.as_mut();
    match catch_unwind(AssertUnwindSafe(|| f(svc))) {
        Ok(result) => result,
        Err(payload) => Err(ApiError::new(
            ApiError::INTERNAL,
            format!("the engine failed: {}", panic_message(payload.as_ref())),
        )),
    }
}

/// Runs `f` on a blocking worker thread with the app state.
pub async fn blocking<T, R, F>(app: AppHandle<R>, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    R: Runtime,
    F: FnOnce(&AppHandle<R>, &AppState) -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        f(&app, state.inner())
    })
    .await
    .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("worker failed: {e}")))?
}

/// Remembers the document's file (for Save) from a returned info.
pub fn track_path(state: &AppState, info: &WorkbookInfo) -> CmdResult<()> {
    let mut paths = lock(&state.paths);
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
pub async fn wb_new(app: AppHandle, template: Option<String>) -> CmdResult<WorkbookInfo> {
    if let Some(t) = &template {
        limits::text_max("the template id", t, limits::MAX_PATH)?;
    }
    blocking(app, move |_, st| new_workbook(st, template.as_deref())).await
}

#[tauri::command]
pub async fn wb_open(app: AppHandle, path: String) -> CmdResult<WorkbookInfo> {
    limits::text_max("the path", &path, limits::MAX_PATH)?;
    blocking(app, move |app, st| {
        open_checked(app, st, std::path::Path::new(&path))
    })
    .await
}

#[tauri::command]
pub async fn wb_close(app: AppHandle, doc: DocId) -> CmdResult<()> {
    blocking(app, move |app, st| {
        with_service(st, |s| s.close(doc))?;
        lock(&st.paths).remove(&doc.0);
        crate::autosave::remove_snapshot(app, doc);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn sheet_layout(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<SheetLayout> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.sheet_layout(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn get_cells(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    ranges: Vec<RangeDto>,
) -> CmdResult<Vec<CellBlock>> {
    limits::ranges("get_cells", &ranges)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.cells(doc, sheet, &ranges))
    })
    .await
}

#[tauri::command]
pub async fn get_styles(app: AppHandle, doc: DocId, ids: Vec<u32>) -> CmdResult<Vec<StyleDto>> {
    if ids.len() > limits::MAX_STYLE_IDS {
        return Err(ApiError::invalid_argument("too many style ids"));
    }
    blocking(app, move |_, st| with_service(st, |s| s.styles(doc, &ids))).await
}

#[tauri::command]
pub async fn cell_edit_info(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<CellEditInfo> {
    limits::pos("cell_edit_info", r, c)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.cell_edit_info(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn apply(app: AppHandle, doc: DocId, tx: Transaction) -> CmdResult<ApplyResult> {
    limits::transaction(&tx)?;
    blocking(app, move |app, st| {
        fixed(app, st, with_service(st, |s| s.apply(doc, tx)))
    })
    .await
}

/// W5-6: puts the shell's path/title/dirty view onto a result's `info` (autosave.rs).
fn fixed<R: Runtime>(
    app: &AppHandle<R>,
    st: &AppState,
    res: CmdResult<ApplyResult>,
) -> CmdResult<ApplyResult> {
    let mut res = res?;
    crate::autosave::fix_result(app, st, &mut res);
    Ok(res)
}

#[tauri::command]
pub async fn undo(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |app, st| {
        fixed(app, st, with_service(st, |s| s.undo(doc)))
    })
    .await
}

#[tauri::command]
pub async fn redo(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |app, st| {
        fixed(app, st, with_service(st, |s| s.redo(doc)))
    })
    .await
}

#[tauri::command]
pub async fn selection_stats(
    app: AppHandle,
    doc: DocId,
    sel: SelectionDto,
) -> CmdResult<SelectionStats> {
    limits::selection("selection_stats", &sel)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.selection_stats(doc, &sel))
    })
    .await
}

#[tauri::command]
pub async fn clipboard_copy(
    app: AppHandle,
    doc: DocId,
    sel: SelectionDto,
) -> CmdResult<ClipboardOut> {
    limits::selection("clipboard_copy", &sel)?;
    blocking(app, move |_, st| with_service(st, |s| s.copy(doc, &sel))).await
}

#[tauri::command]
pub async fn navigate_edge(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    from: CellPos,
    dir: Direction,
) -> CmdResult<CellPos> {
    limits::cell("navigate_edge", &from)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.navigate_edge(doc, sheet, from, dir))
    })
    .await
}

#[tauri::command]
pub async fn resolve_name_box(app: AppHandle, doc: DocId, text: String) -> CmdResult<SelectionDto> {
    limits::chars_max("the name box text", &text, limits::MAX_NAME_BOX_CHARS)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.resolve_name_box(doc, &text))
    })
    .await
}

#[tauri::command]
pub async fn list_functions(app: AppHandle) -> CmdResult<Vec<FunctionInfo>> {
    blocking(app, move |_, st| {
        with_service(st, |s| Ok(s.list_functions()))
    })
    .await
}

#[tauri::command]
pub async fn assist_suggest(
    app: AppHandle,
    doc: DocId,
    query: String,
    sel: SelectionDto,
) -> CmdResult<Vec<Suggestion>> {
    limits::text("the question", &query)?;
    limits::selection("assist_suggest", &sel)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.assist_suggest(doc, &query, &sel))
    })
    .await
}

#[tauri::command]
pub async fn recent_files(app: AppHandle) -> CmdResult<Vec<String>> {
    Ok(lock(&app.state::<AppState>().recent).clone())
}

#[tauri::command]
pub async fn list_templates() -> CmdResult<Vec<TemplateInfo>> {
    Ok(files::list_templates())
}

// ----- v0.2 (1:1 over the new DocumentService methods) ----------------------

#[tauri::command]
pub async fn names(app: AppHandle, doc: DocId) -> CmdResult<Vec<NameInfo>> {
    blocking(app, move |_, st| with_service(st, |s| s.names(doc))).await
}

#[tauri::command]
pub async fn filter_info(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<AutoFilterInfo> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.filter_info(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn find(app: AppHandle, doc: DocId, query: FindQuery) -> CmdResult<FindResult> {
    limits::find_query(&query)?;
    blocking(app, move |_, st| with_service(st, |s| s.find(doc, &query))).await
}

#[tauri::command]
pub async fn detect_header(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    range: RangeDto,
) -> CmdResult<bool> {
    limits::range("detect_header", &range)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.detect_header(doc, sheet, range))
    })
    .await
}

#[tauri::command]
pub async fn recalc_all(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |app, st| {
        fixed(app, st, with_service(st, |s| s.recalc_all(doc)))
    })
    .await
}

// ----- v0.3 (1:1 over the new DocumentService methods) ----------------------

#[tauri::command]
pub async fn conditional_formats(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<Vec<ConditionalFormatDto>> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.conditional_formats(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn validations(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<Vec<ValidationDto>> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.validations(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn validation_at(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<Option<ValidationInfo>> {
    limits::pos("validation_at", r, c)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.validation_at(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn note(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<Option<NoteDto>> {
    limits::pos("note", r, c)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.note(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn notes(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<Vec<NoteDto>> {
    blocking(app, move |_, st| with_service(st, |s| s.notes(doc, sheet))).await
}

#[tauri::command]
pub async fn protection_info(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<ProtectionInfo> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.protection_info(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn page_setup(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<PageSetupDto> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.page_setup(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn document_properties(app: AppHandle, doc: DocId) -> CmdResult<DocPropertiesDto> {
    blocking(app, move |_, st| {
        with_service(st, |s| s.document_properties(doc))
    })
    .await
}

#[tauri::command]
pub async fn goal_seek(
    app: AppHandle,
    doc: DocId,
    req: GoalSeekRequest,
) -> CmdResult<GoalSeekResult> {
    limits::goal_seek(&req)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.goal_seek(doc, &req))
    })
    .await
}

#[tauri::command]
pub async fn list_locales(app: AppHandle) -> CmdResult<Vec<LocaleInfo>> {
    blocking(app, move |_, st| with_service(st, |s| Ok(s.list_locales()))).await
}

#[tauri::command]
pub async fn set_locale(app: AppHandle, doc: DocId, locale: String) -> CmdResult<ApplyResult> {
    limits::text_max("the locale", &locale, limits::MAX_LOCALE)?;
    blocking(app, move |app, st| {
        fixed(app, st, with_service(st, |s| s.set_locale(doc, &locale)))
    })
    .await
}

#[tauri::command]
pub async fn resolve_name_box_in(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    text: String,
) -> CmdResult<SelectionDto> {
    limits::chars_max("the name box text", &text, limits::MAX_NAME_BOX_CHARS)?;
    blocking(app, move |_, st| {
        with_service(st, |s| s.resolve_name_box_in(doc, sheet, &text))
    })
    .await
}

// ----- v0.3 shell commands ---------------------------------------------------

/// `open_url`: opens an http(s) or mailto link with the OS default handler
/// (see [`links::check_url`] for what is accepted).
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> CmdResult<()> {
    let url = links::check_url(&url)?;
    blocking(app, move |_, _| {
        links::open_url_with(&url, links::launch_os)
    })
    .await
}

/// `user_name`: the OS account name (note authors), or `null`.
#[tauri::command]
pub async fn user_name() -> CmdResult<Option<String>> {
    Ok(links::user_name())
}
