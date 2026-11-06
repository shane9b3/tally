//! Detective (Tools ▸ Detective; wave 5, W5-3): the direct precedents and
//! dependents of a cell as arrow targets, and error-source tracing.
//!
//! Precedents come from the formula's static precedents plus the dynamic
//! references recorded by its last evaluation (INDIRECT, OFFSET, …), one
//! level deep, as AOO's `ScDetectiveFunc::ShowPred`. Dependents are the
//! formulas registered in the reverse index as reading the cell (an array
//! formula's whole block counts as one target). A target is flagged
//! `error` when any of its cells holds an error value (the red arrow).
//! [`error_source`] follows every error precedent of an error cell until
//! it reaches cells whose own precedents are error-free (AOO "Trace
//! Error"). Error flags scan at most [`MAX_ERROR_SCAN_PER_QUERY`] stored
//! cells per query.
//!
//! The menu commands ([`run`]) keep AOO's model (`detfunc.cxx`,
//! `detdata.cxx`, `docfunc.cxx`): each sheet holds its arrows (the draw
//! objects of AOO's internal layer) and the operations that drew them
//! ([`SheetDetective`], AOO's document-wide `ScDetOpList` split by the
//! sheet each operation acts on, which is the only page it draws on).
//! Trace Precedents / Dependents add one level per call (iterative
//! deepening until an arrow is new: `ShowPred` / `ShowSucc`), Remove
//! takes the outermost level away (`FindPredLevel` / `FindSuccLevel`),
//! Trace Error draws down to the error origins (`InsertErrorLevel`).
//! Refresh Traces ([`refresh_all`]) clears every sheet's arrows and
//! replays the list (`ScDocFunc::DetectiveRefresh`); AutoRefresh runs it
//! after changes (`ScDocShell::SetDocumentModified`, see
//! [`crate::Document`]). Operations move with inserted and deleted rows
//! and columns ([`adjust_structure`], `ScDetOpList::UpdateReference`).

use std::collections::{HashMap, HashSet};

use tally_api::{ApiError, RangeDto, TraceDto};
use tally_types::{CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::deps::DepIndex;
use crate::formula_cells::FormulaCellId;
use crate::storage::ColumnStore;
use crate::workbook::Workbook;

/// Most arrows one query returns.
pub const MAX_TRACES: usize = 10_000;
/// Most cells scanned for error values per target (larger targets are
/// reported without the error flag).
const MAX_ERROR_SCAN: u64 = 1_000_000;
/// Deepest error-source chain followed.
const MAX_ERROR_DEPTH: usize = 1_000;
/// Most stored cells one query scans for error values, all targets
/// together; past it targets are reported without the error flag
/// (security review 4, LOW-P11: 427 overlapping 1M-row ranges took 17 s).
pub const MAX_ERROR_SCAN_PER_QUERY: u64 = 5_000_000;

fn check(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> Result<CellAddr, ApiError> {
    if wb.sheet(sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {sheet}")));
    }
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(CellAddr::new(sheet, r, c))
}

/// `true` if any cell of `range` holds an error value. `budget` counts
/// the stored cells scanned by one query.
fn has_error(wb: &Workbook, range: &CellRange, budget: &mut u64) -> bool {
    first_error_cell(wb, range, budget).is_some()
}

fn trace(wb: &Workbook, range: CellRange, budget: &mut u64) -> TraceDto {
    TraceDto {
        sheet: range.sheet,
        range: RangeDto {
            r0: range.r0,
            c0: range.c0,
            r1: range.r1,
            c1: range.c1,
        },
        error: has_error(wb, &range, budget),
    }
}

/// The ranges a formula cell reads (static and dynamic), deduplicated.
fn precedent_ranges(wb: &Workbook, addr: CellAddr) -> Vec<CellRange> {
    let Some((_, f)) = wb.formula_at(addr).or_else(|| wb.array_at(addr)) else {
        return Vec::new();
    };
    let mut out: Vec<CellRange> = Vec::new();
    let mut seen: std::collections::HashSet<CellRange> = std::collections::HashSet::new();
    // 3-D spans are expanded lazily: the loop stops at MAX_TRACES.
    let all_spans = f.all_spans();
    let spans = all_spans
        .iter()
        .flat_map(|s| wb.span_sheets(s).iter().map(move |x| s.on(x.id)));
    for r in f
        .precedents
        .iter()
        .copied()
        .chain(f.dynamic_deps.iter().copied())
        .chain(spans)
    {
        if wb.sheet(r.sheet).is_none() {
            continue;
        }
        let clipped = CellRange::new(
            r.sheet,
            r.r0,
            r.c0,
            r.r1.min(MAX_ROWS - 1),
            r.c1.min(MAX_COLS - 1),
        );
        if seen.insert(clipped) {
            out.push(clipped);
        }
        if out.len() >= MAX_TRACES {
            break;
        }
    }
    out
}

/// The cells a formula reads (one level).
pub fn precedents(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Result<Vec<TraceDto>, ApiError> {
    let addr = check(wb, sheet, r, c)?;
    let mut budget = MAX_ERROR_SCAN_PER_QUERY;
    Ok(precedent_ranges(wb, addr)
        .into_iter()
        .map(|range| trace(wb, range, &mut budget))
        .collect())
}

/// The formulas that read a cell (one level).
pub fn dependents(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Result<Vec<TraceDto>, ApiError> {
    let addr = check(wb, sheet, r, c)?;
    let mut ids: Vec<FormulaCellId> = Vec::new();
    wb.deps.dependents_of(&[CellRange::single(addr)], &mut ids);
    // Name users: a formula reading a name that covers the cell.
    let mut seen: HashSet<FormulaCellId> = HashSet::new();
    let mut out = Vec::new();
    let mut budget = MAX_ERROR_SCAN_PER_QUERY;
    for id in ids {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = wb.formulas.get(id) else {
            continue;
        };
        if f.addr == addr {
            continue;
        }
        out.push(trace(wb, f.extent(), &mut budget));
        if out.len() >= MAX_TRACES {
            break;
        }
    }
    out.sort_by_key(|t| (t.sheet, t.range.r0, t.range.c0));
    Ok(out)
}

/// Trace Error: for an error cell, every precedent that holds an error,
/// followed through the first error cell of each such precedent down to
/// the cells where the errors originate (depth first, as AOO's
/// `ScDetectiveFunc::InsertErrorLevel`, sc/source/ui/docshell/detfunc.cxx:
/// 1016-1040, which follows every erroneous reference; correctness review
/// 4, M38: only the first one was followed). Empty when the cell holds no
/// error or is an origin itself. At most [`MAX_TRACES`] arrows.
pub fn error_source(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Result<Vec<TraceDto>, ApiError> {
    let addr = check(wb, sheet, r, c)?;
    if !matches!(wb.value(addr), Value::Error(_)) {
        return Ok(Vec::new());
    }
    let mut chain = Vec::new();
    let mut visited: HashSet<CellAddr> = HashSet::new();
    let mut budget = MAX_ERROR_SCAN_PER_QUERY;
    // (cell, depth); children are pushed in reverse so the first
    // precedent is explored first.
    let mut stack: Vec<(CellAddr, usize)> = vec![(addr, 0)];
    while let Some((at, depth)) = stack.pop() {
        if depth >= MAX_ERROR_DEPTH || !visited.insert(at) {
            continue;
        }
        let mut next: Vec<CellAddr> = Vec::new();
        for p in precedent_ranges(wb, at) {
            let Some(cell) = first_error_cell(wb, &p, &mut budget) else {
                continue;
            };
            chain.push(TraceDto {
                sheet: p.sheet,
                range: RangeDto {
                    r0: p.r0,
                    c0: p.c0,
                    r1: p.r1,
                    c1: p.c1,
                },
                error: true,
            });
            if chain.len() >= MAX_TRACES {
                return Ok(chain);
            }
            next.push(cell);
        }
        for cell in next.into_iter().rev() {
            stack.push((cell, depth + 1));
        }
    }
    Ok(chain)
}

/// The first cell of `range` holding an error value, scanning at most
/// `budget` stored cells (decremented).
fn first_error_cell(wb: &Workbook, range: &CellRange, budget: &mut u64) -> Option<CellAddr> {
    let s = wb.sheet(range.sheet)?;
    if range.is_single() {
        return matches!(wb.value(range.start()), Value::Error(_)).then(|| range.start());
    }
    if range.cell_count() > MAX_ERROR_SCAN || s.columns.is_empty() {
        return None;
    }
    let c1 = range.c1.min(s.columns.len().saturating_sub(1) as u32);
    for c in range.c0..=c1 {
        let Some(col) = s.column(c) else {
            continue;
        };
        for (r, _) in col.cells.iter_range(range.r0, range.r1) {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let a = CellAddr::new(range.sheet, r, c);
            if matches!(wb.value(a), Value::Error(_)) {
                return Some(a);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Menu commands: operation list, arrows, replay (AOO model)
// ---------------------------------------------------------------------------

use tally_api::{CellPos, DetectiveAction, DetectiveArrowDto, DetectiveState};
use tally_model::CellRangeLocal;
pub use tally_model::{
    DetectiveArrowData as Arrow, DetectiveOpData, DetectiveOpKindData as OpKind,
};

/// Work steps (formula visits and references examined) one operation may
/// take; past it the operation stops where it is.
pub const MAX_OP_WORK: u64 = 1_000_000;
/// Work steps one Refresh Traces command may take over all operations.
pub const MAX_REPLAY_WORK: u64 = 4_000_000;
/// Work steps of an automatic replay (AutoRefresh after an edit, undo or
/// redo, and opening a file): it must not stall typing.
pub const MAX_AUTO_REPLAY_WORK: u64 = 250_000;
/// Wall-clock limit of an automatic replay.
pub const AUTO_REPLAY_TIME: std::time::Duration = std::time::Duration::from_millis(40);
/// Most operations kept per document (AOO has one list per document; the
/// oldest go first).
pub const MAX_OPS: usize = 1_000;
/// AOO stops deepening at 1,000 levels (`ShowPred`, `ShowSucc`).
const MAX_LEVEL: u32 = 1_000;

/// One recorded operation at a cell of its sheet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct DetOp {
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u32,
    /// What it does.
    pub kind: OpKind,
}

/// A sheet's detective state ([`crate::storage::SheetMeta::detective`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetDetective {
    /// Operations acting on this sheet, in replay order.
    pub ops: Vec<DetOp>,
    /// The arrows on this sheet.
    pub arrows: Vec<Arrow>,
    /// The last replay ran out of budget: the arrows are the ones drawn
    /// until then.
    pub incomplete: bool,
    /// Bumped when rows or columns move the state in place (the app
    /// re-fetches the arrows).
    pub moves: u64,
}

impl SheetDetective {
    /// `true` when there are neither operations nor arrows.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.arrows.is_empty()
    }
}

/// AOO's `DET_INS_*` results.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Ins {
    Continue,
    Inserted,
    Empty,
    Circular,
}

/// Folds a sub-result into `acc` as `InsertPredLevel` does.
fn merge(acc: Ins, sub: Ins) -> Ins {
    match sub {
        Ins::Inserted => Ins::Inserted,
        Ins::Continue if acc != Ins::Inserted => Ins::Continue,
        Ins::Circular if acc == Ins::Empty => Ins::Circular,
        _ => acc,
    }
}

/// An arrow's identity for `HasArrow`: the start cell (the range's
/// top-left; `None` from another sheet) and the end cell (`None` to
/// another sheet).
type Key = (Option<(u32, u32)>, Option<(u32, u32)>);

fn key(a: &Arrow) -> Key {
    (a.from.map(|f| (f.r0, f.c0)), a.to)
}

fn local(r: &CellRange) -> CellRangeLocal {
    CellRangeLocal {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

/// The work, error-scan and time budgets of one command or one refresh,
/// shared by every sheet it touches, with the error lookups it made.
struct Budget {
    work: u64,
    errors: u64,
    deadline: Option<std::time::Instant>,
    ticks: u32,
    /// A budget ran out: the arrows drawn so far are kept (AOO just stops).
    exhausted: bool,
    /// The first error cell of each range examined (`HasError`).
    error_cells: HashMap<CellRange, Option<CellAddr>>,
}

impl Budget {
    fn new(work: u64, deadline: Option<std::time::Instant>) -> Self {
        Self {
            work,
            errors: MAX_ERROR_SCAN_PER_QUERY,
            deadline,
            ticks: 0,
            exhausted: false,
            error_cells: HashMap::new(),
        }
    }
}

/// One sheet's arrows while operations run on it (AOO `ScDetectiveFunc`
/// for one table).
struct Page<'a> {
    wb: &'a Workbook,
    sheet: SheetId,
    arrows: Vec<Arrow>,
    keys: HashSet<Key>,
    /// Formula cells being followed (AOO `ScFormulaCell::IsRunning`).
    running: HashSet<CellAddr>,
    max_level: u32,
    b: Budget,
}

/// A frame of the explicit stack of [`Page::find_pred_level`].
struct PredFrame {
    addr: CellAddr,
    children: Vec<(u32, u32)>,
    next: usize,
    level: u32,
}

/// A frame of the explicit stack of [`Page::find_succ_level`].
struct SuccFrame {
    range: CellRange,
    level: u32,
    readers: Vec<CellAddr>,
    next: usize,
    /// The reader being examined: (cell, it was running before, its references, next).
    cur: Option<(CellAddr, bool, Vec<CellRange>, usize)>,
}

impl<'a> Page<'a> {
    fn new(wb: &'a Workbook, sheet: SheetId, arrows: Vec<Arrow>, b: Budget) -> Self {
        let keys = arrows.iter().map(key).collect();
        Self {
            wb,
            sheet,
            arrows,
            keys,
            running: HashSet::new(),
            max_level: 0,
            b,
        }
    }

    fn out_of_work(&self) -> bool {
        self.b.work == 0
    }

    /// Takes one work step; `true` when the budget (steps or time) is gone.
    fn spent(&mut self) -> bool {
        if self.b.work == 0 {
            self.b.exhausted = true;
            return true;
        }
        self.b.work -= 1;
        self.b.ticks = self.b.ticks.wrapping_add(1);
        if self.b.ticks.is_multiple_of(1024)
            && self
                .b
                .deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
        {
            self.b.work = 0;
        }
        false
    }

    fn has_arrow(&self, k: Key) -> bool {
        self.keys.contains(&k)
    }

    fn push(&mut self, a: Arrow) -> bool {
        if self.arrows.len() >= MAX_TRACES {
            self.b.work = 0;
            self.b.exhausted = true;
            return false;
        }
        self.keys.insert(key(&a));
        self.arrows.push(a);
        true
    }

    /// `HasError` with its position, cached for the whole command.
    fn first_error(&mut self, r: &CellRange) -> Option<CellAddr> {
        if let Some(hit) = self.b.error_cells.get(r) {
            return *hit;
        }
        let hit = first_error_cell(self.wb, r, &mut self.b.errors);
        if self.b.error_cells.len() < 100_000 {
            self.b.error_cells.insert(*r, hit);
        }
        hit
    }

    /// The references of a formula cell, `None` when it holds no formula.
    fn refs(&self, cell: (u32, u32)) -> Option<Vec<CellRange>> {
        let addr = CellAddr::new(self.sheet, cell.0, cell.1);
        self.wb
            .formula_at(addr)
            .or_else(|| self.wb.array_at(addr))?;
        Some(precedent_ranges(self.wb, addr))
    }

    /// `DrawEntry`: an arrow from `r` to the formula at `cell` unless one
    /// is there; `false` when it was there.
    fn draw_entry(&mut self, cell: (u32, u32), r: &CellRange) -> bool {
        let alien = r.sheet != self.sheet;
        let start = (!alien).then_some((r.r0, r.c0));
        if self.has_arrow((start, Some(cell))) {
            return false;
        }
        let error = self.first_error(r).is_some();
        self.push(Arrow {
            from: (!alien).then(|| local(r)),
            to: Some(cell),
            error,
        })
    }

    /// `DrawAlienEntry`: an arrow from `r` (on this sheet) to a formula on
    /// another sheet.
    fn draw_alien_entry(&mut self, r: &CellRange) -> bool {
        if self.has_arrow((Some((r.r0, r.c0)), None)) {
            return false;
        }
        let error = self.first_error(r).is_some();
        self.push(Arrow {
            from: Some(local(r)),
            to: None,
            error,
        })
    }

    /// `DeleteArrowsAt`: the arrows ending (`dest`) or starting at `cell`.
    fn delete_arrows_at(&mut self, cell: (u32, u32), dest: bool) {
        let before = self.arrows.len();
        self.arrows.retain(|a| {
            if dest {
                a.to != Some(cell)
            } else {
                a.from.map(|f| (f.r0, f.c0)) != Some(cell)
            }
        });
        if self.arrows.len() != before {
            self.keys = self.arrows.iter().map(key).collect();
        }
    }

    /// Formula cells of `r` (this sheet), column by column as AOO's
    /// `ScCellIterator`.
    fn formula_cells_in(&mut self, r: &CellRange) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let Some(s) = self.wb.sheet(self.sheet) else {
            return out;
        };
        if s.columns.is_empty() {
            return out;
        }
        let c1 = r.c1.min(s.columns.len().saturating_sub(1) as u32);
        for c in r.c0..=c1 {
            let Some(col) = s.column(c) else {
                continue;
            };
            for (row, content) in col.cells.iter_range(r.r0, r.r1) {
                if self.spent() {
                    return out;
                }
                if matches!(content, crate::storage::CellContent::Formula(_)) {
                    out.push((row, c));
                }
            }
        }
        out
    }

    /// `InsertPredLevel` (recursion depth at most `max_level` ≤ [`MAX_LEVEL`]).
    fn insert_pred_level(&mut self, cell: (u32, u32), level: u32) -> Ins {
        if self.spent() {
            return Ins::Empty;
        }
        let Some(refs) = self.refs(cell) else {
            return Ins::Empty;
        };
        let addr = CellAddr::new(self.sheet, cell.0, cell.1);
        if !self.running.insert(addr) {
            return Ins::Circular;
        }
        let mut result = Ins::Empty;
        for r in &refs {
            // One step per reference examined.
            if self.spent() {
                break;
            }
            if self.draw_entry(cell, r) {
                result = Ins::Inserted;
            } else if level < self.max_level {
                // Follow the reference (AOO looks other sheets' cells up on
                // this sheet; Tally stops there).
                let sub = if r.sheet != self.sheet || self.out_of_work() {
                    Ins::Empty
                } else if r.is_single() {
                    self.insert_pred_level((r.r0, r.c0), level + 1)
                } else {
                    self.insert_pred_level_area(r, level + 1)
                };
                result = merge(result, sub);
            } else if result != Ins::Inserted {
                result = Ins::Continue;
            }
        }
        self.running.remove(&addr);
        result
    }

    /// `InsertPredLevelArea`.
    fn insert_pred_level_area(&mut self, r: &CellRange, level: u32) -> Ins {
        let mut result = Ins::Empty;
        for cell in self.formula_cells_in(r) {
            let sub = self.insert_pred_level(cell, level);
            result = merge(result, sub);
        }
        result
    }

    /// `FindPredLevel` / `FindPredLevelArea` without recursion: the number
    /// of precedent levels drawn from `cell`; with `del` (that number) it
    /// deletes the outermost level. Depth stops at [`MAX_LEVEL`] (AOO
    /// asserts `nLevel < 1000`).
    fn find_pred_level(&mut self, cell: (u32, u32), del: u32) -> u32 {
        let mut result = 0;
        let mut stack: Vec<PredFrame> = Vec::new();
        if let Some(f) = self.enter_pred(cell, 0, del, &mut result) {
            stack.push(f);
        }
        while let Some(top) = stack.last_mut() {
            if top.next < top.children.len() {
                let child = top.children[top.next];
                top.next += 1;
                let level = top.level + 1;
                if let Some(f) = self.enter_pred(child, level, del, &mut result) {
                    stack.push(f);
                }
            } else {
                let addr = top.addr;
                stack.pop();
                self.running.remove(&addr);
            }
        }
        result
    }

    /// Visits `cell` at `level` for [`Self::find_pred_level`]: deletes at
    /// the delete level, otherwise lists the cells to follow.
    fn enter_pred(
        &mut self,
        cell: (u32, u32),
        level: u32,
        del: u32,
        result: &mut u32,
    ) -> Option<PredFrame> {
        *result = (*result).max(level);
        if level >= MAX_LEVEL || self.spent() {
            return None;
        }
        let refs = self.refs(cell)?;
        let addr = CellAddr::new(self.sheet, cell.0, cell.1);
        if !self.running.insert(addr) {
            return None;
        }
        let mut children = Vec::new();
        if del != 0 && level + 1 == del {
            // The arrows ending here (boxes go with them).
            self.delete_arrows_at(cell, true);
        } else {
            for r in &refs {
                if self.spent() {
                    break;
                }
                if r.sheet != self.sheet {
                    if self.has_arrow((None, Some(cell))) {
                        *result = (*result).max(level + 1);
                    }
                    continue;
                }
                if self.has_arrow((Some((r.r0, r.c0)), Some(cell))) {
                    *result = (*result).max(level + 1);
                    if r.is_single() {
                        children.push((r.r0, r.c0));
                    } else {
                        children.extend(self.formula_cells_in(r));
                    }
                }
            }
        }
        Some(PredFrame {
            addr,
            children,
            next: 0,
            level,
        })
    }

    /// The formulas reading `r` (this sheet), restricted to this sheet
    /// when `local_only`, in AOO's iteration order (sheet, column, row).
    fn readers(&mut self, r: &CellRange, local_only: bool) -> Vec<CellAddr> {
        if self.spent() {
            return Vec::new();
        }
        let mut ids: Vec<FormulaCellId> = Vec::new();
        self.wb
            .deps
            .dependents_of(std::slice::from_ref(r), &mut ids);
        let mut seen: HashSet<FormulaCellId> = HashSet::new();
        let mut out: Vec<CellAddr> = Vec::new();
        for id in ids {
            if self.spent() {
                break;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(f) = self.wb.formulas.get(id) {
                if !local_only || f.addr.sheet == self.sheet {
                    out.push(f.addr);
                }
            }
        }
        let tab = |s: SheetId| self.wb.tab_of(s).unwrap_or(usize::MAX);
        out.sort_by_key(|a| (tab(a.sheet), a.col, a.row));
        out
    }

    /// `InsertSuccLevel` (recursion depth at most `max_level` ≤ [`MAX_LEVEL`]).
    fn insert_succ_level(&mut self, range: &CellRange, level: u32) -> Ins {
        let mut result = Ins::Empty;
        for f in self.readers(range, false) {
            if self.out_of_work() {
                break;
            }
            let was_running = !self.running.insert(f);
            for r in precedent_ranges(self.wb, f) {
                if self.spent() {
                    break;
                }
                if r.sheet != self.sheet || !r.intersects(range) {
                    continue;
                }
                let alien = f.sheet != self.sheet;
                let drawn = if alien {
                    self.draw_alien_entry(&r)
                } else {
                    self.draw_entry((f.row, f.col), &r)
                };
                if drawn {
                    result = Ins::Inserted;
                } else if was_running {
                    if result == Ins::Empty {
                        result = Ins::Circular;
                    }
                } else if level < self.max_level {
                    let sub = if alien {
                        Ins::Empty
                    } else {
                        self.insert_succ_level(&CellRange::single(f), level + 1)
                    };
                    result = merge(result, sub);
                } else if result != Ins::Inserted {
                    result = Ins::Continue;
                }
            }
            if !was_running {
                self.running.remove(&f);
            }
        }
        result
    }

    /// `FindSuccLevel` without recursion (depth stops at [`MAX_LEVEL`]).
    fn find_succ_level(&mut self, range: &CellRange, del: u32) -> u32 {
        let mut result = 0;
        let readers = self.readers(range, true);
        let mut stack = vec![SuccFrame {
            range: *range,
            level: 0,
            readers,
            next: 0,
            cur: None,
        }];
        while let Some(top) = stack.last_mut() {
            let level = top.level;
            let delete = del != 0 && level + 1 == del;
            if let Some((f, was_running, refs, i)) = &mut top.cur {
                if *i < refs.len() && self.b.work > 0 {
                    let r = refs[*i];
                    *i += 1;
                    let (f, was_running) = (*f, *was_running);
                    if self.spent() || r.sheet != self.sheet || !r.intersects(&top.range) {
                        continue;
                    }
                    if delete {
                        // The arrows starting at the reference (boxes go with them).
                        self.delete_arrows_at((r.r0, r.c0), false);
                    } else if !was_running
                        && self.has_arrow((Some((r.r0, r.c0)), Some((f.row, f.col))))
                    {
                        result = result.max(level + 1);
                        if level + 1 < MAX_LEVEL {
                            let child = CellRange::single(f);
                            let readers = self.readers(&child, true);
                            stack.push(SuccFrame {
                                range: child,
                                level: level + 1,
                                readers,
                                next: 0,
                                cur: None,
                            });
                        }
                    }
                    continue;
                }
                let (f, was_running) = (*f, *was_running);
                if !was_running {
                    self.running.remove(&f);
                }
                top.cur = None;
                continue;
            }
            if top.next < top.readers.len() && self.b.work > 0 {
                let f = top.readers[top.next];
                top.next += 1;
                let was_running = !self.running.insert(f);
                top.cur = Some((f, was_running, precedent_ranges(self.wb, f), 0));
            } else {
                stack.pop();
            }
        }
        result
    }

    /// `InsertErrorLevel` (recursion depth at most [`MAX_LEVEL`]).
    fn insert_error_level(&mut self, cell: (u32, u32), level: u32) -> Ins {
        if self.spent() {
            return Ins::Empty;
        }
        let Some(refs) = self.refs(cell) else {
            return Ins::Empty;
        };
        let addr = CellAddr::new(self.sheet, cell.0, cell.1);
        if !self.running.insert(addr) {
            return Ins::Circular;
        }
        let mut result = Ins::Empty;
        let mut has = false;
        for r in &refs {
            if self.spent() {
                break;
            }
            let Some(err) = self.first_error(r) else {
                continue;
            };
            has = true;
            if self.draw_entry(cell, &CellRange::single(err)) {
                result = Ins::Inserted;
            }
            if level < self.max_level
                && err.sheet == self.sheet
                && self.insert_error_level((err.row, err.col), level + 1) == Ins::Inserted
            {
                result = Ins::Inserted;
            }
        }
        self.running.remove(&addr);
        // An origin: its own (error-free) precedents.
        if !has && self.insert_pred_level(cell, self.max_level) == Ins::Inserted {
            result = Ins::Inserted;
        }
        result
    }

    /// `ShowPred`: one more level of precedent arrows.
    fn show_pred(&mut self, cell: (u32, u32)) -> bool {
        let mut result = Ins::Continue;
        let mut level = 0;
        while result == Ins::Continue && level < MAX_LEVEL && !self.out_of_work() {
            self.max_level = level;
            result = self.insert_pred_level(cell, 0);
            level += 1;
        }
        result == Ins::Inserted
    }

    /// `ShowSucc`: one more level of dependent arrows.
    fn show_succ(&mut self, cell: (u32, u32)) -> bool {
        let range = CellRange::single(CellAddr::new(self.sheet, cell.0, cell.1));
        let mut result = Ins::Continue;
        let mut level = 0;
        while result == Ins::Continue && level < MAX_LEVEL && !self.out_of_work() {
            self.max_level = level;
            result = self.insert_succ_level(&range, 0);
            level += 1;
        }
        result == Ins::Inserted
    }

    /// `ShowError`.
    fn show_error(&mut self, cell: (u32, u32)) -> bool {
        let addr = CellAddr::new(self.sheet, cell.0, cell.1);
        if !matches!(self.wb.value(addr), Value::Error(_)) {
            return false;
        }
        self.max_level = MAX_LEVEL;
        self.insert_error_level(cell, 0) == Ins::Inserted
    }

    /// `DeletePred`.
    fn delete_pred(&mut self, cell: (u32, u32)) -> bool {
        let n = self.find_pred_level(cell, 0);
        if n > 0 {
            self.find_pred_level(cell, n);
        }
        n != 0
    }

    /// `DeleteSucc`.
    fn delete_succ(&mut self, cell: (u32, u32)) -> bool {
        let range = CellRange::single(CellAddr::new(self.sheet, cell.0, cell.1));
        let n = self.find_succ_level(&range, 0);
        if n > 0 {
            self.find_succ_level(&range, n);
        }
        n != 0
    }

    fn apply(&mut self, op: DetOp) -> bool {
        let cell = (op.row, op.col);
        match op.kind {
            OpKind::TracePrecedents => self.show_pred(cell),
            OpKind::RemovePrecedents => self.delete_pred(cell),
            OpKind::TraceDependents => self.show_succ(cell),
            OpKind::RemoveDependents => self.delete_succ(cell),
            OpKind::TraceErrors => self.show_error(cell),
        }
    }
}

fn state_of(wb: &Workbook, sheet: SheetId) -> &SheetDetective {
    static EMPTY: std::sync::OnceLock<SheetDetective> = std::sync::OnceLock::new();
    wb.sheet(sheet)
        .map(|s| &*s.meta.detective)
        .unwrap_or_else(|| EMPTY.get_or_init(SheetDetective::default))
}

/// `true` when some sheet has a recorded operation.
pub fn has_ops(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|s| !s.meta.detective.ops.is_empty())
}

/// `true` when the list holds a Trace Error (AOO `ScDetOpList::HasAddError`:
/// AutoRefresh then redraws after any change, not only formula changes).
pub fn has_add_error(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|s| {
        s.meta
            .detective
            .ops
            .iter()
            .any(|o| o.kind == OpKind::TraceErrors)
    })
}

/// The API form of a sheet's state.
pub fn state(wb: &Workbook, sheet: SheetId, done: bool) -> DetectiveState {
    DetectiveState {
        done,
        arrows: state_of(wb, sheet)
            .arrows
            .iter()
            .map(|a| DetectiveArrowDto {
                from: a.from.map(|f| RangeDto {
                    r0: f.r0,
                    c0: f.c0,
                    r1: f.r1,
                    c1: f.c1,
                }),
                to: a.to.map(|(r, c)| CellPos { r, c }),
                error: a.error,
            })
            .collect(),
        has_ops: has_ops(wb),
        dirty: false,
        incomplete: state_of(wb, sheet).incomplete,
    }
}

fn store(wb: &mut Workbook, sheet: SheetId, det: SheetDetective) {
    if let Some(s) = wb.sheet_mut(sheet) {
        if *s.meta.detective != det {
            s.meta.detective = std::sync::Arc::new(det);
        }
    }
}

/// Runs a Tools ▸ Detective command on `(r, c)` of `sheet` (see
/// [`tally_api::DocumentService::detective`]). Returns whether it changed
/// something.
pub fn run(
    wb: &mut Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
    action: DetectiveAction,
) -> Result<bool, ApiError> {
    on_big_stack(|| run_here(wb, sheet, r, c, action))
}

/// Stack for the detective's recursion: up to [`MAX_LEVEL`] levels of
/// frames, more than the 2 MiB of a Tauri worker or test thread holds.
const DETECTIVE_STACK: usize = 256 << 20;

/// Runs `f` on a scoped thread with [`DETECTIVE_STACK`] of stack (virtual
/// memory, committed only as used), or here when no thread can be made.
fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    let mut f = Some(f);
    let out = std::thread::scope(|s| {
        let job = &mut f;
        std::thread::Builder::new()
            .name("tally-detective".into())
            .stack_size(DETECTIVE_STACK)
            .spawn_scoped(s, move || job.take().map(|f| f()))
            .ok()
            .and_then(|h| h.join().ok())
            .flatten()
    });
    match (out, f) {
        (Some(v), _) => v,
        (None, Some(f)) => f(),
        (None, None) => {
            // The thread ran and panicked: re-raise here.
            panic!("detective worker panicked")
        }
    }
}

fn run_here(
    wb: &mut Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
    action: DetectiveAction,
) -> Result<bool, ApiError> {
    check(wb, sheet, r, c)?;
    let kind = match action {
        DetectiveAction::TracePrecedents => OpKind::TracePrecedents,
        DetectiveAction::RemovePrecedents => OpKind::RemovePrecedents,
        DetectiveAction::TraceDependents => OpKind::TraceDependents,
        DetectiveAction::RemoveDependents => OpKind::RemoveDependents,
        DetectiveAction::TraceError => OpKind::TraceErrors,
        DetectiveAction::RemoveAll => return Ok(remove_all(wb, sheet)),
        DetectiveAction::Refresh => return Ok(refresh_here(wb, MAX_REPLAY_WORK, None)),
    };
    let op = DetOp {
        row: r,
        col: c,
        kind,
    };
    let total: usize = wb.sheets.iter().map(|s| s.meta.detective.ops.len()).sum();
    let mut det = state_of(wb, sheet).clone();
    let (done, arrows, exhausted) = {
        let budget = Budget::new(MAX_OP_WORK, None);
        let mut page = Page::new(wb, sheet, std::mem::take(&mut det.arrows), budget);
        let done = page.apply(op);
        (done, page.arrows, page.b.exhausted)
    };
    det.arrows = arrows;
    det.incomplete = exhausted;
    if done {
        // `ScDocFunc::Detective*` records only operations that did something.
        if total >= MAX_OPS && !det.ops.is_empty() {
            det.ops.remove(0);
        }
        det.ops.push(op);
    }
    store(wb, sheet, det);
    Ok(done)
}

/// Remove All Traces (`ScDocFunc::DetectiveDelAll`): the sheet's arrows
/// go and, when there were any, the whole document's operation list is
/// cleared (other sheets keep their arrows until their own Remove All,
/// as in AOO).
fn remove_all(wb: &mut Workbook, sheet: SheetId) -> bool {
    let had = !state_of(wb, sheet).arrows.is_empty();
    let any_ops = has_ops(wb);
    if !had && !any_ops {
        return false;
    }
    let ids: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    for id in ids {
        let mut det = state_of(wb, id).clone();
        det.ops.clear();
        if id == sheet {
            det.arrows.clear();
        }
        store(wb, id, det);
    }
    true
}

/// Refresh Traces (`ScDocFunc::DetectiveRefresh`) run automatically
/// (AutoRefresh, opening a file): when the operation list is not empty,
/// every sheet's arrows are cleared and the list replayed within
/// [`MAX_AUTO_REPLAY_WORK`] steps and [`AUTO_REPLAY_TIME`]; past them the
/// arrows drawn so far stay and the sheets are marked incomplete. Returns
/// whether it ran.
pub fn refresh_all(wb: &mut Workbook) -> bool {
    if !has_ops(wb) {
        return false;
    }
    let deadline = std::time::Instant::now() + AUTO_REPLAY_TIME;
    on_big_stack(|| refresh_here(wb, MAX_AUTO_REPLAY_WORK, Some(deadline)))
}

/// Replays every sheet's operations with one shared budget.
fn refresh_here(wb: &mut Workbook, work: u64, deadline: Option<std::time::Instant>) -> bool {
    if !has_ops(wb) {
        return false;
    }
    let mut budget = Budget::new(work, deadline);
    let ids: Vec<SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    for id in ids {
        let mut det = state_of(wb, id).clone();
        let (arrows, b) = {
            let mut page = Page::new(wb, id, Vec::new(), budget);
            for op in &det.ops {
                if page.out_of_work() {
                    page.b.exhausted = true;
                    break;
                }
                // Each operation gets at most its own budget out of the total.
                let total = page.b.work;
                let cap = total.min(MAX_OP_WORK);
                page.b.work = cap;
                page.apply(*op);
                page.b.work = total - (cap - page.b.work);
            }
            (page.arrows, page.b)
        };
        budget = b;
        det.incomplete = budget.exhausted && !det.ops.is_empty();
        det.arrows = arrows;
        store(wb, id, det);
    }
    true
}

/// Moves the operations and arrows of the changed sheet with inserted or
/// deleted rows or columns (`ScDetOpList::UpdateReference`; arrows are
/// cell-anchored draw objects in AOO). Operations and arrows whose cell
/// was deleted go.
pub fn adjust_structure(det: &mut SheetDetective, sh: &crate::structure::Shape) {
    use crate::structure::{point_after, span_after};
    det.moves = det.moves.wrapping_add(1);
    let cell = |r: u32, c: u32| -> Option<(u32, u32)> {
        if sh.rows {
            point_after(sh, r).map(|r| (r, c))
        } else {
            point_after(sh, c).map(|c| (r, c))
        }
    };
    let range = |f: CellRangeLocal| -> Option<CellRangeLocal> {
        if sh.rows {
            span_after(sh, f.r0, f.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..f })
        } else {
            span_after(sh, f.c0, f.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..f })
        }
    };
    det.ops = det
        .ops
        .iter()
        .filter_map(|o| {
            cell(o.row, o.col).map(|(row, col)| DetOp {
                row,
                col,
                kind: o.kind,
            })
        })
        .collect();
    det.arrows = det
        .arrows
        .iter()
        .filter_map(|a| {
            let from = match a.from {
                Some(f) => Some(range(f)?),
                None => None,
            };
            let to = match a.to {
                Some((r, c)) => Some(cell(r, c)?),
                None => None,
            };
            Some(Arrow {
                from,
                to,
                error: a.error,
            })
        })
        .collect();
}

/// Where a file's operations (all sheets) are cut to the [`MAX_OPS`] with
/// the highest `table:index`: the smallest `(index, tab, position)` kept,
/// `None` when all are kept. `sheets` yields each sheet's list in tab order.
pub fn ops_cut<'a>(
    sheets: impl Iterator<Item = &'a [DetectiveOpData]>,
) -> Option<(u32, usize, usize)> {
    let mut all: Vec<(u32, usize, usize)> = Vec::new();
    for (tab, list) in sheets.enumerate() {
        all.extend(list.iter().enumerate().map(|(i, o)| (o.index, tab, i)));
    }
    if all.len() <= MAX_OPS {
        return None;
    }
    all.sort_unstable_by(|a, b| b.cmp(a));
    Some(all[MAX_OPS - 1])
}

/// The sheet's operations for a file (`index` is renumbered by the
/// writer's caller across sheets).
pub fn ops_to_model(det: &SheetDetective) -> Vec<DetectiveOpData> {
    det.ops
        .iter()
        .enumerate()
        .map(|(i, o)| DetectiveOpData {
            row: o.row,
            col: o.col,
            kind: o.kind,
            index: i as u32,
        })
        .collect()
}

/// A sheet's operations from a file (sheet `tab`), in `index` order,
/// keeping those at or above the document-wide `cut` ([`ops_cut`]);
/// invalid cells are skipped.
pub fn ops_from_model(
    list: &[DetectiveOpData],
    tab: usize,
    cut: Option<(u32, usize, usize)>,
) -> Vec<DetOp> {
    let mut v: Vec<&DetectiveOpData> = list
        .iter()
        .enumerate()
        .filter(|(i, o)| {
            o.row < MAX_ROWS && o.col < MAX_COLS && cut.is_none_or(|c| (o.index, tab, *i) >= c)
        })
        .map(|(_, o)| o)
        .collect();
    v.sort_by_key(|o| o.index);
    v.into_iter()
        .map(|o| DetOp {
            row: o.row,
            col: o.col,
            kind: o.kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recalc::recalc;

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap()
    }

    #[test]
    fn precedents_dependents_and_error_source() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "1");
        wb.set_input(a("B1"), "=1/0");
        wb.set_input(a("C1"), "=A1+B1");
        wb.set_input(a("D1"), "=C1*2");
        wb.set_input(a("E1"), "=SUM(A1:B1)");
        recalc(&mut wb);
        let p = precedents(&wb, 1, 0, 2).unwrap();
        assert_eq!(p.len(), 2);
        assert!(!p[0].error && p[1].error, "{p:?}");
        let d = dependents(&wb, 1, 0, 0).unwrap();
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!((d[0].range.c0, d[1].range.c0), (2, 4));
        assert!(precedents(&wb, 1, 0, 0).unwrap().is_empty());
        let e = error_source(&wb, 1, 0, 3).unwrap();
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e[1].range.c0, 1, "B1 is the origin");
        assert!(error_source(&wb, 1, 0, 0).unwrap().is_empty());
        // Correctness review 4, M38: every error precedent is followed.
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "=1/0");
        wb.set_input(a("A2"), "=NA()");
        wb.set_input(a("A3"), "=A2");
        wb.set_input(a("B1"), "=A1+A3");
        recalc(&mut wb);
        let e = error_source(&wb, 1, 0, 1).unwrap();
        let cells: Vec<(u32, u32)> = e.iter().map(|t| (t.range.r0, t.range.c0)).collect();
        assert_eq!(cells, vec![(0, 0), (2, 0), (1, 0)], "{e:?}");
        assert!(precedents(&wb, 99, 0, 0).is_err());
    }
}
