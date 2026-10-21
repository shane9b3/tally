//! The v0.5 queries behind the `DocumentService` (wave 5, W5-2): pivots,
//! AutoInput, cell styles, scenarios, database ranges, the Navigator and
//! the document statistics. One-line delegations in `mod.rs` call these.

use tally_api::{
    ApiError, AutocompleteDto, CellStyleDto, DatabaseRangeDto, DocumentStatsDto, NavigatorDto,
    PivotDto, PivotFieldItemsDto, PivotHit, PivotSource, Result, ScenarioDto,
};
use tally_types::SheetId;

use crate::workbook::Workbook;

/// Every pivot table of the document.
pub fn pivots(wb: &Workbook) -> Result<Vec<PivotDto>> {
    Ok(crate::pivot::list(wb))
}

/// The fields and distinct items of a pivot source.
pub fn pivot_field_items(wb: &Workbook, source: &PivotSource) -> Result<Vec<PivotFieldItemsDto>> {
    crate::pivot::field_items(wb, source)
}

/// The pivot output a cell belongs to.
pub fn pivot_at(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> Result<Option<PivotHit>> {
    if wb.sheet(sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {sheet}")));
    }
    Ok(crate::pivot::pivot_at(wb, sheet, r, c))
}

/// AutoInput completions.
pub fn autocomplete(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
    prefix: &str,
) -> Result<AutocompleteDto> {
    crate::autocomplete::autocomplete(wb, sheet, r, c, prefix)
}

/// Named cell styles, `Default` first.
pub fn cell_styles(wb: &Workbook) -> Result<Vec<CellStyleDto>> {
    Ok(crate::cellstyles::to_api(wb))
}

/// The scenarios of a base sheet.
pub fn scenarios(wb: &Workbook, sheet: SheetId) -> Result<Vec<ScenarioDto>> {
    crate::scenarios::list(wb, sheet)
}

/// Every named database range.
pub fn database_ranges(wb: &Workbook) -> Result<Vec<DatabaseRangeDto>> {
    Ok(crate::tools::database_ranges(wb))
}

/// The Navigator's entries.
pub fn navigator(wb: &Workbook) -> Result<NavigatorDto> {
    crate::navigator::navigator(wb)
}

/// Document statistics.
pub fn document_statistics(wb: &Workbook) -> Result<DocumentStatsDto> {
    Ok(crate::navigator::statistics(wb))
}
