# Security review 3 (f6193fa -> aa617a3): v0.3 engine, readers and shell

Worktree: `scratchpad/sec3-wt` (detached at aa617a3, removed at the end). Probe harness: `scratchpad/sec3/probe` (a scratch crate depending on the worktree's `tally-engine`/`tally-api`/`tally-io-xlsx`/`tally-numfmt`; `probe open FILE` with env flags `VIEW`, `NOTE`, `EDITS`, `UNPROTECT`, `CF`, `VAL`, `FILL`, `LINKS`, `SAVE`, `RECALC`, and `probe op <name> N` for API operations). Generators: `scratchpad/sec3/gen_ods.py` and `gen_xlsx.py` (stdlib zipfile; run with `python3 -I`); fixtures in `scratchpad/sec3/f/` (everything over 1 MB was deleted afterwards). Every run goes through `scratchpad/sec3/r.sh` = `run.py` with a 60 s wall-clock and a 6 GB RSS watchdog; "viewport" means `cells(A1:T1000)` (20,000 cells, what one scroll asks for). Release build without LTO on a 12-core Mac; RSS is macOS RSS (compressed memory understates repetitive data). No repo file was modified and nothing was committed.

Context that applies to every timing finding: the Tauri shell serializes every command on one service mutex (`apps/desktop/src-tauri/src/commands.rs:49 with_service`), so each hang below freezes every open document, and nothing can cancel an engine call.

## Confirmed findings

### CRITICAL-1: Conditional-format rules are re-evaluated for every cell on every viewport fetch, with no budget
`crates/tally-engine/src/condfmt.rs:1468-1608` (`CfEval::outcome`: for each view, every format on the sheet is tested with `contains` (up to 1,024 ranges each) and every rule formula is evaluated with `FExpr::eval`), `crates/tally-engine/src/service/view.rs:267-345` (`decorate` also inserts every cell of every format range into a `BTreeSet` first), `crates/tally-engine/src/fexpr.rs:136` (the per-formula work budget of 50M applies to one evaluation; there is no budget per `cells()` call and no cache across calls: the "second" fetch always costs the same).

| Input | Generator / command | open | viewport fetch | RSS |
|---|---|---|---|---|
| 10,000 `formula-is([.A1]>0)` formats on A1:T1000, ODS, 25 KB | `gen_ods.py cf f/cf10k.ods 10000 1 100` | 55 ms | > 60 s (killed) | 108 MB |
| same, XLSX (`cfRule type="expression"`), 58 KB | `gen_xlsx.py cfsame f/xcfsame10k.xlsx 10000` | 21 ms | 56.8 s | 37 MB |
| 1,000 stacked rules, XLSX, 7 KB | `gen_xlsx.py cfsame … 1000` | 3 ms | 5.5 s, every fetch | 19 MB |
| 1,000 formats × 1,024 ranges each, ODS, 0.9 MB | `gen_ods.py cf f/cf1k_1024.ods 1000 1024 100` | 142 ms | 36.2 s | 56 MB |
| 10,000 formats with 8 KiB formulas, ODS, 588 KB | `gen_ods.py cf f/cf10k_8k.ods 10000 1 8192` | 145 ms | 6.5 s per fetch; `conditional_formats()` 43 ms | 590 MB |
| one rule `SUM($A:$A)>A1` over 1M numbers (API) | `probe op cf_scan 1000000` | setup 0.29 s | > 60 s (killed) | 166 MB |
| one rule `SUM(Other.A:A)>A1`, 100k rows on another sheet | `probe op cf_sheets 100000` | — | 316 ms for 1,000 matching cells | 30 MB |
| 10,000 formats via the API (`AddConditionalFormat` × 10,000 in one transaction) | `probe op cf_many 10000` | apply 4.1 s, 1.9 GB (see CRITICAL-2) | 55.8 s | 1.9 GB |
| `INDIRECT("A"&ROW())>0`, 100k rows | `F='INDIRECT("A"&ROW())>0' probe op cf_scan 100000` | — | 40 ms | 28 MB |

MAX_CF_PER_SHEET is 10,000 and MAX_RANGES 1,024, so the readers and the API both accept these. Impact: a 25 KB file opens in 55 ms and then freezes the app (all documents) for a minute on the first paint and again on every scroll. Fix: (1) give each `cells()` call a work budget (for example 20M element operations across all rule evaluations; past it stop decorating the rest of the call, mark the block, and show a one-time warning); (2) cache per (format id, document version) the values of rule subexpressions that have no relative reference (`SUM($A:$A)` is the same for every cell) or, simpler, cache per (format, cell, version) the rule outcome; (3) index formats by row/column buckets or an interval tree so `contains` is not O(formats × ranges) per cell, and let `decorate` iterate the intersection once instead of per format; (4) lower MAX_CF_PER_SHEET (1,000) and cap the number of formats that may cover one cell.

### CRITICAL-2: Every v0.3 meta op clones the whole `SheetMeta` twice, and the undo stack keeps the clones
`crates/tally-engine/src/ops_v03.rs:37-55` (`with_meta`: `sheet_meta()` snapshots the meta for undo (`undo.rs:459-464`, a full clone) and then `s.meta.clone()` again as the working copy). `SheetMeta` (`storage.rs:589-614`) holds every conditional format (with all ranges and parsed formulas), every validation, every note, every link, the protection record and the page setup. The undo stack keeps 100 entries (`undo.rs:14`). Cost per op is O(size of all metadata), so N ops in one transaction are O(N²) in time and memory, and the memory stays after the transaction.

| Command | time | peak RSS |
|---|---|---|
| `probe op notes 10000` (10,000 `SetNote` in one transaction) | 1.42 s | 5.07 GB (stays after undo) |
| `probe op notes 30000` | 12 s | killed at 6 GB |
| `probe op links 100000` (`SetLink` × 100k, one transaction) | 50 s | killed (4.1 GB) |
| `probe op val_many 10000` (`AddValidation` × 10k; also `subtract_all` over every older validation) | 7.9 s | 5.5 GB |
| `probe op cf_many 10000` (`AddConditionalFormat` × 10k, 1 range each) | 4.1 s | 1.9 GB |
| `K=1024 probe op cf_many 1000` (1,000 formats × 1,024 ranges) | 1.3 s | 5.3 GB |
| `EDITS=100 NOTE=1 probe open f/notes100k.ods` (100k notes from a 4 MB ODS, then 100 single `SetNote` edits) | 4 ms per edit | +13 MB per edit, 1.36 GB after 100 edits |
| `EDITS=20 NOTE=1 probe open f/cf1k_1024.ods` | 2 ms per edit | +16 MB per edit |
| `probe open f/comments100k.xlsx` (100k XLSX comments) | 3.4 ms per edit | +12 MB per edit |

All of these are within the shell's limits (MAX_OPS 100,000, 64 MiB of strings). Impact: memory exhaustion from one transaction, or from ordinary editing (100 note or link edits on a sheet with 100k notes or 1k×1k conditional-format ranges hold 1.3–1.6 GB in the undo stack). Fix: record inverse deltas for undo (the replaced note/link/format/validation, not the whole meta), keep the collections behind `Arc` with copy-on-write so an unchanged collection is shared, and cap the number of meta ops per transaction (for example 1,000) in `ops::op_cost`.

### HIGH-3: XLSX hyperlink ranges are expanded per cell with a cloned URL before any cap
`crates/tally-io-xlsx/src/read.rs:130-157`: each `<hyperlink ref=…>` is expanded to one `LinkData` per covered cell (cap 10,000 cells per ref, 100,000 refs per sheet, `sheet.rs:197`) with `url.clone()`; the target length is not capped anywhere in the reader (`workbook.rs:80-109 parse_rels`, `sheet_v03.rs:649-657`), and the engine's 8 KiB link cap (`convert.rs:527`) runs only after the whole model exists.

| File | size | result |
|---|---|---|
| `gen_xlsx.py bigtarget f/bigtarget.xlsx 8000000 10000` (one 8 MB external target on A1:J1000) | 5.4 MB | killed at 6.58 GB after 0.5 s |
| `gen_xlsx.py bigtarget f/bigtarget1m.xlsx 1000000 10000` | 678 KB | open 2.9 s, 2.56 GB peak (links then dropped by the engine) |
| `gen_xlsx.py bigtarget f/bigtarget64k.xlsx 65536 10000` | 46 KB | open 46 ms, 794 MB |
| `gen_xlsx.py links f/links100k_10k.xlsx 100000 10000` (100k refs × 10k cells, `location` only) | 15 KB | > 60 s, 1.8 GB and growing |
| `gen_xlsx.py links f/links1k_10k.xlsx 1000 10000` | 1.6 KB | 0.73 s, 545 MB |

Fix: cap the relationship target and `location` at 8 KiB when parsed; share the URL (`Arc<str>`) across a ref's cells; count expanded link cells against the reader's cell budget and stop at 100,000 links per sheet before expanding (not `sort`+`dedup` after).

### HIGH-4: Data-validation checks are O(written cells × list items) and re-evaluate custom formulas per cell
`crates/tally-engine/src/validation.rs:467-514` (`is_valid`: `list_values` rebuilds the list for every checked cell (up to 10,000 values from the range), then `compare` lowercases both sides per pair; `Custom` and `Compare` call `eval_fresh` per cell), `:527-571` (`check_cells` clones every validation into an `Arc` per op and does a linear `find` per cell).

| Command | result |
|---|---|
| `probe op val_list 10000` (list source `$B:$B` with 10,000 text items, then `FillRange` of 100,000 cells) | > 60 s (killed) |
| `VIEW=1 VAL=1 FILL=100000 probe open f/vals10k.ods` (10,000 list validations, numeric items, fill 100k) | FillRange 2.9 s |
| `FILL=10000 probe op val_custom 1000000` (custom rule `SUM($B:$B)>A1`, 1M numbers, fill 10,000 cells) | 30.9 s |
| `probe op val_many 10000` then `FillRange` of 1M cells in a column without validation | 76 ms (fine: the linear find is cheap when nothing matches) |

Fix: resolve each validation's list once per `check_cells` (a `HashSet` of normalized keys), give validation the same per-op work budget as CRITICAL-1 (past it, accept with a warning or refuse the op), and index validations by cell.

### MEDIUM-5: OOXML `spinCount` is honoured up to 10,000,000 hash rounds per unlock attempt
`crates/tally-engine/src/protection.rs:394` (`spin.min(10_000_000)`), read unbounded at `crates/tally-io-xlsx/src/sheet_v03.rs:478`. Each attempt runs under the global lock, and a wrong password costs the same.

| File | per `UnprotectSheet` attempt |
|---|---|
| `gen_xlsx.py spin f/spin.xlsx 4294967295` (1.4 KB) | 3.1–3.3 s |
| `gen_xlsx.py spin f/spin10m.xlsx 10000000` | 3.0–3.3 s |
| `gen_xlsx.py spin f/spin100k.xlsx 100000` (Excel's default) | 30 ms |
| `gen_xlsx.py salt f/salt16m.xlsx 16000000` (16 MB random salt, 16 MB file) | 365 ms (and the salt is cloned with every meta op, CRITICAL-2) |

Fix: clamp `spinCount` to 100,000 at read and in `ooxml_hash`; cap `saltValue`/`hashValue` at 1 KiB (and ODS `table:protection-key` likewise; `gen_ods.py prot f/prot16m.ods 12000000` loads a 12 MB key and saves it back, 12 MB per undo snapshot).

### MEDIUM-6: XLSX hyperlink relationship lookup is linear per link
`crates/tally-io-xlsx/src/read.rs:131-135` (`sheet_rels.iter().find(…)` for each of up to 100k links). `gen_xlsx.py rels f/rels100k.xlsx 100000` (1 MB, 100k links each with its own relationship): open 8.8 s, 55 MB. Fix: a `HashMap<id, &Rel>`.

### MEDIUM-7: The browser dev mock engine ships in the desktop bundle
`apps/desktop/src/main.tsx:7,18`: `WebMockClient` is imported statically from `@tally/app` (`packages/app/src/index.ts:19`) and chosen by a runtime `isTauri()` test, so Vite cannot drop it. Verified by `pnpm --filter tally-desktop build:web` in the worktree: `dist/assets/index-*.js` (687 KB) contains string literals that exist only in `packages/app/src/dev/webMock.ts` ("That column is not in the AutoFilter."), `dev/mockV3.ts` ("a conditional format needs a range and a rule"), `packages/bridge/src/mock.ts` ("block too large for MockClient") and the `__tally` e2e hook. Impact: bundle size, and a complete in-page engine plus the `window.__tally` controller hook are reachable whenever `isTauri()` is false (the hook is only installed in that branch, so inside the Tauri webview it is dormant). Fix: `const { WebMockClient } = await import("@tally/app/dev")` behind `import.meta.env.DEV`, or a separate dev entry file; stop re-exporting `dev/` from the package root.

### MEDIUM-8: Note text from files is not capped per note and is not budgeted
`crates/tally-io-ods/src/v03.rs:1227-1258` (`parse_annotation`: up to 4,096 paragraphs of 64 KiB each, so 256 MiB per note; note bytes are not counted in `max_text_bytes`), XLSX `parse_comments` 64 KiB per comment (bounded by the 64 MiB part cap). `gen_ods.py notebig f/notebig.ods 4096 8192` (22 MB): opens in 112 ms, A1 holds a 33.5 MB note; `note(A1)` returns 33.5 MB to the webview (the popover renders it as one text node) and `notes()` the same; ODS save re-emits it (22 MB), the XLSX writer silently drops it (5 KB file, only an author-length warning). The API caps `SetNote` at 32 KiB (`ops_v03.rs:19`), so this is import-only. Fix: truncate imported notes to MAX_NOTE_LEN with a warning and count note bytes in the text budget.

### LOW-9: Protection gaps (ops that still apply on a protected sheet)
Measured with `probe op protect 0` (every op after `ProtectSheet` with a password; "applied" = no error and effect visible):
- Applied: `SetFrozen`, `SetPageSetup`, `SetPageBreak`, `RefreshFilter`, `DefineName` (redefining a workbook name the sheet uses), `DeleteName`, `RenameSheet` (structure protection is a separate flag, as in Excel). `ops_v03.rs:377-413` has no `check_sheet_action` for page setup/breaks; `ops.rs:347-411` none for `SetFrozen` and names.
- `undo` after `ProtectSheet` silently unprotects the sheet and the next `undo` reverts the edit made before protecting (Excel clears the undo stack on protect).
- `goal_seek` runs on a protected sheet (read-only clone; the UI applies through the gated `SetCellInput`, fine).
- Hidden formulas (`SetCellProtection hidden_formula`): `cell_edit_info` returns "" (good), `clipboard_copy` gives values only (good), but `find` with `in_formulas` still reports the hidden cell as a hit (`probe op hidden 0`).
- Refused as expected: every cell write path (`SetCellInput`, `SetRangeInputs`, `Clear`, `Paste`, `PasteSpecial`, `FillRange`, `SetArrayFormula`, `MoveRange`, `SortRange`, `ReplaceAll` (post-check reverts), `MergeCells`, `SetAutoFilter`, `SetColumnFilter`, `InsertRows/Cols`, `DeleteRows/Cols`, `SetColWidth`, `SetRowHeight`, `SetRows/ColsHidden`, `AutoFitRows`, CF/validation/note/link writes, `SetCellProtection`, wrong/empty `UnprotectSheet`, `ProtectSheet` twice).
Fix: gate `SetPageSetup`/`SetPageBreak`/`SetFrozen` behind `format_cells`; keep `ProtectSheet` off the undo stack (or make undo re-check protection); skip hidden-formula cells in formula search.

### LOW-10: Imported hyperlinks keep hostile targets and are re-exported
`gen_ods.py links f/links.ods 100` (`LINKS=1 probe open`): `javascript:alert(1)`, `JaVaScRiPt:`, `java\nscript:`, `%6A%61vascript:`, `file:///etc/passwd`, `\\server\share\x`, `data:text/html,…`, `vbscript:`, `smb://`, ` javascript:1`, `https://example.com/\0`, a 8,192-byte URL and `mailto:?attach=/etc/passwd` all load (`content.rs:864-870` only caps at 8,192 bytes), reach the webview in `CellView.link`, and are written back by both writers. The engine's scheme check exists only for `SetLink` (`ops_v03.rs:67-87`); the shell's `open_url` (`links.rs:32-72`) and the UI's `isOpenableUrl` (`packages/app/src/v03.ts:47-61`) are the only gates, and they hold (verified by reading and by the links.rs unit tests). Fix: apply `check_url` at import (drop or neutralize `javascript:`/`data:`/`vbscript:`, control characters), so Tally is not a carrier for other consumers.

### LOW-11: `open_url` on Windows goes through `cmd /C start` (by reading)
`links.rs:76-86`: `^`-escaping covers `^ & | < > % ( ) !`, and `"`, whitespace and control characters are refused, but cmd's other delimiters (`,` `;` `=`) are not escaped, so `start "" https://host/a,b` is parsed by `start` as a document plus parameters (the browser receives `b` as an extra argument). Not exploitable for code execution with the current character set, but fragile. Fix: call `ShellExecuteW` (the `windows` crate) or `rundll32 url.dll,FileProtocolHandler <url>` with the URL as one argument, no shell.

### LOW-12 (correctness): undo of `DeleteSheet` leaves CF/validation formulas at `#REF!`
`probe op cf_sheets 100000`: a rule `SUM(Other.A:A)>A1` and a list validation on `Other.$A$1:$A$10` become `SUM(#REF!)` / `#REF!` when `Other` is deleted (expected) and stay so after the delete is undone; the validation then rejects every entry (Stop). Cell formulas are restored by the same undo. Fix: snapshot the meta of every sheet (or the adjusted formulas) in the `DeleteSheet` undo record.

### INFO
- Tauri `limits.rs` coverage (every registered command, `lib.rs:41-81`): `wb_new` template id ≤ 4 KiB; `wb_open` path ≤ 4 KiB plus the allowlist; `wb_open_dialog`/`wb_save`/`wb_close`/`undo`/`redo`/`sheet_layout`/`names`/`filter_info`/`recalc_all`/`conditional_formats`/`validations`/`notes`/`protection_info`/`page_setup`/`document_properties`/`list_locales`/`list_functions`/`recent_files`/`list_templates`/`user_name` take ids or nothing; `get_cells` ≤ 10,000 ranges inside the sheet (engine: ≤ MAX_CELLS_PER_CALL); `get_styles` ≤ 65,536 ids; `cell_edit_info`/`validation_at`/`note`/`navigate_edge` positions; `selection_stats`/`clipboard_copy`/`assist_suggest` selections (query ≤ 64 KiB); `resolve_name_box(_in)` ≤ 1,024 chars; `find` text ≤ 64 KiB; `detect_header` range; `goal_seek` cells + finite target; `set_locale` ≤ 64 bytes; `open_url` ≤ 2,048 bytes and scheme check; `apply`: ≤ 100,000 ops, label ≤ 256 chars, every string ≤ 64 KiB (TSV ≤ 64 MiB), all strings ≤ 64 MiB, every `{r0,c0,r1,c1}`/`{r,c}` inside the sheet, `ranges` lists ≤ 10,000, typed checks for spans, px, names, sort keys, filters, page setup repeats. Strings the shell only bounds at 64 KiB but the engine caps lower: `SetNote.text` 32 KiB, `SetLink.url` 8 KiB, header/footer 4 KiB, doc properties 64 KiB / 1,000 keywords, number formats 4,096, `DefineName.refers_to` (parser 8,192). Array *counts* are not capped by the walk (only `ranges`): CF rules per format (engine 64), CF ranges (engine 1,024), list items (engine 10,000), filter values (engine 100,000), note/link/validation/CF ops per transaction (none, see CRITICAL-2). `user_name` returns only `$USER`/`$USERNAME`/`$LOGNAME`, trimmed to 256 printable characters. Open/save IO errors use `std::io::Error::to_string()` (no path); document and sheet names come from the file stem only.
- Goal seek (`probe op goalseek 250000`, 1M dependent formulas): clone + 4 evaluations 1.5 s, +400 MB; `=RAND()*A1` target: 32 evaluations, 11.5 s (the 100-evaluation bound holds, under the global lock); cycle: `found=false, result=NaN` (serialized as `null` by serde_json, the UI should not print it); variable = formula cell refused; target `1e308`: 100 evaluations, 124 ms.
- Colour-scale / duplicate / top-N aggregates are cached per document version: 1M rows compute in 50–70 ms on the first fetch after any edit (an unrelated edit invalidates all aggregates), 1.4 ms cached (`probe op cf_agg 1000000`, `f/cfagg.ods`).
- `set_locale` is O(1) (the pointer changes; formatting is lazy per view): 1M formatted cells, every locale, viewport 4–5 ms; unknown tags refused; a bare `de` matches `de-DE`.
- `recognize_input` and `format_value_with`: 120,474 random/edge inputs across the six locales and 18,792 formats × values (ja-JP era codes `g/gg/ggg/e/ee/r`, DBNum/NatNum, extreme serials, both epochs, width 1–3) with no panic (`probe op recognize 20000`).
- Page setup: `&[FILE]` is written as `<text:title>???</text:title>` (ODS) and `&F` (XLSX: file name, not path); `&[PATH]`/`&[TAB]` stay literal; header/footer text is capped at 4,096 characters, NaN margins refused, print range clamped to the sheet, document properties capped (64 KiB fields, 1,000 keywords), `created`/`modified`/`generator` from the UI are ignored. An ODS header over 64 KiB rejects the whole file (`f/header1m.ods`: `limit exceeded: header text`), a 60 KB one loads and saves.
- ODS conditional-format formulas over 8,192 characters become invalid rules (`#NAME?`, cheap); 10,000 × 8 KiB formulas parse in 145 ms (CRITICAL-1 still applies to their evaluation).
- A validation whose list range points at another sheet works; after that sheet is deleted every entry is refused (LOW-12).
- The ODS legacy Excel hash `CE4B` is the hash of the empty password, so an XLSX with `password="CE4B"` unlocks with "" (by design; Excel omits the attribute instead).

## Refuted or not reproduced
- `cells()` output size with a long link: links over 8 KiB never reach the view (dropped at `convert.rs:527`), so a 1 MB URL on 10,000 cells does not inflate the viewport response (the damage is at read time, HIGH-3).
- 10,000 conditional formats that do **not** overlap the viewport (`f/xcf10k.xlsx`, one 1,000-row block each): 56 ms per fetch. The cost is per format *covering* a cell.
- Validation dropdown flags: 10,000 list validations with dropdowns (`f/vals10k.ods`): 29 ms per fetch.
- Notes/links at 100k per sheet from files: `f/notes100k.ods` 168 ms, `notes()` 8 ms (5 MB); `f/comments100k.xlsx` 90 ms; `f/links1k…` fine except HIGH-3.
- ODS `table:protection-key` of 12 MB (random): open 69 ms, unlock attempts < 1 ms (the hash is compared as a string).
- Recognizer/formatter panics with locale-specific input: none found.
- `ApiError` messages carrying absolute paths: none (std IO error text only).
- Frontend sinks: no `innerHTML`, `dangerouslySetInnerHTML`, `DOMParser`, `insertAdjacentHTML`, `document.write`, `eval`, `new Function`, `window.open` or `href=` in `packages/app`, `packages/grid`, `packages/bridge`, `apps/desktop/src`; note text, link text and CF rule text are rendered as JSX text nodes (`CellPopovers.tsx:51,184`). `localStorage` holds only the theme and `tally.recentSeen` (keyed by the ≤ 10 recent paths).

## Verified OK
- Review-2 fixes hold at aa617a3 (`fixbench.sh` with the new probe; same inputs as `fixbench-after.txt`): names fan 10×16 1.1 ms; fan 2×1000 cells 17 ms; API fan-10 names 0.5 ms; 200k ODS merges 107 ms (warning); 2M XLSX merges 478 ms; 20/200 array blocks refused in < 2 ms; 200k fan-in open 290 ms + recalc 133 ms; API fan-in edit 59 ms; 100k INDIRECT edit 134 ms; 1 MB number formats view 4–5 ms (General + warning); 100k rows × 10k filter values 12 ms; 1M shared formulas open 5.6 s + recalc 4.9 s, 2.0 GB; 10k `<col style>` 6 ms.
- Engine-side caps that worked as documented: `SetNote` 32 KiB (32,769 refused), links 8 KiB on import, 100k notes/links/validations/CFs per sheet in the readers, 10k CF per sheet and 64 rules per format in the engine, 1,024 ranges per format, list items 10,000 (a 1M-cell list source is truncated), header/footer 4,096, doc properties, page breaks 10,000, print range inside the sheet, `scale` 10–400 %, `FitTo` ≤ 1,000, margins finite and ≤ 10 m.
- Protection: all cell-write paths are gated (list in LOW-9), `SetCellProtection` is refused while protected, wrong and empty passwords are refused for SHA-256 (ODS), SHA-512 with salt and spin (XLSX), and the legacy hash; `protection_info` never returns hash material; hidden formulas are hidden from `cell_edit_info` and the clipboard.
- `open_url` (`links.rs`): http/https/mailto only, host required, ≤ 2,048 bytes, no whitespace/control/quote/backslash/angle/backtick; macOS `open` and Linux `xdg-open` receive the URL as a single argv entry; the UI mirrors the check and handles `#Sheet.A1` itself through the 1,024-character name-box path. `user_name` exposes only the account name.
- `dialogs.rs` after the XLSX addition: open filters list ods/xlsx/csv; the picked path is allow-listed before use; Save As picks the format from the chosen extension and appends one when missing; export dialogs offer the one format. (Functional note from review 2 still true: a typed `.ods` name in an "Export as CSV" dialog writes CSV bytes under `.ods`.)
- XLSX `&[FILE]` round trip and ODS `<text:title>` never carry the path; `\0` in a header is written as `_x0000_`.
- Goal seek never mutates the document (works on a clone), refuses formula variable cells and non-finite targets, and stops at 100 evaluations.

## Priority order for fixes
CRITICAL-1 (CF evaluation budget + range index), CRITICAL-2 (meta deltas instead of snapshots; this also halves the cost of every v0.3 op), HIGH-3 (XLSX link expansion), HIGH-4 (validation work), then MEDIUM-5 (spin count clamp, a one-line fix), MEDIUM-6, MEDIUM-8, MEDIUM-7, and the LOWs. Add `gen_ods.py cf`, `gen_xlsx.py cfsame/bigtarget/links/spin` and `probe op notes/val_list` with small N to the security test suites and assert both time and the cap.

## Housekeeping
The worktree `scratchpad/sec3-wt` was removed (`git worktree remove --force`); `scratchpad/sec3-target` (228 MB) was deleted; fixtures over 1 MB in `scratchpad/sec3/f` were deleted. The probe source, generators, `r.sh`/`run.py` and the small fixtures remain under `scratchpad/sec3/`.
