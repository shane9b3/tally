//! App preferences (Tools ▸ Options): `prefs.json` in the app config dir.
//!
//! The webview owns the schema (`packages/app/src/options/prefs.ts`); the
//! shell stores the JSON text as given, after checking that it is a JSON
//! object of at most [`MAX_PREFS_BYTES`] bytes, and reads the keys it needs
//! itself: `uiLanguage` (the native menu), `openDocumentsIn` (`"tabs"`, the
//! default, or `"windows"`: where new documents open, `windows.rs`) and
//! `autosaveMinutes` (documentation only; the timer runs in the webview).
//!
//! Every document runs its own webview with its own copy of the prefs:
//! after a change in one (Tools ▸ Options, View ▸ Theme) it calls
//! `broadcast_prefs(theme)` and the other document webviews (in every
//! window, shown tab or not) get `prefs-changed { theme }`, re-read
//! `get_prefs` and apply the theme.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;
use tally_api::ApiError;
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};

use crate::registry::{is_doc_label, OpenMode};

/// Event sent to the other document windows after a prefs or theme change.
pub const PREFS_CHANGED: &str = "prefs-changed";

/// Payload of [`PREFS_CHANGED`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrefsChangedEvent {
    /// The theme preference (`system`, `light`, `dark`), if it changed.
    pub theme: Option<String>,
}

/// Largest prefs document accepted (bytes).
pub const MAX_PREFS_BYTES: usize = 1024 * 1024;
const PREFS_FILE: &str = "prefs.json";

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

fn prefs_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(PREFS_FILE))
}

/// The prefs key of the user's own AutoFormats (a list the webview validates).
pub const AUTO_FORMATS_KEY: &str = "autoFormats";
/// Most user AutoFormats stored.
pub const MAX_AUTO_FORMATS: usize = 100;

/// `autoFormats`, when present, must be a list of at most [`MAX_AUTO_FORMATS`] objects.
fn check_auto_formats(v: Option<&Value>) -> Result<(), ApiError> {
    match v {
        None => Ok(()),
        Some(Value::Array(list))
            if list.len() <= MAX_AUTO_FORMATS && list.iter().all(Value::is_object) =>
        {
            Ok(())
        }
        Some(_) => Err(ApiError::invalid_argument(format!(
            "autoFormats must be a list of at most {MAX_AUTO_FORMATS} objects"
        ))),
    }
}

/// Validates a prefs document: a JSON object, at most [`MAX_PREFS_BYTES`].
pub fn check(json: &str) -> Result<(), ApiError> {
    if json.len() > MAX_PREFS_BYTES {
        return Err(ApiError::invalid_argument(format!(
            "the preferences are too large ({} bytes; the limit is {MAX_PREFS_BYTES})",
            json.len()
        )));
    }
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(o)) => check_auto_formats(o.get(AUTO_FORMATS_KEY)),
        Ok(_) => Err(ApiError::invalid_argument(
            "the preferences must be a JSON object",
        )),
        Err(e) => Err(ApiError::invalid_argument(format!(
            "the preferences are not valid JSON: {e}"
        ))),
    }
}

/// The stored prefs text, or `{}` when there is none (or it is unreadable).
pub fn load<R: Runtime>(app: &AppHandle<R>) -> String {
    let Some(path) = prefs_path(app) else {
        return "{}".to_owned();
    };
    match fs::read_to_string(path) {
        Ok(text) => sanitize(&text).unwrap_or_else(|| "{}".to_owned()),
        _ => "{}".to_owned(),
    }
}

/// A stored prefs text made loadable: a bad `autoFormats` (not a list, or
/// entries that are not objects, or more than [`MAX_AUTO_FORMATS`]) is
/// repaired and the other prefs are kept. `None` when the text is not a
/// JSON object or is over the size limit.
pub fn sanitize(json: &str) -> Option<String> {
    if json.len() > MAX_PREFS_BYTES {
        return None;
    }
    let mut v: Value = serde_json::from_str(json).ok()?;
    let obj = v.as_object_mut()?;
    if check_auto_formats(obj.get(AUTO_FORMATS_KEY)).is_ok() {
        return Some(json.to_owned());
    }
    let kept: Vec<Value> = match obj.get(AUTO_FORMATS_KEY) {
        Some(Value::Array(list)) => list
            .iter()
            .filter(|e| e.is_object())
            .take(MAX_AUTO_FORMATS)
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    obj.insert(AUTO_FORMATS_KEY.to_owned(), Value::Array(kept));
    serde_json::to_string(&v).ok()
}

/// Writes the prefs text (validated first).
pub fn store<R: Runtime>(app: &AppHandle<R>, json: &str) -> Result<(), ApiError> {
    check(json)?;
    let path = prefs_path(app).ok_or_else(|| ApiError::new(ApiError::IO, "no config dir"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| io_err("creating the config dir", &e))?;
    }
    fs::write(&path, json).map_err(|e| io_err("writing the preferences", &e))
}

/// A string key of a prefs document (`None` when absent or not a string).
pub fn string_key(json: &str, key: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.get(key)?.as_str().map(str::to_owned)
}

/// The UI language stored in the prefs (`"en"` by default).
pub fn ui_language(json: &str) -> String {
    string_key(json, "uiLanguage").unwrap_or_else(|| "en".to_owned())
}

/// The prefs key of the "Open documents in" setting.
pub const OPEN_MODE_KEY: &str = "openDocumentsIn";

/// The "Open documents in" setting: tabs unless the prefs say `"windows"`.
pub fn open_mode(json: &str) -> OpenMode {
    match string_key(json, OPEN_MODE_KEY).as_deref() {
        Some("windows") => OpenMode::Windows,
        _ => OpenMode::Tabs,
    }
}

/// Replaces (or adds) a string key and returns the new document.
pub fn with_string_key(json: &str, key: &str, value: &str) -> Result<String, ApiError> {
    let mut v: Value =
        serde_json::from_str(json).unwrap_or_else(|_| Value::Object(Default::default()));
    if !v.is_object() {
        v = Value::Object(Default::default());
    }
    if let Some(obj) = v.as_object_mut() {
        obj.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    serde_json::to_string(&v).map_err(|e| ApiError::new(ApiError::INTERNAL, e.to_string()))
}

/// `get_prefs`: the prefs.json text (`{}` when none).
#[tauri::command]
pub async fn get_prefs(app: AppHandle) -> Result<String, ApiError> {
    Ok(load(&app))
}

/// `set_prefs`: stores the prefs.json text (a JSON object ≤ 1 MiB).
#[tauri::command]
pub async fn set_prefs(app: AppHandle, json: String) -> Result<(), ApiError> {
    store(&app, &json)
}

/// A theme preference the UI knows (`None`: unchanged).
pub fn check_theme(theme: Option<&str>) -> Result<(), ApiError> {
    match theme {
        None | Some("system" | "light" | "dark") => Ok(()),
        Some(_) => Err(ApiError::invalid_argument("unknown theme")),
    }
}

/// Pure: the webviews a change in `from` is sent to (the other documents).
pub fn broadcast_targets<'a>(labels: impl IntoIterator<Item = &'a str>, from: &str) -> Vec<String> {
    labels
        .into_iter()
        .filter(|l| is_doc_label(l) && *l != from)
        .map(str::to_owned)
        .collect()
}

/// `broadcast_prefs(theme)`: the calling document changed the prefs
/// (already stored with `set_prefs`) or the theme; the other document
/// webviews get `prefs-changed { theme }`.
#[tauri::command]
pub async fn broadcast_prefs(
    app: AppHandle,
    webview: Webview,
    theme: Option<String>,
) -> Result<(), ApiError> {
    check_theme(theme.as_deref())?;
    let all = app.webviews();
    let targets = broadcast_targets(all.keys().map(String::as_str), webview.label());
    for label in targets {
        let event = PrefsChangedEvent {
            theme: theme.clone(),
        };
        if let Err(e) = app.emit_to(label.as_str(), PREFS_CHANGED, event) {
            eprintln!("tally: emitting {PREFS_CHANGED} to {label}: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_shape_and_size() {
        assert!(check("{}").is_ok());
        assert!(check(r#"{"uiLanguage":"de","autoInput":true}"#).is_ok());
        assert_eq!(check("[]").unwrap_err().code, ApiError::INVALID_ARGUMENT);
        assert_eq!(
            check("not json").unwrap_err().code,
            ApiError::INVALID_ARGUMENT
        );
        let big = format!("{{\"k\":\"{}\"}}", "x".repeat(MAX_PREFS_BYTES));
        assert_eq!(check(&big).unwrap_err().code, ApiError::INVALID_ARGUMENT);
    }

    #[test]
    fn user_autoformats_are_a_capped_list_of_objects() {
        assert!(check(r#"{"autoFormats":[]}"#).is_ok());
        assert!(check(r#"{"autoFormats":[{"name":"Mine","cells":[]}]}"#).is_ok());
        assert!(check(r#"{"autoFormats":"x"}"#).is_err());
        assert!(check(r#"{"autoFormats":[1]}"#).is_err());
        let many = format!("{{\"autoFormats\":[{}]}}", vec!["{}"; 101].join(","));
        assert!(check(&many).is_err());
        let ok = format!("{{\"autoFormats\":[{}]}}", vec!["{}"; 100].join(","));
        assert!(check(&ok).is_ok());
    }

    #[test]
    fn a_bad_autoformats_entry_keeps_the_other_prefs() {
        let bad =
            r#"{"uiLanguage":"de","theme":"dark","autoFormats":[{"name":"a"},7,"x",{"name":"b"}]}"#;
        assert!(check(bad).is_err());
        let fixed = sanitize(bad).unwrap();
        assert!(check(&fixed).is_ok());
        assert_eq!(ui_language(&fixed), "de");
        assert_eq!(string_key(&fixed, "theme").as_deref(), Some("dark"));
        let v: Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["autoFormats"].as_array().unwrap().len(), 2);
        let wrong_type = sanitize(r#"{"uiLanguage":"fr","autoFormats":"x"}"#).unwrap();
        assert_eq!(ui_language(&wrong_type), "fr");
        assert!(check(&wrong_type).is_ok());
        let many = format!(
            "{{\"uiLanguage\":\"es\",\"autoFormats\":[{}]}}",
            vec!["{}"; 150].join(",")
        );
        let capped = sanitize(&many).unwrap();
        assert_eq!(ui_language(&capped), "es");
        let v: Value = serde_json::from_str(&capped).unwrap();
        assert_eq!(v["autoFormats"].as_array().unwrap().len(), MAX_AUTO_FORMATS);
        // a good document is returned as it is; garbage is not repaired
        let good = r#"{"uiLanguage":"de"}"#;
        assert_eq!(sanitize(good).as_deref(), Some(good));
        assert_eq!(sanitize("[]"), None);
        assert_eq!(sanitize("junk"), None);
    }

    #[test]
    fn reads_and_rewrites_keys() {
        assert_eq!(ui_language("{}"), "en");
        assert_eq!(ui_language(r#"{"uiLanguage":"fr"}"#), "fr");
        assert_eq!(ui_language(r#"{"uiLanguage":7}"#), "en");
        let next = with_string_key(r#"{"autoInput":false}"#, "uiLanguage", "es").unwrap();
        assert_eq!(ui_language(&next), "es");
        assert_eq!(string_key(&next, "autoInput"), None);
        assert!(next.contains("\"autoInput\":false"));
        let fresh = with_string_key("garbage", "uiLanguage", "de").unwrap();
        assert_eq!(fresh, r#"{"uiLanguage":"de"}"#);
    }

    #[test]
    fn documents_open_in_tabs_unless_the_prefs_say_windows() {
        assert_eq!(open_mode("{}"), OpenMode::Tabs);
        assert_eq!(open_mode(r#"{"openDocumentsIn":"tabs"}"#), OpenMode::Tabs);
        assert_eq!(
            open_mode(r#"{"openDocumentsIn":"windows"}"#),
            OpenMode::Windows
        );
        assert_eq!(open_mode(r#"{"openDocumentsIn":"popups"}"#), OpenMode::Tabs);
        assert_eq!(open_mode(r#"{"openDocumentsIn":1}"#), OpenMode::Tabs);
        assert_eq!(open_mode("garbage"), OpenMode::Tabs);
    }

    #[test]
    fn prefs_changes_reach_the_other_document_windows() {
        let labels = ["doc-1", "doc-2", "tally-print-4", "doc-7", "win-1"];
        assert_eq!(broadcast_targets(labels, "doc-2"), ["doc-1", "doc-7"]);
        assert!(broadcast_targets(["doc-1"], "doc-1").is_empty());
        assert!(check_theme(None).is_ok());
        assert!(check_theme(Some("dark")).is_ok());
        assert_eq!(
            check_theme(Some("javascript:")).unwrap_err().code,
            ApiError::INVALID_ARGUMENT
        );
    }
}
