//! Goal Seek: find the value of a constant cell for which a formula cell
//! reaches a target. The search runs on a copy of the workbook (the
//! document is never changed); the UI applies the result with
//! `SetCellInput` if the user accepts it.
//!
//! Method: secant steps from the current value; once two points bracket
//! the root, the Illinois variant of regula falsi (which cannot leave the
//! bracket). If the secant stalls, the search widens geometrically on both
//! sides to find a bracket. At most 100 formula evaluations; success when
//! `|f(x) - target| <= 1e-9 · max(1, |target|)`.

use tally_api::{ApiError, GoalSeekRequest, GoalSeekResult};
use tally_types::{CellAddr, Value, MAX_COLS, MAX_ROWS};

use crate::recalc::recalc;
use crate::storage::CellContent;
use crate::workbook::Workbook;

/// Most evaluations of the formula.
pub const MAX_ITERATIONS: u32 = 100;

struct Probe {
    wb: Workbook,
    var: CellAddr,
    formula: CellAddr,
    target: f64,
    evals: u32,
}

impl Probe {
    /// `f(x) - target`, or `None` if the result is not a number.
    fn f(&mut self, x: f64) -> Option<f64> {
        self.evals += 1;
        self.wb.set_value(self.var, Value::Number(x));
        recalc(&mut self.wb);
        match self.wb.value(self.formula) {
            Value::Number(n) if n.is_finite() => Some(n - self.target),
            Value::Bool(b) => Some(f64::from(u8::from(b)) - self.target),
            _ => None,
        }
    }

    fn budget(&self) -> bool {
        self.evals < MAX_ITERATIONS
    }
}

fn check_cell(wb: &Workbook, sheet: tally_types::SheetId, r: u32, c: u32) -> Result<(), ApiError> {
    if wb.sheet(sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {sheet}")));
    }
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(())
}

/// Runs a goal seek.
pub fn goal_seek(wb: &Workbook, req: &GoalSeekRequest) -> Result<GoalSeekResult, ApiError> {
    check_cell(
        wb,
        req.formula_sheet,
        req.formula_cell.r,
        req.formula_cell.c,
    )?;
    check_cell(
        wb,
        req.variable_sheet,
        req.variable_cell.r,
        req.variable_cell.c,
    )?;
    if !req.target.is_finite() {
        return Err(ApiError::invalid_argument("the target must be a number"));
    }
    let formula = CellAddr::new(req.formula_sheet, req.formula_cell.r, req.formula_cell.c);
    let var = CellAddr::new(req.variable_sheet, req.variable_cell.r, req.variable_cell.c);
    if wb.formula_at(formula).is_none() {
        return Err(ApiError::invalid_argument(
            "the formula cell must contain a formula",
        ));
    }
    let x0 = match wb.sheet(var.sheet).and_then(|s| s.get(var.row, var.col)) {
        None => 0.0,
        Some(CellContent::Value(Value::Number(n))) => *n,
        Some(CellContent::Value(Value::Bool(b))) => f64::from(u8::from(*b)),
        Some(CellContent::Value(_)) => {
            return Err(ApiError::invalid_argument(
                "the variable cell must contain a number",
            ))
        }
        Some(_) => {
            return Err(ApiError::invalid_argument(
                "the variable cell must not contain a formula",
            ))
        }
    };
    let mut p = Probe {
        wb: wb.clone(),
        var,
        formula,
        target: req.target,
        evals: 0,
    };
    let tol = 1e-9 * req.target.abs().max(1.0);
    let mut best = (x0, f64::INFINITY);
    // A root is reported only when it was bracketed (a sign change) or the
    // secant steps settled; an asymptote (`EXP(x) = 0`, `1/x = 0`) gets
    // |f| below the tolerance while x keeps running away (review 3, LOW).
    let mut settled = false;
    let note = |x: f64, fx: f64, best: &mut (f64, f64)| {
        if fx.abs() < best.1.abs() {
            *best = (x, fx);
        }
    };
    let done = |best: &(f64, f64), p: &Probe, settled: bool| -> GoalSeekResult {
        GoalSeekResult {
            found: best.1.abs() <= tol && settled,
            value: best.0,
            result: best.1 + p.target,
            iterations: p.evals,
        }
    };

    let Some(f0) = p.f(x0) else {
        return Ok(GoalSeekResult {
            found: false,
            value: x0,
            result: f64::NAN,
            iterations: p.evals,
        });
    };
    note(x0, f0, &mut best);
    if f0.abs() <= tol {
        return Ok(done(&best, &p, true));
    }
    // Secant from (x0, f0) and a nearby point.
    let mut a = (x0, f0);
    let step = if x0 == 0.0 { 1.0 } else { x0.abs() * 0.01 };
    let mut b_x = x0 + step;
    let mut b = match p.f(b_x) {
        Some(fb) => (b_x, fb),
        None => {
            b_x = x0 - step;
            match p.f(b_x) {
                Some(fb) => (b_x, fb),
                None => return Ok(done(&best, &p, false)),
            }
        }
    };
    note(b.0, b.1, &mut best);
    let mut bracket: Option<((f64, f64), (f64, f64))> = None;
    while bracket.is_none() && p.budget() && best.1.abs() > tol {
        if a.1.signum() != b.1.signum() {
            bracket = Some((a, b));
            break;
        }
        let denom = b.1 - a.1;
        let next = if denom != 0.0 && denom.is_finite() {
            b.0 - b.1 * (b.0 - a.0) / denom
        } else {
            f64::NAN
        };
        if !next.is_finite() || (next - b.0).abs() > 1e12 * (1.0 + b.0.abs()) {
            break;
        }
        match p.f(next) {
            Some(fx) => {
                note(next, fx, &mut best);
                if (next - b.0).abs() <= 1e-7 * next.abs().max(1.0) {
                    settled = true;
                }
                a = b;
                b = (next, fx);
            }
            None => break,
        }
    }
    // Widen geometrically to find a bracket.
    if bracket.is_none() && best.1.abs() > tol {
        let mut span = step.max(1.0);
        let base = best;
        while p.budget() && bracket.is_none() {
            for x in [base.0 + span, base.0 - span] {
                if !p.budget() {
                    break;
                }
                if let Some(fx) = p.f(x) {
                    note(x, fx, &mut best);
                    if fx.signum() != base.1.signum() {
                        bracket = Some((base, (x, fx)));
                        break;
                    }
                }
            }
            span *= 4.0;
            if !span.is_finite() || span > 1e300 {
                break;
            }
        }
    }
    // Illinois regula falsi inside the bracket.
    if let Some((mut lo, mut hi)) = bracket {
        settled = true;
        let mut side = 0i8;
        while p.budget() && best.1.abs() > tol {
            let denom = hi.1 - lo.1;
            let mut x = if denom != 0.0 {
                hi.0 - hi.1 * (hi.0 - lo.0) / denom
            } else {
                (lo.0 + hi.0) / 2.0
            };
            if !x.is_finite() || x <= lo.0.min(hi.0) || x >= lo.0.max(hi.0) {
                x = (lo.0 + hi.0) / 2.0;
            }
            let Some(fx) = p.f(x) else {
                break;
            };
            note(x, fx, &mut best);
            if fx.signum() == hi.1.signum() {
                hi = (x, fx);
                if side == -1 {
                    lo.1 /= 2.0;
                }
                side = -1;
            } else {
                lo = (x, fx);
                if side == 1 {
                    hi.1 /= 2.0;
                }
                side = 1;
            }
            if (hi.0 - lo.0).abs() <= f64::EPSILON * hi.0.abs().max(1.0) {
                break;
            }
        }
    }
    // A candidate the secant reached in a few large steps (a linear model
    // hits the root exactly) is confirmed by a sign change around it;
    // an asymptote has none.
    if !settled && best.1.abs() <= tol && p.evals + 2 <= MAX_ITERATIONS {
        let x = best.0;
        let delta = 1e-6 * x.abs().max(1.0);
        if let (Some(lo), Some(hi)) = (p.f(x - delta), p.f(x + delta)) {
            settled = lo == 0.0 || hi == 0.0 || lo.signum() != hi.signum();
        }
    }
    Ok(done(&best, &p, settled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_api::CellPos;

    fn req(target: f64) -> GoalSeekRequest {
        GoalSeekRequest {
            formula_sheet: 1,
            formula_cell: CellPos { r: 0, c: 1 },
            target,
            variable_sheet: 1,
            variable_cell: CellPos { r: 0, c: 0 },
        }
    }

    #[test]
    fn linear_and_nonlinear() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        assert_eq!(s, 1);
        wb.set_input(CellAddr::new(s, 0, 0), "1");
        wb.set_input(CellAddr::new(s, 0, 1), "=A1*3+2");
        recalc(&mut wb);
        let r = goal_seek(&wb, &req(20.0)).unwrap();
        assert!(r.found, "{r:?}");
        assert!((r.value - 6.0).abs() < 1e-9);
        assert!(r.iterations <= MAX_ITERATIONS);
        // The document is untouched.
        assert_eq!(wb.value(CellAddr::new(s, 0, 0)), Value::Number(1.0));
        // Square root by goal seek.
        wb.set_input(CellAddr::new(s, 0, 1), "=A1^2");
        recalc(&mut wb);
        let r = goal_seek(&wb, &req(2.0)).unwrap();
        assert!(r.found, "{r:?}");
        assert!((r.value.abs() - 2f64.sqrt()).abs() < 1e-6);
        // Impossible target.
        let r = goal_seek(&wb, &req(-1.0)).unwrap();
        assert!(!r.found);
        // A step function needs bracketing.
        wb.set_input(CellAddr::new(s, 0, 1), "=IF(A1>50,A1-40,0)");
        recalc(&mut wb);
        let r = goal_seek(&wb, &req(25.0)).unwrap();
        assert!(r.found, "{r:?}");
        assert!((r.value - 65.0).abs() < 1e-6);
    }

    #[test]
    fn bad_requests() {
        let mut wb = Workbook::new();
        let s = wb.sheets[0].id;
        wb.set_input(CellAddr::new(s, 0, 1), "5");
        assert!(goal_seek(&wb, &req(1.0)).is_err(), "not a formula");
        wb.set_input(CellAddr::new(s, 0, 1), "=A1");
        wb.set_input(CellAddr::new(s, 0, 0), "abc");
        assert!(goal_seek(&wb, &req(1.0)).is_err(), "text variable");
    }
}
