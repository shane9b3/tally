# Review round 5: correctness (220e1b1..0a72570 plus f40134b)

Six findings: no High, two Medium, four Low. Each was confirmed by tracing the code at the stated commit. Nothing was built or run.

## Medium

### M1. Chart Data Table: "Move Series Right" throws away a cell edit that is still open (f40134b)
- **Where:** `packages/app/src/charts/ChartDataTableDialog.tsx` (f40134b), `onMoveSeriesRight` (about lines 214-224) together with `run()` (about lines 190-201).
- **Cause:** `run()` first calls `commitEdit()`, which queues `setTable(tb => setCell(...))`. It then runs `f()`. In `onMoveSeriesRight`, `f()` computes `moveSeriesRight(table, curSeries)` from the `table` captured at render time, which does not yet hold the edit, and calls `setTable(next)` with that plain value. React applies the updater and then replaces the result with `next`, so the edit is gone. The other tools pass updater functions and are not affected. The button stays enabled during a valid edit (`busy` is only true when the edit is invalid), and the cell input has no `onBlur` commit, so a mouse click reaches this path.
- **Reproduction:** open Data Table on an own-data chart with two series. In a series-1 cell, type `42` and do not press Enter. Click "Move Series Right". The series moves but the cell holds its old value, and closing the dialog saves the table without 42.
- **Fix:** use an updater, `setTable(tb => moveSeriesRight(tb, curSeries))`, and compute `nextCols` from the same value. Alternatively, have `commitEdit` return the committed table and pass it into `f`.

### M2. A filter on a pivot whose source is a database range is accepted, then dropped without warning on ODS save
- **Where:** `crates/tally-io-ods/src/tools.rs` `write_pivot` (about lines 1266-1293 at 0a72570) and `parse_pivot`; engine `crates/tally-engine/src/pivot.rs` `source_filter_from_api` / `api_source_range` (line 741).
- **Cause:** the engine and the UI (`w56.openPivotFilter` handles `p.source.type !== "range"`) both allow a source filter on a pivot built from a database range, and it is applied on refresh. The ODS writer only writes `table:filter` inside `table:source-cell-range`. For the database-range case it writes `<table:database-source-table .../>` with no filter and no warning. The XLSX path does warn.
- **Reproduction:** define database range `Data` on A1:C20. Create a pivot from source `Data`. Click its Filter button and set Region = East. Save as .ods and reopen: the pivot shows every row and `sourceFilter` is null.
- **Fix:** for a database-range source, write the filter (field numbers = sheet columns of the database range) or at least add a save warning, as `pivot_patches` does for XLSX. Alternatively, refuse `sourceFilter` for database-range sources in `source_filter_from_api` and hide the button.

## Low

### L1. Scatter and bubble data tables turn "no X values" into an all-empty X column, which removes the trend line (f40134b)
- **Where:** `crates/tally-engine/src/charts.rs` (f40134b) `data_table_of` (about line 1260): `x: (xy || s.literal_x.is_some()).then(...)`; also `apply_data_table` (about lines 1349-1351).
- **Cause:** for an XY series without X values (X is then the index), the table gets an X column full of `None`. After any edit in the dialog, `apply_data_table` stores `literal_x = Some(Literal([None; n]))`. `resolve_numbers` then returns `x = Some(all None)`. The trend fit skips every point (`None => continue`), so the trend line disappears. ODS and XLSX now also write an empty X domain / `c:xVal` numLit. Bubble sizes behave the same way (`sizes` all `None`).
- **Reproduction:** take a scatter chart pasted from another document whose source had no X range, or an XLSX scatter with only `c:yVal` numLit, with a linear trend line. Open Data Table, change one Y value, close. The trend line is gone.
- **Fix:** in `apply_data_table`, keep `literal_x` / `literal_sizes` as `None` when the source series had none and the column is all `None`. Or omit `x` in `data_table_of` when `literal_x` is `None`.

### L2. Load Styles can record an empty undo step and mark the document modified when nothing was loaded
- **Where:** `crates/tally-engine/src/loadstyles.rs` `load_cell_styles` (`let mut work = table_of(&old)` … `work.retain(..)` … `if work == old`).
- **Cause:** `table_of` puts stored built-ins first. A document whose stored list has a redefined built-in after a user style (for example `[Mine, Heading]`, which `cellstyles::define` produces when "Heading" is edited after "Mine" is created) gets a reordered `work`. `work == old` is then false even though no style changed. The result is a "Load Styles" undo step that does nothing, `dirty = true`, and a reordered stored list.
- **Reproduction:** create style "Mine", then modify the built-in "Heading". Load Styles from the Budget template with Cell Styles on and Overwrite off. Do it a second time: the second load adds nothing, but Undo shows "Load Styles" and the title bar shows the document as modified.
- **Fix:** compare order-insensitively (for example sorted by name), or rebuild `work` in `old`'s order and append new names.

### L3. Copying a sheet loses the usage flags of its sheet-local names (T20)
- **Where:** `crates/tally-engine/src/tools.rs` (copy sheet, about lines 2558-2582): `wb.names.define(&n, Some(new), def)` creates entries with `NameUsage::default()`.
- **Cause:** only the formula is copied. AOO copies `ScRangeData` together with its type bits.
- **Reproduction:** define the sheet-scoped name `PA` on Sheet1 with "Print range" ticked. Copy Sheet1. The copy's `PA` has no flags, and Page Setup on the copy does not list it.
- **Fix:** collect `e.usage` together with the definition and call `wb.names.set_usage(id, usage)` after `define`.

### L4. XLSX writer can place `c:strLit` inside `c:val`, which the schema does not allow (f40134b)
- **Where:** `crates/tally-io-xlsx/src/chart_write.rs` (f40134b) `add_series`, which calls `literal_range(s.literal_values)` and `literal_xml`.
- **Cause:** `chart_read::data_ref` returns `LiteralText` for a `strLit` in `c:val` / `c:yVal`. Before this commit such a series was dropped. It is now written back as `<c:val><c:strLit>…`, but `CT_NumDataSource` allows only `numRef` or `numLit`, so Excel may report the file as damaged. This needs an unusual input file.
- **Fix:** in `literal_range` for values (and sizes), always emit `numLit`, parsing text to numbers as `literal_numbers` already does on the engine side.

## Checked and not a problem
- **Undo / redo:** `UndoStack` trimming (`push` clears redo before `trim_oldest`; `set_depth` alternates farthest redo / oldest undo, which matches `SfxUndoManager::SetMaxUndoActionCount`). `undo_steps` / `redo_steps` selection, one recalc, `push_redo` / `push_undo_keep_redo` keep undo + redo at or under the depth. `Change::DocView` round trip. `SetNameUsage` uses the names snapshot. `SetChartDataTable` and Load Styles undo by meta / style snapshots. Load Styles rolls back on a page-style error. Lazy depth sync on forks (the desktop sets it on every slot).
- **AOO semantics:**
  - Filtered AutoFill (pattern advance only over written rows, lane skip for left/right, up-fill order) checked against `table4.cxx` FillAuto i89232.
  - `range-usable-as` token order matches.
  - DataPilot filter field numbers are sheet columns (`dptablecache.cxx` #i115431, clamped like Tally), and `match` / `nomatch` / `!=` match `getDPOperatorXML`.
  - `ScDocShell::LoadStylesArgs` (create all, then fill; parents by name).
- **Merge interactions:** `GROUP_OPS` (Rust and TS) need no new entries. `SetNameUsage` is in `meta_op_cost` and `limits::typed_op`. `SetChartDataTable` is in `typed_op` and the object-op refresh list. Read-only refusal goes through `editable()` for apply, `undo_steps`, `redo_steps` and `load_styles`, and through `assertEditable` / apply in the mocks.
- **Merging f40134b:** it branches from f644c12, so it will conflict textually in `apps/desktop/src-tauri/src/limits.rs` `typed_op` (the `SetNameUsage` arm) and in the `docs/parity-deviations.md` T17 row. f40134b still has the old T17 "N" row.
