//! Scenarios (wave 5, W5-2): a base sheet keeps named sets of values for
//! its scenario ranges; showing one writes its values into the sheet
//! (with copy-back of the previous scenario's cells).
//!
//! The model is `SheetMeta::scenarios` (`tally_model::ScenarioData`,
//! which also carries the cells). ODS stores each scenario as a hidden
//! sheet after the base sheet (`table:scenario`); XLSX as `scenarios`
//! on the base sheet. The IO crates convert.
//!
//! TODO(W5-2): `create`, `update`, `show` (copy-back, styles, prevent
//! changes), `delete`, `to_api`, Navigator entries, and the structure
//! adjustment.

use tally_model::ScenarioData;
use tally_types::SheetId;

use crate::structure::Shape;
use crate::workbook::Workbook;

/// A scenario with its sheet-local id.
#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The scenario.
    pub data: ScenarioData,
}

/// Most scenarios per sheet.
pub const MAX_SCENARIOS: usize = 1_000;

/// Loads a sheet's scenarios from the interchange model (W5-0a: identity).
pub fn from_data(wb: &mut Workbook, sheet: SheetId, scenarios: &[ScenarioData]) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    for sc in scenarios.iter().take(MAX_SCENARIOS) {
        let id = s.meta.alloc_id();
        s.meta.scenarios.push(Scenario {
            id,
            data: sc.clone(),
        });
    }
}

/// The interchange form of a sheet's scenarios (W5-0a: identity).
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<ScenarioData> {
    wb.sheet(sheet)
        .map(|s| s.meta.scenarios.iter().map(|sc| sc.data.clone()).collect())
        .unwrap_or_default()
}

/// Moves scenario ranges and cells for a row/column change.
///
/// TODO(W5-2): implement; W5-0a is a no-op hook.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let _ = (wb, sh);
}
