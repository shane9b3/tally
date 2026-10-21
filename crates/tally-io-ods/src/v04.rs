//! Wave 5 ODS placeholders (W5-0a): the ordered write hooks for charts,
//! pictures, DataPilot tables, outline groups, scenarios and named
//! database ranges. Every function here currently emits nothing; W5-1
//! (charts, pictures) and W5-2 (pivots, outline, scenarios, database
//! ranges) fill them in without touching the call sites in `write.rs`.
//!
//! Schema order the call sites follow (`office:spreadsheet` children):
//! `table:calculation-settings`, `table:content-validations`,
//! `table:table`*, `table:named-expressions`, `table:database-ranges`,
//! `table:data-pilot-tables`. Inside `table:table`: `table:scenario`,
//! `table:shapes`, column groups/columns, row groups/rows (frames inside
//! cells), `table:named-expressions`, `calcext:conditional-formats`.

use std::io::{Seek, Write};

use tally_model::{OutlineGroupData, SheetData, WorkbookData};
use zip::ZipWriter;

use crate::{OdsError, WriteReport};

/// Writes the `table:scenario` element of a scenario sheet.
///
/// TODO(W5-2): the writer emits one hidden sheet per
/// `SheetData::scenarios` entry after its base sheet; that sheet gets
/// `<table:scenario table:scenario-ranges=… table:is-active=…
/// table:comment=… table:border-color=… table:display-border=…
/// table:copy-back=… table:copy-styles=… table:protected=…/>`.
pub(crate) fn write_scenario(s: &mut String, sheet: &SheetData) {
    crate::tools::write_scenario(s, sheet);
}

/// Writes `table:shapes` (page-anchored frames).
///
/// TODO(W5-1): nothing in practice, since Tally anchors charts and
/// pictures to cells (`draw:frame` inside the cell, see
/// [`write_cell_frames`]); kept for objects that cannot be cell-anchored.
pub(crate) fn write_shapes(s: &mut String, sheet: &SheetData, tab: usize) {
    let _ = (s, sheet, tab);
}

/// The column/row group walker of a sheet (opens and closes
/// `<table:table-column-group>`/`<table:table-row-group
/// table:display="false|true">` in index order; `write.rs` also stops
/// repeated runs at its boundaries).
pub(crate) fn group_walk(groups: &[OutlineGroupData], rows: bool) -> crate::tools::GroupWalk<'_> {
    crate::tools::GroupWalk::new(groups, rows)
}

/// Writes the `draw:frame` elements of the charts and pictures anchored
/// at cell `(r, c)` of `sheet` (inside the `table:table-cell`); chart
/// objects carry `draw:notify-on-update-of-ranges` with the plotted
/// ranges of `sheets`.
pub(crate) fn write_cell_frames(
    s: &mut String,
    sheets: &[SheetData],
    sheet: &SheetData,
    tab: usize,
    r: u32,
    c: u32,
) {
    crate::frame::write_cell_frames(s, Some(sheets), sheet, tab, r, c);
}

/// `true` when a cell holds an anchored object, so the row/cell writer
/// cannot fold it into a repeated run.
///
/// TODO(W5-1): look up `sheet.charts`/`sheet.images` anchors (index them
/// once per sheet in `write_sheet`).
pub(crate) fn cell_has_frame(sheet: &SheetData, r: u32, c: u32) -> bool {
    crate::frame::cell_has_frame(sheet, r, c)
}

/// The anchor cells of the sheet's charts and pictures (the cell writer
/// gives each an empty cell so the frame has a place to go).
pub(crate) fn frame_anchor_cells(sheet: &SheetData) -> Vec<(u32, u32)> {
    sheet
        .charts
        .iter()
        .map(|c| (c.anchor.row, c.anchor.col))
        .chain(sheet.images.iter().map(|i| (i.anchor.row, i.anchor.col)))
        .collect()
}

/// Writes the named database ranges of every sheet inside
/// `table:database-ranges` (after the anonymous AutoFilter ranges).
///
/// TODO(W5-2): `<table:database-range table:name table:target-range-address
/// table:contains-header table:display-filter-buttons>` with
/// `table:filter` (standard/advanced filter, `table:filter-or`/`-and`,
/// `table:condition-source-range-address`) and `table:subtotal-rules`.
pub(crate) fn write_named_database_ranges(s: &mut String, data: &WorkbookData) {
    crate::tools::write_named_database_ranges(s, data);
}

/// `true` when [`write_named_database_ranges`] will emit something, so
/// `write.rs` opens the `table:database-ranges` element.
pub(crate) fn has_named_database_ranges(data: &WorkbookData) -> bool {
    crate::tools::has_named_database_ranges(data)
}

/// Writes `table:data-pilot-tables` after `table:database-ranges`.
///
/// TODO(W5-2): `table:data-pilot-table` with `table:name`,
/// `table:target-range-address`, `table:grand-total`,
/// `table:ignore-empty-rows`, `table:identify-categories`,
/// `table:show-filter-button`, `table:drill-down-on-double-click`;
/// `table:source-cell-range` or `table:database-source-table`;
/// `table:data-pilot-field` (orientation, function, selected page,
/// `table:is-data-layout-field`) with `table:data-pilot-level`
/// (subtotals, members, display-info, sort-info, layout-info),
/// `table:data-pilot-field-reference` and `table:data-pilot-groups`.
pub(crate) fn write_pivots(s: &mut String, data: &WorkbookData) {
    crate::tools::write_pivots(s, data);
}

/// The manifest entries of the chart objects and pictures
/// (`(full path, media type)`): `Object N/` as
/// `application/vnd.oasis.opendocument.chart`, `Object N/content.xml`
/// and `Object N/styles.xml` as `text/xml`, `Pictures/…` by image type.
/// Pure, so the manifest can stay the second zip entry.
///
/// TODO(W5-1): list them from `SheetData::charts`/`images`.
pub(crate) fn object_manifest_entries(data: &WorkbookData) -> Vec<(String, String)> {
    crate::frame::object_manifest_entries(data)
}

/// Writes the chart objects (`Object N/content.xml`, `Object N/styles.xml`)
/// and the pictures (`Pictures/…`) as zip entries, in the order
/// [`object_manifest_entries`] lists them.
///
/// TODO(W5-1): `office:chart/chart:chart` with class, titles, legend,
/// plot area, axes, series (domain, regression curve, error indicator,
/// mean value) and a `local-table` of cached values; limits 1,000
/// charts per document and 255 series.
pub(crate) fn write_objects<W: Write + Seek>(
    data: &WorkbookData,
    zip: &mut ZipWriter<W>,
    report: &mut WriteReport,
) -> Result<(), OdsError> {
    crate::frame::write_objects(data, zip, report)
}

/// Writes the per-sheet view options and split position into
/// `settings.xml`'s table entry (`ShowFormulas`, `ShowZeroValues`,
/// `ShowGrid`, `HasColumnRowHeaders`, `HorizontalSplitMode`/`Position`
/// for a non-frozen split).
///
/// TODO(W5-2/W5-5): emit the config items.
pub(crate) fn write_view_settings(s: &mut String, sheet: &SheetData) {
    crate::tools::write_view_settings(s, sheet);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_boundaries() {
        let g = [
            OutlineGroupData {
                start: 2,
                end: 5,
                level: 1,
                collapsed: false,
            },
            OutlineGroupData {
                start: 3,
                end: 4,
                level: 2,
                collapsed: true,
            },
        ];
        let mut w = group_walk(&g, true);
        assert_eq!(w.next_boundary(0), Some(2));
        assert_eq!(w.next_boundary(2), Some(3));
        assert_eq!(w.next_boundary(3), Some(5));
        assert_eq!(w.next_boundary(5), Some(6));
        assert_eq!(w.next_boundary(6), None);
    }
}
