# Phase A manual test script (WP7 shell, WP5 file round trip)

Run on **macOS arm64** (required for the Phase A exit), **Windows 11 x64**
and **Ubuntu 24.04 (X11 and Wayland)**. Record OS, build (commit), and
pass/fail per step in the PR. Steps marked **(engine)** need WP1 integrated;
with the stub engine they show an "unsupported" toast instead, which is
the expected result until G2.

## 0. Start

| # | Step | Expected |
|---|---|---|
| 0.1 | `pnpm install`, then `pnpm tauri dev` from the repo root. | One window "Tally", 1280×820, centred; cannot be resized below 900×600. |
| 0.2 | macOS: look at the top-left. | Traffic lights sit inside the sidebar's top area (sidebar open) or before the document title (sidebar closed, ⌘\\); no native title text. Dragging the empty toolbar area or the sidebar top moves the window. |
| 0.3 | Windows/Linux: look at the frame. | Native title bar and decorations; window title follows the document title. |
| 0.4 | Menu bar. | macOS: Tally, File, Edit, View, Sheet, Window. Windows/Linux: File, Edit, View, Sheet, Help. Accelerators shown as in the table in §6. |

## 1. Values and formulas (engine)

| # | Step | Expected |
|---|---|---|
| 1.1 | Click A1, type `2`, Enter. Click B1, type `3`, Enter. Click C1, type `=A1*B1`, Enter. | C1 shows `6`; selection moves down after each Enter. |
| 1.2 | Click C1. | Name box `C1`; formula bar `=A1 × B1`; grey hint at the right `A1 × B1`. |
| 1.3 | Click the formula bar text. | Raw text `=A1*B1` appears, the cell shows the editor. Change to `=A1+B1`, Enter. | C1 shows `5`, selection on C2. |
| 1.4 | ⌘Z / Ctrl+Z. | C1 back to `6`. ⇧⌘Z / Ctrl+Shift+Z redoes. |
| 1.5 | Select A1:C1 (click A1, Shift+click C1). | Status bar `Sum 11  Count 3` (after the 1.4 undo). |
| 1.6 | F2 on C1, Esc. | Edit cancelled, value unchanged. |
| 1.7 | Type `hello` in A3, then ⌘→ from A1 / Ctrl+→. | Jumps to the data edge (C1). |

## 2. Formatting (engine)

| # | Step | Expected |
|---|---|---|
| 2.1 | Select A1, ⌘B. | A1 bold; the toolbar B pill is highlighted. ⌘B again un-bolds. |
| 2.2 | ⌘I, then the toolbar I pill. | Italic toggles on then off. |
| 2.3 | Select A1:B1, "$ Currency". | `$2.00  $3.00`; a negative value shows red with a minus sign. |
| 2.4 | "% Percent" on A1. | `200.00%`. |
| 2.5 | Enter 1, 2, 3 in D1:D3, select D4, "Σ Add up". | D4 contains `=SUM(D1:D3)` and shows 6. With no numbers above but numbers to the left, the left block is used. With neither, the editor opens with `=SUM(`. |
| 2.6 | Hover Sort, Filter, Chart. | Disabled look, tooltip "Coming soon"; clicking does nothing. |

## 3. Sheets (engine)

| # | Step | Expected |
|---|---|---|
| 3.1 | Click "+" in the bottom bar. | New tab `Sheet2`, active (white tab). |
| 3.2 | Double-click `Sheet2`, type `Orders`, Enter. | Tab renamed. Esc while renaming cancels. |
| 3.3 | Drag `Orders` before `Sheet1`. | Order updates; ⌘Z restores it. |
| 3.4 | Right-click a tab → Delete. | Tab removed; Delete is disabled when only one sheet is left. |
| 3.5 | Sheet menu → Insert / Rename / Delete. | Same as above. |

## 4. Clipboard (engine)

| # | Step | Expected |
|---|---|---|
| 4.1 | Select A1:C1, ⌘C, click A5, ⌘V. | Values pasted; formula in C5 re-anchored (`=A5*B5`) via the internal token. |
| 4.2 | Paste into TextEdit / Notepad / gedit. | Tab-separated values. |
| 4.3 | Copy a 2×2 block from another app, paste. | Values land as typed input (numbers recognised). |
| 4.4 | ⌘X on A5:C5. | Copied, then cleared. |

## 5. Palette, sidebar, zoom

| # | Step | Expected |
|---|---|---|
| 5.1 | ⌘K / Ctrl+K. | Centred card "Ask or do anything…", ACTIONS list with grey kind labels and shortcuts. Esc closes and focus returns to the grid. |
| 5.2 | Type `bold`, Enter. | Active cell toggles bold. |
| 5.3 | (WP8) Type `sum of Total` on the invoice template. | ANSWER row with value, monospace formula and "Insert ↵"; Enter inserts it into the active cell. |
| 5.4 | ⌘\\ and View ▸ Toggle Sidebar. | Sidebar hides/shows. |
| 5.5 | Sidebar: "+ New sheet". | New untitled workbook. |
| 5.6 | Sidebar: a template (Invoice tracker). | (engine, WP5) Opens an untitled workbook with the template's rows. |
| 5.7 | Click `100%` in the status bar → 150%. ⌘= / ⌘- / ⌘0. | Grid zooms; status shows the percentage. |

## 6. Files (WP5 + engine)

| # | Step | Expected |
|---|---|---|
| 6.1 | ⌘O → pick a `.ods`. | Native dialog filtered to ODS/CSV; the file opens; title = file name, subtitle `<folder> · Saved`; it appears in MY WORKBOOKS. |
| 6.2 | Edit a cell. | Subtitle `· Edited`; window title gets "— Edited". |
| 6.3 | ⌘S. | Saves in place (no dialog); `· Saved`. |
| 6.4 | ⇧⌘S → new name. | Save dialog; new path becomes current and recent. |
| 6.5 | Share ▸ Export as CSV…. | Save dialog with a `.csv` name; active sheet exported. |
| 6.6 | Cancel any dialog. | Nothing happens, no error toast. |
| 6.7 | Quit, relaunch, click a recent file. | Opens. Delete the file on disk, click it again: error toast, app keeps running. |
| 6.8 | Open the saved `.ods` in LibreOffice. | Values, formulas, bold/italic, number formats, column widths and sheet order match. |

Accelerators: New ⌘N, Open ⌘O, Save ⌘S, Save As ⇧⌘S, Close ⌘W, Undo ⌘Z, Redo ⇧⌘Z, Toggle Sidebar ⌘\\, Zoom ⌘= / ⌘- / ⌘0 (Ctrl on Windows/Linux).

## 7. Security

| # | Step | Expected |
|---|---|---|
| 7.1 | Release build (`pnpm tauri build`), right-click in the window. | No "Inspect Element"; devtools cannot be opened. |
| 7.2 | Debug build devtools console: `window.__TAURI_INTERNALS__.invoke("wb_open", { path: "/etc/hosts" })`. | Rejects with `invalid_argument` (not in the allowlist). |
| 7.3 | Same console: `invoke("plugin:fs|read_file", { path: "/etc/hosts" })` and `invoke("plugin:dialog|open", {})`. | Both rejected: not allowed by any capability. |
| 7.4 | Same console: `fetch("https://example.com")`. | Blocked by CSP. |
| 7.5 | Little Snitch / Windows firewall log / `nettop` while using the app. | No outbound connections. |
| 7.6 | Open `tests/fixtures/malicious/zip-bomb.ods` (and the rest of that folder) via ⌘O. | Error toast with a clear message; memory stays flat; the app keeps running. |

## 8. Browser build (no Tauri)

| # | Step | Expected |
|---|---|---|
| 8.1 | `pnpm dev:web`, open http://localhost:5173. | Full UI on MockClient; `?template=invoices` loads the sample data. |
| 8.2 | `pnpm e2e`. | All Playwright tests pass. |
