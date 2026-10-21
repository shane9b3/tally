# Tally TODO (as of 2026-10-21, after the wave-5 integration pass on b03e603)

Status: waves 1–5 complete and integrated end to end against the real engine in the
desktop app. Next: review round 4 fixes (in flight), then the 1.0 exit work below.

## Done
- All 408 functions (405 AOO 4.1 + IFERROR, IFNA, NUMBERVALUE, ISOWEEKNUM); 5,003 conformance cases. `cargo run -p xtask -- function-coverage --check`.
- Engine v0.3 (names, arrays, merges, sort, filters, find/replace, formatting, CF, validation, notes, links, protection, page setup, locales, goal seek).
- Wave 5: charts and pictures (ODS + XLSX), DataPilot + GETPIVOTDATA, outline, standard/advanced filter, subtotals, consolidate, text to columns, duplicates, scenarios, cell styles, AutoFormat, print preview / PDF export / print (krilla), iterative calculation, precision as shown, solver (linear, `notLinear` status), detective and Mark Invalid Data, XLS/SXC/HTML/dBase import (HTML/dBase export), de/fr/es UI and function names, Options, Function Wizard (params, example, live result), Navigator, templates, autosave/recovery.
- Integration pass (this commit): `FunctionInfo.params/example`, `preview_formula`, `cells_with` (show formulas, value highlighting, zero values), `image_data`/`view_options`/`invalid_cells`/`custom_sort_lists` commands, `save_copy`/`open_untitled` (autosave and templates no longer move the document's path), CopySheet re-points charts and pivots, pivot sources stored by sheet id, statistics pages, ODS split persistence and `draw:notify-on-update-of-ranges`, Open/Save As dialogs for XLS/SXC/HTML/dBase, UI on the engine's outline/view options/sort lists/invalid cells, charts in both split panes, 41 l10n keys.
- Gates: 1,459 Rust tests (16 new command-layer integration tests in `apps/desktop/src-tauri/src/tests.rs`), 745 TS tests, 60 e2e; clippy and fmt clean; release bundle builds (Tally.app 26.8 MiB, dmg 12.3 MiB).

## Known gaps / follow-ups
- macOS printing: `wb_print` shows the PDF in a `tally-print` window and calls `Webview::print()`; if that fails it opens the PDF in Preview. Whether WKWebView's print sheet actually appears for PDF content has not been checked by hand yet (owner: File ▸ Print… in the release build).
- XLSX does not write the Window ▸ Split (`pane` without freeze); ODS does.
- Split is restored on sheet show, not after undo/redo.
- No e2e yet for View toggles and split restore (unit tests only).
- Tauri shell uses one global lock (a slow call blocks all documents); per-document locks planned.
- `bigattr.ods` (400 MiB XML attribute) still peaks ~800 MB RSS.
- Windows branch of `open_url`, Windows title bar, Intel macOS / Windows / Linux builds need CI.
- Main JS chunk is 1.6 MB (echarts); code-split before 1.0.

## Remaining for 1.0 (docs/wave5-plan.md §5)
1. Functions: keep 408/408; DDE and macro calls marked unsupported (done).
2. Conformance ≥ 99 % against the AOO oracle (`xtask aoo-oracle --setup/--run`, isolated profile) — owner runs it locally; record deviations in `docs/functions/deviations.md`.
3. Fidelity corpus: 200 real ODS/XLSX files, `xtask fidelity` ≥ 98 % lossless, 0 crashes — owner sources the corpus.
4. Performance benches (100k-formula recalc < 500 ms, 50 MB ODS open < 10 s, `chart_data` 10k points < 20 ms, 20-page PDF < 1 s, 100k-row pivot < 500 ms, 60 fps scroll) gated on three OSes — needs runners.
5. Accessibility: VoiceOver walkthrough (`docs/qa/a11y.md`), keyboard-only dialogs, accessibility-snapshot test.
6. Security: malicious corpus for chart/pivot/XLS/HTML, 24 h fuzzing of ODS/XLSX/XLS readers, security review round 4.
7. Parity docs: inventory checklist marked done/deviation, `docs/shortcuts.md`, `docs/parity-deviations.md`.
8. Owner-only: Developer ID + notarization, Windows signing, updater key and hosting, CI secrets/runners (incl. x86_64 macOS), App Store decision, AOO GUI fixtures (charts, pivots, scenarios, encrypted file), Inter TTFs, AOO l10n checkout, corpus.

## How to run
- `pnpm install`, `cargo test --workspace --features tally-engine/io`, `pnpm -r test`, `pnpm e2e`
- `pnpm tauri dev` (desktop) or `pnpm --filter tally-desktop dev:web` (browser with mock engine, `?template=invoices`)
- Release: `pnpm --filter tally-desktop tauri build`
