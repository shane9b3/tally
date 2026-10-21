# Manual QA: unsaved-changes guard and the macOS "edited" dot

Since multi-window support, each document has its own window; the per-window
and multi-window checks (quit with several edited windows, tabs, Dock reopen)
are in `docs/qa/multi-window-manual.md`.

The in-app paths are covered automatically (`packages/app/test/unsaved.test.ts`,
`tests/e2e/unsaved.spec.ts`, `apps/desktop/src-tauri/src/closing.rs` and
`menu.rs` unit tests). These checks need the native window and run against
the **release build** (`pnpm --filter tally-desktop tauri build`, then open
`target/release/bundle/macos/Tally.app`). Use a scratch `.ods` saved once
(so it is in Open recent) and keep a second one in the recent list.

| # | Step | Expected |
|---|---|---|
| 1 | Launch, open `A.ods`. Look at the red close button. | No dot (clean). |
| 2 | Type a value into a cell, press Enter. | The red button shows the "edited" dot; the in-app header shows "· Edited". |
| 3 | ⌘Z until the value is gone, then ⌘⇧Z. | The dot follows the engine's dirty flag (shown again after redo). |
| 4 | ⌘S. | Dot disappears. |
| 5 | Edit again, then ⌘W. | Sheet prompt: "Do you want to save the changes you made to “A.ods”?" / "Your changes will be lost if you don't save them.", buttons Don't Save · Cancel · **Save** (default). |
| 6 | Press Escape. | Prompt closes; `A.ods` and the edit are still there; dot still shown. |
| 7 | ⌘W again, choose Save. | `A.ods` is saved, then its window closes. Reopen it from Open recent: no dot. |
| 8 | ⌘N, type a value, then ⌘W. Choose Save. | The Save As dialog opens (untitled). Cancel it: nothing else happens, the untitled document and its value stay. |
| 9 | Repeat 8 but choose Don't Save. | The window closes; the edit is gone. |
| 10 | Edit, then click a template ("Monthly budget") in the sidebar, File ▸ New and File ▸ Open recent ▸ another file. | No prompt: each opens in a new window (or tab) and the edited window keeps its document and dot. |
| 11 | Edit, click the red close button. | The prompt appears in the window; the window stays open. Cancel: still open, dot still shown. |
| 12 | Red close button again → Don't Save. | The window closes; the app keeps running with no window (macOS; Windows/Linux exit). Quit and relaunch: no recovery offer. |
| 13 | Relaunch, edit, ⌘Q (or Tally ▸ Quit Tally). | Prompt; Cancel keeps the app running. ⌘Q → Save on a titled file saves and quits; on an untitled file the Save As dialog opens, and cancelling it keeps the app running. |
| 14 | With a clean document: red button, then relaunch and ⌘Q. | Close / quit immediately, no prompt. |
| 15 | Switch Tools ▸ Options ▸ Language to Deutsch, edit, ⌘Q. | Menu item reads "Tally beenden"; the prompt reads "Möchten Sie die Änderungen an „…“ speichern?" with Nicht speichern / Abbrechen / Speichern. |
| 16 | Windows/Linux build (if available): edit, click the title-bar ✕ / Alt+F4. | Same prompt; Cancel keeps the window. No dot (macOS only). |

| 17 | Choose File ▸ Print so the print window opens. Close nothing; in the document window edit a cell, click the red button, choose Don't Save. | The document window and its print window both close (no orphan print window in Cmd-Tab / the Window menu); macOS keeps the app running with no window. |
| 18 | With a clean document, open the print window again, click the document window's red button. | Both windows close; no prompt. |
| 19 | Edit a cell (do not save), then run `osascript -e 'quit app "Tally"'` (same path as the Dock's Quit). Relaunch. | The app quits without the prompt; recovery offers the document with the edit. |

Known limitation: the Dock's Quit and log-out/shutdown send
`-[NSApp terminate:]` directly, which tao does not let the app veto, so they
quit without the prompt. The shell writes a fresh autosave snapshot of the
edited document on that exit (`RunEvent::Exit`; only if the engine reports unsaved
changes; the wait for the engine lock is capped at about 1.5 s, the write itself
is not; skipped if the engine stays busy, in which case the last periodic snapshot from Tools ▸
Options ▸ AutoRecovery remains) and offers it on the next launch (row 19).
