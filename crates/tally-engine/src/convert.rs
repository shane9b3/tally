//! Conversion between the engine [`Workbook`] and the `tally-model`
//! interchange ([`WorkbookData`]) used by the file readers and writers.
//!
//! Formulas are exchanged as OpenFormula text (`of:=SUM([.A1:.A3])`) with
//! their cached values. Short style runs over empty cells are written as
//! styled empty cells; runs of [`LONG_STYLE_RUN`] rows or more are kept as
//! [`StyleRun`]s, so a bold whole column does not become a million cells
//! and no formatting is lost.

use std::collections::HashMap;
use std::sync::Arc;

use tally_formula::{parse, print, Dialect, Expr, Formula};
use tally_model::{
    CellContentData, CellData, CellRangeLocal, DocSettings, LinkData, NameData, PageSetupData,
    SheetData, StyleData, StyleRun, WorkbookData,
};
use tally_types::{CellAddr, Matrix, Value, MAX_COLS, MAX_ROWS};

use crate::formula_cells::CellState;
use crate::storage::{CellContent, ColumnStore, DimRuns, Sheet, Spans, StyleId};
use crate::workbook::{Pending, SheetNamer, Workbook, MAX_SHEETS};

/// Style runs of at least this many rows are exported as [`StyleRun`]s
/// instead of styled empty cells.
const LONG_STYLE_RUN: u32 = 10_000;

/// Most merged areas kept per sheet (the readers cap at the same number).
pub const MAX_MERGES_PER_SHEET: usize = 100_000;

/// Most cells covered by array formula blocks in one loaded workbook (the
/// readers count block areas against their `max_cells`, 10M by default).
pub const MAX_ARRAY_CELLS: u64 = 10_000_000;

impl Workbook {
    /// Builds a workbook from interchange data. Values are taken as they are.
    pub fn from_data(data: &WorkbookData) -> Workbook {
        Self::from_data_with(data, false)
    }

    /// Builds a workbook; with `recognize`, text values are treated as typed
    /// constants (CSV import: numbers, dates, percentages and booleans are
    /// recognised, while `=…` stays text, see [`Self::from_data_opts`]).
    /// Formula cells are left dirty: call [`crate::recalc::recalc`] next.
    pub fn from_data_with(data: &WorkbookData, recognize: bool) -> Workbook {
        Self::from_data_opts(data, recognize, false)
    }

    /// Like [`Self::from_data_with`]; with `recognize` and `formulas`,
    /// recognised text starting with `=` becomes a live formula (the CSV
    /// "Evaluate formulas" import option, off by default because a CSV from
    /// an untrusted source would otherwise run its formulas on open).
    pub fn from_data_opts(data: &WorkbookData, recognize: bool, formulas: bool) -> Workbook {
        let mut wb = Workbook::empty();
        wb.import_warnings = data.warnings.clone();
        wb.settings = data.settings.calc_settings.clone();
        wb.meta = data.meta.clone();
        if let Some(l) = data
            .settings
            .locale
            .as_deref()
            .and_then(tally_numfmt::locale_by_tag)
        {
            wb.locale = l;
        }
        wb.structure_protection = data.settings.structure_protection.clone();
        // Readers cap the sheet count too; this is the engine's own limit.
        let sheet_data = &data.sheets[..data.sheets.len().min(MAX_SHEETS)];
        let mut ids = Vec::with_capacity(sheet_data.len());
        // All names are made unique in one pass and the sheet index is built
        // once, so loading is linear in the number of sheets.
        let mut namer = SheetNamer::default();
        let mut renamed = false;
        for (tab, sd) in sheet_data.iter().enumerate() {
            // Names from files follow the same rules as names typed in
            // the app (review 2, LOW-2); formulas in the file still find
            // the sheet under its original name while loading.
            let clean = sanitize_sheet_name(&sd.name);
            let name = if clean.trim().is_empty() {
                namer.claim_numbered(tab as u64 + 1)
            } else {
                namer.claim(&clean)
            };
            let id = wb.alloc_sheet_id();
            if clean != sd.name && !sd.name.trim().is_empty() {
                renamed = true;
                wb.load_aliases.entry(sd.name.to_lowercase()).or_insert(id);
            }
            let mut s = Sheet::new(id, &name);
            s.hidden = sd.hidden;
            s.tab_color = sd.tab_color;
            s.col_dims = DimRuns::from_runs(&sd.col_widths);
            s.row_dims = DimRuns::from_runs(&sd.row_heights);
            wb.sheets.push(s);
            ids.push(id);
        }
        wb.reindex();
        if let Some(&first) = ids.first() {
            wb.active_sheet = first;
        }
        if ids.is_empty() {
            ids.push(wb.add_sheet(Some("Sheet1"), None));
        }
        // Names before formulas, so formulas resolve them while parsing.
        load_names(&mut wb, &data.names, &ids);
        let mut merges_capped = false;
        for (sd, &sheet) in sheet_data.iter().zip(&ids) {
            if let Some(s) = wb.sheet_mut(sheet) {
                s.meta.merges = clean_merges(&sd.merges);
                merges_capped |= sd.merges.len() > MAX_MERGES_PER_SHEET
                    && s.meta.merges.len() == MAX_MERGES_PER_SHEET;
                s.meta.frozen = sd
                    .frozen
                    .filter(|(r, c)| (*r > 0 || *c > 0) && *r < MAX_ROWS && *c < MAX_COLS);
                if let Some(af) = &sd.auto_filter {
                    let r = af.range;
                    if r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS {
                        s.meta.auto_filter = Some(crate::filter::from_model(af));
                        s.filtered_rows = Spans::from_spans(
                            af.filtered_rows
                                .iter()
                                .copied()
                                .filter(|(a, b)| *a > r.r0 && *b <= r.r1)
                                .collect(),
                        );
                    }
                }
            }
        }
        if merges_capped {
            let w =
                format!("Only the first {MAX_MERGES_PER_SHEET} merged areas of a sheet are kept.");
            if !wb.import_warnings.contains(&w) {
                wb.import_warnings.push(w);
            }
        }
        // v0.3 sheet state (after the names, so rule formulas resolve them).
        for (sd, &sheet) in sheet_data.iter().zip(&ids) {
            load_v03(&mut wb, sheet, sd);
            load_v04(&mut wb, sheet, sd);
        }
        wb.cell_styles = data.cell_styles.clone();
        wb.custom_sort_lists = data.settings.custom_sort_lists.clone();
        let style_ids: Vec<StyleId> = data
            .styles
            .iter()
            .map(|s| wb.styles.intern(s.clone()))
            .collect();
        // Cells covered by array blocks so far (all sheets).
        let mut array_cells: u64 = 0;
        // Parsed shared-formula groups: (text allocation, anchor) → AST.
        let mut shared: HashMap<(usize, u32, u32), Option<Arc<Formula>>> = HashMap::new();
        for (sd, &sheet) in sheet_data.iter().zip(&ids) {
            shared.clear();
            // Runs first: a cell's own style wins over a run covering it.
            for run in &sd.style_runs {
                let Some(st) = style_ids.get(run.style as usize) else {
                    continue;
                };
                if run.start_row > run.end_row || run.end_row >= MAX_ROWS {
                    continue;
                }
                if let Some(col) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(run.col)) {
                    col.styles.set(run.start_row, run.end_row, *st);
                }
            }
            for cell in &sd.cells {
                let addr = CellAddr::new(sheet, cell.row, cell.col);
                if !addr.is_valid() {
                    continue;
                }
                if let Some(st) = cell.style.and_then(|i| style_ids.get(i as usize)) {
                    if let Some(col) = wb.sheet_mut(sheet).and_then(|s| s.column_mut(cell.col)) {
                        col.styles.set(cell.row, cell.row, *st);
                    }
                }
                match &cell.content {
                    CellContentData::Value(Value::Text(t)) if recognize && formulas => {
                        wb.set_input(addr, t);
                    }
                    CellContentData::Value(Value::Text(t)) if recognize => {
                        wb.set_constant_input(addr, t);
                    }
                    CellContentData::Value(v) => wb.set_value(addr, v.clone()),
                    CellContentData::Formula { text, cached } => {
                        let f = parse(
                            text,
                            addr,
                            Dialect::open_formula(),
                            &crate::workbook::OdfResolver(&wb),
                        );
                        match f {
                            Ok(f) => wb.set_formula(addr, Arc::new(f), None),
                            Err(e) => {
                                let raw = text.strip_prefix("of:").unwrap_or(text);
                                wb.set_formula(
                                    addr,
                                    Arc::new(Formula::new(Expr::Err(e.kind))),
                                    Some(raw.into()),
                                );
                            }
                        }
                        if let (Some(c), Some((id, _))) = (cached, wb.formula_at(addr)) {
                            if let Some(fc) = wb.formulas.get_mut(id) {
                                fc.cached = c.clone();
                            }
                        }
                    }
                    CellContentData::SharedFormula {
                        text,
                        anchor_row,
                        anchor_col,
                        cached,
                    } => {
                        // One parse per group: the AST stores relative
                        // references as offsets, so the anchor's AST is the
                        // member's formula as is (review 2, MEDIUM-1).
                        let key = (
                            Arc::as_ptr(text) as *const u8 as usize,
                            *anchor_row,
                            *anchor_col,
                        );
                        let f = shared
                            .entry(key)
                            .or_insert_with(|| {
                                let at = CellAddr::new(sheet, *anchor_row, *anchor_col);
                                parse(
                                    text,
                                    at,
                                    Dialect::open_formula(),
                                    &crate::workbook::OdfResolver(&wb),
                                )
                                .ok()
                                .map(Arc::new)
                            })
                            .clone();
                        match f {
                            Some(f) => {
                                wb.set_formula(addr, f, None);
                                if let (Some(c), Some((id, _))) = (cached, wb.formula_at(addr)) {
                                    if let Some(fc) = wb.formulas.get_mut(id) {
                                        fc.cached = c.clone();
                                    }
                                }
                            }
                            // The group's formula does not parse: keep the value.
                            None => {
                                if let Some(c) = cached {
                                    wb.set_value(addr, c.clone());
                                }
                            }
                        }
                    }
                }
            }
            for block in &sd.array_formulas {
                if !load_array(&mut wb, sheet, block, &mut array_cells) {
                    let w = format!(
                        "Array formulas covering more than {MAX_ARRAY_CELLS} cells in total were \
                         loaded as plain values."
                    );
                    if !wb.import_warnings.contains(&w) {
                        wb.import_warnings.push(w);
                    }
                }
            }
        }
        let active = ids
            .get(data.settings.active_sheet as usize)
            .or_else(|| ids.first())
            .copied()
            .unwrap_or(1);
        wb.active_sheet = active;
        wb.load_aliases.clear();
        if renamed {
            wb.import_warnings.push(
                "Sheet names with characters that are not allowed ([ ] * ? : / \\ or control \
                 characters) were changed."
                    .into(),
            );
        }
        wb.pending = Pending::default();
        wb.dirty_all();
        wb
    }

    /// Exports the workbook. Styles are numbered in order of first use, so
    /// equal workbooks give equal data.
    pub fn to_data(&self) -> WorkbookData {
        let mut styles: Vec<StyleData> = Vec::new();
        let mut style_map: HashMap<StyleId, u32> = HashMap::new();
        let mut sheets = Vec::with_capacity(self.sheets.len());
        for sheet in &self.sheets {
            let mut cells: Vec<CellData> = Vec::new();
            let mut style_runs: Vec<StyleRun> = Vec::new();
            for (ci, col) in sheet.columns.iter().enumerate() {
                let c = ci as u32;
                for (r, content) in col.cells.iter_range(0, MAX_ROWS - 1) {
                    let addr = CellAddr::new(sheet.id, r, c);
                    let content = match content {
                        CellContent::Value(v) => CellContentData::Value(v.clone()),
                        // Covered cells of an array block keep their values.
                        CellContent::ArrayPart(_) => CellContentData::Value(self.value(addr)),
                        CellContent::Formula(id) => match self.formulas.get(*id) {
                            // A formula that does not parse is saved as a
                            // text cell holding what was typed, so other
                            // applications show the text (it reloads as text).
                            Some(f) if f.invalid_text.is_some() => CellContentData::Value(
                                Value::Text(f.invalid_text.clone().unwrap_or_default()),
                            ),
                            Some(f) => {
                                let text = print(&f.formula, addr, Dialect::open_formula(), self);
                                let cached =
                                    (f.state == CellState::Clean).then(|| f.cached.clone());
                                CellContentData::Formula {
                                    text: text.into(),
                                    cached,
                                }
                            }
                            None => continue,
                        },
                    };
                    let st = col.styles.get(r);
                    cells.push(CellData {
                        row: r,
                        col: c,
                        content,
                        style: (st != StyleId::DEFAULT).then_some(st.0),
                    });
                }
                for (s, e, st) in col.styles.runs_in(0, MAX_ROWS - 1) {
                    if e - s >= LONG_STYLE_RUN - 1 {
                        style_runs.push(StyleRun {
                            col: c,
                            start_row: s,
                            end_row: e,
                            style: st.0,
                        });
                        continue;
                    }
                    for r in s..=e {
                        if col.cells.get(r).is_none() {
                            cells.push(CellData {
                                row: r,
                                col: c,
                                content: CellContentData::Value(Value::Empty),
                                style: Some(st.0),
                            });
                        }
                    }
                }
            }
            cells.sort_by_key(|c| (c.row, c.col));
            let mut index_of = |id: StyleId| {
                let next = styles.len() as u32;
                *style_map.entry(id).or_insert_with(|| {
                    styles.push(self.styles.get(id).data.clone());
                    next
                })
            };
            for cell in &mut cells {
                if let Some(raw) = cell.style {
                    cell.style = Some(index_of(StyleId(raw)));
                }
            }
            for run in &mut style_runs {
                run.style = index_of(StyleId(run.style));
            }
            let mut array_formulas: Vec<CellRangeLocal> = Vec::new();
            for col in &sheet.columns {
                for r in &col.formula_rows {
                    if let Some(CellContent::Formula(id)) = col.cells.get(*r) {
                        if let Some(b) = self.formulas.get(*id).and_then(|f| f.array_range()) {
                            array_formulas.push(CellRangeLocal {
                                r0: b.r0,
                                c0: b.c0,
                                r1: b.r1,
                                c1: b.c1,
                            });
                        }
                    }
                }
            }
            array_formulas.sort_by_key(|b| (b.r0, b.c0));
            sheets.push(SheetData {
                name: sheet.name.to_string(),
                cells,
                col_widths: sheet.col_dims.runs().to_vec(),
                row_heights: sheet.row_dims.runs().to_vec(),
                merges: sheet.meta.merges.clone(),
                frozen: sheet.meta.frozen,
                tab_color: sheet.tab_color,
                hidden: sheet.hidden,
                style_runs,
                array_formulas,
                auto_filter: sheet
                    .meta
                    .auto_filter
                    .as_ref()
                    .map(|af| crate::filter::to_model(af, &sheet.filtered_rows)),
                conditional_formats: sheet
                    .meta
                    .cond_formats
                    .iter()
                    .map(|cf| crate::condfmt::to_model(self, sheet.id, cf))
                    .collect(),
                validations: sheet
                    .meta
                    .validations
                    .iter()
                    .map(|v| crate::validation::to_model(self, sheet.id, v))
                    .collect(),
                notes: sheet
                    .meta
                    .notes
                    .iter()
                    .map(|((r, c), n)| n.to_model(*r, *c))
                    .collect(),
                links: sheet
                    .meta
                    .links
                    .iter()
                    .map(|((r, c), u)| LinkData {
                        row: *r,
                        col: *c,
                        url: u.to_string(),
                    })
                    .collect(),
                protection: sheet.meta.protection.clone(),
                page_setup: (sheet.meta.page_setup != PageSetupData::default())
                    .then(|| sheet.meta.page_setup.clone()),
                row_breaks: sheet.meta.row_breaks.iter().copied().collect(),
                col_breaks: sheet.meta.col_breaks.iter().copied().collect(),
                // Wave 5 (identity pass-through until W5-1/2 land).
                charts: crate::charts::to_data(self, sheet.id),
                images: crate::charts::images_to_data(self, sheet.id),
                pivots: crate::pivot::to_data(self, sheet.id),
                row_groups: sheet.meta.row_groups.clone(),
                col_groups: sheet.meta.col_groups.clone(),
                scenarios: crate::scenarios::to_data(self, sheet.id),
                database_ranges: sheet.meta.database_ranges.clone(),
                standard_filter: sheet.meta.standard_filter.clone(),
                subtotals: sheet.meta.subtotals.clone(),
                view_options: sheet.meta.view_options.clone(),
                split: sheet.meta.split,
            });
        }
        let mut names: Vec<NameData> = Vec::new();
        for (_, e) in self.names.defined() {
            let Some(def) = &e.def else { continue };
            let sheet = match e.scope {
                None => None,
                Some(s) => match self.tab_of(s) {
                    Some(i) => Some(i as u32),
                    None => continue,
                },
            };
            names.push(NameData {
                name: e.name.to_string(),
                sheet,
                formula: print(&def.formula, def.base, Dialect::open_formula(), self),
            });
        }
        names.sort_by(|a, b| {
            (a.sheet, a.name.to_lowercase()).cmp(&(b.sheet, b.name.to_lowercase()))
        });
        WorkbookData {
            sheets,
            styles,
            names,
            settings: DocSettings {
                active_sheet: self.tab_of(self.active_sheet).unwrap_or(0) as u32,
                calc_settings: self.settings.clone(),
                locale: (self.locale.tag != tally_numfmt::EN_US.tag)
                    .then(|| self.locale.tag.to_string()),
                structure_protection: self.structure_protection.clone(),
                custom_sort_lists: self.custom_sort_lists.clone(),
            },
            meta: self.meta.clone(),
            warnings: Vec::new(),
            cell_styles: self.cell_styles.clone(),
        }
    }
}

/// Loads the wave-5 sheet state (charts, pictures, pivots, outline groups,
/// scenarios, database ranges, filters, subtotals, view options, split).
/// W5-0a keeps everything as read (identity); W5-1/2 add validation.
fn load_v04(wb: &mut Workbook, sheet: tally_types::SheetId, sd: &SheetData) {
    crate::charts::from_data(wb, sheet, &sd.charts, &sd.images);
    crate::pivot::from_data(wb, sheet, &sd.pivots);
    crate::scenarios::from_data(wb, sheet, &sd.scenarios);
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    let ok = |g: &tally_model::OutlineGroupData, limit: u32| {
        g.start <= g.end && g.end < limit && (1..=crate::outline::MAX_LEVELS).contains(&g.level)
    };
    s.meta.row_groups = sd
        .row_groups
        .iter()
        .filter(|g| ok(g, MAX_ROWS))
        .take(100_000)
        .copied()
        .collect();
    s.meta.col_groups = sd
        .col_groups
        .iter()
        .filter(|g| ok(g, MAX_COLS))
        .take(100_000)
        .copied()
        .collect();
    s.meta.database_ranges = sd.database_ranges.iter().take(10_000).cloned().collect();
    s.meta.standard_filter = sd.standard_filter.clone();
    s.meta.subtotals = sd.subtotals.clone();
    s.meta.view_options = sd.view_options.clone();
    s.meta.split = sd.split;
}

/// A sheet name from a file made valid: control characters and
/// `[ ] * ? : / \` become `_`, leading and trailing `'` are dropped, and
/// the name is cut to 255 characters (see `ops::check_sheet_name`).
pub fn sanitize_sheet_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '[' | ']' | '*' | '?' | ':' | '/' | '\\') {
                '_'
            } else {
                c
            }
        })
        .take(255)
        .collect();
    mapped.trim_matches('\'').to_string()
}

/// Loads conditional formats, validations, notes, links, protection, page
/// setup and breaks of one sheet (invalid entries are skipped).
fn load_v03(wb: &mut Workbook, sheet: tally_types::SheetId, sd: &SheetData) {
    let mut next_id = 1u32;
    let mut cfs = Vec::new();
    for d in sd
        .conditional_formats
        .iter()
        .take(crate::condfmt::MAX_CF_PER_SHEET)
    {
        if let Some(cf) = crate::condfmt::from_model(wb, sheet, next_id, d) {
            next_id += 1;
            cfs.push(cf);
        }
    }
    let mut vals: Vec<crate::validation::Validation> = Vec::new();
    for d in sd
        .validations
        .iter()
        .take(crate::validation::MAX_VALIDATIONS)
    {
        if let Some(v) = crate::validation::from_model(wb, sheet, next_id, d) {
            next_id += 1;
            vals.push(v);
        }
    }
    let Some(s) = wb.sheet_mut(sheet) else {
        return;
    };
    s.meta.cond_formats = cfs;
    s.meta.validations = vals;
    s.meta.next_id = next_id;
    let ok = |r: u32, c: u32| r < MAX_ROWS && c < MAX_COLS;
    for n in sd.notes.iter().take(crate::ops_v03::MAX_NOTES) {
        if ok(n.row, n.col) {
            s.meta
                .notes
                .insert((n.row, n.col), crate::annotations::Note::from_model(n));
        }
    }
    for l in sd.links.iter().take(crate::ops_v03::MAX_NOTES) {
        if ok(l.row, l.col) && !l.url.is_empty() && l.url.len() <= crate::ops_v03::MAX_URL_LEN {
            s.meta.links.insert((l.row, l.col), l.url.as_str().into());
        }
    }
    s.meta.protection = sd.protection.clone();
    if let Some(p) = &sd.page_setup {
        let mut p = p.clone();
        p.print_range = p
            .print_range
            .filter(|r| r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS);
        p.repeat_rows = p.repeat_rows.filter(|(a, b)| a <= b && *b < MAX_ROWS);
        p.repeat_cols = p.repeat_cols.filter(|(a, b)| a <= b && *b < MAX_COLS);
        s.meta.page_setup = p;
    }
    s.meta.row_breaks = sd
        .row_breaks
        .iter()
        .copied()
        .filter(|b| *b > 0 && *b < MAX_ROWS)
        .take(10_000)
        .collect();
    s.meta.col_breaks = sd
        .col_breaks
        .iter()
        .copied()
        .filter(|b| *b > 0 && *b < MAX_COLS)
        .take(10_000)
        .collect();
}

/// Loads interchange names (invalid, duplicate or unparsable ones are skipped).
fn load_names(wb: &mut Workbook, names: &[NameData], ids: &[tally_types::SheetId]) {
    for n in names.iter().take(crate::names::MAX_NAMES) {
        let scope = match n.sheet {
            None => None,
            Some(i) => match ids.get(i as usize) {
                Some(id) => Some(*id),
                None => continue,
            },
        };
        if crate::names::validate_name(wb, &n.name).is_err() {
            continue;
        }
        if wb
            .names
            .lookup(&n.name, scope)
            .and_then(|id| wb.names.get(id))
            .is_some_and(|e| e.def.is_some())
        {
            continue;
        }
        // Model names are relative to A1 of the first sheet.
        let base = CellAddr::new(ids.first().copied().unwrap_or(1), 0, 0);
        let text = if n.formula.starts_with("of:") || n.formula.starts_with("oooc:") {
            n.formula.clone()
        } else {
            format!("of:={}", n.formula.trim_start_matches('='))
        };
        let Ok(f) = parse(
            &text,
            base,
            Dialect::open_formula(),
            &crate::workbook::OdfResolver(wb),
        ) else {
            continue;
        };
        wb.names.define(
            &n.name,
            scope,
            crate::names::NameDef {
                formula: Arc::new(f),
                base,
            },
        );
    }
}

/// Valid, non-overlapping merges of more than one cell, at most
/// [`MAX_MERGES_PER_SHEET`]. Of overlapping merges the one with the
/// smaller (top row, left column) wins.
///
/// O(n log n): a sweep over rows keeps the accepted merges that span the
/// current row in a map ordered by first column (they are disjoint there),
/// so each candidate checks one neighbour (security review 2, HIGH-1; the
/// pairwise check took 128 s for 1M merges).
fn clean_merges(merges: &[CellRangeLocal]) -> Vec<CellRangeLocal> {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};
    let mut cand: Vec<(usize, CellRangeLocal)> = merges
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, m)| {
            m.r0 <= m.r1
                && m.c0 <= m.c1
                && m.r1 < MAX_ROWS
                && m.c1 < MAX_COLS
                && (m.r0, m.c0) != (m.r1, m.c1)
        })
        .collect();
    cand.sort_by_key(|(i, m)| (m.r0, m.c0, *i));
    // c0 → (c1, r1) of the accepted merges spanning the sweep row.
    let mut active: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut expiry: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    let mut out: Vec<CellRangeLocal> = Vec::new();
    for (_, m) in cand {
        if out.len() >= MAX_MERGES_PER_SHEET {
            break;
        }
        while let Some(Reverse((r1, c0))) = expiry.peek().copied() {
            if r1 >= m.r0 {
                break;
            }
            expiry.pop();
            active.remove(&c0);
        }
        let overlaps = active
            .range(..=m.c1)
            .next_back()
            .is_some_and(|(_, (c1, _))| *c1 >= m.c0);
        if overlaps {
            continue;
        }
        active.insert(m.c0, (m.c1, m.r1));
        expiry.push(Reverse((m.r1, m.c0)));
        out.push(m);
    }
    out.sort_by_key(|m| (m.r0, m.c0));
    out
}

/// Turns the formula at a block's anchor into an array formula, keeping the
/// covered cells' values from the file as the cached result. Blocks that
/// overlap an existing block, or would take the workbook past
/// [`MAX_ARRAY_CELLS`] covered cells, are skipped (the anchor keeps its
/// cached value) before anything is materialized. `false` if the block was
/// skipped for the budget.
fn load_array(
    wb: &mut Workbook,
    sheet: tally_types::SheetId,
    b: &CellRangeLocal,
    used: &mut u64,
) -> bool {
    if b.r0 > b.r1 || b.c0 > b.c1 || b.r1 >= MAX_ROWS || b.c1 >= MAX_COLS {
        return true;
    }
    let (rows, cols) = (b.r1 - b.r0 + 1, b.c1 - b.c0 + 1);
    let area = u64::from(rows) * u64::from(cols);
    if area > crate::ops::MAX_OP_CELLS || used.saturating_add(area) > MAX_ARRAY_CELLS {
        return false;
    }
    let anchor = CellAddr::new(sheet, b.r0, b.c0);
    let Some((_, fc)) = wb.formula_at(anchor) else {
        return true;
    };
    if fc.invalid_text.is_some() || fc.array.is_some() {
        return true;
    }
    // Other blocks overlapping this one: it is dropped.
    for other in wb.array_blocks_in(&tally_types::CellRange::new(sheet, b.r0, b.c0, b.r1, b.c1)) {
        if other.start() != anchor {
            return true;
        }
    }
    *used += area;
    let formula = fc.formula.clone();
    let mut m = Matrix::new_filled(rows, cols, Value::Empty);
    for r in 0..rows {
        for c in 0..cols {
            m.set(r, c, wb.value(CellAddr::new(sheet, b.r0 + r, b.c0 + c)));
        }
    }
    wb.set_array_formula(anchor, formula, rows, cols);
    if let Some((id, _)) = wb.formula_at(anchor) {
        if let Some(fc) = wb.formulas.get_mut(id) {
            fc.cached = m.get(0, 0).cloned().unwrap_or(Value::Empty);
            fc.array_result = Some(Arc::new(m));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recalc::recalc;
    use tally_model::FontData;

    fn m(r0: u32, c0: u32, r1: u32, c1: u32) -> CellRangeLocal {
        CellRangeLocal { r0, c0, r1, c1 }
    }

    #[test]
    fn clean_merges_drops_overlaps_and_invalid() {
        let out = clean_merges(&[
            m(0, 0, 1, 1),
            m(1, 1, 2, 2), // overlaps the first at B2
            m(0, 2, 0, 3),
            m(5, 5, 5, 5), // one cell
            m(3, 0, 10, 0),
            m(10, 0, 11, 1), // overlaps A4:A11 at A11
            m(4, 1, 9, 3),   // next to A4:A11, overlaps nothing
            m(2, 3, 3, 3),   // below C1:D1
            m(1, 0, 0, 0),   // r0 > r1
        ]);
        assert_eq!(
            out,
            vec![
                m(0, 0, 1, 1),
                m(0, 2, 0, 3),
                m(2, 3, 3, 3),
                m(3, 0, 10, 0),
                m(4, 1, 9, 3)
            ]
        );
        // A tall merge blocks later ones it spans.
        let out = clean_merges(&[m(0, 5, 1_000_000, 5), m(500_000, 4, 500_000, 6)]);
        assert_eq!(out, vec![m(0, 5, 1_000_000, 5)]);
    }

    /// HIGH-2: a block that would pass the workbook-wide budget is skipped
    /// before anything is materialized; the anchor keeps its value.
    #[test]
    fn array_budget_is_checked_before_materializing() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        wb.set_input(CellAddr::new(s, 0, 0), "=ROW()");
        let block = m(0, 0, 9, 0);
        let mut used = MAX_ARRAY_CELLS - 5;
        assert!(!load_array(&mut wb, s, &block, &mut used));
        assert_eq!(used, MAX_ARRAY_CELLS - 5);
        let (_, f) = wb.formula_at(CellAddr::new(s, 0, 0)).unwrap();
        assert!(f.array.is_none() && f.array_result.is_none());
        let mut used = 0;
        assert!(load_array(&mut wb, s, &block, &mut used));
        assert_eq!(used, 10);
        let (_, f) = wb.formula_at(CellAddr::new(s, 0, 0)).unwrap();
        assert_eq!(f.array, Some((10, 1)));
    }

    /// HIGH-1: 1M merges (one per row) took 128 s; now sorting dominates.
    #[test]
    fn clean_merges_is_n_log_n() {
        let many: Vec<CellRangeLocal> = (0..1_000_000u32).map(|r| m(r, 0, r, 1)).collect();
        let t = std::time::Instant::now();
        let out = clean_merges(&many);
        assert_eq!(out.len(), MAX_MERGES_PER_SHEET);
        // 200k overlapping copies of one merge collapse to one.
        let same: Vec<CellRangeLocal> = (0..200_000).map(|_| m(0, 0, 3, 3)).collect();
        assert_eq!(clean_merges(&same).len(), 1);
        let took = t.elapsed();
        assert!(took < std::time::Duration::from_secs(3), "{took:?}");
    }

    #[test]
    fn round_trip_values_formulas_styles() {
        let bold = StyleData {
            font: FontData {
                bold: true,
                ..FontData::default()
            },
            ..StyleData::default()
        };
        let data = WorkbookData {
            sheets: vec![
                SheetData {
                    name: "Orders".into(),
                    cells: vec![
                        CellData {
                            row: 0,
                            col: 0,
                            content: CellContentData::Value(Value::Number(2.0)),
                            style: Some(0),
                        },
                        CellData {
                            row: 0,
                            col: 1,
                            content: CellContentData::Value(Value::Number(3.0)),
                            style: None,
                        },
                        CellData {
                            row: 1,
                            col: 0,
                            content: CellContentData::Formula {
                                text: "of:=SUM([.A1:.B1])+[$Other.A1]".into(),
                                cached: Some(Value::Number(0.0)),
                            },
                            style: None,
                        },
                    ],
                    ..SheetData::default()
                },
                SheetData {
                    name: "Other".into(),
                    cells: vec![CellData {
                        row: 0,
                        col: 0,
                        content: CellContentData::Value(Value::Number(10.0)),
                        style: None,
                    }],
                    ..SheetData::default()
                },
            ],
            styles: vec![bold],
            ..WorkbookData::default()
        };
        let mut wb = Workbook::from_data(&data);
        recalc(&mut wb);
        let s1 = wb.sheets[0].id;
        assert_eq!(wb.value(CellAddr::new(s1, 1, 0)), Value::Number(15.0));
        let out = wb.to_data();
        assert_eq!(out.sheets[0].cells.len(), 3);
        match &out.sheets[0].cells[2].content {
            CellContentData::Formula { text, cached } => {
                assert_eq!(&**text, "of:=SUM([.A1:.B1])+[$Other.A1]");
                assert_eq!(cached, &Some(Value::Number(15.0)));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(out.styles.len(), 1);
        assert!(out.styles[0].font.bold);
        let again = Workbook::from_data(&out);
        let mut again = again;
        recalc(&mut again);
        assert_eq!(again.to_data(), out);
    }
}
