//! Document windows and their tabs, and the window chrome.
//!
//! Every open document is a child webview `doc-<n>` of a native window
//! `win-<n>` (Tauri's multi-webview API): a window holds one or more
//! documents as tabs and shows one at a time; the others are hidden. Each
//! document keeps its own webview, so each has its own JS realm and stores,
//! as with one window per document. The document list is drawn by the
//! document webviews themselves, in the sidebar's "Open" section, from
//! `tabs-changed` (see [`notify_tabs`]), which lists every open document
//! (the window's own tabs first). Native macOS window tabs are off
//! (`native::disable_automatic_tabbing`, no tabbing identifier).
//!
//! Every document webview is created here, by [`open_document`]: at launch
//! (`lib.rs` setup), from `place_document` (New, Open, Open Recent,
//! templates and recovery in the webview), from the menu when no window is
//! left (macOS), and from a Dock click with no window ([`reopen`]). Labels
//! are `doc-<n>` / `win-<n>` (`registry.rs`); the capabilities grant the
//! app commands to webviews matching `doc-*` only.
//!
//! How a tab gets its document: the document is created in the engine
//! first (by the calling webview, or by the shell for a menu Open with no
//! window), then `place_document(doc, reuse)` either shows it in the
//! calling tab (the webview asked and its document is untitled and
//! untouched; the shell vetoes reuse of a tab it knows is edited) or
//! registers a new label bound to the document *before* building the
//! webview, as a new tab of the caller's window or in a new window (Tools ▸
//! Options ▸ "Open documents in", prefs `openDocumentsIn`). The new webview
//! asks `window_init()` on start and gets the engine's summary of its own
//! label's document (Tauri supplies the calling webview, so a webview
//! cannot ask for another tab's document; a reload gets the same document
//! again); without one it creates a blank workbook. No document id travels
//! in the URL.
//!
//! Window chrome (W5-5, §4 item 48): on Windows the windows run without OS
//! decorations and the React title bar in `apps/desktop/src/TitleBar.tsx`
//! draws the caption buttons, driven by the `core:window:*` permissions of
//! `capabilities/windows-titlebar.json`. `window_chrome` tells the UI which
//! chrome to draw; [`setup`] restores the DWM shadow. macOS keeps its
//! overlay title bar and Linux its native one. The native window title is
//! the shown tab's title, set here ([`update_window`]).

use serde::Serialize;
use tally_api::{ApiError, DocId, WorkbookInfo};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, Webview, Window};

use crate::closing;
use crate::commands::{blocking, close_engine_doc, with_doc, CmdResult};
use crate::registry::{
    is_doc_label, CloseKind, Created, OpenMode, Placement, Refused, TabClose, TabsState, Target,
    Windows,
};
use crate::state::{lock, AppState};

/// Menu id prefix of the Window menu's document list.
pub const FOCUS_PREFIX: &str = "window.focus.";
/// Offset of a new window from the front one (logical pixels).
const CASCADE: f64 = 28.0;
/// A new window's size when there is no front window (logical pixels).
const DEFAULT_SIZE: (f64, f64) = (1280.0, 820.0);
/// The smallest window (logical pixels).
const MIN_SIZE: (f64, f64) = (900.0, 600.0);
/// Event sent to every document webview of a window when its tabs change.
pub const TABS_CHANGED: &str = "tabs-changed";

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
pub fn setup<R: Runtime>(window: &Window<R>) {
    if cfg!(target_os = "windows") {
        // Decorations are off: keep the DWM shadow so the window still has a visible edge.
        if let Err(e) = window.set_shadow(true) {
            eprintln!("tally: window shadow: {e}");
        }
    }
    // Never a native tab (our own document list replaces them).
    crate::native::disallow_tabbing(window);
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

/// Pure: a new window's logical inner size: the front window's (physical
/// size and scale factor), never below the minimum; the default without one.
pub fn window_size(front: Option<((u32, u32), f64)>) -> (f64, f64) {
    match front {
        Some(((w, h), scale)) if scale.is_finite() && scale > 0.0 => (
            (f64::from(w) / scale).max(MIN_SIZE.0),
            (f64::from(h) / scale).max(MIN_SIZE.1),
        ),
        _ => DEFAULT_SIZE,
    }
}

/// Pure: the bounds of a document webview, the whole content area of its
/// window (`inner` in physical pixels; the overlay title bar on macOS is
/// part of it, as before).
pub fn content_bounds(inner: tauri::PhysicalSize<u32>) -> tauri::Rect {
    tauri::Rect {
        position: tauri::PhysicalPosition::new(0, 0).into(),
        size: inner.into(),
    }
}

/// Pure: the configuration of a document window.
pub fn window_config(
    label: &str,
    position: Option<(f64, f64)>,
    size: (f64, f64),
) -> tauri::utils::config::WindowConfig {
    let mut c = tauri::utils::config::WindowConfig {
        label: label.to_owned(),
        title: "Tally".to_owned(),
        width: size.0,
        height: size.1,
        min_width: Some(MIN_SIZE.0),
        min_height: Some(MIN_SIZE.1),
        center: position.is_none(),
        x: position.map(|p| p.0),
        y: position.map(|p| p.1),
        ..Default::default()
    };
    if cfg!(target_os = "macos") {
        c.title_bar_style = tauri::TitleBarStyle::Overlay;
        c.hidden_title = true;
        c.traffic_light_position = Some(tauri::utils::config::LogicalPosition { x: 18.0, y: 25.0 });
    }
    if cfg!(target_os = "windows") {
        c.decorations = false;
    }
    c
}

/// The front window's outer position and inner size (for a new window).
fn front_geometry<R: Runtime>(
    app: &AppHandle<R>,
    front: Option<&str>,
) -> (Option<(f64, f64)>, (f64, f64)) {
    let w = front.and_then(|l| app.get_window(l));
    let scale = w.as_ref().and_then(|w| w.scale_factor().ok());
    let position = cascade_from(w.as_ref().and_then(|w| {
        let p = w.outer_position().ok()?;
        Some(((p.x, p.y), scale?))
    }));
    let size = window_size(w.as_ref().and_then(|w| {
        let s = w.inner_size().ok()?;
        Some(((s.width, s.height), scale?))
    }));
    (position, size)
}

/// Builds the native window of an already registered label. Never call it
/// with the registry or the service lock held, and on Windows not from a
/// synchronous command or event handler on the main thread (wry#583):
/// callers spawn.
fn build_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    position: Option<(f64, f64)>,
    size: (f64, f64),
) -> tauri::Result<Window<R>> {
    let window =
        tauri::window::WindowBuilder::from_config(app, &window_config(label, position, size))?
            .build()?;
    setup(&window);
    Ok(window)
}

/// Adds the webview of an already registered document to its window,
/// filling the content area and following the window's size.
fn add_webview<R: Runtime>(window: &Window<R>, label: &str) -> tauri::Result<Webview<R>> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    window.add_child(
        tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::default()).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(size.width, size.height),
    )
}

/// The command error for a refused registry change; a document shown in
/// another tab is brought to the front.
pub fn refused<R: Runtime>(app: &AppHandle<R>, why: Refused) -> ApiError {
    if let Refused::ShownIn(other) = &why {
        reveal(app, other);
    }
    ApiError::invalid_argument(why.message())
}

/// Refuses a caller that is not a document webview (defence in depth: the
/// capabilities already grant the commands to `doc-*` only).
pub fn require_doc_window(label: &str) -> CmdResult<()> {
    if is_doc_label(label) {
        Ok(())
    } else {
        Err(ApiError::invalid_argument(Refused::UnknownWindow.message()))
    }
}

/// Opens a document as a new tab of a window or in a new window, showing
/// `doc` (a summary the shell got from the engine), or with `None` a new
/// blank workbook its webview creates. Returns the tab's label.
pub fn open_document<R: Runtime>(
    app: &AppHandle<R>,
    target: Target,
    doc: Option<WorkbookInfo>,
) -> CmdResult<String> {
    let windows = app.state::<Windows>();
    let (created, front) = windows
        .with(|r| {
            let front = r.front_window();
            r.create(target, doc.as_ref()).map(|c| (c, front))
        })
        .map_err(|why| refused(app, why))?;
    let Created {
        doc: label,
        window: win,
        new_window,
    } = created;
    let built = (|| {
        let window = if new_window {
            let (position, size) = front_geometry(app, front.as_deref());
            let w = build_window(app, &win, position, size)?;
            windows.with(|r| r.window_built(&win));
            w
        } else {
            app.get_window(&win).ok_or(tauri::Error::WindowNotFound)?
        };
        add_webview(&window, &label)
    })();
    match built {
        Ok(_) => {
            windows.with(|r| r.built(&label));
            if new_window {
                // Menu routing follows the new window at once (its first
                // Focused event can be missed when built off the main thread).
                focus_window(app, &win);
            }
            show_tab(app, &label);
            Ok(label)
        }
        Err(e) => {
            let empty = windows.with(|r| r.forget(&label));
            if let Some(w) = empty.and_then(|w| app.get_window(&w)) {
                let _ = w.destroy();
            }
            // An exit request was held back while this was being built; with
            // nothing left, Windows and Linux end the app.
            if !crate::closing::KEEP_ALIVE && windows.with(|r| r.is_empty()) {
                app.exit(0);
            }
            Err(ApiError::new(
                ApiError::INTERNAL,
                format!("cannot open a window: {e}"),
            ))
        }
    }
}

/// Opens a document in a new window from the main thread (menu, Dock):
/// creation is moved to the async runtime (wry#583 on Windows).
pub fn spawn_doc_window<R: Runtime>(app: &AppHandle<R>, doc: Option<WorkbookInfo>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = open_document(&app, Target::NewWindow, doc.clone()) {
            eprintln!("tally: {}", e.message);
            if let Some(info) = doc {
                close_doc(&app, info.doc).await;
            }
        }
    });
}

/// The engine's summary of an open document (`DocumentService::info`).
pub async fn engine_info<R: Runtime>(app: &AppHandle<R>, doc: DocId) -> CmdResult<WorkbookInfo> {
    blocking(app.clone(), move |_, st| with_doc(st, doc, |s| s.info(doc))).await
}

/// Brings a window to the front (un-minimising it).
pub fn focus_window<R: Runtime>(app: &AppHandle<R>, win: &str) {
    if let Some(w) = app.get_window(win) {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        if let Err(e) = w.set_focus() {
            eprintln!("tally: focusing {win}: {e}");
        }
        // The Focused event can come late (or not at all for a window made
        // from another thread): menu routing must follow at once.
        if app.state::<Windows>().with(|r| r.focus_changed(win, true)) {
            refresh_window_menu(app);
        }
    }
}

/// Brings a document to the front: its window, then its tab.
pub fn reveal<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(win) = app.state::<Windows>().with(|r| r.window_of_tab(label)) else {
        return;
    };
    focus_window(app, &win);
    show_tab(app, label);
}

/// Shows `label` in its window (hiding the window's other tabs), gives it
/// the keyboard focus, and updates the window title, the document lists and
/// the Window menu.
pub fn show_tab<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(shown) = app.state::<Windows>().with(|r| r.activate(label)) else {
        return;
    };
    if let Some(w) = app.get_webview(&shown.show) {
        // Shown first, then the others hidden: no blank frame in between.
        if let Err(e) = w.show() {
            eprintln!("tally: showing {}: {e}", shown.show);
        }
        if let Err(e) = w.set_focus() {
            eprintln!("tally: focusing {}: {e}", shown.show);
        }
    }
    for h in &shown.hide {
        if let Some(w) = app.get_webview(h) {
            if let Err(e) = w.hide() {
                eprintln!("tally: hiding {h}: {e}");
            }
        }
    }
    update_window(app, &shown.window);
    notify_tabs(app);
    refresh_window_menu(app);
}

/// The open documents as seen from a window, with each document's file
/// from the shell's own record of paths (`AppState.paths`).
pub fn tabs_state<R: Runtime>(app: &AppHandle<R>, win: &str) -> Option<TabsState> {
    let mut state = app.state::<Windows>().with(|r| r.tabs_state(win))?;
    let st = app.state::<AppState>();
    let paths = lock(&st.paths);
    // Paths of the window's own tabs only (tooltips); other windows'
    // documents are listed by title.
    for t in &mut state.tabs {
        t.path = t
            .doc
            .and_then(|d| paths.get(&d.0))
            .map(|p| p.display().to_string());
    }
    Some(state)
}

/// Sends `tabs-changed` to every document webview (each window's list puts
/// its own tabs first; every webview lists all open documents).
pub fn notify_tabs<R: Runtime>(app: &AppHandle<R>) {
    let wins = app.state::<Windows>().with(|r| r.window_labels());
    for win in wins {
        let Some(state) = tabs_state(app, &win) else {
            continue;
        };
        for t in &state.tabs {
            if let Err(e) = app.emit_to(t.label.as_str(), TABS_CHANGED, &state) {
                eprintln!("tally: emitting {TABS_CHANGED} to {}: {e}", t.label);
            }
        }
    }
}

/// Pure: the native title of a window showing `title` (` — Edited` off
/// macOS, where the red button's dot says it).
pub fn native_title(title: &str, edited: bool, mac: bool) -> String {
    if edited && !mac {
        format!("{title} — Edited")
    } else {
        title.to_owned()
    }
}

/// The native window title (the shown tab's title, which also names the
/// window in the Dock, Mission Control and the taskbar) and, on macOS, the
/// red button's "edited" dot (any tab edited: the button closes them all).
pub fn update_window<R: Runtime>(app: &AppHandle<R>, win: &str) {
    let (title, edited) = app
        .state::<Windows>()
        .with(|r| (r.window_title(win), r.window_edited(win)));
    let Some(window) = app.get_window(win) else {
        return;
    };
    if let Some((title, active_edited)) = title {
        let text = native_title(&title, active_edited, cfg!(target_os = "macos"));
        if window.title().ok().as_deref() != Some(text.as_str()) {
            if let Err(e) = window.set_title(&text) {
                eprintln!("tally: window title: {e}");
            }
        }
    }
    crate::native::set_edited(&window, edited);
}

/// Every window's native title and dot (after a UI language change:
/// "Untitled" is localised).
pub fn update_all_windows<R: Runtime>(app: &AppHandle<R>) {
    for win in app.state::<Windows>().with(|r| r.window_labels()) {
        update_window(app, &win);
    }
}

/// Closes a document in the engine, forgets its file and removes this
/// session's autosave snapshot (what the in-app Close did); a document the
/// engine no longer has is fine.
pub async fn close_doc<R: Runtime>(app: &AppHandle<R>, doc: DocId) {
    let result = blocking(app.clone(), move |app, st| {
        let _ = close_engine_doc(st, doc);
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn reopen<R: Runtime>(app: &AppHandle<R>, has_visible_windows: bool) {
    let front = app.state::<Windows>().with(|r| r.front_window());
    match front {
        None => spawn_doc_window(app, None),
        Some(win) if !has_visible_windows => focus_window(app, &win),
        Some(_) => {}
    }
}

/// Rebuilds the Window menu's document list (every platform: the Window
/// menu is the shell's own, not AppKit's `windowsMenu`). Runs on the main
/// thread (inline when already there), so two refreshes from different
/// threads never interleave their removes and appends.
pub fn refresh_window_menu<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        let list = handle.state::<Windows>().with(|r| r.window_list());
        if let Err(e) = crate::menu::set_window_list(&handle, &list) {
            eprintln!("tally: window menu: {e}");
        }
    });
    if let Err(e) = result {
        eprintln!("tally: window menu: {e}");
    }
}

/// Closes a tab that is not the last of its window (the guard has run):
/// the window shows the next tab, the tab's webview goes, and its
/// document, print windows and any running sequence are dealt with.
pub fn close_tab_now<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(win) = app.state::<Windows>().with(|r| r.window_of_tab(label)) else {
        return;
    };
    let gone = app.state::<Windows>().with(|r| r.tab_closed(label));
    let next = gone.activate.clone();
    // The next tab first, so the window never shows an empty frame.
    if let Some(next) = &next {
        show_tab(app, next);
    }
    if let Some(w) = app.get_webview(label) {
        if let Err(e) = w.close() {
            eprintln!("tally: closing {label}: {e}");
        }
    }
    closing::clean_up(app, gone);
    if next.is_none() {
        // Nothing left in it (not expected: the last tab closes the window).
        if let Some(w) = app.get_window(&win) {
            let _ = w.destroy();
        }
    }
}

/// Closes tab `label` (its ×, ⌘W through `confirm_close`, Close Other
/// Tabs): an edited tab is shown and asked first; the last tab of a window
/// closes the window.
pub fn request_tab_close<R: Runtime>(app: &AppHandle<R>, label: &str) {
    match app.state::<Windows>().with(|r| r.close_tab(label)) {
        Some(TabClose::Ask) => {
            reveal(app, label);
            // A failed ask is logged there; this tab just stays open.
            let _ = closing::ask(app, label, CloseKind::Window);
        }
        Some(TabClose::Window(win)) => {
            if let Some(w) = app.get_window(&win) {
                if let Err(e) = w.destroy() {
                    eprintln!("tally: closing the window: {e}");
                }
            }
        }
        Some(TabClose::Now) => close_tab_now(app, label),
        None => {}
    }
}

/// Moves a webview into another window: sized to it first (a larger
/// webview would grow the window on macOS), then reparented, then fitted.
/// The target is looked up again right before the reparent (the runtime
/// panics on a reparent into a window that is gone).
fn move_webview<R: Runtime>(
    app: &AppHandle<R>,
    webview: &Webview<R>,
    to: &str,
) -> tauri::Result<()> {
    let target = app.get_window(to).ok_or(tauri::Error::WindowNotFound)?;
    let inner = target.inner_size()?;
    webview.set_bounds(content_bounds(inner))?;
    let target = app.get_window(to).ok_or(tauri::Error::WindowNotFound)?;
    webview.reparent(&target)?;
    webview.set_bounds(content_bounds(target.inner_size()?))
}

/// Destroys a document window (logged on failure).
fn destroy_window<R: Runtime>(app: &AppHandle<R>, win: &str) {
    if let Some(w) = app.get_window(win) {
        if let Err(e) = w.destroy() {
            eprintln!("tally: closing {win}: {e}");
        }
    }
}

/// Move Tab to New Window: `label` leaves its window (which must keep at
/// least one tab) for a new window of the same size, cascaded from it.
/// Checked and registered in one registry step (no two moves of one tab,
/// not during a close sequence, the window cap); every failure closes and
/// forgets the new window and leaves the tab where it was.
pub fn detach<R: Runtime>(app: &AppHandle<R>, label: &str) -> CmdResult<()> {
    let windows = app.state::<Windows>();
    let Some(d) = windows
        .with(|r| r.begin_detach(label))
        .map_err(|why| refused(app, why))?
    else {
        return Ok(());
    };
    let result = detach_into(app, label, &d.from, &d.to);
    windows.with(|r| r.end_detach(label));
    if result.is_err() {
        destroy_window(app, &d.to);
        windows.with(|r| r.forget_window(&d.to));
    }
    result
}

fn detach_into<R: Runtime>(app: &AppHandle<R>, label: &str, from: &str, to: &str) -> CmdResult<()> {
    let windows = app.state::<Windows>();
    let internal = |what: &str, e: &dyn std::fmt::Display| {
        ApiError::new(ApiError::INTERNAL, format!("{what}: {e}"))
    };
    let webview = app
        .get_webview(label)
        .ok_or_else(|| ApiError::invalid_argument(Refused::UnknownWindow.message()))?;
    let (position, size) = front_geometry(app, Some(from));
    build_window(app, to, position, size).map_err(|e| internal("cannot open a window", &e))?;
    windows.with(|r| r.window_built(to));
    // The tab may have closed or moved while the window was being built.
    if windows.with(|r| r.window_of_tab(label)).as_deref() != Some(from) {
        return Err(ApiError::invalid_argument(Refused::UnknownWindow.message()));
    }
    let moved = windows
        .with(|r| r.move_to_window(label, to))
        .map_err(|why| refused(app, why))?;
    // The old window shows its next tab before this one leaves it.
    if let Some(next) = &moved.from_active {
        show_tab(app, next);
    }
    let _ = webview.hide();
    if let Err(e) = move_webview(app, &webview, to) {
        // Put it back where it was.
        let _ = windows.with(|r| r.move_to_window(label, from));
        show_tab(app, label);
        return Err(internal("cannot move the tab", &e));
    }
    if moved.from_active.is_none() {
        // Its other tab went meanwhile: the old window is empty now.
        destroy_window(app, from);
    }
    focus_window(app, to);
    show_tab(app, label);
    notify_tabs(app);
    Ok(())
}

/// Merge All Windows: every document of the other windows becomes a tab of
/// `into`, which keeps its shown tab; the emptied windows close. A tab that
/// cannot be moved goes back to its window, which then stays open. Nothing
/// happens while a close sequence is asking or a tab is being detached.
pub fn merge<R: Runtime>(app: &AppHandle<R>, into: &str) {
    if app.get_window(into).is_none() {
        return;
    }
    let windows = app.state::<Windows>();
    let m = windows.with(|r| r.merge_into(into));
    let mut kept: Vec<String> = Vec::new();
    for (label, from) in &m.moved {
        let Some(webview) = app.get_webview(label) else {
            continue;
        };
        let _ = webview.hide();
        if let Err(e) = move_webview(app, &webview, into) {
            eprintln!("tally: merging {label}: {e}");
            let _ = windows.with(|r| r.move_to_window(label, from));
            let _ = webview.show();
            kept.push(from.clone());
        }
    }
    for win in m.emptied.iter().filter(|w| !kept.contains(w)) {
        destroy_window(app, win);
    }
    focus_window(app, into);
    if let Some(active) = windows.with(|r| r.active_of(into)) {
        show_tab(app, &active);
    }
    for win in &kept {
        if let Some(active) = windows.with(|r| r.active_of(win)) {
            show_tab(app, &active);
        }
    }
}

/// What a document webview gets on start (`window_init`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInit {
    /// The document this tab shows; `null`: create a blank one.
    pub doc: Option<WorkbookInfo>,
    /// The first tab of the session offers the crash recovery.
    pub offer_recovery: bool,
    /// The calling webview's own label (its tab in `tabs`).
    pub label: String,
    /// The open documents, its window's first.
    pub tabs: Option<TabsState>,
    /// A close sequence is asking this document (the webview was reloaded
    /// while asked): ask again with this kind.
    pub ask: Option<CloseKind>,
    /// Files the OS asked to open before any webview had started (Tally
    /// launched by opening them): the webview opens them, the first in
    /// place of its blank workbook (`dialogs::OsOpen`).
    pub open_paths: Vec<String>,
}

/// `window_init()`: the document the calling tab shows (from the engine,
/// so also after a reload of the webview), whether it offers the recovery
/// of autosaved work (the session's first tab only), and its window's tabs.
#[tauri::command]
pub async fn window_init(app: AppHandle, webview: Webview) -> CmdResult<WindowInit> {
    let label = webview.label().to_owned();
    require_doc_window(&label)?;
    let (doc, offer_recovery) = app
        .state::<Windows>()
        .with(|r| r.init(&label))
        .map_err(|why| refused(&app, why))?;
    let doc = match doc {
        // A document the engine no longer has: start blank instead.
        Some(d) => engine_info(&app, d).await.ok(),
        None => None,
    };
    let (win, ask) = app.state::<Windows>().with(|r| {
        let ask = match r.reask() {
            Some(crate::registry::Step::Ask(l, kind)) if l == label => Some(kind),
            _ => None,
        };
        (r.window_of_tab(&label), ask)
    });
    let open_paths = app
        .try_state::<crate::dialogs::OsOpen>()
        .map(|o| o.take_pending())
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.display().to_string())
        .collect();
    Ok(WindowInit {
        doc,
        offer_recovery,
        tabs: win.and_then(|w| tabs_state(&app, &w)),
        label,
        ask,
        open_paths,
    })
}

/// The answer of `place_document`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Placed {
    /// Show it in the calling tab (`AppController.load`).
    Here,
    /// It opened in a new tab of the caller's window.
    Tab,
    /// It opened in a new window.
    Window,
}

/// The "Open documents in" setting from the stored prefs.
fn open_mode<R: Runtime>(app: &AppHandle<R>) -> OpenMode {
    crate::prefs::open_mode(&crate::prefs::load(app))
}

/// Opens a placement that is not [`Placement::Here`].
fn open_placed<R: Runtime>(
    app: &AppHandle<R>,
    p: Placement,
    doc: Option<WorkbookInfo>,
) -> CmdResult<Placed> {
    match p {
        Placement::Here => Ok(Placed::Here),
        Placement::NewTab(win) => open_document(app, Target::Window(win), doc).map(|_| Placed::Tab),
        Placement::NewWindow => open_document(app, Target::NewWindow, doc).map(|_| Placed::Window),
    }
}

/// `place_document(doc, reuse)`: shows a document the calling webview just
/// created or opened. `doc: null` opens a new blank document (File ▸ New).
/// The summary handed to a new tab comes from the engine (an unknown
/// document is refused), never from the webview. With `reuse` (the
/// caller's document is untitled and untouched) and a tab the shell knows
/// is not edited, the document replaces the caller's (`"here"`); otherwise
/// it opens in a new tab of the caller's window (`"tab"`) or a new window
/// (`"window"`), as the "Open documents in" setting says. A document
/// already shown in another tab is refused (that tab comes to the front);
/// when the new tab cannot be created, the document is closed again.
#[tauri::command]
pub async fn place_document(
    app: AppHandle,
    webview: Webview,
    doc: Option<DocId>,
    reuse: bool,
) -> CmdResult<Placed> {
    let caller = webview.label().to_owned();
    require_doc_window(&caller)?;
    let mode = open_mode(&app);
    let windows = app.state::<Windows>();
    let Some(doc) = doc else {
        let p = windows
            .with(|r| r.place_new(&caller, mode))
            .map_err(|why| refused(&app, why))?;
        return open_placed(&app, p, None);
    };
    let info = engine_info(&app, doc).await?;
    let placed = windows.with(|r| {
        let p = r.place(&caller, doc, reuse, mode)?;
        if p == Placement::Here {
            r.bind(&caller, &info)?;
        }
        Ok(p)
    });
    match placed {
        Err(why) => Err(refused(&app, why)),
        Ok(Placement::Here) => {
            if let Some(win) = windows.with(|r| r.window_of_tab(&caller)) {
                update_window(&app, &win);
                notify_tabs(&app);
            }
            refresh_window_menu(&app);
            Ok(Placed::Here)
        }
        Ok(p) => match open_placed(&app, p, Some(info)) {
            Ok(placed) => Ok(placed),
            Err(e) => {
                close_doc(&app, doc).await;
                Err(e)
            }
        },
    }
}

/// The caller's window, and `label` checked to be one of its tabs.
fn own_tab<R: Runtime>(app: &AppHandle<R>, caller: &str, label: &str) -> CmdResult<String> {
    require_doc_window(caller)?;
    let windows = app.state::<Windows>();
    let (mine, theirs) = windows.with(|r| (r.window_of_tab(caller), r.window_of_tab(label)));
    match (mine, theirs) {
        (Some(a), Some(b)) if a == b => Ok(a),
        _ => Err(ApiError::invalid_argument(Refused::UnknownWindow.message())),
    }
}

/// The caller is a document webview and `label` an open document (in any window).
fn any_tab<R: Runtime>(app: &AppHandle<R>, caller: &str, label: &str) -> CmdResult<()> {
    require_doc_window(caller)?;
    if app
        .state::<Windows>()
        .with(|r| r.window_of_tab(label))
        .is_some()
    {
        Ok(())
    } else {
        Err(ApiError::invalid_argument(Refused::UnknownWindow.message()))
    }
}

/// `tab_activate(label)`: shows an open document: its tab in the caller's
/// window, or its window (brought to the front) and tab. These tab
/// commands are async (off the main thread), so a close they cause never
/// runs inside the calling webview's own IPC request.
#[tauri::command]
pub async fn tab_activate(app: AppHandle, webview: Webview, label: String) -> CmdResult<()> {
    any_tab(&app, webview.label(), &label)?;
    reveal(&app, &label);
    Ok(())
}

/// `tab_close(label)`: closes an open document (an edited one is shown and
/// asks first, in its own webview; the last tab closes its window).
#[tauri::command]
pub async fn tab_close(app: AppHandle, webview: Webview, label: String) -> CmdResult<()> {
    any_tab(&app, webview.label(), &label)?;
    request_tab_close(&app, &label);
    Ok(())
}

/// `tab_close_others(label)`: closes every other tab of the caller's
/// window; the edited ones are asked in turn and Cancel keeps them all.
#[tauri::command]
pub async fn tab_close_others(app: AppHandle, webview: Webview, label: String) -> CmdResult<()> {
    own_tab(&app, webview.label(), &label)?;
    let step = app
        .state::<Windows>()
        .with(|r| r.begin_close_others(&label).or_else(|| r.reask()));
    if let Some(step) = step {
        closing::run_step(&app, step);
    }
    Ok(())
}

/// `tab_move(label, index)`: moves a tab of the caller's window to
/// position `index` of its window.
#[tauri::command]
pub async fn tab_move(
    app: AppHandle,
    webview: Webview,
    label: String,
    index: usize,
) -> CmdResult<()> {
    own_tab(&app, webview.label(), &label)?;
    if app.state::<Windows>().with(|r| r.move_tab(&label, index)) {
        notify_tabs(&app);
        refresh_window_menu(&app);
    }
    Ok(())
}

/// `tab_detach(label)`: Move Tab to New Window (nothing with one tab).
#[tauri::command]
pub async fn tab_detach(app: AppHandle, webview: Webview, label: String) -> CmdResult<()> {
    own_tab(&app, webview.label(), &label)?;
    detach(&app, &label)
}

/// `merge_windows()`: Merge All Windows into the caller's window.
#[tauri::command]
pub async fn merge_windows(app: AppHandle, webview: Webview) -> CmdResult<()> {
    let caller = webview.label().to_owned();
    let win = own_tab(&app, &caller, &caller)?;
    merge(&app, &win);
    Ok(())
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
    fn new_windows_cascade_from_the_front_one_at_its_size() {
        assert_eq!(cascade_from(None), None);
        assert_eq!(cascade_from(Some(((200, 100), 2.0))), Some((128.0, 78.0)));
        assert_eq!(cascade_from(Some(((10, 10), 1.0))), Some((38.0, 38.0)));
        assert_eq!(cascade_from(Some(((10, 10), 0.0))), None);
        assert_eq!(window_size(None), DEFAULT_SIZE);
        assert_eq!(window_size(Some(((2400, 1600), 2.0))), (1200.0, 800.0));
        // Never below the minimum (and a bad scale falls back to the default).
        assert_eq!(window_size(Some(((400, 300), 1.0))), MIN_SIZE);
        assert_eq!(window_size(Some(((2400, 1600), f64::NAN))), DEFAULT_SIZE);
    }

    #[test]
    fn a_document_webview_fills_its_window() {
        let r = content_bounds(tauri::PhysicalSize::new(2560, 1640));
        assert_eq!(
            r.position,
            tauri::Position::Physical(tauri::PhysicalPosition::new(0, 0))
        );
        assert_eq!(
            r.size,
            tauri::Size::Physical(tauri::PhysicalSize::new(2560, 1640))
        );
    }

    #[test]
    fn document_windows_are_configured_per_platform() {
        let c = window_config("win-3", Some((40.0, 50.0)), (1000.0, 700.0));
        assert_eq!(c.label, "win-3");
        assert_eq!((c.width, c.height), (1000.0, 700.0));
        assert_eq!((c.x, c.y, c.center), (Some(40.0), Some(50.0), false));
        assert_eq!(
            (c.min_width, c.min_height),
            (Some(MIN_SIZE.0), Some(MIN_SIZE.1))
        );
        // Native tabs are never asked for.
        assert_eq!(c.tabbing_identifier, None);
        assert!(window_config("win-1", None, DEFAULT_SIZE).center);
        assert_eq!(c.decorations, !cfg!(target_os = "windows"));
        assert_eq!(c.hidden_title, cfg!(target_os = "macos"));
        assert_eq!(
            c.traffic_light_position.is_some(),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn the_native_title_says_edited_off_macos() {
        assert_eq!(native_title("A.ods", true, false), "A.ods — Edited");
        assert_eq!(native_title("A.ods", true, true), "A.ods");
        assert_eq!(native_title("A.ods", false, false), "A.ods");
    }

    #[test]
    fn placement_answers_are_camel_case() {
        for (p, json) in [
            (Placed::Here, r#""here""#),
            (Placed::Tab, r#""tab""#),
            (Placed::Window, r#""window""#),
        ] {
            assert_eq!(serde_json::to_string(&p).unwrap_or_default(), json);
        }
        let init = WindowInit {
            doc: None,
            offer_recovery: true,
            label: "doc-1".into(),
            tabs: None,
            ask: None,
            open_paths: vec!["/tmp/a.xlsm".into()],
        };
        assert_eq!(
            serde_json::to_string(&init).unwrap_or_default(),
            r#"{"doc":null,"offerRecovery":true,"label":"doc-1","tabs":null,"ask":null,"openPaths":["/tmp/a.xlsm"]}"#
        );
    }
}
