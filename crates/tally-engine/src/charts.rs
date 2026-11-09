//! Charts and pictures (wave 5, W5-1): DTO ⇄ model conversion,
//! validation, data resolution, trend and error-bar math, structure
//! adjustment and the change detection behind `ApplyResult.charts_changed`.
//!
//! Inside the engine a chart is the interchange [`ChartData`] with one
//! difference: the `u32` of every `(sheet, range)` area is the **sheet
//! id**, not the tab index (`from_data`/`to_data` translate). Ids never
//! move or get reused, so a chart follows sheet moves for free and a
//! series whose sheet was deleted is detected lazily (`wb.sheet(id)` is
//! `None` → `#REF!`); undoing the deletion re-inserts the sheet with its
//! id and the series is valid again.
//!
//! Range text in the API is the UI dialect (`$Sheet1.$B$2:$B$9`, unions
//! with `~`); `#REF!` is the text of an invalid range.

use std::sync::Arc;

use tally_api::{
    ApiError, AxisCross, AxisLabelPos, AxisMarkPos, AxisType, ChartAnchor, ChartAxisDto,
    ChartDataDto, ChartDataTableDto, ChartDto, ChartKind, ChartRef, ChartSeriesDto, ChartSourceDto,
    DataLabelsDto, DataTableSeriesDto, DateAxisDto, ErrorBarDir, ErrorBarKind, ErrorBarsDto,
    LabelPos, LegendPos, MarkerShape, RangeDto, SeriesDataDto, SeriesName, Stacking, StockVariant,
    TimeIntervalDto, TimeUnit, TrendDataDto, TrendKind, TrendLineDto,
};
use tally_formula::{
    print_expr, BinOp, CellRefExpr, Expr, RefCoord, RefExpr, ResolvedRef, SheetRef,
};
use tally_model::{
    AxisCrossData, AxisLabelPosData, AxisMarkPosData, AxisTypeData, CellRangeLocal,
    ChartAnchorData, ChartAxisData, ChartData, ChartKindData, ChartRangeData, ChartSeriesData,
    ChartSourceData, ChartValuesData, DataLabelsData, ErrorBarDirData, ErrorBarKindData,
    ErrorBarsData, ImageData, LabelPosData, LegendPosData, MarkerShapeData, SeriesNameData,
    StackingData, StockVariantData, TimeIntervalData, TimeUnitData, TrendKindData, TrendLineData,
};
use tally_types::{CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::storage::SheetMeta;
use crate::structure::{point_after, span_after, Shape};
use crate::workbook::Workbook;

/// A chart with its sheet-local id.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The definition (resolved ranges; area sheets are sheet ids).
    pub data: ChartData,
}

/// A picture with its sheet-local id. The list holds `Arc<Image>`, so
/// cloning the sheet's picture list (undo snapshots) never copies the
/// bytes; only `to_data`/`to_api` with data do.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The picture (name, anchor, media type, bytes).
    pub data: ImageData,
}

/// Most charts per document (reader and `AddChart` limit).
pub const MAX_CHARTS: usize = 1_000;
/// Most pictures per sheet.
pub const MAX_IMAGES: usize = 1_000;
/// Most series per chart.
pub const MAX_SERIES: usize = 255;
/// Most points per series in `chart_data`.
pub const MAX_POINTS: usize = 32_000;
/// Most points in one `chart_data` response (categories, values, X
/// values and sizes of every series together).
pub const MAX_CHART_POINTS: usize = 100_000;
/// Most cells one `chart_data` call visits, hidden ones included.
pub const MAX_CHART_VISITS: usize = 2_000_000;
/// Most points of a trend line in `chart_data` (evenly sampled).
pub const MAX_TREND_POINTS: usize = 500;
/// Most areas in one range (`~` union).
pub const MAX_AREAS: usize = 64;
/// Longest title, subtitle, axis title, series name or format code.
pub const MAX_TEXT: usize = 4_096;
/// Largest picture (decoded bytes).
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Largest object size in 1/100 mm (10 m).
pub const MAX_SIZE_HMM: u32 = 1_000_000;

/// AOO's default series colours (`0xRRGGBB`).
pub const PALETTE: [u32; 12] = [
    0x004586, 0xff420e, 0xffd320, 0x579d1c, 0x7e0021, 0x83caff, 0x314004, 0xaecf00, 0x4b1f6f,
    0xff950e, 0xc5000b, 0x0084d1,
];

/// The palette colour of series `i` as `0xRRGGBBAA`.
pub fn palette_color(i: usize) -> u32 {
    (PALETTE[i % PALETTE.len()] << 8) | 0xff
}

// ---------------------------------------------------------------------------
// Interchange ⇄ engine (tab index ⇄ sheet id)
// ---------------------------------------------------------------------------

fn map_range(r: &ChartRangeData, map: &dyn Fn(u32) -> Option<u32>) -> ChartRangeData {
    let mut out = ChartRangeData {
        areas: Vec::with_capacity(r.areas.len()),
        invalid: r.invalid,
    };
    let had = !r.areas.is_empty();
    for (s, a) in &r.areas {
        if let Some(n) = map(*s) {
            out.areas.push((n, *a));
        }
    }
    if had && out.areas.is_empty() {
        out.invalid = true;
    }
    out
}

fn map_chart_ranges(c: &ChartData, map: &dyn Fn(u32) -> Option<u32>) -> ChartData {
    let mut out = c.clone();
    if let Some(s) = &mut out.source {
        s.range = map_range(&s.range, map);
    }
    if let Some(r) = &mut out.categories {
        *r = map_range(r, map);
    }
    for s in &mut out.series {
        s.values = map_range(&s.values, map);
        if let Some(x) = &mut s.x_values {
            *x = map_range(x, map);
        }
        if let Some(z) = &mut s.sizes {
            *z = map_range(z, map);
        }
        if let Some(SeriesNameData::Ref(r)) = &mut s.name {
            *r = map_range(r, map);
        }
        if let Some(ErrorBarsData {
            kind: ErrorBarKindData::Range { plus, minus },
            ..
        }) = &mut s.error_bars
        {
            *plus = map_range(plus, map);
            *minus = map_range(minus, map);
        }
    }
    out
}

/// Loads a sheet's charts and pictures from the interchange model,
/// allocating ids. Tab indices become sheet ids; ranges on unknown tabs
/// become `#REF!`. Document totals hold whatever the reader did: at most
/// [`MAX_CHARTS`] charts, [`tally_model::MAX_SERIES_PER_DOCUMENT`]
/// series, [`tally_model::MAX_LITERAL_POINTS`] literal
/// points (charts past it lose their literal values) and
/// [`crate::images::MAX_TOTAL_IMAGE_BYTES`] of pictures, each counted
/// per reference; what is dropped is reported once in the import
/// warnings.
pub fn from_data(wb: &mut Workbook, sheet: SheetId, charts: &[ChartData], images: &[ImageData]) {
    if charts.is_empty() && images.is_empty() {
        return;
    }
    let ids: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    let tab_to_id = |t: u32| ids.get(t as usize).copied();
    let chart_total = chart_count(wb);
    let mut literal_total: usize = wb
        .sheets
        .iter()
        .flat_map(|s| s.meta.charts.iter())
        .map(|c| c.data.literal_points())
        .sum();
    let mut series_total: usize = wb
        .sheets
        .iter()
        .flat_map(|s| s.meta.charts.iter())
        .map(|c| c.data.series.len())
        .sum();
    let mut image_total = crate::images::total_image_bytes(wb);
    let mut dropped = Vec::new();
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    let room = MAX_CHARTS.saturating_sub(chart_total);
    if charts.len() > room {
        dropped.push(format!(
            "Only the first {MAX_CHARTS} charts of the document were loaded."
        ));
    }
    let mut cs = Vec::with_capacity(charts.len().min(room));
    for c in charts.iter().take(room) {
        let id = s.meta.alloc_id();
        let mut data = map_chart_ranges(c, &tab_to_id);
        clamp_anchor(&mut data.anchor);
        data.series.truncate(MAX_SERIES);
        let room = tally_model::MAX_SERIES_PER_DOCUMENT.saturating_sub(series_total);
        if data.series.len() > room {
            data.series.truncate(room);
            dropped.push(format!(
                "Charts past {} series per document were loaded without their extra series.",
                tally_model::MAX_SERIES_PER_DOCUMENT
            ));
        }
        series_total += data.series.len();
        let n = data.literal_points();
        if literal_total.saturating_add(n) > tally_model::MAX_LITERAL_POINTS {
            data.drop_literals();
            dropped.push(format!(
                "Chart cached values past {} points per document were not loaded.",
                tally_model::MAX_LITERAL_POINTS
            ));
        } else {
            literal_total += n;
        }
        cs.push(Arc::new(Chart { id, data }));
    }
    if !cs.is_empty() {
        Arc::make_mut(&mut s.meta.charts).extend(cs);
    }
    let mut is = Vec::new();
    for i in images.iter().take(MAX_IMAGES) {
        if i.bytes.len() > MAX_IMAGE_BYTES || crate::images::sniff_mime(&i.bytes).is_none() {
            continue;
        }
        if image_total.saturating_add(i.bytes.len()) > crate::images::MAX_TOTAL_IMAGE_BYTES {
            dropped.push("Pictures past 256 MiB per document were not loaded.".to_string());
            continue;
        }
        image_total += i.bytes.len();
        let id = s.meta.alloc_id();
        let mut anchor = i.anchor;
        clamp_anchor(&mut anchor);
        is.push(Arc::new(Image {
            id,
            data: ImageData {
                name: i.name.clone(),
                anchor,
                mime: crate::images::sniff_mime(&i.bytes)
                    .unwrap_or("image/png")
                    .to_string(),
                bytes: i.bytes.clone(),
            },
        }));
    }
    if !is.is_empty() {
        Arc::make_mut(&mut s.meta.images).extend(is);
    }
    for w in dropped {
        if !wb.import_warnings.contains(&w) {
            wb.import_warnings.push(w);
        }
    }
}

fn clamp_anchor(a: &mut ChartAnchorData) {
    a.row = a.row.min(MAX_ROWS - 1);
    a.col = a.col.min(MAX_COLS - 1);
    a.dx_hmm = a.dx_hmm.min(MAX_SIZE_HMM);
    a.dy_hmm = a.dy_hmm.min(MAX_SIZE_HMM);
    if a.width_hmm == 0 || a.width_hmm > MAX_SIZE_HMM {
        a.width_hmm = 16_000;
    }
    if a.height_hmm == 0 || a.height_hmm > MAX_SIZE_HMM {
        a.height_hmm = 9_000;
    }
}

/// The interchange form of a sheet's charts (sheet ids become tab
/// indices; ranges on deleted sheets become `#REF!`).
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<ChartData> {
    let id_to_tab = |id: u32| wb.tab_of(id).map(|t| t as u32);
    wb.sheet(sheet)
        .map(|s| {
            s.meta
                .charts
                .iter()
                .map(|c| map_chart_ranges(&c.data, &id_to_tab))
                .collect()
        })
        .unwrap_or_default()
}

/// The interchange form of a sheet's pictures.
pub fn images_to_data(wb: &Workbook, sheet: SheetId) -> Vec<ImageData> {
    wb.sheet(sheet)
        .map(|s| s.meta.images.iter().map(|i| i.data.clone()).collect())
        .unwrap_or_default()
}

/// For `CopySheet`: `meta` is the copy's meta (cloned from the source
/// sheet). Areas that pointed at the source now point at the copy (a
/// chart of its own sheet's data keeps plotting its own sheet, as in
/// AOO); ids are kept (they stay unique per sheet).
pub fn clone_for_sheet(meta: &mut SheetMeta, old_sheet: SheetId, new_sheet: SheetId) {
    if meta.charts.is_empty() {
        return;
    }
    let map = |s: u32| Some(if s == old_sheet { new_sheet } else { s });
    let list = Arc::make_mut(&mut meta.charts);
    for c in list.iter_mut() {
        let data = map_chart_ranges(&c.data, &map);
        if data != c.data {
            *c = Arc::new(Chart { id: c.id, data });
        }
    }
}

// ---------------------------------------------------------------------------
// Range text ⇄ ranges
// ---------------------------------------------------------------------------

/// The text of an invalid range.
pub const REF_ERROR: &str = "#REF!";

fn collect_areas(
    wb: &Workbook,
    e: &Expr,
    origin: CellAddr,
    out: &mut Vec<(u32, CellRangeLocal)>,
) -> Result<(), ApiError> {
    let bad = || ApiError::invalid_argument("not a cell range");
    match e {
        Expr::Paren(inner) => collect_areas(wb, inner, origin, out),
        Expr::Binary(BinOp::Union, a, b) => {
            collect_areas(wb, a, origin, out)?;
            collect_areas(wb, b, origin, out)
        }
        Expr::Name(_) => {
            let r = crate::names::resolve_chain(&wb.names, e, origin.sheet);
            if matches!(r, Expr::Name(_)) {
                return Err(bad());
            }
            collect_areas(wb, r, origin, out)
        }
        Expr::Ref(r) => match r.resolve(origin) {
            ResolvedRef::Range(cr) if wb.sheet(cr.sheet).is_some() => {
                if out.len() >= MAX_AREAS {
                    return Err(ApiError::invalid_argument(format!(
                        "a chart range may have at most {MAX_AREAS} areas"
                    )));
                }
                out.push((
                    cr.sheet,
                    CellRangeLocal {
                        r0: cr.r0,
                        c0: cr.c0,
                        r1: cr.r1,
                        c1: cr.c1,
                    },
                ));
                Ok(())
            }
            _ => Err(bad()),
        },
        _ => Err(bad()),
    }
}

/// Parses range text in the UI dialect (`$Sheet1.$B$2:$B$9`, `~` unions,
/// names) relative to `sheet`. Empty text is "not set"; `#REF!` is an
/// invalid range.
pub fn parse_range_text(
    wb: &Workbook,
    sheet: SheetId,
    text: &str,
) -> Result<ChartRangeData, ApiError> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(ChartRangeData::default());
    }
    if t.eq_ignore_ascii_case(REF_ERROR) {
        return Ok(ChartRangeData {
            areas: Vec::new(),
            invalid: true,
        });
    }
    if t.len() > MAX_TEXT || t.starts_with('=') {
        return Err(ApiError::invalid_argument(format!("bad range {t:?}")));
    }
    let origin = CellAddr::new(sheet, 0, 0);
    let f = wb
        .parse_formula(&format!("={t}"), origin)
        .map_err(|_| ApiError::invalid_argument(format!("bad range {t:?}")))?;
    let mut areas = Vec::new();
    collect_areas(wb, &f.expr, origin, &mut areas)
        .map_err(|e| ApiError::invalid_argument(format!("bad range {t:?}: {}", e.message)))?;
    Ok(ChartRangeData {
        areas,
        invalid: false,
    })
}

/// A single-cell reference (series name cell).
pub fn parse_cell_text(
    wb: &Workbook,
    sheet: SheetId,
    text: &str,
) -> Result<ChartRangeData, ApiError> {
    let r = parse_range_text(wb, sheet, text)?;
    if r.areas.len() > 1
        || r.areas
            .first()
            .is_some_and(|(_, a)| a.r0 != a.r1 || a.c0 != a.c1)
    {
        return Err(ApiError::invalid_argument(format!(
            "{text:?} is not a single cell"
        )));
    }
    Ok(r)
}

fn area_expr(sheet: SheetId, a: &CellRangeLocal) -> Expr {
    let cell = |r: u32, c: u32, sheet: SheetRef| CellRefExpr {
        sheet,
        row: RefCoord::absolute(r as i32),
        col: RefCoord::absolute(c as i32),
    };
    let start = SheetRef::Id {
        id: sheet,
        abs: true,
    };
    if a.r0 == a.r1 && a.c0 == a.c1 {
        Expr::Ref(RefExpr::Cell(cell(a.r0, a.c0, start)))
    } else {
        // The end endpoint is on the start's sheet (`$Sheet1.$B$2:$B$9`).
        Expr::Ref(RefExpr::Area(
            cell(a.r0, a.c0, start),
            cell(a.r1, a.c1, SheetRef::Current),
        ))
    }
}

/// The canonical UI text of a range (`#REF!` when invalid or when every
/// area's sheet is gone; empty when unset).
pub fn range_text(wb: &Workbook, sheet: SheetId, r: &ChartRangeData) -> String {
    if r.invalid {
        return REF_ERROR.to_string();
    }
    let origin = CellAddr::new(sheet, 0, 0);
    let parts: Vec<String> = r
        .areas
        .iter()
        .filter(|(s, _)| wb.sheet(*s).is_some())
        .map(|(s, a)| print_expr(&area_expr(*s, a), origin, wb.ui_dialect(), wb))
        .collect();
    if parts.is_empty() {
        if r.areas.is_empty() {
            String::new()
        } else {
            REF_ERROR.to_string()
        }
    } else {
        parts.join("~")
    }
}

/// `true` when the range is unusable: marked invalid, or every area's
/// sheet was deleted.
pub fn range_is_invalid(wb: &Workbook, r: &ChartRangeData) -> bool {
    r.invalid || (!r.areas.is_empty() && r.areas.iter().all(|(s, _)| wb.sheet(*s).is_none()))
}

// ---------------------------------------------------------------------------
// DTO ⇄ model
// ---------------------------------------------------------------------------

fn kind_from_api(k: ChartKind) -> ChartKindData {
    match k {
        ChartKind::Column => ChartKindData::Column,
        ChartKind::Bar => ChartKindData::Bar,
        ChartKind::Line => ChartKindData::Line,
        ChartKind::Area => ChartKindData::Area,
        ChartKind::Pie => ChartKindData::Pie,
        ChartKind::Scatter => ChartKindData::Scatter,
        ChartKind::Bubble => ChartKindData::Bubble,
        ChartKind::Radar => ChartKindData::Radar,
        ChartKind::Stock => ChartKindData::Stock,
    }
}

/// The API kind of a model kind.
pub fn kind_to_api(k: ChartKindData) -> ChartKind {
    match k {
        ChartKindData::Column => ChartKind::Column,
        ChartKindData::Bar => ChartKind::Bar,
        ChartKindData::Line => ChartKind::Line,
        ChartKindData::Area => ChartKind::Area,
        ChartKindData::Pie => ChartKind::Pie,
        ChartKindData::Scatter => ChartKind::Scatter,
        ChartKindData::Bubble => ChartKind::Bubble,
        ChartKindData::Radar => ChartKind::Radar,
        ChartKindData::Stock => ChartKind::Stock,
    }
}

fn stacking_from_api(s: Stacking) -> StackingData {
    match s {
        Stacking::None => StackingData::None,
        Stacking::Stacked => StackingData::Stacked,
        Stacking::Percent => StackingData::Percent,
    }
}

fn stacking_to_api(s: StackingData) -> Stacking {
    match s {
        StackingData::None => Stacking::None,
        StackingData::Stacked => Stacking::Stacked,
        StackingData::Percent => Stacking::Percent,
    }
}

fn stock_from_api(s: StockVariant) -> StockVariantData {
    match s {
        StockVariant::Hlc => StockVariantData::Hlc,
        StockVariant::Ohlc => StockVariantData::Ohlc,
        StockVariant::VolumeHlc => StockVariantData::VolumeHlc,
        StockVariant::VolumeOhlc => StockVariantData::VolumeOhlc,
    }
}

fn stock_to_api(s: StockVariantData) -> StockVariant {
    match s {
        StockVariantData::Hlc => StockVariant::Hlc,
        StockVariantData::Ohlc => StockVariant::Ohlc,
        StockVariantData::VolumeHlc => StockVariant::VolumeHlc,
        StockVariantData::VolumeOhlc => StockVariant::VolumeOhlc,
    }
}

fn legend_from_api(l: LegendPos) -> LegendPosData {
    match l {
        LegendPos::None => LegendPosData::None,
        LegendPos::Left => LegendPosData::Left,
        LegendPos::Right => LegendPosData::Right,
        LegendPos::Top => LegendPosData::Top,
        LegendPos::Bottom => LegendPosData::Bottom,
    }
}

fn legend_to_api(l: LegendPosData) -> LegendPos {
    match l {
        LegendPosData::None => LegendPos::None,
        LegendPosData::Left => LegendPos::Left,
        LegendPosData::Right => LegendPos::Right,
        LegendPosData::Top => LegendPos::Top,
        LegendPosData::Bottom => LegendPos::Bottom,
    }
}

fn marker_from_api(m: MarkerShape) -> MarkerShapeData {
    match m {
        MarkerShape::Auto => MarkerShapeData::Auto,
        MarkerShape::None => MarkerShapeData::None,
        MarkerShape::Circle => MarkerShapeData::Circle,
        MarkerShape::Square => MarkerShapeData::Square,
        MarkerShape::Diamond => MarkerShapeData::Diamond,
        MarkerShape::Triangle => MarkerShapeData::Triangle,
        MarkerShape::Cross => MarkerShapeData::Cross,
        MarkerShape::Plus => MarkerShapeData::Plus,
    }
}

fn marker_to_api(m: MarkerShapeData) -> MarkerShape {
    match m {
        MarkerShapeData::Auto => MarkerShape::Auto,
        MarkerShapeData::None => MarkerShape::None,
        MarkerShapeData::Circle => MarkerShape::Circle,
        MarkerShapeData::Square => MarkerShape::Square,
        MarkerShapeData::Diamond => MarkerShape::Diamond,
        MarkerShapeData::Triangle => MarkerShape::Triangle,
        MarkerShapeData::Cross => MarkerShape::Cross,
        MarkerShapeData::Plus => MarkerShape::Plus,
    }
}

fn label_pos_from_api(p: LabelPos) -> LabelPosData {
    match p {
        LabelPos::Auto => LabelPosData::Auto,
        LabelPos::Outside => LabelPosData::Outside,
        LabelPos::Inside => LabelPosData::Inside,
        LabelPos::Center => LabelPosData::Center,
        LabelPos::Above => LabelPosData::Above,
        LabelPos::Below => LabelPosData::Below,
    }
}

fn label_pos_to_api(p: LabelPosData) -> LabelPos {
    match p {
        LabelPosData::Auto => LabelPos::Auto,
        LabelPosData::Outside => LabelPos::Outside,
        LabelPosData::Inside => LabelPos::Inside,
        LabelPosData::Center => LabelPos::Center,
        LabelPosData::Above => LabelPos::Above,
        LabelPosData::Below => LabelPos::Below,
    }
}

fn trend_kind_from_api(k: TrendKind) -> TrendKindData {
    match k {
        TrendKind::Linear => TrendKindData::Linear,
        TrendKind::Logarithmic => TrendKindData::Logarithmic,
        TrendKind::Exponential => TrendKindData::Exponential,
        TrendKind::Power => TrendKindData::Power,
    }
}

fn trend_kind_to_api(k: TrendKindData) -> TrendKind {
    match k {
        TrendKindData::Linear => TrendKind::Linear,
        TrendKindData::Logarithmic => TrendKind::Logarithmic,
        TrendKindData::Exponential => TrendKind::Exponential,
        TrendKindData::Power => TrendKind::Power,
    }
}

fn dir_from_api(d: ErrorBarDir) -> ErrorBarDirData {
    match d {
        ErrorBarDir::Both => ErrorBarDirData::Both,
        ErrorBarDir::Plus => ErrorBarDirData::Plus,
        ErrorBarDir::Minus => ErrorBarDirData::Minus,
    }
}

fn dir_to_api(d: ErrorBarDirData) -> ErrorBarDir {
    match d {
        ErrorBarDirData::Both => ErrorBarDir::Both,
        ErrorBarDirData::Plus => ErrorBarDir::Plus,
        ErrorBarDirData::Minus => ErrorBarDir::Minus,
    }
}

fn mm_to_hmm(mm: f32) -> Result<u32, ApiError> {
    if !mm.is_finite() || mm < 0.0 || mm > (MAX_SIZE_HMM as f32) / 100.0 {
        return Err(ApiError::invalid_argument("bad object size or offset"));
    }
    Ok((mm * 100.0).round() as u32)
}

fn hmm_to_mm(hmm: u32) -> f32 {
    hmm as f32 / 100.0
}

/// Validates and converts a placement.
pub fn anchor_from_api(a: &ChartAnchor) -> Result<ChartAnchorData, ApiError> {
    if a.r >= MAX_ROWS || a.c >= MAX_COLS {
        return Err(ApiError::invalid_argument(
            "the anchor is outside the sheet",
        ));
    }
    let width_hmm = mm_to_hmm(a.width_mm)?;
    let height_hmm = mm_to_hmm(a.height_mm)?;
    if width_hmm == 0 || height_hmm == 0 {
        return Err(ApiError::invalid_argument("the object has no size"));
    }
    Ok(ChartAnchorData {
        row: a.r,
        col: a.c,
        dx_hmm: mm_to_hmm(a.dx_mm)?,
        dy_hmm: mm_to_hmm(a.dy_mm)?,
        width_hmm,
        height_hmm,
    })
}

/// The API view of a placement.
pub fn anchor_to_api(a: &ChartAnchorData) -> ChartAnchor {
    ChartAnchor {
        r: a.row,
        c: a.col,
        dx_mm: hmm_to_mm(a.dx_hmm),
        dy_mm: hmm_to_mm(a.dy_hmm),
        width_mm: hmm_to_mm(a.width_hmm),
        height_mm: hmm_to_mm(a.height_hmm),
    }
}

fn text_opt(t: &Option<String>, what: &str) -> Result<Option<String>, ApiError> {
    match t {
        Some(s) if s.chars().count() > MAX_TEXT => Err(ApiError::invalid_argument(format!(
            "{what} is longer than {MAX_TEXT} characters"
        ))),
        Some(s) => Ok(Some(s.clone())),
        None => Ok(None),
    }
}

fn number_opt(v: Option<f64>, what: &str) -> Result<Option<f64>, ApiError> {
    match v {
        Some(x) if !x.is_finite() => Err(ApiError::invalid_argument(format!(
            "{what} is not a number"
        ))),
        other => Ok(other),
    }
}

fn axis_from_api(a: &ChartAxisDto, what: &str) -> Result<ChartAxisData, ApiError> {
    let min = number_opt(a.min, "axis minimum")?;
    let max = number_opt(a.max, "axis maximum")?;
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo >= hi {
            return Err(ApiError::invalid_argument(format!(
                "{what}: the minimum must be below the maximum"
            )));
        }
    }
    let major = number_opt(a.major, "axis interval")?;
    if major.is_some_and(|m| m <= 0.0) {
        return Err(ApiError::invalid_argument(format!(
            "{what}: the major interval must be positive"
        )));
    }
    if a.minor_count.is_some_and(|m| m == 0 || m > 1000) {
        return Err(ApiError::invalid_argument(format!(
            "{what}: the minor count must be 1 to 1000"
        )));
    }
    if a.log && min.is_some_and(|m| m <= 0.0) {
        return Err(ApiError::invalid_argument(format!(
            "{what}: a logarithmic axis needs a positive minimum"
        )));
    }
    let interval = |t: Option<TimeIntervalDto>, which: &str| -> Result<_, ApiError> {
        match t {
            Some(t) if t.number == 0 || t.number > MAX_TIME_INTERVAL => {
                Err(ApiError::invalid_argument(format!(
                    "{what}: the {which} interval must be 1 to {MAX_TIME_INTERVAL}"
                )))
            }
            Some(t) => Ok(Some(TimeIntervalData {
                number: t.number,
                unit: time_unit_from_api(t.unit),
            })),
            None => Ok(None),
        }
    };
    let crosses = match a.crosses {
        Some(AxisCross::Value { value }) => {
            if !value.is_finite() {
                return Err(ApiError::invalid_argument(format!(
                    "{what}: the crossing value is not a number"
                )));
            }
            Some(AxisCrossData::Value(value))
        }
        Some(AxisCross::Start) => Some(AxisCrossData::Start),
        Some(AxisCross::End) => Some(AxisCrossData::End),
        None => None,
    };
    Ok(ChartAxisData {
        visible: a.visible,
        title: text_opt(&a.title, "axis title")?,
        min,
        max,
        major,
        minor_count: a.minor_count,
        log: a.log,
        reverse: a.reverse,
        major_grid: a.major_grid,
        minor_grid: a.minor_grid,
        number_format: text_opt(&a.number_format, "number format")?,
        axis_type: match a.axis_type {
            AxisType::Auto => AxisTypeData::Auto,
            AxisType::Text => AxisTypeData::Text,
            AxisType::Date => AxisTypeData::Date,
        },
        time_resolution: a.time_resolution.map(time_unit_from_api),
        major_time: interval(a.major_time, "major")?,
        minor_time: interval(a.minor_time, "minor")?,
        crosses,
        label_position: match a.label_position {
            AxisLabelPos::NearAxis => AxisLabelPosData::NearAxis,
            AxisLabelPos::NearAxisOtherSide => AxisLabelPosData::NearAxisOtherSide,
            AxisLabelPos::OutsideStart => AxisLabelPosData::OutsideStart,
            AxisLabelPos::OutsideEnd => AxisLabelPosData::OutsideEnd,
        },
        mark_position: match a.mark_position {
            AxisMarkPos::AtLabelsAndAxis => AxisMarkPosData::AtLabelsAndAxis,
            AxisMarkPos::AtLabels => AxisMarkPosData::AtLabels,
            AxisMarkPos::AtAxis => AxisMarkPosData::AtAxis,
        },
    })
}

/// Largest date axis interval count (AOO's spin field allows more, but
/// 10,000 years is past every serial date).
const MAX_TIME_INTERVAL: u32 = 10_000;

fn time_unit_from_api(u: TimeUnit) -> TimeUnitData {
    match u {
        TimeUnit::Days => TimeUnitData::Days,
        TimeUnit::Months => TimeUnitData::Months,
        TimeUnit::Years => TimeUnitData::Years,
    }
}

fn time_unit_to_api(u: TimeUnitData) -> TimeUnit {
    match u {
        TimeUnitData::Days => TimeUnit::Days,
        TimeUnitData::Months => TimeUnit::Months,
        TimeUnitData::Years => TimeUnit::Years,
    }
}

fn axis_to_api(a: &ChartAxisData) -> ChartAxisDto {
    ChartAxisDto {
        visible: a.visible,
        title: a.title.clone(),
        min: a.min,
        max: a.max,
        major: a.major,
        minor_count: a.minor_count,
        log: a.log,
        reverse: a.reverse,
        major_grid: a.major_grid,
        minor_grid: a.minor_grid,
        number_format: a.number_format.clone(),
        axis_type: match a.axis_type {
            AxisTypeData::Auto => AxisType::Auto,
            AxisTypeData::Text => AxisType::Text,
            AxisTypeData::Date => AxisType::Date,
        },
        time_resolution: a.time_resolution.map(time_unit_to_api),
        major_time: a.major_time.map(|t| TimeIntervalDto {
            number: t.number,
            unit: time_unit_to_api(t.unit),
        }),
        minor_time: a.minor_time.map(|t| TimeIntervalDto {
            number: t.number,
            unit: time_unit_to_api(t.unit),
        }),
        crosses: a.crosses.map(|c| match c {
            AxisCrossData::Start => AxisCross::Start,
            AxisCrossData::End => AxisCross::End,
            AxisCrossData::Value(value) => AxisCross::Value { value },
        }),
        label_position: match a.label_position {
            AxisLabelPosData::NearAxis => AxisLabelPos::NearAxis,
            AxisLabelPosData::NearAxisOtherSide => AxisLabelPos::NearAxisOtherSide,
            AxisLabelPosData::OutsideStart => AxisLabelPos::OutsideStart,
            AxisLabelPosData::OutsideEnd => AxisLabelPos::OutsideEnd,
        },
        mark_position: match a.mark_position {
            AxisMarkPosData::AtLabelsAndAxis => AxisMarkPos::AtLabelsAndAxis,
            AxisMarkPosData::AtLabels => AxisMarkPos::AtLabels,
            AxisMarkPosData::AtAxis => AxisMarkPos::AtAxis,
        },
    }
}

fn labels_from_api(l: &DataLabelsDto) -> DataLabelsData {
    DataLabelsData {
        value: l.value,
        percent: l.percent,
        category: l.category,
        series_name: l.series_name,
        position: label_pos_from_api(l.position),
    }
}

fn labels_to_api(l: &DataLabelsData) -> DataLabelsDto {
    DataLabelsDto {
        value: l.value,
        percent: l.percent,
        category: l.category,
        series_name: l.series_name,
        position: label_pos_to_api(l.position),
    }
}

fn finite(v: f64, what: &str) -> Result<f64, ApiError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ApiError::invalid_argument(format!(
            "{what} is not a number"
        )))
    }
}

fn error_bars_from_api(
    wb: &Workbook,
    sheet: SheetId,
    e: &ErrorBarsDto,
) -> Result<ErrorBarsData, ApiError> {
    let kind = match &e.kind {
        ErrorBarKind::Constant { plus, minus } => ErrorBarKindData::Constant {
            plus: finite(*plus, "error bar length")?.abs(),
            minus: finite(*minus, "error bar length")?.abs(),
        },
        ErrorBarKind::Percent { percent } => ErrorBarKindData::Percent {
            percent: finite(*percent, "error bar percent")?.abs(),
        },
        ErrorBarKind::StdDev { k } => ErrorBarKindData::StdDev {
            k: finite(*k, "error bar multiplier")?.abs(),
        },
        ErrorBarKind::Variance => ErrorBarKindData::Variance,
        ErrorBarKind::StdError => ErrorBarKindData::StdError,
        ErrorBarKind::ErrorMargin { percent } => ErrorBarKindData::ErrorMargin {
            percent: finite(*percent, "error margin")?.abs(),
        },
        ErrorBarKind::Range { plus, minus } => ErrorBarKindData::Range {
            plus: parse_range_text(wb, sheet, plus)?,
            minus: parse_range_text(wb, sheet, minus)?,
        },
    };
    Ok(ErrorBarsData {
        kind,
        direction: dir_from_api(e.direction),
    })
}

fn error_bars_to_api(wb: &Workbook, sheet: SheetId, e: &ErrorBarsData) -> ErrorBarsDto {
    let kind = match &e.kind {
        ErrorBarKindData::Constant { plus, minus } => ErrorBarKind::Constant {
            plus: *plus,
            minus: *minus,
        },
        ErrorBarKindData::Percent { percent } => ErrorBarKind::Percent { percent: *percent },
        ErrorBarKindData::StdDev { k } => ErrorBarKind::StdDev { k: *k },
        ErrorBarKindData::Variance => ErrorBarKind::Variance,
        ErrorBarKindData::StdError => ErrorBarKind::StdError,
        ErrorBarKindData::ErrorMargin { percent } => {
            ErrorBarKind::ErrorMargin { percent: *percent }
        }
        ErrorBarKindData::Range { plus, minus } => ErrorBarKind::Range {
            plus: range_text(wb, sheet, plus),
            minus: range_text(wb, sheet, minus),
        },
    };
    ErrorBarsDto {
        kind,
        direction: dir_to_api(e.direction),
    }
}

fn series_from_api(
    wb: &Workbook,
    sheet: SheetId,
    s: &ChartSeriesDto,
) -> Result<ChartSeriesData, ApiError> {
    let name = match &s.name {
        Some(SeriesName::Ref { r#ref }) => {
            Some(SeriesNameData::Ref(parse_cell_text(wb, sheet, r#ref)?))
        }
        Some(SeriesName::Text { text }) => Some(SeriesNameData::Text(
            text_opt(&Some(text.clone()), "series name")?.unwrap_or_default(),
        )),
        None => None,
    };
    let values = parse_range_text(wb, sheet, &s.values)?;
    let x_values = match &s.x_values {
        Some(t) => Some(parse_range_text(wb, sheet, t)?).filter(|r| !r.is_unset()),
        None => None,
    };
    let sizes = match &s.sizes {
        Some(t) => Some(parse_range_text(wb, sheet, t)?).filter(|r| !r.is_unset()),
        None => None,
    };
    Ok(ChartSeriesData {
        name,
        values,
        literal_values: None,
        x_values,
        literal_x: None,
        sizes,
        literal_sizes: None,
        kind: s.kind.map(kind_from_api),
        secondary_axis: s.secondary_axis,
        color: s.color,
        marker: s.marker.map(marker_from_api),
        labels: labels_from_api(&s.labels),
        trend: s
            .trend
            .as_ref()
            .map(|t| {
                Ok::<_, ApiError>(TrendLineData {
                    kind: trend_kind_from_api(t.kind),
                    show_equation: t.show_equation,
                    show_r2: t.show_r2,
                    name: text_opt(&t.name, "trend line name")?,
                })
            })
            .transpose()?,
        error_bars: s
            .error_bars
            .as_ref()
            .map(|e| error_bars_from_api(wb, sheet, e))
            .transpose()?,
        mean_line: s.mean_line,
    })
}

fn series_to_api(wb: &Workbook, sheet: SheetId, s: &ChartSeriesData) -> ChartSeriesDto {
    ChartSeriesDto {
        name: match &s.name {
            Some(SeriesNameData::Ref(r)) => Some(SeriesName::Ref {
                r#ref: range_text(wb, sheet, r),
            }),
            Some(SeriesNameData::Text(t)) => Some(SeriesName::Text { text: t.clone() }),
            None => None,
        },
        values: range_text(wb, sheet, &s.values),
        x_values: s.x_values.as_ref().map(|r| range_text(wb, sheet, r)),
        sizes: s.sizes.as_ref().map(|r| range_text(wb, sheet, r)),
        kind: s.kind.map(kind_to_api),
        secondary_axis: s.secondary_axis,
        color: s.color,
        marker: s.marker.map(marker_to_api),
        labels: labels_to_api(&s.labels),
        trend: s.trend.as_ref().map(|t| TrendLineDto {
            kind: trend_kind_to_api(t.kind),
            show_equation: t.show_equation,
            show_r2: t.show_r2,
            name: t.name.clone(),
        }),
        error_bars: s
            .error_bars
            .as_ref()
            .map(|e| error_bars_to_api(wb, sheet, e)),
        mean_line: s.mean_line,
    }
}

/// The series the wizard derives from a data range (AOO semantics: with
/// series in columns, a label first row names the series and a label
/// first column holds the categories; with series in rows the roles
/// swap).
pub fn derive_series(source: &ChartSourceData) -> (Option<ChartRangeData>, Vec<ChartSeriesData>) {
    let Some(&(sheet, r)) = source.range.areas.first() else {
        return (None, Vec::new());
    };
    let area = |r0: u32, c0: u32, r1: u32, c1: u32| ChartRangeData {
        areas: vec![(sheet, CellRangeLocal { r0, c0, r1, c1 })],
        invalid: false,
    };
    let mut series = Vec::new();
    let mut categories = None;
    if !source.series_in_rows {
        let data_r0 = if source.first_row_label && r.r1 > r.r0 {
            r.r0 + 1
        } else {
            r.r0
        };
        let data_c0 = if source.first_col_label && r.c1 > r.c0 {
            r.c0 + 1
        } else {
            r.c0
        };
        if source.first_col_label && r.c1 > r.c0 {
            categories = Some(area(data_r0, r.c0, r.r1, r.c0));
        }
        for c in data_c0..=r.c1 {
            if series.len() >= MAX_SERIES {
                break;
            }
            let name = (source.first_row_label && r.r1 > r.r0)
                .then(|| SeriesNameData::Ref(area(r.r0, c, r.r0, c)));
            series.push(ChartSeriesData {
                name,
                values: area(data_r0, c, r.r1, c),
                ..ChartSeriesData::default()
            });
        }
    } else {
        let data_r0 = if source.first_row_label && r.r1 > r.r0 {
            r.r0 + 1
        } else {
            r.r0
        };
        let data_c0 = if source.first_col_label && r.c1 > r.c0 {
            r.c0 + 1
        } else {
            r.c0
        };
        if source.first_row_label && r.r1 > r.r0 {
            categories = Some(area(r.r0, data_c0, r.r0, r.c1));
        }
        for row in data_r0..=r.r1 {
            if series.len() >= MAX_SERIES {
                break;
            }
            let name = (source.first_col_label && r.c1 > r.c0)
                .then(|| SeriesNameData::Ref(area(row, r.c0, row, r.c0)));
            series.push(ChartSeriesData {
                name,
                values: area(row, data_c0, row, r.c1),
                ..ChartSeriesData::default()
            });
        }
    }
    (categories, series)
}

/// A chart from the API (ranges parsed and validated; the anchor checked
/// against the grid). When `series` is empty and `source` is set, the
/// series are derived from the source range.
pub fn from_api(wb: &Workbook, sheet: SheetId, dto: &ChartDto) -> Result<ChartData, ApiError> {
    if dto.series.len() > MAX_SERIES {
        return Err(ApiError::invalid_argument(format!(
            "a chart may have at most {MAX_SERIES} series"
        )));
    }
    let anchor = anchor_from_api(&dto.anchor)?;
    let source = match &dto.source {
        Some(s) => {
            let range = parse_range_text(wb, sheet, &s.range)?;
            Some(ChartSourceData {
                range,
                series_in_rows: s.series_in_rows,
                first_row_label: s.first_row_label,
                first_col_label: s.first_col_label,
            })
        }
        None => None,
    };
    let mut categories = match &dto.categories {
        Some(t) => Some(parse_range_text(wb, sheet, t)?).filter(|r| !r.is_unset()),
        None => None,
    };
    let mut series: Vec<ChartSeriesData> = dto
        .series
        .iter()
        .map(|s| series_from_api(wb, sheet, s))
        .collect::<Result<_, _>>()?;
    if series.is_empty() {
        if let Some(src) = &source {
            let (cats, derived) = derive_series(src);
            if categories.is_none() {
                categories = cats;
            }
            series = derived;
        }
    }
    if let Some(h) = dto.hole {
        if !h.is_finite() || !(0.0..=0.95).contains(&h) {
            return Err(ApiError::invalid_argument("the pie hole must be 0 to 0.95"));
        }
    }
    Ok(ChartData {
        name: dto.name.chars().take(64).collect(),
        anchor,
        kind: kind_from_api(dto.kind),
        stacking: stacking_from_api(dto.stacking),
        lines: dto.lines,
        markers: dto.markers,
        smooth: dto.smooth,
        hole: dto.hole,
        exploded: dto.exploded,
        filled: dto.filled,
        stock: dto.stock.map(stock_from_api),
        three_d: dto.three_d,
        source,
        categories,
        literal_categories: None,
        series,
        title: text_opt(&dto.title, "title")?,
        subtitle: text_opt(&dto.subtitle, "subtitle")?,
        legend: legend_from_api(dto.legend),
        x_axis: axis_from_api(&dto.x_axis, "x axis")?,
        y_axis: axis_from_api(&dto.y_axis, "y axis")?,
        y2_axis: dto
            .y2_axis
            .as_ref()
            .map(|a| axis_from_api(a, "secondary y axis"))
            .transpose()?,
        background: dto.background,
        plot_background: dto.plot_background,
        include_hidden: dto.include_hidden,
    })
}

/// The API view of a chart.
pub fn to_api(wb: &Workbook, sheet: SheetId, chart: &Chart) -> ChartDto {
    let c = &chart.data;
    ChartDto {
        id: chart.id,
        name: c.name.clone(),
        anchor: anchor_to_api(&c.anchor),
        kind: kind_to_api(c.kind),
        stacking: stacking_to_api(c.stacking),
        lines: c.lines,
        markers: c.markers,
        smooth: c.smooth,
        hole: c.hole,
        exploded: c.exploded,
        filled: c.filled,
        stock: c.stock.map(stock_to_api),
        three_d: c.three_d,
        source: c.source.as_ref().map(|s| ChartSourceDto {
            range: range_text(wb, sheet, &s.range),
            series_in_rows: s.series_in_rows,
            first_row_label: s.first_row_label,
            first_col_label: s.first_col_label,
        }),
        categories: c.categories.as_ref().map(|r| range_text(wb, sheet, r)),
        series: c
            .series
            .iter()
            .map(|s| series_to_api(wb, sheet, s))
            .collect(),
        title: c.title.clone(),
        subtitle: c.subtitle.clone(),
        legend: legend_to_api(c.legend),
        x_axis: axis_to_api(&c.x_axis),
        y_axis: axis_to_api(&c.y_axis),
        y2_axis: c.y2_axis.as_ref().map(axis_to_api),
        background: c.background,
        plot_background: c.plot_background,
        include_hidden: c.include_hidden,
        data_table: has_own_data(c).then(|| data_table_of(c)),
    }
}

// ---------------------------------------------------------------------------
// Own data (AOO's internal data provider, View > Data Table)
// ---------------------------------------------------------------------------

/// `true` when the chart reads no cell at all: no data range, category,
/// value, X, size, name or error-bar range, not even an invalid one (AOO's
/// `XChartDocument::hasInternalDataProvider`, which enables View > Data
/// Table in `ControllerCommandDispatch`). Its data are the literal lists.
pub fn has_own_data(c: &ChartData) -> bool {
    c.has_own_data()
}

fn literal_numbers(v: Option<&ChartValuesData>) -> Vec<Option<f64>> {
    match v {
        Some(ChartValuesData::Literal(v)) => v.clone(),
        Some(ChartValuesData::LiteralText(t)) => t
            .iter()
            .map(|x| x.trim().parse::<f64>().ok().filter(|n| n.is_finite()))
            .collect(),
        None => Vec::new(),
    }
}

/// The own data of a chart as the dialog's table: every column padded to
/// the longest one (empty cells), the series' X and size columns present
/// for XY and bubble series (or when the chart has them).
pub fn data_table_of(c: &ChartData) -> ChartDataTableDto {
    let mut categories: Vec<String> = match &c.literal_categories {
        Some(ChartValuesData::LiteralText(t)) => t.clone(),
        Some(ChartValuesData::Literal(v)) => v
            .iter()
            .map(|x| x.map(|n| format!("{n}")).unwrap_or_default())
            .collect(),
        None => Vec::new(),
    };
    let mut series: Vec<DataTableSeriesDto> = c
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let kind = s.kind.unwrap_or(c.kind);
            let xy = matches!(kind, ChartKindData::Scatter | ChartKindData::Bubble);
            DataTableSeriesDto {
                source: u32::try_from(i).ok(),
                name: match &s.name {
                    Some(SeriesNameData::Text(t)) => t.clone(),
                    _ => String::new(),
                },
                values: literal_numbers(s.literal_values.as_ref()),
                x: (xy || s.literal_x.is_some()).then(|| literal_numbers(s.literal_x.as_ref())),
                sizes: (kind == ChartKindData::Bubble || s.literal_sizes.is_some())
                    .then(|| literal_numbers(s.literal_sizes.as_ref())),
            }
        })
        .collect();
    let rows = series
        .iter()
        .flat_map(|s| {
            [
                s.values.len(),
                s.x.as_ref().map_or(0, Vec::len),
                s.sizes.as_ref().map_or(0, Vec::len),
            ]
        })
        .chain([categories.len()])
        .max()
        .unwrap_or(0);
    categories.resize(rows, String::new());
    for s in &mut series {
        s.values.resize(rows, None);
        if let Some(x) = &mut s.x {
            x.resize(rows, None);
        }
        if let Some(z) = &mut s.sizes {
            z.resize(rows, None);
        }
    }
    ChartDataTableDto { categories, series }
}

/// Replaces the own data of `data` with `t` (AOO `DataBrowserModel` edits
/// applied through `InternalDataProvider`). Each table series continues
/// the chart series named by its `source` (formatting kept; moved and
/// deleted series take theirs along) or is a new series with default
/// formatting. Categories that were numbers (dates) stay numbers when
/// every text still is one; all-empty categories are dropped.
pub fn apply_data_table(data: &mut ChartData, t: &ChartDataTableDto) -> Result<(), ApiError> {
    let rows = t.categories.len();
    if rows > MAX_POINTS {
        return Err(ApiError::invalid_argument(format!(
            "a chart data table may have at most {MAX_POINTS} rows"
        )));
    }
    if t.series.len() > MAX_SERIES {
        return Err(ApiError::invalid_argument(format!(
            "a chart may have at most {MAX_SERIES} series"
        )));
    }
    let column = |v: &[Option<f64>]| -> Result<(), ApiError> {
        if v.len() != rows {
            return Err(ApiError::invalid_argument(
                "every column of the chart data table needs one cell per row",
            ));
        }
        if v.iter().flatten().any(|x| !x.is_finite()) {
            return Err(ApiError::invalid_argument(
                "a chart data table value is not a number",
            ));
        }
        Ok(())
    };
    for s in &t.series {
        column(&s.values)?;
        if let Some(x) = &s.x {
            column(x)?;
        }
        if let Some(z) = &s.sizes {
            column(z)?;
        }
        text_opt(&Some(s.name.clone()), "series name")?;
    }
    for c in &t.categories {
        text_opt(&Some(c.clone()), "category")?;
    }
    let old = std::mem::take(&mut data.series);
    data.series = t
        .series
        .iter()
        .map(|ts| {
            let mut s = ts
                .source
                .and_then(|i| old.get(i as usize))
                .cloned()
                .unwrap_or_default();
            s.values = ChartRangeData::default();
            s.x_values = None;
            s.sizes = None;
            s.name = (!ts.name.is_empty()).then(|| SeriesNameData::Text(ts.name.clone()));
            s.literal_values = Some(ChartValuesData::Literal(ts.values.clone()));
            s.literal_x = ts.x.clone().map(ChartValuesData::Literal);
            s.literal_sizes = ts.sizes.clone().map(ChartValuesData::Literal);
            s
        })
        .collect();
    data.categories = None;
    let numeric_before = matches!(data.literal_categories, Some(ChartValuesData::Literal(_)));
    data.literal_categories = if t.categories.iter().all(|c| c.trim().is_empty()) {
        None
    } else if numeric_before
        && t.categories
            .iter()
            .all(|c| c.trim().is_empty() || c.trim().parse::<f64>().is_ok_and(f64::is_finite))
    {
        Some(ChartValuesData::Literal(
            t.categories
                .iter()
                .map(|c| c.trim().parse::<f64>().ok())
                .collect(),
        ))
    } else {
        Some(ChartValuesData::LiteralText(t.categories.clone()))
    };
    Ok(())
}

/// Literal points held by every chart of the document.
pub fn literal_count(wb: &Workbook) -> usize {
    wb.sheets
        .iter()
        .flat_map(|s| s.meta.charts.iter())
        .map(|c| c.data.literal_points())
        .sum()
}

/// Refuses literal data that would take the document past
/// [`tally_model::MAX_LITERAL_POINTS`] (`others`: every other chart's).
pub fn check_literal_budget(others: usize, new: usize) -> Result<(), ApiError> {
    if others.saturating_add(new) > tally_model::MAX_LITERAL_POINTS {
        return Err(ApiError::invalid_argument(format!(
            "a document may hold at most {} chart data table values",
            tally_model::MAX_LITERAL_POINTS
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Data resolution
// ---------------------------------------------------------------------------

/// One resolved cell of a chart range.
struct Point {
    value: Option<f64>,
    label: String,
    format: Option<Arc<str>>,
    /// The cell holds a number (not a boolean).
    number: bool,
    /// The cell's number format is a date or time format.
    date: bool,
}

/// The work and size budget of one `chart_data` call: cells visited
/// (hidden ones included) and points emitted. Once either runs out the
/// walks stop and the response is marked `truncated`.
pub(crate) struct DataBudget {
    visits: usize,
    points: usize,
    truncated: bool,
}

impl DataBudget {
    fn new() -> Self {
        Self {
            visits: MAX_CHART_VISITS,
            points: MAX_CHART_POINTS,
            truncated: false,
        }
    }

    /// Charges one visited cell; `false` (and truncated) when spent.
    fn visit(&mut self) -> bool {
        if self.visits == 0 {
            self.truncated = true;
            return false;
        }
        self.visits -= 1;
        true
    }

    /// How many of `n` literal points still fit (charging them).
    fn take_points(&mut self, n: usize) -> usize {
        let k = n.min(self.points);
        if k < n {
            self.truncated = true;
        }
        self.points -= k;
        k
    }
}

/// Walks the cells of a range in reading order (each area row by row),
/// skipping hidden rows and columns unless `include_hidden`. Emits at
/// most `cap` cells (and what the budget's point allowance leaves);
/// every row and column looked at is charged to the budget's visits.
/// Areas are cut to the sheet's used range (the cells past it are
/// empty), and an area whose columns are all hidden costs one pass over
/// its columns. Reports whether it stopped early.
fn walk_range(
    wb: &Workbook,
    r: &ChartRangeData,
    include_hidden: bool,
    cap: usize,
    budget: &mut DataBudget,
    mut f: impl FnMut(CellAddr),
) -> bool {
    let cap = cap.min(budget.points);
    let mut n = 0usize;
    let mut stopped = false;
    'areas: for (sid, a) in r.areas.iter().take(MAX_AREAS) {
        let Some(sheet) = wb.sheet(*sid) else {
            continue;
        };
        // Long ranges are cut to the used rows and columns; short ones
        // keep their trailing empty points (they show as categories).
        let used = sheet.used_range();
        let r1 = if a.r1 - a.r0 >= 1024 {
            used.map_or(a.r0, |u| u.r1.max(a.r0)).min(a.r1)
        } else {
            a.r1
        };
        let c1 = if a.c1 - a.c0 >= 256 {
            used.map_or(a.c0, |u| u.c1.max(a.c0)).min(a.c1)
        } else {
            a.c1
        };
        let mut cols = Vec::new();
        for col in a.c0..=c1 {
            if !budget.visit() {
                stopped = true;
                break 'areas;
            }
            if include_hidden || !sheet.col_hidden(col) {
                cols.push(col);
            }
        }
        if cols.is_empty() {
            continue;
        }
        for row in a.r0..=r1 {
            if !budget.visit() {
                stopped = true;
                break 'areas;
            }
            if !include_hidden && sheet.row_hidden(row) {
                continue;
            }
            for &col in &cols {
                if n >= cap {
                    stopped = true;
                    break 'areas;
                }
                if !budget.visit() {
                    stopped = true;
                    break 'areas;
                }
                n += 1;
                f(CellAddr::new(*sid, row, col));
            }
        }
    }
    budget.points -= n;
    if stopped {
        budget.truncated = true;
    }
    stopped
}

fn point_of(wb: &Workbook, addr: CellAddr) -> Point {
    let v = wb.value(addr);
    let style = wb.styles.get(wb.style_at(addr));
    let format = (!style.is_general()).then(|| style.format_code.clone());
    let label = match &v {
        Value::Empty => String::new(),
        _ => crate::service::view::format_with(wb, &v, style).text,
    };
    let value = match &v {
        Value::Number(n) if n.is_finite() => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    };
    Point {
        value,
        label,
        format,
        number: matches!(v, Value::Number(n) if n.is_finite()),
        date: style.format.is_date(),
    }
}

fn resolve_points(
    wb: &Workbook,
    r: &ChartRangeData,
    include_hidden: bool,
    budget: &mut DataBudget,
) -> Vec<Point> {
    let mut out = Vec::new();
    walk_range(wb, r, include_hidden, MAX_POINTS, budget, |a| {
        out.push(point_of(wb, a))
    });
    out
}

/// The literal points that fit the per-series cap and the budget.
fn literal_take(len: usize, budget: &mut DataBudget) -> usize {
    if len > MAX_POINTS {
        budget.truncated = true;
    }
    budget.take_points(len.min(MAX_POINTS))
}

fn resolve_numbers(
    wb: &Workbook,
    r: Option<&ChartRangeData>,
    literal: Option<&ChartValuesData>,
    include_hidden: bool,
    budget: &mut DataBudget,
) -> Option<Vec<Option<f64>>> {
    match (r, literal) {
        (Some(r), _) if !r.is_unset() => Some(
            resolve_points(wb, r, include_hidden, budget)
                .into_iter()
                .map(|p| p.value)
                .collect(),
        ),
        (_, Some(ChartValuesData::Literal(v))) => {
            let k = literal_take(v.len(), budget);
            Some(v.iter().take(k).copied().collect())
        }
        (_, Some(ChartValuesData::LiteralText(t))) => {
            let k = literal_take(t.len(), budget);
            Some(t.iter().take(k).map(|s| s.trim().parse().ok()).collect())
        }
        _ => None,
    }
}

fn series_display_name(wb: &Workbook, s: &ChartSeriesData, index: usize) -> String {
    match &s.name {
        Some(SeriesNameData::Text(t)) => t.clone(),
        Some(SeriesNameData::Ref(r)) => {
            // The first cell only: a budget of its own keeps the name
            // independent of how much data the chart has.
            let text = r
                .areas
                .iter()
                .find(|(s, _)| wb.sheet(*s).is_some())
                .map(|(s, a)| {
                    crate::service::view::display_text(wb, CellAddr::new(*s, a.r0, a.c0))
                });
            text.unwrap_or_else(|| format!("Series {}", index + 1))
        }
        None => format!("Series {}", index + 1),
    }
}

/// Sample standard deviation (`STDEV`), `None` below two values.
fn stdev(vals: &[f64]) -> Option<f64> {
    variance(vals).map(f64::sqrt)
}

/// Sample variance (`VAR`).
fn variance(vals: &[f64]) -> Option<f64> {
    let n = vals.len();
    if n < 2 {
        return None;
    }
    let mean = vals.iter().sum::<f64>() / n as f64;
    Some(vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64)
}

fn fmt_coef(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    let s = if !(1e-4..1e9).contains(&a) {
        format!("{v:.3e}")
    } else {
        let digits = (4 - a.log10().floor() as i32).clamp(0, 6) as usize;
        let s = format!("{v:.digits$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    s
}

fn linear_term(a: f64, x: &str, b: f64) -> String {
    let sign = if b < 0.0 { "-" } else { "+" };
    format!("{}{x} {sign} {}", fmt_coef(a), fmt_coef(b.abs()))
}

/// Least-squares fit of a trend line through `(x, y)` pairs. Returns the
/// points of the fitted curve at the given `x`s, the equation text and
/// R² (identical to `RSQ` on the transformed axes).
pub fn trend_fit(kind: TrendKindData, xs: &[f64], ys: &[f64]) -> Option<TrendDataDto> {
    // Transformed samples.
    let mut tx = Vec::new();
    let mut ty = Vec::new();
    let mut keep = Vec::new();
    for (i, (&x, &y)) in xs.iter().zip(ys).enumerate() {
        let (u, v) = match kind {
            TrendKindData::Linear => (x, y),
            TrendKindData::Logarithmic => {
                if x <= 0.0 {
                    continue;
                }
                (x.ln(), y)
            }
            TrendKindData::Exponential => {
                if y <= 0.0 {
                    continue;
                }
                (x, y.ln())
            }
            TrendKindData::Power => {
                if x <= 0.0 || y <= 0.0 {
                    continue;
                }
                (x.ln(), y.ln())
            }
        };
        if !u.is_finite() || !v.is_finite() {
            continue;
        }
        tx.push(u);
        ty.push(v);
        keep.push(i);
    }
    let n = tx.len();
    if n < 2 {
        return None;
    }
    let nf = n as f64;
    let mx = tx.iter().sum::<f64>() / nf;
    let my = ty.iter().sum::<f64>() / nf;
    let sxx: f64 = tx.iter().map(|u| (u - mx).powi(2)).sum();
    let syy: f64 = ty.iter().map(|v| (v - my).powi(2)).sum();
    let sxy: f64 = tx.iter().zip(&ty).map(|(u, v)| (u - mx) * (v - my)).sum();
    if sxx == 0.0 {
        return None;
    }
    let a = sxy / sxx;
    let b = my - a * mx;
    let r2 = if syy == 0.0 {
        1.0
    } else {
        (sxy * sxy) / (sxx * syy)
    };
    let f = |x: f64| -> f64 {
        match kind {
            TrendKindData::Linear => a * x + b,
            TrendKindData::Logarithmic => a * x.ln() + b,
            TrendKindData::Exponential => b.exp() * (a * x).exp(),
            TrendKindData::Power => b.exp() * x.powf(a),
        }
    };
    let equation = match kind {
        TrendKindData::Linear => format!("f(x) = {}", linear_term(a, "x", b)),
        TrendKindData::Logarithmic => format!("f(x) = {}", linear_term(a, "·ln(x)", b)),
        TrendKindData::Exponential => {
            format!("f(x) = {}·exp({}x)", fmt_coef(b.exp()), fmt_coef(a))
        }
        TrendKindData::Power => format!("f(x) = {}·x^{}", fmt_coef(b.exp()), fmt_coef(a)),
    };
    let mut points: Vec<[f64; 2]> = keep
        .iter()
        .map(|&i| [xs[i], f(xs[i])])
        .filter(|p| p[1].is_finite())
        .collect();
    points.sort_by(|p, q| p[0].total_cmp(&q[0]));
    Some(TrendDataDto {
        points,
        equation,
        r2,
    })
}

/// Error bar `[minus, plus]` lengths per point.
pub(crate) fn error_bar_lengths(
    wb: &Workbook,
    e: &ErrorBarsData,
    values: &[Option<f64>],
    include_hidden: bool,
    budget: &mut DataBudget,
) -> Vec<[f64; 2]> {
    let present: Vec<f64> = values.iter().flatten().copied().collect();
    let n = values.len();
    let both = |len: f64| -> Vec<[f64; 2]> { vec![[len, len]; n] };
    let mut out = match &e.kind {
        ErrorBarKindData::Constant { plus, minus } => vec![[*minus, *plus]; n],
        ErrorBarKindData::Percent { percent } => values
            .iter()
            .map(|v| {
                let l = v.unwrap_or(0.0).abs() * percent / 100.0;
                [l, l]
            })
            .collect(),
        ErrorBarKindData::StdDev { k } => both(stdev(&present).unwrap_or(0.0) * k),
        ErrorBarKindData::Variance => both(variance(&present).unwrap_or(0.0)),
        ErrorBarKindData::StdError => {
            both(stdev(&present).map_or(0.0, |s| s / (present.len() as f64).sqrt()))
        }
        ErrorBarKindData::ErrorMargin { percent } => {
            let max = present.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            both(max * percent / 100.0)
        }
        ErrorBarKindData::Range { plus, minus } => {
            let p =
                resolve_numbers(wb, Some(plus), None, include_hidden, budget).unwrap_or_default();
            let m =
                resolve_numbers(wb, Some(minus), None, include_hidden, budget).unwrap_or_default();
            (0..n)
                .map(|i| {
                    [
                        m.get(i).copied().flatten().unwrap_or(0.0).abs(),
                        p.get(i).copied().flatten().unwrap_or(0.0).abs(),
                    ]
                })
                .collect()
        }
    };
    match e.direction {
        ErrorBarDirData::Both => {}
        ErrorBarDirData::Plus => out.iter_mut().for_each(|p| p[0] = 0.0),
        ErrorBarDirData::Minus => out.iter_mut().for_each(|p| p[1] = 0.0),
    }
    out
}

/// At most `max` of `points`, evenly spaced, keeping the first and last.
fn sample_points(points: Vec<[f64; 2]>, max: usize) -> Vec<[f64; 2]> {
    let n = points.len();
    if n <= max || max < 2 {
        return points;
    }
    (0..max).map(|i| points[i * (n - 1) / (max - 1)]).collect()
}

/// A category as the date axis sees it: its number (`None`: empty or
/// not a number), whether its format is a date format, and the format.
type CatPoint = (Option<f64>, bool, Option<Arc<str>>);

/// Chart types that can show a date category axis (AOO
/// `ChartTypeHelper::isSupportingDateAxis`: category charts except pie
/// and net).
fn supports_date_axis(kind: ChartKindData) -> bool {
    matches!(
        kind,
        ChartKindData::Column
            | ChartKindData::Bar
            | ChartKindData::Line
            | ChartKindData::Area
            | ChartKindData::Stock
    )
}

/// The date axis in effect, as AOO 4.1 decides it
/// (`ExplicitCategoriesProvider` / `lcl_fillDateCategories`): the axis
/// type is `Date`, or `Auto` and every non-empty category is a
/// date-formatted number; at least one category must be a date. The
/// days are floored to the resolution, which is the axis' own or AOO's
/// automatic one (`VSeriesPlotter::calculateTimeResolutionOnXAxis`:
/// years, months when two neighbouring sorted dates share a year, days
/// when two share a month).
fn date_axis_of(wb: &Workbook, c: &ChartData, points: &[CatPoint]) -> Option<DateAxisDto> {
    let auto = match c.x_axis.axis_type {
        AxisTypeData::Text => return None,
        AxisTypeData::Auto => true,
        AxisTypeData::Date => false,
    };
    if !supports_date_axis(c.kind) || points.is_empty() {
        return None;
    }
    let epoch = wb.settings.date_epoch;
    let mut days: Vec<Option<i64>> = Vec::with_capacity(points.len());
    for (v, date, _) in points {
        match v {
            None => days.push(None),
            Some(_) if auto && !date => return None,
            Some(n) => days.push(Some(tally_numfmt::date::unix_days_from_serial(epoch, *n)?)),
        }
    }
    let mut sorted: Vec<i64> = days.iter().flatten().copied().collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_unstable();
    let ymd = |d: i64| {
        tally_numfmt::date::ymd_from_serial(tally_types::DateEpoch::D1899, (d + 25_569) as f64)
            .unwrap_or((1970, 1, 1))
    };
    let resolution = c.x_axis.time_resolution.unwrap_or_else(|| {
        let mut r = TimeUnitData::Years;
        for w in sorted.windows(2) {
            let (a, b) = (ymd(w[0]), ymd(w[1]));
            if r == TimeUnitData::Years && a.0 == b.0 {
                r = TimeUnitData::Months;
            }
            if r == TimeUnitData::Months && a.0 == b.0 && a.1 == b.1 {
                r = TimeUnitData::Days;
                break;
            }
        }
        r
    });
    let floor = |d: i64| -> f64 {
        let (y, m, _) = ymd(d);
        let (m, day) = match resolution {
            TimeUnitData::Days => return d as f64,
            TimeUnitData::Months => (m, 1),
            TimeUnitData::Years => (1, 1),
        };
        tally_numfmt::date::serial_from_ymd(tally_types::DateEpoch::D1899, y, m, day)
            .map_or(d as f64, |s| s - 25_569.0)
    };
    let number_format = c.x_axis.number_format.clone().or_else(|| {
        points
            .iter()
            .find(|p| p.0.is_some())
            .and_then(|p| p.2.as_ref().map(|f| f.to_string()))
    });
    Some(DateAxisDto {
        days: days.into_iter().map(|d| d.map(floor)).collect(),
        resolution: time_unit_to_api(resolution),
        number_format,
    })
}

/// Resolves a chart's data from the cached cell values (no recalculation),
/// within [`MAX_CHART_VISITS`] visited cells and [`MAX_CHART_POINTS`]
/// points (`truncated` past either; the series that did not fit are
/// left out).
pub fn resolve_data(wb: &Workbook, sheet: SheetId, chart: &Chart, version: u64) -> ChartDataDto {
    let c = &chart.data;
    let hidden = c.include_hidden;
    let mut budget = DataBudget::new();
    let mut problems = Vec::new();
    // Category points for the date axis.
    let mut cat_points: Vec<CatPoint> = Vec::new();
    let mut cat_any_text = false;
    let mut categories: Vec<String> = match (&c.categories, &c.literal_categories) {
        (Some(r), _) if !r.is_unset() => {
            if range_is_invalid(wb, r) {
                problems.push("the category range is invalid (#REF!)".into());
                Vec::new()
            } else {
                resolve_points(wb, r, hidden, &mut budget)
                    .into_iter()
                    .map(|p| {
                        if p.number {
                            cat_points.push((p.value, p.date, p.format.clone()));
                        } else {
                            cat_any_text |= !p.label.is_empty();
                            cat_points.push((None, false, None));
                        }
                        p.label
                    })
                    .collect()
            }
        }
        (_, Some(ChartValuesData::LiteralText(t))) => {
            let k = literal_take(t.len(), &mut budget);
            t.iter().take(k).cloned().collect()
        }
        (_, Some(ChartValuesData::Literal(v))) => {
            // Own data: dates when the axis has a date format or none (AOO's
            // lcl_fillDateCategories). One axis format: parsed once.
            let date = c
                .x_axis
                .number_format
                .as_deref()
                .is_none_or(|f| tally_numfmt::parse_format(f).is_ok_and(|f| f.is_date()));
            v.iter()
                .take(literal_take(v.len(), &mut budget))
                .inspect(|x| cat_points.push((**x, date, None)))
                .map(|x| {
                    x.map(|n| {
                        crate::service::view::format_with(
                            wb,
                            &Value::Number(n),
                            wb.styles.get(crate::storage::StyleId(0)),
                        )
                        .text
                    })
                    .unwrap_or_default()
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let mut series = Vec::new();
    let mut longest = 0usize;
    for (i, s) in c.series.iter().enumerate() {
        if i >= MAX_SERIES || budget.visits == 0 || budget.points == 0 {
            budget.truncated = true;
            break;
        }
        let name = series_display_name(wb, s, i);
        if range_is_invalid(wb, &s.values) {
            problems.push(format!(
                "series {name:?}: the value range is invalid (#REF!)"
            ));
        }
        let mut values = if range_is_invalid(wb, &s.values) {
            Vec::new()
        } else {
            let pts = if s.values.is_unset() {
                match &s.literal_values {
                    Some(_) => None,
                    None => Some(Vec::new()),
                }
            } else {
                Some(resolve_points(wb, &s.values, hidden, &mut budget))
            };
            match pts {
                Some(p) => p,
                None => resolve_numbers(wb, None, s.literal_values.as_ref(), hidden, &mut budget)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|v| Point {
                        value: v,
                        label: v.map(fmt_coef).unwrap_or_default(),
                        format: None,
                        number: v.is_some(),
                        date: false,
                    })
                    .collect(),
            }
        };
        let number_format = values
            .iter()
            .find_map(|p| p.format.clone())
            .map(|f| f.to_string());
        let labels: Vec<String> = values
            .iter_mut()
            .map(|p| std::mem::take(&mut p.label))
            .collect();
        let values: Vec<Option<f64>> = values.into_iter().map(|p| p.value).collect();
        longest = longest.max(values.len());
        let x = match &s.x_values {
            Some(r) if range_is_invalid(wb, r) => {
                problems.push(format!("series {name:?}: the X range is invalid (#REF!)"));
                None
            }
            r => resolve_numbers(wb, r.as_ref(), s.literal_x.as_ref(), hidden, &mut budget),
        };
        let sizes = match &s.sizes {
            Some(r) if range_is_invalid(wb, r) => {
                problems.push(format!(
                    "series {name:?}: the size range is invalid (#REF!)"
                ));
                None
            }
            r => resolve_numbers(
                wb,
                r.as_ref(),
                s.literal_sizes.as_ref(),
                hidden,
                &mut budget,
            ),
        };
        let trend = s.trend.as_ref().and_then(|t| {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (j, v) in values.iter().enumerate() {
                let Some(y) = v else { continue };
                let xv = match &x {
                    Some(xv) => match xv.get(j).copied().flatten() {
                        Some(v) => v,
                        None => continue,
                    },
                    None => (j + 1) as f64,
                };
                xs.push(xv);
                ys.push(*y);
            }
            trend_fit(t.kind, &xs, &ys).map(|mut fit| {
                fit.points = sample_points(fit.points, MAX_TREND_POINTS);
                fit
            })
        });
        let errors = s
            .error_bars
            .as_ref()
            .map(|e| error_bar_lengths(wb, e, &values, hidden, &mut budget));
        let present: Vec<f64> = values.iter().flatten().copied().collect();
        let mean = (s.mean_line && !present.is_empty())
            .then(|| present.iter().sum::<f64>() / present.len() as f64);
        series.push(SeriesDataDto {
            name,
            color: s.color.unwrap_or_else(|| palette_color(i)),
            values,
            x,
            sizes,
            labels,
            number_format,
            trend,
            errors,
            mean,
        });
    }
    let date_axis = if cat_any_text {
        None
    } else {
        date_axis_of(wb, c, &cat_points)
    };
    if categories.is_empty() && longest > 0 {
        categories = (1..=longest).map(|i| i.to_string()).collect();
    }
    ChartDataDto {
        chart: ChartRef {
            sheet,
            id: chart.id,
        },
        version,
        categories,
        date_axis,
        series,
        truncated: budget.truncated,
        problems,
    }
}

// ---------------------------------------------------------------------------
// Suggestion
// ---------------------------------------------------------------------------

/// The contiguous block of non-empty cells around `(row, col)` (Ctrl+*
/// semantics), or the cell itself.
pub fn current_region(wb: &Workbook, sheet: SheetId, row: u32, col: u32) -> CellRange {
    data_region(wb, sheet, (row, col, row, col))
}

/// `start` (`r0, c0, r1, c1`) grown while any cell on the border just
/// outside it is non-empty (AOO's `ScDocument::GetDataArea` for a selection,
/// the block [`current_region`] finds for one cell), or `start` itself.
pub fn data_region(wb: &Workbook, sheet: SheetId, start: (u32, u32, u32, u32)) -> CellRange {
    let (row, col, row1, col1) = start;
    let Some(s) = wb.sheet(sheet) else {
        return CellRange::new(sheet, row, col, row1, col1);
    };
    let Some(used) = s.used_range() else {
        return CellRange::new(sheet, row, col, row1, col1);
    };
    let filled = |r: u32, c: u32| s.is_non_empty(r, c);
    let (mut r0, mut c0, mut r1, mut c1) = (row, col, row1, col1);
    let mut steps = 0;
    loop {
        steps += 1;
        if steps > 100_000 {
            break;
        }
        let mut grown = false;
        let lo_c = c0.saturating_sub(1).max(used.c0);
        let hi_c = (c1 + 1).min(used.c1);
        if r0 > used.r0 && (lo_c..=hi_c).any(|c| filled(r0 - 1, c)) {
            r0 -= 1;
            grown = true;
        }
        if r1 < used.r1 && (lo_c..=hi_c).any(|c| filled(r1 + 1, c)) {
            r1 += 1;
            grown = true;
        }
        let lo_r = r0.saturating_sub(1).max(used.r0);
        let hi_r = (r1 + 1).min(used.r1);
        if c0 > used.c0 && (lo_r..=hi_r).any(|r| filled(r, c0 - 1)) {
            c0 -= 1;
            grown = true;
        }
        if c1 < used.c1 && (lo_r..=hi_r).any(|r| filled(r, c1 + 1)) {
            c1 += 1;
            grown = true;
        }
        if !grown {
            break;
        }
    }
    CellRange::new(sheet, r0, c0, r1, c1)
}

/// AOO's `lcl_hasValueDataButNoDates` (`chartpos.cxx`): the cell holds a
/// number (or boolean) that is not formatted as a date or time. Such a
/// cell cannot be a label; text, blanks, errors and dates can.
fn is_plain_value(wb: &Workbook, a: CellAddr) -> bool {
    matches!(wb.value(a), Value::Number(_) | Value::Bool(_))
        && !wb.styles.get(wb.style_at(a)).format.is_date()
}

/// A chart proposal for a selection: the data region, the label
/// heuristics of AOO's `ScChartPositioner::CheckColRowHeaders` (the
/// first row names the series when none of its cells, the corner
/// included, is a non-date number; likewise the first column holds the
/// categories; dates and blanks count as labels), more rows than columns
/// means series in columns, and an anchor to the right of the region at
/// 16 × 9 cm.
pub fn suggest(
    wb: &Workbook,
    sheet: SheetId,
    sel: &RangeDto,
    kind: ChartKind,
) -> Result<ChartDto, ApiError> {
    crate::ops::check_sheet(wb, sheet)?;
    let region = if sel.r0 == sel.r1 && sel.c0 == sel.c1 {
        current_region(
            wb,
            sheet,
            sel.r0.min(MAX_ROWS - 1),
            sel.c0.min(MAX_COLS - 1),
        )
    } else {
        crate::ops::to_range(sheet, sel)?
    };
    let rows = region.rows();
    let cols = region.cols();
    let series_in_rows = cols > rows;
    let cell = |r: u32, c: u32| CellAddr::new(sheet, r, c);
    // Label heuristics (chartpos.cxx): no cell of the first row / column,
    // the corner included, holds a plain number. A single row or column
    // keeps its cells as data in that direction (otherwise nothing is
    // left to plot). The scans stop at the first number.
    let first_row_label =
        rows > 1 && (region.c0..=region.c1).all(|c| !is_plain_value(wb, cell(region.r0, c)));
    let first_col_label =
        cols > 1 && (region.r0..=region.r1).all(|r| !is_plain_value(wb, cell(r, region.c0)));
    let range = ChartRangeData {
        areas: vec![(
            sheet,
            CellRangeLocal {
                r0: region.r0,
                c0: region.c0,
                r1: region.r1,
                c1: region.c1,
            },
        )],
        invalid: false,
    };
    let source = ChartSourceData {
        range,
        series_in_rows,
        first_row_label,
        first_col_label,
    };
    let (categories, series) = derive_series(&source);
    let kind_data = kind_from_api(kind);
    let data = ChartData {
        name: String::new(),
        anchor: ChartAnchorData {
            row: region.r0,
            col: (region.c1 + 2).min(MAX_COLS - 1),
            dx_hmm: 0,
            dy_hmm: 0,
            width_hmm: 16_000,
            height_hmm: 9_000,
        },
        kind: kind_data,
        lines: true,
        markers: matches!(kind_data, ChartKindData::Scatter),
        source: Some(source),
        categories,
        series,
        legend: LegendPosData::Right,
        y_axis: ChartAxisData {
            major_grid: true,
            ..ChartAxisData::default()
        },
        ..ChartData::default()
    };
    Ok(to_api(wb, sheet, &Chart { id: 0, data }))
}

// ---------------------------------------------------------------------------
// Change detection
// ---------------------------------------------------------------------------

fn for_each_range(c: &ChartData, mut f: impl FnMut(&ChartRangeData)) {
    if let Some(r) = &c.categories {
        f(r);
    }
    for s in &c.series {
        f(&s.values);
        if let Some(x) = &s.x_values {
            f(x);
        }
        if let Some(z) = &s.sizes {
            f(z);
        }
        if let Some(SeriesNameData::Ref(r)) = &s.name {
            f(r);
        }
        if let Some(ErrorBarsData {
            kind: ErrorBarKindData::Range { plus, minus },
            ..
        }) = &s.error_bars
        {
            f(plus);
            f(minus);
        }
    }
}

fn for_each_range_mut(c: &mut ChartData, mut f: impl FnMut(&mut ChartRangeData)) {
    if let Some(s) = &mut c.source {
        f(&mut s.range);
    }
    if let Some(r) = &mut c.categories {
        f(r);
    }
    for s in &mut c.series {
        f(&mut s.values);
        if let Some(x) = &mut s.x_values {
            f(x);
        }
        if let Some(z) = &mut s.sizes {
            f(z);
        }
        if let Some(SeriesNameData::Ref(r)) = &mut s.name {
            f(r);
        }
        if let Some(ErrorBarsData {
            kind: ErrorBarKindData::Range { plus, minus },
            ..
        }) = &mut s.error_bars
        {
            f(plus);
            f(minus);
        }
    }
}

fn touches(r: &ChartRangeData, changed: &[CellRange]) -> bool {
    r.areas.iter().any(|(sid, a)| {
        changed.iter().any(|c| {
            c.sheet == *sid && c.r0 <= a.r1 && a.r0 <= c.r1 && c.c0 <= a.c1 && a.c0 <= c.c1
        })
    })
}

/// The charts whose plotted data lies in `changed` (or on a sheet whose
/// row/column visibility changed, for charts that skip hidden cells).
pub fn changed_charts(
    wb: &Workbook,
    changed: &[CellRange],
    layout_changed: &[SheetId],
) -> Option<Vec<ChartRef>> {
    let mut out = Vec::new();
    for s in &wb.sheets {
        for c in s.meta.charts.iter() {
            let mut hit = false;
            for_each_range(&c.data, |r| {
                if hit {
                    return;
                }
                if touches(r, changed)
                    || (!c.data.include_hidden
                        && r.areas.iter().any(|(sid, _)| layout_changed.contains(sid)))
                {
                    hit = true;
                }
            });
            if hit {
                out.push(ChartRef {
                    sheet: s.id,
                    id: c.id,
                });
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// Structure changes
// ---------------------------------------------------------------------------

/// Adjusts one range for a row/column change; `true` when it changed.
fn adjust_range(r: &mut ChartRangeData, sh: &Shape) -> bool {
    if r.areas.iter().all(|(s, _)| *s != sh.sheet) {
        return false;
    }
    let had = !r.areas.is_empty();
    let mut out = Vec::with_capacity(r.areas.len());
    for (s, a) in &r.areas {
        if *s != sh.sheet {
            out.push((*s, *a));
            continue;
        }
        let n = if sh.rows {
            span_after(sh, a.r0, a.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..*a })
        } else {
            span_after(sh, a.c0, a.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..*a })
        };
        if let Some(n) = n {
            out.push((*s, n));
        }
    }
    let changed = out != r.areas;
    r.areas = out;
    if had && r.areas.is_empty() {
        r.invalid = true;
    }
    changed
}

fn adjust_anchor(a: &mut ChartAnchorData, sh: &Shape) -> bool {
    let p = if sh.rows { a.row } else { a.col };
    let n = point_after(sh, p).unwrap_or_else(|| {
        // The anchor cell was deleted (or pushed off the grid): the
        // object stays where the deletion happened, like in AOO.
        sh.at.min(sh.limit() - 1)
    });
    if n == p {
        return false;
    }
    if sh.rows {
        a.row = n;
    } else {
        a.col = n;
    }
    true
}

/// Moves chart and picture anchors and series ranges for a row/column
/// change on `sh.sheet`: anchors follow `point_after`, ranges
/// `span_after`; a fully deleted range becomes `invalid`. Charts on other
/// sheets whose series point at the changed sheet are adjusted too.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    for s in &mut wb.sheets {
        let own = s.id == sh.sheet;
        if !s.meta.charts.is_empty() {
            let mut changed_any = false;
            let mut updated: Vec<Option<Chart>> = Vec::with_capacity(s.meta.charts.len());
            for c in s.meta.charts.iter() {
                let mut data = c.data.clone();
                let mut changed = false;
                for_each_range_mut(&mut data, |r| changed |= adjust_range(r, sh));
                if own {
                    changed |= adjust_anchor(&mut data.anchor, sh);
                }
                changed_any |= changed;
                updated.push(changed.then_some(Chart { id: c.id, data }));
            }
            if changed_any {
                let list = Arc::make_mut(&mut s.meta.charts);
                for (slot, n) in list.iter_mut().zip(updated) {
                    if let Some(n) = n {
                        *slot = Arc::new(n);
                    }
                }
            }
        }
        if own && !s.meta.images.is_empty() {
            let mut updated: Vec<Option<Image>> = Vec::with_capacity(s.meta.images.len());
            let mut changed_any = false;
            for i in s.meta.images.iter() {
                let mut anchor = i.data.anchor;
                if adjust_anchor(&mut anchor, sh) {
                    changed_any = true;
                    updated.push(Some(Image {
                        id: i.id,
                        data: ImageData {
                            anchor,
                            ..i.data.clone()
                        },
                    }));
                } else {
                    updated.push(None);
                }
            }
            if changed_any {
                let list = Arc::make_mut(&mut s.meta.images);
                for (slot, n) in list.iter_mut().zip(updated) {
                    if let Some(n) = n {
                        *slot = Arc::new(n);
                    }
                }
            }
        }
    }
}

fn within(a: &CellRangeLocal, b: &CellRange) -> bool {
    b.r0 <= a.r0 && a.r1 <= b.r1 && b.c0 <= a.c0 && a.c1 <= b.c1
}

fn shifted(a: &CellRangeLocal, dr: i64, dc: i64) -> CellRangeLocal {
    let s = |v: u32, d: i64| (i64::from(v) + d).clamp(0, i64::from(u32::MAX)) as u32;
    CellRangeLocal {
        r0: s(a.r0, dr),
        c0: s(a.c0, dc),
        r1: s(a.r1, dr),
        c1: s(a.c1, dc),
    }
}

/// `MoveRange`: areas that lie fully inside the moved block, and objects
/// anchored inside it, move with it (AOO behaviour). Charts on every
/// sheet are checked, since their series may point at `from.sheet`.
pub fn move_block(wb: &mut Workbook, from: &CellRange, dr: i64, dc: i64) {
    let sheet = from.sheet;
    for s in &mut wb.sheets {
        let own = s.id == sheet;
        if !s.meta.charts.is_empty() {
            let mut updated: Vec<Option<Chart>> = Vec::with_capacity(s.meta.charts.len());
            let mut changed_any = false;
            for c in s.meta.charts.iter() {
                let mut data = c.data.clone();
                let mut changed = false;
                for_each_range_mut(&mut data, |r| {
                    for (sid, a) in &mut r.areas {
                        if *sid == sheet && within(a, from) {
                            *a = shifted(a, dr, dc);
                            changed = true;
                        }
                    }
                });
                if own && from.contains(CellAddr::new(sheet, data.anchor.row, data.anchor.col)) {
                    let n = shifted(
                        &CellRangeLocal {
                            r0: data.anchor.row,
                            c0: data.anchor.col,
                            r1: data.anchor.row,
                            c1: data.anchor.col,
                        },
                        dr,
                        dc,
                    );
                    data.anchor.row = n.r0.min(MAX_ROWS - 1);
                    data.anchor.col = n.c0.min(MAX_COLS - 1);
                    changed = true;
                }
                changed_any |= changed;
                updated.push(changed.then_some(Chart { id: c.id, data }));
            }
            if changed_any {
                let list = Arc::make_mut(&mut s.meta.charts);
                for (slot, n) in list.iter_mut().zip(updated) {
                    if let Some(n) = n {
                        *slot = Arc::new(n);
                    }
                }
            }
        }
        if own && !s.meta.images.is_empty() {
            let any =
                s.meta.images.iter().any(|i| {
                    from.contains(CellAddr::new(sheet, i.data.anchor.row, i.data.anchor.col))
                });
            if any {
                let list = Arc::make_mut(&mut s.meta.images);
                for slot in list.iter_mut() {
                    let a = slot.data.anchor;
                    if from.contains(CellAddr::new(sheet, a.row, a.col)) {
                        let mut img = (**slot).clone();
                        let n = shifted(
                            &CellRangeLocal {
                                r0: a.row,
                                c0: a.col,
                                r1: a.row,
                                c1: a.col,
                            },
                            dr,
                            dc,
                        );
                        img.data.anchor.row = n.r0.min(MAX_ROWS - 1);
                        img.data.anchor.col = n.c0.min(MAX_COLS - 1);
                        *slot = Arc::new(img);
                    }
                }
            }
        }
    }
}

/// Number of chart series in the whole document (for
/// [`tally_model::MAX_SERIES_PER_DOCUMENT`]).
pub fn series_count(wb: &Workbook) -> usize {
    wb.sheets
        .iter()
        .flat_map(|s| s.meta.charts.iter())
        .map(|c| c.data.series.len())
        .sum()
}

/// Refuses a chart change that would take the document past
/// [`tally_model::MAX_SERIES_PER_DOCUMENT`]: `others` is the series count
/// of every chart but the one changed, `new` the changed chart's.
pub fn check_series_budget(others: usize, new: usize) -> Result<(), ApiError> {
    if others.saturating_add(new) > tally_model::MAX_SERIES_PER_DOCUMENT {
        return Err(ApiError::invalid_argument(format!(
            "a document may hold at most {} chart series",
            tally_model::MAX_SERIES_PER_DOCUMENT
        )));
    }
    Ok(())
}

/// Number of charts in the whole document (for the `AddChart` limit);
/// `extra` counts the sheet whose meta is currently taken out.
pub fn chart_count(wb: &Workbook) -> usize {
    wb.sheets.iter().map(|s| s.meta.charts.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_fit_matches_slope_intercept() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        let ys = [2.0, 4.1, 5.9, 8.0];
        let t = trend_fit(TrendKindData::Linear, &xs, &ys).unwrap();
        assert!((t.r2 - 0.9984).abs() < 1e-3, "{}", t.r2);
        assert!(
            t.equation.starts_with("f(x) = 1.98x + 0.05"),
            "{}",
            t.equation
        );
        assert_eq!(t.points.len(), 4);
    }

    #[test]
    fn exponential_fit_recovers_parameters() {
        let xs: Vec<f64> = (1..=6).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| 2.0 * (0.5 * x).exp()).collect();
        let t = trend_fit(TrendKindData::Exponential, &xs, &ys).unwrap();
        assert!((t.r2 - 1.0).abs() < 1e-9);
        assert_eq!(t.equation, "f(x) = 2·exp(0.5x)");
    }

    #[test]
    fn power_and_log_fits() {
        let xs: Vec<f64> = (1..=5).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| 3.0 * x.powf(2.0)).collect();
        let t = trend_fit(TrendKindData::Power, &xs, &ys).unwrap();
        assert_eq!(t.equation, "f(x) = 3·x^2");
        let ys: Vec<f64> = xs.iter().map(|x| 4.0 * x.ln() + 1.0).collect();
        let t = trend_fit(TrendKindData::Logarithmic, &xs, &ys).unwrap();
        assert_eq!(t.equation, "f(x) = 4·ln(x) + 1");
        assert!((t.r2 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn coefficient_text() {
        assert_eq!(fmt_coef(1.0), "1");
        assert_eq!(fmt_coef(-2.5), "-2.5");
        assert_eq!(fmt_coef(0.000012), "1.200e-5");
        assert_eq!(fmt_coef(123456.789), "123457");
    }

    #[test]
    fn derive_series_in_columns_with_labels() {
        let src = ChartSourceData {
            range: ChartRangeData {
                areas: vec![(
                    7,
                    CellRangeLocal {
                        r0: 0,
                        c0: 0,
                        r1: 5,
                        c1: 2,
                    },
                )],
                invalid: false,
            },
            series_in_rows: false,
            first_row_label: true,
            first_col_label: true,
        };
        let (cats, series) = derive_series(&src);
        let cats = cats.unwrap();
        assert_eq!(
            cats.areas[0].1,
            CellRangeLocal {
                r0: 1,
                c0: 0,
                r1: 5,
                c1: 0
            }
        );
        assert_eq!(series.len(), 2);
        assert_eq!(
            series[1].values.areas[0].1,
            CellRangeLocal {
                r0: 1,
                c0: 2,
                r1: 5,
                c1: 2
            }
        );
        assert!(matches!(&series[1].name, Some(SeriesNameData::Ref(r)) if r.areas[0].1.r0 == 0));
    }
}
