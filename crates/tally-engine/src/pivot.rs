//! DataPilot (wave 5, W5-2): the pivot cache, aggregation with AOO
//! semantics, "show as", grouping and the tabular layout.
//!
//! W5-0a lands only the storage shape and the hooks (identity
//! pass-through so pivot definitions read from a file survive
//! `to_data`/`from_data` and undo).
//!
//! TODO(W5-2): `from_api`/`to_api`, `refresh` (read the source, build the
//! item cache, group, aggregate: Count = non-empty, CountNums = numeric,
//! StDev/Var with n<2 → `#DIV/0!`; apply show-as; lay out with `Filter`
//! row, page fields, corner caption `Sum - Revenue`, `<item> Result`,
//! `Total Result`, the `Data` pseudo-field; bold headers, light fill on
//! totals), `drill_down`, `field_items`, `pivot_at`, and
//! `EvalContext::pivot_data` for GETPIVOTDATA.

use tally_model::PivotData;
use tally_types::SheetId;

use crate::structure::Shape;
use crate::workbook::Workbook;

/// A pivot table with its sheet-local id (stored on its target sheet).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotTable {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The definition.
    pub data: PivotData,
}

/// Most pivot tables per document.
pub const MAX_PIVOTS: usize = 1_000;

/// Loads a sheet's pivots from the interchange model (W5-0a: identity).
pub fn from_data(wb: &mut Workbook, sheet: SheetId, pivots: &[PivotData]) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    for p in pivots.iter().take(MAX_PIVOTS) {
        let id = s.meta.alloc_id();
        std::sync::Arc::make_mut(&mut s.meta.pivots).push(std::sync::Arc::new(PivotTable {
            id,
            data: p.clone(),
        }));
    }
}

/// The interchange form of a sheet's pivots (W5-0a: identity).
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<PivotData> {
    wb.sheet(sheet)
        .map(|s| s.meta.pivots.iter().map(|p| p.data.clone()).collect())
        .unwrap_or_default()
}

/// Moves pivot sources, targets and outputs for a row/column change.
///
/// TODO(W5-2): implement; W5-0a is a no-op hook.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let _ = (wb, sh);
}
