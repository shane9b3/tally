# Performance baseline (1.0 gate)

Measured 2026-10-23 on the commit after `c992a3d`, release build
(`lto = "thin"`, `codegen-units = 1`). Re-measured 2026-10-27 on top of
`c53b362` after the memory and CSV work below (open/save, RSS and CSV rows
updated; the machine was busy with a VM and a backup, so +-15 % still
applies and RSS varied by up to 0.2 GB between runs).

Machine: Apple M4 Pro (`sysctl -n machdep.cpu.brand_string`), 12 cores,
24 GB RAM (`hw.memsize` = 25769803776), macOS (Darwin 25.5.0), internal SSD
under `$TMPDIR`. Other builds were running on the machine at times, so treat
values as +-15 %.

## Numbers

Engine and IO (`cargo xtask perf --deep --runs 3`; medians; time includes
everything `EngineService::open` / `save` does, i.e. read, build, recalc):

| Scenario | Result | Notes |
|---|---|---|
| Full recalc, 100k formulas in dependency chains | 77 ms (hard recalc 47 ms) | 100 chains x 1000 deep, `=A1+1` style |
| Full recalc, one chain 100k deep | 51 ms | no stack problem |
| Full recalc, 1M-cell sheet (100k rows x 10), SUM + VLOOKUP | 0.26 s (hard recalc 0.19 s); was 7.2 s before the lookup index | 200k formulas: 100k `SUM(A:H)` + 100k exact `VLOOKUP` into 10k rows |
| Same sheet, SUM column only | 121 - 124 ms | so 100k VLOOKUPs now cost about 0.1 s, about 1.4 us per lookup (was about 70 us) |
| Incremental recalc, 1 edit -> 10k dependents | 2.3 ms | `Workbook::set_input` + `recalc` |
| Same edit through `DocumentService::apply` | 2.5 ms | includes undo snapshot and change report |
| Open ODS 10 MB (93k rows x 12) | 1.27 s (was 1.67 s) | |
| Save ODS 10 MB | 0.92 s | |
| Open XLSX 10 MB (95k rows x 12) | 1.22 s (was 1.93 s) | |
| Save XLSX 10 MB | 1.18 s | |
| Open ODS 25 MB (232k rows) | 3.8 s | |
| Open XLSX 25 MB (237k rows) | 3.2 s (was 3.6 s) | |
| Save ODS / XLSX 25 MB | 2.5 s / 2.8 s | |
| Open ODS 50 MB (465k rows) | 7.1 s (9.7 s on a busy machine) | was FAILS (`zip entry uncompressed bytes`, 512 MiB cap); `content.xml` cap raised to 2 GiB, see `docs/security.md` section 1. Re-measured after the security-review fixes (attribute budget, per-package ratio) with `--only io50`, median of 3 |
| Save ODS 50 MB | 5.1 s (15.3 s on a busy machine) | same run |
| Open ODS 50 MB, 2026-10-27 | 7.0 - 7.7 s | same file, after the memory work |
| Open XLSX 50 MB (474k rows) | 6.6 - 7.6 s (was 7.8 - 8.8 s) | |
| Save XLSX 50 MB | 7.2 - 10.4 s | 7.6 s on 2026-10-27 |
| Peak RSS, open XLSX 50 MB | 0.90 - 1.08 GB (was 1.25 - 1.6 GB) | `/usr/bin/time -l` of a child process; peak memory footprint 0.92 - 1.19 GB (was 1.93 GB) |
| Peak RSS, open ODS 50 MB | 0.95 - 1.10 GB (was 1.24 - 1.37 GB) | footprint 1.0 - 1.15 GB (was 1.94 GB) |
| CSV import, 1M rows x 6 cols (47 MB) | 1.15 - 1.35 s (was 7.6 s; 3.9 s on the same machine on 2026-10-27 before the change) | about 35 - 40 MB/s |
| Peak RSS, CSV 1M rows | 0.61 - 0.74 GB (was 0.94 - 1.12 GB) | |
| `chart_data`, line chart with 10k points | 2.6 - 3.7 ms (cache miss after an edit); 0.3 ms cached | gate: < 20 ms |
| PDF export, 20 pages (1060 rows x 6 cols) | 11 - 12 ms | `prepare_print_job` + `print_job_pdf`; gate: < 1 s |
| DataPilot refresh, 100k source rows | 29 ms (create 29 - 33 ms) | 2 row fields (12 x 50 items), 1 column field (4), Sum + Count, `RefreshPivot` through `apply`; gate: < 500 ms |

### Multithreaded recalc (2026-11-10)

`docs/recalc-parallel.md`. Same machine (12 cores; the pool uses 12
threads), release build, `cargo xtask perf --only
recalc,incremental,deep,threads --runs 5`, medians; "1 thread" is
`TALLY_CALC_THREADING=off` (the single-threaded path, same code as before)
or `Threading::Sequential`, "12 threads" the default. Other agents' builds
were running, so +-15 %.

| Scenario | 1 thread | 12 threads | Speed-up |
|---|---|---|---|
| Full recalc, 100 chains x 1000 deep (first) | 43 ms (`threads`: 40 ms) | 27 ms (24 ms) | 1.6x |
| Hard recalc, chains | 35 ms (33 ms) | 19 ms (23 ms) | 1.5 - 1.8x |
| Full recalc, SUM/VLOOKUP mix, 200k formulas (first) | 327 ms (261 ms) | 187 ms (217 ms) | 1.2 - 1.75x |
| Hard recalc, SUM/VLOOKUP mix | 206 ms (201 ms) | 132 ms (112 ms) | 1.6 - 1.8x |
| SUM only, 100k formulas | 170 ms | 72 ms | 2.4x |
| Flat, 200k independent formulas (new, `threads`) | 134 ms | 86 ms | 1.6x |
| One chain 100k deep | 46 - 66 ms | 43 - 54 ms | about 1.2x (no parallelism; the pre-pass loop is cheaper than the on-demand path) |
| Incremental, 1 edit -> 10k dependents | 2.4 ms (2.1 ms) | 1.2 - 1.3 ms | 1.7 - 1.9x |
| Same edit through `DocumentService::apply` | 2.3 ms | 1.3 ms | 1.8x |
| Incremental, 1 edit -> 100 dependents (below the 4,096 threshold) | < 0.05 ms | < 0.05 ms | unchanged (pre-pass skipped) |

Re-checked 2026-11-10 after the review 7 fixes (same machine, `--only
recalc,incremental,deep,threads --runs 5..7`, release builds of 6fe72d5
and of the fixes run back to back): single-threaded numbers unchanged
within noise (chains first 29.7 - 29.8 ms vs 29.9 - 30.2 ms, SUM/VLOOKUP
mix first 229 - 236 ms vs 227 - 237 ms, hard 171 - 172 ms vs 171 - 174 ms,
incremental 2.3 ms vs 2.3 ms, one chain 34 ms vs 35 - 36 ms); 12 threads
also unchanged (chains 18.8 ms, mix first 162 ms, hard 104 ms).

The speed-ups are well below 12x because only formula evaluation is
parallel: dirty propagation, the Tarjan ordering (which now also keeps the
precedent lists), building the dependents index and storing the results in
order stay on one thread and are now the larger part of these recalcs
(cheap formulas, about 0.4 - 1 us each). Formulas that cost more per cell
(lookups that scan, statistics over ranges, text functions) gain more.

Grid scroll (`pnpm perf:e2e`, headless Chromium 1280x820 against
`dev:web`/MockClient, 1M cells = 50k rows x 20 cols loaded in about 2 s;
one scroll step per animation frame; two runs agree):

| Scroll | frames | p50 | p95 | p99 | max | frames > 20 ms |
|---|---|---|---|---|---|---|
| Vertical, 240 px/frame (10 rows) | 300 | 16.7 ms | 50 ms | 67 ms | 200 - 267 ms | 118 - 121 |
| Horizontal, 160 px/frame | 150 | 16.7 ms | 17 - 33 ms | 50 - 67 ms | 50 - 67 ms | 7 - 10 |
| Diagonal | 300 | 16.7 ms | 100 ms | 167 - 183 ms | 167 - 200 ms | 114 - 120 |

Caveats: headless Chromium renders on the CPU (no GPU), the dev server
build is unminified, and 240 px/frame is a fast fling (about 14,000 px/s), so
this is a pessimistic proxy. rAF timestamps are quantised to 16.7 ms.
Still, about 40 % of frames in a fast vertical fling take 2+ vsyncs. The
real number to gate on is a run in the Tauri webview on a Mac (not measured).

## How to rerun

```sh
export CARGO_TARGET_DIR=$PWD/.targets/perf
cargo xtask perf                      # everything above, release mode, ~3 min
cargo xtask perf --quick --runs 1     # 1/10 sizes, smoke test
cargo xtask perf --only recalc,csv    # scenarios: recalc incremental io10 io25 io50 csv rss deep chart pdf pivot threads
TALLY_CALC_THREADING=off cargo xtask perf --only recalc   # single-threaded recalc (TALLY_CALC_THREADS=N caps the pool)
cargo xtask perf --dir /path/on/big/disk
cargo xtask perf --only io50,csv --keep --dir DIR   # files in DIR/tally-perf-<pid>, kept; DIR itself is never deleted
cargo xtask perf --check              # the CI gate (see "CI gate" below)
.targets/perf/release/perf --mem FILE   # RSS, footprint and live malloc bytes after each open step
.targets/perf/release/perf --dump FILE  # digest of the loaded workbook, to diff two builds
pnpm perf:e2e                         # grid scroll frame times (starts dev:web itself)
```

`cargo xtask perf` builds `xtask/src/bin/perf.rs` with the `perf` feature (the
engine dependencies are optional so the other xtask commands stay light). All
files are generated in a temp dir and deleted afterwards. The Playwright
config (`tests/e2e/perf.config.ts`) only matches `*.perf.ts`, so `pnpm e2e`
never runs it. A Vite HMR reload from edits in another terminal can abort a
run with "Execution context was destroyed"; just rerun.

## CI gate (`perf --check`, `.github/workflows/perf.yml`)

The 1.0 targets are product claims for the **reference machine**, the
Apple M4 Pro described at the top (12 cores, release build): 100k-formula
recalc < 500 ms, 50 MB ODS open < 10 s, `chart_data` 10k points < 20 ms,
20-page PDF < 1 s, 100k-row DataPilot refresh < 500 ms. They hold there with
a wide margin (table below). CI does not re-prove those claims on other
hardware; it is a **regression gate** that runs the same workloads on
GitHub-hosted runners (macos-15, ubuntu-24.04, windows-latest) and fails
when one gets clearly slower.

```sh
cargo xtask perf --check                      # all workloads, this OS's limits
cargo xtask perf --check --only recalc_chains,pivot_100k
cargo xtask perf --check --thresholds FILE --json out.json --markdown out.md
```

`--check` runs every workload listed in `xtask/perf-thresholds.txt` at the
size given there, prints a Markdown table (every value next to its limit,
also when passing, then one line per failure with the workload, OS, how
far over and the factor), writes `--json FILE` and appends to
`--markdown FILE` (CI passes `$GITHUB_STEP_SUMMARY`). Exit 0 = all pass,
1 = a workload is over its limit or did not run, 2 = bad arguments or
threshold file. `--runs N` (default 5) and `--dir DIR` apply as usual;
`--quick`/`--deep` are refused, the sizes come from the file. The logic
(file format, comparison, reports) is `xtask/src/perfcheck.rs`, unit-tested
in the plain `xtask` bin so `cargo test --workspace` covers it.

What is gated (all at **full size**, the size the targets are stated for):

| Workload | What | Target (reference machine) | M4 Pro best, 2026-11-13 |
|---|---|---|---|
| `recalc_chains` | first recalc, 100 chains x 1000 deep (99,900 formulas) | < 500 ms | 19 - 20 ms |
| `recalc_mix` | first recalc, 100k SUM + 100k exact VLOOKUP, 1M cells | < 3 s (proposed gate) | 159 - 174 ms |
| `incremental_10k` | 1 edit -> 10k dependents | < 20 ms (proposed gate) | 1.1 - 1.4 ms |
| `open_ods_50mb` | `EngineService::open` of a 50 MB ODS (465k rows x 12) generated in the job | < 10 s | 5.6 - 6.9 s |
| `chart_10k` | `chart_data` of a 10k-point line chart, cache miss | < 20 ms | 2.4 - 2.8 ms |
| `pdf_20p` | `prepare_print_job` + `print_job_pdf`, 20 pages | < 1 s | 10.8 - 17 ms |
| `pivot_100k` | `RefreshPivot` over 100k source rows | < 500 ms | 28 - 32 ms |

Not gated: 60 fps grid scroll (measured in the webview, `pnpm perf:e2e`,
not by this tool); XLSX open/save, ODS save, CSV import and peak RSS (in
`cargo xtask perf`, not in the check: they add minutes and the RSS reading
uses macOS `/usr/bin/time -l`). Nothing is gated at the quick size; the
file can say `quick` for a workload that proves too heavy for a runner, and
the report then prints "quick" next to it.

**How noise is handled.** Each workload is run several times in one
process (5 by default; 3 for `recalc_mix` and `open_ods_50mb`, which take
seconds; 15 for the 1 - 3 ms `incremental_10k` and `chart_10k`) and the
**best** run is compared with the limit: interference from other tenants
on a shared runner only ever adds time, so the minimum is the most stable
statistic (the median is reported next to it). Limits are **explicit
per-OS values** in the file, not a factor applied to the Mac numbers and not
normalised by a calibration loop: the workloads are allocator-, memory- and
(for the open) file-bound, the gaps between the runners come largely from
the allocator, the core count (the parallel recalc pre-pass uses 3 - 4
cores there, 12 here) and Defender on Windows, which a fixed CPU loop does
not see, and dividing by a second noisy timing adds noise. Explicit values
are also what a reviewer can read.

**Threshold file** (`xtask/perf-thresholds.txt`): one line per workload,
`workload size target_ms macos linux windows`, limits in ms for the best
run; `123?` marks a limit not yet calibrated on that OS's runner (it gates
anyway; the report prints "uncalibrated"), `-` means not gated on that OS.

**Status: every limit is a guess until the first CI runs.** Only the M4 Pro
numbers above are measured. Windows and Linux runners have never run these
workloads, and neither has the macos-15 runner (Apple M1 VM, 3 cores, 7 GB).
The initial limits are M4 Pro best x an assumed runner slowdown (macos-15
2x, ubuntu-24.04 2.5x, windows-latest 3x) x 2 for noise and runner-to-runner
variation, at least 10 ms:

| Workload | macos-15 | ubuntu-24.04 | windows-latest |
|---|---|---|---|
| `recalc_chains` | 80 ms? | 100 ms? | 120 ms? |
| `recalc_mix` | 640 ms? | 800 ms? | 1000 ms? |
| `incremental_10k` | 10 ms? | 10 ms? | 10 ms? |
| `open_ods_50mb` | 24 s? | 30 s? | 36 s? |
| `chart_10k` | 10 ms? | 15 ms? | 15 ms? |
| `pdf_20p` | 45 ms? | 55 ms? | 65 ms? |
| `pivot_100k` | 115 ms? | 140 ms? | 170 ms? |

The `open_ods_50mb` limits are above the 10 s target on purpose: a slower
runner is not the reference machine. Every other limit is below its
target, so passing CI also shows that target holds on runner hardware.

**Updating thresholds.** After a CI run, the job summary and the
`perf-<os>` artifact (JSON: every sample, best, median, limit, ratio) show
the numbers. To calibrate an OS: take the slowest best-of-runs from at
least three runs on that OS, set the limit to 1.75x that, rounded up, and
drop the `?`. That catches a regression of about 2x while leaving room for
the spread between runner machines. A change that makes a workload slower
on purpose raises the limit in the same commit, with the reason in the
commit message; a faster one may lower it. Never raise a limit just to make
a red run green without looking at the numbers.

**Workflow** (`.github/workflows/perf.yml`): release build of the `perf`
bin and `perf --check` on macos-15, ubuntu-24.04 and windows-latest, on
pushes to main that touch Rust code, `xtask`, the lock file or the
toolchain, and on manual dispatch (any branch). Not on pull requests: each
run is a release build per OS plus a timing check on noisy runners; a
post-merge signal is enough for regressions that are rare, and a timing
gate should not block contributors. Expected duration per OS (estimated,
not yet observed): release build 5 - 10 min cold (Windows the slowest),
about 1 min with the cache, then 1 - 3 min for the check (26 s here; the
50 MB ODS is built, recalculated and saved in the job, about 7 s here, 3
opens). Peak RSS of the whole check is about 2.9 GB here (generating the
50 MB file), within the 7 GB of the smallest runner (macos-15). No
webview libraries are needed: the `perf` bin pulls in the engine and the
file readers, not Tauri.

## Proposed 1.0 gate (on an M-series Mac, release build)

The original proposal, before the CI gate above. Thresholds are about 2x
the baseline for things that are fine and the target for things that are
not yet. A regression of more than 25 % against the recorded baseline
should also be investigated.

| Check | Pass |
|---|---|
| Full recalc, 100k formulas with chains | < 250 ms |
| Full recalc, 1M cells with SUM/VLOOKUP | < 3 s (0.26 s with the lookup index; 7.2 s before) |
| Incremental recalc, 10k dependents | < 20 ms (hard cap 50 ms) |
| Open + save 10 MB ODS and XLSX | each < 3 s |
| Open 50 MB XLSX and 50 MB ODS | opens, each < 12 s (ODS 7.1 s, XLSX 7.4 s) |
| Save 50 MB | < 15 s (ODS 5.1 s, XLSX 7.9 s) |
| Peak RSS opening 50 MB | < 2 GB (aim < 1 GB; 0.9 - 1.1 GB now) |
| CSV import, 1M rows | < 3 s (1.2 - 1.35 s now) |
| `chart_data`, 10k points | < 20 ms (about 3 ms) |
| PDF export, 20 pages | < 1 s (about 12 ms) |
| DataPilot refresh, 100k rows | < 500 ms (about 30 ms) |
| Grid scroll on the real webview, 1M cells | p95 frame < 20 ms, p99 < 34 ms, no frame > 100 ms during a 10-row/frame fling |
| Grid scroll in headless Chromium (this spec) | p95 < 67 ms, p99 < 100 ms, to catch regressions only |

## Hot spots (1 - 3 fixed, 4 partly; 5 open)

1. **Exact-match VLOOKUP was linear and uncached** (fixed 2026-10-23):
   70 us per lookup into 10k rows, 100k lookups 7 s. Now a per-recalc
   index of each lookup line (`tally_engine::lookup_cache`,
   `tally_functions::LookupIndex`: a hash of the text plus a sorted vector
   of the numbers, built on a line's second request, shared by VLOOKUP,
   HLOOKUP, MATCH and LOOKUP, dropped at the end of every recalc, at most
   256 MB of indexes per recalc, a line that does not fit is never rebuilt)
   answers
   exact lookups; sorted lookups and wildcard/regex keys scan its cached
   cells. `cargo xtask perf --only recalc` on the same machine, under load
   from other builds, went from 13.4 s to 0.26 s for the mix (hard recalc
   11.3 s to 0.19 s).
2. **Fixed: a 50 MB ODS could not be opened** (512 MiB cap on one
   uncompressed zip entry; ODS XML compresses about 12x). `content.xml` and
   XLSX worksheets/shared strings may now be 2 GiB (about a 170 MB file);
   memory is bounded by the cell and text budgets, a 64 MiB per-XML-event
   cap and a per-package attribute budget instead (`docs/security.md`
   section 1). It opens in 7.1 s at 1.37 GB peak RSS. A numeric 100 MB ODS
   of this shape (about 11 M cells) is refused by `max_cells` (10 M), not by
   a byte cap.
3. **Memory** (fixed 2026-10-27, more possible): was 1.4 - 1.6 GB RSS
   (1.93 GB peak footprint) for a 474k-row x 12-column XLSX, about 280
   bytes per cell; now 0.9 - 1.1 GB RSS. `perf --mem` showed where it went
   (live malloc bytes, 5.7M cells, 948k formulas): the decoded model was
   613 MB (64-byte `CellData` in a Vec grown to 8.4M slots, plus one formula
   text per formula cell), the workbook 1.1 GB, and the open held both, plus
   the 50 MB file, at the peak. Changes, each with the same result (see
   "Guards" below):
   - `EngineService::open` drops the file bytes after decoding and builds
     with `Workbook::from_data_owned`, which frees each cell's model data
     (formula texts, unshared texts) as it is loaded and shrinks the model
     list as it goes.
   - Formula ASTs: equal neighbouring formulas in a column (a fill-down of
     `=A1*B1`, `=A2*B2`, which parse to the same AST because relative
     references are offsets) share one `Arc<Formula>`, as shared-formula
     groups already did.
   - Cell maps are packed after a load (`BTreeColumn::compact`: a bulk
     build of the B-tree; ascending inserts leave leaves about 6/11 full).
   - The load no longer grows the edit log to one entry per cell (it was
     discarded at the end of the load anyway).
   - Recalc: single-member SCCs are inline (`SmallVec`), and Tarjan frees a
     node's successor list when the node is finished, instead of holding
     two small allocations per formula until the end (the recalc after an
     open grew the footprint by 230 MB, now 160 MB).
   - ODS and CSV readers share the allocation of repeated cell texts (at
     most 65,536 distinct texts of up to 64 bytes).
   Workbook after open: 1.1 GB -> about 600 MB live. CSV 1M rows: peak RSS
   0.94 - 1.12 GB -> 0.61 - 0.74 GB.
   **Remaining** (about 105 bytes per cell in the engine, plus the model
   while loading): `FormulaCell` is 160 bytes (rarely used fields such as
   `strict_precedents`, `dynamic_deps`, `invalid_text`, `array`,
   `array_result` take about 80 of them; boxing them is about 70 MB on this
   file but touches about 100 call sites), the dependency back-index keeps a
   `SmallVec<[Loc; 4]>` (about 112 bytes) per formula (about 110 MB here),
   `Value` and `CellContent` are 24 bytes because `Arc<str>` is a fat
   pointer, the model's `CellData` is 64 bytes, and on macOS freed large
   blocks (Vec growth, hash-table resizes) stay in the footprint, so the
   peak is 0.2 - 0.4 GB above the live bytes. Getting well under 1 GB needs
   a block column store (dense runs of numbers as `Vec<f64>` per block,
   strings as interned ids) and a compact formula-cell table; that is a
   storage rewrite (estimated 40 - 60 bytes per numeric cell instead of
   about 80), not done here.
4. **Open and save time is roughly linear at 14 - 17 us per row**, with XLSX
   save slower than ODS save at 50 MB (7.6 s vs 5.1 s). The CSV import was
   7.6 s for 47 MB (3.9 s on a quiet machine on 2026-10-27); it is now
   1.15 - 1.35 s (about 3x on the same machine). Profiling (`sample`) showed
   the time in the cell loop, not in parsing: per cell, the input
   recognizer (lower-casing the boolean names, `parse_unsigned` building a
   String), `Local::now()` for the recognizer's "today", and for every date
   a style intern keyed by a `Debug` string. Now plain decimal numbers
   (`-?d+(.d+)?`, `.` only where it is the locale's decimal separator) skip
   the recognizer, a load reads today's date once, remembers up to 65,536
   recognized inputs of up to 64 bytes (a text result reuses the reader's
   allocation) and the style each implied format leads to. What is
   left: B-tree inserts, freeing the model, and the recognizer for distinct
   texts; reading the CSV straight into the workbook (no model) would save
   the model's memory and roughly 0.2 s more. XLSX/ODS opens spend about
   4.2 s of 7 s in the readers (XML parsing) and 0.7 s in recalc.
5. **Scrolling**: a fast vertical fling drops to 2+ vsyncs on about 40 % of
   frames in headless software rendering; worth re-measuring in the Tauri
   webview before deciding whether the tile renderer needs work.

Incremental recalc, deep chains and plain SUM recalc are all fast; nothing to
do there. `chart_data`, PDF export and the DataPilot refresh are 10 - 100x
under their gates, so they were not profiled.

### Guards for the 2026-10-27 changes

- Full suite (1,756 tests) passes, including the new
  `crates/tally-engine/tests/bulk_load.rs`: plain numbers give bitwise the
  recognizer's value and format in all six locales (fixed list plus 400
  proptest cases of number-like strings); a CSV-like sheet of every input
  kind (numbers, grouped, percent, currency, booleans, dates, times,
  fractions, `'`, `=`, spaces), repeated and on General / percent / `@`
  columns, loads to the same `to_data()` as typing each cell, with and
  without "Evaluate formulas", in all six locales; owned and borrowed loads
  agree on every fixture under `tests/fixtures`; fill-down formulas share
  an AST and still evaluate per row.
- Before/after differential: `perf --dump` (digest of `to_data()` cells,
  style runs and styles) of the build at `c53b362` and of this build agree
  on the 47 MB CSV, the 50 MB XLSX and ODS, and two random CSVs (2.6M and
  0.4M cells, 27 kinds of fields).
