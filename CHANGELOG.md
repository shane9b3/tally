# Changelog

What changed in each Tally release, for the people who use it. The
section of a release is its GitHub Release text and what the app shows in
its update prompt (docs/release.md), so write it as plain text: Markdown
symbols are shown as typed.

Sections are `## [x.y.z] - YYYY-MM-DD`, newest first. Before tagging,
rename `[Unreleased]` to the version and start a new empty `[Unreleased]`.

## [Unreleased]

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

### Changed
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
  rules of the selected cells, as in OpenOffice.
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

### Fixed
- Copying a sheet no longer creates a second database range with the same
  name.

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
