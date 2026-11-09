# Review round 5: security (220e1b1..0a72570 plus f40134b)

Scope: `git diff 220e1b1 0a72570` plus `git show f40134b` (chart data table, on branch `worktree-agent-a61844ad16295deed`). I made no edits in the repo. Probes ran on `git archive` copies in `scratchpad/review5/tree` (f40134b) and `scratchpad/review5/tree0` (0a72570), release build, this Mac (12 cores, 24 GB). The probe sources are `tree/crates/tally-engine/tests/review5_probe.rs`, `tree0/crates/tally-engine/tests/review5_styles.rs` and `review5/sim.rs`.

## HIGH-1: a small XLSX makes `charts()` build a table of up to 250M slots and send it to the webview (f40134b)

- **Where:**
  - `crates/tally-engine/src/charts.rs:1207`: `to_api` always attaches `data_table: has_own_data(c).then(|| data_table_of(c))`.
  - `crates/tally-engine/src/charts.rs:1237-1290`: `data_table_of` pads every series' values, and also its X and size columns for XY and bubble charts, out to the longest column (`resize(rows, None)`).
  - `charts()` is called by `objectFeatures.refreshObjects` every time the sheet's objects are refreshed, which includes opening the file.
- **Why the budgets do not stop it:** `MAX_LITERAL_POINTS` (1M per document) counts the slots that actually exist. Series with no values count as 0. So one chart can have 32,000 literal categories and 255 empty series and still cost only 32,000 points. Padding turns that into 255 × 32,000 slots, 3× that for bubble charts.
- **Trigger:** an XLSX with one chart part (`<c:barChart>`).
  - Series 0 has `<c:cat><c:strLit><c:ptCount val="32000"/>` and 8,000 `<c:pt idx="4k">` entries. `literal_limit` allows 4×pts+64 slots, so this gives 32,000 slots.
  - Series 1–254 are empty `<c:ser>`s with `<c:val><c:numLit/></c:val>`.
  - 40 `xdr:graphicFrame` anchors point at that same part. The literal budget keeps the data on 31 of them.
  - The file is **26 KB**. Generator: `package()` in `review5_probe.rs`.
- **Measured (opening the file is 19 ms):**

  | Charts | `charts()` | Padded slots | JSON for IPC | Peak RSS |
  |---|---|---|---|---|
  | 1 | 0.12 s | 8.16M | 41 MB | 195 MB |
  | 40 anchors (31 with data) | 1.1 s | 253M | **1.27 GB** | **2.9 GB** |
  | 4 bubble charts | – | 98M | 490 MB | 2.1 GB |

  Scaled to 31 bubble charts, that is roughly 12–16 GB of RSS and about 3.8 GB of JSON. The webview then has to parse JSON of that size, and this repeats on every object refresh.
- **Related renderer cost:** `ChartDataTableDialog.tsx:361` renders all `rows × columns` cells with no virtualization (32,000 × 256 ≈ 8M `<td>`) if the user opens Data Table on such a chart.
- **Fix:**
  - Take `data_table` out of `charts()`. Serve it only from a separate `chart_data_table(doc, sheet, id)` query that the dialog calls.
  - Refuse that query, and disable the dialog, when `rows × (1 + columns)` exceeds a cap such as 100k–1M slots. Alternatively, send unpadded columns and pad on the client.
  - Virtualize the dialog grid.
  - Optionally drop the trailing all-empty series of own-data charts on load, or count `rows × series` against `MAX_LITERAL_POINTS` in the readers.

## HIGH-2: saving an own-data chart as XLSX duplicates the categories per series and patches them in quadratic time (f40134b)

- **Where:**
  - `crates/tally-io-xlsx/src/chart_write.rs:471`: every series calls `literal_range(cat)`, which re-renders the full `c:strLit` of the chart's categories and stores another copy in the thread-local `LITERALS` (line 65).
  - `chart_write.rs:129-141`: `patch_chart_literals` is called once for every `xl/charts/chartN.xml` (`write_v03.rs:1270`). For each part it loops over **all** literals of the whole save. For each literal it builds the placeholder string, runs `contains` over the growing part, and calls `out.replace`, which copies the whole part every time.
- **Trigger:** the HIGH-1 file, or any file with own-data charts. Use File ▸ Save on the opened `.xlsx`, or Save As XLSX.
- **Measured:**

  | Charts | Time | Peak RSS | Output |
  |---|---|---|---|
  | 1 | 8.6 s | 2.5 GB | 5.5 MB |
  | 2 | 22.8 s | 3.0 GB | – |
  | 4 | **89 s** | 3.8 GB | 22 MB |

  The growth is quadratic, so 31 charts extrapolates to more than an hour.
- **ODS save (for comparison):** the same file with 4 charts saves in 0.42 s. `write_own_table` is dense but linear.
- **Fix:**
  - Patch in one pass: scan for `<c:numRef><c:f>_TallyLit[` and `<c:strRef><c:f>_TallyLit[`, parse the index, and splice `literals[index]` into a new buffer. Keep the literals per chart part so a part never scans the others' markers.
  - Render the category literal once per chart (`Rc<str>`/`Arc<str>`) instead of once per series.
  - Budget the literal XML bytes per save, for example `series × categories` against `MAX_LITERAL_POINTS`. Past the budget, drop the later series' `c:cat` and add a warning, as `CACHE_BUDGET` already does for cached cells.

## MEDIUM-1: Load Styles is quadratic in the source's style count (0a72570)

- **Where:** `crates/tally-engine/src/loadstyles.rs`.
  - Line 147: `picked.sort_by_key(|s| depth(&src, &s.name))`. The sort calls the key O(n log n) times, and each `depth` (line 91) makes up to 64 linear `parent_of` scans.
  - Line 191: the same pattern over `work`.
  - In the per-style loop, `lookup`, `loops` (64 scans) and `put` are all linear scans.
  - `table_of` (line 131) is O(n²).
  - Up to 10,000 source styles are allowed: `named_styles().take(10_000)` in the ODS reader, and `MAX_CELL_STYLES`.
- **Trigger:** an ODS whose `office:styles` holds 10,000 `table-cell` styles in one parent chain (style *i*'s parent is style *i−1*), each with a 245-character name that shares a 240-character prefix. The file is 74 KB.
- **Measured:**
  - Opening the file takes 37 ms.
  - Format ▸ Load Styles from it takes **52 s**, holding the target document's lock the whole time. The UI for that document is frozen.
  - With short names it takes about 15 s (`sim.rs`).
- **Fix:**
  - Build `HashMap<&str, usize>` indexes for `src` and `work`.
  - Compute every depth once with memoization and use `sort_by_cached_key` (or sort a precomputed `Vec<(depth, idx)>`).
  - Make `loops`, `lookup` and `put` use the map, and make `table_of` use a `HashSet`.

## LOW-1: undo depth up to 1000 with no undo memory budget (0a72570)

- **Where:**
  - `crates/tally-engine/src/undo.rs:19`: `UNDO_DEPTH_RANGE = 1..=1000`.
  - `set_undo_depth` (`state.rs`, `service/mod.rs`) is reachable from any window through IPC (`allow-set-undo-depth`).
- **Issue:** the stack is capped by step count only. The memory budgets in `security.md` were measured with 100 steps, for example "the undo stack keeps each previous table" for pivots and the Replace All undo records of up to 10M cells per transaction. A depth of 1000 raises the worst-case retained memory 10×. AOO has the same knob, so this is mostly a documentation gap, but a semi-trusted renderer can set it silently.
- **Fix:** add an approximate byte weight per `UndoRecord` and trim the oldest steps past a cap such as 1–2 GiB. At minimum, document the 10× factor in `security.md`.

## LOW-2: XLSX literal text is not filtered for characters that are illegal in XML (f40134b)

- **Where:** `crates/tally-io-xlsx/src/chart_write.rs:79`. `xml_text` escapes only `& < >`.
- **Issue:** categories and series names from `SetChartDataTable` pass `limits::data_table`, which checks byte length only, and `charts::text_opt`, which checks character count only. A category such as `"a\u{0001}b"` is written as `<c:v>a\x01b</c:v>`, which is not well-formed XML 1.0. Excel reports the workbook as corrupt.
  - Everything else in Tally's XLSX output goes through rust_xlsxwriter, which writes such characters as `_xHHHH_`. Only this new patch path produces them.
  - There is no injection: `<` and `&` are escaped, and the placeholder must be the exact `<c:numRef><c:f>_TallyLit[n]</c:f></c:numRef>`. A sheet reference always prints with `!` and sheet names cannot contain `[`, so user data cannot match it.
- **Fix:** strip, or `_xHHHH_`-escape, characters outside the XML Char production in `xml_text`. Optionally also refuse control characters in `limits::data_table`.

## Checked, no finding

- **`load_styles_from_template` (`templates.rs`):**
  - `user:<stem>` goes through `check_name`, which rejects `/ \ : * ? " < > |`, a leading dot, control and format characters, and names reserved by Windows.
  - The path is then canonicalized and must start with the canonical templates folder, so symlinks that leave the folder are refused.
  - Ids that are not user templates go only to `templates::build`, which accepts three fixed ids.
- **`load_styles_dialog`:** reads only the path returned by `blocking_pick_file`; JS never supplies a path. The label is capped by `chars_max`.
- **`decode_file`:** same 512 MiB, codec and `MAX_SHEETS` limits as Open; read-only documents are refused before the read; no recalc is done.
- **ACL:** the 6 new commands appear in `APP_COMMANDS`, `COMMAND_NAMES`, `default.json` and `lib.rs`; none is over-granted.
- **`undo_steps` / `redo_steps`:** a `u32` count is bounded by the stack (≤ 1000 iterations).
- **`undo_history`:** returns at most 1000 labels per stack.
- **`set_undo_depth`:** range-checked. `set_depth` clamps, and `trim_oldest` uses saturating arithmetic.
- **DataPilot source filter:**
  - The ODS reader keeps 8 conditions (out of at most 64 parsed). Columns outside the source are dropped, and `width` cannot underflow because `parse_range_list` normalizes the range.
  - `from_data` drops values over 4 KiB.
  - The API checks ≤ 8 conditions, ≤ 4096 bytes and `col < src.cols()`.
  - The engine clamps `col` to the source.
  - Regex goes through `build_regex`, using the Rust `regex` crate (linear time) with 1 MiB size and DFA limits. There is no ReDoS.
  - It costs one `visible_rows` pass per refresh.
- **ODS `table:range-usable-as`:** only fixed tokens are written. `settings.xml` view items are capped at 16 each, and `GridColor` parsing is safe.
- **XLSX `showSheetTabs` / `showOutlineSymbols` patches:** fixed strings only.
- **`SetNameUsage`:** name length checked in `limits.rs`, needs an unprotected document structure, and snapshots the name table once per transaction.
- **Fill copy modifier and filtered AutoFill:** no indexing hazards; the filtered-row set is cloned once per fill.
- **Own-data `local-table` reading (`LocalTable::cells`):** at most 32,000 cells per address, rows clamped to the table, 64 address parts, and every list counted against the literal budget.
- **`SetChartDataTable` from IPC:** every column must have exactly `rows` entries, so a dense table counts fully against `MAX_LITERAL_POINTS`. Only file-loaded charts reach the padding in HIGH-1.
- **`set_chart_data_table` budget arithmetic:** the subtractions cannot underflow, because `here` includes the edited chart.
