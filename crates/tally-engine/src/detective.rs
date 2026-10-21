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

use std::collections::HashSet;

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
    for r in f.precedents.iter().chain(f.dynamic_deps.iter()) {
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
        if !out.contains(&clipped) {
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
