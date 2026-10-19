//! Streaming `xl/worksheets/sheetN.xml` → [`SheetData`].
//!
//! Parsed: `sheetPr/tabColor`, `sheetFormatPr`, `sheetViews/sheetView/pane`
//! (frozen panes), `cols/col` (widths, hidden, column styles),
//! `sheetData/row` (heights, hidden) and `c` (values, shared strings,
//! inline strings, booleans, errors, ISO dates, formulas: plain, shared and
//! array), `mergeCells`. Everything else (`dataValidations`,
//! `conditionalFormatting`, `hyperlinks`, drawings, …) is skipped.
//!
//! Cell positions come from the `r` attributes (or are implied by order
//! when absent). Out-of-grid references are skipped; duplicate positions keep
//! the last cell. Every cell element materializes at most one model cell,
//! so the cell budget is bounded by the XML itself and by
//! [`Limits::max_cells`].

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Arc;

use tally_model::{CellContentData, CellData, CellRangeLocal, DimRun, SheetData, StyleRun};
use tally_types::{parse_a1, CellAddr, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::colors::{XColor, INDEXED};
use crate::dates::{excel_serial_to_tally, iso_to_serial};
use crate::formula::{excel_to_of, parse_excel, print_of, XlsxResolver};
use crate::sst::{read_string_item, MAX_CELL_TEXT};
use crate::styles::StyleTable;
use crate::units::{col_width_to_px, parse_bool, pt_to_hmm, px_to_hmm};
use crate::xml::{Elem, Ev, XmlReader};
use crate::{Limits, XlsxError, DEFAULT_ROW_HEIGHT_HMM};

/// Longest formula text accepted: the formula parser's own limit (longer
/// `<f>` text fails with a limit error instead of being kept).
const MAX_FORMULA_TEXT: usize = 8192;

/// Most merged areas read per sheet; later ones are dropped with a warning
/// (security review 2, HIGH-1).
pub const MAX_MERGES_PER_SHEET: usize = 100_000;

/// Most `<col>` elements read per sheet (a sheet has 16,384 columns, so a
/// valid file never needs more; review 2, MEDIUM-2).
const MAX_COL_ELEMENTS: usize = MAX_COLS as usize;

/// Budgets shared by all sheets of one workbook.
#[derive(Debug, Default)]
pub(crate) struct Budget {
    pub cells: u64,
    pub text_bytes: u64,
    /// What was dropped to stay within limits (one entry per kind).
    pub warnings: Vec<String>,
}

impl Budget {
    pub(crate) fn warn(&mut self, w: String) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Counts `n` cells (merges, array block areas, style runs) against
    /// [`Limits::max_cells`].
    fn add_cells(&mut self, n: u64, limits: &Limits) -> Result<(), XlsxError> {
        self.cells = self.cells.saturating_add(n);
        if self.cells > limits.max_cells {
            return Err(XlsxError::LimitExceeded {
                what: "materialized cells",
                limit: limits.max_cells,
            });
        }
        Ok(())
    }

    fn add_cell(&mut self, text_len: usize, limits: &Limits) -> Result<(), XlsxError> {
        self.cells += 1;
        if self.cells > limits.max_cells {
            return Err(XlsxError::LimitExceeded {
                what: "materialized cells",
                limit: limits.max_cells,
            });
        }
        self.text_bytes = self.text_bytes.saturating_add(text_len as u64);
        if self.text_bytes > limits.max_text_bytes {
            return Err(XlsxError::LimitExceeded {
                what: "materialized cell text bytes",
                limit: limits.max_text_bytes,
            });
        }
        Ok(())
    }
}

/// Everything a worksheet parse needs from the rest of the package.
pub(crate) struct SheetCtx<'a> {
    pub sheet: SheetId,
    pub sst: &'a [Arc<str>],
    pub styles: &'a StyleTable,
    pub date1904: bool,
    pub resolver: &'a XlsxResolver,
    pub limits: &'a Limits,
}

/// A shared formula group (`t="shared" si="N"`).
enum Shared {
    /// The anchor's model text (shared by every member) and position.
    Parsed { text: Arc<str>, row: u32, col: u32 },
    /// The anchor text did not parse; members keep their cached values.
    Failed,
}

/// The formula of one `<c>`.
enum FText {
    /// Its own text.
    Own(String),
    /// A shared group's anchor text, valid at (`row`, `col`).
    Shared { text: Arc<str>, row: u32, col: u32 },
}

/// What a worksheet holds besides [`SheetData`] that the reader resolves
/// against other parts (hyperlink relationships, comments).
#[derive(Debug, Default)]
pub(crate) struct SheetExtras {
    /// Hyperlinks: (cells, relationship id, in-document location).
    pub links: Vec<crate::sheet_v03::RawLink>,
    /// Page setup pieces.
    pub page: crate::sheet_v03::PageParts,
}

/// Per-sheet parse state.
struct State<'a, 'b> {
    extras: SheetExtras,
    ctx: &'a SheetCtx<'b>,
    budget: &'a mut Budget,
    out: SheetData,
    shared: HashMap<u32, Shared>,
    default_row_hmm: u32,
    /// Next implied row (0-based) for `<row>` without `r`.
    next_row: u32,
    /// Current row (0-based) and next implied column.
    row: u32,
    next_col: u32,
    sorted: bool,
    last_pos: Option<(u32, u32)>,
    /// `<col>` entries with a style: (first, last, style), in file order.
    col_styles: Vec<(u32, u32, u32)>,
    col_elements: usize,
}

/// Parses one worksheet part.
pub(crate) fn parse_sheet<R: BufRead>(
    xr: &mut XmlReader<R>,
    ctx: &SheetCtx<'_>,
    budget: &mut Budget,
) -> Result<(SheetData, SheetExtras), XlsxError> {
    let mut st = State {
        extras: SheetExtras::default(),
        ctx,
        budget,
        out: SheetData::default(),
        shared: HashMap::new(),
        default_row_hmm: DEFAULT_ROW_HEIGHT_HMM,
        next_row: 0,
        row: 0,
        next_col: 0,
        sorted: true,
        last_pos: None,
        col_styles: Vec::new(),
        col_elements: 0,
    };
    loop {
        match xr.next()? {
            Ev::Start(e) => match e.local() {
                // Containers we descend into.
                "worksheet" | "sheetData" | "cols" | "sheetViews" | "mergeCells" | "sheetPr"
                | "dataValidations" | "hyperlinks" => {}
                "conditionalFormatting" => {
                    if let Some(cf) = crate::sheet_v03::parse_cf(
                        xr,
                        &e,
                        ctx.sheet,
                        &ctx.styles.dxfs,
                        ctx.resolver,
                    )? {
                        if st.out.conditional_formats.len() < 100_000 {
                            st.out.conditional_formats.push(cf);
                        }
                    }
                }
                "dataValidation" => {
                    if let Some(v) = crate::sheet_v03::parse_dv(xr, &e, ctx.sheet, ctx.resolver)? {
                        if st.out.validations.len() < 100_000 {
                            st.out.validations.push(v);
                        }
                    }
                }
                "hyperlink" => {
                    if let Some(l) = crate::sheet_v03::parse_link(&e) {
                        if st.extras.links.len() < 100_000 {
                            st.extras.links.push(l);
                        }
                    }
                    xr.skip()?;
                }
                "sheetProtection" => {
                    st.out.protection = crate::sheet_v03::parse_protection(&e);
                    xr.skip()?;
                }
                "pageMargins" | "pageSetup" | "printOptions" | "pageSetUpPr" => {
                    crate::sheet_v03::page_attrs(&e, &mut st.extras.page);
                    xr.skip()?;
                }
                "headerFooter" => crate::sheet_v03::parse_header_footer(xr, &mut st.extras.page)?,
                "rowBreaks" => crate::sheet_v03::parse_breaks(xr, &mut st.out.row_breaks)?,
                "colBreaks" => crate::sheet_v03::parse_breaks(xr, &mut st.out.col_breaks)?,
                "tabColor" => {
                    let c = XColor::from_attrs(|k| e.attr_local(k).map(str::to_string));
                    st.out.tab_color = c.resolve(&INDEXED);
                    xr.skip()?;
                }
                "sheetFormatPr" => {
                    if let Some(h) = attr_f64(&e, "defaultRowHeight").filter(|h| *h > 0.0) {
                        st.default_row_hmm = pt_to_hmm(h);
                    }
                    xr.skip()?;
                }
                "sheetView" => {}
                "pane" => {
                    st.pane(&e);
                    xr.skip()?;
                }
                "col" => {
                    st.col(&e);
                    xr.skip()?;
                }
                "row" => st.row_start(&e),
                "c" => st.cell(xr, &e)?,
                "mergeCell" => {
                    if let Some(r) = e.attr_local("ref").and_then(parse_range) {
                        if r.r0 != r.r1 || r.c0 != r.c1 {
                            if st.out.merges.len() >= MAX_MERGES_PER_SHEET {
                                st.budget.warn(format!(
                                    "Only the first {MAX_MERGES_PER_SHEET} merged areas of a \
                                     sheet are kept."
                                ));
                            } else {
                                // Merges count against the cell cap.
                                st.budget.add_cells(1, ctx.limits)?;
                                st.out.merges.push(r);
                            }
                        }
                    }
                    xr.skip()?;
                }
                _ => xr.skip()?,
            },
            Ev::Eof => break,
            _ => {}
        }
    }
    let runs = column_style_runs(&st.col_styles);
    st.budget.add_cells(runs.len() as u64, ctx.limits)?;
    st.out.style_runs.extend(runs);
    let mut out = st.out;
    if !st.sorted {
        // Stable: of two cells at one position the later one wins below.
        out.cells.sort_by_key(|c| (c.row, c.col));
    }
    dedup_last(&mut out.cells);
    out.row_breaks.sort_unstable();
    out.row_breaks.dedup();
    out.col_breaks.sort_unstable();
    out.col_breaks.dedup();
    Ok((out, st.extras))
}

/// One whole-column style run per styled column. Overlapping `<col>`
/// entries resolve last-wins in one pass from the end with a "next
/// unpainted column" union-find, so N overlapping entries cost
/// O(N + 16,384) and at most 16,384 runs (review 2, MEDIUM-2: 10,000
/// `<col min="1" max="16384">` made 164M runs, 5.8 s and 3.5 GB).
fn column_style_runs(entries: &[(u32, u32, u32)]) -> Vec<StyleRun> {
    if entries.is_empty() {
        return Vec::new();
    }
    let n = MAX_COLS as usize;
    let mut style: Vec<Option<u32>> = vec![None; n];
    // next[i]: the first unpainted column ≥ i (n when none).
    let mut next: Vec<u32> = (0..=MAX_COLS).collect();
    fn find(next: &mut [u32], mut i: u32) -> u32 {
        let mut root = i;
        while next[root as usize] != root {
            root = next[root as usize];
        }
        while next[i as usize] != root {
            let up = next[i as usize];
            next[i as usize] = root;
            i = up;
        }
        root
    }
    for &(c0, c1, st) in entries.iter().rev() {
        let mut c = find(&mut next, c0);
        while c <= c1 && (c as usize) < n {
            style[c as usize] = Some(st);
            next[c as usize] = c + 1;
            c = find(&mut next, c + 1);
        }
    }
    style
        .iter()
        .enumerate()
        .filter_map(|(c, s)| {
            s.map(|style| StyleRun {
                col: c as u32,
                start_row: 0,
                end_row: MAX_ROWS - 1,
                style,
            })
        })
        .collect()
}

/// Keeps the last of consecutive cells at the same position.
fn dedup_last(cells: &mut Vec<CellData>) {
    if cells
        .windows(2)
        .all(|w| (w[0].row, w[0].col) != (w[1].row, w[1].col))
    {
        return;
    }
    let mut out: Vec<CellData> = Vec::with_capacity(cells.len());
    for c in cells.drain(..) {
        match out.last_mut() {
            Some(last) if (last.row, last.col) == (c.row, c.col) => *last = c,
            _ => out.push(c),
        }
    }
    *cells = out;
}

fn attr_f64(e: &Elem, k: &str) -> Option<f64> {
    e.attr_local(k)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn attr_u32(e: &Elem, k: &str) -> Option<u32> {
    e.attr_local(k).and_then(|v| v.trim().parse::<u32>().ok())
}

/// `A1` → (row, col).
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    if s.len() > 16 {
        return None;
    }
    let (row, col, _, _) = parse_a1(s)?;
    Some((row, col))
}

/// `A1:C3` (or `A1`) → range.
pub(crate) fn parse_range(s: &str) -> Option<CellRangeLocal> {
    let (a, b) = match s.trim().split_once(':') {
        Some((a, b)) => (a, b),
        None => (s.trim(), s.trim()),
    };
    let (r0, c0) = parse_cell_ref(a)?;
    let (r1, c1) = parse_cell_ref(b)?;
    Some(CellRangeLocal {
        r0: r0.min(r1),
        c0: c0.min(c1),
        r1: r0.max(r1),
        c1: c0.max(c1),
    })
}

/// Appends a run, merging with the previous one when contiguous and equal.
fn push_run(runs: &mut Vec<DimRun>, start: u32, end: u32, size_hmm: u32, hidden: bool) {
    if let Some(last) = runs.last_mut() {
        if last.end.checked_add(1) == Some(start)
            && last.size_hmm == size_hmm
            && last.hidden == hidden
        {
            last.end = end;
            return;
        }
    }
    runs.push(DimRun {
        start,
        end,
        size_hmm,
        hidden,
    });
}

/// The value of an error cell (`t="e"`). Excel-only errors (`#SPILL!`,
/// `#CALC!`, `#GETTING_DATA`, …) have no OOo code and become `#VALUE!`
/// (`#N/A` for `#GETTING_DATA`).
fn error_value(s: &str) -> Value {
    let s = s.trim();
    match ErrorKind::parse(s) {
        Some(k) => Value::Error(k),
        None if s.eq_ignore_ascii_case("#GETTING_DATA") => Value::Error(ErrorKind::NA),
        None => Value::Error(ErrorKind::Value),
    }
}

/// What a `<c>` element contained.
#[derive(Default)]
struct RawCell {
    v: Option<String>,
    inline: Option<String>,
    f_text: Option<String>,
    f_type: Option<String>,
    f_ref: Option<String>,
    f_si: Option<u32>,
}

impl State<'_, '_> {
    fn pane(&mut self, e: &Elem) {
        let state = e.attr_local("state").unwrap_or("split");
        if state != "frozen" && state != "frozenSplit" {
            return;
        }
        let split = |k| {
            attr_f64(e, k)
                .filter(|v| *v >= 0.0)
                .map_or(0, |v| (v as u64).min(u64::from(MAX_ROWS)) as u32)
        };
        let cols = split("xSplit").min(MAX_COLS - 1);
        let rows = split("ySplit").min(MAX_ROWS - 1);
        if rows > 0 || cols > 0 {
            self.out.frozen = Some((rows, cols));
        }
    }

    fn col(&mut self, e: &Elem) {
        let (Some(min), Some(max)) = (attr_u32(e, "min"), attr_u32(e, "max")) else {
            return;
        };
        self.col_elements += 1;
        if self.col_elements > MAX_COL_ELEMENTS {
            self.budget.warn(format!(
                "Only the first {MAX_COL_ELEMENTS} column definitions of a sheet are read."
            ));
            return;
        }
        if min == 0 || min > max || min > MAX_COLS {
            return;
        }
        let (c0, c1) = (min - 1, max.min(MAX_COLS) - 1);
        let hidden = e.attr_local("hidden").is_some_and(parse_bool);
        let custom = e.attr_local("customWidth").is_some_and(parse_bool);
        // Excel caps widths at 255 characters.
        let width = attr_f64(e, "width").filter(|w| *w > 0.0 && *w <= 256.0);
        if custom || hidden {
            let hmm = match width {
                Some(w) if w > 0.0 => px_to_hmm(col_width_to_px(w)),
                _ => crate::DEFAULT_COL_WIDTH_HMM,
            };
            push_run(&mut self.out.col_widths, c0, c1, hmm, hidden);
        }
        if let Some(style) = attr_u32(e, "style").and_then(|s| self.ctx.styles.style_of(s)) {
            self.col_styles.push((c0, c1, style));
        }
    }

    fn row_start(&mut self, e: &Elem) {
        let row = match attr_u32(e, "r") {
            Some(r) if (1..=MAX_ROWS).contains(&r) => r - 1,
            // Out of the grid: cells below are skipped by position checks.
            Some(_) => MAX_ROWS,
            None => self.next_row,
        };
        self.row = row;
        self.next_row = row.saturating_add(1);
        self.next_col = 0;
        if row >= MAX_ROWS {
            return;
        }
        // A height without `customHeight` is an auto-fit result (Excel and
        // LibreOffice write one for every row); the app re-fits those.
        let hidden = e.attr_local("hidden").is_some_and(parse_bool);
        let custom = e.attr_local("customHeight").is_some_and(parse_bool);
        let ht = attr_f64(e, "ht")
            .filter(|h| *h >= 0.0 && *h <= 409.5 * 4.0)
            .map(pt_to_hmm);
        if hidden || (custom && ht.is_some()) {
            let size = ht
                .filter(|_| custom || hidden)
                .unwrap_or(self.default_row_hmm);
            push_run(&mut self.out.row_heights, row, row, size, hidden);
        }
    }

    fn read_raw<R: BufRead>(&mut self, xr: &mut XmlReader<R>) -> Result<RawCell, XlsxError> {
        let mut raw = RawCell::default();
        let base = xr.depth();
        loop {
            match xr.next()? {
                Ev::Start(c) => match c.local() {
                    "v" => raw.v = Some(xr.text_content(MAX_CELL_TEXT)?),
                    "is" => raw.inline = Some(read_string_item(xr, MAX_CELL_TEXT)?),
                    "f" => {
                        raw.f_type = c.attr_local("t").map(str::to_string);
                        raw.f_ref = c.attr_local("ref").map(str::to_string);
                        raw.f_si = attr_u32(&c, "si");
                        let text = xr.text_content(MAX_FORMULA_TEXT)?;
                        raw.f_text = Some(text);
                    }
                    _ => xr.skip()?,
                },
                Ev::End if xr.depth() < base => return Ok(raw),
                Ev::Eof => return Ok(raw),
                _ => {}
            }
        }
    }

    fn cell<R: BufRead>(&mut self, xr: &mut XmlReader<R>, e: &Elem) -> Result<(), XlsxError> {
        let pos = match e.attr_local("r") {
            Some(r) => parse_cell_ref(r),
            None if self.row < MAX_ROWS && self.next_col < MAX_COLS => {
                Some((self.row, self.next_col))
            }
            None => None,
        };
        let raw = self.read_raw(xr)?;
        let Some((row, col)) = pos else {
            return Ok(());
        };
        self.next_col = col.saturating_add(1);
        let xf = attr_u32(e, "s").unwrap_or(0);
        let style = self.ctx.styles.style_of(xf);
        let is_date = self.ctx.styles.is_date(xf);
        let t = e.attr_local("t").unwrap_or("n");
        let origin = CellAddr::new(self.ctx.sheet, row, col);
        let formula = self.formula(&raw, origin)?;
        let mut text_len = 0usize;
        let value = match t {
            "s" => raw
                .v
                .as_deref()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .and_then(|i| self.ctx.sst.get(i))
                .map(|s| {
                    text_len = s.len();
                    Value::Text(s.clone())
                }),
            "str" => raw.v.map(|v| {
                text_len = v.len();
                Value::text(v)
            }),
            "inlineStr" => raw.inline.or(raw.v).map(|v| {
                text_len = v.len();
                Value::text(v)
            }),
            "b" => raw.v.map(|v| Value::Bool(parse_bool(&v))),
            "e" => raw.v.map(|v| error_value(&v)),
            "d" => raw.v.as_deref().map(|v| {
                let epoch = if self.ctx.date1904 {
                    tally_types::DateEpoch::D1904
                } else {
                    tally_types::DateEpoch::D1899
                };
                iso_to_serial(v, epoch).map_or(Value::Error(ErrorKind::Value), Value::Number)
            }),
            _ => raw.v.as_deref().and_then(|v| {
                let n = v.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
                Some(Value::Number(if is_date {
                    excel_serial_to_tally(n, self.ctx.date1904)
                } else {
                    n
                }))
            }),
        };
        let content = match formula {
            Some(FText::Own(text)) => {
                text_len += text.len();
                CellContentData::Formula {
                    text: Arc::from(text),
                    cached: value,
                }
            }
            // The anchor: the group's text is counted once, here.
            Some(FText::Shared {
                text,
                row: r0,
                col: c0,
            }) if (r0, c0) == (row, col) => {
                text_len += text.len();
                CellContentData::Formula {
                    text,
                    cached: value,
                }
            }
            Some(FText::Shared {
                text,
                row: r0,
                col: c0,
            }) => CellContentData::SharedFormula {
                text,
                anchor_row: r0,
                anchor_col: c0,
                cached: value,
            },
            None => match value {
                Some(v) => CellContentData::Value(v),
                None if style.is_some() => CellContentData::Value(Value::Empty),
                None => return Ok(()),
            },
        };
        self.budget.add_cell(text_len, self.ctx.limits)?;
        if self.last_pos.is_some_and(|p| p > (row, col)) {
            self.sorted = false;
        }
        self.last_pos = Some((row, col));
        self.out.cells.push(CellData {
            row,
            col,
            content,
            style,
        });
        Ok(())
    }

    /// The model formula text of a cell, if it has one.
    fn formula(&mut self, raw: &RawCell, origin: CellAddr) -> Result<Option<FText>, XlsxError> {
        let ftype = raw.f_type.as_deref().unwrap_or("normal");
        let text = raw.f_text.as_deref().unwrap_or("");
        match ftype {
            "shared" => {
                let Some(si) = raw.f_si else {
                    return Ok(self.plain(text, origin).map(FText::Own));
                };
                if !text.trim().is_empty() {
                    // The group's anchor (it carries `ref` and the text).
                    let entry = match parse_excel(text, origin, self.ctx.resolver) {
                        Ok(f) => Shared::Parsed {
                            text: Arc::from(print_of(&f, origin, self.ctx.resolver)),
                            row: origin.row,
                            col: origin.col,
                        },
                        Err(_) => Shared::Failed,
                    };
                    let failed = matches!(entry, Shared::Failed);
                    self.shared.insert(si, entry);
                    if failed {
                        return Ok(Some(FText::Own(format!("of:={text}"))));
                    }
                }
                Ok(match self.shared.get(&si) {
                    // The anchor's formula, moved to this cell by the
                    // engine (one shared text, one parse per group).
                    Some(Shared::Parsed { text, row, col }) => Some(FText::Shared {
                        text: text.clone(),
                        row: *row,
                        col: *col,
                    }),
                    // A member before its anchor, or a failed anchor: keep
                    // the cached value only.
                    _ => None,
                })
            }
            "array" => {
                let f = self.plain(text, origin);
                if f.is_some() {
                    if let Some(r) = raw.f_ref.as_deref().and_then(parse_range) {
                        if r.r0 == origin.row && r.c0 == origin.col {
                            // Every covered cell becomes an array part in
                            // the engine (review 2, HIGH-2).
                            let area = (u64::from(r.r1 - r.r0) + 1) * (u64::from(r.c1 - r.c0) + 1);
                            self.budget.add_cells(area, self.ctx.limits)?;
                            self.out.array_formulas.push(r);
                        }
                    }
                }
                Ok(f.map(FText::Own))
            }
            // What-if data tables: keep the values Excel computed.
            "dataTable" => Ok(None),
            _ => Ok(self.plain(text, origin).map(FText::Own)),
        }
    }

    fn plain(&self, text: &str, origin: CellAddr) -> Option<String> {
        if text.trim().is_empty() {
            return None;
        }
        Some(excel_to_of(text, origin, self.ctx.resolver).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(
            parse_range("B3:A1"),
            Some(CellRangeLocal {
                r0: 0,
                c0: 0,
                r1: 2,
                c1: 1
            })
        );
        assert_eq!(parse_range("A1").map(|r| (r.r1, r.c1)), Some((0, 0)));
        assert_eq!(parse_range("XFE1:A1"), None);
        assert_eq!(parse_range(&"A".repeat(100)), None);
    }

    #[test]
    fn errors() {
        assert_eq!(error_value("#N/A"), Value::Error(ErrorKind::NA));
        assert_eq!(error_value("#DIV/0!"), Value::Error(ErrorKind::Div0));
        assert_eq!(error_value("#SPILL!"), Value::Error(ErrorKind::Value));
        assert_eq!(error_value("#GETTING_DATA"), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn runs_merge() {
        let mut v = Vec::new();
        push_run(&mut v, 1, 1, 500, false);
        push_run(&mut v, 2, 2, 500, false);
        push_run(&mut v, 3, 3, 500, true);
        push_run(&mut v, 5, 5, 500, true);
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].start, v[0].end), (1, 2));
    }
}
