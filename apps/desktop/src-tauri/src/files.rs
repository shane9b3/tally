//! Recent files, the template list and the `open(path)` allowlist.
//!
//! JS can never make the shell read an arbitrary path: `wb_open` accepts a
//! path only if it is (a) in the recent-files list or (b) was returned by a
//! native dialog this session. Templates are built in code by the engine
//! (`new_workbook(Some(id))`), so no template files are read. See
//! docs/security.md (WP7 section).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tally_api::ApiError;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::{lock, AppState};

/// Maximum number of recent files kept.
pub const MAX_RECENT: usize = 10;
const RECENT_FILE: &str = "recent.json";

/// A template, as returned by `list_templates`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// Stable id passed to `wb_new(template)`.
    pub id: String,
    /// Display title.
    pub title: String,
    /// One-line description.
    pub description: String,
}

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

/// The engine's templates (ids passed to `wb_new`, with display names).
pub fn list_templates() -> Vec<TemplateInfo> {
    tally_engine::service::templates::CATALOG
        .iter()
        .map(|t| TemplateInfo {
            id: t.id.to_owned(),
            title: t.title.to_owned(),
            description: t.description.to_owned(),
        })
        .collect()
}

fn recent_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join(RECENT_FILE))
}

/// Loads the persisted recent list (missing or corrupt file → empty).
pub fn load_recent<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let Some(path) = recent_path(app) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut list: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    list.truncate(MAX_RECENT);
    list
}

fn store_recent<R: Runtime>(app: &AppHandle<R>, list: &[String]) -> Result<(), ApiError> {
    let path = recent_path(app).ok_or_else(|| ApiError::new(ApiError::IO, "no config dir"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_err("creating the config dir", &e))?;
    }
    let json = serde_json::to_string_pretty(list)
        .map_err(|e| ApiError::new(ApiError::INTERNAL, e.to_string()))?;
    fs::write(&path, json).map_err(|e| io_err("writing recent files", &e))
}

/// Moves `path` to the front of the recent list and persists it.
/// Persistence errors are logged, never surfaced: they must not fail a save.
pub fn push_recent<R: Runtime>(app: &AppHandle<R>, state: &AppState, path: &Path) {
    let entry = path.to_string_lossy().into_owned();
    let snapshot = {
        let mut list = lock(&state.recent);
        list.retain(|p| p != &entry);
        list.insert(0, entry);
        list.truncate(MAX_RECENT);
        list.clone()
    };
    if let Err(e) = store_recent(app, &snapshot) {
        eprintln!("tally: {e}");
    }
}

/// Records a path the user chose in a native dialog: both as given and in
/// canonical form, since JS may pass back either (dialog result, or the
/// canonical path from `WorkbookInfo.path` / the recent list).
pub fn allow(state: &AppState, path: &Path) -> Result<(), ApiError> {
    let mut allowed = lock(&state.allowed);
    allowed.insert(path.to_path_buf());
    if let Ok(canon) = fs::canonicalize(path) {
        allowed.insert(canon);
    }
    Ok(())
}

/// Checks the `open(path)` allowlist and returns the canonical path.
///
/// The path is compared *as given* against the allowlist and the recent
/// list before the file system is touched, so a rejected path gets the same
/// error whether or not it exists (no file-existence oracle for the
/// webview). Only an allowed path is canonicalized; the canonical form must
/// be allowed too, so a symlink swapped in afterwards cannot redirect it.
pub fn check_open_allowed(state: &AppState, path: &Path) -> Result<PathBuf, ApiError> {
    let rejected = || {
        ApiError::invalid_argument("this file was not chosen in an Open dialog; use File ▸ Open…")
    };
    let as_given = lock(&state.allowed).contains(path)
        || lock(&state.recent).iter().any(|p| Path::new(p) == path);
    if !as_given {
        return Err(rejected());
    }
    let canon = fs::canonicalize(path).map_err(|e| io_err("opening the file", &e))?;
    let canon_ok = lock(&state.allowed).contains(&canon)
        || lock(&state.recent)
            .iter()
            .any(|p| Path::new(p) == canon || fs::canonicalize(p).is_ok_and(|rp| rp == canon));
    if canon_ok {
        Ok(canon)
    } else {
        Err(rejected())
    }
}
