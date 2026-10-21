//! Frames (wave 5, W5-1): the `draw:frame` of a chart or picture inside
//! its anchor cell, the manifest entries of the objects and the
//! `Pictures/` entries; on read, the frame attributes and the loading of
//! the referenced objects.
//!
//! Chart objects are numbered `Object 1..N` across the document in sheet
//! order. [`object_manifest_entries`] runs first (the manifest is the
//! second zip entry) and records each sheet's first object number in a
//! thread-local, which [`write_cell_frames`] reads; [`write_objects`]
//! recomputes the same numbering from the data.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufRead, Read, Seek, Write};
use std::sync::Arc;

use tally_model::{ChartAnchorData, ChartData, DimRun, ImageData, SheetData, WorkbookData};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::archive::Package;
use crate::units::{hmm_to_cm, length_to_hmm};
use crate::write::odf_sheet_name;
use crate::xml::{escape_attr, Ev, XmlLimits, XmlReader};
use crate::{Limits, OdsError, WriteReport, DEFAULT_COL_WIDTH_HMM, DEFAULT_ROW_HEIGHT_HMM};

/// Media type of a chart object.
pub(crate) const CHART_MEDIA: &str = "application/vnd.oasis.opendocument.chart";
/// Most charts read per document.
pub(crate) const MAX_CHARTS: usize = 1_000;
/// Most pictures read per sheet.
pub(crate) const MAX_IMAGES_PER_SHEET: usize = 1_000;
/// Largest object XML read.
pub(crate) const MAX_OBJECT_XML: u64 = 16 * 1024 * 1024;
/// Largest picture read.
pub(crate) const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Most picture bytes per document, counted once per frame that shows a
/// picture (the bytes themselves are read and stored once).
pub(crate) const MAX_TOTAL_IMAGE_BYTES: u64 = tally_model::MAX_TOTAL_IMAGE_BYTES as u64;

thread_local! {
    /// Per sheet (by tab): the number of the sheet's first chart object,
    /// keyed by the sheet count so a stale value from another document is
    /// never used (see the module documentation).
    static OBJECT_OFFSETS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// The anchored cells of the sheet being written.
    static FRAME_CELLS: RefCell<Option<FrameCache>> = const { RefCell::new(None) };
}

/// Anchored cells of one sheet, keyed by its address and object counts.
struct FrameCache {
    key: (usize, usize, usize),
    cells: HashSet<(u32, u32)>,
}

fn offsets(data: &WorkbookData) -> Vec<usize> {
    let mut out = Vec::with_capacity(data.sheets.len());
    let mut n = 0usize;
    for s in &data.sheets {
        out.push(n);
        n += s.charts.len();
    }
    out
}

/// FNV-1a of the picture bytes, as the file name in `Pictures/`.
fn picture_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn picture_ext(mime: &str, bytes: &[u8]) -> &'static str {
    if mime.eq_ignore_ascii_case("image/jpeg") || bytes.starts_with(b"\xff\xd8\xff") {
        "jpg"
    } else {
        "png"
    }
}

/// `Pictures/<hash>.<ext>` of a picture.
pub(crate) fn picture_path(img: &ImageData) -> String {
    format!(
        "Pictures/{:016x}.{}",
        picture_hash(&img.bytes),
        picture_ext(&img.mime, &img.bytes)
    )
}

fn picture_media(img: &ImageData) -> &'static str {
    match picture_ext(&img.mime, &img.bytes) {
        "jpg" => "image/jpeg",
        _ => "image/png",
    }
}

/// The manifest entries of the chart objects and pictures (`(full path,
/// media type)`); also records the object numbering for the cell frames.
pub(crate) fn object_manifest_entries(data: &WorkbookData) -> Vec<(String, String)> {
    let offs = offsets(data);
    OBJECT_OFFSETS.with(|o| *o.borrow_mut() = offs.clone());
    FRAME_CELLS.with(|f| *f.borrow_mut() = None);
    let mut out = Vec::new();
    let total: usize = data.sheets.iter().map(|s| s.charts.len()).sum();
    for n in 1..=total.min(MAX_CHARTS) {
        out.push((format!("Object {n}/"), CHART_MEDIA.to_string()));
        out.push((format!("Object {n}/content.xml"), "text/xml".to_string()));
        out.push((format!("Object {n}/styles.xml"), "text/xml".to_string()));
    }
    let mut seen = HashSet::new();
    for s in &data.sheets {
        for img in s.images.iter().take(MAX_IMAGES_PER_SHEET) {
            if img.bytes.is_empty() || img.bytes.len() as u64 > MAX_IMAGE_BYTES {
                continue;
            }
            let path = picture_path(img);
            if seen.insert(path.clone()) {
                out.push((path, picture_media(img).to_string()));
            }
        }
    }
    out
}

/// The object number (1-based, document-wide) of chart `i` of sheet `tab`.
fn object_number(tab: usize, i: usize) -> Option<usize> {
    let n = OBJECT_OFFSETS.with(|o| o.borrow().get(tab).copied())? + i + 1;
    (n <= MAX_CHARTS).then_some(n)
}

/// `true` when a cell holds an anchored object.
pub(crate) fn cell_has_frame(sheet: &SheetData, r: u32, c: u32) -> bool {
    if sheet.charts.is_empty() && sheet.images.is_empty() {
        return false;
    }
    let key = (
        sheet as *const SheetData as usize,
        sheet.charts.len(),
        sheet.images.len(),
    );
    FRAME_CELLS.with(|f| {
        let mut f = f.borrow_mut();
        if f.as_ref().is_none_or(|cache| cache.key != key) {
            let cells: HashSet<(u32, u32)> = sheet
                .charts
                .iter()
                .map(|ch| (ch.anchor.row, ch.anchor.col))
                .chain(sheet.images.iter().map(|i| (i.anchor.row, i.anchor.col)))
                .collect();
            *f = Some(FrameCache { key, cells });
        }
        f.as_ref()
            .is_some_and(|cache| cache.cells.contains(&(r, c)))
    })
}

fn run_size(runs: &[DimRun], i: u32, default: u32) -> u32 {
    match runs.iter().find(|r| r.start <= i && i <= r.end) {
        Some(r) if r.hidden => 0,
        Some(r) => r.size_hmm,
        None => default,
    }
}

/// The end cell and the offset inside it for an object anchored at
/// `(row, col)` with the given size.
fn end_cell(sheet: &SheetData, a: &ChartAnchorData) -> (u32, u32, u32, u32) {
    let mut x = u64::from(a.dx_hmm) + u64::from(a.width_hmm);
    let mut col = a.col;
    let mut steps = 0;
    loop {
        let w = u64::from(run_size(&sheet.col_widths, col, DEFAULT_COL_WIDTH_HMM));
        if x < w || col >= tally_types::MAX_COLS - 1 || steps > 10_000 {
            break;
        }
        x -= w;
        col += 1;
        steps += 1;
    }
    let mut y = u64::from(a.dy_hmm) + u64::from(a.height_hmm);
    let mut row = a.row;
    steps = 0;
    loop {
        let h = u64::from(run_size(&sheet.row_heights, row, DEFAULT_ROW_HEIGHT_HMM));
        if y < h || row >= tally_types::MAX_ROWS - 1 || steps > 100_000 {
            break;
        }
        y -= h;
        row += 1;
        steps += 1;
    }
    (row, col, x as u32, y as u32)
}

fn frame_open(s: &mut String, sheet: &SheetData, name: &str, z: usize, a: &ChartAnchorData) {
    let (er, ec, ex, ey) = end_cell(sheet, a);
    let sheet_name = odf_sheet_name(&sheet.name);
    let _ = write!(s, r#"<draw:frame draw:z-index="{z}""#);
    s.push_str(r#" draw:name=""#);
    escape_attr(name, s);
    s.push('"');
    let _ = write!(
        s,
        r#" svg:x="{}" svg:y="{}" svg:width="{}" svg:height="{}""#,
        hmm_to_cm(a.dx_hmm),
        hmm_to_cm(a.dy_hmm),
        hmm_to_cm(a.width_hmm),
        hmm_to_cm(a.height_hmm)
    );
    s.push_str(r#" table:end-cell-address=""#);
    escape_attr(
        &format!(
            "{sheet_name}.{}{}",
            tally_types::col_to_letters(ec),
            u64::from(er) + 1
        ),
        s,
    );
    let _ = write!(
        s,
        r#"" table:end-x="{}" table:end-y="{}">"#,
        hmm_to_cm(ex),
        hmm_to_cm(ey)
    );
}

/// The ranges a chart plots, for `draw:notify-on-update-of-ranges`.
fn notify_ranges(c: &ChartData, sheets: &[SheetData]) -> String {
    let mut parts = Vec::new();
    let mut push = |r: &tally_model::ChartRangeData| {
        for (tab, a) in &r.areas {
            if let Some(sh) = sheets.get(*tab as usize) {
                let n = odf_sheet_name(&sh.name);
                parts.push(format!(
                    "{n}.{}{}:{n}.{}{}",
                    tally_types::col_to_letters(a.c0),
                    u64::from(a.r0) + 1,
                    tally_types::col_to_letters(a.c1),
                    u64::from(a.r1) + 1
                ));
            }
        }
    };
    if let Some(r) = &c.categories {
        push(r);
    }
    for s in &c.series {
        push(&s.values);
    }
    parts.truncate(64);
    parts.join(" ")
}

/// Writes the `draw:frame` elements of the objects anchored at `(r, c)`.
pub(crate) fn write_cell_frames(
    s: &mut String,
    data_sheets: Option<&[SheetData]>,
    sheet: &SheetData,
    tab: usize,
    r: u32,
    c: u32,
) {
    let mut z = 0usize;
    for (i, chart) in sheet.charts.iter().enumerate() {
        if chart.anchor.row != r || chart.anchor.col != c {
            continue;
        }
        let Some(n) = object_number(tab, i) else {
            continue;
        };
        let name = if chart.name.is_empty() {
            format!("Object {n}")
        } else {
            chart.name.clone()
        };
        frame_open(s, sheet, &name, z, &chart.anchor);
        z += 1;
        s.push_str("<draw:object");
        if let Some(sheets) = data_sheets {
            let ranges = notify_ranges(chart, sheets);
            if !ranges.is_empty() {
                s.push_str(r#" draw:notify-on-update-of-ranges=""#);
                escape_attr(&ranges, s);
                s.push('"');
            }
        }
        let _ = write!(
            s,
            r#" xlink:href="./Object {n}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>"#
        );
        s.push_str("</draw:frame>");
    }
    for (i, img) in sheet.images.iter().enumerate().take(MAX_IMAGES_PER_SHEET) {
        if img.anchor.row != r || img.anchor.col != c {
            continue;
        }
        if img.bytes.is_empty() || img.bytes.len() as u64 > MAX_IMAGE_BYTES {
            continue;
        }
        let name = if img.name.is_empty() {
            format!("Image {}", i + 1)
        } else {
            img.name.clone()
        };
        frame_open(s, sheet, &name, z, &img.anchor);
        z += 1;
        s.push_str(r#"<draw:image xlink:href=""#);
        escape_attr(&picture_path(img), s);
        s.push_str(
            r#"" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
        );
    }
}

/// Writes the chart objects and the pictures as zip entries.
pub(crate) fn write_objects<W: Write + Seek>(
    data: &WorkbookData,
    zip: &mut ZipWriter<W>,
    report: &mut WriteReport,
) -> Result<(), OdsError> {
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut cells = crate::chart::CellIndex::new(&data.sheets);
    let mut n = 0usize;
    let mut dropped = 0usize;
    for sheet in &data.sheets {
        for chart in &sheet.charts {
            n += 1;
            if n > MAX_CHARTS {
                dropped += 1;
                continue;
            }
            if chart.series.len() > crate::chart::MAX_SERIES {
                report.warnings.push(format!(
                    "chart {:?}: only the first {} series were written",
                    chart.name,
                    crate::chart::MAX_SERIES
                ));
            }
            zip.start_file(format!("Object {n}/content.xml"), deflated)?;
            zip.write_all(crate::chart::content_xml(chart, &data.sheets, &mut cells).as_bytes())?;
            zip.start_file(format!("Object {n}/styles.xml"), deflated)?;
            zip.write_all(crate::chart::styles_xml().as_bytes())?;
        }
    }
    if dropped > 0 {
        report.warnings.push(format!(
            "only the first {MAX_CHARTS} charts were written ({dropped} dropped)"
        ));
    }
    let mut seen = HashSet::new();
    for sheet in &data.sheets {
        for img in sheet.images.iter().take(MAX_IMAGES_PER_SHEET) {
            if img.bytes.is_empty() || img.bytes.len() as u64 > MAX_IMAGE_BYTES {
                report.warnings.push(format!(
                    "picture {:?} was not written (empty or over 16 MiB)",
                    img.name
                ));
                continue;
            }
            let path = picture_path(img);
            if seen.insert(path.clone()) {
                zip.start_file(path, stored)?;
                zip.write_all(&img.bytes)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// What a `draw:frame` refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameKind {
    /// `draw:object xlink:href` (a chart when the manifest says so).
    Object(String),
    /// `draw:image xlink:href`.
    Image(String),
    /// Something else (text box, OLE…): skipped and counted.
    Other,
}

/// A frame as read from a cell or from `table:shapes`.
#[derive(Clone, Debug)]
pub(crate) struct RawFrame {
    pub name: String,
    pub x_hmm: u32,
    pub y_hmm: u32,
    pub width_hmm: u32,
    pub height_hmm: u32,
    pub kind: FrameKind,
    /// From `table:shapes`: `x`/`y` are page offsets, converted to a
    /// cell anchor once the sheet's dimensions are known.
    pub page_anchored: bool,
}

/// A frame with its sheet and anchor cell.
#[derive(Clone, Debug)]
pub(crate) struct PlacedFrame {
    pub tab: u32,
    pub row: u32,
    pub col: u32,
    pub frame: RawFrame,
}

const MAX_HREF: usize = 1024;

/// Parses a `draw:frame` whose start tag was just read (consumes it).
pub(crate) fn parse_frame<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &crate::xml::Elem,
    page_anchored: bool,
) -> Result<RawFrame, OdsError> {
    let len = |k: &str| e.attr(k).and_then(length_to_hmm).unwrap_or(0);
    let mut f = RawFrame {
        name: e.attr("draw:name").unwrap_or("").chars().take(64).collect(),
        x_hmm: len("svg:x"),
        y_hmm: len("svg:y"),
        width_hmm: len("svg:width"),
        height_hmm: len("svg:height"),
        kind: FrameKind::Other,
        page_anchored,
    };
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) => {
                let href = c
                    .attr("xlink:href")
                    .filter(|h| h.len() <= MAX_HREF)
                    .map(str::to_string);
                match (c.name.as_str(), href) {
                    ("draw:object", Some(h)) if f.kind == FrameKind::Other => {
                        f.kind = FrameKind::Object(h);
                    }
                    ("draw:image", Some(h)) if f.kind == FrameKind::Other => {
                        f.kind = FrameKind::Image(h);
                    }
                    _ => {}
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => break,
            Ev::Eof => break,
            _ => {}
        }
    }
    Ok(f)
}

/// `./Object 12` / `Object 12` → `Object 12`; anything else is refused.
fn object_dir(href: &str) -> Option<String> {
    let t = href.strip_prefix("./").unwrap_or(href);
    let t = t.strip_suffix('/').unwrap_or(t);
    let n = t.strip_prefix("Object ")?;
    if n.is_empty() || n.len() > 5 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(t.to_string())
}

/// `Pictures/abc.png` (one path component under `Pictures/`).
fn picture_entry(href: &str) -> Option<String> {
    let t = href.strip_prefix("./").unwrap_or(href);
    let name = t.strip_prefix("Pictures/")?;
    if name.is_empty() || name.len() > 255 || name.contains(['/', '\\']) || name.starts_with('.') {
        return None;
    }
    Some(t.to_string())
}

/// The cell under a page offset, given the sheet's dimension runs.
fn cell_at(runs: &[DimRun], default: u32, pos: u32, limit: u32) -> (u32, u32) {
    let mut p = u64::from(pos);
    let mut i = 0u32;
    let mut steps = 0;
    loop {
        let size = u64::from(run_size(runs, i, default));
        if p < size || i >= limit - 1 || steps > 100_000 {
            break;
        }
        p -= size;
        i += 1;
        steps += 1;
    }
    (i, p as u32)
}

/// A picture read from `Pictures/`: media type and shared bytes.
type Picture = (String, Arc<[u8]>);

/// Loads the objects the frames refer to: chart objects through the
/// hardened reader, pictures from `Pictures/`. Non-chart objects are
/// skipped and counted in the warnings.
pub(crate) fn attach_objects<R: Read + Seek>(
    pkg: &mut Package<R>,
    wb: &mut WorkbookData,
    frames: Vec<PlacedFrame>,
    media: &HashMap<String, String>,
    limits: &Limits,
) -> Result<(), OdsError> {
    let xl = XmlLimits {
        max_depth: limits.max_xml_depth,
        max_attrs: limits.max_attrs_per_element,
    };
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    let mut charts = 0usize;
    let mut skipped = 0usize;
    let mut capped = false;
    // Picture bytes referenced so far (every frame counts its picture).
    let mut image_bytes: u64 = 0;
    // Each picture entry is read once; frames share its bytes. `None`:
    // unreadable or not PNG/JPEG.
    let mut pictures: HashMap<String, Option<Picture>> = HashMap::new();
    // Each chart object is parsed once per sheet (references to the own
    // sheet depend on it); further frames clone the result.
    let mut objects: HashMap<(String, u32), Option<ChartData>> = HashMap::new();
    let mut literal_budget = tally_model::MAX_LITERAL_POINTS;
    let mut literal_dropped = false;
    let mut series_budget = tally_model::MAX_SERIES_PER_DOCUMENT;
    let mut series_cut = false;
    let mut warnings = Vec::new();
    for pf in frames {
        let Some(sheet) = wb.sheets.get_mut(pf.tab as usize) else {
            continue;
        };
        let (row, col, dx, dy) = if pf.frame.page_anchored {
            let (c, dx) = cell_at(
                &sheet.col_widths,
                DEFAULT_COL_WIDTH_HMM,
                pf.frame.x_hmm,
                tally_types::MAX_COLS,
            );
            let (r, dy) = cell_at(
                &sheet.row_heights,
                DEFAULT_ROW_HEIGHT_HMM,
                pf.frame.y_hmm,
                tally_types::MAX_ROWS,
            );
            (r, c, dx, dy)
        } else {
            (pf.row, pf.col, pf.frame.x_hmm, pf.frame.y_hmm)
        };
        let anchor = ChartAnchorData {
            row,
            col,
            dx_hmm: dx,
            dy_hmm: dy,
            width_hmm: pf.frame.width_hmm.max(100),
            height_hmm: pf.frame.height_hmm.max(100),
        };
        match &pf.frame.kind {
            FrameKind::Object(href) => {
                let Some(dir) = object_dir(href) else {
                    skipped += 1;
                    continue;
                };
                let is_chart = media
                    .get(&format!("{dir}/"))
                    .is_some_and(|m| m == CHART_MEDIA);
                if !is_chart {
                    skipped += 1;
                    continue;
                }
                if charts >= MAX_CHARTS {
                    capped = true;
                    continue;
                }
                let key = (dir.clone(), pf.tab);
                let parsed = if let Some(p) = objects.get(&key) {
                    // A shared object: the copy's literal points count again.
                    p.clone().map(|mut c| {
                        let n = c.literal_points();
                        if n > literal_budget {
                            c.drop_literals();
                            literal_dropped = true;
                        } else {
                            literal_budget -= n;
                        }
                        c
                    })
                } else {
                    let look = crate::chart::SheetLookup {
                        names: &names,
                        own: pf.tab,
                    };
                    let p = pkg
                        .with_entry(&format!("{dir}/content.xml"), MAX_OBJECT_XML, |r| {
                            crate::chart::parse_content_xml(
                                &mut XmlReader::new(r, xl),
                                &look,
                                &mut warnings,
                                &mut literal_budget,
                            )
                        })?
                        .flatten();
                    objects.insert(key, p.clone());
                    p
                };
                match parsed {
                    Some(mut c) => {
                        charts += 1;
                        if c.series.len() > series_budget {
                            c.series.truncate(series_budget);
                            series_cut = true;
                        }
                        series_budget -= c.series.len();
                        c.name = if pf.frame.name.is_empty() {
                            dir.clone()
                        } else {
                            pf.frame.name.clone()
                        };
                        c.anchor = anchor;
                        sheet.charts.push(c);
                    }
                    None => skipped += 1,
                }
            }
            FrameKind::Image(href) => {
                let Some(entry) = picture_entry(href) else {
                    skipped += 1;
                    continue;
                };
                if sheet.images.len() >= MAX_IMAGES_PER_SHEET {
                    capped = true;
                    continue;
                }
                if image_bytes >= MAX_TOTAL_IMAGE_BYTES {
                    capped = true;
                    continue;
                }
                let picture = match pictures.get(&entry) {
                    Some(p) => p.clone(),
                    None => {
                        let p = pkg.read_small(&entry, MAX_IMAGE_BYTES)?.and_then(|bytes| {
                            let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                                "image/png"
                            } else if bytes.starts_with(b"\xff\xd8\xff") {
                                "image/jpeg"
                            } else {
                                return None;
                            };
                            Some((mime.to_string(), Arc::<[u8]>::from(bytes)))
                        });
                        pictures.insert(entry.clone(), p.clone());
                        p
                    }
                };
                let Some((mime, bytes)) = picture else {
                    skipped += 1;
                    continue;
                };
                // Every reference counts against the document total.
                if image_bytes + bytes.len() as u64 > MAX_TOTAL_IMAGE_BYTES {
                    capped = true;
                    continue;
                }
                image_bytes += bytes.len() as u64;
                sheet.images.push(ImageData {
                    name: pf.frame.name.clone(),
                    anchor,
                    mime,
                    bytes,
                });
            }
            FrameKind::Other => skipped += 1,
        }
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} embedded object(s) that are not charts or pictures were skipped."
        ));
    }
    if capped {
        warnings.push(format!(
            "Only the first {MAX_CHARTS} charts and {MAX_IMAGES_PER_SHEET} pictures per sheet (256 MiB in all) were read."
        ));
    }
    if series_cut {
        warnings.push(format!(
            "Charts past {} series per document were read without their extra series.",
            tally_model::MAX_SERIES_PER_DOCUMENT
        ));
    }
    if literal_dropped {
        warnings.push(format!(
            "Chart cached values past {} points per document were not read.",
            tally_model::MAX_LITERAL_POINTS
        ));
    }
    let mut seen = HashSet::new();
    warnings.retain(|w| seen.insert(w.clone()));
    wb.warnings.extend(warnings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_hrefs() {
        assert_eq!(object_dir("./Object 1").as_deref(), Some("Object 1"));
        assert_eq!(object_dir("Object 12/").as_deref(), Some("Object 12"));
        assert!(object_dir("../Object 1").is_none());
        assert!(object_dir("./Object 123456").is_none());
        assert!(object_dir("./Object x").is_none());
        assert!(picture_entry("Pictures/../content.xml").is_none());
        assert!(picture_entry("../Pictures/a.png").is_none());
        assert_eq!(
            picture_entry("./Pictures/a.png").as_deref(),
            Some("Pictures/a.png")
        );
    }

    #[test]
    fn end_cell_walks_columns_and_rows() {
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
        let a = ChartAnchorData {
            row: 1,
            col: 2,
            dx_hmm: 100,
            dy_hmm: 50,
            width_hmm: 5000,
            height_hmm: 1200,
        };
        let (r, c, x, y) = end_cell(&sheet, &a);
        assert_eq!((c, x), (4, 1100));
        assert_eq!((r, y), (3, 250));
    }
}
