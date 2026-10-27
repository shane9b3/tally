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
| 19 | Edit a cell (do not save), then right-click Tally in the Dock ▸ Quit (or `osascript -e 'quit app "Tally"'`, the same `terminate:`; make sure no other Tally is running). | The same prompt as ⌘Q, in the edited document. **Cancel**: the app keeps running, nothing closes. Dock ▸ Quit again → **Don't Save**: the app quits; relaunch: no recovery offer. |
| 20 | Edit two documents (two windows, or two tabs), Dock ▸ Quit. Save / Don't Save in the first prompt. | Only one prompt at a time: the second document asks next → answer → the app quits. |
| 20a | Edit a document, Dock ▸ Quit, leave the prompt open, then Dock ▸ Quit again. Relaunch. | The escape hatch: the app quits anyway, without a second prompt; on relaunch recovery offers the edited document. (Observed on the debug build: a `terminate:` sent while one waits for its answer quits at once; if your macOS instead asks again, the shell re-asks the open prompt the first time and quits on the next Dock Quit, or on any Dock Quit after 5 s.) |
| 21 | With nothing edited (clean documents, a print window open, or no window at all): Dock ▸ Quit. | Quits at once, no prompt. |
| 22 | Edit a cell, then Apple menu ▸ Log Out… ▸ Log Out (save your other work first). | Tally comes forward with the prompt. **Cancel**: the log-out stops and macOS reports that Tally interrupted logging out; Tally keeps running. Log out again → **Save** (or Don't Save): Tally quits and the log-out continues. (Restart and Shut Down behave the same.) |
| 23 | ⌘Q with an edited document, then Dock ▸ Quit while that prompt is open. | One prompt only (the Dock's Quit brings it forward); Cancel keeps the app running; Don't Save quits once (no second prompt). |
| 24 | Debug build only, no Dock needed: `TALLY_DEBUG_TERMINATE_MS=4000 TALLY_DEBUG_TERMINATE_EDITED=1 target/debug/tally-desktop` (built with `pnpm --filter tally-desktop tauri build --debug --no-bundle`). | stderr shows `applicationShouldTerminate: … -> Later`, `replyToApplicationShouldTerminate:true`, `RunEvent::Exit`, and the app exits (the clean webview answers for itself). Without `…_EDITED`: `-> Now`, then `RunEvent::Exit`. With `TALLY_DEBUG_TERMINATE_HANG=1` too (a hung webview: the question is never sent; a second `terminate:` follows after 6 s): `not asking doc-1`, `sending terminate: again`, `RunEvent::Exit`, `exit snapshot plan: 1 to snapshot`. |

The Dock's Quit, log-out, restart and shutdown send `-[NSApp terminate:]`.
tao's app delegate has no `applicationShouldTerminate:`, so the shell adds
one (`crates/tally-macos-terminate`, docs/security.md): it runs the same
sequence as ⌘Q and answers macOS "later" until the questions are answered.
There is no time limit on the prompt itself. If it never appears (a hung or
crashed document webview), **Quit from the Dock again to quit anyway;
unsaved documents are kept for recovery** (row 20a). If the question cannot
be sent at all, the quit goes ahead the same way. At log-out macOS also
gives up on its own and reports that Tally interrupted it.

"Kept for recovery" is the exit snapshot: on `RunEvent::Exit` (every quit
except a Force Quit or a crash, which skip it) the shell writes a fresh
autosave snapshot of every edited document nobody answered for (only if the
engine reports unsaved changes; the wait for the engine lock is capped at
about 1.5 s, the write itself is not; skipped if the engine stays busy) and
offers it on the next launch. After a Force Quit or a crash only the last
periodic snapshot from Tools ▸ Options ▸ AutoRecovery is there. The
snapshot also covers a missing guard (the log then says "quit guard … not
installed": the Dock's Quit then quits unasked, as before).
