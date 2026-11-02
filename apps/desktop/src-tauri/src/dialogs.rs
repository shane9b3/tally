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

/// Format from a file extension (unknown → ODS).
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
        Some("xlsx") => FileFormat::Xlsx,
        Some("html") | Some("htm") => FileFormat::Html,
        Some("dbf") => FileFormat::Dbf,
        _ => FileFormat::Ods,
    }
}

/// `true` if Save can write `path` in its own format; a file Tally only
/// reads (XLS, SXC, …) is saved through the Save As dialog instead of
/// being overwritten with another format.
pub fn writable_in_place(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ods" | "ots" | "xlsx" | "csv" | "tsv" | "txt" | "tab" | "html" | "htm" | "dbf")
    )
}

/// Open dialog filters: every supported format first, then one per format
/// (the engine codec reads all of them; docs/file-formats.md).
pub const OPEN_FILTERS: &[(&str, &[&str])] = &[
    (
        "Spreadsheets",
        &[
            "ods", "ots", "xlsx", "xls", "sxc", "csv", "tsv", "html", "htm", "dbf",
        ],
    ),
    ("OpenDocument Spreadsheet", &["ods", "ots"]),
    ("Excel Workbook", &["xlsx"]),
    ("Excel 97–2003 Workbook", &["xls"]),
    ("OpenOffice.org 1.x Spreadsheet", &["sxc"]),
    ("CSV", &["csv", "tsv"]),
    ("HTML", &["html", "htm"]),
    ("dBase", &["dbf"]),
];

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
        let dialog = OPEN_FILTERS
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

/// File ▸ Open… with no document window (macOS): the dialog, then a window
/// for the file. Errors are logged (there is no webview to show them). A
/// file someone else has locked opens read-only: with no window to ask in,
/// this is AOO's answer when there is no interaction handler
/// (`SfxMedium::ShowLockedDocumentDialog`).
pub fn spawn_open<R: Runtime>(app: &AppHandle<R>, read_only: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let opened = match pick_and_open(app.clone(), read_only).await {
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
        };
        match opened {
            Ok(Some(info)) => crate::windows::spawn_doc_window(&app, Some(info)),
            Ok(None) => {}
            Err(e) => eprintln!("tally: open: {}", e.message),
        }
    });
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
            let picked = tauri::async_runtime::spawn_blocking(move || {
                let dialog = handle
                    .dialog()
                    .file()
                    .set_title(title)
                    .set_file_name(file_name);
                filters
                    .iter()
                    .fold(dialog, |d, (label, ext)| d.add_filter(*label, &[*ext]))
                    .blocking_save_file()
            })
            .await
            .map_err(join_err)?;
            let file = picked.ok_or_else(cancelled)?;
            let mut path = file
                .into_path()
                .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
            if !has_known_ext(&path) {
                path.set_extension(plan.ext);
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
            "ods", "ots", "xlsx", "xls", "sxc", "csv", "html", "htm", "dbf",
        ] {
            assert!(OPEN_FILTERS[0].1.contains(&ext), "{ext}");
        }
        // Every per-format filter is part of the combined one.
        for (_, exts) in &OPEN_FILTERS[1..] {
            assert!(exts.iter().all(|e| OPEN_FILTERS[0].1.contains(e)));
        }
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
