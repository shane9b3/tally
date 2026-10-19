//! Conversion between the engine [`Workbook`] and the `tally-model`
//! interchange ([`WorkbookData`]) used by the file readers and writers.
//!
//! Formulas are exchanged as OpenFormula text (`of:=SUM([.A1:.A3])`) with
//! their cached values. Styled empty cells are written only inside the used
//! area (plus short runs), so a bold whole column does not become a million
//! cells.

use std::collections::HashMap;
use std::sync::Arc;

use tally_formula::{parse, print, Dialect, Expr, Formula};
use tally_model::{CellContentData, CellData, DocSettings, SheetData, StyleData, WorkbookData};
use tally_types::{CellAddr, Value, MAX_ROWS};

use crate::formula_cells::CellState;
use crate::storage::{CellContent, ColumnStore, DimRuns, StyleId};
use crate::workbook::{Pending, Workbook};

/// Style runs longer than this are clipped to the used area when exported.
const LONG_STYLE_RUN: u32 = 10_000;

impl Workbook {
    /// Builds a workbook from interchange data. Values are taken as they are.
    pub fn from_data(data: &WorkbookData) -> Workbook {
        Self::from_data_with(data, false)
    }

    /// Builds a workbook; with `recognize`, text values are treated as typed
    /// input (CSV import: numbers, dates and `=formulas` are recognised).
    /// Formula cells are left dirty: call [`crate::recalc::recalc`] next.
    pub fn from_data_with(data: &WorkbookData, recognize: bool) -> Workbook {
        let mut wb = Workbook::empty();
        wb.settings = data.settings.calc_settings.clone();
        wb.meta = data.meta.clone();
        let mut ids = Vec::with_capacity(data.sheets.len());
        for sd in &data.sheets {
            let id = wb.add_sheet(Some(&sd.name), None);
            if let Some(s) = wb.sheet_mut(id) {
                s.hidden = sd.hidden;
                s.tab_color = sd.tab_color;
                s.col_dims = DimRuns::from_runs(&sd.col_widths);
                s.row_dims = DimRuns::from_runs(&sd.row_heights);
            }
            ids.push(id);
        }
        if ids.is_empty() {
            ids.push(wb.add_sheet(Some("Sheet1"), None));
        }
        let style_ids: Vec<StyleId> = data
            .styles
            .iter()
            .map(|s| wb.styles.intern(s.clone()))
            .collect();
        for (sd, &sheet) in data.sheets.iter().zip(&ids) {
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
                    CellContentData::Value(Value::Text(t)) if recognize => {
                        wb.set_input(addr, t);
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
            let used = sheet.used_range();
            let mut cells: Vec<CellData> = Vec::new();
            for (ci, col) in sheet.columns.iter().enumerate() {
                let c = ci as u32;
                for (r, content) in col.cells.iter_range(0, MAX_ROWS - 1) {
                    let addr = CellAddr::new(sheet.id, r, c);
                    let content = match content {
                        CellContent::Value(v) => CellContentData::Value(v.clone()),
                        CellContent::Formula(id) => match self.formulas.get(*id) {
                            Some(f) => {
                                let text = match &f.invalid_text {
                                    Some(raw) => format!("of:{raw}"),
                                    None => print(&f.formula, addr, Dialect::open_formula(), self),
                                };
                                let cached =
                                    (f.state == CellState::Clean).then(|| f.cached.clone());
                                CellContentData::Formula { text, cached }
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
                    let e = if e - s >= LONG_STYLE_RUN {
                        match used {
                            Some(u) => e.min(u.r1.max(s)),
                            None => s,
                        }
                    } else {
                        e
                    };
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
            for cell in &mut cells {
                if let Some(raw) = cell.style {
                    let id = StyleId(raw);
                    let next = styles.len() as u32;
                    let idx = *style_map.entry(id).or_insert_with(|| {
                        styles.push(self.styles.get(id).data.clone());
                        next
                    });
                    cell.style = Some(idx);
                }
            }
            sheets.push(SheetData {
                name: sheet.name.to_string(),
                cells,
                col_widths: sheet.col_dims.runs().to_vec(),
                row_heights: sheet.row_dims.runs().to_vec(),
                merges: Vec::new(),
                frozen: None,
                tab_color: sheet.tab_color,
                hidden: sheet.hidden,
            });
        }
        WorkbookData {
            sheets,
            styles,
            names: Vec::new(),
            settings: DocSettings {
                active_sheet: self.tab_of(self.active_sheet).unwrap_or(0) as u32,
                calc_settings: self.settings.clone(),
            },
            meta: self.meta.clone(),
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
                assert_eq!(text, "of:=SUM([.A1:.B1])+[$Other.A1]");
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
