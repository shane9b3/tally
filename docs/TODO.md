# Tally TODO (as of 2026-10-27, commit de70ada)

Status: waves 1–5 complete, integrated end to end, and review round 4 (security +
correctness) fixed. Engineering work for the feature set is done; what remains is the
1.0 exit work below, most of which needs the owner (signing, CI runners, AOO oracle run,
real-file corpus).

## Done
- All 408 functions (405 AOO 4.1 + IFERROR, IFNA, NUMBERVALUE, ISOWEEKNUM); 5,003 conformance cases. `cargo run -p xtask -- function-coverage --check`.
- Engine v0.3 (names, arrays, merges, sort, filters, find/replace, formatting, CF, validation, notes, links, protection, page setup, locales, goal seek).
- Wave 5: charts and pictures (ODS + XLSX), DataPilot + GETPIVOTDATA, outline, standard/advanced filter, subtotals, consolidate, text to columns, duplicates, scenarios, cell styles, AutoFormat, print preview / PDF export / print (krilla), iterative calculation, precision as shown, solver (linear, `notLinear` status), detective and Mark Invalid Data, XLS/SXC/HTML/dBase import (HTML/dBase export), de/fr/es UI and function names, Options, Function Wizard (params, example, live result), Navigator, templates, autosave/recovery.
- Integration pass (this commit): `FunctionInfo.params/example`, `preview_formula`, `cells_with` (show formulas, value highlighting, zero values), `image_data`/`view_options`/`invalid_cells`/`custom_sort_lists` commands, `save_copy`/`open_untitled` (autosave and templates no longer move the document's path), CopySheet re-points charts and pivots, pivot sources stored by sheet id, statistics pages, ODS split persistence and `draw:notify-on-update-of-ranges`, Open/Save As dialogs for XLS/SXC/HTML/dBase, UI on the engine's outline/view options/sort lists/invalid cells, charts in both split panes, 41 l10n keys.
- Review round 4 (docs/reviews/*-4-2026-10-21.md): 5 Critical + 15 High security findings and 15 High correctness findings fixed with regression tests (docs/security.md "Review 4 fixes").
- Gates at 7c29f0f: 1,567 Rust tests, 755 TS tests, 60 e2e; clippy and fmt clean; bundle check ok; release bundle builds (Tally.app 27 MiB, dmg 12.4 MiB).
- Since then (ffb0eb1..de70ada):
  - Unsaved-changes guard (pending cell edits count; the Dock's Quit and log-out ask too; exit snapshot as a safety net); macOS edited dot.
  - Multiple windows and document tabs in the sidebar ("Editing" list), Options ▸ Load/Save "Open documents in" Tabs | Separate windows; Window menu, ⌃Tab / ⇧⌘] [ / ⌥⌘1–9.
  - One engine and lock per document (45dd08c): a slow call no longer blocks other windows.
  - MROUND rounds when signs differ (AOO); print header/footer areas (on/off, spacing, AutoFit); optimal-height rows print at their text height; Print Preview clears the traffic lights.
  - Document fonts (default font, metric-compatible substitutes, PDF font resolution); column default cell styles applied correctly (the "Lakefront" file); font name / typeface / size pickers; View ▸ Toolbars and Customize Toolbars.
  - Formula round trips: CalcA1 names vs references (2eb60de); ExcelA1 intersection import/export (204c215).
  - Exact-match lookup index per recalc: 100k VLOOKUPs into 10k rows 7.2 s → 0.27 s (f2ce56c).
  - Large files: 50 MB ODS opens (2 GiB per data part, 4 GiB per package); per-XML-event and retained-attribute budgets, one ratio allowance per package, no shared XLSX worksheet parts; localised "too large" message (86cbf1b).
  - Accessibility: keyboard-only dialogs, ARIA throughout, axe + keyboard e2e specs; WCAG AA contrast in light and dark, gated by axe (cdd91b3, de70ada).
  - Code-split bundle: main chunk 1.65 MB → 511 kB; lazy parts behind error boundaries.
  - Docs: `docs/parity-deviations.md` (110 entries), 1.0 status in the inventories, `docs/perf-baseline.md` (`cargo xtask perf`).
  - 3D references evaluate (2026-10-29): `Sheet1.A5:Sheet3.A5` / `Sheet1:Sheet3!A5` span the sheets in tab order (AOO `svDoubleRef` with `Tab1 != Tab2`); sequence functions iterate every sheet, lookups / `*IF` / matrices give AOO's Err:504 / Err:502, scalar use intersects on the formula's sheet; deleting an end sheet shrinks the span; INDIRECT returns 3D references. Remaining gaps: `docs/parity-deviations.md` P19. References can be picked on other sheets while typing (2026-10-30, U26).
- Gates at de70ada: 1,734 Rust tests, 907 TS unit tests, 135 e2e; clippy (also tally-engine without `io`, xtask `perf`) and fmt clean; release bundle builds (Tally.app 27.8 MiB).

- Read-only mode and AOO lock files (2026-11-02): `.~lock.<name>#` read and written in AOO 4.1's format (`apps/desktop/src-tauri/src/doclock.rs`), "Document in Use" prompts, File ▸ Open Read-Only…, unwritable / Finder-locked files and ODS `LoadReadonly` open read-only, Edit ▸ Edit Mode (⇧⌘M), info bar, " (read-only)" title, engine-level `read_only` refusal. Owner's native checks: docs/qa/read-only-manual.md.
- Chart axes at AOO 4.1 parity (2026-11-06, T18): date axis (Scale ▸ Type Automatic / Text / Date, resolution, major / minor interval; AOO's automatic date axis over date categories, gaps for missing dates, labels in the cells' format) and Positioning (cross other axis at Start / End / Value / Category, label and mark placement), ODS and XLSX round trip.

## Known gaps / follow-ups
- Read-only mode: needs the owner's side-by-side check with OpenOffice (docs/qa/read-only-manual.md); Windows/Linux lock behaviour untested; no Properties ▸ Security tab to set `LoadReadonly`; Save As onto our own lock held elsewhere is refused (AOO offers Retry / Save); see docs/parity-deviations.md U28, X25.
- Documents as tabs (child webviews of `win-*` windows, Tauri `unstable` multi-webview; the sidebar's "Editing" list; `windows.rs`, `registry.rs`, docs/qa/multi-window-manual.md). Verified natively on macOS only through a scripted probe of the debug build (ACL for child webviews, new tab in the front window, Show Next Tab moving focus/visibility, Move Tab to New Window, Merge All Windows, closing a clean tab, the red button asking a dirty tab, Don't Save closing the window, Quit asking dirty tabs across a window and Cancel, Separate windows mode, File ▸ New with no window). Not checked by hand: what it looks like (sidebar list, traffic lights over child webviews), drag/reorder, keyboard shortcuts through real key presses, Dock reopen, print windows per document, full screen and resizing with several tabs (owner: run the checklist on the release build).
- Hidden tabs are hidden WKWebViews: WebKit throttles their timers (measured ~0.6 ticks/s for a 1 s interval), so a hidden document's autosave can fire a little late; `document.visibilityState` is `hidden` there.
- Windows: undecorated windows rely on Tauri's resize handler, which tauri-runtime-wry only attaches to whole-window webviews; with child webviews the top resize edge may not work on Windows (unverified). Linux: child webviews live in a GTK box (unverified).
- The Dock's Quit and log-out/restart/shutdown now ask like ⌘Q: `crates/tally-macos-terminate` (the one crate allowed `unsafe`, docs/security.md) adds `applicationShouldTerminate:` to tao's app delegate and answers "later" until the Quit sequence ends. Verified on the debug build only through `TALLY_DEBUG_TERMINATE_MS` (clean: terminates at once; edited: the webview is asked during AppKit's wait and the reply terminates through `RunEvent::Exit`). Owner tried it on the release build 2026-10-27 ("seems to work"); rows not individually confirmed: a second Dock Quit while a prompt is open, and log-out Cancel / Save (rows 19–23 of docs/qa/unsaved-changes-manual.md, row 23 of multi-window-manual.md, on the release build). No time limit on an unanswered prompt; escape hatch: a second Dock Quit quits anyway (the exit snapshot keeps the unanswered documents for recovery), and a question that cannot be sent quits the same way. On the debug build AppKit itself terminated on a `terminate:` sent while one waited, without asking the delegate again (the shim's own escape rule, 5 s or the second nested request, is the fallback if a real Dock Quit does ask again); owner: row 20a. Drop the shim if tao ever implements `applicationShouldTerminate:` itself (the shim then replaces tao's and logs it).
- With no window open (macOS), only File ▸ New and File ▸ Open… work from the menu; other File items (templates, Open Recent ▸ Clear Menu) need a window.
- The window registry uses a small leaf mutex (never held across engine, dialog, window or menu calls) instead of atomics.
- macOS printing: `wb_print` shows the PDF in a `tally-print` window and calls `Webview::print()`; if that fails it opens the PDF in Preview. Whether WKWebView's print sheet actually appears for PDF content has not been checked by hand yet (owner: File ▸ Print… in the release build).
- XLSX does not write the Window ▸ Split (`pane` without freeze); ODS does.
- Split is restored on sheet show, not after undo/redo.
- No e2e yet for View toggles and split restore (unit tests only).
- Windows branch of `open_url`, Windows title bar, Intel macOS / Windows / Linux builds need CI.
- l10n: the de/fr/es text of the ~1,000 keys added in the literals pass wants a native-speaker review; `FILTER_SOON`/`CHART_SOON` stubs in `commands.ts` are still English.
- XLS import: calamine 0.37 is shielded by a pre-scan, but a hostile file within budget can still use ~1 GB, and an allocation failure there aborts; re-check the pre-scan whenever calamine is upgraded.
- Pivot refresh keeps every previous table on the undo stack (no byte budget on undo).
- Pivot refresh overwrite prompt is covered by a mock-level test only (no e2e of the dialog flow).
- DataPilot L6 (captions with several data fields) and L7 (outline layout) not done.
- DataPilot field chips were restructured for a11y (toggle button inside a draggable group); drag starting on the name is only tested in Chromium — check in WKWebView by hand.
- A11y not covered by axe: Fill Series, Multiple Operations, Create Names, Show Outline Level (don't open from a bare command in the mock); contrast of hover/pressed states, menus and popovers.
- Fonts: render_pdf font warnings are dropped in `CachedJob::pdf_bytes`; only regular/bold/italic/bold-italic faces (no other weights); "Fonts in this document" lists loaded styles only; header/footer AutoFit measures with Inter metrics.
- A whole-column style split by one styled cell is written cell by cell after re-import (docs/file-formats.md).
- Excel export: an intersection imported from XLSX is written as `(Name) (A1:B2)` on the first save (valid; stable afterwards). ExcelA1 never reads `SUM (A1)` with a space as a call (Excel rejects it too).
- Lookup index: overlapping ranges over one column (`$A$1:$A$n`, `$A$2:$A$n`, `A:A`) get separate indexes within the 256 MB budget.
- Perf (docs/perf-baseline.md hot spots 3–5): 50 MB XLSX/ODS open peaks at 0.9–1.1 GB RSS (was 1.4–1.6 GB; further cuts need a block column store and a compact formula-cell table); CSV import 1M rows in ~1.2 s (was 7.6 s); grid fling drops frames in headless Chromium — re-measure in the Tauri webview.
- Not built yet from the plan: multithreaded recalc, auto-update (tauri-plugin-updater), opt-in crash reporting.
- Inventory gaps found 2026-10-27 (docs/calc-features-inventory.md 1.0 status; deviation IDs in docs/parity-deviations.md). Sizes: S < 1 day, M a few days, L a week or more.
  - Input and editing: two-digit year window option (P17, S).
  - Sheets and view: multi-sheet selection and editing, Shift+Ctrl+PgUp/PgDn (wave 5 item 31, L; engine done 2026-10-28: `Transaction.group`, `Op::FillSheets`; UI, mock and dialog in progress); read-only mode (done 2026-11-02, see below); zoom 20-400 % (U18, S); object display modes, guides, scroll-bar and outline-symbol toggles (U18, M); undo depth setting and undo / redo history lists (U17, M).
  - Fill: Ctrl-drag copies, filtered ranges copy (T16, M); Edit > Fill > Up / Left commands (S); Fill Sheet (engine `Op::FillSheets` done; dialog in progress).
  - Formatting: Hide all / Hide when printing (G15, done 2026-11-06); `[NatNum]` / `[DBNum]` / calendar modifiers rendered (G7, L); Load Styles from another document (M).
  - Data: validity "sort entries ascending" incl. ODS `sorted` round trip (G16, S); sort extend-selection prompt (S); DataPilot filter button opens a filter dialog (T17, M); name options print range / filter / repeat incl. ODS `range-usable-as` (M); Data > Select Range and Refresh Range (S).
  - Charts: chart data table editor for internal data (T18, M); cylinder / cone / pyramid shapes (T10, L).
  - Tools: detective Fill Mode, Refresh Traces, AutoRefresh (T12, done 2026-11-06, see below); hyperlink as button and New Document target (M, button needs form controls T11); Function List sidebar (S); Limit decimals for General incl. ODS `table:decimal-places` (S).

- Tools ▸ Detective at AOO 4.1 parity (2026-11-06): the AOO menu (Trace / Remove Precedents, Trace / Remove Dependents, Remove All Traces, Trace Error, Mark Invalid Data, Refresh Traces, AutoRefresh, Fill Mode); each Trace adds one level and each Remove takes the outermost away (`ScDetectiveFunc::ShowPred` / `FindPredLevel`); the operation list (`ScDetOpList`) moves with rows and columns, is replayed by Refresh Traces and by AutoRefresh after formula changes (any change with a Trace Error), and round-trips as ODS `table:detective`/`table:operation` (plus `table:highlighted-range`); arrows from / to other sheets with a sheet icon; Fill Mode clicks trace precedents until Esc. Remaining differences: T12.

## Remaining for 1.0 (docs/wave5-plan.md §5)
1. Functions: keep 408/408; DDE and macro calls marked unsupported (done).
2. Conformance ≥ 99 % against the AOO oracle (`xtask aoo-oracle --setup/--run`, isolated profile) — owner runs it locally; record deviations in `docs/functions/deviations.md`.
3. Fidelity corpus: 200 real ODS/XLSX files, `xtask fidelity` ≥ 98 % lossless, 0 crashes — owner sources the corpus.
4. Performance benches (100k-formula recalc < 500 ms, 50 MB ODS open < 10 s, `chart_data` 10k points < 20 ms, 20-page PDF < 1 s, 100k-row pivot < 500 ms, 60 fps scroll) gated on three OSes — needs runners. macOS baseline and proposed thresholds: `docs/perf-baseline.md` (`cargo xtask perf`, `pnpm perf:e2e`); all recalc/open/save checks pass on an M4 Pro, and so do `chart_data` 10k points (~3 ms), a 20-page PDF (~12 ms) and a 100k-row DataPilot refresh (~30 ms) (`cargo xtask perf --only chart,pdf,pivot`). Not yet measured: scroll in the real webview.
5. Accessibility: keyboard-only dialogs, axe/keyboard e2e specs and WCAG AA contrast done; VoiceOver walkthrough (`docs/qa/a11y.md`) — owner.
6. Security: 24 h fuzzing of ODS/XLSX/XLS readers (cargo-fuzz needs a nightly toolchain / CI); review round 4 done.
7. Parity docs: done (`docs/parity-deviations.md`, inventories marked); ~47 inventory rows still "unverified".
8. Owner-only: Developer ID + notarization, Windows signing, updater key and hosting, CI secrets/runners (incl. x86_64 macOS), App Store decision, AOO GUI fixtures (charts, pivots, scenarios, encrypted file), Inter TTFs, AOO l10n checkout, corpus.

## How to run
- `pnpm install`, `cargo test --workspace --features tally-engine/io`, `pnpm -r test`, `pnpm e2e`
- `pnpm tauri dev` (desktop) or `pnpm --filter tally-desktop dev:web` (browser with mock engine, `?template=invoices`)
- Release: `pnpm --filter tally-desktop tauri build`
