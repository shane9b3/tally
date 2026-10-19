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
    CellContentData, CellData, CellRangeLocal, DocSettings, NameData, SheetData, StyleData,
    StyleRun, WorkbookData,
};
use tally_types::{CellAddr, Matrix, Value, MAX_COLS, MAX_ROWS};

use crate::formula_cells::CellState;
use crate::storage::{CellContent, ColumnStore, DimRuns, Sheet, Spans, StyleId};
use crate::workbook::{Pending, SheetNamer, Workbook, MAX_SHEETS};

/// Style runs of at least this many rows are exported as [`StyleRun`]s
/// instead of styled empty cells.
const LONG_STYLE_RUN: u32 = 10_000;

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
        wb.settings = data.settings.calc_settings.clone();
        wb.meta = data.meta.clone();
        // Readers cap the sheet count too; this is the engine's own limit.
        let sheet_data = &data.sheets[..data.sheets.len().min(MAX_SHEETS)];
        let mut ids = Vec::with_capacity(sheet_data.len());
        // All names are made unique in one pass and the sheet index is built
        // once, so loading is linear in the number of sheets.
        let mut namer = SheetNamer::default();
        for (tab, sd) in sheet_data.iter().enumerate() {
            let name = if sd.name.trim().is_empty() {
                namer.claim_numbered(tab as u64 + 1)
            } else {
                namer.claim(&sd.name)
            };
            let id = wb.alloc_sheet_id();
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
        for (sd, &sheet) in sheet_data.iter().zip(&ids) {
            if let Some(s) = wb.sheet_mut(sheet) {
                s.meta.merges = clean_merges(&sd.merges);
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
        let style_ids: Vec<StyleId> = data
            .styles
            .iter()
            .map(|s| wb.styles.intern(s.clone()))
            .collect();
        for (sd, &sheet) in sheet_data.iter().zip(&ids) {
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
                        let f = parse(text, addr, Dialect::open_formula(), &wb);
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
                }
            }
            for block in &sd.array_formulas {
                load_array(&mut wb, sheet, block);
            }
        }
        let active = ids
            .get(data.settings.active_sheet as usize)
            .or_else(|| ids.first())
            .copied()
            .unwrap_or(1);
        wb.active_sheet = active;
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
            },
            meta: self.meta.clone(),
        }
    }
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
        let Ok(f) = parse(&text, base, Dialect::open_formula(), &*wb) else {
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

/// Valid, non-overlapping merges of more than one cell.
fn clean_merges(merges: &[CellRangeLocal]) -> Vec<CellRangeLocal> {
    let mut out: Vec<CellRangeLocal> = Vec::new();
    for m in merges.iter().take(1_000_000) {
        let ok = m.r0 <= m.r1
            && m.c0 <= m.c1
            && m.r1 < MAX_ROWS
            && m.c1 < MAX_COLS
            && (m.r0, m.c0) != (m.r1, m.c1);
        let overlaps =
            |o: &CellRangeLocal| m.r0 <= o.r1 && o.r0 <= m.r1 && m.c0 <= o.c1 && o.c0 <= m.c1;
        if ok && !out.iter().any(overlaps) {
            out.push(*m);
        }
    }
    out
}

/// Turns the formula at a block's anchor into an array formula, keeping the
/// covered cells' values from the file as the cached result.
fn load_array(wb: &mut Workbook, sheet: tally_types::SheetId, b: &CellRangeLocal) {
    if b.r0 > b.r1 || b.c0 > b.c1 || b.r1 >= MAX_ROWS || b.c1 >= MAX_COLS {
        return;
    }
    let (rows, cols) = (b.r1 - b.r0 + 1, b.c1 - b.c0 + 1);
    if u64::from(rows) * u64::from(cols) > crate::ops::MAX_OP_CELLS {
        return;
    }
    let anchor = CellAddr::new(sheet, b.r0, b.c0);
    let Some((_, fc)) = wb.formula_at(anchor) else {
        return;
    };
    if fc.invalid_text.is_some() || fc.array.is_some() {
        return;
    }
    let formula = fc.formula.clone();
    let mut m = Matrix::new_filled(rows, cols, Value::Empty);
    for r in 0..rows {
        for c in 0..cols {
            m.set(r, c, wb.value(CellAddr::new(sheet, b.r0 + r, b.c0 + c)));
        }
    }
    // Other blocks overlapping this one are dropped first.
    for other in wb.array_blocks_in(&tally_types::CellRange::new(sheet, b.r0, b.c0, b.r1, b.c1)) {
        if other.start() != anchor {
            return;
        }
    }
    wb.set_array_formula(anchor, formula, rows, cols);
    if let Some((id, _)) = wb.formula_at(anchor) {
        if let Some(fc) = wb.formulas.get_mut(id) {
            fc.cached = m.get(0, 0).cloned().unwrap_or(Value::Empty);
            fc.array_result = Some(Arc::new(m));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recalc::recalc;
    use tally_model::FontData;

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
