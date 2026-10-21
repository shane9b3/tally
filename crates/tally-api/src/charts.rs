//! v0.4 DTOs: charts and pictures (wave 5, W5-1).
//!
//! Same serde and ts-rs conventions as [`crate::dto`]. Range fields are
//! text in the UI dialect, like conditional-format formulas (for example
//! `$Sheet1.$B$2:$B$9`; a `~` union is allowed). The engine parses them
//! with `parse_reference_text`, stores the resolved ranges and returns
//! canonical text. Unparsable text is an `invalid_argument` `OpError`; a
//! reference to a deleted sheet becomes `#REF!` and that series renders
//! empty.

use serde::{Deserialize, Serialize};
use tally_types::SheetId;
use ts_rs::TS;

/// Identifies a chart: its sheet and its id on that sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartRef {
    /// Sheet.
    pub sheet: SheetId,
    /// Chart id (stable while the document is open).
    pub id: u32,
}

/// Where an object sits: anchored to a cell with an offset inside it and
/// an absolute size in millimetres (the position follows the cell).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartAnchor {
    /// Anchor row.
    pub r: u32,
    /// Anchor column.
    pub c: u32,
    /// Horizontal offset inside the cell.
    pub dx_mm: f32,
    /// Vertical offset inside the cell.
    pub dy_mm: f32,
    /// Width.
    pub width_mm: f32,
    /// Height.
    pub height_mm: f32,
}

/// Chart types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChartKind {
    /// Vertical bars.
    Column,
    /// Horizontal bars.
    Bar,
    /// Lines.
    Line,
    /// Areas.
    Area,
    /// Pie (donut with `hole`).
    Pie,
    /// XY scatter.
    Scatter,
    /// Bubble.
    Bubble,
    /// Radar (net).
    Radar,
    /// Stock.
    Stock,
}

/// Stacking.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Stacking {
    /// Side by side.
    None,
    /// Stacked.
    Stacked,
    /// Stacked to 100 %.
    Percent,
}

/// Stock chart variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StockVariant {
    /// High, low, close.
    Hlc,
    /// Open, high, low, close.
    Ohlc,
    /// Volume plus high, low, close.
    VolumeHlc,
    /// Volume plus open, high, low, close.
    VolumeOhlc,
}

/// Legend position.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LegendPos {
    /// No legend.
    None,
    /// Left.
    Left,
    /// Right.
    Right,
    /// Top.
    Top,
    /// Bottom.
    Bottom,
}

/// Where a series takes its name from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SeriesName {
    /// A cell reference.
    Ref {
        /// The reference, e.g. `$Sheet1.$B$1`.
        r#ref: String,
    },
    /// Literal text.
    Text {
        /// The text.
        text: String,
    },
}

/// Marker shapes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MarkerShape {
    /// Chosen by the renderer.
    Auto,
    /// No marker.
    None,
    /// Circle.
    Circle,
    /// Square.
    Square,
    /// Diamond.
    Diamond,
    /// Triangle.
    Triangle,
    /// X.
    Cross,
    /// Plus.
    Plus,
}

/// Data label placement.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LabelPos {
    /// Chosen by the renderer.
    Auto,
    /// Outside.
    Outside,
    /// Inside.
    Inside,
    /// Centred.
    Center,
    /// Above.
    Above,
    /// Below.
    Below,
}

/// Data labels of a series.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataLabelsDto {
    /// Show the value.
    pub value: bool,
    /// Show the percentage (pie).
    pub percent: bool,
    /// Show the category.
    pub category: bool,
    /// Show the series name.
    pub series_name: bool,
    /// Placement.
    pub position: LabelPos,
}

impl Default for DataLabelsDto {
    fn default() -> Self {
        Self {
            value: false,
            percent: false,
            category: false,
            series_name: false,
            position: LabelPos::Auto,
        }
    }
}

/// Trend line regression types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TrendKind {
    /// Linear.
    Linear,
    /// Logarithmic.
    Logarithmic,
    /// Exponential.
    Exponential,
    /// Power.
    Power,
}

/// A trend line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrendLineDto {
    /// Regression type.
    pub kind: TrendKind,
    /// Show the equation.
    pub show_equation: bool,
    /// Show R².
    pub show_r2: bool,
    /// Legend entry.
    pub name: Option<String>,
}

/// How error bar lengths are computed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum ErrorBarKind {
    /// Fixed values.
    Constant {
        /// Positive length.
        plus: f64,
        /// Negative length.
        minus: f64,
    },
    /// A percentage of each value.
    Percent {
        /// Percent.
        percent: f64,
    },
    /// `k` standard deviations.
    StdDev {
        /// Multiplier.
        k: f64,
    },
    /// The variance.
    Variance,
    /// The standard error.
    StdError,
    /// Error margin.
    ErrorMargin {
        /// Percent.
        percent: f64,
    },
    /// Lengths from ranges.
    Range {
        /// Positive lengths (reference text).
        plus: String,
        /// Negative lengths (reference text).
        minus: String,
    },
}

/// Which error bar halves are drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorBarDir {
    /// Both.
    Both,
    /// Positive only.
    Plus,
    /// Negative only.
    Minus,
}

/// Error bars of a series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorBarsDto {
    /// Length rule.
    pub kind: ErrorBarKind,
    /// Halves drawn.
    pub direction: ErrorBarDir,
}

/// One series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSeriesDto {
    /// Name.
    pub name: Option<SeriesName>,
    /// Y values (reference text).
    pub values: String,
    /// X values (scatter, bubble).
    pub x_values: Option<String>,
    /// Bubble sizes.
    pub sizes: Option<String>,
    /// Combo override (a line series on a column chart).
    pub kind: Option<ChartKind>,
    /// Plotted against the secondary Y axis.
    pub secondary_axis: bool,
    /// Colour (`0xRRGGBBAA`), or the palette colour.
    pub color: Option<u32>,
    /// Marker shape.
    pub marker: Option<MarkerShape>,
    /// Data labels.
    pub labels: DataLabelsDto,
    /// Trend line.
    pub trend: Option<TrendLineDto>,
    /// Error bars.
    pub error_bars: Option<ErrorBarsDto>,
    /// Mean value line.
    pub mean_line: bool,
}

/// An axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartAxisDto {
    /// Drawn.
    pub visible: bool,
    /// Title.
    pub title: Option<String>,
    /// Minimum (`None`: automatic).
    pub min: Option<f64>,
    /// Maximum.
    pub max: Option<f64>,
    /// Major interval.
    pub major: Option<f64>,
    /// Minor divisions per major interval.
    pub minor_count: Option<u32>,
    /// Logarithmic scale.
    pub log: bool,
    /// Reversed direction.
    pub reverse: bool,
    /// Major grid lines.
    pub major_grid: bool,
    /// Minor grid lines.
    pub minor_grid: bool,
    /// Number format code of the labels.
    pub number_format: Option<String>,
}

/// The wizard's "data range".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartSourceDto {
    /// The range (reference text).
    pub range: String,
    /// Series in rows (`false`: in columns).
    pub series_in_rows: bool,
    /// The first row holds labels.
    pub first_row_label: bool,
    /// The first column holds labels.
    pub first_col_label: bool,
}

/// A chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartDto {
    /// Id (ignored on add).
    pub id: u32,
    /// Object name (`Object 1` from ODS; ignored on add).
    pub name: String,
    /// Placement.
    pub anchor: ChartAnchor,
    /// Type.
    pub kind: ChartKind,
    /// Stacking.
    pub stacking: Stacking,
    /// Lines between points (line, scatter, radar).
    pub lines: bool,
    /// Point markers.
    pub markers: bool,
    /// Smooth lines.
    pub smooth: bool,
    /// Pie hole fraction (`Some(0.5)` = donut).
    pub hole: Option<f32>,
    /// Exploded pie.
    pub exploded: bool,
    /// Filled radar.
    pub filled: bool,
    /// Stock variant.
    pub stock: Option<StockVariant>,
    /// 3D flag (stored for round trip; rendered 2D).
    pub three_d: bool,
    /// Wizard data range. When `series` is empty on add, the engine
    /// derives the series from it.
    pub source: Option<ChartSourceDto>,
    /// Categories (reference text).
    pub categories: Option<String>,
    /// Series.
    pub series: Vec<ChartSeriesDto>,
    /// Title.
    pub title: Option<String>,
    /// Subtitle.
    pub subtitle: Option<String>,
    /// Legend.
    pub legend: LegendPos,
    /// X axis.
    pub x_axis: ChartAxisDto,
    /// Y axis.
    pub y_axis: ChartAxisDto,
    /// Secondary Y axis.
    pub y2_axis: Option<ChartAxisDto>,
    /// Chart background (`0xRRGGBBAA`).
    pub background: Option<u32>,
    /// Plot area background.
    pub plot_background: Option<u32>,
    /// Plot hidden cells (AOO default `false`).
    pub include_hidden: bool,
}

/// Resolved data of one series, for rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SeriesDataDto {
    /// Name.
    pub name: String,
    /// Resolved colour (`0xRRGGBBAA`, from the AOO palette when unset).
    pub color: u32,
    /// Y values (`None` = empty point).
    pub values: Vec<Option<f64>>,
    /// X values (scatter, bubble).
    pub x: Option<Vec<Option<f64>>>,
    /// Bubble sizes.
    pub sizes: Option<Vec<Option<f64>>>,
    /// Formatted value labels.
    pub labels: Vec<String>,
    /// Number format of the values.
    pub number_format: Option<String>,
    /// Trend line points and statistics.
    pub trend: Option<TrendDataDto>,
    /// Error bar `[minus, plus]` lengths per point.
    pub errors: Option<Vec<[f64; 2]>>,
    /// Mean value.
    pub mean: Option<f64>,
}

/// A computed trend line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrendDataDto {
    /// Points `[x, y]` of the fitted curve, in ascending `x`. On category
    /// charts `x` is the 1-based category index; on scatter charts `x`
    /// is the series' X value.
    pub points: Vec<[f64; 2]>,
    /// The equation text.
    pub equation: String,
    /// R² (identical to `RSQ`).
    pub r2: f64,
}

/// Result of `chart_data`: the resolved values of a chart, read from
/// cached cell values without recalculating. Caps: 255 series and 32,000
/// points per series (`truncated` is set beyond them).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChartDataDto {
    /// The chart.
    pub chart: ChartRef,
    /// Document version the data was read at.
    #[ts(type = "number")]
    pub version: u64,
    /// Category labels.
    pub categories: Vec<String>,
    /// Series.
    pub series: Vec<SeriesDataDto>,
    /// Series or points were dropped because of the caps.
    pub truncated: bool,
    /// Problems (invalid ranges, unsupported features).
    pub problems: Vec<String>,
}

/// An embedded picture, anchored like a chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageDto {
    /// Id (ignored on add).
    pub id: u32,
    /// Object name (ignored on add).
    pub name: String,
    /// Placement.
    pub anchor: ChartAnchor,
    /// Media type: `image/png` or `image/jpeg`.
    pub mime: String,
    /// The picture, base64 (≤ 16 MB decoded). Empty in `images()` results;
    /// the UI fetches bytes through the shell when it needs them.
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chart_shapes() {
        assert_eq!(
            serde_json::to_value(ChartKind::Column).unwrap(),
            json!("column")
        );
        assert_eq!(
            serde_json::to_value(LegendPos::Bottom).unwrap(),
            json!("bottom")
        );
        assert_eq!(
            serde_json::to_value(StockVariant::VolumeOhlc).unwrap(),
            json!("volumeOhlc")
        );
        assert_eq!(
            serde_json::to_value(SeriesName::Ref {
                r#ref: "$Sheet1.$B$1".into()
            })
            .unwrap(),
            json!({ "type": "ref", "ref": "$Sheet1.$B$1" })
        );
        assert_eq!(
            serde_json::to_value(ErrorBarKind::StdDev { k: 1.0 }).unwrap(),
            json!({ "type": "stdDev", "k": 1.0 })
        );
        assert_eq!(
            serde_json::to_value(ErrorBarKind::StdError).unwrap(),
            json!({ "type": "stdError" })
        );
        let labels: DataLabelsDto = serde_json::from_value(json!({
            "value": true, "percent": false, "category": false,
            "seriesName": false, "position": "outside"
        }))
        .unwrap();
        assert_eq!(labels.position, LabelPos::Outside);
        let anchor = serde_json::to_value(ChartAnchor {
            r: 1,
            c: 2,
            dx_mm: 0.5,
            dy_mm: 0.0,
            width_mm: 160.0,
            height_mm: 90.0,
        })
        .unwrap();
        assert_eq!(anchor["widthMm"], 160.0);
    }
}
