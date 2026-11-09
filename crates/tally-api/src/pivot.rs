//! v0.5 DTOs: DataPilot (pivot tables), wave 5, W5-2.
//!
//! Same serde and ts-rs conventions as [`crate::dto`].

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

use crate::dto::{CellPos, RangeDto};

/// Aggregation functions (DataPilot, Subtotals, Consolidate).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AggFn {
    /// Sum.
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

/// Where a pivot reads its rows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PivotSource {
    /// A range (header row = field names).
    Range {
        /// Sheet.
        sheet: SheetId,
        /// The range.
        range: RangeDto,
    },
    /// A named database range.
    DatabaseRange {
        /// The name.
        name: String,
    },
}

/// Which way the "Data" pseudo-field goes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DataLayout {
    /// Data fields as rows.
    Row,
    /// Data fields as columns.
    Column,
}

/// Subtotals of a row/column field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PivotSubtotals {
    /// The data field's own function.
    Auto,
    /// None.
    None,
    /// These functions.
    Custom {
        /// Functions.
        fns: Vec<AggFn>,
    },
}

/// Sort order of a field's items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PivotSort {
    /// Source order.
    None,
    /// By item name.
    Name {
        /// Ascending.
        ascending: bool,
    },
    /// By a data field's totals.
    Data {
        /// Index into `data_fields`.
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PivotLayout {
    /// Tabular.
    Tabular,
    /// Outline, subtotals at the top.
    OutlineTop,
    /// Outline, subtotals at the bottom.
    OutlineBottom,
}

/// "Show automatically": only the top/bottom N items by a data field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoShow {
    /// Top (`true`) or bottom.
    pub top: bool,
    /// How many.
    pub count: u32,
    /// Index into `data_fields`.
    pub data_field: u32,
}

/// Date parts of a date grouping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum DatePart {
    /// Seconds.
    Seconds,
    /// Minutes.
    Minutes,
    /// Hours.
    Hours,
    /// Days in steps of `n`.
    Days {
        /// Step.
        n: u32,
    },
    /// Months.
    Months,
    /// Quarters.
    Quarters,
    /// Years.
    Years,
}

/// Grouping of a field's items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum PivotGroupDto {
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
        parts: Vec<DatePart>,
    },
}

/// A row, column or page field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotFieldDto {
    /// Column inside the source (0-based).
    pub source_col: u32,
    /// Field name (the header text).
    pub name: String,
    /// Subtotals.
    pub subtotals: PivotSubtotals,
    /// Show items without data.
    pub show_empty: bool,
    /// Sort.
    pub sort: PivotSort,
    /// Hidden items.
    pub hidden_items: Vec<String>,
    /// Selected page item (`None` = all).
    pub page_item: Option<String>,
    /// Layout.
    pub layout: PivotLayout,
    /// Empty line after each item.
    pub empty_line_after: bool,
    /// Show automatically.
    pub auto_show: Option<AutoShow>,
    /// Grouping.
    pub group: Option<PivotGroupDto>,
}

/// "Show as" kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ShowAsKind {
    /// Difference from the base item.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum BaseItem {
    /// A named item.
    Named {
        /// Item name.
        name: String,
    },
    /// The previous item.
    Previous,
    /// The next item.
    Next,
}

/// "Show as" settings of a data field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShowAs {
    /// Kind.
    pub kind: ShowAsKind,
    /// Base field (source column).
    pub base_field: Option<u32>,
    /// Base item.
    pub base_item: Option<BaseItem>,
}

/// A data field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotDataFieldDto {
    /// Column inside the source.
    pub source_col: u32,
    /// Aggregation.
    pub function: AggFn,
    /// Caption override.
    pub caption: Option<String>,
    /// Show as.
    pub show_as: Option<ShowAs>,
}

/// A pivot table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotDto {
    /// Id (ignored on create).
    pub id: u32,
    /// Name (`DataPilot1`; assigned when empty).
    pub name: String,
    /// Source.
    pub source: PivotSource,
    /// Sheet of the output.
    pub target_sheet: SheetId,
    /// Top-left output cell.
    pub target: CellPos,
    /// The last written output area (read-only).
    pub output: Option<RangeDto>,
    /// Row fields.
    pub row_fields: Vec<PivotFieldDto>,
    /// Column fields.
    pub col_fields: Vec<PivotFieldDto>,
    /// Page (filter) fields.
    pub page_fields: Vec<PivotFieldDto>,
    /// Data fields.
    pub data_fields: Vec<PivotDataFieldDto>,
    /// Orientation of the "Data" pseudo-field.
    pub data_layout: DataLayout,
    /// Grand total of rows.
    pub row_totals: bool,
    /// Grand total of columns.
    pub col_totals: bool,
    /// Ignore empty source rows.
    pub ignore_empty_rows: bool,
    /// Identify categories.
    pub identify_categories: bool,
    /// Show the filter button.
    pub show_filter_button: bool,
    /// Drill down on double-click.
    pub drill_down: bool,
    /// The filter on the source rows (the "Filter" button above the
    /// table); absent or `null` for none.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub source_filter: Option<PivotSourceFilterDto>,
}

/// The filter on a pivot's source rows (AOO `ScPivotFilterDlg`: up to 3
/// conditions; only `=`, `<`, `>`, `<=`, `>=`, `<>`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotSourceFilterDto {
    /// Conditions in order; `col` is the column inside the source (0 =
    /// its first column).
    pub conditions: Vec<crate::tools::StdCondition>,
    /// Case-sensitive.
    pub case_sensitive: bool,
    /// `=` / `<>` operands are regular expressions.
    pub regex: bool,
    /// "No duplications" (kept and saved; AOO's DataPilot ignores it).
    pub no_duplicates: bool,
}

/// One distinct item of a source field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotItemDto {
    /// Formatted text.
    pub text: String,
    /// Occurrences.
    pub count: u32,
}

/// The distinct items of one source field (`pivot_field_items`; the
/// source may be scanned before the pivot exists).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotFieldItemsDto {
    /// Column inside the source.
    pub source_col: u32,
    /// Field name.
    pub name: String,
    /// All items are numbers.
    pub numeric: bool,
    /// All items are dates.
    pub date: bool,
    /// Items in display order (at most 10,000).
    pub items: Vec<PivotItemDto>,
}

/// What part of a pivot's output a cell belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PivotCellRole {
    /// A field header or the corner cell.
    Header,
    /// A row item.
    RowItem,
    /// A column item.
    ColItem,
    /// A page field row.
    PageField,
    /// A data cell.
    Data,
    /// A subtotal or grand total.
    Total,
    /// The "Filter" button above the table.
    FilterButton,
}

/// Result of `pivot_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PivotHit {
    /// Pivot id.
    pub pivot: u32,
    /// What the cell is.
    pub role: PivotCellRole,
    /// The field (source column), if any.
    pub field: Option<u32>,
    /// The item, if any.
    pub item: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pivot_shapes() {
        assert_eq!(
            serde_json::to_value(AggFn::CountNums).unwrap(),
            json!("countNums")
        );
        assert_eq!(
            serde_json::to_value(PivotSource::DatabaseRange {
                name: "Sales".into()
            })
            .unwrap(),
            json!({ "type": "databaseRange", "name": "Sales" })
        );
        assert_eq!(
            serde_json::to_value(PivotSort::Data {
                data_field: 0,
                ascending: false
            })
            .unwrap(),
            json!({ "type": "data", "dataField": 0, "ascending": false })
        );
        assert_eq!(
            serde_json::to_value(PivotSubtotals::Custom {
                fns: vec![AggFn::Sum, AggFn::Max]
            })
            .unwrap(),
            json!({ "type": "custom", "fns": ["sum", "max"] })
        );
        assert_eq!(
            serde_json::to_value(DatePart::Days { n: 7 }).unwrap(),
            json!({ "type": "days", "n": 7 })
        );
        assert_eq!(
            serde_json::to_value(BaseItem::Previous).unwrap(),
            json!({ "type": "previous" })
        );
        let g: PivotGroupDto = serde_json::from_value(json!({
            "type": "numeric", "start": null, "end": 100.0, "step": 10.0
        }))
        .unwrap();
        assert!(matches!(g, PivotGroupDto::Numeric { step, .. } if step == 10.0));
        assert_eq!(
            serde_json::to_value(PivotCellRole::RowItem).unwrap(),
            json!("rowItem")
        );
    }
}
