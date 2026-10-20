//! DataPilot (pivot table) model (wave 5, W5-2).
//!
//! A pivot is stored on its *target* sheet ([`crate::SheetData::pivots`]);
//! its source names a sheet by tab index, so source and target may differ.
//! Field names mirror the DTOs in `tally-api`.

use serde::{Deserialize, Serialize};

use crate::CellRangeLocal;

/// Aggregation functions (DataPilot, Subtotals, Consolidate).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AggFnData {
    /// Sum.
    #[default]
    Sum,
    /// Count of non-empty cells.
    Count,
    /// Average.
    Average,
    /// Maximum.
    Max,
    /// Minimum.
    Min,
    /// Product.
    Product,
    /// Count of numbers.
    CountNums,
    /// Sample standard deviation.
    StDev,
    /// Population standard deviation.
    StDevP,
    /// Sample variance.
    Var,
    /// Population variance.
    VarP,
}

/// Where the pivot reads its rows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PivotSourceData {
    /// A range (header row = field names).
    Range {
        /// Sheet tab index.
        sheet: u32,
        /// The range.
        range: CellRangeLocal,
    },
    /// A named database range.
    DatabaseRange {
        /// The name.
        name: String,
    },
}

impl Default for PivotSourceData {
    fn default() -> Self {
        PivotSourceData::Range {
            sheet: 0,
            range: CellRangeLocal::default(),
        }
    }
}

/// Which way the "Data" pseudo-field goes when there are several data fields.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DataLayoutData {
    /// Data fields as rows.
    #[default]
    Row,
    /// Data fields as columns.
    Column,
}

/// Subtotals of a row/column field.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PivotSubtotalsData {
    /// The data field's own function.
    #[default]
    Auto,
    /// None.
    None,
    /// These functions.
    Custom(Vec<AggFnData>),
}

/// Sort order of a field's items.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub enum PivotSortData {
    /// Source order.
    #[default]
    None,
    /// By item name.
    Name {
        /// Ascending.
        ascending: bool,
    },
    /// By a data field's totals.
    Data {
        /// Index into the data fields.
        data_field: u32,
        /// Ascending.
        ascending: bool,
    },
    /// An explicit order.
    Manual {
        /// Item names in order.
        order: Vec<String>,
    },
}

/// Field layout.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PivotLayoutData {
    /// Tabular.
    #[default]
    Tabular,
    /// Outline, subtotals at the top.
    OutlineTop,
    /// Outline, subtotals at the bottom.
    OutlineBottom,
}

/// "Show automatically": only the top/bottom N items by a data field.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct AutoShowData {
    /// Top (`true`) or bottom.
    pub top: bool,
    /// How many.
    pub count: u32,
    /// Index into the data fields.
    pub data_field: u32,
}

/// Date parts of a date grouping.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatePartData {
    /// Seconds.
    Seconds,
    /// Minutes.
    Minutes,
    /// Hours.
    Hours,
    /// Days, in steps of `n`.
    Days(u32),
    /// Months.
    Months,
    /// Quarters.
    Quarters,
    /// Years.
    Years,
}

/// Grouping of a field's items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PivotGroupData {
    /// Numeric intervals.
    Numeric {
        /// Start (`None`: automatic).
        start: Option<f64>,
        /// End.
        end: Option<f64>,
        /// Interval.
        step: f64,
    },
    /// Date parts.
    Date {
        /// Start serial.
        start: Option<f64>,
        /// End serial.
        end: Option<f64>,
        /// Parts.
        parts: Vec<DatePartData>,
    },
}

/// A row, column or page field.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct PivotFieldData {
    /// Column inside the source (0-based).
    pub source_col: u32,
    /// Field name (the header text).
    pub name: String,
    /// Subtotals.
    #[serde(default)]
    pub subtotals: PivotSubtotalsData,
    /// Show items without data.
    #[serde(default)]
    pub show_empty: bool,
    /// Sort.
    #[serde(default)]
    pub sort: PivotSortData,
    /// Hidden items.
    #[serde(default)]
    pub hidden_items: Vec<String>,
    /// Selected page item (`None` = all).
    #[serde(default)]
    pub page_item: Option<String>,
    /// Layout.
    #[serde(default)]
    pub layout: PivotLayoutData,
    /// Empty line after each item.
    #[serde(default)]
    pub empty_line_after: bool,
    /// Show automatically.
    #[serde(default)]
    pub auto_show: Option<AutoShowData>,
    /// Grouping.
    #[serde(default)]
    pub group: Option<PivotGroupData>,
}

/// "Show as" of a data field.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ShowAsKindData {
    /// Difference from the base item.
    #[default]
    Difference,
    /// Percent of the base item.
    PercentOf,
    /// Percent difference from the base item.
    PercentDifference,
    /// Running total.
    RunningTotal,
    /// Percent of the row total.
    PercentOfRow,
    /// Percent of the column total.
    PercentOfColumn,
    /// Percent of the grand total.
    PercentOfTotal,
    /// Index.
    Index,
}

/// The base item of a "show as".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseItemData {
    /// A named item.
    Named(String),
    /// The previous item.
    Previous,
    /// The next item.
    Next,
}

/// "Show as" settings.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ShowAsData {
    /// Kind.
    pub kind: ShowAsKindData,
    /// Base field (source column).
    #[serde(default)]
    pub base_field: Option<u32>,
    /// Base item.
    #[serde(default)]
    pub base_item: Option<BaseItemData>,
}

/// A data field.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct PivotDataFieldData {
    /// Column inside the source.
    pub source_col: u32,
    /// Aggregation.
    pub function: AggFnData,
    /// Caption override.
    #[serde(default)]
    pub caption: Option<String>,
    /// Show as.
    #[serde(default)]
    pub show_as: Option<ShowAsData>,
}

/// A pivot table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PivotData {
    /// Name (`DataPilot1`).
    #[serde(default)]
    pub name: String,
    /// Source.
    #[serde(default)]
    pub source: PivotSourceData,
    /// Top-left output cell `(row, col)` on the sheet holding this pivot.
    #[serde(default)]
    pub target: (u32, u32),
    /// The last written output area (read-only).
    #[serde(default)]
    pub output: Option<CellRangeLocal>,
    /// Row fields.
    #[serde(default)]
    pub row_fields: Vec<PivotFieldData>,
    /// Column fields.
    #[serde(default)]
    pub col_fields: Vec<PivotFieldData>,
    /// Page (filter) fields.
    #[serde(default)]
    pub page_fields: Vec<PivotFieldData>,
    /// Data fields.
    #[serde(default)]
    pub data_fields: Vec<PivotDataFieldData>,
    /// Orientation of the "Data" pseudo-field.
    #[serde(default)]
    pub data_layout: DataLayoutData,
    /// Grand total of rows.
    pub row_totals: bool,
    /// Grand total of columns.
    pub col_totals: bool,
    /// Ignore empty source rows.
    #[serde(default)]
    pub ignore_empty_rows: bool,
    /// Identify categories.
    #[serde(default)]
    pub identify_categories: bool,
    /// Show the filter button.
    #[serde(default)]
    pub show_filter_button: bool,
    /// Drill down on double-click.
    #[serde(default)]
    pub drill_down: bool,
}

impl Default for PivotData {
    fn default() -> Self {
        Self {
            name: String::new(),
            source: PivotSourceData::default(),
            target: (0, 0),
            output: None,
            row_fields: Vec::new(),
            col_fields: Vec::new(),
            page_fields: Vec::new(),
            data_fields: Vec::new(),
            data_layout: DataLayoutData::Row,
            row_totals: true,
            col_totals: true,
            ignore_empty_rows: false,
            identify_categories: false,
            show_filter_button: true,
            drill_down: true,
        }
    }
}
