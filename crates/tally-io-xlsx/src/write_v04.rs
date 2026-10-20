//! Wave 5 XLSX write placeholders (W5-0a): ordered hooks for charts,
//! pictures, outline groups, scenarios and pivot tables. Every function
//! currently emits nothing; W5-1 and W5-2 fill them in without touching
//! the call sites in `write.rs`.

use rust_xlsxwriter::Worksheet;
use tally_model::{SheetData, WorkbookData};

use crate::write_v03::{Patches, Warn};
use crate::XlsxError;

/// Writes the sheet's charts with rust_xlsxwriter.
///
/// TODO(W5-1): map Column/Bar/Area/Line × stacking to the `ChartType`
/// variants, Pie/Doughnut (hole), Scatter variants, Radar variants,
/// Stock (volume dropped with a warning), Bubble → Scatter with a
/// warning, combo through `chart.combine`, secondary axes, trend lines,
/// error bars, data labels, titles, legend, axis scaling; place with
/// `insert_chart_with_offset` (px from hmm) and `set_width`/`set_height`;
/// 3D and multi-area ranges degrade with a warning.
pub(crate) fn write_charts(
    ws: &mut Worksheet,
    sheet: &SheetData,
    sheet_index: u32,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    let _ = (ws, sheet, sheet_index, report);
    Ok(())
}

/// Writes the sheet's pictures (`insert_image_with_offset`).
///
/// TODO(W5-1 stretch): PNG/JPEG from `ImageData::bytes`.
pub(crate) fn write_images(
    ws: &mut Worksheet,
    sheet: &SheetData,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    let _ = (ws, sheet, report);
    Ok(())
}

/// Writes the row and column outline groups (`outlineLevel`,
/// `collapsed`, `hidden`).
///
/// TODO(W5-2): rust_xlsxwriter row/column group APIs; at most 8 levels.
pub(crate) fn write_outline(ws: &mut Worksheet, sheet: &SheetData) -> Result<(), XlsxError> {
    let _ = (ws, sheet);
    Ok(())
}

/// Adds `<scenarios>` to the sheet XML through the patches pass.
///
/// TODO(W5-2): `<scenarios current="…" show="…"><scenario name comment
/// count><inputCells r val/></scenario></scenarios>` after
/// `sheetProtection`.
pub(crate) fn scenario_patches(data: &WorkbookData, patches: &mut Patches, report: &mut dyn Warn) {
    let _ = (data, patches, report);
}

/// Adds the pivot cache and pivot table parts through the patches pass.
///
/// TODO(W5-2): `xl/pivotCache/pivotCacheDefinition1.xml` (`saveData="0"
/// refreshOnLoad="1"`, `cacheFields` with `sharedItems`),
/// `xl/pivotTables/pivotTable1.xml` (`location`, `pivotFields`,
/// `rowFields`, `colFields`, `pageFields`, `dataFields`), the workbook
/// `pivotCaches`, the rels and the content types. Best effort.
pub(crate) fn pivot_patches(data: &WorkbookData, patches: &mut Patches, report: &mut dyn Warn) {
    let _ = (data, patches, report);
}
