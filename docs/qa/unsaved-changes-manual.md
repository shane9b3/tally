# Manual QA: unsaved-changes guard and the macOS "edited" dot

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
| 5 | Edit again. Click `B.ods` under Open recent in the sidebar. | Sheet prompt: "Do you want to save the changes you made to “A.ods”?" / "Your changes will be lost if you don't save them.", buttons Don't Save · Cancel · **Save** (default). |
| 6 | Press Escape. | Prompt closes; `A.ods` and the edit are still there; dot still shown. |
| 7 | Click `B.ods` again, choose Save. | `A.ods` is saved (reopen it to check), then `B.ods` opens; no dot. |
| 8 | ⌘N, type a value, then ⌘O. Choose Save. | The Save As dialog opens (untitled). Cancel it: nothing else happens, the untitled document and its value stay. |
| 9 | Repeat 8 but choose Don't Save. | The Open dialog appears; the untitled edit is gone after opening a file. |
| 10 | Edit, then click a template ("Monthly budget") and "+ New sheet" in the sidebar, and File ▸ New / File ▸ Close (⌘W). | Each asks first; Cancel keeps the document. |
| 11 | Edit, click the red close button. | The prompt appears in the window; the window stays open. Cancel: still open, dot still shown. |
| 12 | Red close button again → Don't Save. | The window closes and the app exits; no recovery offer on the next launch. |
| 13 | Relaunch, edit, ⌘Q (or Tally ▸ Quit Tally). | Prompt; Cancel keeps the app running. ⌘Q → Save on a titled file saves and quits; on an untitled file the Save As dialog opens, and cancelling it keeps the app running. |
| 14 | With a clean document: red button, then relaunch and ⌘Q. | Close / quit immediately, no prompt. |
| 15 | Switch Tools ▸ Options ▸ Language to Deutsch, edit, ⌘Q. | Menu item reads "Tally beenden"; the prompt reads "Möchten Sie die Änderungen an „…“ speichern?" with Nicht speichern / Abbrechen / Speichern. |
| 16 | Windows/Linux build (if available): edit, click the title-bar ✕ / Alt+F4. | Same prompt; Cancel keeps the window. No dot (macOS only). |

Known limitation: the Dock's Quit and log-out/shutdown send
`-[NSApp terminate:]` directly, which tao does not let the app veto, so they
quit without the prompt. The autosave snapshot (Tools ▸ Options ▸ AutoRecovery)
is kept in that case and offered on the next launch.
