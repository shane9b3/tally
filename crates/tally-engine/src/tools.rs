//! Data tools and editing assists (wave 5, W5-2 / W5-3): the standard and
//! advanced filter, Subtotals, Consolidate, Text to Columns, Remove
//! Duplicates, Insert/Delete Cells with shift, Fill Series, Delete
//! Contents, AutoFormat, AutoInput, the detective, the Navigator,
//! document statistics, the solver and the calculation-settings op.
//!
//! W5-0a lands the module and its constants; `ops_v05::apply` dispatches
//! every op to it once implemented.
//!
//! TODO(W5-2): `std_filter` (16 ops, AND/OR, regex, no duplicates, copy
//! to; stored on the database range), `advanced_filter` (D-function
//! criteria through `tally_functions::criteria`), `subtotals` and
//! `remove_subtotals`, `consolidate`, `text_to_columns` (the
//! `tally-io-csv` tokenizer), `remove_duplicates`, `insert_cells` /
//! `delete_cells` (partial-range reference adjustment in
//! `tally-formula`), `fill_series`, `delete_contents`, `auto_format`
//! (the 17 AOO presets), `autocomplete`, `navigator`, `statistics`,
//! `create_names_from_labels`, database ranges, view options, split and
//! custom sort lists.
//! TODO(W5-3): `precedents`/`dependents` from `DepIndex`, `solver` (two-
//! phase simplex plus branch and bound in a goal-seek style sandbox),
//! `set_calc_settings` (`Change::Settings`, then a hard recalc).

/// Most conditions of a standard filter (AOO's dialog has 4 rows).
pub const MAX_STD_CONDITIONS: usize = 4;
/// Most subtotal groups (AOO's dialog has 3 tabs).
pub const MAX_SUBTOTAL_GROUPS: usize = 3;
/// Most AutoInput candidates returned.
pub const MAX_AUTOCOMPLETE_ITEMS: usize = 1_000;
/// Most `table_op` sandbox evaluations per recalc.
pub const MAX_TABLE_OPS: u32 = 10_000;

/// AOO's AutoFormat presets, in dialog order.
pub const AUTOFORMAT_PRESETS: [&str; 17] = [
    "Default",
    "3D",
    "Black 1",
    "Black 2",
    "Blue",
    "Brown",
    "Currency",
    "Currency 3D",
    "Currency Gray",
    "Currency Lavender",
    "Currency Turquoise",
    "Gray",
    "Green",
    "Lavender",
    "Red",
    "Turquoise",
    "Yellow",
];
