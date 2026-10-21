//! Drawing parts (wave 5, W5-1): `xl/drawings/drawingN.xml` anchors and
//! the objects they hold (`c:chart r:id` graphic frames, `xdr:pic`
//! pictures), plus the loader that follows the drawing's relationships
//! to the chart parts and the media.
//!
//! Anchors: `xdr:twoCellAnchor` (from/to cells with EMU offsets; the size
//! is computed from the sheet's column widths and row heights),
//! `xdr:oneCellAnchor` (from cell plus `xdr:ext`) and
//! `xdr:absoluteAnchor` (`xdr:pos` plus `xdr:ext`, converted to a cell
//! anchor). 1 hmm = 360 EMU.

use std::collections::HashMap;
use std::io::{BufRead, Read, Seek};
use std::sync::Arc;

use tally_model::{ChartAnchorData, ChartData, DimRun, ImageData, SheetData};
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::archive::Package;
use crate::sheet::Budget;
use crate::workbook::{parse_rels, part_dir, rels_path, Rels};
use crate::xml::{Ev, XmlLimits, XmlReader};
use crate::{XlsxError, DEFAULT_COL_WIDTH_HMM, DEFAULT_ROW_HEIGHT_HMM};

/// Most charts read per document.
pub(crate) const MAX_CHARTS: usize = 1_000;
/// Most pictures read per sheet.
pub(crate) const MAX_IMAGES_PER_SHEET: usize = 1_000;
/// Largest chart part read.
pub(crate) const MAX_CHART_XML: u64 = 16 * 1024 * 1024;
/// Largest picture read.
pub(crate) const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Most picture bytes per document, counted once per anchor that shows a
/// picture (the bytes themselves are read and stored once).
pub(crate) const MAX_TOTAL_IMAGE_BYTES: u64 = tally_model::MAX_TOTAL_IMAGE_BYTES as u64;
/// Most objects read per drawing part.
const MAX_OBJECTS: usize = 4_000;

const EMU_PER_HMM: i64 = 360;

/// What an anchor holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DrawingKind {
    /// A chart part, by relationship id.
    Chart(String),
    /// A picture, by relationship id.
    Image(String),
    /// A shape, text box, …: skipped.
    Other,
}

/// A cell marker of an anchor (`xdr:from`/`xdr:to`), offsets in EMU.
#[derive(Clone, Copy, Debug, Default)]
struct Marker {
    col: u32,
    row: u32,
    col_off: i64,
    row_off: i64,
}

/// One anchored object.
#[derive(Clone, Debug)]
pub(crate) struct DrawingObject {
    pub name: String,
    from: Marker,
    to: Option<Marker>,
    /// `xdr:ext` in EMU.
    ext: Option<(i64, i64)>,
    /// `xdr:pos` of an absolute anchor, in EMU.
    pos: Option<(i64, i64)>,
    pub kind: DrawingKind,
}

fn marker<R: BufRead>(xr: &mut XmlReader<R>) -> Result<Marker, XlsxError> {
    let mut m = Marker::default();
    let d = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) => {
                let t = xr.text_content(64)?;
                let n: i64 = t.trim().parse().unwrap_or(0);
                match e.local() {
                    "col" => m.col = n.clamp(0, i64::from(MAX_COLS - 1)) as u32,
                    "row" => m.row = n.clamp(0, i64::from(MAX_ROWS - 1)) as u32,
                    "colOff" => m.col_off = n,
                    "rowOff" => m.row_off = n,
                    _ => {}
                }
            }
            Ev::End if xr.depth() < d => return Ok(m),
            Ev::Eof => return Ok(m),
            _ => {}
        }
    }
}

/// Parses one anchor element's children (after its start tag).
fn anchor<R: BufRead>(
    xr: &mut XmlReader<R>,
    out: &mut Vec<DrawingObject>,
) -> Result<(), XlsxError> {
    let mut obj = DrawingObject {
        name: String::new(),
        from: Marker::default(),
        to: None,
        ext: None,
        pos: None,
        kind: DrawingKind::Other,
    };
    let d = xr.depth();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "from" => obj.from = marker(xr)?,
                "to" => obj.to = Some(marker(xr)?),
                // `xdr:ext`/`xdr:pos` are direct children of the anchor; the
                // `a:ext` inside a frame's `xfrm` is not.
                "ext" if xr.depth() == d + 1 => {
                    let cx = e.attr_local("cx").and_then(|v| v.parse::<i64>().ok());
                    let cy = e.attr_local("cy").and_then(|v| v.parse::<i64>().ok());
                    if let (Some(cx), Some(cy)) = (cx, cy) {
                        obj.ext = Some((cx, cy));
                    }
                    xr.skip()?;
                }
                "pos" if xr.depth() == d + 1 => {
                    let x = e.attr_local("x").and_then(|v| v.parse::<i64>().ok());
                    let y = e.attr_local("y").and_then(|v| v.parse::<i64>().ok());
                    if let (Some(x), Some(y)) = (x, y) {
                        obj.pos = Some((x, y));
                    }
                    xr.skip()?;
                }
                "graphicFrame" | "pic" | "sp" | "grpSp" | "cxnSp" | "nvGraphicFramePr"
                | "nvPicPr" | "graphic" | "graphicData" | "blipFill" | "xfrm" | "spPr"
                | "nvSpPr" => {}
                "cNvPr" => {
                    if let Some(n) = e.attr_local("name") {
                        obj.name = n.chars().take(64).collect();
                    }
                    xr.skip()?;
                }
                "chart" => {
                    if let Some(id) = e.attr_local("id") {
                        if obj.kind == DrawingKind::Other {
                            obj.kind = DrawingKind::Chart(id.to_string());
                        }
                    }
                    xr.skip()?;
                }
                "blip" => {
                    if let Some(id) = e.attr_local("embed") {
                        if obj.kind == DrawingKind::Other {
                            obj.kind = DrawingKind::Image(id.to_string());
                        }
                    }
                    xr.skip()?;
                }
                _ => xr.skip()?,
            },
            Ev::End if xr.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    if out.len() < MAX_OBJECTS {
        out.push(obj);
    }
    Ok(())
}

/// Parses a drawing part.
pub(crate) fn parse_drawing<R: BufRead>(
    xr: &mut XmlReader<R>,
) -> Result<Vec<DrawingObject>, XlsxError> {
    let mut out = Vec::new();
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                "wsDr" => {}
                "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor" => anchor(xr, &mut out)?,
                _ => xr.skip()?,
            },
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn run_size(runs: &[DimRun], i: u32, default: u32) -> i64 {
    match runs.iter().find(|r| r.start <= i && i <= r.end) {
        Some(r) if r.hidden => 0,
        Some(r) => i64::from(r.size_hmm),
        None => i64::from(default),
    }
}

/// The hmm extent between two markers along one axis.
fn extent(runs: &[DimRun], default: u32, a: u32, a_off: i64, b: u32, b_off: i64) -> i64 {
    let mut total = 0i64;
    let mut i = a;
    let mut steps = 0;
    while i < b && steps < 100_000 {
        total += run_size(runs, i, default);
        i += 1;
        steps += 1;
    }
    total - a_off / EMU_PER_HMM + b_off / EMU_PER_HMM
}

/// The cell under a page offset (hmm) and the remainder inside it.
fn cell_at(runs: &[DimRun], default: u32, pos: i64, limit: u32) -> (u32, u32) {
    let mut p = pos.max(0);
    let mut i = 0u32;
    let mut steps = 0;
    loop {
        let size = run_size(runs, i, default);
        if p < size || i >= limit - 1 || steps > 100_000 {
            break;
        }
        p -= size;
        i += 1;
        steps += 1;
    }
    (i, p.clamp(0, i64::from(u32::MAX)) as u32)
}

fn hmm(v: i64) -> u32 {
    (v / EMU_PER_HMM).clamp(0, i64::from(u32::MAX)) as u32
}

/// The cell anchor of an object, using the sheet's dimensions for
/// two-cell and absolute anchors.
pub(crate) fn to_anchor(obj: &DrawingObject, sheet: &SheetData) -> ChartAnchorData {
    let (row, col, dx, dy) = match obj.pos {
        Some((x, y)) => {
            let (c, dx) = cell_at(
                &sheet.col_widths,
                DEFAULT_COL_WIDTH_HMM,
                x / EMU_PER_HMM,
                MAX_COLS,
            );
            let (r, dy) = cell_at(
                &sheet.row_heights,
                DEFAULT_ROW_HEIGHT_HMM,
                y / EMU_PER_HMM,
                MAX_ROWS,
            );
            (r, c, dx, dy)
        }
        None => (
            obj.from.row,
            obj.from.col,
            hmm(obj.from.col_off),
            hmm(obj.from.row_off),
        ),
    };
    let (w, h) = match (obj.ext, obj.to) {
        (Some((cx, cy)), _) => (hmm(cx), hmm(cy)),
        (None, Some(to)) => (
            extent(
                &sheet.col_widths,
                DEFAULT_COL_WIDTH_HMM,
                obj.from.col,
                obj.from.col_off,
                to.col,
                to.col_off,
            )
            .clamp(0, i64::from(u32::MAX)) as u32,
            extent(
                &sheet.row_heights,
                DEFAULT_ROW_HEIGHT_HMM,
                obj.from.row,
                obj.from.row_off,
                to.row,
                to.row_off,
            )
            .clamp(0, i64::from(u32::MAX)) as u32,
        ),
        (None, None) => (16_000, 9_000),
    };
    ChartAnchorData {
        row,
        col,
        dx_hmm: dx,
        dy_hmm: dy,
        width_hmm: w.clamp(100, 1_000_000),
        height_hmm: h.clamp(100, 1_000_000),
    }
}

/// A media part: media type and shared bytes.
type Picture = (String, Arc<[u8]>);

/// Document-wide counters of the objects read, and the parts already
/// loaded (each media part is read once and shared by every anchor that
/// shows it; each chart part is parsed once per sheet).
pub(crate) struct ObjectBudget {
    pub charts: usize,
    /// Picture bytes referenced so far (every anchor counts its picture).
    pub image_bytes: u64,
    pub capped: bool,
    pub skipped: usize,
    /// What remains of [`tally_model::MAX_LITERAL_POINTS`].
    pub literal_budget: usize,
    /// Some chart lost its literal values to the budget.
    pub literal_dropped: bool,
    /// What remains of [`tally_model::MAX_SERIES_PER_DOCUMENT`].
    pub series_budget: usize,
    /// Some chart lost series to that budget.
    pub series_cut: bool,
    /// Media part → (media type, bytes); `None`: unreadable or not PNG/JPEG.
    media: HashMap<String, Option<Picture>>,
    /// (chart part, sheet tab) → the parsed chart.
    parsed: HashMap<(String, u32), Option<ChartData>>,
}

impl Default for ObjectBudget {
    fn default() -> Self {
        Self {
            charts: 0,
            image_bytes: 0,
            capped: false,
            skipped: 0,
            literal_budget: tally_model::MAX_LITERAL_POINTS,
            literal_dropped: false,
            series_budget: tally_model::MAX_SERIES_PER_DOCUMENT,
            series_cut: false,
            media: HashMap::new(),
            parsed: HashMap::new(),
        }
    }
}

impl ObjectBudget {
    /// Charges a further copy of a shared chart's literal points, or
    /// drops its literal values when the budget is spent.
    fn charge_copy(&mut self, c: &mut ChartData) {
        let n = c.literal_points();
        if n > self.literal_budget {
            c.drop_literals();
            self.literal_dropped = true;
        } else {
            self.literal_budget -= n;
        }
    }
}

/// Loads the charts and pictures of a sheet's drawing part into `sheet`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn attach<R: Read + Seek>(
    pkg: &mut Package<R>,
    drawing: &str,
    sheet: &mut SheetData,
    tab: u32,
    names: &[String],
    budget: &mut Budget,
    objects: &mut ObjectBudget,
    xl: XmlLimits,
    other: u64,
) -> Result<(), XlsxError> {
    let objs = pkg
        .with_entry(drawing, other, |r| {
            parse_drawing(&mut XmlReader::new(r, xl))
        })?
        .unwrap_or_default();
    if objs.is_empty() {
        return Ok(());
    }
    let rels: Rels = pkg
        .with_entry(&rels_path(drawing), other, |r| {
            parse_rels(&mut XmlReader::new(r, xl), part_dir(drawing))
        })?
        .unwrap_or_default();
    let by_id: HashMap<&str, &crate::workbook::Rel> = rels
        .list
        .iter()
        .filter(|r| !r.external)
        .map(|r| (r.id.as_str(), r))
        .collect();
    for obj in objs {
        let anchor = to_anchor(&obj, sheet);
        match &obj.kind {
            DrawingKind::Chart(rid) => {
                let Some(rel) = by_id.get(rid.as_str()).filter(|r| r.is("chart")) else {
                    objects.skipped += 1;
                    continue;
                };
                if objects.charts >= MAX_CHARTS {
                    objects.capped = true;
                    continue;
                }
                let key = (rel.target.clone(), tab);
                let parsed = if let Some(p) = objects.parsed.get(&key) {
                    p.clone().map(|mut c| {
                        objects.charge_copy(&mut c);
                        c
                    })
                } else {
                    let mut warnings = Vec::new();
                    let mut literal_budget = objects.literal_budget;
                    let p = pkg
                        .with_entry(&rel.target, MAX_CHART_XML, |r| {
                            crate::chart_read::parse_chart(
                                &mut XmlReader::new(r, xl),
                                names,
                                tab,
                                &mut warnings,
                                &mut literal_budget,
                            )
                        })?
                        .flatten();
                    objects.literal_budget = literal_budget;
                    for w in warnings {
                        budget.warn(w);
                    }
                    objects.parsed.insert(key, p.clone());
                    p
                };
                match parsed {
                    Some(mut c) => {
                        objects.charts += 1;
                        if c.series.len() > objects.series_budget {
                            c.series.truncate(objects.series_budget);
                            objects.series_cut = true;
                        }
                        objects.series_budget -= c.series.len();
                        c.anchor = anchor;
                        c.name = if obj.name.is_empty() {
                            format!("Chart {}", objects.charts)
                        } else {
                            obj.name.clone()
                        };
                        sheet.charts.push(c);
                    }
                    None => objects.skipped += 1,
                }
            }
            DrawingKind::Image(rid) => {
                let Some(rel) = by_id.get(rid.as_str()).filter(|r| r.is("image")) else {
                    objects.skipped += 1;
                    continue;
                };
                if sheet.images.len() >= MAX_IMAGES_PER_SHEET {
                    objects.capped = true;
                    continue;
                }
                if objects.image_bytes >= MAX_TOTAL_IMAGE_BYTES {
                    objects.capped = true;
                    continue;
                }
                let picture = match objects.media.get(&rel.target) {
                    Some(m) => m.clone(),
                    None => {
                        let bytes = pkg.with_entry(&rel.target, MAX_IMAGE_BYTES, |r| {
                            let mut v = Vec::new();
                            r.read_to_end(&mut v)?;
                            Ok(v)
                        })?;
                        let m = bytes.and_then(|bytes| {
                            let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                                "image/png"
                            } else if bytes.starts_with(b"\xff\xd8\xff") {
                                "image/jpeg"
                            } else {
                                return None;
                            };
                            Some((mime.to_string(), Arc::<[u8]>::from(bytes)))
                        });
                        objects.media.insert(rel.target.clone(), m.clone());
                        m
                    }
                };
                let Some((mime, bytes)) = picture else {
                    objects.skipped += 1;
                    continue;
                };
                // Every reference counts against the document total.
                if objects.image_bytes + bytes.len() as u64 > MAX_TOTAL_IMAGE_BYTES {
                    objects.capped = true;
                    continue;
                }
                objects.image_bytes += bytes.len() as u64;
                sheet.images.push(ImageData {
                    name: obj.name.clone(),
                    anchor,
                    mime,
                    bytes,
                });
            }
            DrawingKind::Other => objects.skipped += 1,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_cell_extent_uses_dimensions() {
        let sheet = SheetData {
            col_widths: vec![DimRun {
                start: 0,
                end: 9,
                size_hmm: 2000,
                hidden: false,
            }],
            row_heights: vec![DimRun {
                start: 0,
                end: 99,
                size_hmm: 500,
                hidden: false,
            }],
            ..SheetData::default()
        };
        let obj = DrawingObject {
            name: String::new(),
            from: Marker {
                col: 1,
                row: 2,
                col_off: 360 * 100,
                row_off: 0,
            },
            to: Some(Marker {
                col: 4,
                row: 6,
                col_off: 360 * 500,
                row_off: 360 * 250,
            }),
            ext: None,
            pos: None,
            kind: DrawingKind::Other,
        };
        let a = to_anchor(&obj, &sheet);
        assert_eq!((a.row, a.col, a.dx_hmm, a.dy_hmm), (2, 1, 100, 0));
        assert_eq!(a.width_hmm, 3 * 2000 - 100 + 500);
        assert_eq!(a.height_hmm, 4 * 500 + 250);
    }
}
