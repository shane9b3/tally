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
//!    address-only arguments of ROW/COLUMN) get Err:522 without evaluation.
//!    With `CalcSettings::iterations` set (AOO Tools ▸ Options ▸ Calculate ▸
//!    Iterations) a cyclic component is instead evaluated Gauss–Seidel:
//!    every member once per round in Tarjan order, each reading the
//!    current values of the others (a member still being evaluated hands
//!    out its previous value), starting from the cached values (0 for new
//!    cells), for at most `steps` rounds, until no member changes by more
//!    than `min_change`; a change that is not numeric never converges.
//!    Without convergence every member gets Err:523.
//! 4. Evaluation in that order with [`CalcCtx`]. A dirty precedent met
//!    during evaluation (through a lazy branch or a dynamic reference) is
//!    evaluated on demand, nested at most [`MAX_ON_DEMAND_DEPTH`] deep and
//!    deferred to an explicit work stack beyond that. Reading a
//!    cell that is being evaluated is a cycle that is actually followed
//!    (AOO detects circularity at interpretation time the same way): every
//!    cell on that cycle gets Err:522.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use slotmap::SecondaryMap;
use smallvec::SmallVec;
use tally_formula::{Expr, ResolvedRef};
use tally_functions::{CellFormatInfo, EvalContext, LookupIndex};
use tally_numfmt::date::now_serial;
use tally_types::{
    CalcSettings, CellAddr, CellRange, ErrorKind, IterSettings, Matrix, SheetId, Value, MAX_COLS,
    MAX_ROWS,
};

use crate::deps::{coalesce, DepIndex};
use crate::eval::{array_result, cell_result, eval_with, EvalEnv};
use crate::formula_cells::{CellState, FormulaCellId, Span3d};
use crate::lookup_cache::{LookupCache, Request};
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
    /// Evaluations spent iterating cyclic components (bounded by
    /// [`MAX_ITERATION_EVALS`] per recalc).
    pub iteration_evaluations: u64,
    /// Of `evaluated`, the formulas whose results came from the parallel
    /// pre-pass ([`crate::parallel`]; 0 when it did not run).
    pub precomputed: usize,
    /// Pre-pass evaluations repeated after waiting for a precedent found
    /// during evaluation (at most one per dirty formula in all).
    pub prepass_retries: usize,
    /// Work units ([`crate::eval::MAX_FORMULA_WORK`]) the pre-pass spent on
    /// evaluations it discarded (each stops at its first bail).
    pub prepass_wasted_work: u64,
}

/// Most formula evaluations one recalc spends iterating cyclic components
/// (all components and steps together). Past it the remaining components
/// do not converge (Err:523), so a file asking for 32,767 steps over a
/// large cycle cannot hold the engine for minutes (security review 4, H2).
pub const MAX_ITERATION_EVALS: u64 = 20_000_000;

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
    // Lookup indexes live for this pass only (see `crate::lookup_cache`).
    wb.lookup_cache = Some(LookupCache::default());
    let report = recalc_pass(wb, budget);
    wb.lookup_cache = None;
    report
}

fn recalc_pass(wb: &mut Workbook, budget: &RecalcBudget) -> RecalcReport {
    wb.table_op_calls = 0;
    wb.table_op_work = 0;
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
    if wb.settings.precision_as_shown && !pending.display_only.is_empty() {
        // Precision as shown: a number-format change changes the values
        // formulas read and the rounding of formula results, so the
        // re-formatted formulas and the dependents of the re-formatted
        // cells are recalculated (AOO broadcasts the format change,
        // sc/source/core/data/documen8.cxx:419; correctness review 4, H11).
        let mut found = Vec::new();
        for r in &pending.display_only {
            formulas_in(wb, r, &mut found);
            seed_ranges.push(*r);
        }
        for id in found {
            if let Some(f) = wb.formulas.get_mut(id) {
                f.state = CellState::Dirty;
                dirty.push(id);
            }
        }
    }
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

    let parallel = crate::parallel::wanted(wb, dirty.len(), budget);
    // The pre-pass needs every dirty precedent edge; past the cap (wide
    // ranges over many formulas) it is not worth the memory and the recalc
    // stays single-threaded (security review 7, H2).
    let cap = parallel.then(|| crate::parallel::edge_cap(dirty.len()));
    let (order, preds) = topo_order(wb, &dirty, cap);
    let mut report = RecalcReport::default();
    let now = now_serial(wb.settings.date_epoch);
    // The parallel pre-pass (docs/recalc-parallel.md): results the loop
    // below takes instead of evaluating.
    let mut pre = if let Some(preds) = preds {
        let mut at: SecondaryMap<FormulaCellId, u32> = SecondaryMap::new();
        for (k, scc) in order.iter().enumerate() {
            for id in &scc.members {
                at.insert(*id, k as u32);
            }
        }
        let pos: Vec<u32> = dirty
            .iter()
            .map(|id| at.get(*id).copied().unwrap_or(u32::MAX))
            .collect();
        drop(at);
        crate::parallel::prepass(wb, &dirty, preds, pos, now, budget)
    } else {
        None
    };
    if let Some(p) = &pre {
        report.prepass_retries = p.stats.retries;
        report.prepass_wasted_work = p.stats.wasted_work;
    }
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
        if scc.cyclic {
            if let Some(iter) = wb.settings.iterations {
                for id in &scc.members {
                    if let Some(f) = wb.formulas.get(*id) {
                        match f.array_range() {
                            Some(b) => recalculated_blocks.push(b),
                            None => recalculated.push(f.addr),
                        }
                    }
                }
                iterate_scc(wb, &scc.members, iter, now, budget, &mut report);
                continue;
            }
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
            let Some(f) = wb.formulas.get_mut(id) else {
                continue;
            };
            if report.incomplete {
                // The deadline passed inside this component.
                if f.state == CellState::Dirty {
                    f.state = CellState::Clean;
                    report.skipped += 1;
                }
                continue;
            }
            // Duplicates (strict members, cells evaluated on demand) are
            // removed by `coalesce`.
            match f.array_range() {
                Some(b) => recalculated_blocks.push(b),
                None => recalculated.push(f.addr),
            }
            if f.state != CellState::Dirty {
                continue;
            }
            if let Some(done) = pre.as_mut().and_then(|p| p.take(id)) {
                store_result(wb, id, done);
                report.evaluated += 1;
                report.precomputed += 1;
                continue;
            }
            let mut ctx = CalcCtx::new(wb, now);
            ctx.deadline = budget.deadline;
            ctx.evaluate(id);
            report.circular += ctx.circular;
            if ctx.timed_out {
                // Abandoned inside its deferred-evaluation work stack: it
                // and the cells it was waiting for keep their old values.
                report.incomplete = true;
                report.skipped += ctx.skipped;
            } else {
                report.evaluated += 1;
            }
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
    /// Inline for the usual single-member component, so a recalc of n
    /// formulas does not make n small allocations.
    members: SmallVec<[FormulaCellId; 1]>,
    /// Members on a cycle of strict precedents: Err:522 without evaluation.
    strict: Vec<FormulaCellId>,
    /// Several members or a self-edge (iterated when iterations are on).
    cyclic: bool,
}

/// Evaluates one cyclic component with iterations on (see the module
/// docs): Gauss–Seidel rounds until convergence, else Err:523 everywhere.
fn iterate_scc(
    wb: &mut Workbook,
    members: &[FormulaCellId],
    iter: IterSettings,
    now: f64,
    budget: &RecalcBudget,
    report: &mut RecalcReport,
) {
    // Seed: the previous values; circular/non-converged errors start at 0.
    for id in members {
        if let Some(f) = wb.formulas.get_mut(*id) {
            if matches!(
                f.cached,
                Value::Error(ErrorKind::Circular | ErrorKind::NoConvergence) | Value::Empty
            ) {
                f.cached = Value::Number(0.0);
            }
        }
    }
    let set: Arc<HashSet<FormulaCellId>> = Arc::new(members.iter().copied().collect());
    let mut converged = false;
    for _round in 0..iter.steps.max(1) {
        let n = members.len() as u64;
        if report.iteration_evaluations.saturating_add(n) > MAX_ITERATION_EVALS
            || budget.exhausted(report.evaluated)
        {
            break;
        }
        report.iteration_evaluations += n;
        // Every round re-evaluates clean members: no index may outlive it.
        clear_lookup_cache(wb);
        let before: Vec<Value> = members
            .iter()
            .map(|id| {
                wb.formulas
                    .get(*id)
                    .map_or(Value::Empty, |f| f.cached.clone())
            })
            .collect();
        for id in members {
            if let Some(f) = wb.formulas.get_mut(*id) {
                f.state = CellState::Dirty;
            }
        }
        for id in members {
            let dirty = wb
                .formulas
                .get(*id)
                .is_some_and(|f| f.state == CellState::Dirty);
            if !dirty {
                continue;
            }
            let mut ctx = CalcCtx::new(wb, now);
            ctx.iter_set = Some(set.clone());
            ctx.evaluate(*id);
            report.evaluated += 1;
        }
        let mut max_delta = 0f64;
        let mut numeric = true;
        for (id, prev) in members.iter().zip(&before) {
            let Some(f) = wb.formulas.get(*id) else {
                continue;
            };
            match (prev, &f.cached) {
                (Value::Number(a), Value::Number(b)) => max_delta = max_delta.max((a - b).abs()),
                (a, b) if a == b => {}
                _ => numeric = false,
            }
        }
        if numeric && max_delta <= iter.min_change {
            converged = true;
            break;
        }
    }
    clear_lookup_cache(wb);
    if !converged {
        for id in members {
            if let Some(f) = wb.formulas.get_mut(*id) {
                f.cached = Value::Error(ErrorKind::NoConvergence);
                f.array_result = None;
                f.state = CellState::Clean;
                report.circular += 1;
            }
        }
    }
}

/// Drops every lookup index of the running recalc.
fn clear_lookup_cache(wb: &mut Workbook) {
    if let Some(c) = wb.lookup_cache.as_mut() {
        *c = LookupCache::default();
    }
}

/// `true` if every formula cell inside `range` is clean.
fn all_formulas_clean(wb: &Workbook, range: &CellRange) -> bool {
    let mut found = Vec::new();
    formulas_in(wb, range, &mut found);
    found.iter().all(|id| {
        wb.formulas
            .get(*id)
            .is_some_and(|f| f.state == CellState::Clean)
    })
}

/// Every formula cell inside `range`.
fn formulas_in(wb: &Workbook, range: &CellRange, out: &mut Vec<FormulaCellId>) {
    let Some(sheet) = wb.sheet(range.sheet) else {
        return;
    };
    if sheet.columns.is_empty() {
        return;
    }
    let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
    for c in range.c0..=c1 {
        let Some(col) = sheet.column(c) else {
            continue;
        };
        for r in col.formula_rows.range(range.r0..=range.r1) {
            if let Some(id) = col.cells_get(*r).and_then(CellContent::formula_id) {
                out.push(id);
            }
        }
    }
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

/// The dirty formulas of one topological pass by column, for matching 3-D
/// spans without visiting every spanned sheet: `(row, tab, id)` of each
/// dirty formula anchor, per column.
struct DirtyByCol {
    cols: HashMap<u32, Vec<(u32, u32, FormulaCellId)>>,
}

impl DirtyByCol {
    fn new(wb: &Workbook, ids: &[FormulaCellId]) -> Self {
        let mut cols: HashMap<u32, Vec<(u32, u32, FormulaCellId)>> = HashMap::new();
        if wb.deps.span_count() == 0 {
            // No 3-D reference anywhere: nothing will ask.
            return Self { cols };
        }
        for id in ids {
            if let Some(f) = wb.formulas.get(*id) {
                if let Some(t) = wb.tab_of(f.addr.sheet) {
                    cols.entry(f.addr.col)
                        .or_default()
                        .push((f.addr.row, t as u32, *id));
                }
            }
        }
        Self { cols }
    }

    /// The dirty formulas inside span `s`: only the columns of its
    /// rectangle that hold a dirty formula are looked at.
    fn in_span(&self, wb: &Workbook, s: &Span3d, out: &mut Vec<FormulaCellId>) {
        let (Some(a), Some(b)) = (wb.tab_of(s.first), wb.tab_of(s.last)) else {
            return;
        };
        let (lo, hi) = (a.min(b) as u32, a.max(b) as u32);
        let r = &s.rect;
        let mut visit = |list: &Vec<(u32, u32, FormulaCellId)>| {
            for (row, tab, id) in list {
                if r.r0 <= *row
                    && *row <= r.r1
                    && lo <= *tab
                    && *tab <= hi
                    && wb
                        .formulas
                        .get(*id)
                        .is_some_and(|f| f.state == CellState::Dirty)
                {
                    out.push(*id);
                }
            }
        };
        if u64::from(r.c1 - r.c0) < self.cols.len() as u64 {
            for c in r.c0..=r.c1 {
                if let Some(list) = self.cols.get(&c) {
                    visit(list);
                }
            }
        } else {
            for (c, list) in &self.cols {
                if r.c0 <= *c && *c <= r.c1 {
                    visit(list);
                }
            }
        }
    }
}

/// The dirty formulas in `ranges` and `spans`, as indices into `local`.
fn dirty_successors(
    wb: &Workbook,
    ranges: &mut dyn Iterator<Item = &CellRange>,
    spans: &[Span3d],
    dirty: &DirtyByCol,
    local: &SecondaryMap<FormulaCellId, usize>,
    scratch: &mut Vec<FormulaCellId>,
) -> Vec<usize> {
    scratch.clear();
    for r in ranges {
        dirty_formulas_in(wb, r, scratch);
    }
    for s in spans {
        dirty.in_span(wb, s, scratch);
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
/// With `keep` (a cap on the edges kept), also each dirty formula's dirty
/// precedents (indices into `dirty`, sorted, itself included on a
/// self-reference) for the parallel pre-pass; `None` without `keep` or when
/// there are more edges than the cap.
fn topo_order(
    wb: &Workbook,
    dirty: &[FormulaCellId],
    keep: Option<usize>,
) -> (Vec<Scc>, Option<Vec<Vec<usize>>>) {
    let mut local: SecondaryMap<FormulaCellId, usize> = SecondaryMap::new();
    for (i, id) in dirty.iter().enumerate() {
        local.insert(*id, i);
    }
    let mut scratch: Vec<FormulaCellId> = Vec::new();
    let by_col = DirtyByCol::new(wb, dirty);
    let (sccs, succ, kept) = tarjan(dirty.len(), keep, |v| {
        let Some(f) = wb.formulas.get(dirty[v]) else {
            return Vec::new();
        };
        let mut it = f.precedents.iter().chain(f.dynamic_deps.iter());
        if f.three_d.is_none() {
            return dirty_successors(wb, &mut it, &[], &by_col, &local, &mut scratch);
        }
        let spans = f.all_spans();
        dirty_successors(wb, &mut it, &spans, &by_col, &local, &mut scratch)
    });
    let mut out = Vec::with_capacity(sccs.len());
    for (members, cyclic) in sccs {
        let members: SmallVec<[FormulaCellId; 1]> = members.into_iter().map(|i| dirty[i]).collect();
        let strict = if cyclic {
            strict_cycle_members(wb, &members, &mut scratch)
        } else {
            Vec::new()
        };
        out.push(Scc {
            members,
            strict,
            cyclic,
        });
    }
    let preds = kept.then(|| succ.into_iter().map(Option::unwrap_or_default).collect());
    (out, preds)
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
    let by_col = DirtyByCol::new(wb, scc);
    for (members, cyclic) in tarjan(scc.len(), None, |v| {
        let Some(f) = wb.formulas.get(scc[v]) else {
            return Vec::new();
        };
        let strict = f.strict_precedents.as_deref().unwrap_or(&f.precedents);
        dirty_successors(
            wb,
            &mut strict.iter(),
            f.strict_spans(),
            &by_col,
            &local,
            scratch,
        )
    })
    .0
    {
        if cyclic {
            out.extend(members.into_iter().map(|i| scc[i]));
        }
    }
    out
}

/// Iterative Tarjan over nodes `0..n`; components come out successors
/// first, each with whether it is cyclic (several members or a self-edge).
/// With `keep` (a cap on the edges kept), each node's successor list is
/// returned too (indexed by node) and the last value is `true`; otherwise,
/// or once more than the cap of edges would be kept, the lists are freed as
/// nodes finish (only the DFS path holds any) and the last value is
/// `false`.
#[allow(clippy::type_complexity)]
fn tarjan(
    n: usize,
    keep: Option<usize>,
    mut edges: impl FnMut(usize) -> Vec<usize>,
) -> (
    Vec<(SmallVec<[usize; 1]>, bool)>,
    Vec<Option<Vec<usize>>>,
    bool,
) {
    const UNSEEN: usize = usize::MAX;
    let cap = keep.unwrap_or(0);
    let mut keep = keep.is_some();
    let mut kept = 0usize;
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut succ: Vec<Option<Vec<usize>>> = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<(SmallVec<[usize; 1]>, bool)> = Vec::new();
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
                let mut members = SmallVec::new();
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
            if keep {
                kept = kept.saturating_add(succ[v].as_ref().map_or(0, Vec::len));
                if kept > cap {
                    // Too many edges to keep: free the finished nodes'
                    // lists (those on the DFS path are still needed) and
                    // go on as without `keep`.
                    keep = false;
                    let mut active = vec![false; n];
                    for &(u, _) in &call {
                        active[u] = true;
                    }
                    for (u, s) in succ.iter_mut().enumerate() {
                        if !active[u] {
                            *s = None;
                        }
                    }
                }
            }
            // `v` is finished: its successors are never read again (a node
            // is entered once), so free them now rather than holding one
            // list per formula until the end.
            if !keep {
                succ[v] = None;
            }
        }
    }
    (out, succ, keep)
}

/// How a [`CalcCtx`] holds the workbook: mutably (recalc, which caches
/// results) or shared (previews, which never write).
enum WbAccess<'w> {
    Mut(&'w mut Workbook),
    Shared(&'w Workbook),
    /// The parallel pre-pass (docs/recalc-parallel.md): read-only, reads
    /// only final values, bails on anything else.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    Par(&'w Workbook, &'w ParShared),
}

impl std::ops::Deref for WbAccess<'_> {
    type Target = Workbook;

    fn deref(&self) -> &Workbook {
        match self {
            WbAccess::Mut(wb) => wb,
            WbAccess::Shared(wb) | WbAccess::Par(wb, _) => wb,
        }
    }
}

/// A set of formula ids that is free to create (no random hasher seed:
/// one is made per formula evaluated; ids are not attacker-chosen).
type IdSet = HashSet<
    FormulaCellId,
    std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>,
>;

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
    /// The ranges in `dyn_deps` once there are many (deduplication
    /// without a linear scan; not built for the usual handful).
    dyn_seen: Option<HashSet<CellRange>>,
    /// 3-D references built during evaluation (one entry each).
    dyn_spans: Vec<Span3d>,
    /// RAND source for shared (read-only) contexts.
    local_rng: Option<crate::workbook::Rng>,
    /// Cells being evaluated, outermost first.
    eval_stack: Vec<Frame>,
    /// Dirty precedents deferred in the current attempt (too deep to nest).
    deferred: Vec<FormulaCellId>,
    /// The members of `deferred` (constant-time duplicate test).
    deferred_seen: IdSet,
    /// Cells whose evaluation in the current attempt was left incomplete
    /// (a precedent was deferred). Read again in the same attempt, such a
    /// cell is deferred too rather than evaluated again: re-entering it
    /// would redo its whole on-demand subtree down to the deferral, once
    /// per reader, which grows exponentially with the nesting depth.
    incomplete: IdSet,
    /// Formulas this context finished (stored a result or Err:522): tells
    /// an attempt that made progress from one that cannot.
    finished: u64,
    /// The recalc's deadline, checked between attempts of the
    /// deferred-evaluation work stack ([`Self::evaluate`]).
    deadline: Option<Instant>,
    /// The deadline passed inside [`Self::evaluate`]: the cells still on its
    /// work stack were left with their old values (`skipped` of them).
    timed_out: bool,
    skipped: usize,
    /// Parallel pre-pass: work units of the last [`Self::eval_par`].
    par_work: u64,
    /// Cells given Err:522 because a cycle through them was followed.
    pub circular: usize,
    /// The members of the cyclic component being iterated (iterations on):
    /// a member that is being evaluated hands out its previous value
    /// instead of Err:522 (see the module docs).
    iter_set: Option<Arc<HashSet<FormulaCellId>>>,
    /// `CalcSettings::precision_as_shown`.
    precision: bool,
    /// `MULTIPLE.OPERATIONS` calls of a read-only context (the workbook's
    /// own counter is used when it is writable).
    local_table_ops: u32,
    /// `MULTIPLE.OPERATIONS` sandbox work of a read-only context.
    local_table_work: u64,
    /// Parallel pre-pass: the evaluation read something that is not final
    /// (or must run sequentially); its result is discarded.
    bailed: bool,
    /// Parallel pre-pass: the position in the sequential order of the
    /// formula being evaluated.
    par_pos: u32,
    /// Parallel pre-pass: the first not-yet-computed formula the bailed
    /// evaluation read (node index).
    wait_on: Option<usize>,
    /// Parallel pre-pass: the evaluation bailed for a reason waiting does
    /// not remove.
    bail_final: bool,
}

impl<'w> CalcCtx<'w> {
    /// A context over `wb` with the recalc's fixed `now`.
    pub fn new(wb: &'w mut Workbook, now: f64) -> Self {
        let precision = wb.settings.precision_as_shown;
        Self {
            wb: WbAccess::Mut(wb),
            origin: CellAddr::new(0, 0, 0),
            now,
            depth: 0,
            dyn_deps: Vec::new(),
            dyn_seen: None,
            dyn_spans: Vec::new(),
            local_rng: None,
            eval_stack: Vec::new(),
            deferred: Vec::new(),
            deferred_seen: IdSet::default(),
            incomplete: IdSet::default(),
            finished: 0,
            deadline: None,
            timed_out: false,
            skipped: 0,
            par_work: 0,
            circular: 0,
            iter_set: None,
            precision,
            local_table_ops: 0,
            local_table_work: 0,
            bailed: false,
            par_pos: 0,
            wait_on: None,
            bail_final: false,
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
            precision: wb.settings.precision_as_shown,
            wb: WbAccess::Shared(wb),
            origin,
            depth: 0,
            dyn_deps: Vec::new(),
            dyn_seen: None,
            dyn_spans: Vec::new(),
            local_rng: Some(crate::workbook::Rng::from_time()),
            eval_stack: Vec::new(),
            deferred: Vec::new(),
            deferred_seen: IdSet::default(),
            incomplete: IdSet::default(),
            finished: 0,
            deadline: None,
            timed_out: false,
            skipped: 0,
            par_work: 0,
            circular: 0,
            iter_set: None,
            local_table_ops: 0,
            local_table_work: 0,
            bailed: false,
            par_pos: 0,
            wait_on: None,
            bail_final: false,
        }
    }

    /// A context of the parallel pre-pass over `wb` (see
    /// docs/recalc-parallel.md): nothing is written; dirty formulas are read
    /// from `sh` when pre-computed, otherwise the evaluation bails.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn par(wb: &'w Workbook, sh: &'w ParShared, now: f64) -> Self {
        Self {
            precision: wb.settings.precision_as_shown,
            wb: WbAccess::Par(wb, sh),
            origin: CellAddr::new(0, 0, 0),
            now,
            depth: 0,
            dyn_deps: Vec::new(),
            dyn_seen: None,
            dyn_spans: Vec::new(),
            local_rng: None,
            eval_stack: Vec::new(),
            deferred: Vec::new(),
            deferred_seen: IdSet::default(),
            incomplete: IdSet::default(),
            finished: 0,
            deadline: None,
            timed_out: false,
            skipped: 0,
            par_work: 0,
            circular: 0,
            iter_set: None,
            local_table_ops: 0,
            local_table_work: 0,
            bailed: false,
            par_pos: 0,
            wait_on: None,
            bail_final: false,
        }
    }

    /// Charges `n` units of `MULTIPLE.OPERATIONS` sandbox work to this
    /// recalc; `false` once [`MAX_TABLE_OP_WORK`] is exceeded.
    fn charge_table_op_work(&mut self, n: u64) -> bool {
        let total = match self.wb_mut() {
            Some(wb) => {
                wb.table_op_work = wb.table_op_work.saturating_add(n);
                wb.table_op_work
            }
            None => {
                self.local_table_work = self.local_table_work.saturating_add(n);
                self.local_table_work
            }
        };
        total <= MAX_TABLE_OP_WORK
    }

    fn wb_mut(&mut self) -> Option<&mut Workbook> {
        match &mut self.wb {
            WbAccess::Mut(wb) => Some(wb),
            WbAccess::Shared(_) | WbAccess::Par(..) => None,
        }
    }

    /// The pre-pass state of a parallel context.
    fn par_state(&self) -> Option<&'w ParShared> {
        match self.wb {
            WbAccess::Par(_, sh) => Some(sh),
            _ => None,
        }
    }

    /// A parallel context reading formula `id`, which is not clean: its
    /// pre-computed result, or `None` after marking the evaluation bailed.
    fn par_done(&mut self, sh: &'w ParShared, id: FormulaCellId) -> Option<&'w Done> {
        match sh.read(id, self.par_pos) {
            SlotRead::Done(d) => Some(d),
            SlotRead::Pending(i) => {
                self.bail_waiting(i);
                None
            }
            SlotRead::Never => {
                self.bail_for_good();
                None
            }
        }
    }

    /// The evaluation bailed on a dirty formula (node `i`) not computed
    /// yet: it can be retried once that one is.
    fn bail_waiting(&mut self, i: usize) {
        self.bailed = true;
        self.wait_on.get_or_insert(i);
    }

    /// The evaluation bailed for a reason no retry removes.
    fn bail_for_good(&mut self) {
        self.bailed = true;
        self.bail_final = true;
    }

    /// Parallel pre-pass: work units the last [`Self::eval_par`] spent.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn par_work(&self) -> u64 {
        self.par_work
    }

    /// Parallel pre-pass, after a bailed [`Self::eval_par`]: the node to
    /// wait for before retrying, or `None` if retrying cannot help.
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn retry_after(&self) -> Option<usize> {
        if self.bail_final {
            None
        } else {
            self.wait_on
        }
    }

    /// `true` if `id` is a member of the component being iterated.
    fn in_iteration(&self, id: FormulaCellId) -> bool {
        self.iter_set.as_ref().is_some_and(|s| s.contains(&id))
    }

    /// The previous value of an iterated member (what a cycle reads).
    fn iterated_value(&self, id: FormulaCellId) -> Value {
        match self.wb.formulas.get(id).map(|f| &f.cached) {
            Some(Value::Empty) | None => Value::Number(0.0),
            Some(v) => v.clone(),
        }
    }

    /// With "precision as shown", a number rounded to the decimals its
    /// cell's format displays.
    fn shown(&self, addr: CellAddr, v: Value) -> Value {
        if !self.precision {
            return v;
        }
        match v {
            Value::Number(n) => Value::Number(shown_number(&self.wb, addr, n)),
            other => other,
        }
    }

    /// Evaluates formula cell `id` and stores its result.
    ///
    /// Dirty precedents met on the way are evaluated on demand, nested at
    /// most [`MAX_ON_DEMAND_DEPTH`] deep. A precedent past that depth is
    /// deferred: the attempt is discarded (every cell on the evaluation
    /// stack stays dirty), the deferred cells are evaluated first, and the
    /// attempt is repeated. This explicit work stack keeps the Rust stack
    /// bounded however long the chain (AOO's recursion helper does the
    /// same).
    ///
    /// Every entry of the work stack records the entry that deferred it
    /// (its parent). All cells one attempt deferred are pushed at once, so
    /// the attempt is repeated once after all of them, not once per cell
    /// (security review 7, H1: one at a time made this cubic). A deferred
    /// cell that is an ancestor of the attempt (the cells *waiting*: those
    /// whose deferred cells are still being evaluated) is a cycle; a sibling
    /// further down the stack is not, and is pushed again.
    pub fn evaluate(&mut self, id: FormulaCellId) {
        if self.wb_mut().is_none() {
            return;
        }
        // The usual case: nothing deferred, no work stack needed.
        if !self.is_dirty(id) {
            return;
        }
        self.deferred.clear();
        self.deferred_seen.clear();
        self.incomplete.clear();
        let before = self.finished;
        self.evaluate_once(id);
        if !self.is_dirty(id) {
            return;
        }
        self.evaluate_deferred(id, before);
    }

    /// [`Self::evaluate`] once the first attempt of `id` deferred cells
    /// (they are in `self.deferred`; `before` is `self.finished` before
    /// that attempt).
    fn evaluate_deferred(&mut self, id: FormulaCellId, before: u64) {
        const ROOT: usize = usize::MAX;
        // (cell, index of its parent entry)
        let mut work: Vec<(FormulaCellId, usize)> = vec![(id, ROOT)];
        // The cells whose deferred cells are on the stack above them:
        // exactly the ancestors of the top entry.
        let mut waiting = IdSet::default();
        let mut attempts = 0u32;
        // The first attempt of `id` was made by `evaluate`.
        let mut first = Some(before);
        while let Some(&(top, _)) = work.last() {
            if !self.is_dirty(top) {
                work.pop();
                continue;
            }
            attempts += 1;
            if attempts > 1 && self.deadline.is_some_and(|d| Instant::now() >= d) {
                // The recalc watchdog: the cells still waiting keep their
                // old values, as the formulas the budget skips do.
                self.timed_out = true;
                for (c, _) in work.drain(..) {
                    if let Some(f) = self.wb_mut().and_then(|wb| wb.formulas.get_mut(c)) {
                        if f.state == CellState::Dirty {
                            f.state = CellState::Clean;
                            self.skipped += 1;
                        }
                    }
                }
                return;
            }
            waiting.remove(&top);
            let before = match first.take() {
                Some(b) => b,
                None => {
                    self.deferred.clear();
                    self.deferred_seen.clear();
                    self.incomplete.clear();
                    let b = self.finished;
                    self.evaluate_once(top);
                    b
                }
            };
            let deferred = std::mem::take(&mut self.deferred);
            self.deferred_seen.clear();
            if !self.is_dirty(top) {
                work.pop();
                continue;
            }
            let t = work.len() - 1;
            let deferred: Vec<FormulaCellId> =
                deferred.into_iter().filter(|d| self.is_dirty(*d)).collect();
            if let Some(&d) = deferred.iter().find(|d| **d == top || waiting.contains(*d)) {
                // The deferred cell is waiting for this one: a cycle along
                // the parent chain from it to `top`. Only those entries get
                // Err:522; siblings on the stack are not on the cycle, and
                // entries that finished meanwhile (evaluated on demand by a
                // nested attempt) keep their results (overwriting one would
                // make other cells' values depend on when they read it).
                let mut i = t;
                loop {
                    let (c, parent) = work[i];
                    waiting.remove(&c);
                    if self.is_dirty(c) {
                        self.set_circular(c);
                    }
                    if c == d || parent == ROOT {
                        break;
                    }
                    i = parent;
                }
                continue;
            }
            if deferred.is_empty() {
                if self.finished != before {
                    // Every deferred cell was finished later in the same
                    // attempt (on demand, at a shallower depth): the attempt
                    // made progress, repeat it (correctness review 7,
                    // finding 3).
                    continue;
                }
                // No progress is possible (should not happen): give up on it.
                self.set_circular(top);
                work.pop();
                continue;
            }
            waiting.insert(top);
            // The first deferred cell ends on top: evaluated first.
            for d in deferred.into_iter().rev() {
                work.push((d, t));
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
            self.finished += 1;
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
        let env = if f.has_names {
            EvalEnv::new(crate::names::collect_names(&self.wb.names, &formula.expr, addr.sheet).0)
        } else {
            EvalEnv::default()
        };
        let saved_origin = std::mem::replace(&mut self.origin, addr);
        let saved_dyn = std::mem::take(&mut self.dyn_deps);
        let saved_seen = std::mem::take(&mut self.dyn_seen);
        let saved_spans = std::mem::take(&mut self.dyn_spans);
        self.depth += 1;
        self.eval_stack.push(Frame {
            id,
            circular: false,
            deferred: false,
        });
        let (mut value, matrix) = self.compute(&formula.expr, is_array, &env);
        #[cfg(test)]
        test_hook::on_evaluated(self, addr);
        let frame = self.eval_stack.pop();
        self.depth -= 1;
        self.origin = saved_origin;
        let dynamic = std::mem::replace(&mut self.dyn_deps, saved_dyn);
        let dynamic_spans = std::mem::replace(&mut self.dyn_spans, saved_spans);
        self.dyn_seen = saved_seen;
        let incomplete = frame.is_some_and(|f| f.deferred);
        let mut matrix = matrix;
        if !incomplete && frame.is_some_and(|f| f.circular) {
            value = Value::Error(ErrorKind::Circular);
            matrix = None;
            self.circular += 1;
        }
        let precision = self.precision;
        if !incomplete {
            self.finished += 1;
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
            self.incomplete.insert(id);
            return;
        }
        let value = if precision {
            match value {
                Value::Number(n) => Value::Number(shown_number(wb, addr, n)),
                other => other,
            }
        } else {
            value
        };
        store_result(
            wb,
            id,
            Done {
                value,
                matrix,
                dynamic,
                dynamic_spans,
            },
        );
    }

    /// Evaluates a formula's expression at `self.origin`: the cell value
    /// and, for an array formula, its result matrix.
    fn compute(
        &mut self,
        expr: &Expr,
        is_array: bool,
        env: &EvalEnv,
    ) -> (Value, Option<Arc<Matrix>>) {
        if is_array {
            let v = eval_with(expr, self, true, env);
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
            let v = eval_with(expr, self, false, env);
            (cell_result(&v, self), None)
        }
    }

    /// Evaluates dirty formula `id` in a parallel pre-pass context: its
    /// result as `evaluate_once` would store it, or `None` if the
    /// evaluation bailed (see docs/recalc-parallel.md).
    #[cfg_attr(not(feature = "parallel"), allow(dead_code))]
    pub(crate) fn eval_par(&mut self, id: FormulaCellId) -> Option<Done> {
        let sh = self.par_state()?;
        self.par_pos = *sh.pos.get(*sh.local.get(id)?)?;
        let f = self.wb.formulas.get(id)?;
        let formula = f.formula.clone();
        let addr = f.addr;
        let is_array = f.array.is_some();
        let env = if f.has_names {
            EvalEnv::new(crate::names::collect_names(&self.wb.names, &formula.expr, addr.sheet).0)
        } else {
            EvalEnv::default()
        };
        self.origin = addr;
        self.depth = 1;
        self.eval_stack.push(Frame {
            id,
            circular: false,
            deferred: false,
        });
        let (value, matrix) = self.compute(&formula.expr, is_array, &env);
        self.par_work = env.work();
        let frame = self.eval_stack.pop();
        if self.bailed || frame.is_none_or(|f| f.circular || f.deferred) {
            return None;
        }
        let value = match value {
            Value::Number(n) if self.precision => Value::Number(shown_number(&self.wb, addr, n)),
            other => other,
        };
        Some(Done {
            value,
            matrix: if is_array { matrix } else { None },
            dynamic: std::mem::take(&mut self.dyn_deps),
            dynamic_spans: std::mem::take(&mut self.dyn_spans),
        })
    }
}

/// Stores the result of formula `id` (value already rounded for "precision
/// as shown"), marks it clean and re-registers its dynamic precedents when
/// they changed.
fn store_result(wb: &mut Workbook, id: FormulaCellId, done: Done) {
    let Done {
        value,
        matrix,
        dynamic,
        dynamic_spans,
    } = done;
    let Some(f) = wb.formulas.get_mut(id) else {
        return;
    };
    f.cached = value;
    if f.array.is_some() {
        f.array_result = matrix;
    }
    f.state = CellState::Clean;
    if dynamic != f.dynamic_deps || dynamic_spans != f.dynamic_spans() {
        f.dynamic_deps = dynamic;
        if dynamic_spans != f.dynamic_spans() {
            f.set_dynamic_spans(dynamic_spans);
        }
        let mut all = f.precedents.clone();
        all.extend(f.dynamic_deps.iter().copied());
        let spans = f.all_spans();
        let vol = f.volatile;
        wb.deps.remove(id);
        wb.deps.add(id, &all);
        wb.deps.add_spans(id, &spans);
        wb.deps.set_volatile(id, vol);
    }
}

/// A formula result computed by the parallel pre-pass, not stored yet.
#[derive(Debug)]
pub(crate) struct Done {
    /// The cell value (rounded for "precision as shown").
    value: Value,
    /// The result matrix of an array formula (`None`: a scalar fills it).
    matrix: Option<Arc<Matrix>>,
    /// Dynamic precedents recorded by the evaluation.
    dynamic: Vec<CellRange>,
    /// Dynamic 3-D precedents recorded by the evaluation.
    dynamic_spans: Vec<Span3d>,
}

/// What the workers of the parallel pre-pass share: which dirty formulas
/// are pre-computed (`slots`, indexed like the recalc's dirty list), the
/// position of each in the sequential order, and the lookup index cache,
/// moved out of the workbook for the pre-pass.
pub(crate) struct ParShared {
    /// Dirty formula → index into `slots`.
    pub(crate) local: SecondaryMap<FormulaCellId, usize>,
    /// Index of each dirty formula's component in the sequential order.
    pub(crate) pos: Vec<u32>,
    /// Pre-computed results, set once by the formula's own task.
    pub(crate) slots: Vec<std::sync::OnceLock<Done>>,
    /// The recalc's lookup cache during the pre-pass.
    pub(crate) cache: std::sync::Mutex<ParCache>,
    /// What the pre-pass spent on discarded work (set when it ends).
    pub(crate) stats: PrepassStats,
}

/// What the parallel pre-pass spent on work it discarded.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PrepassStats {
    /// Evaluations repeated after waiting for a precedent.
    pub(crate) retries: usize,
    /// Work units of discarded (bailed) evaluations.
    pub(crate) wasted_work: u64,
}

/// A parallel read of a dirty formula.
enum SlotRead<'a> {
    /// Pre-computed and final for the reader.
    Done(&'a Done),
    /// Not computed yet (node index): the reader may wait for it.
    Pending(usize),
    /// Never final for the reader (not in the dirty set, or ordered after
    /// it): the reader is left to the sequential pass.
    Never,
}

/// The lookup cache during the pre-pass.
#[derive(Default)]
pub(crate) struct ParCache {
    /// The recalc's cache (moved back after the pre-pass).
    pub(crate) cache: Option<LookupCache>,
    /// Lines whose index holds pre-computed formulas: the latest position
    /// among them. A formula ordered before that position may not use the
    /// index (it would read a formula the sequential pass has not reached),
    /// and the sequential pass does not get these indexes.
    pub(crate) with_dirty: HashMap<CellRange, u32>,
}

impl ParShared {
    /// Takes the pre-computed result of `id` out (the sequential pass
    /// stores it).
    pub(crate) fn take(&mut self, id: FormulaCellId) -> Option<Done> {
        let i = *self.local.get(id)?;
        self.slots.get_mut(i)?.take()
    }

    /// The pre-computed result of `id` if it is final for a formula at
    /// position `me` of the sequential order: computed, and ordered before
    /// `me`, so the sequential pass has stored it when it reaches `me`
    /// (a later one is reached only through a dynamic reference found now;
    /// reading it would change where the sequential pass evaluates it).
    /// Otherwise whether it is still to come (worth waiting for) or will
    /// never be final for `me`.
    fn read(&self, id: FormulaCellId, me: u32) -> SlotRead<'_> {
        let Some(&i) = self.local.get(id) else {
            return SlotRead::Never;
        };
        if self.pos.get(i).is_none_or(|p| *p >= me) {
            return SlotRead::Never;
        }
        match self.slots.get(i).and_then(std::sync::OnceLock::get) {
            Some(d) => SlotRead::Done(d),
            None => SlotRead::Pending(i),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, ParCache> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<'w> CalcCtx<'w> {
    fn formula_value(&mut self, id: FormulaCellId) -> Value {
        let Some(f) = self.wb.formulas.get(id) else {
            return Value::Empty;
        };
        if let Some(sh) = self.par_state() {
            return match f.state {
                CellState::Clean => f.cached.clone(),
                _ => match self.par_done(sh, id) {
                    Some(d) => d.value.clone(),
                    None => Value::Error(ErrorKind::Circular),
                },
            };
        }
        match f.state {
            CellState::Clean => f.cached.clone(),
            CellState::Evaluating => {
                if self.in_iteration(id) {
                    return self.iterated_value(id);
                }
                self.cycle_followed(id);
                Value::Error(ErrorKind::Circular)
            }
            CellState::Dirty => {
                let again = self.incomplete.contains(&id) && !self.in_iteration(id);
                if again || self.depth >= MAX_ON_DEMAND_DEPTH {
                    if self.in_iteration(id) {
                        // Too deep to nest inside the iterated component:
                        // this round reads the previous value.
                        return self.iterated_value(id);
                    }
                    // Too deep to nest: defer it; this attempt is discarded.
                    if self.deferred_seen.insert(id) {
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
        if self.in_iteration(id) {
            return;
        }
        if let Some(p) = self.eval_stack.iter().rposition(|f| f.id == id) {
            for f in &mut self.eval_stack[p..] {
                f.circular = true;
            }
        }
    }

    /// `lookup_index` in the parallel pre-pass: the shared cache behind its
    /// mutex; an index is built only from final values (clean or
    /// pre-computed formulas), so it is the index the sequential pass
    /// would build.
    fn par_lookup_index(
        &mut self,
        sh: &'w ParShared,
        line: &CellRange,
    ) -> Option<Arc<LookupIndex>> {
        if self.bailed {
            // The result is discarded anyway: neither build nor count a
            // failure against the line (security review 7, M2).
            return None;
        }
        let bounds = self.used_bounds(line)?;
        let cells = bounds.cell_count();
        if cells < LookupIndex::MIN_CELLS {
            return None;
        }
        let cells = usize::try_from(cells).unwrap_or(usize::MAX);
        let me = self.par_pos;
        {
            let mut g = sh.cache();
            let ParCache { cache, with_dirty } = &mut *g;
            match cache.as_mut()?.request(line, cells) {
                Request::Hit(ix) => {
                    // An index over formulas this one may not read yet:
                    // scan (and bail on them).
                    if with_dirty.get(line).is_some_and(|p| *p >= me) {
                        return None;
                    }
                    return Some(ix);
                }
                Request::Scan => return None,
                Request::Build => {}
            }
        }
        // A failure here is about this reader's position in the order, not
        // about the line: the line goes back to how it was, uncounted
        // (security review 7, M2), so the sequential pass (or a later
        // reader) can still index it.
        let release = |sh: &ParShared| {
            if let Some(c) = sh.cache().cache.as_mut() {
                c.release(line);
            }
        };
        let mut found = Vec::new();
        formulas_in(&self.wb, &bounds, &mut found);
        let mut latest: Option<u32> = None;
        for id in &found {
            if self
                .wb
                .formulas
                .get(*id)
                .is_some_and(|f| f.state == CellState::Clean)
            {
                continue;
            }
            // The sequential pass evaluates the line's dirty formulas
            // before building (`ensure_clean`): one not final for this
            // reader means it must wait, or run sequentially.
            if self.par_done(sh, *id).is_none() {
                release(sh);
                return None;
            }
            if let Some(&i) = sh.local.get(*id) {
                let p = sh.pos.get(i).copied().unwrap_or(0);
                latest = Some(latest.map_or(p, |l| l.max(p)));
            }
        }
        let mut items = Vec::new();
        let (r0, c0) = (line.r0, line.c0);
        self.for_each_in_range(&bounds, true, &mut |a, v| {
            items.push(((a.row - r0) + (a.col - c0), v.clone()));
            ControlFlow::Continue(())
        });
        if self.bailed {
            release(sh);
            return None;
        }
        let ix = Arc::new(LookupIndex::build(items, self.wb.settings.case_sensitive));
        let mut g = sh.cache();
        if let Some(c) = g.cache.as_mut() {
            c.insert(*line, ix.clone());
        }
        if let Some(l) = latest {
            g.with_dirty.insert(*line, l);
        }
        Some(ix)
    }

    fn ensure_clean(&mut self, range: &CellRange) {
        if let Some(sh) = self.par_state() {
            self.par_ensure_final(sh, range);
            return;
        }
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

    /// [`Self::ensure_clean`] in the parallel pre-pass. Nothing is evaluated
    /// on demand; instead the evaluation bails wherever the sequential pass
    /// would have done more than read final values: a dirty formula of the
    /// range that is not pre-computed and ordered earlier (the sequential
    /// pass evaluates it on demand here, even if the reader then stops
    /// reading at an earlier error), or the reader itself inside the range
    /// (a cycle, Err:522). Correctness review 7, finding 2.
    fn par_ensure_final(&mut self, sh: &'w ParShared, range: &CellRange) {
        if self.bailed {
            return;
        }
        let inside = self.eval_stack.iter().any(|fr| {
            self.wb
                .formulas
                .get(fr.id)
                .is_some_and(|f| f.extent().intersects(range))
        });
        if inside {
            self.bail_for_good();
            return;
        }
        let mut ids = Vec::new();
        dirty_formulas_in(&self.wb, range, &mut ids);
        for id in ids {
            if self.par_done(sh, id).is_none() {
                return;
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

/// `n` rounded to the decimals the cell's number format shows ("precision
/// as shown"); `General` (unless its decimals are limited), dates and
/// fraction formats keep the full value,
/// scientific formats round to the mantissa's decimals.
fn shown_number(wb: &Workbook, addr: CellAddr, n: f64) -> f64 {
    let id = wb.style_at(addr);
    let st = wb.styles.get(id);
    match crate::calcsettings::shown_decimals(&wb.settings, &st.format, n) {
        Some(d) => crate::calcsettings::round_to_decimals(n, d),
        None => n,
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
        match crate::names::resolve_chain(&self.names, &f.expr, origin.sheet) {
            Expr::Ref(r) => match r.resolve(origin) {
                ResolvedRef::Range(cr) if self.sheet(cr.sheet).is_some() => Some(cr),
                _ => None,
            },
            _ => None,
        }
    }

    /// [`Workbook::parse_reference_at`] for text spanning several sheets:
    /// the first and last sheets and the rectangle.
    pub fn parse_reference_3d_at(
        &self,
        text: &str,
        origin: CellAddr,
    ) -> Option<(SheetId, SheetId, CellRange)> {
        let t = text.trim();
        if t.is_empty() || t.starts_with('=') {
            return None;
        }
        let f = self.parse_formula(&format!("={t}"), origin).ok()?;
        match crate::names::resolve_chain(&self.names, &f.expr, origin.sheet) {
            Expr::Ref(r) => match r.resolve(origin) {
                ResolvedRef::Span3d { first, last, area }
                    if self.sheet(first).is_some() && self.sheet(last).is_some() =>
                {
                    Some((first, last, area))
                }
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
        let env = EvalEnv::new(crate::names::collect_names(&self.names, &f.expr, addr.sheet).0);
        let mut ctx = CalcCtx::shared_at(self, addr);
        let v = eval_with(&f.expr, &mut ctx, false, &env);
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

    fn locale_tag(&self) -> &str {
        self.wb.locale.tag
    }

    fn cell_value(&mut self, addr: CellAddr) -> Value {
        if self.bailed {
            // Parallel pre-pass: the result is discarded; stop reading.
            return Value::Error(ErrorKind::Circular);
        }
        if !addr.is_valid() {
            return Value::Error(ErrorKind::Ref);
        }
        let Some(sheet) = self.wb.sheet(addr.sheet) else {
            return Value::Error(ErrorKind::Ref);
        };
        match sheet.get(addr.row, addr.col) {
            None => Value::Empty,
            Some(CellContent::Value(v)) => {
                let v = v.clone();
                self.shown(addr, v)
            }
            Some(CellContent::Formula(id)) => {
                let id = *id;
                // Formula results are stored already rounded.
                self.formula_value(id)
            }
            Some(CellContent::ArrayPart(id)) => {
                let id = *id;
                if let Some(sh) = self.par_state() {
                    let f = self.wb.formulas.get(id);
                    return match f {
                        None => Value::Empty,
                        Some(f) if f.state == CellState::Clean => {
                            f.array_value(addr.row - f.addr.row, addr.col - f.addr.col)
                        }
                        Some(f) => {
                            let (r0, c0) = (f.addr.row, f.addr.col);
                            match self.par_done(sh, id) {
                                Some(d) => crate::formula_cells::array_value_of(
                                    &d.value,
                                    d.matrix.as_ref(),
                                    addr.row - r0,
                                    addr.col - c0,
                                ),
                                None => Value::Error(ErrorKind::Circular),
                            }
                        }
                    };
                }
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
        if self.bailed {
            // Parallel pre-pass: the result is discarded; stop reading.
            return;
        }
        let iter_set = self.iter_set.clone();
        let precision = self.precision;
        let par = self.par_state();
        let my_pos = self.par_pos;
        let bailed = std::cell::Cell::new(false);
        let bailed_for_good = std::cell::Cell::new(false);
        let wait = std::cell::Cell::new(None::<usize>);
        // Parallel pre-pass: a dirty formula's final value, or bail.
        let slot = |id: FormulaCellId| -> Option<&Done> {
            match par.map(|sh| sh.read(id, my_pos)) {
                Some(SlotRead::Done(d)) => Some(d),
                Some(SlotRead::Pending(i)) => {
                    bailed.set(true);
                    if wait.get().is_none() {
                        wait.set(Some(i));
                    }
                    None
                }
                _ => {
                    bailed.set(true);
                    bailed_for_good.set(true);
                    None
                }
            }
        };
        let wb = &*self.wb;
        let Some(sheet) = wb.sheet(range.sheet) else {
            return;
        };
        let circular = Value::Error(ErrorKind::Circular);
        let empty = Value::Empty;
        let iterated = |id: FormulaCellId| iter_set.as_ref().is_some_and(|s| s.contains(&id));
        'cols: for c in range.c0..=c1 {
            let col = sheet.column(c);
            let value_of = |content: &'_ CellContent, r: u32| -> Option<Value> {
                match content {
                    CellContent::Value(_) => None,
                    CellContent::Formula(id) => Some(match wb.formulas.get(*id) {
                        Some(fc) if fc.state == CellState::Clean => fc.cached.clone(),
                        Some(_) if par.is_some() => match slot(*id) {
                            Some(d) => d.value.clone(),
                            None => circular.clone(),
                        },
                        Some(fc) if iterated(*id) => match &fc.cached {
                            Value::Empty => Value::Number(0.0),
                            v => v.clone(),
                        },
                        Some(_) => circular.clone(),
                        None => Value::Empty,
                    }),
                    CellContent::ArrayPart(id) => Some(match wb.formulas.get(*id) {
                        Some(fc) if fc.state == CellState::Clean || iterated(*id) => fc
                            .array_value(
                                r.saturating_sub(fc.addr.row),
                                c.saturating_sub(fc.addr.col),
                            ),
                        Some(fc) if par.is_some() => match slot(*id) {
                            Some(d) => crate::formula_cells::array_value_of(
                                &d.value,
                                d.matrix.as_ref(),
                                r.saturating_sub(fc.addr.row),
                                c.saturating_sub(fc.addr.col),
                            ),
                            None => circular.clone(),
                        },
                        Some(_) => circular.clone(),
                        None => Value::Empty,
                    }),
                }
            };
            // "Precision as shown" for constant numbers (formula results
            // are stored rounded).
            let shown = |r: u32, v: &Value| -> Option<Value> {
                if !precision {
                    return None;
                }
                match v {
                    Value::Number(n) => {
                        let st = wb.styles.get(sheet.style(r, c));
                        crate::calcsettings::shown_decimals(&wb.settings, &st.format, *n)
                            .map(|d| Value::Number(crate::calcsettings::round_to_decimals(*n, d)))
                    }
                    _ => None,
                }
            };
            if skip_empty {
                let Some(col) = col else {
                    continue;
                };
                for (r, content) in col.cells.iter_range(range.r0, r1) {
                    let addr = CellAddr::new(range.sheet, r, c);
                    let flow = match content {
                        CellContent::Value(v) => match shown(r, v) {
                            Some(rounded) => f(addr, &rounded),
                            None => f(addr, v),
                        },
                        other => {
                            let v = value_of(other, r).unwrap_or(Value::Empty);
                            f(addr, &v)
                        }
                    };
                    if flow.is_break() || bailed.get() {
                        break 'cols;
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
                                CellContent::Value(v) => match shown(r, v) {
                                    Some(rounded) => f(addr, &rounded),
                                    None => f(addr, v),
                                },
                                other => {
                                    let v = value_of(other, r).unwrap_or(Value::Empty);
                                    f(addr, &v)
                                }
                            }
                        }
                        _ => f(addr, &empty),
                    };
                    if flow.is_break() || bailed.get() {
                        break 'cols;
                    }
                }
            }
        }
        if bailed.get() {
            self.bailed = true;
            self.bail_final |= bailed_for_good.get();
            if let Some(i) = wait.get() {
                self.wait_on.get_or_insert(i);
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

    fn parse_reference_text_3d(&self, text: &str) -> Option<(SheetId, SheetId, CellRange)> {
        self.wb.parse_reference_3d_at(text, self.origin)
    }

    fn record_dynamic_dep(&mut self, range: &CellRange) {
        const LINEAR: usize = 16;
        let fresh = match &mut self.dyn_seen {
            Some(set) => set.insert(*range),
            None if self.dyn_deps.len() < LINEAR => !self.dyn_deps.contains(range),
            None => {
                let mut set: HashSet<CellRange> = self.dyn_deps.iter().copied().collect();
                let fresh = set.insert(*range);
                self.dyn_seen = Some(set);
                fresh
            }
        };
        if fresh {
            self.dyn_deps.push(*range);
        }
    }

    fn record_dynamic_span(&mut self, first: SheetId, last: SheetId, rect: CellRange) {
        let s = Span3d { first, last, rect };
        if !self.dyn_spans.contains(&s) {
            self.dyn_spans.push(s);
        }
    }

    fn now_serial(&self) -> f64 {
        self.now
    }

    fn abandoned(&self) -> bool {
        self.bailed
    }

    fn rand(&mut self) -> f64 {
        if self.par_state().is_some() {
            // The random stream is consumed in the sequential order.
            self.bail_for_good();
            return 0.5;
        }
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

    fn sheet_at(&self, index: u32) -> Option<SheetId> {
        self.wb.sheets.get(index as usize).map(|s| s.id)
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

    /// `MULTIPLE.OPERATIONS`: re-evaluates `formula` with `subs` in place
    /// of the input cells, through a sandbox that recomputes only the
    /// formulas between the inputs and `formula` (see [`SubCtx`]). At most
    /// [`crate::tools::MAX_TABLE_OPS`] calls per recalc; past that the
    /// result is Err:512.
    fn table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value> {
        if self.par_state().is_some() {
            // Per-recalc caps and the dependency walk are order-dependent.
            self.bail_for_good();
            return Some(Value::Error(ErrorKind::FormulaOverflow));
        }
        let calls = match self.wb_mut() {
            Some(wb) => {
                wb.table_op_calls = wb.table_op_calls.saturating_add(1);
                wb.table_op_calls
            }
            None => {
                self.local_table_ops = self.local_table_ops.saturating_add(1);
                self.local_table_ops
            }
        };
        if calls > crate::tools::MAX_TABLE_OPS {
            return Some(Value::Error(ErrorKind::FormulaOverflow));
        }
        self.wb.formula_at(formula)?;
        let mut sub = SubCtx::new(self, subs, 0);
        Some(sub.run(formula))
    }

    fn pivot_data(
        &mut self,
        at: CellAddr,
        field: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        crate::pivot::pivot_data(&self.wb, at, field, filters, false)
    }

    /// The running recalc's index of `line` (see [`crate::lookup_cache`]):
    /// built on the second request, once the line's formulas are all
    /// clean; never while iterating a cyclic component or outside a recalc.
    fn lookup_index(&mut self, line: &CellRange) -> Option<Arc<LookupIndex>> {
        if let Some(sh) = self.par_state() {
            return self.par_lookup_index(sh, line);
        }
        if self.iter_set.is_some() || self.wb.lookup_cache.is_none() {
            return None;
        }
        let bounds = self.used_bounds(line)?;
        let cells = bounds.cell_count();
        if cells < LookupIndex::MIN_CELLS {
            return None;
        }
        let cells = usize::try_from(cells).unwrap_or(usize::MAX);
        match self.wb_mut()?.lookup_cache.as_mut()?.request(line, cells) {
            Request::Hit(ix) => return Some(ix),
            Request::Scan => return None,
            Request::Build => {}
        }
        self.ensure_clean(&bounds);
        if !all_formulas_clean(&self.wb, &bounds) {
            // A cycle or a deferred precedent: values are not final yet.
            self.wb_mut()?.lookup_cache.as_mut()?.not_clean(line);
            return None;
        }
        let mut items = Vec::new();
        let (r0, c0) = (line.r0, line.c0);
        self.for_each_in_range(&bounds, true, &mut |a, v| {
            // One of the two differences is 0: the line is one column or row.
            items.push(((a.row - r0) + (a.col - c0), v.clone()));
            ControlFlow::Continue(())
        });
        let ix = Arc::new(LookupIndex::build(items, self.wb.settings.case_sensitive));
        self.wb_mut()?
            .lookup_cache
            .as_mut()?
            .insert(*line, ix.clone());
        Some(ix)
    }

    fn pivot_data_excel(
        &mut self,
        at: CellAddr,
        words: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        crate::pivot::pivot_data(&self.wb, at, words, filters, true)
    }
}

/// Most formulas the `MULTIPLE.OPERATIONS` sandbox recomputes per call;
/// a larger dependency cone gives Err:512 instead of a partial sandbox
/// (security review 4, LOW-P10).
const MAX_SUB_AFFECTED: usize = 200_000;
/// `MULTIPLE.OPERATIONS` sandbox work per recalc, all calls together:
/// formulas found by the dependency walks, formulas re-evaluated and
/// sandbox entries scanned by range reads. Past it the calls give Err:512
/// (security review 4, H3: 300 calls over 199k formulas took 23 s per
/// edit, 10,000 about 15 min).
pub const MAX_TABLE_OP_WORK: u64 = 20_000_000;
/// Deepest nesting of `MULTIPLE.OPERATIONS` inside a sandbox.
const MAX_TABLE_OP_NESTING: u32 = 8;
/// Largest range the sandbox iterates cell by cell.
const MAX_SUB_RANGE_CELLS: u64 = 1 << 20;
/// Deepest nesting of sandbox re-evaluations.
const MAX_SUB_DEPTH: u32 = 64;

/// The `MULTIPLE.OPERATIONS` sandbox: an [`EvalContext`] over the recalc
/// context in which the input cells have the substituted values and every
/// formula that (transitively) reads them is recomputed on the fly, memoized
/// per call; everything else reads the workbook as is. Nothing is written.
struct SubCtx<'a, 'w> {
    inner: &'a mut CalcCtx<'w>,
    subs: Vec<(CellAddr, Value)>,
    /// Anchors of the formulas depending on the inputs.
    affected: HashSet<CellAddr>,
    /// The extents of the affected formulas, and their bounding box per
    /// sheet (for [`Self::touches`]).
    extents: Vec<CellRange>,
    bounds: HashMap<SheetId, CellRange>,
    /// The dependency cone exceeded [`MAX_SUB_AFFECTED`] or the recalc's
    /// [`MAX_TABLE_OP_WORK`] ran out: the call gives Err:512.
    over: bool,
    /// `MULTIPLE.OPERATIONS` nesting level of this sandbox.
    nest: u32,
    memo: HashMap<CellAddr, Value>,
    arrays: HashMap<CellAddr, Value>,
    in_progress: HashSet<CellAddr>,
    origin: CellAddr,
    depth: u32,
}

impl<'a, 'w> SubCtx<'a, 'w> {
    fn new(inner: &'a mut CalcCtx<'w>, subs: &[(CellAddr, Value)], nest: u32) -> Self {
        // The formulas reachable from the inputs through the reverse index.
        let mut affected: HashSet<CellAddr> = HashSet::new();
        let mut extents: Vec<CellRange> = Vec::new();
        let mut bounds: HashMap<SheetId, CellRange> = HashMap::new();
        let mut frontier: Vec<CellRange> =
            subs.iter().map(|(a, _)| CellRange::single(*a)).collect();
        let mut found: Vec<FormulaCellId> = Vec::new();
        let mut seen: HashSet<FormulaCellId> = HashSet::new();
        let mut over = false;
        while !frontier.is_empty() {
            if affected.len() >= MAX_SUB_AFFECTED {
                over = true;
                break;
            }
            found.clear();
            inner.wb.deps.dependents_of(&frontier, &mut found);
            if !inner.charge_table_op_work(found.len() as u64 + 1) {
                over = true;
                break;
            }
            let mut next = Vec::new();
            for id in found.drain(..) {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(f) = inner.wb.formulas.get(id) {
                    affected.insert(f.addr);
                    let e = f.extent();
                    extents.push(e);
                    bounds
                        .entry(e.sheet)
                        .and_modify(|b| {
                            *b = CellRange::new(
                                e.sheet,
                                b.r0.min(e.r0),
                                b.c0.min(e.c0),
                                b.r1.max(e.r1),
                                b.c1.max(e.c1),
                            );
                        })
                        .or_insert(e);
                    next.push(e);
                }
            }
            frontier = next;
        }
        let origin = inner.origin;
        Self {
            inner,
            subs: subs.to_vec(),
            affected,
            extents,
            bounds,
            over,
            nest,
            memo: HashMap::new(),
            arrays: HashMap::new(),
            in_progress: HashSet::new(),
            origin,
            depth: 0,
        }
    }

    /// The value of `formula` in the sandbox, or Err:512 when the sandbox
    /// is incomplete or over budget.
    fn run(&mut self, formula: CellAddr) -> Value {
        if self.over {
            return Value::Error(ErrorKind::FormulaOverflow);
        }
        let v = self.eval_cell(formula, false);
        if self.over {
            return Value::Error(ErrorKind::FormulaOverflow);
        }
        v
    }

    /// `true` if a read of `range` sees a substituted or recomputed cell.
    fn touches(&mut self, range: &CellRange) -> bool {
        if self.subs.iter().any(|(a, _)| range.contains(*a)) {
            return true;
        }
        if !self
            .bounds
            .get(&range.sheet)
            .is_some_and(|b| b.intersects(range))
        {
            return false;
        }
        if !self.inner.charge_table_op_work(self.extents.len() as u64) {
            self.over = true;
            return false;
        }
        self.extents.iter().any(|e| e.intersects(range))
    }

    /// The value of `addr` inside the sandbox.
    fn eval_cell(&mut self, addr: CellAddr, force: bool) -> Value {
        if let Some((_, v)) = self.subs.iter().find(|(a, _)| *a == addr) {
            return v.clone();
        }
        let content = self
            .inner
            .wb
            .sheet(addr.sheet)
            .and_then(|s| s.get(addr.row, addr.col))
            .cloned();
        match content {
            Some(CellContent::Formula(id)) => {
                if force || self.affected.contains(&addr) {
                    self.eval_formula(addr, id)
                } else {
                    self.inner.cell_value(addr)
                }
            }
            Some(CellContent::ArrayPart(id)) => {
                let anchor = self.inner.wb.formulas.get(id).map(|f| f.addr);
                match anchor {
                    Some(anchor) if self.affected.contains(&anchor) => {
                        let m = self.eval_formula(anchor, id);
                        match m {
                            Value::Array(m) => m
                                .get(addr.row - anchor.row, addr.col - anchor.col)
                                .cloned()
                                .unwrap_or(Value::Error(ErrorKind::NA)),
                            other => other,
                        }
                    }
                    _ => self.inner.cell_value(addr),
                }
            }
            _ => self.inner.cell_value(addr),
        }
    }

    fn eval_formula(&mut self, addr: CellAddr, id: FormulaCellId) -> Value {
        let Some(f) = self.inner.wb.formulas.get(id) else {
            return Value::Empty;
        };
        let is_array = f.array.is_some();
        if let Some(v) = if is_array {
            self.arrays.get(&addr)
        } else {
            self.memo.get(&addr)
        } {
            return v.clone();
        }
        if self.in_progress.contains(&addr) || self.depth >= MAX_SUB_DEPTH {
            return Value::Error(ErrorKind::Circular);
        }
        if f.invalid_text.is_some() {
            return f.cached.clone();
        }
        let formula = f.formula.clone();
        let has_names = f.has_names;
        if self.over || !self.inner.charge_table_op_work(1) {
            self.over = true;
            return Value::Error(ErrorKind::FormulaOverflow);
        }
        let env = if has_names {
            EvalEnv::new(
                crate::names::collect_names(&self.inner.wb.names, &formula.expr, addr.sheet).0,
            )
        } else {
            EvalEnv::default()
        };
        self.in_progress.insert(addr);
        self.depth += 1;
        let saved = std::mem::replace(&mut self.origin, addr);
        let v = eval_with(&formula.expr, self, is_array, &env);
        let out = if is_array {
            array_result(&v, self)
        } else {
            cell_result(&v, self)
        };
        self.origin = saved;
        self.depth -= 1;
        self.in_progress.remove(&addr);
        if is_array {
            self.arrays.insert(addr, out.clone());
        } else {
            self.memo.insert(addr, out.clone());
        }
        out
    }
}

impl EvalContext for SubCtx<'_, '_> {
    fn origin(&self) -> CellAddr {
        self.origin
    }

    fn settings(&self) -> &CalcSettings {
        self.inner.settings()
    }

    fn locale_tag(&self) -> &str {
        self.inner.locale_tag()
    }

    fn cell_value(&mut self, addr: CellAddr) -> Value {
        if !addr.is_valid() {
            return Value::Error(ErrorKind::Ref);
        }
        self.eval_cell(addr, false)
    }

    fn for_each_in_range(
        &mut self,
        range: &CellRange,
        skip_empty: bool,
        f: &mut dyn FnMut(CellAddr, &Value) -> ControlFlow<()>,
    ) {
        let r1 = range.r1.min(MAX_ROWS - 1);
        let c1 = range.c1.min(MAX_COLS - 1);
        if range.r0 > r1 || range.c0 > c1 {
            return;
        }
        let clipped = CellRange::new(range.sheet, range.r0, range.c0, r1, c1);
        if clipped.cell_count() > MAX_SUB_RANGE_CELLS || !self.touches(&clipped) {
            return self.inner.for_each_in_range(range, skip_empty, f);
        }
        for c in clipped.c0..=c1 {
            for r in clipped.r0..=r1 {
                let addr = CellAddr::new(range.sheet, r, c);
                let v = self.eval_cell(addr, false);
                if skip_empty && v.is_empty() {
                    continue;
                }
                if f(addr, &v).is_break() {
                    return;
                }
            }
        }
    }

    fn used_bounds(&self, range: &CellRange) -> Option<CellRange> {
        self.inner.used_bounds(range)
    }

    fn parse_reference_text(&self, text: &str, a1: bool) -> Option<CellRange> {
        self.inner.parse_reference_text(text, a1)
    }

    fn parse_reference_text_3d(&self, text: &str) -> Option<(SheetId, SheetId, CellRange)> {
        self.inner.parse_reference_text_3d(text)
    }

    fn record_dynamic_dep(&mut self, range: &CellRange) {
        self.inner.record_dynamic_dep(range);
    }

    fn record_dynamic_span(&mut self, first: SheetId, last: SheetId, rect: CellRange) {
        self.inner.record_dynamic_span(first, last, rect);
    }

    fn now_serial(&self) -> f64 {
        self.inner.now_serial()
    }

    fn rand(&mut self) -> f64 {
        self.inner.rand()
    }

    fn sheet_index(&self, id: SheetId) -> Option<u32> {
        self.inner.sheet_index(id)
    }

    fn sheet_count(&self) -> u32 {
        self.inner.sheet_count()
    }

    fn sheet_at(&self, index: u32) -> Option<SheetId> {
        self.inner.sheet_at(index)
    }

    fn cell_format_info(&self, addr: CellAddr) -> CellFormatInfo {
        self.inner.cell_format_info(addr)
    }

    fn hidden_row(&self, sheet: SheetId, row: u32) -> bool {
        self.inner.hidden_row(sheet, row)
    }

    fn filtered_row(&self, sheet: SheetId, row: u32) -> bool {
        self.inner.filtered_row(sheet, row)
    }

    /// A nested `MULTIPLE.OPERATIONS`: a sandbox in which both this
    /// sandbox's substitutions and the inner ones (which win) are in
    /// effect, as AOO's table-op stack does (sc/source/core/tool/
    /// interpr4.cxx:2984-3075; correctness review 4, M31).
    fn table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value> {
        if self.nest + 1 > MAX_TABLE_OP_NESTING {
            return Some(Value::Error(ErrorKind::FormulaOverflow));
        }
        self.inner.wb.formula_at(formula)?;
        let mut combined: Vec<(CellAddr, Value)> = subs.to_vec();
        for (a, v) in &self.subs {
            if !combined.iter().any(|(b, _)| b == a) {
                combined.push((*a, v.clone()));
            }
        }
        let nest = self.nest + 1;
        let mut sub = SubCtx::new(&mut *self.inner, &combined, nest);
        let v = sub.run(formula);
        if sub.over {
            self.over = true;
        }
        Some(v)
    }

    fn pivot_data(
        &mut self,
        at: CellAddr,
        field: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        self.inner.pivot_data(at, field, filters)
    }

    fn pivot_data_excel(
        &mut self,
        at: CellAddr,
        words: &str,
        filters: &[(String, String)],
    ) -> Option<Value> {
        self.inner.pivot_data_excel(at, words, filters)
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

    /// Whether a hook is set on this thread.
    pub fn active() -> bool {
        HOOK.with(|h| h.borrow().is_some())
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

    /// `A1..A15` nest on demand down to `A16`, which reads `k` dirty
    /// formulas at depth 16 (all deferred).
    fn deferral_book(k: u32) -> Workbook {
        let mut wb = Workbook::new();
        wb.threading = crate::parallel::Threading::Sequential;
        for i in 1..16 {
            wb.set_input(a(&format!("A{i}")), &format!("=INDIRECT(\"A{}\")", i + 1));
        }
        wb.set_input(a("A16"), &format!("=SUM(INDIRECT(\"B1:B{k}\"))"));
        for r in 1..=k {
            wb.set_input(a(&format!("B{r}")), "=1");
        }
        wb
    }

    /// Security review 7, H1: the formula that deferred `k` cells is
    /// attempted twice in all (it was attempted k + 1 times, each attempt
    /// re-walking the chain).
    #[test]
    fn deferred_siblings_retry_their_reader_once() {
        let k = 500;
        let mut wb = deferral_book(k);
        test_hook::set(Some((a("A16"), CellRange::from_a1(1, "Z999").unwrap())));
        let rep = recalc(&mut wb);
        let count = test_hook::count();
        test_hook::set(None);
        assert_eq!(num(&wb, "A1"), Value::Number(f64::from(k)));
        assert_eq!(rep.circular, 0);
        assert!(count <= 3, "A16 evaluated {count} times");
    }

    /// Security review 7, H1: the watchdog deadline is checked inside the
    /// deferred-evaluation work stack; what is left keeps its old value.
    #[test]
    fn the_deadline_stops_the_deferred_work_stack() {
        let k = 200;
        let mut wb = deferral_book(k);
        let ids: Vec<FormulaCellId> = wb.formulas.keys().collect();
        for id in &ids {
            if let Some(f) = wb.formulas.get_mut(*id) {
                f.state = CellState::Dirty;
            }
        }
        let (a1, _) = wb.formula_at(a("A1")).unwrap();
        let mut ctx = CalcCtx::new(&mut wb, 0.0);
        ctx.deadline = Some(Instant::now());
        ctx.evaluate(a1);
        assert!(ctx.timed_out);
        // A1 and the k deferred cells.
        assert_eq!(ctx.skipped, k as usize + 1);
        assert!(wb
            .formulas
            .values()
            .all(|f| f.state != CellState::Evaluating));
        assert_eq!(wb.formulas.get(a1).unwrap().state, CellState::Clean);
        // Through the recalc: incomplete, nothing left dirty.
        let mut wb = deferral_book(k);
        let rep = recalc_with(
            &mut wb,
            &RecalcBudget {
                max_cells_evaluated: u64::MAX,
                deadline: Some(Instant::now() + Duration::from_secs(3600)),
            },
        );
        assert!(!rep.incomplete);
        assert_eq!(num(&wb, "A1"), Value::Number(f64::from(k)));
    }

    /// Security review 7, H2: past the edge cap no precedent lists are kept.
    #[test]
    fn topo_order_drops_the_edges_past_the_cap() {
        let mut wb = Workbook::new();
        for r in 1..=40u32 {
            wb.set_input(a(&format!("A{r}")), &format!("={r}"));
            wb.set_input(a(&format!("B{r}")), "=SUM($A$1:$A$40)");
        }
        let dirty: Vec<FormulaCellId> = wb.formulas.keys().collect();
        // 40 × 40 edges.
        let (order, preds) = topo_order(&wb, &dirty, Some(1600));
        let preds = preds.expect("within the cap");
        assert_eq!(preds.iter().map(Vec::len).sum::<usize>(), 1600);
        let (order2, none) = topo_order(&wb, &dirty, Some(1599));
        assert!(none.is_none());
        assert_eq!(order.len(), order2.len());
        assert_eq!(topo_order(&wb, &dirty, None).1, None);
    }

    /// A parallel context over `wb` whose dirty set is `dirty` (all at
    /// position 0) and a reader at position 1.
    #[cfg(feature = "parallel")]
    fn par_shared(dirty: &[FormulaCellId]) -> ParShared {
        let mut local = SecondaryMap::new();
        for (i, id) in dirty.iter().enumerate() {
            local.insert(*id, i);
        }
        let mut slots = Vec::new();
        slots.resize_with(dirty.len(), std::sync::OnceLock::new);
        ParShared {
            local,
            pos: vec![0; dirty.len()],
            slots,
            cache: std::sync::Mutex::new(ParCache {
                cache: Some(LookupCache::default()),
                with_dirty: HashMap::new(),
            }),
            stats: PrepassStats::default(),
        }
    }

    /// Security review 7, M2 and correctness review 7, finding 4: a
    /// pre-pass reader that already bailed, or that is ordered before a
    /// formula of the line, does not count a failed build; the line is
    /// indexed once it can be.
    #[cfg(feature = "parallel")]
    #[test]
    fn prepass_lookup_failures_do_not_skip_the_line() {
        let mut wb = Workbook::new();
        for r in 1..=200u32 {
            wb.set_input(a(&format!("A{r}")), &r.to_string());
        }
        wb.set_input(a("A201"), "=1+1");
        let (f, _) = wb.formula_at(a("A201")).unwrap();
        let line = CellRange::from_a1(1, "A1:A201").unwrap();
        let mut sh = par_shared(&[f]);
        let reader = |sh: &ParShared, bail: bool| {
            let mut ctx = CalcCtx::par(&wb, sh, 0.0);
            ctx.par_pos = 1;
            if bail {
                ctx.bail_for_good();
            }
            let ix = ctx.lookup_index(&line);
            (ix.is_some(), ctx.bailed, ctx.retry_after())
        };
        // Already bailed (RAND): nothing is requested.
        for _ in 0..5 {
            assert_eq!(reader(&sh, true), (false, true, None));
        }
        // A201 is not computed yet: wait for it, uncounted.
        assert_eq!(reader(&sh, false), (false, false, None), "first request");
        for _ in 0..5 {
            assert_eq!(reader(&sh, false), (false, true, Some(0)));
        }
        // Computed: the next reader builds the index.
        let done = Done {
            value: Value::Number(2.0),
            matrix: None,
            dynamic: Vec::new(),
            dynamic_spans: Vec::new(),
        };
        assert!(sh.slots[0].set(done).is_ok());
        assert_eq!(reader(&sh, false), (true, false, None));
        let _ = sh.take(f);
    }

    /// Correctness review 7, finding 2: a pre-pass reader of a range that
    /// holds a dirty formula not final for it bails, even if the function
    /// stops reading before that cell; and one inside the range bails for
    /// good.
    #[cfg(feature = "parallel")]
    #[test]
    fn prepass_range_reads_check_the_whole_range() {
        let mut wb = Workbook::new();
        wb.set_input(a("B1"), "#DIV/0!");
        wb.set_input(a("B5"), "=1+1");
        let (f, _) = wb.formula_at(a("B5")).unwrap();
        let sh = par_shared(&[f]);
        let range = CellRange::from_a1(1, "B1:B10").unwrap();
        let mut ctx = CalcCtx::par(&wb, &sh, 0.0);
        ctx.par_pos = 1;
        let mut seen = 0;
        ctx.for_each_in_range(&range, true, &mut |_, _| {
            seen += 1;
            ControlFlow::Break(())
        });
        assert_eq!(seen, 0);
        assert!(ctx.bailed);
        assert_eq!(ctx.retry_after(), Some(0));
        // Later in the order than the reader: never final for it.
        let mut ctx = CalcCtx::par(&wb, &sh, 0.0);
        ctx.par_pos = 0;
        ctx.for_each_in_range(&range, true, &mut |_, _| ControlFlow::Break(()));
        assert!(ctx.bailed);
        assert_eq!(ctx.retry_after(), None);
    }
}
