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
| XLS (Excel 97–2003, BIFF8) | `tally-io-xls` (calamine 0.38 for values and formulas, Tally's own BIFF8 formatting reader) | yes | no | read + write |
| SXC (StarOffice 6 / OpenOffice.org 1.x) | `tally-io-ods` (`sxc.rs`) | yes | no | read + write |
| HTML tables | `tally-io-html` | yes | yes | read + write |
| dBase III/IV (`.dbf`) | `tally-io-dbf` | yes | yes | read + write |
| Flat XML ODF spreadsheet (`.fods`) | `tally-io-ods` (`read_fods`) | yes | no | read + write |
| Excel 2003 XML (SpreadsheetML, `.xml`) | `tally-io-xmlss` | yes | no | read + write |
| SYLK (`.slk`) | `tally-io-sylk` | yes | no | read + write |
| DIF (`.dif`) | `tally-io-dif` | yes | no | read + write |
| PDF | `tally-io-pdf` (krilla 0.8.2) | no | yes (export, print, preview) | export |

AOO 4.1 imports `.xlsx` but cannot save it (see
`docs/calc-features-inventory.md`, "Calc MS Excel 2007 XML"). Tally writes it.
XLSB, XLSM macros, XLS export and strict-OOXML-only packages are not
supported; an `.xlsb` is reported as `XlsxError::Unsupported`. `.xlsm`,
`.xltx` and `.xltm` open through the XLSX reader (their macro-enabled and
template content types are accepted); see "Read-only formats and macros"
below. The wave-5
formats are described at the end of this page.

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
| Open read-only (`LoadReadonly`) | `settings.xml` `config:config-item-set ooo:configuration-settings` → `LoadReadonly` (boolean; only directly in that set) → `DocSettings::load_readonly`; the document opens read-only (AOO `SetReadOnlyUI`, its lock is still taken) | written as `<config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="LoadReadonly" config:type="boolean">true</…>` when set (AOO's other configuration settings are not written); round-trips through save, Save As and recovery | no | no |
| Page setup | `style:page-layout` (size → paper, orientation, margins, `style:print` grid/headers, `scale-to`, `loext:scale-to-X/Y`, centring, page order, first page number) + `style:master-page` header/footer regions (fields → `&[PAGE]` …) via the table style's master page, `style:display="false"` = header/footer off (texts kept), an absent `style:header` = none; `style:header-style`/`footer-style` `fo:min-height` (AutoFit height) or `svg:height` (fixed), `fo:margin-bottom`/`margin-top` (spacing), `fo:margin-left/right`; `table:print-ranges`; `table:table-header-rows/columns` | one page layout + master page per distinct setup; an area that is off is written as an empty `style:header-style` (AOO) | `pageMargins` (see below for `header`/`footer`), `pageSetup`, `printOptions`, `headerFooter` (`&L&C&R`, `&P &N &A &F &D &T`; empty = none), `_xlnm.Print_Area` / `_xlnm.Print_Titles` | yes (first page number patched in; a header/footer that is off is not written) |
| Manual page breaks | `fo:break-before="page"` on row/column styles | yes | `rowBreaks` / `colBreaks` (`man="1"`) | yes |
| Document properties | `meta.xml`: title, subject, keywords, description, initial creator, creator, dates | yes | `docProps/core.xml` | title, subject, author, keywords, comments |
| Locale | default cell style `fo:language`/`fo:country` | yes | – | – |
| Native numbers and calendars (G7) | `number:transliteration-format` / `-language` / `-country` / `-style` → `[NatNumN]` (+ `[$-LCID]` when the language differs from the style's); `number:calendar` on date elements → `[~calendar]` where it changes; a year after an era → `E` / `EE` | `[NatNumN]` / `[DBNumN]` (as its NatNum) as transliteration attributes; `[~calendar]` and, for `E EE R RR`, the language's other calendar as `number:calendar` on the date elements, `AAA` / `AAAA` on their weekday element (xmlnumfe.cxx; en-US / en-GB write `jewish`, their other calendar in en_US.xml) | `[DBNumN]` kept; `[$-NNCCLLLL]` numeral / calendar bytes → `[NatNumN][$-LLLL][~calendar]` (LibreOffice's reading); `[$-ja-JP-x-gannen]` kept | `[DBNumN]` always kept (also without a language tag); NatNum → `[DBNumN]` (zh / ja / ko) or a numeral byte, `[~calendar]` → calendar byte; `G`/`E`/`R` → `g`/`e` in East-Asian sections. Details in docs/numfmt-natnum.md |
| AutoFilter (value lists, one condition per column, Top 10 / top and bottom N and N %) | `table:database-range` with `table:display-filter-buttons`; `table:filter-condition` operators `=` `!=` `<` `>` `<=` `>=` `contains` `begins` `ends` `empty` `!empty` and `top values`, `bottom values`, `top percent`, `bottom percent` (AOO's "Top 10" is `top values` with 10); `table:filter-set-item` value lists; rows with `table:visibility="filter"` | yes | `autoFilter ref` with `filterColumn`: `filters` (+ `blank="1"`), one `customFilter` (`*x*` / `x*` / `*x` → contains / begins / ends, `notEqual " "` → not empty, `~` escapes); a column with two custom conditions opens unfiltered with a warning; `top10 top percent val`; `_xHHHH_` escapes decoded; value lists capped (docs/security.md 3b); rows Excel hid inside the area become the filter's rows; `_xlnm._FilterDatabase` is not shown as a name. An area reaching the sheet's last row or column (whole-column, whole-row or whole-sheet ref, any format) is cut down in that direction to the sheet's cells at load; any other area is kept and saved back exactly as written | patched in (`autofilter.rs`; rust_xlsxwriter has no `top10`) with the hidden `_xlnm._FilterDatabase` name (`localSheetId`); filtered rows are written hidden; tab/LF/CR in values as `&#9;` `&#10;` `&#13;`, other control characters as `_xHHHH_`. A column with both a value list and a condition keeps the list (reported) |

**Header and footer areas** follow AOO: the header area sits inside the
top margin and pushes the body down; its height (`margins.header`, ODS
`fo:min-height` as AOO writes it) includes the spacing towards the body,
and with AutoFit height (`HfAreaData::autofit`) it grows to its lines
(4.94 mm each, the most of the left/centre/right regions) plus the
spacing. The footer mirrors it at the bottom. AOO's dialog shows "Height"
without the spacing, and so does Tally's. Excel instead measures its
`header` margin from the page edge to the header and its `top` margin to
the body, independently: XLSX import sets Tally's top margin to Excel's
header margin and the header area (spacing 0.25 cm, at most half; fixed
height, since Excel's gap does not grow) to the gap up to Excel's top
margin, so the sheet prints like Excel. The imported Excel margins are
kept (`PageSetupData::excel_margins`) and written back unchanged while the
margins and areas are still the ones derived from them, so an untouched
file round-trips exactly (also when the header margin is at or below the
top margin, which Tally cannot lay out). Otherwise export writes the
reverse (`header` = top margin, `top` = top margin + printed header area,
an empty header that is switched on included, so Excel's body sits where
Tally prints it). Without a header Excel's top margin is kept and its
header margin written as 0.3" (or the top margin if smaller). Footers map
the same way. In ODS, `text:line-break` in a region is a new line.

Lines that do not fit an area of fixed height are left off (never drawn
over the body or off the page), and areas taller than the page leaves
are cut down, header first, so the body keeps at least 10 mm; the page
setup dialog refuses such values. "Same content left/right" (different left and
right pages) is not modelled; `style:header-left` is ignored.

Formulas of rules and validations are OpenFormula in the model, relative to
`base` (ODS `base-cell-address`, XLSX the top-left cell of the first range).

**CSV export** writes the active sheet only (the UI says so, as AOO's
"Only the active sheet could be saved" warning) with AOO's Text Export
options (`CsvExportOptions`, `DocumentService::set_csv_options`; the
document keeps them for the next Save): field delimiter (any one ASCII
character), text delimiter (`"`, `'`, another, or none: fields are then
never quoted), "Quote all text cells" (numbers stay bare), "Save cell
content as shown" (off: numbers at full precision and dates in the input
line format, AOO's `GetInputString`) and the character set: UTF-8 (no
BOM), UTF-16 (little-endian with a BOM), Windows-1252 or ISO-8859-1
(unmappable characters become `?`). Defaults: comma, `"` where needed, as
shown, UTF-8, CRLF. The formula-injection guard (`'` prefix) applies to
every option set. An opened CSV keeps the delimiter (tab for `.tsv`) and
character set found on import, so Save writes it back the same way (AOO
reuses the import's filter options); `.csv`, `.tsv`, `.txt` and `.tab`
files save as CSV (after the "Keep current format" prompt), and a file
Tally only reads (`.xls`, `.sxc`) goes through Save As. Saving warns when
cells hold the field delimiter or a line break with no text delimiter,
and counts the characters the character set cannot hold.

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
| `xl/worksheets/sheetN.xml` | `sheetPr/tabColor`, `sheetFormatPr` (`defaultRowHeight`, `zeroHeight`, `defaultColWidth`, `baseColWidth`: the size of every row and column without one of its own, see "Units"), `pane` (frozen → rows / columns; Window ▸ Split, `state="split"` or absent → `xSplit`/`ySplit` in 1/20 pt to 1/100 mm), `cols` (width, hidden, column style → whole-column style run), `row` (height, hidden), `c` (see below), `mergeCells` |
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
- Window ▸ Split: `<pane xSplit ySplit activePane state="split"/>` first in
  `sheetView` (ECMA-376 §18.3.1.66: positions in 1/20 pt, written to one
  decimal so the 1/100 mm value reads back unchanged; `activePane`
  `bottomRight` / `topRight` / `bottomLeft`; no `topLeftCell`). A frozen
  pane wins when a sheet has both, as in the ODS writer. Unverified: whether
  Excel places the line at the same spot (Excel may count the position from
  the window edge, including the row/column headers).
- Only the active sheet gets `sheetView tabSelected="1"`; a multi-sheet
  selection (`Transaction.group`) is not saved in any format (parity
  deviation U24).
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
- Styles: font (bold, italic, underline: single, double, single and
  double accounting; other underline styles as single; strike, family,
  size, colour), solid fill (chip display styles are written as their background
  fill), borders, horizontal/vertical alignment, wrap, indent, number formats
  via `tally_numfmt::excel::to_excel_code` (untranslatable codes fall back
  to General and are reported).
- Column widths and hidden columns, row heights and hidden rows (whole
  pixels at 96 dpi; at most 100,000 individually sized rows per sheet),
  whole-column style runs as column formats, shorter style runs (≤ 4,096
  rows) as styled blanks.
- The sheet's default sizes (`sheetFormatPr`), so that the file says what
  Tally shows and the rows and columns of the default size are not written
  one by one (`write.rs` `SheetDefaults`). The default is what most rows
  are:
  - Most rows have no run (a sheet made in Tally; such rows are 24 px
    and fitted to their content): `defaultRowHeight="18" customHeight="1"`,
    and the `<row>` elements of those rows carry no height, so that Excel
    fits them to their content as Tally does. A row set to exactly 24 px
    is not kept apart from them: it comes back as an automatic row.
  - Most rows have one height (a sheet read from an Excel file, also
    after rows were inserted or deleted, which leaves a few rows without
    a run): that height is the default. The rows without a run are
    written as rows of 24 px (`ht="18" customHeight="1"`): under a
    default that is a set height the file has no way to say "automatic",
    so they come back with that height set, like the other rows of the
    sheet.
  - Most rows are hidden (Excel's "hide the unused rows"):
    `zeroHeight="1"` with the height of those rows; the hidden rows that
    have cells, formatted blanks or an outline level are written hidden
    one by one (a `<row>` element is a shown row unless it says
    otherwise).

  Every other row is written with its height, at most 100,000 per sheet
  (the rest keep the default, with a warning). That cap now also counts
  the rows without a run under a set default and the hidden rows with
  content under a hidden default; it is reached by a sheet with more than
  100,000 rows that differ from what most rows are (for example 150,000
  rows set to 30 px in a sheet made in Tally).
  Columns work the same way, without a hidden default: the width most
  columns have, else Tally's 100 px, written as `defaultColWidth` in
  digits of the default font (14.28515625 for Calibri) when it is not the
  64 px (72, 80 … for other default fonts) that `rust_xlsxwriter` implies.
  A column without a run under another default is written at 100 px and
  comes back without one.
- What `rust_xlsxwriter` cannot write is edited into the finished sheet
  XML (`write.rs` `default_size_patches`, `write_v03.rs` `apply_patches`):
  `defaultColWidth`; the height it repeats on every `<row>` once the
  default is not its own 20 px, taken out again for automatic rows; and
  the real width of a hidden column that is as wide as its default
  (it writes `width="0"`). Nearly every sheet made in Tally needs the
  first two, so those workbooks are written by `rust_xlsxwriter` at its
  fastest compression level and compressed at the normal level by the
  edit pass, each part once: a 200,000-cell sheet saves in about 100 ms
  instead of 88 ms without edits (155 ms when both passes compressed
  fully). The whole package is in memory twice for the pass, the first
  copy lightly compressed (about 1.7 times the final size). Parts that
  are not edited in a package that needs no recompression are copied as
  they are; a part of 4 GiB or more is written with ZIP64 sizes.
  `rust_xlsxwriter` also uses its own 64 px for a column it was told
  nothing about: the columns it writes a `<col>` for (a whole-column
  format, an outline level) get the default width explicitly, and so do
  all columns of a sheet with a chart, picture or note, whose end cells
  it computes from the widths.
- Not checked in Excel: how Excel shows `defaultRowHeight="18"
  customHeight="1"` with `<row>` elements that have no height (by the
  specification, 24 px rows that it fits to their content), and
  `zeroHeight` written by Tally.
- Merges, defined names (workbook and sheet scope), `Print_Area` as the
  sheet's print area, document title.
- `date1904="1"` for D1904 workbooks (`rust_xlsxwriter` has no option for
  it, so the finished package is re-zipped with `xl/workbook.xml` patched).

### Formula dialect (ExcelA1)

`tally_formula::Dialect::ExcelA1`: `Sheet1!A1`, `'My Sheet'!$A$1`, 3D
`Jan:Dec!B2` / `'Q 1:Q 4'!A1:B2`, `,` arguments, arrays `{1,2;3,4}` (`,`
columns, `;` rows), intersection by whitespace between references
(`A1:B2 B1:C3`; a name before ` (` is an intersection too, `MyRange (A1:B2)`,
never a call — Excel rejects `SUM (A1)`; the printer brackets a word or
constant beside the space, so the first export of such a formula is
`(MyRange) (A1:B2)`), union by `,` directly inside parentheses (`(A1,B1)`),
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
  (`tally_io_xlsx::DEFAULT_COL_WIDTH_HMM`).
- Row height: points → hmm (`pt · 2540 / 72`).
- Which rows and columns have a size: all of them, as OpenOffice lays an
  Excel file out and as the `.xls` reader does (`sheet.rs` `finish_rows`,
  `finish_cols`). A row has the height its `<row ht>` stores, whether the
  user set it (`customHeight`) or Excel fitted it; a row without `ht` or
  without a `<row>` has `sheetFormatPr/@defaultRowHeight` (15 pt = 20 px
  when absent) and, without a `<row>`, is hidden when the default is
  (`zeroHeight`, or a default of 0). `ht="0"` is a hidden row of the
  default height. A column has its `<col width>`, with or without
  `customWidth` (of overlapping entries the last counts; width 0 is a
  hidden column of the default width); every other column has
  `defaultColWidth`, or else Excel's derivation from `baseColWidth` (8
  when absent): that many digits of the default font plus 5 px, rounded up
  to a multiple of 8 px (Calibri 11: 61 → 64 px). OpenOffice adds the 5 px
  without the rounding (61 px); Excel's value is used because it is what
  the author of the file saw. Sizes that are negative, not finite,
  larger than 1,638 pt / 256 characters, or not 0 but so small that they
  round to nothing are ignored (only an explicit 0 hides). The sizes are runs
  over the whole sheet (equal neighbours merge; a million rows of one
  height are one run), never one entry per row, and count against
  `Limits::max_cells`. The model has no "automatic height of this size",
  so these rows have a set height: shown and printed at the file's
  height, not growing when their content changes (Format ▸ Row ▸ Optimal
  Height fits them).
- The exception is Tally's own default, so that a workbook made in Tally
  reads back as it was: in a sheet whose default row height is 18 pt
  (24 px), the shown rows of that height get no entry (an automatic row),
  and a shown column of exactly 100 px never gets one
  (`tally_model::APP_DEFAULT_ROW_HEIGHT_HMM`, `APP_DEFAULT_COL_WIDTH_HMM`).
  An `.xlsx` saved by Tally 0.1.2 or older says 15 pt and 64 px for such
  rows and columns (it cannot be told from an Excel file) and opens with
  those.
- Colours: ARGB → `0xRRGGBBAA` with alpha `ff`. Theme colours come from
  the workbook's `xl/theme/theme1.xml` (`colors.rs` `parse_theme`; the
  default Office palette when the part is missing or incomplete), with
  tints applied in HLS; theme 1 without tint, `auto` and indexed 64/65 mean
  "automatic".
- Borders: thin 26 hmm, medium 53, thick 79, double 79, hair (dotted) 9,
  dotted 26, dashed variants 26/53; written back by width.

### Security

The reader applies the same limits as `tally-io-ods` (`Limits::default()`):
10,000 zip entries and a 16 MiB central directory checked from the EOCD
record before the zip index is built; 2 GiB per worksheet/shared-strings
part (like ODS `content.xml`) and 64 MiB per other part, enforced with
`Read::take`; 4 GiB total; 200:1 compression ratio once the package has
inflated 10 MiB; 64 MiB of re-reads; two sheets may not share a worksheet
part; `<!DOCTYPE>` rejected in every part; element depth 256, 256
attributes per element and 64 MiB per XML event (tag or text run); 256 MiB
of attribute values on non-cell elements per package, 1 KiB sheet names,
256-byte relationship ids; 10 M
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

- No tables (Excel structured tables), in either direction; pivot tables and
  named styles are best effort (see "Wave 5 data tools"). Hyperlinks
  on numeric or formula cells are not written (`rust_xlsxwriter` stores
  links as string cells). Custom paper sizes are written as the default
  paper. An ODF-only password hash (SHA-256 without the companion legacy
  hash) is dropped on XLSX export (reported).
- `applyX` flags are not consulted (a cell xf's ids are taken as the
  effective formatting).
- Rich text runs are flattened to plain text.
- Row styles (`<row s customFormat="1">`) are not imported.
- East-Asian built-in formats 27–36 and 50–58 import as `YYYY-MM-DD`.
- Structured references, R1C1 formulas, dynamic-array spill semantics and
  3D whole-column references are not translated (kept as text).
- Error *values* export as `=#N/A`-style formulas.
- A formula cell whose cached result is text that looks like a number is
  written as a numeric result (`rust_xlsxwriter` infers the result type).
- A D1900 workbook re-imports as D1899 (same dates, different epoch).
- Dotted, dashed, wave and bold underlines are written as single
  underlines (single, double and the accounting ones are kept); only solid
  fills are written.

## Wave 5 data tools (W5-2)

What the DataPilot, outline, scenario, database-range, named-style and
format-extras state looks like in each format, and what each direction
loses.

### ODS

- **Column default cell styles** (`table:table-column
  table:default-cell-style-name`): applied as AOO applies them, to the cells
  the rows contain that name no style of their own (and whose row has no
  `table:default-cell-style-name`), merged into vertical runs; a cell naming
  `Default` opts out. A sheet whose rows stop at its content has no
  formatting below it. On export, a style covering a whole column is
  written as the column's default style with rows to the last one, as AOO
  writes them. Known gap: once such a column has one cell with a different
  style, it is no longer a single whole-column run, and the next export
  writes the rest of the column as repeated styled empty cells instead of a
  column default (same formatting, larger `content.xml`).
- **DataPilot**: `table:data-pilot-tables` / `table:data-pilot-table` with
  `table:source-cell-range` or `table:database-source-table`
  (`table:table-name` = a named database range), `table:target-range-address`
  (the last written output), `table:grand-total`, `table:ignore-empty-rows`,
  `table:identify-categories`, `table:show-filter-button`,
  `table:drill-down-on-double-click`; one `table:data-pilot-field` per
  row/column/page/data field with `table:orientation`, `table:function`,
  `table:selected-page`, `table:data-pilot-level` (`table:show-empty`,
  `data-pilot-subtotals` with `auto`/named functions, `data-pilot-members`
  with `table:display` for hidden items and the manual order,
  `data-pilot-display-info`, `data-pilot-sort-info`, `data-pilot-layout-info`
  with `table:layout-mode` = `tabular-layout` / `outline-subtotals-top` /
  `outline-subtotals-bottom` and `table:add-empty-lines`, written for every
  field and read as AOO does, `xmldpimp.cxx` `ScXMLDataPilotLayoutInfoContext`),
  `table:data-pilot-field-reference` (show-as) and `table:data-pilot-groups`
  (numeric and date groups; extra date parts go into
  `table:application-data`). The "Data" pseudo-field is written as
  `table:is-data-layout-field`. Data-field captions are kept in
  `table:application-data`. Field names are resolved against the source's
  header row on read. The output cells are plain values in the target sheet;
  Tally recomputes on `RefreshPivot`, never on load (as AOO).
  The source filter set with the "Filter" button (T17) is a `table:filter`
  inside `table:source-cell-range`, as AOO writes it
  (`ScXMLExportDataPilot::WriteDPFilter`): `table:field-number` is the sheet
  column (not the column inside the source; AOO #i115431),
  `table:condition-source-range-address` is the source range,
  `table:display-duplicates="false"` for No duplications, `=` / `!=` become
  `match` / `nomatch` with Regular expressions, `table:case-sensitive` on
  every condition, AND runs inside `table:filter-or`. On read a field number
  outside the source but below its width is taken as a source column, `empty`
  / `noempty` read as `=` / `<>` "" and at most 8 conditions are kept. A
  pivot over a named database range (`table:database-source-table`) has no
  place for the filter in ODF (AOO 4.1 writes `WriteDPFilter` only inside
  `table:source-cell-range`, XMLExportDataPilot.cxx); it is not written
  and the save reports a warning (`tools::pivot_warnings`).
- **Outline**: `table:table-row-group` / `table:table-column-group`
  (nested, `table:display="false"` for collapsed). The rows/columns of a
  collapsed group are written `table:visibility="collapse"` as AOO does;
  on read they are unhidden again and hidden by the group instead (a third
  hidden source next to user-hidden and filtered rows).
- **Scenarios**: each scenario is a hidden sheet right after its base sheet
  with `table:scenario` (`table:scenario-ranges`, `table:is-active`,
  `table:display-border`, `table:border-color`, `table:copy-back`,
  `table:copy-styles`, `table:copy-formulas`, `table:protected`,
  `table:comment`); the reader folds it into `SheetData::scenarios` of the
  base sheet (values inside the scenario ranges). Tally stores no styles
  with a scenario (`copy_styles` round-trips as a flag only).
- **Database ranges**: `table:database-range` with `table:name`,
  `table:contains-header`, `table:display-filter-buttons`, a `table:filter`
  (the 16 standard operators as `=`, `!=`, `<`, `>`, `<=`, `>=`,
  `top values`, `bottom values`, `top percent`, `bottom percent`, `contains`,
  `does-not-contain`, `begins-with`, `does-not-begin-with`, `ends-with`,
  `does-not-end-with`; AND runs inside `table:filter-or`; `table:case-sensitive`
  on the conditions, `table:display-duplicates="false"` for "no duplicates",
  `table:condition-source-range-address` for an advanced filter,
  `table:target-range-address` for "copy results to") and
  `table:subtotal-rules` (`table:sort-groups`, `table:subtotal-rule` with
  `table:subtotal-field`). A sheet-level standard filter or subtotal
  definition without a named range is written on the anonymous
  `__Anonymous_Sheet_DB__N` range when the sheet has no AutoFilter.
- **Named cell styles**: `office:styles` `style:style
  style:family="table-cell"` with `style:parent-style-name` (and
  `style:display-name`; names are escaped `_20_`-style). Cell automatic
  styles name their base style as parent. Tally keeps the attributes
  resolved in memory and writes a style as its difference from its parent
  (as AOO writes a style's own items), with the off value where the parent
  sets something the style does not have (`fo:font-weight="normal"`,
  `fo:background-color="transparent"`, `fo:border-top="none"`,
  `style:text-align-source="value-type"`, `style:use-window-font-color`,
  a General data style), so a parent redefined in another application
  changes the children there and in Tally. A child that sets the same
  value as its parent is saved as inheriting it. The number format of a named style is written
  as a common data style `NS<i>` in office:styles (since 2026-11-09, for
  Load Styles; before, only the cells carried it) and read back.
- **Format extras**: `style:rotation-angle`, `style:direction="ttb"`
  (stacked), `style:shrink-to-fit`, `style:diagonal-tl-br` /
  `style:diagonal-bl-tr`, `style:text-underline-style` / `-type` / `-width`
  (double, dotted, dashed, wave, bold) and `fo:margin-left` (indent in
  10 pt steps).
- **View options**: `ShowFormulas`, `ShowZeroValues`, `ShowGrid`,
  `HasColumnRowHeaders`, `IsValueHighlightingEnabled` in every sheet's
  `settings.xml` table entry (Tally's per-sheet values). The view entry
  itself carries what AOO 4.1 writes and reads there
  (`ScViewData::WriteUserDataSequence` / `ReadUserDataSequence`, AOO
  ignores the per-table copies): `ShowZeroValues`, `ShowGrid` and
  `HasColumnRowHeaders` of the active sheet, and the document-wide
  `ShowNotes` (comment indicator), `GridColor` (`long`, RGB; AOO's
  `SC_STD_GRIDCOLOR` 12632256 = automatic, also written for it; COL_AUTO
  reads as automatic), `ShowPageBreaks` (kept for the round trip),
  `HasSheetTabs` and `IsOutlineSymbolsSet` (2026-11-09, U18). Reading takes
  the view-level items (or, without them, the same names from
  `ooo:configuration-settings`) as every sheet's defaults, then the sheet's
  own. Not in the file, as in AOO 4.1: scroll bars, guides, handles,
  anchor, text overflow, object Show/Hide (`ShowObjects` / `ShowCharts` /
  `ShowDrawing` are not written by AOO 4.1 and not read) — app prefs.
- **Window ▸ Split**: `HorizontalSplitMode`/`VerticalSplitMode` = 1 with
  `HorizontalSplitPosition`/`VerticalSplitPosition` in pixels at 100 %
  (96 dpi), converted to and from 1/100 mm. A frozen pane (mode 2, positions
  in cells) wins when a sheet has both.
- **Chart frames**: `draw:object` carries `draw:notify-on-update-of-ranges`
  with the plotted category and value ranges (`Sheet1.A2:Sheet1.A6 …`, at
  most 64 areas), as AOO writes it.
- **Charts with their own data** (T18, AOO's internal data provider: a chart
  that reads no cells): written as AOO's SchXMLExport does, `chart:chart
  xlink:href="."`, the plot area `table:cell-range-address=
  "local-table.$A$1:.$D$5"`, each series' `chart:values-cell-range-address`,
  `chart:label-cell-address` and `chart:domain` pointing into the object's
  `local-table` (`InternalDataProvider::convertRangeToXML`: column A the
  categories, then per series its X column, value column and bubble-size
  column; row 1 the names), the x axis `chart:categories` likewise. On read
  an address whose sheet is `local-table` (and no sheet of the document has
  that name) takes its cells from the local table: values, X values, bubble
  sizes (a bubble series' values are its sizes, domain 1 the Y values,
  domain 2 the X values), the series name and the categories (numbers stay
  numbers, e.g. dates). Reads stop at the table's last row and count against
  the document's literal-point budget. Files without addresses keep the
  older rule (series *i* reads column *i* + 1).
- **Names** (review 3, H3/H4): `table:base-cell-address` is read into
  `NameData::base_cell` and relative references are parsed against it, so
  "the cell to the left" names from AOO/LibreOffice evaluate where they
  should; the base is written back. A name that is legal in AOO's
  1,024-column grid but reads as a cell reference in Tally's 16,384 columns
  (`Rel2`, `Tax1`) is renamed with a trailing `_` (`Rel2_`), the file's
  formulas still resolve the original spelling, and `WorkbookInfo.warnings`
  lists the renames; invalid names are dropped with a warning.
- **Name usage flags** (T20): `table:range-usable-as` on `table:named-range`
  (whitespace-separated `print-range`, `filter`, `repeat-row`,
  `repeat-column`; unknown tokens such as `none` are ignored, as in AOO
  `ScXMLImport::GetRangeType`) is read into `NameData::usage` and written
  back in AOO's token order (repeat-column, repeat-row, filter, print-range;
  `ScXMLExport::WriteNamedExpressions`). Like AOO, only names that are cell
  ranges carry it: `table:named-expression` has no such attribute. XLSX has
  no equivalent; the flags are not written to `definedName` (built-in
  `_xlnm.Print_Area` / `_xlnm.Print_Titles` map to the sheet's print range
  and repeats, not to flags).

### XLSX

- **Outline**: `outlineLevel` / `collapsed` / `hidden` on rows and columns
  (`sheetFormatPr outlineLevelRow/Col`, `outlinePr`); on read a group at
  level *k* is every maximal run of rows/columns with level ≥ *k*, collapsed
  when the run is hidden or the next row carries `collapsed`. Adjacent
  groups of the same level cannot be written (rust_xlsxwriter refuses them;
  reported and skipped).
- **Charts with their own data** (T18): a series without a cell range
  writes its literal values as `c:numLit` (and X values in `c:xVal`,
  categories in `c:cat` as `c:strLit` / `c:numLit`; series names as
  `c:tx/c:v`). rust_xlsxwriter only writes references, so the series gets a
  placeholder reference (`_TallyLit[n]`) that the package patch pass
  replaces in `xl/charts/chartN.xml` (one pass per part). Values are always
  `c:numLit` (`CT_NumDataSource`; text values are parsed, non-numbers left
  out). Literal text escapes control characters as `_xHHHH_` (and a literal
  `_xHHHH_` as `_x005F_xHHHH_`), as Excel and rust_xlsxwriter do; reading
  decodes them. A save writes at most 32 MiB of literal lists (counted per
  series reference); past that, later categories and X values are left out
  and values written empty, with a warning per chart. Bubble sizes are
  still dropped with the bubble → scatter degradation. Reading takes
  `numLit` / `strLit` into the same literal data.
- **Scenarios**: `<scenarios current show><scenario name comment locked
  count><inputCells r val/>` (at most 32 cells per scenario, values only; a
  read scenario has one range per cell).
- **Pivot tables**: written as a post-pass patch of the package:
  `xl/pivotCache/pivotCacheDefinitionN.xml` (`saveData="0"
  refreshOnLoad="1"`, `cacheFields` with `sharedItems` for the axis
  fields), `xl/pivotTables/pivotTableN.xml` (`location`, `pivotFields`,
  `rowFields`/`colFields`/`pageFields`, `dataFields` with the 11 subtotal
  functions, the Data pseudo-field as `x="-2"`), the workbook `pivotCaches`,
  the relationships and content types. Best effort: Excel rebuilds the
  table on load. Read: the definition only (fields, functions, layout,
  totals, location); show-as, grouping, sorting and hidden items are not
  translated. A DataPilot source filter (the "Filter" button) has no OOXML
  equivalent: it is dropped with a warning. Field layout (2026-11-13) on
  each axis `pivotField`: tabular is `compact="0" outline="0"`, outline
  with subtotals at the top `compact="0"` (`outline` and `subtotalTop`
  default to true), at the bottom `compact="0" subtotalTop="0"`; "Empty
  line after each item" is `insertBlankRow="1"`. Read as AOO's OOXML
  import does (`oox/source/xls/pivottablebuffer.cxx:352-354,785-791`):
  `outline` (default true) gives outline layout, top or bottom by
  `subtotalTop`, otherwise tabular; `insertBlankRow` the empty line.
  Excel's compact form (`compact`, default true) has no AOO equivalent: it
  is read as outline layout, and the open reports "the compact form is
  shown as outline layout" when an outer row field uses it. So an Excel
  pivot in its default compact form opens in outline layout, subtotals at
  the top, as in AOO. This includes XLSX files written by Tally 0.1.x,
  whose `pivotField`s carry no `outline` / `compact` attributes: they
  reopen as outline (top), with the compact warning when they have more than
  one row field, and the sheet keeps
  the old tabular output cells until the pivot is refreshed.
- **Named cell styles**: `cellStyles`/`cellStyleXfs` (the `Normal` style is
  `Default`; `xfId` gives a cell's base style). Tally writes the names with
  default-looking `cellStyleXfs` entries; the attributes stay on the cells.
- **Format extras**: `alignment textRotation` (0–90, 91–180, 255 =
  stacked), `shrinkToFit`, `indent`, `u val="double|singleAccounting|
  doubleAccounting"`, `border diagonalUp/diagonalDown` with `diagonal`.
- **View options**: `sheetView showGridLines/showZeros/showFormulas/
  showRowColHeaders`. Sheet tabs: `workbookView showSheetTabs="0"`; outline
  symbols: `sheetView showOutlineSymbols="0"` on every sheet (read: hidden
  when any sheet view hides them; document-wide in Tally as in AOO). The
  grid colour and comment indicator are not written to XLSX (2026-11-09).

## Document options in ODS (2026-11-09)

- **Calculation settings** (`table:calculation-settings` in content.xml)
  are written only when a setting differs from AOO's defaults
  (case-sensitive, regular expressions, whole-cell criteria, precision
  not as shown, no iterations, null date 1899-12-30, two-digit years from
  1930), as AOO's `ScXMLExport::WriteCalculationSettings` does
  (xmlexprt.cxx:3688-3713). Tally's own defaults (case-insensitive,
  wildcards) differ, so most Tally files carry the element. When the
  element is present AOO and Tally read a missing `table:null-year` as
  1930; when it is absent, both use the reader's "Year (two digits)"
  option. A Tally file without the element is read with AOO's defaults
  (case-sensitive, regular expressions); other files without it keep
  Tally's defaults (deviation F3). `table:automatic-find-labels="false"`
  is written whenever the element is (Tally has no label lookup).
- **General decimals** ("Limit decimals for general number format") are
  `style:decimal-places` on the default cell style in styles.xml (AOO's
  `StandardDecimals` property, xmlstyle.cxx:109; there is no
  `table:decimal-places`). Written only when limited; absent means
  unlimited (AOO docsh3.cxx:439-445, #i112123). Old OpenOffice.org files
  (before the "unlimited" default, OOo 3.2 and earlier) carry
  `style:decimal-places="2"` on the default style, because the option
  then defaulted to 2 and was always exported. AOO 4.1 applies the value
  on load like any other (no version check in xmlimprt.cxx, xmlstyli.cxx
  or defltuno.cxx), so such files show General numbers with at most 2
  decimals in AOO 4.1 and in Tally alike; Tools ▸ Options ▸ Calculate
  turns the limit off. The OOo 3.x default itself is not verifiable from
  the AOO41X tree (its history starts later).
- **View zoom**: settings.xml `ZoomType` / `ZoomValue` /
  `PageViewZoomValue` per sheet and for the view; read clamped to 20-400 %.

## Wave 5 formats (W5-3)

Formats are sniffed by content before the extension (`codec::detect` in
`service/codec.rs`): a zip whose `mimetype` is the ODS or SXC type or whose
content types name a spreadsheet main part (XLSX family), the OLE2
signature for XLS, a flat ODF `office:document` root with the ODS media
type, a SpreadsheetML root or `mso-application` instruction, the SYLK `ID;`
and DIF `TABLE`/`0,1` headers (not trusted on `.csv`/`.tsv`/`.txt`/`.tab`
files: `ID;Price;…` is a semicolon CSV), then by extension: the dBase header
(only with a `.dbf` extension or no extension), HTML tags for
`.html`/`.htm`/`.xhtml` (and extension-less or `.txt` files that start with
a document), else CSV. A renamed file therefore opens by its content.

### Read-only formats and macros

Save writes a file in place only in a format Tally writes (`.ods`, `.ots`,
`.xlsx`, the CSV family, `.html`/`.htm`, `.dbf`;
`dialogs::writable_in_place`). Every other format (`.xls`, `.xlt`,
`.xlw`, `.xlsm`, `.xltm`, `.xltx`, `.xml`, `.fods`, `.sxc`, `.stc`, `.slk`,
`.dif`) goes to Save As, opened in the file's folder with the same base
name and XLSX preselected for the Excel family, ODS for the rest. Tally
never writes a plain workbook into an `.xlsm` name (Excel refuses one
without its VBA project and content type).

Macros are never read or run. When the opened file holds them — a VBA
project (`xl/vbaProject.bin`) or Excel 4.0 macro sheets in an OOXML package
(`tally_io_xlsx::has_macros`), a `_VBA_PROJECT_CUR` storage in an `.xls`
(`tally_io_xls::has_macros`), Basic modules or scripts in an ODS/SXC
package (`Basic/<lib>/<module>.xml`, `Scripts/`; `tally_io_ods::has_macros`)
or `ooo:module` in a flat ODS (`fods_has_macros`) — the open result has
`WorkbookInfo::has_macros` set once, and the UI shows the localised notice
"This document contains macros. Tally does not run macros; they will be
removed when you save." (`msg.toast.macrosRemoved`).

The desktop Open dialog lists every readable extension ("All
Spreadsheets", one filter per format, "All Files"). On macOS no filter is
set (the dialog plugin merges filters into one allowed list), so any file
can be chosen and is sniffed, as AOO's default "All files" filter. The app
bundle declares these formats (`bundle.fileAssociations` →
`CFBundleDocumentTypes`, Editor for formats Tally writes, Viewer for the
read-only ones), and files opened from Finder (`RunEvent::Opened`) or, on
Windows and Linux, from the command line go through the same open path
(allowlist, lock, already-open check) in a new window.

### Flat XML ODF spreadsheet (`.fods`, `tally_io_ods::read_fods`, read only)

One XML file with an `office:document` root (`office:mimetype` the ODS
type) holding what a package splits into `meta.xml`, `settings.xml`,
`styles.xml` and `content.xml`. Read with the ODS parsers in two passes
over the bytes: meta and settings (stopping at `office:body`), then common
styles, automatic styles, master pages and the body
(`content::parse_content_with(.., flat = true)`). Everything ODS reads is
read, except charts and pictures (embedded `office:document` objects and
`office:binary-data` images), which are dropped with a warning. Not
written (Save As offers ODS).


### XLS, Excel 97–2003 (`crates/tally-io-xls`, read only)

Two passes over the workbook stream that the pre-scan
(`src/prescan.rs`, docs/security.md) extracted and validated.

**Values and formulas** come from calamine 0.38 (MIT): sheet names, order
and visibility; cell values (numbers, text, booleans, errors); formulas
with their cached results, converted from calamine's ExcelA1 text to
OpenFormula through `tally_io_xlsx::excel_formula_to_of` (a formula
calamine cannot decode keeps its cached value and is counted in a
warning); merged areas; the 1904 date system. calamine's
`Range::used_cells()` yields positions relative to the range's first used
row and column; the reader adds `Range::start()` (a sheet whose row 1 or
column A is empty used to load shifted).

**Formatting** comes from Tally's own BIFF8 reader (`src/format.rs`),
which reuses the pre-scan's record framing (substreams, `CONTINUE`). The
sizes of columns and rows follow the OpenOffice 4.1 import filter
(`XclImpColRowSettings`, `XclTools::GetScColumnWidth`), so a sheet is laid
out as OpenOffice lays it out; the rest maps onto the model the way the
XLSX reader maps the equivalent constructs, with its helpers (`pt_to_hmm`,
`border_line_style`, `INDEXED_PALETTE`, `excel_to_ooo`,
`builtin_format_code`, `is_date_format`, `excel_serial_to_tally`,
`PageParts`, `header_footer_from_excel`, `paper_from_excel_code`):

| BIFF8 records | Model | Notes |
|---|---|---|
| `COLINFO`, `DEFCOLWIDTH`, `STANDARDWIDTH` | `col_widths` (width, hidden), for every column of the sheet | Every `COLINFO` width is applied (writers other than Excel do not set its "user set" flag). OpenOffice's conversion: a width is in 1/256 of the width of a character of font 0, and that character is 11/20 of the font's height wide (`XclRoot::SetCharWidth`; OpenOffice measures the digit 0 on its printer and uses this value when it has none, Tally always, so widths do not depend on the machine): 10 characters of a 10 pt font are 1100 twips (73.3 px), where Excel shows 75 px. The result is rounded to twips. A range whose last column is ≥ 255 runs to the last column of the sheet. Width 0 is a hidden column of the default width. The columns without a `COLINFO` have the sheet's default: `STANDARDWIDTH` (1/256 character) if present, else `DEFCOLWIDTH` (characters) plus Excel's extra room `40960 / max(font height − 15, 60) + 50` (1/256 character; `XclTools::GetXclDefColWidthCorrection`), else OpenOffice's standard width of 1285 twips |
| `ROW`, `DEFAULTROWHEIGHT` | `row_heights` (height, hidden), for every row of the sheet | As OpenOffice (`XclImpColRowSettings::Convert`): every `ROW` record gives its row the height it stores (twips), whether it was set by hand ("unsynced") or fitted by Excel; a `ROW` with the default-height bit, and every row without a record, has the `DEFAULTROWHEIGHT` (OpenOffice's standard 256 twips in a file without one). Hidden: the row's hidden flag, a stored height of 0, or `DEFAULTROWHEIGHT` bit 1 (or a default height of 0) for the rows that take the default. The rows are kept as runs (equal neighbours merge; the rows after the last record are one run to the end of the sheet), never row by row. The model has no "automatic height of this size", so every row is a row with a set height: it is shown and printed at the file's height and does not grow when its content changes (Format ▸ Row ▸ Optimal Height fits it). Saved as ODS the runs are repeated rows; saved as XLSX the run that reaches the end of the sheet becomes `sheetFormatPr/@defaultRowHeight` and only the rows that differ are written |
| `FONT`, `FORMAT`, `XF`, `PALETTE` | `styles` (one per distinct look) | Font name, size, bold (weight ≥ 550: semibold and heavier, `XclFontData::GetScWeight`), italic, underline (single, double, accounting), strikeout, colour; fill; the four borders and both diagonals with style and colour; horizontal and vertical alignment, wrap, indent, rotation (0–90, 91–180 → −1…−90, 255 = stacked), shrink to fit; number format (`FORMAT` code or built-in id, translated like XLSX `numFmts`; where OpenOffice's built-in table differs from ECMA-376's, the `.xls` reader follows OpenOffice: ids 32–35 are `h:mm:ss`, and 59–62, 67–81 stand for 1–4, 9, 10, 12–17, 20–22, 45–47); locked and hidden-formula protection. Font index 4 is not in the file (the fifth `FONT` record is font 5): it is font 0 in bold, as in OpenOffice. Superscript and subscript are not read (the model's font has neither). A cell XF's own fields are the effective look: the "used attribute" flags and the parent style XF are not consulted (as for XLSX `cellXfs`, and with the same result as OpenOffice's `XclImpXF`). `STYLE` records (named styles) are not read. Equal looks share one style; the number format codes and font names the styles own are bounded by `Limits::max_style_text_bytes` (16 MiB; past it a format is read without them, with a warning). The family of font 0 becomes the document's default font; a font size is kept unless it is 11 pt, the size cells without one are drawn at |
| Colour indices | `0xRRGGBBAA` | 0–7 fixed, 8–63 the default BIFF8 palette or the file's `PALETTE`, 0x40 / 0x41 the system window text / background (automatic for text and lines), 0x7FFF automatic font colour |
| Fill patterns | `fill` | Pattern 1 is the foreground colour. Patterns 2–18 become one solid colour: the mix of pattern and background colour OpenOffice uses (`XclTools::GetPatternColor`, e.g. 50 % grey for pattern 2); unknown patterns the foreground colour |
| XF index of `LABELSST`, `LABEL`, `RSTRING`, `NUMBER`, `RK`, `MULRK`, `FORMULA`, `BOOLERR` | `CellData::style` | |
| `BLANK`, `MULBLANK` | styled empty cells | When the format shows something besides the default font, and, with any format, inside a column that has a format of its own (the cell then keeps its look instead of taking the column's: a cell with the default format gets a style that is the default look). Formatted empty cells and column formats get the room the value cells leave under `Limits::max_cells`; past it they are left out with a warning. They never make a file exceed the limit |
| `ROW` default format (`fGhostDirty` and the XF in bits 16–27) | styled empty cells | As OpenOffice (`ImportExcel::Row34`, `SetRowDefXF`): the row's cells without a record take the row's format. The model has no row styles (a style run is one column), so the format becomes styled empty cells from column A to the last column the sheet uses (a cell record or a `COLINFO`); a default format that is the plain look only needs cells where a column format would show instead. They count against `Limits::max_styled_cells` (warning past it) |
| Cells with a value in a formatted column or row | `CellData::style` | A cell record's own XF is its look: a value with the default format in a formatted column gets the default look as a style of its own |
| `COLINFO` XF | `style_runs` (whole column) | Columns A–IV only, when the format is more than the default font |
| `WINDOW2`, `PANE`, `SCL` | `view_options` (formulas, grid, headers, zero values), `frozen` or `split`, `zoom` | Frozen panes: the counts of `PANE`. OpenOffice freezes at the first visible cell plus the counts (a sheet frozen while scrolled keeps its scroll position there); the model's frozen panes start at A1, so the counts alone are kept |
| `WINDOW1` | `settings.active_sheet` | |
| `SHEETEXT` | `tab_color` | Palette indices 8–63 only, as OpenOffice |
| `SETUP`, `LEFTMARGIN` … `BOTTOMMARGIN`, `HEADER`, `FOOTER`, `HCENTER`, `VCENTER`, `PRINTGRIDLINES`, `PRINTHEADERS`, `WSBOOL` | `page_setup` | Mapped like XLSX `pageSetup`, `pageMargins`, `headerFooter`, `printOptions` and `pageSetUpPr` (through `tally_io_xlsx::PageParts`, so header and footer sit in the page margins the same way): paper (the sizes the model names, the others as custom sizes from OpenOffice's table; code 0 and unknown codes are the default paper), orientation, scale (10–400 %), fit to pages wide × tall when `WSBOOL` bit 8 is set, first page number (with its flag), page order, the four margins and the header and footer margins (inches; a margin the file does not give is Excel's default of 0.75 / 1 / 0.5 in; NaN and ±∞ are ignored, negative is 0, at most 50 in), header and footer text with Excel's `&L &C &R &P &N &D &T &F &A &Z&F` codes (font codes dropped), centring, grid lines, headers. `SETUP` bit 2 ("never set") drops paper, scale and orientation, bit 6 the orientation. Black and white, draft quality, copies and print resolution are read past (the model has no place for them) |
| `HORIZONTALPAGEBREAKS`, `VERTICALPAGEBREAKS` | `row_breaks`, `col_breaks` | The row or column after each break; at most `Limits::max_page_breaks` (4,096) per sheet and direction |
| `NAME` with the built-in ids 0x06 (`Print_Area`) and 0x07 (`Print_Titles`) of a sheet | `page_setup.print_range`, `repeat_rows`, `repeat_cols` | Like `_xlnm.Print_Area` and `_xlnm.Print_Titles` in XLSX: one area is the print range (several areas are not kept); a title area over all 256 columns names rows, one over all 65,536 rows names columns. Only formulas made of `ptgArea3d`, `ptgRef3d`, `ptgUnion` and `ptgMemFunc` are read; the sheet is the one the name is local to |
| `MSODRAWINGGROUP` (BLIP store), `MSODRAWING` (client anchors) | `images` | PNG and JPEG, recognized by their signature (as for XLSX media; some writers store a PNG in a JPEG record). The two-cell anchor (column + 1/1024, row + 1/256) is converted to position and size with the column widths and row heights the sheet is shown at. A BLIP stored directly in the store (without a store entry) takes its index like an entry. EMF, WMF, PICT, DIB and TIFF pictures are left out with one warning; text boxes, shapes, controls, charts and group shapes with the pictures in them (placed relative to the group) with another |
| `NOTE` + the `TXO` of its object | `notes` (text, author, always shown) | |

Number formats decide what a number is, as for XLSX: under a format that
shows a calendar date the value is a date serial (the 1900 leap-year
mapping of `excel_serial_to_tally`); any other number is kept as it is.
Only a cell without a usable format that calamine reports as a date or
time (a code Tally cannot translate) gets a guessed `YYYY-MM-DD`,
`HH:MM:SS`, `YYYY-MM-DD HH:MM:SS` or `[HH]:MM:SS` format. Rich-text runs
in strings are flattened (the cell has its XF's font), as for XLSX
(parity X3).

A formula whose cell has no value in the file (a text result without its
`STRING` record) is kept as a formula without a cached value.

Not read: defined names other than the print area and print titles,
hyperlinks (`HLINK`), named styles (`STYLE`), superscript and subscript,
right-to-left sheets, conditional formats, data validation, outline
groups, AutoFilter, protection passwords, charts, text boxes, shapes and
grouped pictures.

BIFF5 (Excel 5.0/95) files are read for their values and formulas only:
their `XF`/`FONT` layouts differ, so the formatting pass is skipped and
the workbook opens with a warning saying so (dates then get the guessed
formats above). BIFF2–4 (Excel 2–4) are refused. Export is not supported
(documented in `docs/parity-deviations.md`); save as XLSX instead.

Pre-checks and limits (`tally_io_xls::Limits`, docs/security.md): the OLE2
signature, file size ≤ 256 MiB, ≤ 1024 sheets, ≤ 10 M cells, ≤ 512 MiB of
text; calamine runs under `catch_unwind`, so a malformed stream is an
error. The formatting pass never fails a file: what it cannot read is
skipped, and past a limit (65,536 fonts and cell formats, 16,384 number
formats, 16 MiB of number format codes and font names in the styles, 256
`COLINFO` per sheet, 1,048,576 `ROW` records, 10 M formatted cells, 4,096
page breaks per sheet and direction, 1,000 pictures of at most 16 MiB each
and 256 MiB together, 64 MiB of sheet drawing data, 65,536 notes) the rest
is left out with a warning. Formatted empty cells get the room the value
cells leave under the cell limit, so the formatting never turns a file
that opens into one that does not.
Tests: `crates/tally-io-xls/tests/formatting.rs`, `layout_review.rs`,
`page_setup.rs`, `hostile_formatting.rs` and `positions.rs`, on workbooks
built in code
(`tests/biff/`); `crates/tally-engine/tests/open_formats.rs`
(`xls_opens_with_its_formatting`).

### SXC, OpenOffice.org 1.x (`crates/tally-io-ods/src/sxc.rs`, read only)

The OOo 1.0 XML format maps onto the ODS reader through a name translation
applied while the XML is read (`XmlReader::new_legacy`):
`table:value-type`/`table:value`/`table:date-value`/… → `office:*`;
`office:body` → `office:spreadsheet` (OOo 1.x has no spreadsheet element);
one `style:properties` element per style → the ODF text/cell/paragraph/
column/row properties (also accepted by the ODS parser itself);
`style:text-underline="single"` → `style:text-underline-style="solid"`.
Formulas have no prefix and native syntax (`=SUM(A1:B1)`, `=Sheet2.A1`,
`='My Sheet'.A1`); references are bracketed into OpenFormula
(`oooc:=SUM([.A1:.B1])`). Values, formulas with cached results, styles and
data styles, column widths, row heights, merges, named ranges, annotations,
the document locale (`fo:language`/`fo:country` of the default style) and
print ranges are read. `.stc` templates open as untitled documents. The
same limits as ODS apply. Not written.

### HTML (`crates/tally-io-html`, read and write)

Import: an own tolerant tokenizer (no DOM, no script execution). Every
`<table>` becomes a sheet named after its `<caption>` (else `Table1`,
`Table2`, …; names are made unique); `<tr>`/`<td>`/`<th>` with `colspan`
and `rowspan` (which produce merged areas); `<br>` breaks lines;
`<script>`, `<style>`, `<template>`, comments, doctypes and processing
instructions are skipped; entities are decoded; whitespace is collapsed;
nested tables flow into the parent cell's text; empty tables are dropped.
Cells arrive as text and the engine runs the document locale's input
recognition over them (as it does for CSV), so `1,234.5` and `12/31/2026`
become a number and a date while `=1+1` stays text (formulas are never
evaluated from HTML). Limits: 64 MiB of input, 1024 tables, 10 M cells,
64 KiB per cell; the rest is cut with a warning. The same reader serves the
HTML clipboard paste.

Export (`FileFormat::Html`): a UTF-8 document with one `<table>` per
visible sheet, the sheet name as `<caption>`, `colspan`/`rowspan` for
merges, values formatted with their number formats in the document locale,
inline styles for bold/italic/underline, font family and size, text and
background colours and horizontal alignment (numbers right-aligned by
default), everything escaped. Formulas are exported as their results.

### dBase (`crates/tally-io-dbf`, read and write)

Import: dBase III/IV and FoxPro-compatible tables (version bytes 0x02–0x05,
0x07, 0x30/0x31, 0x43, 0x7B, 0x83, 0x8B, 0x8E, 0xCB, 0xE5, 0xF5, 0xFB).
Field types `C`/`V` (text), `N`/`F`/`Y` (numbers), `D` (`YYYYMMDD` → date
serial with a `YYYY-MM-DD` format), `L` (`T`/`Y` → TRUE, `F`/`N` → FALSE,
`?` → empty); `M`, `B`, `G`, `T`, `I`, `+` and other types leave the cell
empty. Deleted records (`*`) are skipped. The first row holds the field
names (as AOO imports). Text is decoded with the code page of the header's
language driver id (cp437, cp850, cp852 through built-in tables; ibm866,
windows-1250…1257, macintosh, Shift_JIS, GBK, Big5, EUC-KR through
`encoding_rs`; unknown ids → windows-1252) unless `DbfOptions::encoding`
names one. Limits: 256 MiB, 2 M records, 1024 fields, 10 M cells; a header
whose record count or record length disagrees with the data is refused.

Export (`FileFormat::Dbf`, first visible sheet): a dBase III header
(version 0x03, language driver of the chosen code page, windows-1252 by
default). The first row supplies the field names (upper-cased, 10
characters, made unique); each column's type is inferred from its values:
all dates → `D`, all booleans → `L`, all numbers → `N` with the widest
integer part and most decimals seen (≤ 20 characters), otherwise `C` with
the longest encoded text (≤ 254 bytes, longer text is cut). Unencodable
characters become `?`.

### Excel 2003 XML (SpreadsheetML) (`crates/tally-io-xmlss`, read only)

Import of the `.xml` "XML Spreadsheet" format of Excel 2002/2003, following
AOO 4.1's XSLT import filter (`spreadsheetml2ooo.xsl`). Sniffed from the
first 4 KiB: `<?mso-application progid="Excel.Sheet"?>` or a root
`Workbook` element in the `urn:schemas-microsoft-com:office:spreadsheet`
namespace (prefixed or not). Input is UTF-8 (BOM allowed); UTF-16 and a
declared non-UTF-8 `encoding=` are decoded with `encoding_rs`.

Read: document properties (title, subject, author, last author, created,
last saved, keywords, description); the active sheet and the 1904 date
system (`x:ExcelWorkbook`); styles with `ss:Parent` inheritance (every
style inherits `Default`): font (family, size, bold, italic, underline
style, strikethrough, colour), fill (`Solid`, other patterns mixed with
`ss:PatternColor` as AOO does), borders (four sides and diagonals, weight
0–3, continuous/double/dotted/dashed), alignment (horizontal, vertical,
wrap, indent, rotation, vertical text, shrink to fit), cell protection and
number formats: named formats (`General Date`, `Short Date`, `Currency`,
`Standard`, `Percent`, `Yes/No`, … mapped as the XSL does) and Excel codes
translated like XLSX ones. The `Default` style's font becomes the document
font. Sheets: names (made unique and non-empty), hidden sheets
(`SheetHidden`/`SheetVeryHidden`), frozen panes, sheet protection (without
a password), the AutoFilter range; columns and rows with `ss:Index`,
`ss:Span`, widths and heights (points; a row height is fixed unless
`ss:AutoFitHeight="1"`), hidden columns/rows, `ss:DefaultColumnWidth`,
column styles (whole-column style runs) and row styles; cells with
`ss:Index`, `ss:MergeAcross`/`ss:MergeDown` (merged areas), values
(`Number`, `String` including rich HTML text, `Boolean`, `DateTime` →
date serial, with a `YYYY-MM-DD[ HH:MM:SS]` or `HH:MM:SS` format when the
style has none, `Error`), R1C1 formulas converted to OpenFormula with
their cached values (`tally_io_xlsx::r1c1_formula_to_of`: R1C1 references
are rewritten to A1 relative to the cell, then parsed as ExcelA1; an
untranslatable formula keeps its text and cached value, with a warning),
`ss:ArrayRange` array formulas, `ss:HRef` hyperlinks and comments (notes
with author and "show always"); named ranges at workbook and sheet level
(sheet references made absolute), `Print_Area` and `Print_Titles` as the
sheet's print range and repeated rows/columns.

Dropped with a warning: conditional formatting, data validation,
AutoFilter criteria, pivot tables. Dropped silently: page setup, headers
and footers, print options, sorting and advanced filters, unfrozen splits,
zoom and view flags, tab colours, named cell styles, superscript and
subscript, custom document properties, `ss:DefaultRowHeight`.

Limits (`tally_io_xmlss::Limits`): 512 MiB of input, 10 M cells, 1 GiB of
text, 10,000 sheets, 64,000 styles, 100,000 names; see
`docs/security.md` §3c. Not written.

### SYLK (`crates/tally-io-sylk`, read only)

As AOO's `ScImportExport::Sylk2Doc` (`sc/source/ui/docshell/impex.cxx`).
Sniffed by an `ID;` first record. Records: `ID;P<program>` (`CALCOOO32`
and `SCALC3` are AOO's own files, any other producer is foreign), `C`
cells (`X`/`Y` persist between records; `K"text"`, `K<number>`; a value
followed by `;I` is ignored; `E` formula; `M` matrix formula over `R`/`C`
→ an array formula block), `P;P<code>` number formats (Excel codes,
translated as XLSX's) applied by `F;P<n>` at `X`/`Y`, `E` end. String and
formula escapes follow AOO's version rules (`;;` → `;`; StarCalc 3's
doubled quotes; `ESC :` → line break). The single sheet takes the file
name. AOO's own files carry native A1 formulas (`SUM(A1:B1;3)`), read as
`oooc:=` text with bracketed references
(`tally_io_ods::native_formula_to_of`). Dropped as in AOO: column widths
(`F;W`), fonts and styles (`P;F`, `F;S`), bounds, options, names.
Deviations: foreign (Excel) R1C1 formulas are converted (AOO drops them
and keeps the value; a formula that does not convert keeps the value
alone); `KTRUE`/`KFALSE` are booleans and `K#N/A`-style values errors (AOO
reads 0). Text is UTF-8 if valid, else Windows-1252.

### DIF (`crates/tally-io-dif`, read only)

As AOO's `ScFormatFilterPluginImpl::ScImportDif` / `DifParser`
(`sc/source/filter/dif/difimp.cxx`). Header topics (`TABLE` names the
sheet; `VECTORS`, `TUPLES`, `LABEL`, … are skipped) up to `DATA`, then
entries: `-1,0` + `BOT` (new row) / `EOD`; `0,<n>` + `V` (number), `TRUE`
/ `FALSE` (boolean), `NA` / `ERROR` (that text), other indicators (text
`#IND: x?`), an unparsable number (text `#ERR: n (x)`); `1,0` + a quoted
string (`""` → `"`; multi-line strings end at a line followed by an
entry, AOO's look-ahead; an empty string leaves the cell empty). Values
past the sheet are dropped with a warning (AOO's `eERR_RNGOVRFLW`).
Deviation: numbers use AOO's "plain" syntax (`ScanFloatVal`, `.` or `,`
decimal) instead of its number formatter, so no number format is
recognised. Text is UTF-8 if valid, else Windows-1252.

### PDF, printing and preview (`crates/tally-io-pdf`)

One rendering serves Export PDF, Print and the preview: the engine paginates
(`crates/tally-engine/src/print.rs`, see `docs/api.md` for the queries)
and builds a `PrintJob` of fills, lines, text and SVG items in points;
`tally_io_pdf::render_pdf` writes it with krilla 0.8.2 (MIT/Apache-2.0;
fonts subsetted by `subsetter`, charts through krilla-svg/usvg), and
`rasterize` renders a page to PNG with hayro 0.7.1 for the preview. The
static Inter 4.1 faces (Regular, Bold, Italic, BoldItalic; OFL-1.1, see
NOTICE) are embedded; other families are looked up in the system fonts
(fontdb) and fall back to Inter with a warning; characters a face lacks
fall back per character to an installed face (CJK) or are drawn as the
`.notdef` box with a warning. Cells print with their fills (conditional
fills included), borders (solid/dashed/dotted/double), alignment, wrapping,
overflow into empty neighbours, `###` for numbers that do not fit,
optional gridlines and row/column headers, headers and footers with
`&[PAGE]`-style placeholders (`&[PAGE]`, `&[PAGES]`, `&[SHEET]`, `&[FILE]`,
`&[PATH]`, `&[DATE]`, `&[TIME]`; each line of each region in its own 14 pt box
from the top of its area; lines past a fixed height are left off), charts from the SVGs the UI supplies, notes
on pages after each sheet's own pages when the page setup asks for them
(one line `A1: text` per note of the printed area; the pages count in the
page total and carry the header and footer, with "Comments" as the sheet
name, as in AOO). With formulas printed, hiding zero values does not hide
a formula. Without a print
range a sheet prints from A1 to the last cell with content (or with a
note, when notes are printed),
extended by fills and borders as AOO's `ScTable::GetPrintArea` does: a run
of 84 or more equal rows below a column's data, or 30 or more equally
formatted columns after the data, ends the area there, so formatting
repeated down or across whole columns prints no empty pages. Pictures and
conditional data bars/icons are not printed yet. The PDF has no outline,
tags or links.
