//! The v0.4 ops: charts and pictures (wave 5, W5-1). Dispatch pattern as
//! `ops_v03`: every op records the sheet meta it replaces through
//! `with_meta` (`Change::SheetMeta`) and reports its effects
//! (`fx.objects(sheet)`, `fx.created(..)` with the placeholder op index
//! the document fills in).
//!
//! All ops are refused on a protected sheet (AOO protects objects with
//! the sheet).

use std::sync::Arc;

use tally_api::{ApiError, Op};
use tally_types::SheetId;

use crate::charts::{self, Chart, Image, MAX_CHARTS, MAX_IMAGES};
use crate::ops_v03::with_meta;
use crate::protection::check_sheet_action;
use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Applies a v0.4 op; `None` for the other ops.
pub fn apply(
    wb: &mut Workbook,
    op: &Op,
    fx: &mut Effects,
) -> Option<Result<Vec<Change>, ApiError>> {
    Some(match op {
        Op::AddChart { sheet, chart } => add_chart(wb, *sheet, chart, fx),
        Op::UpdateChart { sheet, id, chart } => update_chart(wb, *sheet, *id, chart, fx),
        Op::MoveResizeChart { sheet, id, anchor } => move_resize_chart(wb, *sheet, *id, anchor, fx),
        Op::DeleteChart { sheet, id } => delete_chart(wb, *sheet, *id, fx),
        Op::MoveChartToSheet { sheet, id, to } => move_chart_to_sheet(wb, *sheet, *id, *to, fx),
        Op::AddImage { sheet, image } => add_image(wb, *sheet, image, fx),
        Op::DeleteImage { sheet, id } => delete_image(wb, *sheet, *id, fx),
        Op::MoveResizeImage { sheet, id, anchor } => move_resize_image(wb, *sheet, *id, anchor, fx),
        _ => return None,
    })
}

fn not_found(what: &str, id: u32) -> ApiError {
    ApiError::not_found(format!("no {what} {id}"))
}

fn protected(wb: &Workbook, sheet: SheetId) -> Result<(), ApiError> {
    check_sheet_action(wb, sheet, |_| false)
}

fn add_chart(
    wb: &mut Workbook,
    sheet: SheetId,
    dto: &tally_api::ChartDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let mut created = None;
    let changes = with_meta(wb, sheet, fx, |wb, meta| {
        // `meta` is taken out of the sheet, so the document count is the
        // other sheets plus this one.
        if charts::chart_count(wb) + meta.charts.len() >= MAX_CHARTS {
            return Err(ApiError::invalid_argument(format!(
                "a document may hold at most {MAX_CHARTS} charts"
            )));
        }
        let data = charts::from_api(wb, sheet, dto)?;
        let id = meta.alloc_id();
        let mut data = data;
        if data.name.is_empty() {
            data.name = format!("Object {id}");
        }
        Arc::make_mut(&mut meta.charts).push(Arc::new(Chart { id, data }));
        created = Some(id);
        Ok(())
    })?;
    if let Some(id) = created {
        fx.created(u32::MAX, id);
    }
    fx.objects(sheet);
    Ok(changes)
}

fn update_chart(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    dto: &tally_api::ChartDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let changes = with_meta(wb, sheet, fx, |wb, meta| {
        let i = meta
            .charts
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("chart", id))?;
        let mut data = charts::from_api(wb, sheet, dto)?;
        if data.name.is_empty() {
            data.name.clone_from(&meta.charts[i].data.name);
        }
        // Literal data (from a file) survives an update that keeps the
        // series count: the UI never sends it.
        let old = &meta.charts[i].data;
        if data.literal_categories.is_none() {
            data.literal_categories.clone_from(&old.literal_categories);
        }
        for (n, o) in data.series.iter_mut().zip(&old.series) {
            if n.values.is_unset() && n.literal_values.is_none() {
                n.literal_values.clone_from(&o.literal_values);
            }
        }
        Arc::make_mut(&mut meta.charts)[i] = Arc::new(Chart { id, data });
        Ok(())
    })?;
    fx.objects(sheet);
    Ok(changes)
}

fn move_resize_chart(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    anchor: &tally_api::ChartAnchor,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let anchor = charts::anchor_from_api(anchor)?;
    let changes = with_meta(wb, sheet, fx, |_, meta| {
        let i = meta
            .charts
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("chart", id))?;
        if meta.charts[i].data.anchor == anchor {
            return Ok(());
        }
        let mut data = meta.charts[i].data.clone();
        data.anchor = anchor;
        Arc::make_mut(&mut meta.charts)[i] = Arc::new(Chart { id, data });
        Ok(())
    })?;
    fx.objects(sheet);
    Ok(changes)
}

fn delete_chart(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let changes = with_meta(wb, sheet, fx, |_, meta| {
        let i = meta
            .charts
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("chart", id))?;
        Arc::make_mut(&mut meta.charts).remove(i);
        Ok(())
    })?;
    fx.objects(sheet);
    Ok(changes)
}

fn move_chart_to_sheet(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    to: SheetId,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    protected(wb, to)?;
    crate::ops::check_sheet(wb, to)?;
    if sheet == to {
        return Ok(Vec::new());
    }
    let mut taken: Option<Arc<Chart>> = None;
    let mut changes = with_meta(wb, sheet, fx, |_, meta| {
        let i = meta
            .charts
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("chart", id))?;
        taken = Some(Arc::make_mut(&mut meta.charts).remove(i));
        Ok(())
    })?;
    let chart = taken.ok_or_else(|| not_found("chart", id))?;
    let mut created = None;
    let more = with_meta(wb, to, fx, |_, meta| {
        let new_id = meta.alloc_id();
        Arc::make_mut(&mut meta.charts).push(Arc::new(Chart {
            id: new_id,
            data: chart.data.clone(),
        }));
        created = Some(new_id);
        Ok(())
    })?;
    changes.extend(more);
    if let Some(new_id) = created {
        fx.created(u32::MAX, new_id);
    }
    fx.objects(sheet);
    fx.objects(to);
    Ok(changes)
}

fn add_image(
    wb: &mut Workbook,
    sheet: SheetId,
    dto: &tally_api::ImageDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let mut created = None;
    let changes = with_meta(wb, sheet, fx, |wb, meta| {
        if meta.images.len() >= MAX_IMAGES {
            return Err(ApiError::invalid_argument(format!(
                "a sheet may hold at most {MAX_IMAGES} pictures"
            )));
        }
        let mut image = crate::images::from_api(wb, sheet, dto)?;
        let id = meta.alloc_id();
        image.id = id;
        if image.data.name.is_empty() {
            image.data.name = format!("Image {id}");
        }
        Arc::make_mut(&mut meta.images).push(Arc::new(image));
        created = Some(id);
        Ok(())
    })?;
    if let Some(id) = created {
        fx.created(u32::MAX, id);
    }
    fx.objects(sheet);
    Ok(changes)
}

fn delete_image(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let changes = with_meta(wb, sheet, fx, |_, meta| {
        let i = meta
            .images
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("picture", id))?;
        Arc::make_mut(&mut meta.images).remove(i);
        Ok(())
    })?;
    fx.objects(sheet);
    Ok(changes)
}

fn move_resize_image(
    wb: &mut Workbook,
    sheet: SheetId,
    id: u32,
    anchor: &tally_api::ChartAnchor,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    protected(wb, sheet)?;
    let anchor = charts::anchor_from_api(anchor)?;
    let changes = with_meta(wb, sheet, fx, |_, meta| {
        let i = meta
            .images
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| not_found("picture", id))?;
        if meta.images[i].data.anchor == anchor {
            return Ok(());
        }
        let img = Image {
            id,
            data: tally_model::ImageData {
                anchor,
                ..meta.images[i].data.clone()
            },
        };
        Arc::make_mut(&mut meta.images)[i] = Arc::new(img);
        Ok(())
    })?;
    fx.objects(sheet);
    Ok(changes)
}
