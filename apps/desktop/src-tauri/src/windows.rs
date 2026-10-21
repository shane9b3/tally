//! Document windows (one document per window, native macOS window tabs) and
//! the window chrome.
//!
//! Every document window is created here, by [`open_doc_window`]: at launch
//! (`lib.rs` setup), from `place_document` (New, Open, Open Recent,
//! templates and recovery in the webview), from the menu when no window is
//! left (macOS), and from a Dock click with no window ([`reopen`]). Labels
//! are `doc-<n>` (`registry.rs`), matched by the `doc-*` capability globs.
//!
//! How a window gets its document: the document is created in the engine
//! first (by the calling webview, or by the shell for a menu Open with no
//! window), then `place_document(doc, reuse)` either shows it in the
//! calling window (the webview asked and its document is untitled and
//! untouched; the shell vetoes reuse of a window it knows is edited) or
//! registers a new label bound to the document *before* building the
//! window. The new webview asks `window_init()` on start and gets the
//! engine's summary of its own label's document (Tauri supplies the calling
//! window, so a webview cannot ask for another window's document; a reload
//! gets the same document again); without one it creates a blank workbook.
//! `place_document` takes only the document id: the summary a new window
//! shows always comes from the engine. No document id travels in the URL.
//!
//! Window chrome (W5-5, §4 item 48): on Windows the windows run without OS
//! decorations and the React title bar in `apps/desktop/src/TitleBar.tsx`
//! draws the caption buttons, driven by the `core:window:*` permissions of
//! `capabilities/windows-titlebar.json`. `window_chrome` tells the UI which
//! chrome to draw; [`setup`] restores the DWM shadow and resize borders that
//! go away with the decorations. macOS keeps its overlay title bar and Linux
//! its native one.

use serde::Serialize;
use tally_api::{ApiError, DocId, WorkbookInfo};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

use crate::commands::{blocking, with_service, CmdResult};
use crate::registry::{is_doc_label, Placement, Refused, Windows};
use crate::state::lock;

/// The macOS tabbing identifier shared by every document window, so the
/// system "Prefer tabs" setting and Window ▸ Merge All Windows group them.
pub const TABBING_ID: &str = "dev.tally.document";
/// Menu id prefix of the Window menu's window list (Windows/Linux).
pub const FOCUS_PREFIX: &str = "window.focus.";
/// Offset of a new window from the front one (logical pixels).
const CASCADE: f64 = 28.0;

/// What the UI asks on start (`window_chrome`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// The React title bar replaces the OS one.
    pub custom_title_bar: bool,
    /// `macos`, `windows`, `linux` or `other`.
    pub platform: &'static str,
}

/// The chrome for an OS name (`std::env::consts::OS`).
pub fn chrome_for(os: &str) -> WindowChrome {
    let platform: &'static str = match os {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => "other",
    };
    WindowChrome {
        custom_title_bar: os == "windows",
        platform,
    }
}

/// Platform setup of a document window.
pub fn setup<R: Runtime>(window: &WebviewWindow<R>) {
    if cfg!(target_os = "windows") {
        // Decorations are off: keep the DWM shadow so the window still has a visible edge.
        if let Err(e) = window.set_shadow(true) {
            eprintln!("tally: window shadow: {e}");
        }
    }
}

/// `window_chrome`: which title bar the UI draws.
#[tauri::command]
pub fn window_chrome() -> WindowChrome {
    chrome_for(std::env::consts::OS)
}

/// Pure: where a new window goes, `CASCADE` below and right of the front
/// window (`front` is its outer position in physical pixels and its scale
/// factor); `None` centres it.
pub fn cascade_from(front: Option<((i32, i32), f64)>) -> Option<(f64, f64)> {
    let ((x, y), scale) = front?;
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    Some((
        f64::from(x) / scale + CASCADE,
        f64::from(y) / scale + CASCADE,
    ))
}

/// Builds the window of an already registered label. Never call it with the
/// registry or the service lock held, and on Windows not from a synchronous
/// command or event handler on the main thread (wry#583): callers spawn.
fn build<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    position: Option<(f64, f64)>,
) -> tauri::Result<WebviewWindow<R>> {
    let mut b = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::default())
        .title("Tally")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0);
    b = match position {
        Some((x, y)) => b.position(x, y),
        None => b.center(),
    };
    #[cfg(target_os = "macos")]
    {
        b = b
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(18.0, 25.0))
            .tabbing_identifier(TABBING_ID);
    }
    if cfg!(target_os = "windows") {
        b = b.decorations(false);
    }
    let window = b.build()?;
    setup(&window);
    Ok(window)
}

/// The command error for a refused registry change; a document shown in
/// another window brings that window to the front.
pub fn refused<R: Runtime>(app: &AppHandle<R>, why: Refused) -> ApiError {
    if let Refused::ShownIn(other) = &why {
        focus(app, other);
    }
    ApiError::invalid_argument(why.message())
}

/// Refuses a caller that is not a document window (defence in depth: the
/// capabilities already grant the commands to `doc-*` only).
pub fn require_doc_window(label: &str) -> CmdResult<()> {
    if is_doc_label(label) {
        Ok(())
    } else {
        Err(ApiError::invalid_argument(Refused::UnknownWindow.message()))
    }
}

/// Opens a document window that shows `doc` (a summary the shell got from
/// the engine), or with `None` a new blank workbook its webview creates.
/// Returns the window's label.
pub fn open_doc_window<R: Runtime>(
    app: &AppHandle<R>,
    doc: Option<WorkbookInfo>,
) -> CmdResult<String> {
    let windows = app.state::<Windows>();
    let (label, front) = windows
        .with(|r| {
            let front = r.front();
            r.create(doc.as_ref()).map(|l| (l, front))
        })
        .map_err(|why| refused(app, why))?;
    let position = cascade_from(
        front
            .and_then(|l| app.get_webview_window(&l))
            .and_then(|w| {
                let p = w.outer_position().ok()?;
                Some(((p.x, p.y), w.scale_factor().ok()?))
            }),
    );
    match build(app, &label, position) {
        Ok(_) => {
            windows.with(|r| r.built(&label));
            refresh_window_menu(app);
            Ok(label)
        }
        Err(e) => {
            let empty = windows.with(|r| {
                r.forget(&label);
                r.is_empty()
            });
            // An exit request was held back while this window was being
            // built; with nothing left, Windows and Linux end the app.
            if empty && !crate::closing::KEEP_ALIVE && app.webview_windows().is_empty() {
                app.exit(0);
            }
            Err(ApiError::new(
                ApiError::INTERNAL,
                format!("cannot open a window: {e}"),
            ))
        }
    }
}

/// Opens a document window from the main thread (menu, Dock): window
/// creation is moved to the async runtime (wry#583 on Windows).
pub fn spawn_doc_window<R: Runtime>(app: &AppHandle<R>, doc: Option<WorkbookInfo>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = open_doc_window(&app, doc.clone()) {
            eprintln!("tally: {}", e.message);
            if let Some(info) = doc {
                close_doc(&app, info.doc).await;
            }
        }
    });
}

/// The engine's summary of an open document (`DocumentService::info`).
pub async fn engine_info<R: Runtime>(app: &AppHandle<R>, doc: DocId) -> CmdResult<WorkbookInfo> {
    blocking(app.clone(), move |_, st| with_service(st, |s| s.info(doc))).await
}

/// Brings a window to the front (un-minimising it).
pub fn focus<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        if let Err(e) = w.set_focus() {
            eprintln!("tally: focusing {label}: {e}");
        }
    }
}

/// Closes a document in the engine, forgets its file and removes this
/// session's autosave snapshot (what the in-app Close did); a document the
/// engine no longer has is fine.
pub async fn close_doc<R: Runtime>(app: &AppHandle<R>, doc: DocId) {
    let result = blocking(app.clone(), move |app, st| {
        let _ = with_service(st, |s| s.close(doc));
        lock(&st.paths).remove(&doc.0);
        crate::autosave::remove_snapshot(app, doc);
        Ok(())
    })
    .await;
    if let Err(e) = result {
        eprintln!("tally: closing document {}: {}", doc.0, e.message);
    }
}

/// `RunEvent::Reopen` (Dock icon clicked): with no document window, a new
/// Untitled one; with only minimised ones, the front one comes back.
pub fn reopen<R: Runtime>(app: &AppHandle<R>, has_visible_windows: bool) {
    let front = app.state::<Windows>().with(|r| r.front());
    match front {
        None => spawn_doc_window(app, None),
        Some(label) if !has_visible_windows => focus(app, &label),
        Some(_) => {}
    }
}

/// Rebuilds the Window menu's window list (Windows/Linux). macOS lists the
/// windows itself: the Window submenu is `NSApp.windowsMenu` (Tauri sets it
/// for [`tauri::menu::WINDOW_SUBMENU_ID`]), and AppKit keeps the titles,
/// the check mark on the key window and the tab items up to date.
pub fn refresh_window_menu<R: Runtime>(app: &AppHandle<R>) {
    if cfg!(target_os = "macos") {
        return;
    }
    let list = app.state::<Windows>().with(|r| r.window_list());
    if let Err(e) = crate::menu::set_window_list(app, &list) {
        eprintln!("tally: window menu: {e}");
    }
}

/// What a document window's webview gets on start (`window_init`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInit {
    /// The document this window shows; `null`: create a blank one.
    pub doc: Option<WorkbookInfo>,
    /// The first window of the session offers the crash recovery.
    pub offer_recovery: bool,
}

/// `window_init()`: the document the calling window shows (from the engine,
/// so also after a reload of the webview), and whether it offers the
/// recovery of autosaved work (the session's first window only).
#[tauri::command]
pub async fn window_init(app: AppHandle, window: WebviewWindow) -> CmdResult<WindowInit> {
    require_doc_window(window.label())?;
    let (doc, offer_recovery) = app
        .state::<Windows>()
        .with(|r| r.init(window.label()))
        .map_err(|why| refused(&app, why))?;
    let doc = match doc {
        // A document the engine no longer has: start blank instead.
        Some(d) => engine_info(&app, d).await.ok(),
        None => None,
    };
    Ok(WindowInit {
        doc,
        offer_recovery,
    })
}

/// The answer of `place_document`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Placed {
    /// Show it in the calling window (`AppController.load`).
    Here,
    /// It opened in a new window.
    Window,
}

/// `place_document(doc, reuse)`: shows a document the calling webview just
/// created or opened. `doc: null` opens a new blank window (File ▸ New).
/// The summary handed to a new window comes from the engine (an unknown
/// document is refused), never from the webview. With `reuse` (the
/// caller's document is untitled and untouched) and a window the shell
/// knows is not edited, the document replaces the caller's (`"here"`);
/// otherwise it opens in a new window (`"window"`). A document already
/// shown in another window is refused (that window comes to the front);
/// when the new window cannot be created, the document is closed again.
#[tauri::command]
pub async fn place_document(
    app: AppHandle,
    window: WebviewWindow,
    doc: Option<DocId>,
    reuse: bool,
) -> CmdResult<Placed> {
    let caller = window.label().to_owned();
    require_doc_window(&caller)?;
    let Some(doc) = doc else {
        open_doc_window(&app, None)?;
        return Ok(Placed::Window);
    };
    let info = engine_info(&app, doc).await?;
    let windows = app.state::<Windows>();
    let placed = windows.with(|r| {
        let p = r.place(&caller, doc, reuse)?;
        if p == Placement::Here {
            r.bind(&caller, &info)?;
        }
        Ok(p)
    });
    match placed {
        Err(why) => Err(refused(&app, why)),
        Ok(Placement::Here) => {
            refresh_window_menu(&app);
            Ok(Placed::Here)
        }
        Ok(Placement::NewWindow) => match open_doc_window(&app, Some(info)) {
            Ok(_) => Ok(Placed::Window),
            Err(e) => {
                close_doc(&app, doc).await;
                Err(e)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_gets_the_custom_title_bar() {
        assert_eq!(
            chrome_for("windows"),
            WindowChrome {
                custom_title_bar: true,
                platform: "windows"
            }
        );
        assert!(!chrome_for("macos").custom_title_bar);
        assert_eq!(chrome_for("macos").platform, "macos");
        assert!(!chrome_for("linux").custom_title_bar);
        assert_eq!(chrome_for("freebsd").platform, "other");
    }

    #[test]
    fn new_windows_cascade_from_the_front_one() {
        assert_eq!(cascade_from(None), None);
        assert_eq!(cascade_from(Some(((200, 100), 2.0))), Some((128.0, 78.0)));
        assert_eq!(cascade_from(Some(((10, 10), 1.0))), Some((38.0, 38.0)));
        assert_eq!(cascade_from(Some(((10, 10), 0.0))), None);
    }

    #[test]
    fn placement_answers_are_camel_case() {
        assert_eq!(
            serde_json::to_string(&Placed::Here).unwrap_or_default(),
            r#""here""#
        );
        assert_eq!(
            serde_json::to_string(&Placed::Window).unwrap_or_default(),
            r#""window""#
        );
        let init = WindowInit {
            doc: None,
            offer_recovery: true,
        };
        assert_eq!(
            serde_json::to_string(&init).unwrap_or_default(),
            r#"{"doc":null,"offerRecovery":true}"#
        );
    }
}
