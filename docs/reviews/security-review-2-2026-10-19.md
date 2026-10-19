# Security review 2 (201e669 -> f6193fa): dynamic results

Worktree: scratchpad/sec2-wt (detached at f6193fa, release build in scratchpad/sec2-target). I added two scratch examples to the worktree only (they are untracked and not in the repo):
- `crates/tally-engine/examples/probe.rs`: `probe open FILE [cells]` (with env `SAVE=1` to time ODS/XLSX/CSV save, `VIEW=1` to time `cells(A1:T1000)`, `STACK=bytes` to run on a small stack), and `probe op <name> N` for engine operations.
- `crates/tally-formula/examples/fprobe.rs`: ExcelA1 depth/length bombs and sheet-name round trips.

Generators are in scratchpad/sec2/ (`ods.py`, `xl.py`, `p*.py`). `run.py` runs each command with a wall-time limit and an RSS limit (it polls `ps` and kills the process past the limit) and reports peak RSS. Every measurement below is a release build on a 24 GB, 12-core Mac. macOS compresses memory, so the RSS figures for highly repetitive data understate the real allocation.

Common impact: the Tauri shell serializes every command on a single service mutex (commands.rs `with_service`). Any hang below therefore freezes every open document, not just the hostile one. The open-time recalc watchdog (20 s / 50M) covers only recalculation. It does not cover load, name expansion or merge cleaning, and it cannot interrupt a single slow formula.

## Confirmed findings

### CRITICAL-1: Names expand exponentially (load and edit)
`crates/tally-engine/src/names.rs:173-212` (`expand`). Each use inlines a clone of the name's AST. Depth is capped at 16, but nothing limits size. Expansion happens per formula cell, when the cell is set or loaded and again on hard recalc.
Generator: `python3 -I p1_names.py OUT.ods FAN 16 NCELLS`. It defines nm_0 = 1 and nm_i = nm_{i-1}+… (FAN copies) as ODS `table:named-expression`, then adds NCELLS cells `=nm_15`. Every file is about 1 KB.
| fan | cells | result |
|---|---|---|
| 2 | 1 | 51 ms, 29 MB |
| 2 | 1000 | open 14.3 s, plus 15 s recalc_all, 2.47 GB |
| 2 | 100000 | timed out at 60 s, 4.2 GB |
| 3 | 1 | 44 s, 3.9 GB |
| 4 | 1 | timed out at 60 s |
| 10 | 1 | killed at 6 GB RSS after 1.4 s |
The same thing happens through the API: `DefineName` of gx_0..gx_15 (fan 10) followed by setting a cell to `=gx_15` timed out at 60 s with 2.6 GB (`probe op names_cycle`).
Fix: limit the expanded size. Count nodes during `expand` and return #NAME? or Err:512 past about 100k nodes per formula. Better: stop inlining. Evaluate a name as a node that references its own parsed or cached expression, and register its dependencies once per name rather than per use.

### HIGH-1: Overlap check in `clean_merges` is O(n²) (ODS and XLSX)
`crates/tally-engine/src/convert.rs:364-378`.
- ODS (`p2_merges.py OUT N`; one 2-column merge per row, with random comments so the compression ratio stays under 200):
  | merges | file | time |
  |---|---|---|
  | 100k | 0.5 MB | 1.3 s |
  | 200k | 1 MB | 5.2 s |
  | 400k | 2 MB | 20.4 s |
  | 1M | 5 MB | 128.6 s (64 MB RSS) |
- XLSX (`p5_xmerges.py OUT 2000000`; 2M `<mergeCell>`, 10 MB): open took 128.9 s, because the 1M cap in `clean_merges` applies. This also confirms suspected finding 5: XLSX merges are not counted against `max_cells` (`sheet.rs:142-149`). Memory stays small, so the main damage is CPU time.
Fix: sort by (r0, c0) and sweep with an interval structure, or use a per-row occupancy bitmap. Lower the cap to about 100k merges per sheet. Count XLSX merges in `Budget::add_cell` the way ODS does.

### HIGH-2: Array blocks have no workbook-wide cell budget
`crates/tally-engine/src/convert.rs:384-417`, `workbook.rs:521-566`, and the reader at ODS `content.rs:383-397`, which counts only 1 cell per block. Each block is capped at MAX_OP_CELLS, but every covered cell becomes an ArrayPart.
Generator: `p3_arrays.py OUT NBLOCKS 1048576`, a single row of `table:number-matrix-rows-spanned="1048576"` formula cells.
| blocks | file | cells | time | RSS |
|---|---|---|---|---|
| 1 | 891 B | 1M | 0.5 s | 195 MB |
| 5 | 902 B | 5M | 2.7 s | 1.0 GB |
| 20 | 926 B | 21M | 14.2 s | 2.3 GB |
| 20, `of:=1` | — | — | 8.4 s | 3.75 GB |
| 200 | 1.1 KB | 209.7M | open 35.4 s | 5.2 GB |
With 200 blocks, save was still running when the process was killed at 150 s. The model's 10M `max_cells` is bypassed by a factor of 20.
Fix: count each block's area (rows × cols) against `limits.max_cells` in the reader. Add a workbook-wide array-cell budget in `convert.rs`. Check overlap before building the Matrix.

### HIGH-3: Dependency index is O(n²) when many formulas reference one cell (realistic files)
`crates/tally-engine/src/deps.rs:146-150` (`add`: `list.contains(&id)`) and `:177-182` (`remove`: `list.retain`) on `singles[addr]`. Neither suspected finding covers this. A very common pattern triggers it (many cells `=A2*$B$1`).
Generator: `p8c.py OUT N fanin`, giving N rows of `of:=[.$X$1]+r`.
| N | open | recalc_all | RSS |
|---|---|---|---|
| 50k | 0.4 s | 1.5 s | — |
| 100k | 1.45 s | 5.6 s | — |
| 200k | 5.4 s | 23.1 s | 214 MB |
1M is about 10 minutes by extrapolation. A range reference instead (`SUM($X$1:$X$10)`) is linear: 200k takes 0.33 s.
- The same cost hits INDIRECT, whose dynamic dependencies are re-registered on each evaluation. `probe op indirect 100000` (`=INDIRECT("A"&(ROW()+$D$1))`): setup 7.6 s, and each edit of D1 takes 6.0 s. At 1M rows it timed out at 120 s.
- It also explains suspected finding 8. The sheet-name lookup is linear, but most of the measured time is fan-in. 10k sheets × 100k refs took 66.5 s to open and 11.2 s for recalc_all (ODS, 25 KB file); XLSX with 9,900 sheets took 75 s. The same 100k refs with 100 sheets took 3.3 s plus 11.2 s.
Fix: make `singles` values a set (`HashSet` or `IndexSet`), or store each formula's position in `back` so removal is a `swap_remove`. Deduplicate per formula using `locs` instead of the shared list.

### HIGH-4: Long number formats make the viewport call take minutes (XLSX and ODS)
Neither the format code nor the formatted output has a length limit. `tally-io-xlsx/src/styles.rs:257-266` keeps `formatCode` at any length. On the ODS side the cap is 4 KB per `<number:text>`, but nothing caps the number of parts. `ops.rs:342` (`SetNumberFormat`) does not cap the code either. Formatting produces a string the size of the code for every visible cell.
Generator: `p9_numfmt.py OUT 1048576 KIND` (1 MB code, 1000×20 cells), run with `VIEW=1`:
| KIND | file | `cells(A1:T1000)` | RSS |
|---|---|---|---|
| zeros | 82 KB | 40.8 s | 1.1 GB |
| quoted | 528 KB | 84.8 s | 2.1 GB |
| esc | 82 KB | 30.2 s | 3.4 GB |
| star | — | 19 s | — |
| frac | — | 20 s | 1.9 GB |
XLSX save of these files also takes about 4.3 s. ODS with 256 × 4,000-character `<number:text>` (438 KB file): `cells()` took 53.5 s and 1.4 GB. `get_cells` runs on every scroll, so the app becomes unusable.
Fix: cap the code at 255 characters (Excel's limit) or about 1 KB, both on import (XLSX numFmt, ODS total text across parts) and in `SetNumberFormat`. Also cap a formatted cell's display text, for example at 32,767 characters or at what fits the column.

### HIGH-5: AutoFilter value lists are O(rows × values), and the filter re-applies on every edit
`crates/tally-engine/src/filter.rs:127-131` (`passes`): `values.iter().any(|x| x.to_lowercase() == t)`. This lowercases and allocates for every row × value pair. The filter is re-applied for every edit inside the filtered range.
Run with `V=<values> probe op filter <rows>`:
| rows | values | SetColumnFilter | each edit in range |
|---|---|---|---|
| 100k | 100 | 143 ms | 140 ms |
| 100k | 10,000 (= MAX_UNIQUE, a normal "select all but one") | 11.9 s | 12.2 s |
| 1M | 100 | 1.7 s | 1.7 s |
| 1M | 10,000 | 132 s | — |
| 100k | 1M (no length limit on the Tauri argument) | 71.9 s | — |
Fix: build a `HashSet` of lowercased values once per apply, and limit `values.len()` (for example to MAX_UNIQUE + 1). On an edit, re-evaluate only the changed rows.

### MEDIUM-1: XLSX shared formulas inflate about 450× (9.6 MB file to 4.4 GB, 36 s)
`crates/tally-io-xlsx/src/sheet.rs:451-475`. Each member is printed as a fresh String and then parsed again by the engine.
Generator: `p6_shared.py OUT 1000000 LEN`, one anchor plus 1M `<f t="shared" si="0"/>` members.
| LEN | result |
|---|---|
| 150 | open 21.5 s (above the 20 s "watchdog"), recalc_all 13.3 s, 4.37 GB |
| 240 | open 35.8 s, 4.4 GB |
| 300–2000 | rejected by the 1 GiB text budget after about 9–10 s and 1.1–1.3 GB |
| ≥4000 | anchor fails to parse; members keep their cached values (0.8 s) |
Fix: share one parsed AST per group in the model. A formula-group representation, or at least `Arc<str>` keyed by group with relative printing deferred, both work. Lower the formula-text budget (for example to 256 MiB). Cap MAX_FORMULA_TEXT at 8,192 so it matches the parser.

### MEDIUM-2: XLSX `<col style>` creates 16,384 StyleRuns per element
`crates/tally-io-xlsx/src/sheet.rs:297-306`. Generator: `p4_cols.py OUT N`, N × `<col min="1" max="16384" style="1"/>`.
| N | file | time | RSS |
|---|---|---|---|
| 1,000 | 6.6 KB | 0.5 s | 263 MB |
| 10,000 | 49 KB | 5.8 s | 3.5 GB |
| 100,000 | 478 KB | timed out at 60 s | 3.9 GB and growing |
Fix: emit one run per (c0..c1, style). The model's `StyleRun` needs a column range, or the runs should be expanded lazily. Count runs against a budget the way ODS `count_runs` does, and merge duplicate or overlapping `<col>` entries.

### MEDIUM-3: One formula's cost is unbounded and cannot be interrupted
`=SUM(MMULT(MUNIT(1448)+1;MUNIT(1448)+1))+…` repeated 20 times (a formula under 1 KB) took 46.4 s in `tally-cli eval` with 199 MB. In an ODS with 1000 such cells (`f/mm.ods`), the open watchdog stopped after 1 formula, but that one formula already took 46 s. A later hard recalc would take about 12 hours. A single MMULT at 1448 takes 2.3 s; MINVERSE(MMULT(1448)) takes 3.3 s.
Fix: give evaluation a cooperative budget (a fuel counter or deadline checked inside the heavy matrix loops and array lifts), and expose cancellation through the Tauri layer.

### MEDIUM-4: No per-transaction limit on total work
MAX_OP_CELLS applies per op, but a Transaction may contain any number of ops (Tauri `apply`, commands.rs:200).
- 64 × `SetArrayFormula` of 1M cells in one transaction: 11.4 s apply, 14.6 s undo, 1.7 GB.
- 20 × `FillRange` of 1M: 19.8 s.
- ReplaceAll with a 4 KB replacement over 1M cells: 4.8 s and 5.9 GB peak. The engine has no cell-text cap anywhere (grep finds no 32767/MAX_CELL_TEXT), so 1M × 4 KB is held in memory.
- ReplaceAll in formulas that grows 1M formulas to about 6,000 characters each timed out at 120 s.
Fix: limit total cells touched per transaction (for example 4M) and the op count. Cap the replacement string and cell text (Excel uses 32,767). Budget the total bytes ReplaceAll produces.

### LOW-1: ODS writer is quadratic per row in merge and anchor count
`crates/tally-io-ods/src/write.rs:516-552` (`covered`, `anchor` and `next_boundary` scan every merge in the row for each segment). Saving 100k merges laid out 8,000 per row (from `p5_xmerges.py`) as ODS took 3.6 s; XLSX save took 0.1 s. It is bounded by 16k merges per row, but saving the 1M-merge file costs tens of seconds. Fix: sort covered and anchor lists once per row and walk them with a cursor.

### LOW-2: Sheet names from imported files are not validated, and ExcelA1 parses `'a:b'!A1` as a 3D reference
Only `ops.rs:239` (`check_sheet_name`) rejects `[]*?:/\` and leading or trailing `'`. Import (`from_data`) accepts any name. In an XLSX with a sheet named `a:b`, `='a:b'!A1` evaluates to #VALUE! instead of 100, and `SUM('a:b'!A:A)` gives Err:501 (`f/sn.xlsx`). The fprobe round trip shows `'a:b'!A1` printing as `='a:b'!A1` and re-parsing as `a:b!A1`; with A:A or 1:1 the re-parse fails. The same workbook saved to XLSX contains names Excel rejects. Fix: sanitize imported names with the SheetNamer and the same character rules (replace bad characters, then make the name unique). In the ExcelA1 lexer, treat a quoted name as a single sheet.

### LOW-3: A 1 MB find pattern uses about 640 MB while compiling
`find.rs`: a 1 MB literal, regex or wildcard query is rejected by the regex size limit after about 200 ms, but peak RSS is 640–740 MB. Fix: reject `q.text.len()` over about 8 KB before compiling, and limit the `replace` length at the same point.

### INFO
- `calcPr iterateCount` (suspected finding 7): accepted at any u32 (`xlsx/workbook.rs:193-203`) and written back to ODS verbatim. The engine never reads `iterations` (cycles give Err:522), so this is not exploitable today. Clamp it to 32,767 before iteration support lands.
- Sheet lookups are linear in both `XlsxResolver::sheet_by_name` (`xlsx/formula.rs:47-52`) and `Workbook::sheet_by_name_ci` (`workbook.rs:251-258`, which also calls `to_lowercase()` twice per sheet per lookup). They add to HIGH-3. Use a HashMap keyed on the case-folded name.
- The v0.2 Tauri commands (`names`, `filter_info`, `find`, `detect_header`, `recalc_all`; `commands.rs:270+`) are not registered in `lib.rs generate_handler!`, so they are unreachable today. When they are registered, add argument-size checks (find text, filter values, name strings). The Tauri layer validates no argument sizes at all.
- `wb_open_dialog` filters to ods/csv only, though the engine sniffs XLSX. The Save dialog labels `.xlsx` as "OpenDocument Spreadsheet". With `format: Xlsx` and a user-typed `.ods` name, XLSX bytes are written under `.ods`. These are functional issues, not security issues.
- localStorage holds `tally.recentSeen` (recent file paths with timestamps) and the theme. It is low sensitivity and stored per-origin.

## Refuted or not reproduced
- Suspected finding 7 (iterateCount): no effect, see INFO. The circular reference test gives Err:522 in 7 ms.
- Deep ASTs through nested names (16 levels × 1,000 unary minus, or 250 parens, giving about 16k depth) on a 2 MiB stack (the tokio blocking-thread size): no stack overflow. Open, recalc and save all finish in under 30 ms.
- ExcelA1 parse bombs (fprobe, 2 MiB stack): parens, unary chains, `%` chains, operator chains, whitespace intersections, unions, nested calls, array literals and range-op chains all stop at Err:512 in under 0.5 ms. Strings, quoted sheets, unknown functions, 3D refs and UTF-8 at 8,192 characters parse, print and re-parse identically in under 1 ms. Clone, drop, Debug and explain are fine.
- Function loops from the brief, timed with `runfns.sh fns.txt` (results in `sec2/fns.out`). Each of these finishes in under 0.25 s and under 22 MB, or returns an error at once: BASE(1;2;2147483647), ROMAN(3999;4), BAHTTEXT(1E14), FACTDOUBLE(1E9), MULTINOMIAL(1E6;1E6), COMBIN(1E9;5E8), GCD(1E308;3), IMPOWER("1+i";1E9), BESSELJ(1E6;100), HYPGEOMDIST(1;1E9;1E9;1E9), PERMUTATIONA(1E9;1E9), DEC2BIN(1;1E9), NETWORKDAYS(1;3652425), WORKDAY(1;±1E9), EDATE(1;1E12), TEXT(1;REPT("0";8192)), FIXED(1;2147483647), and nested REPT, CONCATENATE, SUBSTITUTE and `&`. The 4 MiB text cap (Err:513) holds. PERMUT(1E9;5E8) took 0.6 s. MUNIT stops at #VALUE! above about 1448², which is what keeps MMULT and MINVERSE at a few seconds each (see MEDIUM-3).
- Not run: the range-statistics functions over 1M cells (LINEST, MODE, MEDIAN, PERCENTILE, DSUM on A:A, SERIESSUM over a range) were not completed in this pass. Run them next with `probe fn N seq|rand|xy '<formula>'`. SEQUENCE and TEXTJOIN do not exist (#NAME?).

## Verified OK
- Engine operations: SortRange is capped at MAX_OP_CELLS (349,525 rows × 3 columns with formulas takes 1.1 s, undo 0.6 s). MoveRange of 1M cells takes 0.3 s, undo 0.27 s; the whole sheet is refused. FillRange of 1M rows takes 1.2 s; oversize and two-direction fills are refused. SetArrayFormula over the whole sheet is refused. A single 1M SetArrayFormula takes 0.14 s. Name cycles nA→nB→nA give #NAME? instantly, and name-to-cell cycles give Err:522. `filter_info` on 1M unique values takes 0.4 s (MAX_UNIQUE caps it). Regex find over 1M cells: catastrophic patterns take about 115 ms (the regex crate is linear), and nested repetition is rejected by the size limit.
- Frontend: there are no innerHTML, dangerouslySetInnerHTML, eval, new Function, window.open or document.write sinks in packages/ or apps/desktop/src. HTML paste only extracts the token with the regex `data-tally-clip="([^"]+)"` (controller.ts:721,737). No DOMParser is used and HTML is never rendered. Engine clipboard HTML is escaped (`clipboard.rs:160`). HYPERLINK only returns text, and there is no URL-open path. The CSP is strict (`default-src 'self'`, no remote connect), `withGlobalTauri` is false, and the capability file grants only event and window-title/drag permissions (no fs, shell or http).
- Tauri open allowlist (files.rs): the path as given must be in the dialog allowlist or the recent list before the file system is touched. The canonical path is then re-checked. Save targets come from the dialog or the remembered path. Panics are caught per call. The release profile does not set panic=abort, so `catch_unwind` works.
- Re-verification of the first review's inputs (current build, `probe open`):
  - formula_clone.ods: rejected in 3 ms, 7 MB (text budget). Fixed; was 2 GB.
  - sheets1000.ods: 1 ms. Fixed; was 6.2 s.
  - many2m.ods: rejected in 24 ms at 105 MB. Fixed; was 822 MB.
  - slow.csv: 0 formulas, 1 ms. Fixed: CSV is no longer evaluated.
  - csvinj.ods saved as CSV: `'=HYPERLINK(...)+5`, which is prefixed. Fixed.
  - bigattr.ods: 0.57 s, but still 818 MB peak. Not fixed for memory: a 400 MiB attribute on a skipped element is still materialized. Cap attribute and text length in the XML reader for skipped elements.

## Priority order for fixes
CRITICAL-1 (name expansion), then HIGH-3 (deps fan-in, which also hits ordinary files), then HIGH-2 (array budget), HIGH-1 (merges), HIGH-4 (format length), HIGH-5 (filter), then the MEDIUMs. Add each generator to tests/fixtures/*/gen_fixtures.py with a small N and assert both time and an error or cap.

## Housekeeping
Fixtures over 1 MB were deleted. scratchpad/sec2/f now holds 3.1 MB. No repo files were modified and nothing was committed. The worktree has only the two untracked `examples/` directories.
