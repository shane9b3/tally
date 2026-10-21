# Security review 4 (e58a2bb -> b03e603): wave 5 (charts, pictures, DataPilot, outline/data tools, print/PDF, calc semantics, importers, shell)

Worktree: `scratchpad/sec4-wt`, detached at b03e603; it was removed at the end. Target dir: `scratchpad/sec4-target`. Release builds were made with `-j 4`. Five probe crates, all under `scratchpad/sec4/`:
- `probe` (`fixprobe`): the review-3 probe, rebuilt against this commit
- `probe5`: charts, pictures, pivots
- `probe-tools` (`toolsprobe`)
- `probe-print` (`pprobe`)
- `probe-imp` (`impprobe`)

The shell probes are in `shellprobe/`. The generators are `gen5.py` and `gen5x.py` (charts, pictures, pivots), `gen_tools.py`, `print/gen/gen_print.py` and `imp/gen/`. All of them use only the Python stdlib and were run with `python3 -I`. Every measurement went through `sec4/r.sh`, which is `run.py` with a 60 s wall-clock watchdog and an RSS watchdog. The RSS limit was 2–3.5 GB, because four probes ran concurrently, so treat timings as ±30 %. RSS is macOS RSS, which compresses memory and so understates usage. No repo file was modified and nothing was committed. Files over 1 MB were deleted afterwards.

The context from review 3 still applies: every Tauri command runs under the single service mutex, and no engine call can be cancelled. Every multi-second or multi-GB finding below therefore freezes every open document.

## Priority summary (confirmed)

| # | Sev | Finding | Trigger | Measured |
|---|---|---|---|---|
| C1 | Critical | Each frame or anchor gets its own copy of a shared picture. The 256 MiB cap counts unique pictures only (`tally-io-ods/src/frame.rs:596-626`, `tally-io-xlsx/src/drawing_read.rs:379-414`, `tally-engine/src/charts.rs:159-177`). | ODS or XLSX file | 5 KB → 1.33 GB. 166 KB → killed at 3.3 GB in 1.1 s (≈ 17 GB projected). |
| C2 | Critical | Chart literal and local-table data is padded to 32,000 points × 255 series. The shared chart object is re-parsed for every frame (`tally-io-ods/src/chart.rs:1500-1512`, `tally-io-xlsx/src/chart_read.rs:248-288`). | ODS or XLSX file | 7 KB XLSX → 266 MB. 20 KB → killed at 3.7 GB. 144 KB ODS → OS-killed at 2.9 GB (≈ 130 GB projected). |
| C3 | Critical | `chart_data` has no work or size budget. Hidden cells are walked without being counted. The UI re-fetches every changed chart after each edit (`tally-engine/src/charts.rs:1059-1100,1356`, `packages/app/src/objects/objectFeatures.ts:96`). | ODS file (4.8–6 KB) | One `chart_data` takes 1.4 s / 62 MB JSON. With hidden columns it takes > 60 s. One edit with 1,000 charts → ≈ 23 min. |
| C4 | Critical | XLS (calamine 0.37) allocates without bound before Tally's caps run. `catch_unwind` does not catch an allocation failure (`tally-io-xls/src/lib.rs:108-245`). | .xls file (5.6 KB) | SIGABRT, or 1–1.8 GB in < 1 s, or > 3 GB, or a 60 s hang. |
| C5 | Critical | RemoveDuplicates does one shift plus one undo snapshot per run of duplicates (`tally-engine/src/tools.rs:1708-1770`). | Data menu on 10k–100k rows | 10k rows: 3.2 s / 1.67 GB. 30k rows: 43 s. 100k rows: > 3 GB. |
| H1 | High | Print titles are cloned into every page. When they are taller than the page body, each page holds 1 row (`tally-engine/src/print.rs:120-163,653-670`). | 1.5 KB XLSX | > 60 s, 2.2 GB on Print Preview |
| H2 | High | `iterateCount` up to 32,767 is read from the file, with no total budget (`tally-io-xlsx/src/workbook.rs:238-252`, `calcsettings.rs:25`). | 79 KB XLSX | Open takes > 60 s. |
| H3 | High | `MULTIPLE.OPERATIONS` has no budget per recalc (`recalc.rs:1205-1302`). | Formulas | 300 calls over 199k formulas: 23 s per edit (≈ 15 min at the 10k-call cap). |
| H4 | High | Solver linearization is not covered by the timeout. The shell does not cap `timeout_ms` or `max_iterations`. Branch-and-bound recurses (`solver.rs:151-212,497-547,726`, `limits.rs:281-306`). | Solver dialog | 31 s against a 10 s timeout; 53 s with 200k dependents. B&B: > 60 s, 945 MB. |
| H5 | High | Print job cost is O(cells in the print area), with no cell budget (`service/print.rs:428-700`). | 2.7 MB XLSX | 48 s per 100 pages (≈ 9 min). 12.6 s for one API page. |
| H6 | High | XLSX save of a chart over a long range: rust_xlsxwriter clones the cache per series (`tally-io-xlsx/src/chart_write.rs:37-61`). | Saving as XLSX | 8 series × 1M rows: > 3 GB. 1,000 charts: > 60 s. |
| H7 | High | Pivot definitions from files skip the API's 256-field cap. `compute` has no memory budget (`pivot.rs:111-120,1063-1078`, `tally-io-xlsx/src/tools.rs:739-790`). | XLSX file / API | 2,000 × 2,000 fields: killed at 2.6 GB in 2.5 s. 200 fields through the API: 18 s, 2.8 GB. |
| H8 | High | Pivot "show items without data" builds the full cross product (`pivot.rs:879-1010`). | ODS file / dialog | 3 fields × 1,000 items: > 60 s. 8 fields × 1,000 items: > 2.6 GB in 1.1 s. |
| H9 | High | Subtotals and RemoveSubtotals run one InsertRows/DeleteRows per group (`tools.rs:983-1060,1305-1360`). | Data menu | 3k groups: 10 s each way. 10k groups: > 60 s. |
| H10 | High | TextToColumns has no rows × width budget, and `fixed_widths` is uncapped (`tools.rs:1520,1605-1700`). | Data menu | 1 wide row over 100k rows: 23.5 s. 16M cells: 2.3 GB. |
| H11 | High | Consolidate keeps every source address per key. Link formulas are not length-checked (`tools.rs:1373-1505`). | Data menu | 255 × 1M sources: > 3.3 GB. With link: 15.7 s, 2.2 GB. A 25k-char formula gives `Err:512`. |
| H12 | High | Outline groups: the XLSX loader is quadratic, and the ODS writer is O(rows × groups) (`tally-io-xlsx/src/tools.rs:123-170`, `tally-io-ods/src/tools.rs:134-185`, `outline.rs:110-171`). | 1.3 MB XLSX; saving | Open takes > 60 s. A normal 300k-row outline takes 46 s on every ODS save or autosave. |
| H13 | High | HTML rowspan/colspan handling is super-linear (`tally-io-html/src/lib.rs:362-376,418-422`). | 70 KB HTML | > 60 s |
| H14 | High | HTML `<script>`/`<style>` skipping is quadratic (`tally-io-html/src/lib.rs:109-123`). | 17 MB HTML | > 60 s |
| H15 | High | Print SVGs use usvg's default `<image href>` resolver, which reads local files: arbitrary local PNGs get embedded in the PDF, `/dev/zero` grows to 2.25 GB and a FIFO hangs the print. Not reachable from a file today, because ECharts escapes all user text (`tally-io-pdf/src/lib.rs:946-951`). | webview | Measured (the print fork rates the same issue Low, as LOW-P9) |
| M1 | Medium | Pivot auto-show, hidden items and manual order are quadratic (`pivot.rs:961-989,1043-1057`). | ODS file / dialog | Auto-show 1M: 29 s. 4 × 10k hidden items: 17.8 s. |
| M2 | Medium | GETPIVOTDATA scans the whole lookup map with lowercasing (`pivot.rs:2386-2405,2478-2485`). | Formulas | 100k calls: 45 s |
| M3 | Medium | Large pivots within the caps: 1M items take 4.1 s / +600 MB per create or refresh; date grouping 6.6 s / 1.9 GB; a 10M-cell source +920 MB. | Dialog | Measured |
| M4 | Medium | No pixel budget in `rasterize`. Custom paper up to 10 m (ODS page width unbounded) (`tally-io-pdf/src/lib.rs:1080-1104`). | ODS file + preview | 867 MB at 72 dpi; > 3 GB at 150 dpi |
| M5 | Medium | Page caps disagree: the engine allows 100k pages, the PDF crate 10k. A 29k-page job is cached (+500 MB) and then every request for it fails. | Print | Measured |
| M6 | Medium | Chart SVGs anchored in a repeated row are copied onto every page. There is no total cap on SVG size per job (1,000 × 4 MiB). | Print | 286 pages: 1.17 GB; PDF > 60 s |
| M7 | Medium | Crash-recovery snapshots are keyed by a per-process DocId. A later session or a second instance deletes them (`autosave.rs:74-83,138-150`). | Normal use | Data loss (by reading) |
| M8 | Medium | The HTML 64 KiB per-cell cap is not enforced (`tally-io-html/src/lib.rs:389-394`). | 60 MB HTML | One 63 MB cell; 1.45 GB |
| M9 | Medium | `DefineName` snapshots the whole name table per op (`undo.rs:599-601`). | API | 10k ops in one transaction: > 3.6 GB |
| M10 | Medium | Scenarios: the 1M-cell cap is per scenario, × 1,000 scenarios (`scenarios.rs:41-44`). | API | 20 scenarios: 2.9 GB |
| M11 | Medium | No transaction budget for InsertCells / CopySheet / AdvancedFilter. Autocomplete returns full texts (30 MB per keystroke). | API / UI | See part T8 |
| L | Low | Print temp PDFs: 0644, predictable names, never deleted. PDFs kept in memory (4 × 512 MiB). Shell/engine cap mismatches. Autosave/template `save` repoints `&[PATH]`. `pick_image` reads devices and FIFOs. The engine reads up to 512 MiB before the format caps apply. SXC formula rewrite runs before the 8 KiB check. MULTIPLE.OPERATIONS silently returns the unsubstituted value. Trace Precedents: 17 s. ODS writer drops groups past the used range. `charts()` returns 80 MB of JSON. | | See the sections |

The shell/UI section rates H15 High and the print/PDF section rates it Low (LOW-P9). The difference is reachability: a file cannot inject an `<image>` today, but if the webview misbehaves, local files and devices are reachable while the service lock is held. The fix is a one-line `image_href_resolver` that accepts only `data:` URIs.

Section IDs below are local to each part (charts/pivots: C1–C3, H4–H6, M7–M9; tools: T1–T10; print: P1–P11; importers and shell: their own labels). The table above re-numbers them across all parts.

Suggested fix order:
1. C1/C2: share bytes and chart objects, and budget references and literal points.
2. C3: a visited-cell budget and caching in `chart_data`, plus a cap on UI refetches.
3. C4: a calamine pre-scan or a helper process.
4. C5/H9/H10/H11: single-pass rewrites for the data tools.
5. H1/H5: print budgets.
6. H2/H3/H4: calc budgets.
7. H7/H8: validate pivots from files and budget the tree and aggregation.
8. H12: outline indices.
9. Everything else.

Each generator above, run with small N, can go into `crates/*/tests/security4.rs` with both a time assertion and a cap assertion, as in review 3.

---

## Items 1, 2 and 8: charts/pictures, DataPilot, review-3 regression

Harness: `scratchpad/sec4/probe5` (binary `sec4-target/release/probe5`). `probe5 open FILE` with env flags `IMAGES`, `CHARTS[=n]` (charts() plus chart_data for the first n charts of every sheet), `PIVOT` (pivots(), pivot_field_items, RefreshPivot), `EDIT=r,c`, `SAVE` (ODS + XLSX), `REOPEN`. `probe5 op charts N` (env K series, R rows, TREND, FETCH), `probe5 op hidden R` (H0, W, K), `probe5 op pivot ITEMS` (env R rows, F row fields, MOD, SHOWEMPTY, HIDDEN, MANUAL, AUTOSHOW, DATE, COLF, REFRESH, DRILL, GPD/GPDF). Generators: `scratchpad/sec4/gen5.py` (ODS: imgdup, imguniq, charts, chartdeep, hrefs, cellframes, hiddenchart, localbomb) and `gen5x.py` (XLSX: chartdup, imgdup, pivot), stdlib zipfile, run with `python3 -I`. Every run went through `sec4/r.sh` (60 s wall clock, RSS watchdog `MEM` of 2,000–3,500 MB because three other probes ran at the same time; timings are therefore pessimistic by up to ~30 %). "Killed" = watchdog; projections are linear extrapolations of the measured per-unit cost.

### C1 (CRITICAL): one picture referenced by many frames is copied once per frame, and the 256 MiB total cap counts it once
`crates/tally-io-ods/src/frame.rs:596-626` (`pictures` cache: `Some(p) => p.clone()` clones the `Vec<u8>` for every frame; `image_bytes` grows only on the first read), the same pattern in `crates/tally-io-xlsx/src/drawing_read.rs:379-414` (`media.get(...) => m.clone()`), and the engine's `charts::from_data` (`crates/tally-engine/src/charts.rs:159-177`) clones the bytes again and applies no document total (the 256 MiB check exists only in `images::from_api`, `images.rs:55-66`). Limits that do hold: 16 MiB per picture, 1,000 pictures per sheet, 20,000 frames per document, compression ratio 200.

| File (generator) | size | result |
|---|---|---|
| `gen5.py imgdup f/imgdup1000x1m.ods 1000 1 1048576` (1,000 frames → one 1 MiB PNG) | 5 KB | open 0.85 s, **1.33 GB** |
| `gen5.py imgdup f/imgdup1000.ods 1000 1 16777200 400` (1,000 frames → one 16 MiB PNG) | 166 KB | **killed at 3.3 GB after 1.1 s** (projected ≈ 16.8 GB) |
| `gen5.py imgdup f/imgdup20s.ods 1000 20 1048576` (20 sheets × 1,000 frames, 1 MiB) | 62 KB | **killed at 3.9 GB after 0.5 s** (projected ≈ 21 GB) |
| `gen5x.py imgdup f/ximg.xlsx 1000 16777200 400` (XLSX, 1,000 anchors → one `rIdI`) | 177 KB | **killed at 2.5 GB after 0.27 s** (projected ≈ 16.8 GB) |

Impact: a sub-200 KB file exhausts memory on open. Fix: store picture bytes once (`Arc<[u8]>` in `ImageData`, or a per-document picture table that frames index), count every *reference* against MAX_TOTAL_IMAGE_BYTES in both readers (or cap references per picture), and apply the 256 MiB document total in `charts::from_data` as `from_api` does.

### C2 (CRITICAL): chart literal data is padded to 32,000 points per series and every frame re-parses the shared chart object
ODS: `crates/tally-io-ods/src/chart.rs:1500-1512` builds, for each of up to 255 series without a range, a `Vec<Option<f64>>` with one entry per `local-table` row (≤ MAX_LOCAL_ROWS 32,000), even when the rows are empty. XLSX: `crates/tally-io-xlsx/src/chart_read.rs:248-288` pads `numLit`/`strLit` up to the largest `idx` (< 32,000), so one `<c:pt idx="31999">` allocates 32,000 slots. Both readers parse the chart part again for every frame/anchor that points at it (`frame.rs:557-588`, `drawing_read.rs:333-371`), up to 1,000 charts per document, and the engine clones the data again (`charts.rs:108-139 map_chart_ranges`).

| File | size | result |
|---|---|---|
| `gen5.py localbomb f/lb1.ods 1 32000` (255 range-less series, 32,000 empty local-table rows) | 141 KB | open 58 ms, **265 MB**; one `chart_data` 0.2 s, 62 MB JSON, 842 MB peak |
| `gen5.py localbomb f/lb1000.ods 1000 32000` (1,000 frames → that object) | 144 KB | **killed by the OS (SIGKILL) at 2.86 GB after 20 s** (projected ≈ 130 GB) |
| `gen5x.py chartdup f/xch1.xlsx 1 32000 255` (255 `numLit` series, one `pt idx=31999` each) | 7 KB | open 44 ms, **266 MB**; `chart_data` 843 MB peak |
| `gen5x.py chartdup f/xch1000.xlsx 1000 32000 255` (1,000 anchors → `chart1.xml`) | 20 KB | **killed at 3.7 GB after 16 s** |

Fix: a per-document budget of literal points (for example 1M), counted in the readers; keep literal values sparse (or refuse `idx` beyond `ptCount` and beyond a small multiple of the `pt` count); drop trailing empty local-table rows; parse each chart object once per document and share the result (`Arc<ChartData>`), counting shared references against MAX_CHARTS.

### C3 (CRITICAL): `chart_data` has no work or size budget, and the UI re-fetches every changed chart after each edit
`crates/tally-engine/src/charts.rs:1059-1100` (`walk_range`): the 32,000-point cap counts only *emitted* points, so hidden rows/columns are skipped for free; whole columns are cut to the used range only when they are ≥ 1,024 rows / ≥ 256 columns, so `A1:IU1048576` (255 columns) is walked in full. `resolve_data` (`:1356-1515`) runs this for up to 255 series × (values, X, sizes, error bars) × 64 areas each, formats every point (`point_of` → `format_with`), and emits up to 32,000 trend points per series. Nothing is cached (`service/charts.rs:26-36`). In the UI, `objectFeatures.ts:96` calls `refreshData(changed)` with every chart in `chartsChanged` (the DATA_FETCH_CAP of 50 applies only at load, `:64`), and printing fetches every chart of a sheet (`chartFeatures.ts:201-216`). Every call holds the global service mutex.

| Input | result |
|---|---|
| `probe5 op charts 1` (K=255 series over `B1:B1000000`) | one `chart_data` **1.39 s, 62 MB JSON**, +570 MB RSS |
| same with `TREND=1` (linear trend on every series) | **2.68 s, 339 MB JSON**, 670 MB peak |
| `gen5.py charts f/ch1000.ods 1000 255 1000000 0 same` (6 KB: 1,000 frames → one 255-series object) | open 0.38 s; `charts()` 113 ms / **80 MB JSON**; each `chart_data` 1.37 s / 62 MB; one edit in B6 → `charts_changed` lists **all 1,000 charts** (1–5 ms engine side), so the UI issues 1,000 × 1.4 s ≈ **23 min** of locked engine work and 62 GB of JSON |
| `probe5 op hidden 1048576` with H0=0 (columns A:IU hidden, series over `A1:IU1048576`): K=1 / K=8 / K=255 | 0.25 s / 1.96 s / **> 60 s** for one `chart_data` that returns 0 points |
| `gen5.py hiddenchart f/hidch.ods 1000 255` (4.8 KB, the same from a file) | open 0.19 s, first `chart_data` **> 60 s** (the UI asks for 50 of them on open) |

Fix: count every visited cell (and every area) against a per-call budget (for example 2M) and stop with `truncated`; cut ranges to the used range regardless of their size, and skip fully hidden column runs in one step; enforce a total of 32,000 (or 100,000) points per chart across all series; sample trend lines to ≤ 500 points; cache `ChartDataDto` per (chart id, document version); in the UI, cap `refreshData` to the charts in or near the viewport and coalesce refreshes.

### H4 (HIGH): XLSX save of a chart over a long range clones the worksheet data once per series
`crates/tally-io-xlsx/src/chart_write.rs:37-61` passes the stored range straight to rust_xlsxwriter 0.100 (`ChartRange::new_from_range`). On save, `Workbook::populate_chart_caches` (rust_xlsxwriter `workbook.rs:2970-3105`) reads every cell of every range into a `Vec<String>` and clones it into each series that names it.

| File | XLSX save |
|---|---|
| `gen5.py charts f/chk0.ods 0 1 1000000` (no chart, 1M×2 cells) | 1.16 s, peak 1.48 GB, 6.5 MB |
| `f/chk1.ods` (1 chart, 1 series `B1:B1000000`) | 1.87 s, peak 1.50 GB, 11.4 MB |
| `f/chk4.ods` (4 series) | 3.19 s, peak **2.0 GB**, 26 MB |
| `f/chk8.ods` (8 series) | **killed at 3.0 GB** |
| `f/ch1.ods` (255 series) | **killed at 3.1 GB after 21 s** |
| `f/ch1000.ods` (1,000 × 255 series) | **> 60 s, 2.6 GB** (ODS save of the same document: 1.2 s) |

Fix: clamp every written chart range to the sheet's used range and to MAX_POINTS cells (warn when cut), or write the caches yourself (`ChartRange::set_cache`) from a bounded sample; report the per-series clone upstream.

### H5 (HIGH): pivot definitions from files bypass the API's field cap, and the aggregation has no memory budget
`crates/tally-engine/src/pivot.rs:111-120` (`from_data` accepts file definitions as-is; the "at most 256 fields" check exists only in `from_api`, `:380-384`), `crates/tally-io-xlsx/src/tools.rs:739-790` (`rowFields`/`colFields`/`pageFields`/`dataFields` collected without a count limit), and `pivot.rs:1063-1078` (`compute`: per source row, (row fields + 1) × (col fields + 1) × data fields map entries keyed by fresh `Vec<u32>` prefixes; the 4M check runs on the entry count after each row, so memory is O(entries × key length)).

| Input | result |
|---|---|
| `gen5x.py pivot f/xpf2.xlsx 1 2000 2000 1` (2,000 row fields × 2,000 data fields, 3 source rows) | open 2 ms; `RefreshPivot` **killed at 2.6 GB after 2.5 s** |
| `gen5x.py pivot f/xpf.xlsx 1 1000000 500000 1` (6 MB) | open 0.55 s, 324 MB; `pivots()` **233 MB JSON**; refresh killed at 2.5 GB |
| API-legal: `R=20000 F=200 MOD=1 probe5 op pivot 20000` (200 row fields, 1 data field) | CreatePivot **17.9 s, peak 2.8 GB**, 2.0 GB still resident after the refusal ("the pivot result is too large") |

Fix: run file definitions through the same validation as `from_api` (drop the pivot with a warning); lower the axis-field limit to what AOO allows (8 row + 8 column fields is plenty in practice) and data fields to ~64; in `compute`, check `kept × (rows+1) × (cols+1) × data_fields` (or the running byte size of the keys) against a budget before aggregating, and intern prefix keys (one `Vec<u32>` per row, prefixes as slices or hashes).

### H6 (HIGH): "show items without data" builds the full cross product of every field's items with no node budget
`pivot.rs:879-1010` (`build_tree`: `if f.def.show_empty { present = (0..f.items.len()).collect() }` at `:913-915`, recursing for every item into every level); the 4M output check (`:1725`) runs only after the layout's `cells` vector has been built. ODS reads `table:show-empty` (`crates/tally-io-ods/src/tools.rs:1380`).

| Input (`MOD=1 SHOWEMPTY=1 probe5 op pivot N`) | result |
|---|---|
| 3 fields × 100 items, 10k rows | refused after 1.25 s, 760 MB peak |
| 3 fields × 1,000 items, 10k rows | **> 60 s** (1.4 GB) |
| 8 fields × 1,000 items, 1k rows | **killed at 2.6 GB after 1.1 s** |
| without show-empty: 8 fields × 1,000 items, 100k rows (`R=100000 F=8 probe5 op pivot 1000`) | refused after 2.3 s, 900 MB peak |

Fix: before building, multiply the item counts of the axis fields (show-empty ones) and refuse above MAX_OUTPUT_CELLS; count nodes during `build` and stop at the budget; check the row count while emitting lines instead of after.

### M7 (MEDIUM): quadratic pivot options (auto-show, hidden items, manual order)
`pivot.rs:976-989` (auto-show: `present.retain(|i| keep.contains(i))`, O(items × count); `count` from ODS `table:member-count` is any u32), `:1043-1057` (`hidden_items.contains` per source row per field, up to 10,000 names per field from ODS and unbounded from the API), `:961-969` (manual order: `order.iter().position(...)` inside the sort comparator).

| Input (`probe5 op pivot`) | CreatePivot |
|---|---|
| `R=1000000 AUTOSHOW=1000000`, 1M items | **29.4 s** |
| `R=1000000 F=4 HIDDEN=10000`, 10 items per field | **17.8 s** (0.5 s without hidden items) |
| `R=1000000 F=1 HIDDEN=10000` | 3.3 s |
| `R=100000 MANUAL=10000`, 100k items | 1.46 s |

Fix: `HashSet` for hidden items and for the auto-show keep set, a `HashMap<lowercased name, position>` for the manual order, clamp `count` to the item count, and cap `hidden_items`/`order` at MAX_ITEMS in `from_api`.

### M8 (MEDIUM): GETPIVOTDATA scans the whole lookup map with per-entry lowercasing
`pivot.rs:2478-2485` (`lk.values.iter().find(... lower(a) == lower(b))`) and, for the Excel form with item names, `:2386-2405` (`lk.values.keys().any(...)` per field). One call is O(lookup entries) with two `String` allocations per compared item.

| Input | result |
|---|---|
| 100k-item pivot, 100,000 × `=GETPIVOTDATA("v";$Sheet1.$BI$1;"f0";"i5")` (`GPD=100000 GPDF=… R=100000 probe5 op pivot 100000`) | **45 s** (450 µs per call) |
| same pivot, `=GETPIVOTDATA("Sum - v nosuchitem";…)` × 100 | 0.89 s (8.9 ms per call) |

Fix: build a second map keyed by the lowercased (row items, column items, data field) when the lookup is built, and look up in O(1).

### M9 (MEDIUM): large pivots within the caps are slow and memory-hungry
`pivot.rs:543-617` (`read_source` copies every source value and formats every cell), `:1063-1078`, `:1700-1735`.

| Input | result |
|---|---|
| 1M source rows, 1M distinct row items (`R=1000000 probe5 op pivot 1000000`) | CreatePivot 4.1 s (+600 MB), each RefreshPivot 4.4 s, peak 1.8 GB, undo 1.1 s |
| date grouping with all seven parts over 1M rows spanning 100 years (`DATE=1`) | CreatePivot 6.6 s, RefreshPivot 4.9 s, peak 1.9 GB |
| 10M-cell source (1M rows × 10 columns, `F=9`, 2 items) | CreatePivot 1.6 s, +920 MB |
| `pivot_field_items` on 1M rows | 0.8–1.8 s (returns ≤ 10,000 items per field, 0.3 MB) |

RefreshPivot × 10 on a 300k-item pivot stays at ~800 MB (the undo stack does not grow per refresh), and PivotDrillDown of the grand total of a 1M-row source takes 0.78 s. Fix: as H5 (budget), plus skip the refresh when the source range's version is unchanged, and stream the source instead of copying `Value`s.

### L10 (LOW): `charts()` returns every chart DTO of a sheet in one response
`service/charts.rs:14-23`: 1,000 charts × 255 series = 72–80 MB of JSON (`f/ch1000.ods`, `f/hidch.ods`), fetched after every object op and every undo (`objectFeatures.ts:86-91`). Fix: a per-sheet series budget (for example 10,000) or a summary DTO.

### INFO
- XLSX save keeps only one chart per anchor cell: rust_xlsxwriter stores charts in a map keyed by (row, col), so three ODS charts anchored at A1 become one `chart1.xml` (`gen5.py charts f/c3.ods 3 2 100`, then `KEEP=1 SAVE=1 probe5 open`). Data loss, not security. Fix: nudge the anchor offset or warn.
- `charts_changed` itself is cheap: 1,000 charts × 255 series, 1–5 ms per edit (`changed_charts`, `charts.rs:1722-1754`).

### Refuted (items 1 and 2)
- Path traversal through `xlink:href`: `gen5.py hrefs f/hrefs.ods` tries `../content.xml`, `/etc/passwd`, `Pictures/../content.xml`, `Pictures/../../x.png`, `Pictures\..\content.xml`, `Pictures/%2e%2e/content.xml`, `Pictures/.hidden.png`, `file:///etc/passwd`, `http://…`, `mimetype`, `META-INF/manifest.xml`, `Object 1/content.xml`, `Object 1/../content.xml`, `./Object 1/../../content.xml`, `Object 99999`, `Object 00001` on both `draw:image` and `draw:object`. Only `Pictures/a.png` and `Object 1`/`Object 1/` load (`frame.rs:457-476` accepts `Object <1-5 digits>` and a single `Pictures/<name>` component without a leading dot). The XLSX side resolves targets inside the package and keeps `..` on escape so the lookup fails (`workbook.rs:39-62`), and external relationships are filtered (`drawing_read.rs:325-330`).
- Manifest lies: an object declared as a chart with no content.xml, a chart present but undeclared, and a picture outside `Pictures/` are all skipped with one warning ("34 embedded object(s) … were skipped"); a non-PNG/JPEG picture is skipped by magic-byte sniffing.
- Deep chart XML: 100,000 nested `text:span` in a chart subtitle → the whole file is refused ("XML element depth (limit 256)", 0.6 ms); 200 levels load.
- 10k frames per cell: `gen5.py cellframes f/cf10k.ods 10000 1` and `f/cf4k_1k.ods 4096 1000` (4,096 frames repeated over 1,000 columns): 9–13 ms, 23 MB; the reader keeps 4,096 frames per cell and 20,000 per document, the engine 1,000 charts per document; repeated columns do not multiply frames.
- Duplicate object names: 1,000 charts named `c` load and save under that name, no collision.
- XLSX pivotCache with 1,000,000 `sharedItems` (`gen5x.py pivot f/xp1.xlsx 1 1 1 1000000`, 5.4 MB): open 177 ms, 14 MB (the reader skips `sharedItems`). 4,000 pivot tables (`f/xp4k.xlsx`): open 67 ms, `pivots()` 2.6 MB; 10,000 pivot parts hit the 10,000-entry zip cap.
- Pivot undo memory: 10 RefreshPivot in a row on a 300k-item pivot keep RSS flat (~800 MB).
- Trend fits on 32k points are O(n log n): 255 fitted series add 1.3 s (2.68 s vs 1.39 s, ≈ 5 ms per series including serialization); the cost in C3 is the point count and the JSON size, not the regression itself.

### Item 8: review-3 generators re-run at b03e603
`scratchpad/sec3/bench.sh sec4-target/release/fixprobe sec4/regress3.txt` (the review-3 probe, rebuilt against this commit) plus `comments100k.xlsx` regenerated into `sec4/f/`. Every review-3 fix holds, with numbers within noise of the post-fix run (`sec3/after3.txt`):
- CRITICAL-1 (CF): `cf10k.ods` 67–79 ms per viewport; `xcfsame10k.xlsx` 44 ms (2,048-format cap warning); `cf1k_1024.ods` 73–88 ms; `cf10k_8k.ods` 30 ms; `cf_scan 1000000` first fetch 36 ms, cached 1.6 ms (peak 301 MB vs 166 MB before: the larger wave-5 engine; still under 500 MB); `cf_many 10000` capped at 2,048, 91 ms.
- CRITICAL-2 (meta snapshots): `notes 10000` in one transaction 14 ms / 30 MB; 100 SetNote edits on `notes100k.ods` 4–30 µs each, RSS flat at 84 MB; `links 100000` 94 ms / 197 MB; `val_many 10000` 127 ms.
- HIGH-3 (links): `bigtarget.xlsx` 31 ms / 31 MB (targets > 8 KiB dropped with a warning), `links100k_10k.xlsx` 124 ms / 20 MB (100,000-link cap).
- HIGH-4 (validation): `val_list 10000` + FillRange 100k 22 ms; `val_custom 1000000` FillRange 10k 8 ms; `vals10k.ods` FillRange 10 ms.
- MEDIUM-5 (spinCount): 30–31 ms per unlock attempt for `spin.xlsx` (4294967295) and `spin10m.xlsx`. MEDIUM-6: `rels100k.xlsx` 109 ms. MEDIUM-8: `notebig.ods` note truncated to 32,768 bytes. LOW-10: `links.ods` drops 7 hostile targets with a warning (the remaining `%6A%61vascript:`, `file:`, UNC, `smb:` and `mailto:?attach=` targets are still kept, as review 3 accepted). `comments100k.xlsx` 87 ms, 48 MB.
- LOW-9 (protection): `op protect 0` output identical to the post-fix run: `SetPageSetup`/`SetPageBreak` now refused, `undo` of `ProtectSheet` keeps the sheet protected, hidden formulas are not found by `find(in_formulas)`. `RefreshFilter`, `SetFrozen`, `DefineName` (redefine) and `DeleteName` still apply on a protected sheet (as in the post-fix run).
- MEDIUM-7 (mock engine in the bundle): fixed (see item 7).

---

## Item 3: outline and data tools

Harness: scratch crate `scratchpad/sec4/probe-tools` (binary `sec4-target/release/toolsprobe`; `toolsprobe <mode> N` with env flags, or `toolsprobe open FILE` with `EDITS`, `SAVE`, `NAV`, `KEEP`). Generator: `scratchpad/sec4/gen_tools.py KIND OUT args` (stdlib zipfile, run with `python3 -I`), fixtures in `scratchpad/sec4/tools/` (files over 1 MB deleted afterwards). Every run went through `sec4/r.sh` (60 s wall clock, `MEM=3000` RSS watchdog, because other probes ran at the same time). Release build at b03e603. Shell context: every `apply` runs under the single service mutex, and nothing can cancel it.

Shell caps vs engine caps (`apps/desktop/src-tauri/src/limits.rs:405-475 typed_op`): none of the wave-5 tool ops have typed checks. The structural walk caps each string at 64 KiB, all strings in one transaction at 64 MiB, ranges inside the sheet, `ranges` lists at 10,000 and ops at 100,000. Array **counts** are not capped: `Consolidate.sources` (engine: 255), `TextToColumns.options.fixed_widths` / `column_types` (none), `RemoveDuplicates.cols` (none, duplicates allowed), `Subtotals.groups[].columns` (engine: 1,024), and the number of `Group` / `CopySheet` / `InsertCells` / `DefineName` / `CreateScenario` ops in one transaction (none apart from MAX_OPS).

### CRITICAL-T1: RemoveDuplicates shifts the range once per run of duplicates, and snapshots each shift (O(n²) time and memory)
`crates/tally-engine/src/tools.rs:1708-1770`. Every run of adjacent duplicate rows calls `structure::shift_cells` (`structure.rs:692`) on its own. Each call moves the rest of the range up and captures a snapshot for undo. When duplicates alternate with unique rows (one row in two), n rows cause n/2 shifts.

| Command | Time | Peak RSS |
|---|---|---|
| `toolsprobe dedup 10000` (A1:A10000, every other row is "dup") | 3.2 s (undo 3.8 s) | 1.67 GB |
| `toolsprobe dedup 30000` | 43 s | 2.3 GB, then timeout |
| `toolsprobe dedup 100000` | killed after 5.5 s | > 3 GB |
| `KEYS=100000 toolsprobe dedup 1000` (`cols: [0; 100000]`, a 600 KB request) | 5.7 s | 273 MB |
| `L=65536 toolsprobe dedup 100000` (100k copies of a 64 KiB text) | 1.8 s (about 18 s at 1M rows) | 27 MB |

This is an ordinary Data menu action on an ordinary 100k-row column, well inside the 1M-cell cap. Impact: the app freezes and memory is exhausted. Fix:
- Compute the kept rows once and rewrite the range in a single pass, using one snapshot of the range for undo (as sort does).
- Dedup `cols` and require it to be at most `range.cols()` long.
- Hash the borrowed, case-folded text instead of building a joined `String` for each row (or compare the 64-bit hash first).

### HIGH-T2: Subtotals and RemoveSubtotals run one full InsertRows / DeleteRows per group
`tools.rs:983-1060 subtotal_level` (a `run(Op::InsertRows)` for every group break, at `:1041`) and `:1305-1360 remove_subtotals` (a `DeleteRows` for every run of subtotal rows). Each structural op shifts the whole sheet, adjusts every formula (including the SUBTOTAL formulas already written) and records undo. The total cost is therefore O(groups × (rows + formulas)), which grows faster than quadratic.

| Command (3 columns, a group every GS rows) | Subtotals | RemoveSubtotals | Undo |
|---|---|---|---|
| `GS=10 toolsprobe subtotals 10000` (1,000 groups) | 0.85 s | 0.83 s | 0.88 s |
| `GS=10 toolsprobe subtotals 30000` (3,000 groups) | 10.0 s | 9.9 s | 9.4 s |
| `GS=10 toolsprobe subtotals 100000` (10,000 groups) | > 60 s (killed) | | |
| `GS=100000 toolsprobe subtotals 300000` (3 groups) | 0.17 s | 0.23 s | 0.14 s |

Impact: Data > Subtotals on a 100k-row list grouped by an id column freezes the app for minutes, and Remove freezes it again. Fix:
- Compute the output layout in one pass: rows plus the inserted subtotal rows, written as one block shift.
- Write the SUBTOTAL formulas once, after the structure is final.
- Remove subtotal rows the same way (one compaction, not one DeleteRows per run).
- Cap the number of groups (for example 100,000) and refuse beyond it.

### HIGH-T3: TextToColumns output is not budgeted: one wide row or a fixed-width list multiplies the work by up to 16,384
`tools.rs:1605-1700`. Only `range.rows() ≤ MAX_TOOL_ROWS` is checked. `width` is the widest row, and the target is rows × width cells: every row is split, materialized, and cleared across the full width (`for c in out_col..=c1 { clear_content }`). `split_fields` (`:1520`) also returns one field per fixed-width start even when the text is shorter, so `fixed_widths: [0,1,…,16383]` gives every row 16,384 fields.

| Command | Time | Peak RSS |
|---|---|---|
| `W=16000 toolsprobe t2c 100000` (row 1 holds 16,000 commas, the rest "xxx") | 23.5 s (1M rows: about 4 min) | 53 MB |
| `ALL=1 W=16000 toolsprobe t2c 1000` (1,000 rows of `a,`×16000, 32 MB of input) | 10.1 s, 16M cells written (16× MAX_OP_CELLS); undo 2.7 s | 2.3 GB |
| `ALL=1 W=16000 toolsprobe t2c 2000` | 22.4 s; undo 9.2 s | 2.4 GB |
| `FW=16384 W=0 toolsprobe t2c 10000` (16,384 fixed widths, 10k rows of "xxx") | 4.9 s | 1.44 GB |

Fix:
- Refuse when `rows × width` exceeds MAX_OP_CELLS, before any field vectors are kept.
- Stream the split row by row instead of collecting `rows: Vec<(u32, Vec<String>)>`.
- Clear only the used part of `out_col..=c1` (the cells that exist).
- Cap `fixed_widths` and `column_types` at MAX_COLS entries in both the engine and `limits.rs`, and stop at the text's length.

### HIGH-T4: Consolidate keeps every source cell address and builds link formulas without a budget
`tools.rs:1373-1505`. The only checks are `sources.len() ≤ 255`, each source ≤ 1M cells, and `cells.len()` (distinct output keys) ≤ 1M. The `Vec<CellAddr>` for each key grows to 255 entries, so up to 255M addresses can be held. With `link: true`, every output cell gets a formula with up to 255 references, and its length is never checked.

| Command | Time | Peak RSS |
|---|---|---|
| `SAME=255 R=1048576 toolsprobe consolidate 1` (255 × A1:A1048576, values) | killed after 48 s | > 3.3 GB |
| `SAME=255 R=100000 LINK=1 toolsprobe consolidate 1` | 15.7 s, 100k formulas of 2.3 KB each | 2.17 GB |
| `SAME=255 R=1048576 LINK=1 …` | killed after 48 s | > 3.1 GB |
| `NAMELEN=90 R=10 LINK=1 toolsprobe consolidate 255` (sheet names of 93 chars) | 2 ms, but A1 holds a 24,995-char formula that shows `Err:512` | |
| `R=10 LINK=1 toolsprobe consolidate 256` | refused ("between 1 and 255 source ranges") | |

Fix:
- Cap the **sum** of source cells at MAX_OP_CELLS.
- Aggregate on the fly with an `Accumulator` per key instead of a `Vec<CellAddr>`.
- For `link`, refuse (or fall back to values with a warning) when a generated formula would exceed the 8,192-char parser limit, rather than writing a broken `Err:512` cell.
- Mirror the 255-source cap in `limits.rs`.

### HIGH-T5: Outline groups: quadratic loaders and an O(rows × groups) ODS writer
1. XLSX reader: `crates/tally-io-xlsx/src/tools.rs:123-170 groups_from_levels`. For every closed run, `levels.iter().any(|l| l.0 == e + 1 && l.3)` scans all levels (up to 1,000,000 kept, `sheet.rs:554-557`). That is O(levels × groups) for each of the 8 levels, and `truncate(100_000)` only happens afterwards.
2. ODS writer: for every written row, `write.rs:1391/1494/1522` call `write_group_starts`, `next_group_boundary` and `write_group_ends` (`tally-io-ods/src/tools.rs:134-155`, `v04.rs:61-67`). Each of these filters the whole group list, so the cost is O(rows × groups). Saving is also autosave's path.
3. ODS reader: `tally-io-ods/src/tools.rs:178-185 close_group` re-sorts the whole list on every push (O(G² log G)).
4. API: `crates/tally-engine/src/outline.rs:110-171 insert_group` scans the list and re-sorts it for every `Group` op.

| Input (generator / command) | File size | Open | Save ODS | Save XLSX |
|---|---|---|---|---|
| `gen_tools.py xlsxlevels lv1m.xlsx 1000000 1` (500k one-row level-1 groups) | 1.3 MB | > 60 s (killed) | | |
| `xlsxlevels lv300k.xlsx 300000 1` (150k groups) | ~0.4 MB | 5.8 s | 14 ms (groups dropped, see LOW-T9) | 62 ms |
| `xlsxnest nest100k.xlsx 100000` (100k data rows, levels cycle 1..7..1, a realistic deep outline) | 0.8 MB | 1.36 s | **7.8 s** | 0.14 s |
| `xlsxnest nest300k.xlsx 300000` | 2.5 MB | 11.7 s | **46 s** | 0.5 s |
| `xlsxnest nest1m.xlsx 1048576` | 8.5 MB | > 60 s (killed) | | |
| `odsgroups g100kd.ods 100000 1` (100k groups, 200k data rows, stored) | 33 MB | 2.3 s | **24.2 s** | 0.2 s |
| `odsgroups g30kd.ods 30000 1` | | 0.26 s | 2.2 s | 0.07 s |
| `odsgroups g100k.ods 100000` (groups over empty rows, stored) | 17.6 MB | 2.1 s | 9 ms | 56 ms |
| API `LEVELS=0 G=100000 toolsprobe outline 1048576` (100k `Group` ops in one tx) | | Group 9.7 s; the next `InsertRows` takes 0.77 s | > 50 s (timeout) | |
| API `LEVELS=0 G=10000 toolsprobe outline 1048576` | | Group 0.1 s | 10.5 s (1M data rows × 10k groups) | |

The same file deflated (`odsgroups g100k.ods` without STORED) is refused by the 200:1 ratio check, but the XLSX variants pass every cap. Impact: a 1.3 MB XLSX hangs open, and a normal 100k–300k-row outlined sheet takes 8–46 s on every ODS save (manual or autosave) under the global lock. Fix:
- `groups_from_levels`: precompute a `HashSet` of indices that carry `collapsed` (or look at `levels[i]` directly, since the list is sorted). Stop building at 100,000 groups.
- ODS writer: sort the starts and ends once and walk them with cursors, the way merges and validations are walked.
- `close_group`: push, then sort once at the end of the table.
- `insert_group`: binary-search the insert position and the covering groups instead of a linear scan plus a full sort, and cap `Group` ops per transaction.

### MEDIUM-T6: `DefineName` snapshots the whole name table for every op (O(N²) memory in one transaction)
`crates/tally-engine/src/undo.rs:599-601 names_snapshot` (`Change::Names(Box::new(wb.names.clone()))`) is used by `DefineName` / `DeleteName` (`ops.rs:930`) and by `CopySheet` (`tools.rs:2217`). This predates wave 5, but it is what makes "navigator with 100k names" impossible to set up through the API.

| Command | Time | Peak RSS |
|---|---|---|
| `NOTES=0 toolsprobe navigator 1000` (1,000 `DefineName` in one tx) | 13 ms | 83 MB |
| `NOTES=0 toolsprobe navigator 3000` | 117 ms | 630 MB |
| `NOTES=0 toolsprobe navigator 10000` | killed after 0.7 s | > 3.6 GB |
| `NOTES=0 toolsprobe navigator 100000` | killed after 0.7 s | > 3.5 GB |

MAX_NAMES is 100,000, and the shell allows 100,000 ops of up to 64 KiB each. Fix: record the replaced `NameEntry` (or its absence) as the inverse instead of the whole table, or keep the table behind `Arc` with per-entry copy-on-write. Count name ops in `op_cost`.

### MEDIUM-T7: Scenarios: 1M cells per scenario × 1,000 scenarios is accepted; memory is about 120 MB per scenario
`crates/tally-engine/src/scenarios.rs:41-44, 235-250`. The cap is 1M cells for **each** scenario and 1,000 scenarios per sheet, so a sheet may hold 1G stored cells.

| Command | Result |
|---|---|
| `BATCH=0 CELLS=1000000 toolsprobe scenarios 20` (20 separate `CreateScenario`, 1M cells each) | 0.34 s, 2.5 GB; the first `ShowScenario` takes 0.26 s and brings it to 2.9 GB, then killed |
| `CELLS=1000000 toolsprobe scenarios 1000` (one tx) | > 60 s (killed) |
| `CELLS=100 toolsprobe scenarios 1000` | 5.5 ms; `scenarios()` 30 µs, 194 KB; ShowScenario 35–110 µs |
| `CELLS=1048576 …` | refused ("the scenario ranges are too large"), as intended |

Fix: cap the cells **per sheet across all scenarios** (for example 1M), store scenario values compactly (shared `Arc<str>`, numbers unboxed), and count `CreateScenario` against a per-transaction budget.

### MEDIUM-T8: Other per-op costs with no transaction budget
- **InsertCells** (`tools.rs:2064 shift_range` → `structure.rs:692`; no cell-count cap, only the used part moves): one 1M-cell band shift (`C=1 toolsprobe insertcells 1048575`) takes about 180 ms and keeps +80–100 MB of undo/redo per op. `C=10 K=20 toolsprobe insertcells 100000` (20 shifts of a 1M-cell band in one tx) takes 4.9 s and +1.35 GB. With MAX_OPS = 100,000 and 100 undo entries, the memory has no practical bound.
- **CopySheet** (`tools.rs:2147-2315`): a 1M-value sheet copies in about 90 ms and +80 MB per copy. `ONETX=1 K2=50 toolsprobe copysheet 1048576` (50 copies in one tx; MAX_SHEETS is 10,000) was killed after 4.6 s at > 3.1 GB. A 1M-formula chain (`FORMULA=1 K=5`) takes 3.0–5.0 s per copy, because `refresh_all_precedents()` at `:2303` rebuilds the dependency graph of the whole workbook on every copy; RSS reached 2.3 GB after 5 copies and the run was killed. Fix: weight structural and copy ops by cells moved or copied in `op_cost` (for example 10M per transaction), and refresh only the new sheet's precedents.
- **AdvancedFilter** (`tools.rs:886-925`, `advanced_criteria` at `:670`): the criteria range may hold 100,000 cells, so every data row is tested against up to 100k OR rows. `K=10000 toolsprobe advfilter 10000` takes 3.3 s, and RefreshFilter costs another 3.2 s. `K=99999 toolsprobe advfilter 100000` is > 60 s (about 330 s by extrapolation). An ODS file with `table:condition-source-range-address` over 100k criteria rows (`gen_tools.py odsadv adv100k.ods 100000 99999`) opens in 175 ms because the filter is not re-evaluated at load, but the first Refresh would hang. Fix: cap criteria rows (AOO allows 8 condition rows in the dialog; 1,000 is generous) and give evaluation a work budget.
- **Autocomplete** (`crates/tally-engine/src/autocomplete.rs:15-82`): on a 1M-row column each request takes 130 ms (with a prefix) to 385 ms (empty prefix or "I"). It runs under the global lock; the UI debounces it (`packages/app/src/data/w56.ts:944-968`). The response returns up to 1,000 **full** texts but the UI only uses `completion`: `L=30000 toolsprobe autocomplete 1000` returns 30 MB of JSON per keystroke. `items.sort_by_key(|a| a.to_lowercase())` (`:79`) re-lowercases inside the sort, and `items` collects every match before `truncate`. Fix: keep the 1,000 smallest with a bounded heap, using the lowercase keys already computed in `seen`; truncate each item (for example to 256 chars) or return only `completion` on keystrokes; cap the scan at 100k cells.

### LOW-T9 (correctness): the ODS writer drops outline groups beyond the last used row
`gen_tools.py odsgroups g10k.ods 10000` (10,000 groups over empty rows): `content.xml` has 10,000 `<table:table-row-group>` before and **0** after `save(Ods)` (`KEEP=1 SAVE=1 toolsprobe open g10k.ods`); the XLSX save keeps them. Rows are only written up to the used range, so groups after it are lost. Fix: extend the written row range to the last group end, as is done for hidden rows.

### LOW-T10: Consolidate link quoting and length
Link formulas quote plain sheet names (`$'Src1'.A1`, seen in A1 after `R=1000 LINK=1 toolsprobe consolidate 255`). This is harmless but noisy. The broken-formula case is covered in HIGH-T4.

### INFO
- `sheet_layout` grows with the group list: 100k groups give 5.7 MB of JSON in 4–13 ms per fetch (it is refetched on every `layout_changed`), and 10k groups give 550 KB. The navigator with 100k names and 100k notes from ODS (`gen_tools.py navods nav100k.ods 100000 100000 20`) takes 25 ms and returns 9.3 MB; note previews are truncated (`navbig.ods`: 2,236 notes of 30 KB give 312 KB). `document_statistics` takes < 3 µs.
- The standard-filter condition count is capped at 4 in the engine API (`MAX_STD_CONDITIONS`) and at 64 from ODS files. File-sourced standard and advanced filters are not re-evaluated at open (`flt_*.ods` with 64 conditions on 1M rows: open 0.2–0.9 s).

## Refuted
- **StandardFilter regex DoS.** It uses the `regex` crate, which is linear-time with no backtracking (`tools.rs:447-460`, `size_limit` and `dfa_size_limit` at 1 MiB, an invalid pattern falls back to a plain comparison). Condition values over 4,096 bytes are refused (`tools.rs:743`): a 54 KB alternation gives "value is too long". On 1M rows, `(a*)*b`, `(a|aa){1000}{1000}` (fails the size limit and falls back), a 1,000-char CJK class and `\w{100}\w{100}\w{100}b` each take 120–160 ms. `\w{100}…` on 1,000 cells of 60 KB takes 7 ms. Regex is never enabled from files (the ODS filter reader does not read it). Edits do not re-run the filter.
- **8 nested levels over 1M rows.** `toolsprobe outline 1048576`: 8 `Group` ops take 133 µs; single-cell edits 10–20 µs; viewport 0.3 ms; `sheet_layout` 35 µs (649 B); InsertRows/DeleteRows about 52 ms (same as without groups); ShowOutlineLevel, collapse and undo < 60 µs; ODS save 0.5 s. A ninth level is refused ("at most 8 outline levels"), and so are partial overlaps.
- **Scenarios from ODS files.** 1,000 scenario sheets (`odsscen sc1000.ods 1000 10 1000`) open in 30 ms and save in 1.3 s / 0.8 s. A scenario with 50,000 ranges over 1M repeated rows (`sc1.ods`) opens in 0.5 s and saves in 1.6 s / 1.7 s. 5,000 scenario sheets keep 1,000.
- **InsertCells on a 1 × 16,384 band.** About 190 ms; the "cells would be pushed off the sheet" check holds.

## Verified OK
- Caps that held: 8 outline levels, 100,000 groups per sheet (API and both readers), 1,048,576 cells per tool range (`MAX_TOOL_ROWS`), 255 consolidate sources, 3 subtotal groups and 1,024 columns per group, 1M cells per scenario and 1,000 scenarios, 100k criteria cells, autocomplete 1,000 items and 1M scanned cells, prefix ≤ `MAX_AUTOCOMPLETE_CHARS` in the shell, 10,000 database ranges, 1,000 custom sort lists, MAX_NAMES 100,000 (ODS reader), and the 64 MiB note-text budget at import.
- Undo returned Ok after every tool probed (Subtotals, RemoveSubtotals, TextToColumns, RemoveDuplicates, InsertCells, CopySheet, Consolidate, Group); restored contents were not diffed.

---

## Items 4 (Print/PDF) and 5 (Calc semantics)

Probe: `scratchpad/sec4/probe-print` (binary `sec4-target/release/pprobe`, release build of b03e603 with `tally-engine/io`, which includes `pdf`). XLSX generator: `scratchpad/sec4/print/gen/gen_print.py` (stdlib zipfile, run with `python3 -I`). Every run used `sec4/r.sh` with MEM=3000 (3 GB RSS kill) and a 60 s watchdog unless noted. Other probes were running at the same time, so the timings are upper bounds (±30 %). RSS is macOS RSS, which compresses repetitive memory, so it understates the real footprint.

### Confirmed findings

**HIGH-P1: Print titles (repeat rows) are cloned into every page's `PageSpec`, with no cap, from a 1.5 KB XLSX**
`crates/tally-engine/src/print.rs:653-670` (`paginate_sheet` → `push`: `repeat_rows` is rebuilt as a `Vec<(u32,u32)>` for every page that follows the repeated span) and `:120-163` (`bands`: when the repeated span is taller than the page body, `avail` is 0, so each later page holds 1 row). The import path is `crates/tally-io-xlsx/src/read.rs:388-396` (`_xlnm.Print_Titles` → `repeat_rows`), with no size check here or in `convert.rs:762`. Cost is O(pages × repeated rows), and `MAX_PAGES` (100,000) counts pages only.
| Input | Command | Result |
|---|---|---|
| `gen_print.py repeat repeat10k.xlsx 10000 20000` (1.5 KB) | `pprobe openlayout repeat10k.xlsx` | `print_layout` 0.11 s, 10,232 pages, **801 MB** |
| `gen_print.py repeat repeat90k.xlsx 90000 180000` (1.5 KB) | `pprobe openlayout repeat90k.xlsx` | **> 60 s, 2.2 GB and growing** |
| API, repeat 50k rows | `pprobe repeat 50000` | 7.8 s, 1.2 GB |
| API, repeat 90k rows | `pprobe repeat 90000` | killed at 3 GB after 0.5 s |
Opening Print Preview on such a file freezes the app (the global service lock) and can exhaust memory. Fix: (1) when the repeated span does not fit, leave it off the page with a warning (AOO prints without it), or cap it at one page body; (2) keep the repeat span once per sheet (`Arc<[(u32,u32)]>`, or just the `[a,b]` span plus the scale), not per page; (3) clamp `repeat_rows`/`repeat_cols` at import (for example ≤ 1,000 rows).

**HIGH-P2: Iterative calculation honours 32,767 steps from the file, so opening a 79 KB XLSX hangs**
`crates/tally-io-xlsx/src/workbook.rs:238-252` (`calcPr iterate="1" iterateCount` clamped only to 32,767; `iterateDelta="0"` is accepted), engine cap `calcsettings.rs:25 MAX_ITERATION_STEPS = 32_767`, and the same through `Op::SetCalcSettings` and ODS `table:iteration` (`tally-io-ods/src/content.rs:1030`). Every step re-evaluates the whole cycle under the lock, with no total budget and no cancel.
| Input | Result |
|---|---|
| API: a 100k-cell cycle, 100 steps (`pprobe iter 100000`) | 2.1 s per recalc that touches the cycle; `recalc_all` 2.0 s |
| API: the same with 32,767 steps (`STEPS=32767 pprobe iter 100000`) | > 60 s (about 690 s by extrapolation) |
| `gen_print.py cycle cycle10k.xlsx 10000 32767` (79 KB), `pprobe openlayout` | **open > 60 s** |
| `gen_print.py cycle cycle100k.xlsx 100000 32767` (767 KB) | **open > 60 s** |
Fix: give each recalc an evaluation budget across all iteration steps (for example steps × cycle cells ≤ 50M; past it stop with Err:523 "no convergence" and a warning), cap imported `iterateCount` at the AOO default (100) unless the user confirms, and treat `min_change = 0` as "run to the step budget" rather than "unbounded".

**HIGH-P3: `MULTIPLE.OPERATIONS` has a per-call budget but no per-recalc budget (10,000 calls × 200k formulas ≈ 15 min)**
`crates/tally-engine/src/recalc.rs:1205-1222` (`table_op`: up to `MAX_TABLE_OPS` = 10,000 calls per recalc, `tools.rs:48`), `:1257-1290` (`SubCtx::new`: a reverse-dependency BFS of up to `MAX_SUB_AFFECTED` = 200,000 formulas per call, then every one of them is re-evaluated and memoized per call), `:1292-1302` (`touches` is linear in `affected` for every range read).
| Command | Result |
|---|---|
| `CALLS=10 pprobe tableop 199000` (F1 = SUM over 199k formulas that read A1) | 90 ms per call |
| `CALLS=300 pprobe tableop 199000` | filling the table 29.5 s; **every later edit of A1 23.3 s** |
| 10,000 calls (the cap), by extrapolation | about 15 min per recalc |
| `CALLS=10 SHAPE=chain pprobe tableop 100000` | 57 ms per call |
Fix: one shared work budget per recalc (for example 20M sandbox formula evaluations across all calls; past it return Err:523/#VALUE! with a warning); build the `affected` set once per (formula, input cells) per recalc and reuse it across calls; make `touches` an interval or range index instead of a scan.

**HIGH-P4: The solver's `timeout_ms` does not cover linearization, and the shell does not cap `timeout_ms`/`max_iterations`**
`crates/tally-engine/src/solver.rs:151-212` (`linearize`: 2n+2 sandbox recalcs (402 for 200 variables) of a full workbook clone (`:641`); `MAX_EVALS` 10,000 is the only bound, and the deadline is created after this phase, `:726-730`), `:497-547` (`branch` recurses depth-first, clones the LP per child and has no depth cap), and `apps/desktop/src-tauri/src/limits.rs:281-306` (checks counts and the epsilon only; `timeout_ms` up to u32::MAX ms ≈ 49 days, `max_iterations` up to 4e9).
| Command | Result |
|---|---|
| `pprobe solver` (200 vars, 2,000 linear constraint formulas, default 10 s timeout) | **30.9 s**, Optimal, 215 MB |
| `DEP=200000 pprobe solver` (plus 200k formulas reading a variable) | **53.3 s**, 489 MB |
| `pprobe bbdeep` (integer `2x-2y=1`, default options) | 10.2 s (timeout honoured), status Limit, 353 MB |
| `MAXIT=4000000000 TIMEOUT=4000000000 pprobe bbdeep` | **> 60 s, 945 MB and growing** (the DFS path holds an LP clone per level) |
A 1,000 × 1,000 dense model is not reachable: the engine caps 200 variables and 2,000 constraint rows (the shell caps 1,000 constraint *ranges*), so the tableau stays below about 2,200 × 4,600 doubles (≈ 80 MB). Impact: the solver freezes every document for the whole run, with no cancel. Fix: start the deadline before `linearize` and check it per sample; recalc only the dirty cone of the variables in the sandbox (not the whole clone); clamp `timeout_ms` (≤ 60,000) and `max_iterations` (≤ 100,000) in `limits::solver_request` and in `solve`; make `branch` iterative with an explicit stack and a depth cap (the recursion also risks a stack overflow on a 2 MB worker thread).

**HIGH-P5: Print job cost is O(cells in the print area), with no cell budget (a 2.7 MB XLSX takes about 9 min)**
`crates/tally-engine/src/service/print.rs:428-700` (`build_page` walks every row × column of the page, calls `style_of` (which clones `StyleData`) and `is_merge_covered` per cell); `print.rs:622,745` only cap the page count (100,000). Rows of 1 hmm (XLSX `ht="0.03"`) and 1 px columns (`width="0.2"`) put about 24,000 × 650 cells on one A4 page.
| Input | Command | Result |
|---|---|---|
| `gen_print.py tinyrows tinyrows.xlsx 1048576` (2.7 MB; print area A1:XFD1048576, 1,144 pages) | `pprobe openlayout` | `print_layout` 30–44 ms, 240 MB (each `PageSpec` holds 24k row entries) |
| same, `JOB=1 TO=100` | | `prepare_print_job` **48.2 s for 100 pages** (about 550 s for all 1,144) |
| same, `JOB=1` | | > 60 s |
| API, px 0.04 rows/cols on 1M × 16k (`JOB=1 TO=1 pprobe tiny`) | | **12.6 s for one page** (400M cells), 42 pages |
Fix: a per-job budget of visited cells (for example 20M; past it refuse with "the print area is too large; set a print range"), iterate only stored cells and style runs instead of every grid position, and refuse pages whose rows × columns exceed a bound (for example 100k cells per page).

**MEDIUM-P6: No pixel budget in `rasterize`; a 10 m custom paper reaches multiple GB at preview dpi**
`crates/tally-io-pdf/src/lib.rs:1080-1104` (`rasterize`: `scale = dpi/72` with no check on width × height; pages are clamped to 14,400 pt, `:982-984`), `crates/tally-engine/src/pagesetup.rs:29-34,145-157` (custom paper up to 10,000 mm), and ODS `fo:page-width` with no upper bound (`tally-io-ods/src/v03.rs:1454` → `PaperData::from_size_hmm`, `tally-model/src/lib.rs:843-864`). The UI asks for 96 × devicePixelRatio dpi (192 on Retina, `packages/app/src/print/printReducer.ts:56`).
| Command | Result |
|---|---|
| `DPI=72 pprobe bigpaper` (10 m × 10 m paper, one cell) | 0.84 s, **867 MB** (14,400 × 14,400 px) |
| `DPI=150` / `300` / `600 pprobe bigpaper` | killed at 3 GB (would be about 3.6 / 14 / 57 GB uncapped) |
| A4, 600 dpi (`pprobe job 190000`) | 215 ms, about 170 MB (normal) |
Fix: cap width × height in `rasterize` (for example 50 Mpx: lower the effective dpi and report it), cap custom paper at about 1.2 m (A0 plus margin) in `pagesetup.rs` and in the ODS/XLSX readers.

**MEDIUM-P7: Engine and PDF page caps disagree (100,000 vs 10,000); oversized jobs are built and cached, then fail**
`crates/tally-engine/src/print.rs:47` (`MAX_PAGES = 100_000`) vs `crates/tally-io-pdf/src/lib.rs:44` (`MAX_PAGES = 10_000`); the shell's `MAX_PRINT_PAGES` (10,000) bounds only `page_from`/`page_to` (`limits.rs:310-336`). `pprobe job 560000`: `prepare_print_job` succeeds with 29,474 pages (2.1 s, +500 MB, cached in the 4-entry LRU), then `print_job_pdf` and every `page_png` fail with "more than 10000 pages". Four such jobs pin about 2 GB. Fix: use one constant (10,000) in `layout` and refuse before `build_job`; render previews per page without the full-document PDF.

**MEDIUM-P8: A chart anchored in a repeated row is copied, 4 MB SVG included, onto every page; the shell admits 4 GB of SVG per job**
`crates/tally-engine/src/service/print.rs:676-697` (`row_index` includes `repeat_rows`, so the chart matches on every page; `svg: (*svg).to_string()` clones per page), `limits.rs:339-349` (1,000 SVGs × 4 MB each, no total cap; the engine also accepts any count, `service/print.rs:268-279`).
| Command | Result |
|---|---|
| `NOPDF=1 pprobe chartrepeat 200` (one 4 MB SVG, repeat rows 1–3, 286 pages) | job 0.3 s, **1.17 GB** |
| `pprobe chartrepeat 200` (with PDF) | **> 60 s** (286 renders of the same SVG) |
| `NOPDF=1 pprobe chartrepeat 1000` (1,429 pages) | 13.1 s (about 5.7 GB of clones; RSS compressed) |
| `pprobe charts 100` (100 distinct 4 MB SVGs) | job 0.45 s; PDF 13.7 s, 96 MB PDF, 1.37 GB peak (so 1,000 ≈ 140 s and 8 GB for input plus copy) |
| `pprobe charts 10` | PDF 1.3 s, 9.6 MB |
Fix: store the SVG once per job (`Arc<str>`, and have krilla reuse one XObject per chart); draw charts only from the body rows/cols, not the repeat band; cap the total SVG bytes per job (for example 32 MB) in `limits::chart_svgs` and in `build_job`.

**LOW-P9: Chart SVGs can pull local image files into the PDF**
`crates/tally-io-pdf/src/lib.rs:946-951`: `usvg::Options::default()` keeps usvg's default `image_href_resolver`, which reads any existing path with `std::fs::read` (`usvg-0.47.0/src/parser/image.rs:85-113`; `resources_dir` is None, so the path is taken as is). Verified: an SVG with `<image href='/abs/path/local.png'>` (`sec4/print/svg/abs.svg`, `pprobe svgfile`) produces a PDF with `/Subtype/Image … Width 16/Height 16`; `file://` URLs and the control SVG embed nothing. The SVG comes from the webview, so exploiting this needs markup injection into the ECharts SVG (see the shell/UI section). Fix: set `image_href_resolver.resolve_string` to `|_, _| None` (allow only `data:` images) and cap decoded image dimensions.

**LOW-P10 (correctness): `MULTIPLE.OPERATIONS` silently returns the unsubstituted value when the dependency cone exceeds 200,000**
`recalc.rs:1265`: the BFS stops at `MAX_SUB_AFFECTED` and the formula cell itself is not marked as affected, so `eval_cell` returns its current value. `CALLS=10 pprobe tableop 200000`: every row of the table shows 20000100000 (the value for A1 = 1) instead of 200,000 × input + const; at 199,000 the results are correct. Fix: if the cap is hit, return an error (Err:523/#VALUE!) instead of a partial sandbox.

**LOW-P11: Trace Precedents scans every precedent range cell by cell**
`crates/tally-engine/src/detective.rs:45-66` (`has_error` on each of up to 10,000 ranges, each up to 1M cells). `pprobe detective 1000000`: a formula with 427 overlapping `SUM(Bi:B1000000)` ranges → `precedents` **17.2 s**. (`dependents` of a cell with 1M dependents: 2.5 ms, capped at 10,000 traces, fine.) Fix: a shared cell budget per call (for example 5M cells; past it report `error: false` without scanning), or test errors only for ranges up to 64k cells.

**INFO**
- `print_job_pdf` and `print_job_page_png` take only a job id (u32, sequential from 1), not the document; jobs are not dropped when a document closes (`service/print.rs:24-30,108-148`; `print_jobs` is only touched there). Any caller can fetch another document's job by guessing a small integer (relevant to the `tally-print://` scheme, see the shell section). Fix: key jobs by (doc, job) and purge on `wb_close`.
- `print_job_pdf` returns `Vec<u8>` through Tauri IPC; a 10k-page job is 17–29 MB, which serializes to a JSON number array several times larger. Prefer the `tally-print://` stream or a temp file.
- `rasterize` copies and re-parses the whole PDF for every preview page (`Pdf::new(pdf.to_vec())`): 10k-page job, 600 dpi first page 205–215 ms, last page at 96 dpi 16 ms.
- FitTo/FitPages search (91 scales × bands) on 1M × 100: 62 ms in every mode. `FitTo(1,1)` on 1M rows quietly resolves to 10 % and prints 5,210 pages with no warning that it does not fit.
- `SetCellInput` of 200 × 1M-character strings via the API took 1.37 GB RSS (7× the text), outside print scope; the shell caps op strings at 64 KiB. In the PDF the text is capped at 4,096 characters and 1,000 lines per item (`lib.rs:50-52`): job 7 ms, PDF 25–38 ms (wrapped or not).
- Images (`meta.images`, up to 1,000 × 16 MB) extend the print area but are never printed (functional gap).

### Refuted / not reproduced
- print_layout on 1M rows × 100 cols at 100 %: refused in 3.6 ms ("more than 100000 pages"); in every FitTo/FitPages mode 62 ms, 5,210 pages, < 90 MB.
- prepare_print_job for 10,000 pages (190k × 7 numbers): 0.75 s, PDF 1.7 s / 17 MB (2.8 s / 29 MB with gridlines and headers), peak 510–640 MB (borderline, includes the 4M-cell workbook).
- Print-job LRU: bounded at 4 (`JOB_LRU`); six 10k-page jobs in a row peak at 995 MB, then stay at about 730 MB.
- Precision as shown on 1M formatted values plus 1M formulas: toggling 0.47 s / 0.34 s, recalc_all 1.39 s (the same as without it).
- Detective `dependents` on a cell with 1M dependents: 2.5 ms, 10,000 traces.
- Solver dense size: the 1,000 × 1,000 request is refused by the caps (200 variables, 2,000 rows); memory stays below 500 MB.
- Deep branch and bound with the default options stops at 10 s (status `Limit`).

### Verified by reading (no hostile SVG generated)
- usvg/roxmltree: DTDs are allowed (`usvg parser/mod.rs:148`), but roxmltree rejects entity-reference loops and nested entity expansion beyond depth 10 / 255 references (`roxmltree-0.21.1/src/parse.rs:67-70,514-537`); usvg refuses documents over 1,000,000 elements (`svgtree/parse.rs:392`) and guards against self-recursive `<use>` (`:713,759`). Nested SVG images loaded through `data:` cannot load external files (`from_data_nested` sets `resolve_string` to None). Not covered by those guards: decoded size of embedded `data:` raster images and SVGZ decompression (`decompress_svgz` reads to end with no size cap); worth a cap together with LOW-P9.
- The SVG size check (4 MB) exists in both the engine (`service/print.rs:177,272`) and the PDF crate (`lib.rs:48,943`); the PDF crate also caps 5M items per job.
- Per-text-item caps: 4,096 characters, 1,000 lines, font size 1–400 pt; page size clamped to 10–14,400 pt.
- DPI is clamped 18–600 in the engine (`service/print.rs:133`) and 1–600 in the shell.

### Priority
P1 (repeat rows, 1.5 KB file), P2 (iteration steps from the file), P5 (cell budget per print job), P3 (table_op recalc budget), P4 (solver deadline and shell caps), then P6–P8, P9–P11.

Artifacts: generators `sec4/print/gen/gen_print.py` (`repeat`, `tiny`, `tinyrows`, `cycle`); small fixtures kept in `sec4/print/` (`repeat10k.xlsx`, `repeat90k.xlsx`, `cycle10k.xlsx`, `cycle100k.xlsx`, `svg/abs.svg`); files over 1 MB (`tinyrows.xlsx`, `j10k.pdf`) were deleted (regenerate with `gen_print.py tinyrows tinyrows.xlsx 1048576`).

---

## Item 6: new importers (summary)

Harness: scratchpad/sec4/probe-imp (impprobe open|xls|html|dbf FILE). Generators: scratchpad/sec4/imp/gen/. Every run used sec4/r.sh with MEM=3000 and a 60 s limit. Large inputs were deleted.

### Confirmed
- CRITICAL: XLS (calamine 0.37) can be made to allocate memory without bound before Tally's limits apply. Tally's cell and text caps (crates/tally-io-xls/src/lib.rs:121-245) only run after calamine has built a dense range for every sheet. catch_unwind (lib.rs:108-116) catches panics but not allocation failure, so a small malformed file can abort or exhaust the process. Measured with files of about 5.6 KB: process abort (SIGABRT); 1.0–1.8 GB RSS in 0.2–0.6 s; > 3 GB within a second; and a 60 s hang. Inconsistent sheet-dimension records, far-apart cell coordinates and malformed compound-file sector chains all trigger it. Fix: before calamine runs, add a Tally-side pre-scan of the OLE2 container and the BIFF records. It should do chain walks with a visited set and a length bound, require stream length ≤ chain length ≤ 256 MiB, enforce the BIFF8 limits (65,536 rows, 256 cols), require first ≤ last in dimension records, and enforce a per-workbook budget on the dense area (bounding box) of each sheet. Also report the issue upstream, or isolate the decoder in a helper process with an RLIMIT.
- HIGH: HTML rowspan/colspan handling is super-linear (tally-io-html/src/lib.rs:362-376 `skip_covered`, 418-422 `covered` push): 17 KB → 17 s, 35 KB → 35 s, 70 KB > 60 s. Fix: keep "covered until row" per column (Vec<u32>, capped at MAX_COLS), stop advancing past MAX_COLS, and prune expired spans.
- HIGH: HTML `<script>`/`<style>`/`<template>` skipping is quadratic (lib.rs:109-123 lowercases the whole remaining input for each element): 1.7 MB → 2.5–7.5 s, 5.1 MB → 24.4 s, 17 MB > 60 s. Fix: search for `</` and compare the tag name case-insensitively in place, with no allocation.
- MEDIUM: HTML MAX_CELL_BYTES (64 KiB) is not enforced (lib.rs:389-394 appends the whole token once the length is under the cap). A 60 MB file gives one 62.9 MB cell, and open costs 0.59 s and 1.45 GB. Fix: truncate at a char boundary to the remaining budget.
- LOW: the engine reads files up to 512 MiB into memory before a format cap of 64 MiB (HTML) or 256 MiB (XLS/DBF) applies. A 500 MB HTML file costs 506 MB before it is rejected. read_xls also copies the input (`to_vec`). Fix: check the per-format size from the metadata and extension before reading, and pass a slice or Cursor<&[u8]>.
- LOW: SXC `map_formula` runs on formula attributes before the 8,192-character formula check. A 55 MB formula costs 0.85 s and 395 MB before the file is rejected. Fix: skip mapping when the value is over 8 KiB.
- INFO (functional): real OOo 1.x files declare a DOCTYPE in content.xml, styles.xml and manifest.xml, so they will probably all be rejected ("XML DOCTYPE declarations are not allowed"). The fixture ooo1.sxc has no DOCTYPE. Fix: in legacy mode, accept and ignore only the known OOo public-id DOCTYPE with no internal subset.
- INFO: HTML tables inside `<title>`, `<textarea>` and `<noscript>` are imported (browsers treat them as text). A `.txt` file with `<table` in its first 4 KiB opens as HTML. The module doc says HTML clipboard paste uses this reader, but no such call path exists.

### Verified OK
- 500 mutations of basic.xls (byte and bit flips, truncation, header/FAT/directory u32 values, BIFF u16 values, splices): 216 OK, 242 errors, 42 panics that were caught and became "malformed workbook". No crash or hang, and nothing exceeded 2 s or 1.5 GB. No profile sets panic=abort. The shell also wraps engine calls in catch_unwind (commands.rs:55). decode() owns all its state, so a caught panic leaks nothing.
- SXC shares the ODS package and XML caps: DOCTYPE, entity bombs and external entities are rejected, depth is capped at 256, and the compression-ratio cap holds (2 GB bomb rejected in 14 ms).
- HTML: 64 MiB input cap; 10M-cell cap (65 MB file: 7.1 s, 1.7 GB, at the documented cap); 1M-deep nested tables 0.2 s; 1M mixed numeric entities 81 ms (surrogates, > U+10FFFF and NUL dropped); empty merged cells capped by the engine at 100k merges; unterminated quotes and comments are linear.
- DBF: a record count of 2^32−1 is rejected; a header-length lie is rejected; 255×255 fields 9 ms; 1,024 zero-length fields 57 ms; memo fields are ignored (no .dbt/.fpt access); 9.5M cells 0.64 s, 655 MB; a 10,000,005-cell file is rejected.
- External references: aggregated per kind (external.rs:212-228), so 1M references give one warning and one ExternalLinkWarning. Costs: ODS 1M repeated refs 0.68 s, 748 MB (cheaper than 1M ordinary formulas: 1.32 s, 955 MB); 1M distinct ODS refs + DDE 1.38 s; XLSX 1M `[1]Sheet1!` refs 1.62 s, 779 MB; 255 linked sheets with 60 KB hrefs 65 ms (detail clipped to 200 chars).

---

## Item 7: shell and UI (apps/desktop/src-tauri, packages/app)

Method: reading every wave-5 shell file (`commands_v4.rs`, `limits.rs`, `print.rs`, `images.rs`, `prefs.rs`, `autosave.rs`, `templates.rs`, `l10n.rs`, `lib.rs`, capabilities, `tauri.conf.json`), plus three probes under `scratchpad/sec4/shellprobe/`: `pdfhref` (a scratch crate on the worktree's `tally-io-pdf`, renders one `Item::Svg` whose SVG holds `<image href=ARG>`), `svg.mjs` (ECharts 6.1.0 SSR SVG renderer with hostile title/series/category/axis names), and `names.rs` (copies of `check_name`, `snapshot_name` and `job_of_path`). Production bundle: `pnpm install --frozen-lockfile` then `pnpm --filter tally-desktop build:web` in the worktree (built in 11.5 s, `dist/assets/index-B4C-MqK0.js` is 1.59 MB).

### HIGH-S1: chart SVGs in print jobs can read any local file and hang or exhaust memory on device files
`crates/tally-io-pdf/src/lib.rs:946-951` (`draw_svg`) builds `usvg::Options { ..Default::default() }`, so usvg 0.47's `default_string_resolver` (`usvg/src/parser/image.rs:85-113`) runs on every `<image href>`. It resolves the href as a file path (relative paths against the process working directory), and if `path.exists()` it calls `std::fs::read` before it checks the format. The SVG comes over IPC from the webview: `prepare_print_job` takes up to 1,000 SVGs of up to 4 MB each (`limits.rs chart_svgs`), and the engine passes them through unchanged (`service/print.rs:713-716`).

| href (`T=… MEM=2000 r.sh pdfhref HREF`) | result |
|---|---|
| `none.png` | 2 ms, 1,586-byte PDF (image skipped) |
| absolute path of a local 64×64 PNG | 0.7 ms, 1,840-byte PDF: **the file is embedded** |
| `secret.png` (relative path, resolved against the working directory) | embedded |
| `file:///…/secret.png` | not embedded (the resolver does not strip the scheme) |
| `/dev/zero` | **killed at 2.25 GB after 0.7 s** (unbounded `fs::read`) |
| a FIFO named `fifo.png` | **hangs past the 10 s watchdog** (blocks while holding the global service mutex) |

Today the SVG text comes from ECharts, which escapes all user text (see Verified OK), so a hostile spreadsheet cannot inject an `<image>`. This is defense in depth against a compromised or buggy webview. It is still the one place where webview input reaches the local file system, which breaks the shell's "no file system API is exposed to JS" design (`print.rs:10-12`). Impact: any PNG/JPEG/GIF/WebP/SVG the user can read can be pulled into an exported PDF, and the app can hang or OOM while holding the service lock. Fix: set `image_href_resolver: usvg::ImageHrefResolver { resolve_data: ImageHrefResolver::default_data_resolver(), resolve_string: Box::new(|_, _| None) }` (only `data:` URIs), and set `resources_dir: None` explicitly. Optionally, reject SVGs that contain `<image`/`xlink:href` to non-`data:` targets in `limits::chart_svgs`. Add a regression test with `/dev/zero` and an absolute path.

### MEDIUM-S2: Crash-recovery snapshots are keyed by a per-process doc id, so a later session or a second instance deletes or overwrites them
`autosave.rs:74-83` names snapshots `doc-<DocId>-<stem>.ods`. `DocId` restarts at 1 in every process (`tally-engine/src/service/mod.rs:92,135`). `remove_snapshot` (`autosave.rs:138-150`) deletes every file starting with `doc-<id>-`, and it runs on every close (`commands.rs:156`) and every real save (`dialogs.rs:206`). No single-instance plugin is configured. Sequence: Tally crashes with `doc-1-Budget.ods` on disk. On the next launch the recovery dialog is offered (`w56.ts:1084-1091`), and meanwhile the startup document is doc 1 too. If the user saves or closes that document, or the 10-minute autosave of a dirty `Untitled` doc 1 overwrites `doc-1-Untitled.ods`, the unrecovered snapshot from the crashed session is gone. Two running instances destroy each other's live snapshots the same way. `recover_snapshot` also deletes the snapshot as soon as it opens it (`autosave.rs:230-231`), so a second crash before the next autosave loses it. Impact: loss of exactly the data the feature exists to protect (no confidentiality impact). Fix: use a per-process unique key, such as `<pid>-<launch nanos>-<doc>` or a random 64-bit session id, and store it in the sidecar. Make `remove_snapshot` delete only this session's files. At startup, move older sessions' snapshots to a `recovered/` folder (or keep them until the user picks Recover or Discard). Delete the recovered snapshot only after the first successful save or autosave of the recovered document.

### LOW-S3: Non-macOS print temp PDFs are world-readable, predictable and never deleted
`print.rs:101-103,240-261`: `open_in_viewer` writes `$TMPDIR/tally-print-<pid>-<job>.pdf` with `std::fs::write` (mode 0666 & umask, so 0644), launches the viewer, and nothing ever removes the file (no other reference to `temp_pdf_path`). On Linux `$TMPDIR` is usually the shared `/tmp`, so every printed spreadsheet stays readable by other local users until reboot. A local user can also pre-create the predictable target path as a directory, which makes printing fail (`rename` refuses). A pre-planted symlink is harmless, because `rename` replaces the link itself. Fix: create a private per-process directory (`tempfile::Builder::new().permissions(0o700).tempdir()` or `mkdtemp`), write with `OpenOptions::new().create_new(true).mode(0o600)`, and delete the files on exit, or after a delay once the viewer has opened them.

### LOW-S4: Print-job PDFs stay in the shell's memory indefinitely (up to 4 × 512 MB)
`print.rs:30,201-212`: `PrintJobs` keeps the last 4 PDFs (each up to `MAX_PDF_BYTES` = 512 MiB) for the `tally-print` scheme. Nothing removes an entry when its print window closes, and `protocol` clones the whole `Vec` on every request (`print.rs:136`). The engine also caches jobs (item 4). Impact: up to 2 GiB held after printing large documents. Fix: drop the entry when the `tally-print-<job>` window is destroyed (`on_window_event(WindowEvent::Destroyed)`), remove the entry once served (the window has loaded it), and lower the cap to 1–2 entries.

### LOW-S5: Shell caps that do not match the engine's
- `prepare_print_job`: 1,000 × 4 MiB SVGs are allowed with no aggregate limit (`limits.rs chart_svgs`), so a single IPC payload can be 4 GiB of JSON. `apply` caps every transaction's strings at 64 MiB (`MAX_TSV`). Fix: add a total of 64 MiB across `charts` (sum of `svg.len()`).
- `AddImage.data` is allowed up to `MAX_TSV` = 64 MiB of base64 (`limits.rs typed_op`), but the engine rejects anything over 16 MiB × 4/3 + 8 ≈ 22.4 MB (`tally-engine/src/images.rs:29`). So the shell accepts, serializes into a `serde_json::Value` (the structural walk) and forwards up to 3× more than can ever succeed. Fix: cap at `MAX_IMAGE_BYTES.div_ceil(3) * 4 + 8`.
- `MAX_CHART_SERIES` = 256 in the shell vs `charts::MAX_SERIES` = 255 in the engine (the engine rejects 256 with a clean error, `charts.rs:930`; cosmetic).
- `print_job_page_png`: `dpi ≤ 600` is checked, but nothing bounds dpi × page size. The engine clamps page sizes only to 14,400 pt (`tally-io-pdf` `render_pdf`), and `rasterize` (`tally-io-pdf/src/lib.rs:1074-1097`) has no pixel budget: a 5 m page at 600 dpi is ≈ 118k × 118k px. Lead for item 4 (not measured here). Fix: a pixel budget (for example 64 Mpx) in `rasterize`; scale the dpi down to fit.
- Lead for item 4 (by reading): `service/print.rs:699-716` places a chart on every page whose `row_index` contains its anchor row, and `row_index` includes `page.repeat_rows` (`:449,483`). A chart anchored in a print-title row is therefore copied (`(*svg).to_string()`, up to 4 MiB) onto every page, up to 10,000 pages, which is ~40 GB for one 4 MiB chart. Fix: skip charts anchored in repeat rows/cols except on their home page, and share the SVG with `Arc<str>`.
- `print_job_pdf` is registered (`lib.rs:114`) but unused by the UI (only `wb_export_pdf`/`wb_print` use the bytes, inside Rust). It returns the PDF as a JSON number array (~4× size). Fix: unregister it, or return base64.

### LOW-S6: Header/footer fields print the autosave or temp path after an autosave or Save as Template
`autosave.rs:123` and `templates.rs:139` call the engine's `save(doc, Some(path))`, which sets the document's `title` and `path` to the snapshot or temporary file (`tally-engine/src/service/mod.rs:369-371`). `fix_info` repairs only the `WorkbookInfo` that the shell returns. The engine keeps the wrong values, and `hf_context` (`service/print.rs:40-48`) uses them, so after an autosave `&[FILE]` prints `doc-1-Book.ods` and `&[PATH]` prints `~/Library/Application Support/dev.tally.app/autosave/doc-1-Book.ods` (user name included). After Save as Template they print the deleted `.Budget.tmp.ods`. The engine's dirty flag is also cleared by Save as Template. Fix: add an engine `save_copy(doc, path, fmt)` that does not touch `path`/`title`/`dirty` (export semantics), and use it for autosave and templates.

### LOW-S7: The image picker reads devices and FIFOs without a bound
`images.rs:127-143`: `metadata(path).len()` is 0 for `/dev/zero`, a FIFO or a character device, so the 16 MiB check passes and `std::fs::read` reads forever. The Open dialog filters on `.png/.jpg`, so this needs a symlink such as `photo.png → /dev/zero`, for example one dropped by an extracted archive. Fix: open the file, require `file.metadata()?.is_file()`, and read with `file.take(MAX_IMAGE_BYTES + 1)`.

### INFO
- Template names (`templates.rs:48-65`, `names.rs` probe): `CON`, `NUL` and `COM1` are accepted (`CON.ots` is a device name on Windows 10), as are `U+202E` (RTL override), `U+200B` and `U+FEFF` (visual spoofing in the template list), and case variants that collide on case-insensitive volumes. An existing template of the same name is overwritten silently. `list_user_templates` returns the absolute file path as `description` (`templates.rs:168`), which exposes the user name and app-data path to the webview. Fix: reject Windows device names and format characters (`char::is_control` misses category Cf), and return only the stem.
- `prefs.json` is written non-atomically (`prefs.rs:65`). A crash mid-write makes `load` fall back to `{}` (all preferences lost, nothing worse). Unknown keys are kept as given. Size (64 KiB), object shape and serde's recursion limit (128) are enforced. The webview normalizes values (`prefs.ts:93` clamps `autosaveMinutes` to 0–120), and the shell reads only `uiLanguage` (mapped through `locale_of`, unknown → English).
- Desktop pictures: `images()` returns DTOs without bytes (`service/charts.rs:51-59`), and `image_bytes` has no Tauri command, so pictures loaded from files render empty in the desktop app (only freshly inserted pictures have `imageData`). This is a functional gap. When it is added, return one picture per call (base64 up to 22 MB), never all pictures at once (256 MiB per document).
- `wb_print`/`wb_export_pdf` ignore `doc` (`print.rs:191`) and print any job id of the process. Ids are sequential, but the store is in-memory and per-process, so there is no cross-document boundary to cross.
- App commands are not restricted by an app manifest (`build.rs` is the default), so any webview of the app may invoke them. The only other webview is the print window, which shows an engine-generated PDF (no script).
- Autosave writes the whole document as ODS under the service mutex every N minutes. On a large document this is a periodic freeze of every window (the same lock as review 3).

### Refuted
- `tally-print://` traversal or cross-job access: `job_of_path` (`print.rs:106-113`) accepts only `/<1–10 ASCII digits>.pdf` that parses as u32 (`//7.pdf` → 7, `/0000000007.pdf` → 7, `/4294967296.pdf`, `/%37.pdf`, `/7.pdf?x` (path only), `/7.pdf/` and `/../7.pdf` → none). The handler looks up an in-memory map of PDFs this process produced, never the file system. The main window cannot load the scheme (CSP `default-src 'self'`, `connect-src ipc: http://ipc.localhost`).
- HTML injection through chart text: ECharts SSR SVG output escapes `</text><image href=…>`, `&`, `<!ENTITY`, `]]>` and `{a|rich}` in the title, subtitle, axis names, category labels, series names and label-formatter results (they arrive as `&lt;/text&gt;…`, and no `<image` element is produced; `svg.mjs`). The live chart's default HTML tooltip escapes headers, names and values with `encodeHTML` (`echarts/lib/component/tooltip/tooltipMarkup.js:170,234,244`). `toEcharts.ts` defines no string-template formatters, `rich` styles, `renderMode`, `image://` symbols or `useHTML`. Every formatter is a function that returns plain text, colours come from u32 values, and marker symbols come from an enum.
- New HTML sinks: none in `packages/app/src`, `packages/grid/src`, `packages/bridge/src` or `apps/desktop/src` (no `innerHTML`, `dangerouslySetInnerHTML`, `insertAdjacentHTML`, `DOMParser`, `document.write`, `eval`, `new Function`, `window.open`, `outerHTML`). The 16/13 occurrences of `innerHTML`/`dangerouslySetInnerHTML` in the bundle come from React DOM and the ECharts tooltip. Image `data:` URLs use the sniffed MIME (`image/png|jpeg`) only.
- `images()` IPC size: refuted, the DTOs carry no bytes.

### Verified OK
- Production bundle: `window.__tallyCharts` is absent (0 hits in `dist`; `chartMounts.ts:161` gates it behind `import.meta.env.DEV`). `__tally` is absent too. Review-3 MEDIUM-7 is fixed: `main.tsx:29-37` loads `@tally/app/dev` only under `import.meta.env.DEV`. No string unique to `dev/webMock.ts` ("User template (browser demo)", "You cannot change part of an array."), `dev/mockW56.ts` ("The pivot source range does not exist.", "Destination range is not empty.") or `bridge/src/mock.ts` ("Opening files needs the desktop app.", "MockClient does not support goal seek") is in `dist/assets/*.js`. A non-Tauri production page gets no client.
- `pick_image` (`images.rs`): the path comes only from the native dialog and never reaches JS (only the file name). The size is checked from metadata and again on the bytes (no TOCTOU growth past 16 MiB, except device files, LOW-S7). The type is sniffed from magic bytes (PNG and JPEG only, the extension is ignored). `pixel_size` cannot index out of bounds (`i + 9 < len` before reading `i+8`; PNG needs ≥ 24 bytes). The engine re-decodes and re-sniffs, and checks that the declared MIME matches and that the document total is ≤ 256 MiB (`tally-engine/src/images.rs:28-67`).
- `autosave.rs` confinement: `recover_snapshot`/`discard_recovery` canonicalize the JS-supplied path and require it to be under the canonical autosave dir and to end in `.ods` (`:178-187`), so symlinks pointing outside, `..` and a hostile sidecar `path` are rejected before open or delete. `remove_snapshot` deletes directory entries (a symlink is unlinked, its target untouched). `snapshot_name` keeps only alphanumerics, `-_. ` and 80 characters (`../../etc/passwd` → `doc-5-....etcpasswd.ods`, RTL override dropped).
- `templates.rs`: names are trimmed, 1–80 characters, with no `/ \ : * ? " < > |`, control characters or leading dot. `new_from_user_template` re-validates the id, then canonicalizes it and requires it to be inside the templates dir (symlinks out are refused). A template opens as untitled with no path.
- `l10n.rs`: languages ≤ 64 bytes, unknown → English. The menu is rebuilt from static tables.
- `export_pdf`: the PDF is fetched before the dialog, the bytes must start with `%PDF` and be ≤ 512 MiB, `.pdf` is enforced, and the write is atomic (temp file next to the target, then a rename).
- Capabilities are unchanged in spirit: `default` (main window only) adds just `set-fullscreen`/`is-fullscreen`. The Windows-only capability adds the caption-button window APIs. No fs/shell/http plugin. CSP is unchanged from review 3.

---

## Housekeeping
The worktree `scratchpad/sec4-wt` was removed with `git worktree remove --force`, and `scratchpad/sec4-target` (1.2 GB) was deleted. Files over 1 MB under `scratchpad/sec4/` were deleted too. Still in `scratchpad/sec4/`: the probe sources, generators, `r.sh`/`run.py`, small fixtures, `regress3.txt` (the review-3 re-run) and the per-area drafts (`part-*.md`). To rebuild the probes, create a new worktree at b03e603 and point the path dependencies in each probe's `Cargo.toml` at it.
