//! Dirty propagation and recalculation (plan §3.6).
//!
//! 1. Seeds: the cells edited since the last recalc, coalesced into
//!    rectangles, plus new formula cells and every volatile formula.
//! 2. Dirty propagation: breadth-first over the reverse index, level by
//!    level (each level coalesced), stopping at formulas already dirty.
//! 3. Ordering: an iterative Tarjan over the dirty subgraph, whose edges are
//!    the static precedents (plus recorded dynamic ones) that are dirty
//!    formula cells. Strongly connected components come out precedents
//!    first. Members on a cycle of *strict* precedents (references whose
//!    values are always read: not inside IF/IFERROR/IFNA branches, not the
//!    address-only arguments of ROW/COLUMN) get Err:522 without evaluation
//!    (iterations are off in Phase A).
//! 4. Evaluation in that order with [`CalcCtx`]. A dirty precedent met
//!    during evaluation (through a lazy branch or a dynamic reference) is
//!    evaluated on demand, nested at most [`MAX_ON_DEMAND_DEPTH`] deep and
//!    deferred to an explicit work stack beyond that. Reading a
//!    cell that is being evaluated is a cycle that is actually followed
//!    (AOO detects circularity at interpretation time the same way): every
//!    cell on that cycle gets Err:522.

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slotmap::SecondaryMap;
use tally_formula::{Expr, ResolvedRef};
use tally_functions::{CellFormatInfo, EvalContext};
use tally_numfmt::date::now_serial;
use tally_types::{
    CalcSettings, CellAddr, CellRange, ErrorKind, SheetId, Value, MAX_COLS, MAX_ROWS,
};

use crate::deps::{coalesce, DepIndex};
use crate::eval::{array_result, cell_result, eval_expr, eval_in_array_context};
use crate::formula_cells::{CellState, FormulaCellId};
use crate::storage::CellContent;
use crate::workbook::Workbook;

/// Nesting limit for on-demand evaluation of dirty precedents. Deeper
/// precedents are deferred and evaluated from an explicit work stack (see
/// [`CalcCtx::evaluate`]), so chains of any length work on a small stack.
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
    /// The [`RecalcBudget`] ran out: `skipped` formulas kept their cached
    /// values (from the file, or empty) and were marked clean.
    pub incomplete: bool,
    /// Formulas left unevaluated because the budget ran out.
    pub skipped: usize,
}

/// Limits for one recalculation (the recalc watchdog, plan §5.4 item 8).
///
/// When either limit is hit, the remaining formulas keep their cached values
/// and are marked clean, so later edits recalculate their dependents as
/// usual. The check runs between formulas; one formula is never interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecalcBudget {
    /// Most formulas to evaluate.
    pub max_cells_evaluated: u64,
    /// Wall-clock deadline.
    pub deadline: Option<Instant>,
}

impl RecalcBudget {
    /// Default formula limit when a file is opened.
    pub const OPEN_MAX_CELLS: u64 = 50_000_000;
    /// Default time limit when a file is opened.
    pub const OPEN_TIME_LIMIT: Duration = Duration::from_secs(20);

    /// No limits.
    pub fn unbounded() -> Self {
        Self {
            max_cells_evaluated: u64::MAX,
            deadline: None,
        }
    }

    /// At most `max_cells_evaluated` formulas, finishing within `time` from now.
    pub fn within(max_cells_evaluated: u64, time: Option<Duration>) -> Self {
        Self {
            max_cells_evaluated,
            deadline: time.and_then(|t| Instant::now().checked_add(t)),
        }
    }

    /// The budget used when a file is opened: 50M formulas or 20 s.
    pub fn on_open() -> Self {
        Self::within(Self::OPEN_MAX_CELLS, Some(Self::OPEN_TIME_LIMIT))
    }

    fn exhausted(&self, evaluated: usize) -> bool {
        evaluated as u64 >= self.max_cells_evaluated
            || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

impl Default for RecalcBudget {
    fn default() -> Self {
        Self::unbounded()
    }
}

/// Recalculates everything that depends on the pending changes.
pub fn recalc(wb: &mut Workbook) -> RecalcReport {
    recalc_with(wb, &RecalcBudget::unbounded())
}

/// [`recalc`] within `budget`.
pub fn recalc_with(wb: &mut Workbook, budget: &RecalcBudget) -> RecalcReport {
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
    let mut seed_ranges: Vec<CellRange> = pending.ranges.clone();
    for id in &dirty {
        if let Some(f) = wb.formulas.get(*id) {
            match f.array_range() {
                Some(b) => seed_ranges.push(b),
                None => seed_cells.push(f.addr),
            }
        }
    }
    let mut frontier = coalesce(&seed_cells, &seed_ranges);
    let mut found: Vec<FormulaCellId> = Vec::new();
    while !frontier.is_empty() {
        found.clear();
        wb.deps.dependents_of(&frontier, &mut found);
        let mut next: Vec<CellAddr> = Vec::new();
        let mut next_blocks: Vec<CellRange> = Vec::new();
        for id in found.drain(..) {
            if let Some(f) = wb.formulas.get_mut(id) {
                if f.state == CellState::Clean {
                    f.state = CellState::Dirty;
                    dirty.push(id);
                    match f.array_range() {
                        Some(b) => next_blocks.push(b),
                        None => next.push(f.addr),
                    }
                }
            }
        }
        frontier = coalesce(&next, &next_blocks);
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
    let mut recalculated_blocks: Vec<CellRange> = Vec::new();
    for scc in order {
        if report.incomplete || budget.exhausted(report.evaluated) {
            report.incomplete = true;
            for id in &scc.members {
                if let Some(f) = wb.formulas.get_mut(*id) {
                    if f.state == CellState::Dirty {
                        f.state = CellState::Clean;
                        report.skipped += 1;
                    }
                }
            }
            continue;
        }
        for id in &scc.strict {
            if let Some(f) = wb.formulas.get_mut(*id) {
                f.cached = Value::Error(ErrorKind::Circular);
                f.state = CellState::Clean;
                recalculated.push(f.addr);
                report.circular += 1;
            }
        }
        for id in scc.members {
            let Some(f) = wb.formulas.get(id) else {
                continue;
            };
            // Duplicates (strict members, cells evaluated on demand) are
            // removed by `coalesce`.
            match f.array_range() {
                Some(b) => recalculated_blocks.push(b),
                None => recalculated.push(f.addr),
            }
            if f.state != CellState::Dirty {
                continue;
            }
            let mut ctx = CalcCtx::new(wb, now);
            ctx.evaluate(id);
            report.circular += ctx.circular;
            report.evaluated += 1;
        }
    }
    let mut cells = pending.cells;
    cells.extend(recalculated);
    let mut ranges = pending.ranges;
    ranges.extend(pending.display_only);
    ranges.extend(recalculated_blocks);
    report.changed = coalesce(&cells, &ranges);
    report
}

/// Marks every formula dirty and recalculates.
pub fn hard_recalc(wb: &mut Workbook) -> RecalcReport {
    hard_recalc_with(wb, &RecalcBudget::unbounded())
}

/// [`hard_recalc`] within `budget`.
pub fn hard_recalc_with(wb: &mut Workbook, budget: &RecalcBudget) -> RecalcReport {
    wb.dirty_all();
    recalc_with(wb, budget)
}

/// One strongly connected component, in evaluation order.
#[derive(Debug)]
struct Scc {
    members: Vec<FormulaCellId>,
    /// Members on a cycle of strict precedents: Err:522 without evaluation.
    strict: Vec<FormulaCellId>,
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
            if let Some(id) = col.cells_get(*r).and_then(CellContent::formula_id) {
                if wb
                    .formulas
                    .get(id)
                    .is_some_and(|f| f.state == CellState::Dirty)
                {
                    out.push(id);
                }
            }
        }
    }
}

/// The dirty formulas in `ranges`, as indices into `local`.
fn dirty_successors(
    wb: &Workbook,
    ranges: &mut dyn Iterator<Item = &CellRange>,
    local: &SecondaryMap<FormulaCellId, usize>,
    scratch: &mut Vec<FormulaCellId>,
) -> Vec<usize> {
    scratch.clear();
    for r in ranges {
        dirty_formulas_in(wb, r, scratch);
    }
    let mut s: Vec<usize> = scratch
        .iter()
        .filter_map(|id| local.get(*id).copied())
        .collect();
    s.sort_unstable();
    s.dedup();
    s
}

/// Orders the dirty subgraph into SCCs, precedents first.
///
/// The ordering uses every precedent (static, conditional and dynamic). A
/// component is then *statically* circular only for the members on a cycle
/// of strict precedents (references whose values are always read); the
/// other members of a cyclic component are evaluated, and a cycle that is
/// actually followed is caught when a cell re-enters evaluation.
fn topo_order(wb: &Workbook, dirty: &[FormulaCellId]) -> Vec<Scc> {
    let mut local: SecondaryMap<FormulaCellId, usize> = SecondaryMap::new();
    for (i, id) in dirty.iter().enumerate() {
        local.insert(*id, i);
    }
    let mut scratch: Vec<FormulaCellId> = Vec::new();
    let sccs = tarjan(dirty.len(), |v| {
        let Some(f) = wb.formulas.get(dirty[v]) else {
            return Vec::new();
        };
        let mut it = f.precedents.iter().chain(f.dynamic_deps.iter());
        dirty_successors(wb, &mut it, &local, &mut scratch)
    });
    let mut out = Vec::with_capacity(sccs.len());
    for (members, cyclic) in sccs {
        let members: Vec<FormulaCellId> = members.into_iter().map(|i| dirty[i]).collect();
        let strict = if cyclic {
            strict_cycle_members(wb, &members, &mut scratch)
        } else {
            Vec::new()
        };
        out.push(Scc { members, strict });
    }
    out
}

/// The members of `scc` that lie on a cycle of strict precedents.
fn strict_cycle_members(
    wb: &Workbook,
    scc: &[FormulaCellId],
    scratch: &mut Vec<FormulaCellId>,
) -> Vec<FormulaCellId> {
    let mut local: SecondaryMap<FormulaCellId, usize> = SecondaryMap::new();
    for (i, id) in scc.iter().enumerate() {
        local.insert(*id, i);
    }
    let mut out = Vec::new();
    for (members, cyclic) in tarjan(scc.len(), |v| {
        let Some(f) = wb.formulas.get(scc[v]) else {
            return Vec::new();
        };
        let strict = f.strict_precedents.as_ref().unwrap_or(&f.precedents);
        dirty_successors(wb, &mut strict.iter(), &local, scratch)
    }) {
        if cyclic {
            out.extend(members.into_iter().map(|i| scc[i]));
        }
    }
    out
}

/// Iterative Tarjan over nodes `0..n`; components come out successors
/// first, each with whether it is cyclic (several members or a self-edge).
fn tarjan(n: usize, mut edges: impl FnMut(usize) -> Vec<usize>) -> Vec<(Vec<usize>, bool)> {
    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut succ: Vec<Option<Vec<usize>>> = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<(Vec<usize>, bool)> = Vec::new();
    let mut counter = 0usize;

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
        succ[start] = Some(edges(start));
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
                    succ[w] = Some(edges(w));
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
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let cyclic = members.len() > 1
                    || succ[v]
                        .as_ref()
                        .is_some_and(|s| s.binary_search(&v).is_ok());
                out.push((members, cyclic));
            }
        }
    }
    out
}

/// How a [`CalcCtx`] holds the workbook: mutably (recalc, which caches
/// results) or shared (previews, which never write).
enum WbAccess<'w> {
    Mut(&'w mut Workbook),
    Shared(&'w Workbook),
}

impl std::ops::Deref for WbAccess<'_> {
    type Target = Workbook;

    fn deref(&self) -> &Workbook {
        match self {
            WbAccess::Mut(wb) => wb,
            WbAccess::Shared(wb) => wb,
        }
    }
}

/// A cell being evaluated by a [`CalcCtx`].
#[derive(Copy, Clone, Debug)]
struct Frame {
    id: FormulaCellId,
    /// A cycle through this cell was followed: its result is Err:522.
    circular: bool,
    /// A precedent was deferred: the result is discarded.
    deferred: bool,
}

/// The engine's [`EvalContext`].
pub struct CalcCtx<'w> {
    wb: WbAccess<'w>,
    origin: CellAddr,
    now: f64,
    depth: u32,
    dyn_deps: Vec<CellRange>,
    /// RAND source for shared (read-only) contexts.
    local_rng: Option<crate::workbook::Rng>,
    /// Cells being evaluated, outermost first.
    eval_stack: Vec<Frame>,
    /// Dirty precedents deferred in the current attempt (too deep to nest).
    deferred: Vec<FormulaCellId>,
    /// Cells given Err:522 because a cycle through them was followed.
    pub circular: usize,
}

impl<'w> CalcCtx<'w> {
    /// A context over `wb` with the recalc's fixed `now`.
    pub fn new(wb: &'w mut Workbook, now: f64) -> Self {
        Self {
            wb: WbAccess::Mut(wb),
            origin: CellAddr::new(0, 0, 0),
            now,
            depth: 0,
            dyn_deps: Vec::new(),
            local_rng: None,
            eval_stack: Vec::new(),
            deferred: Vec::new(),
            circular: 0,
        }
    }

    /// A context for evaluating an expression at `origin` (no cell is written).
    pub fn at(wb: &'w mut Workbook, origin: CellAddr) -> Self {
        let now = now_serial(wb.settings.date_epoch);
        let mut c = Self::new(wb, now);
        c.origin = origin;
        c
    }

    /// A read-only context at `origin`: formula cells read their cached
    /// values (dirty ones are not recalculated) and nothing is written.
    pub fn shared_at(wb: &'w Workbook, origin: CellAddr) -> Self {
        Self {
            now: now_serial(wb.settings.date_epoch),
            wb: WbAccess::Shared(wb),
            origin,
            depth: 0,
            dyn_deps: Vec::new(),
            local_rng: Some(crate::workbook::Rng::from_time()),
            eval_stack: Vec::new(),
            deferred: Vec::new(),
            circular: 0,
        }
    }

    fn wb_mut(&mut self) -> Option<&mut Workbook> {
        match &mut self.wb {
            WbAccess::Mut(wb) => Some(wb),
            WbAccess::Shared(_) => None,
        }
    }

    /// Evaluates formula cell `id` and stores its result.
    ///
    /// Dirty precedents met on the way are evaluated on demand, nested at
    /// most [`MAX_ON_DEMAND_DEPTH`] deep. A precedent past that depth is
    /// deferred: the attempt is discarded (every cell on the evaluation
    /// stack stays dirty), the deferred cell is evaluated first, and the
    /// attempt is repeated. This explicit work stack keeps the Rust stack
    /// bounded however long the chain (AOO's recursion helper does the
    /// same). A deferred cell that is already on the work stack is a cycle.
    pub fn evaluate(&mut self, id: FormulaCellId) {
        if self.wb_mut().is_none() {
            return;
        }
        let mut work: Vec<FormulaCellId> = vec![id];
        while let Some(&top) = work.last() {
            if !self.is_dirty(top) {
                work.pop();
                continue;
            }
            self.deferred.clear();
            self.evaluate_once(top);
            let deferred = std::mem::take(&mut self.deferred);
            if !self.is_dirty(top) {
                work.pop();
                continue;
            }
            let mut pushed = false;
            for d in deferred {
                if !self.is_dirty(d) {
                    continue;
                }
                if let Some(p) = work.iter().position(|x| *x == d) {
                    // The deferred cell is waiting for this one: a cycle.
                    let cycle: Vec<FormulaCellId> = work.drain(p..).collect();
                    for c in cycle {
                        self.set_circular(c);
                    }
                    pushed = true;
                    break;
                }
                if !work.contains(&d) {
                    work.push(d);
                    pushed = true;
                }
            }
            if !pushed {
                // No progress is possible (should not happen): give up on it.
                self.set_circular(top);
                work.pop();
            }
        }
    }

    fn is_dirty(&self, id: FormulaCellId) -> bool {
        self.wb
            .formulas
            .get(id)
            .is_some_and(|f| f.state == CellState::Dirty)
    }

    fn set_circular(&mut self, id: FormulaCellId) {
        if let Some(f) = self.wb_mut().and_then(|wb| wb.formulas.get_mut(id)) {
            f.cached = Value::Error(ErrorKind::Circular);
            f.state = CellState::Clean;
            self.circular += 1;
        }
    }

    /// One evaluation attempt of `id`. Leaves the cell dirty if a precedent
    /// was deferred (see [`Self::evaluate`]).
    fn evaluate_once(&mut self, id: FormulaCellId) {
        let Some(f) = self.wb_mut().and_then(|wb| wb.formulas.get_mut(id)) else {
            return;
        };
        f.state = CellState::Evaluating;
        let formula = f.formula.clone();
        let addr = f.addr;
        let is_array = f.array.is_some();
        let formula = if f.has_names {
            Arc::new(crate::names::expand(&self.wb.names, &formula, addr.sheet).0)
        } else {
            formula
        };
        let saved_origin = std::mem::replace(&mut self.origin, addr);
        let saved_dyn = std::mem::take(&mut self.dyn_deps);
        self.depth += 1;
        self.eval_stack.push(Frame {
            id,
            circular: false,
            deferred: false,
        });
        let (mut value, matrix) = if is_array {
            let v = eval_in_array_context(&formula.expr, self);
            match array_result(&v, self) {
                Value::Array(m) => {
                    let first = m
                        .get(0, 0)
                        .cloned()
                        .unwrap_or(Value::Error(ErrorKind::Value));
                    (normalize_cell_value(first), Some(m))
                }
                other => (normalize_cell_value(other), None),
            }
        } else {
            let v = eval_expr(&formula.expr, self);
            (cell_result(&v, self), None)
        };
        #[cfg(test)]
        test_hook::on_evaluated(self, addr);
        let frame = self.eval_stack.pop();
        self.depth -= 1;
        self.origin = saved_origin;
        let dynamic = std::mem::replace(&mut self.dyn_deps, saved_dyn);
        let incomplete = frame.is_some_and(|f| f.deferred);
        let mut matrix = matrix;
        if !incomplete && frame.is_some_and(|f| f.circular) {
            value = Value::Error(ErrorKind::Circular);
            matrix = None;
            self.circular += 1;
        }
        let Some(wb) = self.wb_mut() else {
            return;
        };
        let Some(f) = wb.formulas.get_mut(id) else {
            return;
        };
        if incomplete {
            // A precedent was deferred: the result is not final.
            f.state = CellState::Dirty;
            return;
        }
        f.cached = value;
        if f.array.is_some() {
            f.array_result = matrix;
        }
        f.state = CellState::Clean;
        if dynamic != f.dynamic_deps {
            f.dynamic_deps = dynamic;
            let mut all = f.precedents.clone();
            all.extend(f.dynamic_deps.iter().copied());
            let vol = f.volatile;
            wb.deps.remove(id);
            wb.deps.add(id, &all);
            wb.deps.set_volatile(id, vol);
        }
    }

    fn formula_value(&mut self, id: FormulaCellId) -> Value {
        let Some(f) = self.wb.formulas.get(id) else {
            return Value::Empty;
        };
        match f.state {
            CellState::Clean => f.cached.clone(),
            CellState::Evaluating => {
                self.cycle_followed(id);
                Value::Error(ErrorKind::Circular)
            }
            CellState::Dirty => {
                if self.depth >= MAX_ON_DEMAND_DEPTH {
                    // Too deep to nest: defer it; this attempt is discarded.
                    if !self.deferred.contains(&id) {
                        self.deferred.push(id);
                    }
                    for fr in &mut self.eval_stack {
                        fr.deferred = true;
                    }
                    return Value::Error(ErrorKind::Circular);
                }
                self.evaluate_once(id);
                self.wb
                    .formulas
                    .get(id)
                    .map_or(Value::Empty, |f| match f.state {
                        CellState::Clean => f.cached.clone(),
                        _ => Value::Error(ErrorKind::Circular),
                    })
            }
        }
    }

    /// Marks every cell on the evaluation stack from `id` up as circular.
    fn cycle_followed(&mut self, id: FormulaCellId) {
        if let Some(p) = self.eval_stack.iter().rposition(|f| f.id == id) {
            for f in &mut self.eval_stack[p..] {
                f.circular = true;
            }
        }
    }

    fn ensure_clean(&mut self, range: &CellRange) {
        let mut ids = Vec::new();
        dirty_formulas_in(&self.wb, range, &mut ids);
        for id in ids {
            let _ = self.formula_value(id);
        }
        // Cells of the range still being evaluated are on a followed cycle.
        if !self.eval_stack.is_empty() {
            let on_stack: Vec<FormulaCellId> = self
                .eval_stack
                .iter()
                .map(|f| f.id)
                .filter(|x| {
                    self.wb
                        .formulas
                        .get(*x)
                        .is_some_and(|f| f.extent().intersects(range))
                })
                .collect();
            for id in on_stack {
                self.cycle_followed(id);
            }
        }
    }
}

/// A scalar result as a cell stores it (empty → 0, non-finite → `#NUM!`).
fn normalize_cell_value(v: Value) -> Value {
    match v {
        Value::Empty => Value::Number(0.0),
        Value::Number(n) if !n.is_finite() => Value::Error(ErrorKind::Num),
        Value::Array(m) => m
            .get(0, 0)
            .cloned()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        other => other,
    }
}

impl Workbook {
    /// Resolves reference text (INDIRECT/name box style) at `origin`: a
    /// cell, an area, whole columns/rows, sheet-qualified (`Sheet2.A1`,
    /// `$Sheet1.$A$1:$B$5`) or a named range. `None` for anything else or
    /// a reference to a missing sheet.
    pub fn parse_reference_at(&self, text: &str, origin: CellAddr) -> Option<CellRange> {
        let t = text.trim();
        if t.is_empty() || t.starts_with('=') {
            return None;
        }
        let f = self.parse_formula(&format!("={t}"), origin).ok()?;
        let f = if crate::names::has_names(&f.expr) {
            crate::names::expand(&self.names, &f, origin.sheet).0
        } else {
            f
        };
        let mut e = &f.expr;
        while let Expr::Paren(x) = e {
            e = x;
        }
        match e {
            Expr::Ref(r) => match r.resolve(origin) {
                ResolvedRef::Range(cr) if self.sheet(cr.sheet).is_some() => Some(cr),
                _ => None,
            },
            _ => None,
        }
    }

    /// Evaluates formula `text` (CalcA1 en-US, with the leading `=`) as if it
    /// were entered at `addr`, without writing anything (⌘K previews).
    /// Precedents read their current values. `None` if `text` does not parse.
    pub fn eval_formula_text_at(&self, addr: CellAddr, text: &str) -> Option<Value> {
        let f = self.parse_formula(text, addr).ok()?;
        let f = if crate::names::has_names(&f.expr) {
            crate::names::expand(&self.names, &f, addr.sheet).0
        } else {
            f
        };
        let mut ctx = CalcCtx::shared_at(self, addr);
        let v = eval_expr(&f.expr, &mut ctx);
        Some(cell_result(&v, &mut ctx))
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
            Some(CellContent::ArrayPart(id)) => {
                let id = *id;
                match self.formula_value(id) {
                    Value::Error(ErrorKind::Circular)
                        if self
                            .wb
                            .formulas
                            .get(id)
                            .is_none_or(|f| f.state != CellState::Clean) =>
                    {
                        Value::Error(ErrorKind::Circular)
                    }
                    _ => self.wb.formulas.get(id).map_or(Value::Empty, |f| {
                        f.array_value(addr.row - f.addr.row, addr.col - f.addr.col)
                    }),
                }
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
            let value_of = |content: &'_ CellContent, r: u32| -> Option<Value> {
                match content {
                    CellContent::Value(_) => None,
                    CellContent::Formula(id) => Some(match wb.formulas.get(*id) {
                        Some(fc) if fc.state == CellState::Clean => fc.cached.clone(),
                        Some(_) => circular.clone(),
                        None => Value::Empty,
                    }),
                    CellContent::ArrayPart(id) => Some(match wb.formulas.get(*id) {
                        Some(fc) if fc.state == CellState::Clean => fc.array_value(
                            r.saturating_sub(fc.addr.row),
                            c.saturating_sub(fc.addr.col),
                        ),
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
                            let v = value_of(other, r).unwrap_or(Value::Empty);
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
                                    let v = value_of(other, r).unwrap_or(Value::Empty);
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
            return crate::r1c1::parse(&self.wb, text, self.origin);
        }
        self.wb.parse_reference_at(text, self.origin)
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
        if let Some(rng) = self.local_rng.as_mut() {
            return rng.next_f64();
        }
        match self.wb_mut() {
            Some(wb) => wb.rng.next_f64(),
            None => 0.5,
        }
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
        // AOO's default cell style is protected (locked).
        info.protected = style.data.protection.is_none_or(|p| p.locked);
        info
    }

    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        self.wb.sheet(sheet).is_some_and(|s| s.row_hidden(row))
    }

    fn filtered_row(&self, sheet: SheetId, row: u32) -> bool {
        self.wb
            .sheet(sheet)
            .is_some_and(|s| s.filtered_rows.contains(row))
    }
}

/// A test-only hook that makes the evaluation of one chosen cell record a
/// dynamic dependency, as INDIRECT/OFFSET will (they are not registered
/// yet; the registry is global, so a test function cannot be added).
#[cfg(test)]
pub(crate) mod test_hook {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static HOOK: RefCell<Option<(CellAddr, CellRange)>> = const { RefCell::new(None) };
        static COUNT: RefCell<u32> = const { RefCell::new(0) };
    }

    /// While set, evaluating `cell` reads `range` and records it as a
    /// dynamic dependency.
    pub fn set(hook: Option<(CellAddr, CellRange)>) {
        HOOK.with(|h| *h.borrow_mut() = hook);
        COUNT.with(|c| *c.borrow_mut() = 0);
    }

    /// How often the hooked cell was evaluated since [`set`].
    pub fn count() -> u32 {
        COUNT.with(|c| *c.borrow())
    }

    pub(super) fn on_evaluated(ctx: &mut CalcCtx<'_>, addr: CellAddr) {
        let Some((cell, range)) = HOOK.with(|h| *h.borrow()) else {
            return;
        };
        if cell == addr {
            COUNT.with(|c| *c.borrow_mut() += 1);
            ctx.record_dynamic_dep(&range);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap()
    }

    #[test]
    fn dynamic_dependencies_propagate_and_are_recollected() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "=1+1");
        recalc(&mut wb);
        // A1 "reads" B5:B6 dynamically from now on.
        test_hook::set(Some((a("A1"), CellRange::from_a1(1, "B5:B6").unwrap())));
        wb.set_input(a("A1"), "=1+2");
        recalc(&mut wb);
        assert_eq!(test_hook::count(), 1);
        let (id, f) = wb.formula_at(a("A1")).unwrap();
        assert_eq!(
            f.dynamic_deps,
            vec![CellRange::from_a1(1, "B5:B6").unwrap()]
        );
        assert!(wb
            .deps
            .registered_ranges(id)
            .contains(&CellRange::from_a1(1, "B5:B6").unwrap()));
        // Editing a cell of the dynamic range re-evaluates A1.
        wb.set_input(a("B6"), "7");
        let rep = recalc(&mut wb);
        assert_eq!(test_hook::count(), 2);
        assert!(rep.evaluated >= 1);
        // The next evaluation records C1 instead: B5 no longer dirties A1.
        test_hook::set(Some((a("A1"), CellRange::from_a1(1, "C1").unwrap())));
        wb.set_input(a("B5"), "1");
        recalc(&mut wb);
        assert_eq!(test_hook::count(), 1, "still registered for B5 this once");
        wb.set_input(a("B5"), "2");
        recalc(&mut wb);
        assert_eq!(test_hook::count(), 1, "B5 is no longer a dependency");
        wb.set_input(a("C1"), "2");
        recalc(&mut wb);
        assert_eq!(test_hook::count(), 2);
        test_hook::set(None);
    }

    #[test]
    fn parse_reference_text_resolves_names_and_sheets() {
        let mut wb = Workbook::new();
        let s2 = wb.add_sheet(Some("Data"), None);
        let def = wb.parse_name_expr("$Data.$A$1:$B$5", None).unwrap();
        wb.names.define("Block", None, def);
        let o = a("A1");
        assert_eq!(
            wb.parse_reference_at("Block", o),
            Some(CellRange::new(s2, 0, 0, 4, 1))
        );
        assert_eq!(
            wb.parse_reference_at("Data.B2", o),
            Some(CellRange::new(s2, 1, 1, 1, 1))
        );
        assert_eq!(
            wb.parse_reference_at("$Data.$A$1:$B$2", o),
            Some(CellRange::new(s2, 0, 0, 1, 1))
        );
        assert_eq!(
            wb.parse_reference_at("C3", o),
            Some(CellRange::new(1, 2, 2, 2, 2))
        );
        assert_eq!(wb.parse_reference_at("Nope.A1", o), None);
        assert_eq!(wb.parse_reference_at("1+2", o), None);
        let ctx = CalcCtx::shared_at(&wb, o);
        assert_eq!(
            ctx.parse_reference_text("Block", true),
            Some(CellRange::new(s2, 0, 0, 4, 1))
        );
    }

    #[test]
    fn array_formula_spreads_and_updates() {
        let mut wb = Workbook::new();
        for (i, v) in ["1", "2", "3"].iter().enumerate() {
            wb.set_input(CellAddr::new(1, i as u32, 0), v);
        }
        let f = wb.parse_formula("=A1:A3*10", a("B1")).unwrap();
        wb.set_array_formula(a("B1"), Arc::new(f), 4, 1);
        wb.set_input(a("C1"), "=SUM(B1:B3)");
        recalc(&mut wb);
        assert_eq!(wb.value(a("B1")), Value::Number(10.0));
        assert_eq!(wb.value(a("B3")), Value::Number(30.0));
        assert_eq!(
            wb.value(a("B4")),
            Value::Error(ErrorKind::NA),
            "outside the result"
        );
        assert_eq!(wb.value(a("C1")), Value::Number(60.0));
        wb.set_input(a("A2"), "5");
        recalc(&mut wb);
        assert_eq!(wb.value(a("B2")), Value::Number(50.0));
        assert_eq!(
            wb.value(a("C1")),
            Value::Number(90.0),
            "dependents of covered cells"
        );
        // A scalar result fills the block.
        let g = wb.parse_formula("=7", a("D1")).unwrap();
        wb.set_array_formula(a("D1"), Arc::new(g), 2, 2);
        recalc(&mut wb);
        assert_eq!(wb.value(a("E2")), Value::Number(7.0));
        // Clearing the anchor removes the block.
        wb.clear_content(a("D1"));
        recalc(&mut wb);
        assert_eq!(wb.value(a("E2")), Value::Empty);
        assert!(wb.sheet(1).unwrap().get(1, 4).is_none());
    }

    fn num(wb: &Workbook, s: &str) -> Value {
        wb.value(a(s))
    }

    #[test]
    fn eval_formula_text_at_does_not_commit() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "2");
        wb.set_input(a("A2"), "3");
        wb.set_input(a("B1"), "=A1*10");
        recalc(&mut wb);
        assert_eq!(
            wb.eval_formula_text_at(a("C5"), "=SUM(A1:A2)+B1"),
            Some(Value::Number(25.0))
        );
        assert_eq!(wb.value(a("C5")), Value::Empty);
        assert!(wb.formula_at(a("C5")).is_none());
        assert_eq!(wb.eval_formula_text_at(a("C5"), "=SUM("), None);
        let r = wb.eval_formula_text_at(a("C5"), "=RAND()");
        assert!(matches!(r, Some(Value::Number(x)) if (0.0..1.0).contains(&x)));
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
