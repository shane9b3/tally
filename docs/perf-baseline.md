# Performance baseline (1.0 gate)

Measured 2026-10-23 on the commit after `c992a3d`, release build
(`lto = "thin"`, `codegen-units = 1`).

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
| Full recalc, 1M-cell sheet (100k rows x 10), SUM + VLOOKUP | 7.2 s (first recalc 7.2 - 10.5 s) | 200k formulas: 100k `SUM(A:H)` + 100k exact `VLOOKUP` into 10k rows |
| Same sheet, SUM column only | 124 ms | so VLOOKUP is about 7.0 s, about 70 us per lookup |
| Incremental recalc, 1 edit -> 10k dependents | 2.3 ms | `Workbook::set_input` + `recalc` |
| Same edit through `DocumentService::apply` | 2.5 ms | includes undo snapshot and change report |
| Open ODS 10 MB (93k rows x 12) | 1.67 s | |
| Save ODS 10 MB | 0.97 s | |
| Open XLSX 10 MB (95k rows x 12) | 1.93 s | |
| Save XLSX 10 MB | 1.73 s | |
| Open ODS 25 MB (232k rows) | 3.7 s | |
| Open XLSX 25 MB (237k rows) | 3.6 s | |
| Save ODS / XLSX 25 MB | 2.7 s / 5.5 s | |
| Open ODS 50 MB | FAILS | `limit exceeded: zip entry uncompressed bytes (limit 536870912)` |
| Open XLSX 50 MB (474k rows) | 7.8 - 8.8 s | |
| Save XLSX 50 MB | 7.2 - 10.4 s | |
| Peak RSS, open XLSX 50 MB | 1.4 - 1.6 GB | `/usr/bin/time -l` of a child process |
| CSV import, 1M rows x 6 cols (47 MB) | 7.6 s | |
| Peak RSS, CSV 1M rows | 0.94 GB | |

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
cargo xtask perf --only recalc,csv    # scenarios: recalc incremental io10 io25 io50 csv rss deep
cargo xtask perf --dir /path/on/big/disk
pnpm perf:e2e                         # grid scroll frame times (starts dev:web itself)
```

`cargo xtask perf` builds `xtask/src/bin/perf.rs` with the `perf` feature (the
engine dependencies are optional so the other xtask commands stay light). All
files are generated in a temp dir and deleted afterwards. The Playwright
config (`tests/e2e/perf.config.ts`) only matches `*.perf.ts`, so `pnpm e2e`
never runs it. A Vite HMR reload from edits in another terminal can abort a
run with "Execution context was destroyed"; just rerun.

## Proposed 1.0 gate (on an M-series Mac, release build)

Thresholds are about 2x the baseline for things that are fine and the target
for things that are not yet. A regression of more than 25 % against the
recorded baseline should also be investigated.

| Check | Pass |
|---|---|
| Full recalc, 100k formulas with chains | < 250 ms |
| Full recalc, 1M cells with SUM/VLOOKUP | < 3 s (baseline 7.2 s fails today) |
| Incremental recalc, 10k dependents | < 20 ms (hard cap 50 ms) |
| Open + save 10 MB ODS and XLSX | each < 3 s |
| Open 50 MB XLSX and 50 MB ODS | opens, each < 12 s (ODS fails today) |
| Save 50 MB | < 15 s |
| Peak RSS opening 50 MB | < 2 GB (aim < 1.5 GB) |
| CSV import, 1M rows | < 10 s (aim < 5 s) |
| Grid scroll on the real webview, 1M cells | p95 frame < 20 ms, p99 < 34 ms, no frame > 100 ms during a 10-row/frame fling |
| Grid scroll in headless Chromium (this spec) | p95 < 67 ms, p99 < 100 ms, to catch regressions only |

## Hot spots (not fixed; reported only)

1. **Exact-match VLOOKUP is linear and uncached**: 70 us per lookup into
   10k rows; 100k lookups take 7 s while 100k SUMs take 0.12 s. Fix
   candidates: a per-recalc sorted/hash index of the lookup column keyed by
   range and version, shared across all VLOOKUP/MATCH/HLOOKUP calls. This is
   the only thing that fails the proposed gate among the recalc numbers.
2. **A 50 MB ODS cannot be opened**: the reader's limit of 512 MiB for one
   uncompressed zip entry (`content.xml`) trips, since ODS XML compresses
   about 12x. Either raise the limit (with a total-memory check) or accept the
   cap and state it in the docs; the gate must say which. ODS 25 MB works.
3. **Memory**: 1.4 - 1.6 GB RSS for a 474k-row x 12-column XLSX (about 5.7 M
   cells, about 280 bytes per cell). CSV 1M rows x 6 uses 0.94 GB. Cell
   storage is a BTreeMap per column with `Arc<str>` text, so text-heavy sheets
   are the likely culprit.
4. **Open and save time is roughly linear at 14 - 17 us per row**, with XLSX
   save slower than ODS save at 25 MB (5.5 s vs 2.7 s). Nothing alarming, but
   XLSX save (rust_xlsxwriter) and the CSV import (7.6 s for 47 MB, about
   6 MB/s) are the next slowest after VLOOKUP.
5. **Scrolling**: a fast vertical fling drops to 2+ vsyncs on about 40 % of
   frames in headless software rendering; worth re-measuring in the Tauri
   webview before deciding whether the tile renderer needs work.

Incremental recalc, deep chains and plain SUM recalc are all fast; nothing to
do there.
