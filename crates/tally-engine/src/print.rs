//! Pagination (wave 5, W5-3). All sizes are in 1/100 mm (hmm); the job
//! builder in `service/print.rs` converts to points (1 mm = 72/25.4 pt).
//!
//! Rules (AOO `ScPrintFunc`, `ScTable::UpdatePageBreaks`):
//! - **Area**: the print range when set; otherwise the used area (content
//!   plus visibly formatted cells plus chart bounds). A print selection
//!   (`PrintOptionsDto::selection`) replaces it on the active sheet.
//! - **Body**: the paper minus the margins, minus the header area when the
//!   header is on and the footer area likewise (AOO and ODF: the areas sit
//!   inside the page margins and push the body in). An area is
//!   `margins.header` high, its spacing towards the body included (AOO
//!   `ScPrintHFParam::nHeight`), grown with AutoFit height to its lines
//!   ([`HF_LINE_HMM`] each, the most of its three regions) plus spacing.
//! - **Scale**: `Percent` multiplies every row and column size. `FitTo{w,h}`
//!   takes the largest integer percent from 100 % down to 10 % with pages
//!   across ≤ w and pages down ≤ h (0 = unlimited); `FitPages(n)` the
//!   largest with total pages ≤ n. In fit modes manual breaks are ignored
//!   on the fitted axis (AOO `bSkipColBreaks`/`bSkipRowBreaks`, `table5.cxx`).
//! - **Breaks**: a page always holds at least one row and one column;
//!   hidden and filtered rows/columns are skipped; a manual break forces
//!   a page. Repeated rows (columns) take space on every page whose first
//!   row (column) lies after them.
//! - **Order**: top-to-bottom-then-right (default) or left-to-right-then-down.
//! - **Empty pages** (no cell content and no chart) are suppressed when the
//!   options say so (the AOO default).
//! - **Placement**: centring offsets the body origin; `&[PAGE]` starts at
//!   `first_page_number`; `&[PAGES]`, `&[SHEET]`, `&[FILE]`, `&[PATH]`,
//!   `&[DATE]` and `&[TIME]` are resolved with the document locale.

use std::collections::BTreeSet;

use tally_api::{
    ApiError, BreakInfo, PageBreaksDto, PrintLayoutDto, PrintOptionsDto, PrintPageDto, PrintSheets,
    RangeDto,
};
use tally_model::{
    HfAreaData, OrientationData, PageOrderData, PageScaleData, PageSetupData, PaperData,
};
use tally_types::{CellRange, SheetId, MAX_COLS, MAX_ROWS};

use crate::storage::{ColumnStore, Sheet};
use crate::workbook::Workbook;

/// Points per millimetre.
pub const PT_PER_MM: f32 = 72.0 / 25.4;

/// Most jobs kept per process.
pub const JOB_LRU: usize = 4;

/// Most pages one layout may produce (a 1,000 × 1,000-page sheet is
/// refused rather than paginated). The same cap as the PDF renderer's
/// (`tally_io_pdf::MAX_PAGES`), so a job that is laid out can be rendered
/// (security review 4, M5: 100,000 here against 10,000 there).
pub const MAX_PAGES: usize = 10_000;

/// Default row height for printing when a row has no explicit height:
/// AOO's standard row height (256 twips, 0.452 cm, the height ODS files
/// declare; sc/inc/global.hxx:165), so a new sheet paginates 53 default
/// rows per A4 page like AOO (correctness review 4, M22: the 24 px screen
/// default gave 38).
pub const PRINT_DEFAULT_ROW_HMM: u32 = 452;
/// Default column width for printing (AOO's 2.258 cm).
pub const PRINT_DEFAULT_COL_HMM: u32 = 2258;
/// Smallest body height, 1/100 mm.
pub const MIN_BODY_HMM: u32 = 1000;

/// Lines of a header/footer area that fit its text box (a fixed height
/// smaller than the text, or an area cut down to keep the body, leaves the
/// rest off rather than drawing over the body or off the page).
pub fn hf_fitting_lines(b: &HfBlock) -> usize {
    (b.height / HF_LINE_HMM).min(HF_MAX_LINES) as usize
}

/// Header/footer font size, points.
pub const HF_FONT_PT: f32 = 10.0;
/// Height of one header/footer line (the PDF draws each line in a box of
/// this height).
pub use tally_model::{HF_LINE_HMM, HF_MAX_LINES};
/// Row header width (printed row numbers), points.
pub const ROW_HEADER_W_PT: f32 = 28.0;
/// Column header height (printed column letters), points.
pub const COL_HEADER_H_PT: f32 = 14.0;

/// A run of `(index, scaled size)` items shared by all pages of a layout
/// (security review 4, H1/H5: each page held its own copy of its rows and
/// of the repeated rows, 801 MB for 10k pages of a 1.5 KB file).
#[derive(Clone, Debug, Default)]
pub struct Items {
    all: std::sync::Arc<[(u32, u32)]>,
    start: usize,
    end: usize,
}

impl Items {
    /// Items `[start, end)` of `all`.
    pub fn slice(all: &std::sync::Arc<[(u32, u32)]>, start: usize, end: usize) -> Self {
        let end = end.min(all.len());
        Self {
            all: all.clone(),
            start: start.min(end),
            end,
        }
    }
}

impl std::ops::Deref for Items {
    type Target = [(u32, u32)];
    fn deref(&self) -> &[(u32, u32)] {
        &self.all[self.start..self.end]
    }
}

impl From<Vec<(u32, u32)>> for Items {
    fn from(v: Vec<(u32, u32)>) -> Self {
        let end = v.len();
        Self {
            all: v.into(),
            start: 0,
            end,
        }
    }
}

/// Converts 1/100 mm to points.
pub fn hmm_to_pt(hmm: u32) -> f32 {
    hmm as f32 / 100.0 * PT_PER_MM
}

/// Paper `(width, height)` in hmm after orientation.
pub fn paper_hmm(p: &PageSetupData) -> (u32, u32) {
    let (w, h) = match p.paper {
        PaperData::A4 => (21_000, 29_700),
        PaperData::A3 => (29_700, 42_000),
        PaperData::A5 => (14_800, 21_000),
        PaperData::Letter => (21_590, 27_940),
        PaperData::Legal => (21_590, 35_560),
        PaperData::Executive => (18_415, 26_670),
        PaperData::Tabloid => (27_940, 43_180),
        PaperData::B5 => (17_600, 25_000),
        PaperData::Custom {
            width_hmm,
            height_hmm,
        } => (width_hmm.max(1000), height_hmm.max(1000)),
    };
    match p.orientation {
        OrientationData::Portrait => (w, h),
        OrientationData::Landscape => (h, w),
    }
}

/// A repeated span `[first, last]` with its visible `(index, size)` items.
pub type RepeatSpan = ((u32, u32), Vec<(u32, u32)>);

/// One axis of the printed area: the visible indices with their sizes.
#[derive(Clone, Debug, Default)]
pub struct Axis {
    /// `(index, size_hmm)` of every visible row/column of the area.
    pub items: Vec<(u32, u32)>,
    /// Manual breaks (before this index).
    pub manual: BTreeSet<u32>,
    /// The repeated span `[first, last]`, with its visible items.
    pub repeat: Option<RepeatSpan>,
}

/// A laid-out header or footer area, 1/100 mm from the paper's top-left.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HfBlock {
    /// Top of the area.
    pub top: u32,
    /// Height of the text box (the area without its spacing).
    pub height: u32,
    /// Left edge of the text.
    pub left: u32,
    /// Width of the text box.
    pub width: u32,
}

/// A page body and the areas around it, 1/100 mm.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// Body width (without printed row headers).
    pub w: u32,
    /// Body height (without printed column headers).
    pub h: u32,
    /// Top of the body (top margin plus the header area).
    pub top: u32,
    /// The header area, when the header is on.
    pub header: Option<HfBlock>,
    /// The footer area, when the footer is on.
    pub footer: Option<HfBlock>,
}

impl Axis {
    fn repeat_size(&self, scale: u32) -> u32 {
        self.repeat.as_ref().map_or(0, |(_, items)| {
            items.iter().map(|(_, s)| scaled(*s, scale)).sum()
        })
    }
}

/// One page band on an axis: items `[start, end)` of `Axis::items`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Band {
    /// First item index.
    pub start: usize,
    /// One past the last item index.
    pub end: usize,
    /// The band is preceded by a manual break.
    pub manual: bool,
    /// The repeated span is printed before the band.
    pub with_repeat: bool,
}

fn scaled(size: u32, scale: u32) -> u32 {
    ((u64::from(size) * u64::from(scale)) / 100).max(1) as u32
}

/// Splits an axis into bands that fit `body` (scaled sizes).
pub fn bands(axis: &Axis, body: u32, scale: u32, skip_manual: bool) -> Vec<Band> {
    let n = axis.items.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let repeat_end = axis.repeat.as_ref().map(|((_, last), _)| *last);
    let repeat_size = axis.repeat_size(scale);
    let mut start = 0usize;
    let mut manual = false;
    while start < n {
        let (first_index, first_size) = axis.items[start];
        // The repeated span is printed only when the band's first item
        // still fits below it; a span taller than the page is left off (it
        // made every page hold one row, security review 4, H1).
        let with_repeat = repeat_end.is_some_and(|last| first_index > last)
            && repeat_size.saturating_add(scaled(first_size, scale)) <= body;
        let avail = if with_repeat {
            body.saturating_sub(repeat_size)
        } else {
            body
        };
        let mut used = 0u32;
        let mut end = start;
        let mut next_manual = false;
        while end < n {
            let (index, size) = axis.items[end];
            if end > start && !skip_manual && axis.manual.contains(&index) {
                next_manual = true;
                break;
            }
            let s = scaled(size, scale);
            if end > start && used.saturating_add(s) > avail {
                break;
            }
            used = used.saturating_add(s);
            end += 1;
        }
        out.push(Band {
            start,
            end,
            manual,
            with_repeat,
        });
        manual = next_manual;
        start = end;
    }
    out
}

/// A paginated page (internal; the DTO is derived from it).
#[derive(Clone, Debug)]
pub struct PageSpec {
    /// Sheet.
    pub sheet: SheetId,
    /// Rows `(index, scaled height hmm)` on the page.
    pub rows: Items,
    /// Columns `(index, scaled width hmm)`.
    pub cols: Items,
    /// Repeated rows printed above (`(index, scaled height)`), if any.
    pub repeat_rows: Items,
    /// Repeated columns printed left of the page.
    pub repeat_cols: Items,
    /// Scale percent.
    pub scale: u32,
    /// Paper `(w, h)` hmm.
    pub paper: (u32, u32),
    /// Body origin `(x, y)` hmm (margins plus centring).
    pub origin: (u32, u32),
    /// Body `(w, h)` hmm.
    pub body: (u32, u32),
    /// The header area, when the header is on.
    pub header_block: Option<HfBlock>,
    /// The footer area, when the footer is on.
    pub footer_block: Option<HfBlock>,
    /// Printed page number.
    pub number: u32,
    /// Resolved header texts.
    pub header: Option<[String; 3]>,
    /// Resolved footer texts.
    pub footer: Option<[String; 3]>,
    /// Row span of the sheet's print area (for gridline/header drawing).
    pub area: CellRange,
}

impl PageSpec {
    /// Rows `[first, last]` (repeat rows excluded).
    pub fn row_span(&self) -> [u32; 2] {
        [
            self.rows.first().map_or(0, |r| r.0),
            self.rows.last().map_or(0, |r| r.0),
        ]
    }

    /// Columns `[first, last]`.
    pub fn col_span(&self) -> [u32; 2] {
        [
            self.cols.first().map_or(0, |c| c.0),
            self.cols.last().map_or(0, |c| c.0),
        ]
    }

    /// Content size `(w, h)` hmm including the repeated rows/columns.
    pub fn content_size(&self) -> (u32, u32) {
        let w: u32 = self
            .repeat_cols
            .iter()
            .chain(self.cols.iter())
            .map(|c| c.1)
            .sum();
        let h: u32 = self
            .repeat_rows
            .iter()
            .chain(self.rows.iter())
            .map(|r| r.1)
            .sum();
        (w, h)
    }

    /// The DTO.
    pub fn to_dto(&self, index: u32) -> PrintPageDto {
        let repeat = |v: &[(u32, u32)]| (!v.is_empty()).then(|| [v[0].0, v[v.len() - 1].0]);
        PrintPageDto {
            index,
            number: self.number,
            sheet: self.sheet,
            rows: self.row_span(),
            cols: self.col_span(),
            repeat_rows: repeat(&self.repeat_rows),
            repeat_cols: repeat(&self.repeat_cols),
            scale_pct: self.scale,
            paper_mm: [self.paper.0 as f32 / 100.0, self.paper.1 as f32 / 100.0],
            origin_mm: [self.origin.0 as f32 / 100.0, self.origin.1 as f32 / 100.0],
            header: self.header.clone(),
            footer: self.footer.clone(),
        }
    }
}

/// What the header/footer placeholders resolve to.
#[derive(Clone, Debug, Default)]
pub struct HfContext {
    /// `&[FILE]`.
    pub file: String,
    /// `&[PATH]`.
    pub path: String,
    /// `&[DATE]` (locale short date).
    pub date: String,
    /// `&[TIME]`.
    pub time: String,
}

impl HfContext {
    /// A context for the workbook's locale with the current date and time.
    pub fn now(wb: &Workbook, title: &str, path: Option<&str>) -> Self {
        use tally_numfmt::{builtin_format, format_value_with, parse_format, QuickFormat};
        let epoch = wb.settings.date_epoch;
        let serial = tally_numfmt::date::now_serial(epoch);
        let fmt = |code: &str| {
            parse_format(code)
                .map(|ast| {
                    format_value_with(
                        &tally_types::Value::Number(serial),
                        &ast,
                        wb.locale,
                        None,
                        epoch,
                    )
                    .text
                })
                .unwrap_or_default()
        };
        Self {
            file: title.to_string(),
            path: path.unwrap_or("").to_string(),
            date: fmt(&builtin_format(QuickFormat::Date, wb.locale)),
            time: fmt(&builtin_format(QuickFormat::TimeSeconds, wb.locale)),
        }
    }
}

/// Resolves `&[PAGE]`-style placeholders (`&&` is a literal `&`).
pub fn resolve_placeholders(
    text: &str,
    page: u32,
    pages: u32,
    sheet: &str,
    ctx: &HfContext,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(tail) = after.strip_prefix('&') {
            out.push('&');
            rest = tail;
            continue;
        }
        if let Some(tail) = after.strip_prefix('[') {
            if let Some(end) = tail.find(']') {
                let key = &tail[..end];
                let rep = match key.to_ascii_uppercase().as_str() {
                    "PAGE" => Some(page.to_string()),
                    "PAGES" => Some(pages.to_string()),
                    "SHEET" | "TAB" => Some(sheet.to_string()),
                    "FILE" => Some(ctx.file.clone()),
                    "PATH" => Some(ctx.path.clone()),
                    "DATE" => Some(ctx.date.clone()),
                    "TIME" => Some(ctx.time.clone()),
                    _ => None,
                };
                if let Some(r) = rep {
                    out.push_str(&r);
                    rest = &tail[end + 1..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn default_row_hmm() -> u32 {
    PRINT_DEFAULT_ROW_HMM
}

fn default_col_hmm() -> u32 {
    PRINT_DEFAULT_COL_HMM
}

/// Height of a row in hmm (0 when hidden or filtered).
pub fn row_height(sheet: &Sheet, r: u32) -> u32 {
    if sheet.filtered_rows.contains(r) {
        return 0;
    }
    match sheet.row_dims.get(r) {
        Some(d) if d.hidden => 0,
        Some(d) => d.size_hmm,
        None => default_row_hmm(),
    }
}

/// Width of a column in hmm (0 when hidden).
pub fn col_width(sheet: &Sheet, c: u32) -> u32 {
    match sheet.col_dims.get(c) {
        Some(d) if d.hidden => 0,
        Some(d) => d.size_hmm,
        None => default_col_hmm(),
    }
}

/// The cell range a chart anchored at `anchor` covers.
fn chart_cells(sheet: &Sheet, anchor: &tally_model::ChartAnchorData) -> CellRange {
    let mut c = anchor.col.min(MAX_COLS - 1);
    let mut x = i64::from(anchor.dx_hmm) + i64::from(anchor.width_hmm);
    while x > 0 && c < MAX_COLS - 1 {
        x -= i64::from(col_width(sheet, c));
        if x > 0 {
            c += 1;
        }
    }
    let mut r = anchor.row.min(MAX_ROWS - 1);
    let mut y = i64::from(anchor.dy_hmm) + i64::from(anchor.height_hmm);
    while y > 0 && r < MAX_ROWS - 1 {
        y -= i64::from(row_height(sheet, r));
        if y > 0 {
            r += 1;
        }
    }
    CellRange::new(sheet.id, anchor.row, anchor.col, r, c)
}

/// Most stored cells [`print_area`] inspects for text overflow.
const MAX_OVERFLOW_SCAN: usize = 1_000_000;
/// Columns left of the area's last column whose text may overflow past
/// it.
const OVERFLOW_SCAN_COLS: u32 = 64;

/// `area` widened to the columns that left-aligned text near its right
/// edge overflows into (AOO `ExtendPrintArea`, sc/source/ui/view/
/// printfun.cxx:730-737; correctness review 4, M24). The text width is
/// estimated from the character count and the font size.
fn extend_for_overflow(wb: &Workbook, s: &Sheet, area: CellRange) -> CellRange {
    use tally_model::HAlign as HA;
    let mut c1 = area.c1;
    let mut scanned = 0usize;
    let lo = area.c1.saturating_sub(OVERFLOW_SCAN_COLS - 1).max(area.c0);
    for c in lo..=area.c1 {
        let Some(col) = s.column(c) else {
            continue;
        };
        for (r, _) in col.cells.iter_range(area.r0, area.r1) {
            scanned += 1;
            if scanned > MAX_OVERFLOW_SCAN {
                return CellRange::new(area.sheet, area.r0, area.c0, area.r1, c1);
            }
            let addr = tally_types::CellAddr::new(s.id, r, c);
            let tally_types::Value::Text(t) = wb.value(addr) else {
                continue;
            };
            let st = &wb.styles.get(wb.style_at(addr)).data;
            if st.align.wrap
                || !matches!(st.align.h, HA::General | HA::Left)
                || s.merge_at(r, c).is_some()
            {
                continue;
            }
            let size = f64::from(st.font.size_pt.unwrap_or(10.0).clamp(1.0, 400.0));
            let chars = t.chars().take(4096).count() as f64;
            // pt → hmm: 2540 / 72.
            let need = (chars * size * 0.55 * 2540.0 / 72.0) as u64;
            let mut have = u64::from(col_width(s, c));
            let mut cc = c;
            while have < need && cc < MAX_COLS - 1 && cc - c < 256 {
                if s.get(r, cc + 1).is_some() {
                    break;
                }
                cc += 1;
                have += u64::from(col_width(s, cc));
            }
            c1 = c1.max(cc);
        }
    }
    CellRange::new(area.sheet, area.r0, area.c0, area.r1, c1)
}

/// The area a sheet prints: the print range, or the used area plus
/// visibly formatted cells, chart bounds and overflowing text, from A1
/// (AOO starts the implicit area at the first cell, printfun.cxx:679-684;
/// correctness review 4, M23).
pub fn print_area(wb: &Workbook, sheet: SheetId) -> Option<CellRange> {
    let s = wb.sheet(sheet)?;
    if let Some(r) = s.meta.page_setup.print_range {
        return Some(CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1));
    }
    let area = used_print_area(wb, s, sheet)?;
    let area = CellRange::new(sheet, 0, 0, area.r1, area.c1);
    Some(extend_for_overflow(wb, s, area))
}

/// The used area plus formatted cells and charts.
fn used_print_area(wb: &Workbook, s: &Sheet, sheet: SheetId) -> Option<CellRange> {
    let mut area = s.used_range();
    let mut extend = |r: CellRange| {
        area = Some(match area {
            None => r,
            Some(a) => CellRange::new(
                sheet,
                a.r0.min(r.r0),
                a.c0.min(r.c0),
                a.r1.max(r.r1),
                a.c1.max(r.c1),
            ),
        });
    };
    // Styled cells with a fill or borders.
    for (ci, col) in s.columns.iter().enumerate() {
        let Some(last) = col.styles.last_styled_row() else {
            continue;
        };
        let mut first: Option<u32> = None;
        let mut last_visible: Option<u32> = None;
        for (a, b, id) in col.styles.runs_in(0, last) {
            let st = &wb.styles.get(id).data;
            if st.fill.is_some()
                || st.borders.top.is_some()
                || st.borders.bottom.is_some()
                || st.borders.left.is_some()
                || st.borders.right.is_some()
            {
                first = Some(first.map_or(a, |f| f.min(a)));
                last_visible = Some(last_visible.map_or(b, |l| l.max(b)));
            }
        }
        if let (Some(a), Some(b)) = (first, last_visible) {
            extend(CellRange::new(sheet, a, ci as u32, b, ci as u32));
        }
    }
    if s.meta.page_setup.print_charts {
        for ch in s.meta.charts.iter() {
            extend(chart_cells(s, &ch.data.anchor));
        }
        for im in s.meta.images.iter() {
            extend(chart_cells(s, &im.data.anchor));
        }
    }
    area
}

fn axis_of(
    sheet: &Sheet,
    lo: u32,
    hi: u32,
    rows: bool,
    repeat: Option<(u32, u32)>,
    manual: &BTreeSet<u32>,
) -> Axis {
    let size = |i: u32| {
        if rows {
            row_height(sheet, i)
        } else {
            col_width(sheet, i)
        }
    };
    let items: Vec<(u32, u32)> = (lo..=hi)
        .map(|i| (i, size(i)))
        .filter(|(_, s)| *s > 0)
        .collect();
    let repeat = repeat.and_then(|(a, b)| {
        let items: Vec<(u32, u32)> = (a..=b.min(if rows { MAX_ROWS - 1 } else { MAX_COLS - 1 }))
            .map(|i| (i, size(i)))
            .filter(|(_, s)| *s > 0)
            .collect();
        (!items.is_empty()).then_some(((a, b), items))
    });
    // A manual break before a hidden row breaks before the next visible
    // one (AOO keeps it, sc/source/core/data/table5.cxx:212-218,250-257;
    // correctness review 4, M25).
    let mut breaks = BTreeSet::new();
    for b in manual.range(lo + 1..=hi) {
        let k = items.partition_point(|(i, _)| *i < *b);
        if let Some((i, _)) = items.get(k) {
            breaks.insert(*i);
        }
    }
    Axis {
        items,
        manual: breaks,
        repeat,
    }
}

/// The body and the header/footer areas of a page setup.
pub fn body_of(p: &PageSetupData) -> Body {
    let (pw, ph) = paper_hmm(p);
    let m = &p.margins;
    // Printed row/column headers take their space from the body
    // (AOO printfun.cxx:2953-2957; correctness review 4, M26).
    let pt_hmm = |pt: f32| (pt * 2540.0 / 72.0).round() as u32;
    let (hdr_w, hdr_h) = if p.print_headers {
        (pt_hmm(ROW_HEADER_W_PT), pt_hmm(COL_HEADER_H_PT))
    } else {
        (0, 0)
    };
    let inner_w = pw.saturating_sub(m.left + m.right);
    let block = |a: &HfAreaData, top: u32, total: u32| HfBlock {
        top,
        height: total.saturating_sub(a.spacing),
        left: m.left + a.left.min(inner_w),
        width: inner_w
            .saturating_sub(a.left.saturating_add(a.right))
            .max(1000),
    };
    // The areas never squeeze the body below its minimum: what does not
    // fit is taken from the header first, then the footer (their lines
    // that no longer fit are left off, see `hf_fitting_lines`).
    let mut room = ph.saturating_sub(m.top + m.bottom + hdr_h + MIN_BODY_HMM);
    // Header: from the top margin down, its spacing at its foot, then the
    // body.
    let header_room = p
        .header_shown()
        .map_or(0, |h| p.header_area.printed_height(h, m.header))
        .min(room);
    room -= header_room;
    let header = p
        .header_shown()
        .map(|_| block(&p.header_area, m.top, header_room));
    // Footer: ends at the bottom margin, its spacing at its top.
    let footer_room = p
        .footer_shown()
        .map_or(0, |f| p.footer_area.printed_height(f, m.footer))
        .min(room);
    let footer = p.footer_shown().map(|_| {
        let a = &p.footer_area;
        let top = ph
            .saturating_sub(m.bottom.saturating_add(footer_room))
            .saturating_add(a.spacing.min(footer_room));
        block(a, top, footer_room)
    });
    let w = pw.saturating_sub(m.left + m.right + hdr_w).max(1000);
    let h = ph
        .saturating_sub(m.top + m.bottom + hdr_h)
        .saturating_sub(header_room)
        .saturating_sub(footer_room)
        .max(MIN_BODY_HMM);
    Body {
        w,
        h,
        top: m.top.saturating_add(header_room),
        header,
        footer,
    }
}

/// Pages `(row bands, col bands)` at `scale`.
fn bands_at(
    rows: &Axis,
    cols: &Axis,
    body: (u32, u32),
    scale: u32,
    skip: (bool, bool),
) -> (Vec<Band>, Vec<Band>) {
    (
        bands(rows, body.1, scale, skip.0),
        bands(cols, body.0, scale, skip.1),
    )
}

/// The scale percent of a page setup: `Percent` as is, fit modes searched
/// from 100 % down to 10 %.
pub fn resolve_scale(p: &PageSetupData, rows: &Axis, cols: &Axis, body: (u32, u32)) -> u32 {
    match p.scale {
        PageScaleData::Percent(n) => n.clamp(10, 400),
        PageScaleData::FitTo { width, height } => {
            let skip = (height > 0, width > 0);
            let mut best = 10;
            for s in (10..=100).rev() {
                let (rb, cb) = bands_at(rows, cols, body, s, skip);
                let ok = (width == 0 || cb.len() as u32 <= width)
                    && (height == 0 || rb.len() as u32 <= height);
                if ok {
                    best = s;
                    break;
                }
            }
            best
        }
        PageScaleData::FitPages(n) => {
            let n = n.max(1) as usize;
            let mut best = 10;
            for s in (10..=100).rev() {
                let (rb, cb) = bands_at(rows, cols, body, s, (true, true));
                if rb.len().saturating_mul(cb.len()) <= n {
                    best = s;
                    break;
                }
            }
            best
        }
    }
}

fn skip_breaks(p: &PageSetupData) -> (bool, bool) {
    match p.scale {
        PageScaleData::Percent(_) => (false, false),
        PageScaleData::FitTo { width, height } => (height > 0, width > 0),
        PageScaleData::FitPages(_) => (true, true),
    }
}

/// `true` if the page band holds any cell content or chart.
fn band_has_content(
    wb: &Workbook,
    sheet: &Sheet,
    rows: &[(u32, u32)],
    cols: &[(u32, u32)],
) -> bool {
    let (Some(r0), Some(r1)) = (rows.first(), rows.last()) else {
        return false;
    };
    let (Some(c0), Some(c1)) = (cols.first(), cols.last()) else {
        return false;
    };
    let range = CellRange::new(sheet.id, r0.0, c0.0, r1.0, c1.0);
    for c in range.c0..=range.c1.min(sheet.columns.len().saturating_sub(1) as u32) {
        if sheet.columns.is_empty() {
            break;
        }
        let Some(col) = sheet.column(c) else {
            continue;
        };
        if col.cells.iter_range(range.r0, range.r1).next().is_some() {
            return true;
        }
    }
    if sheet.meta.page_setup.print_charts {
        let any = sheet
            .meta
            .charts
            .iter()
            .map(|c| chart_cells(sheet, &c.data.anchor))
            .chain(
                sheet
                    .meta
                    .images
                    .iter()
                    .map(|i| chart_cells(sheet, &i.data.anchor)),
            )
            .any(|r| r.intersects(&range));
        if any {
            return true;
        }
    }
    let _ = wb;
    false
}

/// Paginates one sheet (page numbers and headers are filled in later by
/// [`layout`]).
pub fn paginate_sheet(
    wb: &Workbook,
    sheet: SheetId,
    selection: Option<CellRange>,
    suppress_empty: bool,
) -> Result<Vec<PageSpec>, ApiError> {
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))?;
    let Some(area) = selection.or_else(|| print_area(wb, sheet)) else {
        return Ok(Vec::new());
    };
    let p = &s.meta.page_setup;
    let b = body_of(p);
    let (body_w, body_h) = (b.w, b.h);
    let body = (body_w, body_h);
    let rows = axis_of(s, area.r0, area.r1, true, p.repeat_rows, &s.meta.row_breaks);
    let cols = axis_of(
        s,
        area.c0,
        area.c1,
        false,
        p.repeat_cols,
        &s.meta.col_breaks,
    );
    if rows.items.is_empty() || cols.items.is_empty() {
        return Ok(Vec::new());
    }
    let scale = resolve_scale(p, &rows, &cols, body);
    let (rb, cb) = bands_at(&rows, &cols, body, scale, skip_breaks(p));
    if rb.len().saturating_mul(cb.len()) > MAX_PAGES {
        return Err(ApiError::unsupported(format!(
            "the sheet would print on more than {MAX_PAGES} pages"
        )));
    }
    let paper = paper_hmm(p);
    // Scaled sizes, computed once and shared by every page.
    let scale_all = |v: &[(u32, u32)]| -> std::sync::Arc<[(u32, u32)]> {
        v.iter().map(|(i, h)| (*i, scaled(*h, scale))).collect()
    };
    let row_all = scale_all(&rows.items);
    let col_all = scale_all(&cols.items);
    let repeat_of = |a: &Axis| -> Items {
        a.repeat
            .as_ref()
            .map(|(_, items)| {
                let all = scale_all(items);
                let n = all.len();
                Items::slice(&all, 0, n)
            })
            .unwrap_or_default()
    };
    let repeat_rows_all = repeat_of(&rows);
    let repeat_cols_all = repeat_of(&cols);
    let mut pages = Vec::new();
    let mut push = |r: &Band, c: &Band| {
        let page_rows = Items::slice(&row_all, r.start, r.end);
        let page_cols = Items::slice(&col_all, c.start, c.end);
        if suppress_empty && !band_has_content(wb, s, &page_rows, &page_cols) {
            return;
        }
        let repeat_rows = if r.with_repeat {
            repeat_rows_all.clone()
        } else {
            Items::default()
        };
        let repeat_cols = if c.with_repeat {
            repeat_cols_all.clone()
        } else {
            Items::default()
        };
        let content_w: u32 = repeat_cols
            .iter()
            .chain(page_cols.iter())
            .map(|c| c.1)
            .sum();
        let content_h: u32 = repeat_rows
            .iter()
            .chain(page_rows.iter())
            .map(|r| r.1)
            .sum();
        let mut x = p.margins.left;
        let mut y = b.top;
        if p.center_h && content_w < body_w {
            x += (body_w - content_w) / 2;
        }
        if p.center_v && content_h < body_h {
            y += (body_h - content_h) / 2;
        }
        pages.push(PageSpec {
            sheet,
            rows: page_rows,
            cols: page_cols,
            repeat_rows,
            repeat_cols,
            scale,
            paper,
            origin: (x, y),
            body,
            header_block: b.header,
            footer_block: b.footer,
            number: 0,
            header: None,
            footer: None,
            area,
        });
    };
    match p.page_order {
        PageOrderData::TopToBottom => {
            for c in &cb {
                for r in &rb {
                    push(r, c);
                }
            }
        }
        PageOrderData::LeftToRight => {
            for r in &rb {
                for c in &cb {
                    push(r, c);
                }
            }
        }
    }
    Ok(pages)
}

/// The sheets a print covers, in tab order.
pub fn sheets_of(wb: &Workbook, opts: &PrintOptionsDto) -> Result<Vec<SheetId>, ApiError> {
    Ok(match &opts.sheets {
        PrintSheets::Active => vec![wb.active_sheet],
        PrintSheets::All => wb
            .sheets
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.id)
            .collect(),
        PrintSheets::Some { ids } => {
            let mut out: Vec<SheetId> = Vec::new();
            for s in &wb.sheets {
                if ids.contains(&s.id) && !out.contains(&s.id) {
                    out.push(s.id);
                }
            }
            if let Some(missing) = ids.iter().find(|id| wb.sheet(**id).is_none()) {
                return Err(ApiError::not_found(format!("no sheet {missing}")));
            }
            out
        }
    })
}

/// Paginates every requested sheet, numbers the pages, resolves headers
/// and footers and applies the page range.
pub fn layout(
    wb: &Workbook,
    opts: &PrintOptionsDto,
    ctx: &HfContext,
) -> Result<(Vec<PageSpec>, Vec<String>), ApiError> {
    let mut warnings = Vec::new();
    let mut pages: Vec<PageSpec> = Vec::new();
    let selection = match &opts.selection {
        Some(r) => Some(crate::ops::to_range(wb.active_sheet, r)?),
        None => None,
    };
    for sheet in sheets_of(wb, opts)? {
        let sel = selection.filter(|s| s.sheet == sheet);
        let mut sp = paginate_sheet(wb, sheet, sel, opts.suppress_empty_pages)?;
        if pages.len() + sp.len() > MAX_PAGES {
            return Err(ApiError::unsupported(format!(
                "the print would have more than {MAX_PAGES} pages"
            )));
        }
        pages.append(&mut sp);
    }
    let total = pages.len() as u32;
    // Page numbers run through the job; a sheet's first page number restarts
    // the count there (AOO `first-page-number`).
    let mut number = 0u32;
    let mut last_sheet: Option<SheetId> = None;
    for page in pages.iter_mut() {
        let p = wb
            .sheet(page.sheet)
            .map(|s| s.meta.page_setup.clone())
            .unwrap_or_default();
        if last_sheet != Some(page.sheet) {
            if let Some(first) = p.first_page_number {
                number = first.saturating_sub(1);
            }
            last_sheet = Some(page.sheet);
        }
        number = number.saturating_add(1);
        page.number = number;
        let name = wb
            .sheet(page.sheet)
            .map(|s| s.name.to_string())
            .unwrap_or_default();
        let resolve = |h: &tally_model::HeaderFooterData| {
            [
                resolve_placeholders(&h.left, number, total, &name, ctx),
                resolve_placeholders(&h.center, number, total, &name, ctx),
                resolve_placeholders(&h.right, number, total, &name, ctx),
            ]
        };
        page.header = p.header_shown().map(resolve);
        page.footer = p.footer_shown().map(resolve);
    }
    if let (Some(from), to) = (opts.page_from, opts.page_to) {
        let from = from.max(1) as usize;
        let to = to.map_or(pages.len(), |t| (t as usize).min(pages.len()));
        if from > pages.len() || from > to {
            warnings.push("The page range selects no pages.".into());
            pages.clear();
        } else {
            pages = pages.drain(from - 1..to).collect();
        }
    } else if let Some(to) = opts.page_to {
        pages.truncate(to as usize);
    }
    Ok((pages, warnings))
}

/// `print_layout`: pagination metadata only.
pub fn print_layout(
    wb: &Workbook,
    opts: &PrintOptionsDto,
    ctx: &HfContext,
) -> Result<PrintLayoutDto, ApiError> {
    let (pages, warnings) = layout(wb, opts, ctx)?;
    Ok(PrintLayoutDto {
        pages: pages
            .iter()
            .enumerate()
            .map(|(i, p)| p.to_dto(i as u32))
            .collect(),
        warnings,
    })
}

/// `page_breaks`: the automatic and manual breaks of a sheet at its own
/// settings (Page Break Preview).
pub fn page_breaks(wb: &Workbook, sheet: SheetId) -> Result<PageBreaksDto, ApiError> {
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))?;
    let Some(area) = print_area(wb, sheet) else {
        return Ok(PageBreaksDto {
            area: None,
            row_breaks: Vec::new(),
            col_breaks: Vec::new(),
        });
    };
    let p = &s.meta.page_setup;
    let b = body_of(p);
    let body = (b.w, b.h);
    let rows = axis_of(s, area.r0, area.r1, true, p.repeat_rows, &s.meta.row_breaks);
    let cols = axis_of(
        s,
        area.c0,
        area.c1,
        false,
        p.repeat_cols,
        &s.meta.col_breaks,
    );
    let mut row_breaks = Vec::new();
    let mut col_breaks = Vec::new();
    if !rows.items.is_empty() && !cols.items.is_empty() {
        let scale = resolve_scale(p, &rows, &cols, body);
        let (rb, cb) = bands_at(&rows, &cols, body, scale, skip_breaks(p));
        for b in rb.iter().skip(1) {
            let at = rows.items[b.start].0;
            row_breaks.push(BreakInfo {
                at,
                manual: b.manual,
            });
        }
        for b in cb.iter().skip(1) {
            let at = cols.items[b.start].0;
            col_breaks.push(BreakInfo {
                at,
                manual: b.manual,
            });
        }
    }
    Ok(PageBreaksDto {
        area: Some(RangeDto {
            r0: area.r0,
            c0: area.c0,
            r1: area.r1,
            c1: area.c1,
        }),
        row_breaks,
        col_breaks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_model::{HeaderFooterData, HfAreaData, MarginsData};
    use tally_types::{CellAddr, Value};

    /// A4 portrait, 20 mm margins, no header/footer, 4.52 mm rows and
    /// 22.58 mm columns (the ODS defaults) over `rows × cols` cells.
    fn book(rows: u32, cols: u32) -> Workbook {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        for r in 0..rows {
            for c in 0..cols {
                wb.set_value(
                    CellAddr::new(s, r, c),
                    Value::Number(f64::from(r * cols + c)),
                );
            }
        }
        let sheet = wb.sheet_mut(s).unwrap();
        sheet.row_dims.set(0, MAX_ROWS - 1, 452, false);
        sheet.col_dims.set(0, MAX_COLS - 1, 2258, false);
        let p = std::sync::Arc::make_mut(&mut sheet.meta.page_setup);
        p.margins = MarginsData {
            top: 2000,
            bottom: 2000,
            left: 2000,
            right: 2000,
            header: 500,
            footer: 500,
        };
        p.header = None;
        p.footer = None;
        wb
    }

    fn opts() -> PrintOptionsDto {
        PrintOptionsDto {
            sheets: PrintSheets::Active,
            selection: None,
            suppress_empty_pages: true,
            page_from: None,
            page_to: None,
        }
    }

    fn pages(wb: &Workbook) -> Vec<PageSpec> {
        layout(wb, &opts(), &HfContext::default()).unwrap().0
    }

    fn setup(wb: &mut Workbook) -> &mut PageSetupData {
        let s = wb.sheets[0].id;
        std::sync::Arc::make_mut(&mut wb.sheet_mut(s).unwrap().meta.page_setup)
    }

    #[test]
    fn a4_width_in_points() {
        assert!((hmm_to_pt(21_000) - 595.28).abs() < 0.01);
    }

    #[test]
    fn a4_rows_per_page_is_56() {
        let wb = book(100, 1);
        let p = pages(&wb);
        assert_eq!(p[0].rows.len(), 56, "257 mm / 4.52 mm");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].rows.len(), 44);
        assert_eq!(p[0].row_span(), [0, 55]);
    }

    #[test]
    fn a4_cols_per_page_is_7() {
        let wb = book(1, 20);
        let p = pages(&wb);
        assert_eq!(p[0].cols.len(), 7, "170 mm / 22.58 mm");
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn repeat_rows_every_page() {
        let mut wb = book(130, 1);
        setup(&mut wb).repeat_rows = Some((0, 1));
        let p = pages(&wb);
        assert_eq!(p.len(), 3);
        assert_eq!(p[2].repeat_rows.len(), 2);
        assert_eq!(p[2].rows.len(), 130 - 56 - 54);
        assert!(
            p[0].repeat_rows.is_empty(),
            "the first page holds them naturally"
        );
        assert_eq!(p[1].repeat_rows.len(), 2);
        assert_eq!(
            p[1].rows.len(),
            54,
            "two rows of space taken by the repeats"
        );
    }

    #[test]
    fn repeat_rows_not_duplicated_on_first_page() {
        let mut wb = book(60, 1);
        setup(&mut wb).repeat_rows = Some((0, 2));
        let p = pages(&wb);
        assert_eq!(p[0].rows[0].0, 0);
        assert!(p[0].repeat_rows.is_empty());
        assert_eq!(
            p[1].repeat_rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(p[1].rows[0].0, 56);
    }

    #[test]
    fn fit_to_1x1_finds_largest_scale() {
        let mut wb = book(100, 10);
        setup(&mut wb).scale = PageScaleData::FitTo {
            width: 1,
            height: 1,
        };
        let p = pages(&wb);
        assert_eq!(p.len(), 1);
        let s = p[0].scale;
        // 100 rows × 4.52 mm = 452 mm into 257 mm → 57 % (a scaled row is
        // ⌊452·57/100⌋ = 257 hmm, 100 of them fit exactly); 10 cols ×
        // 22.58 = 225.8 mm into 170 mm → 75 %: the row limit wins.
        assert_eq!(s, 57, "{}", p[0].scale);
    }

    #[test]
    fn fit_width_only() {
        let mut wb = book(100, 10);
        setup(&mut wb).scale = PageScaleData::FitTo {
            width: 1,
            height: 0,
        };
        let p = pages(&wb);
        assert_eq!(p[0].scale, 75);
        assert!(p.iter().all(|pg| pg.cols.len() == 10));
        assert!(p.len() > 1, "height is unlimited");
    }

    #[test]
    fn fit_pages_total() {
        let mut wb = book(100, 10);
        setup(&mut wb).scale = PageScaleData::FitPages(2);
        let p = pages(&wb);
        assert!(p.len() <= 2, "{}", p.len());
        assert!(p[0].scale > 57 && p[0].scale <= 75, "{}", p[0].scale);
        let mut one = book(100, 10);
        setup(&mut one).scale = PageScaleData::FitPages(1);
        assert_eq!(pages(&one)[0].scale, 57);
    }

    #[test]
    fn manual_row_break_forces_page() {
        let mut wb = book(20, 1);
        let s = wb.sheets[0].id;
        std::sync::Arc::make_mut(&mut wb.sheet_mut(s).unwrap().meta.row_breaks).insert(5);
        let p = pages(&wb);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].row_span(), [0, 4]);
        assert_eq!(p[1].row_span(), [5, 19]);
        let b = page_breaks(&wb, s).unwrap();
        assert_eq!(
            b.row_breaks,
            vec![BreakInfo {
                at: 5,
                manual: true
            }]
        );
        // Fit modes ignore manual breaks on the fitted axis.
        setup(&mut wb).scale = PageScaleData::FitPages(1);
        assert_eq!(pages(&wb).len(), 1);
    }

    #[test]
    fn hidden_rows_skipped() {
        let mut wb = book(60, 1);
        let s = wb.sheets[0].id;
        wb.sheet_mut(s)
            .unwrap()
            .row_dims
            .set_hidden(10, 19, true, 452);
        let p = pages(&wb);
        assert_eq!(p.len(), 1, "50 visible rows fit on one page");
        assert!(p[0].rows.iter().all(|(r, _)| !(10..=19).contains(r)));
    }

    #[test]
    fn page_order_left_to_right() {
        let mut wb = book(100, 10);
        let p = pages(&wb);
        assert_eq!(
            (p[0].col_span(), p[1].col_span()),
            ([0, 6], [0, 6]),
            "down first"
        );
        setup(&mut wb).page_order = PageOrderData::LeftToRight;
        let p = pages(&wb);
        assert_eq!(
            (p[0].col_span(), p[1].col_span()),
            ([0, 6], [7, 9]),
            "across first"
        );
    }

    #[test]
    fn print_range_limits_area() {
        let mut wb = book(100, 10);
        setup(&mut wb).print_range = Some(tally_model::CellRangeLocal {
            r0: 2,
            c0: 1,
            r1: 9,
            c1: 3,
        });
        let p = pages(&wb);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].row_span(), [2, 9]);
        assert_eq!(p[0].col_span(), [1, 3]);
    }

    #[test]
    fn empty_pages_suppressed() {
        let mut wb = book(1, 1);
        let s = wb.sheets[0].id;
        wb.set_value(CellAddr::new(s, 150, 0), Value::Number(1.0));
        let p = pages(&wb);
        assert_eq!(p.len(), 2, "page 2 (rows 56–111) is empty");
        assert_eq!(p[1].row_span()[0], 112);
        let mut o = opts();
        o.suppress_empty_pages = false;
        assert_eq!(layout(&wb, &o, &HfContext::default()).unwrap().0.len(), 3);
    }

    #[test]
    fn centering_offsets() {
        let mut wb = book(10, 2);
        let p = pages(&wb);
        assert_eq!(p[0].origin, (2000, 2000));
        let st = setup(&mut wb);
        st.center_h = true;
        st.center_v = true;
        let p = pages(&wb);
        // content 45.16 × 45.2 mm in a 170 × 257 mm body.
        assert_eq!(
            p[0].origin,
            (2000 + (17_000 - 4516) / 2, 2000 + (25_700 - 4520) / 2)
        );
    }

    #[test]
    fn header_placeholders_resolved() {
        let mut wb = book(100, 1);
        let st = setup(&mut wb);
        st.header = Some(HeaderFooterData {
            left: "&[FILE] && &[SHEET]".into(),
            center: "Page &[PAGE] of &[PAGES]".into(),
            right: "&[DATE] &[TIME] &[PATH] &[X]".into(),
        });
        let ctx = HfContext {
            file: "Book.ods".into(),
            path: "/tmp/Book.ods".into(),
            date: "2026-10-20".into(),
            time: "10:00:00".into(),
        };
        let (p, _) = layout(&wb, &opts(), &ctx).unwrap();
        assert_eq!(p.len(), 2);
        let h = p[1].header.as_ref().unwrap();
        assert_eq!(h[0], "Book.ods & Sheet1");
        assert_eq!(h[1], "Page 2 of 2");
        assert_eq!(h[2], "2026-10-20 10:00:00 /tmp/Book.ods &[X]");
        assert!(p[0].header_block.is_some());
        assert_eq!(p[0].rows.len(), 55, "the 5 mm header block takes a row");
    }

    #[test]
    fn first_page_number_offsets() {
        let mut wb = book(100, 1);
        setup(&mut wb).first_page_number = Some(5);
        let p = pages(&wb);
        assert_eq!((p[0].number, p[1].number), (5, 6));
        let dto = print_layout(&wb, &opts(), &HfContext::default()).unwrap();
        assert_eq!(dto.pages[1].number, 6);
        assert_eq!(dto.pages[1].index, 1);
        let mut o = opts();
        o.page_from = Some(2);
        let (p, _) = layout(&wb, &o, &HfContext::default()).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].number, 6);
    }

    #[test]
    fn charts_extend_used_area() {
        let mut wb = book(1, 1);
        let s = wb.sheets[0].id;
        {
            let sheet = wb.sheet_mut(s).unwrap();
            let charts = std::sync::Arc::make_mut(&mut sheet.meta.charts);
            charts.push(std::sync::Arc::new(crate::charts::Chart {
                id: 1,
                data: tally_model::ChartData {
                    anchor: tally_model::ChartAnchorData {
                        row: 3,
                        col: 2,
                        dx_hmm: 0,
                        dy_hmm: 0,
                        width_hmm: 8000,
                        height_hmm: 6000,
                    },
                    ..Default::default()
                },
            }));
        }
        let area = print_area(&wb, s).unwrap();
        assert_eq!((area.r0, area.c0), (0, 0));
        assert!(area.r1 >= 3 + 13 && area.c1 >= 2 + 3, "{area:?}");
        let p = pages(&wb);
        assert_eq!(p.len(), 1);
        assert!(p[0].cols.len() >= 6);
        setup(&mut wb).print_charts = false;
        assert_eq!(print_area(&wb, s).unwrap().r1, 0);
    }

    #[test]
    fn selection_and_sheets() {
        let mut wb = book(100, 10);
        let s2 = wb.add_sheet(Some("Two"), None);
        wb.set_value(CellAddr::new(s2, 0, 0), Value::Number(1.0));
        let mut o = opts();
        o.sheets = PrintSheets::All;
        let (p, _) = layout(&wb, &o, &HfContext::default()).unwrap();
        assert_eq!(p.last().unwrap().sheet, s2);
        assert_eq!(p.len(), 5);
        o.sheets = PrintSheets::Active;
        o.selection = Some(RangeDto {
            r0: 0,
            c0: 0,
            r1: 9,
            c1: 1,
        });
        let (p, _) = layout(&wb, &o, &HfContext::default()).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].row_span(), [0, 9]);
        o.sheets = PrintSheets::Some { ids: vec![99] };
        assert!(layout(&wb, &o, &HfContext::default()).is_err());
    }

    /// The owner's report: a two-line centred header over a small top
    /// margin.
    fn two_line_header(wb: &mut Workbook) {
        let st = setup(wb);
        st.margins.top = 1000;
        st.margins.header = 750;
        st.header = Some(HeaderFooterData {
            center: "The Redwood - 10/22/2026\nBlock 90 - North Side".into(),
            ..HeaderFooterData::default()
        });
        st.header_area = HfAreaData::default();
    }

    #[test]
    fn header_area_autofits_two_lines_and_pushes_the_body_down() {
        let mut wb = book(200, 1);
        two_line_header(&mut wb);
        let p = pages(&wb);
        let b = p[0].header_block.unwrap();
        // Two 4.94 mm lines below the 10 mm top margin, then 2.5 mm spacing.
        assert_eq!(
            b,
            HfBlock {
                top: 1000,
                height: 2 * HF_LINE_HMM,
                left: 2000,
                width: 17_000,
            }
        );
        assert_eq!(p[0].origin.1, 1000 + 2 * HF_LINE_HMM + 250);
        assert!(
            b.top + b.height < p[0].origin.1,
            "the header overlaps the body"
        );
        // Body 297 − 10 − 20 − 12.38 mm = 254.62 mm: 56 rows of 4.52 mm.
        assert_eq!(p[0].body.1, 29_700 - 1000 - 2000 - (2 * HF_LINE_HMM + 250));
        assert_eq!(p[0].rows.len(), 56);
        assert_eq!(p[1].row_span()[0], 56);
        // Page breaks agree.
        let br = page_breaks(&wb, wb.sheets[0].id).unwrap();
        assert_eq!(br.row_breaks[0].at, 56);
    }

    #[test]
    fn one_line_header_keeps_its_set_height() {
        let mut wb = book(200, 1);
        let st = setup(&mut wb);
        st.margins.header = 750;
        st.header = Some(HeaderFooterData {
            center: "x".into(),
            ..HeaderFooterData::default()
        });
        let p = pages(&wb);
        // 4.94 mm + 2.5 mm fit the 7.5 mm area: AOO's 53 rows on A4.
        assert_eq!(p[0].header_block.unwrap().height, 500);
        assert_eq!(p[0].origin.1, 2750);
        assert_eq!(p[0].rows.len(), (29_700 - 4000 - 750) / 452);
    }

    #[test]
    fn fixed_height_does_not_grow() {
        let mut wb = book(200, 1);
        two_line_header(&mut wb);
        setup(&mut wb).header_area.autofit = false;
        let p = pages(&wb);
        assert_eq!(p[0].header_block.unwrap().height, 500);
        assert_eq!(p[0].origin.1, 1000 + 750);
        // Only the first line fits the 5 mm text box.
        assert_eq!(hf_fitting_lines(&p[0].header_block.unwrap()), 1);
        setup(&mut wb).margins.header = 200;
        assert_eq!(hf_fitting_lines(&pages(&wb)[0].header_block.unwrap()), 0);
    }

    #[test]
    fn oversized_areas_keep_the_minimum_body() {
        let mut wb = book(200, 1);
        let st = setup(&mut wb);
        st.header = Some(HeaderFooterData {
            center: "h".into(),
            ..HeaderFooterData::default()
        });
        st.footer = Some(HeaderFooterData {
            center: "f".into(),
            ..HeaderFooterData::default()
        });
        st.margins.header = 30_000;
        st.margins.footer = 20_000;
        let p = pages(&wb);
        let b = body_of(setup(&mut wb));
        // 297 − 40 mm leaves 257 mm: 247 for the header, none for the footer.
        assert_eq!(b.h, MIN_BODY_HMM);
        assert_eq!(b.top, 2000 + 24_700);
        assert_eq!(b.top + b.h, 29_700 - 2000);
        assert_eq!(b.footer.unwrap().height, 0);
        assert_eq!(hf_fitting_lines(&b.footer.unwrap()), 0);
        let h = b.header.unwrap();
        assert!(h.top + h.height <= b.top);
        assert_eq!(p[0].rows.len() as u32, MIN_BODY_HMM / 452);
        // Margins alone taller than the paper: no room for the areas.
        let st = setup(&mut wb);
        st.margins.top = 20_000;
        st.margins.bottom = 20_000;
        let b = body_of(st);
        assert_eq!((b.header.unwrap().height, b.footer.unwrap().height), (0, 0));
    }

    #[test]
    fn footer_area_ends_at_the_bottom_margin() {
        let mut wb = book(200, 1);
        let st = setup(&mut wb);
        st.margins.footer = 750;
        st.footer = Some(HeaderFooterData {
            center: "Page &[PAGE]\nconfidential".into(),
            ..HeaderFooterData::default()
        });
        st.footer_area.left = 1000;
        st.footer_area.right = 500;
        let p = pages(&wb);
        let f = p[0].footer_block.unwrap();
        let room = 2 * HF_LINE_HMM + 250;
        assert_eq!(f.top, 29_700 - 2000 - room + 250);
        assert_eq!(f.top + f.height, 29_700 - 2000, "ends at the bottom margin");
        assert_eq!((f.left, f.width), (3000, 15_500));
        assert_eq!(p[0].body.1, 29_700 - 4000 - room);
        let body_bottom = p[0].origin.1 + p[0].body.1;
        assert!(body_bottom + 250 <= f.top);
        assert_eq!(p[0].rows.len() as u32, (29_700 - 4000 - room) / 452);
        assert_eq!(p[0].footer.as_ref().unwrap()[1], "Page 1\nconfidential");
    }

    #[test]
    fn header_off_keeps_its_text_and_gives_the_room_back() {
        let mut wb = book(200, 1);
        two_line_header(&mut wb);
        setup(&mut wb).header_area.on = false;
        let p = pages(&wb);
        assert!(p[0].header_block.is_none());
        assert!(p[0].header.is_none());
        assert_eq!(p[0].origin.1, 1000);
        assert_eq!(p[0].rows.len() as u32, (29_700 - 3000) / 452);
        let st = setup(&mut wb);
        assert!(st
            .header
            .as_ref()
            .unwrap()
            .center
            .starts_with("The Redwood"));
        st.header_area.on = true;
        assert!(pages(&wb)[0].header_block.is_some());
    }
}
