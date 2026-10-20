//! v0.5 DTOs: outline, standard and advanced filters, subtotals,
//! consolidate, text to columns, remove duplicates, scenarios, named cell
//! styles, AutoFormat, insert/delete cells, sort options, names extras,
//! AutoInput, detective, Navigator, document statistics and the view
//! options (wave 5, W5-2 and W5-6).
//!
//! Same serde and ts-rs conventions as [`crate::dto`].

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

use crate::dto::{RangeDto, StyleDto};
use crate::pivot::AggFn;

// ---------------------------------------------------------------------------
// Shared addressing
// ---------------------------------------------------------------------------

/// A range on an explicit sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SheetRange {
    /// Sheet.
    pub sheet: SheetId,
    /// The range.
    pub range: RangeDto,
}

/// A cell on an explicit sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SheetCell {
    /// Sheet.
    pub sheet: SheetId,
    /// Row.
    pub r: u32,
    /// Column.
    pub c: u32,
}

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

/// Rows or columns, for outline ops.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum OutlineAxis {
    /// Row groups.
    Rows,
    /// Column groups.
    Cols,
}

/// One outline group (for the grid's gutter).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutlineGroupDto {
    /// First row/column.
    pub start: u32,
    /// Last row/column.
    pub end: u32,
    /// Nesting level, 1 = outermost (at most 8).
    pub level: u8,
    /// Collapsed.
    pub collapsed: bool,
}

// ---------------------------------------------------------------------------
// Standard and advanced filter
// ---------------------------------------------------------------------------

/// The 16 conditions of the standard filter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StdFilterOp {
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// The N largest values.
    Largest,
    /// The N smallest values.
    Smallest,
    /// The largest N %.
    LargestPct,
    /// The smallest N %.
    SmallestPct,
    /// Contains.
    Contains,
    /// Does not contain.
    NotContains,
    /// Begins with.
    BeginsWith,
    /// Does not begin with.
    NotBeginsWith,
    /// Ends with.
    EndsWith,
    /// Does not end with.
    NotEndsWith,
}

/// How a condition joins the previous one.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StdConnector {
    /// AND.
    And,
    /// OR.
    Or,
}

/// One standard-filter condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StdCondition {
    /// Join with the previous condition (ignored on the first).
    pub connector: StdConnector,
    /// Absolute column.
    pub col: u32,
    /// Comparison.
    pub op: StdFilterOp,
    /// Operand text.
    pub value: String,
}

/// Standard-filter options.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StdFilterOptions {
    /// Case-sensitive.
    pub case_sensitive: bool,
    /// Operands are regular expressions.
    pub regex: bool,
    /// The first row holds labels.
    pub has_header: bool,
    /// Drop duplicate rows.
    pub no_duplicates: bool,
    /// Copy the result here instead of hiding rows.
    pub copy_to: Option<SheetCell>,
    /// Keep the filter criteria with the range.
    pub keep_criteria: bool,
}

/// The standard filter of a sheet (`database_ranges`, `filter_info`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StandardFilterDto {
    /// The filtered area.
    pub range: RangeDto,
    /// Conditions in order (at most 4).
    pub conditions: Vec<StdCondition>,
    /// Options.
    pub options: StdFilterOptions,
    /// Advanced filter: the criteria range.
    pub criteria: Option<SheetRange>,
}

/// A named database range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseRangeDto {
    /// Name.
    pub name: String,
    /// Sheet.
    pub sheet: SheetId,
    /// The range.
    pub range: RangeDto,
    /// The first row holds labels.
    pub has_header: bool,
    /// Its standard/advanced filter.
    pub filter: Option<StandardFilterDto>,
}

// ---------------------------------------------------------------------------
// Subtotals, consolidate, text to columns, duplicates
// ---------------------------------------------------------------------------

/// One subtotal group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubtotalGroup {
    /// Group-by column (absolute).
    pub group_by: u32,
    /// Columns to total.
    pub columns: Vec<u32>,
    /// Function.
    pub function: AggFn,
}

/// Subtotal options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubtotalOptions {
    /// Page break between groups.
    pub page_breaks: bool,
    /// Case-sensitive grouping.
    pub case_sensitive: bool,
    /// Pre-sort by the group columns.
    pub presort: bool,
    /// Ascending pre-sort.
    pub ascending: bool,
    /// Include formats when sorting.
    pub include_formats: bool,
}

impl Default for SubtotalOptions {
    fn default() -> Self {
        Self {
            page_breaks: false,
            case_sensitive: false,
            presort: true,
            ascending: true,
            include_formats: false,
        }
    }
}

/// Column type of a text-to-columns / CSV import column.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CsvColumnType {
    /// Recognise numbers, dates and booleans.
    Standard,
    /// Keep as text.
    Text,
    /// Date, day-month-year.
    DateDmy,
    /// Date, month-day-year.
    DateMdy,
    /// Date, year-month-day.
    DateYmd,
    /// Skip the column.
    Hide,
}

/// Options of text to columns (and of the CSV import dialog).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CsvImportOptionsDto {
    /// Separator characters (any of them splits), e.g. `";"` or `",\t"`.
    pub delimiters: String,
    /// Treat consecutive separators as one.
    pub merge_delimiters: bool,
    /// Text delimiter (`"` or `'`).
    pub quote: String,
    /// Trim spaces.
    pub trim: bool,
    /// Recognise numbers, dates and booleans (`false`: all text).
    pub detect_numbers: bool,
    /// Fixed-width column starts instead of separators.
    pub fixed_widths: Option<Vec<u32>>,
    /// Per-column types (missing columns are `standard`).
    pub column_types: Vec<CsvColumnType>,
}

impl Default for CsvImportOptionsDto {
    fn default() -> Self {
        Self {
            delimiters: ",".into(),
            merge_delimiters: false,
            quote: "\"".into(),
            trim: true,
            detect_numbers: true,
            fixed_widths: None,
            column_types: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// A scenario of its base sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScenarioDto {
    /// Id (ignored on create).
    pub id: u32,
    /// Name.
    pub name: String,
    /// Comment.
    pub comment: String,
    /// The scenario ranges on the base sheet.
    pub ranges: Vec<RangeDto>,
    /// Border colour (`0xRRGGBBAA`).
    pub border_color: Option<u32>,
    /// Draw the border.
    pub display_border: bool,
    /// Copy edits back into the scenario.
    pub copy_back: bool,
    /// Copy styles too.
    pub copy_styles: bool,
    /// Prevent changes.
    pub prevent_changes: bool,
    /// The scenario currently shown (read-only).
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Named cell styles, AutoFormat, format-cells extras
// ---------------------------------------------------------------------------

/// A named cell style (Styles panel). The attributes are fully resolved
/// through the parent chain; `style.id` is ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellStyleDto {
    /// Name (`Default`, `Heading`, `Result` …).
    pub name: String,
    /// Parent style (`None` for `Default`).
    pub parent: Option<String>,
    /// Built in (cannot be deleted or renamed).
    pub builtin: bool,
    /// The resolved attributes.
    pub style: StyleDto,
}

/// Which parts of an AutoFormat preset to apply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoFormatOptions {
    /// Number formats.
    pub number_format: bool,
    /// Fonts.
    pub font: bool,
    /// Alignment.
    pub alignment: bool,
    /// Borders.
    pub borders: bool,
    /// Fill patterns.
    pub pattern: bool,
    /// AutoFit column widths and row heights.
    pub autofit: bool,
}

impl Default for AutoFormatOptions {
    fn default() -> Self {
        Self {
            number_format: true,
            font: true,
            alignment: true,
            borders: true,
            pattern: true,
            autofit: true,
        }
    }
}

/// Underline styles (`StyleDto::underline_style`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnderlineStyle {
    /// Single line.
    Single,
    /// Double line.
    Double,
    /// Single accounting.
    SingleAccounting,
    /// Double accounting.
    DoubleAccounting,
    /// Dotted.
    Dotted,
    /// Dashed.
    Dashed,
    /// Wave.
    Wave,
    /// Bold single.
    Bold,
}

// ---------------------------------------------------------------------------
// Insert/delete cells, fill series, delete contents, names, sort
// ---------------------------------------------------------------------------

/// Which way cells move for `Op::InsertCells` (`down`, `right`) and
/// `Op::DeleteCells` (`up`, `left`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ShiftDir {
    /// Shift cells down (insert).
    Down,
    /// Shift cells right (insert).
    Right,
    /// Shift cells up (delete).
    Up,
    /// Shift cells left (delete).
    Left,
}

/// Series type of the Fill Series dialog.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SeriesKind {
    /// Linear.
    Linear,
    /// Growth.
    Growth,
    /// Date.
    Date,
    /// AutoFill (like the fill handle).
    AutoFill,
}

/// Time unit of a date series.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DateUnit {
    /// Days.
    Day,
    /// Weekdays.
    Weekday,
    /// Months.
    Month,
    /// Years.
    Year,
}

/// What `Op::DeleteContents` removes (the Delete Contents dialog).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteContentsFlags {
    /// Text constants.
    pub text: bool,
    /// Numbers (not dates).
    pub numbers: bool,
    /// Dates and times.
    pub dates: bool,
    /// Formulas.
    pub formulas: bool,
    /// Notes.
    pub notes: bool,
    /// Formats.
    pub formats: bool,
    /// Charts and pictures anchored in the range.
    pub objects: bool,
}

/// Where `Op::CreateNamesFromLabels` takes the names from.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NameLabelFlags {
    /// Top row.
    pub top: bool,
    /// Left column.
    pub left: bool,
    /// Bottom row.
    pub bottom: bool,
    /// Right column.
    pub right: bool,
}

/// Extra options of `Op::SortRange` (all optional; absent = v0.2
/// behaviour).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SortOptions {
    /// Sort columns left to right (keys are then row numbers).
    pub left_to_right: bool,
    /// Write the sorted copy here and leave the range unchanged.
    pub copy_to: Option<SheetCell>,
    /// A custom order for the first key (entries first, in this order).
    pub custom_order: Option<Vec<String>>,
    /// Move cell formats with the rows (default `true` in AOO).
    pub include_formats: bool,
    /// Natural sort (`a2` before `a10`).
    pub natural: bool,
}

// ---------------------------------------------------------------------------
// View options and split
// ---------------------------------------------------------------------------

/// Per-sheet view toggles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ViewOptionsDto {
    /// Show formulas instead of results.
    pub show_formulas: bool,
    /// Value highlighting.
    pub value_highlighting: bool,
    /// Show zero values.
    pub show_zero_values: bool,
    /// Show grid lines.
    pub show_gridlines: bool,
    /// Show row and column headers.
    pub show_headers: bool,
}

impl Default for ViewOptionsDto {
    fn default() -> Self {
        Self {
            show_formulas: false,
            value_highlighting: false,
            show_zero_values: true,
            show_gridlines: true,
            show_headers: true,
        }
    }
}

/// Window ▸ Split position in CSS pixels at zoom 100 % (`0` = no split
/// on that axis).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SplitDto {
    /// Vertical split line from the left of the grid.
    pub x_px: f32,
    /// Horizontal split line from the top of the grid.
    pub y_px: f32,
}

// ---------------------------------------------------------------------------
// AutoInput, detective, Navigator, statistics
// ---------------------------------------------------------------------------

/// Result of `autocomplete` (AutoInput and the Ctrl+D selection list).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutocompleteDto {
    /// The first match in alphabetical order, as the full text to offer
    /// (the typed prefix is kept as typed), or `None`.
    pub completion: Option<String>,
    /// Every text in the column's data block that matches the prefix
    /// (all of them for an empty prefix), sorted case-insensitively, at
    /// most 1,000.
    pub items: Vec<String>,
}

/// One detective arrow target (`precedents`, `dependents`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TraceDto {
    /// Sheet of the range.
    pub sheet: SheetId,
    /// The range.
    pub range: RangeDto,
    /// The range holds an error (red arrow).
    pub error: bool,
}

/// An entry of the Navigator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NavItem {
    /// Display text.
    pub title: String,
    /// Sheet, if the item lives on one.
    pub sheet: Option<SheetId>,
    /// Where double-clicking goes.
    pub range: Option<RangeDto>,
    /// Object id (charts, pivots, scenarios).
    pub id: Option<u32>,
}

/// Result of `navigator`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NavigatorDto {
    /// Sheets in tab order (hidden ones included; `range` is the used area).
    pub sheets: Vec<NavItem>,
    /// Defined names.
    pub names: Vec<NavItem>,
    /// Database ranges.
    pub database_ranges: Vec<NavItem>,
    /// Charts.
    pub charts: Vec<NavItem>,
    /// Pictures.
    pub images: Vec<NavItem>,
    /// Pivot tables.
    pub pivots: Vec<NavItem>,
    /// Notes.
    pub notes: Vec<NavItem>,
    /// Scenarios.
    pub scenarios: Vec<NavItem>,
}

/// Result of `document_statistics` (File ▸ Properties ▸ Statistics).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentStatsDto {
    /// Sheets.
    pub sheets: u32,
    /// Non-empty cells.
    #[ts(type = "number")]
    pub cells: u64,
    /// Formula cells.
    #[ts(type = "number")]
    pub formulas: u64,
    /// Printed pages (`None` until pagination exists).
    pub pages: Option<u32>,
    /// Charts.
    pub charts: u32,
    /// Notes.
    pub notes: u32,
    /// Defined names.
    pub names: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_shapes() {
        assert_eq!(
            serde_json::to_value(OutlineAxis::Cols).unwrap(),
            json!("cols")
        );
        assert_eq!(
            serde_json::to_value(StdFilterOp::NotBeginsWith).unwrap(),
            json!("notBeginsWith")
        );
        assert_eq!(serde_json::to_value(ShiftDir::Left).unwrap(), json!("left"));
        assert_eq!(
            serde_json::to_value(SeriesKind::AutoFill).unwrap(),
            json!("autoFill")
        );
        assert_eq!(
            serde_json::to_value(UnderlineStyle::DoubleAccounting).unwrap(),
            json!("doubleAccounting")
        );
        let c = StdCondition {
            connector: StdConnector::Or,
            col: 2,
            op: StdFilterOp::LargestPct,
            value: "10".into(),
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({ "connector": "or", "col": 2, "op": "largestPct", "value": "10" })
        );
        let o: SortOptions = serde_json::from_value(json!({
            "leftToRight": true, "copyTo": { "sheet": 1, "r": 0, "c": 5 },
            "customOrder": null, "includeFormats": true, "natural": false
        }))
        .unwrap();
        assert_eq!(
            o.copy_to,
            Some(SheetCell {
                sheet: 1,
                r: 0,
                c: 5
            })
        );
        assert_eq!(CsvImportOptionsDto::default().delimiters, ",");
        assert!(ViewOptionsDto::default().show_gridlines);
        let f = DeleteContentsFlags {
            numbers: true,
            ..DeleteContentsFlags::default()
        };
        assert_eq!(serde_json::to_value(&f).unwrap()["numbers"], true);
    }
}
