//! Data tools and sheet extras (wave 5, W5-2/W5-3): outline groups,
//! standard/advanced filters, database ranges, subtotals, scenarios, view
//! options, split panes, named cell styles and the style extras.
//!
//! Every field has a serde default so older files and clients load.

use serde::{Deserialize, Serialize};

use crate::pivot::AggFnData;
use crate::{CellData, CellRangeLocal, StyleData};

// ---------------------------------------------------------------------------
// Outline
// ---------------------------------------------------------------------------

/// A row or column group of the outline (ODS `table:table-row-group`,
/// XLSX `outlineLevel`). At most 8 levels; `start..=end` inclusive.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OutlineGroupData {
    /// First row/column.
    pub start: u32,
    /// Last row/column.
    pub end: u32,
    /// Nesting level, 1 = outermost.
    pub level: u8,
    /// Collapsed (its rows/columns are hidden).
    pub collapsed: bool,
}

// ---------------------------------------------------------------------------
// Standard and advanced filter, database ranges
// ---------------------------------------------------------------------------

/// The 16 conditions of AOO's standard filter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StdFilterOpData {
    /// `=`
    #[default]
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StdConnectorData {
    /// AND.
    #[default]
    And,
    /// OR.
    Or,
}

/// One standard-filter condition.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct StdConditionData {
    /// Join with the previous condition (ignored on the first).
    pub connector: StdConnectorData,
    /// Absolute column.
    pub col: u32,
    /// Comparison.
    pub op: StdFilterOpData,
    /// Operand text.
    pub value: String,
}

/// Standard-filter options.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StdFilterOptionsData {
    /// Case-sensitive.
    pub case_sensitive: bool,
    /// Operands are regular expressions.
    pub regex: bool,
    /// The first row holds labels.
    pub has_header: bool,
    /// Drop duplicate rows.
    pub no_duplicates: bool,
    /// Copy the result to `(sheet index, row, col)` instead of hiding rows.
    pub copy_to: Option<(u32, u32, u32)>,
    /// Keep the filter criteria with the range.
    pub keep_criteria: bool,
}

/// A standard filter (or an advanced filter, when `criteria` is set).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct StandardFilterData {
    /// The filtered area (header row first when `options.has_header`).
    pub range: CellRangeLocal,
    /// Conditions in order (at most 4 in AOO's dialog).
    #[serde(default)]
    pub conditions: Vec<StdConditionData>,
    /// Options.
    #[serde(default)]
    pub options: StdFilterOptionsData,
    /// Advanced filter: the criteria range `(sheet index, range)`.
    #[serde(default)]
    pub criteria: Option<(u32, CellRangeLocal)>,
}

/// A named database range (Data ▸ Define Range).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DatabaseRangeData {
    /// Name.
    pub name: String,
    /// The range.
    pub range: CellRangeLocal,
    /// The first row holds labels.
    #[serde(default)]
    pub has_header: bool,
    /// AutoFilter buttons shown.
    #[serde(default)]
    pub filter_buttons: bool,
    /// Its standard/advanced filter.
    #[serde(default)]
    pub filter: Option<StandardFilterData>,
    /// Its subtotal rules.
    #[serde(default)]
    pub subtotals: Option<SubtotalsData>,
    /// Its last sort (AOO keeps the `ScSortParam` on the `ScDBData`;
    /// Data ▸ Refresh Range sorts again with it).
    #[serde(default)]
    pub sort: Option<DbSortData>,
}

/// The sort kept with a database range (ODS `table:sort`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DbSortData {
    /// Keys in priority order: the column as an offset from the range's
    /// first column (ODS `table:field-number`, so it moves with the
    /// range), and ascending.
    pub keys: Vec<(u32, bool)>,
    /// Compare text case-sensitively.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Cell formats move with the rows (ODS `table:bind-styles-to-content`).
    #[serde(default)]
    pub include_formats: bool,
    /// Natural sort (`a2` before `a10`).
    #[serde(default)]
    pub natural: bool,
}

// ---------------------------------------------------------------------------
// Subtotals
// ---------------------------------------------------------------------------

/// One subtotal group (AOO allows three).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubtotalGroupData {
    /// Group-by column (absolute).
    pub group_by: u32,
    /// Columns to total.
    pub columns: Vec<u32>,
    /// Function.
    pub function: AggFnData,
}

/// Data ▸ Subtotals definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtotalsData {
    /// The area including its header row.
    pub range: CellRangeLocal,
    /// Groups in order.
    pub groups: Vec<SubtotalGroupData>,
    /// Page break between groups.
    #[serde(default)]
    pub page_breaks: bool,
    /// Case-sensitive grouping.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Pre-sort by the group columns.
    #[serde(default = "yes")]
    pub presort: bool,
    /// Ascending pre-sort.
    #[serde(default = "yes")]
    pub ascending: bool,
    /// Include formats when sorting.
    #[serde(default)]
    pub include_formats: bool,
}

impl Default for SubtotalsData {
    fn default() -> Self {
        Self {
            range: CellRangeLocal::default(),
            groups: Vec::new(),
            page_breaks: false,
            case_sensitive: false,
            presort: true,
            ascending: true,
            include_formats: false,
        }
    }
}

fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// A scenario of its base sheet. AOO keeps each scenario as a hidden sheet
/// after the base sheet (ODS `table:scenario`); the model keeps them on the
/// base sheet with their cell values, as XLSX `scenarios` does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioData {
    /// Name.
    pub name: String,
    /// Comment.
    #[serde(default)]
    pub comment: String,
    /// The scenario ranges on the base sheet.
    #[serde(default)]
    pub ranges: Vec<CellRangeLocal>,
    /// The scenario's cell contents inside `ranges`.
    #[serde(default)]
    pub cells: Vec<CellData>,
    /// Border colour (`0xRRGGBBAA`).
    #[serde(default)]
    pub border_color: Option<u32>,
    /// Draw the border.
    #[serde(default = "yes")]
    pub display_border: bool,
    /// Copy edits back into the scenario.
    #[serde(default = "yes")]
    pub copy_back: bool,
    /// Copy styles too.
    #[serde(default)]
    pub copy_styles: bool,
    /// Prevent changes.
    #[serde(default)]
    pub prevent_changes: bool,
    /// The scenario currently shown.
    #[serde(default)]
    pub active: bool,
}

impl Default for ScenarioData {
    fn default() -> Self {
        Self {
            name: String::new(),
            comment: String::new(),
            ranges: Vec::new(),
            cells: Vec::new(),
            border_color: None,
            display_border: true,
            copy_back: true,
            copy_styles: false,
            prevent_changes: false,
            active: false,
        }
    }
}

// ---------------------------------------------------------------------------
// View options and split
// ---------------------------------------------------------------------------

/// Per-sheet view toggles (ODS settings.xml, XLSX `sheetView`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewOptionsData {
    /// Show formulas instead of results.
    #[serde(default)]
    pub show_formulas: bool,
    /// Value highlighting (numbers blue, formulas green, text black).
    #[serde(default)]
    pub value_highlighting: bool,
    /// Show zero values.
    #[serde(default = "yes")]
    pub show_zero_values: bool,
    /// Show grid lines.
    #[serde(default = "yes")]
    pub show_gridlines: bool,
    /// Show row and column headers.
    #[serde(default = "yes")]
    pub show_headers: bool,
}

impl Default for ViewOptionsData {
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

/// Document-wide view options that AOO keeps per view and saves in
/// `settings.xml` (Tools ▸ Options ▸ OpenOffice Calc ▸ View; AOO
/// `ScViewData::WriteUserDataSequence`): the comment indicator, the grid
/// colour, page breaks, sheet tabs and outline symbols. XLSX keeps the sheet
/// tabs (`workbookView showSheetTabs`) and outline symbols (`sheetView
/// showOutlineSymbols`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocViewData {
    /// Comment indicator (AOO `ShowNotes`, `VOPT_NOTES`).
    #[serde(default = "yes")]
    pub show_notes: bool,
    /// Grid line colour as `0xRRGGBB`; `None` for the automatic colour
    /// (AOO `GridColor`; its default `SC_STD_GRIDCOLOR` is read as `None`).
    #[serde(default)]
    pub grid_color: Option<u32>,
    /// Page breaks (AOO `ShowPageBreaks`, `VOPT_PAGEBREAKS`). Kept for the
    /// round trip; Tally shows the lines with View ▸ Page breaks.
    #[serde(default = "yes")]
    pub show_page_breaks: bool,
    /// Sheet tabs (AOO `HasSheetTabs`, `VOPT_TABCONTROLS`).
    #[serde(default = "yes")]
    pub sheet_tabs: bool,
    /// Outline symbols (AOO `IsOutlineSymbolsSet`, `VOPT_OUTLINER`).
    #[serde(default = "yes")]
    pub outline_symbols: bool,
}

impl Default for DocViewData {
    fn default() -> Self {
        Self {
            show_notes: true,
            grid_color: None,
            show_page_breaks: true,
            sheet_tabs: true,
            outline_symbols: true,
        }
    }
}

/// AOO's automatic grid colour (`SC_STD_GRIDCOLOR`, `COL_LIGHTGRAY`).
pub const STD_GRID_COLOR: u32 = 0x00C0_C0C0;

/// Window ▸ Split position (independent of frozen panes). `0` = no split
/// on that axis. ODS `HorizontalSplitMode`/`Position`, XLSX `pane` (not
/// frozen).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SplitData {
    /// Vertical split line, 1/100 mm from the left of the grid.
    pub x_hmm: u32,
    /// Horizontal split line, 1/100 mm from the top of the grid.
    pub y_hmm: u32,
}

// ---------------------------------------------------------------------------
// Named cell styles and style extras
// ---------------------------------------------------------------------------

/// A named cell style (ODS `office:styles` `style:style
/// style:family="table-cell"`, XLSX `cellStyleXfs`/`cellStyles`). The
/// attributes are stored fully resolved; `parent` keeps the chain for
/// files and the Styles panel.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct CellStyleData {
    /// Name (`Default`, `Heading`, `Result` …).
    pub name: String,
    /// Parent style name (`None` for `Default`).
    #[serde(default)]
    pub parent: Option<String>,
    /// The resolved attributes.
    #[serde(default)]
    pub style: StyleData,
}

/// Underline styles (meaningful when the font is underlined).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum UnderlineStyleData {
    /// Single line.
    #[default]
    Single,
    /// Double line.
    Double,
    /// Single accounting (XLSX).
    SingleAccounting,
    /// Double accounting (XLSX).
    DoubleAccounting,
    /// Dotted (ODS).
    Dotted,
    /// Dashed (ODS).
    Dashed,
    /// Wave (ODS).
    Wave,
    /// Bold single (ODS).
    Bold,
}

/// What a sheet prints of its notes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NotesPrintData {
    /// Notes are not printed.
    #[default]
    None,
    /// Notes are printed on a page at the end.
    AtEnd,
}
