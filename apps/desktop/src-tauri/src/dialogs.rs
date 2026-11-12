//! Commands that show native file dialogs. The dialog plugin is driven from
//! Rust only (no `dialog:*` permission is granted to JS), and dialogs run on
//! a blocking worker thread so the UI thread is never blocked.

use std::path::{Path, PathBuf};

use tally_api::{ApiError, CsvExportOptions, DocId, FileFormat, WorkbookInfo};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{blocking, open_checked, save_to, with_doc, CmdResult, OpenMode};
use crate::files;
use crate::state::{lock, AppState};

fn cancelled() -> ApiError {
    ApiError::new(ApiError::CANCELLED, "cancelled")
}

fn join_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ApiError::INTERNAL, format!("dialog thread failed: {e}"))
}

fn ext_of(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Ods => "ods",
        FileFormat::Xlsx => "xlsx",
        FileFormat::Csv => "csv",
        FileFormat::Html => "html",
        FileFormat::Dbf => "dbf",
    }
}

/// Format from a file extension (unknown → ODS). For a format Tally only
/// reads, this is the format Save As offers first: XLSX for the Excel
/// family (`.xlsm`, `.xltx`, `.xltm`, `.xls`, `.xlt`, `.xlw`, Excel 2003
/// XML), ODS for the rest (`.fods`, `.sxc`, `.slk`, `.dif`, …).
pub fn format_for(path: &Path) -> FileFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        // The text family is saved as CSV with the import's delimiter (a
        // `.tsv` keeps its tabs), never as an ODS zip in a text file.
        Some("csv" | "tsv" | "txt" | "tab") => FileFormat::Csv,
        Some("xlsx" | "xlsm" | "xltx" | "xltm" | "xls" | "xlt" | "xlw" | "xml") => FileFormat::Xlsx,
        Some("html") | Some("htm") => FileFormat::Html,
        Some("dbf") => FileFormat::Dbf,
        _ => FileFormat::Ods,
    }
}

/// `true` if Save can write `path` in its own format. A file Tally only
/// reads goes through the Save As dialog instead of being overwritten with
/// another format: XLS, SXC, flat ODS, SYLK, DIF, Excel 2003 XML, and the
/// macro-enabled and template OOXML files (`.xlsm`, `.xltm`, `.xltx`):
/// Tally drops macros and writes plain workbooks, and Excel refuses an
/// `.xlsm` without its VBA project and content type.
pub fn writable_in_place(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ods" | "ots" | "xlsx" | "csv" | "tsv" | "txt" | "tab" | "html" | "htm" | "dbf")
    )
}

/// Open dialog filters: every format the engine reads first, then one per
/// format (AOO's filter names), then All Files (any file is sniffed by its
/// content, `codec::detect`). docs/file-formats.md lists the formats.
pub const OPEN_FILTERS: &[(&str, &[&str])] = &[
    (
        "All Spreadsheets",
        &[
            "ods", "ots", "fods", "xlsx", "xlsm", "xltx", "xltm", "xls", "xlt", "xlw", "xml",
            "sxc", "stc", "slk", "dif", "csv", "tsv", "txt", "tab", "html", "htm", "xhtml", "dbf",
        ],
    ),
    ("ODF Spreadsheet", &["ods", "ots"]),
    ("ODF Spreadsheet (Flat XML)", &["fods"]),
    (
        "Microsoft Excel 2007–365",
        &["xlsx", "xlsm", "xltx", "xltm"],
    ),
    ("Microsoft Excel 97–2003", &["xls", "xlt", "xlw"]),
    ("Microsoft Excel 2003 XML", &["xml"]),
    ("OpenOffice.org 1.0 Spreadsheet", &["sxc", "stc"]),
    ("SYLK", &["slk"]),
    ("Data Interchange Format", &["dif"]),
    ("Text CSV", &["csv", "tsv", "txt", "tab"]),
    ("HTML Document", &["html", "htm", "xhtml"]),
    ("dBase", &["dbf"]),
    ("All Files", &["*"]),
];

/// The filters the Open panel gets. macOS's panel has no filter menu: the
/// dialog plugin (rfd) merges every filter into one list of allowed
/// extensions, where `*` matches nothing, so no filter is set there and
/// every file can be chosen (AOO's default filter is "All files" too).
pub fn open_dialog_filters() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[]
    } else {
        OPEN_FILTERS
    }
}

/// Save dialog formats as (label, extension), in menu order.
pub const SAVE_FORMATS: &[(&str, &str)] = &[
    ("OpenDocument Spreadsheet", "ods"),
    ("Excel Workbook", "xlsx"),
    ("CSV", "csv"),
    ("HTML", "html"),
    ("dBase", "dbf"),
];

/// What a Save dialog shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavePlan {
    /// Window title.
    pub title: &'static str,
    /// Suggested file name (stem of the current file, or "Untitled").
    pub file_name: String,
    /// Filters as (label, extension); the first is preselected.
    pub filters: Vec<(&'static str, &'static str)>,
    /// Extension appended when the chosen name has none we know.
    pub ext: &'static str,
    /// The folder the dialog starts in: the current file's (AOO's Save As
    /// opens in the document's folder).
    pub directory: Option<PathBuf>,
}

/// `true` if the path ends in one of [`SAVE_FORMATS`]' extensions (any case).
pub fn has_known_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SAVE_FORMATS.iter().any(|(_, x)| x.eq_ignore_ascii_case(e)))
}

/// The Save dialog for `doc`'s current file and the requested format: an
/// export (`format` set) offers that format only; Save As offers them all,
/// the current file's format first, and the format then follows the chosen
/// extension ([`format_for`]).
pub fn save_plan(current: Option<&Path>, format: Option<FileFormat>) -> SavePlan {
    let stem = current
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_owned();
    let (title, first, filters) = match format {
        Some(f) => {
            let ext = ext_of(f);
            let title = match f {
                FileFormat::Csv => "Export as CSV",
                FileFormat::Xlsx => "Export as Excel Workbook",
                FileFormat::Ods => "Export as ODS",
                FileFormat::Html => "Export as HTML",
                FileFormat::Dbf => "Export as dBase",
            };
            let only: Vec<_> = SAVE_FORMATS
                .iter()
                .copied()
                .filter(|(_, x)| *x == ext)
                .collect();
            (title, ext, only)
        }
        None => {
            let ext = current.map_or("ods", |p| ext_of(format_for(p)));
            let mut all: Vec<_> = SAVE_FORMATS.to_vec();
            all.sort_by_key(|(_, x)| *x != ext);
            ("Save As", ext, all)
        }
    };
    SavePlan {
        title,
        file_name: format!("{stem}.{first}"),
        filters,
        ext: first,
        directory: current
            .and_then(Path::parent)
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf),
    }
}

/// Shows the Open dialog ([`OPEN_FILTERS`]), then opens the chosen file
/// (read-only for File ▸ Open Read-Only…). `None` when the user cancels,
/// or when the file is already open in a window, which is brought to the
/// front instead ([`open_checked`]). A file someone else has locked is
/// `locked` (the UI asks and opens it again with `wb_open(path, mode)`).
pub async fn pick_and_open<R: Runtime>(
    app: AppHandle<R>,
    read_only: bool,
) -> CmdResult<Option<WorkbookInfo>> {
    let handle = app.clone();
    let title = if read_only { "Open Read-Only" } else { "Open" };
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = handle.dialog().file().set_title(title);
        let dialog = open_dialog_filters()
            .iter()
            .fold(dialog, |d, (label, exts)| d.add_filter(*label, exts));
        dialog.blocking_pick_file()
    })
    .await
    .map_err(join_err)?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
    files::allow(&app.state::<AppState>(), &path)?;
    let mode = if read_only {
        OpenMode::ReadOnly
    } else {
        OpenMode::Normal
    };
    match blocking(app, move |app, st| open_checked(app, st, &path, mode)).await {
        Ok(info) => Ok(Some(info)),
        Err(e) if e.code == ApiError::CANCELLED => Ok(None),
        Err(e) => Err(e),
    }
}

/// `wb_open_dialog(readOnly)`: [`pick_and_open`] for the calling webview,
/// which then places the document (`place_document`). Returns `null` when
/// the user cancels or the file is already open in a window (now in front).
#[tauri::command]
pub async fn wb_open_dialog(
    app: AppHandle,
    read_only: Option<bool>,
) -> CmdResult<Option<WorkbookInfo>> {
    pick_and_open(app, read_only.unwrap_or(false)).await
}

/// `load_styles_dialog(doc, opts, label)`: Load Styles ▸ From File…: the
/// Open panel ([`OPEN_FILTERS`]), then the chosen file's styles are loaded
/// into `doc` (`templates::load_styles_file`, the engine's open limits).
/// `null` when the user cancels.
#[tauri::command]
pub async fn load_styles_dialog(
    app: AppHandle,
    doc: DocId,
    opts: tally_api::LoadStylesOptions,
    label: Option<String>,
) -> CmdResult<Option<tally_api::ApplyResult>> {
    if let Some(l) = &label {
        crate::limits::chars_max("the undo label", l, crate::limits::MAX_NAME_CHARS)?;
    }
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = handle.dialog().file().set_title("Load Styles");
        let dialog = open_dialog_filters()
            .iter()
            .fold(dialog, |d, (label, exts)| d.add_filter(*label, exts));
        dialog.blocking_pick_file()
    })
    .await
    .map_err(join_err)?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
    blocking(app, move |_, st| {
        crate::templates::load_styles_file(st, doc, &path, opts, label.as_deref())
    })
    .await
    .map(Some)
}

/// File ▸ Open… with no document window (macOS): the dialog, then a window
/// for the file. Errors are logged (there is no webview to show them). A
/// file someone else has locked opens read-only: with no window to ask in,
/// this is AOO's answer when there is no interaction handler
/// (`SfxMedium::ShowLockedDocumentDialog`).
pub fn spawn_open<R: Runtime>(app: &AppHandle<R>, read_only: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let opened = read_only_if_locked(&app, pick_and_open(app.clone(), read_only).await).await;
        match opened {
            Ok(Some(info)) => crate::windows::spawn_doc_window(&app, Some(info)),
            Ok(None) => {}
            Err(e) => eprintln!("tally: open: {}", e.message),
        }
    });
}

/// With no window to ask in, a file someone else has locked opens
/// read-only (AOO's answer without an interaction handler).
async fn read_only_if_locked<R: Runtime>(
    app: &AppHandle<R>,
    opened: CmdResult<Option<WorkbookInfo>>,
) -> CmdResult<Option<WorkbookInfo>> {
    match opened {
        Err(e) if e.code == ApiError::LOCKED => {
            let path = serde_json::from_str::<serde_json::Value>(&e.message)
                .ok()
                .and_then(|v| v["path"].as_str().map(PathBuf::from));
            match path {
                Some(p) => blocking(app.clone(), move |app, st| {
                    open_checked(app, st, &p, OpenMode::ReadOnly)
                })
                .await
                .map(Some),
                None => Err(e),
            }
        }
        other => other,
    }
}

/// The local files among the URLs the OS asks Tally to open (Finder's
/// double-click and Open With, a file dropped on the Dock icon:
/// `RunEvent::Opened`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn file_paths(urls: &[impl AsRef<str>]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|u| {
            let u = u.as_ref();
            let rest = u.strip_prefix("file://")?;
            // `file:///path` or `file://localhost/path`, percent-encoded.
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            if !rest.starts_with('/') {
                return None;
            }
            let bytes = percent_decode(rest)?;
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
            }
            #[cfg(not(unix))]
            {
                String::from_utf8(bytes).ok().map(PathBuf::from)
            }
        })
        .collect()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// Most files one OS request opens without asking (review M3: Finder can
/// hand over thousands of files at once).
pub const MAX_OS_OPEN_BATCH: usize = 32;

/// Files the OS asked to open before any document webview had started
/// (the launch of Tally by opening a file): the first webview opens them
/// (`window_init` → `WindowInit::open_paths`), the first in place of its
/// untouched blank workbook.
#[derive(Default)]
pub struct OsOpen {
    inner: std::sync::Mutex<OsOpenState>,
}

#[derive(Default)]
struct OsOpenState {
    /// A document webview has started (asked `window_init`).
    started: bool,
    pending: Vec<PathBuf>,
}

impl OsOpen {
    fn lock(&self) -> std::sync::MutexGuard<'_, OsOpenState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `window_init`: marks a webview as started and hands it the files
    /// that were waiting for one.
    pub fn take_pending(&self) -> Vec<PathBuf> {
        let mut st = self.lock();
        st.started = true;
        std::mem::take(&mut st.pending)
    }

    /// Queues `paths` if no webview has started yet; else gives them back.
    fn queue_or_return(&self, paths: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        let mut st = self.lock();
        if st.started {
            Some(paths)
        } else {
            st.pending.extend(paths);
            None
        }
    }
}

/// The prefix of the `menu-action` id that asks a document webview to open
/// a file the OS handed over (`os.open:<path>`; the webview opens it like
/// a recent file, in place of its untouched blank workbook if it has one).
pub const OS_OPEN_ACTION: &str = "os.open:";

fn os_open_error<R: Runtime>(app: &AppHandle<R>, shown: &str, message: &str) {
    eprintln!("tally: open {shown}: {message}");
    app.dialog()
        .message(format!("{shown}\n\n{message}"))
        .title("Tally")
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

/// Opens files the OS hands to Tally (Finder, the Dock, Open With; a second
/// launch with files on the command line, through the single-instance
/// plugin; the command line of the first launch on Windows and Linux). The
/// user chose them, so they join the `wb_open` allowlist. More than
/// [`MAX_OS_OPEN_BATCH`] files ask first (only that many open). Before any
/// webview has started they wait for the first one ([`OsOpen`]); with a
/// document window they go to its front tab (`os.open:` menu action), which
/// opens them as File ▸ Open does (lock questions, already-open check,
/// reuse of an untouched blank workbook); with no window, they open one
/// after another here, each in a new window.
pub fn open_os_files<R: Runtime>(app: &AppHandle<R>, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut paths = paths;
        if paths.len() > MAX_OS_OPEN_BATCH {
            let n = paths.len();
            let handle = app.clone();
            let ok = tauri::async_runtime::spawn_blocking(move || {
                let loc = crate::l10n::current_locale(&handle);
                let t = |k| tally_l10n::t(loc, k);
                handle
                    .dialog()
                    .message(
                        t("msg.os.openMany")
                            .replace("{count}", &n.to_string())
                            .replace("{max}", &MAX_OS_OPEN_BATCH.to_string()),
                    )
                    .title("Tally")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
                    .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
                        t("msg.os.openFirst").replace("{max}", &MAX_OS_OPEN_BATCH.to_string()),
                        t("dialog.cancel").to_string(),
                    ))
                    .blocking_show()
            })
            .await
            .unwrap_or(false);
            if !ok {
                return;
            }
            paths.truncate(MAX_OS_OPEN_BATCH);
        }
        let state = app.state::<AppState>();
        for p in &paths {
            if let Err(e) = files::allow(&state, p) {
                os_open_error(&app, &p.display().to_string(), &e.message);
            }
        }
        let Some(paths) = app.state::<OsOpen>().queue_or_return(paths) else {
            return;
        };
        let front = app
            .try_state::<crate::registry::Windows>()
            .and_then(|w| w.with(|r| r.front()));
        if let Some(label) = front {
            for p in paths {
                let id = format!("{OS_OPEN_ACTION}{}", p.display());
                crate::events::emit_menu_action(&app, &label, &id);
            }
            return;
        }
        // No document window (macOS keeps running without one): one after
        // another, each in a new window.
        for path in paths {
            let shown = path.display().to_string();
            let p = path.clone();
            let res = blocking(app.clone(), move |app, st| {
                open_checked(app, st, &p, OpenMode::Normal)
            })
            .await
            .map(Some);
            match read_only_if_locked(&app, res).await {
                Ok(Some(info)) => crate::windows::spawn_doc_window(&app, Some(info)),
                Ok(None) => {}
                Err(e) if e.code == ApiError::CANCELLED => {}
                Err(e) => os_open_error(&app, &shown, &e.message),
            }
        }
    });
}

/// Pure part of [`confirm_replace`]: the added extension made the name
/// another file, and that file exists (the panel checked only `picked`).
pub fn must_ask_replace(picked: &Path, target: &Path) -> bool {
    picked != target && target.exists()
}

/// The extension Save As adds to a typed name can make it another file
/// than the one the panel checked (`Report.xlsm` → `Report.xlsx`, review
/// M4): if that file exists, ask before replacing it (AOO's "already
/// exists" question). `true`: go on.
fn confirm_replace<R: Runtime>(app: &AppHandle<R>, picked: &Path, target: &Path) -> bool {
    if !must_ask_replace(picked, target) {
        return true;
    }
    let loc = crate::l10n::current_locale(app);
    let t = |k| tally_l10n::t(loc, k);
    let name = target
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    app.dialog()
        .message(t("msg.save.replaceExisting").replace("{name}", &name))
        .title("Tally")
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
            t("msg.save.replace").to_string(),
            t("dialog.cancel").to_string(),
        ))
        .blocking_show()
}

/// Saves `doc`. Shows a Save dialog when `save_as` is set or the document
/// has no file yet; `format` defaults to the chosen file's extension (else
/// ODS), so Save As to `name.xlsx` writes XLSX.
#[tauri::command]
pub async fn wb_save(
    app: AppHandle,
    state: State<'_, AppState>,
    doc: DocId,
    save_as: bool,
    format: Option<FileFormat>,
    csv: Option<CsvExportOptions>,
) -> CmdResult<WorkbookInfo> {
    if let Some(o) = &csv {
        o.delimiters()?;
    }
    let current: Option<PathBuf> = lock(&state.paths).get(&doc.0).cloned();
    let mut from_dialog = false;
    // Exporting to another format always asks where.
    let format_change = match (&current, format) {
        (Some(p), Some(f)) => format_for(p) != f,
        _ => false,
    };
    let target = match current {
        Some(p) if !save_as && !format_change && writable_in_place(&p) => p,
        current => {
            let plan = save_plan(current.as_deref(), format);
            let handle = app.clone();
            let title = plan.title;
            let file_name = plan.file_name.clone();
            let filters = plan.filters.clone();
            let directory = plan.directory.clone();
            let picked = tauri::async_runtime::spawn_blocking(move || {
                let mut dialog = handle
                    .dialog()
                    .file()
                    .set_title(title)
                    .set_file_name(file_name);
                if let Some(dir) = directory.filter(|d| d.is_dir()) {
                    dialog = dialog.set_directory(dir);
                }
                filters
                    .iter()
                    .fold(dialog, |d, (label, ext)| d.add_filter(*label, &[*ext]))
                    .blocking_save_file()
            })
            .await
            .map_err(join_err)?;
            let file = picked.ok_or_else(cancelled)?;
            let picked = file
                .into_path()
                .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
            let mut path = picked.clone();
            if !has_known_ext(&path) {
                path.set_extension(plan.ext);
            }
            if path != picked {
                let (handle, a, b) = (app.clone(), picked.clone(), path.clone());
                let ok =
                    tauri::async_runtime::spawn_blocking(move || confirm_replace(&handle, &a, &b))
                        .await
                        .map_err(join_err)?;
                if !ok {
                    return Err(cancelled());
                }
            }
            files::allow(&state, &path)?;
            from_dialog = true;
            path
        }
    };
    let fmt = format.unwrap_or_else(|| format_for(&target));
    if fmt == FileFormat::Csv && csv.is_none() && from_dialog {
        // As AOO: the Text Export options are asked after the file is
        // chosen; the webview answers with `wb_save_csv` (or drops it).
        let name = target
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        lock(&state.pending_csv).insert(doc.0, target);
        return Err(ApiError::new(ApiError::CSV_OPTIONS, name));
    }
    lock(&state.pending_csv).remove(&doc.0);
    blocking(app, move |app, st| {
        if let Some(o) = &csv {
            with_doc(st, doc, |s| s.set_csv_options(doc, o))?;
        }
        let info = save_to(st, doc, &target, fmt)?;
        files::push_recent(app, st, &target);
        crate::autosave::remove_snapshot(app, doc);
        Ok(info)
    })
    .await
}

/// Finishes a Save whose CSV file was picked by `wb_save` (code
/// `csv_options`): sets the Text Export options and writes the file.
/// `csv: None` drops the pending save (the options dialog was cancelled).
#[tauri::command]
pub async fn wb_save_csv(
    app: AppHandle,
    state: State<'_, AppState>,
    doc: DocId,
    csv: Option<CsvExportOptions>,
) -> CmdResult<Option<WorkbookInfo>> {
    let Some(csv) = csv else {
        lock(&state.pending_csv).remove(&doc.0);
        return Ok(None);
    };
    csv.delimiters()?;
    let target = lock(&state.pending_csv)
        .remove(&doc.0)
        .ok_or_else(|| ApiError::not_found("no CSV save is waiting for options"))?;
    blocking(app, move |app, st| {
        with_doc(st, doc, |s| s.set_csv_options(doc, &csv))?;
        let info = save_to(st, doc, &target, FileFormat::Csv)?;
        files::push_recent(app, st, &target);
        crate::autosave::remove_snapshot(app, doc);
        Ok(Some(info))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_offers_every_format() {
        for ext in [
            "ods", "ots", "fods", "xlsx", "xlsm", "xltx", "xltm", "xls", "xlt", "xlw", "xml",
            "sxc", "stc", "slk", "dif", "csv", "tsv", "txt", "tab", "html", "htm", "xhtml", "dbf",
        ] {
            assert!(OPEN_FILTERS[0].1.contains(&ext), "{ext}");
        }
        // Every per-format filter is part of the combined one, except the
        // last: All Files.
        let (last, rest) = OPEN_FILTERS[1..]
            .split_last()
            .unwrap_or_else(|| unreachable!());
        for (label, exts) in rest {
            assert!(
                exts.iter().all(|e| OPEN_FILTERS[0].1.contains(e)),
                "{label}"
            );
        }
        assert_eq!(*last, ("All Files", &["*"][..]));
        // macOS merges filters into one allowed list: none, so any file opens.
        if cfg!(target_os = "macos") {
            assert!(open_dialog_filters().is_empty());
        } else {
            assert_eq!(open_dialog_filters().len(), OPEN_FILTERS.len());
        }
    }

    #[test]
    fn a_rewritten_save_name_asks_before_replacing() {
        let dir = std::env::temp_dir().join(format!("tally-m4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_default();
        let typed = dir.join("Report.xlsm");
        let mut target = typed.clone();
        assert!(!has_known_ext(&target));
        target.set_extension("xlsx");
        assert!(!must_ask_replace(&typed, &target), "nothing to replace");
        std::fs::write(&target, b"x").unwrap_or_default();
        assert!(must_ask_replace(&typed, &target), "Report.xlsx exists");
        assert!(
            !must_ask_replace(&target, &target),
            "the panel asked already"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn os_opens_wait_for_the_first_webview() {
        let q = OsOpen::default();
        assert!(q.queue_or_return(vec![PathBuf::from("/a.ods")]).is_none());
        assert!(q.queue_or_return(vec![PathBuf::from("/b.ods")]).is_none());
        assert_eq!(
            q.take_pending(),
            [PathBuf::from("/a.ods"), PathBuf::from("/b.ods")]
        );
        // Started: later files go straight to a window.
        assert_eq!(
            q.queue_or_return(vec![PathBuf::from("/c.ods")]),
            Some(vec![PathBuf::from("/c.ods")])
        );
        assert!(q.take_pending().is_empty());
        assert_eq!(MAX_OS_OPEN_BATCH, 32);
    }

    #[test]
    fn os_open_urls_become_paths() {
        let urls = [
            "file:///Users/me/Book%20One.xlsm",
            "file://localhost/tmp/a.ods",
            "file:///tmp/caf%C3%A9.slk",
            "https://example.com/x.xlsx",
            "file://server/share/x.ods",
            "file:///bad%zz",
        ];
        let paths = file_paths(&urls);
        assert_eq!(
            paths,
            [
                PathBuf::from("/Users/me/Book One.xlsm"),
                PathBuf::from("/tmp/a.ods"),
                PathBuf::from("/tmp/café.slk"),
            ]
        );
    }

    #[test]
    fn macro_and_template_files_save_through_save_as_as_xlsx() {
        for name in [
            "Book.xlsm",
            "Book.XLTM",
            "Book.xltx",
            "Book.xlt",
            "Book.xls",
            "Book.xml",
        ] {
            let p = Path::new("/data/reports").join(name);
            assert!(!writable_in_place(&p), "{name}");
            assert_eq!(format_for(&p), FileFormat::Xlsx, "{name}");
            let plan = save_plan(Some(&p), None);
            assert_eq!(plan.file_name, "Book.xlsx", "{name}");
            assert_eq!(plan.ext, "xlsx");
            assert_eq!(plan.filters[0], ("Excel Workbook", "xlsx"));
            assert_eq!(plan.directory.as_deref(), Some(Path::new("/data/reports")));
            // A name typed with the read-only extension gets the plan's.
            assert!(!has_known_ext(&p));
        }
        for name in ["t.fods", "t.slk", "t.dif", "t.sxc", "t.stc"] {
            let p = Path::new("/d").join(name);
            assert!(!writable_in_place(&p), "{name}");
            assert_eq!(save_plan(Some(&p), None).file_name, "t.ods", "{name}");
        }
        assert!(writable_in_place(Path::new("a.xlsx")));
        assert_eq!(save_plan(None, None).directory, None);
    }

    #[test]
    fn save_as_lists_every_format_current_first() {
        let p = save_plan(Some(Path::new("/tmp/Book.xlsx")), None);
        assert_eq!(p.title, "Save As");
        assert_eq!(p.file_name, "Book.xlsx");
        let exts: Vec<_> = p.filters.iter().map(|(_, e)| *e).collect();
        assert_eq!(exts, ["xlsx", "ods", "csv", "html", "dbf"]);
        let p = save_plan(None, None);
        assert_eq!(p.file_name, "Untitled.ods");
        assert_eq!(p.filters.len(), 5);
        let p = save_plan(Some(Path::new("/tmp/t.dbf")), Some(FileFormat::Html));
        assert_eq!(p.filters, [("HTML", "html")]);
        assert_eq!(p.title, "Export as HTML");
        assert!(has_known_ext(Path::new("a.DBF")));
    }

    #[test]
    fn export_offers_its_format_only() {
        let p = save_plan(Some(Path::new("/tmp/Book.ods")), Some(FileFormat::Xlsx));
        assert_eq!(p.file_name, "Book.xlsx");
        assert_eq!(p.filters, [("Excel Workbook", "xlsx")]);
        assert_eq!(p.ext, "xlsx");
    }

    #[test]
    fn format_follows_the_extension() {
        assert_eq!(format_for(Path::new("a.XLSX")), FileFormat::Xlsx);
        assert_eq!(format_for(Path::new("a.csv")), FileFormat::Csv);
        assert_eq!(format_for(Path::new("a.tsv")), FileFormat::Csv);
        assert_eq!(format_for(Path::new("a.TXT")), FileFormat::Csv);
        assert!(writable_in_place(Path::new("a.tsv")));
        assert!(!writable_in_place(Path::new("a.xls")));
        assert!(!writable_in_place(Path::new("a.sxc")));
        assert!(!writable_in_place(Path::new("a")));
        assert_eq!(format_for(Path::new("a.ods")), FileFormat::Ods);
        assert_eq!(format_for(Path::new("a")), FileFormat::Ods);
        assert!(has_known_ext(Path::new("a.Xlsx")));
        assert!(!has_known_ext(Path::new("a.txt")));
    }
}
