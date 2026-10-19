# Phase A: first runnable build

Phase A is the minimal subset of v0.1 (see architecture-plan.md §6–7) that yields a running Tauri app with a live engine grid, formulas with the 68 batch-0 functions, undo/redo, number formats, ODS + CSV round trip, the mockup chrome, and the ⌘K rule-based assist.

## Status (integration, 2026-10-19)

All Phase A work packages are merged and wired together in the desktop app:

- `apps/desktop/src-tauri` runs `tally_engine::EngineService::with_default_codec()` (ODS + CSV through `tally-io-ods`/`tally-io-csv`, `io` feature) instead of the WP7 stub, with `tally-assist`'s `RuleProvider` installed via `set_assist_provider` (`src/assist.rs` adapts the two `SheetView` traits).
- ⌘K previews are computed by the engine without committing (`Workbook::eval_formula_text_at`, `AssistRequest::preview`).
- Templates are built in code by the engine from an id (`invoices`, `budget`, `stock`); `wb_new(template)` passes the id through and `list_templates` returns the engine catalog. The bundled CSV template files are gone.
- Suggestions carry `actionArgs`; the palette runs `format.currency` on the named column body; `filter.byColumn` / `chart.byColumn` show a "coming in v0.2" toast.
- Shell methods live in `@tally/bridge`'s `ShellClient` (implemented by `TauriClient` and `MockClient`); `@tally/bridge/tauri` is a package export.

Gates passed at integration (macOS arm64):

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --features tally-engine/io -- -D warnings` | clean |
| `cargo test --workspace --features tally-engine/io` | 453 passed, 0 failed, 5 ignored |
| `pnpm -r typecheck` / `pnpm -r test` | clean / 102 passed (tokens 3, bridge 13, grid 58, app 28) |
| `pnpm e2e` | 5 passed |
| Command-layer end-to-end (`apps/desktop/src-tauri/src/tests.rs`) | invoices template → SUMIF → ⌘K formula + preview → ODS round trip → CSV with injection guard |
| `pnpm tauri dev` | compiles, 1280×820 window on screen |
| `pnpm tauri build` | `Tally.app` 9.1 MiB, unsigned `.dmg` 4.0 MiB |

Still manual: the WP7/WP5 click-through scripts (docs/qa/phase-a-manual.md), opening a saved file in LibreOffice, and signed CI bundles. The WP5 check below now opens a saved copy of the built-in `invoices` template (there is no `templates/invoices.ods`).

## v0.2 engine status (2026-10-19)

Engine, API and ODS work for v0.2 is in (see docs/api.md for the shapes):
named ranges (workbook/sheet scope, ODS `table:named-expressions`), array (CSE)
formulas (ODS matrix spans), dynamic dependencies indexed for INDIRECT/OFFSET
(`record_dynamic_dep`, `parse_reference_text` resolves names and sheet-qualified
references), cut/move with `move_refs`, merges (ODS write with covered cells),
freeze panes, sort with header detection, AutoFilter (ODS database ranges, filter-hidden
rows distinct from user-hidden rows), hidden rows/columns, find & replace, per-edge
borders (ODS `fo:border*`), paste special, fill series, hard recalc, and XLSX open/save
through `tally-io-xlsx` in `IoCodec`. Pending outside the engine: registering the five
new Tauri commands in `apps/desktop/src-tauri/src/lib.rs`, `TauriClient` methods for
`EngineClientV2`, and the UI. `EvalContext::hidden_row(sheet, row)` now exists
(default `false`); SUBTOTAL 101–111 skip rows it reports, so the engine only has to
implement it. SUM includes filtered rows, as in AOO.

## Function library status (2026-10-19)

`crates/tally-functions` registers **407 functions**: all 405 of AOO 4.1
(`docs/calc-functions.json`: 296 built-in, 101 Analysis add-in, 8 Date add-in) plus
the extensions IFERROR and IFNA; NUMBERVALUE, unimplemented in AOO, is implemented
as the 3-argument ODFF/Excel extension. Per-category table:
`docs/functions/coverage.md` (`cargo xtask function-coverage --doc`). ODF/OOXML
names follow the master list and AOO's add-in clash rule (overrides in
`docs/functions/gen-master-list.py`). Gates: `cargo fmt`/`clippy -D warnings` clean
for `tally-functions`, 312 crate tests, and 407 conformance files / 5003 cases
passing. Stubs that need engine hooks: GETPIVOTDATA (`#REF!`, no pivot tables),
MULTIPLE.OPERATIONS (`Err:504`), CURRENT (`#N/A`), and filtered-row exclusion for
SUBTOTAL 1–11.

## Dropped or stubbed in Phase A (all return in v0.2)

- Insert/delete rows/columns, cut-move reference adjustment (cut = copy then clear), merges, frozen panes, row resize (column resize stays), multi-range selection.
- Point mode, coloured reference highlighting (`@tally/formula-ts` not needed), IME acceptance test (hidden focused textarea stays).
- Named ranges, CSE array formulas (but elementwise array arithmetic inside array-forcing params is required for SUMPRODUCT), iterative calc (cycles → Err:522), ExcelA1 printer (CalcA1 + OpenFormula only).
- Dark theme (tokens structured for it), ARIA mirror, status chips, "At a glance" panel.
- XLSX, HTML clipboard (internal + TSV only), fuzz targets (limits and malicious corpus tests stay).
- ODS subset: values, formulas + cached values, bold/italic, number format, column widths, sheet names/order, active sheet. Files are always recalculated on load; warn when saving files from other apps.
  - Formatting of blank areas is kept: long style runs are saved as repeated rows of styled empty cells, and a formatted whole column as the column's `table:default-cell-style-name` (both read back as `SheetData::style_runs`).
  - Whole-column/row references are saved as full areas with absolute rows (`[.A$1:.A$1048576]`, `[.$A2:.$XFD3]`) because AOO 4.1 cannot read `[.A:.A]`; both forms load as whole columns/rows.
  - A formula that does not parse is saved as a text cell holding what was typed (no `table:formula`), so other applications show the text. It reloads as text.
- en-US only. Formats: General, 0, 0.00, #,##0, #,##0.00, 0%, 0.00%, $#,##0.00 with [RED] negative section, 0.00E+00, @, MM/DD/YYYY, YYYY-MM-DD, DD MMM YYYY, HH:MM, HH:MM:SS, AM/PM; up to 4 sections with conditions and colours; General shrinks to fit or shows ###.

## Kept

1M-row scrolling, keyboard navigation, edge jumps; undo/redo for every op; volatile NOW/TODAY/RAND/RANDBETWEEN; plain-English formula hint; sheet tabs add/rename/delete/reorder; status bar Sum/Count/Average and zoom; toolbar Undo/Redo/Bold/Italic/$/%/Σ (Sort, Filter, Chart, Share disabled); ⌘K palette.

## Merge order (each step leaves main green)

1. WP0 contracts: workspace, `tally-types`, `tally-model`, `tally-api` DTOs + `DocumentService` trait + ts-rs types, `EngineClient` + `MockClient`, CI.
2. WP2 `tally-formula`: parse, print, explain.
3. WP4 `tally-numfmt`: formatting, input recognition, dates.
4. WP3 core: registry, coerce, criteria, MockEvalContext, 6 functions (SUM, IF, AVERAGE, COUNT, CONCATENATE, ROUND).
5. WP1 `tally-engine` + `DocumentService` impl + `tally-cli`. Gate G1: CLI evaluates formulas end to end.
6. WP6 `@tally/grid` (against MockClient + synthetic 1M-row source).
7. WP7 Tauri app + `@tally/app` chrome. Gate G2: live engine grid in the desktop app.
8. WP5 `tally-io-ods` + `tally-io-csv`. Gate G3: ODS and CSV round trip in the app.
9. WP3 remainder: the rest of the 68 functions with TOML vectors.
10. WP8 `tally-assist`. Gate G4: Phase A exit.

## Done checks

Bindings regenerate with `TS_RS_EXPORT_DIR=$PWD/packages/bridge/src/generated`.

| WP | Check |
|---|---|
| WP0 | `cargo check --workspace && TS_RS_EXPORT_DIR=$PWD/packages/bridge/src/generated cargo test -p tally-api && git diff --exit-code packages/bridge && pnpm -r typecheck` |
| WP2 | `cargo test -p tally-formula` (insta snapshots, CalcA1↔OpenFormula round trip, `=A1+$B$1` copied C1→D5 prints `=B5+$B$1`, Err:501/508/509) |
| WP4 | `cargo test -p tally-numfmt` (≥250 format cases, ≥80 recognizer cases, date round-trip proptest) |
| WP3 core | `cargo test -p tally-functions` |
| WP1 | `cargo test -p tally-engine && cargo run -p tally-cli -- eval --cells 'A1=2,B1=3' '=SUM(A1:B1)*2'` prints `10`; bench gates below |
| WP6 | `pnpm --filter @tally/grid test`; manual fling-scroll of the 1M-row demo without jank |
| WP7 | `pnpm tauri dev`; type 2 in A1, 3 in B1, `=A1*B1` in C1 → 6; hint "A1 × B1"; ⌘B bolds; ⌘Z undoes; add/rename sheet; Sum = 11 for A1:C1 |
| WP5 | `cargo test -p tally-io-ods -p tally-io-csv`; open templates/invoices.ods, edit, save, reopen identically; opens in LibreOffice |
| WP3 full | `cargo test -p tally-engine --test conformance && cargo run -p xtask -- function-coverage --batch 0` → 68/68 |
| WP8 | `cargo test -p tally-assist` (≥60 golden cases); ⌘K "total of unpaid orders" → `=SUMIF(G2:G15,"<>Paid",F2:F15)` |
| Exit | `cargo test --workspace && pnpm -r test && pnpm e2e`; manual WP7/WP5 scripts on macOS arm64; CI bundles green |

## Dependency graph (Phase A): go, with conditions

- BTreeMap per column behind `ColumnStore`; dependents behind `trait DepIndex { add, remove, dependents_of(changed: &CellRange, out) }` in `tally-engine/src/deps.rs`.
- Never expand ranges into cells: area refs stay `(CellRange, FormulaCellId)`; `swap_remove` with a back-index; whole-column/row refs clamp to used area at evaluation time.
- Coalesce changed cells into rectangles before the area scan.
- No Rust recursion in evaluation: collect dirty set, iterative DFS topological order over static precedents, SCCs → Err:522, then evaluate in order.
- Targets: ≤20k area entries/sheet with single-edit ≤50 ms (warn >50k); 1M non-empty cells; readers stop at 10M cells; paste ≤1M cells; get_cells ≤20k cells/call; formula ≤8192 chars, nesting ≤256 (Err:512); undo depth 100. Bench gates (M-series): 100k formulas full recalc <500 ms; SUM(A1:A1000000) <30 ms; 10k-chain edit <50 ms.

## Batch-0 functions (68, AOO categories)

- Mathematical (18): SUM, PRODUCT, ROUND, ROUNDUP, ROUNDDOWN, INT, TRUNC, ABS, SIGN, MOD, POWER, SQRT, RAND, SUMIF, SUMIFS, COUNTIF, COUNTBLANK, AVERAGEIF (AOO files AVERAGEIF under Mathematical)
- Statistical (6): AVERAGE, COUNT, COUNTA, MIN, MAX, MEDIAN
- Logical (6 + 2 extensions): AND, OR, NOT, IF, TRUE, FALSE; plus IFERROR, IFNA, which are NOT in AOO 4.1 (Excel/LibreOffice compatible extensions, flagged `extension = true` in the registry; they write to ODS as COM.MICROSOFT.IFERROR / COM.MICROSOFT.IFNA)
- Text (14): CONCATENATE, LEFT, RIGHT, MID, LEN, UPPER, LOWER, PROPER, TRIM, TEXT, VALUE, FIND, SEARCH, SUBSTITUTE
- Date&Time (8): TODAY, NOW, DATE, YEAR, MONTH, DAY, WEEKDAY, DATEVALUE
- Spreadsheet (6): VLOOKUP, HLOOKUP, INDEX, MATCH, ROW, COLUMN
- Information (6): ISBLANK, ISNUMBER, ISTEXT, ISERROR, ISNA, NA
- Array (1): SUMPRODUCT
- Add-in (1): RANDBETWEEN

CEILING and FLOOR are deliberately deferred (AOO's 3-arg semantics differ from Excel).

The authoritative function list is docs/calc-functions.json (405 entries parsed from the AOO source: 296 built-in, 101 Analysis add-in, 8 Date add-in; NUMBERVALUE is declared but unimplemented in AOO). Notable AOO facts from docs/calc-features-inventory.md: unary minus binds tighter than `^` (=-2^2 is 4), `^` is left-associative, error display mapping is 503 #NUM!, 519/526 #VALUE!, 521 #NULL!, 524 #REF!, 525 #NAME?, 530 #ADDIN?, 531 #MACRO?, 532 #DIV/0!, 32767 #N/A, everything else Err:nnn. Add-in name clashes: plain ISEVEN/ISODD/GCD/LCM/EFFECT/CUMPRINC/CUMIPMT/NOMINAL mean the built-in; plain WEEKNUM/CONVERT/DURATION mean the add-in.
