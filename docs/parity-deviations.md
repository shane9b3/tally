# Parity deviations from OpenOffice Calc 4.1.x

Every place Tally knowingly differs from Apache OpenOffice Calc 4.1.x, for the 1.0 release. Written 2026-10-23 against the tree after commit c992a3d.

**Why** column:

- **D** = deliberate: a decision, revisit only with a new ADR.
- **N** = not implemented (or out of scope for 1.0).
- **P** = platform limitation (macOS, Tauri/WebKit, a library).

Entries marked *unverified* come from a review or doc that I could not re-check against the current code. The Medium and Low findings of `docs/reviews/correctness-review-4-2026-10-21.md` (45 Medium, 33 Low) were not all triaged; only the High ones and the items named in `docs/security.md` "Review 4 fixes" are recorded as fixed. The Lows I know about are in section 5.

Companion files: `docs/calc-features-inventory.md` and `docs/calc-functions-inventory.md` (checklists), `docs/shortcuts.md` (keys), `docs/file-formats.md` (per-format losses), `docs/functions/deviations.md` (planned home for AOO oracle diffs; the file does not exist yet because the oracle run is owner-only, see `docs/TODO.md`).

## 1. Functions

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| F1 | Elementwise array ops on operands of different size (`{1;2;3}*{1;2}`, `SUMPRODUCT(A1:A3*B1:B2)`) | Result takes the smaller extent (`lcl_GetMinExtent`) | Larger extent, missing positions are `#N/A`, a single row or column is replicated (Excel/LibreOffice rule) | D | `docs/adr/0001-stack.md`; `crates/tally-engine/src/eval.rs` |
| F2 | Text ordering (`<`, `>`, sort, criteria like `">b"`) | Locale collator (`"é"<"f"` is TRUE) | Case-folded Unicode code-point order (`"é"<"f"` is FALSE); accented and non-Latin text can order differently | D | `docs/adr/0001-stack.md` |
| F3 | Case sensitivity default | Document default is case-sensitive | `CalcSettings::case_sensitive` defaults to false (`="A"="a"` is TRUE, criteria ignore case). ODS files that set `table:case-sensitive` keep their setting | D | `crates/tally-types/src/settings.rs`; ADR 0001 |
| F4 | String length functions (LEN, MID, LEFT, RIGHT, FIND ...) | Count UTF-16 code units (`LEN("😀")` = 2) | Count Unicode scalar values (`LEN("😀")` = 1) | D | ADR 0001 |
| F5 | NPV, IRR, MIRR with inline arrays | Only references (Err:504 for arrays) | Accept arrays, like Excel/LibreOffice | D | `tests/conformance/financial/{NPV,IRR,MIRR}.toml` |
| F6 | ODDFPRICE, ODDFYIELD | Not implemented in 4.1 (`#VALUE!`) | Implemented with the Excel formula | D | `tests/conformance/addin/ODDFPRICE.toml` |
| F7 | Functions AOO 4.1 lacks | n/a (NUMBERVALUE is declared but suppressed) | IFERROR, IFNA, NUMBERVALUE, ISOWEEKNUM(d) registered: 408 functions, 405 from AOO | D | `docs/functions/coverage.md` |
| F8 | SUBTOTAL codes 101-111 | Not available | Available (skip hidden rows as well as filtered rows); 1-11 skip filtered rows only | D | correctness review 4 (re-verification, `rv01`) |
| F9 | Whole-column / whole-row references (`A:A`, `1:1`) in Calc A1 syntax | Only in Excel A1 syntax | Accepted in both. ODS export writes `[.A$1:.A$1048576]` because AOO 4.1 cannot read `[.A:.A]` | D | `docs/phase-a.md`; correctness review 1 M8 |
| F10 | CURRENT() | Returns the running value of the formula | Always `#N/A` (the evaluator has no running-value hook) | N | `crates/tally-functions/src/categories/information/cell.rs` |
| F11 | DDE() | Reads a live DDE link | Always `#N/A`: DDE is disabled by policy | D (security) | `crates/tally-functions/src/categories/spreadsheet/misc.rs`; `docs/security.md` |
| F12 | Calling a Basic / Python macro as a function (`=MYFUNC(1)`) | Runs the macro | Not supported (no scripting) | N | T15 |
| F13 | Add-in name clashes with built-ins (`WEEKNUM`, `CONVERT`, `DURATION`, `EFFECT`, `NOMINAL`) | Built-in and add-in versions coexist; the add-in ones have `_ADD` names | Same rules, with documented XLSX name mapping. `WEEKNUM(d;2)` is written to XLSX as `_xlfn.ISOWEEKNUM(d)`; other modes as `_xlfn.ORG.OPENOFFICE.WEEKNUM` (Excel shows `#NAME?`) | P (Excel has no equivalent) | `docs/file-formats.md` |
| F14 | `VALUE("TRUE")`, `VALUE("")`, `DATEVALUE("14:30")` | May return 1 / Err:502 (reviewer's reading of the source) | `#VALUE!` | unverified (needs the AOO oracle run) | correctness review 1 M8 |

### Where Tally follows AOO and Excel or LibreOffice differ

These are not deviations from AOO. They are listed so nobody "fixes" them.

| Function / area | AOO and Tally | Excel / LibreOffice |
|---|---|---|
| MROUND with differing signs (`MROUND(-7;5)`) | Rounds: -5 | Err:502 / `#NUM!` |
| Booleans are numbers (`ISNUMBER(TRUE())` is TRUE; SUM/COUNT of a range include them as 1/0) | yes | Excel: FALSE; skipped in ranges |
| `=-2^2` | 4 (unary minus binds tighter than `^`) | -4 |
| MINVERSE singular matrix, MMULT with text | Illegal argument (Err:502) | `#NUM!` / `#VALUE!` |
| COUNTIFS / AVERAGEIFS with ranges of different size | Err:502 | `#VALUE!` |
| IMARGUMENT(0) and similar complex-number domain errors | Illegal argument | `#DIV/0!` |
| COMBINA domain errors, ATAN2(0;0) | Err:502 / documented per case | `#NUM!` / `#DIV/0!` |
| Database functions: criteria header naming no field; blank criteria row | Err:504; the blank row adds no OR alternative | Excel matches every record on a blank row |
| `WEEKNUM(d;mode)` | ISO-style rule (4-day minimum), every mode other than 1 is Monday | Excel week systems |
| FREQUENCY | Counts written at the bin's original index | same as Excel |
| ACCRINT `first_interest` | Ignored | Used |
| Transposed paste | Only fully relative references swap | n/a |

## 2. Formulas and parsing

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| P1 | Grid size | 1,024 columns (A..AMJ), 1,048,576 rows, 256 sheets | 16,384 columns (A..XFD). Names legal in AOO that read as cells in Tally (`Rel2`, `Tax1`) are renamed with a trailing `_` on ODS import, with a warning | D (Excel/XLSX compatibility) | `docs/file-formats.md` "Names" |
| P2 | R1C1 reference style | Selectable (Calc A1, Excel A1, Excel R1C1) | Calc A1 and Excel A1 only. `INDIRECT(...;0)` accepts R1C1 text. R1C1 and structured (table) references in XLSX formulas are kept as text | N | `crates/tally-formula/src/dialect.rs`; file-formats "Known gaps" |
| P3 | Natural-language labels (`Automatically find column and row labels`, `=SUM('Sales')`) | On by default | Not implemented. Create Names from labels works | N | grep found no label resolver in `tally-formula` |
| P4 | Regular expressions in criteria, MATCH, LOOKUP, SEARCH | ICU regex, on by default | `criteria_mode` setting (literal / wildcard / regex) using the Rust `regex` crate, size-limited. Syntax differences between ICU and Rust regex exist (no look-around, no back-references) | D | `crates/tally-functions/src/criteria.rs`; `crates/tally-engine/src/calcsettings.rs` |
| P5 | Circular references | Detected at interpretation time; an untaken IF branch is not circular | Review 1 M1 fixed: `=IF(TRUE;5;A1)` at A1 gives 5 (checked with `tally-cli`) | parity | correctness review 1 |
| P6 | Sheet references copied to another sheet (`=Two.A1` without `$`) | Relative to the tab and may shift | Stored as absolute sheet ids, so the reference keeps pointing at `Two` | D, *unverified* | correctness review 1 M8 |
| P7 | Formula printer for reversed ranges (`=B2:A1`) | Normalised | Printed as written | N, *unverified* | correctness review 1 M8 |
| P9 | Formulas that do not parse | Err:5xx cell | Saved to ODS as a text cell holding what was typed | D | `docs/phase-a.md` |
| P10 | External references and DDE | Live links (`'file:///x.ods'#$Sheet1.A1`, DDE) with an Edit > Links dialog | Disabled by policy: DDE gives `#N/A`, external references `#REF!` with a warning, linked sheets are imported as values, linked pictures are dropped. The original text is kept in the cell | D (security) | `docs/security.md` "External links and DDE"; `crates/tally-engine/src/external.rs` |
| P11 | WEBSERVICE, FILTERXML | Not in AOO 4.1 | Not registered | parity | `docs/functions/master-list.toml` |
| P12 | Locale-dependent parsing | Locale of the document | en-US plus de, fr, es function names and separators. Other locales fall back to en-US with a warning | N | `crates/tally-engine/src/convert.rs:80`; `docs/l10n.md` |
| P13 | Iterative references | Steps 100, min change 0.001 | Same defaults; from a file at most 1,000 steps (32,767 in the UI) | D (DoS limit) | `docs/security.md` |
| P14 | Computation budgets | Unlimited (hangs) | Per-formula, per-transaction and MULTIPLE.OPERATIONS caps give Err:512 (e.g. 10,000 table ops per recalc, 50M cells per transaction, VDB loop cap) | D (security) | `docs/security.md` section 8 |
| P15 | Sheets per document | 256 | 10,000 (`MAX_SHEETS`); files with more are cut with a warning | D (Excel/LibreOffice-sized documents, DoS cap) | `crates/tally-engine/src/workbook.rs:23`; test `security.rs` `sheet_count_is_capped` |
| P16 | Formula size limits | 512 tokens per formula (Err:512), 1,024-character symbols | No token limit: 8,192 characters per formula, 256 nested parentheses/calls, tree depth 1,024, 255 arguments (all Err:512 / Err:504 past the cap). A formula AOO rejects as too long can work in Tally | D (parser caps instead of a token count) | `crates/tally-formula/src/parser.rs:35-42`; test `inventory_1_0.rs` `formula_limits_are_larger_than_aoo` |
| P17 | Two-digit years | Window 1930-2029 by default, set in Tools > Options > General | Same 1930 window for input and DATEVALUE, but fixed: no option, not read from or written to files | N | `crates/tally-numfmt/src/locale.rs:119`; test `inventory_1_0.rs` `two_digit_year_window_is_1930` |
| P18 | Formula entry with a leading `+` or `-` | `+A1+B1` and `-A1` become formulas (`=+A1+B1`); `+5` stays a number | Only `=` starts a formula; `+B1+B2` and `-B1` are stored as text. Signed numbers work | N | `crates/tally-engine/src/workbook.rs` `set_input_opts` (checked 2026-10-27) |

## 3. File formats

| # | Format | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| X1 | XLSX | Import only | Import and export (via rust_xlsxwriter and patch passes) | D (extension) | `docs/file-formats.md` |
| X2 | XLSX unsupported content | Imports XLSB; macros as comments | `.xlsb`, VBA, XLSM macros and strict-OOXML-only packages are refused or ignored (`XlsxError::Unsupported`) | N | `docs/file-formats.md` |
| X3 | XLSX losses on read/write | n/a | No Excel tables (ListObjects), rich text flattened, `applyX` flags ignored, row styles (`<row s>`) not imported, East-Asian built-in formats 27-36 / 50-58 read as `YYYY-MM-DD`, error values written as `=#N/A` formulas, cached text that looks like a number written as number, D1900 workbooks re-import as D1899, hyperlinks on number/formula cells not written, custom paper sizes written as default, structured/R1C1/dynamic-array formulas kept as text. Some items predate wave 5 (custom `theme1.xml`, underline styles); *unverified* | N / P (library) | file-formats "Known gaps" |
| X4 | XLSX window split | n/a | Window > Split is not written to XLSX (`pane` without freeze); ODS keeps it | N | `docs/TODO.md` |
| X5 | XLSX charts | n/a | Bubble is written as Scatter, stock volume dropped, variance / error-margin error bars and mean-value lines dropped, surface charts skipped on read, unsupported trend lines read as linear. Each case adds a warning. Charts sharing an anchor are nudged 1 px apart | P (rust_xlsxwriter) | `docs/wave5-plan.md`; `crates/tally-io-xlsx/src/chart_write.rs` |
| X6 | XLSX pivot tables, named styles | n/a | Pivot written as a best-effort patch (Excel rebuilds on load); read: definition only (no show-as, grouping, sorting, hidden items). Named styles: names written, attributes stay on cells | P | file-formats "Wave 5 data tools" |
| X7 | XLS (Excel 97-2003) | Import and export | Import only (calamine). Not read: column widths, row heights, cell styles, number formats other than dates, names, hidden rows, notes, charts. BIFF2-4 refused. Save as XLSX instead | N / P (calamine) | `crates/tally-io-xls/src/lib.rs`; file-formats |
| X8 | XLS hostile files | n/a | Pre-scan guards calamine; a hostile file within budget can still use about 1 GB and an allocation failure aborts. Re-check on every calamine upgrade | P | `docs/TODO.md` |
| X9 | SXC / STC | Import and export | Import only; `.stc` opens as untitled | N | file-formats "SXC" |
| X10 | Formats not supported | Lotus 1-2-3, Quattro Pro, StarCalc 1.0-5.0, UOF, Pocket Excel, MiniCalc, DIF, SYLK, RTF import, SpreadsheetML 2003 XML, XHTML export, XLT/XLTX templates, Web Query | None of these | N (out of scope) | `docs/wave5-plan.md` "Never or post-1.0" |
| X11 | Password-protected ODS | Opens and saves (Blowfish/SHA1, AES) | Refused with `OdsError::Unsupported`. No "password to open" or "password to modify" at file level | N (wave 5 item 45 was a stretch goal, not done) | `crates/tally-io-ods/src/read.rs:60`; security.md section 5 |
| X12 | Sheet / cell protection hashes | ODF hash | Read and verified; XLSX spinCount clamped to 100,000; a hash with no legacy companion is dropped on XLSX export (reported). Unknown hash: any password unlocks | D | security.md |
| X13 | ODS child styles | Parent-child chain | Written fully resolved, so editing a parent style in AOO does not propagate (correctness review 4 L11) | N, *unverified* | review 4 |
| X14 | ODS whole-column style | Column default style, one cell with another style leaves the column default | Once a whole-column style run is split by one differently styled cell, the next export writes the rest of the column as repeated styled empty rows (same look, bigger `content.xml`) | N | `docs/file-formats.md` "Column default cell styles" |
| X15 | ODS fourth (text) number-format section | Supported | Not supported in ODS data styles (read error) | N | `crates/tally-io-ods/src/numfmt.rs:819` |
| X16 | ODS conditional formats | `style:map` | Reads legacy `style:map` and LibreOffice `calcext`; writes `calcext` plus a `style:map` fallback. Colour scales, data bars, icon sets are LibreOffice-style extensions (not in AOO 4.x) | D | file-formats "v0.3 feature coverage" |
| X17 | CSV import | Dialog with fixed width, merge delimiters, quoted-field-as-text, per-column types, language | Delimiter, quote, header row, encoding, trim, skip empty rows. Per-column types and fixed width are not in the engine options (*unverified* in the Text to Columns dialog). Formulas in fields (`=1+2`) are always imported as text unless `evaluate_formulas` is set | D (security) / N | `crates/tally-io-csv/src/lib.rs`; security.md |
| X18 | CSV export | Plain | Formula-injection guard on by default (fields starting with `=`, `+`, `-`, `@` are neutralised); export is sparse and size-capped | D (security) | security.md "CSV formula-injection guard" |
| X19 | HTML | Import with Web Query and live refresh; export keeps styles | Import by own tokenizer (no scripts, no web query); export one table per visible sheet with inline styles; formulas exported as results | D | file-formats "HTML" |
| X20 | dBase | dBase with memo support | III/IV/FoxPro tables; memo, binary and other field types leave the cell empty; export is dBase III with types inferred from the values, text cut at 254 bytes | N | file-formats "dBase" |
| X21 | PDF | Export | Export, print and preview from one engine (krilla). Pictures and conditional data bars/icons are not printed yet; no outline, tags or links in the PDF | N | file-formats "PDF" |
| X22 | Fonts in PDF | System fonts | Document fonts are looked up with fontdb; unknown families fall back to embedded Inter with a warning | D | file-formats; security.md 3d |
| X23 | Edit > Links, Link to External Data, Insert Sheet from File | Present | Not implemented (no live external data). Move/Copy sheet works within one document | N | features inventory sections 1 and 21 |
| X24 | CSV export | Options dialog: character set, field and text delimiters, save as shown, quote all text | No dialog: active sheet, UTF-8, comma, `"` only where needed, CRLF, as shown. The writer supports other delimiters and quote-all, but the engine always uses the defaults | N | `crates/tally-engine/src/service/codec.rs:193`; `crates/tally-io-csv/src/lib.rs` `CsvWriteOptions` |

## 4. Formatting and styles

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| G1 | Font weights | Any weight the family offers | Regular and Bold plus italic only | P (static Inter faces, single bold flag in `FontData`) | `crates/tally-model/src/lib.rs` `FontData` |
| G2 | Font effects | Relief, outline, shadow, double / bold / `/` / `X` strikethrough, wave underline with its own colour, emphasis marks, super/subscript, Western/Asian/CTL font sets | Bold, italic, underline (single, double, accounting variants), single strikethrough, colour, size, family | N | `FontData`; features inventory section 5 |
| G3 | Alignment | Also Filled, text direction (LTR/RTL), hyphenation, Asian typography | General, left, centre, right, justify; top/middle/bottom; wrap, indent, rotation, stacked, shrink to fit. No Filled, no RTL, no hyphenation | N | `AlignData` |
| G4 | Borders | Adds spacing to contents and a border shadow | Solid, dashed, dotted, double with width and colour, plus diagonals. XLSX diagonals with different up/down styles collapse to one; ODS accounting-double underline reads as Double (no warning) | N | `BordersData`; review 4 L16, L17 |
| G5 | Background | Solid colour | Solid colour only | parity | |
| G6 | Rich text in cells | Mixed formatting inside one cell | Not supported: a cell has one style; imported rich runs are flattened | N | file-formats |
| G7 | Number formats | NatNum / DBNum transliteration, calendar modifiers (`[~buddhist]`, `[~hijri]`), era codes | Format grammar with sections, conditions, colours, elapsed time, fractions, scaling. `[NatNum]` / `[DBNum]` are parsed and ignored (Western digits); calendar modifiers are ignored (Gregorian years); G / GG / GGG and E / EE give the Gregorian era (`AD`, `Anno Domini`) and year; `R` is not a code (checked with `tally-cli eval` 2026-10-27) | N | `crates/tally-numfmt` |
| G8 | Cell styles | Default, Result, Result2, Heading, Heading1, parent chains; page styles (Default, Report) | Cell styles as in AOO. No separate page-style objects: page setup is per sheet. Deleting a style moves its cells to the parent, not to Default (review 4 L10). `Result2` hard-codes `[$$-409]` under de-DE (L15) | N, *unverified* for L10 / L15 | `crates/tally-engine/src/cellstyles.rs` |
| G9 | Conditional formatting | Up to three conditions, applies named cell styles | Rules apply differential styles (bold, italic, underline, strike, colours), not named cell styles; unlimited rules; adds colour scales, data bars, icon sets, top/bottom N, duplicates, date periods | D | `crates/tally-api/src/features.rs` |
| G10 | Data validation | Error action Stop / Warning / Information / Macro | Macro action not supported (no scripting) | N | `ValidationErrorStyleData` |
| G11 | Column and row auto-size | Uses the printer / system font metrics | Header and column autofit measure with Inter metrics; other fonts are measured at runtime where available. Optimal heights can differ by a pixel or two from AOO | P (font rendering differs per OS) | `crates/tally-engine/src/print.rs:479`; `crates/tally-engine/src/autofit.rs` |
| G12 | Delete Contents > Formats | Clears merges, CF and validation | Keeps merges, CF and validation (review 4 L9) | N, *unverified* | review 4 |
| G13 | Measurement units | mm, cm, m, inch, pt, pica ... selectable | No units option found in Tools > Options | N, *unverified* | `packages/app/src/options/OptionsDialog.tsx` |
| G14 | Format Cells > Numbers | Category list, format list, language, decimal places, leading zeros, negative numbers red, thousands separator, format code | Preset chips, a format-code field and a live preview. No category or format list, no option controls and no language selector (`[$-LCID]` typed in the code works) | N | `packages/app/src/styles/StylesUi.tsx` numbers tab |
| G15 | Cell protection "Hide all", "Hide when printing" | Set in Format Cells > Cell Protection and applied | Read from and written to ODS (`hidden-and-protected`, `print-content`) only: no UI, cells are still shown and printed | N | `crates/tally-model/src/lib.rs:777`; `crates/tally-io-ods/src/styles.rs:397` |
| G16 | Validity selection list order | "Sort entries ascending" option (`display-list="sorted"`) | No option; ODS `sorted` is read as unsorted and saved as `unsorted` | N | `crates/tally-io-ods/src/v03.rs:1107,1156` |

## 5. Data tools, charts and objects

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| T1 | Find & Replace | ICU regex, similarity search, search for styles, search in comments, current selection only | Modes literal / wildcard (`*`, `?`, `~`) / regex (Rust `regex`); All sheets works (`FindQuery.sheet = None`). No similarity search, no search for styles, no search in comments, no current-selection scope | D / N | `crates/tally-api/src/dto.rs` `FindQuery`; `packages/app/src/components/Dialogs.tsx` find dialog |
| T2 | Standard filter | Includes "Does not begin with", "Does not end with", regex, no duplications, copy results | Operators = <> < > <= >= largest, smallest, % variants, contains, does not contain, begins with, ends with, empty, not empty. "Does not begin/end with" *unverified* | N | `crates/tally-model/src/tools.rs` |
| T3 | Sort | Collation algorithm and language options | Natural sort, left-to-right, copy to, custom order, include formats; no collator choice (see F2) | D | `crates/tally-api/src/tools.rs` |
| T4 | DataPilot sources | Sheet range, registered data source, external service | Sheet range only | N | features inventory 13 |
| T5 | DataPilot layout | Tabular and outline layouts; captions with several data fields | Outline layout ignored (`lay_out` never reads `layout`); captions for several data fields differ ("East Result \| _ \| Sum - Revenue") | N | `docs/TODO.md` (L6, L7); review 4 L6, L7 |
| T6 | DataPilot drill-down | Sheet named by `CreateValidTabName`, inserted at the current tab | Named `DataPilot1_Data`, placed after the pivot's sheet | N, *unverified* | review 4 L8 |
| T7 | Subtotals, Consolidate, Text to Columns | Grand total uses group 0; consolidate makes no row for an empty label; link mode builds detail rows and an outline; quote recognised only at field start | Grand total includes the second group's columns (L1); empty label makes a row (L2); link writes a single `COUNTA` with no detail or outline (L3); `ab"c,d"e,f` splits differently (L4); Remove Duplicates treats 1 and 1.0-formatted as distinct (L5). Remove Duplicates itself is a Tally extra | N, *unverified* | review 4 L1-L5 |
| T8 | Scenarios | "Prevent changes" honoured on a protected sheet | Ignored (L13). CopySheet duplicates DB range names (L14) | N, *unverified* | review 4 |
| T9 | Solver | lp_solve linear solver with options | Own two-phase simplex plus branch and bound (linear, integer, binary), `notLinear` status for non-linear models; no engine choice | D (lp_solve is LGPL) | `docs/wave5-plan.md` item 15 |
| T10 | Charts | Wizard; 3D charts with scene settings; column/cylinder/cone/pyramid shapes; net "filled"; charts are OLE objects | Chart editor panel instead of a wizard; types column, bar, line, area, pie, scatter, bubble, radar, stock (4 variants), stacking, combo line series, secondary axis, trend lines, error bars, mean line. The 3D flag is stored but 3D is not rendered; no shape variants | N | `crates/tally-model/src/charts.rs`; wave5-plan "Never or post-1.0" |
| T11 | Drawing layer, Gallery, Form controls, OLE, Floating frames | Present | Only charts and pictures are objects; no shapes, text boxes, form controls, Design Mode | N (out of scope) | wave5-plan |
| T12 | Detective | Trace Error, Fill Mode, Refresh Traces, AutoRefresh | Trace precedents / dependents / error, remove traces, mark invalid data. No Fill Mode, no Refresh Traces, and traces are not redrawn after edits (no AutoRefresh) (checked 2026-10-27) | N | `crates/tally-engine/src/detective.rs`; `packages/app/src/commands.ts:323-326` |
| T13 | Print comments / notes | Notes end pages counted; notes extend the print area | Notes-at-end pages not counted ("1 of 1" while the PDF has 2); notes do not extend the print area (L19, L20) | N, *unverified* | review 4 |
| T14 | Change tracking, shared workbooks | Edit > Changes, shared documents | Not implemented | N (post-1.0) | wave5-plan |
| T15 | Macros | StarBasic, Python, BeanShell, JavaScript; Alt+F11; document and sheet events | Not executed and never read (security). The ODS/XLSX reader skips `Basic/`, `Scripts/`, VBA. There is no "macros disabled" banner (wave 5 plan promised one; not found in `packages/app`) | N (security) | security.md section 5 |
| T16 | AutoFill (fill handle) | Ctrl-drag copies; filtered ranges copy; custom sort lists fill; ordinal suffixes (`1st`, `2nd`); a single number dragged up or left counts down | Always AutoFill analysis (no copy modifier, filtered rows not special); only the built-in en-US day and month names fill, not the Options sort lists; `1st` fills `2st`, `3st`; a single number dragged up or left counts up (5 gives 6, 7). Linear, date, numbered-text and formula fills match | N | `crates/tally-engine/src/fill.rs`; `packages/app/src/fill.ts:31` |
| T17 | DataPilot "Add filter" | Filter button above the table opens the filter dialog | Option kept; a plain `Filter` text row is written, with no button behind it | N | `crates/tally-engine/src/pivot.rs:2332` |
| T18 | Chart axes and data | Date axis type, axis crossing position, Chart Data Table editor | No date axis or crossing settings; internal (literal) data from files is drawn but cannot be edited | N | `crates/tally-model/src/charts.rs:349` |

## 6. Printing

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| R1 | Header / footer "Same content left/right" | Option on the Header and Footer tabs | Not implemented: one header and one footer for all pages | N | `PageSetupData` has no left/right pair |
| R2 | Page layout (right and left, mirrored, only right, only left) | Present | Not implemented | N | `PageSetupData` |
| R3 | Page style Borders and Background tabs, header / footer borders and background | Present | Not implemented | N | `PageSetupData` |
| R4 | Header area | Height includes spacing handled by the dialog; AutoFit height | Same model (area at the top margin, AutoFit height). XLSX import maps Excel's header margin into the area so sheets print as in Excel; Excel margins are kept and written back unchanged while untouched | D | `docs/file-formats.md` "Header and footer areas" |
| R5 | Header / footer text | Title, File name, Path, Sheet name, Page, Pages, Date, Time fields; predefined headers | `&[PAGE]`-style placeholders; each line of each region in its own 14 pt box; lines past a fixed height are dropped. Field set and presets *unverified* | N | `docs/file-formats.md` PDF section |
| R6 | Print objects, graphics, drawing objects options | Separate switches | Charts switch only (`print_charts`); pictures are not printed yet | N | `PageSetupData`; file-formats |
| R7 | Printing flow on macOS | Native print dialog | The PDF is shown in a `tally-print` window and `Webview::print()` is called; if that fails it opens the PDF in Preview. Whether WKWebView shows the print sheet for PDF content is not checked by hand | P | `docs/TODO.md`; `apps/desktop/src-tauri/src/print.rs` |
| R8 | Print-area edge cases | n/a | Formulas-print + hide zeros hides zero-result formulas (L18); a whole-row fill makes the area 16,384 columns wide (2,731 pages when suppression is off, L21) | N, *unverified* | review 4 |
| R9 | Optimal row heights when printing | AOO font metrics | Rows print at the height their text needs with AOO baseline; fonts differ from AOO's | P | git log; `print.rs:479` |
| R10 | Pagination limits | none | At most 10,000 pages per job and 20M cell positions | D (security) | security.md |
| R11 | Print range choice | Print dialog: All sheets, Selected sheets, Selected cells | Print, print preview and PDF export cover the active sheet only. The engine accepts all / some sheets and a selection (`PrintSheets`, `PrintOptionsDto.selection`), but the UI always sends the default | N | `crates/tally-api/src/print.rs:25`; `packages/app/src/print/printStore.ts:38` |

## 7. UI, shortcuts and platform

| # | Area | AOO 4.1 | Tally | Why | Reference |
|---|---|---|---|---|---|
| U1 | Fill Down | Ctrl+D is the selection list | `⌘D` fills down (Excel/macOS usage); selection list is `⌃D` | D | `docs/shortcuts.md` |
| U2 | Command palette and shortcut sheet | `⌘K` inserts a hyperlink; no palette | `⌘K` opens the command palette, `⌘/` the shortcut reference (AOO's numpad `⌘/` selects the array formula), hyperlink is `⌥⌘K` | D | shortcuts.md |
| U3 | Recalculation | F9 recalculates, Ctrl+Shift+F9 hard recalc, AutoCalculate toggle | `⇧⌘F9` is hard recalc, F9 recalculates dirty cells; AutoCalculate is always on (no toggle) | D | shortcuts.md |
| U4 | Sheet and tab switching | Ctrl+PgUp / PgDn switch sheets | Same (`⌃PgUp` / `⌃PgDn`). `⌘1` stays Format Cells, so document tabs use `⌃⇥` / `⌃⇧⇥`, `⇧⌘]` / `⇧⌘[` (macOS) and `⌥⌘1-9` (Alt+1-9 on Windows and Linux); 9 = last tab | D | shortcuts.md |
| U5 | Unbound AOO keys | F4 data source browser, F7 spelling, Ctrl+F7 thesaurus, Alt+F11 macros, Shift+F4 toggle reference, Shift+Space / Ctrl+Shift+Space row/column select, Shift+Backspace, Ctrl+Backspace, Ctrl+Alt+Shift+arrows, Ctrl+; date, Ctrl+Shift+R, Ctrl+J justify, Ctrl+Shift+B/P | Not bound. The features behind F4, F7, Alt+F11 are out of scope; the rest are missing (checked in `keymap.ts`, `selection.ts`, `W56Chrome.tsx`) | N | `packages/app/src/keymap.ts` |
| U6 | Alignment shortcuts | Ctrl+L / E / R / J | `⇧⌘L` / `⇧⌘E` / `⇧⌘R`; no justify key | D | shortcuts.md |
| U7 | New line in a cell | Ctrl+Enter | `⌥↩` while editing. Outside editing `⌥↩` fills the selection with the active cell (AOO: Alt+Enter commits into the whole selection). Tab / Shift+Tab move to the next / previous cell without skipping locked cells on a protected sheet (AOO jumps to the next unprotected cell) | D / N | `packages/app/src/data/W56Chrome.tsx`; `packages/grid/src/interaction.ts:757` |
| U8 | Spell check, AutoSpellcheck, thesaurus, hyphenation, Hangul/Hanja, Chinese conversion | Present | Skipped. The editor's `spellcheck` attribute gives macOS red underlines while editing only | N (platform spell checker only) | `docs/wave5-plan.md` item 18 |
| U9 | Euro Converter wizard | Present | Not implemented (EUROCONVERT function is) | N | wave5-plan |
| U10 | Documents as tabs | Separate windows (Window menu) | Tally's own tab system: child webviews in a window, a sidebar "Editing" list, Window menu items, Move Tab to New Window / Merge All Windows, "Open documents in: tabs / windows" option. Native macOS tabs are off | D / P (Tauri child webviews) | git 685c7cf; `docs/qa/multi-window-manual.md` |
| U11 | Quit and log-out with unsaved documents | Prompts to save | `⌘Q`, the red button, the Dock's Quit and log-out/restart/shutdown all prompt (`terminate:` through `crates/tally-macos-terminate`); Cancel at log-out interrupts it, as in other Mac apps. A second Dock Quit while the prompt is open quits anyway, keeping the unsaved documents for recovery (escape hatch for a hung prompt). Not yet checked by hand (Dock and log-out rows of the QA checklist) | P (manual check pending) | `docs/TODO.md`; `docs/qa/unsaved-changes-manual.md` |
| U12 | No window open (macOS) | Start Center | Only File > New and File > Open work from the menu | P | `docs/TODO.md` |
| U13 | Hidden tabs | n/a | Hidden documents are hidden WKWebViews; WebKit throttles their timers (about 0.6 ticks/s for a 1 s timer), so autosave can fire a little late | P (WebKit) | `docs/TODO.md` |
| U14 | Windows and Linux shells | Native | Undecorated windows rely on Tauri's resize handler; with child webviews the top resize edge may not work on Windows. Linux child webviews live in a GTK box. Both unverified; Intel macOS, Windows and Linux builds need CI | P | `docs/TODO.md` |
| U15 | UI languages and function names | About 40 UI languages | en, de, fr, es (menus, dialogs, panels, function names). Typing `=SUMME(...)` commits canonical English names | N | `docs/l10n.md`; `docs/qa/v0.5-data-tools-manual.md` |
| U16 | Accessibility | Java Access Bridge / NSAccessibility tables | Canvas grid with an ARIA mirror and live region. VoiceOver table navigation (VO+arrows reading headers) is not available; Narrator / NVDA unchecked | P (canvas) | `docs/qa/a11y.md` |
| U17 | Undo depth | 100 steps, configurable 1-1,000; undo / redo history drop-downs | Undo for every op, 100 steps fixed (`UNDO_DEPTH`), no setting; only the top undo / redo label is shown, no history list. Pivot refresh keeps every previous table on the undo stack (no byte budget) | N | `crates/tally-engine/src/undo.rs:15`; test `undo.rs` `undo_depth_is_100`; `docs/TODO.md` |
| U18 | Zoom and view | 20-400 %, Synchronize sheets, units, object display modes | Zoom 50-200 % (presets 50-200 %, optimal, page width, whole page); Options > View has grid lines, headers, zero values, show formulas, value highlighting; page breaks toggle. No object display modes (Show / Hide / Placeholder), guides, scroll-bar or outline-symbol toggles | N | `packages/app/src/formats.ts` `MIN_ZOOM` / `MAX_ZOOM`; `OptionsDialog.tsx` |
| U19 | Status bar | Selection-function menu | Sum / Count / Average; the choice menu and the status-bar items for signatures and selection mode are *unverified* | N | `packages/app/src/sumRange.ts` |
| U20 | Extras AOO lacks | n/a | Command palette, "At a glance" panel, assist (rule-based, no network), sidebar, light/dark themes, Remove Duplicates, sheet-local names | D (extensions) | `docs/architecture-plan.md` |
| U21 | Hyperlinks | Click or Ctrl-click follows | `⌘`-click opens http/https/mailto through Rust only; a plain click selects | D (security) | `docs/security.md` |
| U22 | Paste over data | Asks "You are pasting data into cells that already contain data" (Input option, on by default) | Pastes at once; Undo restores. No option | N | `packages/app/src/controller.ts:918` |

## Counts

| Area | Entries |
|---|---:|
| Functions (section 1) | 14 |
| Formulas and parsing (section 2) | 17 |
| File formats (section 3) | 24 |
| Formatting and styles (section 4) | 16 |
| Data tools, charts, objects (section 5) | 18 |
| Printing (section 6) | 11 |
| UI, shortcuts, platform (section 7) | 22 |
| **Total** | **122** |

Recounted 2026-10-27 after the inventory check added P15-P18, X24, G14-G16, T16-T18, R11 and U22 (the earlier table counted 14 P and 24 U entries; there were 13 and 21).
