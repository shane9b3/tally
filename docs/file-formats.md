# File formats

What Tally reads and writes, per format, and where each direction loses
information. All readers produce, and all writers consume,
`tally_model::WorkbookData`; formulas in the model are always OpenFormula text
(`of:=SUM([.A1:.A3])`).

| Format | Crate | Read | Write | AOO 4.1 |
|---|---|---|---|---|
| ODS (OpenDocument spreadsheet) | `tally-io-ods` | yes | yes | read + write |
| XLSX (Office Open XML, Excel 2007+) | `tally-io-xlsx` | yes | yes | read only |
| CSV / TSV | `tally-io-csv` | yes | yes | read + write |

AOO 4.1 imports `.xlsx` but cannot save it (see
`docs/calc-features-inventory.md`, "Calc MS Excel 2007 XML"). Tally writes it.
XLSB, XLSM macros, XLS (BIFF) and strict-OOXML-only packages are not
supported; an `.xlsb` is reported as `XlsxError::Unsupported`.

## v0.3 feature coverage

| Feature | ODS read | ODS write | XLSX read | XLSX write |
|---|---|---|---|---|
| Conditional formats: cell value, formula, duplicate/unique, top/bottom N (%), above/below average, text tests, error/no error | `calcext:condition` (LibreOffice); legacy `style:map` (`cell-content()…`, `cell-content-is-between`, `is-true-formula`) when the sheet has no `calcext` block | `calcext:conditional-formats` + a `style:map` fallback on the cell styles of non-empty cells for value and formula rules (OpenOffice 4.1 shows those) | `conditionalFormatting/cfRule` with `dxfs` styles | `rust_xlsxwriter` conditional formats with dxf styles |
| Blank / not blank | `formula-is(LEN(TRIM([.A1]))=0)` recognised | written as that formula (LibreOffice has no token) | `containsBlanks` / `notContainsBlanks` | yes |
| Colour scales, data bars, icon sets | `calcext:color-scale`, `data-bar`, `icon-set` | yes (`tally:reverse` for reversed icons) | `colorScale`, `dataBar`, `iconSet` (x14 extensions ignored) | yes (data bars as Excel 2010 bars) |
| Date periods | `calcext:date-is` | yes | `timePeriod` | today … next month; years as a `YEAR()` formula rule |
| Data validation | `table:content-validations` + `table:content-validation-name` (all `cell-content-…` conditions, lists of values or ranges, help and error messages) | yes | `dataValidations` (`showDropDown="1"` means *no* dropdown) | yes |
| Notes | `office:annotation` (author, date, shown) | yes | `xl/comments*.xml` (author prefix stripped; the shown state lives in VML and is not read) | `rust_xlsxwriter` notes |
| Hyperlinks | first `text:a xlink:href` of a cell | `text:a` around the cell text | `hyperlinks` (external relationship targets, `location` → `#Sheet.A1`) | text and empty cells only (links on numbers/formulas are dropped and reported) |
| Sheet protection | `table:protected`, `table:protection-key` (+ digest algorithm), `loext:table-protection` options | yes | `sheetProtection` (legacy `password`, or `algorithmName`/`hashValue`/`saltValue`/`spinCount`; flags inverted to "allowed") | `protect_with_options` + the stored hash patched in (an ODF SHA hash uses the companion legacy hash) |
| Cell protection | `style:cell-protect`, `style:print-content` | yes | `xf/protection locked hidden` | `set_unlocked` / `set_hidden` |
| Structure protection | `table:structure-protected` on `office:spreadsheet` | yes | `workbookProtection lockStructure` | patched into `workbook.xml` |
| Page setup | `style:page-layout` (size → paper, orientation, margins, `style:print` grid/headers, `scale-to`, `loext:scale-to-X/Y`, centring, page order, first page number) + `style:master-page` header/footer regions (fields → `&[PAGE]` …) via the table style's master page; `table:print-ranges`; `table:table-header-rows/columns` | one page layout + master page per distinct setup | `pageMargins`, `pageSetup`, `printOptions`, `headerFooter` (`&L&C&R`, `&P &N &A &F &D &T`), `_xlnm.Print_Area` / `_xlnm.Print_Titles` | yes (first page number patched in) |
| Manual page breaks | `fo:break-before="page"` on row/column styles | yes | `rowBreaks` / `colBreaks` (`man="1"`) | yes |
| Document properties | `meta.xml`: title, subject, keywords, description, initial creator, creator, dates | yes | `docProps/core.xml` | title, subject, author, keywords, comments |
| Locale | default cell style `fo:language`/`fo:country` | yes | – | – |

Formulas of rules and validations are OpenFormula in the model, relative to
`base` (ODS `base-cell-address`, XLSX the top-left cell of the first range).

## XLSX (`crates/tally-io-xlsx`)

```rust
pub fn read_xlsx(reader: impl Read + Seek, limits: &Limits) -> Result<WorkbookData, XlsxError>;
pub fn write_xlsx(data: &WorkbookData, writer: impl Write + Seek) -> Result<WriteReport, XlsxError>;
pub fn sniff_is_xlsx(bytes: &[u8]) -> bool;
```

The reader is our own streaming quick-xml parser (shared formulas need
AST-level re-anchoring that string-level readers cannot give). The writer
uses `rust_xlsxwriter` (pinned `=0.100.0`).

### Import (XLSX → model)

| Part | What is read |
|---|---|
| `[Content_Types].xml` | optional; detects `.xlsb` and non-default workbook part names |
| `_rels/.rels`, `xl/_rels/workbook.xml.rels` | workbook, sheet, shared-string and style targets (relative, absolute, `..` folded); `TargetMode="External"` targets are kept only as hyperlink URLs (never opened) |
| `xl/workbook.xml` | sheet order, names, `state="hidden"`/`veryHidden`; `workbookPr date1904`; `workbookView activeTab`; `calcPr iterate/iterateCount/iterateDelta`; `definedNames` |
| `xl/sharedStrings.xml` | `<t>` and rich `<r><t>` runs concatenated; phonetic `<rPh>` dropped; whitespace kept verbatim |
| `xl/styles.xml` | `numFmts` (custom codes → OOo codes), built-in `numFmtId` 0–49 (en-US), `fonts` (b, i, u, strike, sz, name, colour rgb/theme+tint/indexed), `fills` (pattern fill fg colour), `borders` (left/right/top/bottom/start/end style + colour), `cellXfs` (font, fill, border, number format, alignment horizontal/vertical/wrapText/indent), custom `indexedColors` |
| `xl/worksheets/sheetN.xml` | `sheetPr/tabColor`, `sheetFormatPr defaultRowHeight`, frozen `pane`, `cols` (custom width, hidden, column style → whole-column style run), `row` (custom height, hidden), `c` (see below), `mergeCells` |
| `docProps/core.xml`, `docProps/app.xml` | title, created, modified, application |

Cells (`<c r s t>`):

- `t="s"` shared string, `t="inlineStr"` `<is>` text, `t="str"` formula
  string result, `t="b"` boolean, `t="e"` error (`#N/A` → `Value::Error(NA)`;
  Excel-only errors `#SPILL!`, `#CALC!`, … → `#VALUE!`, `#GETTING_DATA` →
  `#N/A`), `t="d"` ISO 8601 date, otherwise a number. Styled empty cells are
  kept as `Value::Empty`.
- `<f>` plain formulas are parsed with `Dialect::ExcelA1` and printed as
  OpenFormula. Shared formulas (`t="shared" si ref`): the anchor's AST is
  printed at every member cell that names the same `si` (members carry
  `<f t="shared" si="N"/>`; cells inside `ref` without that marker are not
  members). Array formulas (`t="array" ref`) put the formula on the anchor
  and record the block in `SheetData::array_formulas`; the other cells keep
  their cached values. Data tables (`t="dataTable"`) keep only values.
- Cached values (`<v>`) are always kept in `CellContentData::Formula::cached`.
- A formula that does not parse (structured references `Table1[Col]`,
  R1C1, …) is stored as `of:=` + the original text with its cached value;
  the engine reports Err:5xx for it until the user edits it.
- Functions resolve through the `tally-functions` registry with OOXML names
  first (`Registry::lookup_ooxml_first`: Excel's `CONVERT`, `DURATION` and
  `WEEKNUM` are the Analysis add-ins), so the model gets ODF spellings
  (`IFERROR` → `COM.MICROSOFT.IFERROR`). Export parses the model's ODF names
  ODF-first. AOO's built-in WEEKNUM (ODF `ISOWEEKNUM(d; mode)`) has no exact
  Excel function: with a literal mode other than 1 (Monday start, ≥ 4 days =
  ISO 8601) it is written as Excel's one-argument `_xlfn.ISOWEEKNUM(d)`;
  otherwise as `_xlfn.ORG.OPENOFFICE.WEEKNUM(d, mode)` (Excel shows
  `#NAME?`; Tally reads it back as the built-in). `_xlfn.` /
  `_xlws.` prefixes are stripped for known functions; unknown ones keep the
  prefix (`of:=_xlfn.NOSUCHFN(1)`) and round-trip back to Excel unchanged.

Defined names become `NameData` with OpenFormula text and absolute sheet
markers (`Sheet1!$A$1:$B$10` → `of:=[$Sheet1.$A$1:.$B$10]`); `localSheetId`
becomes `sheet`. `_xlnm.Print_Area` (one area) becomes the sheet's
`PageSetupData::print_range` and `_xlnm.Print_Titles` its repeated
rows/columns; other built-in `_xlnm.*` names are dropped (a multi-area print
area stays a sheet-local `Print_Area` name).

Ignored: `cellStyleXfs`/`cellStyles` (named styles), drawings, charts,
images, pivot tables, tables (ListObjects), external links, row styles
(`<row s customFormat>`), outline levels, VBA, x14 extensions of
conditional formats.

### Export (model → XLSX)

- Sheets in order, hidden sheets (`state="hidden"`; the active sheet is
  never hidden), active sheet, tab colours, frozen panes.
- Sheet names are made valid for Excel (31 characters, none of `[]:*?/\`,
  no leading/trailing `'`, not `History`, unique case-insensitively);
  formulas and names that refer to a renamed sheet use the new name. Each
  rename is reported in `WriteReport::warnings`.
- Values: numbers, text (truncated to Excel's 32,767 characters, reported),
  booleans, styled blanks. Error values are written as an error formula
  (`=#N/A` with the cached error), the only way `rust_xlsxwriter` can store
  one; they read back as formulas. OOo-only error codes become `#VALUE!`.
- Formulas: OpenFormula (or CalcA1 `=…`) → AST → ExcelA1 text, with the
  registry's OOXML spelling (`ooxml_name`, else the canonical name;
  `rust_xlsxwriter` adds `_xlfn.` to the future functions it knows). An
  unknown `COM.MICROSOFT.X` becomes `_xlfn.X`. Cached results are written
  with `Formula::set_result`. Formulas that cannot be translated are written
  as their cached value and reported.
- Array formulas from `SheetData::array_formulas` (`{=…}` over the block).
- Styles: font (bold, italic, single underline, strike, family, size,
  colour), solid fill (chip display styles are written as their background
  fill), borders, horizontal/vertical alignment, wrap, indent, number formats
  via `tally_numfmt::excel::to_excel_code` (untranslatable codes fall back
  to General and are reported).
- Column widths and hidden columns, row heights and hidden rows (whole
  pixels at 96 dpi; a run over all rows becomes the default row height; at
  most 100,000 individually sized rows per sheet), whole-column style runs
  as column formats, shorter style runs (≤ 4,096 rows) as styled blanks.
- Merges, defined names (workbook and sheet scope), `Print_Area` as the
  sheet's print area, document title.
- `date1904="1"` for D1904 workbooks (`rust_xlsxwriter` has no option for
  it, so the finished package is re-zipped with `xl/workbook.xml` patched).

### Formula dialect (ExcelA1)

`tally_formula::Dialect::ExcelA1`: `Sheet1!A1`, `'My Sheet'!$A$1`, 3D
`Jan:Dec!B2` / `'Q 1:Q 4'!A1:B2`, `,` arguments, arrays `{1,2;3,4}` (`,`
columns, `;` rows), intersection by whitespace between references
(`A1:B2 B1:C3`), union by `,` directly inside parentheses (`(A1,B1)`),
`Sheet1!#REF!`, unquoted dotted sheet names on input (`Sales.EU!A1`, printed
quoted), `TRUE`/`FALSE` literals, the seven Excel error literals. The
printer never writes `$` before a sheet name, quotes names that look like
A1 or R1C1 references, wraps unions in parentheses and prints OOo-only error
literals as `#VALUE!`. R1C1 and structured references are not supported.

### Dates

Only cells whose number format shows a calendar date (day, month, year,
weekday; `tally_io_xlsx::is_date_format`) are treated as date serials. Plain
numbers, times of day and durations (`[h]:mm`) are never changed.

Excel's 1900 date system counts 1900-01-01 as 1 and has a fictitious
1900-02-29 (serial 60). Tally's default epoch D1899 (serial 0 =
1899-12-30) agrees with Excel from 1900-03-01 (serial 61) on.

| Date | Excel 1900 | Tally D1899 | Tally D1900 | Excel 1904 = Tally D1904 |
|---|---|---|---|---|
| 1899-12-30 | – | 0 | −2 | −1462 |
| 1899-12-31 | (0, "1900-01-00") | 1 | −1 | −1461 |
| 1900-01-01 | 1 | 2 | 0 | −1460 |
| 1900-02-28 | 59 | 60 | 58 | −1403 |
| (1900-02-29) | 60 | – | – | – |
| 1900-03-01 | 61 | 61 | 59 | −1402 |
| 1904-01-01 | 1462 | 1462 | 1460 | 0 |
| 2026-10-19 | 46314 | 46314 | 46312 | 44852 |

- **Import, 1900 system:** the workbook epoch is D1899. Date serials
  0 ≤ s < 60 gain one day, so the calendar date Excel shows is kept (Excel's
  day 0, "1900-01-00", becomes 1899-12-31 and exports back to 0); the
  fictitious 60 (1900-02-29) becomes 1900-02-28 (D1899 60), keeping its time
  of day; everything else is unchanged.
- **Import, 1904 system:** the workbook epoch becomes D1904 and serials are
  kept unchanged (as LibreOffice does), so formulas mixing dates and plain
  numbers stay consistent. This is lossless; converting serials to D1899
  (+1462) would break such formulas.
- **Export from D1899:** 1 ≤ s < 61 loses one day; 1899-12-31 has no Excel
  date and is written as Excel 0. **From D1900:** +1 before 1900-03-01, +2
  from then on (the file is a normal 1900-system workbook and reads back as
  D1899 with the same calendar dates). **From D1904:** serials kept,
  `date1904="1"` set.
- Cached values of date-formatted formulas are mapped the same way.

### Units

- Column width: `<col width>` is in characters of the default font's
  maximum digit width (7 px for Calibri 11) *including* the 5 px padding:
  `px = trunc((256·width + trunc(128/7)) / 256 · 7)` (ECMA-376 §18.3.1.13),
  then 1/100 mm at 96 dpi (`px · 2540 / 96`). Excel's default column (8.43
  characters shown, stored as 9.140625) is 64 px = 1693 hmm
  (`tally_io_xlsx::DEFAULT_COL_WIDTH_HMM`). Only `customWidth` or hidden
  columns are recorded.
- Row height: points → hmm (`pt · 2540 / 72`). Only `customHeight` or hidden
  rows are recorded; Excel and LibreOffice write auto-fit heights for many
  rows, which the app re-fits itself.
- Colours: ARGB → `0xRRGGBBAA` with alpha `ff`. Theme colours use a fixed
  copy of the default Office palette (not `theme1.xml`), with tints applied
  in HLS; theme 1 without tint, `auto` and indexed 64/65 mean "automatic".
- Borders: thin 26 hmm, medium 53, thick 79, double 79, hair (dotted) 9,
  dotted 26, dashed variants 26/53; written back by width.

### Security

The reader applies the same limits as `tally-io-ods` (`Limits::default()`):
10,000 zip entries and a 16 MiB central directory checked from the EOCD
record before the zip index is built; 512 MiB per worksheet/shared-strings
part and 64 MiB per other part, enforced with `Read::take`; 2 GiB total;
200:1 compression ratio once an entry passes 10 MiB; `<!DOCTYPE>` rejected in
every part; element depth 256 and 256 attributes per element; 10 M
materialized cells and 1 GiB of materialized text (every shared-string use
counts); at most 10 M shared strings, 16 MiB per string, 10,000 sheets.
Entry names with `..`, absolute paths, drive letters or backslashes are
never read; external relationship targets are never followed; nothing is
extracted to disk. Out-of-grid cell references are skipped and repeated
positions collapse to one cell, so a cell element never materializes more
than one cell. The hardened zip/XML layer is a copy of the ODS one (TODO:
share it in a common crate). Tests: `crates/tally-io-xlsx/tests/malicious.rs`
(corpus in `tests/fixtures/xlsx/malicious/`, generated by
`tests/fixtures/xlsx/gen_fixtures.py`).

### Known gaps

- No images, charts, tables or pivot tables, in either direction. Hyperlinks
  on numeric or formula cells are not written (`rust_xlsxwriter` stores
  links as string cells). Custom paper sizes are written as the default
  paper. An ODF-only password hash (SHA-256 without the companion legacy
  hash) is dropped on XLSX export (reported).
- Named cell styles are flattened into cell formats; `applyX` flags are not
  consulted (a cell xf's ids are taken as the effective formatting).
- Rich text runs are flattened to plain text.
- Row styles (`<row s customFormat="1">`) are not imported.
- Theme colours ignore a custom `theme1.xml`.
- East-Asian built-in formats 27–36 and 50–58 import as `YYYY-MM-DD`.
- Structured references, R1C1 formulas, dynamic-array spill semantics and
  3D whole-column references are not translated (kept as text).
- Error *values* export as `=#N/A`-style formulas.
- A formula cell whose cached result is text that looks like a number is
  written as a numeric result (`rust_xlsxwriter` infers the result type).
- A D1900 workbook re-imports as D1899 (same dates, different epoch).
- Only one underline style (single) and solid fills are written.
