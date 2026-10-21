//! The v0.4 chart and picture queries behind the `DocumentService`
//! (`charts`, `chart_data`, `suggest_chart`, `images`).
//!
//! `chart_data` responses are cached per (document, version, sheet,
//! chart) in a small LRU ([`ChartDataCache`]); a chart whose data
//! definition equals a cached one (frames that show the same chart
//! object) reuses that response. Any mutable access to a document
//! (`EngineService::document_mut`) forgets its entries.

use std::collections::VecDeque;
use std::sync::Arc;

use tally_api::{ApiError, ChartDataDto, ChartDto, ChartKind, DocId, ImageDto, RangeDto, Result};
use tally_model::ChartData;
use tally_types::SheetId;

use super::EngineService;
use crate::charts::{Chart, MAX_CHART_POINTS};

/// Most responses kept.
const CACHE_ENTRIES: usize = 64;
/// Most points kept over all entries.
const CACHE_POINTS: usize = 4 * MAX_CHART_POINTS;

struct Entry {
    doc: u32,
    version: u64,
    sheet: SheetId,
    chart: Arc<Chart>,
    dto: Arc<ChartDataDto>,
    points: usize,
}

/// The `chart_data` cache (see the module documentation).
#[derive(Default)]
pub(crate) struct ChartDataCache {
    entries: VecDeque<Entry>,
    points: usize,
}

/// Everything `resolve_data` reads from a chart definition.
fn same_data(a: &ChartData, b: &ChartData) -> bool {
    a.include_hidden == b.include_hidden
        && a.categories == b.categories
        && a.literal_categories == b.literal_categories
        && a.series == b.series
}

fn points_of(d: &ChartDataDto) -> usize {
    d.categories.len()
        + d.series
            .iter()
            .map(|s| {
                1 + s.values.len()
                    + s.x.as_ref().map_or(0, Vec::len)
                    + s.sizes.as_ref().map_or(0, Vec::len)
                    + s.trend.as_ref().map_or(0, |t| t.points.len())
            })
            .sum::<usize>()
}

impl ChartDataCache {
    /// Drops every entry of `doc`.
    pub(crate) fn forget(&mut self, doc: u32) {
        if self.entries.iter().any(|e| e.doc == doc) {
            self.entries.retain(|e| e.doc != doc);
            self.points = self.entries.iter().map(|e| e.points).sum();
        }
    }

    fn get(
        &mut self,
        doc: u32,
        version: u64,
        sheet: SheetId,
        chart: &Arc<Chart>,
    ) -> Option<ChartDataDto> {
        let i = self.entries.iter().position(|e| {
            e.doc == doc
                && e.version == version
                && e.sheet == sheet
                && (Arc::ptr_eq(&e.chart, chart) || same_data(&e.chart.data, &chart.data))
        })?;
        let e = self.entries.remove(i)?;
        let mut out = (*e.dto).clone();
        out.chart.id = chart.id;
        self.entries.push_back(e);
        Some(out)
    }

    fn put(
        &mut self,
        doc: u32,
        version: u64,
        sheet: SheetId,
        chart: Arc<Chart>,
        dto: ChartDataDto,
    ) {
        // Older versions of the document are never asked for again.
        if self
            .entries
            .iter()
            .any(|e| e.doc == doc && e.version != version)
        {
            self.entries
                .retain(|e| e.doc != doc || e.version == version);
            self.points = self.entries.iter().map(|e| e.points).sum();
        }
        let points = points_of(&dto);
        if points > CACHE_POINTS {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.points + points > CACHE_POINTS {
            match self.entries.pop_front() {
                Some(e) => self.points -= e.points,
                None => break,
            }
        }
        self.points += points;
        self.entries.push_back(Entry {
            doc,
            version,
            sheet,
            chart,
            dto: Arc::new(dto),
            points,
        });
    }
}

fn no_sheet(sheet: SheetId) -> ApiError {
    ApiError::not_found(format!("no sheet {sheet}"))
}

impl EngineService {
    /// The charts of a sheet, in id order.
    pub fn charts_impl(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ChartDto>> {
        let wb = &self.document(doc)?.wb;
        let s = wb.sheet(sheet).ok_or_else(|| no_sheet(sheet))?;
        Ok(s.meta
            .charts
            .iter()
            .map(|c| crate::charts::to_api(wb, sheet, c))
            .collect())
    }

    /// The resolved data of a chart (cached values, no recalculation),
    /// from the cache when the document has not changed since.
    pub fn chart_data_impl(&self, doc: DocId, sheet: SheetId, id: u32) -> Result<ChartDataDto> {
        let d = self.document(doc)?;
        let s = d.wb.sheet(sheet).ok_or_else(|| no_sheet(sheet))?;
        let chart = s
            .meta
            .charts
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ApiError::not_found(format!("no chart {id}")))?;
        let mut cache = self
            .chart_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hit) = cache.get(doc.0, d.version, sheet, chart) {
            return Ok(hit);
        }
        let dto = crate::charts::resolve_data(&d.wb, sheet, chart, d.version);
        cache.put(doc.0, d.version, sheet, Arc::clone(chart), dto.clone());
        Ok(dto)
    }

    /// A chart proposal for a selection (nothing is added).
    pub fn suggest_chart_impl(
        &self,
        doc: DocId,
        sheet: SheetId,
        sel: RangeDto,
        kind: ChartKind,
    ) -> Result<ChartDto> {
        let wb = &self.document(doc)?.wb;
        crate::charts::suggest(wb, sheet, &sel, kind)
    }

    /// The pictures of a sheet, without their bytes.
    pub fn images_impl(&self, doc: DocId, sheet: SheetId) -> Result<Vec<ImageDto>> {
        let wb = &self.document(doc)?.wb;
        let s = wb.sheet(sheet).ok_or_else(|| no_sheet(sheet))?;
        Ok(s.meta
            .images
            .iter()
            .map(|i| crate::images::to_api(i, false))
            .collect())
    }

    /// The bytes of a picture (base64), for the shell to serve to the UI.
    pub fn image_bytes(&self, doc: DocId, sheet: SheetId, id: u32) -> Result<ImageDto> {
        let wb = &self.document(doc)?.wb;
        let s = wb.sheet(sheet).ok_or_else(|| no_sheet(sheet))?;
        let img = s
            .meta
            .images
            .iter()
            .find(|i| i.id == id)
            .ok_or_else(|| ApiError::not_found(format!("no picture {id}")))?;
        Ok(crate::images::to_api(img, true))
    }
}
