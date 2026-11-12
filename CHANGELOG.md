# Changelog

What changed in each Tally release, for the people who use it. The
section of a release is its GitHub Release text and what the app shows in
its update prompt (docs/release.md), so write it as plain text: Markdown
symbols are shown as typed.

Sections are `## [x.y.z] - YYYY-MM-DD`, newest first. Before tagging,
rename `[Unreleased]` to the version and start a new empty `[Unreleased]`.

## [Unreleased]

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
