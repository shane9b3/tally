//! The v0.4 chart and picture queries behind the `DocumentService`
//! (`charts`, `chart_data`, `suggest_chart`, `images`).

use tally_api::{ApiError, ChartDataDto, ChartDto, ChartKind, DocId, ImageDto, RangeDto, Result};
use tally_types::SheetId;

use super::EngineService;

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

    /// The resolved data of a chart (cached values, no recalculation).
    pub fn chart_data_impl(&self, doc: DocId, sheet: SheetId, id: u32) -> Result<ChartDataDto> {
        let d = self.document(doc)?;
        let s = d.wb.sheet(sheet).ok_or_else(|| no_sheet(sheet))?;
        let chart = s
            .meta
            .charts
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ApiError::not_found(format!("no chart {id}")))?;
        Ok(crate::charts::resolve_data(&d.wb, sheet, chart, d.version))
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
