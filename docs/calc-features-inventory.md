# OpenOffice Calc: User-Facing Feature Inventory

Scope: Apache OpenOffice Calc 4.x, taken from `github.com/apache/openoffice` (branch `trunk`). This file covers everything that is not a spreadsheet function. Functions are in `calc-functions-inventory.md` and `calc-functions.json`.

**Source key.** Each section names the files it was taken from. Paths are relative to `main/`.

- **[src]** means the item was confirmed in source code, resource (`.src`) files or configuration schema (`.xcs`/`.xcu`) files.
- **[help]** means the item is standard OpenOffice behavior described in the help or wiki that I did not trace to a specific line of code. Items marked [help] should be checked against a running AOO 4.1.x before they are treated as pass/fail parity tests.

---

## 0. Hard limits and global defaults

| Item | Value | Source |
|---|---|---|
| Rows per sheet | 1,048,576 (`MAXROWCOUNT_DEFINE`; old 65536 commented out) | `sc/inc/address.hxx` [src] |
| Columns per sheet | 1,024 (A..AMJ) (`MAXCOLCOUNT_DEFINE`) | `sc/inc/address.hxx` [src] |
| Sheets per document | 256 (`MAXTABCOUNT`) | `sc/inc/address.hxx` [src] |
| Max tokens per formula | 512 (`MAXCODE`) | `formula/inc/formula/FormulaCompiler.hxx`, `sc/inc/compiler.hxx` [src] |
| Max length of one symbol in a formula | 1024 chars (`MAXSTRLEN`) | `sc/inc/compiler.hxx` [src] |
| Max jumps in CHOOSE | 32 (`MAXJUMPCOUNT`), giving index plus 30 values | `FormulaCompiler.hxx` [src] |
| Max expression nesting depth | 42 recursion levels in `FormulaCompiler::Expression`, beyond which the formula fails with Err:514 | `FormulaCompiler.cxx` [src] |
| Max function arguments | 255 (the count is stored in a `sal_uInt8`). The Function Wizard UI shows up to 30 (`VAR_ARGS` = 30) | `FormulaCompiler.cxx`, `sc/inc/sc.hrc` [src] |
| Undo steps | default 100, configurable 1..1000 (`Office.Common/Undo/Steps`) | `officecfg/.../Common.xcs` [src] |
| Two-digit year window | 1930, so `00..29` becomes 2000..2029 and `30..99` becomes 1930..1999 (`Office.Common/DateFormat/TwoDigitYear`) | `Common.xcs` [src] |
| Null date (epoch) | 1899-12-30, serial 0 (`ScDocOptions::ResetDocOptions`: nDay=30, nMonth=12, nYear=1899) | `sc/source/core/tool/docoptio.cxx` [src] |
| Default zoom | 100%, "Synchronize sheets" on | `Calc.xcs Layout/Zoom` [src] |

---

## 1. Sheet management

Sources: `sc/source/ui/src/miscdlgs.src`, `globstr.src`, `popup.src` [src], plus [help].

- **Insert sheet.** Choose the position (before or after the current sheet), the number of sheets, and a name. You can also create a sheet from a file (Insert > Sheet from file), which can be kept as a **link** to that file.
- **Delete sheet**, **Rename sheet**, and **Move/Copy sheet**. The Move/Copy dialog offers: To document (the current document, another open document, or "- new document -"), Insert before, and Copy.
- **Hide and Show sheets.** The Show Sheet dialog lists the hidden sheets.
- **Sheet tab color** [help].
- **Select multiple sheets**, using Ctrl/Shift-click on tabs. Edits then apply to all selected sheets.
- **Navigation:** Ctrl+PageDown and Ctrl+PageUp go to the next and previous sheet. Adding Shift extends the sheet selection. The Navigator is F5.
- **Sheet events** (macro on activate/deactivate/double-click/right-click/change/calculate) [help].
- **Scenario sheets** (see section 13).
- **Sheet references:** `$Sheet1.A1` in Calc A1 syntax.
- **Names:** sheet names may contain spaces. Quote them as `'My Sheet'.A1`.

## 2. Cell editing and input behaviors

Sources: `sc/source/ui/view/tabvwsh4.cxx` (`ScTabViewShell::TabKeyInput`), `Calc.xcs Input/*` [src].

| Behavior | Detail |
|---|---|
| Enter | Commits the entry and moves the cursor. Direction defaults to Down and is configurable (Down/Right/Up/Left) through `Input/MoveSelection=true` and `MoveSelectionDirection=0` |
| Shift+Enter | Commits and moves in the opposite direction (`SID_CURSORENTERUP`) |
| Alt+Enter | Commits the entry into **every cell of the selection** (`SC_ENTER_BLOCK`) |
| Ctrl+Shift+Enter | Commits as an **array (matrix) formula** (`SC_ENTER_MATRIX`) |
| Ctrl+Enter | Inserts a manual line break inside the cell text (the key is passed through to the edit engine) |
| "Press Enter to switch to edit mode" | Option `Input/SwitchToEditMode`, default false |
| F2 | Enters edit mode (`.uno:SetInputMode`). Ctrl+Shift+F2 focuses the input line |
| Esc | Cancels editing |
| Backspace (not editing) | Clears contents directly. Delete opens the **Delete Contents** dialog, whose options are Delete all, Text, Numbers, Date & time, Formulas, Formats, Comments and Objects |
| **AutoInput** (autocomplete) | Suggests completions from other entries in the same column. Option `Input/AutoInput`, default true |
| Expand formatting | Option `Input/ExpandFormatting`, default false. Formatting a cell also affects cells entered next to it |
| Expand references | Option `Input/ExpandReference`, default false. References grow when rows or columns are inserted next to them |
| Show references in color | Option `Input/ShowReference`, default true. References in the formula being edited are highlighted in color in the grid |
| Highlight selection in headers | Option `Input/HighlightSelection`, default true |
| Use Tab column on Enter | Option `Input/UseTabCol`, default false. After Tab, Tab, Enter, the cursor returns to the column where tabbing started |
| Overwrite warning when pasting | Option `Input/ReplaceCellsWarning`, default true |
| Use printer metrics | Option `Input/UsePrinterMetrics`, default false |
| Detective auto-refresh | Option `Input/DetectiveAuto`, default true |
| Automatic number recognition | Typed text is parsed by the number formatter and the cell gets a matching format: dates by locale pattern, times, percent `5%`, currency, scientific `1E5`, fractions `1 1/2`, booleans TRUE/FALSE [help] |
| Leading apostrophe | `'123` forces text entry |
| Formula entry | Starts with `=`. A leading `+` or `-` also starts a formula [help] |
| Alt+Up/Down/Left/Right | Change row height or column width. Adding Shift sets optimal height or width (`ModifyCellSize`) [src] |
| Ctrl+Alt+Shift+arrows | Move the cursor without discarding the current selection [src] |
| Alt+PageUp/PageDown | Scroll one page left or right (Shift extends the selection) [src] |
| Insert/Delete Cells dialogs | Insert: Shift cells down, Shift cells right, Entire row, Entire column. Delete: Shift cells up, Shift cells left, Delete entire row(s), Delete entire column(s) (`miscdlgs.src`) |
| Paste Special | Selection: Paste all, Text, Numbers, Date & time, Formulas, Formats, Comments, Objects. Operations: None, Add, Subtract, Multiply, Divide. Options: Skip empty cells, Transpose, Link. Shift cells: Don't shift, Down, Right (`miscdlgs.src`) |
| Selection list | Ctrl+D (`.uno:DataSelect`) opens a pick-list of the entries already in the column |
| Merge cells | Format > Merge Cells. If the hidden cells contain data, you are asked whether to move it into the first cell |
| Text to Columns | Data > Text to Columns. Uses the CSV import options from section 20 |
| Recognize column/row labels in formulas | "Automatically find column and row labels", `Calculate/Other/FindLabel` default true. Natural-language references such as `=SUM('Sales')` [help] |

## 3. Selection and navigation

- Click, Shift-click (extend), and Ctrl-click (multi-range selection; Cut is refused on a multi-selection with error `STR_NOMULTISELECT` [src]).
- Clicking a row or column header selects the whole row or column. **Shift+Space** selects the row and **Ctrl+Shift+Space** selects the column (`Accelerators.xcu`) [src]. Ctrl+A selects all.
- **Ctrl+\*** (numpad multiply) selects the data area around the cursor (`.uno:SelectData`). **Ctrl+/** selects the array formula containing the cursor (`.uno:SelectArrayFormula`) [src].
- **F8** turns on extend-selection mode (`StatusSelectionModeExt`). **Shift+F8** turns on add-selection mode (`StatusSelectionModeExp`) [src].
- Shift+Backspace deselects. Ctrl+Backspace scrolls back to the current cell [src].
- **Ctrl+arrows** jump to the edge of the data block. **Ctrl+Home** goes to A1. **Ctrl+End** goes to the last used cell. **Home** and **End** go to the start or end of the row [src].
- PageUp and PageDown move one screen up or down [src].
- **Tab** and **Shift+Tab** move to the next or previous unprotected cell (`JumpToNextUnprotected`) [src].
- The **Name Box** (cell address field, Ctrl+Shift+T is `.uno:FocusCellAddress`) [src] accepts an address, a range or a named range to jump to.
- **Navigator** (F5) lists sheets, range names, database ranges, linked areas, images, OLE objects, comments and drawing objects.

## 4. Fill handle, AutoFill and Fill Series

Sources: `sc/source/core/data/table4.cxx` (`ScTable::FillAnalyse`, `FillAuto`, `FillSeries`), `sc/inc/global.hxx` (`FillCmd`, `FillDateCmd`), `sc/source/core/tool/userlist.cxx`, `miscdlgs.src` [src].

**Fill commands** (`FillCmd`): `FILL_SIMPLE` (copy pattern), `FILL_LINEAR`, `FILL_GROWTH`, `FILL_DATE`, `FILL_AUTO`. **Date units** (`FillDateCmd`): `FILL_DAY`, `FILL_WEEKDAY`, `FILL_MONTH`, `FILL_YEAR`.

**AutoFill analysis**, which runs when you drag the fill handle:

1. **Ctrl held while dragging**, or **the data is filtered**: the selection is copied, with no increment (`nScFillModeMouseModifier & KEY_MOD1`).
2. **Date-formatted numbers**:
   - A single cell steps by +1 day.
   - Several cells: if the day-of-month differs, the step is a constant day difference. If only the month differs, the step is in months. A month step that is a multiple of 12 becomes a year step.
   - The series must be consistent across the whole source range. Otherwise the source pattern is repeated.
3. **Numbers** (two or more cells): if every difference is approximately the same, it fills a linear series with that step.
4. **A single number** (or an inconsistent series): the pattern is copied, but each repetition is incremented by +1 (or -1 when dragging up or left). This is `FILL_SIMPLE` with `nDelta = ±1` and is the classic Calc behavior.
5. **Text matching a sort list** (Tools > Options > Sort Lists): steps through the list, wrapping around at the end. The default lists are built from the locale calendar: abbreviated and full **day names**, and abbreviated and full **month names**.
6. **Text with a leading or trailing integer** (`Item1`, `1st`, `Q3`): the number is incremented, keeping leading zeros (`rMinDigits`). English ordinal suffixes (1st, 2nd, 3rd...) are handled (`bIsOrdinalSuffix`).
7. **Formulas**: copied with relative references adjusted.

**Fill Series dialog** (Edit > Fill > Series):

- Direction: Down, Right, Up, Left.
- Series type: Linear, Growth, Date, AutoFill.
- Time unit: Day, Weekday, Month, Year.
- Start value, End value, Increment.

**Other fill commands:** Edit > Fill > Down, Right, Up, Left, Sheet (fill selected sheets) [src/help].

## 5. Cell formatting

Format Cells dialog tabs (`attrdlg.src`) [src]: **Numbers**, **Font**, **Font Effects**, **Alignment**, **Asian Typography**, **Borders**, **Background**, **Cell Protection**.

- **Font:** family, style (bold or italic), size, language. Western, Asian and CTL fonts are set separately.
- **Font Effects:**
  - Underline, double underline and wave underline, each with its own color.
  - Strikethrough: single, double, bold, `/` and `X`.
  - Font color.
  - Relief (embossed or engraved), outline, shadow.
  - Emphasis marks (Asian).
- **Alignment:**
  - Horizontal: Default, Left, Center, Right, Justified, Filled.
  - Indent.
  - Vertical: Default, Top, Middle, Bottom.
  - Text orientation: rotation angle 0-360 degrees, a reference edge, and vertically stacked text.
  - Wrap text automatically, Hyphenation, Shrink to fit cell size.
  - Text direction (writing direction LTR/RTL).
- **Borders:**
  - Presets.
  - Separate left, right, top and bottom lines, plus inner horizontal and vertical lines for a multi-cell selection.
  - Diagonal lines (top-left to bottom-right and bottom-left to top-right).
  - Line style (several single and double widths), line color, spacing to contents.
  - Shadow: position, distance, color.
- **Background:** solid color (graphics are not available for cells).
- **Cell Protection:** Protected, Hide formula, Hide all, Hide when printing. The dialog notes that protection only takes effect once the sheet itself is protected.
- **Merge and Center** / Merge cells, and Unmerge.
- Toolbar shortcuts: bold, italic and underline (Ctrl+B, Ctrl+I, Ctrl+U), align left, center, right and justify (Ctrl+L, Ctrl+E, Ctrl+R, Ctrl+J), and number formats (Ctrl+Shift+1..6, see section 26).
- **Default formatting** (`.uno:ResetAttributes`, Ctrl+M) removes direct formatting.
- **AutoFormat** (`autofmt.src`): table presets. Each preset can apply Number format, Font, Alignment, Borders, Pattern, and AutoFit width/height. You can add, rename and delete presets.
- **Row height and column width:**
  - Explicit value, "Default value", Optimal (with an "Add" margin).
  - Hide and Show.
  - Header context menus (`hdrcont.src`) offer Row Height..., Optimal Row Height..., Insert/Delete Rows, Hide/Show, Column Width..., and Optimal Column Width....
- **Group and Outline** (F12 groups, Ctrl+F12 ungroups), AutoOutline, Show/Hide Details, Remove outline.
- **Value highlighting** (Ctrl+F8 `ViewValueHighlighting`): text is shown black, numbers blue, and formulas and other content green (`Calc.xcs Content/Display/ValueHighlighting`).

## 6. Number format codes (grammar)

Implemented by the shared number formatter in `svl/source/numbers/zformat.cxx`, `zforscan.cxx`, `svl/inc/svl/nfkeytab.hxx` [src]. Formatting details not read line-by-line are marked [help].

### 6.1 Structure

```
format      := section [ ";" section [ ";" section [ ";" section ] ] ]
section     := [ "[" color "]" ] [ "[" condition "]" ] [ "[$" currency "-" LCID "]" ] [ modifier ]* body
```

- **Default meaning when there are no conditions:**
  - Section 1 is for positive numbers and zero. If there are two or more sections, section 1 is positive only.
  - Section 2 is for negative numbers. These are displayed **without** a minus sign unless the section adds one itself.
  - Section 3 is for zero.
  - Section 4 is for text.
- **Conditions** `[<0]`, `[>=100]`, `[=1]`, `[<>0]` and so on replace these defaults. The compare operators are `< > = <= >= <>`. Up to 3 conditional sections can be used, plus a final "else" section.
- **Single-section formats with `@` only** are text formats.

### 6.2 Number placeholders and symbols

| Code | Meaning |
|---|---|
| `0` | Digit placeholder; shows a zero when there is no digit |
| `#` | Digit placeholder; no leading or trailing zeros |
| `?` | Digit placeholder; pads with a space (aligns decimals and fractions) |
| `.` | Decimal separator (locale decimal in the UI; `.` in English or ODF format codes) |
| `,` | Thousands separator when placed between digit placeholders. **Trailing** `,` after the digits scales the value by 1000 for each comma |
| `%` | Multiply by 100 and show `%` |
| `E+` `E-` `e+` `e-` | Scientific notation (`NF_KEY_E`). `E+` always shows the exponent sign; `E-` shows it only when negative. The exponent digits come from the following `0`/`#` |
| `/` | Fraction: `# ?/?`, `# ??/??`, fixed denominator `# ?/16`. Without an integer part, as in `?/?`, the whole value is shown as an improper fraction |
| `@` | Text placeholder (the cell's text) |
| `"text"` | Literal text |
| `\c` | Literal single character |
| `_c` | Space as wide as character `c` |
| `*c` | Repeat character `c` to fill the column width |
| `-` `+` `(` `)` `:` `$` space | Shown literally |
| `General` (English) / `Standard` (German keyword) | Standard format (`NF_KEY_GENERAL`). Decimal places are capped by "Limit decimals for general number format" (`Calculate/Other/DecimalPlaces`, default -1, meaning unlimited) |

### 6.3 Date and time codes (`NF_KEY_*` from `nfkeytab.hxx`)

| Code | Meaning |
|---|---|
| `D` / `DD` | Day of month, 1-31 / 01-31 |
| `DDD` / `NN` | Abbreviated weekday name |
| `DDDD` / `NNN` | Full weekday name |
| `NNNN` | Full weekday name followed by the separator |
| `AAA` / `AAAA` | Abbreviated / full weekday name (Japanese Excel compatibility) |
| `M` / `MM` | Month, 1-12 / 01-12. `M` right after `H` or `HH`, or right before `S` or `SS`, means **minutes** |
| `MMM` / `MMMM` / `MMMMM` | Abbreviated month name / full month name / first letter of month name |
| `YY` / `YYYY` | Two-digit / four-digit year |
| `Q` / `QQ` | Quarter: "Q1" / "1st quarter" |
| `WW` | Calendar week |
| `H` / `HH` | Hours |
| `[H]` `[HH]` `[M]` `[MM]` `[S]` `[SS]` | **Elapsed** time: the hours (or minutes, or seconds) count past 24 (or 60) |
| `MI` / `MMI` | Explicit minute keyword (internal) |
| `S` / `SS` | Seconds. Fractional seconds as `SS.00` |
| `AM/PM` / `A/P` | 12-hour clock with marker (`NF_KEY_AMPM`, `NF_KEY_AP`) |
| `E` / `EE` / `R` / `RR`, `G` / `GG` / `GGG` | Non-Gregorian calendar year and era (Japanese Gengou and others) |
| `[~calendar]` | Calendar modifier, e.g. `[~buddhist]`, `[~gengou]`, `[~ROC]`, `[~hanja]`, `[~hijri]`, `[~jewish]` [help] |

### 6.4 Bracket modifiers

| Modifier | Meaning |
|---|---|
| `[BLACK] [BLUE] [GREEN] [CYAN] [RED] [MAGENTA] [BROWN] [GREY] [YELLOW] [WHITE]` | Color keywords (`NF_KEY_BLACK`..`NF_KEY_WHITE`) |
| `[COLORn]` | Palette color n (`NF_KEY_COLOR`) [help] |
| `[$sym-LCID]` | Currency symbol and/or locale, e.g. `[$€-407]`, `[$$-409]`, `[$-409]` (locale only) |
| `[NatNumN]` | Native number transliteration (N = 0..11, locale dependent) |
| `[DBNumN]` | Excel-compatible DBNum (mapped to NatNum) |
| `CCC` | Bank currency code (old keyword) |
| `TRUE` / `FALSE` / `BOOLEAN` | Boolean format keywords (`NF_KEY_TRUE`, `NF_KEY_FALSE`, `NF_KEY_BOOLEAN`); `BOOLEAN` shows TRUE/FALSE |

### 6.5 Format categories in the dialog

All, User-defined, Number, Percent, Currency, Date, Time, Scientific, Fraction, Boolean Value, Text.

The dialog options are Decimal places, Leading zeroes, Negative numbers red, Thousands separator, and Format code (editable, with comment). Formats are language-dependent: there is a "Language" selector, and "Default" follows the cell's locale [help].

## 7. Styles

- **Cell styles** (`globstr.src` `STR_STYLENAME_*`) [src]: built-in **Default**, **Result**, **Result2**, **Heading**, **Heading1**.
- **Page styles:** **Default** and **Report**.
- Style hierarchy: a style inherits from its parent. Styles are created from a selection, updated from a selection, applied with the fill-format (paint can) mode, or applied by dragging.
- Stylist/Styles window: F11 (`.uno:StyleCatalog` is bound to Ctrl+F11). Shift+F11 saves the document as a template. Styles can be loaded from other documents.
- The `STYLE()` function applies a cell style from a formula, with an optional timed switch to a second style.
- Conditional formats apply **cell styles** (they do not apply direct formatting).

## 8. Conditional formatting

Source: `sc/source/ui/src/condfrmt.src` [src]. The AOO 4 dialog has **three condition rows**.

- **Condition N:** "Cell value is" or "Formula is".
- **Operators:** equal to, less than, greater than, less than or equal to, greater than or equal to, not equal to, between, not between.
- Each condition has one or two value/formula fields, which may contain references, and a **Cell Style** to apply. A "New Style..." button is offered [help].
- The first matching condition wins.
- Color scales, data bars and icon sets are **not** in AOO 4.x. Those are LibreOffice-only.

## 9. Data validation (Data > Validity)

Source: `sc/source/ui/dbgui/validate.src` [src].

- **Criteria tab:**
  - **Allow:** All values, Whole Numbers, Decimal, Date, Time, Cell range, List, Text length.
  - **Data:** equal, less than, greater than, less than or equal, greater than or equal to, not equal, between, not between.
  - Minimum/Maximum, or Source (for Cell range and List).
  - Allow empty cells.
  - Show selection list, with optional ascending sort of the entries (for List and Cell range).
- **Input Help tab:** show input help when the cell is selected, with a Title and an Input help text.
- **Error Alert tab:**
  - Show error message.
  - **Action:** Stop, Warning, Information, or Macro (Browse to pick it).
  - Title and Error message.
- **Tools > Detective > Mark Invalid Data** circles cells that fail validation [help].

## 10. Sort

Source: `sc/source/ui/src/sortdlg.src` [src].

- **Sort Criteria:** Sort by, then by, then by (3 keys), each Ascending or Descending.
- **Options:**
  - Case sensitive.
  - Range contains column labels (or row labels when sorting left to right).
  - Include formats.
  - Copy sort results to (an output position).
  - Custom sort order (any of the sort lists).
  - Language (collator) and collation algorithm options for the locale [help].
  - Direction: Top to bottom (sort rows) or Left to right (sort columns).
- **Range extension prompt:** "The cells next to the current selection also contain data..." with the choices **Extend selection** and **Current selection**.
- Toolbar Sort Ascending/Descending sorts the current data area by the cursor column.

## 11. Filters

Source: `sc/source/ui/src/filter.src`, `scstring.src` [src].

### AutoFilter

- Adds a drop-down button to each column header in the database range.
- **Entries:** **All**, **Standard Filter...**, **Top 10**, **- empty -**, and the list of unique values in the column. "(empty)" is the label for blank cells.
- A filter applies to one database range per sheet. An anonymous database range is created automatically when needed.

### Standard Filter

- Up to **4 condition rows**, combined with **AND** or **OR**.
- Each row has a Field name, a Condition and a Value. The value drop-down lists unique entries.
- **Conditions:** `=`, `<`, `>`, `<=`, `>=`, `<>`, Largest, Smallest, Largest %, Smallest %, Contains, Does not contain, Begins with, Does not begin with, Ends with, Does not end with.
- **Options:**
  - Case sensitive.
  - Regular expression (ICU regex through `utl::TextSearch`). Wildcards `*` and `?` are **not** a separate mode in AOO; that is LibreOffice-only.
  - Range contains column labels.
  - No duplications.
  - Copy results to... (an output position), with "Keep filter criteria" (keeps the output linked to the source).

### Advanced Filter

- "Read filter criteria from" a cell range laid out in Excel style:
  - The header row holds field names.
  - Criteria in the same row are combined with AND; separate rows are combined with OR.
  - Criteria are written like `>100` or `="text"`.
- Same Options as the Standard Filter.

### Other filter commands

- **Reset Filter** and **Hide AutoFilter**.
- Filtered rows are hidden. Copy and Fill operate only on visible cells when the data is filtered (see the AutoFill rule in section 4).

## 12. Subtotals (Data > Subtotals)

Source: `sc/source/ui/src/subtdlg.src` [src].

- **1st, 2nd and 3rd Group tabs.** Each has:
  - **Group by** (a column).
  - **Calculate subtotals for** (checklist of columns).
  - **Use function:** Sum, Count, Average, Max, Min, Product, Count (numbers only), StDev (Sample), StDevP (Population), Var (Sample), VarP (Population).
- **Options tab:**
  - Page break between groups.
  - Case sensitive.
  - Pre-sort area according to groups, Ascending or Descending.
  - Include formats.
  - Custom sort order.
- **Delete** removes the subtotals.
- Inserts `SUBTOTAL(n; range)` formulas and an outline.

**SUBTOTAL function codes:** 1 AVERAGE, 2 COUNT, 3 COUNTA, 4 MAX, 5 MIN, 6 PRODUCT, 7 STDEV, 8 STDEVP, 9 SUM, 10 VAR, 11 VARP. Nested SUBTOTAL results are excluded. AOO 4.x has no 101-111 variants [help].

## 13. DataPilot (pivot tables)

Sources: `sc/source/ui/dbgui/pivot.src`, `pvfundlg.src`, `dapitype.src`, `dpgroupdlg.src` [src].

- **Source:**
  - Current selection.
  - Data source registered in OpenOffice (a database table or query).
  - External source/interface (a UNO DataPilot service, with Source, Name, User and Password).
- **Layout areas:** Page Fields, Column Fields, Row Fields, Data Fields. Fields are dragged from the field list. A "Data" pseudo-field is used when there are several data fields.
- **Data field functions:** Sum, Count, Average, Max, Min, Product, Count (Numbers only), StDev (Sample), StDevP (Population), Var (Sample), VarP (Population).
- **"Displayed value" (show data as):** Normal, Difference from, % of, % difference from, Running total in, % of row, % of column, % of total, Index.
  - Base field and base item are chosen for the relative modes.
  - Base item can be "- previous item -" or "- next item -".
- **Field options:**
  - Subtotals: none, automatic, or user-defined functions.
  - Show items without data.
  - Sort: ascending, descending or manual, by name or by data field.
  - Layout: Tabular layout, Outline layout with subtotals at the top, or Outline layout with subtotals at the bottom. Optional empty line after each item, and optional page break [help].
  - AutoShow: **Top** or **Bottom** N items by a data field.
  - Hide items.
  - Hierarchy.
- **Result options:**
  - Selection from, Results to (output position, which can be a new sheet).
  - Ignore empty rows.
  - Identify categories (fills blank category cells).
  - Total columns, Total rows.
  - Add filter (a Filter button above the table).
  - Enable drill to details (double-click a result to see its source rows).
- **Grouping:**
  - Numeric ranges: Start (Automatically or Manually at), End, Group by (interval).
  - Dates: Seconds, Minutes, Hours, Days (with "Number of days"), Months, Quarters, Years.
  - Manual grouping of selected items.
- **Refresh** and **Delete**. Double-click on a field button opens field options.
- The `GETPIVOTDATA` function reads results from a pivot table.

## 14. Charts

Source: `chart2/source/controller/dialogs/Strings_ChartTypes.src`, `ChartTypeDialogController.cxx`, `tp_ChartType.src`, `res_Trendline_tmpl.hrc`, `res_ErrorBar_tmpl.hrc`, `Strings_Statistic.src`, `res_BarGeometry.src` [src].

### Chart wizard steps

1. Chart Type.
2. Data Range (data series in rows or columns, first row as label, first column as label).
3. Data Series (customize each series range: name, values, categories).
4. Chart Elements (titles, legend, grids).

### Chart types

| Type | Variants |
|---|---|
| **Column** | Normal, Stacked, Percent Stacked. 3D: Deep or flat. Shapes: box, cylinder, cone, pyramid |
| **Bar** | Same variants as Column |
| **Pie** | Normal, Exploded Pie, Donut, Exploded Donut. 3D look |
| **Area** | Normal, Stacked, Percent Stacked. 3D look |
| **Line** | Points Only, Points and Lines, Lines Only, 3D Lines. Each can be Normal, Stacked or Percent Stacked. Smooth lines: Cubic spline or B-spline, with resolution and polynomial degree |
| **XY (Scatter)** | Points Only, Points and Lines, Lines Only, 3D Lines. Optional "Sort by X values" and smooth lines |
| **Bubble** | Bubble Chart |
| **Net** (radar) | Points Only, Points and Lines, Lines Only, Filled. Each can be Normal, Stacked or Percent Stacked |
| **Stock** | Stock Chart 1 (Low-High-Close), 2 (Open-Low-High-Close), 3 (Volume-Low-High-Close), 4 (Volume-Open-Low-High-Close) |
| **Column and Line** | Columns and Lines, Stacked Columns and Lines (with the number of lines set) |

The internal UNO template services are `com.sun.star.chart2.template.*`, e.g. Column, StackedColumn, PercentStackedColumn, ThreeDColumnDeep, ThreeDColumnFlat, Pie, PieAllExploded, Donut, ThreeDPie, Line, LineSymbol, StackedLine, ScatterLine, ScatterSymbol, ScatterLineSymbol, ThreeDScatter, Net, NetLine, NetSymbol, FilledNet, StockLowHighClose, StockOpenLowHighClose, StockVolumeLowHighClose, StockVolumeOpenLowHighClose, ColumnWithLine, StackedColumnWithLine, Bubble, Surface (internal) and Addin.

**3D options:** 3D Look Simple or Realistic. Scene settings for perspective, rotation, shading, illumination with 8 light sources, and ambient light.

### Chart elements

- Main title, subtitle, and axis titles: X, Y, Z, plus secondary X and Y.
- Legend: Left, Right, Top or Bottom.
- Axes and secondary axes.
- Major and minor grids.
- Data labels: value, percent, category, legend key, separator, placement.
- Data table (in-chart data editor).
- Wall, floor, chart area and plot area formatting.
- Axis scale: min, max, major and minor interval, logarithmic, reverse direction, date axis.
- Axis positions: where axes cross, label placement, tick marks.

### Statistics

- **Trend lines (regression):** None, Linear, Logarithmic, Exponential, Power. Optional equation and R² display.
- **Mean value line.**
- **Error bars:**
  - Category: None, Constant Value, Percentage, Cell Range, or a statistical function: Standard Deviation, Variance, Error Margin [help: labels].
  - Indicator: Positive and Negative, Positive, or Negative. "Same value for both".

### Data and editing

- Chart data can come from a cell range or from the chart's internal data table.
- Charts update live from cell ranges.
- Charts are inserted as OLE objects. Edit mode is entered by double-click.

## 15. Goal Seek (Tools > Goal Seek)

Source: `sc/source/ui/src/solvrdlg.src`, `ocBackSolver` [src].

- Fields: **Formula cell**, **Target value**, **Variable cell**.
- Errors: "Invalid target value", "Undefined name for variable cell", "Undefined name as formula cell", "Cell must contain a formula".
- Result dialog: apply the result to the variable cell, or cancel.
- Uses an iterative root search (internal opcode `GOALSEEK`, `ScBackSolver` in `interpr`).

## 16. Solver (Tools > Solver)

Source: `sc/source/ui/src/optsolver.src`, `solveroptions.src` [src].

- **Target cell.** **Optimize result to:** Maximum, Minimum, or Value of.
- **By changing cells.**
- **Limiting conditions** (4 visible rows, scrollable): Cell reference, Operator, Value. The operators are `<=`, `=`, `>=`, **Integer** and **Binary**.
- **Options...** opens solver engine selection and settings. The bundled "OpenOffice.org Linear Solver" (lp_solve) offers [help]:
  - Assume variables as integer.
  - Assume variables as non-negative.
  - Epsilon level.
  - Limit branch-and-bound depth.
  - Solving time limit.
- Progress: "Solving in progress..." with a time limit in seconds.
- **Result:** "Solving successfully finished. Result: ..." with **Keep Result** or **Restore Previous**, or "No solution was found."

## 17. Scenarios (Tools > Scenarios)

Source: `sc/source/ui/dbgui/scendlg.src` [src].

- **Create Scenario:** Name of scenario, Comment ("Created by <user> on <date>").
- **Settings:**
  - Display border, with border color.
  - Copy back (write changes back into the scenario).
  - Copy entire sheet.
  - Prevent changes.
  - Print border [help].
- Scenarios live as hidden sheets after their base sheet. You switch between them with a drop-down in the scenario range, or from the Navigator.

## 18. Consolidate (Data > Consolidate)

Source: `sc/source/ui/dbgui/consdlg.src` [src].

- **Function:** Sum, Count, Average, Max, Min, Product, Count (numbers only), StDev (sample), StDevP (population), Var (sample), VarP (population).
- Consolidation ranges (Add and Delete), Source data range, Copy results to.
- **Options:** Consolidate by **Row labels** and/or **Column labels**. **Link to source data** inserts formulas and an outline.

## 19. Multiple operations (Data > Multiple Operations)

Source: `sc/source/ui/src/tabopdlg.src`, `ocTableOp` [src].

- Fields: **Formulas** (range), **Row input cell**, **Column input cell**.
- Generates `MULTIPLE.OPERATIONS(formula; colCell; colReplacement[; rowCell; rowReplacement])` formulas. This gives one-variable and two-variable data tables.
- Errors: "No formula specified", "Neither row or column specified", "Undefined name or range", "Formulas don't form a column/row".

## 20. Names and database ranges

- **Define Names** (Ctrl+F3 `.uno:DefineName`):
  - Name, and "Assigned to" (a range or any formula expression, so constants are allowed).
  - Options: Print range, Filter, Repeat column, Repeat row.
  - Names are document-global. Sheet-local names are not exposed in the AOO UI [help].
- **Insert Name** (paste a name or Insert All, which writes a list of names and ranges).
- **Create Names** from labels: Top row, Left column, Bottom row, Right column (`miscdlgs.src`) [src].
- **Labels** (column/row label ranges for natural-language formulas).
- **Database ranges** (Data > Define Range, `dbnamdlg.src`):
  - Name and range.
  - Options: Contains column labels, Insert or delete cells, Keep formatting, Don't save imported data.
  - Source (shown when the data was imported).
  - Operations (sort, filter and subtotal settings are stored per range).
- **Select Range** (Data > Select Range) and **Refresh Range**.
- **Data > Import from a registered data source:** drag from the data source browser (F4 `ViewDataSourceBrowser`).

## 21. External data and links

- **Insert > Link to External Data:** URL of an external source (HTML, CSV, ODS, XLS...), "Available tables/ranges" (HTML tables, named ranges, `HTML_all`, `HTML_tables`), and "Update every N seconds" [help].
- **Sheet links:** Insert > Sheet from file with Link. Sheet link options are Normal (values and formats) or Values only.
- **External cell references:** `='file:///path/doc.ods'#$Sheet1.A1` (Calc A1 syntax). The ODF form is `['file:///path/doc.ods'#$Sheet1.A1]`.
- **DDE links:** `DDE("soffice"; "c:\file.ods"; "Sheet1.A1"; mode)`, and Paste Special > Link.
- **Edit > Links** dialog: update, change source, break link. "Update links when opening" can be **Always**, **On request** or **Never** (`Calc.xcs Content/Update/Link=2`, on request) [src].
- **Web Query** import filter (`calc_HTML_WebQuery`) [src].

## 22. Hyperlinks, comments, detective

- **Hyperlinks:**
  - Insert > Hyperlink (as text or as a button) for Internet, Mail & News, Document and New Document targets.
  - The `HYPERLINK()` function.
  - Ctrl-click follows a link (configurable) [help].
- **Comments (notes)** (Ctrl+Alt+C `.uno:InsertAnnotation`) [src]:
  - Show/Hide comment.
  - Comment indicator, a small red square (`Content/Display/NoteTag`) [src].
  - Comments are printable: at the end of the sheet, or as displayed.
  - Author and date metadata.
- **Detective** (Tools > Detective):
  - Trace Precedents (Shift+F7) and Trace Dependents (Shift+F5) [src].
  - Remove Precedents, Remove Dependents, Remove All Traces.
  - Trace Error, Fill Mode, Mark Invalid Data, Refresh Traces, AutoRefresh.

## 23. Find & Replace, spelling, AutoCorrect

- **Find & Replace** (Ctrl+F `.uno:SearchDialog`; Ctrl+Shift+F repeats the search) [src]:
  - Search for, Replace with.
  - **Match case**, **Entire cells**.
  - Search in **Formulas**, **Values** or **Comments**.
  - Search direction **Rows** or **Columns**. **Search in all sheets**. **Current selection only**.
  - **Regular expressions** using ICU regex syntax, including `$` for end, `^` for start, and `&` / `$1` back-references in the replacement.
  - **Similarity search** (Levenshtein, with "exchange/add/remove characters" counts).
  - **Search for Styles**, which finds and replaces cell styles.
  - Find All selects all matches. Replace All.
- **Find toolbar** (Ctrl+Alt+F) [src].
- **Spellcheck** (F7) [src] and **AutoSpellcheck** (red wavy underline; document option `bDoAutoSpell` default false in `docoptio.cxx`) [src].
- **Thesaurus** (Ctrl+F7) [src], hyphenation, Hangul/Hanja conversion (Ctrl+Shift+F7) [src], Chinese conversion.
- **AutoCorrect** (Tools > AutoCorrect Options, shared with Writer):
  - Replace table.
  - Exceptions (abbreviations; words starting with TWo INitial CApitals).
  - Options: correct TWo INitial CApitals, capitalize the first letter of every sentence, automatic **bold** / _underline_, URL recognition, replace dashes, ignore double spaces, ordinal suffixes 1st.
  - Localized options: single and double quote replacement.
  - Word completion is Writer-only. Calc's equivalent is **AutoInput** (section 2).
- **Special-character and formula autocomplete:**
  - Typing `=` followed by letters shows a tooltip suggesting matching function names. Enter accepts the suggestion and inserts the opening parenthesis [help].
  - Function tips appear in the status bar and input line.

## 24. Protection

- **Tools > Protect Document > Sheet:**
  - Optional password.
  - Protected cells cannot be edited ("Protected cells can not be modified.", `STR_PROTECTIONERR`) [src].
  - Hidden formulas are not shown.
  - In AOO 4.x the only choice is the password; LibreOffice's "allow select locked/unlocked" options are not present [help].
- **Tools > Protect Document > Document:** protects the sheet structure (insert, delete, rename, move and hide sheets).
- **Cell attributes** (Format Cells > Cell Protection): Protected (default on), Hide formula, Hide all, Hide when printing.
- **File-level protection:** password to open (ODF encryption, Blowfish/SHA1 in AOO 4.x, configurable) and password to modify (filter flags `ENCRYPTION`, `PASSWORDTOMODIFY` on calc8, MS Excel 97) [src].
- Read-only document mode.
- **Record changes** (Edit > Changes): record, show, accept/reject, comment, protect records. Colors are set in `Calc.xcs Revision/Color/*` [src].
- Shared workbooks (collaborative editing with a lock file; `Misc/SharedDocument/ShowWarning`) [src].

## 25. Printing, page setup, headers and footers

- **Print ranges** (Format > Print Ranges): Define, Add, Remove, Edit. "Rows to repeat" and "Columns to repeat" (repeat rows and columns).
- **Page break preview** (View > Page Break Preview), with manual row and column breaks.
- **Page style dialog:**
  - **Page tab:** paper format, orientation, margins, layout (right and left, mirrored, only right, only left), page numbering format, table alignment (horizontal and vertical centering).
  - Borders and Background tabs.
  - **Header** and **Footer** tabs: on or off, same content left/right, margins, spacing, height, AutoFit height.
  - **Sheet tab:**
    - Page order: Top to bottom then right, or Left to right then down.
    - First page number.
    - Print: column and row headers, grid, comments, objects/graphics, charts, drawing objects, formulas, zero values.
    - Scaling mode: Reduce/enlarge printout (%), Fit print range(s) to width/height (N pages wide by M high), or Fit print range(s) to number of pages.
- **Header/Footer editor** has Left, Center and Right areas.
  - Fields: **Sheet name**, **Page**, **Pages**, **Date**, **Time**, **Title** (file name), **File name** and **Path**.
  - Predefined headers and footers ("Page 1", "Sheet1", "Confidential", "Created by..." and others).
  - Text attributes can be set per area [help].
- **Options** (Tools > Options > Calc > Print): Suppress output of empty pages (`Print/Page/EmptyPages=false`), Print only selected sheets (`Print/Other/AllSheets=false`) [src].
- **PDF export** (`calc_pdf_Export`) [src].

## 26. Keyboard shortcuts (standard table)

Sources: `officecfg/registry/data/org/openoffice/Office/Accelerators.xcu` (PrimaryKeys, Global and `com.sun.star.sheet.SpreadsheetDocument`, en-US), plus keys hard-coded in `sc/source/ui/view/tabvwsh4.cxx` [src].

- MOD1 is Ctrl (Cmd on macOS), MOD2 is Alt (Option on macOS), and MOD3 is Ctrl on macOS.
- The table lists the en-US bindings. Some locales (de, es) remap a few letter shortcuts.

| Key | Command | Key | Command |
|---|---|---|---|
| Ctrl+A | Select all | Ctrl+B / I / U | Bold / Italic / Underline |
| Ctrl+C / X / V | Copy / Cut / Paste | Ctrl+Shift+V | Paste Special |
| Ctrl+Z | Undo | Ctrl+Y, Ctrl+Shift+Z | Redo |
| Ctrl+Shift+Y | Repeat last command | Ctrl+F / Ctrl+Shift+F | Find & Replace / Repeat search |
| Ctrl+Alt+F | Focus the Find toolbar | Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Shift+S | New / Open / Save / Save As |
| Ctrl+P | Print | Ctrl+W / Ctrl+Q | Close window / Quit |
| Ctrl+E / L / R / J | Center / Left / Right / Justify | Ctrl+M | Default formatting (clear direct formatting) |
| Ctrl+1 | Format Cells dialog | Ctrl+Shift+1 | Number format: two decimals (`NumberFormatDecimal`) |
| Ctrl+Shift+2 | Scientific | Ctrl+Shift+3 | Date |
| Ctrl+Shift+4 | Currency | Ctrl+Shift+5 | Percent |
| Ctrl+Shift+6 | Standard (General) | Ctrl+2 / Ctrl+5 | Line spacing 2 / 1.5 (text edit) |
| Ctrl+Shift+B / Ctrl+Shift+P | Subscript / Superscript (text edit) | Ctrl+D | Selection list (`DataSelect`) |
| Ctrl++ (numpad) / Ctrl+- (numpad) | Insert cells / Delete cells | Ctrl+* (numpad) | Select data area |
| Ctrl+/ (numpad) | Select array formula | Ctrl+` (grave) | Show formulas (`ToggleFormula`) |
| F2 | Edit mode | Ctrl+F2 | Function Wizard |
| Ctrl+Shift+F2 | Focus input line | Ctrl+F3 | Define Names |
| F4 | Data source browser (sheet only; `.uno:ViewDataSourceBrowser`) | Shift+F4 | Cycle relative/absolute references while editing (`ToggleRelative`) |
| F5 | Navigator | Shift+F5 | Trace Dependents |
| Shift+F7 | Trace Precedents | F7 / Ctrl+F7 | Spelling / Thesaurus |
| F8 | Extend-selection mode | Shift+F8 | Add-selection mode |
| Ctrl+F8 | Value highlighting | F9 | Recalculate |
| Ctrl+Shift+F9 | Hard recalculation (`CalculateHard`) | Ctrl+F11 | Styles and Formatting (`StyleCatalog`) |
| Shift+F11 | Save as Template | Alt+F11 | Macro dialog (Global) |
| F12 / Ctrl+F12 | Group / Ungroup | Ctrl+Shift+J | Full screen |
| Ctrl+Shift+R | Repaint (redraw) | Ctrl+Shift+T | Focus Name Box |
| Ctrl+Alt+C | Insert comment | Ctrl+F5 (Global) | Toggle control focus |
| Arrows | Move (Shift extends the selection) | Ctrl+arrows | Edge of data block (Shift extends) |
| Home / End | Start / end of row (Shift extends) | Ctrl+Home / Ctrl+End | A1 / last used cell (Shift extends) |
| PageUp / PageDown | One screen up / down (Shift extends) | Ctrl+PageUp / Ctrl+PageDown | Previous / next sheet (Shift extends the sheet selection) |
| Alt+PageUp / Alt+PageDown | One screen left / right (hard-coded) | Tab / Shift+Tab | Next / previous unprotected cell |
| Shift+Space | Select row | Ctrl+Shift+Space | Select column |
| Backspace | Clear contents (no dialog) | Delete | Delete Contents dialog |
| Shift+Backspace | Deselect | Ctrl+Backspace | Scroll back to the cursor |
| Insert | Paste-contents / insert-mode command (`InsertContents`) | Esc | Cancel |
| Enter / Shift+Enter | Commit and move down / up | Alt+Enter | Commit into the whole selection |
| Ctrl+Shift+Enter | Commit as array formula | Ctrl+Enter | Line break inside the cell |
| Alt+arrows | Change row height or column width (Shift sets optimal) | Ctrl+Alt+Shift+arrows | Move the cursor while keeping the selection |

Ctrl+; (insert current date) and Ctrl+Shift+; (insert current time) are **not** bound in the Calc accelerator config. Whether they work through code depends on the build. Verify both in a running AOO 4.1.x [help].

## 27. Formula syntax (as implemented)

Sources: `formula/source/core/api/FormulaCompiler.cxx`, `formula/inc/formula/grammar.hxx`, `formula/source/core/resource/core_resource.src`, `sc/source/core/tool/compiler.cxx`, `sc/source/core/tool/address.cxx` [src].

### 27.1 Grammars (`formula::FormulaGrammar`)

- **GRAM_ODFF** (OpenFormula, ODF 1.2): used in `.ods` files.
- **GRAM_PODF** (pre-ODFF ODF 1.0/1.1 "of:" syntax).
- **GRAM_ENGLISH** (API English names, Calc A1).
- **GRAM_NATIVE** (UI-localized names). The UI default is `GRAM_NATIVE_UI`.
- Reference conventions:
  - `CONV_OOO`: Calc A1, `Sheet.A1`.
  - `CONV_ODF`: `[.A1]`, `[$Sheet1.A1:.B2]`.
  - `CONV_XL_A1`: Excel A1, `Sheet1!A1`.
  - `CONV_XL_R1C1`: Excel R1C1, `R[1]C[-1]`.
- The user selects one of Calc A1, Excel A1 or Excel R1C1. In AOO 4.x this is under Tools > Options > Calc > Formula [help: dialog location].

### 27.2 References

| Syntax | Meaning |
|---|---|
| `A1`, `$A$1`, `$A1`, `A$1` | Relative or absolute column and row. Shift+F4 cycles through the forms |
| `Sheet1.A1`, `$Sheet1.A1`, `'My Sheet'.A1` | Sheet-qualified reference (Calc A1). `$` makes the sheet absolute. Excel syntax is `Sheet1!A1` |
| `A1:B2`, `Sheet1.A1:Sheet3.B2` | Range, including 3D ranges across sheets |
| `A:A`, `1:1` | Whole column or row. **Only in Excel A1 syntax** (`lcl_ScRange_Parse_XL_A1`); not parsed in Calc A1 syntax in AOO |
| `'file:///C:/x.ods'#$Sheet1.A1` | External reference |
| `[.A1]`, `[.A1:.B2]`, `[$Sheet1.A1]` | ODFF reference syntax (in files) |
| Named range / label | `=SUM(MyRange)`. Natural-language labels when "Automatically find column and row labels" is on |
| `#REF!` | Deleted reference |

### 27.3 Operators and precedence

From `FormulaCompiler::Expression` and its `*Line` functions; the first row binds loosest.

| Level | Operators | Notes |
|---|---|---|
| 1 | `AND`, `OR` as infix (internal) | `Expression` loop; the UI uses the functions |
| 2 | `NOT` (internal) | `NotLine` |
| 3 | `=` `<>` `<` `>` `<=` `>=` | `CompareLine` |
| 4 | `&` | String concatenation (`ConcatLine`) |
| 5 | `+` `-` | `AddSubLine` |
| 6 | `*` `/` | `MulDivLine` |
| 7 | `^` | `PowLine`, left-associative: `2^3^2` = 64 |
| 8 | `%` (postfix) | `PostOpLine`; `50%` = 0.5 |
| 9 | unary `-`, unary `+` | `UnaryLine`. Unary minus binds **tighter than `^`**, so **`=-2^2` = 4** |
| 10 | `~` | **Reference union** (`UnionLine`); e.g. `SUM(A1:A3~C1:C3)` |
| 11 | `!` | **Reference intersection** in Calc syntax (`IntersectionLine`). Excel uses a space |
| 12 | `:` | Range (`RangeLine`) |

- **Separators** (en-US Calc UI, `core_resource.src`):
  - Function arguments: `;`. Calc also accepts `,` in en-US, depending on the "separators" option [help].
  - Inline array column separator: `;`. Row separator: `|`.
  - Array delimiters `{` `}`.
  - ODFF uses `;` for arguments, `;` for array columns and `|` for array rows.
- **String literals** are written as `"text"`. A quote inside a string is doubled: `"say ""hi"""`.
- **Booleans:** `TRUE` and `FALSE` are zero-argument functions (`TRUE()`, `FALSE()`). Logical values are numbers 1 and 0 carrying a boolean number format.
- **Empty (missing) arguments** are allowed, e.g. `=ADDRESS(1;1;;0)`. The missing argument is passed to the interpreter as a missing token.
- **Inline arrays:** `={1;2;3|4;5;6}` is a 2x3 matrix. Nested arrays give Err:533 ("Nested arrays are not supported.").
- **Array formulas:**
  - Entered with Ctrl+Shift+Enter, or the "Array" checkbox in the Function Wizard.
  - Displayed as `{=formula}` over the whole result range.
  - The range can only be edited as a whole.
  - `MVALUE` is the internal matrix-value opcode.
- **Implicit intersection:** when a range is passed to a scalar parameter outside array context, the value in the same row (or column) as the formula cell is used. Otherwise the result is #VALUE! (`DoubleRefToPosSingleRef`).
- **Parameter classes** (`parclass.cxx`) define per-parameter Value, Reference, Array, ForceArray or ReferenceOrForceArray conversion. These are listed in `calc-functions.json` (`parameterClasses`).
- **Internal opcodes** that are not user functions: `ocBackSolver` (GOALSEEK), `ocMultiArea` (MULTIRANGE), `ocMatValue` (MVALUE), `ocErrCell`, `ocIndirectXL` and `ocAddressXL` (Excel-syntax variants of INDIRECT and ADDRESS chosen by grammar), `ocExternal` (add-in calls), `ocMacro` (Basic function calls, which display #MACRO? when not found).
- **Basic macros as functions:** `=MYBASICFUNC(args)` calls a StarBasic function in the document or in My Macros.

### 27.4 Error values

From `formula/inc/formula/errorcodes.hxx` (codes) and `sc/source/core/data/global.cxx` (`ScGlobal::GetErrorString` shows the cell text; `GetLongErrorString` shows the status bar text from `globstr.src`) [src].

| Code | Constant | Cell display | Status-bar text / meaning |
|---|---|---|---|
| 501 | errIllegalChar | Err:501 | Error: Invalid character |
| 502 | errIllegalArgument | Err:502 | Error: Invalid argument (e.g. SQRT(-1) in some functions, out-of-domain argument) |
| 503 | errIllegalFPOperation | **#NUM!** | Error: Invalid numeric value (overflow, infinity, domain error) |
| 504 | errIllegalParameter | Err:504 | Error in parameter list (wrong type or parameter count) |
| 505 | errIllegalJump | Err:505 | Internal syntactical error |
| 506 | errSeparator | Err:506 | Error: Invalid semicolon |
| 507 | errPair | Err:507 | Error: in bracketing (missing pair) |
| 508 | errPairExpected | Err:508 | Error: in bracketing (missing closing parenthesis) |
| 509 | errOperatorExpected | Err:509 | Error: Operator missing |
| 510 | errVariableExpected | Err:510 | Error: Variable missing |
| 511 | errParameterExpected | Err:511 | Error: Variable missing (too few parameters) |
| 512 | errCodeOverflow | Err:512 | Error: Formula overflow (more than 512 tokens) |
| 513 | errStringOverflow | Err:513 | Error: String overflow (identifier longer than 1024) |
| 514 | errStackOverflow | Err:514 | Error: Internal overflow (nesting or stack) |
| 515 | errUnknownState | Err:515 | Internal syntactical error |
| 516 | errUnknownVariable | Err:516 | Internal syntactical error |
| 517 | errUnknownOpCode | Err:517 | Internal syntactical error |
| 518 | errUnknownStackVariable | Err:518 | Internal syntactical error |
| 519 | errNoValue | **#VALUE!** | Error: Wrong data type |
| 520 | errUnknownToken | Err:520 | Internal syntactical error |
| 521 | errNoCode | **#NULL!** | Internal syntactical error (empty intersection / no code) |
| 522 | errCircularReference | Err:522 | Error: Circular reference (when iteration is off) |
| 523 | errNoConvergence | Err:523 | Error: Calculation does not converge (iteration or goal seek / iterative functions) |
| 524 | errNoRef | **#REF!** | Error: Not a valid reference |
| 525 | errNoName | **#NAME?** | Error: Invalid name |
| 526 | errDoubleRef | #VALUE! | Internal syntactical error (range where a single value was required) |
| 527 | errInterpOverflow | Err:527 | Error: Internal overflow (interpreter recursion too deep) |
| 528 | errTrackFromCircRef | (not displayed) | Internal: cell depends on a circular reference |
| 529 | errCellNoValue | (internal) | Interpreter internal: no numeric value |
| 530 | errNoAddin | **#ADDIN?** | Error: Add-in not found |
| 531 | errNoMacro | **#MACRO?** | Error: Macro not found |
| 532 | errDivisionByZero | **#DIV/0!** | Error: Division by zero |
| 533 | errNestedArray | Err:533 | Nested arrays are not supported. |
| 534 | errNotNumericString | (internal) | Interpreter internal; never propagated |
| 0x7FFF (32767) | NOTAVAILABLE | **#N/A** | Error: Value not available |

- Errors are carried inside NaN doubles: the error code sits in the low 16 bits of the mantissa (`CreateDoubleError` and `GetDoubleErrorValue`). A plain NaN maps to 519 and Inf maps to 503.
- `ERRORTYPE(ref)` returns the raw code, e.g. 502 or 32767.
- Error **literals** recognized by the compiler: `#NULL!`, `#DIV/0!`, `#VALUE!`, `#REF!`, `#NAME?`, `#NUM!`, `#N/A`.
- `ISERR` is TRUE for any error except #N/A. `ISERROR` is TRUE for any error. `ISNA` is TRUE for #N/A only.

### 27.5 Calculation settings (Tools > Options > Calc > Calculate)

Sources: `docoptio.cxx`, `optdlg.src`, `Calc.xcs` [src].

| Setting | Default | Notes |
|---|---|---|
| Iterative references: Iterations | off | Steps **100**, Minimum Change **0.001**. With iteration off, a circular reference gives Err:522. With iteration on, the loop stops when the change is below the minimum or the steps run out; if it has not converged the result is Err:523 |
| Date (null date) | **12/30/1899** | Choices: 12/30/1899 (default), 01/01/1900 (StarCalc 1.0), 01/01/1904. Serial 0 is the chosen date. Note: **no 1900 leap-year bug** with the default; serial 60 = 1900-02-28, serial 61 = 1900-03-01 [help: check against Excel interop] |
| Case sensitive | **on** (`CaseSensitive=true`; `bIsIgnoreCase=false`) | Affects text comparisons in formulas, lookups and criteria |
| Precision as shown | off | When on, values are rounded to their displayed format before use |
| Search criteria = and <> must apply to whole cells | **on** | When off, criteria match substrings |
| Enable regular expressions in formulas | **on** | Applies to MATCH, LOOKUP, xLOOKUP, COUNTIF, SUMIF, D* functions and similar |
| Automatically find column and row labels | **on** | |
| Limit decimals for general number format | off (-1, unlimited) | |

- **AutoCalculate:** Tools > Cell Contents > AutoCalculate, default on. F9 recalculates. Ctrl+Shift+F9 does a hard recalculation of all formulas.
- **Recalculation modes** (`FormulaCompiler.cxx`):
  - **Always** (volatile): RAND, TODAY, NOW, FORMULA, INFO, INDIRECT, OFFSET. Also any formula that uses a function flagged volatile.
  - **On load:** CONVERT (the euro table).
  - **On reference move:** COLUMN, ROW, CELL.
  - HYPERLINK marks the cell as a hyperlink cell.
- **Status bar function:** the result for the selection is shown in the status bar. The default is Sum (`Layout/Other/StatusbarFunction = 9`). The choices are Average, CountA, Count, Max, Min, Sum, None [help: list].

## 28. View features

- **Freeze panes** (Window > Freeze), at the cursor row and/or column. **Split window** (Window > Split, or drag the split handles).
- **Zoom:** 20%-400%, Optimal, Whole page, Page width, Variable. "Synchronize sheets" (`Layout/Zoom/Synchronize=true`) [src].
- **View options** (Tools > Options > Calc > View; `Calc.xcs`) [src]:
  - Grid lines and their color: default 12632256 = #C0C0C0.
  - Page breaks, Guides while moving, Large handles.
  - Formulas instead of results (Ctrl+`), Zero values, Comment indicator, Value highlighting, Anchor, Text overflow (the triangle shown when text is clipped), Show references in color.
  - Objects, Charts and Drawing objects: each can be Show, Hide or Placeholder.
  - Column/row headers, Horizontal and Vertical scroll bars, Sheet tabs, Outline symbols.
- **Full screen** (Ctrl+Shift+J). **Normal view** versus **Page Break Preview**.
- **Units:** Millimeter, Centimeter, Meter, Kilometer, Inch, Foot, Miles, Pica, Point. Tab stops default to 1.25 cm or 0.5 in.

## 29. Undo / Redo

- Multi-level undo and redo with a history list on the toolbar drop-down. Depth defaults to **100** and can be set from 1 to 1000 (`Office.Common/Undo/Steps`).
- Repeat last command: Ctrl+Shift+Y.
- Undo covers cell edits, formatting, insert/delete, sort, filter, subtotals, DataPilot, sheet operations, names, outline and merge. Each has an `STR_UNDO_*` string in `globstr.src` [src].

## 30. Import/export formats

Source: `filter/source/config/fragments/filters/*.xcu`, the entries whose `DocumentService` is `com.sun.star.sheet.SpreadsheetDocument` [src].

| Filter (internal name) | Format | Import | Export | Notes |
|---|---|---|---|---|
| calc8 | **ODF Spreadsheet .ods** (default) | yes | yes | DEFAULT, password encryption, password to modify |
| calc8_template | ODF Spreadsheet Template .ots | yes | yes | |
| StarOffice XML (Calc) | **.sxc** (OOo 1.x) | yes | yes | |
| calc_StarOffice_XML_Calc_Template | .stc | yes | yes | |
| MS Excel 97 | **.xls** (BIFF8, Excel 97-2003) | yes | yes | encryption, password to modify |
| MS Excel 97 Vorlage/Template | .xlt | yes | yes | |
| MS Excel 95 / 5.0/95 (+ templates) | .xls BIFF5/7 | yes | yes | |
| MS Excel 4.0 (+ template) | .xls BIFF4 | yes | no | |
| Calc MS Excel 2007 XML | **.xlsx** (OOXML) | yes | **no** | AOO 4.x imports OOXML only |
| Calc MS Excel 2007 XML Template | .xltx | yes | no | |
| Calc MS Excel 2007 Binary | .xlsb | yes | no | |
| MS Excel 2003 XML | .xml (SpreadsheetML 2003) | yes | yes | XSLT filter |
| Text - txt - csv (StarCalc) | **CSV / TXT** | yes | yes | Options dialog, see below |
| HTML (StarCalc) | .html / .htm | yes | yes | |
| calc_HTML_WebQuery | HTML web query | yes | no | |
| XHTML Calc File | .xhtml | no | yes | XSLT |
| dBase | **.dbf** | yes | yes | Character set option |
| DIF | .dif | yes | yes | |
| SYLK | .slk | yes | yes | |
| Lotus | .wk1/.wks/.123 | yes | no | `Filter/Import/Lotus123/WK3=false` |
| Quattro Pro 6.0 | .wb2 | yes | no | |
| Rich Text Format (StarCalc) | .rtf | yes | no | |
| StarCalc 1.0/3.0/4.0/5.0 (+ templates) | .sdc / .vor | yes | no | |
| UOF spreadsheet / NSO Calc UOF2 | .uos / .uof | yes | yes | Chinese UOF standard |
| Pocket Excel | .pxl | yes | yes | |
| MiniCalc (Palm) | .pdb | yes | yes | |
| calc_pdf_Export | **PDF** | no | yes | PDF options dialog |

### CSV / Text import options (`asciiopt.src`, `Calc.xcs Dialogs/CSVImport`) [src]

- Character set (default -1, meaning system), Language, From row (1).
- **Separator options:**
  - **Fixed width**, or **Separated by**: Tab, Comma, Semicolon, Space, Other.
  - Default separators string: `;` plus Tab.
  - **Merge delimiters** (off).
  - **Text delimiter** (`"`).
- **Other options:** **Quoted field as text** (off), **Detect special numbers** (on; dates, scientific notation and similar).
- **Fields:** per-column **Column type**: Standard, Text, Date (DMY/MDY/YMD), US English, Hide [help: list].

### CSV / dBase export options (`imoptdlg.src`) [src]

- Character set.
- Field delimiter, Text delimiter.
- **Save cell content as shown** (formatted text instead of raw values).
- **Quote all text cells**.
- **Fixed column width**.

### Excel import settings

- VBA import: Load as comments = true, Executable = false, Save original VBA storage = true (`Filter/Import/VBA/*`).
- BIFF5 column and row scale factor: 1.0.

### Excel function mapping

- Built-ins map to the same names, except where the ODF name differs.
- Analysis add-in functions map to the Excel ATP names via `odffmap.cxx` and the compatibility names in `analysis_deffuncnames.src`.

## 31. Other Calc tools

- **Function Wizard** (Ctrl+F2): categories, a "Last Used" list (default opcodes 224 226 222 223 6 = SUM, AVERAGE, MIN, MAX, IF), a formula structure tree, and per-argument help text from `scfuncs.src` [src].
- **Navigator**, **Data source browser** (F4), **Gallery**.
- **Insert > Function List** (sidebar list of functions).
- **Insert > Special Character**, **Insert > Image / Chart / Object / Floating Frame / Form controls**. Drawing objects can be anchored to the page or to a cell.
- **Form controls** (buttons, list boxes) and **Form Design mode**, with cell links and source-range binding.
- **Macros:** StarBasic, plus BeanShell, JavaScript and Python through the scripting framework. Macro dialog (Alt+F11). Document events.
- **Tools > Options > Calc > Sort Lists:** default lists are the locale's day and month names, abbreviated and full (`ScUserList` constructor). Lists can be added, deleted and copied from a cell range ("Copy list from").
- **Change tracking** (Edit > Changes > Record / Show / Accept or Reject / Comments / Merge Document / Compare Document).
- **Document statistics** (File > Properties > Statistics: sheets, cells, pages; `tpstat.src`) [src].
- **Hyperlink bar**, **Formula bar** (input line with Sum and Function buttons), **status bar** (selection mode, zoom, page, document modified, digital signature, status function).
- **Euro Converter wizard**, an AutoPilot that is separate from the EUROCONVERT function [help].
- **Text to Columns**, **Data > Validity**, **Data > Group and Outline**, **Data > Refresh Range**.

---

## Not determined from source (open items)

- **Exact layout of the Format Cells > Numbers dialog and the full built-in format list.** These are generated by `SvNumberFormatter` from locale data (i18npool locale XML), which was not checked out.
- **The en-US separator option.** AOO 4.x accepts `;` and `,` according to a "Separators" setting; LibreOffice moved this to the Formula options page. The defaults come from locale data.
- **Exact chart error-bar category list labels** (standard deviation, variance, error margin). These live in a `StringList` that was not read.
- **Solver option names and defaults.** They come from the solver component in `main/sccomp`, which was not checked out.
- **Help-documented behaviors** marked [help] above, including Ctrl+; for the date, DataPilot layout options and header/footer presets.
