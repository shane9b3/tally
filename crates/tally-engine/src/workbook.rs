//! The workbook: sheets in tab order, styles, formula cells, the dependency
//! index and settings. Cell-level mutators record what changed so the next
//! [`crate::recalc`] knows where to start.

use std::collections::HashMap;
use std::sync::Arc;

use tally_formula::{
    parse, print, Dialect, Expr, Formula, FuncId as AstFuncId, FuncRef, NameId, ResolvedRef,
    Resolver,
};
use tally_functions::{registry, FnFlags, FuncId};
use tally_model::DocMeta;
use tally_numfmt::{recognize_input_with, RecognizeContext, EN_US};
use tally_types::{CalcSettings, CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::deps::{DepIndex, HashDepIndex};
use crate::formula_cells::{CellState, FormulaCell, FormulaCellId, FormulaCells};
use crate::storage::{CellContent, ColumnStore, Sheet, StyleId};
use crate::styles::StyleTable;

/// Content of one cell as captured for undo and the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapContent {
    /// A constant.
    Value(Value),
    /// A formula (relative AST) and, for unparsable input, the raw text.
    Formula {
        /// The formula.
        formula: Arc<Formula>,
        /// Raw text of an invalid formula.
        invalid_text: Option<Arc<str>>,
    },
}

/// A small xorshift64* generator for RAND (the `rand` crate is not a dependency).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// Seeded from the clock.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self::new(nanos)
    }

    /// Seeded with `seed` (0 is replaced by a constant).
    pub fn new(seed: u64) -> Self {
        let s = seed ^ 0x9E37_79B9_7F4A_7C15;
        Rng(if s == 0 { 0x2545_F491_4F6C_DD1D } else { s })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// What changed since the last recalc.
#[derive(Clone, Debug, Default)]
pub struct Pending {
    /// Edited cells.
    pub cells: Vec<CellAddr>,
    /// Edited ranges (clears, pastes, style changes).
    pub ranges: Vec<CellRange>,
    /// Formula cells created dirty.
    pub dirty: Vec<FormulaCellId>,
    /// Ranges whose display changed without a value change (styles).
    pub display_only: Vec<CellRange>,
}

/// The workbook.
#[derive(Clone, Debug)]
pub struct Workbook {
    /// Sheets in tab order.
    pub sheets: Vec<Sheet>,
    /// Sheet id → tab index.
    pub sheet_index: HashMap<SheetId, usize>,
    next_sheet_id: SheetId,
    /// Styles.
    pub styles: StyleTable,
    /// Calculation settings.
    pub settings: CalcSettings,
    /// Formula cells.
    pub formulas: FormulaCells,
    /// Reverse dependency index.
    pub deps: HashDepIndex,
    /// Random numbers for RAND/RANDBETWEEN.
    pub rng: Rng,
    /// Changes since the last recalc.
    pub pending: Pending,
    /// Document metadata.
    pub meta: DocMeta,
    /// Id of the active sheet.
    pub active_sheet: SheetId,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    /// A workbook with one sheet, "Sheet1".
    pub fn new() -> Self {
        let mut wb = Self::empty();
        wb.add_sheet(Some("Sheet1"), None);
        wb
    }

    /// A workbook without sheets (for loading).
    pub fn empty() -> Self {
        Self {
            sheets: Vec::new(),
            sheet_index: HashMap::new(),
            next_sheet_id: 1,
            styles: StyleTable::new(),
            settings: CalcSettings::default(),
            formulas: FormulaCells::with_key(),
            deps: HashDepIndex::new(),
            rng: Rng::from_time(),
            pending: Pending::default(),
            meta: DocMeta::default(),
            active_sheet: 1,
        }
    }

    // ----- sheets ---------------------------------------------------------

    fn reindex(&mut self) {
        self.sheet_index = self
            .sheets
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, i))
            .collect();
    }

    /// The sheet with id `id`.
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheet_index.get(&id).and_then(|i| self.sheets.get(*i))
    }

    /// The sheet with id `id`, mutably.
    pub fn sheet_mut(&mut self, id: SheetId) -> Option<&mut Sheet> {
        let i = *self.sheet_index.get(&id)?;
        self.sheets.get_mut(i)
    }

    /// Tab index of a sheet.
    pub fn tab_of(&self, id: SheetId) -> Option<usize> {
        self.sheet_index.get(&id).copied()
    }

    /// Sheet id by (case-insensitive) name.
    pub fn sheet_by_name_ci(&self, name: &str) -> Option<SheetId> {
        self.sheets
            .iter()
            .find(|s| {
                s.name.eq_ignore_ascii_case(name) || s.name.to_lowercase() == name.to_lowercase()
            })
            .map(|s| s.id)
    }

    /// The next free `SheetN` name.
    pub fn next_sheet_name(&self) -> String {
        let mut n = self.sheets.len() + 1;
        loop {
            let name = format!("Sheet{n}");
            if self.sheet_by_name_ci(&name).is_none() {
                return name;
            }
            n += 1;
        }
    }

    /// `name` made unique among the sheets by appending `_2`, `_3`, …
    pub fn unique_sheet_name(&self, name: &str, except: Option<SheetId>) -> String {
        let taken = |n: &str| {
            self.sheet_by_name_ci(n)
                .is_some_and(|id| Some(id) != except)
        };
        if !taken(name) {
            return name.to_owned();
        }
        let mut i = 2;
        loop {
            let cand = format!("{name}_{i}");
            if !taken(&cand) {
                return cand;
            }
            i += 1;
        }
    }

    /// Adds a sheet (name made unique) at tab `index` (appends when `None`).
    pub fn add_sheet(&mut self, name: Option<&str>, index: Option<usize>) -> SheetId {
        let id = self.next_sheet_id;
        self.next_sheet_id += 1;
        let name = match name {
            Some(n) if !n.trim().is_empty() => self.unique_sheet_name(n, None),
            _ => self.next_sheet_name(),
        };
        let at = index.unwrap_or(self.sheets.len()).min(self.sheets.len());
        self.sheets.insert(at, Sheet::new(id, &name));
        self.reindex();
        if self.sheets.len() == 1 {
            self.active_sheet = id;
        }
        id
    }

    /// Inserts a previously removed sheet with its id at `index`.
    pub fn insert_sheet(&mut self, sheet: Sheet, index: usize) {
        if sheet.id >= self.next_sheet_id {
            self.next_sheet_id = sheet.id + 1;
        }
        let at = index.min(self.sheets.len());
        self.sheets.insert(at, sheet);
        self.reindex();
    }

    /// Removes a sheet (and its formula cells) and returns it.
    pub fn remove_sheet(&mut self, id: SheetId) -> Option<Sheet> {
        let i = self.tab_of(id)?;
        let sheet = self.sheets.remove(i);
        self.reindex();
        for col in &sheet.columns {
            for (_, c) in col.cells.iter_range(0, MAX_ROWS - 1) {
                if let CellContent::Formula(fid) = c {
                    self.deps.remove(*fid);
                    self.formulas.remove(*fid);
                }
            }
        }
        if self.active_sheet == id {
            self.active_sheet = self
                .sheets
                .get(i.min(self.sheets.len().saturating_sub(1)))
                .map_or(0, |s| s.id);
        }
        Some(sheet)
    }

    /// Moves a sheet to tab `to`; returns the old tab index.
    pub fn move_sheet(&mut self, id: SheetId, to: usize) -> Option<usize> {
        let from = self.tab_of(id)?;
        let s = self.sheets.remove(from);
        let to = to.min(self.sheets.len());
        self.sheets.insert(to, s);
        self.reindex();
        Some(from)
    }

    /// Re-resolves the precedents of formulas with 3-D references (after
    /// tab-order changes, which move what a 3-D span covers).
    pub fn refresh_all_precedents(&mut self) {
        let ids: Vec<FormulaCellId> = self
            .formulas
            .iter()
            .filter(|(_, f)| f.has_3d)
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            let Some(fc) = self.formulas.get(id) else {
                continue;
            };
            let p = self.precedents_of(&fc.formula, fc.addr);
            if p != fc.precedents {
                let dynamic = fc.dynamic_deps.clone();
                self.deps.remove(id);
                let mut all = p.clone();
                all.extend(dynamic);
                self.deps.add(id, &all);
                let vol = fc.volatile;
                self.deps.set_volatile(id, vol);
                if let Some(fc) = self.formulas.get_mut(id) {
                    fc.precedents = p;
                }
            }
        }
    }

    // ----- cells ----------------------------------------------------------

    /// The value of a cell (the cached result for formulas).
    pub fn value(&self, addr: CellAddr) -> Value {
        match self
            .sheet(addr.sheet)
            .and_then(|s| s.get(addr.row, addr.col))
        {
            None => Value::Empty,
            Some(CellContent::Value(v)) => v.clone(),
            Some(CellContent::Formula(id)) => self
                .formulas
                .get(*id)
                .map_or(Value::Empty, |f| f.cached.clone()),
        }
    }

    /// The formula cell at `addr`.
    pub fn formula_at(&self, addr: CellAddr) -> Option<(FormulaCellId, &FormulaCell)> {
        match self.sheet(addr.sheet)?.get(addr.row, addr.col)? {
            CellContent::Formula(id) => self.formulas.get(*id).map(|f| (*id, f)),
            CellContent::Value(_) => None,
        }
    }

    /// The style of a cell.
    pub fn style_at(&self, addr: CellAddr) -> StyleId {
        self.sheet(addr.sheet)
            .map_or(StyleId::DEFAULT, |s| s.style(addr.row, addr.col))
    }

    /// Captures the content of a cell.
    pub fn snapshot_content(&self, addr: CellAddr) -> Option<SnapContent> {
        match self.sheet(addr.sheet)?.get(addr.row, addr.col)? {
            CellContent::Value(v) => Some(SnapContent::Value(v.clone())),
            CellContent::Formula(id) => self.formulas.get(*id).map(|f| SnapContent::Formula {
                formula: f.formula.clone(),
                invalid_text: f.invalid_text.clone(),
            }),
        }
    }

    /// Restores captured content (or clears with `None`).
    pub fn restore_content(&mut self, addr: CellAddr, content: Option<SnapContent>) {
        match content {
            None => self.clear_content(addr),
            Some(SnapContent::Value(v)) => self.set_value(addr, v),
            Some(SnapContent::Formula {
                formula,
                invalid_text,
            }) => self.set_formula(addr, formula, invalid_text),
        }
    }

    fn drop_formula(&mut self, content: Option<CellContent>) {
        if let Some(CellContent::Formula(id)) = content {
            self.deps.remove(id);
            self.formulas.remove(id);
        }
    }

    /// Removes the content of a cell.
    pub fn clear_content(&mut self, addr: CellAddr) {
        let old = self
            .sheet_mut(addr.sheet)
            .and_then(|s| s.remove(addr.row, addr.col));
        if old.is_some() {
            self.pending.cells.push(addr);
        }
        self.drop_formula(old);
    }

    /// Stores a constant (`Empty` clears).
    pub fn set_value(&mut self, addr: CellAddr, v: Value) {
        if v.is_empty() {
            self.clear_content(addr);
            return;
        }
        let v = match v {
            Value::Array(m) => m.get(0, 0).cloned().unwrap_or(Value::Empty),
            other => other,
        };
        let Some(sheet) = self.sheet_mut(addr.sheet) else {
            return;
        };
        let old = sheet.set(addr.row, addr.col, CellContent::Value(v));
        self.pending.cells.push(addr);
        self.drop_formula(old);
    }

    /// The static precedents of `f` at `addr`, 3-D spans expanded by tab order.
    pub fn precedents_of(&self, f: &Formula, addr: CellAddr) -> Vec<CellRange> {
        let mut out = Vec::new();
        for r in f.static_precedents(addr) {
            match r {
                ResolvedRef::Range(cr) => out.push(cr),
                ResolvedRef::Span3d { first, last, area } => {
                    let (Some(a), Some(b)) = (self.tab_of(first), self.tab_of(last)) else {
                        continue;
                    };
                    let (lo, hi) = (a.min(b), a.max(b));
                    for s in self.sheets.get(lo..=hi).into_iter().flatten() {
                        out.push(CellRange {
                            sheet: s.id,
                            ..area
                        });
                    }
                }
                ResolvedRef::Invalid => {}
            }
        }
        out.sort_by_key(|r| (r.sheet, r.r0, r.c0, r.r1, r.c1));
        out.dedup();
        out
    }

    /// Stores a formula; the cell becomes dirty.
    pub fn set_formula(
        &mut self,
        addr: CellAddr,
        formula: Arc<Formula>,
        invalid_text: Option<Arc<str>>,
    ) {
        if self.sheet(addr.sheet).is_none() || addr.row >= MAX_ROWS || addr.col >= MAX_COLS {
            return;
        }
        let volatile = is_volatile(&formula);
        let precedents = self.precedents_of(&formula, addr);
        let has_3d = formula
            .static_precedents(addr)
            .iter()
            .any(|r| matches!(r, ResolvedRef::Span3d { .. }));
        let id = self.formulas.insert(FormulaCell {
            addr,
            formula,
            cached: Value::Empty,
            state: CellState::Dirty,
            volatile,
            invalid_text,
            precedents: precedents.clone(),
            dynamic_deps: Vec::new(),
            has_3d,
        });
        self.deps.add(id, &precedents);
        self.deps.set_volatile(id, volatile);
        let old = self
            .sheet_mut(addr.sheet)
            .and_then(|s| s.set(addr.row, addr.col, CellContent::Formula(id)));
        self.drop_formula(old);
        self.pending.cells.push(addr);
        self.pending.dirty.push(id);
    }

    /// Parses `text` (CalcA1, en-US) as a formula at `addr`.
    pub fn parse_formula(
        &self,
        text: &str,
        addr: CellAddr,
    ) -> Result<Formula, tally_formula::ParseError> {
        parse(text, addr, Dialect::calc_a1_en_us(), self)
    }

    /// Stores what the user typed: a formula (`=…`), or a value through the
    /// input recognizer, applying its implied number format to cells whose
    /// format is General. A leading `'` forces text; cells formatted `@`
    /// keep the input as text.
    pub fn set_input(&mut self, addr: CellAddr, input: &str) {
        if input.is_empty() {
            self.clear_content(addr);
            return;
        }
        if input.starts_with('=') && input.len() > 1 {
            match self.parse_formula(input, addr) {
                Ok(f) => self.set_formula(addr, Arc::new(f), None),
                Err(e) => self.set_formula(
                    addr,
                    Arc::new(Formula::new(Expr::Err(e.kind))),
                    Some(input.into()),
                ),
            }
            return;
        }
        let style = self.style_at(addr);
        if self.styles.get(style).is_text_format() {
            self.set_value(addr, Value::text(input));
            return;
        }
        let ctx = RecognizeContext::now(self.settings.date_epoch);
        let rec = recognize_input_with(input, &EN_US, &ctx);
        self.set_value(addr, rec.value);
        if let Some(code) = rec.implied_format {
            if self.styles.get(style).is_general() {
                let new = self.styles.with_format(style, &code);
                if let Some(s) = self.sheet_mut(addr.sheet) {
                    if let Some(col) = s.column_mut(addr.col) {
                        col.styles.set(addr.row, addr.row, new);
                    }
                }
            }
        }
    }

    /// Text for the formula bar: the formula in the UI dialect, or the raw text.
    pub fn formula_text(&self, id: FormulaCellId) -> Option<String> {
        let f = self.formulas.get(id)?;
        if let Some(t) = &f.invalid_text {
            return Some(t.to_string());
        }
        Some(print(&f.formula, f.addr, Dialect::calc_a1_en_us(), self))
    }

    /// Marks every formula dirty (hard recalc).
    pub fn dirty_all(&mut self) {
        let ids: Vec<FormulaCellId> = self.formulas.keys().collect();
        for id in &ids {
            if let Some(f) = self.formulas.get_mut(*id) {
                f.state = CellState::Dirty;
            }
        }
        self.pending.dirty.extend(ids);
    }

    /// Number of non-empty cells in the workbook.
    pub fn cell_count(&self) -> usize {
        self.sheets.iter().map(Sheet::cell_count).sum()
    }
}

/// `true` if the formula calls a volatile function.
pub fn is_volatile(f: &Formula) -> bool {
    let reg = registry();
    let mut v = false;
    f.expr.walk(|e| {
        if let Expr::Call {
            func: FuncRef::Known(id),
            ..
        } = e
        {
            if reg
                .get(FuncId(id.0))
                .is_some_and(|d| d.flags.contains(FnFlags::VOLATILE))
            {
                v = true;
            }
        }
    });
    v
}

impl Resolver for Workbook {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.sheet_by_name_ci(name)
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.sheet(id).map(|s| s.name.clone())
    }

    fn name(&self, _name: &str, _sheet: SheetId) -> Option<NameId> {
        None
    }

    fn function(&self, name: &str) -> Option<AstFuncId> {
        registry().lookup_any(name).map(|id| AstFuncId(id.0))
    }

    fn function_name(&self, id: AstFuncId) -> Option<Arc<str>> {
        registry().get(FuncId(id.0)).map(|d| Arc::from(d.name))
    }

    fn function_name_in(&self, id: AstFuncId, dialect: Dialect) -> Option<Arc<str>> {
        let d = registry().get(FuncId(id.0))?;
        Some(Arc::from(if dialect.is_open_formula() {
            d.odf_name
        } else {
            d.name
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_and_names() {
        let mut wb = Workbook::new();
        assert_eq!(wb.sheets.len(), 1);
        let s2 = wb.add_sheet(None, None);
        assert_eq!(
            wb.sheet(s2).map(|s| s.name.to_string()),
            Some("Sheet2".into())
        );
        let s3 = wb.add_sheet(Some("sheet2"), Some(0));
        assert_eq!(
            wb.sheet(s3).map(|s| s.name.to_string()),
            Some("sheet2_2".into())
        );
        assert_eq!(wb.tab_of(s3), Some(0));
        assert_eq!(wb.move_sheet(s3, 5), Some(0));
        assert_eq!(wb.tab_of(s3), Some(2));
        assert!(wb.remove_sheet(s2).is_some());
        let s4 = wb.add_sheet(None, None);
        assert!(s4 > s3, "ids are never reused");
    }

    #[test]
    fn rng_in_unit_interval() {
        let mut r = Rng::new(42);
        for _ in 0..1000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
