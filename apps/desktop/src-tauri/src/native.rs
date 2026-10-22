//! The few AppKit calls the shell makes itself (safe `objc2-app-kit` API
//! only; the workspace forbids `unsafe`): the red close button's "document
//! edited" dot, and turning native window tabs off (Tally draws its own
//! document list in the sidebar, `windows.rs`). No-ops on the other platforms.

use tauri::{AppHandle, Runtime, Window};

/// Runs `f` on the main thread with the `NSWindow` of `window`, found by
/// comparing pointers with `NSApp.windows` (no pointer is dereferenced).
#[cfg(target_os = "macos")]
fn with_ns_window<R: Runtime>(
    window: &Window<R>,
    f: impl FnOnce(&objc2_app_kit::NSWindow) + Send + 'static,
) {
    let target = match window.ns_window() {
        Ok(ptr) => ptr as usize,
        Err(e) => {
            eprintln!("tally: no NSWindow: {e}");
            return;
        }
    };
    let result = window.run_on_main_thread(move || {
        use objc2::rc::Retained;
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let windows = NSApplication::sharedApplication(mtm).windows();
        for i in 0..windows.count() {
            let w = windows.objectAtIndex(i);
            if Retained::as_ptr(&w) as usize == target {
                f(&w);
                return;
            }
        }
    });
    if let Err(e) = result {
        eprintln!("tally: window call: {e}");
    }
}

/// The macOS "document edited" dot on a window's red close button.
#[cfg(target_os = "macos")]
pub fn set_edited<R: Runtime>(window: &Window<R>, edited: bool) {
    with_ns_window(window, move |w| {
        if w.isDocumentEdited() != edited {
            w.setDocumentEdited(edited);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn set_edited<R: Runtime>(_window: &Window<R>, _edited: bool) {}

/// This window never becomes a native tab, whatever the "Prefer tabs"
/// setting (belt and braces with [`disable_automatic_tabbing`]).
#[cfg(target_os = "macos")]
pub fn disallow_tabbing<R: Runtime>(window: &Window<R>) {
    with_ns_window(window, |w| {
        w.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);
    });
}

#[cfg(not(target_os = "macos"))]
pub fn disallow_tabbing<R: Runtime>(_window: &Window<R>) {}

/// Turns native window tabbing off for the app: no automatic tabs ("Prefer
/// tabs: Always"), no tab items in the Window menu, no View ▸ Show Tab Bar.
#[cfg(target_os = "macos")]
pub fn disable_automatic_tabbing<R: Runtime>(app: &AppHandle<R>) {
    let result = app.run_on_main_thread(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
        }
    });
    if let Err(e) = result {
        eprintln!("tally: window tabbing: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn disable_automatic_tabbing<R: Runtime>(_app: &AppHandle<R>) {}
