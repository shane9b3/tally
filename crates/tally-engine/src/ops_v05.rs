//! The v0.5/v0.6 ops: DataPilot, outline, filters, subtotals, data tools,
//! scenarios, cell styles, AutoFormat, sheet extras, names extras, view
//! options and calculation settings (wave 5, W5-2 and W5-3).
//!
//! Each arm delegates to the module that implements it: pivots →
//! `crate::pivot`; outline → `crate::outline`; filters, subtotals,
//! consolidate, text to columns, duplicates, insert/delete cells, delete
//! contents, fill series, sheet and name extras, view options, split,
//! sort lists → `crate::tools`; scenarios → `crate::scenarios`; cell
//! styles and AutoFormat → `crate::cellstyles`.
//!
//! W5-3: `SetCalcSettings` → `crate::calcsettings::set_calc_settings`.

use tally_api::{ApiError, Op};

use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Applies a v0.5/v0.6 op; `None` for the other ops.
pub fn apply(
    wb: &mut Workbook,
    op: &Op,
    fx: &mut Effects,
) -> Option<Result<Vec<Change>, ApiError>> {
    Some(match op {
        Op::CreatePivot { pivot, overwrite } => crate::pivot::create(wb, pivot, *overwrite, fx),
        Op::UpdatePivot {
            id,
            pivot,
            overwrite,
        } => crate::pivot::update(wb, *id, pivot, *overwrite, fx),
        Op::RefreshPivot { id, overwrite } => crate::pivot::refresh(wb, *id, *overwrite, fx),
        Op::DeletePivot { id, clear_output } => crate::pivot::delete(wb, *id, *clear_output, fx),
        Op::PivotDrillDown { id, r, c } => crate::pivot::drill_down(wb, *id, *r, *c, fx),
        Op::Group {
            sheet,
            axis,
            start,
            end,
        } => crate::outline::group(wb, *sheet, *axis, *start, *end, fx),
        Op::Ungroup {
            sheet,
            axis,
            start,
            end,
        } => crate::outline::ungroup(wb, *sheet, *axis, *start, *end, fx),
        Op::SetOutlineCollapsed {
            sheet,
            axis,
            start,
            end,
            collapsed,
        } => crate::outline::set_collapsed(wb, *sheet, *axis, *start, *end, *collapsed, fx),
        Op::ShowOutlineLevel { sheet, axis, level } => {
            crate::outline::show_level(wb, *sheet, *axis, *level, fx)
        }
        Op::AutoOutline { sheet, range } => crate::outline::auto_outline(wb, *sheet, *range, fx),
        Op::ClearOutline { sheet, axis } => crate::outline::clear(wb, *sheet, *axis, fx),
        Op::StandardFilter {
            sheet,
            range,
            conditions,
            options,
        } => crate::tools::standard_filter(wb, *sheet, range, conditions, options, fx),
        Op::AdvancedFilter {
            sheet,
            range,
            criteria,
            options,
        } => crate::tools::advanced_filter(wb, *sheet, range, criteria, options, fx),
        Op::ResetFilter { sheet } => crate::tools::reset_filter(wb, *sheet, fx),
        Op::Subtotals {
            sheet,
            range,
            groups,
            options,
        } => crate::tools::subtotals(wb, *sheet, range, groups, options, fx),
        Op::RemoveSubtotals { sheet, range } => {
            crate::tools::remove_subtotals(wb, *sheet, range, fx)
        }
        Op::Consolidate {
            function,
            sources,
            dest,
            by_row_labels,
            by_col_labels,
            link,
        } => crate::tools::consolidate(
            wb,
            *function,
            sources,
            *dest,
            *by_row_labels,
            *by_col_labels,
            *link,
            fx,
        ),
        Op::TextToColumns {
            sheet,
            range,
            options,
        } => crate::tools::text_to_columns(wb, *sheet, range, options, fx),
        Op::RemoveDuplicates {
            sheet,
            range,
            cols,
            has_header,
        } => crate::tools::remove_duplicates(wb, *sheet, range, cols, *has_header, fx),
        Op::InsertCells {
            sheet,
            range,
            shift,
        }
        | Op::DeleteCells {
            sheet,
            range,
            shift,
        } => crate::tools::shift_range(wb, *sheet, range, *shift, fx),
        Op::DeleteContents {
            sheet,
            ranges,
            what,
        } => crate::tools::delete_contents(wb, *sheet, ranges, what, fx),
        Op::FillSeries {
            sheet,
            range,
            direction,
            kind,
            unit,
            start,
            end,
            increment,
        } => crate::tools::fill_series(
            wb,
            *sheet,
            range,
            *direction,
            *kind,
            *unit,
            start.as_deref(),
            end.as_deref(),
            increment.as_deref(),
            fx,
        ),
        Op::CreateScenario { sheet, scenario } => {
            crate::scenarios::create(wb, *sheet, scenario, fx)
        }
        Op::UpdateScenario {
            sheet,
            id,
            scenario,
        } => crate::scenarios::update(wb, *sheet, *id, scenario, fx),
        Op::ShowScenario { sheet, id } => crate::scenarios::show(wb, *sheet, *id, fx),
        Op::DeleteScenario { sheet, id } => crate::scenarios::delete(wb, *sheet, *id, fx),
        Op::DefineCellStyle { style } => crate::cellstyles::define(wb, style),
        Op::DeleteCellStyle { name } => crate::cellstyles::delete(wb, name),
        Op::ApplyCellStyle {
            sheet,
            ranges,
            name,
        } => crate::cellstyles::apply(wb, *sheet, ranges, name),
        Op::ApplyAutoFormat {
            sheet,
            range,
            preset,
            options,
        } => crate::cellstyles::apply_auto_format(wb, *sheet, range, preset, options, fx),
        Op::ApplyAutoFormatCustom {
            sheet,
            range,
            cells,
            autofit,
        } => crate::cellstyles::apply_auto_format_custom(wb, *sheet, range, cells, *autofit, fx),
        Op::CopySheet { sheet, name, index } => {
            crate::tools::copy_sheet(wb, *sheet, name.as_deref(), *index, fx)
        }
        Op::SetSheetHidden { sheet, hidden } => {
            crate::tools::set_sheet_hidden(wb, *sheet, *hidden, fx)
        }
        Op::SetTabColor { sheet, color } => crate::tools::set_tab_color(wb, *sheet, *color, fx),
        Op::CreateNamesFromLabels { sheet, range, from } => {
            crate::tools::create_names_from_labels(wb, *sheet, range, from, fx)
        }
        Op::DefineDatabaseRange {
            name,
            sheet,
            range,
            has_header,
        } => crate::tools::define_database_range(wb, name, *sheet, range, *has_header, fx),
        Op::DeleteDatabaseRange { name } => crate::tools::delete_database_range(wb, name, fx),
        Op::SetViewOptions { sheet, options } => {
            crate::tools::set_view_options(wb, *sheet, options, fx)
        }
        Op::SetSplit { sheet, split } => crate::tools::set_split(wb, *sheet, *split, fx),
        Op::SetCustomSortLists { lists } => crate::tools::set_custom_sort_lists(wb, lists),
        Op::SetCalcSettings { settings } => {
            crate::calcsettings::set_calc_settings(wb, settings, fx)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_api::{Direction, SeriesKind};

    #[test]
    fn v05_ops_are_dispatched_and_old_ops_pass() {
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
            start: Some("1".into()),
            end: None,
            increment: None,
        };
        let r = apply(&mut wb, &op, &mut fx).expect("a v0.5 op");
        assert!(r.is_ok());
        assert_eq!(
            wb.value(tally_types::CellAddr::new(1, 3, 0)),
            tally_types::Value::Number(4.0)
        );
        let old = Op::AddSheet {
            name: None,
            index: None,
        };
        assert!(apply(&mut wb, &old, &mut fx).is_none());
    }
}
