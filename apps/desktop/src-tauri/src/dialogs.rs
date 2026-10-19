//! Commands that show native file dialogs. The dialog plugin is driven from
//! Rust only (no `dialog:*` permission is granted to JS), and dialogs run on
//! a blocking worker thread so the UI thread is never blocked.

use std::path::{Path, PathBuf};

use tally_api::{ApiError, DocId, FileFormat, WorkbookInfo};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{blocking, open_checked, save_to, CmdResult};
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

/// Open dialog filters: every supported format first, then one per format.
pub const OPEN_FILTERS: &[(&str, &[&str])] = &[
    ("Spreadsheets", &["ods", "xlsx", "csv"]),
    ("OpenDocument Spreadsheet", &["ods"]),
    ("Excel Workbook", &["xlsx"]),
    ("CSV", &["csv"]),
];

/// Save dialog formats as (label, extension), in menu order.
pub const SAVE_FORMATS: &[(&str, &str)] = &[
    ("OpenDocument Spreadsheet", "ods"),
    ("Excel Workbook", "xlsx"),
    ("CSV", "csv"),
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

/// `true` if the path ends in `.ods`, `.xlsx` or `.csv` (any case).
pub fn has_known_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SAVE_FORMATS.iter().any(|(_, x)| x.eq_ignore_ascii_case(e)))
}

/// The Save dialog for `doc`'s current file and the requested format: an
/// export (`format` set) offers that format only; Save As offers all three,
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

/// Shows the Open dialog (ODS/XLSX/CSV), then opens the chosen file.
/// Returns `null` when the user cancels.
#[tauri::command]
pub async fn wb_open_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<WorkbookInfo>> {
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = handle.dialog().file().set_title("Open");
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
    files::allow(&state, &path)?;
    blocking(app, move |app, st| open_checked(app, st, &path).map(Some)).await
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
) -> CmdResult<WorkbookInfo> {
    let current: Option<PathBuf> = lock(&state.paths).get(&doc.0).cloned();
    // Exporting to another format always asks where.
    let format_change = match (&current, format) {
        (Some(p), Some(f)) => format_for(p) != f,
        _ => false,
    };
    let target = match current {
        Some(p) if !save_as && !format_change => p,
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
            path
        }
    };
    let fmt = format.unwrap_or_else(|| format_for(&target));
    blocking(app, move |app, st| {
        let info = save_to(st, doc, &target, fmt)?;
        files::push_recent(app, st, &target);
        Ok(info)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_offers_every_format() {
        assert_eq!(OPEN_FILTERS[0].1, &["ods", "xlsx", "csv"]);
    }

    #[test]
    fn save_as_lists_three_formats_current_first() {
        let p = save_plan(Some(Path::new("/tmp/Book.xlsx")), None);
        assert_eq!(p.title, "Save As");
        assert_eq!(p.file_name, "Book.xlsx");
        let exts: Vec<_> = p.filters.iter().map(|(_, e)| *e).collect();
        assert_eq!(exts, ["xlsx", "ods", "csv"]);
        let p = save_plan(None, None);
        assert_eq!(p.file_name, "Untitled.ods");
        assert_eq!(p.filters.len(), 3);
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
        assert_eq!(format_for(Path::new("a.ods")), FileFormat::Ods);
        assert_eq!(format_for(Path::new("a")), FileFormat::Ods);
        assert!(has_known_ext(Path::new("a.Xlsx")));
        assert!(!has_known_ext(Path::new("a.txt")));
    }
}
