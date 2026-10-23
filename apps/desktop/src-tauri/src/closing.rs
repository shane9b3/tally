//! Unsaved-changes guard for closing tabs and windows and for quitting.
//!
//! Each document webview owns its dirty state (the engine's
//! `WorkbookInfo.dirty` as kept in that webview's doc store, plus uncommitted
//! typing) and reports every change through
//! `set_document_edited(edited, doc, title)`. The shell keeps it per tab
//! label in the [`Windows`] registry (`registry.rs`), behind a leaf lock
//! that is never held across an engine call, a dialog or a window/menu call
//! and never touches a document's engine mutex: `WindowEvent::CloseRequested` and
//! `RunEvent::ExitRequested` are handled on the main thread, which must not
//! wait for an engine call in progress.
//!
//! Closing one edited document (⌘W, its tab's ×, the red button of a
//! one-tab window) sends `close-requested { kind: "window" }` to *that*
//! document's webview (shown first), which asks Save / Don't Save / Cancel
//! (packages/app/src/unsaved.ts). After Save or Don't Save it calls
//! `confirm_close(false)`: the tab is marked confirmed and closed (its
//! window too when it was the last tab). Cancel leaves things as they are.
//!
//! Sequences ask several edited tabs one at a time
//! ([`crate::registry::Registry::begin_quit`] and friends): Quit (the app
//! menu's Quit, ⌘Q, or an exit request while documents are open; every
//! window, the front one first), the red button of a window with several
//! tabs, and Close Other Tabs. The shell brings the tab's window to the
//! front, shows the tab and sends it `close-requested { kind: "quit" }` (or
//! `"tabs"`). Save or Don't Save there calls `confirm_close(true)` and the
//! shell moves on to the next edited tab; once none is left it exits, closes
//! the window, or closes the tabs. Clean tabs are never asked. Cancel (or a
//! cancelled Save As) calls `cancel_quit()`, which aborts the whole sequence
//! and forgets the earlier answers; Don't Save is only applied when the
//! sequence completes.
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
//! A closed tab (or every tab of a destroyed window) closes its document in
//! the engine (and removes its autosave snapshot, as the in-app Close did)
//! and destroys the print windows it opened. With no document left, macOS
//! keeps running (File ▸ New / Open and a Dock click open a new window);
//! Windows and Linux exit.

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, Webview};

use tally_api::DocId;

use crate::autosave;
use crate::commands::CmdResult;
use crate::registry::{
    exit_action, is_win_label, CloseKind, ExitAction, Goal, Gone, Step, TabClose, WindowClose,
    Windows,
};
use crate::state::AppState;
use crate::windows;

/// Event sent to a document webview when its close or a sequence waits for the guard.
pub const CLOSE_REQUESTED: &str = "close-requested";
/// Menu id of the app menu's Quit item (handled here, not forwarded).
pub const QUIT_ID: &str = "app-quit";
/// The platform keeps the app running with no window (macOS).
pub const KEEP_ALIVE: bool = cfg!(target_os = "macos");

/// Payload of [`CLOSE_REQUESTED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CloseRequestedEvent {
    pub kind: CloseKind,
}

/// How long the exit snapshots wait (together) to get the engine lock
/// before giving up. This bounds only the wait for the lock, not each
/// `save_copy`, which runs to completion once the lock is held.
const EXIT_LOCK_WAIT: Duration = Duration::from_millis(1500);

/// Sends `close-requested` to one document webview (for a sequence, its
/// window comes to the front and the tab is shown first, so the user sees
/// which document is asking).
pub fn ask<R: Runtime>(app: &AppHandle<R>, label: &str, kind: CloseKind) {
    if kind != CloseKind::Window {
        windows::reveal(app, label);
    }
    if let Err(e) = app.emit_to(label, CLOSE_REQUESTED, CloseRequestedEvent { kind }) {
        eprintln!("tally: emitting {CLOSE_REQUESTED} to {label}: {e}");
    }
}

/// Carries out a step of a sequence.
pub fn run_step<R: Runtime>(app: &AppHandle<R>, step: Step) {
    match step {
        Step::Ask(label, kind) => ask(app, &label, kind),
        Step::Done(Goal::Exit) => app.exit(0),
        Step::Done(Goal::CloseWindow(win)) => {
            if let Some(w) = app.get_window(&win) {
                if let Err(e) = w.destroy() {
                    eprintln!("tally: closing the window: {e}");
                }
            }
        }
        Step::Done(Goal::CloseTabs(tabs)) => {
            // Re-checked tab by tab against the current state: a tab closed
            // or edited again meanwhile is skipped, never closed unasked.
            for label in tabs {
                match app.state::<Windows>().with(|r| r.closable(&label)) {
                    Some(TabClose::Now) => windows::close_tab_now(app, &label),
                    Some(TabClose::Window(win)) => {
                        if let Some(w) = app.get_window(&win) {
                            let _ = w.destroy();
                        }
                    }
                    Some(TabClose::Ask) | None => {}
                }
            }
        }
    }
}

/// After tabs went away: their print windows close, their documents close
/// in the engine, and a sequence that was asking one of them goes on.
pub fn clean_up<R: Runtime>(app: &AppHandle<R>, gone: Gone) {
    for print in &gone.prints {
        if let Some(w) = app.get_webview_window(print) {
            if let Err(e) = w.destroy() {
                eprintln!("tally: closing window {print}: {e}");
            }
        }
    }
    for doc in gone.close_docs {
        // The snapshot goes now (files only), so an exit right after (last
        // window on Windows/Linux, ⌘Q) cannot leave it to be offered for
        // recovery; the engine close needs the document's lock and runs on a
        // worker.
        autosave::remove_snapshot(app, doc);
        let app = app.clone();
        tauri::async_runtime::spawn(async move { windows::close_doc(&app, doc).await });
    }
    if let Some(step) = gone.step {
        run_step(app, step);
    }
}

/// `on_window_event` hook: guards closing a document window, tracks focus
/// (menu routing, the document list) and cleans up after a destroyed window.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let label = window.label();
    let app = window.app_handle();
    let windows = app.state::<Windows>();
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } if is_win_label(label) => {
            match windows.with(|r| r.begin_close_window(label)) {
                WindowClose::Proceed => {}
                WindowClose::Busy => {
                    api.prevent_close();
                    // Bring the pending question forward again.
                    if let Some(step) = windows.with(|r| r.reask()) {
                        run_step(app, step);
                    }
                }
                WindowClose::Hold(step) => {
                    api.prevent_close();
                    run_step(app, step);
                }
            }
        }
        tauri::WindowEvent::Focused(focused) => {
            if windows.with(|r| r.focus_changed(label, *focused)) {
                windows::refresh_window_menu(app);
            }
        }
        tauri::WindowEvent::Destroyed => {
            let gone = windows.with(|r| r.window_destroyed(label));
            clean_up(app, gone);
            if is_win_label(label) {
                // The other windows' "Open" lists lose these documents.
                windows::notify_tabs(app);
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
                    if let Step::Ask(label, kind) = windows.with(|r| r.begin_quit()) {
                        api.prevent_exit();
                        ask(app, &label, kind);
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
        autosave::try_with_doc(st, doc, deadline, save)
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
    run_step(app, step);
}

/// `set_document_edited(edited, doc, title)`: the calling tab reports its
/// dirty state, its document's id (tab ↔ document, the exit snapshot) and
/// its title (document lists, window title, Window menu); on macOS its window's
/// "edited" dot follows. A synchronous command on purpose: Tauri runs those
/// on the main thread in the order sent, so rapid true/false/true reports
/// cannot land out of order (an async command is its own task). It never
/// touches a document's engine mutex, and `run_on_main_thread` runs inline when
/// already on the main thread.
#[tauri::command]
pub fn set_document_edited(
    app: AppHandle,
    webview: Webview,
    edited: bool,
    doc: Option<DocId>,
    title: Option<String>,
) -> CmdResult<()> {
    let label = webview.label();
    windows::require_doc_window(label)?;
    if let Some(t) = &title {
        crate::limits::text_max("the title", t, crate::limits::MAX_PATH)?;
    }
    let state = app.state::<Windows>();
    let changed = state
        .with(|r| r.report(label, edited, doc, title.as_deref()))
        .map_err(|why| windows::refused(&app, why))?;
    if changed {
        if let Some(win) = state.with(|r| r.window_of_tab(label)) {
            windows::update_window(&app, &win);
            windows::notify_tabs(&app);
        }
        windows::refresh_window_menu(&app);
    }
    Ok(())
}

/// `confirm_close(quit)`: the calling tab's prompt resolved with Save or
/// Don't Save. `quit: false` closes the tab (its document is closed in the
/// engine; the last tab closes its window); `quit: true` (a sequence step)
/// moves on to the next edited tab, or finishes the sequence.
#[tauri::command]
pub async fn confirm_close(app: AppHandle, webview: Webview, quit: bool) -> CmdResult<()> {
    let label = webview.label().to_owned();
    windows::require_doc_window(&label)?;
    let state = app.state::<Windows>();
    if quit {
        // Only the tab the sequence is asking can answer for it.
        if let Some(step) = state.with(|r| r.answered(&label)) {
            run_step(&app, step);
        }
    } else {
        state.with(|r| r.confirm(&label));
        windows::request_tab_close(&app, &label);
    }
    Ok(())
}

/// `cancel_quit()`: Cancel (or a cancelled Save As) in the tab a sequence
/// is asking: the sequence is over, nothing is closed. Ignored from a tab
/// the sequence is not asking.
#[tauri::command]
pub fn cancel_quit(webview: Webview, windows: State<'_, Windows>) -> CmdResult<()> {
    windows::require_doc_window(webview.label())?;
    windows.with(|r| r.cancel(webview.label()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_payload_is_camel_case() {
        for (kind, json) in [
            (CloseKind::Quit, r#"{"kind":"quit"}"#),
            (CloseKind::Window, r#"{"kind":"window"}"#),
            (CloseKind::Tabs, r#"{"kind":"tabs"}"#),
        ] {
            let got = serde_json::to_string(&CloseRequestedEvent { kind }).unwrap_or_default();
            assert_eq!(got, json);
        }
    }

    #[test]
    fn only_macos_keeps_running_without_windows() {
        assert_eq!(KEEP_ALIVE, cfg!(target_os = "macos"));
    }
}
