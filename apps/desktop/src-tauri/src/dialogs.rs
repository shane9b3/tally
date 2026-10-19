//! Commands that show native file dialogs. The dialog plugin is driven from
//! Rust only (no `dialog:*` permission is granted to JS), and dialogs run on
//! a blocking worker thread so the UI thread is never blocked.

use std::path::{Path, PathBuf};

use tally_api::{ApiError, DocId, FileFormat, WorkbookInfo};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{open_checked, save_to, CmdResult};
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
        Some("csv") => FileFormat::Csv,
        Some("xlsx") => FileFormat::Xlsx,
        _ => FileFormat::Ods,
    }
}

/// Shows the Open dialog (ODS/CSV), then opens the chosen file.
/// Returns `null` when the user cancels.
#[tauri::command]
pub async fn wb_open_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<WorkbookInfo>> {
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Open")
            .add_filter("Spreadsheets", &["ods", "csv"])
            .blocking_pick_file()
    })
    .await
    .map_err(join_err)?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
    files::allow(&state, &path)?;
    open_checked(&app, &state, &path).map(Some)
}

/// Saves `doc`. Shows a Save dialog when `save_as` is set or the document
/// has no file yet; `format` defaults to the file's extension (else ODS).
#[tauri::command]
pub async fn wb_save(
    app: AppHandle,
    state: State<'_, AppState>,
    doc: DocId,
    save_as: bool,
    format: Option<FileFormat>,
) -> CmdResult<WorkbookInfo> {
    let current: Option<PathBuf> = lock(&state.paths, "paths")?.get(&doc.0).cloned();
    // Exporting to another format always asks where.
    let format_change = match (&current, format) {
        (Some(p), Some(f)) => format_for(p) != f,
        _ => false,
    };
    let target = match current {
        Some(p) if !save_as && !format_change => p,
        current => {
            let fmt = format.unwrap_or(FileFormat::Ods);
            let ext = ext_of(fmt);
            let stem = current
                .as_deref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_owned();
            let handle = app.clone();
            let picked = tauri::async_runtime::spawn_blocking(move || {
                let label = if ext == "csv" {
                    "CSV"
                } else {
                    "OpenDocument Spreadsheet"
                };
                handle
                    .dialog()
                    .file()
                    .set_title(if ext == "csv" {
                        "Export as CSV"
                    } else {
                        "Save As"
                    })
                    .set_file_name(format!("{stem}.{ext}"))
                    .add_filter(label, &[ext])
                    .blocking_save_file()
            })
            .await
            .map_err(join_err)?;
            let file = picked.ok_or_else(cancelled)?;
            let mut path = file
                .into_path()
                .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
            if path.extension().is_none() {
                path.set_extension(ext);
            }
            files::allow(&state, &path)?;
            path
        }
    };
    let fmt = format.unwrap_or_else(|| format_for(&target));
    let info = save_to(&state, doc, &target, fmt)?;
    files::push_recent(&app, &state, &target);
    Ok(info)
}
