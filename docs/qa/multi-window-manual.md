# Manual QA: one document per window, native tabs, per-window guard

The decisions are unit-tested (`apps/desktop/src-tauri/src/registry.rs`:
window reuse, quit sequencing, exit plan, print-window ownership, the window
list; `menu.rs`: menu routing; `windows.rs`, `prefs.rs`; and
`packages/app/test/windows.test.ts` / `unsaved.test.ts` for the webview
side). Window creation, AppKit tabs, focus and the Dock cannot be driven
from the tests, so these checks run against the **release build**
(`pnpm --filter tally-desktop tauri build`, then open
`target/release/bundle/macos/Tally.app`). Prepare two scratch files `A.ods`
and `B.ods` (saved once, so both are in Open recent) and leave System
Settings ▸ Desktop & Dock ▸ "Prefer tabs when opening documents" at
"In full screen" unless a row says otherwise.

| # | Step | Expected |
|---|---|---|
| 1 | Launch. | One window "Untitled" (the first window; recovery is offered here only, if there is anything to recover). |
| 2 | File ▸ Open… `A.ods`. | `A.ods` replaces the untouched Untitled in the same window (no second window). |
| 3 | Sidebar ▸ Open recent → `B.ods`. | `B.ods` opens in a **new** window, offset from the first; `A.ods` stays in its window. |
| 4 | In the `B.ods` window open `A.ods` again (Open recent). | No third window: the `A.ods` window comes to the front. |
| 5 | ⌘N, type a value in the new Untitled window, then open a template from its sidebar. | ⌘N opens a new Untitled window; the template opens in another new window (the edited Untitled is kept). |
| 6 | Window menu. | Minimize, Zoom, Enter Full Screen, Bring All to Front, then the open windows (`A.ods`, `B.ods`, Untitled…, check mark on the front one) and the tab items (Show Previous/Next Tab, Move Tab to New Window, Merge All Windows). Choosing a window brings it to the front. |
| 7 | Window ▸ Merge All Windows. | All document windows become tabs of one window, with the document titles on the tabs. Check that the tab bar does not cover the toolbar (overlay title bar). View ▸ Show Tab Bar / Show All Tabs are present (AppKit adds them). |
| 8 | Drag a tab out, or Window ▸ Move Tab to New Window. | It becomes a separate window again; both keep working (type in each). |
| 9 | Set "Prefer tabs" to **Always**, then ⌘N and open another file. | The new documents open as tabs of the front window. |
| 10 | ⌘` (and ⌘⇧`) with three windows. | Cycles through the document windows. |
| 11 | Edit `A.ods` (do not save). Bring `B.ods` to the front; Format ▸ Bold, ⌘W. | Bold applies to `B.ods` only; ⌘W closes the `B.ods` window at once (clean); `A.ods` and its dot are untouched. |
| 12 | Edit two windows (`A.ods` and an Untitled); leave a third clean. ⌘Q. | The front edited window comes forward and asks; answer **Don't Save** → the other edited window comes forward and asks; answer **Save** → the Save As dialog (untitled); save it → the app quits. The clean window is never asked. |
| 13 | Repeat 12, but answer **Cancel** in the second window. | The app keeps running with all windows; the first window's document is still open and still edited (its dot is back), and ⌘Q asks it again. |
| 14 | Repeat 12, but cancel the Save As dialog in the second window. | Same as 13. |
| 15 | With an edited window: red close button on it → Cancel; then on a clean window. | The prompt appears in the edited window only; the clean one closes at once. |
| 16 | Close every window (⌘W on each). | macOS: the app keeps running (menu bar, Dock dot). File ▸ New opens a window; close it; File ▸ Open… opens the chosen file in a new window. |
| 17 | Close every window, then click the Dock icon. | A new Untitled window. Minimise all windows and click the Dock icon: the front one comes back. |
| 18 | In `A.ods` choose File ▸ Print (a print window opens); in `B.ods` too. Close the `A.ods` window. | Only the `A.ods` print window closes with it; the `B.ods` print window stays. |
| 18b | Click the `B.ods` print window, press ⌘W. | Only the print window closes; `B.ods` stays open. |
| 19 | Edit `A.ods` and an Untitled window (do not save; let one autosave interval pass or not), then `osascript -e 'quit app "Tally"'` (the Dock's Quit). Relaunch. | The app quits without asking. The first window offers recovery of **both** documents; Recover on the first takes over the untouched start-up window (the dialog stays up with the other one), the second opens in its own window. A second window opened afterwards (⌘N) offers nothing. |
| 20 | Tools ▸ Options in one window: switch the language to Deutsch. | The menu bar and **every** window switch to German. View ▸ Theme in one window: every window follows. |
| 21 | Set Tools ▸ Options ▸ AutoRecovery to 1 minute; edit two windows; wait 1 minute; force quit (Activity Monitor). Relaunch. | Both documents are offered for recovery. |
| 22 | Windows/Linux build (if available): open two files, close one window, then the other. | Each window has a Window menu with the window list (check mark follows focus); closing the last window exits the app. ⌘/Ctrl+W and the title-bar ✕ ask in that window only. |

Known limitations: the Dock's Quit and log-out/shutdown still quit without the
prompt (they send `terminate:`, which tao does not let the app veto); every
edited document gets an exit snapshot instead (row 19). The tab items and the
window list in the macOS Window menu are AppKit's (localised by the system),
so an open print window ("Print") is listed there too; that is expected.
