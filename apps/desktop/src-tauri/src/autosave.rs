//! Autosave and crash recovery (§4 item 43).
//!
//! The webview asks for `autosave_snapshot(doc)` every N minutes while a
//! document is dirty. The snapshot is an ODS in `<app data>/autosave/`
//! named after the document's file (`Book.ods`) or `Untitled`, with a
//! `<name>.json` sidecar (original path, title, time). A clean close removes
//! the snapshot; whatever is left when the app starts is offered through
//! `list_recovery`, and `recover_snapshot` opens it as an untitled, dirty
//! workbook.
//!
//! Snapshots are written with the engine's `save_copy`, which leaves the
//! document's path, title and dirty flag alone; a recovered snapshot is
//! opened with `open_untitled(path, dirty: true)`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tally_api::{ApiError, DocId, FileFormat, WorkbookInfo};
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::{blocking, with_service, CmdResult};
use crate::state::{lock, AppState};

const DIR: &str = "autosave";

/// Sidecar of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Snapshot file.
    pub path: String,
    /// Original document title.
    pub title: String,
    /// Original file, if the document had one.
    pub original: Option<String>,
    /// When it was written (RFC 3339-ish, local time).
    pub saved_at: String,
}

/// What `list_recovery` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryFile {
    pub path: String,
    pub title: String,
    pub saved_at: String,
}

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, ApiError> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::new(ApiError::IO, format!("no app data dir: {e}")))?;
    Ok(base.join(DIR))
}

/// `doc-7-Book.ods` style file name: unique per document, readable.
pub fn snapshot_name(doc: DocId, title: &str) -> String {
    let stem: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .take(80)
        .collect();
    let stem = stem.trim().trim_end_matches(".ods");
    let stem = if stem.is_empty() { "Untitled" } else { stem };
    format!("doc-{}-{stem}.ods", doc.0)
}

fn now_text() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Date arithmetic without chrono: days since the epoch → civil date (Howard Hinnant).
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Writes the snapshot of `doc` (an engine `save_copy` into the autosave dir
/// plus the sidecar).
pub fn write_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    st: &AppState,
    doc: DocId,
) -> CmdResult<Snapshot> {
    let folder = dir(app)?;
    fs::create_dir_all(&folder).map_err(|e| io_err("creating the autosave dir", &e))?;
    let original = lock(&st.paths).get(&doc.0).cloned();
    let title = original.as_ref().and_then(|p| p.file_name()).map_or_else(
        || "Untitled".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let path = folder.join(snapshot_name(doc, &title));
    with_service(st, |s| s.save_copy(doc, &path, FileFormat::Ods))?;
    let snap = Snapshot {
        path: path.display().to_string(),
        title,
        original: original.map(|p| p.display().to_string()),
        saved_at: now_text(),
    };
    let side = serde_json::to_string_pretty(&snap)
        .map_err(|e| ApiError::new(ApiError::INTERNAL, e.to_string()))?;
    fs::write(path.with_extension("json"), side).map_err(|e| io_err("writing the sidecar", &e))?;
    Ok(snap)
}

/// Deletes the snapshot of `doc` (a clean close or a real save).
pub fn remove_snapshot<R: Runtime>(app: &AppHandle<R>, doc: DocId) {
    let Ok(folder) = dir(app) else { return };
    let prefix = format!("doc-{}-", doc.0);
    if let Ok(entries) = fs::read_dir(&folder) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Snapshots on disk (the sidecars found under the autosave dir).
pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<RecoveryFile> {
    let Ok(folder) = dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut out: Vec<RecoveryFile> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let text = fs::read_to_string(e.path()).ok()?;
            let snap: Snapshot = serde_json::from_str(&text).ok()?;
            Path::new(&snap.path).is_file().then_some(RecoveryFile {
                path: snap.path,
                title: snap.title,
                saved_at: snap.saved_at,
            })
        })
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

/// Only files inside the autosave dir may be recovered or discarded.
fn check_inside<R: Runtime>(app: &AppHandle<R>, path: &Path) -> CmdResult<PathBuf> {
    let folder = dir(app)?;
    let canon = fs::canonicalize(path).map_err(|e| io_err("opening the snapshot", &e))?;
    let folder_canon =
        fs::canonicalize(&folder).map_err(|e| io_err("opening the autosave dir", &e))?;
    if !canon.starts_with(&folder_canon) || canon.extension().is_none_or(|x| x != "ods") {
        return Err(ApiError::invalid_argument("not a recovery snapshot"));
    }
    Ok(canon)
}

/// `autosave_snapshot(doc)`.
#[tauri::command]
pub async fn autosave_snapshot(app: AppHandle, doc: DocId) -> CmdResult<()> {
    blocking(app, move |app, st| write_snapshot(app, st, doc).map(|_| ())).await
}

/// `list_recovery()`.
#[tauri::command]
pub async fn list_recovery(app: AppHandle) -> CmdResult<Vec<RecoveryFile>> {
    Ok(list(&app))
}

/// `recover_snapshot(path)`: opens the snapshot as an untitled, dirty workbook and removes it.
#[tauri::command]
pub async fn recover_snapshot(app: AppHandle, path: String) -> CmdResult<WorkbookInfo> {
    let canon = check_inside(&app, Path::new(&path))?;
    blocking(app, move |_, st| {
        let info = with_service(st, |s| s.open_untitled(&canon, true))?;
        lock(&st.paths).remove(&info.doc.0);
        let _ = fs::remove_file(&canon);
        let _ = fs::remove_file(canon.with_extension("json"));
        Ok(info)
    })
    .await
}

/// `discard_recovery(path)`.
#[tauri::command]
pub async fn discard_recovery(app: AppHandle, path: String) -> CmdResult<()> {
    let canon = check_inside(&app, Path::new(&path))?;
    let _ = fs::remove_file(&canon);
    let _ = fs::remove_file(canon.with_extension("json"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_names_are_safe_and_unique_per_doc() {
        assert_eq!(snapshot_name(DocId(3), "Book.ods"), "doc-3-Book.ods");
        assert_eq!(snapshot_name(DocId(4), ""), "doc-4-Untitled.ods");
        assert_eq!(
            snapshot_name(DocId(5), "../../etc/passwd"),
            "doc-5-....etcpasswd.ods"
        );
        assert!(snapshot_name(DocId(6), &"x".repeat(200)).len() < 100);
    }

    #[test]
    fn now_text_is_a_date() {
        let t = now_text();
        assert!(t.len() >= 20, "{t}");
        assert!(t.starts_with("20"));
        assert!(t.ends_with("UTC"));
    }
}
