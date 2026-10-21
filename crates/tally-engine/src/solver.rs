//! Solver (Tools ▸ Solver; wave 5, W5-3): a linear solver over the
//! workbook, run in a sandbox like Goal Seek (the document is never
//! changed; the UI applies `values` with `SetCellInput` if the user
//! accepts).
//!
//! The model is sampled by finite differences on a copy of the workbook:
//! the objective and every constraint cell are evaluated at the current
//! variable values, at a unit step along each variable and at a combined
//! step. A model whose samples are not affine (products, powers, IFs …)
//! is reported as [`SolverStatus::NotLinear`] (with the current values).
//! The affine model is then solved by a two-phase
//! dense simplex (Bland's rule, so it cannot cycle); integer and binary
//! constraints by depth-first branch and bound on the LP relaxation, within
//! `max_iterations` nodes and `timeout_ms`. Free variables are split into
//! positive and negative parts; `assume_non_negative` keeps them ≥ 0.
//! At most [`MAX_EVALS`] sandbox recalculations per request.

use std::time::{Duration, Instant};

use tally_api::{
    ApiError, SolverGoal, SolverOp, SolverRequest, SolverResult, SolverStatus, SolverValue,
};
use tally_types::{CellAddr, CellRange, Value, MAX_COLS, MAX_ROWS};

use crate::recalc::recalc;
use crate::storage::CellContent;
use crate::workbook::Workbook;

/// Most sandbox recalculations per request.
pub const MAX_EVALS: u32 = 10_000;
/// Most variable cells.
pub const MAX_VARIABLES: usize = 200;
/// Most constraint rows (after expanding ranges).
pub const MAX_CONSTRAINTS: usize = 2_000;
/// Finite-difference step.
const STEP: f64 = 1.0;
/// Relative tolerance of the linearity check.
const LIN_TOL: f64 = 1e-6;
/// Simplex pivot tolerance.
const EPS: f64 = 1e-9;

struct Probe {
    wb: Workbook,
    vars: Vec<CellAddr>,
    objective: CellAddr,
    /// `(lhs cell, rhs cell or constant)` per constraint row.
    rows: Vec<(CellAddr, Rhs)>,
    evals: u32,
}

#[derive(Clone, Copy, Debug)]
enum Rhs {
    Const(f64),
    Cell(CellAddr),
}

/// One sample: the objective and every row's `lhs - rhs`.
struct Sample {
    objective: f64,
    rows: Vec<f64>,
}

impl Probe {
    fn number(&self, addr: CellAddr) -> Result<f64, ApiError> {
        match self.wb.value(addr) {
            Value::Number(n) if n.is_finite() => Ok(n),
            Value::Bool(b) => Ok(f64::from(u8::from(b))),
            Value::Empty => Ok(0.0),
            other => Err(ApiError::invalid_argument(format!(
                "{} is not a number ({other:?})",
                addr.to_a1()
            ))),
        }
    }

    fn sample(&mut self, x: &[f64]) -> Result<Sample, ApiError> {
        if self.evals >= MAX_EVALS {
            return Err(ApiError::unsupported(format!(
                "the solver stopped after {MAX_EVALS} evaluations"
            )));
        }
        self.evals += 1;
        for (addr, v) in self.vars.iter().zip(x) {
            self.wb.set_value(*addr, Value::Number(*v));
        }
        recalc(&mut self.wb);
        let objective = self.number(self.objective)?;
        let mut rows = Vec::with_capacity(self.rows.len());
        for (lhs, rhs) in &self.rows {
            let l = self.number(*lhs)?;
            let r = match rhs {
                Rhs::Const(c) => *c,
                Rhs::Cell(a) => self.number(*a)?,
            };
            rows.push(l - r);
        }
        Ok(Sample { objective, rows })
    }
}

fn check_cell(
    wb: &Workbook,
    sheet: tally_types::SheetId,
    r: u32,
    c: u32,
) -> Result<CellAddr, ApiError> {
    if wb.sheet(sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {sheet}")));
    }
    if r >= MAX_ROWS || c >= MAX_COLS {
        return Err(ApiError::invalid_argument("cell outside the grid"));
    }
    Ok(CellAddr::new(sheet, r, c))
}

fn cells_of(
    wb: &Workbook,
    r: &tally_api::SheetRange,
    cap: usize,
) -> Result<Vec<CellAddr>, ApiError> {
    let range = crate::ops::to_range(r.sheet, &r.range)?;
    if wb.sheet(range.sheet).is_none() {
        return Err(ApiError::not_found(format!("no sheet {}", range.sheet)));
    }
    if range.cell_count() > cap as u64 {
        return Err(ApiError::invalid_argument(format!(
            "at most {cap} cells are allowed here"
        )));
    }
    let mut out = Vec::new();
    for c in range.c0..=range.c1 {
        for r in range.r0..=range.r1 {
            out.push(CellAddr::new(range.sheet, r, c));
        }
    }
    Ok(out)
}

/// An affine model: `f(x) = f0 + a·(x - x0)`.
struct Affine {
    f0: f64,
    a: Vec<f64>,
}

struct Linear {
    objective: Affine,
    rows: Vec<Affine>,
}

/// Samples the model and checks it is affine in the variables; `None`
/// when it is not.
fn linearize(p: &mut Probe, x0: &[f64]) -> Result<Option<Linear>, ApiError> {
    let n = x0.len();
    let base = p.sample(x0)?;
    let m = base.rows.len();
    let mut obj_a = vec![0.0; n];
    let mut rows_a = vec![vec![0.0; n]; m];
    let close = |expected: f64, got: f64| {
        (expected - got).abs() <= LIN_TOL * expected.abs().max(got.abs()).max(1.0)
    };
    for i in 0..n {
        let mut x = x0.to_vec();
        x[i] += STEP;
        let s1 = p.sample(&x)?;
        x[i] += STEP;
        let s2 = p.sample(&x)?;
        obj_a[i] = (s1.objective - base.objective) / STEP;
        if !close(base.objective + 2.0 * obj_a[i] * STEP, s2.objective) {
            return Ok(None);
        }
        for (k, row) in rows_a.iter_mut().enumerate() {
            row[i] = (s1.rows[k] - base.rows[k]) / STEP;
            if !close(base.rows[k] + 2.0 * row[i] * STEP, s2.rows[k]) {
                return Ok(None);
            }
        }
    }
    if n > 1 {
        // A combined step catches products between variables.
        let x: Vec<f64> = x0.iter().map(|v| v + STEP).collect();
        let s = p.sample(&x)?;
        let exp = base.objective + obj_a.iter().sum::<f64>() * STEP;
        if !close(exp, s.objective) {
            return Ok(None);
        }
        for (k, row) in rows_a.iter().enumerate() {
            let exp = base.rows[k] + row.iter().sum::<f64>() * STEP;
            if !close(exp, s.rows[k]) {
                return Ok(None);
            }
        }
    }
    Ok(Some(Linear {
        objective: Affine {
            f0: base.objective,
            a: obj_a,
        },
        rows: base
            .rows
            .iter()
            .zip(rows_a)
            .map(|(f0, a)| Affine { f0: *f0, a })
            .collect(),
    }))
}

// ---------------------------------------------------------------------------
// LP: minimize c·y subject to A y (≤ | ≥ | =) b, y ≥ 0
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rel {
    Le,
    Ge,
    Eq,
}

#[derive(Clone, Debug)]
struct Lp {
    a: Vec<Vec<f64>>,
    b: Vec<f64>,
    rel: Vec<Rel>,
    c: Vec<f64>,
}

#[derive(Debug)]
enum LpOutcome {
    Optimal {
        y: Vec<f64>,
        value: f64,
        pivots: u32,
    },
    Infeasible,
    Unbounded,
    Limit,
}

/// Two-phase dense simplex with Bland's rule.
fn simplex(lp: &Lp, max_pivots: u32, deadline: Instant) -> LpOutcome {
    let m = lp.a.len();
    let n = lp.c.len();
    // Columns: n structural, then one slack/surplus per ≤/≥ row, then one
    // artificial per ≥/= row, then the rhs.
    let mut rows: Vec<Vec<f64>> = Vec::with_capacity(m);
    let mut rel: Vec<Rel> = lp.rel.clone();
    let mut b = lp.b.clone();
    let mut a = lp.a.clone();
    for i in 0..m {
        if b[i] < 0.0 {
            b[i] = -b[i];
            for v in &mut a[i] {
                *v = -*v;
            }
            rel[i] = match rel[i] {
                Rel::Le => Rel::Ge,
                Rel::Ge => Rel::Le,
                Rel::Eq => Rel::Eq,
            };
        }
    }
    let n_slack = rel.iter().filter(|r| **r != Rel::Eq).count();
    let n_art = rel.iter().filter(|r| **r != Rel::Le).count();
    let total = n + n_slack + n_art;
    let mut basis: Vec<usize> = vec![0; m];
    let mut slack_i = 0;
    let mut art_i = 0;
    let mut artificial: Vec<bool> = vec![false; total];
    for i in 0..m {
        let mut row = vec![0.0; total + 1];
        row[..n].copy_from_slice(&a[i]);
        row[total] = b[i];
        match rel[i] {
            Rel::Le => {
                row[n + slack_i] = 1.0;
                basis[i] = n + slack_i;
                slack_i += 1;
            }
            Rel::Ge => {
                row[n + slack_i] = -1.0;
                slack_i += 1;
                let col = n + n_slack + art_i;
                row[col] = 1.0;
                artificial[col] = true;
                basis[i] = col;
                art_i += 1;
            }
            Rel::Eq => {
                let col = n + n_slack + art_i;
                row[col] = 1.0;
                artificial[col] = true;
                basis[i] = col;
                art_i += 1;
            }
        }
        rows.push(row);
    }
    let mut pivots = 0u32;
    // Phase 1: minimize the sum of artificials.
    if n_art > 0 {
        let mut obj = vec![0.0; total + 1];
        for (i, row) in rows.iter().enumerate() {
            if artificial[basis[i]] {
                for j in 0..=total {
                    obj[j] -= row[j];
                }
            }
        }
        for col in 0..total {
            if artificial[col] {
                obj[col] = 0.0;
            }
        }
        match run_phase(
            &mut rows,
            &mut obj,
            &mut basis,
            &artificial,
            false,
            max_pivots,
            deadline,
            &mut pivots,
        ) {
            Phase::Optimal => {}
            Phase::Unbounded => return LpOutcome::Infeasible,
            Phase::Limit => return LpOutcome::Limit,
        }
        if -obj[total] > 1e-7 {
            return LpOutcome::Infeasible;
        }
        // Drive artificials out of the basis where possible.
        for i in 0..m {
            if artificial[basis[i]] {
                if let Some(j) = (0..total).find(|j| !artificial[*j] && rows[i][*j].abs() > EPS) {
                    pivot(&mut rows, &mut basis, i, j);
                }
            }
        }
    }
    // Phase 2: minimize c·y with the artificial columns blocked.
    let mut obj = vec![0.0; total + 1];
    obj[..n].copy_from_slice(&lp.c[..n]);
    for (i, row) in rows.iter().enumerate() {
        let cb = if basis[i] < n { lp.c[basis[i]] } else { 0.0 };
        if cb != 0.0 {
            for j in 0..=total {
                obj[j] -= cb * row[j];
            }
        }
    }
    match run_phase(
        &mut rows,
        &mut obj,
        &mut basis,
        &artificial,
        true,
        max_pivots,
        deadline,
        &mut pivots,
    ) {
        Phase::Optimal => {}
        Phase::Unbounded => return LpOutcome::Unbounded,
        Phase::Limit => return LpOutcome::Limit,
    }
    let mut y = vec![0.0; n];
    for (i, &bv) in basis.iter().enumerate() {
        if bv < n {
            y[bv] = rows[i][total];
        }
    }
    LpOutcome::Optimal {
        y,
        value: -obj[total],
        pivots,
    }
}

enum Phase {
    Optimal,
    Unbounded,
    Limit,
}

#[allow(clippy::too_many_arguments)]
fn run_phase(
    rows: &mut [Vec<f64>],
    obj: &mut [f64],
    basis: &mut [usize],
    artificial: &[bool],
    block_artificial: bool,
    max_pivots: u32,
    deadline: Instant,
    pivots: &mut u32,
) -> Phase {
    let total = obj.len() - 1;
    loop {
        if *pivots >= max_pivots || Instant::now() >= deadline {
            return Phase::Limit;
        }
        // Bland: the lowest-index column with a negative reduced cost.
        let Some(col) =
            (0..total).find(|j| !(block_artificial && artificial[*j]) && obj[*j] < -EPS)
        else {
            return Phase::Optimal;
        };
        // Ratio test, ties by the lowest basis index.
        let mut best: Option<(usize, f64)> = None;
        for (i, row) in rows.iter().enumerate() {
            if row[col] > EPS {
                let ratio = row[total] / row[col];
                let better = match best {
                    None => true,
                    Some((bi, br)) => {
                        ratio < br - EPS || ((ratio - br).abs() <= EPS && basis[i] < basis[bi])
                    }
                };
                if better {
                    best = Some((i, ratio));
                }
            }
        }
        let Some((row_i, _)) = best else {
            return Phase::Unbounded;
        };
        pivot(rows, basis, row_i, col);
        let f = obj[col];
        if f != 0.0 {
            for j in 0..=total {
                obj[j] -= f * rows[row_i][j];
            }
        }
        *pivots += 1;
    }
}

fn pivot(rows: &mut [Vec<f64>], basis: &mut [usize], r: usize, c: usize) {
    let total = rows[r].len() - 1;
    let p = rows[r][c];
    for v in rows[r].iter_mut().take(total + 1) {
        *v /= p;
    }
    let pr = rows[r].clone();
    for (i, row) in rows.iter_mut().enumerate() {
        if i == r {
            continue;
        }
        let f = row[c];
        if f != 0.0 {
            for j in 0..=total {
                row[j] -= f * pr[j];
            }
        }
    }
    basis[r] = c;
}

// ---------------------------------------------------------------------------
// Branch and bound
// ---------------------------------------------------------------------------

struct Model {
    /// The LP over `y` (split variables).
    lp: Lp,
    /// `x_i = y[pos_i] - y[neg_i]` (`neg_i` absent when non-negative).
    pos: Vec<usize>,
    neg: Vec<Option<usize>>,
    integer: Vec<bool>,
    /// Objective constant and sign (`+1` minimize, `-1` maximize).
    constant: f64,
    sign: f64,
}

impl Model {
    fn x_of(&self, y: &[f64]) -> Vec<f64> {
        self.pos
            .iter()
            .zip(&self.neg)
            .map(|(p, n)| y[*p] - n.map_or(0.0, |n| y[n]))
            .collect()
    }

    /// A bound on `x_i` as an extra LP row.
    fn bound_row(&self, i: usize, rel: Rel, v: f64) -> (Vec<f64>, Rel, f64) {
        let mut row = vec![0.0; self.lp.c.len()];
        row[self.pos[i]] = 1.0;
        if let Some(n) = self.neg[i] {
            row[n] = -1.0;
        }
        (row, rel, v)
    }
}

struct Bb {
    nodes: u32,
    pivots: u32,
    max_nodes: u32,
    deadline: Instant,
    eps: f64,
    best: Option<(f64, Vec<f64>)>,
    limit_hit: bool,
    unbounded: bool,
}

fn branch(model: &Model, lp: Lp, bb: &mut Bb) {
    if bb.nodes >= bb.max_nodes || Instant::now() >= bb.deadline {
        bb.limit_hit = true;
        return;
    }
    bb.nodes += 1;
    let remaining = bb.max_nodes.saturating_sub(bb.pivots).max(1_000);
    match simplex(&lp, remaining, bb.deadline) {
        LpOutcome::Infeasible => {}
        LpOutcome::Unbounded => bb.unbounded = true,
        LpOutcome::Limit => bb.limit_hit = true,
        LpOutcome::Optimal { y, value, pivots } => {
            bb.pivots += pivots;
            if bb.best.as_ref().is_some_and(|(b, _)| value >= *b - 1e-12) {
                return;
            }
            let x = model.x_of(&y);
            let frac = x
                .iter()
                .enumerate()
                .find(|(i, v)| model.integer[*i] && (*v - v.round()).abs() > bb.eps.max(1e-9));
            match frac {
                None => bb.best = Some((value, x)),
                Some((i, v)) => {
                    let floor = v.floor();
                    for (rel, bound) in [(Rel::Le, floor), (Rel::Ge, floor + 1.0)] {
                        let (row, rel, b) = model.bound_row(i, rel, bound);
                        let mut child = lp.clone();
                        child.a.push(row);
                        child.rel.push(rel);
                        child.b.push(b);
                        branch(model, child, bb);
                        if bb.unbounded {
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Runs the solver.
pub fn solve(wb: &Workbook, req: &SolverRequest) -> Result<SolverResult, ApiError> {
    let objective = check_cell(wb, req.objective.sheet, req.objective.r, req.objective.c)?;
    if wb.formula_at(objective).is_none() {
        return Err(ApiError::invalid_argument(
            "the target cell must contain a formula",
        ));
    }
    let mut vars: Vec<CellAddr> = Vec::new();
    for r in &req.variables {
        for a in cells_of(wb, r, MAX_VARIABLES)? {
            if !vars.contains(&a) {
                vars.push(a);
            }
        }
    }
    if vars.is_empty() {
        return Err(ApiError::invalid_argument("no variable cells"));
    }
    if vars.len() > MAX_VARIABLES {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_VARIABLES} variable cells"
        )));
    }
    let mut x0 = Vec::with_capacity(vars.len());
    for a in &vars {
        x0.push(match wb.sheet(a.sheet).and_then(|s| s.get(a.row, a.col)) {
            None => 0.0,
            Some(CellContent::Value(Value::Number(n))) => *n,
            Some(CellContent::Value(Value::Bool(b))) => f64::from(u8::from(*b)),
            Some(CellContent::Value(Value::Empty)) => 0.0,
            Some(CellContent::Value(_)) => {
                return Err(ApiError::invalid_argument(format!(
                    "variable cell {} must contain a number",
                    a.to_a1()
                )))
            }
            Some(_) => {
                return Err(ApiError::invalid_argument(format!(
                    "variable cell {} must not contain a formula",
                    a.to_a1()
                )))
            }
        });
    }
    let eps = if req.options.epsilon.is_finite() && req.options.epsilon >= 0.0 {
        req.options.epsilon
    } else {
        1e-9
    };
    let mut integer = vec![req.options.integer; vars.len()];
    let mut binary: Vec<bool> = vec![false; vars.len()];
    let mut rows: Vec<(CellAddr, Rhs)> = Vec::new();
    let mut rels: Vec<Rel> = Vec::new();
    for c in &req.constraints {
        let lhs = cells_of(wb, &c.lhs, MAX_CONSTRAINTS)?;
        match c.op {
            SolverOp::Integer | SolverOp::Binary => {
                for a in lhs {
                    let Some(i) = vars.iter().position(|v| *v == a) else {
                        return Err(ApiError::invalid_argument(format!(
                            "{} is not a variable cell (integer constraints apply to variables)",
                            a.to_a1()
                        )));
                    };
                    integer[i] = true;
                    if c.op == SolverOp::Binary {
                        binary[i] = true;
                    }
                }
            }
            SolverOp::Le | SolverOp::Ge | SolverOp::Eq => {
                let rhs = parse_rhs(wb, &c.rhs, lhs.first().copied().unwrap_or(objective))?;
                let rel = match c.op {
                    SolverOp::Le => Rel::Le,
                    SolverOp::Ge => Rel::Ge,
                    _ => Rel::Eq,
                };
                for a in lhs {
                    rows.push((a, rhs));
                    rels.push(rel);
                }
            }
        }
    }
    if rows.len() > MAX_CONSTRAINTS {
        return Err(ApiError::invalid_argument(format!(
            "at most {MAX_CONSTRAINTS} constraints"
        )));
    }
    let mut probe = Probe {
        wb: wb.clone(),
        vars: vars.clone(),
        objective,
        rows,
        evals: 0,
    };
    let Some(lin) = linearize(&mut probe, &x0)? else {
        let objective = match wb.value(objective) {
            Value::Number(n) => n,
            _ => 0.0,
        };
        return Ok(SolverResult {
            status: SolverStatus::NotLinear,
            objective,
            values: vars
                .iter()
                .zip(&x0)
                .map(|(a, v)| SolverValue {
                    cell: tally_api::SheetCell {
                        sheet: a.sheet,
                        r: a.row,
                        c: a.col,
                    },
                    value: *v,
                })
                .collect(),
            iterations: probe.evals,
        });
    };
    let n = vars.len();

    // Variables: y = [x⁺…, x⁻… (free vars only)].
    let non_negative = req.options.assume_non_negative;
    let mut pos = Vec::with_capacity(n);
    let mut neg = Vec::with_capacity(n);
    let mut ny = 0;
    for is_binary in binary.iter().take(n) {
        pos.push(ny);
        ny += 1;
        if non_negative || *is_binary {
            neg.push(None);
        } else {
            neg.push(Some(ny));
            ny += 1;
        }
    }
    let expand = |a: &[f64]| -> Vec<f64> {
        let mut row = vec![0.0; ny];
        for i in 0..n {
            row[pos[i]] = a[i];
            if let Some(k) = neg[i] {
                row[k] = -a[i];
            }
        }
        row
    };
    // Each affine row g(x) = g0 + a·(x - x0) (rel) 0  ⇔  a·x (rel) a·x0 - g0.
    let mut lp = Lp {
        a: Vec::new(),
        b: Vec::new(),
        rel: Vec::new(),
        c: vec![0.0; ny],
    };
    for (k, row) in lin.rows.iter().enumerate() {
        let dot: f64 = row.a.iter().zip(&x0).map(|(a, x)| a * x).sum();
        lp.a.push(expand(&row.a));
        lp.b.push(dot - row.f0);
        lp.rel.push(rels[k]);
    }
    for i in 0..n {
        if binary[i] {
            let mut row = vec![0.0; ny];
            row[pos[i]] = 1.0;
            lp.a.push(row);
            lp.b.push(1.0);
            lp.rel.push(Rel::Le);
        }
    }
    let obj_dot: f64 = lin.objective.a.iter().zip(&x0).map(|(a, x)| a * x).sum();
    let constant = lin.objective.f0 - obj_dot;
    let sign = match req.goal {
        SolverGoal::Maximize => -1.0,
        SolverGoal::Minimize => 1.0,
        SolverGoal::Value { value } => {
            if !value.is_finite() {
                return Err(ApiError::invalid_argument(
                    "the target value must be a number",
                ));
            }
            // Reach the value: an equality constraint, any feasible point.
            lp.a.push(expand(&lin.objective.a));
            lp.b.push(value - constant);
            lp.rel.push(Rel::Eq);
            0.0
        }
    };
    if sign != 0.0 {
        lp.c = expand(&lin.objective.a).iter().map(|v| sign * v).collect();
    }
    let model = Model {
        lp,
        pos,
        neg,
        integer,
        constant,
        sign,
    };
    let max_nodes = req.options.max_iterations.max(1);
    let timeout = Duration::from_millis(u64::from(req.options.timeout_ms.max(1)));
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let mut bb = Bb {
        nodes: 0,
        pivots: 0,
        max_nodes,
        deadline,
        eps,
        best: None,
        limit_hit: false,
        unbounded: false,
    };
    branch(&model, model.lp.clone(), &mut bb);
    let iterations = bb.nodes.saturating_add(bb.pivots);
    let (status, x) = match (bb.best.take(), bb.unbounded, bb.limit_hit) {
        (_, true, _) => (SolverStatus::Unbounded, x0.clone()),
        (Some((_, x)), _, limit) => (
            if limit {
                SolverStatus::Limit
            } else {
                SolverStatus::Optimal
            },
            x,
        ),
        (None, _, true) => (SolverStatus::Limit, x0.clone()),
        (None, _, false) => (SolverStatus::Infeasible, x0.clone()),
    };
    // Clean integers and tiny noise.
    let x: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if model.integer[i] {
                v.round()
            } else if v.abs() < 1e-12 {
                0.0
            } else {
                *v
            }
        })
        .collect();
    let objective_value = match probe.sample(&x) {
        Ok(s) => s.objective,
        Err(_) => model.constant + model.sign * 0.0,
    };
    Ok(SolverResult {
        status,
        objective: objective_value,
        values: vars
            .iter()
            .zip(&x)
            .map(|(a, v)| SolverValue {
                cell: tally_api::SheetCell {
                    sheet: a.sheet,
                    r: a.row,
                    c: a.col,
                },
                value: *v,
            })
            .collect(),
        iterations,
    })
}

/// A constraint's right side: a number (en-US) or a single-cell reference
/// in the UI dialect.
fn parse_rhs(wb: &Workbook, text: &str, origin: CellAddr) -> Result<Rhs, ApiError> {
    let t = text.trim();
    if let Ok(n) = t.replace(',', "").parse::<f64>() {
        if n.is_finite() {
            return Ok(Rhs::Const(n));
        }
    }
    if let Some(r) = wb.parse_reference_at(t, origin) {
        if r.is_single() {
            return Ok(Rhs::Cell(r.start()));
        }
        return Err(ApiError::invalid_argument(format!(
            "constraint value {t:?} must be a single cell"
        )));
    }
    let _ = CellRange::single(origin);
    Err(ApiError::invalid_argument(format!(
        "constraint value {t:?} is neither a number nor a cell"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_api::{RangeDto, SheetCell, SheetRange, SolverConstraint, SolverOptions};

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap()
    }

    fn cell(s: &str) -> SheetCell {
        let x = a(s);
        SheetCell {
            sheet: 1,
            r: x.row,
            c: x.col,
        }
    }

    fn rng(s: &str) -> SheetRange {
        let r = CellRange::from_a1(1, s).unwrap();
        SheetRange {
            sheet: 1,
            range: RangeDto {
                r0: r.r0,
                c0: r.c0,
                r1: r.r1,
                c1: r.c1,
            },
        }
    }

    fn con(lhs: &str, op: SolverOp, rhs: &str) -> SolverConstraint {
        SolverConstraint {
            lhs: rng(lhs),
            op,
            rhs: rhs.into(),
        }
    }

    /// The AOO help "maximum profit" example: products A, B, C with unit
    /// profits 5, 4, 6 and a shared resource budget.
    fn profit_book() -> Workbook {
        let mut wb = Workbook::new();
        wb.set_input(a("B1"), "0"); // units of A
        wb.set_input(a("B2"), "0"); // units of B
        wb.set_input(a("B3"), "0"); // units of C
        wb.set_input(a("C1"), "=5*B1+4*B2+6*B3"); // profit
        wb.set_input(a("C2"), "=B1+B2+B3"); // material ≤ 100
        wb.set_input(a("C3"), "=2*B1+B2+3*B3"); // hours ≤ 180
        wb.set_input(a("D1"), "40");
        recalc(&mut wb);
        wb
    }

    #[test]
    fn max_profit_example() {
        let wb = profit_book();
        let req = SolverRequest {
            objective: cell("C1"),
            goal: SolverGoal::Maximize,
            variables: vec![rng("B1:B3")],
            constraints: vec![
                con("C2", SolverOp::Le, "100"),
                con("C3", SolverOp::Le, "180"),
                con("B1", SolverOp::Le, "D1"),
            ],
            options: SolverOptions {
                assume_non_negative: true,
                ..SolverOptions::default()
            },
        };
        let r = solve(&wb, &req).unwrap();
        assert_eq!(r.status, SolverStatus::Optimal, "{r:?}");
        // LP optimum: A=40, B=40, C=20 → 480 (material 100, hours 180).
        assert!((r.objective - 480.0).abs() < 1e-6, "{r:?}");
        let v: Vec<f64> = r.values.iter().map(|v| v.value).collect();
        assert!(
            (v[0] - 40.0).abs() < 1e-6 && (v[1] - 40.0).abs() < 1e-6 && (v[2] - 20.0).abs() < 1e-6,
            "{v:?}"
        );
        // The document is untouched.
        assert_eq!(wb.value(a("B1")), Value::Number(0.0));
    }

    #[test]
    fn integer_constraint_respected() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "0");
        wb.set_input(a("A2"), "0");
        wb.set_input(a("B1"), "=A1+A2");
        wb.set_input(a("B2"), "=2*A1+3*A2");
        recalc(&mut wb);
        // max A1+A2 s.t. 2A1+3A2 ≤ 7.5, A1 ≤ 2.5, integer → A1=2, A2=1 (3).
        let req = SolverRequest {
            objective: cell("B1"),
            goal: SolverGoal::Maximize,
            variables: vec![rng("A1:A2")],
            constraints: vec![
                con("B2", SolverOp::Le, "7.5"),
                con("A1", SolverOp::Le, "2.5"),
                con("A1:A2", SolverOp::Integer, ""),
            ],
            options: SolverOptions {
                assume_non_negative: true,
                ..SolverOptions::default()
            },
        };
        let r = solve(&wb, &req).unwrap();
        assert_eq!(r.status, SolverStatus::Optimal, "{r:?}");
        assert_eq!(r.objective, 3.0, "{r:?}");
        assert!(r.values.iter().all(|v| v.value.fract() == 0.0), "{r:?}");
        // Without the integer constraint: A1 = 2.5, A2 = 5/6 → 3.333.
        let mut relaxed = req.clone();
        relaxed.constraints.pop();
        let r = solve(&wb, &relaxed).unwrap();
        assert!((r.objective - 3.0 - 1.0 / 3.0).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn minimize_free_variables_and_statuses() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "5");
        wb.set_input(a("B1"), "=A1*3-4");
        wb.set_input(a("B2"), "=A1");
        recalc(&mut wb);
        // min 3x-4 s.t. x ≥ -2 → x = -2 (free variable).
        let req = SolverRequest {
            objective: cell("B1"),
            goal: SolverGoal::Minimize,
            variables: vec![rng("A1")],
            constraints: vec![con("B2", SolverOp::Ge, "-2")],
            options: SolverOptions::default(),
        };
        let r = solve(&wb, &req).unwrap();
        assert_eq!(r.status, SolverStatus::Optimal);
        assert!((r.values[0].value + 2.0).abs() < 1e-9, "{r:?}");
        assert!((r.objective + 10.0).abs() < 1e-9);
        // Unbounded without the constraint.
        let mut unb = req.clone();
        unb.constraints.clear();
        assert_eq!(solve(&wb, &unb).unwrap().status, SolverStatus::Unbounded);
        // Infeasible: x ≥ 1 and x ≤ 0.
        let mut inf = req.clone();
        inf.constraints = vec![con("B2", SolverOp::Ge, "1"), con("B2", SolverOp::Le, "0")];
        assert_eq!(solve(&wb, &inf).unwrap().status, SolverStatus::Infeasible);
        // Value goal.
        let mut val = req.clone();
        val.goal = SolverGoal::Value { value: 11.0 };
        let r = solve(&wb, &val).unwrap();
        assert_eq!(r.status, SolverStatus::Optimal);
        assert!((r.values[0].value - 5.0).abs() < 1e-9, "{r:?}");
    }

    #[test]
    fn non_linear_is_refused() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "1");
        wb.set_input(a("A2"), "1");
        wb.set_input(a("B1"), "=A1*A2");
        recalc(&mut wb);
        let req = SolverRequest {
            objective: cell("B1"),
            goal: SolverGoal::Maximize,
            variables: vec![rng("A1:A2")],
            constraints: vec![con("A1:A2", SolverOp::Le, "3")],
            options: SolverOptions::default(),
        };
        let r = solve(&wb, &req).unwrap();
        assert_eq!(r.status, SolverStatus::NotLinear);
        assert_eq!(r.values.len(), 2);
        assert!((r.values[0].value - 1.0).abs() < 1e-12);
        wb.set_input(a("B1"), "=A1^2");
        recalc(&mut wb);
        assert_eq!(solve(&wb, &req).unwrap().status, SolverStatus::NotLinear);
    }
}
