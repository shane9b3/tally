# Tally TODO (as of 2026-10-21, commit 7c29f0f)

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

## Known gaps / follow-ups
- Documents as tabs (child webviews of `win-*` windows, Tauri `unstable` multi-webview; the sidebar's "Open" list; `windows.rs`, `registry.rs`, docs/qa/multi-window-manual.md). Verified natively on macOS only through a scripted probe of the debug build (ACL for child webviews, new tab in the front window, Show Next Tab moving focus/visibility, Move Tab to New Window, Merge All Windows, closing a clean tab, the red button asking a dirty tab, Don't Save closing the window, Quit asking dirty tabs across a window and Cancel, Separate windows mode, File ▸ New with no window). Not checked by hand: what it looks like (sidebar list, traffic lights over child webviews), drag/reorder, keyboard shortcuts through real key presses, Dock reopen, print windows per document, full screen and resizing with several tabs (owner: run the checklist on the release build).
- Hidden tabs are hidden WKWebViews: WebKit throttles their timers (measured ~0.6 ticks/s for a 1 s interval), so a hidden document's autosave can fire a little late; `document.visibilityState` is `hidden` there.
- Windows: undecorated windows rely on Tauri's resize handler, which tauri-runtime-wry only attaches to whole-window webviews; with child webviews the top resize edge may not work on Windows (unverified). Linux: child webviews live in a GTK box (unverified).
- The Dock's Quit and log-out still cannot show the save prompt (`terminate:` is not vetoable through tao); every edited document gets an exit snapshot instead.
- With no window open (macOS), only File ▸ New and File ▸ Open… work from the menu; other File items (templates, Open Recent ▸ Clear Menu) need a window.
- The window registry uses a small leaf mutex (never held across engine, dialog, window or menu calls) instead of atomics; per-document engine locks are still a separate task.
- macOS printing: `wb_print` shows the PDF in a `tally-print` window and calls `Webview::print()`; if that fails it opens the PDF in Preview. Whether WKWebView's print sheet actually appears for PDF content has not been checked by hand yet (owner: File ▸ Print… in the release build).
- XLSX does not write the Window ▸ Split (`pane` without freeze); ODS does.
- Split is restored on sheet show, not after undo/redo.
- No e2e yet for View toggles and split restore (unit tests only).
- Tauri shell uses one global lock (a slow call blocks all documents); per-document locks planned.
- `bigattr.ods` (400 MiB XML attribute) still peaks ~800 MB RSS.
- Windows branch of `open_url`, Windows title bar, Intel macOS / Windows / Linux builds need CI.
- Main JS chunk is 1.6 MB (echarts); code-split before 1.0.
- XLS import: calamine 0.37 is shielded by a pre-scan, but a hostile file within budget can still use ~1 GB, and an allocation failure there aborts; re-check the pre-scan whenever calamine is upgraded.
- Pivot refresh keeps every previous table on the undo stack (no byte budget on undo).
- Pivot refresh overwrite prompt is covered by a mock-level test only (no e2e of the dialog flow).
- DataPilot L6 (captions with several data fields) and L7 (outline layout) not done.

## Remaining for 1.0 (docs/wave5-plan.md §5)
1. Functions: keep 408/408; DDE and macro calls marked unsupported (done).
2. Conformance ≥ 99 % against the AOO oracle (`xtask aoo-oracle --setup/--run`, isolated profile) — owner runs it locally; record deviations in `docs/functions/deviations.md`.
3. Fidelity corpus: 200 real ODS/XLSX files, `xtask fidelity` ≥ 98 % lossless, 0 crashes — owner sources the corpus.
4. Performance benches (100k-formula recalc < 500 ms, 50 MB ODS open < 10 s, `chart_data` 10k points < 20 ms, 20-page PDF < 1 s, 100k-row pivot < 500 ms, 60 fps scroll) gated on three OSes — needs runners.
5. Accessibility: VoiceOver walkthrough (`docs/qa/a11y.md`), keyboard-only dialogs, accessibility-snapshot test.
6. Security: 24 h fuzzing of ODS/XLSX/XLS readers (cargo-fuzz needs a nightly toolchain / CI); review round 4 done.
7. Parity docs: inventory checklist marked done/deviation, `docs/shortcuts.md`, `docs/parity-deviations.md`.
8. Owner-only: Developer ID + notarization, Windows signing, updater key and hosting, CI secrets/runners (incl. x86_64 macOS), App Store decision, AOO GUI fixtures (charts, pivots, scenarios, encrypted file), Inter TTFs, AOO l10n checkout, corpus.

## How to run
- `pnpm install`, `cargo test --workspace --features tally-engine/io`, `pnpm -r test`, `pnpm e2e`
- `pnpm tauri dev` (desktop) or `pnpm --filter tally-desktop dev:web` (browser with mock engine, `?template=invoices`)
- Release: `pnpm --filter tally-desktop tauri build`
