//! Dirty propagation and recalculation (plan §3.6).
//!
//! 1. Seeds: the cells edited since the last recalc, coalesced into
//!    rectangles, plus new formula cells and every volatile formula.
//! 2. Dirty propagation: breadth-first over the reverse index, level by
//!    level (each level coalesced), stopping at formulas already dirty.
//! 3. Ordering: an iterative Tarjan over the dirty subgraph, whose edges are
//!    the static precedents (plus recorded dynamic ones) that are dirty
//!    formula cells. Strongly connected components come out precedents
//!    first; every member of a cycle gets Err:522 (iterations are off in
//!    Phase A).
//! 4. Evaluation in that order with [`CalcCtx`]. A dirty precedent met
//!    during evaluation (only possible through dynamic references) is
//!    evaluated on demand, bounded by [`MAX_ON_DEMAND_DEPTH`].

use std::ops::ControlFlow;

use slotmap::SecondaryMap;
use tally_formula::{Expr, ResolvedRef};
use tally_functions::{CellFormatInfo, EvalContext};
use tally_numfmt::date::now_serial;
use tally_types::{
    CalcSettings, CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS,
};

use crate::deps::{coalesce, DepIndex};
use crate::eval::{cell_result, eval_expr};
use crate::formula_cells::{CellState, FormulaCellId};
use crate::storage::CellContent;
use crate::workbook::Workbook;

/// Nesting limit for on-demand evaluation of dirty precedents.
pub const MAX_ON_DEMAND_DEPTH: u32 = 16;

/// What a recalc did.
#[derive(Clone, Debug, Default)]
pub struct RecalcReport {
    /// Edited cells and ranges plus every recalculated formula, coalesced.
    pub changed: Vec<CellRange>,
    /// Number of formulas evaluated.
    pub evaluated: usize,
    /// Number of formulas in cycles.
    pub circular: usize,
}

/// Recalculates everything that depends on the pending changes.
pub fn recalc(wb: &mut Workbook) -> RecalcReport {
    let pending = std::mem::take(&mut wb.pending);
    let mut dirty: Vec<FormulaCellId> = Vec::new();
    for id in pending.dirty {
        if let Some(f) = wb.formulas.get_mut(id) {
            f.state = CellState::Dirty;
            dirty.push(id);
        }
    }
    for id in wb.deps.volatile() {
        if let Some(f) = wb.formulas.get_mut(id) {
            if f.state == CellState::Clean {
                f.state = CellState::Dirty;
            }
            dirty.push(id);
        }
    }
    let mut seed_cells: Vec<CellAddr> = pending.cells.clone();
    seed_cells.extend(
        dirty
            .iter()
            .filter_map(|id| wb.formulas.get(*id).map(|f| f.addr)),
    );
    let mut frontier = coalesce(&seed_cells, &pending.ranges);
    let mut found: Vec<FormulaCellId> = Vec::new();
    while !frontier.is_empty() {
        found.clear();
        wb.deps.dependents_of(&frontier, &mut found);
        let mut next: Vec<CellAddr> = Vec::new();
        for id in found.drain(..) {
            if let Some(f) = wb.formulas.get_mut(id) {
                if f.state == CellState::Clean {
                    f.state = CellState::Dirty;
                    dirty.push(id);
                    next.push(f.addr);
                }
            }
        }
        frontier = coalesce(&next, &[]);
    }
    dirty.sort();
    dirty.dedup();
    dirty.retain(|id| {
        wb.formulas
            .get(*id)
            .is_some_and(|f| f.state == CellState::Dirty)
    });

    let order = topo_order(wb, &dirty);
    let mut report = RecalcReport::default();
    let now = now_serial(wb.settings.date_epoch);
    let mut recalculated: Vec<CellAddr> = Vec::with_capacity(dirty.len());
    for scc in order {
        if scc.cyclic {
            for id in &scc.members {
                if let Some(f) = wb.formulas.get_mut(*id) {
                    f.cached = Value::Error(ErrorKind::Circular);
                    f.state = CellState::Clean;
                    recalculated.push(f.addr);
                    report.circular += 1;
                }
            }
            continue;
        }
        for id in scc.members {
            let Some(f) = wb.formulas.get(id) else {
                continue;
            };
            recalculated.push(f.addr);
            if f.state != CellState::Dirty {
                continue;
            }
            let mut ctx = CalcCtx::new(wb, now);
            ctx.evaluate(id);
            report.evaluated += 1;
        }
    }
    let mut cells = pending.cells;
    cells.extend(recalculated);
    let mut ranges = pending.ranges;
    ranges.extend(pending.display_only);
    report.changed = coalesce(&cells, &ranges);
    report
}

/// Marks every formula dirty and recalculates.
pub fn hard_recalc(wb: &mut Workbook) -> RecalcReport {
    wb.dirty_all();
    recalc(wb)
}

/// One strongly connected component, in evaluation order.
#[derive(Debug)]
struct Scc {
    members: Vec<FormulaCellId>,
    cyclic: bool,
}

/// The dirty formula cells inside `range`.
fn dirty_formulas_in(wb: &Workbook, range: &CellRange, out: &mut Vec<FormulaCellId>) {
    let Some(sheet) = wb.sheet(range.sheet) else {
        return;
    };
    let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
    if sheet.columns.is_empty() {
        return;
    }
    for c in range.c0..=c1 {
        let Some(col) = sheet.column(c) else {
            continue;
        };
        if col.formula_rows.is_empty() {
            continue;
        }
        for r in col.formula_rows.range(range.r0..=range.r1) {
            if let Some(CellContent::Formula(id)) = col.cells_get(*r) {
                if wb
                    .formulas
                    .get(*id)
                    .is_some_and(|f| f.state == CellState::Dirty)
                {
                    out.push(*id);
                }
            }
        }
    }
}

/// Iterative Tarjan over the dirty subgraph; SCCs come out precedents first.
fn topo_order(wb: &Workbook, dirty: &[FormulaCellId]) -> Vec<Scc> {
    const UNSEEN: usize = usize::MAX;
    let n = dirty.len();
    let mut local: SecondaryMap<FormulaCellId, usize> = SecondaryMap::new();
    for (i, id) in dirty.iter().enumerate() {
        local.insert(*id, i);
    }
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut succ: Vec<Option<Vec<usize>>> = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Scc> = Vec::new();
    let mut counter = 0usize;
    let mut scratch: Vec<FormulaCellId> = Vec::new();

    let edges = |v: usize, scratch: &mut Vec<FormulaCellId>| -> Vec<usize> {
        scratch.clear();
        if let Some(f) = wb.formulas.get(dirty[v]) {
            for r in f.precedents.iter().chain(f.dynamic_deps.iter()) {
                dirty_formulas_in(wb, r, scratch);
            }
        }
        let mut s: Vec<usize> = scratch
            .iter()
            .filter_map(|id| local.get(*id).copied())
            .collect();
        s.sort_unstable();
        s.dedup();
        s
    };

    for start in 0..n {
        if index[start] != UNSEEN {
            continue;
        }
        let mut call: Vec<(usize, usize)> = Vec::new();
        index[start] = counter;
        low[start] = counter;
        counter += 1;
        stack.push(start);
        on_stack[start] = true;
        succ[start] = Some(edges(start, &mut scratch));
        call.push((start, 0));
        while let Some(&(v, pos)) = call.last() {
            let next = succ[v].as_ref().and_then(|s| s.get(pos).copied());
            if let Some(w) = next {
                if let Some(top) = call.last_mut() {
                    top.1 += 1;
                }
                if index[w] == UNSEEN {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    succ[w] = Some(edges(w, &mut scratch));
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    members.push(dirty[w]);
                    if w == v {
                        break;
                    }
                }
                let cyclic = members.len() > 1
                    || succ[v]
                        .as_ref()
                        .is_some_and(|s| s.binary_search(&v).is_ok());
                out.push(Scc { members, cyclic });
            }
        }
    }
    out
}

/// The engine's [`EvalContext`].
pub struct CalcCtx<'w> {
    wb: &'w mut Workbook,
    origin: CellAddr,
    now: f64,
    depth: u32,
    dyn_deps: Vec<CellRange>,
}

impl<'w> CalcCtx<'w> {
    /// A context over `wb` with the recalc's fixed `now`.
    pub fn new(wb: &'w mut Workbook, now: f64) -> Self {
        Self {
            wb,
            origin: CellAddr::new(0, 0, 0),
            now,
            depth: 0,
            dyn_deps: Vec::new(),
        }
    }

    /// A context for evaluating an expression at `origin` (no cell is written).
    pub fn at(wb: &'w mut Workbook, origin: CellAddr) -> Self {
        let now = now_serial(wb.settings.date_epoch);
        let mut c = Self::new(wb, now);
        c.origin = origin;
        c
    }

    /// Evaluates formula cell `id` and stores its result.
    pub fn evaluate(&mut self, id: FormulaCellId) {
        let Some(f) = self.wb.formulas.get_mut(id) else {
            return;
        };
        f.state = CellState::Evaluating;
        let formula = f.formula.clone();
        let addr = f.addr;
        let saved_origin = std::mem::replace(&mut self.origin, addr);
        let saved_dyn = std::mem::take(&mut self.dyn_deps);
        self.depth += 1;
        let v = eval_expr(&formula.expr, self);
        let value = cell_result(&v, self);
        self.depth -= 1;
        self.origin = saved_origin;
        let dynamic = std::mem::replace(&mut self.dyn_deps, saved_dyn);
        let Some(f) = self.wb.formulas.get_mut(id) else {
            return;
        };
        f.cached = value;
        f.state = CellState::Clean;
        if dynamic != f.dynamic_deps {
            f.dynamic_deps = dynamic;
            let mut all = f.precedents.clone();
            all.extend(f.dynamic_deps.iter().copied());
            let vol = f.volatile;
            self.wb.deps.remove(id);
            self.wb.deps.add(id, &all);
            self.wb.deps.set_volatile(id, vol);
        }
    }

    fn formula_value(&mut self, id: FormulaCellId) -> Value {
        let Some(f) = self.wb.formulas.get(id) else {
            return Value::Empty;
        };
        match f.state {
            CellState::Clean => f.cached.clone(),
            CellState::Evaluating => Value::Error(ErrorKind::Circular),
            CellState::Dirty => {
                if self.depth > MAX_ON_DEMAND_DEPTH {
                    return Value::Error(ErrorKind::Circular);
                }
                self.evaluate(id);
                self.wb
                    .formulas
                    .get(id)
                    .map_or(Value::Empty, |f| f.cached.clone())
            }
        }
    }

    fn ensure_clean(&mut self, range: &CellRange) {
        let mut ids = Vec::new();
        dirty_formulas_in(self.wb, range, &mut ids);
        for id in ids {
            let _ = self.formula_value(id);
        }
    }
}

impl crate::storage::Column {
    /// The content at `row` (shorthand over the column store).
    pub fn cells_get(&self, row: u32) -> Option<&CellContent> {
        use crate::storage::ColumnStore;
        self.cells.get(row)
    }
}

impl EvalContext for CalcCtx<'_> {
    fn origin(&self) -> CellAddr {
        self.origin
    }

    fn settings(&self) -> &CalcSettings {
        &self.wb.settings
    }

    fn cell_value(&mut self, addr: CellAddr) -> Value {
        if !addr.is_valid() {
            return Value::Error(ErrorKind::Ref);
        }
        let Some(sheet) = self.wb.sheet(addr.sheet) else {
            return Value::Error(ErrorKind::Ref);
        };
        match sheet.get(addr.row, addr.col) {
            None => Value::Empty,
            Some(CellContent::Value(v)) => v.clone(),
            Some(CellContent::Formula(id)) => {
                let id = *id;
                self.formula_value(id)
            }
        }
    }

    fn for_each_in_range(
        &mut self,
        range: &CellRange,
        skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> ControlFlow<()>,
    ) {
        use crate::storage::ColumnStore;
        let r1 = range.r1.min(MAX_ROWS - 1);
        let c1 = range.c1.min(MAX_COLS - 1);
        if range.r0 > r1 || range.c0 > c1 {
            return;
        }
        self.ensure_clean(range);
        let wb = &*self.wb;
        let Some(sheet) = wb.sheet(range.sheet) else {
            return;
        };
        let circular = Value::Error(ErrorKind::Circular);
        let empty = Value::Empty;
        for c in range.c0..=c1 {
            let col = sheet.column(c);
            let value_of = |content: &'_ CellContent| -> Option<Value> {
                match content {
                    CellContent::Value(_) => None,
                    CellContent::Formula(id) => Some(match wb.formulas.get(*id) {
                        Some(fc) if fc.state == CellState::Clean => fc.cached.clone(),
                        Some(_) => circular.clone(),
                        None => Value::Empty,
                    }),
                }
            };
            if skip_empty {
                let Some(col) = col else {
                    continue;
                };
                for (r, content) in col.cells.iter_range(range.r0, r1) {
                    let addr = CellAddr::new(range.sheet, r, c);
                    let flow = match content {
                        CellContent::Value(v) => f(addr, v),
                        other => {
                            let v = value_of(other).unwrap_or(Value::Empty);
                            f(addr, &v)
                        }
                    };
                    if flow.is_break() {
                        return;
                    }
                }
            } else {
                let mut it = col
                    .map(|c| c.cells.iter_range(range.r0, r1))
                    .into_iter()
                    .flatten()
                    .peekable();
                for r in range.r0..=r1 {
                    let addr = CellAddr::new(range.sheet, r, c);
                    let flow = match it.peek() {
                        Some((row, content)) if *row == r => {
                            let content = *content;
                            it.next();
                            match content {
                                CellContent::Value(v) => f(addr, v),
                                other => {
                                    let v = value_of(other).unwrap_or(Value::Empty);
                                    f(addr, &v)
                                }
                            }
                        }
                        _ => f(addr, &empty),
                    };
                    if flow.is_break() {
                        return;
                    }
                }
            }
        }
    }

    fn used_bounds(&self, range: &CellRange) -> Option<CellRange> {
        let used = self.wb.sheet(range.sheet)?.used_range()?;
        used.intersect(range)
    }

    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange> {
        if !a1 {
            return None;
        }
        let src = format!("={}", text.trim());
        let f = self.wb.parse_formula(&src, self.origin).ok()?;
        match &f.expr {
            Expr::Ref(r) => match r.resolve(self.origin) {
                ResolvedRef::Range(cr) if self.wb.sheet(cr.sheet).is_some() => Some(cr),
                _ => None,
            },
            _ => None,
        }
    }

    fn record_dynamic_dep(&mut self, range: &CellRange) {
        if !self.dyn_deps.contains(range) {
            self.dyn_deps.push(*range);
        }
    }

    fn now_serial(&self) -> f64 {
        self.now
    }

    fn rand(&mut self) -> f64 {
        self.wb.rng.next_f64()
    }

    fn sheet_index(&self, id: SheetId) -> Option<u32> {
        self.wb.tab_of(id).map(|i| i as u32)
    }

    fn sheet_count(&self) -> u32 {
        self.wb.sheets.len() as u32
    }

    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo {
        let mut info = CellFormatInfo::default();
        if let Some((id, f)) = self.wb.formula_at(addr) {
            info.has_formula = true;
            info.formula = match &f.invalid_text {
                Some(t) => Some(t.clone()),
                None => self.wb.formula_text(id).map(Into::into),
            };
        }
        let style = self.wb.styles.get(self.wb.style_at(addr));
        if !style.is_general() {
            info.format_code = Some(style.format_code.clone());
        }
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap()
    }

    fn num(wb: &Workbook, s: &str) -> Value {
        wb.value(a(s))
    }

    #[test]
    fn simple_and_diamond() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "1");
        wb.set_input(a("B1"), "=A1+1");
        wb.set_input(a("C1"), "=A1*2");
        wb.set_input(a("D1"), "=B1+C1");
        recalc(&mut wb);
        assert_eq!(num(&wb, "D1"), Value::Number(4.0));
        wb.set_input(a("A1"), "10");
        let rep = recalc(&mut wb);
        assert_eq!(num(&wb, "D1"), Value::Number(31.0));
        assert_eq!(rep.evaluated, 3, "each of B1, C1, D1 once");
    }

    #[test]
    fn sum_of_range_and_whole_column() {
        let mut wb = Workbook::new();
        for r in 1..=10 {
            wb.set_input(a(&format!("A{r}")), &r.to_string());
        }
        wb.set_input(a("B1"), "=SUM(A1:A10)");
        wb.set_input(a("B2"), "=SUM(A:A)");
        recalc(&mut wb);
        assert_eq!(num(&wb, "B1"), Value::Number(55.0));
        assert_eq!(num(&wb, "B2"), Value::Number(55.0));
        wb.set_input(a("A5"), "100");
        recalc(&mut wb);
        assert_eq!(num(&wb, "B1"), Value::Number(150.0));
        assert_eq!(num(&wb, "B2"), Value::Number(150.0));
    }

    #[test]
    fn cycles_get_522_and_recover() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "=B1");
        wb.set_input(a("B1"), "=A1+1");
        wb.set_input(a("C1"), "=A1*2");
        recalc(&mut wb);
        assert_eq!(num(&wb, "A1"), Value::Error(ErrorKind::Circular));
        assert_eq!(num(&wb, "B1"), Value::Error(ErrorKind::Circular));
        assert_eq!(num(&wb, "C1"), Value::Error(ErrorKind::Circular));
        wb.set_input(a("B1"), "5");
        recalc(&mut wb);
        assert_eq!(num(&wb, "A1"), Value::Number(5.0));
        assert_eq!(num(&wb, "C1"), Value::Number(10.0));
    }

    #[test]
    fn self_reference() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "=A1+1");
        recalc(&mut wb);
        assert_eq!(num(&wb, "A1"), Value::Error(ErrorKind::Circular));
    }
}
