//! One `#[tauri::command]` per entry in docs/api.md. Each locks its
//! document's service ([`with_doc`]; see `state.rs` for the per-document
//! locks and the lock order) and forwards to the matching
//! [`DocumentService`] method. Errors are
//! [`ApiError`] (`{ code, message }`), which Tauri serializes as the
//! rejection value of the JS promise.
//!
//! Commands are `async` so they run on Tauri's async runtime instead of the
//! main (UI) thread, and each engine call runs on a blocking worker
//! ([`blocking`]): a document's mutex is a std `Mutex` and an open or recalc
//! can take seconds, which must not park the async runtime's workers.
//!
//! Every engine call runs under `catch_unwind` ([`with_doc`]): a panic
//! becomes an `internal` error for that one call, and the other documents
//! (and their unsaved work) stay usable.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use tally_api::{
    ApiError, ApplyResult, AutoFilterInfo, CellBlock, CellEditInfo, CellPos, ClipboardOut,
    ConditionalFormatDto, Direction, DocId, DocPropertiesDto, DocumentService, FindQuery,
    FindResult, FunctionInfo, GoalSeekRequest, GoalSeekResult, LocaleInfo, NameInfo, NoteDto,
    PageSetupDto, PasteOptions, PastePayload, ProtectionInfo, RangeDto, SelectionDto,
    SelectionStats, SheetLayout, StyleDto, Suggestion, Transaction, ValidationDto, ValidationInfo,
    WorkbookInfo,
};
use tally_types::SheetId;
use tauri::{AppHandle, Manager, Runtime};

use crate::doclock::{self, Acquired};
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

/// Runs `f` on `svc`; a panic inside `f` is returned as an `internal` error.
/// Callers hold `svc`'s lock across this, so the panic is caught before the
/// guard drops and the mutex is not poisoned.
pub fn guarded<T>(
    svc: &mut dyn DocumentService,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<T>,
) -> CmdResult<T> {
    match catch_unwind(AssertUnwindSafe(|| f(svc))) {
        Ok(result) => result,
        Err(payload) => Err(ApiError::new(
            ApiError::INTERNAL,
            format!("the engine failed: {}", panic_message(payload.as_ref())),
        )),
    }
}

/// Runs `f` with `doc`'s service locked (only that document's lock; an
/// unknown `doc` is `not_found`). Other documents stay usable meanwhile.
pub fn with_doc<T>(
    state: &AppState,
    doc: DocId,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<T>,
) -> CmdResult<T> {
    let slot = state.doc(doc)?;
    let mut guard = lock(&slot);
    guarded(&mut *guard, f)
}

/// Runs a document-free call (function list, locales, a print job's pages
/// or PDF) on a detached service: no lock is held while `f` runs.
pub fn with_engine<T>(
    state: &AppState,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<T>,
) -> CmdResult<T> {
    guarded(&mut state.detached(), f)
}

/// Opens or creates one document with `f` on a fresh detached service
/// (no lock held, so a slow open blocks nobody), then files that service as
/// the new document's.
pub fn with_new_doc(
    state: &AppState,
    f: impl FnOnce(&mut dyn DocumentService) -> CmdResult<WorkbookInfo>,
) -> CmdResult<WorkbookInfo> {
    let mut svc = state.detached();
    let info = guarded(&mut svc, f)?;
    state.insert_doc(info.doc, svc);
    Ok(info)
}

/// Closes `doc` in the engine and forgets its service (also when the engine
/// close fails, so a broken document cannot linger). Its lock file goes
/// (if it is still ours).
pub fn close_engine_doc(state: &AppState, doc: DocId) -> CmdResult<()> {
    let result = with_doc(state, doc, |s| s.close(doc));
    state.remove_doc(doc);
    state.locks.forget(doc.0);
    // A CSV save waiting for its options dies with the document.
    lock(&state.pending_csv).remove(&doc.0);
    result
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
/// check and no lock; callers check first).
pub fn open_path(state: &AppState, path: &std::path::Path) -> CmdResult<WorkbookInfo> {
    let info = with_new_doc(state, |s| s.open(path))?;
    track_path(state, &info)?;
    state.locks.remember(info.doc.0, path);
    Ok(info)
}

/// How a file is opened (`wb_open(path, mode)`, the lock dialog's answers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// For editing: lock it, or report someone else's lock (`locked`).
    Normal,
    /// Read-only (File ▸ Open Read-Only…, the lock dialog's Open Read-Only):
    /// no lock.
    ReadOnly,
    /// An untitled editable copy (the lock dialog's Open Copy): no path, no lock.
    Copy,
    /// For editing, taking over a lock of this OS account written
    /// elsewhere (the "locked by yourself" dialog's Open).
    IgnoreLock,
}

/// Opens `canon` (already allowlisted and canonical) as AOO's
/// `LockOrigFileOnDemand` decides: a file Tally cannot write opens
/// read-only; otherwise it is locked, or someone else's lock is reported
/// with code `locked` (the UI asks, then opens again with its answer).
pub fn open_with_mode(
    state: &AppState,
    canon: &std::path::Path,
    mode: OpenMode,
) -> CmdResult<WorkbookInfo> {
    match mode {
        OpenMode::Copy => {
            let info = with_new_doc(state, |s| s.open_untitled(canon, false))?;
            track_path(state, &info)?;
            Ok(info)
        }
        OpenMode::ReadOnly => open_read_only(state, canon),
        OpenMode::Normal | OpenMode::IgnoreLock => {
            if !doclock::is_writable(canon) {
                return open_read_only(state, canon);
            }
            let me = state.locks.identity();
            match state
                .locks
                .try_acquire(None, canon, mode == OpenMode::IgnoreLock)
            {
                Acquired::Busy(info) => Err(info.error()),
                Acquired::Unlocked => open_path(state, canon),
                Acquired::Locked(lp) => match open_path(state, canon) {
                    Ok(info) => {
                        state.locks.adopt(info.doc.0, lp);
                        Ok(info)
                    }
                    Err(e) => {
                        doclock::release(me, &lp);
                        Err(e)
                    }
                },
            }
        }
    }
}

/// [`open_with_mode`] once per file: the file must not be open in another
/// document or being opened right now (checked and reserved in one step, so
/// two concurrent opens of one file cannot both lock it; the loser gets
/// `cancelled`, which the UI ignores). An untitled copy is always allowed.
pub fn open_unique(
    state: &AppState,
    canon: &std::path::Path,
    mode: OpenMode,
) -> CmdResult<WorkbookInfo> {
    if mode == OpenMode::Copy {
        return open_with_mode(state, canon, mode);
    }
    let already = || ApiError::new(ApiError::CANCELLED, "already open");
    let _reserved = {
        let paths = lock(&state.paths);
        if doc_with_path(paths.iter(), canon).is_some() {
            return Err(already());
        }
        state.locks.reserve(canon).ok_or_else(already)?
    };
    open_with_mode(state, canon, mode)
}

/// Opens `canon` read-only (no lock).
fn open_read_only(state: &AppState, canon: &std::path::Path) -> CmdResult<WorkbookInfo> {
    let info = open_path(state, canon)?;
    if info.read_only {
        return Ok(info);
    }
    with_doc(state, info.doc, |s| s.set_read_only(info.doc, true))
}

/// Result of `wb_set_read_only`.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditModeResult {
    /// The document now.
    pub info: WorkbookInfo,
    /// It was read again from disk (it changed there, or changes were
    /// discarded).
    pub reloaded: bool,
}

/// How Edit Mode is switched (`wb_set_read_only(doc, readOnly, mode)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    /// Plain switch.
    Plain,
    /// To read-only: discard unsaved changes (read the file again), AOO's
    /// reload after "Don't Save".
    Discard,
    /// To edit: make it an untitled editable copy (AOO's "edit a copy"
    /// when the file is locked or cannot be written).
    Copy,
    /// To edit: take over a lock of this OS account written elsewhere.
    IgnoreLock,
}

/// Edit ▸ Edit Mode (AOO's `SID_EDITDOC`, `sfx2/source/view/viewfrm.cxx`).
///
/// To read-only: the document must be saved (or `Discard` reads the file
/// again); its lock is released. To edit: a document opened read-only
/// only because of `LoadReadonly` still holds its lock and just switches
/// (AOO's "read-only UI"); otherwise the file must be writable
/// (`not_writable`) and lockable (`locked`), and it is read again if it
/// changed on disk meanwhile (AOO always reloads here).
pub fn set_edit_mode(
    state: &AppState,
    doc: DocId,
    read_only: bool,
    mode: EditMode,
) -> CmdResult<EditModeResult> {
    let path = lock(&state.paths).get(&doc.0).cloned();
    if read_only {
        let mut reloaded = false;
        if mode == EditMode::Discard && path.is_some() {
            with_doc(state, doc, |s| s.reload(doc))?;
            reloaded = true;
        }
        let info = with_doc(state, doc, |s| s.set_read_only(doc, true))?;
        state.locks.release(doc.0);
        if let Some(p) = &path {
            state.locks.remember(doc.0, p);
        }
        return Ok(EditModeResult { info, reloaded });
    }
    let Some(path) = path else {
        let info = with_doc(state, doc, |s| s.set_read_only(doc, false))?;
        return Ok(EditModeResult {
            info,
            reloaded: false,
        });
    };
    if mode == EditMode::Copy {
        let info = with_doc(state, doc, |s| s.detach_path(doc))?;
        track_path(state, &info)?;
        state.locks.forget(doc.0);
        return Ok(EditModeResult {
            info,
            reloaded: false,
        });
    }
    let current = with_doc(state, doc, |s| s.info(doc))?;
    if !current.read_only {
        return Ok(EditModeResult {
            info: current,
            reloaded: false,
        });
    }
    if !(current.load_readonly && state.locks.holds(doc.0)) {
        if !doclock::is_writable(&path) {
            return Err(ApiError::new(
                ApiError::NOT_WRITABLE,
                "This document cannot be edited, possibly due to missing access rights.",
            ));
        }
        if let Acquired::Busy(info) =
            state
                .locks
                .acquire(doc.0, &path, mode == EditMode::IgnoreLock)
        {
            return Err(info.error());
        }
    }
    let reloaded = state.locks.changed_on_disk(doc.0, &path);
    if reloaded {
        if let Err(e) = with_doc(state, doc, |s| s.reload(doc)) {
            state.locks.release(doc.0);
            return Err(e);
        }
        state.locks.remember(doc.0, &path);
    }
    let info = with_doc(state, doc, |s| s.set_read_only(doc, false))?;
    Ok(EditModeResult { info, reloaded })
}

/// Pure: the open document whose file is `canon`, among the remembered
/// files (`paths`, doc id → path; compared as given and canonicalized).
pub fn doc_with_path<'a>(
    paths: impl IntoIterator<Item = (&'a u32, &'a PathBuf)>,
    canon: &std::path::Path,
) -> Option<DocId> {
    paths
        .into_iter()
        .find(|(_, p)| p.as_path() == canon || std::fs::canonicalize(p).is_ok_and(|c| c == canon))
        .map(|(d, _)| DocId(*d))
}

/// Opens an allowlisted path and records it in the recent list. A file that
/// is already open is not opened twice: its window comes to the front with
/// its tab shown and the result is `cancelled` (the UI shows nothing).
/// Someone else's lock is reported as `locked` ([`open_with_mode`]).
pub fn open_checked<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    path: &std::path::Path,
    mode: OpenMode,
) -> CmdResult<WorkbookInfo> {
    let canon = files::check_open_allowed(state, path)?;
    let open_doc = doc_with_path(lock(&state.paths).iter(), &canon);
    if let Some(doc) = open_doc {
        let tab = app
            .try_state::<crate::registry::Windows>()
            .and_then(|w| w.with(|r| r.tab_of(doc)));
        if let Some(label) = tab {
            crate::windows::reveal(app, &label);
            files::push_recent(app, state, &canon);
            return Err(ApiError::new(ApiError::CANCELLED, "already open"));
        }
    }
    let info = open_unique(state, &canon, mode)?;
    files::push_recent(app, state, &canon);
    Ok(info)
}

/// `wb_new`: a blank workbook, or one built by the engine from a template
/// id (`invoices`, `budget`, `stock`; unknown ids are `not_found`).
pub fn new_workbook(state: &AppState, template: Option<&str>) -> CmdResult<WorkbookInfo> {
    let info = with_new_doc(state, |s| s.new_workbook(template))?;
    track_path(state, &info)?;
    Ok(info)
}

/// Saves `doc` to `path` (already chosen in a dialog or remembered) and
/// remembers it for the next Save. Saving to another file than the
/// document's own refuses a file someone else has locked (`locked`, as
/// AOO's access-denied on a locked target) and moves the document's lock
/// there (a read-only document becomes editable: Save As makes an editable
/// copy).
pub fn save_to(
    state: &AppState,
    doc: DocId,
    path: &std::path::Path,
    format: FileFormat,
) -> CmdResult<WorkbookInfo> {
    let old = lock(&state.paths).get(&doc.0).cloned();
    let moving = !old.as_deref().is_some_and(|o| same_file(o, path));
    if moving {
        // Another of our documents has that file open (AOO: locked for
        // editing, by us): writing it would leave two documents on one file
        // and one lock.
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let other = doc_with_path(
            lock(&state.paths).iter().filter(|(d, _)| **d != doc.0),
            &canon,
        );
        if other.is_some() {
            let me = state.locks.identity();
            let lp = doclock::lock_path(path);
            return Err(doclock::LockInfo::ours(path, me, lp.as_deref()).error());
        }
        if let Some(busy) = state.locks.foreign(path, doc.0) {
            return Err(busy.error());
        }
    }
    let info = with_doc(state, doc, |s| s.save(doc, Some(path), format))?;
    track_path(state, &info)?;
    if moving {
        state.locks.release(doc.0);
        if !info.read_only {
            // Best effort: a directory Tally cannot write a lock into is
            // fine, and the target was checked above.
            let _ = state.locks.acquire(doc.0, path, false);
        }
    }
    state.locks.remember(doc.0, path);
    Ok(info)
}

/// `true` if `a` and `b` name the same file (as given or canonicalized).
pub fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

#[tauri::command]
pub async fn wb_new(app: AppHandle, template: Option<String>) -> CmdResult<WorkbookInfo> {
    if let Some(t) = &template {
        limits::text_max("the template id", t, limits::MAX_PATH)?;
    }
    blocking(app, move |_, st| new_workbook(st, template.as_deref())).await
}

#[tauri::command]
pub async fn wb_open(
    app: AppHandle,
    path: String,
    mode: Option<String>,
) -> CmdResult<WorkbookInfo> {
    limits::text_max("the path", &path, limits::MAX_PATH)?;
    let mode = limits::open_mode(mode.as_deref())?;
    blocking(app, move |app, st| {
        open_checked(app, st, std::path::Path::new(&path), mode)
    })
    .await
}

/// `wb_set_read_only(doc, readOnly, mode)`: Edit ▸ Edit Mode
/// ([`set_edit_mode`]).
#[tauri::command]
pub async fn wb_set_read_only(
    app: AppHandle,
    doc: DocId,
    read_only: bool,
    mode: Option<String>,
) -> CmdResult<EditModeResult> {
    let mode = limits::edit_mode(read_only, mode.as_deref())?;
    blocking(app, move |_, st| set_edit_mode(st, doc, read_only, mode)).await
}

#[tauri::command]
pub async fn wb_close(app: AppHandle, doc: DocId) -> CmdResult<()> {
    blocking(app, move |app, st| {
        close_engine_doc(st, doc)?;
        lock(&st.paths).remove(&doc.0);
        crate::autosave::remove_snapshot(app, doc);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn sheet_layout(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<SheetLayout> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.sheet_layout(doc, sheet))
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
        with_doc(st, doc, |s| s.cells(doc, sheet, &ranges))
    })
    .await
}

#[tauri::command]
pub async fn get_styles(app: AppHandle, doc: DocId, ids: Vec<u32>) -> CmdResult<Vec<StyleDto>> {
    if ids.len() > limits::MAX_STYLE_IDS {
        return Err(ApiError::invalid_argument("too many style ids"));
    }
    blocking(app, move |_, st| with_doc(st, doc, |s| s.styles(doc, &ids))).await
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
        with_doc(st, doc, |s| s.cell_edit_info(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn apply(app: AppHandle, doc: DocId, tx: Transaction) -> CmdResult<ApplyResult> {
    limits::transaction(&tx)?;
    blocking(app, move |_, st| with_doc(st, doc, |s| s.apply(doc, tx))).await
}

#[tauri::command]
pub async fn undo(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.undo(doc))).await
}

#[tauri::command]
pub async fn redo(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.redo(doc))).await
}

#[tauri::command]
pub async fn selection_stats(
    app: AppHandle,
    doc: DocId,
    sel: SelectionDto,
) -> CmdResult<SelectionStats> {
    limits::selection("selection_stats", &sel)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.selection_stats(doc, &sel))
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
    blocking(app, move |_, st| with_doc(st, doc, |s| s.copy(doc, &sel))).await
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
        with_doc(st, doc, |s| s.navigate_edge(doc, sheet, from, dir))
    })
    .await
}

#[tauri::command]
pub async fn next_unlocked(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    from: CellPos,
    dir: Direction,
    within: Option<RangeDto>,
) -> CmdResult<Option<CellPos>> {
    limits::cell("next_unlocked", &from)?;
    if let Some(w) = &within {
        limits::range("next_unlocked", w)?;
    }
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.next_unlocked(doc, sheet, from, dir, within))
    })
    .await
}

#[tauri::command]
pub async fn data_region(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    range: RangeDto,
) -> CmdResult<RangeDto> {
    limits::range("data_region", &range)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.data_region(doc, sheet, range))
    })
    .await
}

#[tauri::command]
pub async fn paste_overwrites(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    target: RangeDto,
    payload: PastePayload,
    options: PasteOptions,
) -> CmdResult<bool> {
    limits::range("paste_overwrites", &target)?;
    limits::paste_payload(&payload)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| {
            s.paste_overwrites(doc, sheet, target, payload, options)
        })
    })
    .await
}

#[tauri::command]
pub async fn resolve_name_box(app: AppHandle, doc: DocId, text: String) -> CmdResult<SelectionDto> {
    limits::chars_max("the name box text", &text, limits::MAX_NAME_BOX_CHARS)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.resolve_name_box(doc, &text))
    })
    .await
}

#[tauri::command]
pub async fn list_functions(app: AppHandle) -> CmdResult<Vec<FunctionInfo>> {
    blocking(app, move |_, st| {
        with_engine(st, |s| Ok(s.list_functions()))
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
        with_doc(st, doc, |s| s.assist_suggest(doc, &query, &sel))
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
    blocking(app, move |_, st| with_doc(st, doc, |s| s.names(doc))).await
}

#[tauri::command]
pub async fn filter_info(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<AutoFilterInfo> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.filter_info(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn find(app: AppHandle, doc: DocId, query: FindQuery) -> CmdResult<FindResult> {
    limits::find_query(&query)?;
    blocking(app, move |_, st| with_doc(st, doc, |s| s.find(doc, &query))).await
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
        with_doc(st, doc, |s| s.detect_header(doc, sheet, range))
    })
    .await
}

#[tauri::command]
pub async fn recalc_all(app: AppHandle, doc: DocId) -> CmdResult<ApplyResult> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.recalc_all(doc))).await
}

// ----- v0.3 (1:1 over the new DocumentService methods) ----------------------

#[tauri::command]
pub async fn conditional_formats(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<Vec<ConditionalFormatDto>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.conditional_formats(doc, sheet))
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
        with_doc(st, doc, |s| s.validations(doc, sheet))
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
        with_doc(st, doc, |s| s.validation_at(doc, sheet, r, c))
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
        with_doc(st, doc, |s| s.note(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn notes(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<Vec<NoteDto>> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.notes(doc, sheet))).await
}

#[tauri::command]
pub async fn protection_info(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
) -> CmdResult<ProtectionInfo> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.protection_info(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn page_setup(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<PageSetupDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.page_setup(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn document_properties(app: AppHandle, doc: DocId) -> CmdResult<DocPropertiesDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.document_properties(doc))
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
        with_doc(st, doc, |s| s.goal_seek(doc, &req))
    })
    .await
}

#[tauri::command]
pub async fn list_locales(app: AppHandle) -> CmdResult<Vec<LocaleInfo>> {
    blocking(app, move |_, st| with_engine(st, |s| Ok(s.list_locales()))).await
}

#[tauri::command]
pub async fn set_locale(app: AppHandle, doc: DocId, locale: String) -> CmdResult<ApplyResult> {
    limits::text_max("the locale", &locale, limits::MAX_LOCALE)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.set_locale(doc, &locale))
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
        with_doc(st, doc, |s| s.resolve_name_box_in(doc, sheet, &text))
    })
    .await
}

/// `set_active_sheet(doc, sheet)`: the sheet the user switched to (view
/// state; no undo step, the document stays unmodified). The arguments are
/// two ids, checked by the engine (`not_found`).
#[tauri::command]
pub async fn set_active_sheet(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<()> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.set_active_sheet(doc, sheet))
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
