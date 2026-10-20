//! The v0.5/v0.6 ops: DataPilot, outline, filters, subtotals, data tools,
//! scenarios, cell styles, AutoFormat, sheet extras, names extras, view
//! options and calculation settings (wave 5, W5-2 and W5-3).
//!
//! W5-0a: every op is recognised and refused with `OpError { code:
//! "unsupported" }`. Each arm names the module that implements it.
//!
//! TODO(W5-2): pivots → `crate::pivot`; outline → `crate::outline`;
//! filters, subtotals, consolidate, text to columns, duplicates,
//! insert/delete cells, delete contents, fill series, AutoFormat, sheet
//! and name extras, view options, split, sort lists → `crate::tools`;
//! scenarios → `crate::scenarios`; cell styles → `crate::cellstyles`.
//! TODO(W5-3): `SetCalcSettings` → `crate::tools::set_calc_settings`.

use tally_api::{ApiError, Op};

use crate::ops_v04::unsupported;
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Applies a v0.5/v0.6 op; `None` for the other ops.
pub fn apply(
    wb: &mut Workbook,
    op: &Op,
    fx: &mut Effects,
) -> Option<Result<Vec<Change>, ApiError>> {
    let _ = (wb, fx);
    let what = match op {
        Op::CreatePivot { .. }
        | Op::UpdatePivot { .. }
        | Op::RefreshPivot { .. }
        | Op::DeletePivot { .. }
        | Op::PivotDrillDown { .. } => "pivot tables",
        Op::Group { .. }
        | Op::Ungroup { .. }
        | Op::SetOutlineCollapsed { .. }
        | Op::ShowOutlineLevel { .. }
        | Op::AutoOutline { .. }
        | Op::ClearOutline { .. } => "outline groups",
        Op::StandardFilter { .. } | Op::AdvancedFilter { .. } | Op::ResetFilter { .. } => {
            "standard and advanced filters"
        }
        Op::Subtotals { .. } | Op::RemoveSubtotals { .. } => "subtotals",
        Op::Consolidate { .. } => "consolidate",
        Op::TextToColumns { .. } => "text to columns",
        Op::RemoveDuplicates { .. } => "remove duplicates",
        Op::InsertCells { .. } | Op::DeleteCells { .. } => "insert and delete cells",
        Op::DeleteContents { .. } => "delete contents",
        Op::FillSeries { .. } => "fill series",
        Op::CreateScenario { .. }
        | Op::UpdateScenario { .. }
        | Op::ShowScenario { .. }
        | Op::DeleteScenario { .. } => "scenarios",
        Op::DefineCellStyle { .. } | Op::DeleteCellStyle { .. } | Op::ApplyCellStyle { .. } => {
            "cell styles"
        }
        Op::ApplyAutoFormat { .. } => "AutoFormat",
        Op::CopySheet { .. } | Op::SetSheetHidden { .. } | Op::SetTabColor { .. } => "sheet extras",
        Op::CreateNamesFromLabels { .. }
        | Op::DefineDatabaseRange { .. }
        | Op::DeleteDatabaseRange { .. } => "names extras",
        Op::SetViewOptions { .. } | Op::SetSplit { .. } => "view options",
        Op::SetCustomSortLists { .. } => "custom sort lists",
        Op::SetCalcSettings { .. } => "calculation settings",
        _ => return None,
    };
    Some(Err(unsupported(what)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_api::{Direction, SeriesKind};

    #[test]
    fn new_ops_are_refused_as_unsupported_and_old_ops_pass() {
        let mut wb = Workbook::new();
        let mut fx = Effects::default();
        let op = Op::FillSeries {
            sheet: 1,
            range: tally_api::RangeDto {
                r0: 0,
                c0: 0,
                r1: 3,
                c1: 0,
            },
            direction: Direction::Down,
            kind: SeriesKind::Linear,
            unit: None,
            start: None,
            end: None,
            increment: None,
        };
        let r = apply(&mut wb, &op, &mut fx).expect("a v0.5 op");
        assert_eq!(r.unwrap_err().code, ApiError::UNSUPPORTED);
        assert!(crate::ops_v04::apply(&mut wb, &op, &mut fx).is_none());
        let old = Op::AddSheet {
            name: None,
            index: None,
        };
        assert!(apply(&mut wb, &old, &mut fx).is_none());
        assert!(crate::ops_v04::apply(&mut wb, &old, &mut fx).is_none());
    }
}
