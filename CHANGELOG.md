# Changelog

What changed in each Tally release, for the people who use it. The
section of a release is its GitHub Release text and what the app shows in
its update prompt (docs/release.md), so write it as plain text: Markdown
symbols are shown as typed.

Sections are `## [x.y.z] - YYYY-MM-DD`, newest first. Before tagging,
rename `[Unreleased]` to the version and start a new empty `[Unreleased]`.

## [Unreleased]

### Added
- Excel 97-2003 (.xls) files open with their named ranges (Insert, Names,
  Define): names of the whole workbook and names that belong to one
  sheet, so formulas that use them calculate instead of showing #NAME?.
  A name whose formula uses a function is not read yet; Tally says how
  many were left out when it opens the file.
- Excel 97-2003 files open with their hyperlinks: web and e-mail
  addresses, links to other files and links to a cell or a name in the
  same workbook.

### Changed
- Clear formats (⌘M) also removes the conditional formatting and the
  validity rules of the selected cells, as Default Formatting does in
  OpenOffice. Merged cells stay merged.

### Fixed
- On a protected sheet, moving unlocked cells to another sheet with Cut
  and Paste no longer removes their conditional formatting or validity
  rules. The move is refused instead, as Delete Contents with Formats
  already was; Clear formats is refused there in the same case.
- Windows: the menu bar (File, Edit, Insert, Format, Data, View, Sheet,
  Tools, Window, Help) was missing. It is now at the top of the window,
  next to the Tally logo, with the keyboard shortcuts shown beside the
  items. Press Alt or F10 to reach it from the keyboard, then use the
  arrow keys, Enter and Esc.
- Excel 97-2003 (`.xls`) files with small defects now open instead of
  being reported as malformed: a defined name or a formula that is cut
  short (the cell keeps its last calculated value, and a message says
  what was left out), a few optional records that are too short, and a
  workbook stream whose name is written in another case.

### Security
- `.xls` import: the checks that run before the file is decoded were
  re-audited against the decoder in use. Two kinds of crafted file that
  could make opening take minutes or use far more memory than the limits
  allow (Excel 5.0/95 formulas with padded strings; shared-string tables
  with empty continuation records) are now handled or refused at once.
  A workbook whose sheets overlap each other or the workbook's own
  header records, which could be used to get a sheet past those checks,
  is refused.

## [0.1.3] - 2026-11-17

### Added
- DataPilot field options: the Layout choices (outline layout with
  subtotals at the top or at the bottom) and "Empty line after each item"
  now change the table, as in OpenOffice. They apply to row fields that
  have another row field (or the Data field) inside them; the dialog
  greys them out elsewhere. They are saved in ODS and XLSX files.
- Status bar: choose what is calculated for the selection. Click (or
  right-click) the "Sum=" field and pick Average, CountA, Count, Maximum,
  Minimum, Sum or None, as in OpenOffice. The choice is remembered.
- Status bar: a STD / EXT / ADD field shows the selection mode (F8 and
  Shift+F8); clicking it switches to the next mode.
- Excel 97-2003 (.xls) files open with their layout: column widths, row
  heights, hidden rows and columns, fonts, text and background colours,
  borders, alignment, text wrapping and rotation, number formats (dates,
  currency, percent and custom formats), cell protection, frozen panes,
  hidden grid lines and the other view settings, sheet tab colours,
  pictures (PNG and JPEG) and cell notes. Until now only the values,
  formulas and merged cells were read. Files from Excel 5.0 / 95 still
  open with their values only, and Tally says so when it opens one.
- Excel 97-2003 files keep their page setup: paper size, orientation,
  scale or "fit to pages", margins, header and footer, centring, printed
  grid lines and headers, manual page breaks, the print area and the rows
  and columns to repeat.
- Excel 97-2003 files are laid out as in OpenOffice: every row has the
  height stored in the file, column widths use OpenOffice's conversion,
  rows and columns hidden through a height or width of 0 or a hidden
  default are hidden, and formatting applied to a whole row or column
  shows on its empty cells.

### Changed
- Excel workbooks (.xlsx) open with the row heights and column widths of
  the file: a sheet from Excel has its 20 px rows and
  64 px columns instead of Tally's 24 px and 100 px, and rows that Excel
  fitted to their content keep that height. As with .xls files, such rows
  have a set height and do not grow when their content changes (Format >
  Row > Optimal Height fits them).
- Workbooks made in Tally and saved as .xlsx now say Tally's own row
  height and column width, so they open in Excel laid out as in Tally,
  and reopen in Tally as they were. A sheet from Excel keeps the file's
  row height and column width when it is saved as .xlsx again, also
  after rows or columns were inserted or deleted, and rows hidden to the
  end of the sheet stay hidden. An .xlsx saved by Tally 0.1.2 or
  older opens with Excel's row height and column width, which is how
  Excel shows that file.
- The local clock (NOW, TODAY, the lock-file time stamp) is read with
  `jiff`; `chrono` is flagged unmaintained (RUSTSEC-2026-0543) and is no
  longer a dependency.
- DataPilot totals with several data fields are labelled as in OpenOffice:
  "East Sum - Revenue" and "Total Sum - Revenue" instead of "East Result"
  with the data field in the next column. Function names in DataPilot
  captions are the short ones ("Count - Revenue", "StDev - Revenue").
  GETPIVOTDATA formulas that use the old names keep working.
- XLSX files with a pivot table that Tally 0.1.x saved now open with the
  pivot set to outline layout (subtotals at the top), because those files
  do not say which layout they use (OpenOffice reads them the same way),
  and with more than one row field the open reports that the compact
  form is shown as outline layout.
  The cells on the sheet keep the old layout until you refresh the
  pivot table; to go back, choose Tabular layout in the field options.
- VALUE("TRUE") is 1 and VALUE("FALSE") is 0, as in OpenOffice (the words
  of the document language). Text that VALUE or DATEVALUE cannot convert
  now gives Err:502 instead of #VALUE!, as in OpenOffice.
- Delete Contents with Formats checked also removes merged cells that
  start in the selection, and the conditional formatting and validity
  rules of the selected cells, as in OpenOffice. On a protected sheet it
  is refused when it would remove any of these, as Merge Cells,
  Conditional Formatting and Validity are there.
- Double-clicking a DataPilot value puts the details on a sheet with the
  next default name (Sheet2, Sheet3, ...) in front of the DataPilot's
  sheet, as in OpenOffice. It was named after the table and placed behind.
- Consolidate by labels leaves out rows and columns whose label is empty
  and keeps the top-left label when all source ranges share it.
- Text to Columns treats a quote as the start of a quoted field only when
  it is the first character of the field; a quote in the middle of a field
  is kept as typed.
- Standard filter: a condition such as "Begins with" with an empty value
  matches no row, and "Does not begin with" every row, as in OpenOffice.
- Scenarios: on a protected sheet, the cells of the scenario being shown
  cannot be edited when it has both "Prevent changes" and "Copy back", and
  a scenario with "Prevent changes" cannot be changed or deleted.
- The status bar shows one result for the selection ("Sum=11") instead of
  Sum and Count side by side, and leaves out cells in hidden or filtered
  rows and hidden columns, as in OpenOffice.
- Deleting a cell style: cells that used it now get the Default style and
  keep their own formatting, as in OpenOffice. Before, they took the
  deleted style's parent. Styles based on the deleted style move to its
  parent.
- Printing notes (Page Setup, Sheet tab): the notes pages now come after
  each sheet's own pages, appear in the print preview, count in the page
  total and carry the header and footer, with "Comments" in place of the
  sheet name, as in OpenOffice. Each note is listed as "A1: text".
- A note outside the data no longer widens the printed area unless notes
  are printed.

### Fixed
- Copying a sheet no longer creates a second database range with the same
  name.
- ODS: a cell style based on another style, or a cell formatted on top of
  a named style, lost what it had switched off: after saving and
  reopening, the parent's bold, background, borders, alignment or number
  format came back. Styles are now saved as their difference from the
  parent, so OpenOffice and Tally both read them back as they were, and
  an edit to the parent style in OpenOffice reaches its children. The
  built-in styles Heading, Heading1, Result and Result2 are saved with
  the cells and styles that use them.
- Printing: text without a font size of its own prints at 11 pt, the size
  it has on screen. It printed at 10 pt.
- Zoom to Page Width and Whole Page no longer take a notes page for the
  first page of the sheet.
- Printing with "Formulas" on and "Zero values" off no longer hides
  formulas whose result is zero.
- Headers and footers: the "Path/File Name" field of an OpenOffice or
  Excel file is kept (as `&[PATH]`) instead of turning into the file name
  or disappearing.
- Excel 97-2003 (.xls) files whose first row or first column is empty
  opened with every cell shifted up and to the left, so texts ended up
  under the wrong merged cells or disappeared and formulas pointed at
  the wrong cells. Cells are now where the file puts them.

## [0.1.2] - 2026-11-12

### Added
- Page Setup, Sheet tab: choose whether zero values, formulas, notes, and
  charts and pictures are printed. These choices are saved in ODS files.

### Changed
- Entering data across a row: after moving with Tab, Enter now goes to the
  next row in the column where you started, instead of straight down.
  Clicking another cell or using the arrow keys starts over.

## [0.1.1] - 2026-11-12

### Changed
- No changes to the spreadsheet itself. This is the first release
  delivered through the in-app updater.

## [0.1.0] - 2026-11-12

First public build of Tally, a spreadsheet compatible with OpenOffice Calc
files (ODS, XLSX, XLS, CSV).

### Added
- Automatic updates: Tally checks GitHub for a new version once a day and
  asks before installing it (Tools ▸ Options ▸ Online Update to turn the
  check off; Help ▸ Check for Updates… to check now, in the Tally menu on
  macOS). Unsaved documents are asked about before Tally restarts.
