//! Print service (wave 5, W5-3): `print_layout`, `page_breaks`,
//! `prepare_print_job`, `print_job_page_png` and `print_job_pdf`.
//!
//! A job is built once from the paginated sheets ([`crate::print`]) into
//! `tally_io_pdf` items: cell fills (style and conditional-format fills),
//! borders, optional gridlines and row/column headers, the display text
//! with the cell's alignment, wrap and overflow into free neighbouring
//! cells, headers and footers, charts from the SVGs the UI supplied, and
//! notes on a page at the end when the page setup asks for it. The engine
//! keeps the last [`crate::print::JOB_LRU`] jobs; the PDF is rendered on
//! first use and cached with the job, so preview, print and export share
//! one rendering. Without the `pdf` feature every method reports
//! `unsupported`.

use tally_api::{
    ApiError, ChartSvgDto, DocId, PageBreaksDto, PrintJobInfo, PrintLayoutDto, PrintOptionsDto,
    Result,
};
use tally_types::SheetId;

use super::EngineService;
use crate::print::HfContext;

/// The LRU of prepared jobs.
#[derive(Default)]
pub struct JobCache {
    #[cfg(feature = "pdf")]
    jobs: Vec<(u32, std::sync::Arc<pdf::CachedJob>)>,
    next: u32,
}

impl std::fmt::Debug for JobCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobCache")
            .field("next", &self.next)
            .finish()
    }
}

fn hf_context(doc: &crate::document::Document) -> HfContext {
    HfContext::now(
        &doc.wb,
        &doc.title,
        doc.path
            .as_ref()
            .map(|p| p.display().to_string())
            .as_deref(),
    )
}

/// `print_layout`.
pub fn print_layout(
    svc: &EngineService,
    doc: DocId,
    opts: &PrintOptionsDto,
) -> Result<PrintLayoutDto> {
    let d = svc.document(doc)?;
    crate::print::print_layout(&d.wb, opts, &hf_context(d))
}

/// `page_breaks`.
pub fn page_breaks(svc: &EngineService, doc: DocId, sheet: SheetId) -> Result<PageBreaksDto> {
    crate::print::page_breaks(&svc.document(doc)?.wb, sheet)
}

/// `prepare_print_job`.
pub fn prepare_print_job(
    svc: &mut EngineService,
    doc: DocId,
    opts: &PrintOptionsDto,
    charts: &[ChartSvgDto],
) -> Result<PrintJobInfo> {
    #[cfg(feature = "pdf")]
    {
        let d = svc.document(doc)?;
        let ctx = hf_context(d);
        let (pages, warnings) = crate::print::layout(&d.wb, opts, &ctx)?;
        let (job, layout) = pdf::build_job(d, &pages, warnings, charts)?;
        let mut jobs = super::leaf(&svc.shared.print_jobs);
        let id = jobs.next.wrapping_add(1).max(1);
        jobs.next = id;
        if jobs.jobs.len() >= crate::print::JOB_LRU {
            jobs.jobs.remove(0);
        }
        let info = PrintJobInfo {
            job: id,
            layout: layout.clone(),
        };
        jobs.jobs.push((
            id,
            std::sync::Arc::new(pdf::CachedJob {
                layout,
                job,
                pdf: std::sync::Mutex::new(None),
            }),
        ));
        Ok(info)
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (svc, doc, opts, charts);
        Err(ApiError::unsupported(
            "printing needs the `pdf` feature of tally-engine",
        ))
    }
}

/// `print_job_pdf`.
pub fn print_job_pdf(svc: &EngineService, job: u32) -> Result<Vec<u8>> {
    #[cfg(feature = "pdf")]
    {
        let j = svc.job(job)?;
        Ok((*j.pdf_bytes()?).clone())
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (svc, job);
        Err(ApiError::unsupported(
            "PDF export needs the `pdf` feature of tally-engine",
        ))
    }
}

/// `print_job_page_png`.
pub fn print_job_page_png(svc: &EngineService, job: u32, page: u32, dpi: u32) -> Result<String> {
    #[cfg(feature = "pdf")]
    {
        let j = svc.job(job)?;
        if page as usize >= j.layout.pages.len() {
            return Err(ApiError::not_found(format!("no page {page} in job {job}")));
        }
        let bytes = j.pdf_bytes()?;
        let png = tally_io_pdf::rasterize(&bytes, page as usize, dpi.clamp(18, 600))
            .map_err(|e| ApiError::new(ApiError::INTERNAL, e.to_string()))?;
        Ok(pdf::base64(&png))
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (svc, job, page, dpi);
        Err(ApiError::unsupported(
            "print preview needs the `pdf` feature of tally-engine",
        ))
    }
}

#[cfg(feature = "pdf")]
impl EngineService {
    fn job(&self, id: u32) -> Result<std::sync::Arc<pdf::CachedJob>> {
        super::leaf(&self.shared.print_jobs)
            .jobs
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, j)| j.clone())
            .ok_or_else(|| ApiError::not_found(format!("no print job {id} (prepare it again)")))
    }
}

#[cfg(feature = "pdf")]
mod pdf {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tally_api::{
        cell_flags, ApiError, CellView, ChartSvgDto, PrintLayoutDto, RangeDto, Result, ValueKind,
    };
    use tally_io_pdf::{FontSpec, HAlign, Item, LineStyle, PageJob, PrintJob, Rect, VAlign};
    use tally_model::{
        BorderLine, BorderStyle, HAlign as HA, NotesPrintData, StyleData, VAlign as VA,
    };
    use tally_types::{CellAddr, SheetId, Value};

    use crate::document::Document;
    use crate::print::{hmm_to_pt, HfBlock, PageSpec, HF_LINE_HMM};
    use crate::storage::StyleId;
    use crate::workbook::Workbook;

    /// Largest chart SVG accepted (bytes).
    pub const MAX_CHART_SVG: usize = 4 * 1024 * 1024;
    /// Most chart SVG bytes one job accepts, all charts together
    /// (security review 4, M6: 1,000 × 4 MB were accepted).
    pub const MAX_JOB_SVG_BYTES: usize = 32 * 1024 * 1024;
    /// Most cell positions one job visits, all pages together (security
    /// review 4, H5: 1 hmm rows put 24,000 × 650 cells on one page and a
    /// 2.7 MB file took about 9 min).
    pub const MAX_JOB_CELLS: u64 = 20_000_000;
    /// Most cell positions on one page.
    pub const MAX_PAGE_CELLS: u64 = 1_000_000;
    /// Header/footer and note font size.
    const HF_SIZE: f32 = crate::print::HF_FONT_PT;
    /// Row/column header sizes in points (when printed).
    const COL_HEADER_H: f32 = crate::print::COL_HEADER_H_PT;
    const ROW_HEADER_W: f32 = crate::print::ROW_HEADER_W_PT;
    const GRID_COLOR: u32 = 0xc0c0c0ff;
    const HEADER_FILL: u32 = 0xeeeeeeff;

    /// A prepared job.
    pub struct CachedJob {
        pub layout: PrintLayoutDto,
        pub job: PrintJob,
        pub pdf: Mutex<Option<Arc<Vec<u8>>>>,
    }

    impl CachedJob {
        /// The PDF, rendered on first use.
        pub fn pdf_bytes(&self) -> Result<Arc<Vec<u8>>> {
            let mut g = match self.pdf.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if let Some(b) = g.as_ref() {
                return Ok(b.clone());
            }
            let r = tally_io_pdf::render_pdf(&self.job, &tally_io_pdf::PdfOptions::default())
                .map_err(|e| ApiError::new(ApiError::INTERNAL, e.to_string()))?;
            let bytes = Arc::new(r.pdf);
            *g = Some(bytes.clone());
            Ok(bytes)
        }
    }

    pub fn base64(data: &[u8]) -> String {
        crate::protection::base64(data)
    }

    fn style_of(wb: &Workbook, id: u32) -> StyleData {
        if wb.styles.contains(id) {
            wb.styles.get(StyleId(id)).data.clone()
        } else if let Some(e) = wb.transient.style(id) {
            e.data.clone()
        } else {
            StyleData::default()
        }
    }

    fn line_style(s: BorderStyle) -> LineStyle {
        match s {
            BorderStyle::Solid => LineStyle::Solid,
            BorderStyle::Dashed => LineStyle::Dashed,
            BorderStyle::Dotted => LineStyle::Dotted,
            BorderStyle::Double => LineStyle::Double,
        }
    }

    /// The print font of a cell: the style's family, else the document's
    /// default font (`None`: Inter).
    fn font_of(wb: &Workbook, st: &StyleData, color: Option<u32>) -> FontSpec {
        FontSpec {
            family: crate::fonts::family(wb, st).map(str::to_string),
            bold: st.font.bold,
            italic: st.font.italic,
            size_pt: st.font.size_pt.unwrap_or(10.0).clamp(1.0, 400.0),
            color: color.or(st.font.color).unwrap_or(0x000000ff),
            underline: st.font.underline,
            strike: st.font.strike,
        }
    }

    /// Rounds half away from zero to `decimals` and drops trailing zeros
    /// (`1.50` → `1.5`).
    fn fixed(num: f64, decimals: usize) -> String {
        let decimals = decimals.min(20);
        let f = 10f64.powi(decimals as i32);
        let rounded = if (num * f).abs() < 1e15 {
            (num * f).round() / f
        } else {
            num
        };
        let mut s = format!("{rounded:.decimals$}");
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        if s == "-0" {
            s = "0".into();
        }
        s
    }

    /// Scientific with up to `digits` mantissa decimals: `1.23E+10`.
    fn sci(num: f64, digits: usize) -> String {
        let t = format!("{num:.prec$e}", prec = digits.min(20));
        let (m, e) = t.split_once('e').unwrap_or((t.as_str(), "0"));
        let m = if m.contains('.') {
            m.trim_end_matches('0').trim_end_matches('.')
        } else {
            m
        };
        let e: i32 = e.parse().unwrap_or(0);
        let sign = if e < 0 { '-' } else { '+' };
        format!("{m}E{sign}{:02}", e.unsigned_abs())
    }

    /// `true` if `text` looks like General output (`sep`: the locale's
    /// decimal separator).
    fn is_general_number_text(text: &str, sep: char) -> bool {
        let t = text.strip_prefix('-').unwrap_or(text);
        let (mant, exp) = match t.find(['E', 'e']) {
            Some(i) => (&t[..i], Some(&t[i + 1..])),
            None => (t, None),
        };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let mant_ok = match mant.split_once(sep) {
            Some((a, b)) => digits(a) && digits(b),
            None => digits(mant),
        };
        let exp_ok = exp.is_none_or(|e| e.strip_prefix(['+', '-']).is_some_and(digits));
        mant_ok && exp_ok
    }

    /// General "shrink to fit" for a number that does not fit `avail`
    /// points: fewer decimals while a significant digit survives, then
    /// scientific notation with fewer mantissa digits, else `###` (the
    /// grid's `fitNumber`). `sep` is the locale's decimal separator.
    fn fit_general(
        num: f64,
        text: &str,
        avail: f32,
        sep: char,
        mut measure: impl FnMut(&str) -> f32,
    ) -> String {
        if !num.is_finite() {
            return "###".into();
        }
        let local = |s: String| {
            if sep == '.' {
                s
            } else {
                s.replace('.', sep.encode_utf8(&mut [0u8; 4]))
            }
        };
        let is_sci = text.contains(['E', 'e']);
        if !is_sci && num != 0.0 {
            let decimals = text
                .rfind(sep)
                .map_or(0, |d| text[d + sep.len_utf8()..].chars().count());
            for d in (0..decimals).rev() {
                let s = fixed(num, d);
                if s.parse::<f64>().is_ok_and(|x| x == 0.0) {
                    break;
                }
                let s = local(s);
                if measure(&s) <= avail {
                    return s;
                }
            }
        }
        if num != 0.0 {
            let start = if is_sci {
                text.split(['E', 'e'])
                    .next()
                    .and_then(|m| m.split_once(sep))
                    .map_or(0, |(_, f)| f.len().saturating_sub(1))
            } else {
                9
            };
            for m in (0..=start).rev() {
                let s = local(sci(num, m));
                if measure(&s) <= avail {
                    return s;
                }
            }
        }
        "###".into()
    }

    /// One axis of a page in points: `(index, x/y, size)`.
    fn positions(start: f32, items: &[(u32, u32)]) -> Vec<(u32, f32, f32)> {
        let mut out = Vec::with_capacity(items.len());
        let mut pos = start;
        for (i, size) in items {
            let s = hmm_to_pt(*size);
            out.push((*i, pos, s));
            pos += s;
        }
        out
    }

    fn col_letters(c: u32) -> String {
        tally_types::col_to_letters(c)
    }

    /// Builds the job and the layout DTO.
    pub fn build_job(
        doc: &Document,
        pages: &[PageSpec],
        mut warnings: Vec<String>,
        charts: &[ChartSvgDto],
    ) -> Result<(PrintJob, PrintLayoutDto)> {
        let wb = &doc.wb;
        // The cost of a job is the cell positions it visits: refuse
        // oversized print areas before building anything.
        let mut cells: u64 = 0;
        for page in pages {
            let n = (page.rows.len() + page.repeat_rows.len()) as u64
                * (page.cols.len() + page.repeat_cols.len()) as u64;
            cells = cells.saturating_add(n);
            if n > MAX_PAGE_CELLS || cells > MAX_JOB_CELLS {
                return Err(ApiError::unsupported(
                    "the print area is too large to print (more than 20 million cells, or \
                     1 million on one page); set a print range or a larger scale",
                ));
            }
        }
        let mut svgs: HashMap<(SheetId, u32), Arc<str>> = HashMap::new();
        // Fonts for measuring, created on first use and dropped with the job.
        let mut fonts: Option<Option<tally_io_pdf::FontContext>> = None;
        let mut svg_bytes = 0usize;
        for c in charts {
            if c.svg.len() > MAX_CHART_SVG {
                warnings.push(format!(
                    "Chart {} on sheet {} was skipped: its image is larger than 4 MB.",
                    c.id, c.sheet
                ));
                continue;
            }
            if svg_bytes + c.svg.len() > MAX_JOB_SVG_BYTES {
                warnings.push(format!(
                    "Chart {} on sheet {} was skipped: the charts of one print job may \
                     total at most 32 MB.",
                    c.id, c.sheet
                ));
                continue;
            }
            svg_bytes += c.svg.len();
            svgs.insert((c.sheet, c.id), Arc::from(c.svg.as_str()));
        }
        let mut job = PrintJob {
            pages: Vec::with_capacity(pages.len()),
            title: Some(doc.title.clone()),
        };
        let mut missing_charts = 0usize;
        for page in pages {
            let (items, missing) = build_page(wb, doc.version, page, &svgs, &mut fonts);
            missing_charts += missing;
            job.pages.push(PageJob {
                width_pt: hmm_to_pt(page.paper.0),
                height_pt: hmm_to_pt(page.paper.1),
                items,
            });
        }
        if missing_charts > 0 {
            warnings.push(format!(
                "{missing_charts} chart(s) had no image supplied and were not printed."
            ));
        }
        // Notes at the end.
        let mut note_lines: Vec<String> = Vec::new();
        let mut seen: Vec<SheetId> = Vec::new();
        for page in pages {
            if seen.contains(&page.sheet) {
                continue;
            }
            seen.push(page.sheet);
            let Some(s) = wb.sheet(page.sheet) else {
                continue;
            };
            if s.meta.page_setup.print_notes != NotesPrintData::AtEnd {
                continue;
            }
            for ((r, c), note) in s.meta.notes.iter() {
                if page.area.contains(CellAddr::new(page.sheet, *r, *c)) {
                    let text: String = note.text.chars().take(2000).collect();
                    note_lines.push(format!(
                        "{}.{}{}: {}",
                        s.name,
                        col_letters(*c),
                        r + 1,
                        text.replace('\n', " ")
                    ));
                }
            }
        }
        if !note_lines.is_empty() {
            if let Some(first) = pages.first() {
                let room = tally_io_pdf::MAX_PAGES.saturating_sub(job.pages.len());
                let mut notes = note_pages(first, &note_lines, wb.default_font.as_deref());
                if notes.len() > room {
                    notes.truncate(room);
                    warnings
                        .push("Some notes were not printed: the job has too many pages.".into());
                }
                job.pages.extend(notes);
            }
        }
        let layout = PrintLayoutDto {
            pages: pages
                .iter()
                .enumerate()
                .map(|(i, p)| p.to_dto(i as u32))
                .collect(),
            warnings,
        };
        Ok((job, layout))
    }

    fn note_pages(like: &PageSpec, lines: &[String], family: Option<&str>) -> Vec<PageJob> {
        let w = hmm_to_pt(like.paper.0);
        let h = hmm_to_pt(like.paper.1);
        let x = hmm_to_pt(like.origin.0);
        let top = hmm_to_pt(like.origin.1);
        let line_h = HF_SIZE * 1.4;
        let per_page = (((h - 2.0 * top) / line_h).floor() as usize).max(1);
        lines
            .chunks(per_page)
            .map(|chunk| {
                let mut items = Vec::with_capacity(chunk.len());
                for (i, l) in chunk.iter().enumerate() {
                    items.push(Item::Text {
                        rect: Rect::new(x, top + line_h * i as f32, w - 2.0 * x, line_h),
                        text: l.clone(),
                        font: FontSpec {
                            family: family.map(str::to_string),
                            size_pt: HF_SIZE,
                            ..FontSpec::default()
                        },
                        h_align: HAlign::Left,
                        v_align: VAlign::Middle,
                        wrap: false,
                        overflow: (0.0, 0.0),
                        clip: true,
                    });
                }
                PageJob {
                    width_pt: w,
                    height_pt: h,
                    items,
                }
            })
            .collect()
    }

    /// The header or footer lines of a page: each line of each region in
    /// its own box of [`HF_LINE_HMM`] from the top of the area down, so a
    /// multi-line text is laid out like AOO's (never centred around the
    /// area and clipped by it; AutoFit height makes the area fit its lines,
    /// see [`crate::print::body_of`]). Lines past a fixed height are left
    /// off so they never run over the body or off the page. The text is in
    /// the document's default font (`family`; `None`: Inter), as in AOO.
    pub(super) fn hf_items(
        items: &mut Vec<Item>,
        texts: &Option<[String; 3]>,
        block: Option<HfBlock>,
        family: Option<&str>,
    ) {
        let (Some(t), Some(b)) = (texts, block) else {
            return;
        };
        let line_h = hmm_to_pt(HF_LINE_HMM);
        let x = hmm_to_pt(b.left);
        let w = hmm_to_pt(b.width).max(1.0);
        let top = hmm_to_pt(b.top);
        let fit = crate::print::hf_fitting_lines(&b);
        for (i, align) in [HAlign::Left, HAlign::Center, HAlign::Right]
            .into_iter()
            .enumerate()
        {
            for (n, line) in t[i].split('\n').take(fit).enumerate() {
                if line.is_empty() {
                    continue;
                }
                items.push(Item::Text {
                    rect: Rect::new(x, top + line_h * n as f32, w, line_h),
                    text: line.to_string(),
                    font: FontSpec {
                        family: family.map(str::to_string),
                        size_pt: HF_SIZE,
                        ..FontSpec::default()
                    },
                    h_align: align,
                    v_align: VAlign::Middle,
                    wrap: false,
                    overflow: (0.0, 0.0),
                    clip: false,
                });
            }
        }
    }

    /// Builds one page's items; returns them with the number of charts
    /// that had no SVG.
    fn build_page(
        wb: &Workbook,
        version: u64,
        page: &PageSpec,
        svgs: &HashMap<(SheetId, u32), Arc<str>>,
        fonts: &mut Option<Option<tally_io_pdf::FontContext>>,
    ) -> (Vec<Item>, usize) {
        let mut items: Vec<Item> = Vec::new();
        let Some(sheet) = wb.sheet(page.sheet) else {
            return (items, 0);
        };
        let setup = sheet.meta.page_setup.clone();
        let family = wb.default_font.as_deref();
        hf_items(&mut items, &page.header, page.header_block, family);
        hf_items(&mut items, &page.footer, page.footer_block, family);
        let headers = setup.print_headers;
        let mut x0 = hmm_to_pt(page.origin.0);
        let mut y0 = hmm_to_pt(page.origin.1);
        if headers {
            x0 += ROW_HEADER_W;
            y0 += COL_HEADER_H;
        }
        let scale = page.scale as f32 / 100.0;
        let mut row_items: Vec<(u32, u32)> = page.repeat_rows.to_vec();
        row_items.extend(page.rows.iter().copied());
        let mut col_items: Vec<(u32, u32)> = page.repeat_cols.to_vec();
        col_items.extend(page.cols.iter().copied());
        let rows = positions(y0, &row_items);
        let cols = positions(x0, &col_items);
        let content_w: f32 = cols.iter().map(|c| c.2).sum();
        let content_h: f32 = rows.iter().map(|r| r.2).sum();
        // Cell views of the printed rectangles.
        let mut ranges: Vec<RangeDto> = Vec::new();
        let span = |v: &[(u32, u32)]| (v.first().map(|x| x.0), v.last().map(|x| x.0));
        let (r_first, r_last) = span(&page.rows);
        let (c_first, c_last) = span(&page.cols);
        let (rr_first, rr_last) = span(&page.repeat_rows);
        let (rc_first, rc_last) = span(&page.repeat_cols);
        let row_spans = [(r_first, r_last), (rr_first, rr_last)];
        let col_spans = [(c_first, c_last), (rc_first, rc_last)];
        for (ra, rb) in row_spans {
            for (ca, cb) in col_spans {
                if let (Some(r0), Some(r1), Some(c0), Some(c1)) = (ra, rb, ca, cb) {
                    ranges.push(RangeDto { r0, c0, r1, c1 });
                }
            }
        }
        let mut views: HashMap<(u32, u32), CellView> = HashMap::new();
        if let Ok(blocks) = crate::service::view::cells(wb, version, page.sheet, &ranges) {
            for b in blocks {
                for v in b.cells {
                    views.insert((v.r, v.c), v);
                }
            }
        }
        // Charts are placed from the body rows and columns only, so a
        // chart anchored in a repeated row is printed once, not on every
        // page (security review 4, M6).
        let col_index: HashMap<u32, usize> = cols
            .iter()
            .enumerate()
            .skip(page.repeat_cols.len())
            .map(|(i, c)| (c.0, i))
            .collect();
        let row_index: HashMap<u32, usize> = rows
            .iter()
            .enumerate()
            .skip(page.repeat_rows.len())
            .map(|(i, r)| (r.0, i))
            .collect();
        let has_content = |r: u32, c: u32| {
            views.get(&(r, c)).is_some_and(|v| !v.text.is_empty()) || sheet.is_merge_covered(r, c)
        };
        // Headers.
        if headers {
            items.push(Item::Fill {
                rect: Rect::new(
                    x0 - ROW_HEADER_W,
                    y0 - COL_HEADER_H,
                    ROW_HEADER_W + content_w,
                    COL_HEADER_H,
                ),
                color: HEADER_FILL,
            });
            items.push(Item::Fill {
                rect: Rect::new(x0 - ROW_HEADER_W, y0, ROW_HEADER_W, content_h),
                color: HEADER_FILL,
            });
            let hf = FontSpec {
                family: wb.default_font.clone(),
                size_pt: 8.0,
                color: 0x444444ff,
                ..FontSpec::default()
            };
            for (c, x, w) in &cols {
                items.push(Item::Text {
                    rect: Rect::new(*x, y0 - COL_HEADER_H, *w, COL_HEADER_H),
                    text: col_letters(*c),
                    font: hf.clone(),
                    h_align: HAlign::Center,
                    v_align: VAlign::Middle,
                    wrap: false,
                    overflow: (0.0, 0.0),
                    clip: true,
                });
            }
            for (r, y, h) in &rows {
                items.push(Item::Text {
                    rect: Rect::new(x0 - ROW_HEADER_W, *y, ROW_HEADER_W, *h),
                    text: (r + 1).to_string(),
                    font: hf.clone(),
                    h_align: HAlign::Center,
                    v_align: VAlign::Middle,
                    wrap: false,
                    overflow: (0.0, 0.0),
                    clip: true,
                });
            }
        }
        // Fills first, then gridlines, then borders and text.
        let mut texts: Vec<Item> = Vec::new();
        let mut borders: Vec<Item> = Vec::new();
        for (ri, (r, y, h)) in rows.iter().enumerate() {
            for (ci, (c, x, w)) in cols.iter().enumerate() {
                if sheet.is_merge_covered(*r, *c) {
                    continue;
                }
                let addr = CellAddr::new(page.sheet, *r, *c);
                // The cell box (merges span the visible merged cells).
                let (mut bw, mut bh) = (*w, *h);
                if let Some(m) = sheet.merge_at(*r, *c) {
                    bw = cols[ci..]
                        .iter()
                        .take_while(|(cc, _, _)| *cc <= m.c1)
                        .map(|c| c.2)
                        .sum();
                    bh = rows[ri..]
                        .iter()
                        .take_while(|(rr, _, _)| *rr <= m.r1)
                        .map(|r| r.2)
                        .sum();
                }
                let rect = Rect::new(*x, *y, bw, bh);
                let view = views.get(&(*r, *c));
                let style_id =
                    view.map_or_else(|| wb.style_at(addr).0, |v| v.cf_style.unwrap_or(v.style));
                let st = style_of(wb, style_id);
                if let Some(fill) = st.fill {
                    items.push(Item::Fill { rect, color: fill });
                }
                let edge =
                    |line: &Option<BorderLine>, from: (f32, f32), to: (f32, f32)| -> Option<Item> {
                        let l = line.as_ref()?;
                        Some(Item::Line {
                            from,
                            to,
                            width: (hmm_to_pt(l.width_hmm) * scale).max(0.5),
                            color: l.color,
                            style: line_style(l.style),
                        })
                    };
                let (x1, y1) = (rect.x + rect.w, rect.y + rect.h);
                borders.extend(edge(&st.borders.top, (rect.x, rect.y), (x1, rect.y)));
                borders.extend(edge(&st.borders.bottom, (rect.x, y1), (x1, y1)));
                borders.extend(edge(&st.borders.left, (rect.x, rect.y), (rect.x, y1)));
                borders.extend(edge(&st.borders.right, (x1, rect.y), (x1, y1)));
                let Some(v) = view else { continue };
                let mut text = v.text.clone();
                if setup.print_formulas && v.flags & cell_flags::FORMULA != 0 {
                    if let Some((id, _)) = wb.formula_at(addr) {
                        text = wb.formula_text(id).unwrap_or(text);
                    }
                }
                if !setup.print_zero_values
                    && v.kind == ValueKind::Number
                    && matches!(wb.value(addr), Value::Number(n) if n == 0.0)
                {
                    continue;
                }
                if text.is_empty() {
                    continue;
                }
                let hide = v.cf_bar.as_ref().is_some() && v.text.is_empty();
                if hide {
                    continue;
                }
                let numeric = matches!(
                    v.kind,
                    ValueKind::Number | ValueKind::Bool | ValueKind::Error
                );
                let h_align = match st.align.h {
                    HA::General => {
                        if numeric && !setup.print_formulas {
                            HAlign::Right
                        } else {
                            HAlign::Left
                        }
                    }
                    HA::Left | HA::Justify => HAlign::Left,
                    HA::Center => HAlign::Center,
                    HA::Right => HAlign::Right,
                };
                let v_align = match st.align.v {
                    VA::Top => VAlign::Top,
                    VA::Middle => VAlign::Middle,
                    VA::Bottom => VAlign::Bottom,
                };
                let mut font = font_of(wb, &st, v.color);
                font.size_pt = (font.size_pt * scale).max(1.0);
                let wrap = st.align.wrap;
                // Free space on either side (empty neighbours on this page).
                let mut free_right = 0.0f32;
                let mut free_left = 0.0f32;
                if !wrap && v.kind == ValueKind::Text {
                    let merged_last = sheet.merge_at(*r, *c).map_or(*c, |m| m.c1);
                    for (cc, _, cw) in cols.iter().skip(ci + 1) {
                        if *cc <= merged_last {
                            continue;
                        }
                        if has_content(*r, *cc) || sheet.merge_at(*r, *cc).is_some() {
                            break;
                        }
                        free_right += cw;
                    }
                    for (cc, _, cw) in cols[..ci].iter().rev() {
                        if has_content(*r, *cc) || sheet.merge_at(*r, *cc).is_some() {
                            break;
                        }
                        free_left += cw;
                    }
                }
                if numeric && !wrap {
                    // Numbers that do not fit: General drops decimals (then
                    // goes scientific), anything else prints as `###` (AOO;
                    // the grid does the same, packages/grid/src/fitNumber.ts).
                    let avail = (rect.w - 3.0).max(0.0);
                    // One font context per job; each cell's font resolved
                    // once (security review: no global lock per string).
                    let ctx = fonts.get_or_insert_with(|| {
                        tally_io_pdf::FontContext::new(&tally_io_pdf::PdfOptions::default()).ok()
                    });
                    if let Some(ctx) = ctx.as_mut() {
                        let (face, _) = ctx.face(&font);
                        let size = font.size_pt;
                        let mut measure = |s: &str| ctx.width(face, s, size);
                        if measure(&text) > avail {
                            let sep = wb.locale.decimal_sep;
                            let general =
                                st.number_format.as_deref().is_none_or(|f| f == "General")
                                    || is_general_number_text(&text, sep);
                            text = match v.num {
                                Some(n) if general && !setup.print_formulas => {
                                    fit_general(n, &text, avail, sep, measure)
                                }
                                _ => "###".into(),
                            };
                        }
                    }
                }
                let overflow = match h_align {
                    HAlign::Left => (0.0, free_right),
                    HAlign::Right => (free_left, 0.0),
                    HAlign::Center => (free_left, free_right),
                };
                texts.push(Item::Text {
                    rect,
                    text,
                    font,
                    h_align,
                    v_align,
                    wrap,
                    overflow,
                    clip: true,
                });
            }
        }
        if setup.print_gridlines {
            let x_end = x0 + content_w;
            let y_end = y0 + content_h;
            let mut xs: Vec<f32> = cols.iter().map(|c| c.1).collect();
            xs.push(x_end);
            let mut ys: Vec<f32> = rows.iter().map(|r| r.1).collect();
            ys.push(y_end);
            for x in xs {
                items.push(Item::Line {
                    from: (x, y0),
                    to: (x, y_end),
                    width: 0.4,
                    color: GRID_COLOR,
                    style: LineStyle::Solid,
                });
            }
            for y in ys {
                items.push(Item::Line {
                    from: (x0, y),
                    to: (x_end, y),
                    width: 0.4,
                    color: GRID_COLOR,
                    style: LineStyle::Solid,
                });
            }
        }
        items.extend(borders);
        items.extend(texts);
        // Charts whose anchor cell is on this page.
        let mut missing = 0usize;
        if setup.print_charts {
            for ch in sheet.meta.charts.iter() {
                let a = &ch.data.anchor;
                let (Some(ri), Some(ci)) = (row_index.get(&a.row), col_index.get(&a.col)) else {
                    continue;
                };
                let Some(svg) = svgs.get(&(page.sheet, ch.id)) else {
                    missing += 1;
                    continue;
                };
                let x = cols[*ci].1 + hmm_to_pt(a.dx_hmm) * scale;
                let y = rows[*ri].1 + hmm_to_pt(a.dy_hmm) * scale;
                let w = hmm_to_pt(a.width_hmm) * scale;
                let h = hmm_to_pt(a.height_hmm) * scale;
                items.push(Item::Svg {
                    rect: Rect::new(x, y, w.max(1.0), h.max(1.0)),
                    svg: svg.clone(),
                });
            }
        }
        (items, missing)
    }

    #[cfg(test)]
    mod fit_tests {
        use super::*;

        #[test]
        fn general_numbers_shrink_before_hashes() {
            let w = |s: &str| s.chars().count() as f32;
            assert_eq!(
                fit_general(12.671_428_6, "12.6714286", 7.0, '.', w),
                "12.6714"
            );
            assert_eq!(fit_general(1234.5, "1234.5", 4.0, '.', w), "1235");
            assert_eq!(
                fit_general(123_456_789.0, "123456789", 7.0, '.', w),
                "1.2E+08"
            );
            assert_eq!(fit_general(0.000_01, "1E-05", 2.0, '.', w), "###");
            assert_eq!(fit_general(f64::NAN, "x", 9.0, '.', w), "###");
            // Locales with a decimal comma keep it.
            assert_eq!(
                fit_general(12.671_428_6, "12,6714286", 7.0, ',', w),
                "12,6714"
            );
            assert_eq!(
                fit_general(123_456_789.5, "123456789,5", 7.0, ',', w),
                "1,2E+08"
            );
            assert!(is_general_number_text("-12,5", ','));
            assert_eq!(fixed(-0.0001, 2), "0");
            assert!(is_general_number_text("-12.5", '.') && is_general_number_text("1.5E+10", '.'));
            assert!(!is_general_number_text("$1.00", '.') && !is_general_number_text("1,000", '.'));
        }
    }
}
