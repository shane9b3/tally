//! Unsaved-changes guard for closing the window and quitting, and the macOS
//! "document edited" dot on the red close button.
//!
//! The webview owns the dirty state (the engine's `WorkbookInfo.dirty` as
//! kept in the app's doc store) and reports every change through
//! `set_document_edited(edited)`. That flag lives here, in two atomics, and
//! never behind the service mutex: `WindowEvent::CloseRequested` and
//! `RunEvent::ExitRequested` are handled on the main thread, which must not
//! wait for an engine call in progress.
//!
//! While the document is edited, a close or quit request is prevented and
//! `close-requested { kind: "window" | "quit" }` is sent to the webview,
//! which asks Save / Don't Save / Cancel (packages/app/src/unsaved.ts).
//! After Save or Don't Save it calls `confirm_close(quit)`: the flag
//! `confirmed` is set first, so the `destroy()` / `exit(0)` that follows
//! (and the `ExitRequested` it triggers) goes through instead of looping.
//! Cancel just leaves things as they are.
//!
//! The app menu's Quit is a custom item ([`QUIT_ID`]) rather than the
//! predefined one: the predefined item sends `-[NSApp terminate:]`, which
//! tao does not route through `ExitRequested`, so it could not be stopped.
//! (The Dock's Quit and log-out still use `terminate:`; the autosave
//! snapshot stays on disk in that case and is offered on the next launch.)

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use crate::commands::CmdResult;

/// Event sent to the webview when a close or quit waits for the guard.
pub const CLOSE_REQUESTED: &str = "close-requested";
/// Menu id of the app menu's Quit item (handled here, not forwarded).
pub const QUIT_ID: &str = "app-quit";
/// The main window's label (`tauri.conf.json`).
const MAIN: &str = "main";

/// What is being closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseKind {
    /// The main window (red button, Windows title bar ✕, Alt+F4).
    Window,
    /// The whole app (⌘Q / app menu Quit, or an exit request).
    Quit,
}

/// Payload of [`CLOSE_REQUESTED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CloseRequestedEvent {
    pub kind: CloseKind,
}

/// The shell's answer to a close or quit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Close / exit now.
    Proceed,
    /// Prevent it and let the webview ask about the unsaved changes.
    Ask,
}

/// Pure decision: ask only when the document has unsaved changes, the user
/// has not already answered the prompt (`confirmed`), and a webview is
/// there to ask (`ui_present`; with no window left, nothing could answer).
pub fn decide(edited: bool, confirmed: bool, ui_present: bool) -> Decision {
    if edited && !confirmed && ui_present {
        Decision::Ask
    } else {
        Decision::Proceed
    }
}

/// Managed close/quit state.
#[derive(Debug, Default)]
pub struct CloseState {
    edited: AtomicBool,
    confirmed: AtomicBool,
}

impl CloseState {
    /// Records the document's dirty state (`set_document_edited`).
    pub fn set_edited(&self, edited: bool) {
        self.edited.store(edited, Ordering::SeqCst);
    }

    /// Whether the document has unsaved changes.
    pub fn edited(&self) -> bool {
        self.edited.load(Ordering::SeqCst)
    }

    /// The user answered Save / Don't Save: the next close or exit goes through.
    pub fn confirm(&self) {
        self.confirmed.store(true, Ordering::SeqCst);
    }

    /// `WindowEvent::CloseRequested` of the main window.
    pub fn on_close_requested(&self) -> Decision {
        decide(self.edited(), self.confirmed.load(Ordering::SeqCst), true)
    }

    /// `RunEvent::ExitRequested` (`code` is `None` for user-initiated exits).
    pub fn on_exit_requested(&self, code: Option<i32>, ui_present: bool) -> Decision {
        if code == Some(tauri::RESTART_EXIT_CODE) {
            return Decision::Proceed;
        }
        decide(
            self.edited(),
            self.confirmed.load(Ordering::SeqCst),
            ui_present,
        )
    }
}

fn ask<R: Runtime>(app: &AppHandle<R>, kind: CloseKind) {
    if let Err(e) = app.emit_to(MAIN, CLOSE_REQUESTED, CloseRequestedEvent { kind }) {
        eprintln!("tally: emitting {CLOSE_REQUESTED}: {e}");
    }
}

/// `on_window_event` hook: guards closing the main window.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if window.label() != MAIN {
            return;
        }
        let state = window.state::<CloseState>();
        if state.on_close_requested() == Decision::Ask {
            api.prevent_close();
            ask(window.app_handle(), CloseKind::Window);
        }
    }
}

/// `App::run` hook: guards exit requests.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &tauri::RunEvent) {
    if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
        let ui_present = app.get_webview_window(MAIN).is_some();
        let state = app.state::<CloseState>();
        if state.on_exit_requested(*code, ui_present) == Decision::Ask {
            api.prevent_exit();
            ask(app, CloseKind::Quit);
        }
    }
}

/// The app menu's Quit ([`QUIT_ID`]).
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    let ui_present = app.get_webview_window(MAIN).is_some();
    if app
        .state::<CloseState>()
        .on_exit_requested(None, ui_present)
        == Decision::Ask
    {
        ask(app, CloseKind::Quit);
    } else {
        app.exit(0);
    }
}

/// `set_document_edited(edited)`: the webview reports the dirty state; on
/// macOS it also sets the window's "document edited" dot (no-op elsewhere).
#[tauri::command]
pub async fn set_document_edited(app: AppHandle, edited: bool) -> CmdResult<()> {
    app.state::<CloseState>().set_edited(edited);
    if let Some(window) = app.get_webview_window(MAIN) {
        set_native_edited(&window, edited);
    }
    Ok(())
}

/// `confirm_close(quit)`: the guard resolved with Save or Don't Save; close
/// the main window (`quit: false`) or exit the app (`quit: true`) for real.
#[tauri::command]
pub async fn confirm_close(app: AppHandle, quit: bool) -> CmdResult<()> {
    app.state::<CloseState>().confirm();
    match app.get_webview_window(MAIN) {
        Some(window) if !quit => {
            if let Err(e) = window.destroy() {
                eprintln!("tally: closing the window: {e}");
                app.exit(0);
            }
        }
        _ => app.exit(0),
    }
    Ok(())
}

/// Sets `-[NSWindow setDocumentEdited:]` on the main thread through the safe
/// `objc2-app-kit` binding (the same call tao's own
/// `WindowExtMacOS::set_is_document_edited` makes; Tauri 2.12 does not
/// expose tao's window). The `NSWindow` is found among `NSApp.windows` by
/// address, so no raw pointer is dereferenced and no `unsafe` is needed.
#[cfg(target_os = "macos")]
fn set_native_edited<R: Runtime>(window: &WebviewWindow<R>, edited: bool) {
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
                if w.isDocumentEdited() != edited {
                    w.setDocumentEdited(edited);
                }
                return;
            }
        }
    });
    if let Err(e) = result {
        eprintln!("tally: document edited: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn set_native_edited<R: Runtime>(_window: &WebviewWindow<R>, _edited: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_only_when_edited_unconfirmed_and_a_window_can_answer() {
        assert_eq!(decide(false, false, true), Decision::Proceed);
        assert_eq!(decide(true, false, true), Decision::Ask);
        assert_eq!(decide(true, true, true), Decision::Proceed);
        assert_eq!(decide(true, false, false), Decision::Proceed);
        assert_eq!(decide(false, true, false), Decision::Proceed);
    }

    #[test]
    fn clean_document_closes_and_quits_immediately() {
        let s = CloseState::default();
        assert_eq!(s.on_close_requested(), Decision::Proceed);
        assert_eq!(s.on_exit_requested(None, true), Decision::Proceed);
        assert_eq!(s.on_exit_requested(Some(0), true), Decision::Proceed);
    }

    #[test]
    fn edited_document_is_guarded_until_confirmed() {
        let s = CloseState::default();
        s.set_edited(true);
        assert_eq!(s.on_close_requested(), Decision::Ask);
        assert_eq!(s.on_exit_requested(None, true), Decision::Ask);
        // Cancel changes nothing: still guarded on the next request.
        assert_eq!(s.on_close_requested(), Decision::Ask);
        // Saving clears the flag.
        s.set_edited(false);
        assert_eq!(s.on_close_requested(), Decision::Proceed);
        s.set_edited(true);
        // Don't Save / Save confirmed: the destroy()/exit(0) that follows is not re-guarded (no loop).
        s.confirm();
        assert_eq!(s.on_close_requested(), Decision::Proceed);
        assert_eq!(s.on_exit_requested(Some(0), true), Decision::Proceed);
        assert_eq!(s.on_exit_requested(None, false), Decision::Proceed);
    }

    #[test]
    fn exit_without_a_window_or_on_restart_is_not_blocked() {
        let s = CloseState::default();
        s.set_edited(true);
        assert_eq!(s.on_exit_requested(None, false), Decision::Proceed);
        assert_eq!(
            s.on_exit_requested(Some(tauri::RESTART_EXIT_CODE), true),
            Decision::Proceed
        );
    }

    #[test]
    fn event_payload_is_camel_case() {
        let json = serde_json::to_string(&CloseRequestedEvent {
            kind: CloseKind::Quit,
        })
        .unwrap_or_default();
        assert_eq!(json, r#"{"kind":"quit"}"#);
        let json = serde_json::to_string(&CloseRequestedEvent {
            kind: CloseKind::Window,
        })
        .unwrap_or_default();
        assert_eq!(json, r#"{"kind":"window"}"#);
    }
}
