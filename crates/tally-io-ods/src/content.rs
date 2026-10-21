//! Streaming reader for `content.xml`.
//!
//! Rows and cells are parsed as run-length *runs* first and only expanded
//! into [`CellData`] after the materialized cell count has been checked
//! against [`Limits::max_cells`], so repeat-count bombs fail fast instead of
//! allocating (§5.4 requirement 3).

use std::io::BufRead;
use std::sync::Arc;

use tally_model::{
    AutoFilterData, CellContentData, CellData, CellRangeLocal, DimRun, FilterColumnData,
    FilterConditionData, FilterOpData, NameData, SheetData, StandardFilterData, StyleRun,
    SubtotalsData,
};
use tally_types::{
    CalcSettings, CriteriaMode, DateEpoch, ErrorKind, IterSettings, Value, MAX_COLS, MAX_ROWS,
};

use crate::styles::{Origin, StyleSheet};
use crate::units::{date_value_to_serial, duration_to_days, epoch_from_null_date};
use crate::xml::{Elem, Ev, XmlReader};
use crate::{Limits, OdsError, DEFAULT_COL_WIDTH_HMM};

/// Maximum bytes of text in one cell.
const MAX_CELL_TEXT: usize = 16 * 1024 * 1024;
/// Maximum length of a `table:formula` attribute in characters (the engine's
/// parser limit; a longer formula could never be parsed anyway).
pub(crate) const MAX_FORMULA_LEN: usize = 8192;
/// Styled-but-empty runs larger than this (rows × columns) are kept as
/// [`StyleRun`]s (one per column) instead of being materialized as cells.
const MAX_STYLED_EMPTY_RUN: u64 = 4096;
/// Maximum named expressions read from one document.
pub(crate) const MAX_NAMES: usize = 100_000;
/// Maximum length of a named expression's text.
const MAX_NAME_EXPR_LEN: usize = 8192;

/// What content.xml yields.
#[derive(Debug, Default)]
pub(crate) struct Content {
    pub sheets: Vec<SheetData>,
    pub calc: Option<CalcSettings>,
    pub is_spreadsheet: bool,
    pub names: Vec<NameData>,
    /// `table:structure-protected` on `office:spreadsheet`.
    pub structure_protection: Option<tally_model::SheetProtectionData>,
    /// Per sheet: what `read.rs` resolves against styles.xml.
    pub extras: Vec<SheetExtra>,
    /// What was dropped to stay within limits.
    pub warnings: Vec<String>,
    /// Chart and picture frames with their anchor cells (W5-1; the
    /// objects themselves are loaded by `read.rs` from the package).
    pub frames: Vec<crate::frame::PlacedFrame>,
}

/// Most merged areas read per sheet; later ones are dropped with a warning
/// (security review 2, HIGH-1).
pub const MAX_MERGES_PER_SHEET: usize = 100_000;

/// Sheet state resolved after content.xml (page styles live in styles.xml).
#[derive(Debug, Default)]
pub(crate) struct SheetExtra {
    /// `table:scenario` of a scenario sheet (folded into the base sheet).
    pub scenario: Option<tally_model::ScenarioData>,
    pub master_page: Option<String>,
    pub print_range: Option<CellRangeLocal>,
    pub repeat_rows: Option<(u32, u32)>,
    pub repeat_cols: Option<(u32, u32)>,
}

struct CellParsed {
    repeat: u32,
    content: CellContentData,
    style: Option<String>,
    span: (u32, u32),
    /// `table:number-matrix-columns/rows-spanned` of an array formula.
    matrix: Option<(u32, u32)>,
    /// `table:content-validation-name`.
    validation: Option<String>,
    /// `office:annotation`.
    note: Option<tally_model::NoteData>,
    /// The first `text:a` target.
    link: Option<String>,
    /// `draw:frame`s anchored in the cell (W5-1).
    frames: Vec<crate::frame::RawFrame>,
}

struct Run {
    col: u32,
    len: u32,
    cell: CellParsed,
}

struct Parser<'a> {
    styles: &'a mut StyleSheet,
    limits: &'a Limits,
    cells_total: u64,
    /// Text and formula bytes of every materialized cell so far (a shared
    /// `Arc` counts once per cell, so this tracks the dense expansion).
    text_bytes: u64,
    sheets: usize,
    epoch: DateEpoch,
    names: Vec<NameData>,
    /// Per sheet: rows read with `table:visibility="filter"` (as dimension
    /// runs, hidden), resolved once the database ranges are known.
    filter_rows: Vec<Vec<DimRun>>,
    /// `table:content-validations` by name.
    validations: std::collections::HashMap<String, tally_model::ValidationData>,
    /// Cells of each validation on the current sheet.
    val_rects: std::collections::HashMap<String, Vec<CellRangeLocal>>,
    /// What was dropped to stay within limits (one entry per kind).
    warnings: Vec<String>,
    /// Frames found so far (W5-1).
    frames: Vec<crate::frame::PlacedFrame>,
}

/// Most frames kept per document.
const MAX_FRAMES: usize = 20_000;

/// Bytes of text a cell's content carries (formula text plus text value).
fn content_bytes(c: &CellContentData) -> u64 {
    let value_bytes = |v: &Value| match v {
        Value::Text(t) => t.len() as u64,
        _ => 0,
    };
    match c {
        CellContentData::Value(v) => value_bytes(v),
        CellContentData::Formula { text, cached } => {
            text.len() as u64 + cached.as_ref().map_or(0, value_bytes)
        }
        // The text is the anchor's and counted there.
        CellContentData::SharedFormula { cached, .. } => cached.as_ref().map_or(0, value_bytes),
    }
}

/// Parses `content.xml`, adding its automatic styles to `styles`.
pub(crate) fn parse_content<R: BufRead>(
    xml: &mut XmlReader<R>,
    styles: &mut StyleSheet,
    limits: &Limits,
) -> Result<Content, OdsError> {
    let mut out = Content::default();
    let mut p = Parser {
        styles,
        limits,
        cells_total: 0,
        text_bytes: 0,
        sheets: 0,
        epoch: DateEpoch::default(),
        names: Vec::new(),
        filter_rows: Vec::new(),
        validations: std::collections::HashMap::new(),
        val_rects: std::collections::HashMap::new(),
        warnings: Vec::new(),
        frames: Vec::new(),
    };
    loop {
        match xml.next()? {
            Ev::Start(e) => match e.name.as_str() {
                "office:document-content" | "office:body" => {}
                "office:font-face-decls" => p.styles.parse_font_faces(xml)?,
                "office:automatic-styles" => p.styles.parse_block(xml, Origin::Content)?,
                "office:spreadsheet" => {
                    out.is_spreadsheet = true;
                    out.structure_protection =
                        crate::v03::parse_protection_attrs(&e, "table:structure-protected");
                }
                "table:content-validations" if out.is_spreadsheet => {
                    p.validations = crate::v03::parse_validations(xml)?;
                }
                "table:calculation-settings" if out.is_spreadsheet => {
                    let calc = parse_calc_settings(xml, &e)?;
                    p.epoch = calc.date_epoch;
                    out.calc = Some(calc);
                }
                "table:table" if out.is_spreadsheet => {
                    p.sheets += 1;
                    if p.sheets > p.limits.max_sheets {
                        return Err(OdsError::LimitExceeded {
                            what: "sheets",
                            limit: p.limits.max_sheets as u64,
                        });
                    }
                    p.filter_rows.push(Vec::new());
                    let (sheet, mut extra) = p.parse_table(xml, &e)?;
                    // A scenario sheet folds into the sheet before it.
                    if let (Some(mut sc), Some(base)) =
                        (extra.scenario.take(), out.sheets.last_mut())
                    {
                        sc.name = sheet.name.clone();
                        sc.cells = sheet
                            .cells
                            .into_iter()
                            .filter(|c| {
                                sc.ranges.iter().any(|r| {
                                    r.r0 <= c.row && c.row <= r.r1 && r.c0 <= c.col && c.col <= r.c1
                                })
                            })
                            .collect();
                        if base.scenarios.len() < 1_000 {
                            base.scenarios.push(sc);
                        }
                        p.filter_rows.pop();
                        p.sheets -= 1;
                        continue;
                    }
                    out.sheets.push(sheet);
                    out.extras.push(extra);
                }
                "table:named-expressions" if out.is_spreadsheet => {
                    p.parse_named_expressions(xml, None)?;
                }
                "table:database-ranges" if out.is_spreadsheet => {
                    parse_database_ranges(xml, &mut out.sheets)?;
                }
                "table:data-pilot-tables" if out.is_spreadsheet => {
                    crate::tools::parse_pivot_tables(xml, &mut out.sheets)?;
                }
                _ => xml.skip()?,
            },
            Ev::Eof => {
                attach_filter_rows(&mut out.sheets, std::mem::take(&mut p.filter_rows));
                out.names = std::mem::take(&mut p.names);
                out.warnings = std::mem::take(&mut p.warnings);
                out.frames = std::mem::take(&mut p.frames);
                return Ok(out);
            }
            _ => {}
        }
    }
}

fn parse_count(e: &Elem, key: &str, max: u32) -> u32 {
    e.attr(key)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(1, |n| n.clamp(1, u64::from(max)) as u32)
}

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

impl Parser<'_> {
    fn parse_table<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
    ) -> Result<(SheetData, SheetExtra), OdsError> {
        let mut sheet = SheetData {
            name: e.attr("table:name").unwrap_or_default().to_string(),
            ..SheetData::default()
        };
        let mut extra = SheetExtra::default();
        if let Some(ts) = e
            .attr("table:style-name")
            .and_then(|n| self.styles.table_style(n))
        {
            sheet.hidden = ts.hidden;
            sheet.tab_color = ts.tab_color;
            extra.master_page.clone_from(&ts.master_page);
        }
        sheet.protection = crate::v03::parse_protection_attrs(e, "table:protected");
        if let Some(pr) = e.attr("table:print-ranges") {
            extra.print_range = crate::v03::parse_range_list(pr).first().copied();
        }
        self.val_rects.clear();
        let mut header_rows: Option<(u32, u32)> = None;
        let mut header_cols: Option<(u32, u32)> = None;
        let mut col: u32 = 0;
        let mut col_defaults: Vec<(u32, u32, String)> = Vec::new();
        let mut col_runs_done = false;
        let mut row: u32 = 0;
        let mut open_groups: Vec<crate::tools::OpenGroup> = Vec::new();
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => match c.name.as_str() {
                    "table:table-column" => {
                        let rep = parse_count(&c, "table:number-columns-repeated", MAX_COLS);
                        if col < MAX_COLS {
                            let end = (u64::from(col) + u64::from(rep) - 1)
                                .min(u64::from(MAX_COLS - 1))
                                as u32;
                            let width = c
                                .attr("table:style-name")
                                .and_then(|n| self.styles.column_width(n))
                                .unwrap_or(DEFAULT_COL_WIDTH_HMM);
                            let hidden = c.attr("table:visibility").is_some_and(|v| v != "visible");
                            if col > 0
                                && c.attr("table:style-name")
                                    .is_some_and(|n| self.styles.column_break(n))
                                && sheet.col_breaks.len() < 10_000
                            {
                                sheet.col_breaks.push(col);
                            }
                            if width != DEFAULT_COL_WIDTH_HMM || hidden {
                                push_run(&mut sheet.col_widths, col, end, width, hidden);
                            }
                            if let Some(ds) = c
                                .attr("table:default-cell-style-name")
                                .filter(|n| *n != "Default")
                            {
                                col_defaults.push((col, end, ds.to_string()));
                            }
                        }
                        col = col.saturating_add(rep);
                        xml.skip()?;
                    }
                    "table:table-row" => {
                        if !col_runs_done {
                            // A column default cell style formats the whole
                            // column, empty cells included. Emitted before
                            // the row runs, which override it.
                            col_runs_done = true;
                            self.push_column_defaults(&mut sheet, &col_defaults)?;
                        }
                        self.parse_row(xml, &c, &mut row, &mut sheet, &col_defaults)?;
                    }
                    "table:named-expressions" => {
                        let tab = self.sheets.saturating_sub(1) as u32;
                        self.parse_named_expressions(xml, Some(tab))?;
                    }
                    "loext:table-protection" => {
                        if let Some(p) = sheet.protection.as_mut() {
                            crate::v03::parse_protection_options(&c, p);
                        }
                        xml.skip()?;
                    }
                    "calcext:conditional-formats" => {
                        let styles = &*self.styles;
                        sheet.conditional_formats =
                            crate::v03::parse_cond_formats(xml, &mut |n| styles.cf_style(n))?;
                    }
                    "table:table-header-rows" => header_rows = Some((xml.depth(), row)),
                    "table:table-header-columns" => header_cols = Some((xml.depth(), col)),
                    "table:table-columns" | "table:table-rows" => {}
                    "table:table-column-group" | "table:table-row-group" => {
                        // An outline group: from the current index to the
                        // end of the element (closed in the `Ev::End` arm).
                        let rows = c.name == "table:table-row-group";
                        if open_groups.len() < 64 {
                            open_groups.push(crate::tools::OpenGroup {
                                rows,
                                start: if rows { row } else { col },
                                depth: xml.depth(),
                                collapsed: c.attr("table:display") == Some("false"),
                            });
                        }
                    }
                    "table:scenario" => {
                        extra.scenario = Some(crate::tools::scenario_from_attrs(&c));
                        xml.skip()?;
                    }
                    "table:table-source" => {
                        // A sheet linked to another file: imported as the
                        // values the file holds; the link is disabled by
                        // policy (W5-3, `tally_engine::external`).
                        let href = c.attr("xlink:href").map(str::to_string);
                        self.warnings.push(format!(
                            "tally:external-link:linkedSheet{}",
                            href.map(|h| format!(":{h}")).unwrap_or_default()
                        ));
                        xml.skip()?;
                    }
                    "table:shapes" => {
                        // Page-anchored frames (charts, pictures): converted
                        // to cell anchors once the sheet's dimensions are
                        // known (`frame::attach_objects`).
                        let tab = self.sheets.saturating_sub(1) as u32;
                        let sd = xml.depth();
                        loop {
                            match xml.next()? {
                                Ev::Start(f) if f.name == "draw:frame" => {
                                    let frame = crate::frame::parse_frame(xml, &f, true)?;
                                    if self.frames.len() < MAX_FRAMES {
                                        self.frames.push(crate::frame::PlacedFrame {
                                            tab,
                                            row: 0,
                                            col: 0,
                                            frame,
                                        });
                                    }
                                }
                                Ev::Start(_) => xml.skip()?,
                                Ev::End if xml.depth() < sd => break,
                                Ev::Eof => break,
                                _ => {}
                            }
                        }
                    }
                    _ => xml.skip()?,
                },
                Ev::End if xml.depth() < d => break,
                Ev::End => {
                    while open_groups.last().is_some_and(|g| xml.depth() < g.depth) {
                        let Some(g) = open_groups.pop() else { break };
                        let end = if g.rows { row } else { col }.saturating_sub(1);
                        let still = open_groups.iter().filter(|x| x.rows == g.rows).count();
                        crate::tools::close_group(&mut sheet, &g, end, still);
                    }
                    if let Some((hd, start)) = header_rows {
                        if xml.depth() < hd {
                            if row > start {
                                extra.repeat_rows = Some((start, row - 1));
                            }
                            header_rows = None;
                        }
                    }
                    if let Some((hd, start)) = header_cols {
                        if xml.depth() < hd {
                            if col > start {
                                extra.repeat_cols = Some((start, (col - 1).min(MAX_COLS - 1)));
                            }
                            header_cols = None;
                        }
                    }
                }
                Ev::Eof => break,
                _ => {}
            }
        }
        if !col_runs_done {
            self.push_column_defaults(&mut sheet, &col_defaults)?;
        }
        // Cells name their validation; collect each one's ranges.
        let mut names: Vec<String> = self.val_rects.keys().cloned().collect();
        names.sort();
        for name in names {
            let (Some(v), Some(rects)) =
                (self.validations.get(&name), self.val_rects.remove(&name))
            else {
                continue;
            };
            let mut v = v.clone();
            v.ranges = crate::v03::merge_rects(rects);
            sheet.validations.push(v);
        }
        sheet.notes.sort_by_key(|n| (n.row, n.col));
        sheet.links.sort_by_key(|l| (l.row, l.col));
        sheet.row_breaks.sort_unstable();
        sheet.row_breaks.dedup();
        Ok((sheet, extra))
    }

    /// Records a warning once per distinct text.
    fn warn(&mut self, w: String) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Counts `n` style runs against the materialized-cell limit.
    fn count_runs(&mut self, n: u64) -> Result<(), OdsError> {
        self.cells_total = self.cells_total.saturating_add(n);
        if self.cells_total > self.limits.max_cells {
            return Err(OdsError::LimitExceeded {
                what: "materialized cells",
                limit: self.limits.max_cells,
            });
        }
        Ok(())
    }

    fn push_column_defaults(
        &mut self,
        sheet: &mut SheetData,
        col_defaults: &[(u32, u32, String)],
    ) -> Result<(), OdsError> {
        for (c0, c1, name) in col_defaults {
            let Some(style) = self.styles.resolve(name) else {
                continue;
            };
            self.count_runs(u64::from(c1 - c0) + 1)?;
            for col in *c0..=*c1 {
                sheet.style_runs.push(StyleRun {
                    col,
                    start_row: 0,
                    end_row: MAX_ROWS - 1,
                    style,
                });
            }
        }
        Ok(())
    }

    fn parse_row<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
        row: &mut u32,
        sheet: &mut SheetData,
        col_defaults: &[(u32, u32, String)],
    ) -> Result<(), OdsError> {
        let rep = parse_count(e, "table:number-rows-repeated", MAX_ROWS);
        let row_default = e
            .attr("table:default-cell-style-name")
            .filter(|n| *n != "Default")
            .map(str::to_string);
        let height = e
            .attr("table:style-name")
            .and_then(|n| self.styles.row_style(n))
            .and_then(|rs| if rs.optimal { None } else { rs.height_hmm });
        if *row > 0
            && *row < MAX_ROWS
            && sheet.row_breaks.len() < 10_000
            && e.attr("table:style-name")
                .and_then(|n| self.styles.row_style(n))
                .is_some_and(|rs| rs.break_before)
        {
            sheet.row_breaks.push(*row);
        }
        let visibility = e.attr("table:visibility");
        let filtered = visibility == Some("filter");
        let hidden = !filtered && visibility.is_some_and(|v| v != "visible");

        let mut runs: Vec<Run> = Vec::new();
        let mut col: u32 = 0;
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => match c.name.as_str() {
                    "table:table-cell" | "table:covered-table-cell" => {
                        let mut cell = self.parse_cell(xml, &c)?;
                        if c.name == "table:covered-table-cell" {
                            // Content hidden under a merge is not kept.
                            cell.content = CellContentData::Value(Value::Empty);
                            cell.span = (1, 1);
                            cell.matrix = None;
                        }
                        let crep = cell.repeat;
                        if col < MAX_COLS {
                            let len = crep.min(MAX_COLS - col);
                            let keep =
                                !matches!(cell.content, CellContentData::Value(Value::Empty))
                                    || cell.style.is_some()
                                    || cell.span != (1, 1)
                                    || cell.validation.is_some()
                                    || cell.note.is_some()
                                    || cell.link.is_some()
                                    || !cell.frames.is_empty();
                            if keep {
                                runs.push(Run { col, len, cell });
                            }
                        }
                        col = col.saturating_add(crep);
                    }
                    _ => xml.skip()?,
                },
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }

        if *row >= MAX_ROWS {
            return Ok(());
        }
        let rep = rep.min(MAX_ROWS - *row);
        let last = *row + (rep - 1);
        if filtered {
            let h = height.unwrap_or(crate::DEFAULT_ROW_HEIGHT_HMM);
            if let Some(v) = self.filter_rows.last_mut() {
                push_run(v, *row, last, h, true);
            }
        } else if height.is_some() || hidden {
            let h = height.unwrap_or(crate::DEFAULT_ROW_HEIGHT_HMM);
            push_run(&mut sheet.row_heights, *row, last, h, hidden);
        }

        // Validations, notes and links of the runs.
        let rep_end = (*row).saturating_add(rep - 1);
        let tab = self.sheets.saturating_sub(1) as u32;
        for run in &mut runs {
            let c1 = run.col.saturating_add(run.len - 1);
            for frame in run.cell.frames.drain(..) {
                if self.frames.len() < MAX_FRAMES {
                    self.frames.push(crate::frame::PlacedFrame {
                        tab,
                        row: *row,
                        col: run.col,
                        frame,
                    });
                }
            }
            if let Some(v) = run.cell.validation.take() {
                let rects = self.val_rects.entry(v).or_default();
                if rects.len() < 1_000_000 {
                    rects.push(CellRangeLocal {
                        r0: *row,
                        c0: run.col,
                        r1: rep_end,
                        c1,
                    });
                }
            }
            let cells = u64::from(run.len) * u64::from(rep);
            if cells <= 10_000 {
                if let Some(n) = run.cell.note.take() {
                    for r in *row..=rep_end {
                        for c in run.col..=c1 {
                            if sheet.notes.len() < crate::v03::MAX_ITEMS {
                                sheet.notes.push(tally_model::NoteData {
                                    row: r,
                                    col: c,
                                    ..n.clone()
                                });
                            }
                        }
                    }
                }
                if let Some(url) = run.cell.link.take() {
                    let url: std::sync::Arc<str> = url.into();
                    for r in *row..=rep_end {
                        for c in run.col..=c1 {
                            if sheet.links.len() < crate::v03::MAX_ITEMS {
                                sheet.links.push(tally_model::LinkData {
                                    row: r,
                                    col: c,
                                    url: url.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        // Resolve styles and decide which runs materialize.
        let mut kept: Vec<(u32, u32, CellContentData, Option<u32>)> = Vec::new();
        let mut per_row: u64 = 0;
        let mut per_row_bytes: u64 = 0;
        for run in runs {
            let non_empty = !matches!(run.cell.content, CellContentData::Value(Value::Empty));
            if run.cell.span != (1, 1)
                && rep == 1
                && run.len == 1
                && sheet.merges.len() >= MAX_MERGES_PER_SHEET
            {
                self.warn(format!(
                    "Only the first {MAX_MERGES_PER_SHEET} merged areas of a sheet are kept."
                ));
            } else if run.cell.span != (1, 1) && rep == 1 && run.len == 1 {
                // Merges count against the cell cap like cells do.
                self.cells_total += 1;
                if self.cells_total > self.limits.max_cells {
                    return Err(OdsError::LimitExceeded {
                        what: "materialized cells",
                        limit: self.limits.max_cells,
                    });
                }
                let (cs, rs) = run.cell.span;
                sheet.merges.push(CellRangeLocal {
                    r0: *row,
                    c0: run.col,
                    r1: (*row).saturating_add(rs - 1).min(MAX_ROWS - 1),
                    c1: run.col.saturating_add(cs - 1).min(MAX_COLS - 1),
                });
            }
            if let (Some((cs, rs)), CellContentData::Formula { .. }, 1, 1) =
                (run.cell.matrix, &run.cell.content, rep, run.len)
            {
                // Every covered cell becomes an array part in the engine,
                // so the block's whole area counts (review 2, HIGH-2: 200
                // blocks of 1M rows in a 1 KB file).
                let rows = u64::from(rs.min(MAX_ROWS - (*row).min(MAX_ROWS - 1)));
                let cols = u64::from(cs.min(MAX_COLS - run.col.min(MAX_COLS - 1)));
                self.cells_total = self.cells_total.saturating_add(rows * cols);
                if self.cells_total > self.limits.max_cells {
                    return Err(OdsError::LimitExceeded {
                        what: "materialized cells",
                        limit: self.limits.max_cells,
                    });
                }
                sheet.array_formulas.push(CellRangeLocal {
                    r0: *row,
                    c0: run.col,
                    r1: (*row).saturating_add(rs - 1).min(MAX_ROWS - 1),
                    c1: run.col.saturating_add(cs - 1).min(MAX_COLS - 1),
                });
            }
            let name = run.cell.style.clone().or_else(|| {
                if non_empty {
                    row_default.clone().or_else(|| {
                        col_defaults
                            .iter()
                            .find(|(s, e, _)| (*s..=*e).contains(&run.col))
                            .map(|(_, _, n)| n.clone())
                    })
                } else {
                    None
                }
            });
            let style = name.and_then(|n| self.styles.resolve(&n));
            if !non_empty {
                let Some(st) = style else {
                    continue;
                };
                if u64::from(run.len) * u64::from(rep) > MAX_STYLED_EMPTY_RUN {
                    // Formatting of a large blank area: keep it as runs.
                    self.count_runs(u64::from(run.len))?;
                    for col in run.col..run.col + run.len {
                        sheet.style_runs.push(StyleRun {
                            col,
                            start_row: *row,
                            end_row: last,
                            style: st,
                        });
                    }
                    continue;
                }
            }
            per_row += u64::from(run.len);
            per_row_bytes = per_row_bytes.saturating_add(
                u64::from(run.len).saturating_mul(content_bytes(&run.cell.content)),
            );
            kept.push((run.col, run.len, run.cell.content, style));
        }
        if per_row == 0 {
            *row = row.saturating_add(rep);
            return Ok(());
        }
        let eff = rep.min(self.limits.max_repeat.max(1));
        let total = per_row.saturating_mul(u64::from(eff));
        if self.cells_total.saturating_add(total) > self.limits.max_cells {
            return Err(OdsError::LimitExceeded {
                what: "materialized cells",
                limit: self.limits.max_cells,
            });
        }
        let bytes = per_row_bytes.saturating_mul(u64::from(eff));
        if self.text_bytes.saturating_add(bytes) > self.limits.max_text_bytes {
            return Err(OdsError::LimitExceeded {
                what: "materialized cell text bytes",
                limit: self.limits.max_text_bytes,
            });
        }
        self.cells_total += total;
        self.text_bytes += bytes;
        sheet.cells.reserve(total.min(1 << 20) as usize);
        for r in 0..eff {
            for (c0, len, content, style) in &kept {
                for k in 0..*len {
                    sheet.cells.push(CellData {
                        row: *row + r,
                        col: c0 + k,
                        content: content.clone(),
                        style: *style,
                    });
                }
            }
        }
        *row = row.saturating_add(rep);
        Ok(())
    }

    fn parse_cell<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        e: &Elem,
    ) -> Result<CellParsed, OdsError> {
        let repeat = parse_count(e, "table:number-columns-repeated", MAX_COLS);
        let span = (
            parse_count(e, "table:number-columns-spanned", MAX_COLS),
            parse_count(e, "table:number-rows-spanned", MAX_ROWS),
        );
        let matrix = (e.attr("table:number-matrix-columns-spanned").is_some()
            || e.attr("table:number-matrix-rows-spanned").is_some())
        .then(|| {
            (
                parse_count(e, "table:number-matrix-columns-spanned", MAX_COLS),
                parse_count(e, "table:number-matrix-rows-spanned", MAX_ROWS),
            )
        });
        let mut paragraphs: Vec<String> = Vec::new();
        let mut budget = MAX_CELL_TEXT;
        let mut note = None;
        let mut link: Option<String> = None;
        let mut frames: Vec<crate::frame::RawFrame> = Vec::new();
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => {
                    if c.name == "text:p" {
                        let p = parse_paragraph(xml, &mut budget, &mut link)?;
                        paragraphs.push(p);
                    } else if c.name == "office:annotation" {
                        note = Some(crate::v03::parse_annotation(xml, &c)?);
                    } else if c.name == "draw:frame" {
                        // A chart or picture anchored to this cell; the
                        // object is loaded later from the package.
                        let frame = crate::frame::parse_frame(xml, &c, false)?;
                        if frames.len() < 4096 {
                            frames.push(frame);
                        }
                    } else {
                        xml.skip()?;
                    }
                }
                Ev::End if xml.depth() < d => break,
                Ev::Eof => break,
                _ => {}
            }
        }
        // A link on an otherwise empty cell (`<text:p><text:a …/></text:p>`).
        if link.is_some() && paragraphs.iter().all(String::is_empty) {
            paragraphs.clear();
        }
        let text = (!paragraphs.is_empty()).then(|| paragraphs.join("\n"));
        let formula: Option<Arc<str>> = match e.attr("table:formula") {
            Some(f) if f.chars().count() > MAX_FORMULA_LEN => {
                return Err(OdsError::LimitExceeded {
                    what: "formula length",
                    limit: MAX_FORMULA_LEN as u64,
                });
            }
            Some(f) => Some(Arc::from(f)),
            None => None,
        };
        if e.attr("office:string-value")
            .is_some_and(|s| s.len() > MAX_CELL_TEXT)
        {
            return Err(OdsError::LimitExceeded {
                what: "cell text bytes",
                limit: MAX_CELL_TEXT as u64,
            });
        }
        let vt = e.attr("office:value-type");
        let number = |key: &str| e.attr(key).and_then(|v| v.trim().parse::<f64>().ok());
        let mut value = match vt {
            Some("float" | "percentage" | "currency") => match number("office:value") {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("date") => match e
                .attr("office:date-value")
                .and_then(|v| date_value_to_serial(v, self.epoch))
                .or_else(|| number("office:value"))
            {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("time") => match e
                .attr("office:time-value")
                .and_then(duration_to_days)
                .or_else(|| number("office:value"))
            {
                Some(n) => Value::Number(n),
                None => text.as_deref().map_or(Value::Empty, Value::text),
            },
            Some("boolean") => match e.attr("office:boolean-value").map(str::trim) {
                Some("true" | "1") => Value::Bool(true),
                Some("false" | "0") => Value::Bool(false),
                _ => number("office:value").map_or(Value::Empty, |n| Value::Bool(n != 0.0)),
            },
            Some("string") => match e.attr("office:string-value") {
                Some(s) => Value::text(s),
                None => Value::text(text.as_deref().unwrap_or_default()),
            },
            _ => text.as_deref().map_or(Value::Empty, Value::text),
        };
        let display = text
            .as_deref()
            .or(e.attr("office:string-value"))
            .unwrap_or_default();
        let explicit_error = e.attr("calcext:value-type") == Some("error");
        let error = ErrorKind::parse(display.trim());
        let error_ok =
            explicit_error || (formula.is_some() && matches!(vt, None | Some("string" | "float")));
        if let Some(err) = error.filter(|_| error_ok) {
            value = Value::Error(err);
        }
        // NaN and infinities (`office:value="NaN"`, `1e309`) never reach the
        // model: AOO cannot store them, and they break `#NUM!` invariants.
        if matches!(value, Value::Number(n) if !n.is_finite()) {
            value = Value::Empty;
        }
        let content = match formula {
            Some(text) => CellContentData::Formula {
                text,
                cached: (!value.is_empty()).then_some(value),
            },
            None => CellContentData::Value(value),
        };
        Ok(CellParsed {
            repeat,
            content,
            style: e
                .attr("table:style-name")
                .filter(|n| *n != "Default")
                .map(str::to_string),
            span,
            matrix,
            validation: e
                .attr("table:content-validation-name")
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            note,
            link,
            frames,
        })
    }
}

/// Collects the text of one `text:p`, applying ODF white-space collapsing and
/// `text:s` / `text:tab` / `text:line-break`. Annotations, notes and drawing
/// objects are skipped; other inline elements (spans, links, fields) are
/// transparent.
fn parse_paragraph<R: BufRead>(
    xml: &mut XmlReader<R>,
    budget: &mut usize,
    link: &mut Option<String>,
) -> Result<String, OdsError> {
    let mut out = String::new();
    let mut last_space = true;
    let d = xml.depth();
    let over = || OdsError::LimitExceeded {
        what: "cell text bytes",
        limit: MAX_CELL_TEXT as u64,
    };
    loop {
        match xml.next()? {
            Ev::Text(t) => {
                if t.len() > *budget {
                    return Err(over());
                }
                let before = out.len();
                for ch in t.chars() {
                    if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                        if !last_space {
                            out.push(' ');
                            last_space = true;
                        }
                    } else {
                        out.push(ch);
                        last_space = false;
                    }
                }
                *budget -= out.len() - before;
            }
            Ev::Start(e) => match e.name.as_str() {
                "text:s" => {
                    let n = e
                        .attr("text:c")
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .unwrap_or(1)
                        .max(1);
                    if n > *budget as u64 {
                        return Err(over());
                    }
                    *budget -= n as usize;
                    out.extend(std::iter::repeat_n(' ', n as usize));
                    last_space = false;
                    xml.skip()?;
                }
                "text:tab" | "text:line-break" => {
                    if *budget == 0 {
                        return Err(over());
                    }
                    *budget -= 1;
                    out.push(if e.name == "text:tab" { '\t' } else { '\n' });
                    last_space = false;
                    xml.skip()?;
                }
                "text:note"
                | "office:annotation"
                | "text:ruby-text"
                | "text:bookmark-start"
                | "text:bookmark-end" => xml.skip()?,
                n if n.starts_with("draw:") || n.starts_with("svg:") => xml.skip()?,
                "text:a" => {
                    if link.is_none() {
                        *link = e
                            .attr("xlink:href")
                            .filter(|h| !h.is_empty() && h.len() <= 8192)
                            .map(str::to_string);
                    }
                }
                _ => {}
            },
            Ev::End if xml.depth() < d => return Ok(out),
            Ev::Eof => return Ok(out),
            _ => {}
        }
    }
}

fn parse_calc_settings<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
) -> Result<CalcSettings, OdsError> {
    // ODF attribute defaults apply when the element is present.
    let regex = e.attr("table:use-regular-expressions") != Some("false");
    let wildcards = e.attr("table:use-wildcards") == Some("true");
    let mut c = CalcSettings {
        case_sensitive: e.attr("table:case-sensitive") != Some("false"),
        precision_as_shown: e.attr("table:precision-as-shown") == Some("true"),
        whole_cell_match: e.attr("table:search-criteria-must-apply-to-whole-cell") != Some("false"),
        criteria_mode: if wildcards {
            CriteriaMode::Wildcards
        } else if regex {
            CriteriaMode::Regex
        } else {
            CriteriaMode::Literal
        },
        ..CalcSettings::default()
    };
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(ch) => {
                match ch.name.as_str() {
                    "table:null-date" => {
                        c.date_epoch = ch
                            .attr("table:date-value")
                            .map_or(DateEpoch::D1899, epoch_from_null_date);
                    }
                    "table:iteration" => {
                        if ch.attr("table:status") == Some("enable") {
                            let def = IterSettings::default();
                            c.iterations = Some(IterSettings {
                                steps: ch
                                    .attr("table:steps")
                                    .and_then(|v| v.trim().parse::<u32>().ok())
                                    .unwrap_or(def.steps)
                                    // Excel's maximum (review 2 INFO).
                                    .min(32_767),
                                min_change: ch
                                    .attr("table:minimum-difference")
                                    .and_then(|v| v.trim().parse().ok())
                                    .unwrap_or(def.min_change),
                            });
                        }
                    }
                    _ => {}
                }
                xml.skip()?;
            }
            Ev::End if xml.depth() < d => return Ok(c),
            Ev::Eof => return Ok(c),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Named expressions, database ranges (AutoFilter)
// ---------------------------------------------------------------------------

impl Parser<'_> {
    /// Parses `table:named-expressions`; `sheet` is the scope (tab index).
    fn parse_named_expressions<R: BufRead>(
        &mut self,
        xml: &mut XmlReader<R>,
        sheet: Option<u32>,
    ) -> Result<(), OdsError> {
        let d = xml.depth();
        loop {
            match xml.next()? {
                Ev::Start(c) => {
                    let formula = match c.name.as_str() {
                        "table:named-range" => c
                            .attr("table:cell-range-address")
                            .and_then(range_address_formula),
                        "table:named-expression" => c.attr("table:expression").map(|x| {
                            let x = x.trim();
                            if x.starts_with("of:") || x.starts_with("oooc:") {
                                x.to_string()
                            } else if x.starts_with('=') {
                                format!("of:{x}")
                            } else {
                                format!("of:={x}")
                            }
                        }),
                        _ => None,
                    };
                    let name = c.attr("table:name").map(str::trim).unwrap_or_default();
                    if let Some(formula) = formula {
                        if !name.is_empty() && formula.len() <= MAX_NAME_EXPR_LEN {
                            if self.names.len() >= MAX_NAMES {
                                return Err(OdsError::LimitExceeded {
                                    what: "named expressions",
                                    limit: MAX_NAMES as u64,
                                });
                            }
                            self.names.push(NameData {
                                base_cell: None,
                                name: name.to_string(),
                                sheet,
                                formula,
                            });
                        }
                    }
                    xml.skip()?;
                }
                Ev::End if xml.depth() < d => return Ok(()),
                Ev::Eof => return Ok(()),
                _ => {}
            }
        }
    }
}

/// Splits `s` at `sep` outside single quotes.
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        } else if ch == sep && !quoted {
            out.push(&s[start..i]);
            start = i + ch.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

/// Splits `Sheet.A1` at its last unquoted `.` into (sheet, cell); the sheet
/// is `None` for `.A1` or `A1`.
fn split_sheet_cell(part: &str) -> (Option<&str>, &str) {
    let mut quoted = false;
    let mut dot = None;
    for (i, ch) in part.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        } else if ch == '.' && !quoted {
            dot = Some(i);
        }
    }
    match dot {
        Some(0) => (None, &part[1..]),
        Some(i) => (Some(&part[..i]), &part[i + 1..]),
        None => (None, part),
    }
}

/// The plain sheet name of a written sheet qualifier (`$'My ''x'''` → `My 'x'`).
fn sheet_key(raw: &str) -> String {
    let t = raw.trim().trim_start_matches('$');
    match t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => t.to_string(),
    }
}

/// A sheet qualifier for OpenFormula (`$` kept, quoted when needed).
fn quote_sheet(raw: &str) -> String {
    let t = raw.trim();
    let (dollar, rest) = match t.strip_prefix('$') {
        Some(r) => ("$", r),
        None => ("", t),
    };
    if rest.starts_with('\'') {
        return format!("{dollar}{rest}");
    }
    let plain = !rest.is_empty()
        && rest.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !rest.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        format!("{dollar}{rest}")
    } else {
        format!("{dollar}'{}'", rest.replace('\'', "''"))
    }
}

/// One ODF cell-range-address (`$Sheet1.$A$1:.$B$5`) as an OpenFormula
/// reference body (`$Sheet1.$A$1:.$B$5`).
fn normalize_range(addr: &str) -> Option<String> {
    let parts = split_unquoted(addr.trim(), ':');
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|p| p.trim().is_empty()) {
        return None;
    }
    let (s1, c1) = split_sheet_cell(parts[0].trim());
    if c1.is_empty() {
        return None;
    }
    let mut out = match s1 {
        Some(s) => format!("{}.{c1}", quote_sheet(s)),
        None => format!(".{c1}"),
    };
    if let Some(p2) = parts.get(1) {
        let (s2, c2) = split_sheet_cell(p2.trim());
        if c2.is_empty() {
            return None;
        }
        match (s1, s2) {
            (Some(a), Some(b)) if sheet_key(a) != sheet_key(b) => {
                out.push_str(&format!(":{}.{c2}", quote_sheet(b)));
            }
            _ => out.push_str(&format!(":.{c2}")),
        }
    }
    Some(out)
}

/// A `table:cell-range-address` (possibly a space-separated list) as an
/// OpenFormula formula `of:=[…]` (a list becomes a `~` union).
fn range_address_formula(addr: &str) -> Option<String> {
    let mut quoted = false;
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in addr.trim().chars() {
        if ch == '\'' {
            quoted = !quoted;
        }
        if ch == ' ' && !quoted {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    let refs: Vec<String> = parts
        .iter()
        .map(|p| normalize_range(p).map(|r| format!("[{r}]")))
        .collect::<Option<_>>()?;
    (!refs.is_empty()).then(|| format!("of:={}", refs.join("~")))
}

/// `A1` / `$A$1` → (row, col).
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let t: String = s.chars().filter(|c| *c != '$').collect();
    let split = t.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = t.split_at(split);
    let col = tally_types::letters_to_col(letters)?;
    let row: u32 = digits.parse().ok()?;
    ((1..=MAX_ROWS).contains(&row) && col < MAX_COLS).then(|| (row - 1, col))
}

/// A filter tree node.
enum FNode {
    Cond {
        field: u32,
        op: String,
        value: String,
        set: Vec<String>,
    },
    And(Vec<FNode>),
    Or(Vec<FNode>),
}

/// Most filter nodes read for one database range.
const MAX_FILTER_NODES: usize = 4096;

fn parse_fnode<R: BufRead>(
    xml: &mut XmlReader<R>,
    e: &Elem,
    budget: &mut usize,
) -> Result<Option<FNode>, OdsError> {
    if *budget == 0 {
        xml.skip()?;
        return Ok(None);
    }
    *budget -= 1;
    match e.name.as_str() {
        "table:filter-condition" => {
            let field = e
                .attr("table:field-number")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|f| *f < MAX_COLS);
            let op = e.attr("table:operator").unwrap_or("=").to_string();
            let value = e.attr("table:value").unwrap_or_default().to_string();
            let mut set = Vec::new();
            let d = xml.depth();
            loop {
                match xml.next()? {
                    Ev::Start(c) => {
                        if c.name == "table:filter-set-item" && set.len() < 10_000 {
                            set.push(c.attr("table:value").unwrap_or_default().to_string());
                        }
                        xml.skip()?;
                    }
                    Ev::End if xml.depth() < d => break,
                    Ev::Eof => break,
                    _ => {}
                }
            }
            Ok(field.map(|field| FNode::Cond {
                field,
                op,
                value,
                set,
            }))
        }
        "table:filter-and" | "table:filter-or" | "table:filter" => {
            let mut kids = Vec::new();
            let d = xml.depth();
            loop {
                match xml.next()? {
                    Ev::Start(c) => {
                        if let Some(n) = parse_fnode(xml, &c, budget)? {
                            kids.push(n);
                        }
                    }
                    Ev::End if xml.depth() < d => break,
                    Ev::Eof => break,
                    _ => {}
                }
            }
            Ok(Some(if e.name == "table:filter-or" {
                FNode::Or(kids)
            } else {
                FNode::And(kids)
            }))
        }
        _ => {
            xml.skip()?;
            Ok(None)
        }
    }
}

fn filter_op(op: &str) -> Option<FilterOpData> {
    Some(match op {
        "=" => FilterOpData::Eq,
        "!=" => FilterOpData::Ne,
        ">" => FilterOpData::Gt,
        ">=" => FilterOpData::Ge,
        "<" => FilterOpData::Lt,
        "<=" => FilterOpData::Le,
        "contains" => FilterOpData::Contains,
        "begins" => FilterOpData::BeginsWith,
        "ends" => FilterOpData::EndsWith,
        "empty" => FilterOpData::Empty,
        "!empty" => FilterOpData::NotEmpty,
        _ => return None,
    })
}

/// Adds the column filters expressed by one top-level item of a filter.
fn filter_item(node: &FNode, c0: u32, out: &mut Vec<FilterColumnData>) {
    let slot = |out: &mut Vec<FilterColumnData>, field: u32| -> usize {
        let col = c0.saturating_add(field);
        match out.iter().position(|c| c.col == col) {
            Some(i) => i,
            None => {
                out.push(FilterColumnData {
                    col,
                    values: None,
                    condition: None,
                });
                out.len() - 1
            }
        }
    };
    match node {
        FNode::Cond {
            field,
            op,
            value,
            set,
        } => {
            if !set.is_empty() && op == "=" {
                let i = slot(out, *field);
                if out[i].values.is_none() {
                    out[i].values = Some(set.clone());
                }
            } else if let Some(op) = filter_op(op) {
                let i = slot(out, *field);
                if out[i].condition.is_none() {
                    out[i].condition = Some(FilterConditionData {
                        op,
                        value: value.clone(),
                    });
                }
            }
        }
        FNode::And(kids) => {
            for k in kids {
                filter_item(k, c0, out);
            }
        }
        FNode::Or(kids) => {
            let mut field: Option<u32> = None;
            let mut values = Vec::new();
            for k in kids {
                let FNode::Cond {
                    field: f,
                    op,
                    value,
                    set,
                } = k
                else {
                    return;
                };
                if field.is_some_and(|x| x != *f) || !set.is_empty() {
                    return;
                }
                field = Some(*f);
                match op.as_str() {
                    "=" => values.push(value.clone()),
                    "empty" => values.push(String::new()),
                    _ => return,
                }
            }
            if let Some(f) = field {
                let i = slot(out, f);
                if out[i].values.is_none() {
                    out[i].values = Some(values);
                }
            }
        }
    }
}

/// Parses `table:database-ranges`, attaching AutoFilters to `sheets`.
fn parse_database_ranges<R: BufRead>(
    xml: &mut XmlReader<R>,
    sheets: &mut [SheetData],
) -> Result<(), OdsError> {
    let d = xml.depth();
    loop {
        match xml.next()? {
            Ev::Start(c) if c.name == "table:database-range" => {
                let buttons = c.attr("table:display-filter-buttons") == Some("true");
                let target = c
                    .attr("table:target-range-address")
                    .map(str::to_string)
                    .unwrap_or_default();
                let name = c.attr("table:name").unwrap_or_default().to_string();
                // The target range first: filters need its columns.
                let parts = split_unquoted(target.trim(), ':');
                let located = if parts.len() == 2 {
                    let (s1, a) = split_sheet_cell(parts[0].trim());
                    let (s2, b) = split_sheet_cell(parts[1].trim());
                    match (s1.map(sheet_key), parse_cell_ref(a), parse_cell_ref(b)) {
                        (Some(s1), Some((r0, c0)), Some((r1, c1)))
                            if s2.is_none_or(|s| sheet_key(s) == s1) =>
                        {
                            sheets.iter().position(|s| s.name == s1).map(|i| {
                                (
                                    i,
                                    CellRangeLocal {
                                        r0: r0.min(r1),
                                        c0: c0.min(c1),
                                        r1: r0.max(r1),
                                        c1: c0.max(c1),
                                    },
                                )
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
                let sheet_index = |n: &str| -> Option<u32> {
                    let key = sheet_key(n);
                    names.iter().position(|x| *x == key).map(|i| i as u32)
                };
                let mut tree: Option<FNode> = None;
                let mut std_filter: Option<StandardFilterData> = None;
                let mut subtotals: Option<SubtotalsData> = None;
                let mut budget = MAX_FILTER_NODES;
                let dd = xml.depth();
                loop {
                    match xml.next()? {
                        Ev::Start(ch) => {
                            if ch.name == "table:filter" {
                                let standard = !buttons || crate::tools::is_standard_filter(&ch);
                                match (standard, located) {
                                    (true, Some((_, range))) => {
                                        std_filter = Some(crate::tools::parse_standard_filter(
                                            xml,
                                            &ch,
                                            range,
                                            &sheet_index,
                                        )?);
                                    }
                                    _ => tree = parse_fnode(xml, &ch, &mut budget)?,
                                }
                            } else if ch.name == "table:subtotal-rules" {
                                if let Some((_, range)) = located {
                                    subtotals =
                                        Some(crate::tools::parse_subtotal_rules(xml, &ch, range)?);
                                } else {
                                    xml.skip()?;
                                }
                            } else {
                                xml.skip()?;
                            }
                        }
                        Ev::End if xml.depth() < dd => break,
                        Ev::Eof => break,
                        _ => {}
                    }
                }
                let Some((tab, range)) = located else {
                    continue;
                };
                if !buttons {
                    let Some(sheet) = sheets.get_mut(tab) else {
                        continue;
                    };
                    if name.starts_with("__Anonymous_Sheet_DB__") {
                        // The sheet's own standard filter / subtotals.
                        if let Some(f) = std_filter {
                            sheet.standard_filter = Some(f);
                        }
                        if let Some(st) = subtotals {
                            sheet.subtotals = Some(st);
                        }
                    } else if sheet.database_ranges.len() < 10_000 {
                        sheet
                            .database_ranges
                            .push(crate::tools::database_range_from(
                                &name, range, &c, std_filter, subtotals,
                            ));
                    }
                    continue;
                }
                let Some(sheet) = sheets.get_mut(tab) else {
                    continue;
                };
                if let Some(f) = std_filter.take() {
                    sheet.standard_filter = Some(f);
                }
                if let Some(st) = subtotals.take() {
                    sheet.subtotals = Some(st);
                }
                if sheet.auto_filter.is_some() {
                    continue;
                }
                let mut columns = Vec::new();
                if let Some(t) = &tree {
                    match t {
                        FNode::Or(_) => filter_item(t, range.c0, &mut columns),
                        other => filter_item(other, range.c0, &mut columns),
                    }
                }
                columns.retain(|c| c.col <= range.c1);
                columns.sort_by_key(|c| c.col);
                sheet.auto_filter = Some(AutoFilterData {
                    range,
                    columns,
                    filtered_rows: Vec::new(),
                });
            }
            Ev::Start(_) => xml.skip()?,
            Ev::End if xml.depth() < d => return Ok(()),
            Ev::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// Sorts row runs and merges equal neighbours.
fn normalize_runs(runs: &mut Vec<DimRun>) {
    runs.sort_by_key(|r| r.start);
    let old = std::mem::take(runs);
    for r in old {
        push_run(runs, r.start, r.end, r.size_hmm, r.hidden);
    }
}

/// Resolves rows read with `table:visibility="filter"`: filter-hidden rows
/// of a sheet with an AutoFilter, plain hidden rows otherwise.
fn attach_filter_rows(sheets: &mut [SheetData], filter_rows: Vec<Vec<DimRun>>) {
    for (sheet, runs) in sheets.iter_mut().zip(filter_rows) {
        if runs.is_empty() {
            continue;
        }
        match sheet.auto_filter.as_mut() {
            Some(af) => {
                let mut spans: Vec<(u32, u32)> = af.filtered_rows.clone();
                for r in &runs {
                    spans.push((r.start, r.end));
                    if r.size_hmm != crate::DEFAULT_ROW_HEIGHT_HMM {
                        sheet.row_heights.push(DimRun {
                            hidden: false,
                            ..r.clone()
                        });
                    }
                }
                spans.sort_unstable();
                let mut merged: Vec<(u32, u32)> = Vec::new();
                for (a, b) in spans {
                    match merged.last_mut() {
                        Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
                        _ => merged.push((a, b)),
                    }
                }
                af.filtered_rows = merged;
            }
            None => sheet.row_heights.extend(runs),
        }
        normalize_runs(&mut sheet.row_heights);
    }
}
