//! Unsaved-changes guard for closing the window and quitting, and the macOS
//! "document edited" dot on the red close button.
//!
//! The webview owns the dirty state (the engine's `WorkbookInfo.dirty` as
//! kept in the app's doc store) and reports every change through
//! `set_document_edited(edited, doc)`. That flag (and the document's id)
//! lives here, in atomics, and never behind the service mutex: `WindowEvent::CloseRequested` and
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
//! (The Dock's Quit and log-out still use `terminate:`, which cannot be
//! vetoed. tao routes it through `applicationWillTerminate` to
//! `RunEvent::Exit`, where [`exit_snapshot`] writes a last autosave
//! snapshot of an edited, unconfirmed document; it is offered on the next
//! launch.)
//!
//! When the main window is destroyed, the app is over: every other webview
//! window (a File ▸ Print preview) is destroyed and the app exits, so no
//! orphan window keeps the process alive.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use tally_api::DocId;

use crate::autosave;
use crate::commands::CmdResult;
use crate::state::AppState;

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

/// Pure: whether the exit needs a last snapshot, and of which document: the
/// document is edited, the user did not answer Save / Don't Save (a
/// confirmed exit means the changes were saved or deliberately dropped) and
/// the shell knows which document it is.
pub fn exit_snapshot_target(edited: bool, confirmed: bool, doc: Option<DocId>) -> Option<DocId> {
    if edited && !confirmed {
        doc
    } else {
        None
    }
}

/// Pure: destroying this window ends the app (the main window; a print
/// window closing leaves the main window alone).
pub fn ends_app(label: &str) -> bool {
    label == MAIN
}

/// How long the exit snapshot waits to get the engine lock before giving up.
/// This bounds only the wait for the lock, not the `save_copy` itself, which
/// runs to completion once the lock is held.
const EXIT_LOCK_WAIT: Duration = Duration::from_millis(1500);

/// Bit of the packed report that holds the edited flag (above the doc id).
const EDITED_BIT: u64 = 1 << 40;

/// `doc` stored in an atomic: `0` is none, otherwise the id plus one.
fn pack_doc(doc: Option<DocId>) -> u64 {
    doc.map_or(0, |d| u64::from(d.0) + 1)
}

fn unpack_doc(v: u64) -> Option<DocId> {
    (v & (EDITED_BIT - 1))
        .checked_sub(1)
        .and_then(|id| u32::try_from(id).ok())
        .map(DocId)
}

/// The edited flag and the document id as one value, so a report is stored
/// and read as a unit.
fn pack_report(edited: bool, doc: Option<DocId>) -> u64 {
    pack_doc(doc) | if edited { EDITED_BIT } else { 0 }
}

/// Managed close/quit state.
#[derive(Debug, Default)]
pub struct CloseState {
    /// The last report: edited flag and document id ([`pack_report`]); the
    /// id is for the exit snapshot.
    report: AtomicU64,
    confirmed: AtomicBool,
    /// The main window was destroyed: nothing is left to ask.
    main_gone: AtomicBool,
}

impl CloseState {
    /// Records the document's dirty state and id (`set_document_edited`).
    pub fn set_edited(&self, edited: bool, doc: Option<DocId>) {
        self.report
            .store(pack_report(edited, doc), Ordering::SeqCst);
    }

    /// The document to snapshot at exit, if any ([`exit_snapshot_target`]).
    pub fn exit_target(&self) -> Option<DocId> {
        let report = self.report.load(Ordering::SeqCst);
        exit_snapshot_target(
            report & EDITED_BIT != 0,
            self.confirmed.load(Ordering::SeqCst),
            unpack_doc(report),
        )
    }

    /// The main window was destroyed.
    pub fn mark_main_gone(&self) {
        self.main_gone.store(true, Ordering::SeqCst);
    }

    /// Whether the main window was destroyed.
    pub fn main_gone(&self) -> bool {
        self.main_gone.load(Ordering::SeqCst)
    }

    /// Whether the document has unsaved changes.
    pub fn edited(&self) -> bool {
        self.report.load(Ordering::SeqCst) & EDITED_BIT != 0
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

/// Whether the main webview is still there to answer a prompt.
fn ui_present<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(MAIN).is_some() && !app.state::<CloseState>().main_gone()
}

/// `on_window_event` hook: guards closing the main window, and ends the app
/// once it is gone.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            if window.label() != MAIN {
                return;
            }
            let state = window.state::<CloseState>();
            if state.on_close_requested() == Decision::Ask {
                api.prevent_close();
                ask(window.app_handle(), CloseKind::Window);
            }
        }
        tauri::WindowEvent::Destroyed if ends_app(window.label()) => {
            // The main window is gone (red button on a clean document, or
            // Don't Save): destroy the print windows too and exit. `main_gone`
            // makes the exit request proceed instead of asking a dead webview.
            let app = window.app_handle();
            app.state::<CloseState>().mark_main_gone();
            for (label, other) in app.webview_windows() {
                if !ends_app(&label) {
                    if let Err(e) = other.destroy() {
                        eprintln!("tally: closing window {label}: {e}");
                    }
                }
            }
            app.exit(0);
        }
        _ => {}
    }
}

/// `App::run` hook: guards exit requests.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            let state = app.state::<CloseState>();
            if state.on_exit_requested(*code, ui_present(app)) == Decision::Ask {
                api.prevent_exit();
                ask(app, CloseKind::Quit);
            }
        }
        tauri::RunEvent::Exit => exit_snapshot(app),
        _ => {}
    }
}

/// `RunEvent::Exit` (tao's `applicationWillTerminate`, so also the Dock's
/// Quit, log-out and shutdown, which cannot be vetoed): writes a last
/// autosave snapshot of an edited document the user did not answer for.
/// The engine lock is only tried for a short while ([`EXIT_LOCK_WAIT`]; the
/// save itself is not time-bounded) so a hung engine cannot hang the
/// shutdown. Only a document the engine itself reports dirty is written: the
/// webview's flag also covers uncommitted typing, which a snapshot cannot
/// contain. Failures are logged, never fatal.
fn exit_snapshot<R: Runtime>(app: &AppHandle<R>) {
    let Some(doc) = app.state::<CloseState>().exit_target() else {
        return;
    };
    let st = app.state::<AppState>();
    let deadline = Instant::now() + EXIT_LOCK_WAIT;
    match autosave::write_snapshot_by(app, &st, doc, true, |save| {
        autosave::try_with_service(&st, deadline, save)
    }) {
        Ok(_) => {}
        Err(e) => eprintln!("tally: exit snapshot skipped: {}", e.message),
    }
}

/// The app menu's Quit ([`QUIT_ID`]).
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    if app
        .state::<CloseState>()
        .on_exit_requested(None, ui_present(app))
        == Decision::Ask
    {
        ask(app, CloseKind::Quit);
    } else {
        app.exit(0);
    }
}

/// `set_document_edited(edited, doc)`: the webview reports the dirty state
/// and the document's id (for the exit snapshot); on macOS it also sets the
/// window's "document edited" dot (no-op elsewhere). A synchronous command
/// on purpose: Tauri runs those on the main thread in the order sent, so
/// rapid true/false/true reports cannot land out of order (an async command
/// is its own task). It never touches the service mutex, and
/// `run_on_main_thread` runs inline when already on the main thread.
#[tauri::command]
pub fn set_document_edited(app: AppHandle, edited: bool, doc: Option<DocId>) -> CmdResult<()> {
    app.state::<CloseState>().set_edited(edited, doc);
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
        s.set_edited(true, None);
        assert_eq!(s.on_close_requested(), Decision::Ask);
        assert_eq!(s.on_exit_requested(None, true), Decision::Ask);
        // Cancel changes nothing: still guarded on the next request.
        assert_eq!(s.on_close_requested(), Decision::Ask);
        // Saving clears the flag.
        s.set_edited(false, None);
        assert_eq!(s.on_close_requested(), Decision::Proceed);
        s.set_edited(true, None);
        // Don't Save / Save confirmed: the destroy()/exit(0) that follows is not re-guarded (no loop).
        s.confirm();
        assert_eq!(s.on_close_requested(), Decision::Proceed);
        assert_eq!(s.on_exit_requested(Some(0), true), Decision::Proceed);
        assert_eq!(s.on_exit_requested(None, false), Decision::Proceed);
    }

    #[test]
    fn exit_without_a_window_or_on_restart_is_not_blocked() {
        let s = CloseState::default();
        s.set_edited(true, None);
        assert_eq!(s.on_exit_requested(None, false), Decision::Proceed);
        assert_eq!(
            s.on_exit_requested(Some(tauri::RESTART_EXIT_CODE), true),
            Decision::Proceed
        );
    }

    #[test]
    fn main_window_destruction_ends_the_app() {
        assert!(ends_app("main"));
        assert!(!ends_app("tally-print-3"));
        // The exit that follows must not re-ask, even for an edited document.
        let s = CloseState::default();
        s.set_edited(true, Some(DocId(1)));
        s.mark_main_gone();
        assert!(s.main_gone());
        assert_eq!(s.on_exit_requested(Some(0), false), Decision::Proceed);
    }

    #[test]
    fn exit_snapshot_only_for_edited_unconfirmed_documents() {
        let d = Some(DocId(4));
        assert_eq!(exit_snapshot_target(true, false, d), d);
        assert_eq!(exit_snapshot_target(false, false, d), None);
        assert_eq!(exit_snapshot_target(true, true, d), None);
        assert_eq!(exit_snapshot_target(true, false, None), None);
        let s = CloseState::default();
        assert_eq!(s.exit_target(), None);
        s.set_edited(true, Some(DocId(0)));
        assert_eq!(s.exit_target(), Some(DocId(0)));
        s.confirm();
        assert_eq!(s.exit_target(), None);
    }

    #[test]
    fn doc_ids_round_trip_through_the_atomic() {
        for d in [None, Some(DocId(0)), Some(DocId(7)), Some(DocId(u32::MAX))] {
            for edited in [false, true] {
                let r = pack_report(edited, d);
                assert_eq!(unpack_doc(r), d);
                assert_eq!(r & EDITED_BIT != 0, edited);
            }
        }
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
