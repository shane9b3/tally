//! Recent files, bundled templates and the `open(path)` allowlist.
//!
//! JS can never make the shell read an arbitrary path: `wb_open` accepts a
//! path only if it is (a) in the recent-files list, (b) was returned by a
//! native dialog this session, or (c) lies inside the bundled `templates/`
//! directory. See docs/security.md (WP7 section).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tally_api::ApiError;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::{lock, AppState};

/// Maximum number of recent files kept.
pub const MAX_RECENT: usize = 10;
const RECENT_FILE: &str = "recent.json";

/// A bundled template, as returned by `list_templates`.
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

/// (id, file, title, description). Phase A ships CSV; ODS once WP5 lands.
const TEMPLATES: &[(&str, &str, &str, &str)] = &[
    (
        "invoices",
        "invoices.csv",
        "Invoice tracker",
        "Customers, amounts and payment status",
    ),
    (
        "budget",
        "budget.csv",
        "Monthly budget",
        "Planned versus actual spending",
    ),
    (
        "stock",
        "stock.csv",
        "Stock count",
        "Items on hand and reorder levels",
    ),
];

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

/// The bundled templates directory (resources in a bundle; the source
/// directory in debug builds if resources were not copied).
pub fn templates_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    if let Ok(dir) = app.path().resolve("templates", BaseDirectory::Resource) {
        if dir.is_dir() {
            return Some(dir);
        }
    }
    if cfg!(debug_assertions) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        if dir.is_dir() {
            return Some(dir);
        }
    }
    None
}

/// Bundled templates whose file exists.
pub fn list_templates<R: Runtime>(app: &AppHandle<R>) -> Vec<TemplateInfo> {
    let Some(dir) = templates_dir(app) else {
        return Vec::new();
    };
    TEMPLATES
        .iter()
        .filter(|(_, file, _, _)| dir.join(file).is_file())
        .map(|(id, _, title, description)| TemplateInfo {
            id: (*id).to_owned(),
            title: (*title).to_owned(),
            description: (*description).to_owned(),
        })
        .collect()
}

/// Absolute path of a bundled template by id.
pub fn template_path<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<PathBuf, ApiError> {
    let (_, file, _, _) = TEMPLATES
        .iter()
        .find(|(tid, _, _, _)| *tid == id)
        .ok_or_else(|| ApiError::not_found(format!("no template \"{id}\"")))?;
    let dir = templates_dir(app).ok_or_else(|| ApiError::not_found("templates are not bundled"))?;
    let path = dir.join(file);
    if path.is_file() {
        Ok(path)
    } else {
        Err(ApiError::not_found(format!(
            "template file {file} is missing"
        )))
    }
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
    let snapshot = match lock(&state.recent, "recent") {
        Ok(mut list) => {
            list.retain(|p| p != &entry);
            list.insert(0, entry);
            list.truncate(MAX_RECENT);
            list.clone()
        }
        Err(e) => {
            eprintln!("tally: {e}");
            return;
        }
    };
    if let Err(e) = store_recent(app, &snapshot) {
        eprintln!("tally: {e}");
    }
}

/// Records a path the user chose in a native dialog.
pub fn allow(state: &AppState, path: &Path) -> Result<(), ApiError> {
    let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    lock(&state.allowed, "allowlist")?.insert(canon);
    Ok(())
}

/// Checks the `open(path)` allowlist and returns the canonical path.
pub fn check_open_allowed<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    path: &Path,
) -> Result<PathBuf, ApiError> {
    let canon = fs::canonicalize(path).map_err(|e| io_err("opening the file", &e))?;
    if let Some(dir) = templates_dir(app).and_then(|d| fs::canonicalize(d).ok()) {
        if canon.starts_with(&dir) {
            return Ok(canon);
        }
    }
    if lock(&state.allowed, "allowlist")?.contains(&canon) {
        return Ok(canon);
    }
    let recent = lock(&state.recent, "recent")?;
    let in_recent = recent
        .iter()
        .any(|p| fs::canonicalize(p).is_ok_and(|rp| rp == canon));
    if in_recent {
        return Ok(canon);
    }
    Err(ApiError::invalid_argument(
        "this file was not chosen in an Open dialog; use File ▸ Open…",
    ))
}
