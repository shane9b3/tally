//! Charts and pictures (wave 5, W5-1): DTO ⇄ model conversion,
//! validation, data resolution, trend and error-bar math, and the
//! `ChartIndex` that maps changed cells to charts.
//!
//! W5-0a lands only the storage shape and the hooks. Every function here
//! is a pass-through so that chart data read from a file survives
//! `to_data`/`from_data` and undo; the behaviour comes in W5-1.
//!
//! TODO(W5-1): `from_api`/`to_api` (range text ⇄ `ChartRangeData` through
//! `parse_reference_text`), `resolve_data` (cached values, ≤255 series,
//! ≤32,000 points, AOO palette `004586 ff420e ffd320 579d1c 7e0021 83caff
//! 314004 aecf00 4b1f6f ff950e c5000b 0084d1`), `trend` (least squares on
//! transformed axes; R² identical to `RSQ`), `error_bars`, `suggest`
//! (region detection, labels heuristic, 16 × 9 cm anchor), and
//! `ChartIndex` for `ApplyResult.charts_changed`.

use tally_model::{ChartData, ImageData};
use tally_types::SheetId;

use crate::structure::Shape;
use crate::workbook::Workbook;

/// A chart with its sheet-local id.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The definition (resolved ranges).
    pub data: ChartData,
}

/// A picture with its sheet-local id.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Id from `SheetMeta::alloc_id`.
    pub id: u32,
    /// The picture.
    pub data: ImageData,
}

/// Most charts per document (reader and `AddChart` limit).
pub const MAX_CHARTS: usize = 1_000;
/// Most series per chart.
pub const MAX_SERIES: usize = 255;
/// Most points per series in `chart_data`.
pub const MAX_POINTS: usize = 32_000;

/// Loads a sheet's charts and pictures from the interchange model,
/// allocating ids (W5-0a: identity; W5-1 validates and resolves ranges).
pub fn from_data(wb: &mut Workbook, sheet: SheetId, charts: &[ChartData], images: &[ImageData]) {
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    for c in charts.iter().take(MAX_CHARTS) {
        let id = s.meta.alloc_id();
        s.meta.charts.push(Chart {
            id,
            data: c.clone(),
        });
    }
    for i in images.iter().take(MAX_CHARTS) {
        let id = s.meta.alloc_id();
        s.meta.images.push(Image {
            id,
            data: i.clone(),
        });
    }
}

/// The interchange form of a sheet's charts (W5-0a: identity).
pub fn to_data(wb: &Workbook, sheet: SheetId) -> Vec<ChartData> {
    wb.sheet(sheet)
        .map(|s| s.meta.charts.iter().map(|c| c.data.clone()).collect())
        .unwrap_or_default()
}

/// The interchange form of a sheet's pictures (W5-0a: identity).
pub fn images_to_data(wb: &Workbook, sheet: SheetId) -> Vec<ImageData> {
    wb.sheet(sheet)
        .map(|s| s.meta.images.iter().map(|i| i.data.clone()).collect())
        .unwrap_or_default()
}

/// Moves chart anchors and series ranges for a row/column change on
/// `sh.sheet`: anchors follow `point_after`, ranges `span_after`; a fully
/// deleted range becomes `invalid`. Charts on other sheets whose series
/// point at the changed sheet are adjusted too (as the CF code in
/// `structure.rs` does).
///
/// TODO(W5-1): implement; W5-0a is a no-op hook called from
/// `Workbook::adjust_meta_for_structure`.
pub fn adjust_structure(wb: &mut Workbook, sh: &Shape) {
    let _ = (wb, sh);
}
