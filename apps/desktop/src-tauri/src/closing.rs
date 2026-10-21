//! Unsaved-changes guard for closing document windows and quitting, and the
//! macOS "document edited" dot on each window's red close button.
//!
//! Each document window's webview owns its dirty state (the engine's
//! `WorkbookInfo.dirty` as kept in that webview's doc store, plus uncommitted
//! typing) and reports every change through
//! `set_document_edited(edited, doc, title)`. The shell keeps it per window
//! label in the [`Windows`] registry (`registry.rs`), behind a leaf lock
//! that is never held across an engine call, a dialog or a window/menu call
//! and never touches the service mutex: `WindowEvent::CloseRequested` and
//! `RunEvent::ExitRequested` are handled on the main thread, which must not
//! wait for an engine call in progress.
//!
//! Closing an edited window is prevented and `close-requested { kind:
//! "window" }` is sent to *that* window, which asks Save / Don't Save /
//! Cancel (packages/app/src/unsaved.ts). After Save or Don't Save it calls
//! `confirm_close(false)`: the window is marked confirmed and destroyed.
//! Cancel leaves things as they are.
//!
//! Quit (the app menu's Quit, ⌘Q, or an exit request while windows are
//! open) asks the edited windows one at a time, the front one first
//! ([`crate::registry::Registry::begin_quit`]): the shell focuses the window
//! and sends it `close-requested { kind: "quit" }`. Save or Don't Save there
//! calls `confirm_close(true)` and the shell moves on to the next edited
//! window, or exits when none is left; clean windows are never asked. Cancel
//! (or a cancelled Save As) in any window calls `cancel_quit()`, which aborts
//! the whole quit and forgets the earlier answers.
//!
//! The app menu's Quit is a custom item ([`QUIT_ID`]) rather than the
//! predefined one: the predefined item sends `-[NSApp terminate:]`, which
//! tao does not route through `ExitRequested`, so it could not be stopped.
//! (The Dock's Quit and log-out still use `terminate:`, which cannot be
//! vetoed. tao routes it through `applicationWillTerminate` to
//! `RunEvent::Exit`, where [`exit_snapshot`] writes a last autosave
//! snapshot of every edited, unanswered document; they are offered on the
//! next launch.)
//!
//! A destroyed document window closes its document in the engine (and
//! removes its autosave snapshot, as the in-app Close did) and destroys the
//! print windows it opened. With no document window left, macOS keeps
//! running (File ▸ New / Open and a Dock click open a new window); Windows
//! and Linux exit.

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

use tally_api::DocId;

use crate::autosave;
use crate::commands::CmdResult;
use crate::registry::{exit_action, is_doc_label, ExitAction, QuitStep, Windows};
use crate::state::AppState;
use crate::windows;

/// Event sent to a document window when its close or a quit waits for the guard.
pub const CLOSE_REQUESTED: &str = "close-requested";
/// Menu id of the app menu's Quit item (handled here, not forwarded).
pub const QUIT_ID: &str = "app-quit";
/// The platform keeps the app running with no window (macOS).
pub const KEEP_ALIVE: bool = cfg!(target_os = "macos");

/// What is being closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseKind {
    /// One document window (red button, ⌘W, Windows title bar ✕, Alt+F4).
    Window,
    /// The whole app (⌘Q / app menu Quit, or an exit request).
    Quit,
}

/// Payload of [`CLOSE_REQUESTED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CloseRequestedEvent {
    pub kind: CloseKind,
}

/// How long the exit snapshots wait (together) to get the engine lock
/// before giving up. This bounds only the wait for the lock, not each
/// `save_copy`, which runs to completion once the lock is held.
const EXIT_LOCK_WAIT: Duration = Duration::from_millis(1500);

/// Sends `close-requested` to one window (focusing it first for a quit, so
/// the user sees which document is asking).
fn ask<R: Runtime>(app: &AppHandle<R>, label: &str, kind: CloseKind) {
    if kind == CloseKind::Quit {
        windows::focus(app, label);
    }
    if let Err(e) = app.emit_to(label, CLOSE_REQUESTED, CloseRequestedEvent { kind }) {
        eprintln!("tally: emitting {CLOSE_REQUESTED} to {label}: {e}");
    }
}

/// Carries out a step of the quit sequence.
fn run_quit_step<R: Runtime>(app: &AppHandle<R>, step: QuitStep) {
    match step {
        QuitStep::Ask(label) => ask(app, &label, CloseKind::Quit),
        QuitStep::Exit => app.exit(0),
    }
}

/// `on_window_event` hook: guards closing a document window, tracks focus
/// (menu routing, the window list) and cleans up after a destroyed window.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let label = window.label();
    let app = window.app_handle();
    let windows = app.state::<Windows>();
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } if is_doc_label(label) => {
            if windows.with(|r| r.must_ask(label)) {
                api.prevent_close();
                ask(app, label, CloseKind::Window);
            }
        }
        tauri::WindowEvent::Focused(focused) => {
            if windows.with(|r| r.focus_changed(label, *focused)) {
                windows::refresh_window_menu(app);
            }
        }
        tauri::WindowEvent::Destroyed => {
            let gone = windows.with(|r| r.destroyed(label));
            for print in &gone.prints {
                if let Some(w) = app.get_webview_window(print) {
                    if let Err(e) = w.destroy() {
                        eprintln!("tally: closing window {print}: {e}");
                    }
                }
            }
            if let Some(doc) = gone.close_doc {
                // The snapshot goes now (files only), so an exit right after
                // (last window on Windows/Linux, ⌘Q) cannot leave it to be
                // offered for recovery; the engine close needs the service
                // lock and runs on a worker.
                autosave::remove_snapshot(app, doc);
                let app = app.clone();
                tauri::async_runtime::spawn(async move { windows::close_doc(&app, doc).await });
            }
            if let Some(step) = gone.quit {
                run_quit_step(app, step);
            }
            if is_doc_label(label) {
                windows::refresh_window_menu(app);
                if !KEEP_ALIVE && windows.with(|r| r.is_empty()) {
                    app.exit(0);
                }
            }
        }
        _ => {}
    }
}

/// `App::run` hook: guards exit requests, writes the exit snapshots and
/// reopens a window from the Dock.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            let windows = app.state::<Windows>();
            let (open, pending) = windows.with(|r| (r.len(), r.pending()));
            match exit_action(*code, KEEP_ALIVE, open, pending) {
                ExitAction::Proceed => {}
                ExitAction::StayAlive => api.prevent_exit(),
                ExitAction::Quit => {
                    if let QuitStep::Ask(label) = windows.with(|r| r.begin_quit()) {
                        api.prevent_exit();
                        ask(app, &label, CloseKind::Quit);
                    }
                }
            }
        }
        tauri::RunEvent::Exit => exit_snapshot(app),
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => windows::reopen(app, *has_visible_windows),
        _ => {}
    }
}

/// `RunEvent::Exit` (tao's `applicationWillTerminate`, so also the Dock's
/// Quit, log-out and shutdown, which cannot be vetoed): writes a last
/// autosave snapshot of every edited document the user did not answer for,
/// and removes the snapshots of the clean and answered ones (as a clean
/// close does). The engine lock is only tried for a short while in total
/// ([`EXIT_LOCK_WAIT`]; a save itself is not time-bounded) so a hung engine
/// cannot hang the shutdown. Only a document the engine itself reports
/// dirty is written: the webview's flag also covers uncommitted typing,
/// which a snapshot cannot contain. Failures are logged, never fatal.
fn exit_snapshot<R: Runtime>(app: &AppHandle<R>) {
    let plan = app.state::<Windows>().with(|r| r.exit_plan());
    for doc in plan.discard {
        autosave::remove_snapshot(app, doc);
    }
    if plan.snapshot.is_empty() {
        return;
    }
    let st = app.state::<AppState>();
    let deadline = Instant::now() + EXIT_LOCK_WAIT;
    for doc in plan.snapshot {
        snapshot_one(app, &st, doc, deadline);
    }
}

fn snapshot_one<R: Runtime>(app: &AppHandle<R>, st: &AppState, doc: DocId, deadline: Instant) {
    if let Err(e) = autosave::write_snapshot_by(app, st, doc, true, |save| {
        autosave::try_with_service(st, deadline, save)
    }) {
        eprintln!(
            "tally: exit snapshot of document {} skipped: {}",
            doc.0, e.message
        );
    }
}

/// The app menu's Quit ([`QUIT_ID`]).
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    let step = app.state::<Windows>().with(|r| r.begin_quit());
    run_quit_step(app, step);
}

/// `set_document_edited(edited, doc, title)`: the calling window reports its
/// dirty state, its document's id (window ↔ document, the exit snapshot) and
/// its title (the Window menu); on macOS it also sets that window's
/// "document edited" dot (no-op elsewhere). A synchronous command on
/// purpose: Tauri runs those on the main thread in the order sent, so rapid
/// true/false/true reports cannot land out of order (an async command is
/// its own task). It never touches the service mutex, and
/// `run_on_main_thread` runs inline when already on the main thread.
#[tauri::command]
pub fn set_document_edited(
    app: AppHandle,
    window: WebviewWindow,
    edited: bool,
    doc: Option<DocId>,
    title: Option<String>,
) -> CmdResult<()> {
    windows::require_doc_window(window.label())?;
    if let Some(t) = &title {
        crate::limits::text_max("the title", t, crate::limits::MAX_PATH)?;
    }
    let renamed = app
        .state::<Windows>()
        .with(|r| r.report(window.label(), edited, doc, title.as_deref()))
        .map_err(|why| windows::refused(&app, why))?;
    set_native_edited(&window, edited);
    if renamed {
        windows::refresh_window_menu(&app);
    }
    Ok(())
}

/// `confirm_close(quit)`: the calling window's prompt resolved with Save or
/// Don't Save. `quit: false` destroys the window (its document is closed
/// in the engine on `Destroyed`); `quit: true` moves the quit on to the next
/// edited window, or exits.
#[tauri::command]
pub async fn confirm_close(app: AppHandle, window: WebviewWindow, quit: bool) -> CmdResult<()> {
    windows::require_doc_window(window.label())?;
    let windows = app.state::<Windows>();
    let label = window.label().to_owned();
    if quit {
        // Only the window the quit is asking can answer for it.
        if let Some(step) = windows.with(|r| r.quit_answered(&label)) {
            run_quit_step(&app, step);
        }
    } else {
        windows.with(|r| r.confirm(&label));
        if let Err(e) = window.destroy() {
            eprintln!("tally: closing the window: {e}");
        }
    }
    Ok(())
}

/// `cancel_quit()`: Cancel (or a cancelled Save As) in the window asked
/// during a quit: the quit is over, the app keeps running. Ignored from a
/// window the quit is not asking.
#[tauri::command]
pub fn cancel_quit(window: WebviewWindow, windows: State<'_, Windows>) -> CmdResult<()> {
    windows::require_doc_window(window.label())?;
    windows.with(|r| r.cancel_quit(window.label()));
    Ok(())
}

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

    #[test]
    fn only_macos_keeps_running_without_windows() {
        assert_eq!(KEEP_ALIVE, cfg!(target_os = "macos"));
    }
}
