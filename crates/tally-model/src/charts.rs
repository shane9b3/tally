//! Chart and image model (wave 5, W5-1).
//!
//! The model does not depend on any file format: ranges are resolved
//! `(sheet index, local range)` lists, anchors are "to cell" like AOO's,
//! and the remaining fields mirror the chart DTO in `tally-api`.
//! Every field has a serde default so older files and clients load.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::CellRangeLocal;

/// Most literal points (number and text slots of `numLit`/`strLit` and
/// `local-table` values, counted per chart reference) one document may
/// hold. Readers and the engine drop the literal data of the charts past
/// it with a warning.
pub const MAX_LITERAL_POINTS: usize = 1_000_000;

/// Most chart series one document may hold (all charts, counted per
/// chart reference); readers and the engine cut the series of the charts
/// past it with a warning.
pub const MAX_SERIES_PER_DOCUMENT: usize = 32_000;

/// Most picture bytes per document, counted once per picture reference
/// (a picture shown by 1,000 frames counts 1,000 times).
pub const MAX_TOTAL_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// A data range of a chart: one or more areas (`~` unions), each on a
/// sheet given by its tab index. An empty, non-invalid value means "not
/// set"; `invalid` marks a range whose cells or sheet were deleted
/// (`#REF!`, the series renders empty).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartRangeData {
    /// The areas as `(sheet index, range)`.
    #[serde(default)]
    pub areas: Vec<(u32, CellRangeLocal)>,
    /// The range no longer points at cells.
    #[serde(default)]
    pub invalid: bool,
}

impl ChartRangeData {
    /// `true` when no area is set and the range is not invalid.
    pub fn is_unset(&self) -> bool {
        self.areas.is_empty() && !self.invalid
    }
}

/// Literal series data (XLSX `c:numLit`/`c:strLit`, an ODS
/// `chart:local-table` without ranges).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChartValuesData {
    /// Numbers (`None` = empty point).
    Literal(Vec<Option<f64>>),
    /// Texts (categories, series names).
    LiteralText(Vec<String>),
}

/// Where a chart or image sits: anchored to a cell, with an offset inside
/// it and an absolute size. The position follows the cell; the end cell
/// is computed when writing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChartAnchorData {
    /// Anchor row.
    pub row: u32,
    /// Anchor column.
    pub col: u32,
    /// Horizontal offset inside the cell, 1/100 mm.
    pub dx_hmm: u32,
    /// Vertical offset inside the cell, 1/100 mm.
    pub dy_hmm: u32,
    /// Width, 1/100 mm.
    pub width_hmm: u32,
    /// Height, 1/100 mm.
    pub height_hmm: u32,
}

/// Chart types.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChartKindData {
    /// Vertical bars (ODS `chart:bar`).
    #[default]
    Column,
    /// Horizontal bars (`chart:bar` + `chart:vertical`).
    Bar,
    /// Lines.
    Line,
    /// Areas.
    Area,
    /// Pie (`chart:circle`; a hole makes it `chart:ring`).
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

/// Stacking of column, bar, line and area charts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StackingData {
    /// Side by side.
    #[default]
    None,
    /// Stacked.
    Stacked,
    /// Stacked to 100 %.
    Percent,
}

/// Stock chart variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StockVariantData {
    /// High, low, close.
    #[default]
    Hlc,
    /// Open, high, low, close.
    Ohlc,
    /// Volume plus high, low, close.
    VolumeHlc,
    /// Volume plus open, high, low, close.
    VolumeOhlc,
}

/// Legend position.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LegendPosData {
    /// No legend.
    None,
    /// Left.
    Left,
    /// Right (AOO's default).
    #[default]
    Right,
    /// Top.
    Top,
    /// Bottom.
    Bottom,
}

/// Where a series takes its name from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SeriesNameData {
    /// A cell (`chart:label-cell-address`).
    Ref(ChartRangeData),
    /// Literal text.
    Text(String),
}

/// Marker shapes of line, scatter and radar series.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MarkerShapeData {
    /// Chosen by the renderer.
    #[default]
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

/// Data label position.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LabelPosData {
    /// Chosen by the renderer.
    #[default]
    Auto,
    /// Outside the point.
    Outside,
    /// Inside the point.
    Inside,
    /// Centred on the point.
    Center,
    /// Above the point.
    Above,
    /// Below the point.
    Below,
}

/// Data labels of a series.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DataLabelsData {
    /// Show the value.
    pub value: bool,
    /// Show the value as a percentage (pie).
    pub percent: bool,
    /// Show the category.
    pub category: bool,
    /// Show the series name.
    pub series_name: bool,
    /// Placement.
    pub position: LabelPosData,
}

/// Trend line regression types (AOO 4.1 has no polynomial or moving
/// average; those import as `Linear` with a warning).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TrendKindData {
    /// `y = a·x + b`.
    #[default]
    Linear,
    /// `y = a·ln(x) + b`.
    Logarithmic,
    /// `y = b·e^(a·x)`.
    Exponential,
    /// `y = b·x^a`.
    Power,
}

/// A trend line.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrendLineData {
    /// Regression type.
    pub kind: TrendKindData,
    /// Show the equation.
    pub show_equation: bool,
    /// Show R².
    pub show_r2: bool,
    /// Legend entry.
    pub name: Option<String>,
}

/// How error bar lengths are computed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ErrorBarKindData {
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
    /// `k` standard deviations of the series.
    StdDev {
        /// Multiplier.
        k: f64,
    },
    /// The variance of the series.
    Variance,
    /// The standard error of the series.
    StdError,
    /// Error margin (percent of the largest value).
    ErrorMargin {
        /// Percent.
        percent: f64,
    },
    /// Lengths from cell ranges.
    Range {
        /// Positive lengths.
        plus: ChartRangeData,
        /// Negative lengths.
        minus: ChartRangeData,
    },
}

/// Which error bar halves are drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ErrorBarDirData {
    /// Both.
    #[default]
    Both,
    /// Positive only.
    Plus,
    /// Negative only.
    Minus,
}

/// Error bars of a series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBarsData {
    /// Length rule.
    pub kind: ErrorBarKindData,
    /// Halves drawn.
    #[serde(default)]
    pub direction: ErrorBarDirData,
}

/// One series.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartSeriesData {
    /// Name.
    #[serde(default)]
    pub name: Option<SeriesNameData>,
    /// Y values (or the values of a pie).
    #[serde(default)]
    pub values: ChartRangeData,
    /// Literal Y values, used when `values` is unset.
    #[serde(default)]
    pub literal_values: Option<ChartValuesData>,
    /// X values (scatter, bubble).
    #[serde(default)]
    pub x_values: Option<ChartRangeData>,
    /// Literal X values.
    #[serde(default)]
    pub literal_x: Option<ChartValuesData>,
    /// Bubble sizes.
    #[serde(default)]
    pub sizes: Option<ChartRangeData>,
    /// Literal bubble sizes.
    #[serde(default)]
    pub literal_sizes: Option<ChartValuesData>,
    /// Combo override (a line series on a column chart).
    #[serde(default)]
    pub kind: Option<ChartKindData>,
    /// Plotted against the secondary Y axis.
    #[serde(default)]
    pub secondary_axis: bool,
    /// Colour (`0xRRGGBBAA`), or the palette colour.
    #[serde(default)]
    pub color: Option<u32>,
    /// Marker shape.
    #[serde(default)]
    pub marker: Option<MarkerShapeData>,
    /// Data labels.
    #[serde(default)]
    pub labels: DataLabelsData,
    /// Trend line.
    #[serde(default)]
    pub trend: Option<TrendLineData>,
    /// Error bars.
    #[serde(default)]
    pub error_bars: Option<ErrorBarsData>,
    /// Mean value line.
    #[serde(default)]
    pub mean_line: bool,
}

/// An axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartAxisData {
    /// Drawn.
    pub visible: bool,
    /// Title.
    #[serde(default)]
    pub title: Option<String>,
    /// Minimum (`None`: automatic).
    #[serde(default)]
    pub min: Option<f64>,
    /// Maximum.
    #[serde(default)]
    pub max: Option<f64>,
    /// Major interval.
    #[serde(default)]
    pub major: Option<f64>,
    /// Minor divisions per major interval.
    #[serde(default)]
    pub minor_count: Option<u32>,
    /// Logarithmic scale.
    #[serde(default)]
    pub log: bool,
    /// Reversed direction.
    #[serde(default)]
    pub reverse: bool,
    /// Major grid lines.
    #[serde(default)]
    pub major_grid: bool,
    /// Minor grid lines.
    #[serde(default)]
    pub minor_grid: bool,
    /// Number format code of the labels.
    #[serde(default)]
    pub number_format: Option<String>,
}

impl Default for ChartAxisData {
    fn default() -> Self {
        Self {
            visible: true,
            title: None,
            min: None,
            max: None,
            major: None,
            minor_count: None,
            log: false,
            reverse: false,
            major_grid: false,
            minor_grid: false,
            number_format: None,
        }
    }
}

/// The wizard's "data range" the series were derived from.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartSourceData {
    /// The range.
    pub range: ChartRangeData,
    /// Series in rows (`false`: in columns).
    pub series_in_rows: bool,
    /// The first row holds labels.
    pub first_row_label: bool,
    /// The first column holds labels.
    pub first_col_label: bool,
}

/// A chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartData {
    /// Object name (`Object 1` in ODS).
    #[serde(default)]
    pub name: String,
    /// Placement.
    #[serde(default)]
    pub anchor: ChartAnchorData,
    /// Type.
    #[serde(default)]
    pub kind: ChartKindData,
    /// Stacking.
    #[serde(default)]
    pub stacking: StackingData,
    /// Lines between points (line, scatter, radar).
    #[serde(default)]
    pub lines: bool,
    /// Point markers.
    #[serde(default)]
    pub markers: bool,
    /// Smooth (cubic spline) lines.
    #[serde(default)]
    pub smooth: bool,
    /// Pie hole fraction (`Some(0.5)` = donut).
    #[serde(default)]
    pub hole: Option<f32>,
    /// Exploded pie.
    #[serde(default)]
    pub exploded: bool,
    /// Filled radar.
    #[serde(default)]
    pub filled: bool,
    /// Stock variant.
    #[serde(default)]
    pub stock: Option<StockVariantData>,
    /// 3D flag (kept for round trip; rendered 2D).
    #[serde(default)]
    pub three_d: bool,
    /// Wizard data range.
    #[serde(default)]
    pub source: Option<ChartSourceData>,
    /// Categories.
    #[serde(default)]
    pub categories: Option<ChartRangeData>,
    /// Literal categories.
    #[serde(default)]
    pub literal_categories: Option<ChartValuesData>,
    /// Series.
    #[serde(default)]
    pub series: Vec<ChartSeriesData>,
    /// Title.
    #[serde(default)]
    pub title: Option<String>,
    /// Subtitle.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Legend.
    #[serde(default)]
    pub legend: LegendPosData,
    /// X axis.
    #[serde(default)]
    pub x_axis: ChartAxisData,
    /// Y axis.
    #[serde(default)]
    pub y_axis: ChartAxisData,
    /// Secondary Y axis.
    #[serde(default)]
    pub y2_axis: Option<ChartAxisData>,
    /// Chart background (`0xRRGGBBAA`).
    #[serde(default)]
    pub background: Option<u32>,
    /// Plot area background.
    #[serde(default)]
    pub plot_background: Option<u32>,
    /// Plot hidden cells (AOO default `false`).
    #[serde(default)]
    pub include_hidden: bool,
}

impl ChartValuesData {
    /// Number of slots (points, empty or not).
    pub fn len(&self) -> usize {
        match self {
            Self::Literal(v) => v.len(),
            Self::LiteralText(v) => v.len(),
        }
    }

    /// `true` when there is no slot.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ChartData {
    /// Literal slots held by the chart (categories and every series'
    /// values, X values and sizes), for [`MAX_LITERAL_POINTS`].
    pub fn literal_points(&self) -> usize {
        let n = |v: &Option<ChartValuesData>| v.as_ref().map_or(0, ChartValuesData::len);
        n(&self.literal_categories)
            + self
                .series
                .iter()
                .map(|s| n(&s.literal_values) + n(&s.literal_x) + n(&s.literal_sizes))
                .sum::<usize>()
    }

    /// Drops every literal value list (the ranges are kept).
    pub fn drop_literals(&mut self) {
        self.literal_categories = None;
        for s in &mut self.series {
            s.literal_values = None;
            s.literal_x = None;
            s.literal_sizes = None;
        }
    }
}

impl Default for ChartData {
    fn default() -> Self {
        Self {
            name: String::new(),
            anchor: ChartAnchorData {
                width_hmm: 16_000,
                height_hmm: 9_000,
                ..ChartAnchorData::default()
            },
            kind: ChartKindData::Column,
            stacking: StackingData::None,
            lines: true,
            markers: false,
            smooth: false,
            hole: None,
            exploded: false,
            filled: false,
            stock: None,
            three_d: false,
            source: None,
            categories: None,
            literal_categories: None,
            series: Vec::new(),
            title: None,
            subtitle: None,
            legend: LegendPosData::Right,
            x_axis: ChartAxisData::default(),
            y_axis: ChartAxisData {
                major_grid: true,
                ..ChartAxisData::default()
            },
            y2_axis: None,
            background: None,
            plot_background: None,
            include_hidden: false,
        }
    }
}

/// An embedded picture, anchored like a chart (ODS `draw:image` in
/// `Pictures/`, XLSX drawing).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImageData {
    /// Object name.
    #[serde(default)]
    pub name: String,
    /// Placement.
    #[serde(default)]
    pub anchor: ChartAnchorData,
    /// Media type (`image/png`, `image/jpeg`).
    #[serde(default)]
    pub mime: String,
    /// The encoded picture, shared by every frame that shows it (readers
    /// load a picture once per document; cloning never copies the bytes).
    #[serde(default)]
    pub bytes: Arc<[u8]>,
}
