//! Detective (Tools ▸ Detective; wave 5, W5-3): the direct precedents and
//! dependents of a cell as arrow targets, and error-source tracing.
//!
//! Precedents come from the formula's static precedents plus the dynamic
//! references recorded by its last evaluation (INDIRECT, OFFSET, …), one
//! level deep, as AOO's `ScDetectiveFunc::ShowPred`. Dependents are the
//! formulas registered in the reverse index as reading the cell (an array
//! formula's whole block counts as one target). A target is flagged
//! `error` when any of its cells holds an error value (the red arrow).
//! [`error_source`] follows the error precedents of an error cell until it
//! reaches one whose own precedents are error-free (AOO "Trace Error").

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

fn check(wb: &Workbook, sheet: SheetId, r: u32, c: u32) -> Result<CellAddr, ApiError> {
    if wb.sheet(sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {sheet}")));
    }
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(CellAddr::new(sheet, r, c))
}

/// `true` if any cell of `range` holds an error value.
fn has_error(wb: &Workbook, range: &CellRange) -> bool {
    let Some(s) = wb.sheet(range.sheet) else {
        return false;
    };
    if range.is_single() {
        return matches!(wb.value(range.start()), Value::Error(_));
    }
    if range.cell_count() > MAX_ERROR_SCAN {
        return false;
    }
    let c1 = range.c1.min(s.columns.len().saturating_sub(1) as u32);
    if s.columns.is_empty() {
        return false;
    }
    for c in range.c0..=c1 {
        let Some(col) = s.column(c) else {
            continue;
        };
        for (r, _) in col.cells.iter_range(range.r0, range.r1) {
            if matches!(wb.value(CellAddr::new(range.sheet, r, c)), Value::Error(_)) {
                return true;
            }
        }
    }
    false
}

fn trace(wb: &Workbook, range: CellRange) -> TraceDto {
    TraceDto {
        sheet: range.sheet,
        range: RangeDto {
            r0: range.r0,
            c0: range.c0,
            r1: range.r1,
            c1: range.c1,
        },
        error: has_error(wb, &range),
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
    Ok(precedent_ranges(wb, addr)
        .into_iter()
        .map(|range| trace(wb, range))
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
        out.push(trace(wb, f.extent()));
        if out.len() >= MAX_TRACES {
            break;
        }
    }
    out.sort_by_key(|t| (t.sheet, t.range.r0, t.range.c0));
    Ok(out)
}

/// Trace Error: for an error cell, the precedent chain down to the cell
/// where the error originates (the last cell holding an error whose own
/// precedents hold none). Empty when the cell holds no error or is the
/// origin itself.
pub fn error_source(
    wb: &Workbook,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> Result<Vec<TraceDto>, ApiError> {
    let mut addr = check(wb, sheet, r, c)?;
    if !matches!(wb.value(addr), Value::Error(_)) {
        return Ok(Vec::new());
    }
    let mut chain = Vec::new();
    let mut visited: HashSet<CellAddr> = HashSet::new();
    for _ in 0..MAX_ERROR_DEPTH {
        if !visited.insert(addr) {
            break;
        }
        let Some(next) = precedent_ranges(wb, addr)
            .into_iter()
            .find(|p| has_error(wb, p))
        else {
            break;
        };
        chain.push(trace(wb, next));
        // Continue from the first error cell of that precedent.
        let Some(cell) = first_error_cell(wb, &next) else {
            break;
        };
        addr = cell;
    }
    Ok(chain)
}

fn first_error_cell(wb: &Workbook, range: &CellRange) -> Option<CellAddr> {
    let s = wb.sheet(range.sheet)?;
    if range.is_single() {
        return Some(range.start());
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
        assert!(precedents(&wb, 99, 0, 0).is_err());
    }
}
