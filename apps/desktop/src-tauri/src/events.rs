//! Events sent to the webview.

use tally_api::{ApplyResult, DocChangedEvent, DocId, MenuActionEvent};
use tauri::{AppHandle, Emitter, Runtime};

/// Event name for engine-initiated changes.
pub const DOC_CHANGED: &str = "doc-changed";
/// Event name for native menu selections.
pub const MENU_ACTION: &str = "menu-action";

/// Emits `doc-changed` for a change the UI did not initiate (volatile
/// recalc, async load), to every window (each filters by `doc`). Results of the UI's own apply/undo/redo are return
/// values, not events.
// Used once the engine (WP1) recalculates volatile functions on a timer.
#[allow(dead_code)]
pub fn emit_doc_changed<R: Runtime>(app: &AppHandle<R>, doc: DocId, result: ApplyResult) {
    if let Err(e) = app.emit(DOC_CHANGED, DocChangedEvent { doc, result }) {
        eprintln!("tally: emitting {DOC_CHANGED}: {e}");
    }
}

/// Emits `menu-action { id }` to one document window (`label`): the
/// webviews listen to their own window's events only, so the other windows
/// never see it.
pub fn emit_menu_action<R: Runtime>(app: &AppHandle<R>, label: &str, id: &str) {
    if let Err(e) = app.emit_to(label, MENU_ACTION, MenuActionEvent { id: id.to_owned() }) {
        eprintln!("tally: emitting {MENU_ACTION} to {label}: {e}");
    }
}
