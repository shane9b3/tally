//! Row and column outline groups (wave 5, W5-2): Group/Ungroup,
//! collapse/expand (hides the rows/columns), level buttons, AutoOutline
//! from the formula structure, and how groups follow row/column changes.
//!
//! The model is `SheetMeta::row_groups`/`col_groups`
//! (`tally_model::OutlineGroupData`, at most 8 levels). ODS writes
//! `table:table-row-group`/`table:table-column-group table:display`;
//! XLSX `outlineLevel`/`collapsed`/`hidden`.
//!
//! TODO(W5-2): `group`, `ungroup`, `set_collapsed`, `show_level`,
//! `auto_outline`, `clear`, `to_api`, and the structure adjustment.

use crate::structure::Shape;
use crate::workbook::Workbook;

/// Deepest outline level (AOO and Excel).
pub const MAX_LEVELS: u8 = 8;

/// Moves outline groups for a row/column change on `sh.sheet` (a group
/// grows, shrinks or goes away like an area reference).
///
/// TODO(W5-2): implement; W5-0a is a no-op hook.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let _ = (wb, sh);
}
