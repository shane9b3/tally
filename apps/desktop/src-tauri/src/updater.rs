//! Auto-update from GitHub Releases (docs/release.md, docs/security.md
//! "Auto-update"): the app's only network access.
//!
//! `tauri-plugin-updater` does the work, driven from Rust only (no
//! `updater:*` permission is granted to any webview, the CSP stays
//! IPC-only): it fetches the static manifest
//! `https://github.com/shane9b3/tally/releases/latest/download/latest.json`
//! (`plugins.updater.endpoints` in tauri.conf.json), offers a release only
//! when its version is greater than ours (no downgrades), downloads the
//! package for this platform over HTTPS and checks its minisign signature
//! against the public key compiled in from tauri.conf.json, including the
//! version the signature was made for (`requireSignedVersion`).
//!
//! The flow, all user-driven except the check:
//! - Automatic check ([`start`]): [`FIRST_CHECK_DELAY`] after launch, then
//!   every [`CHECK_INTERVAL`] while the app runs, only while the
//!   `checkForUpdates` pref is on (read each time; default on). A newer
//!   version that is not skipped is announced once, to the front document
//!   (`update-available`), never once per window. Failures are logged only.
//! - `update_check`: Help ▸ Check for Updates… (app menu on macOS) and
//!   Options ▸ Online Update ▸ Check Now. Ignores the skipped version;
//!   failures go back to the UI.
//! - `update_download`: Install and Restart. Downloads and verifies, with
//!   `update-progress` events to the asking webview; the verified package
//!   is kept in memory ("staged") for this session.
//! - `update_restart`: runs the unsaved-changes guard over every open
//!   document like Quit (registry `Goal::Restart`); once all answered,
//!   [`install_and_restart`] installs and relaunches. Cancel there keeps
//!   everything open and the update staged (`update-restart-aborted`).
//! - `update_skip`: Skip This Version (`updater.json` in the config dir);
//!   a newer version prompts again.
//!
//! At most one check runs at a time; requests time out
//! ([`CHECK_TIMEOUT`], [`DOWNLOAD_TIMEOUT`]). Debug builds and Linux
//! installs that are not an AppImage never check (`update_status` says
//! why); the browser build uses the MockClient instead.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tally_api::ApiError;
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::registry::Windows;
use crate::state::lock;

/// Event to the front document: an automatic check found an update ([`UpdateInfo`]).
pub const UPDATE_AVAILABLE: &str = "update-available";
/// Event to the downloading document: [`UpdateProgress`].
pub const UPDATE_PROGRESS: &str = "update-progress";
/// Event: the restart to update ended without restarting ([`RestartAborted`]).
pub const UPDATE_RESTART_ABORTED: &str = "update-restart-aborted";
/// The prefs key of Options ▸ Online Update ▸ "Check for updates automatically".
pub const AUTO_CHECK_KEY: &str = "checkForUpdates";
/// First automatic check after launch.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);
/// Automatic checks while the app runs.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Whole-request timeout of the manifest request.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// Whole-request timeout of the package download.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Longest release notes shown (characters; the rest is cut).
pub const MAX_NOTES_CHARS: usize = 20_000;
/// Longest version string accepted from the manifest or the UI.
pub const MAX_VERSION_LEN: usize = 64;
/// Error code: updates are not available in this build or installation.
pub const UNAVAILABLE: &str = ApiError::UNSUPPORTED;
const SKIP_FILE: &str = "updater.json";
const SKIP_KEY: &str = "skippedVersion";

/// An update offered to the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// The new version (`0.2.0`).
    pub version: String,
    /// The running version.
    pub current_version: String,
    /// Release notes as plain text (untrusted: shown as text, never as HTML).
    pub notes: String,
    /// Release date (`YYYY-MM-DD`), when the manifest has one.
    pub date: Option<String>,
    /// Already downloaded and verified this session: Restart installs it.
    pub downloaded: bool,
}

/// Download progress ([`UPDATE_PROGRESS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    /// Bytes received so far.
    pub received: u64,
    /// Total bytes, when the server says.
    pub total: Option<u64>,
}

/// Why a restart to update ended without restarting ([`UPDATE_RESTART_ABORTED`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartAborted {
    /// Cancel in an unsaved-changes prompt (otherwise the install failed).
    pub cancelled: bool,
    /// The install error.
    pub message: Option<String>,
}

/// `update_status`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The running version.
    pub current_version: String,
    /// Whether this build checks for updates at all.
    pub supported: bool,
    /// Why not (`supported: false`).
    pub reason: Option<String>,
    /// An update downloaded this session, waiting for a restart.
    pub staged: Option<UpdateInfo>,
}

struct Staged {
    update: Update,
    bytes: Vec<u8>,
}

/// The updater's session state (managed).
#[derive(Default)]
pub struct UpdaterState {
    /// Held for the duration of a check: at most one in flight.
    checking: tauri::async_runtime::Mutex<()>,
    /// The last update a check offered (what Install downloads).
    found: Mutex<Option<Update>>,
    /// Downloaded and verified, waiting for the restart.
    staged: Mutex<Option<Staged>>,
    downloading: AtomicBool,
}

// -- pure logic (unit-tested) -------------------------------------------------

/// "Check for updates automatically": on unless the prefs say `false`.
pub fn auto_check_enabled(prefs_json: &str) -> bool {
    serde_json::from_str::<Value>(prefs_json)
        .ok()
        .and_then(|v| v.get(AUTO_CHECK_KEY)?.as_bool())
        .unwrap_or(true)
}

/// A version string from the manifest or the UI (`v` prefix allowed).
pub fn parse_version(v: &str) -> Option<semver::Version> {
    if v.len() > MAX_VERSION_LEN {
        return None;
    }
    semver::Version::parse(v.trim().trim_start_matches('v')).ok()
}

/// Whether `candidate` is newer than `current` (both semver; anything
/// unparsable is not newer).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// Whether an update to `version` is announced. A manual check always
/// shows it; an automatic one stays silent for the skipped version and
/// anything older (a newer one prompts again).
pub fn should_prompt(version: &str, skipped: Option<&str>, manual: bool) -> bool {
    if manual {
        return true;
    }
    match (parse_version(version), skipped.and_then(parse_version)) {
        (Some(v), Some(s)) => v > s,
        _ => true,
    }
}

/// The skipped version stored in `updater.json` (`None` when absent or bad).
pub fn skipped_version(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    let s = v.get(SKIP_KEY)?.as_str()?;
    parse_version(s).map(|_| s.to_owned())
}

/// `updater.json` remembering `version` as skipped.
pub fn skip_json(version: &str) -> String {
    serde_json::json!({ SKIP_KEY: version }).to_string()
}

/// Release notes made safe to show as plain text: control characters
/// other than newline and tab dropped, CRLF folded, at most
/// [`MAX_NOTES_CHARS`] characters (the UI never renders them as HTML).
pub fn clean_notes(raw: Option<&str>) -> String {
    let text = raw.unwrap_or_default().replace("\r\n", "\n");
    let mut out: String = text
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .filter(|c| !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .take(MAX_NOTES_CHARS + 1)
        .collect();
    if out.chars().count() > MAX_NOTES_CHARS {
        out = out.chars().take(MAX_NOTES_CHARS).collect();
        out.push('…');
    }
    out.trim().to_owned()
}

/// Only HTTPS downloads (the manifest itself is fetched over HTTPS; the
/// plugin would only warn about a plain-HTTP package URL).
pub fn require_https(url: &tauri::Url) -> Result<(), ApiError> {
    if url.scheme() == "https" {
        Ok(())
    } else {
        Err(ApiError::invalid_argument(
            "the update package is not served over HTTPS",
        ))
    }
}

/// Throttles progress events: one per whole percent, or per 512 KiB when
/// the size is unknown, and always the last chunk.
#[derive(Debug, Default)]
pub struct ProgressMeter {
    received: u64,
    last_step: Option<u64>,
}

impl ProgressMeter {
    /// Adds a chunk; returns the progress to report, if any.
    pub fn add(&mut self, chunk: usize, total: Option<u64>) -> Option<UpdateProgress> {
        self.received = self.received.saturating_add(chunk as u64);
        let step = match total {
            Some(t) if t > 0 => self.received.saturating_mul(100) / t,
            _ => self.received / (512 * 1024),
        };
        let done = total.is_some_and(|t| self.received >= t);
        if self.last_step == Some(step) && !done {
            return None;
        }
        self.last_step = Some(step);
        Some(UpdateProgress {
            received: self.received,
            total,
        })
    }
}

/// Why this build does not check for updates (`None`: it does).
pub fn unsupported_reason() -> Option<&'static str> {
    if cfg!(debug_assertions) {
        return Some("Updates are checked only in release builds of Tally.");
    }
    #[cfg(target_os = "linux")]
    if tauri::utils::platform::bundle_type() != Some(tauri::utils::config::BundleType::AppImage) {
        return Some("This copy of Tally was not installed as an AppImage: update it with your package manager or download the new version from GitHub.");
    }
    None
}

fn unsupported() -> Result<(), ApiError> {
    match unsupported_reason() {
        Some(why) => Err(ApiError::new(UNAVAILABLE, why)),
        None => Ok(()),
    }
}

fn info_of(update: &Update, downloaded: bool) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: clean_notes(update.body.as_deref()),
        date: update
            .date
            .map(|d| format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())),
        downloaded,
    }
}

fn plugin_err(what: &str, e: &tauri_plugin_updater::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

// -- skipped version --------------------------------------------------------------

fn skip_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(SKIP_FILE))
}

fn load_skipped<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let text = fs::read_to_string(skip_path(app)?).ok()?;
    if text.len() > 4096 {
        return None;
    }
    skipped_version(&text)
}

// -- checking -----------------------------------------------------------------------

/// The updater for one check: the configured endpoint and key, a request
/// timeout, and (Windows, where installing ends the process) our exit
/// clean-up before the installer takes over.
fn build_updater<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri_plugin_updater::Updater, ApiError> {
    let handle = app.clone();
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .on_before_exit(move || crate::closing::exit_cleanup(&handle))
        .build()
        .map_err(|e| plugin_err("the updater could not start", &e))
}

/// One check (callers hold `checking`). `Ok(None)`: up to date.
async fn check_once<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, ApiError> {
    let updater = build_updater(app)?;
    let found = updater
        .check()
        .await
        .map_err(|e| plugin_err("checking for updates failed", &e))?;
    // The plugin only offers newer versions; the manifest's version must
    // also be one we can compare (and skip).
    Ok(found.filter(|u| is_newer(&u.version, &u.current_version)))
}

fn staged_info(st: &UpdaterState) -> Option<UpdateInfo> {
    lock(&st.staged).as_ref().map(|s| info_of(&s.update, true))
}

/// The automatic check (silent on failure). Returns what was announced.
async fn auto_check<R: Runtime>(app: &AppHandle<R>) -> Option<UpdateInfo> {
    if !auto_check_enabled(&crate::prefs::load(app)) {
        return None;
    }
    let st = app.state::<UpdaterState>();
    // A check already running (a manual one) or an update already
    // downloaded: nothing to announce.
    let _guard = st.checking.try_lock().ok()?;
    if lock(&st.staged).is_some() {
        return None;
    }
    let update = match check_once(app).await {
        Ok(Some(u)) => u,
        Ok(None) => return None,
        Err(e) => {
            eprintln!("tally: automatic update check: {}", e.message);
            return None;
        }
    };
    if !should_prompt(&update.version, load_skipped(app).as_deref(), false) {
        return None;
    }
    let info = info_of(&update, false);
    *lock(&st.found) = Some(update);
    // Once, to the front document (the window the user is looking at).
    let front = app.state::<Windows>().with(|r| r.front());
    match front {
        Some(label) => {
            if let Err(e) = app.emit_to(label.as_str(), UPDATE_AVAILABLE, info.clone()) {
                eprintln!("tally: emitting {UPDATE_AVAILABLE}: {e}");
            }
        }
        None => eprintln!("tally: update {} found, no window to show it", info.version),
    }
    Some(info)
}

/// Starts the automatic checks (release builds; nothing in debug builds).
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    if let Some(why) = unsupported_reason() {
        if cfg!(debug_assertions) {
            eprintln!("tally(debug): no update checks: {why}");
        }
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("tally-update-check".to_owned())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                tauri::async_runtime::block_on(auto_check(&app));
                std::thread::sleep(CHECK_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        eprintln!("tally: update checks not started: {e}");
    }
}

// -- restart --------------------------------------------------------------------------

fn aborted<R: Runtime>(app: &AppHandle<R>, label: Option<String>, event: RestartAborted) {
    let target = label.or_else(|| app.state::<Windows>().with(|r| r.front()));
    if let Some(label) = target {
        if let Err(e) = app.emit_to(label.as_str(), UPDATE_RESTART_ABORTED, event) {
            eprintln!("tally: emitting {UPDATE_RESTART_ABORTED}: {e}");
        }
    }
}

/// Cancel in an unsaved-changes prompt ended the restart (closing.rs):
/// nothing closed, the update stays downloaded; `label` (the tab that was
/// asking) says so.
pub fn restart_cancelled<R: Runtime>(app: &AppHandle<R>, label: &str) {
    aborted(
        app,
        Some(label.to_owned()),
        RestartAborted {
            cancelled: true,
            message: None,
        },
    );
}

/// Every edited document answered the restart's question (closing.rs
/// `Goal::Restart`): installs the staged update and relaunches. Runs on a
/// blocking worker (the macOS install may wait for the main thread to ask
/// for an administrator password). On Windows the plugin starts the
/// installer and ends the process itself (after `exit_cleanup`); the
/// installer relaunches Tally. A failed install keeps every document open
/// (their answers no longer count) and says why.
pub fn install_and_restart<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<UpdaterState>();
        let staged = lock(&st.staged).take();
        let result = match &staged {
            None => Err("no update has been downloaded".to_owned()),
            Some(s) => s.update.install(&s.bytes).map_err(|e| e.to_string()),
        };
        match result {
            Ok(()) => {
                // A log-out arriving now goes through without asking.
                tally_macos_terminate::approve();
                app.request_restart();
            }
            Err(message) => {
                eprintln!("tally: installing the update failed: {message}");
                *lock(&st.staged) = staged;
                app.state::<Windows>().with(|r| r.forget_answers());
                aborted(
                    &app,
                    None,
                    RestartAborted {
                        cancelled: false,
                        message: Some(message),
                    },
                );
            }
        }
    });
}

// -- commands ----------------------------------------------------------------------

/// `update_status`: the running version, whether this build checks for
/// updates, and an update downloaded this session.
#[tauri::command]
pub async fn update_status(app: AppHandle) -> Result<UpdateStatus, ApiError> {
    let st = app.state::<UpdaterState>();
    let reason = unsupported_reason();
    Ok(UpdateStatus {
        current_version: app.package_info().version.to_string(),
        supported: reason.is_none(),
        reason: reason.map(str::to_owned),
        staged: staged_info(&st),
    })
}

/// `update_check`: a manual check (Help ▸ Check for Updates…). `null`
/// when up to date; ignores the skipped version; an update downloaded
/// this session is returned as it is (`downloaded: true`). Waits for an
/// automatic check in progress rather than running two at once.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, ApiError> {
    unsupported()?;
    let st = app.state::<UpdaterState>();
    let _guard = st.checking.lock().await;
    if let Some(info) = staged_info(&st) {
        return Ok(Some(info));
    }
    let found = check_once(&app).await?;
    let info = found.as_ref().map(|u| info_of(u, false));
    *lock(&st.found) = found;
    Ok(info)
}

/// Clears the "downloading" flag however the download ends.
struct Downloading<'a>(&'a AtomicBool);

impl Drop for Downloading<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// `update_download`: downloads the update the last check offered and
/// verifies its signature, with `update-progress` events to the calling
/// document. The verified package is kept for `update_restart`.
#[tauri::command]
pub async fn update_download(app: AppHandle, webview: Webview) -> Result<UpdateInfo, ApiError> {
    unsupported()?;
    crate::windows::require_doc_window(webview.label())?;
    let st = app.state::<UpdaterState>();
    if let Some(info) = staged_info(&st) {
        return Ok(info);
    }
    let mut update = lock(&st.found)
        .clone()
        .ok_or_else(|| ApiError::invalid_argument("no update to download: check first"))?;
    require_https(&update.download_url)?;
    if st.downloading.swap(true, Ordering::SeqCst) {
        return Err(ApiError::invalid_argument(
            "the update is already being downloaded",
        ));
    }
    let _downloading = Downloading(&st.downloading);
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let label = webview.label().to_owned();
    let mut meter = ProgressMeter::default();
    let bytes = update
        .download(
            |chunk, total| {
                if let Some(p) = meter.add(chunk, total) {
                    let _ = app.emit_to(label.as_str(), UPDATE_PROGRESS, p);
                }
            },
            || {},
        )
        .await
        .map_err(|e| plugin_err("downloading the update failed", &e))?;
    let info = info_of(&update, true);
    *lock(&st.staged) = Some(Staged { update, bytes });
    Ok(info)
}

/// `update_restart`: Install and Restart once downloaded. Asks every
/// edited document (Save / Don't Save / Cancel, as Quit does), then
/// installs and relaunches; Cancel keeps everything open and the update
/// downloaded.
#[tauri::command]
pub async fn update_restart(app: AppHandle, webview: Webview) -> Result<(), ApiError> {
    unsupported()?;
    crate::windows::require_doc_window(webview.label())?;
    if lock(&app.state::<UpdaterState>().staged).is_none() {
        return Err(ApiError::invalid_argument("no update has been downloaded"));
    }
    let step = app
        .state::<Windows>()
        .with(|r| r.begin_restart())
        .ok_or_else(|| ApiError::invalid_argument("another close is in progress"))?;
    crate::closing::run_step(&app, step);
    Ok(())
}

/// `update_skip(version)`: Skip This Version (automatic checks stay
/// silent for it and anything older).
#[tauri::command]
pub async fn update_skip(app: AppHandle, version: String) -> Result<(), ApiError> {
    if parse_version(&version).is_none() {
        return Err(ApiError::invalid_argument("not a version"));
    }
    let path = skip_path(&app).ok_or_else(|| ApiError::new(ApiError::IO, "no config dir"))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| ApiError::new(ApiError::IO, format!("creating the config dir: {e}")))?;
    }
    fs::write(&path, skip_json(&version))
        .map_err(|e| ApiError::new(ApiError::IO, format!("writing {SKIP_FILE}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_checks_default_on() {
        assert!(auto_check_enabled("{}"));
        assert!(auto_check_enabled("garbage"));
        assert!(auto_check_enabled(r#"{"checkForUpdates":"no"}"#));
        assert!(auto_check_enabled(r#"{"checkForUpdates":true}"#));
        assert!(!auto_check_enabled(r#"{"checkForUpdates":false}"#));
    }

    #[test]
    fn versions_compare_as_semver_and_never_downgrade() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer(&"9".repeat(100), "0.1.0"));
    }

    #[test]
    fn a_skipped_version_is_silent_until_a_newer_one() {
        assert!(should_prompt("0.2.0", None, false));
        assert!(!should_prompt("0.2.0", Some("0.2.0"), false));
        assert!(!should_prompt("0.1.5", Some("0.2.0"), false));
        assert!(should_prompt("0.2.1", Some("0.2.0"), false));
        assert!(should_prompt("v0.3.0", Some("v0.2.0"), false));
        // A manual check ignores the skip.
        assert!(should_prompt("0.2.0", Some("0.2.0"), true));
        // A bad skip entry does not silence anything.
        assert!(should_prompt("0.2.0", Some("junk"), false));
    }

    #[test]
    fn the_skip_file_round_trips_and_rejects_junk() {
        assert_eq!(
            skipped_version(&skip_json("0.2.0")).as_deref(),
            Some("0.2.0")
        );
        assert_eq!(skipped_version("{}"), None);
        assert_eq!(skipped_version(r#"{"skippedVersion":"soon"}"#), None);
        assert_eq!(skipped_version(r#"{"skippedVersion":7}"#), None);
        assert_eq!(skipped_version("not json"), None);
    }

    #[test]
    fn notes_are_plain_text_and_bounded() {
        assert_eq!(clean_notes(None), "");
        assert_eq!(
            clean_notes(Some("## Fixes\r\n- a\u{0}b\u{1b}[31m\t<b>x</b>\n")),
            "## Fixes\n- ab[31m\t<b>x</b>"
        );
        assert_eq!(clean_notes(Some("a\u{202E}b\u{2067}c")), "abc");
        let long = "x".repeat(MAX_NOTES_CHARS + 50);
        let cut = clean_notes(Some(&long));
        assert_eq!(cut.chars().count(), MAX_NOTES_CHARS + 1);
        assert!(cut.ends_with('…'));
        let exact = "y".repeat(MAX_NOTES_CHARS);
        assert_eq!(clean_notes(Some(&exact)), exact);
    }

    #[test]
    fn only_https_packages() {
        let ok: tauri::Url = "https://github.com/shane9b3/tally/releases/download/v0.2.0/x"
            .parse()
            .unwrap();
        assert!(require_https(&ok).is_ok());
        let plain: tauri::Url = "http://example.com/x".parse().unwrap();
        assert!(require_https(&plain).is_err());
        let file: tauri::Url = "file:///tmp/x".parse().unwrap();
        assert!(require_https(&file).is_err());
    }

    #[test]
    fn progress_is_throttled_to_whole_percents() {
        let mut m = ProgressMeter::default();
        let total = Some(1000);
        assert_eq!(m.add(1, total), Some(UpdateProgress { received: 1, total }));
        // 0.2 % is still 0 %.
        assert_eq!(m.add(1, total), None);
        assert!(m.add(10, total).is_some());
        let mut events = 0;
        for _ in 0..988 {
            if m.add(1, total).is_some() {
                events += 1;
            }
        }
        // 1 % … 100 %: one event per percent, the last chunk included.
        assert_eq!(events, 99);
        let mut unknown = ProgressMeter::default();
        assert!(unknown.add(1000, None).is_some());
        assert!(unknown.add(1000, None).is_none());
        assert!(unknown.add(512 * 1024, None).is_some());
    }

    #[test]
    fn debug_builds_never_check() {
        assert_eq!(unsupported_reason().is_some(), cfg!(debug_assertions));
    }

    #[test]
    fn event_payloads_are_camel_case() {
        let info = UpdateInfo {
            version: "0.2.0".into(),
            current_version: "0.1.0".into(),
            notes: "n".into(),
            date: None,
            downloaded: false,
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"version":"0.2.0","currentVersion":"0.1.0","notes":"n","date":null,"downloaded":false}"#
        );
        let a = RestartAborted {
            cancelled: true,
            message: None,
        };
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"{"cancelled":true,"message":null}"#
        );
    }
}
