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
