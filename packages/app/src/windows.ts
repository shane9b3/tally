/**
 * One webview per document (desktop shell, `apps/desktop/src-tauri/src/windows.rs`),
 * shown as a tab of a window or in a window of its own (tabs.ts; Tools ▸
 * Options ▸ "Open documents in").
 *
 * Each document runs its own webview with its own stores and
 * `AppController`. On start the controller asks `window_init()` for the
 * document the window shows (the engine's summary, also after a reload;
 * `null`: create a blank workbook) and
 * whether it is the session's first window (only that one offers crash
 * recovery). New, Open, Open Recent, templates and recovery create the
 * document in the engine and then call `place_document(doc, reuse)`: it
 * replaces this window's document only when that is an untitled blank
 * workbook nobody has touched (the macOS convention; {@link canReuseWindow}),
 * otherwise it opens in a new tab or window. File ▸ New always opens a new
 * tab or window.
 *
 * The browser build (mock engine, e2e) has no window shell: there the old
 * single-document behaviour stays (replace the document after the
 * unsaved-changes guard).
 */
import type { DocId, WorkbookInfo } from "@tally/bridge";
import type { TabsState } from "@tally/bridge/tauri";

/** The theme preference sent with `prefs-changed`. */
export type ThemeChoice = "system" | "light" | "dark";

/** Optional desktop-shell hooks for document windows (`TauriClient`; the browser has none). */
export interface WindowShell {
  /** `window_init`: the document handed to this webview, whether it offers recovery, its own label and its window's tabs. */
  windowInit?(): Promise<{ doc: WorkbookInfo | null; offerRecovery: boolean; label?: string; tabs?: TabsState | null; ask?: "window" | "quit" | "tabs" | null; openPaths?: string[] }>;
  /**
   * `place_document`: show document `doc` here (`"here"`), as a new tab of
   * this window (`"tab"`) or in a new window (`"window"`), per the "Open
   * documents in" setting; `null` opens a blank document the same way.
   */
  placeDocument?(doc: DocId | null, reuse: boolean): Promise<"here" | "tab" | "window">;
  /** `broadcast_prefs`: the prefs (or the theme) changed in this window. */
  broadcastPrefs?(theme: ThemeChoice | null): Promise<void>;
  /** `prefs-changed`: another window changed the prefs or the theme. */
  onPrefsChanged?(cb: (event: { theme: ThemeChoice | null }) => void): () => void;
}

/** What {@link canReuseWindow} looks at. */
export interface ReuseState {
  /** The window's document. */
  info: Pick<WorkbookInfo, "doc" | "path" | "dirty"> | null;
  /** The blank workbook this window started with (cleared on the first change). */
  pristineDoc: DocId | null;
  /** Unsaved changes, including uncommitted typing. */
  edited: boolean;
}

/**
 * Pure: a document opened from this window may replace its current one only
 * when that is the blank workbook the window started with, never saved
 * (no path), never changed and not being typed into.
 */
export function canReuseWindow(s: ReuseState): boolean {
  const info = s.info;
  return info !== null && info.doc === s.pristineDoc && (info.path ?? null) === null && !info.dirty && !s.edited;
}
