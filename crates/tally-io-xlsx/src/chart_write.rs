//! Chart writing (wave 5, W5-1) through `rust_xlsxwriter`.
//!
//! What the library cannot express degrades with a warning: bubble
//! charts are written as scatter charts, stock volume is dropped, 3D is
//! written flat, multi-area ranges keep their first area, asymmetric
//! constant error bars use the positive length, variance and
//! error-margin error bars are dropped.
//!
//! Cached values: rust_xlsxwriter reads every cell of every chart range
//! into a string cache and clones it into each series that names the
//! range. Ranges over [`MAX_CACHED_RANGE_CELLS`] cells, and every range
//! once a save has cached [`MAX_CACHED_CELLS`] cells (counted per
//! reference), are written as a bare reference (`('Sheet'!$B$1:$B$9)`,
//! which the library leaves uncached); Excel and Tally recompute the
//! values from the cells on open.
//!
//! Literal data (a chart with its own data, AOO's internal data
//! provider): rust_xlsxwriter only writes references, so each literal
//! list is written as a placeholder reference (`_TallyLit[n]`) that the
//! patch pass replaces with `c:numLit` / `c:strLit` (see
//! [`patch_chart_literals`]).
//!
//! The library keys objects by anchor cell and offset, so two charts (or
//! pictures) at the same spot would overwrite each other: later ones are
//! nudged by one pixel per collision.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use rust_xlsxwriter::{
    Chart, ChartDataLabel, ChartDataLabelPosition, ChartErrorBars, ChartErrorBarsDirection,
    ChartErrorBarsType, ChartFormat, ChartLegendPosition, ChartLine, ChartMarker, ChartMarkerType,
    ChartRange, ChartSolidFill, ChartTrendline, ChartTrendlineType, ChartType, Color, Image,
    Worksheet,
};
use tally_formula::Resolver;
use tally_model::{
    AxisCrossData, AxisLabelPosData, AxisTypeData, ChartAxisData, ChartData, ChartKindData,
    ChartRangeData, ChartSeriesData, ChartValuesData, ErrorBarDirData, ErrorBarKindData,
    LabelPosData, LegendPosData, MarkerShapeData, SeriesNameData, SheetData, StackingData,
    StockVariantData, TimeIntervalData, TimeUnitData, TrendKindData,
};

use crate::colors::model_to_rgb;
use crate::formula::XlsxResolver;
use crate::units::hmm_to_px;
use crate::write_v03::Warn;
use crate::XlsxError;

/// Most series written per chart.
const MAX_SERIES: usize = 255;
/// Largest range written with cached values.
pub(crate) const MAX_CACHED_RANGE_CELLS: u64 = 32_000;
/// Most cached cells per save, counted once per series reference.
pub(crate) const MAX_CACHED_CELLS: u64 = 1_000_000;

thread_local! {
    /// (cached cells so far, ranges written uncached) of the save in
    /// progress; reset by [`begin_save`] when the first sheet is written.
    static CACHE_BUDGET: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

thread_local! {
    /// The literal lists of the save in progress: `(marker, xml)`.
    static LITERALS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Starts the cached-cell budget of a save.
pub(crate) fn begin_save() {
    CACHE_BUDGET.with(|b| b.set((0, 0)));
    LITERALS.with(|l| l.borrow_mut().clear());
}

/// The literal lists the save in progress wrote (for the patch pass).
pub(crate) fn take_literals() -> Vec<(String, String)> {
    LITERALS.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `c:numLit` / `c:strLit` of a literal list.
fn literal_xml(v: &ChartValuesData) -> String {
    let mut out = String::new();
    match v {
        ChartValuesData::Literal(v) => {
            out.push_str("<c:numLit><c:formatCode>General</c:formatCode>");
            out.push_str(&format!(r#"<c:ptCount val="{}"/>"#, v.len()));
            for (i, x) in v.iter().enumerate() {
                if let Some(x) = x.filter(|x| x.is_finite()) {
                    out.push_str(&format!(r#"<c:pt idx="{i}"><c:v>{x}</c:v></c:pt>"#));
                }
            }
            out.push_str("</c:numLit>");
        }
        ChartValuesData::LiteralText(t) => {
            out.push_str(&format!(r#"<c:strLit><c:ptCount val="{}"/>"#, t.len()));
            for (i, x) in t.iter().enumerate() {
                if !x.is_empty() {
                    out.push_str(&format!(
                        r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#,
                        xml_text(x)
                    ));
                }
            }
            out.push_str("</c:strLit>");
        }
    }
    out
}

/// A placeholder reference for a literal list (replaced by the patch pass).
fn literal_range(v: &ChartValuesData) -> ChartRange {
    let marker = LITERALS.with(|l| {
        let mut l = l.borrow_mut();
        let m = format!("_TallyLit[{}]", l.len());
        l.push((m.clone(), literal_xml(v)));
        m
    });
    // `=…[…]` is passed through as written and never cached.
    ChartRange::new_from_string(&format!("={marker}"))
}

/// Replaces the placeholder references of a chart part with their
/// literal lists.
pub(crate) fn patch_chart_literals(xml: &str, literals: &[(String, String)]) -> String {
    if !xml.contains("_TallyLit[") {
        return xml.to_string();
    }
    let mut out = xml.to_string();
    for (marker, lit) in literals {
        for tag in ["c:numRef", "c:strRef"] {
            let placeholder = format!("<{tag}><c:f>{marker}</c:f></{tag}>");
            if out.contains(&placeholder) {
                out = out.replace(&placeholder, lit);
            }
        }
    }
    out
}

/// Number of ranges the save in progress wrote without cached values.
pub(crate) fn uncached_ranges() -> u64 {
    CACHE_BUDGET.with(|b| b.get().1)
}

fn rgb(c: u32) -> Color {
    Color::RGB(model_to_rgb(c))
}

/// The first area of a range as a `ChartRange` (`None` when unset,
/// invalid or on an unknown sheet).
fn chart_range(
    r: &ChartRangeData,
    resolver: &XlsxResolver,
    name: &str,
    what: &str,
    report: &mut dyn Warn,
) -> Option<ChartRange> {
    chart_range_refs(r, resolver, name, what, report, 1)
}

/// [`chart_range`] for a range that `refs` series will name (each clones
/// the cache).
fn chart_range_refs(
    r: &ChartRangeData,
    resolver: &XlsxResolver,
    name: &str,
    what: &str,
    report: &mut dyn Warn,
    refs: u64,
) -> Option<ChartRange> {
    if r.invalid || r.areas.is_empty() {
        return None;
    }
    if r.areas.len() > 1 {
        report.warn(format!(
            "chart {name:?}: the {what} range has several areas; only the first was written"
        ));
    }
    let (tab, a) = &r.areas[0];
    let sheet = resolver.sheet_name(*tab)?;
    let (c0, c1) = (u16::try_from(a.c0).ok()?, u16::try_from(a.c1).ok()?);
    let cells = u64::from(a.r1.saturating_sub(a.r0) + 1) * u64::from(c1.saturating_sub(c0) + 1);
    let cost = cells.saturating_mul(refs.max(1));
    let cached = CACHE_BUDGET.with(|b| {
        let (used, uncached) = b.get();
        if cells <= MAX_CACHED_RANGE_CELLS && used.saturating_add(cost) <= MAX_CACHED_CELLS {
            b.set((used + cost, uncached));
            true
        } else {
            b.set((used, uncached + 1));
            false
        }
    });
    if cached {
        Some(ChartRange::new_from_range(&sheet, a.r0, c0, a.r1, c1))
    } else {
        // `=(...)` is passed through as written and never cached.
        let abs = rust_xlsxwriter::utility::worksheet_range_absolute(&sheet, a.r0, c0, a.r1, c1);
        Some(ChartRange::new_from_string(&format!("=({abs})")))
    }
}

fn chart_type(kind: ChartKindData, c: &ChartData) -> ChartType {
    let stacked = |n, s, p| match c.stacking {
        StackingData::None => n,
        StackingData::Stacked => s,
        StackingData::Percent => p,
    };
    match kind {
        ChartKindData::Column => stacked(
            ChartType::Column,
            ChartType::ColumnStacked,
            ChartType::ColumnPercentStacked,
        ),
        ChartKindData::Bar => stacked(
            ChartType::Bar,
            ChartType::BarStacked,
            ChartType::BarPercentStacked,
        ),
        ChartKindData::Line => stacked(
            ChartType::Line,
            ChartType::LineStacked,
            ChartType::LinePercentStacked,
        ),
        ChartKindData::Area => stacked(
            ChartType::Area,
            ChartType::AreaStacked,
            ChartType::AreaPercentStacked,
        ),
        ChartKindData::Pie => {
            if c.hole.is_some() {
                ChartType::Doughnut
            } else {
                ChartType::Pie
            }
        }
        ChartKindData::Scatter | ChartKindData::Bubble => match (c.lines, c.markers, c.smooth) {
            (false, _, _) => ChartType::Scatter,
            (true, true, true) => ChartType::ScatterSmoothWithMarkers,
            (true, false, true) => ChartType::ScatterSmooth,
            (true, true, false) => ChartType::ScatterStraightWithMarkers,
            (true, false, false) => ChartType::ScatterStraight,
        },
        ChartKindData::Radar => {
            if c.filled {
                ChartType::RadarFilled
            } else if c.markers {
                ChartType::RadarWithMarkers
            } else {
                ChartType::Radar
            }
        }
        ChartKindData::Stock => ChartType::Stock,
    }
}

fn legend_position(l: LegendPosData) -> Option<ChartLegendPosition> {
    match l {
        LegendPosData::None => None,
        LegendPosData::Left => Some(ChartLegendPosition::Left),
        LegendPosData::Right => Some(ChartLegendPosition::Right),
        LegendPosData::Top => Some(ChartLegendPosition::Top),
        LegendPosData::Bottom => Some(ChartLegendPosition::Bottom),
    }
}

fn marker_type(m: MarkerShapeData) -> Option<ChartMarkerType> {
    match m {
        MarkerShapeData::Auto | MarkerShapeData::None => None,
        MarkerShapeData::Circle => Some(ChartMarkerType::Circle),
        MarkerShapeData::Square => Some(ChartMarkerType::Square),
        MarkerShapeData::Diamond => Some(ChartMarkerType::Diamond),
        MarkerShapeData::Triangle => Some(ChartMarkerType::Triangle),
        MarkerShapeData::Cross => Some(ChartMarkerType::X),
        MarkerShapeData::Plus => Some(ChartMarkerType::X),
    }
}

fn label_position(p: LabelPosData, kind: ChartKindData) -> Option<ChartDataLabelPosition> {
    let bar_like = matches!(
        kind,
        ChartKindData::Column | ChartKindData::Bar | ChartKindData::Pie
    );
    match p {
        LabelPosData::Auto => None,
        LabelPosData::Outside => Some(if bar_like {
            ChartDataLabelPosition::OutsideEnd
        } else {
            ChartDataLabelPosition::Above
        }),
        LabelPosData::Inside => Some(if bar_like {
            ChartDataLabelPosition::InsideEnd
        } else {
            ChartDataLabelPosition::Below
        }),
        LabelPosData::Center => Some(ChartDataLabelPosition::Center),
        LabelPosData::Above => Some(if bar_like {
            ChartDataLabelPosition::OutsideEnd
        } else {
            ChartDataLabelPosition::Above
        }),
        LabelPosData::Below => Some(if bar_like {
            ChartDataLabelPosition::InsideBase
        } else {
            ChartDataLabelPosition::Below
        }),
    }
}

fn date_unit(u: TimeUnitData) -> rust_xlsxwriter::ChartAxisDateUnitType {
    match u {
        TimeUnitData::Days => rust_xlsxwriter::ChartAxisDateUnitType::Days,
        TimeUnitData::Months => rust_xlsxwriter::ChartAxisDateUnitType::Months,
        TimeUnitData::Years => rust_xlsxwriter::ChartAxisDateUnitType::Years,
    }
}

/// The scale type of the category axis: `c:catAx` with `c:auto` for
/// automatic, without it for text, `c:dateAx` for a date axis (and for an
/// automatic one with date intervals, with `c:auto`). The date resolution
/// (`c:baseTimeUnit`) is not written: rust_xlsxwriter has no setter.
fn apply_category_type(axis: &mut rust_xlsxwriter::ChartAxis, a: &ChartAxisData) {
    let timed = a.major_time.is_some() || a.minor_time.is_some();
    match a.axis_type {
        AxisTypeData::Text => {
            axis.set_text_axis(true);
        }
        AxisTypeData::Auto if !timed => {}
        AxisTypeData::Auto | AxisTypeData::Date => {
            axis.set_date_axis(true);
            if a.axis_type == AxisTypeData::Auto {
                axis.set_automatic_axis(true);
            }
            if let Some(v) = a.min {
                axis.set_min(v);
            }
            if let Some(v) = a.max {
                axis.set_max(v);
            }
            let unit = |t: TimeIntervalData| (t.number, date_unit(t.unit));
            if let Some((n, u)) = a.major_time.map(unit) {
                axis.set_major_unit(n);
                axis.set_major_unit_date_type(u);
            }
            if let Some((n, u)) = a.minor_time.map(unit) {
                axis.set_minor_unit(n);
                axis.set_minor_unit_date_type(u);
            }
        }
    }
}

/// `c:crosses`/`c:crossesAt` of `a` (rust_xlsxwriter writes an axis'
/// crossing from the other axis' setting, so this goes on `other`).
/// `category`: `other` is a text category axis (crossing by category
/// number).
fn apply_crossing(other: &mut rust_xlsxwriter::ChartAxis, a: &ChartAxisData, category: bool) {
    use rust_xlsxwriter::ChartAxisCrossing as X;
    let crossing = match a.crosses {
        None => return,
        Some(AxisCrossData::Start) => X::Min,
        Some(AxisCrossData::End) => X::Max,
        Some(AxisCrossData::Value(v)) if category => X::CategoryNumber(v.round().max(1.0) as u32),
        Some(AxisCrossData::Value(v)) => X::AxisValue(v),
    };
    other.set_crossing(crossing);
}

fn apply_axis(axis: &mut rust_xlsxwriter::ChartAxis, a: &ChartAxisData, value_axis: bool) {
    match a.label_position {
        AxisLabelPosData::NearAxis | AxisLabelPosData::NearAxisOtherSide => {}
        AxisLabelPosData::OutsideStart => {
            axis.set_label_position(rust_xlsxwriter::ChartAxisLabelPosition::Low);
        }
        AxisLabelPosData::OutsideEnd => {
            axis.set_label_position(rust_xlsxwriter::ChartAxisLabelPosition::High);
        }
    }
    if let Some(t) = &a.title {
        axis.set_name(t.as_str());
    }
    if !a.visible {
        axis.set_hidden(true);
    }
    if value_axis {
        if let Some(v) = a.min {
            axis.set_min(v);
        }
        if let Some(v) = a.max {
            axis.set_max(v);
        }
        if let Some(v) = a.major {
            axis.set_major_unit(v);
        }
        if a.log {
            axis.set_log_base(10);
        }
    }
    if a.reverse {
        axis.set_reverse();
    }
    axis.set_major_gridlines(a.major_grid);
    axis.set_minor_gridlines(a.minor_grid);
    if let Some(f) = &a.number_format {
        axis.set_num_format(f.as_str());
    }
}

fn series_fill(color: u32, line_like: bool) -> ChartFormat {
    let mut f = ChartFormat::new();
    if line_like {
        f.set_line(ChartLine::new().set_color(rgb(color)));
    } else {
        f.set_solid_fill(ChartSolidFill::new().set_color(rgb(color)));
    }
    f
}

fn add_series(
    chart: &mut Chart,
    c: &ChartData,
    s: &ChartSeriesData,
    kind: ChartKindData,
    categories: Option<&ChartRange>,
    resolver: &XlsxResolver,
    report: &mut dyn Warn,
) -> bool {
    let name = c.name.as_str();
    let values = match chart_range(&s.values, resolver, name, "value", report) {
        Some(v) => v,
        // Own data: the literal values.
        None if s.values.is_unset() && s.literal_values.is_some() => literal_range(
            s.literal_values
                .as_ref()
                .unwrap_or(&ChartValuesData::Literal(Vec::new())),
        ),
        None => {
            if s.values.invalid {
                report.warn(format!(
                    "chart {name:?}: a series with a #REF! range was dropped"
                ));
            }
            return false;
        }
    };
    let ser = chart.add_series();
    ser.set_values(&values);
    let xy = matches!(kind, ChartKindData::Scatter | ChartKindData::Bubble);
    if xy {
        if let Some(x) = s
            .x_values
            .as_ref()
            .and_then(|r| chart_range(r, resolver, name, "X", report))
        {
            ser.set_categories(&x);
        } else if let Some(x) = s.literal_x.as_ref().filter(|_| s.x_values.is_none()) {
            ser.set_categories(&literal_range(x));
        }
    } else if let Some(cat) = categories {
        ser.set_categories(cat);
    } else if let Some(cat) = c
        .literal_categories
        .as_ref()
        .filter(|_| c.categories.is_none())
    {
        ser.set_categories(&literal_range(cat));
    }
    match &s.name {
        Some(SeriesNameData::Ref(r)) => {
            if let Some(n) = chart_range(r, resolver, name, "name", report) {
                ser.set_name(&n);
            }
        }
        Some(SeriesNameData::Text(t)) => {
            ser.set_name(t.as_str());
        }
        None => {}
    }
    if s.secondary_axis {
        ser.set_secondary_axis(true);
    }
    let line_like = matches!(
        kind,
        ChartKindData::Line | ChartKindData::Scatter | ChartKindData::Radar | ChartKindData::Stock
    );
    if let Some(col) = s.color {
        ser.set_format(&mut series_fill(col, line_like));
    }
    if line_like {
        let mut marker = ChartMarker::new();
        match s.marker {
            Some(MarkerShapeData::None) => {
                marker.set_none();
                ser.set_marker(&marker);
            }
            Some(m) => {
                if let Some(t) = marker_type(m) {
                    marker.set_type(t);
                    ser.set_marker(&marker);
                } else if c.markers {
                    marker.set_automatic();
                    ser.set_marker(&marker);
                } else if kind == ChartKindData::Line {
                    marker.set_none();
                    ser.set_marker(&marker);
                }
            }
            None => {
                if kind == ChartKindData::Line && !c.markers {
                    marker.set_none();
                    ser.set_marker(&marker);
                } else if c.markers && kind == ChartKindData::Line {
                    marker.set_automatic();
                    ser.set_marker(&marker);
                }
            }
        }
        if c.smooth && kind == ChartKindData::Line {
            ser.set_smooth(true);
        }
    }
    let l = &s.labels;
    if l.value || l.percent || l.category || l.series_name {
        let mut dl = ChartDataLabel::new();
        if l.value {
            dl.show_value();
        }
        if l.percent && kind == ChartKindData::Pie {
            dl.show_percentage();
        }
        if l.category {
            dl.show_category_name();
        }
        if l.series_name {
            dl.show_series_name();
        }
        if let Some(p) = label_position(l.position, kind) {
            dl.set_position(p);
        }
        ser.set_data_label(&dl);
    }
    if let Some(t) = &s.trend {
        let mut tl = ChartTrendline::new();
        tl.set_type(match t.kind {
            TrendKindData::Linear => ChartTrendlineType::Linear,
            TrendKindData::Logarithmic => ChartTrendlineType::Logarithmic,
            TrendKindData::Exponential => ChartTrendlineType::Exponential,
            TrendKindData::Power => ChartTrendlineType::Power,
        });
        tl.display_equation(t.show_equation);
        tl.display_r_squared(t.show_r2);
        if let Some(n) = &t.name {
            tl.set_name(n.as_str());
        }
        ser.set_trendline(&tl);
    }
    if let Some(e) = &s.error_bars {
        let ty = match &e.kind {
            ErrorBarKindData::Constant { plus, minus } => {
                if (plus - minus).abs() > f64::EPSILON {
                    report.warn(format!(
                        "chart {name:?}: asymmetric constant error bars use the positive length"
                    ));
                }
                Some(ChartErrorBarsType::FixedValue(*plus))
            }
            ErrorBarKindData::Percent { percent } => Some(ChartErrorBarsType::Percentage(*percent)),
            ErrorBarKindData::StdDev { k } => Some(ChartErrorBarsType::StandardDeviation(*k)),
            ErrorBarKindData::StdError => Some(ChartErrorBarsType::StandardError),
            ErrorBarKindData::Range { plus, minus } => {
                match (
                    chart_range(plus, resolver, name, "error bar", report),
                    chart_range(minus, resolver, name, "error bar", report),
                ) {
                    (Some(p), Some(m)) => Some(ChartErrorBarsType::Custom(p, m)),
                    _ => None,
                }
            }
            ErrorBarKindData::Variance | ErrorBarKindData::ErrorMargin { .. } => {
                report.warn(format!(
                    "chart {name:?}: variance and error-margin error bars are not supported in XLSX and were dropped"
                ));
                None
            }
        };
        if let Some(ty) = ty {
            let mut eb = ChartErrorBars::new();
            eb.set_type(ty);
            eb.set_direction(match e.direction {
                ErrorBarDirData::Both => ChartErrorBarsDirection::Both,
                ErrorBarDirData::Plus => ChartErrorBarsDirection::Plus,
                ErrorBarDirData::Minus => ChartErrorBarsDirection::Minus,
            });
            ser.set_y_error_bars(&eb);
        }
    }
    if s.mean_line {
        report.warn(format!(
            "chart {name:?}: mean value lines are not supported in XLSX and were dropped"
        ));
    }
    true
}

/// Builds the `rust_xlsxwriter` chart of a chart definition.
pub(crate) fn build_chart(
    c: &ChartData,
    resolver: &XlsxResolver,
    report: &mut dyn Warn,
) -> Option<Chart> {
    let name = c.name.as_str();
    let mut kind = c.kind;
    if kind == ChartKindData::Bubble {
        report.warn(format!(
            "chart {name:?}: bubble charts are written as scatter charts (sizes dropped)"
        ));
        kind = ChartKindData::Scatter;
    }
    if c.three_d {
        report.warn(format!("chart {name:?}: 3D charts are written flat"));
    }
    let mut chart = Chart::new(chart_type(kind, c));
    let refs = c.series.len().min(MAX_SERIES) as u64;
    let categories = c
        .categories
        .as_ref()
        .and_then(|r| chart_range_refs(r, resolver, name, "category", report, refs));
    // Series of the main kind go into the main chart; others into one
    // combined chart per kind (Excel supports one secondary group).
    let mut series: Vec<&ChartSeriesData> = c.series.iter().take(MAX_SERIES).collect();
    if c.series.len() > MAX_SERIES {
        report.warn(format!(
            "chart {name:?}: only the first {MAX_SERIES} series were written"
        ));
    }
    if kind == ChartKindData::Stock {
        let volume = matches!(
            c.stock,
            Some(StockVariantData::VolumeHlc | StockVariantData::VolumeOhlc)
        );
        if volume && !series.is_empty() {
            report.warn(format!(
                "chart {name:?}: the volume series of a stock chart was dropped"
            ));
            series.remove(0);
        }
        chart.set_high_low_lines(true);
        if matches!(
            c.stock,
            Some(StockVariantData::Ohlc | StockVariantData::VolumeOhlc)
        ) {
            chart.set_up_down_bars(true);
        }
    }
    let mut written = 0usize;
    let mut other: Option<(ChartKindData, Chart)> = None;
    for s in series {
        let sk = s.kind.unwrap_or(kind);
        let sk = if sk == ChartKindData::Bubble {
            ChartKindData::Scatter
        } else {
            sk
        };
        if sk == kind || kind == ChartKindData::Stock || kind == ChartKindData::Pie {
            if add_series(
                &mut chart,
                c,
                s,
                kind,
                categories.as_ref(),
                resolver,
                report,
            ) {
                written += 1;
            }
        } else {
            let target = match &mut other {
                Some((k, ch)) if *k == sk => ch,
                Some(_) => {
                    report.warn(format!(
                        "chart {name:?}: Excel combines two chart types at most; a series was written in the main type"
                    ));
                    &mut chart
                }
                None => {
                    other = Some((sk, Chart::new(chart_type(sk, c))));
                    &mut other.as_mut().map(|(_, ch)| ch).unwrap_or(&mut chart)
                }
            };
            if add_series(target, c, s, sk, categories.as_ref(), resolver, report) {
                written += 1;
            }
        }
    }
    if written == 0 {
        report.warn(format!(
            "chart {name:?} has no plottable series and was not written"
        ));
        return None;
    }
    if let Some((_, o)) = &other {
        chart.combine(o);
    }
    match (&c.title, &c.subtitle) {
        (Some(t), Some(s)) => {
            chart.title().set_name(format!("{t}\n{s}").as_str());
        }
        (Some(t), None) => {
            chart.title().set_name(t.as_str());
        }
        (None, Some(s)) => {
            chart.title().set_name(s.as_str());
        }
        (None, None) => {
            chart.title().set_hidden();
        }
    }
    match legend_position(c.legend) {
        Some(p) => {
            chart.legend().set_position(p);
        }
        None => {
            chart.legend().set_hidden();
        }
    }
    let xy = matches!(kind, ChartKindData::Scatter);
    apply_axis(chart.x_axis(), &c.x_axis, xy);
    apply_axis(chart.y_axis(), &c.y_axis, true);
    let category_x = !xy && !matches!(kind, ChartKindData::Pie | ChartKindData::Radar);
    if category_x {
        apply_category_type(chart.x_axis(), &c.x_axis);
    }
    let text_x = category_x
        && match c.x_axis.axis_type {
            AxisTypeData::Text => true,
            AxisTypeData::Auto => c.x_axis.major_time.is_none() && c.x_axis.minor_time.is_none(),
            AxisTypeData::Date => false,
        };
    apply_crossing(chart.y_axis(), &c.x_axis, false);
    apply_crossing(chart.x_axis(), &c.y_axis, text_x);
    if let Some(y2) = &c.y2_axis {
        apply_axis(chart.y2_axis(), y2, true);
    }
    if let Some(h) = c.hole {
        if kind == ChartKindData::Pie {
            chart.set_hole_size((h * 100.0).round().clamp(10.0, 90.0) as u8);
        }
    }
    if let Some(bg) = c.background {
        chart.chart_area().set_format(
            ChartFormat::new().set_solid_fill(ChartSolidFill::new().set_color(rgb(bg))),
        );
    }
    if let Some(bg) = c.plot_background {
        chart.plot_area().set_format(
            ChartFormat::new().set_solid_fill(ChartSolidFill::new().set_color(rgb(bg))),
        );
    }
    if c.include_hidden {
        chart.show_hidden_data();
    }
    chart.set_width(hmm_to_px(c.anchor.width_hmm).max(10));
    chart.set_height(hmm_to_px(c.anchor.height_hmm).max(10));
    Some(chart)
}

/// The pixel offset of an object at `(row, col)`: its own, moved right
/// one pixel at a time while another object of the sheet already holds
/// that exact spot (rust_xlsxwriter keeps one object per spot).
fn free_offset(
    taken: &mut HashSet<(u32, u16, u32, u32)>,
    row: u32,
    col: u16,
    dx_hmm: u32,
    dy_hmm: u32,
) -> (u32, u32) {
    let (mut dx, dy) = (hmm_to_px(dx_hmm), hmm_to_px(dy_hmm));
    while !taken.insert((row, col, dx, dy)) {
        dx = dx.saturating_add(1);
    }
    (dx, dy)
}

/// Writes the sheet's charts at their anchors.
pub(crate) fn write_charts(
    ws: &mut Worksheet,
    sheet: &SheetData,
    resolver: &XlsxResolver,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    let mut taken = HashSet::new();
    for c in &sheet.charts {
        let Some(chart) = build_chart(c, resolver, report) else {
            continue;
        };
        let a = &c.anchor;
        if a.row >= tally_types::MAX_ROWS || a.col >= tally_types::MAX_COLS {
            continue;
        }
        let col = u16::try_from(a.col).unwrap_or(u16::MAX);
        let (dx, dy) = free_offset(&mut taken, a.row, col, a.dx_hmm, a.dy_hmm);
        if let Err(e) = ws.insert_chart_with_offset(a.row, col, &chart, dx, dy) {
            report.warn(format!("chart {:?} was not written: {e}", c.name));
        }
    }
    Ok(())
}

/// Writes the sheet's pictures at their anchors.
pub(crate) fn write_images(
    ws: &mut Worksheet,
    sheet: &SheetData,
    report: &mut dyn Warn,
) -> Result<(), XlsxError> {
    let mut taken = HashSet::new();
    for img in &sheet.images {
        let a = &img.anchor;
        if a.row >= tally_types::MAX_ROWS || a.col >= tally_types::MAX_COLS {
            continue;
        }
        if img.bytes.is_empty() || img.bytes.len() > 16 * 1024 * 1024 {
            report.warn(format!(
                "picture {:?} was not written (empty or over 16 MiB)",
                img.name
            ));
            continue;
        }
        let image = match Image::new_from_buffer(&img.bytes) {
            Ok(i) => i,
            Err(e) => {
                report.warn(format!("picture {:?} was not written: {e}", img.name));
                continue;
            }
        };
        let image = image.set_scale_to_size(
            f64::from(hmm_to_px(a.width_hmm).max(1)),
            f64::from(hmm_to_px(a.height_hmm).max(1)),
            false,
        );
        let col = u16::try_from(a.col).unwrap_or(u16::MAX);
        let (dx, dy) = free_offset(&mut taken, a.row, col, a.dx_hmm, a.dy_hmm);
        if let Err(e) = ws.insert_image_with_offset(a.row, col, &image, dx, dy) {
            report.warn(format!("picture {:?} was not written: {e}", img.name));
        }
    }
    Ok(())
}
