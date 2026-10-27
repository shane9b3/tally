# Manual QA: documents as tabs and windows, per-document guard

The decisions are unit-tested (`apps/desktop/src-tauri/src/registry.rs`:
placement and the "Open documents in" setting, tab activate/cycle/reorder,
Move Tab to New Window, Merge All Windows, closing tabs and windows with
several edited tabs, Quit across windows, Close Other Tabs, the exit plan,
print-window ownership, the Window menu list; `windows.rs`: window size,
cascade and webview bounds; `menu.rs`: menu routing; `prefs.rs`; and in
`packages/app/test` the sidebar "Editing" list, `tabs.test.ts`,
`windows.test.ts`, `unsaved.test.ts`). A scripted probe of the debug build
checked the shell paths natively on macOS (see docs/TODO.md), but nothing
was looked at: these checks run against the **release build**
(`pnpm --filter tally-desktop tauri build`, then open
`.targets/<dir>/release/bundle/macos/Tally.app`). Prepare two scratch files
`A.ods` and `B.ods` (saved once, so both are in Open recent). Tools ▸
Options ▸ Load / Save ▸ "Open documents in" starts at **Tabs in one window**.

| # | Step | Expected |
|---|---|---|
| 1 | Launch. | One window "Untitled"; the sidebar's **Open** section has one row (Untitled), looking like the old single-document section. Recovery is offered here only, if there is anything to recover. |
| 2 | File ▸ Open… `A.ods`. | `A.ods` replaces the untouched Untitled (still one row, one window). |
| 3 | Sidebar ▸ Open recent → `B.ods`. | `B.ods` opens as a second document **in the same window**; the Editing section lists `A.ods` and `B.ods`, `B.ods` highlighted; the window title (Dock, Window menu, Mission Control) is `B.ods`. |
| 4 | Click `A.ods` in the Editing section; then ⌃Tab, ⌃⇧Tab, ⇧⌘] and ⇧⌘[; then ⌥⌘1 and ⌥⌘9. | Each switches the shown document at once (no flicker, no blank frame); typing goes to the shown document right away (no extra click); ⌥⌘9 shows the last one. ⌘1 still opens Format ▸ Cells. ⌃PgDn/⌃PgUp still switch **sheets**. ⌃Tab also switches while typing in a cell and in the formula bar (the typing is not committed and the cursor does not move); plain Tab still commits and moves right. |
| 5 | Hover a row: tooltip; drag `B.ods` above `A.ods`. | Tooltip shows the file path; the order changes and stays after switching documents; Window menu lists them in the new order. |
| 6 | Arrow keys in the Editing list (focus a row first), Enter, Delete. | ↑/↓/Home/End move the focus, Enter shows that document, Delete closes it (asking if edited). |
| 7 | In `A.ods` open the same `B.ods` again (Open recent). | No second copy: `B.ods` is shown. |
| 8 | Right-click `B.ods` ▸ Move to New Window (or Window ▸ Move Tab to New Window while it is shown). | A new window of the same size, offset, shows `B.ods`; the first window shows `A.ods`; both windows' Editing sections list both documents (own window's first); typing works in each. Clicking the other window's document in a sidebar brings that window to the front with it shown. |
| 9 | Window ▸ Merge All Windows. | Every document is back in the front window; the emptied window closes; the front window keeps showing its document. |
| 10 | Window menu. | Minimize, Zoom, Enter Full Screen, Show Next Tab ⌃⇥, Show Previous Tab ⌃⇧⇥, Move Tab to New Window, Merge All Windows, Bring All to Front, then every open document with a check on the shown one of the front window. Choosing one shows its window and document. No AppKit window/tab items, and no View ▸ Show Tab Bar. |
| 11 | System Settings ▸ Desktop & Dock ▸ "Prefer tabs when opening documents": **Always**. Open a file, ⌘N, Window ▸ Move Tab to New Window, go full screen. | **No native macOS tab bar ever appears** (windows never merge into AppKit tabs); only the sidebar lists documents. Reset the setting afterwards. |
| 12 | Hide the sidebar (⌘\\). Switch documents with ⌃Tab and the Window menu. | Switching still works; the sidebar comes back with the right row highlighted. |
| 13 | Edit `A.ods` (do not save); show `B.ods`; Format ▸ Bold; ⌘W. | Bold applies to `B.ods` only; ⌘W closes `B.ods` at once (clean); `A.ods` is shown; the red button shows the edited dot (a document of the window is edited). |
| 14 | Edit two documents of one window, leave a third clean; click its **red close button**. | The shown edited document asks first, then the other edited one is shown and asks (`Save` / `Don't Save` / `Cancel`); after the last answer the window closes with all three. The clean one is never asked. |
| 15 | Repeat 14, but **Cancel** at the second prompt. | Nothing closes; the first document is still edited and asks again next time. |
| 16 | Right-click a row ▸ Close Others with two edited others. | Each edited one is shown and asks in turn; then the others close; Cancel keeps all. |
| 17 | Hover a hidden edited row, click its ×. | That document is shown and asks; Don't Save closes it; the window shows its neighbour. |
| 18 | Two windows, edited documents in both. ⌘Q. | The front window's shown document asks first, then the window's other edited ones, then the other window comes forward and its edited ones ask; Don't Save on all → the app quits. Repeat with Cancel in the second window: the app keeps running, everything still open and edited. |
| 19 | Resize a window with three documents, enter and leave full screen, switch documents. | Every document fills the window (the hidden ones too, once shown); the overlay title bar and traffic lights sit as before (traffic lights at the top left, not covering the sidebar title). |
| 20 | Tools ▸ Options ▸ Load / Save ▸ Open documents in: **Separate windows**; open a file and ⌘N. | Each opens in a new window (existing windows unchanged); Window ▸ Merge All Windows still combines them. Switch back to Tabs. |
| 21 | Print `A.ods` (File ▸ Print, a print window opens) and `B.ods` in the same window; close `A.ods` with its ×. | Only `A.ods`'s print window closes; `B.ods`'s stays. ⌘W in a print window closes only it. |
| 22 | Close every document (⌘W each). | macOS: the app keeps running. File ▸ New opens a window; close it; a Dock click opens a new Untitled window; minimise it and click the Dock icon: it comes back. |
| 23 | Edit two documents in two windows and one more as a tab of the first window, then Dock ▸ Quit (or `osascript -e 'quit app "Tally"'`). Repeat in Separate windows mode. | Each edited document asks in turn (its window comes forward, its tab is shown), as for ⌘Q. Cancel at any step: nothing closes, earlier answers are forgotten. Answer all: the app quits. |
| 24 | Tools ▸ Options: language Deutsch; View ▸ Theme. | Menu bar and every document (shown or hidden, every window) switch; the Editing section heading reads "In Bearbeitung". |
| 25 | Windows/Linux build (if available): steps 3, 4 (Ctrl+Tab / Ctrl+Shift+Tab, Alt+1…9), 8, 9, 14, 18, and resizing from every edge (Windows: top edge especially). | As on macOS; closing the last window exits the app; the Windows title bar buttons act on the right window after Move to New Window. |

Known limitations: hidden documents' timers
are throttled by WebKit (autosave may run a little late).
